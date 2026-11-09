//! Stable lifecycle domains shared by job admission and scope writers.
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
    UnknownEffects,
    InvalidKey,
}

#[derive(Default)]
struct Domain {
    epoch: u64,
    closed: bool,
    permanent: bool,
    unknown: bool,
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
    settled: bool,
}
pub struct ScopeChangeGuard {
    coordinator: Arc<ScopeCoordinator>,
    keys: Vec<ScopeKey>,
    drained: bool,
}

impl ScopeCoordinator {
    /// Canonical project/root strings must come from the already-validated scope,
    /// never a model argument or an ambient current directory.
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
            settled: false,
        })
    }

    /// Closes every domain atomically BEFORE cancellation. No caller may hold
    /// session, context, board or tool-pool locks while awaiting this method.
    /// Dropping a pending transition leaves admission closed (fail closed).
    pub async fn begin_scope_change(
        self: &Arc<Self>,
        keys: &[ScopeKey],
    ) -> Result<ScopeChangeGuard, ScopeChangeError> {
        let keys = Self::keys(keys)?;
        {
            loop {
                let notified = self.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                let acquired = {
                    let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                    if state.shutdown {
                        return Err(ScopeChangeError::Closed);
                    }
                    if keys
                        .iter()
                        .any(|key| state.domains.get(key).is_some_and(|d| d.unknown))
                    {
                        return Err(ScopeChangeError::UnknownEffects);
                    }
                    if keys
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
                            let domain = state.domains.entry(key.clone()).or_default();
                            domain.closed = true;
                            domain.epoch += 1;
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
        }
        let mut guard = ScopeChangeGuard {
            coordinator: self.clone(),
            keys,
            drained: false,
        };
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                if guard
                    .keys
                    .iter()
                    .any(|key| state.domains.get(key).is_some_and(|d| d.permanent))
                {
                    return Err(ScopeChangeError::UnknownEffects);
                }
                if guard
                    .keys
                    .iter()
                    .all(|key| state.domains.get(key).is_none_or(|d| d.active == 0))
                {
                    guard.drained = true;
                    return Ok(guard);
                }
            }
            notified.await;
        }
    }

    pub(crate) fn mark_unknown(&self, keys: &[ScopeKey]) {
        let keys = match Self::keys(keys) {
            Ok(keys) => keys,
            Err(_) => {
                self.begin_shutdown();
                return;
            }
        };
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        for key in &keys {
            let domain = state.domains.entry(key.clone()).or_default();
            domain.closed = true;
            domain.permanent = true;
            domain.unknown = true;
            domain.epoch += 1;
        }
        for job in state.jobs.values() {
            if job.keys.iter().any(|key| keys.contains(key)) {
                job.cancel.store(true, Ordering::Release);
            }
        }
        self.changed.notify_waiters();
    }

    pub async fn drain_shutdown(&self) -> Result<(), ScopeChangeError> {
        self.begin_shutdown();
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                if state
                    .domains
                    .values()
                    .any(|domain| domain.permanent && domain.unknown)
                {
                    return Err(ScopeChangeError::UnknownEffects);
                }
                if state.jobs.is_empty() {
                    return Ok(());
                }
            }
            notified.await;
        }
    }

    /// Permanently closes admission first, then requests cancellation. Shutdown
    /// may abort tasks, but must not describe those aborts as proven quiescence.
    pub fn begin_shutdown(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.shutdown = true;
        for domain in state.domains.values_mut() {
            domain.closed = true;
            domain.permanent = true;
            domain.epoch += 1;
        }
        for job in state.jobs.values() {
            job.cancel.store(true, Ordering::Release);
        }
        self.changed.notify_waiters();
    }
}

impl ExecutionLease {
    /// Invoke ONLY after the actual effect-owning future has settled and cleanup
    /// established quiescence. Dropping an uncompleted future is not settlement.
    pub fn settle(mut self) {
        self.settled = true;
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
                let domain = state.domains.get_mut(&key).expect("registered domain");
                domain.active -= 1;
                if !self.settled {
                    domain.closed = true;
                    domain.permanent = true;
                    domain.unknown = true;
                    domain.epoch += 1;
                }
            }
        }
        self.coordinator.changed.notify_waiters();
    }
}
impl ScopeChangeGuard {
    /// Closing a chat is permanent; reopening requires a separately authorized
    /// lifecycle rather than implicitly reusing the old execution generation.
    pub fn close_permanently(mut self) {
        self.drained = false;
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
        if !self.drained {
            for key in &self.keys {
                if let Some(domain) = state.domains.get_mut(key) {
                    domain.closed = true;
                    domain.permanent = true;
                }
            }
            self.coordinator.changed.notify_waiters();
            return;
        }
        if state.shutdown {
            return;
        }
        for key in &self.keys {
            if let Some(domain) = state.domains.get_mut(key) {
                if !domain.permanent && domain.active == 0 {
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
    async fn recovered_unknown_domains_cancel_existing_work_and_never_reopen() {
        let coordinator = Arc::new(ScopeCoordinator::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let lease = coordinator.acquire(&[key()], cancel.clone()).unwrap();
        coordinator.mark_unknown(&[key()]);
        assert!(cancel.load(Ordering::Acquire));
        lease.settle();
        assert!(matches!(
            coordinator.acquire(&[key()], Arc::new(AtomicBool::new(false))),
            Err(ScopeChangeError::Closed)
        ));
        assert!(matches!(
            coordinator.begin_scope_change(&[key()]).await,
            Err(ScopeChangeError::UnknownEffects)
        ));
        let other = ScopeKey::Chat {
            project: "/project".into(),
            chat_id: "other".into(),
        };
        coordinator
            .acquire(&[other], Arc::new(AtomicBool::new(false)))
            .unwrap()
            .settle();
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
            coordinator.acquire(&[key()], cancel.clone()),
            Err(ScopeChangeError::Closed)
        ));
        lease.settle();
        let guard = transition.await.unwrap();
        assert!(matches!(
            coordinator.acquire(&[key()], cancel.clone()),
            Err(ScopeChangeError::Closed)
        ));
        drop(guard);
        coordinator.acquire(&[key()], cancel).unwrap().settle();
    }
    #[tokio::test]
    async fn dropped_work_and_interrupted_transition_never_reopen_domains() {
        let coordinator = Arc::new(ScopeCoordinator::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let lease = coordinator.acquire(&[key()], cancel.clone()).unwrap();
        drop(lease);
        assert!(matches!(
            coordinator.acquire(&[key()], cancel),
            Err(ScopeChangeError::Closed)
        ));
        assert!(matches!(
            coordinator.begin_scope_change(&[key()]).await,
            Err(ScopeChangeError::UnknownEffects)
        ));
    }
}
