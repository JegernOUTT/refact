pub use refact_worktrees::service::*;

use std::path::PathBuf;
use std::sync::Arc;

use crate::tool_jobs::scope::{ScopeCoordinator, ScopeKey};

struct ApplicationEffectFence(Arc<ScopeCoordinator>);

impl std::fmt::Debug for ApplicationEffectFence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApplicationEffectFence")
    }
}

impl WorktreeEffectFence for ApplicationEffectFence {
    fn begin(
        &self,
        project: PathBuf,
        roots: Vec<PathBuf>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<EffectGuard, String>> + Send + '_>>
    {
        Box::pin(async move {
            let mut keys = Vec::new();
            for root in roots {
                for key in [
                    ScopeKey::Worktree {
                        project: project.clone(),
                        root: root.clone(),
                    },
                    ScopeKey::Worktree {
                        project: root.clone(),
                        root,
                    },
                ] {
                    if !keys.contains(&key) {
                        keys.push(key);
                    }
                }
            }
            let guard = self
                .0
                .begin_scope_change_named(&keys, "worktree mutation/delete")
                .await
                .map_err(|error| format!("Worktree change unavailable: {error}"))?;
            Ok(Arc::new(guard) as EffectGuard)
        })
    }
}

/// Bind to this application's coordinator explicitly; cache paths never confer
/// access to a runtime, and the worktree crate has no dependency on the engine.
pub fn fenced_service(service: WorktreeService, scopes: Arc<ScopeCoordinator>) -> WorktreeService {
    service.with_effect_fence(Arc::new(ApplicationEffectFence(scopes)))
}

#[cfg(test)]
mod effect_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use crate::tool_jobs::scope::ScopeChangeError;

    fn fixture() -> (tempfile::TempDir, WorktreeService, Arc<ScopeCoordinator>) {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        for args in [
            vec!["init"],
            vec!["config", "core.autocrlf", "false"],
            vec!["checkout", "-b", "main"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "Test"],
            vec!["commit", "--allow-empty", "-m", "initial"],
        ] {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(&source)
                .output()
                .unwrap()
                .status
                .success());
        }
        let scopes = Arc::new(ScopeCoordinator::default());
        let service = fenced_service(
            WorktreeService::new(temp.path().join("cache"), source).unwrap(),
            scopes.clone(),
        );
        (temp, service, scopes)
    }

    #[tokio::test]
    async fn merge_preserves_target_and_main_jobs_and_admits_new_work() {
        let (temp, service, scopes) = fixture();
        let target = temp.path().join("primary-target");
        assert!(std::process::Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                "landing",
                target.to_str().unwrap(),
                "main"
            ])
            .current_dir(service.source_workspace_root())
            .output()
            .unwrap()
            .status
            .success());
        let created = service
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                base_branch: Some("landing".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        std::fs::write(
            created.worktree.meta.root.join("candidate.txt"),
            "candidate\n",
        )
        .unwrap();
        let key = ScopeKey::Worktree {
            project: target.clone(),
            root: target.clone(),
        };
        let managed_key = ScopeKey::Worktree {
            project: service.source_workspace_root().into(),
            root: target.clone(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let lease = scopes.acquire(&[key.clone()], cancel.clone()).unwrap();
        let main_key = ScopeKey::Worktree {
            project: service.source_workspace_root().into(),
            root: service.source_workspace_root().into(),
        };
        let main_cancel = Arc::new(AtomicBool::new(false));
        let main_lease = scopes
            .acquire(&[main_key.clone()], main_cancel.clone())
            .unwrap();
        let service = service
            .fence_merge_worktree(&created.worktree.meta.id, Some("landing"))
            .await
            .unwrap();
        scopes
            .acquire(&[main_key.clone()], Arc::new(AtomicBool::new(false)))
            .unwrap()
            .settle();
        let merging = service.merge_worktree(
            &created.worktree.meta.id,
            crate::worktrees::types::MergeWorktreeRequest {
                target_branch: Some("landing".into()),
                delete_after_merge: false,
                ..Default::default()
            },
        );
        assert!(merging.await.unwrap().merged);
        assert!(!cancel.load(Ordering::Acquire));
        assert!(!main_cancel.load(Ordering::Acquire));
        main_lease.settle();
        for key in [key, managed_key, main_key] {
            scopes
                .acquire(&[key], Arc::new(AtomicBool::new(false)))
                .unwrap()
                .settle();
        }
        lease.settle();
        assert_eq!(
            std::fs::read_to_string(target.join("candidate.txt")).unwrap(),
            "candidate\n"
        );
    }

    #[tokio::test]
    async fn central_delete_drains_jobs_without_latching_released_execution() {
        let (_temp, service, scopes) = fixture();
        let created = service
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest::default())
            .await
            .unwrap();
        let key = ScopeKey::Worktree {
            project: service.source_workspace_root().into(),
            root: created.worktree.meta.root.clone(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let lease = scopes.acquire(&[key.clone()], cancel.clone()).unwrap();
        let deleting = service.delete_worktree(&created.worktree.meta.id, true, true);
        tokio::pin!(deleting);
        assert!(futures::poll!(&mut deleting).is_pending());
        // Validation is asynchronous: poll until coordinator cancellation is observed.
        tokio::select! {
            result = &mut deleting => panic!("delete before drain: {result:?}"),
            _ = async { while !cancel.load(Ordering::Acquire) { tokio::task::yield_now().await; } } => {}
        }
        assert!(created.worktree.meta.root.exists());
        assert!(matches!(
            scopes.acquire(&[key.clone()], cancel.clone()),
            Err(ScopeChangeError::Busy)
        ));
        lease.settle();
        assert!(deleting.await.unwrap().deleted);

        let created = service
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest::default())
            .await
            .unwrap();
        let key = ScopeKey::Worktree {
            project: service.source_workspace_root().into(),
            root: created.worktree.meta.root.clone(),
        };
        drop(scopes.acquire(&[key], cancel).unwrap());
        assert!(
            service
                .delete_worktree(&created.worktree.meta.id, true, true)
                .await
                .unwrap()
                .deleted
        );
        assert!(!created.worktree.meta.root.exists());
    }

    #[tokio::test]
    async fn aborted_awaiter_keeps_blocking_effect_fenced_until_quiescent() {
        let (_temp, service, scopes) = fixture();
        let created = service
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest::default())
            .await
            .unwrap();
        let key = ScopeKey::Worktree {
            project: service.source_workspace_root().into(),
            root: created.worktree.meta.root.clone(),
        };
        let fenced = service
            .fence_worktrees(&[created.worktree.meta.id.clone()], true)
            .await
            .unwrap()
            .acquire_merge_permit()
            .await
            .unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let barrier = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let worker_barrier = barrier.clone();
        let task = tokio::spawn(async move {
            fenced
                .run_merge_git_phase(move || {
                    started_tx.send(()).unwrap();
                    let (lock, changed) = &*worker_barrier;
                    let mut released = lock.lock().unwrap();
                    while !*released {
                        released = changed.wait(released).unwrap();
                    }
                    Ok(())
                })
                .await
        });
        started_rx.await.unwrap();
        task.abort();
        let _ = task.await;
        let cancel = Arc::new(AtomicBool::new(false));
        assert!(matches!(
            scopes.acquire(&[key.clone()], cancel.clone()),
            Err(ScopeChangeError::Busy)
        ));
        {
            let (lock, changed) = &*barrier;
            *lock.lock().unwrap() = true;
            changed.notify_all();
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Ok(lease) = scopes.acquire(&[key.clone()], cancel.clone()) {
                    lease.settle();
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn unconfigured_service_refuses_before_mutation() {
        let (_temp, service, _scopes) = fixture();
        let created = service
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest::default())
            .await
            .unwrap();
        let raw = WorktreeService::new(
            service
                .registry_dir()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .into(),
            service.source_workspace_root().into(),
        )
        .unwrap();
        assert!(raw
            .delete_worktree(&created.worktree.meta.id, true, true)
            .await
            .unwrap_err()
            .contains("coordinator"));
        assert!(created.worktree.meta.root.exists());
    }
}
