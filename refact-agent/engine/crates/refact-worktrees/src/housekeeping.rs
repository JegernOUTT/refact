//! Checked, non-destructive quarantine owned by WorktreeService. Never calls cleanup/remove.
use super::*;
use crate::types::WorktreeArchiveIdentity;
use std::io::Write;

impl WorktreeService {
    pub async fn verify_worktree_archive_version(
        &self,
        id: &str,
        version: &str,
    ) -> Result<bool, String> {
        validate_worktree_id(id)?;
        if version.len() != 64 || !version.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Invalid archive version".into());
        }
        let directory = self
            .cache_dir
            .join("worktree-archive")
            .join(&self.project_hash)
            .join(version);
        reject_symlink_ancestors(&directory.join("identity.json"))?;
        let bytes = tokio::fs::read(directory.join("identity.json"))
            .await
            .map_err(|e| e.to_string())?;
        let identity: WorktreeArchiveIdentity =
            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if identity.record.meta.id != id || identity.version != version {
            return Err("Archive evidence identity mismatch".into());
        }
        self.verify_worktree_archive(&identity).await
    }

    /// Side-effect-free: unlike list_worktrees, does not reconcile or prune registrations.
    pub async fn preview_worktree_archive(
        &self,
        id: &str,
    ) -> Result<WorktreeArchiveIdentity, String> {
        let view = self.get_worktree(id).await?;
        let registry = self.load_registry_unlocked().await?;
        let record = registry
            .records
            .into_iter()
            .find(|r| r.meta.id == id)
            .ok_or_else(|| "Registered worktree disappeared".to_string())?;
        if !record.references.is_empty()
            || record.meta.task_id.is_some()
            || record.meta.card_id.is_some()
            || record.meta.agent_id.is_some()
            || view.status.dirty
            || view.status.conflicted
            || view.status.error.is_some()
            || !view.status.path_exists
            || !view.status.is_git_worktree
        {
            return Err("Active/referenced/dirty/conflicted/unknown checkout is protected".into());
        }
        let root = record.meta.root.clone();
        let head = tokio::task::spawn_blocking(move || {
            git::ensure_no_merge_in_progress(&root, "Archive checkout")?;
            git::ensure_clean_worktree(&root, "Archive checkout")?;
            git::head_rev(&root)
        })
        .await
        .map_err(|e| e.to_string())??;
        let version = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(&record, &head)).map_err(|e| e.to_string())?)
        );
        Ok(WorktreeArchiveIdentity {
            record,
            head,
            version,
        })
    }

    fn archive_directory(&self, identity: &WorktreeArchiveIdentity) -> Result<PathBuf, String> {
        validate_worktree_id(&identity.record.meta.id)?;
        let expected = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(&identity.record, &identity.head))
                    .map_err(|e| e.to_string())?
            )
        );
        if identity.version != expected
            || identity.record.meta.source_workspace_root != self.source_workspace_root
            || identity.record.meta.root != self.worktree_path_for_id(&identity.record.meta.id)?
        {
            return Err("Archive identity does not belong to this owner".into());
        }
        Ok(self
            .cache_dir
            .join("worktree-archive")
            .join(&self.project_hash)
            .join(&identity.version))
    }

    /// Caller holds explicit resource authority and checks external references. This owner rechecks
    /// the exact native record under the same locks as native merge/registry mutations.
    pub async fn archive_worktree_checked(
        &self,
        identity: WorktreeArchiveIdentity,
    ) -> Result<(), String> {
        let service = self
            .fence_worktrees(&[identity.record.meta.id.clone()], false)
            .await?;
        async move {
            let service = service.acquire_merge_permit().await?;
            let _registry = service.registry_write_lock().await?;
            if service
                .preview_worktree_archive(&identity.record.meta.id)
                .await?
                != identity
            {
                return Err("Archive version changed; no writes performed".into());
            }
            let directory = service.archive_directory(&identity)?;
            let mut registry = service.load_registry_unlocked().await?;
            let blocking_identity = identity.clone();
            let registry_guard = _registry.clone();
            let merge_service = service.clone();
            tokio::task::spawn_blocking(move || {
                let _registry_guard = registry_guard;
                merge_service.validate_effect_token_blocking()?;
                merge_service.validate_merge_phase()?;
                reject_symlink_ancestors(&directory)?;
                if directory.try_exists().map_err(|e| e.to_string())? {
                    return Err(
                        "Archive already attempted; inspect retained evidence, never replay".into(),
                    );
                }
                std::fs::create_dir_all(directory.parent().ok_or("Archive parent missing")?)
                    .map_err(|e| e.to_string())?;
                std::fs::create_dir(&directory).map_err(|e| e.to_string())?;
                let bytes = serde_json::to_vec(&blocking_identity).map_err(|e| e.to_string())?;
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(directory.join("identity.json"))
                    .map_err(|e| e.to_string())?;
                file.write_all(&bytes).map_err(|e| e.to_string())?;
                file.sync_all().map_err(|e| e.to_string())?;
                move_checkout(
                    &blocking_identity.record.meta.source_workspace_root,
                    &blocking_identity.record.meta.root,
                    &directory.join("checkout"),
                )
            })
            .await
            .map_err(|e| e.to_string())??;
            registry
                .records
                .retain(|r| r.meta.id != identity.record.meta.id);
            service.save_registry_unlocked(&registry, &_registry).await
        }
        .await
    }

    pub async fn verify_worktree_archive(
        &self,
        identity: &WorktreeArchiveIdentity,
    ) -> Result<bool, String> {
        let directory = self.archive_directory(identity)?;
        let expected = identity.clone();
        tokio::task::spawn_blocking(move || {
            reject_symlink_ancestors(&directory.join("checkout"))?;
            let stored: WorktreeArchiveIdentity = serde_json::from_slice(
                &std::fs::read(directory.join("identity.json")).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            if stored != expected
                || expected
                    .record
                    .meta
                    .root
                    .try_exists()
                    .map_err(|e| e.to_string())?
            {
                return Ok(false);
            }
            let mut archived = expected.record.clone();
            archived.meta.root = directory.join("checkout");
            validate_registered_record_checkout(&archived)?;
            git::ensure_no_merge_in_progress(&archived.meta.root, "Archived checkout")?;
            git::ensure_clean_worktree(&archived.meta.root, "Archived checkout")?;
            Ok(git::head_rev(&archived.meta.root)? == expected.head)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    /// Exact move-back only. Retains the evidence directory forever; never replaces a checkout,
    /// rewinds a branch, force-removes a directory, or automatically retries interrupted work.
    pub(super) fn validate_archive_effect_identity(
        &self,
        identity: &WorktreeArchiveIdentity,
    ) -> Result<(), String> {
        let directory = self.archive_directory(identity)?;
        reject_symlink_ancestors(&directory.join("checkout"))?;
        let stored: WorktreeArchiveIdentity = serde_json::from_slice(
            &std::fs::read(directory.join("identity.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if &stored != identity
            || identity.record.meta.source_workspace_root != self.source_workspace_root
            || identity.record.meta.root != self.worktree_path_for_id(&identity.record.meta.id)?
            || identity
                .record
                .meta
                .root
                .try_exists()
                .map_err(|e| e.to_string())?
        {
            return Err("Archive recovery evidence or original registered identity changed".into());
        }
        let mut archived = identity.record.clone();
        archived.meta.root = directory.join("checkout");
        validate_registered_record_checkout(&archived)?;
        git::ensure_clean_worktree(&archived.meta.root, "Archived checkout")?;
        git::ensure_no_merge_in_progress(&archived.meta.root, "Archived checkout")?;
        if git::head_rev(&archived.meta.root)? != identity.head {
            return Err("Archive recovery head changed".into());
        }
        Ok(())
    }

    /// Verify persisted archive proof and drain its original checkout domain
    /// before callers acquire adapter/trajectory or repository locks.
    pub async fn fence_archive_recovery(
        &self,
        identity: &WorktreeArchiveIdentity,
    ) -> Result<Self, String> {
        if let Some(token) = &self.effect_token {
            if token.archive.as_ref() == Some(identity) {
                return Ok(self.clone());
            }
            return Err("Archive recovery token identity mismatch".into());
        }
        if self.merge_permit.is_some() {
            return Err("Archive recovery requires a fresh fence before permits".into());
        }
        let service = self.clone();
        let checked = identity.clone();
        tokio::task::spawn_blocking(move || service.validate_archive_effect_identity(&checked))
            .await
            .map_err(|e| e.to_string())??;
        let fence = self
            .effect_fence
            .as_ref()
            .ok_or("Archive recovery requires application effect coordinator")?;
        let roots = vec![identity.record.meta.root.clone()];
        let guard = fence
            .begin(self.source_workspace_root.clone(), roots.clone())
            .await?;
        let service = Self {
            effect_token: Some(Arc::new(EffectToken {
                records: vec![],
                includes_source: false,
                archive: Some(identity.clone()),
                legacy: None,
                target: None,
                target_materialized: Arc::new(AtomicBool::new(false)),
                legacy_removed: Arc::new(AtomicBool::new(false)),
                fenced_roots: roots,
                _guard: guard,
            })),
            ..self.clone()
        };
        service.validate_effect_token().await?;
        Ok(service)
    }

    pub async fn recover_worktree_archive(
        &self,
        identity: WorktreeArchiveIdentity,
    ) -> Result<(), String> {
        let service = self.fence_archive_recovery(&identity).await?;
        let service = service.acquire_merge_permit().await?;
        let registry_guard = service.registry_write_lock().await?;
        service.validate_effect_token().await?;
        let mut registry = service.load_registry_unlocked().await?;
        if registry
            .records
            .iter()
            .any(|r| r.meta.id == identity.record.meta.id && r != &identity.record)
        {
            return Err("Recovery registration changed; refusing overwrite".into());
        }
        let directory = service.archive_directory(&identity)?;
        let record = identity.record.clone();
        let held = registry_guard.clone();
        let worker = service.clone();
        tokio::task::spawn_blocking(move || {
            let _held = held;
            worker.validate_effect_token_blocking()?;
            worker.validate_merge_phase()?;
            move_checkout(
                &record.meta.source_workspace_root,
                &directory.join("checkout"),
                &record.meta.root,
            )
        })
        .await
        .map_err(|e| e.to_string())??;
        if !registry
            .records
            .iter()
            .any(|r| r.meta.id == identity.record.meta.id)
        {
            registry.records.push(identity.record);
        }
        service
            .save_registry_unlocked(&registry, &registry_guard)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(root: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(root)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    async fn checked_archive_moves_only_exact_scratch_checkout_and_retains_branch_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir(&source).unwrap();
        git(&source, &["init", "-b", "main"]);
        git(&source, &["config", "user.name", "Scratch"]);
        git(
            &source,
            &["config", "user.email", "scratch@example.invalid"],
        );
        git(&source, &["config", "core.autocrlf", "false"]);
        std::fs::write(source.join("file"), "retained\n").unwrap();
        git(&source, &["add", "file"]);
        git(&source, &["commit", "-m", "scratch", "--no-gpg-sign"]);
        let service = super::super::worktree_registry_tests::test_service(
            temp.path().join("cache"),
            source.clone(),
        )
        .unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/archive-test".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let id = created.worktree.meta.id;
        let identity = service.preview_worktree_archive(&id).await.unwrap();
        let before = std::fs::read(service.registry_path()).unwrap();
        assert!(!service.archive_directory(&identity).unwrap().exists());
        assert_eq!(
            service.preview_worktree_archive(&id).await.unwrap(),
            identity
        );
        assert_eq!(std::fs::read(service.registry_path()).unwrap(), before);
        let mut stale = identity.clone();
        stale.version = "0".repeat(64);
        assert!(service.archive_worktree_checked(stale).await.is_err());
        assert_eq!(std::fs::read(service.registry_path()).unwrap(), before);
        service
            .archive_worktree_checked(identity.clone())
            .await
            .unwrap();
        assert!(!identity.record.meta.root.exists());
        assert!(service.verify_worktree_archive(&identity).await.unwrap());
        assert!(service
            .archive_worktree_checked(identity.clone())
            .await
            .is_err());
        let evidence = service
            .archive_directory(&identity)
            .unwrap()
            .join("identity.json");
        let evidence_bytes = std::fs::read(&evidence).unwrap();
        std::fs::create_dir(&identity.record.meta.root).unwrap();
        std::fs::write(
            identity.record.meta.root.join("user-work"),
            "never overwrite",
        )
        .unwrap();
        assert!(service
            .recover_worktree_archive(identity.clone())
            .await
            .is_err());
        assert_eq!(
            std::fs::read_to_string(identity.record.meta.root.join("user-work")).unwrap(),
            "never overwrite"
        );
        std::fs::remove_file(identity.record.meta.root.join("user-work")).unwrap();
        std::fs::remove_dir(&identity.record.meta.root).unwrap();
        service
            .recover_worktree_archive(identity.clone())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(identity.record.meta.root.join("file")).unwrap(),
            "retained\n"
        );
        assert_eq!(
            service.preview_worktree_archive(&id).await.unwrap(),
            identity
        );
        assert_eq!(std::fs::read(evidence).unwrap(), evidence_bytes);
        assert!(service
            .recover_worktree_archive(identity.clone())
            .await
            .is_err());
        // The evidence tombstone prevents old authorization from archiving again after recovery.
        assert!(service
            .archive_worktree_checked(identity.clone())
            .await
            .is_err());
        git(
            &source,
            &[
                "show-ref",
                "--verify",
                "refs/heads/refact/chat/archive-test",
            ],
        );
    }
}

#[cfg(test)]
mod quarantine_tests {
    use super::*;
    fn git_cmd(root: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[tokio::test]
    async fn checked_quarantine_scratch_stale_protection_and_no_overwrite() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        git_cmd(&root, &["init", "-b", "main"]);
        git_cmd(&root, &["config", "user.name", "Scratch"]);
        git_cmd(&root, &["config", "user.email", "scratch@example.invalid"]);
        git_cmd(&root, &["config", "core.autocrlf", "false"]);
        std::fs::write(root.join("tracked"), "retained\n").unwrap();
        git_cmd(&root, &["add", "tracked"]);
        git_cmd(&root, &["commit", "-m", "initial"]);
        let owner = super::super::worktree_registry_tests::test_service(
            scratch.path().join("cache"),
            root.clone(),
        )
        .unwrap();
        let created = owner
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/archive-test".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let id = created.worktree.meta.id;
        let checkout = created.worktree.meta.root;
        let before = std::fs::read(owner.registry_path()).unwrap();
        let identity = owner.preview_worktree_archive(&id).await.unwrap();
        assert_eq!(before, std::fs::read(owner.registry_path()).unwrap());
        assert!(!owner.archive_directory(&identity).unwrap().exists());
        std::fs::write(checkout.join("dirty"), "keep").unwrap();
        assert!(owner
            .archive_worktree_checked(identity.clone())
            .await
            .is_err());
        assert_eq!(before, std::fs::read(owner.registry_path()).unwrap());
        assert!(!owner.archive_directory(&identity).unwrap().exists());
        std::fs::remove_file(checkout.join("dirty")).unwrap();
        let reference = WorktreeReference {
            kind: "chat".into(),
            chat_id: Some("pinned-native".into()),
            ..Default::default()
        };
        owner.add_reference(&id, reference.clone()).await.unwrap();
        assert!(owner.preview_worktree_archive(&id).await.is_err());
        owner.remove_reference(&id, &reference).await.unwrap();
        assert!(
            owner.archive_worktree_checked(identity).await.is_err(),
            "changed registry revision is stale"
        );
        let identity = owner.preview_worktree_archive(&id).await.unwrap();
        owner
            .archive_worktree_checked(identity.clone())
            .await
            .unwrap();
        assert!(!checkout.exists());
        assert!(owner.verify_worktree_archive(&identity).await.unwrap());
        assert!(owner
            .archive_worktree_checked(identity.clone())
            .await
            .is_err());
        assert!(git::branch_exists(&root, "refact/chat/archive-test").unwrap());
        std::fs::create_dir(&checkout).unwrap();
        std::fs::write(checkout.join("unrelated"), "do not overwrite").unwrap();
        assert!(owner
            .recover_worktree_archive(identity.clone())
            .await
            .is_err());
        assert_eq!(
            std::fs::read_to_string(checkout.join("unrelated")).unwrap(),
            "do not overwrite"
        );
        std::fs::remove_file(checkout.join("unrelated")).unwrap();
        std::fs::remove_dir(&checkout).unwrap();
        owner
            .recover_worktree_archive(identity.clone())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(checkout.join("tracked")).unwrap(),
            "retained\n"
        );
        assert!(owner.get_worktree(&id).await.is_ok());
        assert!(owner
            .archive_directory(&identity)
            .unwrap()
            .join("identity.json")
            .is_file());
        assert!(owner.recover_worktree_archive(identity).await.is_err());
    }
}

fn reject_symlink_ancestors(path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err("Archive path contains symlink".into())
            }
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

fn move_checkout(source: &Path, from: &Path, to: &Path) -> Result<(), String> {
    reject_symlink_ancestors(from)?;
    reject_symlink_ancestors(to)?;
    // symlink_metadata also detects dangling links, which must never be replaced.
    match std::fs::symlink_metadata(to) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        _ => return Err("Archive/recovery destination occupied or unreadable".into()),
    }
    git::ensure_no_merge_in_progress(from, "Moved checkout")?;
    git::ensure_clean_worktree(from, "Moved checkout")?;
    git::run_git(
        source,
        &[
            "worktree",
            "move",
            "--",
            from.to_str().ok_or("Non-UTF8 checkout")?,
            to.to_str().ok_or("Non-UTF8 destination")?,
        ],
    )?;
    Ok(())
}
