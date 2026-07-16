use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;
use uuid::Uuid;
use chrono::{DateTime, Utc};

use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum, ContextFile};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::chat::internal_roles::{event, EventSubkind};
use crate::chat::trajectories::resolve_task_planner_controller_chat_id;
use crate::tasks::storage;
use crate::tasks::types::{BoardCard, StatusUpdate};
use crate::global_context::{GlobalContext, try_load_caps_quickly_if_not_present};
use crate::tasks::types::TaskMeta as StoredTaskMeta;
use crate::worktrees::git;
use crate::worktrees::service::WorktreeService;
use refact_chat_api::{ChatCommand, TaskMeta, ThreadParams};
use crate::worktrees::types::{CreateWorktreeRequest, WorktreeMeta};
use refact_runtime_api::CreateSessionRequest;

const FILES_TO_OPEN_PER_FILE_LIMIT: usize = 256 * 1024;
const FILES_TO_OPEN_TOTAL_LIMIT: usize = 1024 * 1024;
const MAX_ID_LEN: usize = 64;

fn validate_id(id: &str, name: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err(format!("{name} must not be empty"));
    }
    if id.len() > MAX_ID_LEN {
        return Err(format!("ID '{id}' is too long (max {MAX_ID_LEN} chars)"));
    }
    if id
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        Ok(())
    } else {
        Err(format!(
            "{name} '{id}' contains invalid characters (only alphanumeric, '-', '_' allowed)"
        ))
    }
}

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

pub(crate) async fn resolve_agent_model(
    gcx: Arc<GlobalContext>,
    current_model: &str,
) -> Result<String, String> {
    let caps = try_load_caps_quickly_if_not_present(gcx, 0)
        .await
        .map_err(|e| format!("Failed to load caps for model resolution: {}", e))?;

    let task_planner_model = caps.defaults.task_planner_agent_model.trim();
    if !task_planner_model.is_empty() {
        return Ok(task_planner_model.to_string());
    }

    if !current_model.is_empty() {
        return Ok(current_model.to_string());
    }

    let default_model = &caps.defaults.chat_default_model;
    if !default_model.is_empty() {
        return Ok(default_model.clone());
    }

    Err(
        "No model available: task_planner_agent_model, current_model, and global default are all empty"
            .to_string(),
    )
}

#[derive(Debug)]
pub(crate) struct PreparedWorktree {
    meta: WorktreeMeta,
    branch_was_created: bool,
    pub(crate) spawned_with_dirty_tree: bool,
    pub(crate) base_branch_mismatch_warning: Option<String>,
}

impl PreparedWorktree {
    pub(crate) fn branch_name(&self) -> Option<String> {
        self.meta.branch.clone()
    }

    pub(crate) fn worktree_name(&self) -> String {
        self.meta.id.clone()
    }

    pub(crate) fn worktree_path(&self) -> PathBuf {
        self.meta.root.clone()
    }

    pub(crate) fn source_workspace_root(&self) -> PathBuf {
        self.meta.source_workspace_root.clone()
    }

    pub(crate) fn worktree_meta(&self) -> WorktreeMeta {
        self.meta.clone()
    }

    pub(crate) async fn cleanup_unlinked(self, gcx: Arc<GlobalContext>) {
        let cache_dir = gcx.cache_dir.clone();
        if let Ok(service) =
            WorktreeService::new_async(cache_dir, self.meta.source_workspace_root.clone()).await
        {
            if service
                .delete_worktree(&self.meta.id, self.branch_was_created, true)
                .await
                .is_ok()
            {
                return;
            }
        }
        if self.meta.root.exists() {
            let _ = std::fs::remove_dir_all(&self.meta.root);
        }
        if self.branch_was_created {
            if let Some(branch) = self.meta.branch.as_deref() {
                if let Err(e) =
                    crate::worktrees::git::delete_branch(&self.meta.source_workspace_root, branch)
                {
                    tracing::warn!(
                        "Failed to delete leaked agent branch '{}' during cleanup_unlinked: {}",
                        branch,
                        e
                    );
                }
            }
        }
    }

    pub(crate) async fn retain(self, _gcx: Arc<GlobalContext>) {
        tracing::info!(
            "Retaining worktree '{}' / branch '{}' after spawn failure for inspection.",
            self.worktree_name(),
            self.branch_name()
                .unwrap_or_else(|| "<unknown>".to_string())
        );
    }
}

fn current_branch_for_workspace(
    workspace_root: &std::path::Path,
) -> Result<Option<String>, String> {
    let repo = git::discover_repo(workspace_root)?;
    Ok(git::current_branch(&repo))
}

fn task_base_branch_missing_error(branch: &str) -> String {
    format!(
        "Task base branch '{}' no longer exists. Update the task base branch or create a new task on the current branch.",
        branch
    )
}

fn map_task_base_branch_error(error: String, base_branch: Option<&str>) -> String {
    if let Some(branch) = base_branch {
        if error.contains(&format!("Base branch '{}' not found", branch)) {
            return task_base_branch_missing_error(branch);
        }
    }
    format!("Failed to create task-agent worktree: {}", error)
}

pub(crate) fn find_abandoned_worktrees(board: &crate::tasks::types::TaskBoard) -> Vec<String> {
    board
        .cards
        .iter()
        .filter(|card| card.column != "doing" && card.column != "failed")
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

pub(crate) async fn prepare_agent_worktree(
    gcx: Arc<GlobalContext>,
    task_meta: &StoredTaskMeta,
    task_id: &str,
    agent_id: &str,
    card_id: &str,
    agent_chat_id: &str,
) -> Result<PreparedWorktree, String> {
    prepare_agent_worktree_with_suffix(
        gcx,
        task_meta,
        task_id,
        agent_id,
        card_id,
        agent_chat_id,
        None,
    )
    .await
}

pub(crate) async fn prepare_agent_worktree_with_suffix(
    gcx: Arc<GlobalContext>,
    task_meta: &StoredTaskMeta,
    task_id: &str,
    agent_id: &str,
    card_id: &str,
    agent_chat_id: &str,
    branch_suffix: Option<&str>,
) -> Result<PreparedWorktree, String> {
    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    let workspace_root = project_dirs.first().cloned().ok_or_else(|| {
        "No workspace folder found; task agents require an isolated git worktree".to_string()
    })?;

    let agent_id_short = &agent_id[..agent_id.len().min(8)];
    let branch_name = format!(
        "refact/task/{}/card/{}/{}{}",
        task_id,
        card_id,
        agent_id_short,
        branch_suffix.unwrap_or("")
    );
    if branch_name.len() > 200 {
        return Err(format!(
            "Generated branch name '{branch_name}' exceeds 200 chars"
        ));
    }
    let cache_dir = gcx.cache_dir.clone();
    let service = WorktreeService::new_async(cache_dir, workspace_root.clone()).await?;
    let task_base_branch = task_meta.base_branch.clone();
    let current_branch = if task_base_branch.is_some() {
        current_branch_for_workspace(&workspace_root)?
    } else {
        None
    };
    if let Some(branch) = task_base_branch.as_deref() {
        if !git::branch_exists(&workspace_root, branch)? {
            return Err(task_base_branch_missing_error(branch));
        }
    }
    let base_branch_mismatch_warning = match (task_base_branch.as_deref(), current_branch.as_deref()) {
        (Some(task_branch), Some(current_branch)) if task_branch != current_branch => Some(format!(
            "Current repo HEAD is on branch '{}' but this task was created from '{}'; spawning agent from the task base branch.",
            current_branch, task_branch
        )),
        (Some(task_branch), None) => Some(format!(
            "Current repo HEAD is detached but this task was created from '{}'; spawning agent from the task base branch.",
            task_branch
        )),
        _ => None,
    };
    let created = service
        .create_worktree(CreateWorktreeRequest {
            source_workspace_root: Some(workspace_root.to_string_lossy().to_string()),
            branch: Some(branch_name),
            base_branch: task_base_branch,
            chat_id: Some(agent_chat_id.to_string()),
            kind: Some("task_agent".to_string()),
            task_id: Some(task_id.to_string()),
            card_id: Some(card_id.to_string()),
            agent_id: Some(agent_id.to_string()),
        })
        .await
        .map_err(|e| map_task_base_branch_error(e, task_meta.base_branch.as_deref()))?;

    if created.dirty_source_warning {
        tracing::warn!(
            "Spawning agent from committed base — local uncommitted changes are excluded from the agent's worktree"
        );
    }
    if let Some(warning) = base_branch_mismatch_warning.as_deref() {
        tracing::warn!("spawn_agent: {}", warning);
    }

    Ok(PreparedWorktree {
        meta: created.worktree.meta,
        branch_was_created: created.branch_was_created,
        spawned_with_dirty_tree: created.dirty_source_warning,
        base_branch_mismatch_warning,
    })
}

fn parse_rfc3339_to_utc(ts: &str) -> Option<DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn validate_files_to_open(args: &HashMap<String, Value>) -> Result<Vec<String>, String> {
    let raw = args
        .get("files_to_open")
        .and_then(|v| v.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    let mut paths = Vec::with_capacity(raw.len());
    for (idx, v) in raw.iter().enumerate() {
        match v.as_str() {
            None => {
                let type_name = match v {
                    Value::Null => "null",
                    Value::Bool(_) => "bool",
                    Value::Number(_) => "number",
                    Value::Array(_) => "array",
                    Value::Object(_) => "object",
                    Value::String(_) => unreachable!(),
                };
                return Err(format!(
                    "files_to_open[{idx}] must be a string, got {type_name}"
                ));
            }
            Some(s) if s.is_empty() => {
                return Err(format!("files_to_open[{idx}] is empty"));
            }
            Some(s) => paths.push(s.to_string()),
        }
    }
    let mut seen = std::collections::HashSet::new();
    let mut has_duplicates = false;
    let mut deduped = Vec::with_capacity(paths.len());
    for path in paths {
        if seen.insert(path.clone()) {
            deduped.push(path);
        } else {
            has_duplicates = true;
        }
    }
    if has_duplicates {
        tracing::warn!("files_to_open contains duplicate paths; deduplicating");
    }
    Ok(deduped)
}

pub struct ToolTaskSpawnAgent;

impl ToolTaskSpawnAgent {
    pub fn new() -> Self {
        Self
    }
}

pub(crate) fn build_agent_prompt(
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
- **Remember to call `agent_finish()` when done!**"#
    )
}

pub(crate) fn mark_card_agent_started(
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

pub(crate) fn restore_original_card_if_current_agent(
    board: &mut crate::tasks::types::TaskBoard,
    original_card: &BoardCard,
    guard_chat_id: &str,
) -> bool {
    if let Some(card) = board.get_card_mut(&original_card.id) {
        if card.agent_chat_id.as_deref() == Some(guard_chat_id) {
            *card = original_card.clone();
            return true;
        }
    }
    false
}

pub(crate) async fn rollback_retain(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
    guard_chat_id: &str,
    error_msg: &str,
) {
    let card_id = card_id.to_string();
    let guard_chat_id = guard_chat_id.to_string();
    let truncated: String = error_msg.chars().take(300).collect();
    let msg = format!(
        "Spawn rolled back: {}. Worktree retained for inspection.",
        truncated
    );
    let _ = storage::update_board_atomic(gcx.clone(), task_id, move |board| {
        if let Some(card) = board.get_card_mut(&card_id) {
            if card.agent_chat_id.as_deref() == Some(&guard_chat_id) {
                card.column = "failed".to_string();
                card.status_updates.push(StatusUpdate {
                    timestamp: Utc::now().to_rfc3339(),
                    message: msg,
                });
            }
        }
        Ok(Some(()))
    })
    .await;
    let _ = storage::update_task_stats(gcx, task_id).await;
}

pub(crate) fn build_agent_thread_params(
    agent_chat_id: &str,
    card_title: &str,
    model: &str,
    task_id: &str,
    agent_id: &str,
    card_id: &str,
    planner_chat_id: &str,
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
        autonomous_no_confirm: false,
        task_meta: Some(TaskMeta {
            task_id: task_id.to_string(),
            role: "agents".to_string(),
            agent_id: Some(agent_id.to_string()),
            card_id: Some(card_id.to_string()),
            planner_chat_id: Some(planner_chat_id.to_string()),
        }),
        worktree: Some(worktree),
        parent_id: Some(planner_chat_id.to_string()),
        link_type: Some("task_agent".to_string()),
        root_chat_id: Some(planner_chat_id.to_string()),
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
        auto_compact_enabled: None,
        frozen_request_prefix: None,
        claude_code_identity: None,
        reactive_compact_attempts: None,
    }
}

pub(crate) async fn resolve_invoking_planner_chat_id(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
    root_chat_id: &str,
    task_meta: Option<&TaskMeta>,
) -> String {
    if let Some(task_meta) = task_meta {
        if !root_chat_id.is_empty()
            && root_chat_id != chat_id
            && root_chat_is_planner_for_task(gcx.clone(), root_chat_id, &task_meta.task_id).await
        {
            return root_chat_id.to_string();
        }
    }
    resolve_task_planner_controller_chat_id(gcx, chat_id, Some(root_chat_id), task_meta).await
}

async fn root_chat_is_planner_for_task(
    gcx: Arc<GlobalContext>,
    root_chat_id: &str,
    task_id: &str,
) -> bool {
    let session_arc = {
        let sessions = gcx.chat_sessions.read().await;
        sessions.get(root_chat_id).cloned()
    };
    if let Some(session_arc) = session_arc {
        let session = session_arc.lock().await;
        if session
            .thread
            .task_meta
            .as_ref()
            .is_some_and(|meta| meta.role == "planner" && meta.task_id == task_id)
        {
            return true;
        }
    }
    crate::chat::trajectories::load_trajectory_for_chat(gcx, root_chat_id)
        .await
        .is_some_and(|loaded| {
            loaded
                .thread
                .task_meta
                .as_ref()
                .is_some_and(|meta| meta.role == "planner" && meta.task_id == task_id)
        })
}

fn resolve_files_to_open_path(worktree_root: &Path, source_root: &Path, path_str: &str) -> PathBuf {
    let requested = Path::new(path_str);
    if requested.is_absolute() {
        if let Ok(relative) = requested.strip_prefix(source_root) {
            return worktree_root.join(relative);
        }
        return requested.to_path_buf();
    }
    worktree_root.join(path_str)
}

async fn context_files_from_files_to_open(
    gcx: Arc<GlobalContext>,
    worktree_root: &Path,
    source_root: &Path,
    files_to_open: &[String],
) -> Result<Vec<ContextFile>, String> {
    let worktree_canonical = dunce::canonicalize(worktree_root).map_err(|e| {
        format!(
            "Failed to canonicalize agent worktree '{}': {}",
            worktree_root.display(),
            e
        )
    })?;
    let mut context_files = Vec::new();
    let mut total_bytes = 0usize;
    for path_str in files_to_open {
        let resolved = resolve_files_to_open_path(worktree_root, source_root, path_str);
        let canonical_resolved = dunce::canonicalize(&resolved).map_err(|e| {
            format!(
                "files_to_open '{}' does not exist or cannot be resolved: {}",
                path_str, e
            )
        })?;
        if !canonical_resolved.starts_with(&worktree_canonical) {
            return Err(format!(
                "files_to_open '{}' resolves outside agent worktree: {}",
                path_str,
                canonical_resolved.display()
            ));
        }
        crate::files_in_workspace::check_file_privacy_for_send(gcx.clone(), &canonical_resolved)
            .await
            .map_err(|e| {
                format!(
                    "files_to_open '{}' is blocked by privacy settings: {}",
                    path_str, e
                )
            })?;
        if total_bytes >= FILES_TO_OPEN_TOTAL_LIMIT {
            context_files.push(ContextFile {
                file_name: canonical_resolved.to_string_lossy().to_string(),
                ..Default::default()
            });
            continue;
        }
        let bytes = tokio::fs::read(&canonical_resolved).await.map_err(|e| {
            format!(
                "Could not read files_to_open '{}': {}",
                canonical_resolved.display(),
                e
            )
        })?;
        let remaining_total = FILES_TO_OPEN_TOTAL_LIMIT - total_bytes;
        let read_limit = FILES_TO_OPEN_PER_FILE_LIMIT.min(remaining_total);
        let bytes_to_include = bytes.len().min(read_limit);
        let omitted_bytes = bytes.len().saturating_sub(bytes_to_include);
        let mut content = String::from_utf8_lossy(&bytes[..bytes_to_include]).to_string();
        if omitted_bytes > 0 {
            content.push_str(&format!("\n... (truncated, {} more bytes)", omitted_bytes));
        }
        total_bytes += bytes_to_include;
        let line_count = content.lines().count().max(1);
        context_files.push(ContextFile {
            file_name: canonical_resolved.to_string_lossy().to_string(),
            file_content: content,
            line1: 1,
            line2: line_count,
            ..Default::default()
        });
    }
    Ok(context_files)
}

#[async_trait]
impl Tool for ToolTaskSpawnAgent {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "spawn_agent".to_string(),
            display_name: "Task Spawn Agent".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Spawn an agent to work on a specific task card. The agent runs in the background as a real chat session. Returns immediately with a hyperlink to view the agent's progress. The agent will call agent_finish() when done.".to_string(),
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
            return Err("spawn_agent can only be called by the task planner. \
                 Switch to the planner chat to spawn agents."
                .to_string());
        }

        drop(ccx_lock);

        let (gcx, current_model, task_id, chat_id, root_chat_id, task_meta) = {
            let ccx_lock = ccx.lock().await;
            let task_id = ccx_lock
                .task_meta
                .as_ref()
                .map(|m| m.task_id.clone())
                .unwrap_or_else(|| {
                    args.get("task_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                        .unwrap_or_default()
                });
            (
                ccx_lock.app.gcx.clone(),
                ccx_lock.current_model.clone(),
                task_id,
                ccx_lock.chat_id.clone(),
                ccx_lock.root_chat_id.clone(),
                ccx_lock.task_meta.clone(),
            )
        };
        let planner_chat_id = resolve_invoking_planner_chat_id(
            gcx.clone(),
            &chat_id,
            &root_chat_id,
            task_meta.as_ref(),
        )
        .await;
        let task_id = if task_id.is_empty() {
            get_task_id(&ccx, args).await?
        } else {
            task_id
        };
        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'card_id'")?;
        let files_to_open = validate_files_to_open(args)?;
        let suggested_steps: usize = match args
            .get("suggested_steps")
            .or_else(|| args.get("max_steps"))
        {
            Some(Value::String(s)) => s.parse().unwrap_or(30),
            Some(Value::Number(n)) => n.as_u64().unwrap_or(30) as usize,
            _ => 30,
        };
        let suggested_steps = suggested_steps.min(50).max(1);

        let task_meta = storage::load_task_meta(gcx.clone(), &task_id).await?;

        let model = resolve_agent_model(gcx.clone(), &current_model).await?;
        crate::tools::task_tool_helpers::preflight_agent_model(gcx.clone(), &model).await?;

        validate_id(&task_id, "task_id")?;
        validate_id(card_id, "card_id")?;

        let board = storage::load_board(gcx.clone(), &task_id).await?;
        let abandoned_worktrees = find_abandoned_worktrees(&board);
        if !abandoned_worktrees.is_empty() {
            return Err(format!(
                "Cannot spawn a new task agent while abandoned task worktrees exist. \
                Clean them first with `merge_agent(card_id=...)` for merged cards, or remove them manually if they were intentionally abandoned.\n\n{}",
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
                "Card {} already has an active agent ({}). Use check_agents to monitor it, or move the card back to 'planned' to respawn.",
                card_id, card.agent_chat_id.as_ref().unwrap()
            ));
        }

        let agent_id = Uuid::new_v4().to_string();
        let agent_chat_id = format!("agent-{}-{}", card_id, &agent_id[..8]);

        let prepared_worktree = prepare_agent_worktree(
            gcx.clone(),
            &task_meta,
            &task_id,
            &agent_id,
            card_id,
            &agent_chat_id,
        )
        .await?;

        let dirty_tree_warning = prepared_worktree.spawned_with_dirty_tree;
        let base_branch_mismatch_warning = prepared_worktree.base_branch_mismatch_warning.clone();

        let initial_context_files = if files_to_open.is_empty() {
            Vec::new()
        } else {
            let worktree_path = prepared_worktree.worktree_path();
            let source_workspace_root = prepared_worktree.source_workspace_root();
            match context_files_from_files_to_open(
                gcx.clone(),
                &worktree_path,
                &source_workspace_root,
                &files_to_open,
            )
            .await
            {
                Ok(context_files) => context_files,
                Err(e) => {
                    prepared_worktree.cleanup_unlinked(gcx.clone()).await;
                    return Err(e);
                }
            }
        };

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
                        "Card {} already has an active agent ({}). Use check_agents to monitor it, or move the card back to 'planned' to respawn.",
                        card_id_owned, existing_chat_id
                    ));
                }

                mark_card_agent_started(
                    card,
                    &agent_id_clone,
                    &agent_chat_id_clone,
                    worktree_branch.clone(),
                    worktree_path_str.clone(),
                    worktree_name.clone(),
                );

                Ok(Some(agents_active_before == 0))
            },
        )
        .await;
        let (board, commit_info) = match board_update_result {
            Ok(result) => result,
            Err(e) => {
                prepared_worktree.cleanup_unlinked(gcx.clone()).await;
                return Err(e);
            }
        };

        let starting_new_run = commit_info.unwrap();

        if let Err(e) = storage::update_task_stats(gcx.clone(), &task_id).await {
            rollback_retain(gcx.clone(), &task_id, card_id, &agent_chat_id, &e).await;
            prepared_worktree.retain(gcx.clone()).await;
            return Err(e);
        }

        let mut meta = match storage::load_task_meta(gcx.clone(), &task_id).await {
            Ok(m) => m,
            Err(e) => {
                rollback_retain(gcx.clone(), &task_id, card_id, &agent_chat_id, &e).await;
                prepared_worktree.retain(gcx.clone()).await;
                return Err(e);
            }
        };
        meta.base_branch = base_branch_from_prep;
        meta.base_commit = base_commit_from_prep;
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
            rollback_retain(gcx.clone(), &task_id, card_id, &agent_chat_id, &e).await;
            prepared_worktree.retain(gcx.clone()).await;
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

        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let thread = build_agent_thread_params(
            &agent_chat_id,
            &card_title,
            &model,
            &task_id,
            &agent_id,
            card_id,
            &planner_chat_id,
            prepared_worktree.meta.clone(),
        );
        let user_prompt = build_agent_prompt(
            &card_title,
            &card_instructions,
            &dependency_context,
            suggested_steps,
        );
        let user_msg = event(
            EventSubkind::SystemNotice,
            "tool.task_spawn_agent",
            json!({
                "task_id": &task_id,
                "planner_chat_id": &planner_chat_id,
                "card_id": card_id,
                "card_title": &card_title,
                "agent_id": &agent_id,
                "agent_chat_id": &agent_chat_id,
                "model": &model,
                "suggested_steps": suggested_steps,
                "files_to_open": &files_to_open,
                "worktree": prepared_worktree.worktree_path().to_string_lossy().to_string(),
                "worktree_name": prepared_worktree.worktree_name(),
                "branch": prepared_worktree.branch_name(),
            }),
            user_prompt,
        );
        let mut messages = vec![user_msg];

        if !initial_context_files.is_empty() {
            messages.push(ChatMessage {
                role: "context_file".to_string(),
                content: ChatContent::ContextFiles(initial_context_files),
                tool_call_id: "initial_files".to_string(),
                ..Default::default()
            });
        }

        let session_result = async {
            app.chat
                .facade
                .create_session(CreateSessionRequest {
                    chat_id: agent_chat_id.clone(),
                    thread,
                    messages,
                })
                .await?;
            app.chat.facade.maybe_save_session(&agent_chat_id).await?;
            app.chat
                .facade
                .push_priority_command(&agent_chat_id, ChatCommand::Regenerate {})
                .await
        }
        .await;
        if let Err(e) = session_result {
            rollback_retain(gcx.clone(), &task_id, card_id, &agent_chat_id, &e).await;
            prepared_worktree.retain(gcx.clone()).await;
            return Err(e);
        }

        tracing::info!(
            "Spawned agent {} for card {}: {} (model: {})",
            agent_id,
            card_id,
            card_title,
            model
        );

        let dirty_note = if dirty_tree_warning {
            "\n\n⚠️ Note: agent works from the committed base. Your uncommitted local changes are not included in the agent's worktree."
        } else {
            ""
        };
        let branch_note = base_branch_mismatch_warning
            .as_ref()
            .map(|warning| format!("\n\n⚠️ Note: {}", warning))
            .unwrap_or_default();

        let result_message = format!(
            r#"# Agent Spawned: {}

**Card:** {}
**Agent ID:** {}
**Model:** {}
**Status:** Running in background

The agent will call `agent_finish()` when done. Use `check_agents` to monitor progress.{}{}"#,
            card_title, card_id, agent_id, model, dirty_note, branch_note
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
    use crate::privacy::{FilePrivacySettings, PrivacySettings};
    use crate::tasks::storage;
    use crate::tasks::types::{BoardCard, TaskBoard, TaskMeta, TaskStatus};
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

    fn commit_file(root: &Path, name: &str, content: &str, message: &str) -> String {
        std::fs::write(root.join(name), content).unwrap();
        run_git(root, &["add", name]);
        run_git(root, &["commit", "-m", message]);
        run_git(root, &["rev-parse", "HEAD"]).trim().to_string()
    }

    async fn set_workspace(gcx: Arc<GlobalContext>, root: &Path) {
        let root = root.canonicalize().unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root];
    }

    async fn set_caps(gcx: &Arc<GlobalContext>, caps: crate::caps::CodeAssistantCaps) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut state = gcx.caps_state.write().await;
        state.caps = Some(Arc::new(caps));
        state.last_attempted_ts = now;
    }

    async fn test_gcx_with_privacy(blocked: Vec<String>) -> Arc<GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.privacy_settings.write().unwrap() = Arc::new(PrivacySettings {
            privacy_rules: FilePrivacySettings {
                only_send_to_servers_I_control: vec![],
                blocked,
            },
            loaded_ts: u64::MAX / 2,
        });
        gcx
    }

    fn sample_task_meta(base_branch: Option<&str>) -> StoredTaskMeta {
        let now = Utc::now().to_rfc3339();
        StoredTaskMeta {
            schema_version: 1,
            id: "task-1".to_string(),
            name: "Task".to_string(),
            status: TaskStatus::Active,
            created_at: now.clone(),
            updated_at: now,
            cards_total: 0,
            cards_done: 0,
            cards_failed: 0,
            agents_active: 0,
            base_branch: base_branch.map(|branch| branch.to_string()),
            base_commit: None,
            default_agent_model: None,
            is_name_generated: false,
            last_agents_summary_at: None,
            planner_session_state: None,
        }
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

    #[test]
    fn build_agent_thread_params_starts_with_fresh_compaction_and_cache_state() {
        let temp = tempfile::tempdir().unwrap();
        let params = build_agent_thread_params(
            "agent-chat-1",
            "Card Title",
            "provider/model",
            "task-1",
            "agent-1",
            "T-1",
            "planner-chat-1",
            sample_worktree_meta(temp.path()),
        );

        // Task-agent children must never inherit the planner's compaction or
        // provider-cache state; a stale cap/response id would corrupt the
        // child's first generation.
        assert_eq!(params.context_tokens_cap, None);
        assert_eq!(params.previous_response_id, None);
        assert_eq!(params.auto_compact_enabled, None);
        assert_eq!(params.reactive_compact_attempts, None);
        assert_eq!(params.frozen_request_prefix, None);
        assert_eq!(params.mode, "task_agent");
        assert_eq!(params.parent_id.as_deref(), Some("planner-chat-1"));
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
            comments: vec![],
            final_report: None,
            final_report_structured: None,
            verifier_report: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            started_at: None,
            last_heartbeat_at: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: worktree,
            agent_worktree_name: None,
            ab_variants: None,
            team_members: vec![],
            target_files: vec![],
            scope_guard_mode: Default::default(),
        }
    }

    #[test]
    fn validate_id_rejects_overlong_ids() {
        assert!(validate_id("short-id", "test_id").is_ok());
        assert!(validate_id(&"a".repeat(MAX_ID_LEN), "test_id").is_ok());
        let err = validate_id(&"a".repeat(MAX_ID_LEN + 1), "test_id").unwrap_err();
        assert!(
            err.contains("too long"),
            "expected 'too long' in error: {err}"
        );
    }

    #[test]
    fn validate_id_rejects_empty_string() {
        let err = validate_id("", "task_id").unwrap_err();
        assert!(err.contains("must not be empty"), "{err}");
    }

    #[tokio::test]
    async fn resolve_agent_model_uses_task_planner_agent_model_over_current() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut caps = crate::caps::CodeAssistantCaps::default();
        caps.defaults.task_planner_agent_model = "provider/task-planner".to_string();
        caps.defaults.chat_default_model = "provider/chat".to_string();
        set_caps(&gcx, caps).await;

        let model = resolve_agent_model(gcx, "provider/current").await.unwrap();

        assert_eq!(model, "provider/task-planner");
    }

    #[tokio::test]
    async fn resolve_agent_model_falls_back_without_task_planner_agent_model() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut caps = crate::caps::CodeAssistantCaps::default();
        caps.defaults.chat_default_model = "provider/chat".to_string();
        set_caps(&gcx, caps).await;

        let model = resolve_agent_model(gcx, "provider/current").await.unwrap();

        assert_eq!(model, "provider/current");
    }

    #[tokio::test]
    async fn resolve_agent_model_uses_global_default_when_current_is_empty() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut caps = crate::caps::CodeAssistantCaps::default();
        caps.defaults.chat_default_model = "provider/chat".to_string();
        set_caps(&gcx, caps).await;

        let model = resolve_agent_model(gcx, "").await.unwrap();

        assert_eq!(model, "provider/chat");
    }

    #[tokio::test]
    async fn spawn_agent_rejects_overlong_branch_combination() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        // 100-char IDs: "refact/task/" (12) + 100 + "/card/" (6) + 100 + "/" (1) + 8 = 227 > 200
        let task_id = "a".repeat(100);
        let card_id = "b".repeat(100);
        let task_meta = sample_task_meta(None);
        let err = prepare_agent_worktree_with_suffix(
            gcx,
            &task_meta,
            &task_id,
            "agent-12345678",
            &card_id,
            "agent-chat-id",
            None,
        )
        .await
        .unwrap_err();
        assert!(
            err.contains("exceeds 200 chars"),
            "expected branch-too-long error: {err}"
        );
    }

    #[tokio::test]
    async fn spawn_agent_non_git_workspace_fails_clearly() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir_all(&source).unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;

        let task_meta = sample_task_meta(None);
        let err = prepare_agent_worktree(
            gcx,
            &task_meta,
            "task-1",
            "agent-12345678",
            "T-1",
            "agent-T-1-12345678",
        )
        .await
        .unwrap_err();

        assert!(err.contains("not a git repository"), "{err}");
    }

    #[tokio::test]
    async fn spawn_agent_prepare_sets_worktree_meta_without_global_workspace_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        let before = crate::files_correction::get_project_dirs(gcx.clone()).await;

        let task_meta = sample_task_meta(None);
        let prepared = prepare_agent_worktree(
            gcx.clone(),
            &task_meta,
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

        prepared.cleanup_unlinked(gcx).await;
    }

    #[tokio::test]
    async fn spawn_agent_rollback_cleanup_removes_worktree_branch_and_registry() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        let task_meta = sample_task_meta(None);
        let prepared = prepare_agent_worktree(
            gcx.clone(),
            &task_meta,
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
        let cache_dir = gcx.cache_dir.clone();
        let service = WorktreeService::new(cache_dir, source.canonicalize().unwrap()).unwrap();
        assert!(service.get_worktree(&id).await.is_ok());

        prepared.cleanup_unlinked(gcx).await;

        assert!(!root.exists());
        assert!(service.get_worktree(&id).await.is_err());
        let branches = run_git(&source, &["branch", "--list", &branch]);
        assert!(branches.trim().is_empty());
    }

    #[tokio::test]
    async fn spawn_agent_uses_stored_base_branch_not_current_head() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let main_head = commit_file(&source, "main_only.txt", "only main\n", "main-only");
        run_git(&source, &["checkout", "-b", "dev"]);
        let dev_head = commit_file(&source, "dev_only.txt", "only dev\n", "dev-only");
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        let task_meta = sample_task_meta(Some("main"));

        let prepared = prepare_agent_worktree(
            gcx.clone(),
            &task_meta,
            "task-1",
            "agent-11111111",
            "T-1",
            "agent-T-1-11111111",
        )
        .await
        .unwrap();

        assert_eq!(prepared.meta.base_branch.as_deref(), Some("main"));
        assert_eq!(
            prepared.meta.base_commit.as_deref(),
            Some(main_head.as_str())
        );
        assert!(prepared.meta.root.join("main_only.txt").is_file());
        assert!(!prepared.meta.root.join("dev_only.txt").exists());
        assert_eq!(run_git(&source, &["rev-parse", "dev"]).trim(), dev_head);

        prepared.cleanup_unlinked(gcx).await;
    }

    #[tokio::test]
    async fn spawn_agent_none_base_branch_falls_back_to_current_head() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        run_git(&source, &["checkout", "-b", "dev"]);
        let dev_head = commit_file(&source, "dev_only.txt", "only dev\n", "dev-only");
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        let task_meta = sample_task_meta(None);

        let prepared = prepare_agent_worktree(
            gcx.clone(),
            &task_meta,
            "task-1",
            "agent-22222222",
            "T-1",
            "agent-T-1-22222222",
        )
        .await
        .unwrap();

        assert_eq!(prepared.meta.base_branch.as_deref(), Some("dev"));
        assert_eq!(
            prepared.meta.base_commit.as_deref(),
            Some(dev_head.as_str())
        );
        assert!(prepared.meta.root.join("dev_only.txt").is_file());
        assert!(prepared.base_branch_mismatch_warning.is_none());

        prepared.cleanup_unlinked(gcx).await;
    }

    #[tokio::test]
    async fn spawn_agent_deleted_base_branch_returns_clear_error() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        run_git(&source, &["branch", "task-base"]);
        run_git(&source, &["branch", "-D", "task-base"]);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        let task_meta = sample_task_meta(Some("task-base"));

        let err = prepare_agent_worktree(
            gcx,
            &task_meta,
            "task-1",
            "agent-33333333",
            "T-1",
            "agent-T-1-33333333",
        )
        .await
        .unwrap_err();

        assert_eq!(
            err,
            "Task base branch 'task-base' no longer exists. Update the task base branch or create a new task on the current branch."
        );
    }

    #[tokio::test]
    async fn spawn_agent_warns_when_current_head_differs_from_task_base_branch() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        run_git(&source, &["checkout", "-b", "dev"]);
        commit_file(&source, "dev_only.txt", "only dev\n", "dev-only");
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        let task_meta = sample_task_meta(Some("main"));

        let prepared = prepare_agent_worktree(
            gcx.clone(),
            &task_meta,
            "task-1",
            "agent-44444444",
            "T-1",
            "agent-T-1-44444444",
        )
        .await
        .unwrap();

        let warning = prepared.base_branch_mismatch_warning.as_deref().unwrap();
        assert!(warning.contains("Current repo HEAD is on branch 'dev'"));
        assert!(warning.contains("this task was created from 'main'"));

        prepared.cleanup_unlinked(gcx).await;
    }

    #[test]
    fn spawn_agent_successful_spawn_sets_board_mirrors_and_thread_worktree() {
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
            "planner-task-1-1",
            worktree.clone(),
        );
        assert_eq!(thread.mode, "task_agent");
        assert_eq!(thread.task_meta.as_ref().unwrap().role, "agents");
        assert_eq!(
            thread
                .task_meta
                .as_ref()
                .unwrap()
                .planner_chat_id
                .as_deref(),
            Some("planner-task-1-1")
        );
        assert_eq!(thread.parent_id.as_deref(), Some("planner-task-1-1"));
        assert_eq!(thread.root_chat_id.as_deref(), Some("planner-task-1-1"));
        assert_eq!(thread.worktree.as_ref(), Some(&worktree));
        assert!(thread.worktree.as_ref().unwrap().enforce);
    }

    #[tokio::test]
    async fn spawn_agent_planner_chat_id_uses_active_root_before_legacy_meta() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut session = crate::chat::types::ChatSession::new("controller-planner".to_string());
        session.thread.task_meta = Some(refact_chat_api::TaskMeta {
            task_id: "task-1".to_string(),
            role: "planner".to_string(),
            agent_id: None,
            card_id: None,
            planner_chat_id: Some("controller-planner".to_string()),
        });
        gcx.chat_sessions.write().await.insert(
            "controller-planner".to_string(),
            Arc::new(tokio::sync::Mutex::new(session)),
        );
        let task_meta = refact_chat_api::TaskMeta {
            task_id: "task-1".to_string(),
            role: "subchats".to_string(),
            agent_id: None,
            card_id: None,
            planner_chat_id: Some("stale-planner".to_string()),
        };

        assert_eq!(
            resolve_invoking_planner_chat_id(
                gcx.clone(),
                "child-chat",
                "controller-planner",
                Some(&task_meta)
            )
            .await,
            "controller-planner"
        );
        assert_eq!(
            resolve_invoking_planner_chat_id(gcx, "child-chat", "", Some(&task_meta)).await,
            "stale-planner"
        );
    }

    #[tokio::test]
    async fn spawn_agent_planner_chat_id_ignores_non_planner_root() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let task_meta = refact_chat_api::TaskMeta {
            task_id: "task-1".to_string(),
            role: "planner".to_string(),
            agent_id: None,
            card_id: None,
            planner_chat_id: Some("planner-chat".to_string()),
        };

        assert_eq!(
            resolve_invoking_planner_chat_id(
                gcx,
                "planner-chat",
                "normal-source-chat",
                Some(&task_meta)
            )
            .await,
            "planner-chat"
        );
    }

    #[test]
    fn spawn_agent_rollback_restore_original_card_after_session_failure() {
        let mut board = TaskBoard::default();
        let original = test_card("T-1", "planned", None);
        let mut spawned = original.clone();
        mark_card_agent_started(
            &mut spawned,
            "agent-1",
            "agent-chat-1",
            Some("branch-new".to_string()),
            Some("/tmp/new-worktree".to_string()),
            Some("wt-new".to_string()),
        );
        board.cards.push(spawned);

        assert!(restore_original_card_if_current_agent(
            &mut board,
            &original,
            "agent-chat-1"
        ));

        let card = board.get_card("T-1").unwrap();
        assert_eq!(card.column, "planned");
        assert!(card.assignee.is_none());
        assert!(card.agent_chat_id.is_none());
        assert!(card.agent_worktree.is_none());
        assert!(card.agent_branch.is_none());
    }

    #[test]
    fn spawn_agent_rollback_guard_skips_different_agent() {
        let mut board = TaskBoard::default();
        let original = test_card("T-1", "planned", None);
        let mut spawned = original.clone();
        mark_card_agent_started(&mut spawned, "agent-2", "agent-chat-2", None, None, None);
        board.cards.push(spawned.clone());

        assert!(!restore_original_card_if_current_agent(
            &mut board,
            &original,
            "agent-chat-1"
        ));

        let card = board.get_card("T-1").unwrap();
        assert_eq!(card.agent_chat_id, spawned.agent_chat_id);
        assert_eq!(card.assignee, spawned.assignee);
        assert_eq!(card.column, "doing");
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

    #[test]
    fn worktree_scope_files_to_open_path_traversal_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let worktree = temp.path().join("worktree");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "secret").unwrap();
        std::fs::write(worktree.join("safe.txt"), "safe").unwrap();

        let worktree_canonical = dunce::canonicalize(&worktree).unwrap();

        let traversal = worktree.join("../outside/secret.txt");
        let traversal_canonical = dunce::canonicalize(&traversal).unwrap();
        assert!(
            !traversal_canonical.starts_with(&worktree_canonical),
            "traversal path should escape worktree: {:?} not under {:?}",
            traversal_canonical,
            worktree_canonical
        );

        let safe_canonical = dunce::canonicalize(worktree.join("safe.txt")).unwrap();
        assert!(
            safe_canonical.starts_with(&worktree_canonical),
            "in-worktree path should pass check: {:?} under {:?}",
            safe_canonical,
            worktree_canonical
        );
    }

    #[tokio::test]
    async fn spawn_agent_files_to_open_rejects_outside_traversal() {
        let temp = tempfile::tempdir().unwrap();
        let worktree = temp.path().join("worktree");
        let source = temp.path().join("source");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "secret").unwrap();
        let gcx = test_gcx_with_privacy(vec![]).await;

        let err = context_files_from_files_to_open(
            gcx,
            &worktree,
            &source,
            &["../outside/secret.txt".to_string()],
        )
        .await
        .unwrap_err();

        assert!(err.contains("outside agent worktree"), "{err}");
    }

    #[tokio::test]
    async fn spawn_agent_files_to_open_rejects_privacy_blocked_file() {
        let temp = tempfile::tempdir().unwrap();
        let worktree = temp.path().join("worktree");
        let source = temp.path().join("source");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(worktree.join(".env"), "SECRET=value\n").unwrap();
        let gcx = test_gcx_with_privacy(vec!["*.env".to_string()]).await;

        let err = context_files_from_files_to_open(gcx, &worktree, &source, &[".env".to_string()])
            .await
            .unwrap_err();

        assert!(err.contains("blocked by privacy settings"), "{err}");
        assert!(err.contains("Blocked"), "{err}");
    }

    #[tokio::test]
    async fn spawn_agent_files_to_open_valid_file_works() {
        let temp = tempfile::tempdir().unwrap();
        let worktree = temp.path().join("worktree");
        let source = temp.path().join("source");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(worktree.join("safe.txt"), "safe\ncontent\n").unwrap();
        let gcx = test_gcx_with_privacy(vec![]).await;

        let files =
            context_files_from_files_to_open(gcx, &worktree, &source, &["safe.txt".to_string()])
                .await
                .unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_content, "safe\ncontent\n");
        assert_eq!(files[0].line1, 1);
        assert_eq!(files[0].line2, 2);
        assert!(files[0].file_name.ends_with("safe.txt"));
    }

    #[tokio::test]
    async fn files_to_open_truncates_oversized_file() {
        let temp = tempfile::tempdir().unwrap();
        let worktree = temp.path().join("worktree");
        let source = temp.path().join("source");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(worktree.join("large.txt"), vec![b'a'; 1024 * 1024]).unwrap();
        let gcx = test_gcx_with_privacy(vec![]).await;

        let files =
            context_files_from_files_to_open(gcx, &worktree, &source, &["large.txt".to_string()])
                .await
                .unwrap();

        assert_eq!(files.len(), 1);
        assert!(files[0]
            .file_content
            .starts_with(&"a".repeat(FILES_TO_OPEN_PER_FILE_LIMIT)));
        assert!(files[0]
            .file_content
            .contains("... (truncated, 786432 more bytes)"));
        assert!(files[0].file_name.ends_with("large.txt"));
    }

    #[tokio::test]
    async fn files_to_open_drops_remaining_after_total_cap() {
        let temp = tempfile::tempdir().unwrap();
        let worktree = temp.path().join("worktree");
        let source = temp.path().join("source");
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::create_dir_all(&source).unwrap();
        let paths = (0..5)
            .map(|idx| {
                let name = format!("file-{idx}.txt");
                std::fs::write(worktree.join(&name), vec![b'a' + idx as u8; 256 * 1024]).unwrap();
                name
            })
            .collect::<Vec<_>>();
        let gcx = test_gcx_with_privacy(vec![]).await;

        let files = context_files_from_files_to_open(gcx, &worktree, &source, &paths)
            .await
            .unwrap();

        assert_eq!(files.len(), 5);
        for file in files.iter().take(4) {
            assert_eq!(file.file_content.len(), 256 * 1024);
            assert_eq!(file.line1, 1);
            assert_eq!(file.line2, 1);
        }
        assert!(files[4].file_name.ends_with("file-4.txt"));
        assert!(files[4].file_content.is_empty());
        assert_eq!(files[4].line1, 0);
        assert_eq!(files[4].line2, 0);
    }

    #[test]
    fn abandoned_worktree_detection_ignores_failed_cards_with_retained_worktrees() {
        let tempdir = tempfile::tempdir().unwrap();
        let worktree_path = tempdir.path().join("retained-failed-worktree");
        std::fs::create_dir_all(&worktree_path).unwrap();

        let mut board = TaskBoard::default();
        board.cards.push(test_card(
            "T-1",
            "failed",
            Some(worktree_path.to_string_lossy().to_string()),
        ));
        board.cards.push(test_card(
            "T-2",
            "done",
            Some(worktree_path.to_string_lossy().to_string()),
        ));

        let abandoned = find_abandoned_worktrees(&board);
        assert_eq!(abandoned.len(), 1, "only done card should be flagged");
        assert!(
            !abandoned[0].contains("T-1"),
            "failed card retained worktree should not block spawning"
        );
        assert!(
            abandoned[0].contains("T-2"),
            "done card with retained worktree should still be flagged"
        );
    }

    async fn setup_task_storage(
        gcx: Arc<GlobalContext>,
        source: &Path,
        task_id: &str,
        card: BoardCard,
    ) {
        let task_dir = source.join(".refact").join("tasks").join(task_id);
        tokio::fs::create_dir_all(task_dir.join("trajectories").join("planner"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(task_dir.join("trajectories").join("agents"))
            .await
            .unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let meta = TaskMeta {
            schema_version: 1,
            id: task_id.to_string(),
            name: "Test Task".to_string(),
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
        };
        storage::save_task_meta(gcx.clone(), task_id, &meta)
            .await
            .unwrap();
        storage::save_board(
            gcx.clone(),
            task_id,
            &TaskBoard {
                cards: vec![card],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn spawn_rollback_before_board_write_cleans_up_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        let task_meta = sample_task_meta(None);
        let prepared = prepare_agent_worktree(
            gcx.clone(),
            &task_meta,
            "task-1",
            "agent-abcdef12",
            "T-1",
            "agent-T-1-abcdef12",
        )
        .await
        .unwrap();
        let root = prepared.meta.root.clone();
        assert!(root.is_dir());

        prepared.cleanup_unlinked(gcx).await;

        assert!(!root.exists());
    }

    #[tokio::test]
    async fn spawn_rollback_after_board_write_retains_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;

        let task_id = "task-retain-test";
        let task_meta = sample_task_meta(None);
        let prepared = prepare_agent_worktree(
            gcx.clone(),
            &task_meta,
            task_id,
            "agent-abcdef12",
            "T-1",
            "agent-T-1-abcdef12",
        )
        .await
        .unwrap();
        let root = prepared.meta.root.clone();
        let branch = prepared.meta.branch.clone().unwrap();

        let mut card = test_card("T-1", "doing", Some(root.to_string_lossy().to_string()));
        card.agent_chat_id = Some("agent-T-1-abcdef12".to_string());
        setup_task_storage(gcx.clone(), &source, task_id, card).await;

        rollback_retain(
            gcx.clone(),
            task_id,
            "T-1",
            "agent-T-1-abcdef12",
            "model unavailable",
        )
        .await;
        prepared.retain(gcx.clone()).await;

        assert!(
            root.exists(),
            "worktree dir should still exist after retain"
        );
        let branches = run_git(&source, &["branch", "--list", &branch]);
        assert!(
            !branches.trim().is_empty(),
            "branch should still exist after retain"
        );

        let board = storage::load_board(gcx.clone(), task_id).await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert_eq!(card.column, "failed");
        assert!(
            card.status_updates
                .iter()
                .any(|u| u.message.contains("retained for inspection")),
            "status update should mention retained for inspection"
        );
    }

    #[tokio::test]
    async fn spawn_rollback_message_includes_error() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;

        let task_id = "task-msg-test";
        let mut card = test_card("T-2", "doing", None);
        card.agent_chat_id = Some("agent-T-2-msgtest".to_string());
        setup_task_storage(gcx.clone(), &source, task_id, card).await;

        let long_error: String = "x".repeat(400);
        rollback_retain(
            gcx.clone(),
            task_id,
            "T-2",
            "agent-T-2-msgtest",
            &long_error,
        )
        .await;

        let board = storage::load_board(gcx.clone(), task_id).await.unwrap();
        let card = board.get_card("T-2").unwrap();
        let msg = &card.status_updates.last().unwrap().message;
        assert!(
            msg.contains(&"x".repeat(300)),
            "message should include first 300 chars of error"
        );
        assert!(
            !msg.contains(&"x".repeat(301)),
            "message should not include more than 300 chars of error"
        );
        assert!(
            msg.contains("Worktree retained for inspection"),
            "message should contain retention notice"
        );
        assert!(
            msg.contains("Worktree retained for inspection"),
            "message should contain retention notice"
        );
    }

    #[test]
    fn files_to_open_rejects_non_string_entry() {
        use serde_json::json;
        let args = HashMap::from([(
            "files_to_open".to_string(),
            json!([{"path": "/foo"}, "/bar"]),
        )]);
        let err = validate_files_to_open(&args).unwrap_err();
        assert!(err.contains("files_to_open[0]"), "{err}");
        assert!(err.contains("must be a string"), "{err}");
        assert!(err.contains("object"), "{err}");
    }

    #[test]
    fn files_to_open_rejects_empty_string() {
        use serde_json::json;
        let args = HashMap::from([(
            "files_to_open".to_string(),
            json!(["valid.txt", "", "other.txt"]),
        )]);
        let err = validate_files_to_open(&args).unwrap_err();
        assert!(err.contains("files_to_open[1]"), "{err}");
        assert!(err.contains("is empty"), "{err}");
    }

    #[test]
    fn files_to_open_dedupes_with_warning() {
        use serde_json::json;
        let args = HashMap::from([(
            "files_to_open".to_string(),
            json!(["/foo/bar.txt", "/baz.txt", "/foo/bar.txt"]),
        )]);
        let paths = validate_files_to_open(&args).unwrap();
        assert_eq!(paths, vec!["/foo/bar.txt", "/baz.txt"]);
    }
}
