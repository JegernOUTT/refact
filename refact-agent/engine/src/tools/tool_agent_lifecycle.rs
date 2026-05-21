use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::global_context::GlobalContext;
use crate::tasks::storage;
use crate::tasks::types::StatusUpdate;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::worktrees::service::WorktreeService;
use refact_chat_api::ChatCommand;
use refact_runtime_api::{ChatSessionFacade, SessionState};

struct PlannerContext {
    task_id: String,
    gcx: Arc<GlobalContext>,
    chat_facade: Arc<dyn ChatSessionFacade>,
}

#[derive(Default)]
struct CleanupResult {
    worktree_removed: bool,
    branch_deleted: bool,
}

struct PushReport {
    pushed: bool,
    prior_state: Option<SessionState>,
}

pub struct ToolCancelAgent;
pub struct ToolPauseAgent;
pub struct ToolResumeAgent;

impl ToolCancelAgent {
    pub fn new() -> Self {
        Self
    }
}

impl ToolPauseAgent {
    pub fn new() -> Self {
        Self
    }
}

impl ToolResumeAgent {
    pub fn new() -> Self {
        Self
    }
}

async fn planner_context(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
    tool_name: &str,
) -> Result<PlannerContext, String> {
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
    if let Some(task_id) = args.get("task_id").and_then(|value| value.as_str()) {
        if task_id != meta.task_id {
            return Err(format!(
                "Supplied task_id '{}' does not match bound task_id '{}'",
                task_id, meta.task_id
            ));
        }
    }
    Ok(PlannerContext {
        task_id: meta.task_id.clone(),
        gcx: ccx_lock.app.gcx.clone(),
        chat_facade: ccx_lock.app.chat.facade.clone(),
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

fn optional_string(args: &HashMap<String, Value>, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn optional_bool(args: &HashMap<String, Value>, key: &str, default: bool) -> bool {
    match args.get(key) {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => match value.to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" => true,
            "false" | "no" | "0" => false,
            _ => default,
        },
        _ => default,
    }
}

fn user_message_command(content: String) -> ChatCommand {
    ChatCommand::UserMessage {
        content: Value::String(content),
        attachments: vec![],
        context_files: vec![],
        suppress_auto_enrichment: false,
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

fn state_label(state: Option<SessionState>) -> String {
    state
        .map(|state| state.to_string())
        .unwrap_or_else(|| "unavailable".to_string())
}

fn push_report_text(report: &PushReport, command_name: &str) -> String {
    if report.pushed {
        format!(
            "{} command queued; prior agent state: {}",
            command_name,
            state_label(report.prior_state)
        )
    } else {
        "No live agent session found; no command was queued.".to_string()
    }
}

async fn push_if_live(
    chat_facade: Arc<dyn ChatSessionFacade>,
    chat_id: &str,
    command: ChatCommand,
) -> Result<PushReport, String> {
    let prior_state = chat_facade.session_state(chat_id).await?;
    if prior_state.is_some() {
        chat_facade.push_command(chat_id, command).await?;
        Ok(PushReport {
            pushed: true,
            prior_state,
        })
    } else {
        Ok(PushReport {
            pushed: false,
            prior_state,
        })
    }
}

async fn state_after(chat_facade: Arc<dyn ChatSessionFacade>, chat_id: Option<&str>) -> String {
    let Some(chat_id) = chat_id else {
        return "unavailable".to_string();
    };
    match chat_facade.session_state(chat_id).await {
        Ok(state) => state_label(state),
        Err(error) => format!("unknown ({})", error),
    }
}

async fn cleanup_agent_worktree(
    gcx: Arc<GlobalContext>,
    agent_worktree_name: Option<String>,
    agent_worktree: Option<String>,
    agent_branch: Option<String>,
) -> CleanupResult {
    let mut result = CleanupResult::default();
    if let Some(worktree_name) = agent_worktree_name.as_deref() {
        let cache_dir = gcx.cache_dir.clone();
        let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
        if let Some(source_root) = project_dirs.first() {
            if let Ok(service) = WorktreeService::new(cache_dir, source_root.clone()) {
                if let Ok(deleted) = service.delete_worktree(worktree_name, true).await {
                    result.worktree_removed = deleted.deleted
                        && agent_worktree
                            .as_deref()
                            .map(|path| !Path::new(path).exists())
                            .unwrap_or(true);
                    result.branch_deleted = deleted.branch_deleted;
                    if result.worktree_removed || agent_worktree.is_none() {
                        return result;
                    }
                }
            }
        }
    }

    let project_dirs = crate::files_correction::get_project_dirs(gcx).await;
    if let (Some(workspace_root), Some(worktree)) =
        (project_dirs.first(), agent_worktree.as_deref())
    {
        let removed = Command::new("git")
            .args(["worktree", "remove", worktree, "--force"])
            .current_dir(workspace_root)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);
        result.worktree_removed = removed || !Path::new(worktree).exists();
    }

    if let Some(worktree) = agent_worktree.as_deref() {
        if Path::new(worktree).exists() && std::fs::remove_dir_all(worktree).is_ok() {
            result.worktree_removed = true;
        }
    }

    if let (Some(workspace_root), Some(branch)) = (project_dirs.first(), agent_branch.as_deref()) {
        let deleted = Command::new("git")
            .args(["branch", "-D", branch])
            .current_dir(workspace_root)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);
        result.branch_deleted = deleted;
    }

    result
}

fn lifecycle_source() -> ToolSource {
    ToolSource {
        source_type: ToolSourceType::Builtin,
        config_path: String::new(),
    }
}

fn cancel_description() -> ToolDesc {
    ToolDesc {
        name: "cancel_agent".to_string(),
        display_name: "Cancel Agent".to_string(),
        source: lifecycle_source(),
        experimental: false,
        allow_parallel: false,
        description: "Planner-only tool that gracefully aborts a task agent, marks its card failed, and retains the worktree by default for restart_agent.".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "card_id": {
                    "type": "string",
                    "description": "Card ID whose agent should be cancelled"
                },
                "reason": {
                    "type": "string",
                    "description": "Planner-visible cancellation reason"
                },
                "retain_worktree": {
                    "type": "boolean",
                    "description": "Keep the agent worktree for restart_agent. Default: true"
                },
                "task_id": {
                    "type": "string",
                    "description": "Task ID (optional if chat is bound to a task)"
                }
            },
            "required": ["card_id", "reason"]
        }),
        output_schema: None,
        annotations: None,
    }
}

fn pause_description() -> ToolDesc {
    ToolDesc {
        name: "pause_agent".to_string(),
        display_name: "Pause Agent".to_string(),
        source: lifecycle_source(),
        experimental: false,
        allow_parallel: false,
        description: "Planner-only tool that asks a task agent to pause after its current action by queueing a planner pause message.".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "card_id": {
                    "type": "string",
                    "description": "Card ID whose agent should pause"
                },
                "reason": {
                    "type": "string",
                    "description": "Why the agent should pause"
                },
                "task_id": {
                    "type": "string",
                    "description": "Task ID (optional if chat is bound to a task)"
                }
            },
            "required": ["card_id", "reason"]
        }),
        output_schema: None,
        annotations: None,
    }
}

fn resume_description() -> ToolDesc {
    ToolDesc {
        name: "resume_agent".to_string(),
        display_name: "Resume Agent".to_string(),
        source: lifecycle_source(),
        experimental: false,
        allow_parallel: false,
        description: "Planner-only tool that resumes a paused task agent by queueing a planner resume message with an optional note.".to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "card_id": {
                    "type": "string",
                    "description": "Card ID whose agent should resume"
                },
                "note": {
                    "type": "string",
                    "description": "Optional resume note. Defaults to continuing from where the agent paused."
                },
                "task_id": {
                    "type": "string",
                    "description": "Task ID (optional if chat is bound to a task)"
                }
            },
            "required": ["card_id"]
        }),
        output_schema: None,
        annotations: None,
    }
}

#[async_trait]
impl Tool for ToolCancelAgent {
    fn tool_description(&self) -> ToolDesc {
        cancel_description()
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let planner = planner_context(&ccx, args, "cancel_agent").await?;
        let card_id = required_string(args, "card_id")?;
        let reason = required_string(args, "reason")?;
        let retain_worktree = optional_bool(args, "retain_worktree", true);

        let board = storage::load_board(planner.gcx.clone(), &planner.task_id).await?;
        let card = board
            .get_card(&card_id)
            .ok_or_else(|| format!("Card {} not found", card_id))?;
        let card_title = card.title.clone();
        let agent_chat_id = card.agent_chat_id.clone();
        let agent_worktree = card.agent_worktree.clone();
        let agent_branch = card.agent_branch.clone();
        let agent_worktree_name = card.agent_worktree_name.clone();

        let push_report = if let Some(chat_id) = agent_chat_id.as_deref() {
            push_if_live(planner.chat_facade.clone(), chat_id, ChatCommand::Abort {}).await?
        } else {
            PushReport {
                pushed: false,
                prior_state: None,
            }
        };

        let cleanup_result = if retain_worktree {
            CleanupResult::default()
        } else {
            cleanup_agent_worktree(
                planner.gcx.clone(),
                agent_worktree_name.clone(),
                agent_worktree.clone(),
                agent_branch.clone(),
            )
            .await
        };

        let card_id_for_update = card_id.clone();
        let reason_for_update = reason.clone();
        let cleanup_clears_worktree = !retain_worktree && cleanup_result.worktree_removed;
        let cleanup_clears_branch = !retain_worktree && cleanup_result.branch_deleted;
        storage::update_board_atomic(planner.gcx.clone(), &planner.task_id, move |board| {
            let card = board
                .get_card_mut(&card_id_for_update)
                .ok_or_else(|| format!("Card {} not found", card_id_for_update))?;
            let now = Utc::now().to_rfc3339();
            card.column = "failed".to_string();
            card.completed_at = Some(now.clone());
            card.final_report = Some(format!("Cancelled: {}", reason_for_update));
            card.final_report_structured = None;
            card.last_heartbeat_at = Some(now.clone());
            card.status_updates.push(StatusUpdate {
                timestamp: now,
                message: format!("Cancelled by planner: {}", reason_for_update),
            });
            if cleanup_clears_worktree {
                card.agent_worktree = None;
                card.agent_worktree_name = None;
            }
            if cleanup_clears_branch {
                card.agent_branch = None;
            }
            Ok(())
        })
        .await?;
        storage::update_task_stats(planner.gcx.clone(), &planner.task_id).await?;

        let cleanup_text = if retain_worktree {
            "Worktree retained for restart_agent.".to_string()
        } else if cleanup_result.worktree_removed {
            "Worktree cleanup completed.".to_string()
        } else if agent_worktree.is_some() {
            "Worktree cleanup requested but no worktree removal was confirmed.".to_string()
        } else {
            "No worktree was recorded on the card.".to_string()
        };

        let output = format!(
            "✅ Cancelled {} ({})\n\n{}\n{}\nFinal report: Cancelled: {}",
            card_id,
            card_title,
            push_report_text(&push_report, "Abort"),
            cleanup_text,
            reason
        );

        Ok((false, vec![tool_message(tool_call_id, output)]))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolPauseAgent {
    fn tool_description(&self) -> ToolDesc {
        pause_description()
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let planner = planner_context(&ccx, args, "pause_agent").await?;
        let card_id = required_string(args, "card_id")?;
        let reason = required_string(args, "reason")?;

        let board = storage::load_board(planner.gcx.clone(), &planner.task_id).await?;
        let card = board
            .get_card(&card_id)
            .ok_or_else(|| format!("Card {} not found", card_id))?;
        let agent_chat_id = card.agent_chat_id.clone();
        let pause_message = format!("[Planner PAUSE] {}\n\nWait for resume signal.", reason);

        let push_report = if let Some(chat_id) = agent_chat_id.as_deref() {
            push_if_live(
                planner.chat_facade.clone(),
                chat_id,
                user_message_command(pause_message),
            )
            .await?
        } else {
            PushReport {
                pushed: false,
                prior_state: None,
            }
        };

        let card_id_for_update = card_id.clone();
        let reason_for_update = reason.clone();
        storage::update_board_atomic(planner.gcx.clone(), &planner.task_id, move |board| {
            let card = board
                .get_card_mut(&card_id_for_update)
                .ok_or_else(|| format!("Card {} not found", card_id_for_update))?;
            let now = Utc::now().to_rfc3339();
            card.last_heartbeat_at = Some(now.clone());
            card.status_updates.push(StatusUpdate {
                timestamp: now,
                message: format!("Paused by planner: {}", reason_for_update),
            });
            Ok(())
        })
        .await?;

        let output = format!(
            "⏸️ Paused {}; use resume_agent to continue.\n\n{}",
            card_id,
            push_report_text(&push_report, "Pause")
        );

        Ok((false, vec![tool_message(tool_call_id, output)]))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolResumeAgent {
    fn tool_description(&self) -> ToolDesc {
        resume_description()
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let planner = planner_context(&ccx, args, "resume_agent").await?;
        let card_id = required_string(args, "card_id")?;
        let note = optional_string(args, "note")
            .unwrap_or_else(|| "Continue from where you paused.".to_string());

        let board = storage::load_board(planner.gcx.clone(), &planner.task_id).await?;
        let card = board
            .get_card(&card_id)
            .ok_or_else(|| format!("Card {} not found", card_id))?;
        let agent_chat_id = card.agent_chat_id.clone();
        let resume_message = format!("[Planner RESUME]\n\n{}", note);

        let push_report = if let Some(chat_id) = agent_chat_id.as_deref() {
            push_if_live(
                planner.chat_facade.clone(),
                chat_id,
                user_message_command(resume_message),
            )
            .await?
        } else {
            PushReport {
                pushed: false,
                prior_state: None,
            }
        };

        let card_id_for_update = card_id.clone();
        storage::update_board_atomic(planner.gcx.clone(), &planner.task_id, move |board| {
            let card = board
                .get_card_mut(&card_id_for_update)
                .ok_or_else(|| format!("Card {} not found", card_id_for_update))?;
            let now = Utc::now().to_rfc3339();
            card.last_heartbeat_at = Some(now.clone());
            card.status_updates.push(StatusUpdate {
                timestamp: now,
                message: "Resumed by planner".to_string(),
            });
            Ok(())
        })
        .await?;

        let agent_state = state_after(planner.chat_facade.clone(), agent_chat_id.as_deref()).await;
        let output = format!(
            "▶️ Resumed {}.\n\n{}\nAgent state: {}",
            card_id,
            push_report_text(&push_report, "Resume"),
            agent_state
        );

        Ok((false, vec![tool_message(tool_call_id, output)]))
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
        ChatSessionSnapshot, ChatSessionUpdate, CreateSessionRequest, RuntimeTrajectorySnapshot,
    };
    use std::sync::Mutex as StdMutex;

    struct MockChatFacade {
        state: StdMutex<Option<SessionState>>,
        pushed: StdMutex<Vec<(String, ChatCommand)>>,
    }

    impl MockChatFacade {
        fn new(state: Option<SessionState>) -> Self {
            Self {
                state: StdMutex::new(state),
                pushed: StdMutex::new(vec![]),
            }
        }

        fn pushed_commands(&self) -> Vec<(String, ChatCommand)> {
            self.pushed.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl ChatSessionFacade for MockChatFacade {
        async fn session_snapshot(&self, _chat_id: &str) -> Result<ChatSessionSnapshot, String> {
            Ok(ChatSessionSnapshot {
                messages: vec![],
                thread: refact_chat_api::ThreadParams::default(),
                session_state: self.state.lock().unwrap().unwrap_or(SessionState::Idle),
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

        async fn push_command(&self, chat_id: &str, command: ChatCommand) -> Result<(), String> {
            self.pushed
                .lock()
                .unwrap()
                .push((chat_id.to_string(), command));
            Ok(())
        }

        async fn session_state(&self, _chat_id: &str) -> Result<Option<SessionState>, String> {
            Ok(*self.state.lock().unwrap())
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

    fn test_card(
        column: &str,
        agent_chat_id: Option<String>,
        worktree: Option<String>,
    ) -> BoardCard {
        BoardCard {
            id: "T-39".to_string(),
            title: "Lifecycle card".to_string(),
            column: column.to_string(),
            priority: "P1".to_string(),
            depends_on: vec![],
            instructions: String::new(),
            assignee: Some("agent-1".to_string()),
            agent_chat_id,
            status_updates: vec![],
            final_report: None,
            final_report_structured: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: Some(Utc::now().to_rfc3339()),
            last_heartbeat_at: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: worktree,
            agent_worktree_name: None,
            target_files: vec![],
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

    fn output_text(result: (bool, Vec<ContextEnum>)) -> String {
        match result.1.into_iter().next().unwrap() {
            ContextEnum::ChatMessage(message) => match message.content {
                ChatContent::SimpleText(text) => text,
                _ => panic!("expected text output"),
            },
            _ => panic!("expected chat message"),
        }
    }

    #[test]
    fn tool_agent_lifecycle_descriptions_are_planner_only() {
        let cancel = ToolCancelAgent::new().tool_description();
        let pause = ToolPauseAgent::new().tool_description();
        let resume = ToolResumeAgent::new().tool_description();

        assert_eq!(cancel.name, "cancel_agent");
        assert_eq!(pause.name, "pause_agent");
        assert_eq!(resume.name, "resume_agent");
        assert!(cancel.description.contains("Planner-only"));
        assert!(pause.description.contains("Planner-only"));
        assert!(resume.description.contains("Planner-only"));
        assert_eq!(
            cancel.input_schema["required"],
            json!(["card_id", "reason"])
        );
        assert_eq!(pause.input_schema["required"], json!(["card_id", "reason"]));
        assert_eq!(resume.input_schema["required"], json!(["card_id"]));
        assert!(cancel.input_schema["properties"]
            .get("retain_worktree")
            .is_some());
    }

    #[tokio::test]
    async fn tool_agent_lifecycle_rejects_non_planner_role() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(
            temp.path(),
            test_card("doing", Some("agent-chat-1".to_string()), None),
        )
        .await;
        let mock = Arc::new(MockChatFacade::new(Some(SessionState::Idle)));
        let ccx = planner_ccx(gcx, mock.clone(), "agents").await;
        let call_id = "call".to_string();

        let err = ToolCancelAgent::new()
            .tool_execute(
                ccx.clone(),
                &call_id,
                &args(&[("card_id", json!("T-39")), ("reason", json!("stop"))]),
            )
            .await
            .unwrap_err();
        assert!(err.contains("can only be called by the task planner"));

        let err = ToolPauseAgent::new()
            .tool_execute(
                ccx.clone(),
                &call_id,
                &args(&[("card_id", json!("T-39")), ("reason", json!("wait"))]),
            )
            .await
            .unwrap_err();
        assert!(err.contains("can only be called by the task planner"));

        let err = ToolResumeAgent::new()
            .tool_execute(ccx, &call_id, &args(&[("card_id", json!("T-39"))]))
            .await
            .unwrap_err();
        assert!(err.contains("can only be called by the task planner"));
        assert!(mock.pushed_commands().is_empty());
    }

    #[tokio::test]
    async fn cancel_agent_retains_worktree_by_default_and_pushes_abort() {
        let temp = tempfile::tempdir().unwrap();
        let worktree = temp.path().join("agent-worktree");
        tokio::fs::create_dir_all(&worktree).await.unwrap();
        let worktree_str = worktree.to_string_lossy().to_string();
        let gcx = write_task(
            temp.path(),
            test_card(
                "doing",
                Some("agent-chat-1".to_string()),
                Some(worktree_str.clone()),
            ),
        )
        .await;
        let mock = Arc::new(MockChatFacade::new(Some(SessionState::Generating)));
        let ccx = planner_ccx(gcx.clone(), mock.clone(), "planner").await;

        let output = output_text(
            ToolCancelAgent::new()
                .tool_execute(
                    ccx,
                    &"call".to_string(),
                    &args(&[
                        ("card_id", json!("T-39")),
                        ("reason", json!("wrong branch")),
                    ]),
                )
                .await
                .unwrap(),
        );

        let pushed = mock.pushed_commands();
        assert_eq!(pushed.len(), 1);
        assert_eq!(pushed[0].0, "agent-chat-1");
        assert!(matches!(pushed[0].1, ChatCommand::Abort {}));
        assert!(output.contains("Worktree retained"));

        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-39").unwrap();
        assert_eq!(card.column, "failed");
        assert_eq!(
            card.final_report.as_deref(),
            Some("Cancelled: wrong branch")
        );
        assert_eq!(card.agent_worktree.as_deref(), Some(worktree_str.as_str()));
        assert!(worktree.exists());
        assert!(card
            .status_updates
            .iter()
            .any(|update| update.message == "Cancelled by planner: wrong branch"));
    }

    #[tokio::test]
    async fn cancel_agent_cleanup_removes_worktree_when_requested() {
        let temp = tempfile::tempdir().unwrap();
        let worktree = temp.path().join("agent-worktree-cleanup");
        tokio::fs::create_dir_all(&worktree).await.unwrap();
        tokio::fs::write(worktree.join("file.txt"), "work")
            .await
            .unwrap();
        let gcx = write_task(
            temp.path(),
            test_card(
                "doing",
                Some("agent-chat-1".to_string()),
                Some(worktree.to_string_lossy().to_string()),
            ),
        )
        .await;
        let mock = Arc::new(MockChatFacade::new(Some(SessionState::ExecutingTools)));
        let ccx = planner_ccx(gcx.clone(), mock, "planner").await;

        let output = output_text(
            ToolCancelAgent::new()
                .tool_execute(
                    ccx,
                    &"call".to_string(),
                    &args(&[
                        ("card_id", json!("T-39")),
                        ("reason", json!("superseded")),
                        ("retain_worktree", json!(false)),
                    ]),
                )
                .await
                .unwrap(),
        );

        assert!(output.contains("Worktree cleanup completed"));
        assert!(!worktree.exists());
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-39").unwrap();
        assert_eq!(card.column, "failed");
        assert!(card.agent_worktree.is_none());
    }

    #[tokio::test]
    async fn pause_agent_pushes_pause_message_and_updates_card() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(
            temp.path(),
            test_card("doing", Some("agent-chat-1".to_string()), None),
        )
        .await;
        let mock = Arc::new(MockChatFacade::new(Some(SessionState::ExecutingTools)));
        let ccx = planner_ccx(gcx.clone(), mock.clone(), "planner").await;

        let output = output_text(
            ToolPauseAgent::new()
                .tool_execute(
                    ccx,
                    &"call".to_string(),
                    &args(&[("card_id", json!("T-39")), ("reason", json!("need review"))]),
                )
                .await
                .unwrap(),
        );

        let pushed = mock.pushed_commands();
        assert_eq!(pushed.len(), 1);
        match &pushed[0].1 {
            ChatCommand::UserMessage { content, .. } => {
                assert_eq!(
                    content.as_str(),
                    Some("[Planner PAUSE] need review\n\nWait for resume signal.")
                );
            }
            _ => panic!("expected user message"),
        }
        assert!(output.contains("use resume_agent to continue"));
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-39").unwrap();
        assert!(card
            .status_updates
            .iter()
            .any(|update| update.message == "Paused by planner: need review"));
    }

    #[tokio::test]
    async fn resume_agent_pushes_resume_message_and_updates_card() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(
            temp.path(),
            test_card("doing", Some("agent-chat-1".to_string()), None),
        )
        .await;
        let mock = Arc::new(MockChatFacade::new(Some(SessionState::Idle)));
        let ccx = planner_ccx(gcx.clone(), mock.clone(), "planner").await;

        let output = output_text(
            ToolResumeAgent::new()
                .tool_execute(
                    ccx,
                    &"call".to_string(),
                    &args(&[
                        ("card_id", json!("T-39")),
                        ("note", json!("Continue with tests")),
                    ]),
                )
                .await
                .unwrap(),
        );

        let pushed = mock.pushed_commands();
        assert_eq!(pushed.len(), 1);
        match &pushed[0].1 {
            ChatCommand::UserMessage { content, .. } => {
                assert_eq!(
                    content.as_str(),
                    Some("[Planner RESUME]\n\nContinue with tests")
                );
            }
            _ => panic!("expected user message"),
        }
        assert!(output.contains("Agent state: idle"));
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-39").unwrap();
        assert!(card
            .status_updates
            .iter()
            .any(|update| update.message == "Resumed by planner"));
    }
}
