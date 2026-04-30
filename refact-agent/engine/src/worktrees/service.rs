use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use chrono::Utc;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;

use super::git;
use super::types::{
    CreateWorktreeRequest, CreateWorktreeResponse, DeleteWorktreeResponse, MergeWorktreeRequest,
    MergeWorktreeResponse, OpenWorktreeResponse, WorktreeCleanupResult, WorktreeConflictState,
    WorktreeDiffResponse, WorktreeListResponse, WorktreeMeta, WorktreeRecordView,
    WorktreeReference, WorktreeRegistry, WorktreeRegistryRecord,
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
        Ok(WorktreeListResponse {
            project_hash: self.project_hash.clone(),
            source_workspace_root: self.source_workspace_root.clone(),
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
            root: worktree_path.clone(),
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
            git::run_git(
                &record.meta.source_workspace_root,
                &["merge", "--no-ff", &source_branch, "-m", &commit_message],
            )
        };
        if let Err(e) = merge_result {
            let conflict_files = git::conflict_files_for_path(&record.meta.source_workspace_root);
            if !conflict_files.is_empty() {
                let aborted = git::abort_merge(&record.meta.source_workspace_root);
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
                    warnings: Vec::new(),
                });
            }
            return Err(format!("Merge failed: {}", e));
        }
        if request.strategy.as_str() == "squash" {
            git::run_git(
                &record.meta.source_workspace_root,
                &["commit", "-m", &commit_message, "--no-gpg-sign"],
            )
            .map_err(|e| format!("Failed to commit squash merge: {}", e))?;
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

    async fn cleanup_registered_worktree(
        &self,
        registry: &mut WorktreeRegistry,
        index: usize,
        record: &WorktreeRegistryRecord,
        delete_branch: bool,
    ) -> Result<WorktreeCleanupResult, String> {
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
        Ok(WorktreeCleanupResult {
            worktree_deleted,
            branch_deleted,
            registry_deleted,
            stale_path,
            warnings,
        })
    }

    fn record_view(&self, record: &WorktreeRegistryRecord) -> Result<WorktreeRecordView, String> {
        validate_worktree_id(&record.meta.id)?;
        validate_registry_root(&self.registry_dir(), &record.meta.root)?;
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
        let registry_root = normalize_lexical(&registry.source_workspace_root)?;
        if registry_root != self.source_workspace_root {
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
            validate_registry_root(&self.registry_dir(), &record.meta.root)?;
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

fn validate_registry_root(registry_dir: &Path, root: &Path) -> Result<(), String> {
    let registry_dir = normalize_lexical(registry_dir)?;
    let root = normalize_lexical(root)?;
    if !root.starts_with(&registry_dir) {
        return Err(format!(
            "Worktree root '{}' is outside registry directory '{}'",
            root.display(),
            registry_dir.display()
        ));
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
    Ok(canonical)
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
            "Merge conflicts were detected during preflight or were aborted; resolve the source branch against the target branch and retry.".to_string()
        } else {
            "Merge conflicts remain in the target workspace; resolve or abort the merge before retrying.".to_string()
        },
    }
}

#[cfg(test)]
mod worktree_registry_tests {
    use std::process::Command;

    use super::*;

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
        run_git(root, &["init", "-b", "main"]);
        run_git(root, &["config", "user.email", "test@example.com"]);
        run_git(root, &["config", "user.name", "Test User"]);
        std::fs::write(root.join("file.txt"), "hello\n").unwrap();
        run_git(root, &["add", "file.txt"]);
        run_git(root, &["commit", "-m", "initial"]);
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
    async fn worktree_merge_delete_after_merge_removes_registry_and_reports_references() {
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
                    delete_after_merge: true,
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
    }
}
