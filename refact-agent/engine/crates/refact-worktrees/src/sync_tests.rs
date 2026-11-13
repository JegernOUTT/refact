use super::*;

#[derive(Debug)]
struct RetainedSyncFence(Arc<AtomicBool>);

#[derive(Debug)]
struct ReplaceSyncRegistration(PathBuf);

impl WorktreeEffectFence for ReplaceSyncRegistration {
    fn begin(
        &self,
        _project: PathBuf,
        _roots: Vec<PathBuf>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<EffectGuard, String>> + Send + '_>>
    {
        Box::pin(async move {
            let mut registry: WorktreeRegistry =
                serde_json::from_slice(&std::fs::read(&self.0).unwrap()).unwrap();
            registry.records[0].meta.branch = Some("replacement".into());
            std::fs::write(&self.0, serde_json::to_vec(&registry).unwrap()).unwrap();
            Ok(Arc::new(()) as EffectGuard)
        })
    }
}

#[tokio::test]
async fn native_sync_registration_changed_during_drain_is_not_authorized() {
    let (_temp, service, meta) = fixture().await;
    let service =
        service.with_effect_fence(Arc::new(ReplaceSyncRegistration(service.registry_path())));
    let error = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Start {
                target_branch: Some("main".into()),
                expected_source_head: native::head(&meta.root).unwrap(),
                expected_target_head: native::target(&meta.root, "main").unwrap(),
                checkpoint: false,
            },
        )
        .await
        .unwrap_err();
    assert!(error.contains("identity changed"), "{error}");
    assert!(!service.sync_path(&meta.id).unwrap().exists());
}

struct RetainedSyncGuard(Arc<AtomicBool>);

impl Drop for RetainedSyncGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl WorktreeEffectFence for RetainedSyncFence {
    fn begin(
        &self,
        _project: PathBuf,
        roots: Vec<PathBuf>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<EffectGuard, String>> + Send + '_>>
    {
        Box::pin(async move {
            assert_eq!(roots.len(), 1);
            self.0.store(true, Ordering::Release);
            Ok(Arc::new(RetainedSyncGuard(self.0.clone())) as EffectGuard)
        })
    }
}

#[tokio::test]
async fn native_sync_worker_retains_fence_and_invalidates_after_drop_and_panic() {
    for panic_worker in [false, true] {
        let (_temp, service, meta) = fixture().await;
        let active = Arc::new(AtomicBool::new(false));
        let service = service.with_effect_fence(Arc::new(RetainedSyncFence(active.clone())));
        let generation = registry_generation(&service.cache_dir);
        let before = generation.load(Ordering::SeqCst);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let started_tx = std::sync::Mutex::new(Some(started_tx));
        let barrier = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let worker_barrier = barrier.clone();
        let service = service.with_merge_authority(Arc::new(move || {
            if let Some(tx) = started_tx.lock().unwrap().take() {
                tx.send(()).unwrap();
                let (lock, changed) = &*worker_barrier;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = changed.wait(released).unwrap();
                }
                assert!(!panic_worker, "sync worker panic");
            }
            Ok(())
        }));
        let task = tokio::spawn(async move { start(&service, &meta, false).await });
        started_rx.await.unwrap();
        task.abort();
        let _ = task.await;
        assert!(active.load(Ordering::Acquire));
        assert_eq!(generation.load(Ordering::SeqCst), before);
        {
            let (lock, changed) = &*barrier;
            *lock.lock().unwrap() = true;
            changed.notify_all();
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while active.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(generation.load(Ordering::SeqCst) > before);
    }
}

#[tokio::test]
async fn native_sync_status_is_read_only_and_raw_mutation_requires_fence() {
    let (_temp, service, meta) = fixture().await;
    commit(&meta.root, "file", b"ours");
    commit(service.source_workspace_root(), "file", b"theirs");
    start(&service, &meta, false).await;
    let path = service.sync_path(&meta.id).unwrap();
    let before = std::fs::read(&path).unwrap();
    let raw = WorktreeService::new(
        service.cache_dir.clone(),
        service.source_workspace_root.clone(),
    )
    .unwrap();
    let status = raw
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Status {
                target_branch: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), before);
    let error = raw
        .sync_worktree(&meta.id, abort(&status.operation.unwrap(), false))
        .await
        .unwrap_err();
    assert!(error.contains("coordinator"), "{error}");
    assert!(git::has_merge_in_progress(&meta.root));
}

fn git(root: &Path, args: &[&str]) -> String {
    native::text(root, args).unwrap()
}
fn commit(root: &Path, path: &str, bytes: &[u8]) {
    std::fs::write(root.join(path), bytes).unwrap();
    git(root, &["add", "--", path]);
    git(root, &["commit", "-m", path]);
}
async fn fixture() -> (tempfile::TempDir, WorktreeService, WorktreeMeta) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("repo");
    std::fs::create_dir(&source).unwrap();
    git(&source, &["init", "-b", "main"]);
    git(&source, &["config", "user.name", "Sync Test"]);
    git(&source, &["config", "user.email", "sync@example.invalid"]);
    git(&source, &["config", "core.autocrlf", "false"]);
    commit(&source, "file", b"base\n");
    let service =
        super::super::worktree_registry_tests::test_service(temp.path().join("cache"), source)
            .unwrap();
    let created = service
        .create_worktree(CreateWorktreeRequest {
            branch: Some("refact/chat/sync".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    (temp, service, created.worktree.meta)
}
async fn start(
    service: &WorktreeService,
    meta: &WorktreeMeta,
    checkpoint: bool,
) -> WorktreeSyncResponse {
    service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Start {
                target_branch: Some("main".into()),
                expected_source_head: native::head(&meta.root).unwrap(),
                expected_target_head: native::target(&meta.root, "main").unwrap(),
                checkpoint,
            },
        )
        .await
        .unwrap()
}
fn cont(op: &WorktreeSyncOperation) -> WorktreeSyncRequest {
    WorktreeSyncRequest::Continue {
        operation_id: op.operation_id.clone(),
        expected_revision: op.revision,
        commit_message: None,
    }
}
fn abort(op: &WorktreeSyncOperation, force: bool) -> WorktreeSyncRequest {
    WorktreeSyncRequest::Abort {
        operation_id: op.operation_id.clone(),
        expected_revision: op.revision,
        force,
    }
}

#[tokio::test]
async fn shell_resolution_lands_with_stale_active_or_unreadable_native_journal() {
    for corrupt in [false, true] {
        let (_temp, service, meta) = fixture().await;
        commit(&meta.root, "file", b"source\n");
        commit(service.source_workspace_root(), "file", b"target\n");
        let response = start(&service, &meta, false).await;
        assert_eq!(response.status, WorktreeSyncStatus::Conflicts);
        let journal = std::fs::read(service.sync_path(&meta.id).unwrap()).unwrap();
        let refused = service
            .merge_worktree(
                &meta.id,
                MergeWorktreeRequest {
                    include_uncommitted: true,
                    delete_after_merge: false,
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(refused.contains("merge in progress"), "{refused}");
        assert_eq!(native::conflicts(&meta.root).unwrap().len(), 1);
        std::fs::write(meta.root.join("file"), b"shell resolution\n").unwrap();
        git(&meta.root, &["add", "file"]);
        git(&meta.root, &["commit", "-m", "shell merge resolution"]);
        assert_eq!(
            std::fs::read(service.sync_path(&meta.id).unwrap()).unwrap(),
            journal
        );
        if corrupt {
            std::fs::write(service.sync_path(&meta.id).unwrap(), b"invalid journal").unwrap();
        }
        let restarted = super::super::worktree_registry_tests::test_service(
            service.cache_dir.clone(),
            service.source_workspace_root.clone(),
        )
        .unwrap();
        let landed = restarted
            .merge_worktree(
                &meta.id,
                MergeWorktreeRequest {
                    delete_after_merge: false,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(landed.merged);
        assert_eq!(
            std::fs::read(service.source_workspace_root().join("file")).unwrap(),
            b"shell resolution\n"
        );
        let deleted = restarted
            .delete_worktree(&meta.id, true, true)
            .await
            .unwrap();
        assert!(!meta.root.exists());
        assert!(deleted.deleted);
    }
}

#[tokio::test]
async fn native_sync_real_conflicts_restart_cas_ancestry_and_landing() {
    let (_temp, service, meta) = fixture().await;
    commit(&meta.root, "file", b"source\n");
    commit(service.source_workspace_root(), "file", b"target\n");
    let target_head = native::head(service.source_workspace_root()).unwrap();
    let response = start(&service, &meta, false).await;
    assert_eq!(response.status, WorktreeSyncStatus::Conflicts);
    assert_eq!(response.conflicts.len(), 1);
    assert_eq!(response.conflicts[0].path, "file");
    assert_eq!(
        native::head(service.source_workspace_root()).unwrap(),
        target_head
    );
    let op = response.operation.unwrap();
    let service = super::super::worktree_registry_tests::test_service(
        service.cache_dir.clone(),
        service.source_workspace_root.clone(),
    )
    .unwrap();
    assert_eq!(
        service
            .sync_worktree(
                &meta.id,
                WorktreeSyncRequest::Status {
                    target_branch: None
                }
            )
            .await
            .unwrap()
            .status,
        WorktreeSyncStatus::Conflicts
    );
    let resolved = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Resolve {
                operation_id: op.operation_id.clone(),
                expected_revision: op.revision,
                resolutions: vec![WorktreeSyncResolution {
                    path: "file".into(),
                    choice: WorktreeSyncResolutionChoice::Source,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(resolved.status, WorktreeSyncStatus::Ready);
    assert!(service
        .sync_worktree(&meta.id, cont(&op))
        .await
        .unwrap_err()
        .contains("Stale"));
    let completed = service
        .sync_worktree(&meta.id, cont(resolved.operation.as_ref().unwrap()))
        .await
        .unwrap();
    assert_eq!(completed.status, WorktreeSyncStatus::Completed);
    let repo = git2::Repository::open(&meta.root).unwrap();
    let commit = repo.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(commit.parent_count(), 2);
    assert_eq!(commit.parent_id(1).unwrap().to_string(), target_head);
    assert_eq!(
        native::head(service.source_workspace_root()).unwrap(),
        target_head
    );
    let landed = service
        .merge_worktree(
            &meta.id,
            MergeWorktreeRequest {
                delete_after_merge: false,
                target_branch: Some("main".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(landed.merged);
    assert_eq!(
        std::fs::read(service.source_workspace_root().join("file")).unwrap(),
        b"source\n"
    );
}

#[tokio::test]
async fn native_sync_checkpoint_force_abort_archives_edited_ignored_and_untracked_bytes() {
    let (_temp, service, meta) = fixture().await;
    std::fs::write(meta.root.join("file"), b"checkpoint source\n").unwrap();
    commit(service.source_workspace_root(), "file", b"target\n");
    let response = start(&service, &meta, true).await;
    let op = response.operation.unwrap();
    assert!(op.checkpoint_commit.is_some());
    std::fs::write(meta.root.join("file"), b"precious resolution\0").unwrap();
    std::fs::write(meta.root.join("notes"), b"untracked precious\0").unwrap();
    std::fs::write(meta.root.join(".gitignore"), b"ignored\n").unwrap();
    std::fs::write(meta.root.join("ignored"), b"ignored precious\0").unwrap();
    assert!(service
        .sync_worktree(&meta.id, abort(&op, false))
        .await
        .unwrap_err()
        .contains("would discard"));
    let before = native::snapshot(&meta.root).unwrap();
    let aborted = service
        .sync_worktree(&meta.id, abort(&op, true))
        .await
        .unwrap();
    assert_eq!(aborted.status, WorktreeSyncStatus::Aborted);
    let path = aborted.operation.unwrap().recovery_path.unwrap();
    let archived: native::Snapshot = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(archived == before);
    assert_eq!(
        std::fs::read(meta.root.join("file")).unwrap(),
        b"checkpoint source\n"
    );
    assert_eq!(
        std::fs::read(meta.root.join("notes")).unwrap(),
        b"untracked precious\0"
    );
    assert_eq!(native::head(&meta.root).unwrap(), op.source_head);
}

#[tokio::test]
async fn native_sync_add_add_delete_modify_binary_and_target_advance() {
    let (_temp, service, meta) = fixture().await;
    commit(&meta.root, "source-only", b"source-only change");
    commit(&meta.root, "added", b"source add\n");
    commit(service.source_workspace_root(), "added", b"target add\n");
    commit(&meta.root, "file", b"source\0binary");
    git(service.source_workspace_root(), &["rm", "file"]);
    git(service.source_workspace_root(), &["commit", "-m", "delete"]);
    let response = start(&service, &meta, false).await;
    assert_eq!(response.conflicts.len(), 2);
    assert!(response.conflicts.iter().any(|c| c.kind == "add_add"));
    assert!(response.conflicts.iter().any(|c| c.kind == "delete_modify"));
    let op = response.operation.unwrap();
    let resolved = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Resolve {
                operation_id: op.operation_id,
                expected_revision: op.revision,
                resolutions: vec![
                    WorktreeSyncResolution {
                        path: "added".into(),
                        choice: WorktreeSyncResolutionChoice::Target,
                    },
                    WorktreeSyncResolution {
                        path: "file".into(),
                        choice: WorktreeSyncResolutionChoice::Delete,
                    },
                ],
            },
        )
        .await
        .unwrap();
    commit(service.source_workspace_root(), "advanced", b"advance");
    assert_eq!(
        service
            .sync_worktree(
                &meta.id,
                WorktreeSyncRequest::Status {
                    target_branch: None
                }
            )
            .await
            .unwrap()
            .status,
        WorktreeSyncStatus::TargetAdvanced
    );
    let completed = service
        .sync_worktree(&meta.id, cont(resolved.operation.as_ref().unwrap()))
        .await
        .unwrap();
    assert_eq!(completed.status, WorktreeSyncStatus::Completed);
    let landed = service
        .merge_worktree(
            &meta.id,
            MergeWorktreeRequest {
                delete_after_merge: false,
                target_branch: Some("main".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(landed.merged);
    assert_eq!(
        std::fs::read(service.source_workspace_root().join("advanced")).unwrap(),
        b"advance"
    );
}

#[tokio::test]
async fn native_sync_authority_and_batch_validation_preserve_index() {
    let (_temp, service, meta) = fixture().await;
    commit(&meta.root, "file", b"ours");
    commit(service.source_workspace_root(), "file", b"theirs");
    let op = start(&service, &meta, false).await.operation.unwrap();
    let before = native::output(&meta.root, &["ls-files", "--stage", "-z"]).unwrap();
    let request = WorktreeSyncRequest::Resolve {
        operation_id: op.operation_id.clone(),
        expected_revision: op.revision,
        resolutions: vec![
            WorktreeSyncResolution {
                path: "file".into(),
                choice: WorktreeSyncResolutionChoice::Source,
            },
            WorktreeSyncResolution {
                path: "../escape".into(),
                choice: WorktreeSyncResolutionChoice::Delete,
            },
        ],
    };
    assert!(service
        .sync_worktree(&meta.id, request)
        .await
        .unwrap_err()
        .contains("Unsafe literal"));
    assert_eq!(
        native::output(&meta.root, &["ls-files", "--stage", "-z"]).unwrap(),
        before
    );
    let denied = service.with_merge_authority(Arc::new(|| Err("revoked lease".into())));
    assert!(denied
        .sync_worktree(&meta.id, abort(&op, true))
        .await
        .unwrap_err()
        .contains("revoked lease"));
    assert_eq!(
        native::output(&meta.root, &["ls-files", "--stage", "-z"]).unwrap(),
        before
    );
}

#[tokio::test]
async fn native_sync_restore_preserves_merge_and_refuses_newer_edits() {
    let (_temp, service, meta) = fixture().await;
    commit(&meta.root, "file", b"ours");
    commit(service.source_workspace_root(), "file", b"theirs");
    let op = start(&service, &meta, false).await.operation.unwrap();
    std::fs::write(meta.root.join("file"), b"precious resolution").unwrap();
    let before = native::output(&meta.root, &["ls-files", "--stage", "-z"]).unwrap();
    let aborted = service
        .sync_worktree(&meta.id, abort(&op, true))
        .await
        .unwrap()
        .operation
        .unwrap();
    std::fs::write(meta.root.join("file"), b"newer edit").unwrap();
    let restore = WorktreeSyncRequest::Restore {
        archive_id: None,
        operation_id: aborted.operation_id.clone(),
        expected_revision: aborted.revision,
    };
    assert!(service
        .sync_worktree(&meta.id, restore.clone())
        .await
        .unwrap_err()
        .contains("newer work"));
    assert_eq!(
        std::fs::read(meta.root.join("file")).unwrap(),
        b"newer edit"
    );
    std::fs::write(meta.root.join("file"), b"ours").unwrap();
    let restored = service.sync_worktree(&meta.id, restore).await.unwrap();
    assert_eq!(restored.status, WorktreeSyncStatus::Conflicts);
    assert_eq!(
        std::fs::read(meta.root.join("file")).unwrap(),
        b"precious resolution"
    );
    assert_eq!(
        native::output(&meta.root, &["ls-files", "--stage", "-z"]).unwrap(),
        before
    );
    assert!(git::has_merge_in_progress(&meta.root));
    assert!(Path::new(
        restored
            .operation
            .as_ref()
            .unwrap()
            .recovery_path
            .as_ref()
            .unwrap()
    )
    .exists());
}

#[tokio::test]
async fn native_sync_reconcile_interrupted_merge_and_checkpoint() {
    let (_temp, service, meta) = fixture().await;
    commit(&meta.root, "file", b"ours");
    commit(service.source_workspace_root(), "file", b"theirs");
    start(&service, &meta, false).await;
    let mut journal = service.sync_journal(&meta.id).unwrap().unwrap();
    journal.operation.phase = "merging".into();
    journal.scope.clear();
    journal.abort_fingerprint = None;
    service.sync_save(&journal).unwrap();
    let recovered = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Status {
                target_branch: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(recovered.status, WorktreeSyncStatus::Conflicts);
    assert!(service
        .sync_journal(&meta.id)
        .unwrap()
        .unwrap()
        .scope
        .is_empty());
    assert_eq!(recovered.conflicts[0].path, "file");
    service
        .sync_worktree(&meta.id, abort(recovered.operation.as_ref().unwrap(), true))
        .await
        .unwrap();
    std::fs::write(meta.root.join("file"), b"checkpoint crash").unwrap();
    let mut journal = service.sync_journal(&meta.id).unwrap().unwrap();
    journal.operation.phase = "checkpointing".into();
    journal.status = WorktreeSyncStatus::RecoveryRequired;
    journal.original_head = native::head(&meta.root).unwrap();
    git(&meta.root, &["add", "--all"]);
    journal.checkpoint_tree = Some(git(&meta.root, &["write-tree"]));
    service.sync_save(&journal).unwrap();
    git(&meta.root, &["commit", "-m", "checkpoint crash"]);
    let checkpoint = native::head(&meta.root).unwrap();
    let recovered = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Status {
                target_branch: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        recovered
            .operation
            .as_ref()
            .unwrap()
            .checkpoint_commit
            .as_deref(),
        Some(checkpoint.as_str())
    );
    let aborted = service
        .sync_worktree(&meta.id, abort(recovered.operation.as_ref().unwrap(), true))
        .await
        .unwrap();
    assert_eq!(aborted.status, WorktreeSyncStatus::Aborted);
    assert_eq!(native::head(&meta.root).unwrap(), checkpoint);
}

#[tokio::test]
async fn native_sync_restage_followup_and_refresh_completed_target() {
    let (_temp, service, meta) = fixture().await;
    commit(&meta.root, "file", b"ours");
    commit(service.source_workspace_root(), "file", b"theirs");
    let op = start(&service, &meta, false).await.operation.unwrap();
    let first = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Resolve {
                operation_id: op.operation_id,
                expected_revision: op.revision,
                resolutions: vec![WorktreeSyncResolution {
                    path: "file".into(),
                    choice: WorktreeSyncResolutionChoice::Source,
                }],
            },
        )
        .await
        .unwrap();
    std::fs::write(meta.root.join("file"), b"followup").unwrap();
    std::fs::write(meta.root.join("new"), b"new followup").unwrap();
    let op = first.operation.unwrap();
    let ready = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Resolve {
                operation_id: op.operation_id,
                expected_revision: op.revision,
                resolutions: vec![
                    WorktreeSyncResolution {
                        path: "file".into(),
                        choice: WorktreeSyncResolutionChoice::WorkingTree,
                    },
                    WorktreeSyncResolution {
                        path: "new".into(),
                        choice: WorktreeSyncResolutionChoice::WorkingTree,
                    },
                ],
            },
        )
        .await
        .unwrap();
    service
        .sync_worktree(&meta.id, cont(ready.operation.as_ref().unwrap()))
        .await
        .unwrap();
    commit(service.source_workspace_root(), "advance", b"advance");
    let status = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Status {
                target_branch: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(status.status, WorktreeSyncStatus::TargetAdvanced);
    assert_ne!(
        status.target_head.as_ref(),
        Some(&status.operation.as_ref().unwrap().target_head)
    );
    let next = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Start {
                target_branch: status.target_branch,
                expected_source_head: status.source_head,
                expected_target_head: status.target_head.unwrap(),
                checkpoint: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(next.status, WorktreeSyncStatus::Ready);
}

#[tokio::test]
async fn native_sync_identity_replacement_and_missing_source_keep_evidence() {
    let (_temp, service, meta) = fixture().await;
    commit(&meta.root, "file", b"ours");
    commit(service.source_workspace_root(), "file", b"theirs");
    let op = start(&service, &meta, false).await.operation.unwrap();
    let held = meta.root.with_extension("held");
    std::fs::rename(&meta.root, &held).unwrap();
    assert!(service.sync_journal(&meta.id).unwrap().is_some());
    std::fs::create_dir(&meta.root).unwrap();
    git(&meta.root, &["init", "-b", "refact/chat/sync"]);
    commit(&meta.root, "unrelated", b"never touch");
    assert!(service
        .sync_worktree(&meta.id, abort(&op, true))
        .await
        .is_err());
    assert_eq!(
        std::fs::read(meta.root.join("unrelated")).unwrap(),
        b"never touch"
    );
}

#[tokio::test]
async fn native_sync_resume_restore_and_precommit_checkpoint_recovery() {
    let (_temp, service, meta) = fixture().await;
    commit(&meta.root, "file", b"ours");
    commit(service.source_workspace_root(), "file", b"theirs");
    let op = start(&service, &meta, false).await.operation.unwrap();
    let aborted = service
        .sync_worktree(&meta.id, abort(&op, true))
        .await
        .unwrap();
    let mut journal = service.sync_journal(&meta.id).unwrap().unwrap();
    journal.operation.phase = "restoring".into();
    journal.status = WorktreeSyncStatus::RecoveryRequired;
    service.sync_save(&journal).unwrap();
    let result = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Restore {
                archive_id: None,
                operation_id: aborted.operation.as_ref().unwrap().operation_id.clone(),
                expected_revision: aborted.operation.as_ref().unwrap().revision,
            },
        )
        .await;
    assert!(
        result.is_ok(),
        "interrupted restore must remain natively resumable: {result:?}"
    );
    service
        .sync_worktree(
            &meta.id,
            abort(result.unwrap().operation.as_ref().unwrap(), true),
        )
        .await
        .unwrap();
    let original_archive = journal.operation.recovery_path.clone();
    assert_eq!(
        service
            .sync_journal(&meta.id)
            .unwrap()
            .unwrap()
            .operation
            .recovery_path,
        original_archive
    );
}

#[tokio::test]
async fn native_sync_completed_pin_does_not_restrict_landing_target() {
    let (_temp, service, meta) = fixture().await;
    commit(&meta.root, "source", b"source");
    git(service.source_workspace_root(), &["branch", "other"]);
    let sync = start(&service, &meta, false).await;
    assert_eq!(sync.status, WorktreeSyncStatus::Completed);
    let other_head = native::target(&meta.root, "other").unwrap();
    let landed = service
        .merge_worktree(
            &meta.id,
            MergeWorktreeRequest {
                target_branch: Some("other".into()),
                delete_after_merge: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(landed.merged);
    assert_ne!(native::target(&meta.root, "other").unwrap(), other_head);
    assert_eq!(git(&meta.root, &["show", "other:source"]), "source");
}

#[tokio::test]
async fn native_sync_corrupt_journal_recover_and_missing_root_archive() {
    let (_temp, service, meta) = fixture().await;
    commit(&meta.root, "file", b"ours");
    commit(service.source_workspace_root(), "file", b"theirs");
    start(&service, &meta, false).await;
    std::fs::write(service.sync_path(&meta.id).unwrap(), b"broken journal").unwrap();
    let status = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Status {
                target_branch: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(status.status, WorktreeSyncStatus::RecoveryRequired);
    let recovered = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Recover {
                expected_journal_digest: status.recovery_journal_digest.unwrap(),
            },
        )
        .await
        .unwrap();
    assert_eq!(recovered.status, WorktreeSyncStatus::Conflicts);
    let held = meta.root.with_extension("held");
    std::fs::rename(&meta.root, &held).unwrap();
    let status = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Status {
                target_branch: None,
            },
        )
        .await
        .unwrap();
    let archived = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Recover {
                expected_journal_digest: status.recovery_journal_digest.unwrap(),
            },
        )
        .await
        .unwrap();
    assert_eq!(archived.status, WorktreeSyncStatus::Aborted);
    assert!(held.join("file").exists());
}

#[tokio::test]
async fn native_sync_restore_resumes_each_atomic_write_and_precommit_source() {
    let (_temp, service, meta) = fixture().await;
    commit(&meta.root, "file", b"ours");
    commit(service.source_workspace_root(), "file", b"theirs");
    let op = start(&service, &meta, false).await.operation.unwrap();
    service
        .sync_worktree(&meta.id, abort(&op, true))
        .await
        .unwrap();
    let mut journal = service.sync_journal(&meta.id).unwrap().unwrap();
    let archive = PathBuf::from(journal.operation.recovery_path.as_ref().unwrap());
    journal.restore_preimages =
        native::restore_preimages(&meta.root, &archive, &journal.git_dir).unwrap();
    journal.operation.phase = "restoring".into();
    journal.status = WorktreeSyncStatus::RecoveryRequired;
    service.sync_save(&journal).unwrap();
    for cursor in 0..journal.restore_preimages.len() {
        native::restore_step(
            &meta.root,
            &archive,
            &journal.git_dir,
            cursor,
            &journal.restore_preimages[cursor],
            || Ok(()),
        )
        .unwrap();
        assert_eq!(
            native::head(&meta.root).unwrap(),
            journal.operation.source_head
        );
        assert!(native::restore_step(
            &meta.root,
            &archive,
            &journal.git_dir,
            cursor,
            &journal.restore_preimages[cursor],
            || Ok(())
        )
        .unwrap());
        journal.restore_cursor = cursor + 1;
        service.sync_save(&journal).unwrap();
    }
    let restored = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Restore {
                archive_id: None,
                operation_id: journal.operation.operation_id.clone(),
                expected_revision: journal.operation.revision,
            },
        )
        .await
        .unwrap();
    assert_eq!(restored.status, WorktreeSyncStatus::Conflicts);
    service
        .sync_worktree(&meta.id, abort(restored.operation.as_ref().unwrap(), true))
        .await
        .unwrap();
    let mut journal = service.sync_journal(&meta.id).unwrap().unwrap();
    journal.operation.recovery_path = None;
    journal.archive_digest = None;
    journal.recovery_archives.clear();
    journal.restore_archive_id = None;
    journal.operation.phase = "checkpointing".into();
    journal.status = WorktreeSyncStatus::RecoveryRequired;
    std::fs::write(meta.root.join("file"), b"dirty precommit").unwrap();
    git(&meta.root, &["add", "file"]);
    journal.checkpoint_tree = Some(git(&meta.root, &["write-tree"]));
    service.sync_save(&journal).unwrap();
    let aborted = service
        .sync_worktree(&meta.id, abort(&journal.operation, true))
        .await
        .unwrap();
    let restored = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Restore {
                archive_id: None,
                operation_id: aborted.operation.as_ref().unwrap().operation_id.clone(),
                expected_revision: aborted.operation.as_ref().unwrap().revision,
            },
        )
        .await
        .unwrap();
    assert_eq!(restored.status, WorktreeSyncStatus::Aborted);
    assert_eq!(restored.operation.as_ref().unwrap().phase, "restored");
    assert_eq!(
        std::fs::read(meta.root.join("file")).unwrap(),
        b"dirty precommit"
    );
}

#[test]
fn native_sync_strict_requests() {
    assert_eq!(
        serde_json::from_str::<WorktreeSyncRequest>(r#"{"action":"status"}"#).unwrap(),
        WorktreeSyncRequest::Status {
            target_branch: None
        }
    );
    assert_eq!(
        serde_json::from_str::<WorktreeSyncRequest>(
            r#"{"action":"status","target_branch":"release"}"#
        )
        .unwrap(),
        WorktreeSyncRequest::Status {
            target_branch: Some("release".into())
        }
    );
    assert!(
        serde_json::from_str::<WorktreeSyncRequest>(r#"{"action":"status","force":true}"#).is_err()
    );
    assert!(serde_json::from_str::<WorktreeSyncRequest>(
        r#"{"action":"continue","operation_id":"x","expected_revision":1,"force":true}"#
    )
    .is_err());
}

#[tokio::test]
async fn native_sync_status_pins_requested_target_after_completed_integration() {
    let (_temp, service, meta) = fixture().await;
    git(service.source_workspace_root(), &["branch", "release"]);
    commit(&meta.root, "local", b"local work\n");
    commit(service.source_workspace_root(), "incoming", b"main work\n");
    let ready = start(&service, &meta, false).await;
    let completed = service
        .sync_worktree(&meta.id, cont(ready.operation.as_ref().unwrap()))
        .await
        .unwrap();
    assert_eq!(completed.status, WorktreeSyncStatus::Completed);

    let release = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Status {
                target_branch: Some("release".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(release.status, WorktreeSyncStatus::Idle);
    assert_eq!(release.target_branch.as_deref(), Some("release"));
    assert!(release.operation.is_none());
    let pinned_release = release.target_head.unwrap();
    let synchronized = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Start {
                target_branch: Some("release".into()),
                expected_source_head: release.source_head,
                expected_target_head: pinned_release.clone(),
                checkpoint: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(synchronized.status, WorktreeSyncStatus::Completed);
    assert_eq!(synchronized.operation.unwrap().target_head, pinned_release);
    assert!(service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Status {
                target_branch: Some("missing-target".into()),
            },
        )
        .await
        .is_err());
    let landed = service
        .merge_worktree(
            &meta.id,
            MergeWorktreeRequest {
                target_branch: Some("release".into()),
                delete_after_merge: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(landed.merged);
    assert_eq!(landed.target_branch, "release");
}

#[cfg(unix)]
#[tokio::test]
async fn native_sync_round2_directory_restore() {
    use std::os::unix::fs::PermissionsExt;
    let (_temp, service, meta) = fixture().await;
    commit(&meta.root, "file", b"ours");
    commit(service.source_workspace_root(), "file", b"theirs");
    let op = start(&service, &meta, false).await.operation.unwrap();
    std::fs::remove_file(meta.root.join("file")).unwrap();
    std::fs::create_dir(meta.root.join("file")).unwrap();
    std::fs::set_permissions(
        meta.root.join("file"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    std::fs::write(meta.root.join("file/precious"), b"saved child").unwrap();
    let aborted = service
        .sync_worktree(&meta.id, abort(&op, true))
        .await
        .unwrap()
        .operation
        .unwrap();
    assert!(meta.root.join("file").is_file());
    let restored = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Restore {
                archive_id: None,
                operation_id: aborted.operation_id,
                expected_revision: aborted.revision,
            },
        )
        .await
        .unwrap();
    assert_eq!(restored.status, WorktreeSyncStatus::Conflicts);
    assert_eq!(
        std::fs::read(meta.root.join("file/precious")).unwrap(),
        b"saved child"
    );
    assert_eq!(
        std::fs::metadata(meta.root.join("file"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(native::head(&meta.root).unwrap(), op.source_head);
}

#[tokio::test]
async fn native_sync_round2_recover_rekeys_old_cas() {
    let (_temp, service, meta) = fixture().await;
    commit(&meta.root, "file", b"ours");
    commit(service.source_workspace_root(), "file", b"theirs");
    let old = start(&service, &meta, false).await.operation.unwrap();
    let mut journal = service.sync_journal(&meta.id).unwrap().unwrap();
    journal.operation.revision += 1;
    service.sync_save(&journal).unwrap();
    std::fs::write(service.sync_path(&meta.id).unwrap(), b"corrupt").unwrap();
    let before = native::output(&meta.root, &["ls-files", "--stage", "-z"]).unwrap();
    let recovered = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Recover {
                expected_journal_digest: service.sync_journal_digest(&meta.id).unwrap(),
            },
        )
        .await
        .unwrap()
        .operation
        .unwrap();
    assert_ne!(recovered.operation_id, old.operation_id);
    assert!(service
        .sync_worktree(&meta.id, abort(&old, true))
        .await
        .unwrap_err()
        .contains("Stale"));
    assert_eq!(
        native::output(&meta.root, &["ls-files", "--stage", "-z"]).unwrap(),
        before
    );
    let resolved = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Resolve {
                operation_id: recovered.operation_id,
                expected_revision: recovered.revision,
                resolutions: vec![WorktreeSyncResolution {
                    path: "file".into(),
                    choice: WorktreeSyncResolutionChoice::Source,
                }],
            },
        )
        .await
        .unwrap();
    assert_eq!(
        service
            .sync_worktree(&meta.id, cont(resolved.operation.as_ref().unwrap()))
            .await
            .unwrap()
            .status,
        WorktreeSyncStatus::Completed
    );
}

#[tokio::test]
async fn native_sync_cross_initiator_mutation_keeps_revision_cas() {
    let (_temp, service, meta) = fixture().await;
    commit(&meta.root, "file", b"ours");
    commit(service.source_workspace_root(), "file", b"theirs");
    let op = start(&service.with_sync_actor("chat:first".into()), &meta, false)
        .await
        .operation
        .unwrap();
    let other = service.with_sync_actor("chat:other".into());
    let ready = other
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Resolve {
                operation_id: op.operation_id.clone(),
                expected_revision: op.revision,
                resolutions: vec![WorktreeSyncResolution {
                    path: "file".into(),
                    choice: WorktreeSyncResolutionChoice::Source,
                }],
            },
        )
        .await
        .unwrap();
    let before = native::output(&meta.root, &["ls-files", "--stage", "-z"]).unwrap();
    assert!(other
        .sync_worktree(&meta.id, cont(&op))
        .await
        .unwrap_err()
        .contains("Stale"));
    assert_eq!(
        native::output(&meta.root, &["ls-files", "--stage", "-z"]).unwrap(),
        before
    );
    let completed = other
        .sync_worktree(&meta.id, cont(ready.operation.as_ref().unwrap()))
        .await
        .unwrap();
    assert_eq!(completed.status, WorktreeSyncStatus::Completed);
    assert_eq!(std::fs::read(meta.root.join("file")).unwrap(), b"ours");
}

#[cfg(unix)]
#[tokio::test]
async fn native_sync_round2_cached_directory_atomic_windows_and_linear_reads() {
    use std::os::unix::fs::PermissionsExt;
    let (_temp, service, meta) = fixture().await;
    commit(&meta.root, "file", b"ours");
    commit(service.source_workspace_root(), "file", b"theirs");
    let op = start(&service, &meta, false).await.operation.unwrap();
    std::fs::remove_file(meta.root.join("file")).unwrap();
    std::fs::create_dir(meta.root.join("file")).unwrap();
    std::fs::set_permissions(
        meta.root.join("file"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    for i in 0..12 {
        std::fs::write(
            meta.root.join(format!("file/child{i:02}")),
            vec![i as u8; 4096],
        )
        .unwrap();
    }
    service
        .sync_worktree(&meta.id, abort(&op, true))
        .await
        .unwrap();
    let mut j = service.sync_journal(&meta.id).unwrap().unwrap();
    let archive = PathBuf::from(j.operation.recovery_path.as_ref().unwrap());
    native::ARCHIVE_READS.with(|n| n.set(0));
    let cached = native::CachedRestore::load(&archive).unwrap();
    let preimages = cached.preimages(&meta.root, &j.git_dir).unwrap();
    cached
        .validate_entries(&meta.root, &j.git_dir, &preimages, 0)
        .unwrap();
    // Stop in the portable file-displaced / directory-not-yet-published window.
    let calls = std::cell::Cell::new(0);
    assert!(cached
        .step(&meta.root, &j.git_dir, 0, &preimages[0], || {
            calls.set(calls.get() + 1);
            if calls.get() == 4 {
                Err("injected after displacement".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err()
        .contains("injected"));
    assert!(!meta.root.join("file").exists());
    cached
        .validate_entries(&meta.root, &j.git_dir, &preimages, 0)
        .unwrap();
    cached
        .step(&meta.root, &j.git_dir, 0, &preimages[0], || Ok(()))
        .unwrap();
    std::fs::write(meta.root.join("file/newer"), b"never discard").unwrap();
    assert!(cached
        .validate_entries(&meta.root, &j.git_dir, &preimages, 0)
        .unwrap_err()
        .contains("unknown child"));
    assert_eq!(
        std::fs::read(meta.root.join("file/newer")).unwrap(),
        b"never discard"
    );
    std::fs::remove_file(meta.root.join("file/newer")).unwrap();
    for (cursor, preimage) in preimages.iter().enumerate() {
        cached
            .validate_entries(&meta.root, &j.git_dir, &preimages, cursor)
            .unwrap();
        cached
            .step(&meta.root, &j.git_dir, cursor, preimage, || Ok(()))
            .unwrap();
        // Atomic write happened but durable cursor has not advanced: replay must be safe.
        cached
            .step(&meta.root, &j.git_dir, cursor, preimage, || Ok(()))
            .unwrap();
    }
    assert_eq!(native::ARCHIVE_READS.with(|n| n.get()), 1);
    assert_eq!(
        std::fs::metadata(meta.root.join("file"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(native::head(&meta.root).unwrap(), op.source_head);
    j.restore_preimages = preimages;
    j.restore_cursor = 0;
    service.sync_save(&j).unwrap();
    let bytes = std::fs::read(service.sync_path(&meta.id).unwrap()).unwrap();
    let compact: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(compact["restore_preimages"], serde_json::json!([]));
    assert_eq!(
        compact["restore_preimages_digest"].as_str().unwrap().len(),
        64
    );
    assert_eq!(
        service
            .sync_journal(&meta.id)
            .unwrap()
            .unwrap()
            .restore_preimages,
        j.restore_preimages
    );
}

#[cfg(unix)]
#[tokio::test]
async fn native_sync_readonly_directory_restore() {
    use std::os::unix::fs::PermissionsExt;
    if std::process::Command::new("id")
        .arg("-u")
        .output()
        .unwrap()
        .stdout
        == b"0\n"
    {
        eprintln!("read-only permission regression requires non-root uid; skipped");
        return;
    }
    let (_temp, service, meta) = fixture().await;
    commit(&meta.root, "file", b"ours");
    commit(service.source_workspace_root(), "file", b"theirs");
    let op = start(&service, &meta, false).await.operation.unwrap();
    std::fs::remove_file(meta.root.join("file")).unwrap();
    std::fs::create_dir(meta.root.join("file")).unwrap();
    std::fs::set_permissions(
        meta.root.join("file"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    std::fs::write(meta.root.join("file/precious"), b"saved child").unwrap();
    std::fs::set_permissions(
        meta.root.join("file"),
        std::fs::Permissions::from_mode(0o555),
    )
    .unwrap();
    let aborted = service
        .sync_worktree(&meta.id, abort(&op, true))
        .await
        .unwrap()
        .operation
        .unwrap();
    assert!(meta.root.join("file").is_file());
    let restored = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Restore {
                archive_id: None,
                operation_id: aborted.operation_id,
                expected_revision: aborted.revision,
            },
        )
        .await
        .unwrap();
    assert_eq!(restored.status, WorktreeSyncStatus::Conflicts);
    assert_eq!(
        std::fs::read(meta.root.join("file/precious")).unwrap(),
        b"saved child"
    );
    assert_eq!(
        std::fs::metadata(meta.root.join("file"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o555
    );
    assert_eq!(native::head(&meta.root).unwrap(), op.source_head);
}

#[cfg(unix)]
#[tokio::test]
async fn native_sync_readonly_directory_atomic_windows() {
    use std::os::unix::fs::PermissionsExt;
    let (_temp, service, meta) = fixture().await;
    commit(&meta.root, "file", b"ours");
    commit(service.source_workspace_root(), "file", b"theirs");
    let op = start(&service, &meta, false).await.operation.unwrap();
    std::fs::remove_file(meta.root.join("file")).unwrap();
    std::fs::create_dir(meta.root.join("file")).unwrap();
    std::fs::set_permissions(
        meta.root.join("file"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    for i in 0..12 {
        std::fs::write(
            meta.root.join(format!("file/child{i:02}")),
            vec![i as u8; 4096],
        )
        .unwrap();
    }
    std::fs::create_dir(meta.root.join("file/nested")).unwrap();
    std::fs::write(meta.root.join("file/nested/child"), b"nested saved").unwrap();
    for directory in ["file/nested", "file"] {
        std::fs::set_permissions(
            meta.root.join(directory),
            std::fs::Permissions::from_mode(0o555),
        )
        .unwrap();
    }
    service
        .sync_worktree(&meta.id, abort(&op, true))
        .await
        .unwrap();
    let mut j = service.sync_journal(&meta.id).unwrap().unwrap();
    let archive = PathBuf::from(j.operation.recovery_path.as_ref().unwrap());
    native::ARCHIVE_READS.with(|n| n.set(0));
    let cached = native::CachedRestore::load(&archive).unwrap();
    let preimages = cached.preimages(&meta.root, &j.git_dir).unwrap();
    cached
        .validate_entries(&meta.root, &j.git_dir, &preimages, 0)
        .unwrap();
    // Stop in the portable file-displaced / directory-not-yet-published window.
    let calls = std::cell::Cell::new(0);
    assert!(cached
        .step(&meta.root, &j.git_dir, 0, &preimages[0], || {
            calls.set(calls.get() + 1);
            if calls.get() == 4 {
                Err("injected after displacement".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err()
        .contains("injected"));
    assert!(!meta.root.join("file").exists());
    cached
        .validate_entries(&meta.root, &j.git_dir, &preimages, 0)
        .unwrap();
    cached
        .step(&meta.root, &j.git_dir, 0, &preimages[0], || Ok(()))
        .unwrap();
    std::fs::write(meta.root.join("file/newer"), b"never discard").unwrap();
    assert!(cached
        .validate_entries(&meta.root, &j.git_dir, &preimages, 0)
        .unwrap_err()
        .contains("unknown child"));
    assert_eq!(
        std::fs::read(meta.root.join("file/newer")).unwrap(),
        b"never discard"
    );
    std::fs::remove_file(meta.root.join("file/newer")).unwrap();
    for (cursor, preimage) in preimages.iter().enumerate() {
        cached
            .validate_entries(&meta.root, &j.git_dir, &preimages, cursor)
            .unwrap();
        let calls = std::cell::Cell::new(0);
        let injected = cached.step(&meta.root, &j.git_dir, cursor, preimage, || {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                Err("injected before publication".into())
            } else {
                Ok(())
            }
        });
        if let Err(error) = injected {
            assert!(error.contains("injected"), "{error}");
        }
        cached
            .validate_entries(&meta.root, &j.git_dir, &preimages, cursor)
            .unwrap();
        let calls = std::cell::Cell::new(0);
        let injected = cached.step(&meta.root, &j.git_dir, cursor, preimage, || {
            calls.set(calls.get() + 1);
            if calls.get() == 3 {
                Err("injected after final chmod".into())
            } else {
                Ok(())
            }
        });
        if let Err(error) = injected {
            assert!(error.contains("injected"), "{error}");
        }
        cached
            .validate_entries(&meta.root, &j.git_dir, &preimages, cursor)
            .unwrap();
        cached
            .step(&meta.root, &j.git_dir, cursor, preimage, || Ok(()))
            .unwrap();
        // Atomic write happened but durable cursor has not advanced: replay must be safe.
        cached
            .step(&meta.root, &j.git_dir, cursor, preimage, || Ok(()))
            .unwrap();
    }
    assert_eq!(native::ARCHIVE_READS.with(|n| n.get()), 1);
    assert_eq!(
        std::fs::metadata(meta.root.join("file"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o555
    );
    assert_eq!(native::head(&meta.root).unwrap(), op.source_head);
    j.restore_preimages = preimages;
    j.restore_cursor = 0;
    service.sync_save(&j).unwrap();
    let bytes = std::fs::read(service.sync_path(&meta.id).unwrap()).unwrap();
    let compact: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(compact["restore_preimages"], serde_json::json!([]));
    assert_eq!(
        compact["restore_preimages_digest"].as_str().unwrap().len(),
        64
    );
    assert_eq!(
        service
            .sync_journal(&meta.id)
            .unwrap()
            .unwrap()
            .restore_preimages,
        j.restore_preimages
    );
}

#[cfg(unix)]
#[tokio::test]
async fn native_sync_discovers_and_selects_immutable_abort_archives() {
    use std::os::unix::fs::PermissionsExt;
    let (_temp, service, meta) = fixture().await;
    commit(&meta.root, "file", b"ours");
    commit(service.source_workspace_root(), "file", b"theirs");
    let op = start(&service, &meta, false).await.operation.unwrap();
    std::fs::remove_file(meta.root.join("file")).unwrap();
    std::fs::create_dir_all(meta.root.join("file/nested")).unwrap();
    std::fs::write(
        meta.root.join("file/nested/resolution"),
        b"first resolution",
    )
    .unwrap();
    for dir in ["file/nested", "file"] {
        std::fs::set_permissions(meta.root.join(dir), std::fs::Permissions::from_mode(0o555))
            .unwrap();
    }
    let first = service
        .sync_worktree(&meta.id, abort(&op, true))
        .await
        .unwrap();
    let original = std::fs::read(&first.recovery_archives[0].path).unwrap();
    let restore = |response: &WorktreeSyncResponse, archive_id: Option<String>| {
        WorktreeSyncRequest::Restore {
            operation_id: response.operation.as_ref().unwrap().operation_id.clone(),
            expected_revision: response.operation.as_ref().unwrap().revision,
            archive_id,
        }
    };
    let restored = service
        .sync_worktree(&meta.id, restore(&first, None))
        .await
        .unwrap();
    std::fs::set_permissions(
        meta.root.join("file/nested"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    std::fs::write(
        meta.root.join("file/nested/resolution"),
        b"second resolution",
    )
    .unwrap();
    std::fs::set_permissions(
        meta.root.join("file/nested"),
        std::fs::Permissions::from_mode(0o555),
    )
    .unwrap();
    let second = service
        .sync_worktree(&meta.id, abort(restored.operation.as_ref().unwrap(), true))
        .await
        .unwrap();
    assert_eq!(second.recovery_archives.len(), 2);
    assert_eq!(second.recovery_archives[0].archive_id, "primary");
    assert_ne!(
        second.recovery_archives[0].path,
        second.recovery_archives[1].path
    );
    assert_eq!(
        original,
        std::fs::read(&second.recovery_archives[0].path).unwrap()
    );
    assert!(service
        .sync_worktree(&meta.id, restore(&first, None))
        .await
        .unwrap_err()
        .contains("Stale"));
    assert!(service
        .sync_worktree(&meta.id, restore(&second, Some("/tmp/arbitrary".into())))
        .await
        .unwrap_err()
        .contains("Unknown"));
    let status = service
        .sync_worktree(
            &meta.id,
            WorktreeSyncRequest::Status {
                target_branch: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(status.recovery_archives, second.recovery_archives);
    let later_id = second.recovery_archives[1].archive_id.clone();
    let later_path = &second.recovery_archives[1].path;
    let later_bytes = std::fs::read(later_path).unwrap();
    std::fs::write(later_path, b"altered manifest").unwrap();
    assert!(service
        .sync_worktree(&meta.id, restore(&status, Some(later_id.clone())))
        .await
        .unwrap_err()
        .contains("manifest changed"));
    std::fs::write(later_path, &later_bytes).unwrap();
    std::fs::write(meta.root.join("file"), b"newer work").unwrap();
    assert!(service
        .sync_worktree(&meta.id, restore(&status, Some(later_id.clone())))
        .await
        .unwrap_err()
        .contains("changed after abort"));
    std::fs::write(meta.root.join("file"), b"ours").unwrap();

    let later = service
        .sync_worktree(&meta.id, restore(&status, Some(later_id.clone())))
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(meta.root.join("file/nested/resolution")).unwrap(),
        b"second resolution"
    );
    let aborted = service
        .sync_worktree(&meta.id, abort(later.operation.as_ref().unwrap(), true))
        .await
        .unwrap();
    let earlier = service
        .sync_worktree(&meta.id, restore(&aborted, Some("primary".into())))
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(meta.root.join("file/nested/resolution")).unwrap(),
        b"first resolution"
    );
    let mut journal = service.sync_journal(&meta.id).unwrap().unwrap();
    journal.operation.phase = "restoring".into();
    journal.status = WorktreeSyncStatus::RecoveryRequired;
    service.sync_save(&journal).unwrap();
    assert!(service
        .sync_worktree(&meta.id, restore(&earlier, Some(later_id)))
        .await
        .unwrap_err()
        .contains("partial restore"));
    assert_eq!(
        original,
        std::fs::read(&second.recovery_archives[0].path).unwrap()
    );
}
