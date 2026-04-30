use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;
use async_trait::async_trait;
use uuid::Uuid;
use chrono::{DateTime, Utc};

use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum, ContextFile};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::tasks::storage;
use crate::tasks::types::{BoardCard, StatusUpdate};
use crate::global_context::{GlobalContext, try_load_caps_quickly_if_not_present};
use crate::chat::types::{ThreadParams, TaskMeta, CommandRequest, ChatCommand};
use crate::chat::{get_or_create_session_with_trajectory, process_command_queue};
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::{CreateWorktreeRequest, WorktreeMeta};

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

#[derive(Debug)]
struct PreparedWorktree {
    meta: WorktreeMeta,
    branch_was_created: bool,
    spawned_with_dirty_tree: bool,
}

impl PreparedWorktree {
    fn branch_name(&self) -> Option<String> {
        self.meta.branch.clone()
    }

    fn worktree_name(&self) -> String {
        self.meta.id.clone()
    }

    fn worktree_path(&self) -> PathBuf {
        self.meta.root.clone()
    }

    fn source_workspace_root(&self) -> PathBuf {
        self.meta.source_workspace_root.clone()
    }

    async fn cleanup(&self, gcx: Arc<ARwLock<GlobalContext>>) {
        let cache_dir = gcx.read().await.cache_dir.clone();
        if let Ok(service) =
            WorktreeService::new(cache_dir, self.meta.source_workspace_root.clone())
        {
            if service
                .delete_worktree(&self.meta.id, self.branch_was_created)
                .await
                .is_ok()
            {
                return;
            }
        }
        if self.meta.root.exists() {
            let _ = std::fs::remove_dir_all(&self.meta.root);
        }
    }
}

fn find_abandoned_worktrees(board: &crate::tasks::types::TaskBoard) -> Vec<String> {
    board
        .cards
        .iter()
        .filter(|card| card.column != "doing")
        .filter_map(|card| {
            let worktree = card.agent_worktree.as_ref()?;
            if !std::path::Path::new(worktree).exists() {
                return None;
            }
            Some(format!(
                "- {} ({}) in column `{}`: `{}`",
                card.id, card.title, card.column, worktree
            ))
        })
        .collect()
}

async fn prepare_agent_worktree(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
    agent_id: &str,
    card_id: &str,
    agent_chat_id: &str,
) -> Result<PreparedWorktree, String> {
    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    let workspace_root = project_dirs.first().cloned().ok_or_else(|| {
        "No workspace folder found; task agents require an isolated git worktree".to_string()
    })?;

    let agent_id_short = &agent_id[..agent_id.len().min(8)];
    let branch_name = format!(
        "refact/task/{}/card/{}/{}",
        task_id, card_id, agent_id_short
    );
    let cache_dir = gcx.read().await.cache_dir.clone();
    let service = WorktreeService::new(cache_dir, workspace_root.clone())?;
    let created = service
        .create_worktree(CreateWorktreeRequest {
            source_workspace_root: Some(workspace_root.to_string_lossy().to_string()),
            branch: Some(branch_name),
            base_branch: None,
            chat_id: Some(agent_chat_id.to_string()),
            kind: Some("task_agent".to_string()),
            task_id: Some(task_id.to_string()),
            card_id: Some(card_id.to_string()),
            agent_id: Some(agent_id.to_string()),
        })
        .await
        .map_err(|e| format!("Failed to create task-agent worktree: {}", e))?;

    if created.dirty_source_warning {
        tracing::warn!(
            "Spawning agent from HEAD — local uncommitted changes are excluded from the agent's worktree"
        );
    }

    Ok(PreparedWorktree {
        meta: created.worktree.meta,
        branch_was_created: created.branch_was_created,
        spawned_with_dirty_tree: created.dirty_source_warning,
    })
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

fn mark_card_agent_started(
    card: &mut BoardCard,
    agent_id: &str,
    agent_chat_id: &str,
    worktree_branch: Option<String>,
    worktree_path: Option<String>,
    worktree_name: Option<String>,
) {
    card.assignee = Some(agent_id.to_string());
    card.agent_chat_id = Some(agent_chat_id.to_string());
    card.started_at = Some(Utc::now().to_rfc3339());
    if card.column == "planned" {
        card.column = "doing".to_string();
    }
    card.agent_branch = worktree_branch;
    card.agent_worktree = worktree_path;
    card.agent_worktree_name = worktree_name;
    card.status_updates.push(StatusUpdate {
        timestamp: Utc::now().to_rfc3339(),
        message: "Agent started working on card".to_string(),
    });
}

fn build_agent_thread_params(
    agent_chat_id: &str,
    card_title: &str,
    model: &str,
    task_id: &str,
    agent_id: &str,
    card_id: &str,
    worktree: WorktreeMeta,
) -> ThreadParams {
    ThreadParams {
        id: agent_chat_id.to_string(),
        title: format!("Agent: {}", card_title),
        model: model.to_string(),
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
            task_id: task_id.to_string(),
            role: "agents".to_string(),
            agent_id: Some(agent_id.to_string()),
            card_id: Some(card_id.to_string()),
        }),
        worktree: Some(worktree),
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
        buddy_meta: None,
    }
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
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "card_id": {
                        "type": "string",
                        "description": "Card ID to work on"
                    },
                    "suggested_steps": {
                        "type": "integer",
                        "description": "Suggested step budget for the agent (default: 30). This is a hint, not enforced."
                    },
                    "files_to_open": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of file paths to open immediately when the agent starts. The agent will see these files as context at the beginning of its session."
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
        let files_to_open: Vec<String> = args
            .get("files_to_open")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
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
        let abandoned_worktrees = find_abandoned_worktrees(&board);
        if !abandoned_worktrees.is_empty() {
            return Err(format!(
                "Cannot spawn a new task agent while abandoned task worktrees exist. \
                Clean them first with `task_merge_agent(card_id=...)` for merged cards, or remove them manually if they were intentionally abandoned.\n\n{}",
                abandoned_worktrees.join("\n")
            ));
        }

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
            prepare_agent_worktree(gcx.clone(), &task_id, &agent_id, card_id, &agent_chat_id)
                .await?;

        let dirty_tree_warning = prepared_worktree.spawned_with_dirty_tree;

        let card_id_owned = card_id.to_string();
        let agent_id_clone = agent_id.clone();
        let agent_chat_id_clone = agent_chat_id.clone();
        let worktree_branch = prepared_worktree.branch_name();
        let worktree_path_str = Some(
            prepared_worktree
                .worktree_path()
                .to_string_lossy()
                .to_string(),
        );
        let worktree_name = Some(prepared_worktree.worktree_name());
        let base_branch_from_prep = prepared_worktree.meta.base_branch.clone();
        let base_commit_from_prep = prepared_worktree.meta.base_commit.clone();

        let board_update_result = storage::update_board_atomic(
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

                mark_card_agent_started(
                    card,
                    &agent_id_clone,
                    &agent_chat_id_clone,
                    worktree_branch.clone(),
                    worktree_path_str.clone(),
                    worktree_name.clone(),
                );

                Ok(Some((original_card, agents_active_before == 0)))
            },
        )
        .await;
        let (board, commit_info) = match board_update_result {
            Ok(result) => result,
            Err(e) => {
                prepared_worktree.cleanup(gcx.clone()).await;
                return Err(e);
            }
        };

        let (original_card, starting_new_run) = commit_info.unwrap();

        async fn rollback(
            gcx: Arc<ARwLock<GlobalContext>>,
            task_id: &str,
            original_card: crate::tasks::types::BoardCard,
            guard_chat_id: String,
            prepared_worktree: &PreparedWorktree,
        ) {
            let card_id_rb = original_card.id.clone();
            let cleanup_gcx = gcx.clone();
            let _ = storage::update_board_atomic(gcx, task_id, move |board| {
                if let Some(card) = board.get_card_mut(&card_id_rb) {
                    if card.agent_chat_id.as_deref() == Some(&guard_chat_id) {
                        *card = original_card.clone();
                    }
                }
                Ok(Some(()))
            })
            .await;
            prepared_worktree.cleanup(cleanup_gcx).await;
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
        if meta.base_commit.is_none() {
            meta.base_commit = base_commit_from_prep;
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

            session.thread = build_agent_thread_params(
                &agent_chat_id,
                &card_title,
                &model,
                &task_id,
                &agent_id,
                card_id,
                prepared_worktree.meta.clone(),
            );

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

            if !files_to_open.is_empty() {
                let mut context_files: Vec<ContextFile> = Vec::new();
                for path_str in &files_to_open {
                    let orig = std::path::Path::new(path_str);
                    let source_root = prepared_worktree.source_workspace_root();
                    let worktree_path = prepared_worktree.worktree_path();
                    let resolved = match orig.strip_prefix(&source_root) {
                        Ok(rel) => worktree_path.join(rel),
                        Err(_) => worktree_path.join(path_str.trim_start_matches('/')),
                    };
                    match tokio::fs::read_to_string(&resolved).await {
                        Ok(content) => {
                            let line_count = content.lines().count().max(1);
                            context_files.push(ContextFile {
                                file_name: resolved.to_string_lossy().to_string(),
                                file_content: content,
                                line1: 1,
                                line2: line_count,
                                ..Default::default()
                            });
                        }
                        Err(e) => {
                            tracing::warn!(
                                "task_spawn_agent: could not read file {:?}: {}",
                                resolved,
                                e
                            );
                        }
                    }
                }
                if !context_files.is_empty() {
                    let ctx_msg = ChatMessage {
                        role: "context_file".to_string(),
                        content: ChatContent::ContextFiles(context_files),
                        tool_call_id: "initial_files".to_string(),
                        ..Default::default()
                    };
                    session.add_message(ctx_msg);
                }
            }

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
        } else {
            ""
        };

        let result_message = format!(
            r#"# Agent Spawned: {}

**Card:** {}
**Agent ID:** {}
**Model:** {}
**Status:** Running in background

The agent will call `task_agent_finish()` when done. Use `task_check_agents` to monitor progress.{}"#,
            card_title, card_id, agent_id, model, dirty_note
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::types::{BoardCard, TaskBoard};
    use std::path::Path;
    use std::process::Command;

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

    async fn set_workspace(gcx: Arc<ARwLock<GlobalContext>>, root: &Path) {
        let root = root.canonicalize().unwrap();
        let gcx_locked = gcx.read().await;
        *gcx_locked.documents_state.workspace_folders.lock().unwrap() = vec![root];
    }

    fn sample_worktree_meta(temp: &Path) -> WorktreeMeta {
        let root = temp.join("agent-root");
        let source = temp.join("source-root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&source).unwrap();
        WorktreeMeta {
            id: "wt-task-agent".to_string(),
            kind: "task_agent".to_string(),
            root,
            source_workspace_root: source.clone(),
            repo_root: source,
            branch: Some("refact/task/task-1/card/T-1/agent".to_string()),
            base_branch: Some("main".to_string()),
            base_commit: Some("base".to_string()),
            task_id: Some("task-1".to_string()),
            card_id: Some("T-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            enforce: true,
        }
    }

    fn test_card(id: &str, column: &str, worktree: Option<String>) -> BoardCard {
        BoardCard {
            id: id.to_string(),
            title: format!("Card {}", id),
            column: column.to_string(),
            priority: "P1".to_string(),
            depends_on: vec![],
            instructions: String::new(),
            assignee: None,
            agent_chat_id: None,
            status_updates: vec![],
            final_report: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            started_at: None,
            last_heartbeat_at: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: worktree,
            agent_worktree_name: None,
            target_files: vec![],
        }
    }

    #[tokio::test]
    async fn task_spawn_agent_non_git_workspace_fails_clearly() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;

        let err =
            prepare_agent_worktree(gcx, "task-1", "agent-12345678", "T-1", "agent-T-1-12345678")
                .await
                .unwrap_err();

        assert!(err.contains("not a git repository"), "{err}");
    }

    #[tokio::test]
    async fn task_spawn_agent_prepare_sets_worktree_meta_without_global_workspace_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        let before = crate::files_correction::get_project_dirs(gcx.clone()).await;

        let prepared = prepare_agent_worktree(
            gcx.clone(),
            "task-1",
            "agent-12345678",
            "T-1",
            "agent-T-1-12345678",
        )
        .await
        .unwrap();
        let after = crate::files_correction::get_project_dirs(gcx.clone()).await;

        assert_eq!(after, before);
        assert!(!after.contains(&prepared.meta.root));
        assert_eq!(prepared.meta.kind, "task_agent");
        assert!(prepared.meta.enforce);
        assert_eq!(prepared.meta.task_id.as_deref(), Some("task-1"));
        assert_eq!(prepared.meta.card_id.as_deref(), Some("T-1"));
        assert_eq!(prepared.meta.agent_id.as_deref(), Some("agent-12345678"));
        assert!(prepared.meta.root.is_dir());

        prepared.cleanup(gcx).await;
    }

    #[tokio::test]
    async fn task_spawn_agent_rollback_cleanup_removes_worktree_branch_and_registry() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        let prepared = prepare_agent_worktree(
            gcx.clone(),
            "task-1",
            "agent-abcdef12",
            "T-1",
            "agent-T-1-abcdef12",
        )
        .await
        .unwrap();
        let id = prepared.meta.id.clone();
        let branch = prepared.meta.branch.clone().unwrap();
        let root = prepared.meta.root.clone();
        let cache_dir = gcx.read().await.cache_dir.clone();
        let service = WorktreeService::new(cache_dir, source.canonicalize().unwrap()).unwrap();
        assert!(service.get_worktree(&id).await.is_ok());

        prepared.cleanup(gcx).await;

        assert!(!root.exists());
        assert!(service.get_worktree(&id).await.is_err());
        let branches = run_git(&source, &["branch", "--list", &branch]);
        assert!(branches.trim().is_empty());
    }

    #[test]
    fn task_spawn_agent_successful_spawn_sets_board_mirrors_and_thread_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let worktree = sample_worktree_meta(temp.path());
        let mut card = test_card("T-1", "planned", None);
        mark_card_agent_started(
            &mut card,
            "agent-1",
            "agent-chat-1",
            worktree.branch.clone(),
            Some(worktree.root.to_string_lossy().to_string()),
            Some(worktree.id.clone()),
        );

        assert_eq!(card.column, "doing");
        assert_eq!(card.assignee.as_deref(), Some("agent-1"));
        assert_eq!(card.agent_chat_id.as_deref(), Some("agent-chat-1"));
        assert_eq!(card.agent_branch, worktree.branch.clone());
        assert_eq!(
            card.agent_worktree.as_deref(),
            Some(worktree.root.to_str().unwrap())
        );
        assert_eq!(card.agent_worktree_name.as_deref(), Some("wt-task-agent"));

        let thread = build_agent_thread_params(
            "agent-chat-1",
            "Card T-1",
            "model-a",
            "task-1",
            "agent-1",
            "T-1",
            worktree.clone(),
        );
        assert_eq!(thread.mode, "task_agent");
        assert_eq!(thread.task_meta.as_ref().unwrap().role, "agents");
        assert_eq!(thread.worktree.as_ref(), Some(&worktree));
        assert!(thread.worktree.as_ref().unwrap().enforce);
    }

    #[test]
    fn abandoned_worktree_detection_ignores_active_agents() {
        let tempdir = tempfile::tempdir().unwrap();
        let worktree_path = tempdir.path().join("agent-worktree");
        std::fs::create_dir_all(&worktree_path).unwrap();

        let mut board = TaskBoard::default();
        board.cards.push(test_card(
            "T-1",
            "done",
            Some(worktree_path.to_string_lossy().to_string()),
        ));
        board.cards.push(test_card(
            "T-2",
            "doing",
            Some(worktree_path.to_string_lossy().to_string()),
        ));
        board.cards.push(test_card(
            "T-3",
            "failed",
            Some(tempdir.path().join("missing").to_string_lossy().to_string()),
        ));

        let abandoned = find_abandoned_worktrees(&board);
        assert_eq!(abandoned.len(), 1);
        assert!(abandoned[0].contains("T-1"));
        assert!(!abandoned[0].contains("T-2"));
        assert!(!abandoned[0].contains("T-3"));
    }
}
