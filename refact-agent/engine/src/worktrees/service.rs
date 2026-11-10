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
                .begin_scope_change(&keys)
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
    async fn merge_drains_unregistered_target_primary_namespace_before_effects() {
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
        let merging = service.merge_worktree(
            &created.worktree.meta.id,
            crate::worktrees::types::MergeWorktreeRequest {
                target_branch: Some("landing".into()),
                delete_after_merge: false,
                ..Default::default()
            },
        );
        tokio::pin!(merging);
        tokio::select! {
            result = &mut merging => panic!("merge before primary target drain: {result:?}"),
            _ = async { while !cancel.load(Ordering::Acquire) { tokio::task::yield_now().await; } } => {}
        }
        assert!(!target.join("candidate.txt").exists());
        for key in [key, managed_key] {
            assert!(matches!(
                scopes.acquire(&[key], Arc::new(AtomicBool::new(false))),
                Err(ScopeChangeError::Closed)
            ));
        }
        lease.settle();
        assert!(merging.await.unwrap().merged);
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
            Err(ScopeChangeError::Closed)
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
            .fence_merge_worktree(&created.worktree.meta.id, Some("main"))
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
            Err(ScopeChangeError::Closed)
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
