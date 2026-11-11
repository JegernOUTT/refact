use super::*;
use crate::types::*;

#[path = "sync_git.rs"]
mod native;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct RecoveryArchive {
    archive_id: String,
    path: String,
    digest: String,
    post_abort_fingerprint: Option<String>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema_version: u32,
    meta: WorktreeMeta,
    repository: PathBuf,
    git_dir: PathBuf,
    original_head: String,
    operation: WorktreeSyncOperation,
    status: WorktreeSyncStatus,
    scope: Vec<String>,
    abort_fingerprint: Option<String>,
    commit_tree: Option<String>,
    terminal_head: Option<String>,
    post_abort_fingerprint: Option<String>,
    checkpoint_tree: Option<String>,
    #[serde(default)]
    archive_digest: Option<String>,
    #[serde(default)]
    secondary_archives: Vec<String>,
    #[serde(default)]
    recovery_archives: Vec<RecoveryArchive>,
    #[serde(default)]
    restore_archive_id: Option<String>,
    #[serde(default)]
    restore_preimages: Vec<String>,
    #[serde(default)]
    restore_cursor: usize,
    #[serde(default)]
    restore_preimages_digest: Option<String>,
}

impl Journal {
    fn active(&self) -> bool {
        !matches!(
            self.status,
            WorktreeSyncStatus::Completed | WorktreeSyncStatus::Aborted
        )
    }
}

struct SyncCacheInvalidation(WorktreeService);

impl Drop for SyncCacheInvalidation {
    fn drop(&mut self) {
        self.0.invalidate_registry_cache();
    }
}

impl WorktreeService {
    pub async fn fence_sync_worktree(&self, id: &str, allow_missing: bool) -> Result<Self, String> {
        validate_worktree_id(id)?;
        let registry = self.load_registry_unlocked().await?;
        let record = registry
            .records
            .iter()
            .find(|r| r.meta.id == id)
            .cloned()
            .ok_or("Sync worktree is not registered")?;
        let checked = record.clone();
        let worker = self.clone();
        tokio::task::spawn_blocking(move || worker.sync_validate_record(&checked, allow_missing))
            .await
            .map_err(|e| e.to_string())??;
        let service = if let Some(token) = &self.effect_token {
            if !token.records.contains(&record.meta)
                || !token.fenced_roots.contains(&record.meta.root)
            {
                return Err("Existing effect token does not cover sync source identity".into());
            }
            self.clone()
        } else {
            if self.merge_permit.is_some() {
                return Err("Acquire sync effect fence before merge permit".into());
            }
            let fence = self
                .effect_fence
                .as_ref()
                .ok_or("Sync mutation requires an application effect coordinator")?;
            let roots = vec![record.meta.root.clone()];
            let guard = fence
                .begin(self.source_workspace_root.clone(), roots.clone())
                .await?;
            Self {
                effect_token: Some(Arc::new(EffectToken {
                    records: vec![record.meta.clone()],
                    includes_source: false,
                    archive: None,
                    legacy: None,
                    target: None,
                    target_materialized: Arc::new(AtomicBool::new(false)),
                    legacy_removed: Arc::new(AtomicBool::new(false)),
                    fenced_roots: roots,
                    _guard: guard,
                })),
                ..self.clone()
            }
        };
        let worker = service.clone();
        tokio::task::spawn_blocking(move || worker.sync_validate_record(&record, allow_missing))
            .await
            .map_err(|e| e.to_string())??;
        Ok(service)
    }

    fn sync_validate_record(
        &self,
        record: &WorktreeRegistryRecord,
        allow_missing: bool,
    ) -> Result<(), String> {
        let registry: WorktreeRegistry = serde_json::from_slice(
            &std::fs::read(self.registry_path()).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if registry.project_hash != self.project_hash
            || registry.source_workspace_root != self.source_workspace_root
            || !registry.records.iter().any(|r| r.meta == record.meta)
            || record.meta.source_workspace_root != self.source_workspace_root
            || record.meta.root == self.source_workspace_root
            || !record.meta.root.is_absolute()
            || record
                .meta
                .root
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err("Sync registered identity changed after effect drain".into());
        }
        if allow_missing && !record.meta.root.try_exists().map_err(|e| e.to_string())? {
            return Ok(());
        }
        validate_registered_record_checkout(record)?;
        if dunce::canonicalize(&record.meta.root).map_err(|e| e.to_string())? != record.meta.root {
            return Err("Sync canonical source identity changed".into());
        }
        Ok(())
    }

    pub fn with_sync_actor(&self, actor: String) -> Self {
        Self {
            sync_actor: Some(actor),
            ..self.clone()
        }
    }

    fn sync_path(&self, id: &str) -> Result<PathBuf, String> {
        validate_worktree_id(id)?;
        Ok(self.registry_dir().join("sync").join(format!("{id}.json")))
    }

    fn sync_journal(&self, id: &str) -> Result<Option<Journal>, String> {
        match std::fs::read(self.sync_path(id)?) {
            Ok(bytes) => {
                let mut journal: Journal = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("Sync journal unreadable; retain worktree: {e}"))?;
                self.sync_load_preimages(&mut journal)?;
                Ok(Some(journal))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn sync_load_preimages(&self, journal: &mut Journal) -> Result<(), String> {
        if let Some(digest) = &journal.restore_preimages_digest {
            if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("Invalid restore preimage binding".into());
            }
            let bytes = std::fs::read(
                self.registry_dir()
                    .join("sync")
                    .join(format!("preimages-{digest}.json")),
            )
            .map_err(|e| e.to_string())?;
            if format!("{:x}", Sha256::digest(&bytes)) != *digest {
                return Err("Restore preimages changed".into());
            }
            journal.restore_preimages =
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn sync_save(&self, journal: &Journal) -> Result<(), String> {
        let path = self.sync_path(&journal.meta.id)?;
        let mut compact = journal.clone();
        if !compact.restore_preimages.is_empty() {
            let bytes =
                serde_json::to_vec(&compact.restore_preimages).map_err(|e| e.to_string())?;
            let digest = format!("{:x}", Sha256::digest(&bytes));
            let preimages = path
                .parent()
                .ok_or("Missing sync directory")?
                .join(format!("preimages-{digest}.json"));
            if !preimages.exists() {
                native::durable_json(&preimages, &compact.restore_preimages)?;
            }
            compact.restore_preimages_digest = Some(digest);
            compact.restore_preimages.clear();
        } else {
            compact.restore_preimages_digest = None;
        }
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(previous) = serde_json::from_slice::<Journal>(&bytes) {
                native::durable_json(&path.with_extension("bak"), &previous)?;
            }
        }
        native::durable_json(&path, &compact)
    }

    fn sync_journal_digest(&self, id: &str) -> Result<String, String> {
        Ok(format!(
            "{:x}",
            Sha256::digest(std::fs::read(self.sync_path(id)?).map_err(|e| e.to_string())?)
        ))
    }

    fn sync_metadata_response(
        &self,
        id: &str,
        record: &WorktreeRegistryRecord,
        warnings: Vec<String>,
    ) -> WorktreeSyncResponse {
        WorktreeSyncResponse { restore_archive_id: None, recovery_archives: Vec::new(), recovery_journal_digest: self.sync_journal_digest(id).ok(), id: id.into(), status: WorktreeSyncStatus::RecoveryRequired, source_head: String::new(), target_branch: record.meta.base_branch.clone(), target_head: None, operation: None, conflicts: Vec::new(), warnings, instructions: "Recover with the exact recovery_journal_digest archives this metadata and restores a validated journal backup when possible. Recover never changes Git or checkout files; unknown Git state is retained and never replayed.".into() }
    }

    fn sync_recover_metadata(
        &self,
        id: &str,
        record: &WorktreeRegistryRecord,
        expected: &str,
    ) -> Result<WorktreeSyncResponse, String> {
        self.validate_merge_phase()?;
        if self.sync_journal_digest(id)? != expected {
            return Err("Journal changed; refresh Status before Recover".into());
        }
        let path = self.sync_path(id)?;
        let current = std::fs::read(&path).map_err(|e| e.to_string())?;
        let backup = std::fs::read(path.with_extension("bak")).ok();
        let quarantine = self
            .registry_dir()
            .join("sync")
            .join(format!("{id}-metadata-recovery-{}.json", Uuid::new_v4()));
        native::durable_json(
            &quarantine,
            &serde_json::json!({ "record": record, "journal_digest": expected, "journal_bytes": current, "backup_bytes": backup }),
        )?;
        self.validate_merge_phase()?;
        if self.sync_journal_digest(id)? != expected {
            return Err("Journal changed while archiving; no metadata replaced".into());
        }
        let parsed = serde_json::from_slice::<Journal>(&current).ok();
        let mut candidate = parsed.clone().or_else(|| {
            backup
                .as_ref()
                .and_then(|bytes| serde_json::from_slice::<Journal>(bytes).ok())
        });
        if let Some(journal) = candidate.as_mut() {
            self.sync_load_preimages(journal)?;
            journal.operation.operation_id = Uuid::new_v4().to_string();
            journal.operation.revision = 1;
            if journal.meta != record.meta {
                return Err("Recovery journal metadata does not match registered identity; evidence archived but no replacement".into());
            }
            if !record.meta.root.try_exists().map_err(|e| e.to_string())? {
                journal.status = WorktreeSyncStatus::Aborted;
                journal.operation.phase = "metadata_archived".into();
                journal.operation.revision += 1;
                native::durable_json(&path, journal)?;
                let mut response = self.sync_response(
                    journal,
                    WorktreeSyncStatus::Aborted,
                    vec![format!(
                        "Missing source metadata archived at {}. No Git mutation performed.",
                        quarantine.display()
                    )],
                    Vec::new(),
                );
                response.recovery_journal_digest = self.sync_journal_digest(id).ok();
                return Ok(response);
            }
            self.sync_validate(journal, false)?;
            let head = native::head(&journal.meta.root)?;
            if head != journal.operation.source_head
                && journal.operation.phase != "committing"
                && journal.operation.phase != "checkpointing"
            {
                return Err(
                    "Backup source HEAD is unknown; metadata archived without replacement".into(),
                );
            }
            native::durable_json(&path, journal)?;
            self.sync_reconcile(journal)?;
            return Ok(self.sync_response(
                journal,
                journal.status.clone(),
                vec![format!(
                    "Validated journal recovered; original evidence at {}",
                    quarantine.display()
                )],
                native::conflicts(&journal.meta.root)?,
            ));
        }
        let mut response = self.sync_metadata_response(id, record, vec![format!("Unparseable journal and backup archived at {}; no Git changes or untrusted metadata deletion performed", quarantine.display())]);
        response.instructions = "No valid operation provenance is available. Archived metadata is retained; Recover cannot infer or replay unknown Git operations. Worktree deletion remains fenced until identity is established.".into();
        Ok(response)
    }

    fn sync_validate(&self, journal: &Journal, check_head: bool) -> Result<(), String> {
        self.validate_merge_phase()?;
        let registry: WorktreeRegistry = serde_json::from_slice(
            &std::fs::read(self.registry_path()).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if registry.project_hash != self.project_hash
            || registry.source_workspace_root != self.source_workspace_root
            || !registry.records.iter().any(|r| r.meta == journal.meta)
        {
            return Err(
                "Sync registered identity changed; retain integration and recovery material".into(),
            );
        }
        validate_worktree_meta_checkout(&journal.meta)?;
        let repo = git2::Repository::open(&journal.meta.root).map_err(|e| e.to_string())?;
        if journal.schema_version != 1
            || crate::locking::repository_identity(&journal.meta.root)? != journal.repository
            || dunce::canonicalize(repo.path()).map_err(|e| e.to_string())? != journal.git_dir
            || journal.meta.root == self.source_workspace_root
            || journal.meta.branch.as_deref() == Some(&journal.operation.target_branch)
            || (check_head && native::head(&journal.meta.root)? != journal.operation.source_head)
        {
            return Err(
                "Sync physical repository, source branch or HEAD changed; refuse mutation".into(),
            );
        }
        Ok(())
    }

    fn sync_mutate(&self, journal: &Journal, args: &[&str]) -> Result<Vec<u8>, String> {
        self.sync_validate(journal, true)?;
        crate::locking::with_git_phase(self.merge_authority.clone(), || {
            crate::locking::checkpoint_git_phase()?;
            let result = native::output(&journal.meta.root, args);
            self.validate_merge_phase()?;
            result
        })
    }

    pub async fn sync_worktree(
        &self,
        id: &str,
        request: WorktreeSyncRequest,
    ) -> Result<WorktreeSyncResponse, String> {
        validate_worktree_id(id)?;
        let mutating = !matches!(request, WorktreeSyncRequest::Status { .. });
        let allow_missing = matches!(request, WorktreeSyncRequest::Recover { .. });
        let service = if mutating {
            self.fence_sync_worktree(id, allow_missing).await?
        } else {
            self.clone()
        };
        let service = service.acquire_merge_permit().await?;
        let guard = service.registry_write_lock().await?;
        let registry = service.load_registry_unlocked().await?;
        let record = registry
            .records
            .iter()
            .find(|r| r.meta.id == id)
            .cloned()
            .ok_or("Sync worktree is not registered")?;
        let id = id.to_string();
        let worker = service.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            let _invalidation = mutating.then(|| SyncCacheInvalidation(worker.clone()));
            if mutating {
                if !worker
                    .effect_token
                    .as_ref()
                    .is_some_and(|token| token.records.contains(&record.meta))
                {
                    return Err("Sync effect token does not cover registered source".into());
                }
                worker.sync_validate_record(&record, allow_missing)?;
            }
            worker.sync_inner(&id, record, request)
        })
        .await
        .map_err(|e| format!("Sync task interrupted; inspect status before retry: {e}"))?
    }

    fn sync_inner(
        &self,
        id: &str,
        record: WorktreeRegistryRecord,
        request: WorktreeSyncRequest,
    ) -> Result<WorktreeSyncResponse, String> {
        self.validate_merge_phase()?;
        if let WorktreeSyncRequest::Recover {
            expected_journal_digest,
        } = &request
        {
            return self.sync_recover_metadata(id, &record, expected_journal_digest);
        }
        let mut journal = match self.sync_journal(id) {
            Ok(journal) => journal,
            Err(error) if matches!(request, WorktreeSyncRequest::Status { .. }) => {
                return Ok(self.sync_metadata_response(id, &record, vec![error]))
            }
            Err(error) => return Err(error),
        };
        if let WorktreeSyncRequest::Status {
            target_branch,
        } = &request
        {
            let requested = target_branch.as_deref();
            if let Some(branch) = requested {
                validate_branch_name(branch)?;
            }
            if let Some(j) = journal.as_mut() {
                if let Err(error) = self.sync_validate(j, false) {
                    return Ok(self.sync_response(
                        j,
                        WorktreeSyncStatus::RecoveryRequired,
                        vec![error],
                        Vec::new(),
                    ));
                }
                self.sync_reconcile_observed(j, false)?;
                if !j.active()
                    && requested.is_some_and(|branch| branch != j.operation.target_branch)
                {
                    return self.sync_idle_response(id, &record, target_branch.clone());
                }
                let conflicts = native::conflicts(&j.meta.root)?;
                let current_target = native::target(&j.meta.root, &j.operation.target_branch)?;
                let status = if j.status != WorktreeSyncStatus::Aborted
                    && current_target != j.operation.target_head
                {
                    WorktreeSyncStatus::TargetAdvanced
                } else {
                    j.status.clone()
                };
                let mut response = self.sync_response(j, status, Vec::new(), conflicts);
                if j.active() && requested.is_some_and(|branch| branch != j.operation.target_branch)
                {
                    response.warnings.push(format!(
                        "An active synchronization is pinned to {}; finish or abort it before synchronizing with {}",
                        j.operation.target_branch,
                        requested.unwrap_or_default()
                    ));
                }
                if !j.active() {
                    response.target_head = Some(current_target);
                }
                return Ok(response);
            }
            return self.sync_idle_response(id, &record, target_branch.clone());
        }
        if let WorktreeSyncRequest::Start {
            target_branch,
            expected_source_head,
            expected_target_head,
            checkpoint,
        } = request
        {
            if journal.as_ref().is_some_and(|j| j.active()) {
                return Err(
                    "Synchronization already active; inspect status instead of replaying Start"
                        .into(),
                );
            }
            validate_registered_record_checkout(&record)?;
            let root = &record.meta.root;
            git::ensure_no_merge_in_progress(root, "Sync source")?;
            let branch = target_branch
                .or_else(|| record.meta.base_branch.clone())
                .ok_or("Sync target branch missing")?;
            let source_head = native::head(root)?;
            let target_head = native::target(root, &branch)?;
            if source_head != expected_source_head || target_head != expected_target_head {
                return Err(
                    "Sync source or target HEAD changed; refresh Status and pin new heads".into(),
                );
            }
            let dirty = git::has_porcelain_changes(root)?;
            if dirty && !checkpoint {
                return Err(
                    "Sync source is dirty; commit changes or explicitly request checkpoint=true"
                        .into(),
                );
            }
            let repo = git2::Repository::open(root).map_err(|e| e.to_string())?;
            let mut j = Journal {
                schema_version: 1,
                meta: record.meta.clone(),
                repository: crate::locking::repository_identity(root)?,
                git_dir: dunce::canonicalize(repo.path()).map_err(|e| e.to_string())?,
                original_head: source_head.clone(),
                operation: WorktreeSyncOperation {
                    initiator: self.sync_actor.clone(),
                    operation_id: Uuid::new_v4().to_string(),
                    revision: 1,
                    source_head,
                    target_branch: branch,
                    target_head,
                    phase: "prepared".into(),
                    checkpoint_commit: None,
                    recovery_path: None,
                },
                status: WorktreeSyncStatus::RecoveryRequired,
                scope: Vec::new(),
                abort_fingerprint: None,
                commit_tree: None,
                terminal_head: None,
                post_abort_fingerprint: None,
                checkpoint_tree: None,
                archive_digest: None,
                secondary_archives: Vec::new(),
                recovery_archives: Vec::new(),
                restore_archive_id: None,
                restore_preimages: Vec::new(),
                restore_cursor: 0,
                restore_preimages_digest: None,
            };
            self.sync_validate(&j, true)?;
            self.sync_save(&j)?;
            if dirty {
                j.operation.phase = "checkpointing".into();
                self.sync_save(&j)?;
                self.sync_mutate(&j, &["add", "--all", "--", "."])?;
                j.checkpoint_tree = Some(native::text(root, &["write-tree"])?);
                self.sync_save(&j)?;
                self.sync_mutate(
                    &j,
                    &[
                        "commit",
                        "--no-gpg-sign",
                        "-m",
                        "Checkpoint before native worktree synchronization",
                    ],
                )?;
                j.operation.source_head = native::head(root)?;
                j.operation.checkpoint_commit = Some(j.operation.source_head.clone());
                self.sync_validate(&j, true)?;
                self.sync_save(&j)?;
            }
            git::ensure_clean_worktree(root, "Sync source")?;
            j.operation.phase = "merging".into();
            self.sync_save(&j)?;
            let merge_result = self.sync_mutate(
                &j,
                &[
                    "merge",
                    "--no-commit",
                    "--no-ff",
                    "--no-edit",
                    "--no-gpg-sign",
                    &j.operation.target_head,
                ],
            );
            if let Err(error) = merge_result {
                if !git::has_merge_in_progress(root) {
                    return Err(format!(
                        "Sync merge stopped; inspect Status without replay: {error}"
                    ));
                }
            }
            self.sync_reconcile(&mut j)?;
            if j.active() {
                j.scope = native::names(
                    root,
                    &[
                        "diff",
                        "--name-only",
                        "--no-renames",
                        "-z",
                        &j.operation.source_head,
                    ],
                )?;
                j.scope
                    .extend(native::conflicts(root)?.into_iter().map(|c| c.path));
                j.scope.sort();
                j.scope.dedup();
                j.abort_fingerprint = Some(native::fingerprint(root)?);
                self.sync_save(&j)?;
            }
            return Ok(self.sync_response(
                &j,
                j.status.clone(),
                Vec::new(),
                native::conflicts(root)?,
            ));
        }
        let mut j = journal.ok_or("No synchronization operation; request Status")?;
        let (operation_id, revision) = match &request {
            WorktreeSyncRequest::Resolve {
                operation_id,
                expected_revision,
                ..
            }
            | WorktreeSyncRequest::Continue {
                operation_id,
                expected_revision,
                ..
            }
            | WorktreeSyncRequest::Abort {
                operation_id,
                expected_revision,
                ..
            }
            | WorktreeSyncRequest::Restore {
                operation_id,
                expected_revision,
                ..
            } => (operation_id, *expected_revision),
            _ => unreachable!(),
        };
        if j.operation.operation_id != *operation_id || j.operation.revision != revision {
            return Err("Stale sync operation_id or expected_revision; refresh Status".into());
        }
        self.sync_validate(&j, false)?;
        self.sync_reconcile(&mut j)?;
        self.sync_validate(&j, true)?;
        if let WorktreeSyncRequest::Restore {
            archive_id, ..
        } = &request
        {
            self.sync_restore(&mut j, archive_id.as_deref())?;
            return Ok(self.sync_response(
                &j,
                j.status.clone(),
                Vec::new(),
                native::conflicts(&j.meta.root)?,
            ));
        }
        if !j.active() {
            return Err("Sync operation is terminal; request Status".into());
        }
        match request {
            WorktreeSyncRequest::Resolve {
                resolutions, ..
            } => self.sync_resolve(&mut j, resolutions)?,
            WorktreeSyncRequest::Continue {
                commit_message, ..
            } => self.sync_continue(&mut j, commit_message)?,
            WorktreeSyncRequest::Abort {
                force, ..
            } => self.sync_abort(&mut j, force)?,
            _ => unreachable!(),
        }
        Ok(self.sync_response(
            &j,
            j.status.clone(),
            Vec::new(),
            native::conflicts(&j.meta.root)?,
        ))
    }

    fn sync_idle_response(
        &self,
        id: &str,
        record: &WorktreeRegistryRecord,
        target_branch: Option<String>,
    ) -> Result<WorktreeSyncResponse, String> {
        validate_registered_record_checkout(record)?;
        let branch = target_branch.or_else(|| record.meta.base_branch.clone());
        let target_head = branch
            .as_ref()
            .map(|branch| native::target(&record.meta.root, branch))
            .transpose()?;
        Ok(WorktreeSyncResponse {
            recovery_journal_digest: None,
            recovery_archives: Vec::new(),
            restore_archive_id: None,
            id: id.into(),
            status: WorktreeSyncStatus::Idle,
            source_head: native::head(&record.meta.root)?,
            target_branch: branch,
            target_head,
            operation: None,
            conflicts: Vec::new(),
            warnings: Vec::new(),
            instructions:
                "Pin source_head and target_head from this status when starting synchronization."
                    .into(),
        })
    }

    fn sync_response(
        &self,
        j: &Journal,
        status: WorktreeSyncStatus,
        warnings: Vec<String>,
        conflicts: Vec<WorktreeSyncConflict>,
    ) -> WorktreeSyncResponse {
        WorktreeSyncResponse { restore_archive_id: j.restore_archive_id.clone(), recovery_archives: Self::sync_archives(j).into_iter().map(|a| WorktreeSyncArchive { archive_id: a.archive_id, path: a.path }).collect(), recovery_journal_digest: self.sync_journal_digest(&j.meta.id).ok(), id: j.meta.id.clone(), status, source_head: native::head(&j.meta.root).unwrap_or_else(|_| j.operation.source_head.clone()), target_branch: Some(j.operation.target_branch.clone()), target_head: Some(j.operation.target_head.clone()), operation: Some(j.operation.clone()), conflicts, warnings, instructions: "Resolve only listed index conflicts using explicit choices, then Continue with current operation_id/revision. Abort refuses edited state; force Abort first writes a verified durable recovery archive. Target branch is never modified. Recovery-required states must be inspected; never replay an unknown mutation.".into() }
    }

    fn sync_reconcile(&self, j: &mut Journal) -> Result<(), String> {
        self.sync_reconcile_observed(j, true)
    }

    fn sync_reconcile_observed(&self, j: &mut Journal, persist: bool) -> Result<(), String> {
        if !j.active() {
            return Ok(());
        }
        let root = &j.meta.root;
        let head = native::head(root)?;
        if j.operation.phase == "checkpointing" && head != j.operation.source_head {
            let repo = git2::Repository::open(root).map_err(|e| e.to_string())?;
            let commit = repo
                .find_commit(git2::Oid::from_str(&head).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            if commit.parent_count() != 1
                || commit.parent_id(0).map_err(|e| e.to_string())?.to_string() != j.original_head
                || Some(commit.tree_id().to_string()) != j.checkpoint_tree
            {
                return Err("Unknown checkpoint post-image; retain source and journal".into());
            }
            j.operation.source_head = head.clone();
            j.operation.checkpoint_commit = Some(head);
            j.operation.phase = "checkpointed".into();
            j.status = WorktreeSyncStatus::RecoveryRequired;
        } else if j.operation.phase == "committing" && head != j.operation.source_head {
            self.sync_verify_commit(j, &head)?;
            j.status = WorktreeSyncStatus::Completed;
            j.terminal_head = Some(head);
            j.operation.phase = "completed".into();
        } else if head != j.operation.source_head {
            j.status = WorktreeSyncStatus::RecoveryRequired;
        } else if git::has_merge_in_progress(root) {
            let merge_head =
                std::fs::read_to_string(j.git_dir.join("MERGE_HEAD")).map_err(|e| e.to_string())?;
            if merge_head.trim() != j.operation.target_head {
                return Err("Foreign merge state detected; retain all material".into());
            }
            if j.operation.phase == "merging" && j.scope.is_empty() {
                j.scope = native::names(
                    root,
                    &[
                        "diff",
                        "--name-only",
                        "--no-renames",
                        "-z",
                        &j.operation.source_head,
                    ],
                )?;
                j.scope
                    .extend(native::conflicts(root)?.into_iter().map(|c| c.path));
                j.scope.sort();
                j.scope.dedup();
            }
            if !matches!(
                j.operation.phase.as_str(),
                "aborting" | "committing" | "restoring"
            ) {
                j.operation.phase = "integrating".into();
                j.status = if native::conflicts(root)?.is_empty() {
                    WorktreeSyncStatus::Ready
                } else {
                    WorktreeSyncStatus::Conflicts
                };
            }
        } else if matches!(j.operation.phase.as_str(), "merging" | "prepared")
            && native::output(
                root,
                &[
                    "merge-base",
                    "--is-ancestor",
                    &j.operation.target_head,
                    &head,
                ],
            )
            .is_ok()
        {
            j.status = WorktreeSyncStatus::Completed;
            j.operation.phase = "completed".into();
            j.terminal_head = Some(head);
        } else if j.operation.phase == "aborting"
            && git::ensure_clean_worktree(root, "Aborted source").is_ok()
        {
            if let Some(path) = &j.operation.recovery_path {
                j.post_abort_fingerprint =
                    Some(native::recovery_fingerprint(root, Path::new(path))?);
            }
            self.sync_archive_fingerprints(j)?;
            j.status = WorktreeSyncStatus::Aborted;
            j.operation.phase = "aborted".into();
        } else {
            j.status = WorktreeSyncStatus::RecoveryRequired;
        }
        if persist {
            self.sync_save(j)
        } else {
            Ok(())
        }
    }

    fn sync_resolve(
        &self,
        j: &mut Journal,
        resolutions: Vec<WorktreeSyncResolution>,
    ) -> Result<(), String> {
        if !git::has_merge_in_progress(&j.meta.root) {
            return Err("No pinned merge index available; Abort with recovery instead".into());
        }
        let conflicts = native::conflicts(&j.meta.root)?;
        let mut seen = HashSet::new();
        for r in &resolutions {
            let file = native::safe_path(&j.meta.root, &r.path)?;
            if !seen.insert(&r.path) {
                return Err("Duplicate resolution path".into());
            }
            let conflict = conflicts.iter().find(|c| c.path == r.path);
            let selected = match r.choice {
                WorktreeSyncResolutionChoice::Source => conflict.and_then(|c| c.source.as_ref()),
                WorktreeSyncResolutionChoice::Target => conflict.and_then(|c| c.target.as_ref()),
                _ => None,
            };
            if matches!(
                r.choice,
                WorktreeSyncResolutionChoice::Source | WorktreeSyncResolutionChoice::Target
            ) && selected.is_none()
            {
                return Err("Selected side deleted this path; use explicit delete choice".into());
            }
            if selected.is_some_and(|s| s.mode != 0o100644 && s.mode != 0o100755) {
                return Err("Automatic side resolution supports regular files only; use working_tree after inspecting symlink/submodule".into());
            }
            if let Ok(meta) = std::fs::symlink_metadata(&file) {
                if !meta.is_file() || meta.file_type().is_symlink() {
                    return Err("Resolution leaf must be a regular file or absent; move obstructing directory/symlink before resolving".into());
                }
            }
            if matches!(r.choice, WorktreeSyncResolutionChoice::WorkingTree) && !file.is_file() {
                return Err(
                    "working_tree resolution requires a regular file; use delete for absent path"
                        .into(),
                );
            }
        }
        j.scope.extend(resolutions.iter().map(|r| r.path.clone()));
        j.scope.sort();
        j.scope.dedup();
        j.operation.phase = "resolving".into();
        j.operation.revision += 1;
        self.sync_save(j)?;
        for r in resolutions {
            let file = native::safe_path(&j.meta.root, &r.path)?;
            self.sync_validate(j, true)?;
            match r.choice {
                WorktreeSyncResolutionChoice::WorkingTree => {
                    self.sync_mutate(j, &["add", "--", &r.path])?;
                }
                WorktreeSyncResolutionChoice::Delete => {
                    self.sync_mutate(j, &["rm", "--force", "--ignore-unmatch", "--", &r.path])?;
                }
                WorktreeSyncResolutionChoice::Source | WorktreeSyncResolutionChoice::Target => {
                    let side = if r.choice == WorktreeSyncResolutionChoice::Source {
                        "--ours"
                    } else {
                        "--theirs"
                    };
                    if let Some(parent) = file.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    self.sync_mutate(j, &["checkout", side, "--", &r.path])?;
                    self.sync_mutate(j, &["add", "--", &r.path])?;
                }
            }
        }
        j.operation.phase = "integrating".into();
        j.status = if native::conflicts(&j.meta.root)?.is_empty() {
            WorktreeSyncStatus::Ready
        } else {
            WorktreeSyncStatus::Conflicts
        };
        self.sync_save(j)
    }

    fn sync_continue(&self, j: &mut Journal, message: Option<String>) -> Result<(), String> {
        let root = &j.meta.root;
        if !git::has_merge_in_progress(root) || !native::conflicts(root)?.is_empty() {
            return Err("Continue requires the pinned merge and no unmerged index entries; resolve conflicts first".into());
        }
        if !native::names(root, &["diff", "--name-only", "-z"])?.is_empty() {
            return Err("Unstaged edits remain; explicitly stage intended conflict resolutions before Continue".into());
        }
        let staged = native::names(
            root,
            &["diff", "--cached", "--name-only", "--no-renames", "-z"],
        )?;
        if staged.iter().any(|p| !j.scope.contains(p)) {
            return Err(
                "Staged changes outside sync scope; unstage unrelated edits before Continue".into(),
            );
        }
        j.commit_tree = Some(native::text(root, &["write-tree"])?);
        j.operation.phase = "committing".into();
        j.operation.revision += 1;
        self.sync_save(j)?;
        self.sync_mutate(
            j,
            &[
                "commit",
                "--no-gpg-sign",
                "-m",
                message
                    .as_deref()
                    .filter(|m| !m.trim().is_empty())
                    .unwrap_or("Synchronize registered source with pinned target"),
            ],
        )?;
        self.sync_reconcile(j)
    }

    fn sync_verify_commit(&self, j: &Journal, head: &str) -> Result<(), String> {
        let repo = git2::Repository::open(&j.meta.root).map_err(|e| e.to_string())?;
        let commit = repo
            .find_commit(git2::Oid::from_str(head).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if commit.parent_count() != 2
            || commit.parent_id(0).map_err(|e| e.to_string())?.to_string()
                != j.operation.source_head
            || commit.parent_id(1).map_err(|e| e.to_string())?.to_string()
                != j.operation.target_head
            || Some(commit.tree_id().to_string()) != j.commit_tree
        {
            return Err(
                "Unknown commit post-image; retain journal and inspect without replay".into(),
            );
        }
        Ok(())
    }

    fn sync_abort(&self, j: &mut Journal, force: bool) -> Result<(), String> {
        if !force && j.abort_fingerprint.as_ref() != Some(&native::fingerprint(&j.meta.root)?) {
            return Err("Abort would discard changes made after integration; use force=true to archive all material durably before aborting".into());
        }
        let mut abort_archive = None;
        if force {
            let archive = self.registry_dir().join("sync").join(format!(
                "{}-recovery-{}.json",
                j.meta.id,
                Uuid::new_v4()
            ));
            self.sync_validate(j, true)?;
            native::archive(&j.meta.root, &archive)?;
            let digest = format!(
                "{:x}",
                Sha256::digest(std::fs::read(&archive).map_err(|e| e.to_string())?)
            );
            if j.recovery_archives.is_empty() {
                j.recovery_archives = Self::sync_archives(j);
            }
            j.recovery_archives.push(RecoveryArchive {
                archive_id: if j.operation.recovery_path.is_none() {
                    "primary".into()
                } else {
                    Uuid::new_v4().to_string()
                },
                path: archive.to_string_lossy().into_owned(),
                digest: digest.clone(),
                post_abort_fingerprint: None,
            });
            if j.operation.recovery_path.is_none() {
                j.archive_digest = Some(digest);
                j.operation.recovery_path = Some(archive.to_string_lossy().into_owned());
            } else {
                j.secondary_archives
                    .push(archive.to_string_lossy().into_owned());
            }
            abort_archive = Some(archive);
        }
        j.operation.phase = "aborting".into();
        j.restore_preimages.clear();
        j.restore_archive_id = None;
        j.restore_cursor = 0;
        j.operation.revision += 1;
        self.sync_save(j)?;
        if force {
            let archive = abort_archive
                .as_ref()
                .ok_or("Forced abort archive missing")?;
            native::CachedRestore::load(archive)?
                .prepare_abort_directories(&j.meta.root, || self.sync_validate(j, true))?;
            self.sync_mutate(j, &["reset", "--hard", &j.operation.source_head])?;
        } else {
            self.sync_mutate(j, &["merge", "--abort"])?;
        }
        if git::has_merge_in_progress(&j.meta.root)
            || native::head(&j.meta.root)? != j.operation.source_head
        {
            return Err("Abort post-image uncertain; retain archive and inspect Status".into());
        }
        if let Some(path) = &j.operation.recovery_path {
            j.post_abort_fingerprint =
                Some(native::recovery_fingerprint(&j.meta.root, Path::new(path))?);
        }
        self.sync_archive_fingerprints(j)?;
        j.status = WorktreeSyncStatus::Aborted;
        j.operation.phase = "aborted".into();
        self.sync_save(j)
    }

    fn sync_archives(j: &Journal) -> Vec<RecoveryArchive> {
        if !j.recovery_archives.is_empty() {
            return j.recovery_archives.clone();
        }
        match (&j.operation.recovery_path, &j.archive_digest) {
            (Some(path), Some(digest)) => vec![RecoveryArchive {
                archive_id: "primary".into(),
                path: path.clone(),
                digest: digest.clone(),
                post_abort_fingerprint: j.post_abort_fingerprint.clone(),
            }],
            _ => Vec::new(),
        }
    }

    fn sync_archive_fingerprints(&self, j: &mut Journal) -> Result<(), String> {
        for archive in &mut j.recovery_archives {
            let path = Path::new(&archive.path);
            if std::fs::read(path)
                .ok()
                .is_some_and(|bytes| format!("{:x}", Sha256::digest(bytes)) == archive.digest)
            {
                archive.post_abort_fingerprint =
                    Some(native::recovery_fingerprint(&j.meta.root, path)?);
            }
        }
        Ok(())
    }

    fn sync_restore(&self, j: &mut Journal, archive_id: Option<&str>) -> Result<(), String> {
        let resuming = j.operation.phase == "restoring";
        if j.status != WorktreeSyncStatus::Aborted && !resuming {
            return Err(
                "Restore requires an aborted operation with a verified native recovery archive"
                    .into(),
            );
        }
        let selected_id = archive_id.unwrap_or("primary");
        if resuming && j.restore_archive_id.as_deref().unwrap_or("primary") != selected_id {
            return Err("Cannot change archive selection during a partial restore; resume the pinned archive or force Abort".into());
        }
        let archive = Self::sync_archives(j)
            .into_iter()
            .find(|a| a.archive_id == selected_id)
            .ok_or("Unknown recovery archive_id; refresh Status")?;
        let path = PathBuf::from(&archive.path);
        if path.parent() != Some(self.registry_dir().join("sync").as_path())
            || std::fs::symlink_metadata(&path)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_symlink()
        {
            return Err(
                "Recovery archive path is outside the owned sync directory or is a symlink".into(),
            );
        }
        if archive.digest
            != format!(
                "{:x}",
                Sha256::digest(std::fs::read(&path).map_err(|e| e.to_string())?)
            )
        {
            return Err("Recovery manifest changed since this operation archived it".into());
        }
        let cached = native::CachedRestore::load(&path)?;
        if !resuming
            && archive.post_abort_fingerprint.as_ref()
                != Some(&native::recovery_fingerprint(&j.meta.root, &path)?)
        {
            return Err("Source index or managed files changed after abort; Restore refuses to overwrite newer work".into());
        }
        self.sync_validate(j, true)?;
        if !resuming {
            j.restore_preimages = cached.preimages(&j.meta.root, &j.git_dir)?;
            j.restore_cursor = 0;
            j.operation.revision += 1;
        } else if j.restore_preimages.is_empty() {
            if archive.post_abort_fingerprint.as_ref()
                != Some(&native::recovery_fingerprint(&j.meta.root, &path)?)
            {
                return Err("Interrupted restore lacks a known preimage; force Abort preserves current work".into());
            }
            j.restore_preimages = cached.preimages(&j.meta.root, &j.git_dir)?;
        }
        cached.validate_entries(
            &j.meta.root,
            &j.git_dir,
            &j.restore_preimages,
            j.restore_cursor,
        )?;
        j.restore_archive_id = Some(selected_id.into());
        j.operation.phase = "restoring".into();
        j.status = WorktreeSyncStatus::RecoveryRequired;
        self.sync_save(j)?;
        let mut progress = j.clone();
        progress.restore_preimages_digest = Some(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&j.restore_preimages).map_err(|e| e.to_string())?)
        ));
        progress.restore_preimages.clear();
        self.sync_validate(j, true)?;
        while j.restore_cursor < j.restore_preimages.len() {
            let preimage = j.restore_preimages[j.restore_cursor].clone();
            cached.step(
                &j.meta.root,
                &j.git_dir,
                j.restore_cursor,
                &preimage,
                || self.sync_validate(j, true),
            )?;
            j.restore_cursor += 1;
            progress.restore_cursor = j.restore_cursor;
            // The immutable preimages live in a content-addressed sidecar. Only
            // bounded progress is persisted between atomic restore units.
            native::durable_json(&self.sync_path(&j.meta.id)?, &progress)?;
        }
        self.sync_validate(j, true)?;
        if git::has_merge_in_progress(&j.meta.root) {
            j.operation.phase = "integrating".into();
            self.sync_reconcile(j)
        } else {
            j.operation.phase = "restored".into();
            j.status = WorktreeSyncStatus::Aborted;
            self.sync_save(j)
        }
    }
}

#[cfg(test)]
#[path = "sync_tests.rs"]
mod tests;
