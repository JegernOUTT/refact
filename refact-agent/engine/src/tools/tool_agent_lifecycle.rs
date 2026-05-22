use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
use crate::tasks::types::{BoardCard, StatusUpdate};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::worktrees::service::WorktreeService;
use refact_chat_api::ChatCommand;
use refact_runtime_api::{ChatSessionFacade, SessionState};

struct PlannerContext {
    task_id: String,
    gcx: Arc<GlobalContext>,
    chat_facade: Arc<dyn ChatSessionFacade>,
}

#[derive(Default, Clone)]
struct LifecycleCardSnapshot {
    title: String,
    agent_chat_id: Option<String>,
    agent_worktree: Option<String>,
    agent_branch: Option<String>,
    agent_worktree_name: Option<String>,
}

impl LifecycleCardSnapshot {
    fn from_card(card: &BoardCard) -> Self {
        Self {
            title: card.title.clone(),
            agent_chat_id: card.agent_chat_id.clone(),
            agent_worktree: card.agent_worktree.clone(),
            agent_branch: card.agent_branch.clone(),
            agent_worktree_name: card.agent_worktree_name.clone(),
        }
    }
}

#[derive(Default)]
struct CleanupResult {
    worktree_removed: bool,
    branch_deleted: bool,
}

enum CleanupTarget {
    Registered {
        service: WorktreeService,
        worktree_name: String,
        delete_branch: bool,
    },
    Filesystem {
        workspace_root: PathBuf,
        worktree: PathBuf,
        branch: Option<String>,
    },
    None,
}

struct PushReport {
    pushed: bool,
    prior_state: Option<SessionState>,
    skipped_reason: Option<String>,
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
    } else if let Some(reason) = report.skipped_reason.as_deref() {
        format!("{} command not queued: {}", command_name, reason)
    } else {
        "No live agent session found; no command was queued.".to_string()
    }
}

fn card_state_error(action: &str, card_id: &str, column: &str) -> String {
    match column {
        "planned" => format!(
            "Cannot {} card {}: it is planned and has no active task agent.",
            action, card_id
        ),
        "done" => format!("Cannot {} card {}: it is already done.", action, card_id),
        "failed" => format!(
            "Cannot {} card {}: it has already failed. Use restart_agent to continue it.",
            action, card_id
        ),
        other => format!(
            "Cannot {} card {}: expected column 'doing', found '{}'.",
            action, card_id, other
        ),
    }
}

fn validate_doing_card(card: &BoardCard, action: &str) -> Result<String, String> {
    if card.column != "doing" {
        return Err(card_state_error(action, &card.id, &card.column));
    }
    card.agent_chat_id
        .as_deref()
        .filter(|chat_id| !chat_id.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            format!(
                "Cannot {} card {}: it has no active agent chat id.",
                action, card.id
            )
        })
}

fn validate_current_doing_card(
    card: &BoardCard,
    action: &str,
    expected_chat_id: &str,
) -> Result<(), String> {
    let current_chat_id = validate_doing_card(card, action)?;
    if current_chat_id != expected_chat_id {
        return Err(format!(
            "Cannot {} card {}: agent_chat_id changed from '{}' to '{}'.",
            action, card.id, expected_chat_id, current_chat_id
        ));
    }
    Ok(())
}

fn validate_current_agent_chat_id(
    card: &BoardCard,
    action: &str,
    expected_chat_id: &str,
) -> Result<(), String> {
    if card.agent_chat_id.as_deref() != Some(expected_chat_id) {
        return Err(format!(
            "Cannot {} card {}: agent_chat_id changed from '{}' to '{}'.",
            action,
            card.id,
            expected_chat_id,
            card.agent_chat_id.as_deref().unwrap_or("none")
        ));
    }
    Ok(())
}

async fn load_doing_card_snapshot(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
    action: &str,
) -> Result<LifecycleCardSnapshot, String> {
    let board = storage::load_board(gcx, task_id).await?;
    let card = board
        .get_card(card_id)
        .ok_or_else(|| format!("Card {} not found", card_id))?;
    validate_doing_card(card, action)?;
    Ok(LifecycleCardSnapshot::from_card(card))
}

async fn push_if_card_current_and_live(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
    expected_chat_id: &str,
    require_doing: bool,
    chat_facade: Arc<dyn ChatSessionFacade>,
    command: ChatCommand,
) -> Result<PushReport, String> {
    let skipped_reason = |card: Option<&BoardCard>| -> Option<String> {
        let Some(card) = card else {
            return Some(format!("card {} no longer exists", card_id));
        };
        if require_doing && card.column != "doing" {
            return Some(format!(
                "card {} is now in column '{}'",
                card_id, card.column
            ));
        }
        if card.agent_chat_id.as_deref() != Some(expected_chat_id) {
            return Some(format!(
                "agent_chat_id changed from '{}' to '{}'",
                expected_chat_id,
                card.agent_chat_id.as_deref().unwrap_or("none")
            ));
        }
        None
    };

    let board = storage::load_board(gcx.clone(), task_id).await?;
    if let Some(reason) = skipped_reason(board.get_card(card_id)) {
        return Ok(PushReport {
            pushed: false,
            prior_state: None,
            skipped_reason: Some(reason),
        });
    }

    let prior_state = chat_facade.session_state(expected_chat_id).await?;
    if prior_state.is_none() {
        return Ok(PushReport {
            pushed: false,
            prior_state,
            skipped_reason: None,
        });
    }

    let board = storage::load_board(gcx, task_id).await?;
    if let Some(reason) = skipped_reason(board.get_card(card_id)) {
        return Ok(PushReport {
            pushed: false,
            prior_state,
            skipped_reason: Some(reason),
        });
    }

    chat_facade.push_command(expected_chat_id, command).await?;
    Ok(PushReport {
        pushed: true,
        prior_state,
        skipped_reason: None,
    })
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

fn expected_branch_prefix(task_id: &str, card_id: &str) -> String {
    format!("refact/task/{}/card/{}/", task_id, card_id)
}

fn branch_is_safe(task_id: &str, card_id: &str, branch: &str) -> bool {
    branch.starts_with(&expected_branch_prefix(task_id, card_id))
}

fn validate_safe_branch(
    task_id: &str,
    card_id: &str,
    branch: Option<&str>,
) -> Result<bool, String> {
    let Some(branch) = branch else {
        return Ok(false);
    };
    if branch_is_safe(task_id, card_id, branch) {
        Ok(true)
    } else {
        Err(format!(
            "Refusing to delete unsafe branch '{}'; expected prefix '{}'.",
            branch,
            expected_branch_prefix(task_id, card_id)
        ))
    }
}

fn canonical_existing_path(path: &Path) -> Result<PathBuf, String> {
    dunce::canonicalize(path)
        .map_err(|e| format!("Failed to canonicalize '{}': {}", path.display(), e))
}

fn normalized_existing_or_lexical_path(path: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(path)
        .map(|path| dunce::simplified(&path).to_path_buf())
        .or_else(|_| {
            path.canonicalize()
                .map(|path| dunce::simplified(&path).to_path_buf())
        })
        .or_else(|_| {
            let path = dunce::simplified(path).to_path_buf();
            if path.is_absolute() {
                Ok(path)
            } else {
                std::env::current_dir().map(|cwd| dunce::simplified(&cwd.join(path)).to_path_buf())
            }
        })
        .map_err(|e| format!("Failed to resolve '{}': {}", path.display(), e))
}

fn validate_fallback_worktree_path(
    gcx: Arc<GlobalContext>,
    workspace_roots: &[PathBuf],
    worktree: &Path,
) -> Result<PathBuf, String> {
    let worktree = canonical_existing_path(worktree)?;
    let cache_dir = canonical_existing_path(&gcx.cache_dir)?;
    let cache_root = gcx.cache_dir.join("worktrees");
    let cache_root = if cache_root.exists() {
        canonical_existing_path(&cache_root)?
    } else {
        std::fs::create_dir_all(&cache_root).map_err(|e| {
            format!(
                "Failed to create worktree cache '{}': {}",
                cache_root.display(),
                e
            )
        })?;
        canonical_existing_path(&cache_root)?
    };
    if worktree == Path::new("/") || worktree == cache_dir || worktree == cache_root {
        return Err(format!(
            "Refusing to delete unsafe worktree path '{}'.",
            worktree.display()
        ));
    }
    for workspace_root in workspace_roots {
        if let Ok(workspace_root) = canonical_existing_path(workspace_root) {
            if worktree == workspace_root {
                return Err(format!(
                    "Refusing to delete workspace root '{}'.",
                    worktree.display()
                ));
            }
        }
    }
    if !worktree.starts_with(&cache_root) {
        return Err(format!(
            "Refusing to delete worktree path '{}' outside Refact worktree cache '{}'.",
            worktree.display(),
            cache_root.display()
        ));
    }
    Ok(worktree)
}

fn paths_match_if_recorded(recorded: Option<&str>, actual: &Path) -> Result<(), String> {
    let Some(recorded) = recorded else {
        return Ok(());
    };
    let recorded = Path::new(recorded);
    if !recorded.exists() {
        return Ok(());
    }
    let recorded = normalized_existing_or_lexical_path(recorded)?;
    let actual = normalized_existing_or_lexical_path(actual)?;
    if recorded != actual {
        return Err(format!(
            "Recorded worktree path '{}' does not match registered path '{}'.",
            recorded.display(),
            actual.display()
        ));
    }
    Ok(())
}

async fn prepare_cleanup_target(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
    snapshot: &LifecycleCardSnapshot,
) -> Result<CleanupTarget, String> {
    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    if let Some(worktree_name) = snapshot.agent_worktree_name.as_deref() {
        for source_root in &project_dirs {
            let Ok(service) = WorktreeService::new(gcx.cache_dir.clone(), source_root.clone())
            else {
                continue;
            };
            let Ok(view) = service.get_worktree(worktree_name).await else {
                continue;
            };
            if view.meta.kind != "task_agent"
                || view.meta.task_id.as_deref() != Some(task_id)
                || view.meta.card_id.as_deref() != Some(card_id)
            {
                return Err(format!(
                    "Registered worktree '{}' does not match task {} card {}.",
                    worktree_name, task_id, card_id
                ));
            }
            paths_match_if_recorded(snapshot.agent_worktree.as_deref(), &view.meta.root)?;
            if let Some(recorded_branch) = snapshot.agent_branch.as_deref() {
                if view.meta.branch.as_deref() != Some(recorded_branch) {
                    return Err(format!(
                        "Recorded branch '{}' does not match registered branch '{}'.",
                        recorded_branch,
                        view.meta.branch.as_deref().unwrap_or("none")
                    ));
                }
            }
            let delete_branch = validate_safe_branch(
                task_id,
                card_id,
                view.meta
                    .branch
                    .as_deref()
                    .or(snapshot.agent_branch.as_deref()),
            )?;
            return Ok(CleanupTarget::Registered {
                service,
                worktree_name: worktree_name.to_string(),
                delete_branch,
            });
        }
    }

    let Some(worktree) = snapshot.agent_worktree.as_deref() else {
        return Ok(CleanupTarget::None);
    };
    let worktree = validate_fallback_worktree_path(gcx, &project_dirs, Path::new(worktree))?;
    validate_safe_branch(task_id, card_id, snapshot.agent_branch.as_deref())?;
    let workspace_root = project_dirs
        .first()
        .cloned()
        .ok_or_else(|| "No workspace folder found for worktree cleanup".to_string())?;
    Ok(CleanupTarget::Filesystem {
        workspace_root,
        worktree,
        branch: snapshot.agent_branch.clone(),
    })
}

async fn cleanup_agent_worktree(target: CleanupTarget) -> CleanupResult {
    let mut result = CleanupResult::default();
    match target {
        CleanupTarget::Registered {
            service,
            worktree_name,
            delete_branch,
        } => {
            if let Ok(deleted) = service.delete_worktree(&worktree_name, delete_branch).await {
                result.worktree_removed = deleted.deleted;
                result.branch_deleted = deleted.branch_deleted;
            }
        }
        CleanupTarget::Filesystem {
            workspace_root,
            worktree,
            branch,
        } => {
            let worktree_arg = worktree.to_string_lossy().to_string();
            let removed = Command::new("git")
                .args(["worktree", "remove", &worktree_arg, "--force"])
                .current_dir(&workspace_root)
                .output()
                .map(|output| output.status.success())
                .unwrap_or(false);
            result.worktree_removed = removed || !worktree.exists();
            if worktree.exists() && std::fs::remove_dir_all(&worktree).is_ok() {
                result.worktree_removed = true;
            }
            if let Some(branch) = branch.as_deref() {
                let deleted = Command::new("git")
                    .args(["branch", "-D", branch])
                    .current_dir(&workspace_root)
                    .output()
                    .map(|output| output.status.success())
                    .unwrap_or(false);
                result.branch_deleted = deleted;
            }
        }
        CleanupTarget::None => {}
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
        let snapshot =
            load_doing_card_snapshot(planner.gcx.clone(), &planner.task_id, &card_id, "cancel")
                .await?;
        let agent_chat_id = snapshot.agent_chat_id.clone().ok_or_else(|| {
            format!(
                "Cannot cancel card {}: it has no active agent chat id.",
                card_id
            )
        })?;
        let cleanup_target = if retain_worktree {
            CleanupTarget::None
        } else {
            prepare_cleanup_target(planner.gcx.clone(), &planner.task_id, &card_id, &snapshot)
                .await?
        };

        let card_id_for_update = card_id.clone();
        let reason_for_update = reason.clone();
        let agent_chat_id_for_update = agent_chat_id.clone();
        storage::update_board_atomic(planner.gcx.clone(), &planner.task_id, move |board| {
            let card = board
                .get_card_mut(&card_id_for_update)
                .ok_or_else(|| format!("Card {} not found", card_id_for_update))?;
            validate_current_doing_card(card, "cancel", &agent_chat_id_for_update)?;
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
            Ok(())
        })
        .await?;

        let push_report = push_if_card_current_and_live(
            planner.gcx.clone(),
            &planner.task_id,
            &card_id,
            &agent_chat_id,
            false,
            planner.chat_facade.clone(),
            ChatCommand::Abort {},
        )
        .await?;

        let cleanup_result = if retain_worktree {
            CleanupResult::default()
        } else {
            cleanup_agent_worktree(cleanup_target).await
        };

        if !retain_worktree && (cleanup_result.worktree_removed || cleanup_result.branch_deleted) {
            let card_id_for_update = card_id.clone();
            let agent_chat_id_for_update = agent_chat_id.clone();
            let cleanup_clears_worktree = cleanup_result.worktree_removed;
            let cleanup_clears_branch = cleanup_result.branch_deleted;
            storage::update_board_atomic(planner.gcx.clone(), &planner.task_id, move |board| {
                let card = board
                    .get_card_mut(&card_id_for_update)
                    .ok_or_else(|| format!("Card {} not found", card_id_for_update))?;
                validate_current_agent_chat_id(card, "cleanup", &agent_chat_id_for_update)?;
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
        }
        storage::update_task_stats(planner.gcx.clone(), &planner.task_id).await?;

        let cleanup_text = if retain_worktree {
            "Worktree retained for restart_agent.".to_string()
        } else if cleanup_result.worktree_removed {
            "Worktree cleanup completed.".to_string()
        } else if snapshot.agent_worktree.is_some() {
            "Worktree cleanup requested but no worktree removal was confirmed.".to_string()
        } else {
            "No worktree was recorded on the card.".to_string()
        };

        let output = format!(
            "✅ Cancelled {} ({})\n\n{}\n{}\nFinal report: Cancelled: {}",
            card_id,
            snapshot.title,
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
        let snapshot =
            load_doing_card_snapshot(planner.gcx.clone(), &planner.task_id, &card_id, "pause")
                .await?;
        let agent_chat_id = snapshot.agent_chat_id.clone().ok_or_else(|| {
            format!(
                "Cannot pause card {}: it has no active agent chat id.",
                card_id
            )
        })?;

        let card_id_for_update = card_id.clone();
        let reason_for_update = reason.clone();
        let agent_chat_id_for_update = agent_chat_id.clone();
        storage::update_board_atomic(planner.gcx.clone(), &planner.task_id, move |board| {
            let card = board
                .get_card_mut(&card_id_for_update)
                .ok_or_else(|| format!("Card {} not found", card_id_for_update))?;
            validate_current_doing_card(card, "pause", &agent_chat_id_for_update)?;
            let now = Utc::now().to_rfc3339();
            card.last_heartbeat_at = Some(now.clone());
            card.status_updates.push(StatusUpdate {
                timestamp: now,
                message: format!("Paused by planner: {}", reason_for_update),
            });
            Ok(())
        })
        .await?;

        let pause_message = format!("[Planner PAUSE] {}\n\nWait for resume signal.", reason);
        let push_report = push_if_card_current_and_live(
            planner.gcx.clone(),
            &planner.task_id,
            &card_id,
            &agent_chat_id,
            true,
            planner.chat_facade.clone(),
            user_message_command(pause_message),
        )
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
        let snapshot =
            load_doing_card_snapshot(planner.gcx.clone(), &planner.task_id, &card_id, "resume")
                .await?;
        let agent_chat_id = snapshot.agent_chat_id.clone().ok_or_else(|| {
            format!(
                "Cannot resume card {}: it has no active agent chat id.",
                card_id
            )
        })?;

        let card_id_for_update = card_id.clone();
        let agent_chat_id_for_update = agent_chat_id.clone();
        storage::update_board_atomic(planner.gcx.clone(), &planner.task_id, move |board| {
            let card = board
                .get_card_mut(&card_id_for_update)
                .ok_or_else(|| format!("Card {} not found", card_id_for_update))?;
            validate_current_doing_card(card, "resume", &agent_chat_id_for_update)?;
            let now = Utc::now().to_rfc3339();
            card.last_heartbeat_at = Some(now.clone());
            card.status_updates.push(StatusUpdate {
                timestamp: now,
                message: "Resumed by planner".to_string(),
            });
            Ok(())
        })
        .await?;

        let resume_message = format!("[Planner RESUME]\n\n{}", note);
        let push_report = push_if_card_current_and_live(
            planner.gcx.clone(),
            &planner.task_id,
            &card_id,
            &agent_chat_id,
            true,
            planner.chat_facade.clone(),
            user_message_command(resume_message),
        )
        .await?;

        let agent_state = state_after(planner.chat_facade.clone(), Some(&agent_chat_id)).await;
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
    use crate::tasks::types::{TaskBoard, TaskMeta, TaskStatus};
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
            verifier_report: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: Some(Utc::now().to_rfc3339()),
            last_heartbeat_at: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: worktree,
            agent_worktree_name: None,
            ab_variants: None,
            target_files: vec![],
            scope_guard_mode: Default::default(),
        }
    }

    fn run_git(cwd: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap_or_else(|e| panic!("failed to run git {:?}: {}", args, e));
        if !output.status.success() {
            panic!(
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    fn init_repo(root: &Path) {
        run_git(root, &["init"]);
        run_git(root, &["checkout", "-b", "main"]);
        run_git(root, &["config", "user.email", "test@example.com"]);
        run_git(root, &["config", "user.name", "Test User"]);
        std::fs::write(root.join("file.txt"), "hello\n").unwrap();
        run_git(root, &["add", "file.txt"]);
        run_git(root, &["commit", "-m", "initial"]);
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
    async fn lifecycle_tools_reject_planned_done_failed_cards() {
        for column in ["planned", "done", "failed"] {
            let temp = tempfile::tempdir().unwrap();
            let gcx = write_task(
                temp.path(),
                test_card(column, Some("agent-chat-1".to_string()), None),
            )
            .await;
            let mock = Arc::new(MockChatFacade::new(Some(SessionState::Idle)));
            let ccx = planner_ccx(gcx, mock.clone(), "planner").await;
            let call_id = "call".to_string();

            let err = ToolCancelAgent::new()
                .tool_execute(
                    ccx.clone(),
                    &call_id,
                    &args(&[("card_id", json!("T-39")), ("reason", json!("stop"))]),
                )
                .await
                .unwrap_err();
            assert!(err.to_ascii_lowercase().contains(column), "{err}");

            let err = ToolPauseAgent::new()
                .tool_execute(
                    ccx.clone(),
                    &call_id,
                    &args(&[("card_id", json!("T-39")), ("reason", json!("wait"))]),
                )
                .await
                .unwrap_err();
            assert!(err.to_ascii_lowercase().contains(column), "{err}");

            let err = ToolResumeAgent::new()
                .tool_execute(ccx, &call_id, &args(&[("card_id", json!("T-39"))]))
                .await
                .unwrap_err();
            assert!(err.to_ascii_lowercase().contains(column), "{err}");
            assert!(mock.pushed_commands().is_empty());
        }
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
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = write_task(
            &source,
            test_card("doing", Some("agent-chat-1".to_string()), None),
        )
        .await;
        let service =
            WorktreeService::new(gcx.cache_dir.clone(), source.canonicalize().unwrap()).unwrap();
        let created = service
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                branch: Some("refact/task/task-1/card/T-39/agent".to_string()),
                kind: Some("task_agent".to_string()),
                task_id: Some("task-1".to_string()),
                card_id: Some("T-39".to_string()),
                agent_id: Some("agent-1".to_string()),
                chat_id: Some("agent-chat-1".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let worktree = created.worktree.meta.root.clone();
        storage::update_board_atomic(gcx.clone(), "task-1", {
            let worktree = worktree.clone();
            let branch = created.worktree.meta.branch.clone();
            let name = created.worktree.meta.id.clone();
            move |board| {
                let card = board.get_card_mut("T-39").unwrap();
                card.agent_worktree = Some(worktree.to_string_lossy().to_string());
                card.agent_branch = branch.clone();
                card.agent_worktree_name = Some(name.clone());
                Ok(())
            }
        })
        .await
        .unwrap();
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
        assert!(service
            .get_worktree(&created.worktree.meta.id)
            .await
            .is_err());
        assert!(run_git(
            &source,
            &["branch", "--list", "refact/task/task-1/card/T-39/agent"]
        )
        .trim()
        .is_empty());
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-39").unwrap();
        assert_eq!(card.column, "failed");
        assert!(card.agent_worktree.is_none());
        assert!(card.agent_worktree_name.is_none());
        assert!(card.agent_branch.is_none());
    }

    #[tokio::test]
    async fn cleanup_refuses_path_outside_worktree_cache() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside-worktree");
        tokio::fs::create_dir_all(&outside).await.unwrap();
        let mut card = test_card(
            "doing",
            Some("agent-chat-1".to_string()),
            Some(outside.to_string_lossy().to_string()),
        );
        card.agent_branch = Some("refact/task/task-1/card/T-39/agent".to_string());
        let gcx = write_task(temp.path(), card).await;
        let mock = Arc::new(MockChatFacade::new(Some(SessionState::ExecutingTools)));
        let ccx = planner_ccx(gcx.clone(), mock.clone(), "planner").await;

        let err = ToolCancelAgent::new()
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[
                    ("card_id", json!("T-39")),
                    ("reason", json!("unsafe")),
                    ("retain_worktree", json!(false)),
                ]),
            )
            .await
            .unwrap_err();

        assert!(err.contains("outside Refact worktree cache"), "{err}");
        assert!(outside.exists());
        assert!(mock.pushed_commands().is_empty());
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        assert_eq!(board.get_card("T-39").unwrap().column, "doing");
    }

    #[tokio::test]
    async fn cleanup_refuses_unsafe_branch_names() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let cache_dir = temp.path().join("cache");
        let config_dir = temp.path().join("config");
        let gcx =
            crate::global_context::tests::make_test_gcx_with_dirs(cache_dir, config_dir).await;
        let service =
            WorktreeService::new(gcx.cache_dir.clone(), source.canonicalize().unwrap()).unwrap();
        let created = service
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                branch: Some("refact/task/task-1/card/T-39/agent".to_string()),
                kind: Some("task_agent".to_string()),
                task_id: Some("task-1".to_string()),
                card_id: Some("T-39".to_string()),
                agent_id: Some("agent-1".to_string()),
                chat_id: Some("agent-chat-1".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let mut card = test_card(
            "doing",
            Some("agent-chat-1".to_string()),
            Some(created.worktree.meta.root.to_string_lossy().to_string()),
        );
        card.agent_branch = Some("main".to_string());
        card.agent_worktree_name = Some(created.worktree.meta.id.clone());
        let task_dir = source.join(".refact").join("tasks").join("task-1");
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
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
        let mock = Arc::new(MockChatFacade::new(Some(SessionState::ExecutingTools)));
        let ccx = planner_ccx(gcx.clone(), mock.clone(), "planner").await;

        let err = ToolCancelAgent::new()
            .tool_execute(
                ccx,
                &"call".to_string(),
                &args(&[
                    ("card_id", json!("T-39")),
                    ("reason", json!("unsafe branch")),
                    ("retain_worktree", json!(false)),
                ]),
            )
            .await
            .unwrap_err();

        assert!(
            err.contains("unsafe branch") || err.contains("Recorded branch"),
            "{err}"
        );
        assert!(created.worktree.meta.root.exists());
        assert!(mock.pushed_commands().is_empty());
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        assert_eq!(board.get_card("T-39").unwrap().column, "doing");
    }

    #[tokio::test]
    async fn push_guard_rejects_stale_agent_chat_id() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = write_task(
            temp.path(),
            test_card("doing", Some("agent-chat-2".to_string()), None),
        )
        .await;
        let mock = Arc::new(MockChatFacade::new(Some(SessionState::ExecutingTools)));

        let report = push_if_card_current_and_live(
            gcx,
            "task-1",
            "T-39",
            "agent-chat-1",
            true,
            mock.clone(),
            user_message_command("hello".to_string()),
        )
        .await
        .unwrap();

        assert!(!report.pushed);
        assert!(report
            .skipped_reason
            .as_deref()
            .unwrap()
            .contains("agent_chat_id changed"));
        assert!(mock.pushed_commands().is_empty());
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
