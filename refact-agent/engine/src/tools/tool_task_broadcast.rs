use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::global_context::GlobalContext;
use crate::tasks::storage;
use crate::tools::planner_delivery::{self, CardGuard, CardTarget, DeliveryReport};
use crate::tools::task_tool_helpers::required_string;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use refact_core::chat_types::PushMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BroadcastPriority {
    Info,
    Steer,
    Urgent,
}

impl BroadcastPriority {
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

struct BroadcastResult {
    card_id: String,
    title: String,
    status: BroadcastStatus,
}

#[derive(Debug, PartialEq, Eq)]
enum BroadcastStatus {
    Notified,
    SkippedExcluded,
    Skipped(String),
    Failed(String),
}

pub struct ToolTaskBroadcast;

impl ToolTaskBroadcast {
    pub fn new() -> Self {
        Self
    }
}

fn parse_exclude_cards(args: &HashMap<String, Value>) -> Result<HashSet<String>, String> {
    let Some(value) = args.get("exclude_cards") else {
        return Ok(HashSet::new());
    };
    let Some(items) = value.as_array() else {
        return Err("exclude_cards must be an array of card IDs".to_string());
    };
    let mut excluded = HashSet::new();
    for item in items {
        let card_id = item
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "exclude_cards must contain only non-empty strings".to_string())?;
        excluded.insert(card_id.to_string());
    }
    Ok(excluded)
}

async fn planner_task_id(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<String, String> {
    let ccx_lock = ccx.lock().await;
    let meta = ccx_lock
        .task_meta
        .as_ref()
        .ok_or_else(|| "task_broadcast can only be called by the task planner.".to_string())?;
    if meta.role != "planner" {
        return Err("task_broadcast can only be called by the task planner.".to_string());
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

fn tool_message(tool_call_id: &str, content: String) -> ContextEnum {
    ContextEnum::ChatMessage(ChatMessage {
        role: "tool".to_string(),
        content: ChatContent::SimpleText(content),
        tool_calls: None,
        tool_call_id: tool_call_id.to_string(),
        ..Default::default()
    })
}

fn format_output(results: &[BroadcastResult], notified_count: usize, message: &str) -> String {
    if results.is_empty() {
        return format!(
            "📢 No running agents to broadcast to.\n\nMessage: \"{}\"",
            message
        );
    }

    let agent_word = if notified_count == 1 {
        "agent"
    } else {
        "agents"
    };
    let mut lines = vec![format!(
        "📢 Broadcast sent to {} {}",
        notified_count, agent_word
    )];
    lines.push(String::new());
    for result in results {
        let status = match &result.status {
            BroadcastStatus::Notified => "notified".to_string(),
            BroadcastStatus::SkippedExcluded => "skipped (in exclude_cards)".to_string(),
            BroadcastStatus::Skipped(reason) => format!("skipped ({})", reason),
            BroadcastStatus::Failed(error) => format!("failed ({})", error),
        };
        lines.push(format!(
            "- {} ({}): {}",
            result.card_id, result.title, status
        ));
    }
    lines.push(String::new());
    lines.push(format!("Message: \"{}\"", message));
    lines.join("\n")
}

async fn broadcast_to_target(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    target: CardTarget,
    message: &str,
    broadcast_message: &str,
    push: PushMode,
    chat_facade: Arc<dyn refact_runtime_api::ChatSessionFacade>,
) -> (BroadcastResult, bool) {
    let result = |status| BroadcastResult {
        card_id: target.card_id.clone(),
        title: target.title.clone(),
        status,
    };

    let pending_status = format!(
        "Broadcast pending: {}",
        planner_delivery::truncate_chars(message, 80)
    );
    match planner_delivery::reserve_card_status(
        gcx.clone(),
        task_id,
        &target,
        pending_status,
        true,
        false,
    )
    .await
    {
        Ok(None) => {}
        Ok(Some(reason)) => return (result(BroadcastStatus::Skipped(reason)), false),
        Err(error) => {
            return (
                result(BroadcastStatus::Failed(format!(
                    "status update failed: {}",
                    error
                ))),
                false,
            )
        }
    }

    let report = planner_delivery::deliver_to_card(
        gcx.clone(),
        chat_facade,
        task_id,
        &target,
        CardGuard::doing(),
        planner_delivery::single_message_delivery(
            broadcast_message.to_string(),
            push,
            "tools.task_broadcast",
            true,
        ),
    )
    .await;

    let reached = report.reached_agent();
    let follow_up = if reached {
        format!(
            "Broadcast delivered: {}",
            planner_delivery::truncate_chars(message, 80)
        )
    } else {
        format!("Broadcast {}", report.short_status())
    };
    let record_error = planner_delivery::record_card_status(gcx, task_id, &target, follow_up)
        .await
        .err();

    let status = match (&report, record_error) {
        (DeliveryReport::Delivered { .. }, None) if reached => BroadcastStatus::Notified,
        (DeliveryReport::Delivered { .. }, Some(error)) if reached => BroadcastStatus::Failed(
            format!("delivered, but delivery status update failed: {}", error),
        ),
        (DeliveryReport::Skipped { reason }, _) => BroadcastStatus::Skipped(reason.clone()),
        (report, Some(error)) => BroadcastStatus::Failed(format!(
            "{}; status update failed: {}",
            report.short_status(),
            error
        )),
        (report, None) => BroadcastStatus::Failed(report.short_status()),
    };

    (result(status), reached)
}

#[async_trait]
impl Tool for ToolTaskBroadcast {
    fn tool_description(&self) -> ToolDesc {
        let push_mode_schema = planner_delivery::push_mode_schema();
        ToolDesc {
            name: "task_broadcast".to_string(),
            display_name: "Task Broadcast".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Planner-only tool that broadcasts the same user message into all running task agent chats.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "Message to send to every running task agent"
                    },
                    "exclude_cards": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Doing card IDs whose agents should not receive the broadcast"
                    },
                    "priority": {
                        "type": "string",
                        "enum": ["info", "steer", "urgent"],
                        "description": "Wording of the message only (FYI/STEER/URGENT framing). It does NOT change delivery timing; use `push` for that. Default: steer"
                    },
                    "push": push_mode_schema,
                    "task_id": {
                        "type": "string",
                        "description": "Task ID (optional if chat is bound to a task)"
                    }
                },
                "required": ["message"]
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
        let message = required_string(args, "message")?;
        let excluded = parse_exclude_cards(args)?;
        let priority = BroadcastPriority::parse(args.get("priority"))?;
        // priority only shapes the wording; delivery timing comes from `push`.
        let push = planner_delivery::parse_push_mode(args)?;
        let broadcast_message = priority.format_message(&message);

        let (gcx, chat_facade) = {
            let ccx_lock = ccx.lock().await;
            (ccx_lock.app.gcx.clone(), ccx_lock.app.chat.facade.clone())
        };

        let board = storage::load_board(gcx.clone(), &task_id).await?;
        let targets: Vec<CardTarget> = board
            .cards
            .iter()
            .filter(|card| card.column == "doing")
            .filter_map(|card| {
                card.agent_chat_id
                    .as_ref()
                    .map(|chat_id| CardTarget::from_card(card, chat_id.clone()))
            })
            .collect();

        let mut results = Vec::new();
        let mut notified_count = 0usize;
        for target in targets {
            if excluded.contains(&target.card_id) {
                results.push(BroadcastResult {
                    card_id: target.card_id,
                    title: target.title,
                    status: BroadcastStatus::SkippedExcluded,
                });
                continue;
            }

            let (result, delivered) = broadcast_to_target(
                gcx.clone(),
                &task_id,
                target,
                &message,
                &broadcast_message,
                push,
                chat_facade.clone(),
            )
            .await;
            if delivered {
                notified_count += 1;
            }
            results.push(result);
        }

        Ok((
            false,
            vec![tool_message(
                tool_call_id,
                format_output(&results, notified_count, &message),
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
    use refact_chat_api::ChatCommand;
    use crate::app_state::AppState;
    use crate::chat::types::TaskMeta as ThreadTaskMeta;
    use crate::tasks::types::{BoardCard, TaskBoard, TaskMeta, TaskStatus};
    use crate::tools::tools_description::Tool;
    use refact_core::chat_types::{DeliveryOutcome, PendingDelivery};
    use refact_runtime_api::{
        ChatSessionFacade, ChatSessionSnapshot, ChatSessionUpdate, CreateSessionRequest,
        RuntimeTrajectorySnapshot, SessionState,
    };
    use chrono::Utc;
    use std::sync::Mutex as StdMutex;

    struct MockChatFacade {
        delivered: StdMutex<Vec<(String, PendingDelivery)>>,
        failures: StdMutex<HashMap<String, String>>,
    }

    impl MockChatFacade {
        fn new(failures: &[(&str, &str)]) -> Self {
            Self {
                delivered: StdMutex::new(vec![]),
                failures: StdMutex::new(
                    failures
                        .iter()
                        .map(|(chat_id, error)| ((*chat_id).to_string(), (*error).to_string()))
                        .collect(),
                ),
            }
        }

        fn deliveries(&self) -> Vec<(String, PendingDelivery)> {
            self.delivered.lock().unwrap().clone()
        }

        fn delivered_chat_ids(&self) -> Vec<String> {
            self.deliveries()
                .into_iter()
                .map(|(chat_id, _)| chat_id)
                .collect()
        }
    }

    #[async_trait]
    impl ChatSessionFacade for MockChatFacade {
        async fn session_snapshot(&self, _chat_id: &str) -> Result<ChatSessionSnapshot, String> {
            Ok(ChatSessionSnapshot {
                messages: vec![],
                thread: refact_chat_api::ThreadParams::default(),
                session_state: SessionState::Idle,
                pause_reasons: vec![],
                goal: None,
            })
        }

        async fn update_session(
            &self,
            _chat_id: &str,
            _update: ChatSessionUpdate,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn create_session(&self, _request: CreateSessionRequest) -> Result<(), String> {
            Ok(())
        }

        async fn push_command(&self, _chat_id: &str, _command: ChatCommand) -> Result<(), String> {
            panic!("task_broadcast must deliver through deliver_messages");
        }

        async fn deliver_messages(
            &self,
            chat_id: &str,
            delivery: PendingDelivery,
        ) -> Result<DeliveryOutcome, String> {
            if let Some(error) = self.failures.lock().unwrap().get(chat_id).cloned() {
                return Err(error);
            }
            self.delivered
                .lock()
                .unwrap()
                .push((chat_id.to_string(), delivery));
            Ok(DeliveryOutcome::Queued)
        }

        async fn session_state(&self, _chat_id: &str) -> Result<Option<SessionState>, String> {
            Ok(Some(SessionState::Idle))
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

    fn test_card(id: &str, title: &str, column: &str, agent_chat_id: Option<&str>) -> BoardCard {
        BoardCard {
            id: id.to_string(),
            title: title.to_string(),
            column: column.to_string(),
            priority: "P1".to_string(),
            depends_on: vec![],
            instructions: String::new(),
            assignee: Some("agent-1".to_string()),
            agent_chat_id: agent_chat_id.map(str::to_string),
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
            base_branch: None,
            base_commit: None,
            ab_variants: None,
            team_members: vec![],
            target_files: vec![],
            scope_guard_mode: Default::default(),
        }
    }

    fn test_card_with_heartbeat(
        id: &str,
        title: &str,
        column: &str,
        agent_chat_id: Option<&str>,
        last_heartbeat_at: &str,
    ) -> BoardCard {
        BoardCard {
            last_heartbeat_at: Some(last_heartbeat_at.to_string()),
            ..test_card(id, title, column, agent_chat_id)
        }
    }

    fn task_meta(cards_total: usize, agents_active: usize) -> TaskMeta {
        let now = Utc::now().to_rfc3339();
        TaskMeta {
            schema_version: 1,
            id: "task-1".to_string(),
            name: "Task".to_string(),
            status: TaskStatus::Active,
            created_at: now.clone(),
            updated_at: now,
            cards_total,
            cards_done: 0,
            cards_failed: 0,
            agents_active,
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
        let agents_active = cards
            .iter()
            .filter(|card| card.column == "doing" && card.agent_chat_id.is_some())
            .count();
        storage::save_task_meta(
            gcx.clone(),
            "task-1",
            &task_meta(cards.len(), agents_active),
        )
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
    async fn tool_task_broadcast_rejects_non_planner_role() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(
            temp.path(),
            vec![test_card("T-22", "auto-nudge", "doing", Some("chat-22"))],
        )
        .await;
        let mock = Arc::new(MockChatFacade::new(&[]));
        let ccx = planner_ccx(gcx, mock.clone(), "agents").await;

        let err = ToolTaskBroadcast::new()
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[("message", json!("Use API Y"))]),
            )
            .await
            .unwrap_err();

        assert!(err.contains("can only be called by the task planner"));
        assert!(mock.deliveries().is_empty());
    }

    #[tokio::test]
    async fn tool_task_broadcast_excludes_cards_and_updates_notified_cards() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(
            temp.path(),
            vec![
                test_card("T-22", "auto-nudge", "doing", Some("chat-22")),
                test_card("T-23", "documents", "doing", Some("chat-23")),
                test_card("T-29", "check_agents", "doing", Some("chat-29")),
            ],
        )
        .await;
        let mock = Arc::new(MockChatFacade::new(&[]));
        let ccx = planner_ccx(gcx.clone(), mock.clone(), "planner").await;

        let output = output_text(
            ToolTaskBroadcast::new()
                .tool_execute(
                    ccx,
                    &"call".to_string(),
                    &args(&[
                        (
                            "message",
                            json!("API X is now deprecated, use Y in all new code"),
                        ),
                        ("priority", json!("info")),
                        ("exclude_cards", json!(["T-23"])),
                    ]),
                )
                .await
                .unwrap(),
        );

        let deliveries = mock.deliveries();
        assert_eq!(deliveries.len(), 2);
        assert_eq!(mock.delivered_chat_ids(), vec!["chat-22", "chat-29"]);
        for (_, delivery) in deliveries {
            assert_eq!(delivery.messages.len(), 1);
            assert_eq!(
                delivery.messages[0].content.content_text_only(),
                "[Planner FYI] API X is now deprecated, use Y in all new code"
            );
            assert_eq!(delivery.push, PushMode::Append);
            assert_eq!(delivery.source, "tools.task_broadcast");
        }

        let board = storage::load_board(gcx, "task-1").await.unwrap();
        assert_eq!(board.get_card("T-22").unwrap().status_updates.len(), 2);
        assert_eq!(
            board.get_card("T-22").unwrap().status_updates[0].message,
            "Broadcast pending: API X is now deprecated, use Y in all new code"
        );
        assert_eq!(
            board.get_card("T-22").unwrap().status_updates[1].message,
            "Broadcast delivered: API X is now deprecated, use Y in all new code"
        );
        assert!(board.get_card("T-22").unwrap().last_heartbeat_at.is_none());
        assert!(board.get_card("T-23").unwrap().status_updates.is_empty());
        assert_eq!(board.get_card("T-29").unwrap().status_updates.len(), 2);
        assert_eq!(
            board.get_card("T-29").unwrap().status_updates[0].message,
            "Broadcast pending: API X is now deprecated, use Y in all new code"
        );
        assert_eq!(
            board.get_card("T-29").unwrap().status_updates[1].message,
            "Broadcast delivered: API X is now deprecated, use Y in all new code"
        );
        assert!(output.contains("📢 Broadcast sent to 2 agents"));
        assert!(output.contains("- T-23 (documents): skipped (in exclude_cards)"));
        assert!(output.contains("- T-22 (auto-nudge): notified"));
        assert!(output.contains("- T-29 (check_agents): notified"));
    }

    #[tokio::test]
    async fn task_broadcast_does_not_update_heartbeat() {
        let temp = tempfile::tempdir().unwrap();
        let initial_heartbeat = "2026-05-22T00:00:00+00:00";
        let gcx = write_task(
            temp.path(),
            vec![test_card_with_heartbeat(
                "T-22",
                "auto-nudge",
                "doing",
                Some("chat-22"),
                initial_heartbeat,
            )],
        )
        .await;
        let mock = Arc::new(MockChatFacade::new(&[]));
        let ccx = planner_ccx(gcx.clone(), mock.clone(), "planner").await;

        ToolTaskBroadcast::new()
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[("message", json!("Use API Y"))]),
            )
            .await
            .unwrap();

        let board = storage::load_board(gcx, "task-1").await.unwrap();
        assert_eq!(
            board.get_card("T-22").unwrap().last_heartbeat_at.as_deref(),
            Some(initial_heartbeat)
        );
    }

    #[tokio::test]
    async fn task_broadcast_appends_delivered_status_on_success() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(
            temp.path(),
            vec![test_card("T-22", "auto-nudge", "doing", Some("chat-22"))],
        )
        .await;
        let mock = Arc::new(MockChatFacade::new(&[]));
        let ccx = planner_ccx(gcx.clone(), mock.clone(), "planner").await;

        ToolTaskBroadcast::new()
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[("message", json!("Use API Y"))]),
            )
            .await
            .unwrap();

        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let status_updates = &board.get_card("T-22").unwrap().status_updates;
        assert_eq!(status_updates.len(), 2);
        assert_eq!(status_updates[0].message, "Broadcast pending: Use API Y");
        assert_eq!(status_updates[1].message, "Broadcast delivered: Use API Y");
    }

    #[tokio::test]
    async fn tool_task_broadcast_empty_doing_list_returns_no_running_agents() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(
            temp.path(),
            vec![
                test_card("T-22", "auto-nudge", "planned", Some("chat-22")),
                test_card("T-23", "documents", "doing", None),
            ],
        )
        .await;
        let mock = Arc::new(MockChatFacade::new(&[]));
        let ccx = planner_ccx(gcx, mock.clone(), "planner").await;

        let output = output_text(
            ToolTaskBroadcast::new()
                .tool_execute(
                    ccx,
                    &"call".to_string(),
                    &args(&[("message", json!("Use API Y"))]),
                )
                .await
                .unwrap(),
        );

        assert!(mock.deliveries().is_empty());
        assert!(output.contains("No running agents"));
    }

    #[tokio::test]
    async fn tool_task_broadcast_skips_card_finished_before_push() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(
            temp.path(),
            vec![test_card("T-22", "auto-nudge", "doing", Some("chat-22"))],
        )
        .await;
        storage::update_board_atomic(gcx.clone(), "task-1", |board| {
            let card = board.get_card_mut("T-22").unwrap();
            card.column = "done".to_string();
            card.completed_at = Some(Utc::now().to_rfc3339());
            Ok(())
        })
        .await
        .unwrap();
        let mock = Arc::new(MockChatFacade::new(&[]));
        let target = CardTarget {
            card_id: "T-22".to_string(),
            title: "auto-nudge".to_string(),
            chat_id: "chat-22".to_string(),
        };

        let (result, delivered) = broadcast_to_target(
            gcx.clone(),
            "task-1",
            target,
            "Use API Y",
            &BroadcastPriority::Steer.format_message("Use API Y"),
            PushMode::Append,
            mock.clone(),
        )
        .await;
        let output = format_output(&[result], 0, "Use API Y");

        assert!(!delivered);

        assert!(mock.deliveries().is_empty());
        assert!(output.contains("📢 Broadcast sent to 0 agents"));
        assert!(output.contains("- T-22 (auto-nudge): skipped (card T-22 is now in column 'done')"));
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-22").unwrap();
        assert_eq!(card.column, "done");
        assert!(card.status_updates.is_empty());
    }

    #[tokio::test]
    async fn tool_task_broadcast_skips_agent_chat_id_changed_before_push() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(
            temp.path(),
            vec![test_card("T-22", "auto-nudge", "doing", Some("chat-22"))],
        )
        .await;
        storage::update_board_atomic(gcx.clone(), "task-1", |board| {
            let card = board.get_card_mut("T-22").unwrap();
            card.agent_chat_id = Some("replacement-chat".to_string());
            Ok(())
        })
        .await
        .unwrap();
        let mock = Arc::new(MockChatFacade::new(&[]));
        let target = CardTarget {
            card_id: "T-22".to_string(),
            title: "auto-nudge".to_string(),
            chat_id: "chat-22".to_string(),
        };

        let (result, delivered) = broadcast_to_target(
            gcx.clone(),
            "task-1",
            target,
            "Use API Y",
            &BroadcastPriority::Steer.format_message("Use API Y"),
            PushMode::Append,
            mock.clone(),
        )
        .await;
        let output = format_output(&[result], 0, "Use API Y");

        assert!(!delivered);

        assert!(mock.deliveries().is_empty());
        assert!(output.contains("📢 Broadcast sent to 0 agents"));
        assert!(output.contains("- T-22 (auto-nudge): skipped (agent_chat_id changed"));
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-22").unwrap();
        assert_eq!(card.agent_chat_id.as_deref(), Some("replacement-chat"));
        assert!(card.status_updates.is_empty());
    }

    #[tokio::test]
    async fn tool_task_broadcast_reports_push_failures_without_aborting() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(
            temp.path(),
            vec![
                test_card("T-22", "auto-nudge", "doing", Some("chat-22")),
                test_card("T-29", "check_agents", "doing", Some("chat-29")),
            ],
        )
        .await;
        let mock = Arc::new(MockChatFacade::new(&[("chat-29", "queue unavailable")]));
        let ccx = planner_ccx(gcx.clone(), mock.clone(), "planner").await;

        let output = output_text(
            ToolTaskBroadcast::new()
                .tool_execute(
                    ccx,
                    &"call".to_string(),
                    &args(&[("message", json!("Use API Y"))]),
                )
                .await
                .unwrap(),
        );

        assert_eq!(mock.delivered_chat_ids(), vec!["chat-22"]);
        assert!(output.contains("📢 Broadcast sent to 1 agent"));
        assert!(output.contains("- T-22 (auto-nudge): notified"));
        assert!(
            output.contains("- T-29 (check_agents): failed")
                && output.contains("queue unavailable"),
            "{output}"
        );

        let board = storage::load_board(gcx, "task-1").await.unwrap();
        assert_eq!(board.get_card("T-22").unwrap().status_updates.len(), 2);
        assert_eq!(
            board.get_card("T-22").unwrap().status_updates[0].message,
            "Broadcast pending: Use API Y"
        );
        assert_eq!(
            board.get_card("T-22").unwrap().status_updates[1].message,
            "Broadcast delivered: Use API Y"
        );
        assert_eq!(board.get_card("T-29").unwrap().status_updates.len(), 2);
        assert_eq!(
            board.get_card("T-29").unwrap().status_updates[0].message,
            "Broadcast pending: Use API Y"
        );
        let failed_status = &board.get_card("T-29").unwrap().status_updates[1].message;
        assert!(
            failed_status.starts_with("Broadcast not delivered")
                && failed_status.contains("queue unavailable"),
            "failed broadcast must not read as delivered, got: {failed_status}"
        );
    }

    #[tokio::test]
    async fn task_broadcast_priority_urgent_still_defaults_to_append() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(
            temp.path(),
            vec![test_card("T-22", "auto-nudge", "doing", Some("chat-22"))],
        )
        .await;
        let mock = Arc::new(MockChatFacade::new(&[]));
        let ccx = planner_ccx(gcx, mock.clone(), "planner").await;

        ToolTaskBroadcast::new()
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[
                    ("message", json!("Use API Y")),
                    ("priority", json!("urgent")),
                ]),
            )
            .await
            .unwrap();

        let deliveries = mock.deliveries();
        assert_eq!(deliveries.len(), 1);
        assert_eq!(
            deliveries[0].1.push,
            PushMode::Append,
            "priority=urgent must NOT imply preempt"
        );
        assert!(deliveries[0].1.messages[0]
            .content
            .content_text_only()
            .starts_with("[Planner URGENT]"));
    }

    #[tokio::test]
    async fn task_broadcast_propagates_explicit_push_to_every_target() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(
            temp.path(),
            vec![
                test_card("T-22", "auto-nudge", "doing", Some("chat-22")),
                test_card("T-29", "check_agents", "doing", Some("chat-29")),
            ],
        )
        .await;
        let mock = Arc::new(MockChatFacade::new(&[]));
        let ccx = planner_ccx(gcx, mock.clone(), "planner").await;

        ToolTaskBroadcast::new()
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[
                    ("message", json!("Stop and re-read the plan")),
                    ("push", json!("preempt")),
                ]),
            )
            .await
            .unwrap();

        let deliveries = mock.deliveries();
        assert_eq!(deliveries.len(), 2);
        for (_, delivery) in deliveries {
            assert_eq!(delivery.push, PushMode::Preempt);
        }
    }

    #[tokio::test]
    async fn task_broadcast_rejects_invalid_push_before_any_delivery() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(
            temp.path(),
            vec![test_card("T-22", "auto-nudge", "doing", Some("chat-22"))],
        )
        .await;
        let mock = Arc::new(MockChatFacade::new(&[]));
        let ccx = planner_ccx(gcx.clone(), mock.clone(), "planner").await;

        let err = ToolTaskBroadcast::new()
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[("message", json!("hi")), ("push", json!("now"))]),
            )
            .await
            .unwrap_err();

        assert!(err.to_ascii_lowercase().contains("push"), "{err}");
        assert!(mock.deliveries().is_empty());
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        assert!(board.get_card("T-22").unwrap().status_updates.is_empty());
    }
}
