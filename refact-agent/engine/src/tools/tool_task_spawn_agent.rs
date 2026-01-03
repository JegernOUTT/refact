use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;
use async_trait::async_trait;
use uuid::Uuid;
use chrono::Utc;

use crate::tools::tools_description::{Tool, ToolDesc, ToolParam, ToolSource, ToolSourceType};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::tasks::storage;
use crate::tasks::types::StatusUpdate;
use crate::global_context::{GlobalContext, try_load_caps_quickly_if_not_present};
use crate::chat::types::{ThreadParams, TaskMeta, CommandRequest, ChatCommand};
use crate::chat::{get_or_create_session_with_trajectory, process_command_queue};
use crate::git::operations;

async fn get_task_id(ccx: &Arc<AMutex<AtCommandsContext>>, args: &HashMap<String, Value>) -> Result<String, String> {
    if let Some(id) = args.get("task_id").and_then(|v| v.as_str()) {
        return Ok(id.to_string());
    }
    let ccx_lock = ccx.lock().await;
    if let Some(ref meta) = ccx_lock.task_meta {
        return Ok(meta.task_id.clone());
    }
    storage::infer_task_id_from_chat_id(&ccx_lock.chat_id)
        .ok_or_else(|| "Missing 'task_id' (and chat is not bound to a task)".to_string())
}

async fn resolve_agent_model(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_default_model: Option<&str>,
    current_model: &str,
) -> Result<String, String> {
    if let Some(model) = task_default_model {
        if !model.is_empty() {
            return Ok(model.to_string());
        }
    }

    if !current_model.is_empty() {
        return Ok(current_model.to_string());
    }

    let caps = try_load_caps_quickly_if_not_present(gcx, 0).await
        .map_err(|e| format!("Failed to load caps for model resolution: {}", e))?;

    let default_model = &caps.defaults.chat_default_model;
    if !default_model.is_empty() {
        return Ok(default_model.clone());
    }

    Err("No model available: task default, current_model, and global default are all empty".to_string())
}

async fn setup_agent_worktree(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
    agent_id: &str,
    card_id: &str,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    let workspace_root = project_dirs.first().ok_or("No workspace folder found")?;

    let repo = match git2::Repository::open(workspace_root) {
        Ok(r) => r,
        Err(_) => {
            tracing::warn!("Workspace is not a git repository, skipping worktree creation");
            return Ok((None, None, None));
        }
    };

    if operations::has_uncommitted_changes(&repo)? {
        return Err("Please commit or stash changes before spawning agents".to_string());
    }

    let mut task_meta = storage::load_task_meta(gcx.clone(), task_id).await?;

    if task_meta.base_branch.is_none() || task_meta.base_commit.is_none() {
        let base_branch = operations::get_current_branch(&repo)?;
        let base_commit = operations::get_head_commit(&repo)?;
        task_meta.base_branch = Some(base_branch);
        task_meta.base_commit = Some(base_commit);
        storage::save_task_meta(gcx.clone(), task_id, &task_meta).await?;
    }

    let base_commit = task_meta.base_commit.as_ref().ok_or("No base commit found")?;
    let agent_id_short = &agent_id[..agent_id.len().min(8)];
    let branch_name = format!("refact/task/{}/card/{}/{}", task_id, card_id, agent_id_short);
    let worktree_name = format!("{}-{}-{}", task_id, card_id, agent_id_short);
    let worktree_path = workspace_root
        .join(".refact")
        .join("tasks")
        .join(task_id)
        .join("worktrees")
        .join(agent_id_short);

    tokio::fs::create_dir_all(worktree_path.parent().unwrap())
        .await
        .map_err(|e| format!("Failed to create worktree parent dir: {}", e))?;

    operations::create_worktree(&repo, &worktree_path, &worktree_name, &branch_name, base_commit)?;

    let card_id_owned = card_id.to_string();
    let branch_name_clone = branch_name.clone();
    let worktree_name_clone = worktree_name.clone();
    let worktree_path_str = worktree_path.to_string_lossy().to_string();
    let worktree_path_clone = worktree_path_str.clone();

    storage::update_board_atomic(gcx.clone(), task_id, move |board| {
        if let Some(card) = board.get_card_mut(&card_id_owned) {
            card.agent_branch = Some(branch_name_clone.clone());
            card.agent_worktree = Some(worktree_path_clone.clone());
            card.agent_worktree_name = Some(worktree_name_clone.clone());
        }
        Ok(())
    }).await?;

    Ok((Some(branch_name), Some(worktree_path_str), Some(worktree_name)))
}

pub struct ToolTaskSpawnAgent;

impl ToolTaskSpawnAgent {
    pub fn new() -> Self { Self }
}

fn build_agent_prompt(
    card_title: &str,
    instructions: &str,
    dependency_context: &str,
    suggested_steps: usize,
) -> String {
    let dep_section = if dependency_context.is_empty() {
        String::new()
    } else {
        format!("\n\n## Context from Dependencies\n{}", dependency_context)
    };

    format!(
        r#"# Card: {card_title}

## Instructions
{instructions}{dep_section}

## Guidelines
- Suggested step budget: ~{suggested_steps} steps
- Focus only on this specific card
- Report progress clearly
- **Remember to call `task_agent_finish()` when done!**"#
    )
}

#[async_trait]
impl Tool for ToolTaskSpawnAgent {
    fn as_any(&self) -> &dyn std::any::Any { self }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_spawn_agent".to_string(),
            display_name: "Task Spawn Agent".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            agentic: true,
            experimental: false,
            description: "Spawn an agent to work on a specific task card. The agent runs in the background as a real chat session. Returns immediately with a hyperlink to view the agent's progress. The agent will call task_agent_finish() when done.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "card_id".to_string(),
                    param_type: "string".to_string(),
                    description: "Card ID to work on".to_string(),
                },
                ToolParam {
                    name: "suggested_steps".to_string(),
                    param_type: "integer".to_string(),
                    description: "Suggested step budget for the agent (default: 30). This is a hint, not enforced.".to_string(),
                },
            ],
            parameters_required: vec!["card_id".to_string()],
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let ccx_lock = ccx.lock().await;
        
        let is_planner = ccx_lock.task_meta.as_ref()
            .map(|m| m.role == "planner")
            .unwrap_or(false);

        if !is_planner {
            return Err(
                "task_spawn_agent can only be called by the task planner. \
                 Switch to the planner chat to spawn agents.".to_string()
            );
        }
        
        drop(ccx_lock);
        
        let task_id = get_task_id(&ccx, args).await?;
        let card_id = args.get("card_id").and_then(|v| v.as_str())
            .ok_or("Missing 'card_id'")?;
        let suggested_steps: usize = match args.get("suggested_steps").or_else(|| args.get("max_steps")) {
            Some(Value::String(s)) => s.parse().unwrap_or(30),
            Some(Value::Number(n)) => n.as_u64().unwrap_or(30) as usize,
            _ => 30,
        };
        let suggested_steps = suggested_steps.min(50).max(1);

        let gcx = ccx.lock().await.global_context.clone();
        let current_model = ccx.lock().await.current_model.clone();

        let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
        if let Some(workspace_root) = project_dirs.first() {
            if let Ok(repo) = git2::Repository::open(workspace_root) {
                if operations::has_uncommitted_changes(&repo)? {
                    return Err("Cannot spawn agent: Please commit or stash changes before spawning agents".to_string());
                }
            }
        }

        let task_meta = storage::load_task_meta(gcx.clone(), &task_id).await?;
        let task_default_model = task_meta.default_agent_model.as_deref();

        let model = resolve_agent_model(gcx.clone(), task_default_model, &current_model).await?;

        let agent_id = Uuid::new_v4().to_string();
        let agent_chat_id = format!("agent-{}-{}", card_id, &agent_id[..8]);

        let (card_title, card_instructions, dependency_context) = {
            let card_id_owned = card_id.to_string();
            let agent_id_clone = agent_id.clone();
            let agent_chat_id_clone = agent_chat_id.clone();

            let (board, _) = storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
                let card = board.get_card_mut(&card_id_owned)
                    .ok_or(format!("Card {} not found", card_id_owned))?;

                if card.column == "done" {
                    return Err(format!("Card {} is already done", card_id_owned));
                }
                if card.column == "failed" {
                    return Err(format!("Card {} has failed. Reset it first if you want to retry.", card_id_owned));
                }
                if card.column != "planned" && card.column != "doing" {
                    return Err(format!("Card {} is in column '{}', expected 'planned' or 'doing'", card_id_owned, card.column));
                }
                if card.column == "doing" && card.agent_chat_id.is_some() {
                    let existing_chat_id = card.agent_chat_id.as_ref().unwrap();
                    return Err(format!(
                        "Card {} already has an active agent ({}). Use task_check_agents to monitor it, or move the card back to 'planned' to respawn.",
                        card_id_owned, existing_chat_id
                    ));
                }

                card.assignee = Some(agent_id_clone.clone());
                card.agent_chat_id = Some(agent_chat_id_clone.clone());
                card.started_at = Some(Utc::now().to_rfc3339());
                if card.column == "planned" {
                    card.column = "doing".to_string();
                }
                card.status_updates.push(StatusUpdate {
                    timestamp: Utc::now().to_rfc3339(),
                    message: "Agent started working on card".to_string(),
                });
                Ok(())
            }).await?;

            storage::update_task_stats(gcx.clone(), &task_id).await?;

            let card = board.get_card(card_id).unwrap();
            let dep_context = board.get_dependency_reports(card_id)
                .into_iter()
                .map(|(title, report)| format!("### {}\n{}", title, report))
                .collect::<Vec<_>>()
                .join("\n\n");

            let (_agent_branch, _agent_worktree, _agent_worktree_name) = match setup_agent_worktree(
                gcx.clone(),
                &task_id,
                &agent_id,
                card_id,
            ).await {
                Ok(result) => result,
                Err(e) if e.contains("not a git repository") || e.contains("No workspace folder") => {
                    tracing::warn!("Workspace is not a git repo, agent will work in main directory: {}", e);
                    (None, None, None)
                }
                Err(e) => {
                    return Err(format!("Cannot spawn agent: {}", e));
                }
            };

            (card.title.clone(), card.instructions.clone(), dep_context)
        };

        let sessions = {
            let gcx_locked = gcx.read().await;
            gcx_locked.chat_sessions.clone()
        };

        let session_arc = get_or_create_session_with_trajectory(gcx.clone(), &sessions, &agent_chat_id).await;

        {
            let mut session = session_arc.lock().await;

            session.thread = ThreadParams {
                id: agent_chat_id.clone(),
                title: format!("Agent: {}", card_title),
                model: model.clone(),
                mode: "TASK_AGENT".to_string(),
                tool_use: "agent".to_string(),
                boost_reasoning: false,
                context_tokens_cap: None,
                include_project_info: true,
                checkpoints_enabled: false,
                use_compression: true,
                is_title_generated: true,
                automatic_patch: false,
                task_meta: Some(TaskMeta {
                    task_id: task_id.clone(),
                    role: "agents".to_string(),
                    agent_id: Some(agent_id.clone()),
                    card_id: Some(card_id.to_string()),
                }),
            };

            let user_prompt = build_agent_prompt(&card_title, &card_instructions, &dependency_context, suggested_steps);
            let user_msg = ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText(user_prompt),
                ..Default::default()
            };
            session.add_message(user_msg);

            session.increment_version();
        }

        crate::chat::maybe_save_trajectory(gcx.clone(), session_arc.clone()).await;

        {
            let mut session = session_arc.lock().await;

            let request = CommandRequest {
                client_request_id: Uuid::new_v4().to_string(),
                priority: false,
                command: ChatCommand::Regenerate {},
            };
            session.command_queue.push_back(request);
            session.touch();

            let processor_running = session.queue_processor_running.clone();
            let queue_notify = session.queue_notify.clone();

            drop(session);

            if !processor_running.swap(true, Ordering::SeqCst) {
                tokio::spawn(process_command_queue(gcx.clone(), session_arc.clone(), processor_running));
            } else {
                queue_notify.notify_one();
            }
        }

        tracing::info!("Spawned agent {} for card {}: {} (model: {})", agent_id, card_id, card_title, model);

        let result_message = format!(
            r#"# Agent Spawned: {}

**Card:** {}
**Agent ID:** {}
**Model:** {}
**Status:** Running in background

📎 [View Agent Chat](refact://chat/{})

The agent will call `task_agent_finish()` when done. Use `task_check_agents` to monitor progress."#,
            card_title, card_id, agent_id, model, agent_chat_id
        );

        Ok((false, vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(result_message),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        })]))
    }

    fn tool_depends_on(&self) -> Vec<String> { vec![] }
}
