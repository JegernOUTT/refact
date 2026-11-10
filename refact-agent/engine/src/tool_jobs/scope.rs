use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScopeKey {
    Chat {
        project: std::path::PathBuf,
        chat_id: String,
    },
    Worktree {
        project: std::path::PathBuf,
        root: std::path::PathBuf,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScopeChangeError {
    Closed,
    Busy,
    InvalidKey,
}

#[derive(Default)]
struct Domain {
    closed: bool,
    permanent: bool,
    active: usize,
}
struct Registration {
    keys: Vec<ScopeKey>,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
struct State {
    domains: HashMap<ScopeKey, Domain>,
    jobs: HashMap<u64, Registration>,
    next: u64,
    shutdown: bool,
}

#[derive(Default)]
pub struct ScopeCoordinator {
    state: Mutex<State>,
    changed: Notify,
}

pub struct ExecutionLease {
    coordinator: Arc<ScopeCoordinator>,
    id: u64,
}
pub struct ScopeChangeGuard {
    coordinator: Arc<ScopeCoordinator>,
    keys: Vec<ScopeKey>,
}

impl ScopeCoordinator {
    fn keys(keys: &[ScopeKey]) -> Result<Vec<ScopeKey>, ScopeChangeError> {
        if keys.is_empty()
            || keys.iter().any(|key| match key {
                ScopeKey::Chat {
                    project,
                    chat_id,
                } => project.as_os_str().is_empty() || chat_id.is_empty(),
                ScopeKey::Worktree {
                    project,
                    root,
                } => project.as_os_str().is_empty() || root.as_os_str().is_empty(),
            })
        {
            return Err(ScopeChangeError::InvalidKey);
        }
        Ok(keys
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect())
    }

    pub fn acquire(
        self: &Arc<Self>,
        keys: &[ScopeKey],
        cancel: Arc<AtomicBool>,
    ) -> Result<ExecutionLease, ScopeChangeError> {
        let keys = Self::keys(keys)?;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.shutdown
            || keys
                .iter()
                .any(|key| state.domains.get(key).is_some_and(|d| d.closed))
        {
            return Err(ScopeChangeError::Closed);
        }
        for key in &keys {
            state.domains.entry(key.clone()).or_default().active += 1;
        }
        state.next += 1;
        let id = state.next;
        state.jobs.insert(
            id,
            Registration {
                keys,
                cancel,
            },
        );
        Ok(ExecutionLease {
            coordinator: self.clone(),
            id,
        })
    }

    pub async fn begin_scope_change(
        self: &Arc<Self>,
        keys: &[ScopeKey],
    ) -> Result<ScopeChangeGuard, ScopeChangeError> {
        let keys = Self::keys(keys)?;
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let acquired = {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                if state.shutdown
                    || keys
                        .iter()
                        .any(|key| state.domains.get(key).is_some_and(|d| d.permanent))
                {
                    return Err(ScopeChangeError::Closed);
                }
                if keys
                    .iter()
                    .any(|key| state.domains.get(key).is_some_and(|d| d.closed))
                {
                    false
                } else {
                    for key in &keys {
                        state.domains.entry(key.clone()).or_default().closed = true;
                    }
                    for job in state.jobs.values() {
                        if job.keys.iter().any(|key| keys.contains(key)) {
                            job.cancel.store(true, Ordering::Release);
                        }
                    }
                    true
                }
            };
            if acquired {
                break;
            }
            notified.await;
        }
        let guard = ScopeChangeGuard {
            coordinator: self.clone(),
            keys,
        };
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                if state.shutdown
                    || guard
                        .keys
                        .iter()
                        .any(|key| state.domains.get(key).is_some_and(|d| d.permanent))
                {
                    return Err(ScopeChangeError::Closed);
                }
                if guard
                    .keys
                    .iter()
                    .all(|key| state.domains.get(key).is_none_or(|d| d.active == 0))
                {
                    return Ok(guard);
                }
            }
            notified.await;
        }
    }

    pub async fn drain_shutdown(&self) -> Result<(), ScopeChangeError> {
        self.begin_shutdown();
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .jobs
                .is_empty()
            {
                return Ok(());
            }
            notified.await;
        }
    }

    pub fn begin_shutdown(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.shutdown = true;
        for domain in state.domains.values_mut() {
            domain.closed = true;
            domain.permanent = true;
        }
        for job in state.jobs.values() {
            job.cancel.store(true, Ordering::Release);
        }
        self.changed.notify_waiters();
    }
}

impl ExecutionLease {
    pub fn settle(self) {
        drop(self);
    }
}
impl Drop for ExecutionLease {
    fn drop(&mut self) {
        let mut state = self
            .coordinator
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(job) = state.jobs.remove(&self.id) {
            for key in job.keys {
                state
                    .domains
                    .get_mut(&key)
                    .expect("registered domain")
                    .active -= 1;
            }
        }
        self.coordinator.changed.notify_waiters();
    }
}
impl ScopeChangeGuard {
    pub fn close_permanently(self) {
        let mut state = self
            .coordinator
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for key in &self.keys {
            if let Some(domain) = state.domains.get_mut(key) {
                domain.permanent = true;
            }
        }
    }
}
impl std::fmt::Display for ScopeChangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ScopeChangeError {}

impl Drop for ScopeChangeGuard {
    fn drop(&mut self) {
        let mut state = self
            .coordinator
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if state.shutdown {
            return;
        }
        for key in &self.keys {
            if let Some(domain) = state.domains.get_mut(key) {
                if !domain.permanent {
                    domain.closed = false;
                }
            }
        }
        self.coordinator.changed.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key() -> ScopeKey {
        ScopeKey::Chat {
            project: "/project".into(),
            chat_id: "chat".into(),
        }
    }

    #[tokio::test]
    async fn transition_cancels_then_drains_and_fences_new_admission() {
        let coordinator = Arc::new(ScopeCoordinator::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let lease = coordinator.acquire(&[key()], cancel.clone()).unwrap();
        let keys = [key()];
        let transition = coordinator.begin_scope_change(&keys);
        tokio::pin!(transition);
        assert!(futures::poll!(&mut transition).is_pending());
        assert!(cancel.load(Ordering::Acquire));
        assert!(matches!(
            coordinator.acquire(&keys, cancel.clone()),
            Err(ScopeChangeError::Closed)
        ));
        lease.settle();
        let guard = transition.await.unwrap();
        assert!(matches!(
            coordinator.acquire(&keys, cancel.clone()),
            Err(ScopeChangeError::Closed)
        ));
        drop(guard);
        coordinator.acquire(&keys, cancel).unwrap().settle();
    }

    #[tokio::test]
    async fn cancelled_transition_can_retry_without_skipping_active_work() {
        let coordinator = Arc::new(ScopeCoordinator::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let lease = coordinator.acquire(&[key()], cancel.clone()).unwrap();
        let keys = [key()];
        let mut transition = Box::pin(coordinator.begin_scope_change(&keys));
        assert!(futures::poll!(transition.as_mut()).is_pending());
        drop(transition);
        coordinator
            .acquire(&keys, Arc::new(AtomicBool::new(false)))
            .unwrap()
            .settle();
        let mut retry = Box::pin(coordinator.begin_scope_change(&keys));
        assert!(futures::poll!(retry.as_mut()).is_pending());
        drop(lease);
        let guard = retry.await.unwrap();
        guard.close_permanently();
        assert!(matches!(
            coordinator.acquire(&keys, cancel),
            Err(ScopeChangeError::Closed)
        ));
        assert!(matches!(
            coordinator.begin_scope_change(&keys).await,
            Err(ScopeChangeError::Closed)
        ));
    }

    #[tokio::test]
    async fn released_execution_does_not_poison_scope_but_shutdown_stays_closed() {
        let coordinator = Arc::new(ScopeCoordinator::default());
        let cancel = Arc::new(AtomicBool::new(false));
        drop(coordinator.acquire(&[key()], cancel.clone()).unwrap());
        drop(coordinator.begin_scope_change(&[key()]).await.unwrap());
        let lease = coordinator.acquire(&[key()], cancel.clone()).unwrap();
        let mut shutdown = Box::pin(coordinator.drain_shutdown());
        assert!(futures::poll!(shutdown.as_mut()).is_pending());
        assert!(cancel.load(Ordering::Acquire));
        drop(lease);
        shutdown.await.unwrap();
        assert!(matches!(
            coordinator.acquire(&[key()], cancel),
            Err(ScopeChangeError::Closed)
        ));
    }
}
