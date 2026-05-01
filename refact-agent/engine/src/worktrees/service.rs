use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use chrono::Utc;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;

use super::git;
use super::types::{
    CreateWorktreeRequest, CreateWorktreeResponse, DeleteWorktreeResponse, MergeWorktreeRequest,
    MergeWorktreeResponse, OpenWorktreeResponse, WorktreeCleanupDeleted, WorktreeCleanupPlan,
    WorktreeCleanupRequest, WorktreeCleanupResult, WorktreeCleanupSkipped, WorktreeCleanupTarget,
    WorktreeConflictState, WorktreeDiffResponse, WorktreeInspection, WorktreeInventory,
    WorktreeInventorySummary, WorktreeListResponse, WorktreeMeta, WorktreeRecordView,
    WorktreeReference, WorktreeRegistry, WorktreeRegistryRecord, WorktreeRemovalResult,
};

const DEFAULT_MAX_PATCH_BYTES: usize = 200_000;

static REGISTRY_WRITE_LOCK: OnceLock<AMutex<()>> = OnceLock::new();
static MERGE_LOCK: OnceLock<AMutex<()>> = OnceLock::new();

fn registry_write_lock() -> &'static AMutex<()> {
    REGISTRY_WRITE_LOCK.get_or_init(|| AMutex::new(()))
}

pub fn worktree_merge_lock() -> &'static AMutex<()> {
    MERGE_LOCK.get_or_init(|| AMutex::new(()))
}

#[derive(Debug, Clone)]
pub struct WorktreeService {
    cache_dir: PathBuf,
    source_workspace_root: PathBuf,
    project_hash: String,
}

impl WorktreeService {
    pub fn new(cache_dir: PathBuf, source_workspace_root: PathBuf) -> Result<Self, String> {
        let cache_dir = normalize_existing_or_parent(&cache_dir);
        let source_workspace_root = canonicalize_existing_dir(&source_workspace_root)?;
        let project_hash = project_hash_for_path(&source_workspace_root);
        Ok(Self {
            cache_dir,
            source_workspace_root,
            project_hash,
        })
    }

    pub fn source_workspace_root(&self) -> &Path {
        &self.source_workspace_root
    }

    pub fn project_hash(&self) -> &str {
        &self.project_hash
    }

    pub fn registry_dir(&self) -> PathBuf {
        self.cache_dir.join("worktrees").join(&self.project_hash)
    }

    pub fn registry_path(&self) -> PathBuf {
        self.registry_dir().join("index.json")
    }

    pub fn worktree_path_for_id(&self, id: &str) -> Result<PathBuf, String> {
        validate_worktree_id(id)?;
        let path = self.registry_dir().join(id);
        if !path.starts_with(self.registry_dir()) {
            return Err("Resolved worktree path escapes registry directory".to_string());
        }
        Ok(path)
    }

    pub async fn load_registry(&self) -> Result<WorktreeRegistry, String> {
        self.load_registry_unlocked().await
    }

    pub async fn save_registry(&self, registry: &WorktreeRegistry) -> Result<(), String> {
        let _guard = registry_write_lock().lock().await;
        self.validate_registry(registry)?;
        self.save_registry_unlocked(registry).await
    }

    pub async fn list_worktrees(&self) -> Result<WorktreeListResponse, String> {
        let registry = self.load_registry_unlocked().await?;
        let mut worktrees = registry
            .records
            .iter()
            .map(|record| self.record_view(record))
            .collect::<Result<Vec<_>, _>>()?;
        worktrees.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        let (source_current_branch, source_branches) =
            match git::discover_repo(&self.source_workspace_root) {
                Ok(repo) => (git::current_branch(&repo), git::local_branches(&repo)),
                Err(_) => (None, Vec::new()),
            };
        Ok(WorktreeListResponse {
            project_hash: self.project_hash.clone(),
            source_workspace_root: self.source_workspace_root.clone(),
            source_current_branch,
            source_branches,
            worktrees,
        })
    }

    pub async fn get_worktree(&self, id: &str) -> Result<WorktreeRecordView, String> {
        validate_worktree_id(id)?;
        let registry = self.load_registry_unlocked().await?;
        let record = registry
            .records
            .iter()
            .find(|record| record.meta.id == id)
            .ok_or_else(|| format!("Worktree '{}' not found", id))?;
        self.record_view(record)
    }

    pub async fn create_worktree(
        &self,
        request: CreateWorktreeRequest,
    ) -> Result<CreateWorktreeResponse, String> {
        let kind = validate_kind(request.kind.as_deref().unwrap_or("chat"))?;
        let branch = match request.branch.clone() {
            Some(branch) => {
                validate_branch_name(&branch)?;
                branch
            }
            None => default_branch_name(&kind, request.chat_id.as_deref()),
        };
        validate_branch_name(&branch)?;
        if let Some(base) = request.base_branch.as_deref() {
            validate_branch_name(base)?;
        }

        let _guard = registry_write_lock().lock().await;
        let mut registry = self.load_registry_unlocked().await?;
        let id = self.next_worktree_id(&registry)?;
        let worktree_path = self.worktree_path_for_id(&id)?;
        if worktree_path.exists() {
            return Err(format!(
                "Worktree path '{}' already exists",
                worktree_path.display()
            ));
        }
        let parent = worktree_path
            .parent()
            .ok_or_else(|| "Worktree path has no parent".to_string())?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create worktree parent: {}", e))?;

        let created = git::create_worktree(
            &self.source_workspace_root,
            &worktree_path,
            &id,
            &branch,
            request.base_branch.as_deref(),
        )?;
        let now = Utc::now().to_rfc3339();
        let reference = request_to_reference(&kind, &request);
        let references = reference.into_iter().collect::<Vec<_>>();
        let meta = WorktreeMeta {
            id: id.clone(),
            kind,
            root: normalized_path_key(&worktree_path)?,
            source_workspace_root: self.source_workspace_root.clone(),
            repo_root: created.repo_root,
            branch: Some(branch.clone()),
            base_branch: created.base_branch,
            base_commit: Some(created.base_commit),
            task_id: request.task_id.clone(),
            card_id: request.card_id.clone(),
            agent_id: request.agent_id.clone(),
            enforce: true,
        };
        let status = git::status_for_path(&worktree_path);
        let record = WorktreeRegistryRecord {
            meta,
            created_at: now.clone(),
            updated_at: now,
            last_seen_at: Some(Utc::now().to_rfc3339()),
            references,
            last_known_status: Some(status),
        };
        registry.records.push(record.clone());
        if let Err(e) = self.save_registry_unlocked(&registry).await {
            let mut warnings =
                git::remove_worktree(&self.source_workspace_root, &id, &worktree_path);
            if created.branch_was_created {
                if let Err(branch_err) = git::delete_branch(&self.source_workspace_root, &branch) {
                    warnings.push(branch_err);
                }
            }
            return Err(format!(
                "Failed to save worktree registry: {}; cleanup warnings: {}",
                e,
                warnings.join("; ")
            ));
        }

        let mut warnings = Vec::new();
        if created.dirty_source {
            warnings.push(
                "Source checkout has uncommitted changes; worktree was created from committed HEAD/base only"
                    .to_string(),
            );
        }
        Ok(CreateWorktreeResponse {
            worktree: self.record_view(&record)?,
            branch_was_created: created.branch_was_created,
            dirty_source_warning: created.dirty_source,
            warnings,
        })
    }

    pub async fn add_reference(
        &self,
        id: &str,
        reference: WorktreeReference,
    ) -> Result<WorktreeRecordView, String> {
        validate_worktree_id(id)?;
        if !reference.has_identity() {
            return Err("Worktree reference must include at least one id".to_string());
        }
        let _guard = registry_write_lock().lock().await;
        let mut registry = self.load_registry_unlocked().await?;
        let record = registry
            .records
            .iter_mut()
            .find(|record| record.meta.id == id)
            .ok_or_else(|| format!("Worktree '{}' not found", id))?;
        if !record.references.contains(&reference) {
            record.references.push(reference);
            record.updated_at = Utc::now().to_rfc3339();
        }
        let view = self.record_view(record)?;
        self.save_registry_unlocked(&registry).await?;
        Ok(view)
    }

    pub async fn remove_reference(
        &self,
        id: &str,
        reference: &WorktreeReference,
    ) -> Result<WorktreeRecordView, String> {
        validate_worktree_id(id)?;
        if !reference.has_identity() {
            return Err("Worktree reference must include at least one id".to_string());
        }
        let _guard = registry_write_lock().lock().await;
        let mut registry = self.load_registry_unlocked().await?;
        let record = registry
            .records
            .iter_mut()
            .find(|record| record.meta.id == id)
            .ok_or_else(|| format!("Worktree '{}' not found", id))?;
        let before = record.references.len();
        record.references.retain(|item| item != reference);
        if record.references.len() != before {
            record.updated_at = Utc::now().to_rfc3339();
        }
        let view = self.record_view(record)?;
        self.save_registry_unlocked(&registry).await?;
        Ok(view)
    }

    pub async fn validate_worktree_meta(
        &self,
        meta: &WorktreeMeta,
    ) -> Result<WorktreeMeta, String> {
        self.validate_worktree_meta_strict(meta).await
    }

    pub async fn validate_worktree_meta_strict(
        &self,
        meta: &WorktreeMeta,
    ) -> Result<WorktreeMeta, String> {
        validate_worktree_id(&meta.id)?;
        let meta_source = canonicalize_existing_dir(&meta.source_workspace_root)?;
        if normalized_path_key(&meta_source)? != normalized_path_key(&self.source_workspace_root)? {
            return Err("Worktree source root does not match current workspace".to_string());
        }
        let meta_root = normalized_path_key(&meta.root)?;
        let registry = self.load_registry_unlocked().await?;
        let record = registry
            .records
            .iter()
            .find(|record| record.meta.id == meta.id)
            .ok_or_else(|| format!("Worktree '{}' is not registered", meta.id))?;
        let record_root = normalized_path_key(&record.meta.root)?;
        if record_root != meta_root {
            return Err(format!(
                "Worktree '{}' root mismatch: '{}' != '{}'",
                meta.id,
                meta_root.display(),
                record_root.display()
            ));
        }
        let record_source = canonicalize_existing_dir(&record.meta.source_workspace_root)?;
        if normalized_path_key(&record_source)? != normalized_path_key(&meta_source)? {
            return Err(format!("Worktree '{}' source root mismatch", meta.id));
        }
        Ok(record.meta.clone())
    }

    pub async fn validate_legacy_task_agent_worktree_meta(
        &self,
        meta: &WorktreeMeta,
    ) -> Result<WorktreeMeta, String> {
        validate_worktree_id(&meta.id)?;
        if meta.kind != "task_agent" {
            return Err("Legacy worktree metadata must be task_agent kind".to_string());
        }
        if meta.task_id.as_deref().unwrap_or_default().is_empty()
            || meta.card_id.as_deref().unwrap_or_default().is_empty()
            || meta.agent_id.as_deref().unwrap_or_default().is_empty()
        {
            return Err("Legacy task-agent worktree metadata is missing identity".to_string());
        }
        let meta_source = canonicalize_existing_dir(&meta.source_workspace_root)?;
        if normalized_path_key(&meta_source)? != normalized_path_key(&self.source_workspace_root)? {
            return Err("Worktree source root does not match current workspace".to_string());
        }
        let meta_root = normalized_path_key(&meta.root)?;
        if let Ok(validated) = self.validate_worktree_meta_strict(meta).await {
            return Ok(validated);
        }
        let discovered = git::list_git_worktrees(&self.source_workspace_root)
            .into_iter()
            .any(|entry| {
                normalized_path_key(&entry.root)
                    .map(|root| root == meta_root)
                    .unwrap_or(false)
            });
        let status = git::status_for_path(&meta_root);
        if discovered && status.path_exists && status.is_git_worktree {
            return Ok(meta.clone());
        }
        Err(format!(
            "Worktree '{}' is not registered or discoverable for current workspace",
            meta.id
        ))
    }

    pub async fn diff_worktree(&self, id: &str) -> Result<WorktreeDiffResponse, String> {
        self.diff_worktree_with_limit(id, DEFAULT_MAX_PATCH_BYTES)
            .await
    }

    pub async fn diff_worktree_with_limit(
        &self,
        id: &str,
        max_patch_bytes: usize,
    ) -> Result<WorktreeDiffResponse, String> {
        validate_worktree_id(id)?;
        let registry = self.load_registry_unlocked().await?;
        let record = registry
            .records
            .iter()
            .find(|record| record.meta.id == id)
            .ok_or_else(|| format!("Worktree '{}' not found", id))?;
        if !record.meta.root.exists() {
            return Err(format!(
                "Worktree '{}' path '{}' does not exist",
                id,
                record.meta.root.display()
            ));
        }
        let diff = git::diff_for_path(
            &record.meta.root,
            record.meta.base_commit.as_deref(),
            record.meta.base_branch.as_deref(),
            max_patch_bytes,
        )?;
        Ok(WorktreeDiffResponse {
            id: id.to_string(),
            branch: record.meta.branch.clone(),
            base_branch: record.meta.base_branch.clone(),
            base_commit: record.meta.base_commit.clone(),
            status: git::status_for_path(&record.meta.root),
            files: diff.files,
            stats: diff.stats,
            patch: diff.patch,
            patch_truncated: diff.patch_truncated,
        })
    }

    pub async fn merge_worktree(
        &self,
        id: &str,
        request: MergeWorktreeRequest,
    ) -> Result<MergeWorktreeResponse, String> {
        validate_worktree_id(id)?;
        if let Some(target_branch) = request.target_branch.as_deref() {
            validate_branch_name(target_branch)?;
        }
        let _merge_guard = worktree_merge_lock().lock().await;
        let _registry_guard = registry_write_lock().lock().await;
        let mut registry = self.load_registry_unlocked().await?;
        let index = registry
            .records
            .iter()
            .position(|record| record.meta.id == id)
            .ok_or_else(|| format!("Worktree '{}' not found", id))?;
        let record = registry.records[index].clone();
        if !record.meta.root.exists() {
            return Err(format!(
                "Worktree '{}' path '{}' does not exist",
                id,
                record.meta.root.display()
            ));
        }
        let source_branch = record
            .meta
            .branch
            .clone()
            .ok_or_else(|| format!("Worktree '{}' has no source branch", id))?;
        validate_branch_name(&source_branch)?;
        let target_branch = request
            .target_branch
            .clone()
            .or_else(|| record.meta.base_branch.clone())
            .ok_or_else(|| format!("Worktree '{}' has no base branch", id))?;
        validate_branch_name(&target_branch)?;
        if !git::branch_exists(&record.meta.source_workspace_root, &source_branch)? {
            return Err(format!("Source branch '{}' not found", source_branch));
        }
        if !git::branch_exists(&record.meta.source_workspace_root, &target_branch)? {
            return Err(format!("Target branch '{}' not found", target_branch));
        }
        git::ensure_clean_worktree(&record.meta.source_workspace_root, "Target workspace")?;
        let status = git::status_for_path(&record.meta.root);
        if !status.is_git_worktree {
            return Err(format!("Worktree '{}' is not a git worktree", id));
        }
        let commit_message = request
            .commit_message
            .clone()
            .filter(|message| !message.trim().is_empty())
            .unwrap_or_else(|| fallback_merge_message(&record, &source_branch));
        let mut committed_uncommitted = None;
        if status.dirty {
            if !request.include_uncommitted {
                return Err(format!(
                    "Worktree '{}' has uncommitted changes; set include_uncommitted=true to auto-commit them first",
                    id
                ));
            }
            committed_uncommitted = git::commit_all(&record.meta.root, &commit_message)?;
        }
        git::ensure_clean_worktree(&record.meta.root, "Source worktree")?;
        let ahead = git::commits_ahead(
            &record.meta.source_workspace_root,
            &target_branch,
            &source_branch,
        )?;
        let affected_references = record.references.clone();
        let affected_reference_count = affected_references.len();
        if ahead == 0 {
            let cleanup = if request.delete_after_merge && source_branch != target_branch {
                Some(
                    self.cleanup_registered_worktree(&mut registry, index, &record, true)
                        .await?,
                )
            } else {
                None
            };
            return Ok(MergeWorktreeResponse {
                id: id.to_string(),
                status: "nothing_to_merge".to_string(),
                merged: false,
                strategy: request.strategy.as_str().to_string(),
                source_branch,
                target_branch,
                committed_uncommitted,
                merge_commit: None,
                cleanup,
                conflict: None,
                affected_references,
                affected_reference_count,
                warnings: Vec::new(),
            });
        }
        let conflicts = git::preflight_merge_conflicts(
            &record.meta.source_workspace_root,
            &target_branch,
            &source_branch,
            request.strategy.as_str(),
        )?;
        if !conflicts.is_empty() {
            return Ok(MergeWorktreeResponse {
                id: id.to_string(),
                status: "conflict".to_string(),
                merged: false,
                strategy: request.strategy.as_str().to_string(),
                source_branch,
                target_branch,
                committed_uncommitted,
                merge_commit: None,
                cleanup: None,
                conflict: Some(conflict_state(conflicts, true, false)),
                affected_references,
                affected_reference_count,
                warnings: Vec::new(),
            });
        }
        git::checkout_branch(&record.meta.source_workspace_root, &target_branch).map_err(|e| {
            format!(
                "Failed to checkout target branch '{}': {}",
                target_branch, e
            )
        })?;
        let merge_result = if request.strategy.as_str() == "squash" {
            git::run_git(
                &record.meta.source_workspace_root,
                &["merge", "--squash", &source_branch],
            )
        } else {
            git::run_git_with_refact_author(
                &record.meta.source_workspace_root,
                &["merge", "--no-ff", &source_branch, "-m", &commit_message],
            )
        };
        if let Err(e) = merge_result {
            let conflict_files = git::conflict_files_for_path(&record.meta.source_workspace_root);
            let cleanup_warnings = git::cleanup_failed_merge(&record.meta.source_workspace_root);
            if !conflict_files.is_empty() {
                let aborted = cleanup_warnings.is_empty();
                return Ok(MergeWorktreeResponse {
                    id: id.to_string(),
                    status: "conflict".to_string(),
                    merged: false,
                    strategy: request.strategy.as_str().to_string(),
                    source_branch,
                    target_branch,
                    committed_uncommitted,
                    merge_commit: None,
                    cleanup: None,
                    conflict: Some(conflict_state(conflict_files, aborted, !aborted)),
                    affected_references,
                    affected_reference_count,
                    warnings: cleanup_warnings,
                });
            }
            return Err(format_merge_failure("Merge failed", &e, cleanup_warnings));
        }
        if request.strategy.as_str() == "squash" {
            if let Err(e) = git::run_git_with_refact_author(
                &record.meta.source_workspace_root,
                &["commit", "-m", &commit_message, "--no-gpg-sign"],
            ) {
                let cleanup_warnings =
                    git::cleanup_failed_merge(&record.meta.source_workspace_root);
                return Err(format_merge_failure(
                    "Failed to commit squash merge",
                    &e,
                    cleanup_warnings,
                ));
            }
        }
        let merge_commit = Some(git::head_rev(&record.meta.source_workspace_root)?);
        let cleanup = if request.delete_after_merge && source_branch != target_branch {
            Some(
                self.cleanup_registered_worktree(&mut registry, index, &record, true)
                    .await?,
            )
        } else {
            registry.records[index].last_known_status =
                Some(git::status_for_path(&record.meta.root));
            registry.records[index].updated_at = Utc::now().to_rfc3339();
            self.save_registry_unlocked(&registry).await?;
            None
        };
        Ok(MergeWorktreeResponse {
            id: id.to_string(),
            status: "merged".to_string(),
            merged: true,
            strategy: request.strategy.as_str().to_string(),
            source_branch,
            target_branch,
            committed_uncommitted,
            merge_commit,
            cleanup,
            conflict: None,
            affected_references,
            affected_reference_count,
            warnings: Vec::new(),
        })
    }

    pub async fn delete_worktree(
        &self,
        id: &str,
        delete_branch: bool,
    ) -> Result<DeleteWorktreeResponse, String> {
        validate_worktree_id(id)?;
        let _guard = registry_write_lock().lock().await;
        let mut registry = self.load_registry_unlocked().await?;
        let index = registry
            .records
            .iter()
            .position(|record| record.meta.id == id)
            .ok_or_else(|| format!("Worktree '{}' not found", id))?;
        let record = registry.records[index].clone();
        let stale_path = !record.meta.root.exists();
        let mut warnings = git::remove_worktree(
            &record.meta.source_workspace_root,
            &record.meta.id,
            &record.meta.root,
        );
        if record.meta.root.exists() {
            return Err(format!(
                "Failed to remove worktree directory '{}': {}",
                record.meta.root.display(),
                warnings.join("; ")
            ));
        }

        let mut branch_deleted = false;
        if delete_branch {
            if let Some(branch) = record.meta.branch.as_deref() {
                match git::delete_branch(&record.meta.source_workspace_root, branch) {
                    Ok(deleted) => branch_deleted = deleted,
                    Err(e) => warnings.push(e),
                }
            }
        }
        registry.records.remove(index);
        self.save_registry_unlocked(&registry).await?;
        let affected_reference_count = record.references.len();
        Ok(DeleteWorktreeResponse {
            deleted: true,
            branch_deleted,
            stale_path,
            affected_references: record.references,
            affected_reference_count,
            warnings,
        })
    }

    pub async fn open_worktree(&self, id: &str) -> Result<OpenWorktreeResponse, String> {
        let view = self.get_worktree(id).await?;
        Ok(OpenWorktreeResponse {
            id: view.meta.id,
            path: view.meta.root,
            branch: view.meta.branch,
            can_open_folder: view.status.path_exists && view.status.is_git_worktree,
        })
    }

    pub async fn inspect_worktrees(&self) -> Result<WorktreeInventory, String> {
        self.inspect_worktrees_with_min_age(24).await
    }

    pub async fn inspect_worktrees_with_min_age(
        &self,
        min_age_hours: u64,
    ) -> Result<WorktreeInventory, String> {
        let registry = self.load_registry_unlocked().await?;
        self.inspect_worktrees_from_registry(&registry, min_age_hours)
    }

    pub async fn cleanup_worktrees_dry_run(
        &self,
        request: WorktreeCleanupRequest,
    ) -> Result<WorktreeCleanupPlan, String> {
        validate_cleanup_request(&request)?;
        let registry = self.load_registry_unlocked().await?;
        let inventory = self.inspect_worktrees_from_registry(&registry, request.min_age_hours)?;
        Ok(self.cleanup_plan_from_inventory(&inventory, request))
    }

    pub async fn cleanup_worktrees(
        &self,
        request: WorktreeCleanupRequest,
    ) -> Result<WorktreeCleanupResult, String> {
        validate_cleanup_request(&request)?;
        let _guard = registry_write_lock().lock().await;
        let mut registry = self.load_registry_unlocked().await?;
        let inventory = self.inspect_worktrees_from_registry(&registry, request.min_age_hours)?;
        let plan = self.cleanup_plan_from_inventory(&inventory, request.clone());
        let mut deleted = Vec::new();
        let mut skipped = plan.skipped.clone();
        let warnings = Vec::new();

        for target in plan.candidates {
            if let Some(index) = registry
                .records
                .iter()
                .position(|record| record.meta.id == target.id)
            {
                let record = registry.records[index].clone();
                let removal = self
                    .cleanup_registered_worktree(
                        &mut registry,
                        index,
                        &record,
                        target.delete_branch,
                    )
                    .await?;
                deleted.push(WorktreeCleanupDeleted {
                    id: target.id,
                    root: target.root,
                    branch: target.branch,
                    worktree_deleted: removal.worktree_deleted,
                    branch_deleted: removal.branch_deleted,
                    registry_deleted: removal.registry_deleted,
                    stale_path: removal.stale_path,
                    warnings: removal.warnings,
                });
            } else {
                let stale_path = !target.root.exists();
                let mut item_warnings =
                    git::remove_worktree_path(&self.source_workspace_root, &target.root);
                let mut branch_deleted = false;
                if target.delete_branch {
                    if let Some(branch) = target.branch.as_deref() {
                        match git::delete_branch(&self.source_workspace_root, branch) {
                            Ok(deleted) => branch_deleted = deleted,
                            Err(e) => item_warnings.push(e),
                        }
                    }
                }
                deleted.push(WorktreeCleanupDeleted {
                    id: target.id,
                    root: target.root.clone(),
                    branch: target.branch.clone(),
                    worktree_deleted: !target.root.exists(),
                    branch_deleted,
                    registry_deleted: false,
                    stale_path,
                    warnings: item_warnings,
                });
            }
        }

        skipped.sort_by(|a, b| a.id.cmp(&b.id));
        deleted.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(WorktreeCleanupResult {
            generated_at: Utc::now().to_rfc3339(),
            request,
            deleted,
            skipped,
            warnings,
        })
    }

    fn inspect_worktrees_from_registry(
        &self,
        registry: &WorktreeRegistry,
        min_age_hours: u64,
    ) -> Result<WorktreeInventory, String> {
        let mut worktrees = Vec::new();
        let mut registered_roots = HashSet::new();
        let registry_dir = self.registry_dir();

        for record in &registry.records {
            let root_key = normalize_lexical(&record.meta.root)?;
            registered_roots.insert(root_key);
            worktrees.push(self.inspect_registered_record(record, min_age_hours));
        }

        let mut discovered_roots = HashSet::new();
        for entry in git::list_git_worktrees(&self.source_workspace_root) {
            let root_key = normalize_lexical(&entry.root)?;
            if root_key == self.source_workspace_root || registered_roots.contains(&root_key) {
                continue;
            }
            if !root_key.starts_with(&registry_dir) || !discovered_roots.insert(root_key.clone()) {
                continue;
            }
            worktrees.push(self.inspect_discovered_root(
                worktree_id_from_path(&root_key),
                root_key,
                entry.branch,
                entry.head,
                true,
                min_age_hours,
            ));
        }

        if registry_dir.exists() {
            if let Ok(read_dir) = std::fs::read_dir(&registry_dir) {
                for entry in read_dir.flatten() {
                    let path = entry.path();
                    if !path.is_dir() {
                        continue;
                    }
                    let root_key = normalize_lexical(&path)?;
                    if registered_roots.contains(&root_key) || discovered_roots.contains(&root_key)
                    {
                        continue;
                    }
                    if !discovered_roots.insert(root_key.clone()) {
                        continue;
                    }
                    worktrees.push(self.inspect_discovered_root(
                        worktree_id_from_path(&root_key),
                        root_key,
                        None,
                        None,
                        true,
                        min_age_hours,
                    ));
                }
            }
        }

        worktrees.sort_by(|a, b| a.id.cmp(&b.id));
        let summary = summarize_inventory(&worktrees);
        let cleanup_candidates = worktrees
            .iter()
            .filter(|item| item.cleanup_candidate)
            .map(|item| item.id.clone())
            .collect();
        Ok(WorktreeInventory {
            project_hash: self.project_hash.clone(),
            source_workspace_root: self.source_workspace_root.clone(),
            generated_at: Utc::now().to_rfc3339(),
            summary,
            worktrees,
            cleanup_candidates,
        })
    }

    fn inspect_registered_record(
        &self,
        record: &WorktreeRegistryRecord,
        min_age_hours: u64,
    ) -> WorktreeInspection {
        let age_hours = age_hours_since(
            record
                .last_seen_at
                .as_deref()
                .unwrap_or(record.updated_at.as_str()),
        );
        let last_used_at = record
            .last_seen_at
            .clone()
            .or_else(|| Some(record.updated_at.clone()));
        let mut item = self.inspect_root(
            record.meta.id.clone(),
            "registered".to_string(),
            record.meta.root.clone(),
            record.meta.branch.clone(),
            record.meta.base_branch.clone(),
            record.meta.base_commit.clone(),
            record.references.clone(),
            age_hours,
            last_used_at,
            false,
            false,
            min_age_hours,
        );
        item.attached_chat_ids = attached_chat_ids(&item.references);
        item.attached_task_ids = attached_task_ids(&item.references);
        item
    }

    fn inspect_discovered_root(
        &self,
        id: String,
        root: PathBuf,
        branch: Option<String>,
        _head: Option<String>,
        cache_dir_missing_from_registry: bool,
        min_age_hours: u64,
    ) -> WorktreeInspection {
        let age_hours = filesystem_age_hours(&root);
        let mut item = self.inspect_root(
            id,
            "discovered".to_string(),
            root,
            branch,
            None,
            None,
            Vec::new(),
            age_hours,
            None,
            true,
            cache_dir_missing_from_registry,
            min_age_hours,
        );
        item.attached_chat_ids = attached_chat_ids(&item.references);
        item.attached_task_ids = attached_task_ids(&item.references);
        item
    }

    fn inspect_root(
        &self,
        id: String,
        source: String,
        root: PathBuf,
        branch: Option<String>,
        base_branch: Option<String>,
        base_commit: Option<String>,
        references: Vec<WorktreeReference>,
        age_hours: Option<u64>,
        last_used_at: Option<String>,
        registry_missing: bool,
        cache_dir_missing_from_registry: bool,
        min_age_hours: u64,
    ) -> WorktreeInspection {
        let mut status = git::status_for_path(&root);
        if let Some(branch) = branch.as_deref() {
            if status.branch.is_none() {
                status.branch = Some(branch.to_string());
            }
        }
        let mut diff_stats = None;
        let mut diff_error = None;
        if status.path_exists && status.is_git_worktree {
            if base_commit.is_none() && base_branch.is_none() {
                diff_error = Some("base metadata is missing".to_string());
            } else {
                match git::diff_for_path(&root, base_commit.as_deref(), base_branch.as_deref(), 1) {
                    Ok(diff) => diff_stats = Some(diff.stats),
                    Err(e) => diff_error = Some(e),
                }
            }
        }
        if let Some(error) = diff_error {
            status.error = Some(format!("diff failed: {}", error));
        }
        let conflicted = status.conflicted
            || (status.path_exists
                && status.is_git_worktree
                && !git::conflict_files_for_path(&root).is_empty());
        status.conflicted = conflicted;
        let shared = references.len() > 1;
        let stale = !status.path_exists || !status.is_git_worktree;
        let changed_files = diff_stats
            .as_ref()
            .map(|stats| stats.files_changed)
            .unwrap_or_else(|| {
                status.staged_count + status.unstaged_count + status.untracked_count
            });
        let committed_files = diff_stats
            .as_ref()
            .map(|stats| stats.committed_files)
            .unwrap_or(0);
        let staged_files = diff_stats
            .as_ref()
            .map(|stats| stats.staged_files)
            .unwrap_or(status.staged_count);
        let unstaged_files = diff_stats
            .as_ref()
            .map(|stats| stats.unstaged_files)
            .unwrap_or(status.unstaged_count);
        let untracked_files = diff_stats
            .as_ref()
            .map(|stats| stats.untracked_files)
            .unwrap_or(status.untracked_count);
        let additions = diff_stats
            .as_ref()
            .map(|stats| stats.additions)
            .unwrap_or(0);
        let deletions = diff_stats
            .as_ref()
            .map(|stats| stats.deletions)
            .unwrap_or(0);
        let branch_merged = branch.as_deref().and_then(|branch| {
            base_branch
                .as_deref()
                .map(|base| git::branch_merged_into(&self.source_workspace_root, branch, base))
        });
        let mut item = WorktreeInspection {
            id,
            source,
            root: root.clone(),
            branch,
            base_branch,
            base_commit,
            status,
            reference_count: references.len(),
            references,
            shared,
            stale,
            conflicted,
            changed_files,
            committed_files,
            staged_files,
            unstaged_files,
            untracked_files,
            additions,
            deletions,
            cleanup_candidate: false,
            cleanup_blockers: Vec::new(),
            disk_usage_bytes: cheap_disk_usage(&root),
            age_hours,
            last_used_at,
            branch_merged,
            registry_missing,
            cache_dir_missing_from_registry,
            attached_chat_ids: Vec::new(),
            attached_task_ids: Vec::new(),
        };
        item.reference_count = item.references.len();
        item.shared = item.reference_count > 1;
        let request = WorktreeCleanupRequest {
            ids: vec![item.id.clone()],
            min_age_hours,
            ..WorktreeCleanupRequest::default()
        };
        item.cleanup_blockers = cleanup_blockers_for_item(&item, &request);
        item.cleanup_candidate = item.cleanup_blockers.is_empty();
        item
    }

    fn cleanup_plan_from_inventory(
        &self,
        inventory: &WorktreeInventory,
        request: WorktreeCleanupRequest,
    ) -> WorktreeCleanupPlan {
        let by_id: HashMap<&str, &WorktreeInspection> = inventory
            .worktrees
            .iter()
            .map(|item| (item.id.as_str(), item))
            .collect();
        let mut candidates = Vec::new();
        let mut skipped = Vec::new();
        let mut seen = HashSet::new();
        for id in &request.ids {
            if !seen.insert(id.clone()) {
                continue;
            }
            let Some(item) = by_id.get(id.as_str()) else {
                skipped.push(WorktreeCleanupSkipped {
                    id: id.clone(),
                    root: None,
                    reason: "not_found".to_string(),
                    details: vec!["worktree id was not found in registry or discovery".to_string()],
                });
                continue;
            };
            let blockers = cleanup_blockers_for_item(item, &request);
            if blockers.is_empty() {
                candidates.push(WorktreeCleanupTarget {
                    id: item.id.clone(),
                    root: item.root.clone(),
                    branch: item.branch.clone(),
                    shared: item.shared,
                    stale: item.stale,
                    changed_files: item.changed_files,
                    additions: item.additions,
                    deletions: item.deletions,
                    delete_branch: request.delete_branches,
                    references: item.references.clone(),
                    disk_usage_bytes: item.disk_usage_bytes,
                });
            } else {
                skipped.push(WorktreeCleanupSkipped {
                    id: item.id.clone(),
                    root: Some(item.root.clone()),
                    reason: blockers
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "blocked".to_string()),
                    details: blockers,
                });
            }
        }
        candidates.sort_by(|a, b| a.id.cmp(&b.id));
        skipped.sort_by(|a, b| a.id.cmp(&b.id));
        WorktreeCleanupPlan {
            generated_at: Utc::now().to_rfc3339(),
            request,
            candidates,
            skipped,
        }
    }

    async fn cleanup_registered_worktree(
        &self,
        registry: &mut WorktreeRegistry,
        index: usize,
        record: &WorktreeRegistryRecord,
        delete_branch: bool,
    ) -> Result<WorktreeRemovalResult, String> {
        let stale_path = !record.meta.root.exists();
        let mut warnings = git::remove_worktree(
            &record.meta.source_workspace_root,
            &record.meta.id,
            &record.meta.root,
        );
        let worktree_deleted = !record.meta.root.exists();
        let mut branch_deleted = false;
        if delete_branch {
            if let Some(branch) = record.meta.branch.as_deref() {
                match git::delete_branch(&record.meta.source_workspace_root, branch) {
                    Ok(deleted) => branch_deleted = deleted,
                    Err(e) => warnings.push(e),
                }
            }
        }
        let registry_deleted = worktree_deleted;
        if registry_deleted {
            registry.records.remove(index);
        } else if let Some(record) = registry.records.get_mut(index) {
            record.last_known_status = Some(git::status_for_path(&record.meta.root));
            record.updated_at = Utc::now().to_rfc3339();
        }
        self.save_registry_unlocked(registry).await?;
        Ok(WorktreeRemovalResult {
            worktree_deleted,
            branch_deleted,
            registry_deleted,
            stale_path,
            warnings,
        })
    }

    fn record_view(&self, record: &WorktreeRegistryRecord) -> Result<WorktreeRecordView, String> {
        validate_worktree_id(&record.meta.id)?;
        validate_registry_root(&self.registry_dir(), &record.meta.id, &record.meta.root)?;
        let mut seen = HashSet::new();
        let references = record
            .references
            .iter()
            .filter(|reference| reference.has_identity())
            .filter(|reference| seen.insert(reference_key(reference)))
            .cloned()
            .collect::<Vec<_>>();
        let reference_count = references.len();
        Ok(WorktreeRecordView {
            meta: record.meta.clone(),
            created_at: record.created_at.clone(),
            updated_at: record.updated_at.clone(),
            last_seen_at: record.last_seen_at.clone(),
            references,
            reference_count,
            status: git::status_for_path(&record.meta.root),
        })
    }

    async fn load_registry_unlocked(&self) -> Result<WorktreeRegistry, String> {
        let path = self.registry_path();
        if !path.exists() {
            return Ok(WorktreeRegistry {
                schema_version: 1,
                source_workspace_root: self.source_workspace_root.clone(),
                project_hash: self.project_hash.clone(),
                records: Vec::new(),
            });
        }
        let content = tokio::fs::read_to_string(&path).await.map_err(|e| {
            format!(
                "Failed to read worktree registry '{}': {}",
                path.display(),
                e
            )
        })?;
        let registry: WorktreeRegistry = serde_json::from_str(&content).map_err(|e| {
            format!(
                "Failed to parse worktree registry '{}': {}",
                path.display(),
                e
            )
        })?;
        self.validate_registry(&registry)?;
        Ok(registry)
    }

    async fn save_registry_unlocked(&self, registry: &WorktreeRegistry) -> Result<(), String> {
        self.validate_registry(registry)?;
        let dir = self.registry_dir();
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| format!("Failed to create worktree registry dir: {}", e))?;
        let path = self.registry_path();
        let tmp_path = path.with_extension("json.tmp");
        let content = serde_json::to_string_pretty(registry)
            .map_err(|e| format!("Failed to serialize worktree registry: {}", e))?;
        tokio::fs::write(&tmp_path, content)
            .await
            .map_err(|e| format!("Failed to write worktree registry temp file: {}", e))?;
        #[cfg(windows)]
        if path.exists() {
            tokio::fs::remove_file(&path)
                .await
                .map_err(|e| format!("Failed to remove existing worktree registry: {}", e))?;
        }
        tokio::fs::rename(&tmp_path, &path)
            .await
            .map_err(|e| format!("Failed to replace worktree registry: {}", e))
    }

    fn validate_registry(&self, registry: &WorktreeRegistry) -> Result<(), String> {
        if registry.project_hash != self.project_hash {
            return Err("Worktree registry project hash mismatch".to_string());
        }
        let registry_root = normalized_path_key(&registry.source_workspace_root)?;
        if registry_root != normalized_path_key(&self.source_workspace_root)? {
            return Err("Worktree registry source root mismatch".to_string());
        }
        for record in &registry.records {
            validate_worktree_id(&record.meta.id)?;
            validate_kind(&record.meta.kind)?;
            if let Some(branch) = record.meta.branch.as_deref() {
                validate_branch_name(branch)?;
            }
            if let Some(base_branch) = record.meta.base_branch.as_deref() {
                validate_branch_name(base_branch)?;
            }
            validate_registry_root(&self.registry_dir(), &record.meta.id, &record.meta.root)?;
        }
        Ok(())
    }

    fn next_worktree_id(&self, registry: &WorktreeRegistry) -> Result<String, String> {
        for _ in 0..16 {
            let id = Uuid::new_v4().to_string();
            if registry.records.iter().all(|record| record.meta.id != id)
                && !self.worktree_path_for_id(&id)?.exists()
            {
                return Ok(id);
            }
        }
        Err("Failed to allocate unique worktree id".to_string())
    }
}

pub fn project_hash_for_path(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    hash.chars().take(16).collect()
}

pub fn validate_worktree_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("Worktree ID cannot be empty".to_string());
    }
    if id.len() > 128 {
        return Err("Worktree ID is too long".to_string());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(
            "Worktree ID must contain only ASCII alphanumeric characters, hyphens, or underscores"
                .to_string(),
        );
    }
    Ok(())
}

pub fn validate_branch_name(branch: &str) -> Result<(), String> {
    if branch.is_empty() {
        return Err("Branch name cannot be empty".to_string());
    }
    if branch.len() > 240 {
        return Err("Branch name is too long".to_string());
    }
    if branch.starts_with('/') || branch.ends_with('/') || branch.starts_with('-') {
        return Err(format!("Invalid branch name '{}'", branch));
    }
    if branch.contains("..")
        || branch.contains("//")
        || branch.contains("@{")
        || branch.ends_with('.')
        || branch.ends_with(".lock")
    {
        return Err(format!("Invalid branch name '{}'", branch));
    }
    if branch.chars().any(|c| {
        c.is_control() || c.is_whitespace() || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
    }) {
        return Err(format!("Invalid branch name '{}'", branch));
    }
    Ok(())
}

fn validate_kind(kind: &str) -> Result<String, String> {
    if kind.is_empty() || kind.len() > 64 {
        return Err("Worktree kind must be 1-64 characters".to_string());
    }
    if !kind
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("Worktree kind contains invalid characters".to_string());
    }
    Ok(kind.to_string())
}

fn validate_registry_root(registry_dir: &Path, id: &str, root: &Path) -> Result<(), String> {
    let registry_dir = normalize_lexical(registry_dir)?;
    let root = normalize_lexical(root)?;
    let expected = normalize_lexical(&registry_dir.join(id))?;
    if root != expected {
        return Err(format!(
            "Worktree root '{}' must exactly match registry path '{}'",
            root.display(),
            expected.display()
        ));
    }
    if let Ok(metadata) = std::fs::symlink_metadata(&expected) {
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Worktree root '{}' must not be a symlink",
                expected.display()
            ));
        }
    }
    Ok(())
}

fn canonicalize_existing_dir(path: &Path) -> Result<PathBuf, String> {
    let canonical = std::fs::canonicalize(path).map_err(|e| {
        format!(
            "Failed to resolve source workspace root '{}': {}",
            path.display(),
            e
        )
    })?;
    if !canonical.is_dir() {
        return Err(format!(
            "Source workspace root '{}' is not a directory",
            canonical.display()
        ));
    }
    Ok(dunce::simplified(&canonical).to_path_buf())
}

fn normalize_existing_or_parent(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return dunce::simplified(&canonical).to_path_buf();
    }
    let Some(parent) = path.parent() else {
        return normalize_lexical(path).unwrap_or_else(|_| path.to_path_buf());
    };
    let Some(file_name) = path.file_name() else {
        return normalize_lexical(path).unwrap_or_else(|_| path.to_path_buf());
    };
    match std::fs::canonicalize(parent) {
        Ok(canonical_parent) => dunce::simplified(&canonical_parent).join(file_name),
        Err(_) => normalize_lexical(path).unwrap_or_else(|_| path.to_path_buf()),
    }
}

fn normalized_path_key(path: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(path)
        .map(|path| dunce::simplified(&path).to_path_buf())
        .or_else(|_| normalize_lexical(path).map(|path| dunce::simplified(&path).to_path_buf()))
}

fn normalize_lexical(path: &Path) -> Result<PathBuf, String> {
    let mut normalized = if path.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir().map_err(|e| format!("Failed to read current dir: {}", e))?
    };
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    Ok(normalized)
}

fn sanitize_branch_component(value: &str) -> String {
    let component = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if component.is_empty() {
        "chat".to_string()
    } else {
        component.chars().take(32).collect()
    }
}

fn default_branch_name(kind: &str, chat_id: Option<&str>) -> String {
    let seed = chat_id
        .map(sanitize_branch_component)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let short = seed.chars().take(12).collect::<String>();
    format!("refact/{}/{}", sanitize_branch_component(kind), short)
}

fn request_to_reference(kind: &str, request: &CreateWorktreeRequest) -> Option<WorktreeReference> {
    let reference = WorktreeReference {
        kind: kind.to_string(),
        chat_id: request.chat_id.clone(),
        task_id: request.task_id.clone(),
        card_id: request.card_id.clone(),
        agent_id: request.agent_id.clone(),
    };
    reference.has_identity().then_some(reference)
}

fn reference_key(reference: &WorktreeReference) -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}",
        reference.kind,
        reference.chat_id.as_deref().unwrap_or_default(),
        reference.task_id.as_deref().unwrap_or_default(),
        reference.card_id.as_deref().unwrap_or_default(),
        reference.agent_id.as_deref().unwrap_or_default()
    )
}

fn fallback_merge_message(record: &WorktreeRegistryRecord, branch: &str) -> String {
    if let Some(card_id) = record.meta.card_id.as_deref() {
        format!("Merge worktree {} for card {}", branch, card_id)
    } else if let Some(task_id) = record.meta.task_id.as_deref() {
        format!("Merge worktree {} for task {}", branch, task_id)
    } else {
        format!("Merge worktree {}", branch)
    }
}

fn format_merge_failure(prefix: &str, error: &str, cleanup_warnings: Vec<String>) -> String {
    if cleanup_warnings.is_empty() {
        format!("{}: {}", prefix, error)
    } else {
        format!(
            "{}: {}; cleanup warnings: {}",
            prefix,
            error,
            cleanup_warnings.join("; ")
        )
    }
}

fn conflict_state(
    files: Vec<String>,
    aborted: bool,
    merge_in_progress: bool,
) -> WorktreeConflictState {
    WorktreeConflictState {
        files,
        aborted,
        merge_in_progress,
        instructions: if aborted {
            "Merge conflicts were detected during preflight or the merge was aborted and cleaned up; no conflict markers were left behind. Merge the source branch changes against the target branch yourself, then retry.".to_string()
        } else {
            "Merge conflicts remain in the target workspace; resolve or abort the merge before retrying.".to_string()
        },
    }
}

fn validate_cleanup_request(request: &WorktreeCleanupRequest) -> Result<(), String> {
    if request.ids.is_empty() {
        return Err("Cleanup requires explicit worktree ids".to_string());
    }
    for id in &request.ids {
        validate_worktree_id(id)?;
    }
    Ok(())
}

fn cleanup_blockers_for_item(
    item: &WorktreeInspection,
    request: &WorktreeCleanupRequest,
) -> Vec<String> {
    let mut blockers = Vec::new();
    if !item.status.path_exists {
        blockers.push("missing_path".to_string());
    }
    if !item.status.is_git_worktree {
        blockers.push("not_git_worktree".to_string());
    }
    if item.conflicted {
        blockers.push("conflicted".to_string());
    }
    if item.base_commit.is_none() && item.base_branch.is_none() {
        blockers.push("base_unknown".to_string());
    }
    if item.status.error.is_some() {
        blockers.push("diff_error".to_string());
    }
    if request.clean_only && (item.changed_files > 0 || item.status.dirty) {
        blockers.push("dirty".to_string());
    }
    if !request.allow_shared && item.shared {
        blockers.push("shared".to_string());
    }
    if item.reference_count > 0 {
        blockers.push("referenced".to_string());
    }
    match item.age_hours {
        Some(age) if age < request.min_age_hours => blockers.push("too_recent".to_string()),
        None if request.min_age_hours > 0 => blockers.push("age_unknown".to_string()),
        _ => {}
    }
    if request.delete_branches {
        match (&item.branch, item.branch_merged) {
            (Some(_), Some(true)) => {}
            (Some(_), Some(false)) => blockers.push("branch_not_merged".to_string()),
            (Some(_), None) => blockers.push("branch_safety_unknown".to_string()),
            (None, _) => {}
        }
    }
    blockers
}

fn summarize_inventory(worktrees: &[WorktreeInspection]) -> WorktreeInventorySummary {
    let mut summary = WorktreeInventorySummary::default();
    let mut disk_usage = 0u64;
    let mut has_disk_usage = false;
    for item in worktrees {
        if item.source == "registered" {
            summary.total_registered += 1;
        } else {
            summary.total_discovered += 1;
        }
        if item.stale {
            summary.stale += 1;
        }
        if item.conflicted {
            summary.conflicted += 1;
        }
        if item.shared {
            summary.shared += 1;
        }
        let unknown = item.status.error.is_some()
            || (item.base_commit.is_none() && item.base_branch.is_none());
        if unknown {
            summary.unknown += 1;
        }
        if item.changed_files > 0 || item.status.dirty {
            summary.dirty += 1;
        } else if !item.stale && !item.conflicted && !unknown {
            summary.clean += 1;
        }
        if item.cleanup_candidate {
            summary.abandoned_clean += 1;
        }
        if item.source == "registered" && !item.status.path_exists {
            summary.missing_registry_paths += 1;
        }
        if item.cache_dir_missing_from_registry {
            summary.unregistered_cache_dirs += 1;
        }
        if item.branch_merged == Some(true) {
            summary.merged_branches += 1;
        }
        summary.changed_files += item.changed_files;
        summary.additions += item.additions;
        summary.deletions += item.deletions;
        if let Some(bytes) = item.disk_usage_bytes {
            has_disk_usage = true;
            disk_usage = disk_usage.saturating_add(bytes);
        }
        if let Some(age) = item.age_hours {
            summary.newest_age_hours = Some(
                summary
                    .newest_age_hours
                    .map(|current| current.min(age))
                    .unwrap_or(age),
            );
            summary.oldest_age_hours = Some(
                summary
                    .oldest_age_hours
                    .map(|current| current.max(age))
                    .unwrap_or(age),
            );
        }
    }
    summary.total = worktrees.len();
    if has_disk_usage {
        summary.disk_usage_bytes = Some(disk_usage);
    }
    summary
}

fn worktree_id_from_path(path: &Path) -> String {
    if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
        if validate_worktree_id(name).is_ok() {
            return name.to_string();
        }
    }
    format!("discovered_{}", project_hash_for_path(path))
}

fn age_hours_since(timestamp: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|dt| Utc::now().signed_duration_since(dt.with_timezone(&Utc)))
        .map(|duration| duration.num_hours().max(0) as u64)
}

fn filesystem_age_hours(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    std::time::SystemTime::now()
        .duration_since(modified)
        .ok()
        .map(|duration| duration.as_secs() / 3600)
}

fn cheap_disk_usage(path: &Path) -> Option<u64> {
    if !path.exists() {
        return None;
    }
    let mut total = 0u64;
    let mut seen = 0usize;
    let mut stack = vec![path.to_path_buf()];
    while let Some(path) = stack.pop() {
        seen += 1;
        if seen > 5000 {
            return None;
        }
        let metadata = std::fs::symlink_metadata(&path).ok()?;
        if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        } else if metadata.is_dir() {
            for entry in std::fs::read_dir(&path).ok()? {
                stack.push(entry.ok()?.path());
            }
        }
    }
    Some(total)
}

fn attached_chat_ids(references: &[WorktreeReference]) -> Vec<String> {
    let mut ids = references
        .iter()
        .filter_map(|reference| reference.chat_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

fn attached_task_ids(references: &[WorktreeReference]) -> Vec<String> {
    let mut ids = references
        .iter()
        .filter_map(|reference| reference.task_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

#[cfg(test)]
mod worktree_registry_tests {
    use std::process::Command;

    use super::*;
    use crate::worktrees::types::WorktreeMergeStrategy;

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
        run_git(root, &["config", "core.autocrlf", "false"]);
        run_git(root, &["config", "user.email", "test@example.com"]);
        run_git(root, &["config", "user.name", "Test User"]);
        std::fs::write(root.join("file.txt"), "hello\n").unwrap();
        run_git(root, &["add", "file.txt"]);
        run_git(root, &["commit", "-m", "initial"]);
    }

    fn unset_user_config(root: &Path) {
        let _ = Command::new("git")
            .args(["config", "--unset", "user.email"])
            .current_dir(root)
            .output();
        let _ = Command::new("git")
            .args(["config", "--unset", "user.name"])
            .current_dir(root)
            .output();
    }

    fn commit_file(root: &Path, file: &str, content: &str, message: &str) {
        std::fs::write(root.join(file), content).unwrap();
        run_git(root, &["add", file]);
        run_git(root, &["commit", "-m", message]);
    }

    fn branch_exists(root: &Path, branch: &str) -> bool {
        !run_git(root, &["branch", "--list", branch])
            .trim()
            .is_empty()
    }

    fn head_parent_count(root: &Path) -> usize {
        run_git(root, &["rev-list", "--parents", "-n", "1", "HEAD"])
            .split_whitespace()
            .count()
            .saturating_sub(1)
    }

    fn sample_record(service: &WorktreeService, id: &str) -> WorktreeRegistryRecord {
        let now = Utc::now().to_rfc3339();
        WorktreeRegistryRecord {
            meta: WorktreeMeta {
                id: id.to_string(),
                kind: "chat".to_string(),
                root: service.worktree_path_for_id(id).unwrap(),
                source_workspace_root: service.source_workspace_root().to_path_buf(),
                repo_root: service.source_workspace_root().to_path_buf(),
                branch: Some(format!("refact/chat/{}", id)),
                base_branch: Some("main".to_string()),
                base_commit: Some("abc".to_string()),
                task_id: None,
                card_id: None,
                agent_id: None,
                enforce: true,
            },
            created_at: now.clone(),
            updated_at: now,
            last_seen_at: None,
            references: Vec::new(),
            last_known_status: None,
        }
    }

    #[tokio::test]
    async fn worktree_registry_load_save_update_with_temp_dirs() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        let service = WorktreeService::new(cache, source).unwrap();
        let mut registry = service.load_registry().await.unwrap();
        assert!(registry.records.is_empty());
        registry.records.push(sample_record(&service, "wt_1"));
        service.save_registry(&registry).await.unwrap();
        let loaded = service.load_registry().await.unwrap();
        assert_eq!(loaded.records.len(), 1);
        let reference = WorktreeReference {
            kind: "chat".to_string(),
            chat_id: Some("chat-1".to_string()),
            task_id: None,
            card_id: None,
            agent_id: None,
        };
        let view = service.add_reference("wt_1", reference).await.unwrap();
        assert_eq!(view.reference_count, 1);
        let loaded = service.load_registry().await.unwrap();
        assert_eq!(loaded.records[0].references.len(), 1);
    }

    #[tokio::test]
    async fn worktree_registry_multiple_references_share_metadata_without_duplication() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        let service = WorktreeService::new(cache, source).unwrap();
        let mut registry = service.load_registry().await.unwrap();
        registry.records.push(sample_record(&service, "wt_1"));
        service.save_registry(&registry).await.unwrap();
        let first = WorktreeReference {
            kind: "chat".to_string(),
            chat_id: Some("chat-1".to_string()),
            task_id: None,
            card_id: None,
            agent_id: None,
        };
        let second = WorktreeReference {
            kind: "chat".to_string(),
            chat_id: Some("chat-2".to_string()),
            task_id: None,
            card_id: None,
            agent_id: None,
        };
        service.add_reference("wt_1", first.clone()).await.unwrap();
        service.add_reference("wt_1", first).await.unwrap();
        let view = service.add_reference("wt_1", second).await.unwrap();
        assert_eq!(view.meta.id, "wt_1");
        assert_eq!(view.reference_count, 2);
        assert_eq!(service.load_registry().await.unwrap().records.len(), 1);
    }

    #[tokio::test]
    async fn worktree_registry_create_list_get_delete_normal_and_stale_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                chat_id: Some("chat-1".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(created.worktree.meta.root.is_dir());
        assert_eq!(service.list_worktrees().await.unwrap().worktrees.len(), 1);
        let got = service
            .get_worktree(&created.worktree.meta.id)
            .await
            .unwrap();
        assert_eq!(got.meta.id, created.worktree.meta.id);
        let deleted = service
            .delete_worktree(&created.worktree.meta.id, true)
            .await
            .unwrap();
        assert!(deleted.deleted);
        assert!(!created.worktree.meta.root.exists());
        assert_eq!(service.list_worktrees().await.unwrap().worktrees.len(), 0);

        let mut registry = service.load_registry().await.unwrap();
        registry.records.push(sample_record(&service, "stale_1"));
        service.save_registry(&registry).await.unwrap();
        let deleted = service.delete_worktree("stale_1", false).await.unwrap();
        assert!(deleted.deleted);
        assert!(deleted.stale_path);
        assert_eq!(service.list_worktrees().await.unwrap().worktrees.len(), 0);
    }

    #[tokio::test]
    async fn worktree_registry_create_without_identity_has_no_reference() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/unreferenced-create".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();

        let view = service
            .get_worktree(&created.worktree.meta.id)
            .await
            .unwrap();

        assert_eq!(view.reference_count, 0);
        assert!(view.references.is_empty());
    }

    #[tokio::test]
    async fn worktree_registry_create_without_base_uses_current_branch_head() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        run_git(&source, &["checkout", "-b", "dev"]);
        commit_file(&source, "dev_only.txt", "only on dev\n", "dev-only");
        let dev_head = run_git(&source, &["rev-parse", "HEAD"]).trim().to_string();
        let service = WorktreeService::new(cache, source).unwrap();

        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/current-branch".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(created.worktree.meta.base_branch.as_deref(), Some("dev"));
        assert_eq!(
            created.worktree.meta.base_commit.as_deref(),
            Some(dev_head.as_str())
        );
        assert!(created.worktree.meta.root.join("dev_only.txt").is_file());
    }

    #[tokio::test]
    async fn worktree_registry_create_rejects_existing_branch_reuse() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        run_git(&source, &["branch", "refact/chat/reused"]);
        run_git(&source, &["checkout", "-b", "dev"]);
        commit_file(&source, "dev_only.txt", "only on dev\n", "dev-only");
        let service = WorktreeService::new(cache, source).unwrap();

        let error = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/reused".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap_err();

        assert!(error.contains("already exists"));
    }

    #[tokio::test]
    async fn worktree_registry_create_cleans_branch_after_worktree_failure() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let invalid_worktree_path = source.join("file.txt");

        let error = match git::create_worktree(
            &source,
            &invalid_worktree_path,
            "cleanup-failure",
            "refact/chat/cleanup-failure",
            None,
        ) {
            Ok(_) => panic!("worktree creation unexpectedly succeeded"),
            Err(error) => error,
        };

        assert!(error.contains("Failed to create worktree"));
        assert!(!branch_exists(&source, "refact/chat/cleanup-failure"));
    }

    #[tokio::test]
    async fn worktree_registry_list_detached_head_has_no_source_branch() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let head = run_git(&source, &["rev-parse", "HEAD"]);
        run_git(&source, &["checkout", "--detach", head.trim()]);
        let service = WorktreeService::new(cache, source).unwrap();

        let list = service.list_worktrees().await.unwrap();

        assert!(list.source_current_branch.is_none());
    }

    #[tokio::test]
    async fn worktree_registry_diff_returns_changed_files_and_patch() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/diff-test".to_string()),
                chat_id: Some("diff-chat".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let root = created.worktree.meta.root.clone();
        std::fs::write(root.join("file.txt"), "hello committed\n").unwrap();
        run_git(&root, &["add", "file.txt"]);
        run_git(&root, &["commit", "-m", "change tracked"]);
        std::fs::write(root.join("untracked.txt"), "new content\n").unwrap();
        let diff = service
            .diff_worktree_with_limit(&created.worktree.meta.id, 50_000)
            .await
            .unwrap();
        assert!(diff.files.iter().any(|file| file.path == "file.txt"));
        assert!(diff.files.iter().any(|file| file.path == "untracked.txt"));
        assert!(diff.patch.contains("hello committed"));
        assert!(diff.patch.contains("new content"));
        assert!(!diff.patch_truncated);
    }

    #[tokio::test]
    async fn worktree_merge_diff_returns_committed_branch_patch() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/merge-diff-committed".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "committed change\n",
            "committed change",
        );

        let diff = service
            .diff_worktree_with_limit(&created.worktree.meta.id, 50_000)
            .await
            .unwrap();

        assert!(diff.files.iter().any(|file| {
            file.path == "file.txt" && file.source == "committed" && file.additions == Some(1)
        }));
        assert!(diff.patch.contains("committed change"));
        assert!(!diff.status.dirty);
    }

    #[tokio::test]
    async fn worktree_merge_diff_includes_uncommitted_change() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/merge-diff-uncommitted".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        std::fs::write(
            created.worktree.meta.root.join("file.txt"),
            "dirty change\n",
        )
        .unwrap();
        std::fs::write(created.worktree.meta.root.join("new.txt"), "new dirty\n").unwrap();

        let diff = service
            .diff_worktree_with_limit(&created.worktree.meta.id, 50_000)
            .await
            .unwrap();

        assert!(diff.status.dirty);
        assert!(diff
            .files
            .iter()
            .any(|file| file.path == "file.txt" && file.source == "unstaged"));
        assert!(diff
            .files
            .iter()
            .any(|file| file.path == "new.txt" && file.source == "untracked"));
        assert!(diff.patch.contains("dirty change"));
        assert!(diff.patch.contains("new dirty"));
    }

    #[tokio::test]
    async fn worktree_merge_diff_falls_back_to_base_branch_when_base_commit_missing() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/missing-base-commit".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "branch change\n",
            "branch change",
        );
        let mut registry = service.load_registry().await.unwrap();
        registry.records[0].meta.base_commit = None;
        service.save_registry(&registry).await.unwrap();

        let diff = service
            .diff_worktree_with_limit(&created.worktree.meta.id, 50_000)
            .await
            .unwrap();

        assert!(diff.patch.contains("branch change"));
        assert_eq!(diff.stats.committed_files, 1);
    }

    #[tokio::test]
    async fn worktree_merge_diff_invalid_base_blocks_diff_and_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/invalid-base".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &created.worktree.meta.id, 48).await;
        let mut registry = service.load_registry().await.unwrap();
        registry.records[0].meta.base_commit = Some("not-a-commit".to_string());
        service.save_registry(&registry).await.unwrap();

        assert!(service
            .diff_worktree_with_limit(&created.worktree.meta.id, 50_000)
            .await
            .is_err());
        let plan = service
            .cleanup_worktrees_dry_run(WorktreeCleanupRequest {
                ids: vec![created.worktree.meta.id.clone()],
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(plan.candidates.is_empty());
        assert!(plan.skipped.iter().any(|item| item.reason == "diff_error"));
        let inventory = service.inspect_worktrees_with_min_age(24).await.unwrap();
        assert_eq!(inventory.summary.clean, 0);
        assert_eq!(inventory.summary.unknown, 1);
    }

    #[tokio::test]
    async fn worktree_merge_diff_large_untracked_file_truncates_safely() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/large-untracked".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        std::fs::write(
            created.worktree.meta.root.join("large.txt"),
            "x".repeat(300_000),
        )
        .unwrap();

        let diff = service
            .diff_worktree_with_limit(&created.worktree.meta.id, 1_000)
            .await
            .unwrap();

        assert!(diff.files.iter().any(|file| file.path == "large.txt"));
        assert!(diff.patch_truncated);
        assert!(diff.patch.len() <= 1_128);
    }

    #[tokio::test]
    async fn worktree_merge_squash_creates_single_parent_change_on_base() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/squash-merge".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "squash change\n",
            "agent change",
        );

        let merged = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("squash merge".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(merged.merged);
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "squash change\n"
        );
        assert_eq!(head_parent_count(&source), 1);
        assert!(created.worktree.meta.root.exists());
        assert!(service
            .get_worktree(&created.worktree.meta.id)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn worktree_merge_regular_no_ff_merge_works() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/regular-merge".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "regular change\n",
            "agent change",
        );

        let merged = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Merge,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("regular merge".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(merged.merged);
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "regular change\n"
        );
        assert_eq!(head_parent_count(&source), 2);
    }

    #[tokio::test]
    async fn worktree_merge_squash_uses_refact_author_without_repo_identity() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/no-identity".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "identity change\n",
            "agent change",
        );
        unset_user_config(&source);

        let merged = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("identity merge".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(merged.merged);
        assert_eq!(
            run_git(&source, &["log", "-1", "--format=%an <%ae>"]).trim(),
            "Refact Agent <agent@refact.ai>"
        );
    }

    #[tokio::test]
    async fn worktree_merge_squash_commit_failure_resets_target_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/hook-failure".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "hook change\n",
            "agent change",
        );
        let hook = source.join(".git/hooks/pre-commit");
        std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&hook).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&hook, perms).unwrap();
        }

        let error = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("hook failure".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();

        assert!(error.contains("Failed to commit squash merge"));
        git::ensure_clean_worktree(&source, "Target workspace").unwrap();
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "hello\n"
        );
    }

    #[tokio::test]
    async fn worktree_merge_defaults_to_delete_after_merge_and_reports_references() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/delete-after-merge".to_string()),
                kind: Some("task_agent".to_string()),
                chat_id: Some("chat-1".to_string()),
                task_id: Some("task-1".to_string()),
                card_id: Some("T-1".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        service
            .add_reference(
                &created.worktree.meta.id,
                WorktreeReference {
                    kind: "chat".to_string(),
                    chat_id: Some("chat-2".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "delete after merge\n",
            "agent change",
        );
        let branch = created.worktree.meta.branch.clone().unwrap();
        let root = created.worktree.meta.root.clone();

        let merged = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("delete after merge".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let cleanup = merged.cleanup.unwrap();
        assert!(cleanup.worktree_deleted);
        assert!(cleanup.registry_deleted);
        assert!(cleanup.branch_deleted);
        assert_eq!(merged.affected_reference_count, 2);
        assert!(!root.exists());
        assert!(!branch_exists(&source, &branch));
        assert!(service
            .get_worktree(&created.worktree.meta.id)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn worktree_merge_discard_delete_unmerged_worktree_removes_path_and_branch() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/delete-unmerged".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "unmerged change\n",
            "agent change",
        );
        let branch = created.worktree.meta.branch.clone().unwrap();
        let root = created.worktree.meta.root.clone();

        let deleted = service
            .delete_worktree(&created.worktree.meta.id, true)
            .await
            .unwrap();

        assert!(deleted.deleted);
        assert!(deleted.branch_deleted);
        assert!(!root.exists());
        assert!(!branch_exists(&source, &branch));
        assert!(service
            .get_worktree(&created.worktree.meta.id)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn worktree_merge_conflict_returns_files_and_keeps_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/conflict-merge".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(&source, "file.txt", "target change\n", "target change");
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "source change\n",
            "source change",
        );

        let merged = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Merge,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("conflict merge".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(merged.status, "conflict");
        assert!(!merged.merged);
        assert!(merged
            .conflict
            .unwrap()
            .files
            .contains(&"file.txt".to_string()));
        assert!(created.worktree.meta.root.exists());
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "target change\n"
        );
        assert!(service
            .get_worktree(&created.worktree.meta.id)
            .await
            .is_ok());
    }

    async fn mark_worktree_old(service: &WorktreeService, id: &str, hours: i64) {
        let mut registry = service.load_registry().await.unwrap();
        let record = registry
            .records
            .iter_mut()
            .find(|record| record.meta.id == id)
            .unwrap();
        let ts = (Utc::now() - chrono::Duration::hours(hours)).to_rfc3339();
        record.created_at = ts.clone();
        record.updated_at = ts.clone();
        record.last_seen_at = Some(ts);
        service.save_registry(&registry).await.unwrap();
    }

    #[tokio::test]
    async fn worktree_hygiene_classifies_clean_dirty_stale_shared_and_conflicted() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source.clone()).unwrap();
        let clean = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/hygiene-clean".to_string()),
                chat_id: Some("chat-clean".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &clean.worktree.meta.id, 48).await;
        service
            .add_reference(
                &clean.worktree.meta.id,
                WorktreeReference {
                    kind: "chat".to_string(),
                    chat_id: Some("chat-clean-2".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let dirty = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/hygiene-dirty".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &dirty.worktree.meta.id, 48).await;
        std::fs::write(dirty.worktree.meta.root.join("dirty.txt"), "dirty\n").unwrap();
        let conflicted = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/hygiene-conflict".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &conflicted.worktree.meta.id, 48).await;
        commit_file(&source, "file.txt", "target\n", "target");
        commit_file(
            &conflicted.worktree.meta.root,
            "file.txt",
            "source\n",
            "source",
        );
        let _ = Command::new("git")
            .args(["merge", "main"])
            .current_dir(&conflicted.worktree.meta.root)
            .output()
            .unwrap();
        let mut registry = service.load_registry().await.unwrap();
        registry.records.push(sample_record(&service, "stale_1"));
        service.save_registry(&registry).await.unwrap();

        let inventory = service.inspect_worktrees_with_min_age(24).await.unwrap();

        assert_eq!(inventory.summary.total_registered, 4);
        assert!(inventory.summary.dirty >= 1);
        assert!(inventory.summary.stale >= 1);
        assert!(inventory.summary.conflicted >= 1);
        assert!(inventory.summary.shared >= 1);
        assert!(inventory
            .worktrees
            .iter()
            .any(|item| item.id == dirty.worktree.meta.id && item.changed_files > 0));
    }

    #[tokio::test]
    async fn worktree_hygiene_changed_file_count_aggregation_and_candidate_selection() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source).unwrap();
        let clean = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/hygiene-candidate".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &clean.worktree.meta.id, 48).await;
        let dirty = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/hygiene-aggregate".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &dirty.worktree.meta.id, 48).await;
        std::fs::write(dirty.worktree.meta.root.join("a.txt"), "a\n").unwrap();
        std::fs::write(dirty.worktree.meta.root.join("b.txt"), "b\n").unwrap();

        let inventory = service.inspect_worktrees_with_min_age(24).await.unwrap();

        assert!(inventory.summary.changed_files >= 2);
        assert!(inventory.summary.additions >= 2);
        assert!(inventory
            .cleanup_candidates
            .contains(&clean.worktree.meta.id));
        assert!(!inventory
            .cleanup_candidates
            .contains(&dirty.worktree.meta.id));
    }

    #[tokio::test]
    async fn worktree_hygiene_cleanup_dry_run_does_not_delete_and_blocks_shared_dirty() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source).unwrap();
        let clean = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/hygiene-dry-clean".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &clean.worktree.meta.id, 48).await;
        let shared = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/hygiene-dry-shared".to_string()),
                chat_id: Some("chat-shared".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &shared.worktree.meta.id, 48).await;
        service
            .add_reference(
                &shared.worktree.meta.id,
                WorktreeReference {
                    kind: "chat".to_string(),
                    chat_id: Some("chat-shared-2".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let dirty = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/hygiene-dry-dirty".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &dirty.worktree.meta.id, 48).await;
        std::fs::write(dirty.worktree.meta.root.join("dirty.txt"), "dirty\n").unwrap();

        let plan = service
            .cleanup_worktrees_dry_run(WorktreeCleanupRequest {
                ids: vec![
                    clean.worktree.meta.id.clone(),
                    shared.worktree.meta.id.clone(),
                    dirty.worktree.meta.id.clone(),
                ],
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(plan.candidates.len(), 1);
        assert_eq!(plan.candidates[0].id, clean.worktree.meta.id);
        assert!(plan.skipped.iter().any(|item| item.reason == "shared"));
        assert!(plan.skipped.iter().any(|item| item.reason == "dirty"));
        assert!(clean.worktree.meta.root.exists());
        assert!(service.get_worktree(&clean.worktree.meta.id).await.is_ok());
    }

    #[tokio::test]
    async fn worktree_hygiene_cleanup_actual_deletes_only_selected_safe_worktrees() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source.clone()).unwrap();
        let clean = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/hygiene-delete-clean".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &clean.worktree.meta.id, 48).await;
        let dirty = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/hygiene-delete-dirty".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &dirty.worktree.meta.id, 48).await;
        std::fs::write(dirty.worktree.meta.root.join("dirty.txt"), "dirty\n").unwrap();
        let clean_root = clean.worktree.meta.root.clone();
        let dirty_root = dirty.worktree.meta.root.clone();
        let branch = clean.worktree.meta.branch.clone().unwrap();

        let result = service
            .cleanup_worktrees(WorktreeCleanupRequest {
                ids: vec![
                    clean.worktree.meta.id.clone(),
                    dirty.worktree.meta.id.clone(),
                ],
                delete_branches: true,
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(result.deleted.len(), 1);
        assert_eq!(result.deleted[0].id, clean.worktree.meta.id);
        assert!(result.deleted[0].worktree_deleted);
        assert!(result.deleted[0].branch_deleted);
        assert!(result.skipped.iter().any(|item| item.reason == "dirty"));
        assert!(!clean_root.exists());
        assert!(dirty_root.exists());
        assert!(!branch_exists(&source, &branch));
        assert!(service.get_worktree(&clean.worktree.meta.id).await.is_err());
    }

    #[tokio::test]
    async fn worktree_hygiene_cleanup_blocks_single_referenced_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source).unwrap();
        let referenced = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/referenced-clean".to_string()),
                chat_id: Some("chat-referenced".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &referenced.worktree.meta.id, 48).await;

        let plan = service
            .cleanup_worktrees_dry_run(WorktreeCleanupRequest {
                ids: vec![referenced.worktree.meta.id.clone()],
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(plan.candidates.is_empty());
        assert!(plan.skipped.iter().any(|item| item.reason == "referenced"));
    }

    #[tokio::test]
    async fn worktree_hygiene_allow_shared_still_blocks_single_reference() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source).unwrap();
        let referenced = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/allow-shared-single".to_string()),
                chat_id: Some("chat-referenced".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &referenced.worktree.meta.id, 48).await;

        let plan = service
            .cleanup_worktrees_dry_run(WorktreeCleanupRequest {
                ids: vec![referenced.worktree.meta.id.clone()],
                allow_shared: true,
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(plan.candidates.is_empty());
        assert!(plan.skipped.iter().any(|item| item.reason == "referenced"));
    }

    #[tokio::test]
    async fn worktree_hygiene_allow_shared_still_blocks_shared_references() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source).unwrap();
        let referenced = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/allow-shared-many".to_string()),
                chat_id: Some("chat-referenced".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        service
            .add_reference(
                &referenced.worktree.meta.id,
                WorktreeReference {
                    kind: "chat".to_string(),
                    chat_id: Some("chat-referenced-2".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        mark_worktree_old(&service, &referenced.worktree.meta.id, 48).await;

        let plan = service
            .cleanup_worktrees_dry_run(WorktreeCleanupRequest {
                ids: vec![referenced.worktree.meta.id.clone()],
                allow_shared: true,
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(plan.candidates.is_empty());
        assert!(plan.skipped.iter().any(|item| item.reason == "referenced"));
    }

    #[tokio::test]
    async fn worktree_hygiene_clean_only_blocks_status_dirty_even_without_diff_stats() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = WorktreeService::new(cache, source).unwrap();
        let dirty = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/status-dirty".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &dirty.worktree.meta.id, 48).await;
        std::fs::write(dirty.worktree.meta.root.join("dirty.txt"), "dirty\n").unwrap();
        let mut registry = service.load_registry().await.unwrap();
        registry.records[0].meta.base_commit = None;
        registry.records[0].meta.base_branch = None;
        service.save_registry(&registry).await.unwrap();

        let plan = service
            .cleanup_worktrees_dry_run(WorktreeCleanupRequest {
                ids: vec![dirty.worktree.meta.id.clone()],
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(plan.candidates.is_empty());
        assert!(plan
            .skipped
            .iter()
            .any(|item| item.details.contains(&"dirty".to_string())));
    }

    #[tokio::test]
    async fn worktree_registry_invalid_ids_paths_and_non_git_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        let service = WorktreeService::new(cache, source).unwrap();
        assert!(service.get_worktree("../bad").await.is_err());
        assert!(service.worktree_path_for_id("../bad").is_err());
        assert!(service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("../bad".to_string()),
                ..Default::default()
            })
            .await
            .is_err());
        assert!(service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/non-git".to_string()),
                ..Default::default()
            })
            .await
            .unwrap_err()
            .contains("not a git repository"));
        let mut registry = service.load_registry().await.unwrap();
        let mut record = sample_record(&service, "wt_1");
        record.meta.root = temp.path().join("outside");
        registry.records.push(record);
        assert!(service.save_registry(&registry).await.is_err());
        let mut registry = service.load_registry().await.unwrap();
        let mut record = sample_record(&service, "wt_2");
        record.meta.root = service.registry_dir();
        registry.records.push(record);
        let error = service.save_registry(&registry).await.unwrap_err();
        assert!(error.contains("must exactly match registry path"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn worktree_registry_rejects_symlink_escape_root() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let cache = temp.path().join("cache");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let service = WorktreeService::new(cache, source).unwrap();
        std::fs::create_dir_all(service.registry_dir()).unwrap();
        std::os::unix::fs::symlink(&outside, service.registry_dir().join("wt_1")).unwrap();
        let mut registry = service.load_registry().await.unwrap();
        let mut record = sample_record(&service, "wt_1");
        record.meta.root = outside;
        registry.records.push(record);

        let error = service.save_registry(&registry).await.unwrap_err();

        assert!(error.contains("must exactly match registry path"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn worktree_registry_rejects_expected_root_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let cache = temp.path().join("cache");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let service = WorktreeService::new(cache, source).unwrap();
        std::fs::create_dir_all(service.registry_dir()).unwrap();
        std::os::unix::fs::symlink(&outside, service.registry_dir().join("wt_1")).unwrap();
        let mut registry = service.load_registry().await.unwrap();
        registry.records.push(sample_record(&service, "wt_1"));

        let error = service.save_registry(&registry).await.unwrap_err();

        assert!(error.contains("must not be a symlink"));
    }
}
