use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;
use async_trait::async_trait;
use uuid::Uuid;
use chrono::{DateTime, Utc};

use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::tasks::storage;
use crate::tasks::types::StatusUpdate;
use crate::global_context::{GlobalContext, try_load_caps_quickly_if_not_present};
use crate::chat::types::{ThreadParams, TaskMeta, CommandRequest, ChatCommand};
use crate::chat::{get_or_create_session_with_trajectory, process_command_queue};
use crate::git::operations;

async fn get_task_id(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<String, String> {
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

    let caps = try_load_caps_quickly_if_not_present(gcx, 0)
        .await
        .map_err(|e| format!("Failed to load caps for model resolution: {}", e))?;

    let default_model = &caps.defaults.chat_default_model;
    if !default_model.is_empty() {
        return Ok(default_model.clone());
    }

    Err(
        "No model available: task default, current_model, and global default are all empty"
            .to_string(),
    )
}

struct PreparedWorktree {
    branch_name: String,
    worktree_name: String,
    worktree_path: std::path::PathBuf,
    git_dir: std::path::PathBuf,
    base_branch: Option<String>,
    spawned_with_dirty_tree: bool,
    branch_was_created: bool,
}

impl PreparedWorktree {
    fn cleanup_sync(&self) {
        if let Ok(repo) = git2::Repository::open(&self.git_dir) {
            operations::remove_worktree(&repo, &self.worktree_name, &self.worktree_path);
            if self.branch_was_created {
                operations::delete_branch_if_exists(&repo, &self.branch_name);
            }
        } else {
            tracing::warn!(
                "cleanup: could not open repo at {} to remove worktree '{}'",
                self.git_dir.display(),
                self.worktree_name
            );
        }
    }
}

async fn prepare_agent_worktree(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
    agent_id: &str,
    card_id: &str,
) -> Result<Option<PreparedWorktree>, String> {
    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    let workspace_root = match project_dirs.first() {
        Some(p) => p.clone(),
        None => {
            tracing::warn!(
                "No workspace folder found, agent will work without an isolated worktree"
            );
            return Ok(None);
        }
    };

    let repo = match git2::Repository::discover(&workspace_root) {
        Ok(r) => r,
        Err(_) => {
            tracing::warn!(
                "Workspace is not a git repository — agent will work in the main workspace without isolation"
            );
            return Ok(None);
        }
    };

    let spawned_with_dirty_tree = operations::has_uncommitted_changes(&repo).unwrap_or(false);
    if spawned_with_dirty_tree {
        tracing::warn!(
            "Spawning agent from HEAD — local uncommitted changes are excluded from the agent's worktree"
        );
    }

    let base_commit = operations::get_head_commit(&repo)?;
    let base_branch = operations::get_current_branch(&repo).ok();
    let git_dir = repo.path().to_path_buf();

    let agent_id_short = &agent_id[..agent_id.len().min(8)];
    let branch_name = format!(
        "refact/task/{}/card/{}/{}",
        task_id, card_id, agent_id_short
    );
    let worktree_name = format!("{}-{}-{}", task_id, card_id, agent_id_short);
    let cache_dir = gcx.read().await.cache_dir.clone();
    let worktree_path = cache_dir
        .join("worktrees")
        .join(task_id)
        .join(agent_id_short);

    tokio::fs::create_dir_all(worktree_path.parent().unwrap())
        .await
        .map_err(|e| format!("Failed to create worktree parent dir: {}", e))?;

    let branch_was_created = operations::create_worktree(
        &repo,
        &worktree_path,
        &worktree_name,
        &branch_name,
        &base_commit,
    )?;

    Ok(Some(PreparedWorktree {
        branch_name,
        worktree_name,
        worktree_path,
        git_dir,
        base_branch,
        spawned_with_dirty_tree,
        branch_was_created,
    }))
}

fn parse_rfc3339_to_utc(ts: &str) -> Option<DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

pub struct ToolTaskSpawnAgent;

impl ToolTaskSpawnAgent {
    pub fn new() -> Self {
        Self
    }
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
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_spawn_agent".to_string(),
            display_name: "Task Spawn Agent".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Spawn an agent to work on a specific task card. The agent runs in the background as a real chat session. Returns immediately with a hyperlink to view the agent's progress. The agent will call task_agent_finish() when done.".to_string(),
            input_schema: json_schema_from_params(&[("card_id", "string", "Card ID to work on"), ("suggested_steps", "integer", "Suggested step budget for the agent (default: 30). This is a hint, not enforced.")], &["card_id"]),
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
            return Err("task_spawn_agent can only be called by the task planner. \
                 Switch to the planner chat to spawn agents."
                .to_string());
        }

        drop(ccx_lock);

        let task_id = get_task_id(&ccx, args).await?;
        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'card_id'")?;
        let suggested_steps: usize = match args
            .get("suggested_steps")
            .or_else(|| args.get("max_steps"))
        {
            Some(Value::String(s)) => s.parse().unwrap_or(30),
            Some(Value::Number(n)) => n.as_u64().unwrap_or(30) as usize,
            _ => 30,
        };
        let suggested_steps = suggested_steps.min(50).max(1);

        let gcx = ccx.lock().await.global_context.clone();
        let current_model = ccx.lock().await.current_model.clone();

        let task_meta = storage::load_task_meta(gcx.clone(), &task_id).await?;
        let task_default_model = task_meta.default_agent_model.as_deref();

        let model = resolve_agent_model(gcx.clone(), task_default_model, &current_model).await?;

        fn validate_id(id: &str, name: &str) -> Result<(), String> {
            if id
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            {
                Ok(())
            } else {
                Err(format!(
                    "{} '{}' contains invalid characters (only alphanumeric, '-', '_' allowed)",
                    name, id
                ))
            }
        }
        validate_id(&task_id, "task_id")?;
        validate_id(card_id, "card_id")?;

        let board = storage::load_board(gcx.clone(), &task_id).await?;
        let card = board
            .get_card(card_id)
            .ok_or_else(|| format!("Card {} not found", card_id))?;
        if card.column == "done" {
            return Err(format!("Card {} is already done", card_id));
        }
        if card.column == "failed" {
            return Err(format!(
                "Card {} has failed. Reset it first if you want to retry.",
                card_id
            ));
        }
        if card.column != "planned" && card.column != "doing" {
            return Err(format!(
                "Card {} is in column '{}', expected 'planned' or 'doing'",
                card_id, card.column
            ));
        }
        if card.column == "doing" && card.agent_chat_id.is_some() {
            return Err(format!(
                "Card {} already has an active agent ({}). Use task_check_agents to monitor it, or move the card back to 'planned' to respawn.",
                card_id, card.agent_chat_id.as_ref().unwrap()
            ));
        }

        let agent_id = Uuid::new_v4().to_string();
        let agent_chat_id = format!("agent-{}-{}", card_id, &agent_id[..8]);

        let prepared_worktree =
            prepare_agent_worktree(gcx.clone(), &task_id, &agent_id, card_id).await?;

        let dirty_tree_warning = prepared_worktree
            .as_ref()
            .map(|w| w.spawned_with_dirty_tree)
            .unwrap_or(false);

        let card_id_owned = card_id.to_string();
        let agent_id_clone = agent_id.clone();
        let agent_chat_id_clone = agent_chat_id.clone();
        let worktree_branch = prepared_worktree.as_ref().map(|w| w.branch_name.clone());
        let worktree_path_str = prepared_worktree
            .as_ref()
            .map(|w| w.worktree_path.to_string_lossy().to_string());
        let worktree_name = prepared_worktree.as_ref().map(|w| w.worktree_name.clone());
        let base_branch_from_prep = prepared_worktree
            .as_ref()
            .and_then(|w| w.base_branch.clone());

        let (board, commit_info) = storage::update_board_atomic(
            gcx.clone(),
            &task_id,
            move |board| {
                let agents_active_before = board
                    .cards
                    .iter()
                    .filter(|c| c.column == "doing" && c.assignee.is_some())
                    .count();

                let card = board
                    .get_card_mut(&card_id_owned)
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

                let original_card = card.clone();

                card.assignee = Some(agent_id_clone.clone());
                card.agent_chat_id = Some(agent_chat_id_clone.clone());
                card.started_at = Some(Utc::now().to_rfc3339());
                if card.column == "planned" {
                    card.column = "doing".to_string();
                }
                card.agent_branch = worktree_branch.clone();
                card.agent_worktree = worktree_path_str.clone();
                card.agent_worktree_name = worktree_name.clone();
                card.status_updates.push(StatusUpdate {
                    timestamp: Utc::now().to_rfc3339(),
                    message: "Agent started working on card".to_string(),
                });

                Ok(Some((original_card, agents_active_before == 0)))
            },
        )
        .await
        .map_err(|e| {
            if let Some(ref pw) = prepared_worktree {
                pw.cleanup_sync();
            }
            e
        })?;

        let (original_card, starting_new_run) = commit_info.unwrap();

        async fn rollback(
            gcx: Arc<ARwLock<GlobalContext>>,
            task_id: &str,
            original_card: crate::tasks::types::BoardCard,
            guard_chat_id: String,
            prepared_worktree: &Option<PreparedWorktree>,
        ) {
            let card_id_rb = original_card.id.clone();
            let _ = storage::update_board_atomic(gcx, task_id, move |board| {
                if let Some(card) = board.get_card_mut(&card_id_rb) {
                    if card.agent_chat_id.as_deref() == Some(&guard_chat_id) {
                        *card = original_card.clone();
                    }
                }
                Ok(Some(()))
            })
            .await;
            if let Some(pw) = prepared_worktree {
                pw.cleanup_sync();
            }
        }

        if let Err(e) = storage::update_task_stats(gcx.clone(), &task_id).await {
            rollback(
                gcx.clone(),
                &task_id,
                original_card,
                agent_chat_id.clone(),
                &prepared_worktree,
            )
            .await;
            return Err(e);
        }

        let mut meta = match storage::load_task_meta(gcx.clone(), &task_id).await {
            Ok(m) => m,
            Err(e) => {
                rollback(
                    gcx.clone(),
                    &task_id,
                    original_card,
                    agent_chat_id.clone(),
                    &prepared_worktree,
                )
                .await;
                return Err(e);
            }
        };
        if meta.base_branch.is_none() {
            meta.base_branch = base_branch_from_prep;
        }
        if starting_new_run {
            meta.last_agents_summary_at = Some(Utc::now().to_rfc3339());
        } else if meta.last_agents_summary_at.is_none() {
            let earliest = board
                .cards
                .iter()
                .filter(|c| c.column == "doing" && c.assignee.is_some())
                .filter_map(|c| c.started_at.as_deref())
                .filter_map(parse_rfc3339_to_utc)
                .min();
            meta.last_agents_summary_at = Some(earliest.unwrap_or_else(Utc::now).to_rfc3339());
        }
        if let Err(e) = storage::save_task_meta(gcx.clone(), &task_id, &meta).await {
            rollback(
                gcx.clone(),
                &task_id,
                original_card,
                agent_chat_id.clone(),
                &prepared_worktree,
            )
            .await;
            return Err(e);
        }

        if let Some(ref pw) = prepared_worktree {
            crate::files_in_workspace::add_folder(gcx.clone(), &pw.worktree_path).await;
        }

        let card_title = board
            .get_card(card_id)
            .map(|c| c.title.clone())
            .unwrap_or_default();
        let card_instructions = board
            .get_card(card_id)
            .map(|c| c.instructions.clone())
            .unwrap_or_default();
        let dependency_context = board
            .get_dependency_reports(card_id)
            .into_iter()
            .map(|(title, report)| format!("### {}\n{}", title, report))
            .collect::<Vec<_>>()
            .join("\n\n");

        let sessions = {
            let gcx_locked = gcx.read().await;
            gcx_locked.chat_sessions.clone()
        };

        let session_arc =
            get_or_create_session_with_trajectory(gcx.clone(), &sessions, &agent_chat_id).await;

        {
            let mut session = session_arc.lock().await;

            session.thread = ThreadParams {
                id: agent_chat_id.clone(),
                title: format!("Agent: {}", card_title),
                model: model.clone(),
                mode: "task_agent".to_string(),
                tool_use: "agent".to_string(),
                boost_reasoning: Some(false),
                context_tokens_cap: None,
                include_project_info: true,
                checkpoints_enabled: false,
                is_title_generated: true,
                auto_approve_editing_tools: true,
                auto_approve_dangerous_commands: false,
                task_meta: Some(TaskMeta {
                    task_id: task_id.clone(),
                    role: "agents".to_string(),
                    agent_id: Some(agent_id.clone()),
                    card_id: Some(card_id.to_string()),
                }),
                parent_id: None,
                link_type: None,
                root_chat_id: None,
                reasoning_effort: None,
                thinking_budget: None,
                temperature: None,
                frequency_penalty: None,
                max_tokens: None,
                parallel_tool_calls: None,
                previous_response_id: None,
                browser_meta: None,
                active_skill: None,
                auto_enrichment_enabled: None,
            };

            let user_prompt = build_agent_prompt(
                &card_title,
                &card_instructions,
                &dependency_context,
                suggested_steps,
            );
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
                tokio::spawn(process_command_queue(
                    gcx.clone(),
                    session_arc.clone(),
                    processor_running,
                ));
            } else {
                queue_notify.notify_one();
            }
        }

        tracing::info!(
            "Spawned agent {} for card {}: {} (model: {})",
            agent_id,
            card_id,
            card_title,
            model
        );

        let dirty_note = if dirty_tree_warning {
            "\n\n⚠️ Note: agent works from HEAD. Your uncommitted local changes are not included in the agent's worktree."
        } else if prepared_worktree.is_none() {
            "\n\n⚠️ Note: workspace is not a git repository — agent is working directly in the main workspace without isolation."
        } else {
            ""
        };

        let result_message = format!(
            r#"# Agent Spawned: {}

**Card:** {}
**Agent ID:** {}
**Model:** {}
**Status:** Running in background

📎 [View Agent Chat](refact://chat/{})

The agent will call `task_agent_finish()` when done. Use `task_check_agents` to monitor progress.{}"#,
            card_title, card_id, agent_id, model, agent_chat_id, dirty_note
        );

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
