use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum, ContextFile};
use crate::chat::internal_roles::{event, EventSubkind};
use crate::global_context::GlobalContext;
use crate::tasks::storage;
use crate::tasks::types::{BoardCard, StatusUpdate};
use crate::tools::task_tool_helpers::{wait_for_agent_abort, AGENT_ABORT_TIMEOUT};
use crate::tasks::assignment::{self, Assignment, AssignmentScope};
use crate::tools::tool_task_spawn_agent::{
    build_agent_thread_params, mark_card_agent_started, prepare_agent_worktree,
    resolve_agent_model, restore_original_card_if_current_agent,
};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::WorktreeMeta;
use refact_chat_api::ChatCommand;
use refact_runtime_api::CreateSessionRequest;

fn context_file_message(
    gcx: &Arc<GlobalContext>,
    context_files: Vec<ContextFile>,
) -> Result<ChatMessage, String> {
    let records = crate::privacy::records::declared_file_records(
        gcx,
        context_files
            .iter()
            .map(|file| PathBuf::from(&file.file_name)),
    )?;
    let mut message = ChatMessage {
        role: "context_file".to_string(),
        content: ChatContent::ContextFiles(context_files),
        tool_call_id: "initial_files".to_string(),
        ..Default::default()
    };
    crate::privacy::records::merge_records(&mut message, records);
    Ok(message)
}

async fn cleanup_old_worktree(
    gcx: Arc<GlobalContext>,
    agent_worktree_name: Option<&str>,
    agent_worktree: Option<&str>,
    agent_branch: Option<&str>,
) {
    if let Some(wt_name) = agent_worktree_name {
        let cache_dir = gcx.cache_dir.clone();
        let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
        if let Some(source_root) = project_dirs.first() {
            if let Ok(service) = WorktreeService::new_async(cache_dir, source_root.clone()).await {
                if service.delete_worktree(wt_name, true, true).await.is_ok() {
                    return;
                }
            }
        }
    }

    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    if let Some(workspace_root) = project_dirs.first() {
        if let Some(wt) = agent_worktree {
            let _ = tokio::process::Command::new("git")
                .args(["worktree", "remove", wt, "--force"])
                .current_dir(workspace_root)
                .output()
                .await;
        }
        if let Some(branch) = agent_branch {
            let _ = tokio::process::Command::new("git")
                .args(["branch", "-D", branch])
                .current_dir(workspace_root)
                .output()
                .await;
        }
    }
}

async fn fresh_restart_diff_report(
    card: &BoardCard,
    force: bool,
) -> Result<Option<String>, String> {
    if card.agent_worktree.is_none()
        && card.agent_worktree_name.is_none()
        && card.agent_branch.is_none()
    {
        return Ok(None);
    }
    let capture = match card.agent_worktree.as_deref() {
        Some(worktree) => {
            crate::chat::task_agent_monitor::capture_failed_agent_diff(
                worktree,
                card.base_commit.as_deref(),
                card.base_branch.as_deref(),
            )
            .await
        }
        None => Err("Previous worktree path is missing".to_string()),
    };
    match capture {
        Ok(diff) => Ok(Some(format!("Fresh restart: previous worktree diff captured.{diff}"))),
        Err(error) if force => Ok(Some(format!(
            "Fresh restart forced; no diff captured: {error}"
        ))),
        Err(error) => Err(format!(
            "Fresh restart refused: {error}. Previous worktree retained; use force=true to restart without capturing its diff."
        )),
    }
}

async fn restore_card_after_restart_failure(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    mut original_card: BoardCard,
    guard_chat_id: String,
    status_update: Option<String>,
) {
    let stats_gcx = gcx.clone();
    let _ = storage::update_board_atomic(gcx, task_id, move |board| {
        if board
            .get_card(&original_card.id)
            .is_some_and(|card| card.column == "doing")
        {
            if let Some(message) = status_update {
                original_card.status_updates.push(StatusUpdate {
                    timestamp: Utc::now().to_rfc3339(),
                    message,
                });
            }
            restore_original_card_if_current_agent(board, &original_card, &guard_chat_id);
        }
        Ok(())
    })
    .await;
    let _ = storage::update_task_stats(stats_gcx, task_id).await;
}

fn mark_card_restarted_fresh(
    card: &mut BoardCard,
    new_agent_id: &str,
    new_agent_chat_id: &str,
    worktree_branch: Option<String>,
    worktree_path: Option<String>,
    worktree_name: Option<String>,
) {
    if let Some(prev_report) = card.final_report.take() {
        let preview: String = prev_report.chars().take(300).collect();
        card.status_updates.push(StatusUpdate {
            timestamp: Utc::now().to_rfc3339(),
            message: format!("Previous failure: {}", preview),
        });
    }
    card.final_report_structured = None;
    card.column = "doing".to_string();
    card.completed_at = None;
    mark_card_agent_started(
        card,
        new_agent_id,
        new_agent_chat_id,
        worktree_branch,
        worktree_path,
        worktree_name,
    );
    card.status_updates.last_mut().map(|u| {
        u.message = "Fresh restart: new agent started".to_string();
    });
}

fn mark_card_restarted_resume(card: &mut BoardCard, new_agent_id: &str, new_agent_chat_id: &str) {
    if let Some(prev_report) = card.final_report.take() {
        let preview: String = prev_report.chars().take(300).collect();
        card.status_updates.push(StatusUpdate {
            timestamp: Utc::now().to_rfc3339(),
            message: format!("Previous failure: {}", preview),
        });
    }
    card.final_report_structured = None;
    card.column = "doing".to_string();
    card.completed_at = None;
    card.assignee = Some(new_agent_id.to_string());
    card.agent_chat_id = Some(new_agent_chat_id.to_string());
    card.started_at = Some(Utc::now().to_rfc3339());
    card.status_updates.push(StatusUpdate {
        timestamp: Utc::now().to_rfc3339(),
        message: "Resume restart: new agent attached to existing worktree".to_string(),
    });
}

fn ensure_restart_source_current(card: &BoardCard, original: &BoardCard) -> Result<(), String> {
    if card.column == original.column
        && card.assignee == original.assignee
        && card.agent_chat_id == original.agent_chat_id
        && card.agent_branch == original.agent_branch
        && card.agent_worktree == original.agent_worktree
        && card.agent_worktree_name == original.agent_worktree_name
    {
        return Ok(());
    }
    Err(format!(
        "Card {} changed while restart was preparing; expected column={}, assignee={}, agent_chat_id={}, found column={}, assignee={}, agent_chat_id={}. Run check_agents and retry.",
        original.id,
        original.column,
        original.assignee.as_deref().unwrap_or("none"),
        original.agent_chat_id.as_deref().unwrap_or("none"),
        card.column,
        card.assignee.as_deref().unwrap_or("none"),
        card.agent_chat_id.as_deref().unwrap_or("none")
    ))
}

async fn claim_restart(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    original: &BoardCard,
    agent_id: &str,
    agent_chat_id: &str,
    mode: &str,
) -> Result<BoardCard, String> {
    let original = original.clone();
    let agent_id = agent_id.to_string();
    let agent_chat_id = agent_chat_id.to_string();
    let mode = mode.to_string();
    let (_, queued) = storage::update_board_atomic(gcx, task_id, move |board| {
        board
            .dependency_gate(&original.id)
            .map_err(storage::violation_error)?;
        let card = board
            .get_card_mut(&original.id)
            .ok_or("restart card removed")?;
        ensure_restart_source_current(card, &original)?;
        card.column = "doing".into();
        card.assignee = Some(agent_id);
        card.agent_chat_id = Some(agent_chat_id);
        card.started_at = None;
        // Preserve the old checkout until the admitted replacement succeeds.
        // A queued restart or cancellation must not discard interrupted edits.
        card.status_updates.push(StatusUpdate {
            timestamp: Utc::now().to_rfc3339(),
            message: format!("Agent restart queued (mode={mode}); previous checkout retained"),
        });
        Ok(Some(card.clone()))
    })
    .await?;
    queued.ok_or_else(|| "restart claim returned no card".into())
}

async fn get_worktree_meta_for_resume(
    gcx: Arc<GlobalContext>,
    card: &BoardCard,
) -> Result<WorktreeMeta, String> {
    let wt_name = card.agent_worktree_name.as_deref().ok_or_else(|| {
        format!(
            "Card {} has no retained worktree (agent_worktree_name is missing). Use mode=fresh to create a new worktree.",
            card.id
        )
    })?;
    let wt_path = card.agent_worktree.as_deref().unwrap_or("");
    if !wt_path.is_empty() && !Path::new(wt_path).exists() {
        return Err(format!(
            "Retained worktree path '{}' for card {} no longer exists. Use mode=fresh to create a new worktree.",
            wt_path, card.id
        ));
    }
    let cache_dir = gcx.cache_dir.clone();
    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    let source_root = project_dirs
        .first()
        .cloned()
        .ok_or("No workspace folder found")?;
    let service = WorktreeService::new_async(cache_dir, source_root).await?;
    let view = service.get_worktree(wt_name).await.map_err(|e| {
        format!(
            "Retained worktree '{}' is no longer registered: {}. Use mode=fresh.",
            wt_name, e
        )
    })?;
    if !view.meta.root.exists() {
        return Err(format!(
            "Retained worktree root '{}' no longer exists on disk. Use mode=fresh.",
            view.meta.root.display()
        ));
    }
    Ok(view.meta)
}

pub struct ToolTaskRestartAgent;

impl ToolTaskRestartAgent {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskRestartAgent {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "restart_agent".to_string(),
            display_name: "Task Restart Agent".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Restart a failed task agent. Use mode=fresh (default) to clean up the old worktree and start fresh, or mode=resume to attach a new agent to the retained worktree and continue where the previous agent left off.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "card_id": {
                        "type": "string",
                        "description": "Card ID to restart"
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["fresh", "resume"],
                        "description": "Restart mode: 'fresh' (default) deletes the old worktree and creates a new one; 'resume' reuses the retained worktree from the previous run."
                    },
                    "force": {
                        "type": "boolean",
                        "description": "If true, allow restarting an active agent (aborts it first) and allow a fresh restart when the previous worktree diff cannot be captured, recording the capture failure."
                    },
                    "suggested_steps": {
                        "type": "integer",
                        "description": "Suggested step budget for the agent (default: 30)."
                    },
                    "files_to_open": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of file paths to open immediately when the agent starts."
                    }
                },
                "required": ["card_id"]
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
        let ccx_lock = ccx.lock().await;
        let is_planner = ccx_lock
            .task_meta
            .as_ref()
            .map(|m| m.role == "planner")
            .unwrap_or(false);
        if !is_planner {
            return Err("restart_agent can only be called by the task planner.".to_string());
        }
        let task_id = ccx_lock
            .task_meta
            .as_ref()
            .map(|m| m.task_id.clone())
            .unwrap_or_default();
        let planner_chat_id = ccx_lock
            .task_meta
            .as_ref()
            .and_then(|m| m.planner_chat_id.clone())
            .unwrap_or_else(|| ccx_lock.chat_id.clone());
        let current_chat_id = ccx_lock.chat_id.clone();
        let root_chat_id = ccx_lock.root_chat_id.clone();
        let depth = ccx_lock.subchat_depth;
        let source_root = ccx_lock
            .execution_scope_worktree()
            .map(|w| w.source_workspace_root);
        let app = ccx_lock.app.clone();
        let current_model = ccx_lock.current_model.clone();
        let gcx = ccx_lock.app.gcx.clone();
        drop(ccx_lock);

        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'card_id'")?;
        let mode = args.get("mode").and_then(|v| v.as_str()).unwrap_or("fresh");
        if mode != "fresh" && mode != "resume" {
            return Err(format!(
                "Invalid mode '{}', must be 'fresh' or 'resume'",
                mode
            ));
        }
        let force = args
            .get("force")
            .and_then(refact_tool_api::coerce_bool)
            .unwrap_or(false);
        let suggested_steps: usize = match args
            .get("suggested_steps")
            .or_else(|| args.get("max_steps"))
        {
            Some(Value::String(s)) => s.parse().unwrap_or(30),
            Some(Value::Number(n)) => n.as_u64().unwrap_or(30) as usize,
            _ => 30,
        };
        let suggested_steps = suggested_steps.min(50).max(1);
        let files_to_open: Vec<String> = args
            .get("files_to_open")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let task_meta = storage::load_task_meta(gcx.clone(), &task_id).await?;
        let model = resolve_agent_model(gcx.clone(), &current_model).await?;
        crate::tools::task_tool_helpers::preflight_agent_model(gcx.clone(), &model).await?;

        let board = storage::load_board(gcx.clone(), &task_id).await?;
        let card = board
            .get_card(card_id)
            .ok_or_else(|| format!("Card {} not found", card_id))?;
        if card
            .ab_variants
            .as_ref()
            .is_some_and(|variants| variants.winner.is_none())
        {
            return Err("Pick or cancel the A/B variants before restarting this card".into());
        }
        match card.column.as_str() {
            "done" => {
                return Err(format!(
                    "Card {} is already done and cannot be restarted.",
                    card_id
                ))
            }
            "planned" => {
                return Err(format!(
                "Card {} is in 'planned' column. Use spawn_agent to start it for the first time.",
                card_id
            ))
            }
            "failed" => {}
            "doing" => {
                if card.agent_chat_id.is_some() && !force {
                    return Err(format!(
                        "Card {} has an active agent ({}). Use force=true to abort it and restart, or wait for it to finish.",
                        card_id,
                        card.agent_chat_id.as_ref().unwrap()
                    ));
                }
            }
            other => {
                return Err(format!(
                    "Card {} is in unexpected column '{}'",
                    card_id, other
                ))
            }
        }
        board
            .dependency_gate(card_id)
            .map_err(storage::violation_error)?;
        if mode == "fresh" {
            fresh_restart_diff_report(card, force).await?;
        }
        let card_title = card.title.clone();
        let card_instructions = card.instructions.clone();
        let dependency_context = board
            .get_dependency_reports(card_id)
            .into_iter()
            .map(|(title, report)| format!("### {}\n{}", title, report))
            .collect::<Vec<_>>()
            .join("\n\n");

        let source_root = source_root
            .or(crate::files_correction::get_active_project_path(gcx.clone()).await)
            .ok_or("No active project for restarted agent")?;
        let project = crate::coordination::roots::project_identity(&source_root);
        let root = if root_chat_id.is_empty() {
            &planner_chat_id
        } else {
            &root_chat_id
        };
        let original = card.clone();
        let agent_id = Uuid::new_v4().to_string();
        let agent_chat_id = format!("agent-{card_id}-{}", &agent_id[..8]);
        crate::tasks::admission::reserve(&app, &agent_chat_id, &project, root, depth)
            .await
            .map_err(|e| e.to_string())?;
        let result_message = json!({
            "summary": format!("Agent restart queued: {card_title}"),
            "card_id": card_id, "agent_id": agent_id, "agent_chat_id": agent_chat_id,
            "mode": mode, "execution_state": "queued", "model": model,
            "worktree": null,
        })
        .to_string();
        let card_id = card_id.to_string();
        let mode = mode.to_string();
        let tracked_chat_id = agent_chat_id.clone();
        let (accepted, acceptance) = tokio::sync::oneshot::channel();
        let registered = crate::tasks::admission::spawn_tracked(&tracked_chat_id, async move {
            use futures::FutureExt;
            let mut accepted = Some(accepted);
            let setup = async {
                if accepted.as_ref().is_none_or(|tx| tx.is_closed()) {
                    return Err("restart caller cancelled before claim".to_string());
                }
                let queued = claim_restart(
                    gcx.clone(),
                    &task_id,
                    &original,
                    &agent_id,
                    &agent_chat_id,
                    &mode,
                )
                .await?;
                if accepted
                    .take()
                    .ok_or("restart acceptance lost")?
                    .send(Ok(()))
                    .is_err()
                {
                    restore_card_after_restart_failure(
                        gcx.clone(),
                        &task_id,
                        original.clone(),
                        agent_chat_id.clone(),
                        None,
                    )
                    .await;
                    return Err("restart caller cancelled during claim".into());
                }
                // Stop the old attempt before waiting for capacity: it may be
                // the holder of the only permit. No worktree changes occur yet.
                if let Some(old_chat) = original.agent_chat_id.as_deref() {
                    wait_for_agent_abort(gcx.clone(), old_chat, AGENT_ABORT_TIMEOUT).await?;
                }
                crate::tasks::admission::wait_for_permit(&agent_chat_id)
                    .await
                    .map_err(|e| e.to_string())?;
                let execution = app
                    .agent_executions
                    .get(&agent_chat_id)
                    .ok_or("restart admission lost")?;
                let _operation = execution
                    .begin_operation()
                    .await
                    .map_err(|e| e.to_string())?;
                let board = storage::load_board(gcx.clone(), &task_id).await?;
                ensure_restart_source_current(
                    board.get_card(&card_id).ok_or("restart card removed")?,
                    &queued,
                )?;
                let tool = ToolTaskRestartAgent::new();
                if mode == "resume" {
                    tool.execute_resume(
                        app.clone(),
                        &task_id,
                        &planner_chat_id,
                        &current_chat_id,
                        &card_id,
                        &card_title,
                        &card_instructions,
                        &dependency_context,
                        &model,
                        suggested_steps,
                        files_to_open,
                        &original,
                        &queued,
                        &agent_id,
                        &agent_chat_id,
                    )
                    .await
                } else {
                    tool.execute_fresh(
                        app.clone(),
                        &task_id,
                        &planner_chat_id,
                        &task_meta,
                        &card_id,
                        &card_title,
                        &card_instructions,
                        &dependency_context,
                        &model,
                        suggested_steps,
                        files_to_open,
                        &original,
                        &queued,
                        &agent_id,
                        &agent_chat_id,
                        force,
                    )
                    .await
                }
            };
            let result = std::panic::AssertUnwindSafe(setup)
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err("agent restart setup panicked".into()));
            if let Err(error) = result {
                if let Some(accepted) = accepted.take() {
                    let _ = accepted.send(Err(error.clone()));
                }
                let id = card_id.clone();
                let chat = agent_chat_id.clone();
                let _ = storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
                    if let Some(card) = board.get_card_mut(&id) {
                        if card.column == "doing" && card.agent_chat_id.as_deref() == Some(&chat) {
                            card.column = "failed".into();
                            card.final_report = Some(format!("Restart setup failed: {error}"));
                            card.completed_at = Some(Utc::now().to_rfc3339());
                        }
                    }
                    Ok(())
                })
                .await;
                crate::tasks::admission::end(&agent_chat_id).await;
            }
            let _ = storage::update_task_stats(gcx, &task_id).await;
        })
        .await;
        if let Err(error) = registered {
            crate::tasks::admission::end(&tracked_chat_id).await;
            return Err(error.to_string());
        }
        acceptance
            .await
            .map_err(|_| "restart setup ended before acceptance".to_string())??;

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result_message),
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

impl ToolTaskRestartAgent {
    async fn execute_fresh(
        &self,
        app: crate::app_state::AppState,
        task_id: &str,
        planner_chat_id: &str,
        task_meta: &crate::tasks::types::TaskMeta,
        card_id: &str,
        card_title: &str,
        card_instructions: &str,
        dependency_context: &str,
        model: &str,
        suggested_steps: usize,
        files_to_open: Vec<String>,
        card: &BoardCard,
        queued_card: &BoardCard,
        agent_id: &str,
        agent_chat_id: &str,
        force: bool,
    ) -> Result<String, String> {
        let gcx = app.gcx.clone();
        let original_card = card.clone();
        let diff_report = match fresh_restart_diff_report(&original_card, force).await {
            Ok(report) => report,
            Err(error) => {
                tracing::warn!(card_id, reason = %error, "Fresh restart refused after it was queued");
                restore_card_after_restart_failure(
                    gcx.clone(),
                    task_id,
                    original_card,
                    agent_chat_id.to_string(),
                    Some(format!(
                        "Fresh restart refused after it was queued: {error}; retry with force=true to restart without the capture"
                    )),
                )
                .await;
                return Err(error);
            }
        };

        let mut restart_task_meta = task_meta.clone();
        if original_card.base_branch.is_some() || original_card.base_commit.is_some() {
            restart_task_meta.base_branch = original_card.base_branch.clone();
            restart_task_meta.base_commit = original_card.base_commit.clone();
        }
        let prepared_worktree = prepare_agent_worktree(
            gcx.clone(),
            &restart_task_meta,
            task_id,
            &agent_id,
            card_id,
            &agent_chat_id,
        )
        .await?;

        let worktree_branch = prepared_worktree.branch_name();
        let worktree_path_str = Some(
            prepared_worktree
                .worktree_path()
                .to_string_lossy()
                .to_string(),
        );
        let worktree_name = Some(prepared_worktree.worktree_name());
        let worktree_meta = prepared_worktree.worktree_meta();
        let prepared_base_branch = worktree_meta.base_branch.clone();
        let prepared_base_commit = worktree_meta.base_commit.clone();
        let card_id_owned = card_id.to_string();
        let agent_id_clone = agent_id.to_string();
        let agent_chat_id_clone = agent_chat_id.to_string();
        let source_card = queued_card.clone();

        let link_execution = app
            .agent_executions
            .get(agent_chat_id)
            .ok_or("restart execution lost")?;
        let board_update_result =
            storage::update_board_atomic(gcx.clone(), task_id, move |board| {
                if link_execution.is_cancelled() {
                    return Err("restart setup cancelled".into());
                }
                let card = board
                    .get_card_mut(&card_id_owned)
                    .ok_or(format!("Card {} not found", card_id_owned))?;
                ensure_restart_source_current(card, &source_card)?;
                if let Some(message) = diff_report {
                    card.status_updates.push(StatusUpdate {
                        timestamp: Utc::now().to_rfc3339(),
                        message,
                    });
                }
                mark_card_restarted_fresh(
                    card,
                    &agent_id_clone,
                    &agent_chat_id_clone,
                    worktree_branch.clone(),
                    worktree_path_str.clone(),
                    worktree_name.clone(),
                );
                card.base_branch = prepared_base_branch.clone();
                card.base_commit = prepared_base_commit.clone();
                Ok(())
            })
            .await;

        if let Err(e) = board_update_result {
            prepared_worktree.cleanup_unlinked(gcx.clone()).await;
            return Err(e);
        }

        let _ = storage::update_task_stats(gcx.clone(), task_id).await;

        if let Err(e) = self
            .create_and_dispatch_session(
                app,
                task_id,
                planner_chat_id,
                card_id,
                card_title,
                card_instructions,
                dependency_context,
                model,
                &agent_id,
                &agent_chat_id,
                worktree_meta.clone(),
                suggested_steps,
                files_to_open,
            )
            .await
        {
            prepared_worktree.cleanup_unlinked(gcx.clone()).await;
            restore_card_after_restart_failure(
                gcx.clone(),
                task_id,
                original_card,
                agent_chat_id.to_string(),
                None,
            )
            .await;
            return Err(e);
        }

        cleanup_old_worktree(
            gcx.clone(),
            original_card.agent_worktree_name.as_deref(),
            original_card.agent_worktree.as_deref(),
            original_card.agent_branch.as_deref(),
        )
        .await;

        Ok(format!(
            "# Agent Restarted (Fresh): {}\n\n**Card:** {}\n**Agent ID:** {}\n**Model:** {}\n**Mode:** fresh\n**Status:** Running in background\n\nPrevious worktree cleaned up. New agent started from scratch.",
            card_title, card_id, agent_id, model
        ))
    }

    async fn execute_resume(
        &self,
        app: crate::app_state::AppState,
        task_id: &str,
        planner_chat_id: &str,
        resume_chat_id: &str,
        card_id: &str,
        card_title: &str,
        card_instructions: &str,
        dependency_context: &str,
        model: &str,
        suggested_steps: usize,
        files_to_open: Vec<String>,
        card: &BoardCard,
        queued_card: &BoardCard,
        agent_id: &str,
        agent_chat_id: &str,
    ) -> Result<String, String> {
        let gcx = app.gcx.clone();
        let original_card = card.clone();

        let worktree_meta = get_worktree_meta_for_resume(gcx.clone(), card).await?;
        let card_id_owned = card_id.to_string();
        let agent_id_clone = agent_id.to_string();
        let agent_chat_id_clone = agent_chat_id.to_string();
        let _ = resume_chat_id;
        let source_card = queued_card.clone();

        let link_execution = app
            .agent_executions
            .get(agent_chat_id)
            .ok_or("restart execution lost")?;
        storage::update_board_atomic(gcx.clone(), task_id, move |board| {
            if link_execution.is_cancelled() {
                return Err("restart setup cancelled".into());
            }
            let card = board
                .get_card_mut(&card_id_owned)
                .ok_or(format!("Card {} not found", card_id_owned))?;
            ensure_restart_source_current(card, &source_card)?;
            mark_card_restarted_resume(card, &agent_id_clone, &agent_chat_id_clone);
            Ok(())
        })
        .await?;

        let _ = storage::update_task_stats(gcx.clone(), task_id).await;

        if let Err(e) = self
            .create_and_dispatch_session(
                app,
                task_id,
                planner_chat_id,
                card_id,
                card_title,
                card_instructions,
                dependency_context,
                model,
                &agent_id,
                &agent_chat_id,
                worktree_meta,
                suggested_steps,
                files_to_open,
            )
            .await
        {
            restore_card_after_restart_failure(
                gcx.clone(),
                task_id,
                original_card,
                agent_chat_id.to_string(),
                None,
            )
            .await;
            return Err(e);
        }

        Ok(format!(
            "# Agent Restarted (Resume): {}\n\n**Card:** {}\n**Agent ID:** {}\n**Model:** {}\n**Mode:** resume\n**Status:** Running in background\n\nAgent attached to retained worktree and continuing.",
            card_title, card_id, agent_id, model
        ))
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_and_dispatch_session(
        &self,
        app: crate::app_state::AppState,
        task_id: &str,
        planner_chat_id: &str,
        card_id: &str,
        card_title: &str,
        card_instructions: &str,
        dependency_context: &str,
        model: &str,
        agent_id: &str,
        agent_chat_id: &str,
        worktree_meta: WorktreeMeta,
        suggested_steps: usize,
        files_to_open: Vec<String>,
    ) -> Result<(), String> {
        let gcx = app.gcx.clone();
        let execution = app
            .agent_executions
            .get(agent_chat_id)
            .ok_or("restart execution lost")?;
        if execution.is_cancelled() {
            return Err("restart setup cancelled".into());
        }

        let thread = build_agent_thread_params(
            agent_chat_id,
            card_title,
            model,
            task_id,
            agent_id,
            card_id,
            planner_chat_id,
            worktree_meta.clone(),
        );

        let scope = storage::load_board(gcx.clone(), task_id)
            .await
            .ok()
            .and_then(|board| {
                board.get_card(card_id).map(|card| AssignmentScope {
                    target_files: card.target_files.clone(),
                    guard: card.scope_guard_mode,
                })
            })
            .unwrap_or_default();
        let assignment = Assignment::new(card_id, card_title, card_instructions)
            .with_dependency_context(dependency_context)
            .with_suggested_steps(suggested_steps)
            .with_scope(scope);
        assignment::record_assignment(gcx.clone(), task_id, &assignment).await;
        let user_prompt = assignment.render_prompt();

        let user_msg = event(
            EventSubkind::SystemNotice,
            "tool.task_restart_agent",
            json!({
                "task_id": task_id,
                "planner_chat_id": planner_chat_id,
                "card_id": card_id,
                "card_title": card_title,
                "agent_id": agent_id,
                "agent_chat_id": agent_chat_id,
                "model": model,
                "suggested_steps": suggested_steps,
                "files_to_open": &files_to_open,
                "worktree": worktree_meta.root.to_string_lossy().to_string(),
                "worktree_name": &worktree_meta.id,
                "branch": worktree_meta.branch.as_deref(),
            }),
            user_prompt,
        );
        let mut messages = vec![user_msg];

        if !files_to_open.is_empty() {
            let mut context_files: Vec<ContextFile> = Vec::new();
            let wt_root = &worktree_meta.root;
            let worktree_canonical =
                dunce::canonicalize(wt_root).unwrap_or_else(|_| wt_root.clone());
            for path_str in &files_to_open {
                let orig = std::path::Path::new(path_str);
                let source_root = &worktree_meta.source_workspace_root;
                let resolved = match orig.strip_prefix(source_root) {
                    Ok(rel) => wt_root.join(rel),
                    Err(_) => wt_root.join(path_str.trim_start_matches('/')),
                };
                let canonical_resolved = match dunce::canonicalize(&resolved) {
                    Ok(p) => p,
                    Err(_) => {
                        tracing::warn!(
                            "restart_agent: files_to_open '{}' does not exist, skipping",
                            path_str
                        );
                        continue;
                    }
                };
                if !canonical_resolved.starts_with(&worktree_canonical) {
                    tracing::warn!(
                        "restart_agent: files_to_open '{}' escapes worktree, rejecting",
                        path_str
                    );
                    continue;
                }
                if crate::files_in_workspace::check_file_privacy_for_model_context(
                    gcx.clone(),
                    &canonical_resolved,
                )
                .await
                .is_err()
                {
                    tracing::warn!(
                        "restart_agent: files_to_open '{}' blocked by privacy settings, skipping",
                        path_str
                    );
                    continue;
                }
                match tokio::fs::read_to_string(&canonical_resolved).await {
                    Ok(content) => {
                        let line_count = content.lines().count().max(1);
                        context_files.push(ContextFile {
                            file_name: canonical_resolved.to_string_lossy().to_string(),
                            file_content: content,
                            line1: 1,
                            line2: line_count,
                            ..Default::default()
                        });
                    }
                    Err(e) => {
                        tracing::warn!(
                            "restart_agent: could not read file {:?}: {}",
                            canonical_resolved,
                            e
                        );
                    }
                }
            }
            if !context_files.is_empty() {
                messages.push(context_file_message(&gcx, context_files)?);
            }
        }

        app.chat
            .facade
            .create_session(CreateSessionRequest {
                chat_id: agent_chat_id.to_string(),
                thread,
                messages,
            })
            .await?;
        app.chat.facade.maybe_save_session(agent_chat_id).await?;
        if execution.is_cancelled() {
            return Err("restart setup cancelled".into());
        }
        app.chat
            .facade
            .push_priority_command(agent_chat_id, ChatCommand::Regenerate {})
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::types::{BoardCard, TaskBoard};

    #[tokio::test]
    async fn restart_tool_queues_at_capacity_then_admits_without_losing_retained_edits() {
        use crate::tools::tool_spawn_ab::tests::{setup_repo_task, RecordingFacade, await_started};
        use crate::app_state::AppState;
        let (gcx, _temp, source) = setup_repo_task().await;
        let mut caps = crate::caps::CodeAssistantCaps::default();
        caps.chat_models.insert(
            "test/model".into(),
            Arc::new(crate::caps::ChatModelRecord::default()),
        );
        gcx.caps_state.write().await.caps = Some(Arc::new(caps));
        gcx.caps_state.write().await.last_attempted_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let meta = storage::load_task_meta(gcx.clone(), "task-1")
            .await
            .unwrap();
        let old =
            prepare_agent_worktree(gcx.clone(), &meta, "task-1", "old-agent", "T-1", "old-chat")
                .await
                .unwrap();
        let old_root = old.worktree_path();
        std::fs::write(old_root.join("retained.txt"), "preserve me").unwrap();
        let mut failed = failed_card(
            "T-1",
            Some(old_root.to_string_lossy().into_owned()),
            old.branch_name(),
        );
        failed.agent_worktree_name = Some(old.worktree_name());
        failed.agent_chat_id = Some("old-chat".into());
        storage::update_board_atomic(gcx.clone(), "task-1", move |board| {
            *board.get_card_mut("T-1").unwrap() = failed;
            Ok(())
        })
        .await
        .unwrap();
        let mut app = AppState::from_gcx(gcx.clone()).await;
        app.work_scheduler = Arc::new(refact_work_core::AdmissionScheduler::new(
            refact_work_core::CoordinationSettings {
                max_active_engine: 1,
                max_active_project: 1,
                max_active_root: 1,
                ..Default::default()
            },
        ));
        let facade = Arc::new(RecordingFacade::default());
        app.chat.facade = facade.clone();
        let project = crate::coordination::roots::project_identity(&source);
        let holder = app
            .work_scheduler
            .reserve(refact_work_core::AdmissionRequest::new(
                refact_work_core::WorkKey::background_agent("holder").run(1),
                crate::tasks::admission::board_scope(&project, "planner"),
            ))
            .unwrap()
            .acquire()
            .await
            .unwrap();
        let ccx = Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app.clone(),
                4096,
                20,
                false,
                vec![],
                "planner".into(),
                Some("planner".into()),
                "test/model".into(),
                Some(refact_chat_api::TaskMeta {
                    task_id: "task-1".into(),
                    role: "planner".into(),
                    planner_chat_id: Some("planner".into()),
                    agent_id: None,
                    card_id: None,
                }),
                None,
            )
            .await,
        ));
        let (_, result) = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            ToolTaskRestartAgent::new().tool_execute(
                ccx,
                &"restart-call".into(),
                &HashMap::from([
                    ("card_id".into(), json!("T-1")),
                    ("mode".into(), json!("resume")),
                ]),
            ),
        )
        .await
        .expect("restart returns without waiting for capacity")
        .unwrap();
        let ContextEnum::ChatMessage(message) = &result[0] else {
            panic!("tool result");
        };
        let response: Value = serde_json::from_str(&message.content.content_text_only()).unwrap();
        assert_eq!(response["execution_state"], "queued");
        assert!(response["worktree"].is_null());
        assert!(facade.created.lock().unwrap().is_empty());
        assert_eq!(
            std::fs::read_to_string(old_root.join("retained.txt")).unwrap(),
            "preserve me"
        );
        let queued = storage::load_board(gcx.clone(), "task-1").await.unwrap();
        assert!(queued.get_card("T-1").unwrap().started_at.is_none());
        holder.release();
        await_started(&facade, 1).await;
        let chat = response["agent_chat_id"].as_str().unwrap();
        assert_eq!(
            facade.started.lock().unwrap().as_slice(),
            &[chat.to_string()]
        );
        assert_eq!(app.work_scheduler.snapshot().active, 1);
        assert_eq!(
            std::fs::read_to_string(old_root.join("retained.txt")).unwrap(),
            "preserve me"
        );
        crate::tasks::admission::cancel_setup_and_wait(chat, std::time::Duration::from_secs(5))
            .await
            .unwrap();
        crate::tasks::admission::end(chat).await;
        old.cleanup_unlinked(gcx).await;
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
    }

    #[tokio::test]
    async fn dropped_restart_caller_during_claim_keeps_previous_worktree_and_binding() {
        use std::future::Future;
        use crate::tools::tool_spawn_ab::tests::setup_repo_task;
        let (gcx, _temp, source) = setup_repo_task().await;
        let original = failed_card("T-1", Some("/retained".into()), None);
        storage::update_board_atomic(gcx.clone(), "task-1", {
            let original = original.clone();
            move |board| {
                *board.get_card_mut("T-1").unwrap() = original;
                Ok(())
            }
        })
        .await
        .unwrap();
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let id = format!("restart-test-{}", Uuid::new_v4());
        crate::tasks::admission::reserve(&app, &id, "proj", "root", 0)
            .await
            .unwrap();
        let lock = crate::tasks::board_lock::acquire_board(&source.join(".refact/tasks/task-1"))
            .await
            .unwrap();
        let (accepted, acceptance) = tokio::sync::oneshot::channel::<Result<(), String>>();
        let (entered, entry) = tokio::sync::oneshot::channel();
        let task_id = id.clone();
        let task_gcx = gcx.clone();
        let old = original.clone();
        crate::tasks::admission::spawn_tracked(&id, async move {
            let mut claim = Box::pin(claim_restart(
                task_gcx.clone(),
                "task-1",
                &old,
                "new-agent",
                &task_id,
                "resume",
            ));
            std::future::poll_fn(|cx| {
                assert!(claim.as_mut().poll(cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            entered.send(()).unwrap();
            match claim.await {
                Ok(_) => {
                    if accepted.send(Ok(())).is_err() {
                        restore_card_after_restart_failure(
                            task_gcx,
                            "task-1",
                            old,
                            task_id.clone(),
                            None,
                        )
                        .await;
                    }
                }
                Err(error) => {
                    let _ = accepted.send(Err(error));
                }
            }
            crate::tasks::admission::end(&task_id).await;
        })
        .await
        .unwrap();
        entry.await.unwrap();
        drop(acceptance);
        drop(lock);
        crate::tasks::admission::shutdown(&app).await;
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert_eq!(card.agent_chat_id, original.agent_chat_id);
        assert_eq!(card.agent_worktree, original.agent_worktree);
        assert_eq!(card.column, "failed");
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
    }

    #[tokio::test]
    async fn restart_claim_is_conditional_and_preserves_old_checkout_while_queued() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![temp.path().canonicalize().unwrap()];
        let task_dir = temp.path().join(".refact/tasks/restart-claim");
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        let mut original = failed_card(
            "T-1",
            Some("/retained/worktree".into()),
            Some("retained-branch".into()),
        );
        original.agent_worktree_name = Some("retained-id".into());
        let board = TaskBoard {
            cards: vec![original.clone()],
            ..Default::default()
        };
        storage::save_board(gcx.clone(), "restart-claim", &board)
            .await
            .unwrap();
        let queued = claim_restart(
            gcx.clone(),
            "restart-claim",
            &original,
            "new-agent",
            "new-chat",
            "fresh",
        )
        .await
        .unwrap();
        assert_eq!(queued.column, "doing");
        assert_eq!(queued.agent_chat_id.as_deref(), Some("new-chat"));
        assert!(queued.started_at.is_none());
        assert_eq!(queued.agent_worktree, original.agent_worktree);
        assert_eq!(queued.final_report, original.final_report);
        let error = claim_restart(
            gcx.clone(),
            "restart-claim",
            &original,
            "duplicate",
            "duplicate-chat",
            "fresh",
        )
        .await
        .unwrap_err();
        assert!(error.contains("changed while restart"));
        let current = storage::load_board(gcx, "restart-claim").await.unwrap();
        assert_eq!(
            current.get_card("T-1").unwrap().agent_chat_id,
            queued.agent_chat_id
        );
    }

    #[tokio::test]
    async fn restart_agent_context_file_message_has_declared_record() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("safe.txt");
        std::fs::write(&path, "safe\n").unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let message = context_file_message(
            &gcx,
            vec![ContextFile {
                file_name: path.to_string_lossy().into_owned(),
                file_content: "safe\n".to_string(),
                line1: 1,
                line2: 1,
                ..Default::default()
            }],
        )
        .unwrap();
        let privacy: refact_privacy::PrivacyRecord =
            serde_json::from_value(message.extra["privacy"].clone()).unwrap();

        assert_eq!(privacy.files.len(), 1);
        assert!(privacy.files[0].path.ends_with("safe.txt"));
        assert_eq!(
            privacy.files[0].attribution,
            refact_privacy::Attribution::Declared
        );
    }

    fn failed_card(id: &str, worktree: Option<String>, branch: Option<String>) -> BoardCard {
        BoardCard {
            id: id.to_string(),
            title: format!("Card {}", id),
            column: "failed".to_string(),
            priority: "P1".to_string(),
            depends_on: vec![],
            instructions: "Fix the bug".to_string(),
            assignee: Some("old-agent".to_string()),
            agent_chat_id: Some(format!("agent-{}-old", id)),
            status_updates: vec![StatusUpdate {
                timestamp: "2024-01-01T10:00:00Z".to_string(),
                message: "Agent started".to_string(),
            }],
            comments: vec![],
            final_report: Some("FAILED: network error after 3 retries".to_string()),
            final_report_structured: None,
            verifier_report: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: Some(Utc::now().to_rfc3339()),
            last_heartbeat_at: None,
            paused_at: None,
            paused_by: None,
            completed_at: Some(Utc::now().to_rfc3339()),
            agent_branch: branch,
            agent_worktree: worktree,
            agent_worktree_name: None,
            base_branch: None,
            base_commit: None,
            ab_variants: None,
            team_members: vec![],
            target_files: vec![],
            scope_guard_mode: Default::default(),
            attention: None,
            deliverable: None,
            verification: Vec::new(),
        }
    }

    #[test]
    fn restart_agent_fresh_clears_failure_and_sets_doing() {
        let mut card = failed_card("T-1", None, None);
        let new_id = "new-agent-1";
        let new_chat = "agent-T-1-new";

        mark_card_restarted_fresh(
            &mut card,
            new_id,
            new_chat,
            Some("refact/task/task-1/card/T-1/new".to_string()),
            Some("/tmp/wt".to_string()),
            Some("wt-new".to_string()),
        );

        assert_eq!(card.column, "doing");
        assert_eq!(card.assignee.as_deref(), Some(new_id));
        assert_eq!(card.agent_chat_id.as_deref(), Some(new_chat));
        assert!(card.completed_at.is_none());
        assert!(card.final_report.is_none());
        assert!(card
            .status_updates
            .iter()
            .any(|u| u.message.contains("Previous failure")));
        assert_eq!(card.agent_worktree.as_deref(), Some("/tmp/wt"));
        assert_eq!(card.agent_worktree_name.as_deref(), Some("wt-new"));
    }

    #[test]
    fn restart_agent_resume_keeps_worktree_fields() {
        let mut card = failed_card(
            "T-2",
            Some("/tmp/retained-wt".to_string()),
            Some("refact/task/task-1/card/T-2/old".to_string()),
        );
        let old_worktree = card.agent_worktree.clone();
        let old_branch = card.agent_branch.clone();
        let new_id = "resume-agent-1";
        let new_chat = "agent-T-2-resume";

        mark_card_restarted_resume(&mut card, new_id, new_chat);

        assert_eq!(card.column, "doing");
        assert_eq!(card.assignee.as_deref(), Some(new_id));
        assert_eq!(card.agent_chat_id.as_deref(), Some(new_chat));
        assert!(card.completed_at.is_none());
        assert!(card.final_report.is_none());
        assert!(card
            .status_updates
            .iter()
            .any(|u| u.message.contains("Previous failure")));
        assert!(card
            .status_updates
            .iter()
            .any(|u| u.message.contains("Resume restart")));
        assert_eq!(card.agent_worktree, old_worktree, "worktree path preserved");
        assert_eq!(card.agent_branch, old_branch, "branch preserved");
    }

    #[test]
    fn restart_agent_refuses_active_doing_card_without_force() {
        let mut board = TaskBoard::default();
        let doing_card = BoardCard {
            id: "T-3".to_string(),
            title: "Card T-3".to_string(),
            column: "doing".to_string(),
            priority: "P1".to_string(),
            depends_on: vec![],
            instructions: "do stuff".to_string(),
            assignee: Some("active-agent".to_string()),
            agent_chat_id: Some("agent-T-3-active".to_string()),
            status_updates: vec![],
            comments: vec![],
            final_report: None,
            final_report_structured: None,
            verifier_report: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: Some(Utc::now().to_rfc3339()),
            last_heartbeat_at: None,
            paused_at: None,
            paused_by: None,
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
            attention: None,
            deliverable: None,
            verification: Vec::new(),
        };
        board.cards.push(doing_card);

        let card = board.get_card("T-3").unwrap();
        let should_refuse = card.column == "doing" && card.agent_chat_id.is_some();
        assert!(
            should_refuse,
            "active doing card should trigger refusal without force"
        );
    }

    #[test]
    fn restart_agent_allows_failed_card_without_force() {
        let card = failed_card("T-4", Some("/tmp/wt".to_string()), None);
        let should_refuse = card.column == "doing" && card.agent_chat_id.is_some();
        assert!(!should_refuse, "failed card should not require force");
        assert_eq!(card.column, "failed");
    }

    #[test]
    fn restart_agent_fresh_preserves_failure_in_status_history() {
        let mut card = failed_card("T-5", None, None);
        let original_status_count = card.status_updates.len();

        mark_card_restarted_fresh(&mut card, "new-agent", "agent-T-5-new", None, None, None);

        assert!(
            card.status_updates.len() > original_status_count,
            "status updates should grow"
        );
        assert!(
            card.status_updates
                .iter()
                .any(|u| u.message.starts_with("Previous failure:")),
            "old failure should appear in status history"
        );
        assert!(
            card.final_report.is_none(),
            "final_report should be cleared after restart"
        );
    }

    #[test]
    fn restart_agent_resume_rejects_card_without_worktree_name() {
        let card = failed_card("T-6", None, None);

        let wt_name = card.agent_worktree_name.as_deref();
        assert!(wt_name.is_none());
    }

    #[test]
    fn restart_agent_fresh_rollback_resets_card_to_failed() {
        let original = failed_card(
            "T-7",
            Some("/tmp/old-wt".to_string()),
            Some("old-branch".to_string()),
        );
        let mut board = TaskBoard::default();
        let mut restarted = original.clone();
        mark_card_restarted_fresh(
            &mut restarted,
            "new-agent",
            "agent-T-7-new",
            Some("new-branch".to_string()),
            Some("/tmp/new-wt".to_string()),
            Some("new-wt".to_string()),
        );
        board.cards.push(restarted);

        assert!(restore_original_card_if_current_agent(
            &mut board,
            &original,
            "agent-T-7-new"
        ));

        let card = board.get_card("T-7").unwrap();
        assert_eq!(card.column, original.column);
        assert_eq!(card.agent_chat_id, original.agent_chat_id);
        assert_eq!(card.assignee, original.assignee);
        assert_eq!(card.agent_worktree, original.agent_worktree);
        assert_eq!(card.agent_branch, original.agent_branch);
        assert_eq!(card.final_report, original.final_report);
        assert_eq!(card.completed_at, original.completed_at);
    }

    #[test]
    fn restart_agent_rollback_guard_keeps_newer_agent() {
        let original = failed_card("T-8", None, None);
        let mut board = TaskBoard::default();
        let mut newer = original.clone();
        mark_card_restarted_fresh(
            &mut newer,
            "newer-agent",
            "agent-T-8-newer",
            None,
            None,
            None,
        );
        board.cards.push(newer.clone());

        assert!(!restore_original_card_if_current_agent(
            &mut board,
            &original,
            "agent-T-8-stale"
        ));

        let card = board.get_card("T-8").unwrap();
        assert_eq!(card.agent_chat_id, newer.agent_chat_id);
        assert_eq!(card.assignee, newer.assignee);
        assert_eq!(card.column, "doing");
    }

    #[test]
    fn restart_agent_source_guard_accepts_exact_previous_binding() {
        let original = failed_card(
            "T-9",
            Some("/tmp/original-wt".to_string()),
            Some("refact/task/task-1/card/T-9/old".to_string()),
        );

        assert!(ensure_restart_source_current(&original, &original).is_ok());
    }

    #[test]
    fn restart_agent_source_guard_rejects_stale_previous_binding() {
        let original = failed_card("T-10", Some("/tmp/original-wt".to_string()), None);
        let mut changed = original.clone();
        changed.agent_chat_id = Some("agent-T-10-newer".to_string());
        changed.assignee = Some("newer-agent".to_string());

        let err = ensure_restart_source_current(&changed, &original).unwrap_err();

        assert!(err.contains("changed while restart was preparing"), "{err}");
        assert!(err.contains("agent-T-10-newer"), "{err}");
    }
}
