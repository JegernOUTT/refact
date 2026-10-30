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
use crate::tasks::assignment::{self, Assignment};
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
    source: AcceptedAgentSource,
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
        if let Err(error) = self.source.validate(gcx.clone()).await {
            tracing::warn!(%error, "Unlinked task-agent checkout retained; reconciliation required");
            return;
        }
        let cache_dir = gcx.cache_dir.clone();
        if let Ok(service) =
            WorktreeService::new_async(cache_dir, self.meta.source_workspace_root.clone()).await
        {
            match service.validate_worktree_meta_strict(&self.meta).await {
                Ok(meta) if meta == self.meta => {
                    if service
                        .delete_worktree(&self.meta.id, self.branch_was_created, true)
                        .await
                        .is_ok()
                    {
                        return;
                    }
                }
                _ => {}
            }
        }
        tracing::warn!(worktree = %self.meta.id, "Unlinked task-agent checkout retained; cleanup identity could not be verified");
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

fn optional_nonempty_string_arg(
    args: &HashMap<String, Value>,
    name: &str,
) -> Result<Option<String>, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() => {
            Ok(Some(value.trim().to_string()))
        }
        Some(Value::String(_)) => Err(format!("'{}' must be a non-empty string", name)),
        Some(_) => Err(format!("'{}' must be a string", name)),
    }
}

fn git_base_at(path: &Path) -> Result<(Option<String>, String), String> {
    let repo = git::discover_repo(path)?;
    Ok((git::current_branch(&repo), git::head_commit(&repo)?))
}

#[derive(Clone, Debug)]
pub(crate) struct AcceptedAgentSource {
    root: PathBuf,
    project: String,
    git_dir: PathBuf,
    common_dir: PathBuf,
    common_identity: std::fs::Metadata,
    root_identity: std::fs::Metadata,
    git_identity: std::fs::Metadata,
    planner_worktree: Option<WorktreeMeta>,
    task_dir: Option<storage::TaskDirectoryBinding>,
    attempt: Option<storage::AcceptedTaskAttempt>,
}

impl AcceptedAgentSource {
    pub(crate) async fn capture(
        gcx: Arc<GlobalContext>,
        root: &Path,
        planner_worktree: Option<&WorktreeMeta>,
    ) -> Result<Self, String> {
        let root = dunce::canonicalize(root)
            .map_err(|e| format!("Accepted task-agent source is unavailable: {e}"))?;
        let repo = git::discover_repo(&root)?;
        let git_dir = dunce::canonicalize(repo.path()).map_err(|e| e.to_string())?;
        let common_dir = dunce::canonicalize(repo.commondir()).map_err(|e| e.to_string())?;
        drop(repo);
        let source = Self {
            project: crate::coordination::roots::project_identity(&root),
            root_identity: std::fs::metadata(&root).map_err(|e| e.to_string())?,
            git_identity: std::fs::metadata(&git_dir).map_err(|e| e.to_string())?,
            common_identity: std::fs::metadata(&common_dir).map_err(|e| e.to_string())?,
            common_dir,
            root,
            git_dir,
            planner_worktree: planner_worktree.cloned(),
            task_dir: None,
            attempt: None,
        };
        source.validate(gcx).await?;
        Ok(source)
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn project(&self) -> &str {
        &self.project
    }

    pub(crate) async fn bind_task(
        mut self,
        gcx: Arc<GlobalContext>,
        task_id: &str,
    ) -> Result<Self, String> {
        self.task_dir =
            Some(storage::TaskDirectoryBinding::capture(gcx, &self.root, task_id).await?);
        Ok(self)
    }

    pub(crate) fn task_binding(
        &self,
        task_id: &str,
    ) -> Result<&storage::TaskDirectoryBinding, String> {
        let binding = self
            .task_dir
            .as_ref()
            .ok_or("Accepted task directory is not bound")?;
        binding.validate_task(task_id)?;
        Ok(binding)
    }

    pub(crate) fn with_attempt(
        mut self,
        task_id: &str,
        original: &BoardCard,
        agent_id: &str,
        chat_id: &str,
        variant: Option<&'static str>,
    ) -> Result<Self, String> {
        let attempt = crate::tasks::admission::current_attempt(chat_id)
            .ok_or("Accepted task admission attempt unavailable")?;
        let binding = self
            .task_dir
            .as_ref()
            .filter(|binding| binding.task_id() == task_id)
            .ok_or("Accepted task directory is not bound")?;
        let binding = binding.with_attempt(&attempt, &original.id, agent_id, variant);
        self.attempt = Some(storage::AcceptedTaskAttempt::new(
            &binding, original, agent_id, attempt, variant,
        ));
        self.task_dir = Some(binding);
        Ok(self)
    }

    pub(crate) async fn reconcile(&self, gcx: Arc<GlobalContext>, restore: bool, error: &str) {
        let result = match &self.attempt {
            Some(attempt) => attempt.reconcile(gcx, restore, error).await,
            None => Err("Accepted task attempt unavailable".into()),
        };
        if let Err(binding_error) = result {
            tracing::warn!(%binding_error, "Task claim retained; reconciliation required");
        }
    }

    pub(crate) async fn validate_task(
        &self,
        gcx: Arc<GlobalContext>,
        task_id: &str,
    ) -> Result<(), String> {
        self.validate(gcx).await?;
        self.task_binding(task_id).map(|_| ())
    }

    pub(crate) async fn validate(&self, gcx: Arc<GlobalContext>) -> Result<(), String> {
        let current = crate::coordination::roots::resolve_source_workspace_root(
            gcx.clone(),
            Some(&self.root.to_string_lossy()),
        )
        .await
        .map_err(|e| format!("Accepted task-agent source is no longer authorized: {e}"))?;
        let repo = git::discover_repo(&current)?;
        let git_dir = dunce::canonicalize(repo.path()).map_err(|e| e.to_string())?;
        let common_dir = dunce::canonicalize(repo.commondir()).map_err(|e| e.to_string())?;
        drop(repo);
        if current != self.root
            || crate::coordination::roots::project_identity(&current) != self.project
            || git_dir != self.git_dir
            || common_dir != self.common_dir
            || !storage::same_directory_identity(&self.common_identity, &common_dir)?
            || !storage::same_directory_identity(&self.root_identity, &current)?
            || !storage::same_directory_identity(&self.git_identity, &git_dir)?
        {
            return Err("Accepted task-agent source repository changed; retry the launch".into());
        }
        if let Some(planner) = &self.planner_worktree {
            let service =
                WorktreeService::new_async(gcx.cache_dir.clone(), self.root.clone()).await?;
            let registered = service.validate_worktree_meta_strict(planner).await?;
            if &registered != planner {
                return Err(
                    "Accepted planner worktree registration changed; retry the launch".into(),
                );
            }
        }
        Ok(())
    }
}

fn resolve_spawn_base(
    explicit_branch: Option<String>,
    explicit_commit: Option<String>,
    planner_worktree: Option<&WorktreeMeta>,
    workspace_root: &Path,
    task_meta: &StoredTaskMeta,
) -> (Option<String>, Option<String>) {
    if explicit_branch.is_some() || explicit_commit.is_some() {
        let fallback = planner_worktree
            .and_then(|worktree| git_base_at(&worktree.root).ok())
            .or_else(|| git_base_at(workspace_root).ok());
        return (
            explicit_branch
                .or_else(|| fallback.as_ref().and_then(|(branch, _)| branch.clone()))
                .or_else(|| task_meta.base_branch.clone()),
            explicit_commit,
        );
    }
    if let Some(base) = planner_worktree.and_then(|worktree| git_base_at(&worktree.root).ok()) {
        return (base.0, Some(base.1));
    }
    if let Ok((branch, commit)) = git_base_at(workspace_root) {
        return (branch, Some(commit));
    }
    (task_meta.base_branch.clone(), task_meta.base_commit.clone())
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

fn retained_worktree_cards(board: &crate::tasks::types::TaskBoard) -> Vec<&BoardCard> {
    board
        .cards
        .iter()
        .filter(|card| card.column != "doing" && card.column != "failed")
        .filter(|card| {
            card.agent_worktree
                .as_deref()
                .is_some_and(|worktree| Path::new(worktree).exists())
        })
        .collect()
}

const SPAWN_GATE_MAX_CHANGED_FILES: usize = 4096;

fn launch_base_commit(
    root: &Path,
    base_branch: Option<&str>,
    base_commit: Option<&str>,
) -> Result<String, String> {
    let repo = git::discover_repo(root)?;
    match (base_commit, base_branch) {
        (Some(commit), _) => git::commit_for_ref(&repo, commit),
        (None, Some(branch)) => git::commit_for_branch(&repo, branch),
        (None, None) => git::head_commit(&repo),
    }
}

fn retained_checkout_changes(
    root: &Path,
    card: &BoardCard,
    branch: &str,
) -> Result<(String, Vec<PathBuf>), String> {
    let source = git::discover_repo(root)?;
    let worktree = dunce::canonicalize(card.agent_worktree.as_deref().ok_or("Missing worktree")?)
        .map_err(|error| error.to_string())?;
    let repo = git2::Repository::open(&worktree).map_err(|error| error.to_string())?;
    if dunce::canonicalize(git::repo_root(&repo)?).map_err(|error| error.to_string())? != worktree
        || git::canonical_common_dir(&repo)? != git::canonical_common_dir(&source)?
    {
        return Err("Retained checkout repository identity changed".into());
    }
    let head = repo.head().map_err(|error| error.to_string())?;
    let commit = head.peel_to_commit().map_err(|error| error.to_string())?;
    if !head.is_branch()
        || head.name() != Some(format!("refs/heads/{branch}").as_str())
        || git::commit_for_branch(&source, branch)? != commit.id().to_string()
        || repo.state() != git2::RepositoryState::Clean
    {
        return Err("Retained checkout branch or HEAD changed".into());
    }
    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false)
        .exclude_submodules(false);
    let statuses = repo
        .statuses(Some(&mut options))
        .map_err(|error| error.to_string())?;
    let mut paths = std::collections::BTreeSet::new();
    for status in statuses.iter() {
        if status.status() == git2::Status::CURRENT {
            continue;
        }
        let mut found = false;
        for delta in [status.head_to_index(), status.index_to_workdir()]
            .into_iter()
            .flatten()
        {
            for path in [delta.old_file().path(), delta.new_file().path()]
                .into_iter()
                .flatten()
            {
                paths.insert(path.to_path_buf());
                found = true;
            }
        }
        if !found || paths.len() > SPAWN_GATE_MAX_CHANGED_FILES {
            return Err("Retained checkout dirty inventory is unavailable or too large".into());
        }
    }
    if git::head_commit(&repo)? != commit.id().to_string()
        || git::current_branch(&repo).as_deref() != Some(branch)
    {
        return Err("Retained checkout HEAD changed during inspection".into());
    }
    Ok((commit.id().to_string(), paths.into_iter().collect()))
}

fn branch_changes(
    root: &Path,
    branch: &str,
    base: &str,
) -> Result<(bool, Vec<PathBuf>), git2::Error> {
    let repo = git2::Repository::discover(root)?;
    let branch = repo.revparse_single(branch)?.peel_to_commit()?;
    let base = repo.revparse_single(base)?.peel_to_commit()?;
    if branch.id() == base.id() || repo.graph_descendant_of(base.id(), branch.id())? {
        return Ok((true, Vec::new()));
    }
    let ancestor = repo
        .find_commit(repo.merge_base(base.id(), branch.id())?)?
        .tree()?;
    let branch = branch.tree()?;
    let base = base.tree()?;
    let mut paths = std::collections::BTreeSet::new();
    repo.diff_tree_to_tree(Some(&ancestor), Some(&branch), None)?
        .foreach(
            &mut |delta, _| {
                for path in [delta.old_file().path(), delta.new_file().path()]
                    .into_iter()
                    .flatten()
                {
                    paths.insert(path.to_path_buf());
                }
                paths.len() <= SPAWN_GATE_MAX_CHANGED_FILES
            },
            None,
            None,
            None,
        )?;
    let entry = |tree: &git2::Tree<'_>, path: &Path| match tree.get_path(path) {
        Ok(entry) => Ok(Some((entry.id(), entry.filemode()))),
        Err(error) if error.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(error) => Err(error),
    };
    let mut merged = true;
    for path in &paths {
        if entry(&base, path)? != entry(&branch, path)? {
            merged = false;
            break;
        }
    }
    Ok((merged, paths.into_iter().collect()))
}

fn spawn_gate_path(root: &Path, worktree: Option<&str>, path: &Path) -> PathBuf {
    let relative = path
        .strip_prefix(root)
        .ok()
        .or_else(|| worktree.and_then(|worktree| path.strip_prefix(worktree).ok()));
    let path = relative.unwrap_or(path);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn find_abandoned_worktrees(
    board: &crate::tasks::types::TaskBoard,
    card_id: &str,
    root: &Path,
    base: &str,
    launch_base: &str,
    action: &str,
) -> Result<Option<String>, String> {
    let card = board
        .get_card(card_id)
        .ok_or_else(|| format!("Card {card_id} not found"))?;
    let mut dependencies = std::collections::HashSet::new();
    let mut pending = card.depends_on.clone();
    while let Some(id) = pending.pop() {
        if dependencies.insert(id.clone()) {
            if let Some(dependency) = board.get_card(&id) {
                pending.extend(dependency.depends_on.iter().cloned());
            }
        }
    }
    let targets: Vec<_> = card
        .target_files
        .iter()
        .filter(|path| !path.trim().is_empty())
        .map(|path| spawn_gate_path(root, card.agent_worktree.as_deref(), Path::new(path)))
        .collect();
    let mut blockers = Vec::new();
    let mut unmerged = Vec::new();
    let mut included = Vec::new();
    for other in retained_worktree_cards(board) {
        if other.id == card_id {
            continue;
        }
        let branch = other.agent_branch.as_deref().unwrap_or("<unknown branch>");
        let checkout = retained_checkout_changes(root, other, branch);
        let clean = checkout.as_ref().is_ok_and(|(_, paths)| paths.is_empty());
        let head = checkout
            .as_ref()
            .map(|(head, _)| head.as_str())
            .unwrap_or(branch);
        let changes = branch_changes(root, head, base);
        if clean && changes.as_ref().is_ok_and(|(merged, _)| *merged) {
            continue;
        }
        let label = format!("{} (`{branch}`)", other.id);
        if clean && branch_changes(root, head, launch_base).is_ok_and(|(merged, _)| merged) {
            included.push(label);
            continue;
        }
        let mut paths = changes
            .map(|(_, paths)| paths)
            .unwrap_or_else(|_| other.target_files.iter().map(PathBuf::from).collect());
        match checkout {
            Ok((_, dirty_paths)) => paths.extend(dirty_paths),
            Err(_) => paths.extend(other.target_files.iter().map(PathBuf::from)),
        }
        let overlap = paths.iter().find(|path| {
            let path = spawn_gate_path(root, other.agent_worktree.as_deref(), path);
            targets
                .iter()
                .any(|target| path.starts_with(target) || target.starts_with(&path))
        });
        let mut reasons = Vec::new();
        if dependencies.contains(&other.id) {
            reasons.push("dependency (direct or transitive)".to_string());
        }
        if let Some(path) = overlap {
            reasons.push(format!("file overlap: {}", path.display()));
        }
        if !reasons.is_empty() {
            blockers.push(format!("- {label}: {}", reasons.join("; ")));
        }
        unmerged.push(label);
    }
    if !blockers.is_empty() {
        return Err(abandoned_worktrees_error(action, &blockers));
    }
    let mut warnings = Vec::new();
    if !unmerged.is_empty() {
        warnings.push(format!(
            "Warning: unmerged task work remains: {}. Merge these cards when verification is complete.",
            unmerged.join(", ")
        ));
    }
    if !included.is_empty() {
        warnings.push(format!(
            "Advisory: retained task source {} is included in requested launch base `{launch_base}`, not merged into target `{base}`. Checkouts and branches are retained; inclusion is not verification or main landing.",
            included.join(", ")
        ));
    }
    Ok((!warnings.is_empty()).then(|| warnings.join(" ")))
}

pub(crate) async fn scoped_worktree_gate(
    board: &crate::tasks::types::TaskBoard,
    card_id: &str,
    root: &Path,
    base_branch: Option<&str>,
    base_commit: Option<&str>,
    action: &str,
) -> Result<Option<String>, String> {
    let board = board.clone();
    let card_id = card_id.to_string();
    let root = root.to_path_buf();
    let base_branch = base_branch.map(str::to_string);
    let base_commit = base_commit.map(str::to_string);
    let action = action.to_string();
    tokio::task::spawn_blocking(move || {
        let resolved_branch = base_branch.as_deref().filter(|branch| {
            git::discover_repo(&root)
                .and_then(|repo| git::commit_for_branch(&repo, branch))
                .is_ok()
        });
        let base = resolved_branch.or(base_commit.as_deref()).unwrap_or("HEAD");
        let launch_base = launch_base_commit(&root, base_branch.as_deref(), base_commit.as_deref())
            .map_err(|error| {
                format!("Cannot {action}: requested launch base is unavailable: {error}")
            })?;
        let note = resolved_branch.is_none().then(|| {
            let kind = if base_commit.is_some() {
                "pinned base commit"
            } else {
                "workspace HEAD"
            };
            format!(
                "Base branch `{}` is unavailable; merged-work recognition used {kind} `{base}`.",
                base_branch.as_deref().unwrap_or("not recorded")
            )
        });
        match find_abandoned_worktrees(&board, &card_id, &root, base, &launch_base, &action) {
            Ok(Some(warning)) => Ok(Some(match note {
                Some(note) => format!("{warning} {note}"),
                None => warning,
            })),
            Err(error) => Err(match note {
                Some(note) => format!("{error}\n{note}"),
                None => error,
            }),
            result => result,
        }
    })
    .await
    .map_err(|error| format!("Task worktree spawn check failed: {error}"))?
}

fn abandoned_worktrees_error(action: &str, blockers: &[String]) -> String {
    format!(
        "Cannot {action}: unmerged task work blocks this card. Merge the blocking cards with `merge_agent(card_id=...)` or discard unwanted done/failed worktrees with `cancel_agent(card_id=..., discard_worktree=true)`, then retry:\n{}",
        blockers.join("\n")
    )
}

pub(crate) async fn prepare_agent_worktree(
    gcx: Arc<GlobalContext>,
    source: &AcceptedAgentSource,
    task_meta: &StoredTaskMeta,
    task_id: &str,
    agent_id: &str,
    card_id: &str,
    agent_chat_id: &str,
) -> Result<PreparedWorktree, String> {
    prepare_agent_worktree_with_suffix(
        gcx,
        source,
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
    source: &AcceptedAgentSource,
    task_meta: &StoredTaskMeta,
    task_id: &str,
    agent_id: &str,
    card_id: &str,
    agent_chat_id: &str,
    branch_suffix: Option<&str>,
) -> Result<PreparedWorktree, String> {
    prepare_agent_worktree_with_base_and_suffix(
        gcx,
        source,
        task_meta,
        task_id,
        agent_id,
        card_id,
        agent_chat_id,
        task_meta.base_branch.clone(),
        task_meta.base_commit.clone(),
        branch_suffix,
    )
    .await
}

async fn prepare_agent_worktree_with_base_and_suffix(
    gcx: Arc<GlobalContext>,
    source: &AcceptedAgentSource,
    task_meta: &StoredTaskMeta,
    task_id: &str,
    agent_id: &str,
    card_id: &str,
    agent_chat_id: &str,
    base_branch: Option<String>,
    base_commit: Option<String>,
    branch_suffix: Option<&str>,
) -> Result<PreparedWorktree, String> {
    source.validate(gcx.clone()).await?;
    let workspace_root = source.root().to_path_buf();
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
    let task_base_branch = base_branch;
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
    source.validate(gcx.clone()).await?;
    let created = service
        .create_worktree(CreateWorktreeRequest {
            source_workspace_root: Some(workspace_root.to_string_lossy().to_string()),
            branch: Some(branch_name),
            base_branch: task_base_branch,
            base_commit,
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
        source: source.clone(),
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

async fn run_queued_board_agent(
    gcx: Arc<GlobalContext>,
    app: crate::app_state::AppState,
    task_id: String,
    card_id: String,
    agent_id: String,
    agent_chat_id: String,
    planner_chat_id: String,
    task_meta: StoredTaskMeta,
    source: AcceptedAgentSource,
    base_branch: Option<String>,
    base_commit: Option<String>,
    files_to_open: Vec<String>,
    suggested_steps: usize,
    model: String,
    starting_new_run: bool,
) -> Result<(), String> {
    crate::tasks::admission::wait_for_permit(&agent_chat_id)
        .await
        .map_err(|e| e.to_string())?;
    let execution = app
        .agent_executions
        .get(&agent_chat_id)
        .ok_or("board setup admission lost")?;
    let _operation = execution
        .begin_operation()
        .await
        .map_err(|e| e.to_string())?;
    source.validate(gcx.clone()).await?;
    source.validate_task(gcx.clone(), &task_id).await?;
    let board = storage::load_board(gcx.clone(), &task_id).await?;
    if !board.get_card(&card_id).is_some_and(|card| {
        !execution.is_cancelled()
            && card.column == "doing"
            && card.assignee.as_deref() == Some(&agent_id)
            && card.agent_chat_id.as_deref() == Some(&agent_chat_id)
    }) {
        return Err("queued board claim was cancelled or replaced".into());
    }
    scoped_worktree_gate(
        &board,
        &card_id,
        source.root(),
        base_branch.as_deref(),
        base_commit.as_deref(),
        "spawn a new task agent",
    )
    .await?;
    let prepared_worktree = match prepare_agent_worktree_with_base_and_suffix(
        gcx.clone(),
        &source,
        &task_meta,
        &task_id,
        &agent_id,
        &card_id,
        &agent_chat_id,
        base_branch,
        base_commit,
        None,
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(e) => {
            crate::tasks::admission::end(&agent_chat_id).await;
            return Err(e);
        }
    };

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
                crate::tasks::admission::end(&agent_chat_id).await;
                prepared_worktree.cleanup_unlinked(gcx.clone()).await;
                return Err(e);
            }
        }
    };
    let initial_context_message = if initial_context_files.is_empty() {
        None
    } else {
        match context_file_message(&gcx, initial_context_files) {
            Ok(message) => Some(message),
            Err(e) => {
                crate::tasks::admission::end(&agent_chat_id).await;
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

    if let Err(error) = source.validate_task(gcx.clone(), &task_id).await {
        prepared_worktree.cleanup_unlinked(gcx.clone()).await;
        return Err(error);
    }
    let link_execution = execution.clone();
    let board_result = storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
            let card = board.get_card_mut(&card_id_owned).ok_or("queued card removed")?;
            if link_execution.is_cancelled() || card.agent_chat_id.as_deref() != Some(&agent_chat_id_clone) || card.column != "doing" {
                return Err("queued claim was cancelled or replaced".into());
            }
            mark_card_agent_started(card, &agent_id_clone, &agent_chat_id_clone,
                worktree_branch, worktree_path_str, worktree_name);
            card.base_branch = base_branch_from_prep;
            card.base_commit = base_commit_from_prep;
            if dirty_tree_warning {
                card.status_updates.push(StatusUpdate { timestamp: Utc::now().to_rfc3339(), message: "Agent starts from the committed base; local uncommitted changes are not included.".into() });
            }
            if let Some(warning) = base_branch_mismatch_warning {
                card.status_updates.push(StatusUpdate { timestamp: Utc::now().to_rfc3339(), message: warning });
            }
            Ok(())
        }).await;
    let (board, _) = match board_result {
        Ok(result) => result,
        Err(error) => {
            prepared_worktree.cleanup_unlinked(gcx.clone()).await;
            return Err(error);
        }
    };
    if let Err(e) = storage::update_task_stats(gcx.clone(), &task_id).await {
        rollback_retain(gcx.clone(), &source, &task_id, &card_id, &agent_chat_id, &e).await;
        crate::tasks::admission::end(&agent_chat_id).await;
        prepared_worktree.retain(gcx.clone()).await;
        return Err(e);
    }

    let mut meta = match storage::load_task_meta(gcx.clone(), &task_id).await {
        Ok(m) => m,
        Err(e) => {
            rollback_retain(gcx.clone(), &source, &task_id, &card_id, &agent_chat_id, &e).await;
            crate::tasks::admission::end(&agent_chat_id).await;
            prepared_worktree.retain(gcx.clone()).await;
            return Err(e);
        }
    };
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
    source.validate_task(gcx.clone(), &task_id).await?;
    if let Err(e) = storage::save_task_meta(gcx.clone(), &task_id, &meta).await {
        rollback_retain(gcx.clone(), &source, &task_id, &card_id, &agent_chat_id, &e).await;
        prepared_worktree.retain(gcx.clone()).await;
        return Err(e);
    }

    let card_title = board
        .get_card(&card_id)
        .map(|c| c.title.clone())
        .unwrap_or_default();
    let dependency_context = board
        .get_dependency_reports(&card_id)
        .into_iter()
        .map(|(title, report)| format!("### {}\n{}", title, report))
        .collect::<Vec<_>>()
        .join("\n\n");
    let assignment = board
        .get_card(&card_id)
        .map(Assignment::from_card)
        .unwrap_or_else(|| Assignment::new(&card_id, &card_title, String::new()))
        .with_dependency_context(dependency_context)
        .with_suggested_steps(suggested_steps);
    source.validate_task(gcx.clone(), &task_id).await?;
    assignment::record_assignment(gcx.clone(), &task_id, &assignment).await;

    let mut thread = build_agent_thread_params(
        &agent_chat_id,
        &card_title,
        &model,
        &task_id,
        &agent_id,
        &card_id,
        &planner_chat_id,
        prepared_worktree.meta.clone(),
    );
    thread.auto_approve_dangerous_commands =
        crate::agents::approval::dangerous_confirmation_enabled(&app, &planner_chat_id).await;
    let user_prompt = assignment.render_prompt();
    let user_msg = event(
        EventSubkind::SystemNotice,
        "tool.task_spawn_agent",
        json!({
            "task_id": &task_id,
            "planner_chat_id": &planner_chat_id,
            "card_id": &card_id,
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

    if let Some(context_message) = initial_context_message {
        messages.push(context_message);
    }

    let session_result = async {
        source.validate(gcx.clone()).await?;
        source.validate_task(gcx.clone(), &task_id).await?;
        if execution.is_cancelled() {
            return Err("board setup cancelled".to_string());
        }
        app.chat
            .facade
            .create_session(CreateSessionRequest {
                chat_id: agent_chat_id.clone(),
                thread,
                messages,
            })
            .await?;
        app.chat.facade.maybe_save_session(&agent_chat_id).await?;
        if execution.is_cancelled() {
            return Err("board setup cancelled".to_string());
        }
        app.chat
            .facade
            .push_priority_command(&agent_chat_id, ChatCommand::Regenerate {})
            .await
    }
    .await;
    if let Err(e) = session_result {
        rollback_retain(gcx.clone(), &source, &task_id, &card_id, &agent_chat_id, &e).await;
        crate::tasks::admission::end(&agent_chat_id).await;
        prepared_worktree.retain(gcx.clone()).await;
        return Err(e);
    }

    Ok(())
}

pub struct ToolTaskSpawnAgent;

impl ToolTaskSpawnAgent {
    pub fn new() -> Self {
        Self
    }
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
    source: &AcceptedAgentSource,
    task_id: &str,
    card_id: &str,
    guard_chat_id: &str,
    error_msg: &str,
) {
    let _ = (task_id, card_id, guard_chat_id);
    source.reconcile(gcx, false, error_msg).await;
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
        auto_compression_cap: None,
        auto_compression_cap_pending: true,
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
        crate::files_in_workspace::check_file_privacy_for_model_context(
            gcx.clone(),
            &canonical_resolved,
        )
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

struct SpawnRequest {
    ccx: Arc<AMutex<AtCommandsContext>>,
    gcx: Arc<GlobalContext>,
    current_model: String,
    task_id: String,
    root_chat_id: String,
    planner_worktree: Option<WorktreeMeta>,
    planner_chat_id: String,
    card_id: String,
    files_to_open: Vec<String>,
    suggested_steps: usize,
    tool_call_id: String,
}

struct PreparedSpawn {
    request: SpawnRequest,
    task_meta: StoredTaskMeta,
    base_branch: Option<String>,
    base_commit: Option<String>,
    model: String,
    agent_id: String,
    agent_chat_id: String,
    source: AcceptedAgentSource,
    app: crate::app_state::AppState,
    original_card: BoardCard,
    worktree_warning: Option<String>,
}

struct PersistedSpawn {
    acceptance: tokio::sync::oneshot::Receiver<Result<(), String>>,
    response: Value,
    tool_call_id: String,
}

async fn resolve_spawn_request(
    ccx: Arc<AMutex<AtCommandsContext>>,
    tool_call_id: &str,
    args: &HashMap<String, Value>,
) -> Result<SpawnRequest, String> {
    let ccx_lock = ccx.lock().await;
    let is_planner = ccx_lock
        .task_meta
        .as_ref()
        .map(|meta| meta.role == "planner")
        .unwrap_or(false);
    if !is_planner {
        return Err("spawn_agent can only be called by the task planner. \
                 Switch to the planner chat to spawn agents."
            .to_string());
    }
    drop(ccx_lock);

    let (
        gcx,
        current_model,
        task_id,
        chat_id,
        root_chat_id,
        task_meta_from_context,
        planner_worktree,
    ) = {
        let ccx_lock = ccx.lock().await;
        let task_id = ccx_lock
            .task_meta
            .as_ref()
            .map(|meta| meta.task_id.clone())
            .unwrap_or_else(|| {
                args.get("task_id")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string())
                    .unwrap_or_default()
            });
        (
            ccx_lock.app.gcx.clone(),
            ccx_lock.current_model.clone(),
            task_id,
            ccx_lock.chat_id.clone(),
            ccx_lock.root_chat_id.clone(),
            ccx_lock.task_meta.clone(),
            ccx_lock
                .execution_scope
                .as_ref()
                .map(|scope| scope.worktree().clone()),
        )
    };
    let planner_chat_id = resolve_invoking_planner_chat_id(
        gcx.clone(),
        &chat_id,
        &root_chat_id,
        task_meta_from_context.as_ref(),
    )
    .await;
    let task_id = if task_id.is_empty() {
        get_task_id(&ccx, args).await?
    } else {
        task_id
    };
    let card_id = args
        .get("card_id")
        .and_then(|value| value.as_str())
        .ok_or("Missing 'card_id'")?
        .to_string();
    let files_to_open = validate_files_to_open(args)?;
    let suggested_steps = match args
        .get("suggested_steps")
        .or_else(|| args.get("max_steps"))
    {
        Some(Value::String(value)) => value.parse().unwrap_or(30),
        Some(Value::Number(value)) => value.as_u64().unwrap_or(30) as usize,
        _ => 30,
    }
    .clamp(1, 50);

    Ok(SpawnRequest {
        ccx,
        gcx,
        current_model,
        task_id,
        root_chat_id,
        planner_worktree,
        planner_chat_id,
        card_id,
        files_to_open,
        suggested_steps,
        tool_call_id: tool_call_id.to_string(),
    })
}

async fn prepare_spawn(
    request: SpawnRequest,
    args: &HashMap<String, Value>,
) -> Result<PreparedSpawn, String> {
    let project_dirs = crate::files_correction::get_project_dirs(request.gcx.clone()).await;
    let workspace_root = project_dirs.first().ok_or_else(|| {
        "No workspace folder found; task agents require an isolated git worktree".to_string()
    })?;
    let workspace_root = request
        .planner_worktree
        .as_ref()
        .map(|worktree| worktree.source_workspace_root.clone())
        .unwrap_or_else(|| workspace_root.clone());
    let source = AcceptedAgentSource::capture(
        request.gcx.clone(),
        &workspace_root,
        request.planner_worktree.as_ref(),
    )
    .await?
    .bind_task(request.gcx.clone(), &request.task_id)
    .await?;
    let binding = source.task_binding(&request.task_id)?.clone();
    binding.scope(async {
    let task_meta = storage::load_task_meta(request.gcx.clone(), &request.task_id).await?;
    let explicit_base_branch = optional_nonempty_string_arg(args, "base_branch")?;
    let explicit_base_commit = optional_nonempty_string_arg(args, "base_commit")?;
    let model = resolve_agent_model(request.gcx.clone(), &request.current_model).await?;
    crate::tools::task_tool_helpers::preflight_agent_model(request.gcx.clone(), &model).await?;

    validate_id(&request.task_id, "task_id")?;
    validate_id(&request.card_id, "card_id")?;

    let board = storage::load_board(request.gcx.clone(), &request.task_id).await?;
    let card = board
        .get_card(&request.card_id)
        .ok_or_else(|| format!("Card {} not found", request.card_id))?;
    if card.column == "done" {
        return Err(format!("Card {} is already done", request.card_id));
    }
    if card.column == "failed" {
        return Err(format!(
            "Card {} has failed. Reset it first if you want to retry.",
            request.card_id
        ));
    }
    if card.column != "planned" && card.column != "doing" {
        return Err(format!(
            "Card {} is in column '{}', expected 'planned' or 'doing'",
            request.card_id, card.column
        ));
    }
    if card.column == "doing" && card.agent_chat_id.is_some() {
        return Err(format!(
            "Card {} already has an active agent ({}). Use check_agents to monitor it, or move the card back to 'planned' to respawn.",
            request.card_id,
            card.agent_chat_id.as_ref().unwrap()
        ));
    }
    board
        .dependency_gate(&request.card_id)
        .map_err(storage::violation_error)?;

    let agent_id = Uuid::new_v4().to_string();
    let agent_chat_id = format!("agent-{}-{}", request.card_id, &agent_id[..8]);
    let (base_branch, base_commit) = resolve_spawn_base(
        explicit_base_branch.clone(),
        explicit_base_commit.clone(),
        request.planner_worktree.as_ref(),
        source.root(),
        &task_meta,
    );
    let base_commit = Some(launch_base_commit(
        source.root(), base_branch.as_deref(), base_commit.as_deref(),
    )?);
    let worktree_warning = scoped_worktree_gate(
        &board,
        &request.card_id,
        source.root(),
        base_branch.as_deref(),
        base_commit.as_deref(),
        "spawn a new task agent",
    )
    .await?;
    let (app, depth) = {
        let ccx = request.ccx.lock().await;
        (ccx.app.clone(), ccx.subchat_depth)
    };
    let original_card = card.clone();
    let root = if request.root_chat_id.is_empty() {
        request.planner_chat_id.as_str()
    } else {
        request.root_chat_id.as_str()
    };
    crate::tasks::admission::reserve(&app, &agent_chat_id, source.project(), root, depth)
        .await
        .map_err(|error| error.to_string())?;

    Ok(PreparedSpawn {
        request,
        task_meta,
        base_branch,
        base_commit,
        model,
        agent_id,
        agent_chat_id,
        source,
        app,
        original_card,
        worktree_warning,
    })
    }).await
}

async fn persist_spawn(prepared: PreparedSpawn) -> Result<PersistedSpawn, String> {
    let request = prepared.request;
    let gcx = request.gcx;
    let task_id = request.task_id;
    let card_id = request.card_id;
    let agent_id = prepared.agent_id;
    let agent_chat_id = prepared.agent_chat_id;
    let planner_chat_id = request.planner_chat_id;
    let task_meta = prepared.task_meta;
    let source = prepared.source;
    let base_branch = prepared.base_branch;
    let base_commit = prepared.base_commit;
    let files_to_open = request.files_to_open;
    let suggested_steps = request.suggested_steps;
    let model = prepared.model;
    let original = prepared.original_card;
    let app = prepared.app;
    let claim_card = card_id.clone();
    let claim_agent = agent_id.clone();
    let claim_chat = agent_chat_id.clone();
    let mut response = json!({
        "summary": format!("Agent queued: {}", original.title),
        "card_id": card_id,
        "agent_id": agent_id,
        "agent_chat_id": agent_chat_id,
        "model": model,
        "execution_state": "queued",
        "worktree": null,
        "base_branch": base_branch,
        "base_commit": base_commit,
    });
    if let Some(warning) = prepared.worktree_warning {
        response["warning"] = json!(warning);
    }
    let source = source.with_attempt(&task_id, &original, &agent_id, &agent_chat_id, None)?;
    let binding = source
        .task_dir
        .as_ref()
        .ok_or("Accepted task directory is not bound")?
        .clone();
    let tracked_chat = agent_chat_id.clone();
    let (accepted, acceptance) = tokio::sync::oneshot::channel();
    let registered = crate::tasks::admission::spawn_tracked(&tracked_chat, async move {
        binding
            .scope(async move {
                use futures::FutureExt;
                let mut accepted = Some(accepted);
                let cleanup_source = source.clone();
                let setup = async {
                    if accepted.as_ref().is_none_or(|tx| tx.is_closed()) {
                        return Err("spawn caller cancelled before claim".to_string());
                    }
                    source.validate(gcx.clone()).await?;
                    source.validate_task(gcx.clone(), &task_id).await?;
                    let (_, starting_new_run) =
                        storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
                            let starting = storage::active_agent_count(&board.cards) == 0;
                            board
                                .dependency_gate(&claim_card)
                                .map_err(storage::violation_error)?;
                            let card = board
                                .get_card_mut(&claim_card)
                                .ok_or("card removed before claim")?;
                            if !matches!(card.column.as_str(), "planned" | "doing")
                                || card.agent_chat_id.is_some()
                            {
                                return Err("card already claimed or no longer spawnable".into());
                            }
                            card.assignee = Some(claim_agent);
                            card.agent_chat_id = Some(claim_chat);
                            card.column = "doing".into();
                            card.started_at = None;
                            card.agent_worktree = None;
                            card.agent_worktree_name = None;
                            card.agent_branch = None;
                            card.status_updates.push(StatusUpdate {
                                timestamp: Utc::now().to_rfc3339(),
                                message: "Agent queued for execution capacity".into(),
                            });
                            Ok(starting)
                        })
                        .await?;
                    if accepted
                        .take()
                        .ok_or("spawn acceptance lost")?
                        .send(Ok(()))
                        .is_err()
                    {
                        return Err("spawn caller cancelled during claim".into());
                    }
                    run_queued_board_agent(
                        gcx.clone(),
                        app.clone(),
                        task_id.clone(),
                        card_id.clone(),
                        agent_id,
                        agent_chat_id.clone(),
                        planner_chat_id,
                        task_meta,
                        source,
                        base_branch,
                        base_commit,
                        files_to_open,
                        suggested_steps,
                        model,
                        starting_new_run,
                    )
                    .await
                };
                let result = std::panic::AssertUnwindSafe(setup)
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|_| Err("board agent setup panicked".into()));
                if let Err(error) = result {
                    if let Some(accepted) = accepted.take() {
                        let _ = accepted.send(Err(error.clone()));
                    }
                    cleanup_source.reconcile(gcx.clone(), false, &error).await;
                    crate::tasks::admission::end(&agent_chat_id).await;
                    tracing::warn!("Board agent setup failed: {error}");
                }
                if cleanup_source
                    .validate_task(gcx.clone(), &task_id)
                    .await
                    .is_ok()
                {
                    let _ = storage::update_task_stats(gcx, &task_id).await;
                }
            })
            .await
    })
    .await;
    if let Err(error) = registered {
        crate::tasks::admission::end(&tracked_chat).await;
        return Err(error.to_string());
    }

    Ok(PersistedSpawn {
        acceptance,
        response,
        tool_call_id: request.tool_call_id,
    })
}

async fn start_persisted_spawn(
    persisted: PersistedSpawn,
) -> Result<(bool, Vec<ContextEnum>), String> {
    persisted
        .acceptance
        .await
        .map_err(|_| "board setup ended before acceptance".to_string())??;
    Ok((
        false,
        vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".into(),
            content: ChatContent::SimpleText(persisted.response.to_string()),
            tool_call_id: persisted.tool_call_id,
            ..Default::default()
        })],
    ))
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
                    },
                    "base_branch": { "type": "string", "description": "Optional merge target branch for this card" },
                    "base_commit": { "type": "string", "description": "Optional exact commit from which to create this card's agent branch" }
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
        let request = resolve_spawn_request(ccx, tool_call_id, args).await?;
        let prepared = prepare_spawn(request, args).await?;
        let persisted = persist_spawn(prepared).await?;
        start_persisted_spawn(persisted).await
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
            paused_at: None,
            paused_by: None,
            completed_at: None,
            agent_branch: None,
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

    fn evidence_defaults() -> crate::chat::evidence::EvidenceDefaults {
        crate::chat::evidence::EvidenceDefaults {
            timeout_secs: 60,
            output_cap_bytes: 4_000,
        }
    }

    #[test]
    fn spawn_agent_preserves_dispatched_scope_but_uses_live_verify_lines() {
        let mut card = test_card("T-1", "doing", None);
        card.instructions = "## Verification\n- Verify: `cargo test --lib`".to_string();
        card.target_files = vec!["src/lib.rs".to_string()];

        let assignment = Assignment::from_card(&card).with_suggested_steps(40);
        let dispatched_prompt = assignment.render_prompt();

        card.instructions =
            "## Verification\n- Verify: `cargo test --lib -- rewritten`".to_string();
        card.target_files = vec!["src/rewritten.rs".to_string()];
        card.scope_guard_mode = crate::tasks::types::ScopeGuardMode::Strict;

        let validated = crate::tasks::assignment::specs_for_validation(
            Some(&assignment),
            &card,
            &evidence_defaults(),
        );

        assert_eq!(assignment.render_prompt(), dispatched_prompt);
        assert!(dispatched_prompt.contains("Verify: `cargo test --lib`"));
        assert_eq!(
            validated
                .iter()
                .map(|spec| spec.command.clone())
                .collect::<Vec<_>>(),
            vec!["cargo test --lib -- rewritten".to_string()]
        );
        assert_eq!(
            assignment.scope.target_files,
            vec!["src/lib.rs".to_string()],
            "the dispatched scope is not rewritten by a later card edit"
        );
    }

    #[tokio::test]
    async fn spawn_agent_prepare_sets_worktree_meta_without_global_workspace_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let accepted_head = commit_file(&source, "project.txt", "accepted project\n", "accepted");
        let other = temp.path().join("other-repo");
        std::fs::create_dir_all(&other).unwrap();
        init_repo(&other);
        let other_head = commit_file(&other, "project.txt", "foreign project\n", "foreign");
        assert_ne!(accepted_head, other_head);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;

        let task_meta = sample_task_meta(None);
        let accepted_source = AcceptedAgentSource::capture(gcx.clone(), &source, None)
            .await
            .unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![
            other.canonicalize().unwrap(),
            source.canonicalize().unwrap(),
        ];
        let before = crate::files_correction::get_project_dirs(gcx.clone()).await;
        assert_eq!(before[0], other.canonicalize().unwrap());
        let (branch, commit) =
            resolve_spawn_base(None, None, None, accepted_source.root(), &task_meta);
        let prepared = prepare_agent_worktree_with_base_and_suffix(
            gcx.clone(),
            &accepted_source,
            &task_meta,
            "task-1",
            "agent-12345678",
            "T-1",
            "agent-T-1-12345678",
            branch,
            commit,
            None,
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
        assert_eq!(
            prepared.meta.source_workspace_root,
            source.canonicalize().unwrap()
        );
        assert_eq!(
            prepared.meta.base_commit.as_deref(),
            Some(accepted_head.as_str())
        );
        assert_eq!(
            run_git(&prepared.meta.root, &["rev-parse", "HEAD"]).trim(),
            accepted_head
        );
        assert_eq!(
            std::fs::read_to_string(prepared.meta.root.join("project.txt")).unwrap(),
            "accepted project\n"
        );
        let service =
            WorktreeService::new(gcx.cache_dir.clone(), source.canonicalize().unwrap()).unwrap();
        assert_eq!(service.project_hash(), accepted_source.project());
        assert_eq!(
            service.get_worktree(&prepared.meta.id).await.unwrap().meta,
            prepared.meta
        );
        assert!(run_git(
            &other,
            &["branch", "--list", prepared.meta.branch.as_deref().unwrap()]
        )
        .trim()
        .is_empty());

        prepared.cleanup_unlinked(gcx).await;
    }

    #[tokio::test]
    async fn spawn_agent_removed_source_refuses_without_creating_foreign_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let other = temp.path().join("other-repo");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        init_repo(&source);
        init_repo(&other);
        let other_head = commit_file(&other, "project.txt", "foreign project\n", "foreign");
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        let accepted = AcceptedAgentSource::capture(gcx.clone(), &source, None)
            .await
            .unwrap();
        set_workspace(gcx.clone(), &other).await;
        let before = run_git(&other, &["branch", "--list"]);
        let error = prepare_agent_worktree(
            gcx.clone(),
            &accepted,
            &sample_task_meta(None),
            "task-1",
            "agent-refused",
            "T-1",
            "refused-chat",
        )
        .await
        .unwrap_err();
        assert!(error.contains("no longer authorized"), "{error}");
        assert_eq!(run_git(&other, &["branch", "--list"]), before);
        assert_eq!(run_git(&other, &["rev-parse", "HEAD"]).trim(), other_head);
        assert!(!gcx.cache_dir.join("worktrees").exists());
        assert_eq!(
            crate::files_correction::get_project_dirs(gcx).await,
            vec![other.canonicalize().unwrap()]
        );
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
        let accepted_source = AcceptedAgentSource::capture(gcx.clone(), &source, None)
            .await
            .unwrap();
        let prepared = prepare_agent_worktree(
            gcx.clone(),
            &accepted_source,
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

        let accepted_source = AcceptedAgentSource::capture(gcx.clone(), &source, None)
            .await
            .unwrap();
        let prepared = prepare_agent_worktree(
            gcx.clone(),
            &accepted_source,
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

        let accepted_source = AcceptedAgentSource::capture(gcx.clone(), &source, None)
            .await
            .unwrap();
        let err = prepare_agent_worktree(
            gcx,
            &accepted_source,
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

        let abandoned = retained_worktree_cards(&board);
        assert_eq!(abandoned.len(), 1);
        assert_eq!(abandoned[0].id, "T-1");
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

        let abandoned = retained_worktree_cards(&board);
        assert_eq!(abandoned.len(), 1, "only done card should be flagged");
        assert_eq!(abandoned[0].id, "T-2");
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
        let accepted_source = AcceptedAgentSource::capture(gcx.clone(), &source, None)
            .await
            .unwrap();
        let prepared = prepare_agent_worktree(
            gcx.clone(),
            &accepted_source,
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
        let accepted_source = AcceptedAgentSource::capture(gcx.clone(), &source, None)
            .await
            .unwrap();
        let prepared = prepare_agent_worktree(
            gcx.clone(),
            &accepted_source,
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
        card.assignee = Some("agent-abcdef12".into());
        card.agent_chat_id = Some("agent-T-1-abcdef12".to_string());
        setup_task_storage(gcx.clone(), &source, task_id, card).await;

        let accepted_source = accepted_source
            .bind_task(gcx.clone(), task_id)
            .await
            .unwrap();
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        crate::tasks::admission::reserve(
            &app,
            "agent-T-1-abcdef12",
            accepted_source.project(),
            "planner",
            0,
        )
        .await
        .unwrap();
        let original = storage::load_board(gcx.clone(), task_id)
            .await
            .unwrap()
            .get_card("T-1")
            .unwrap()
            .clone();
        let accepted_source = accepted_source
            .with_attempt(
                task_id,
                &original,
                "agent-abcdef12",
                "agent-T-1-abcdef12",
                None,
            )
            .unwrap();
        rollback_retain(
            gcx.clone(),
            &accepted_source,
            task_id,
            "T-1",
            "agent-T-1-abcdef12",
            "model unavailable",
        )
        .await;
        crate::tasks::admission::end("agent-T-1-abcdef12").await;
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
}
