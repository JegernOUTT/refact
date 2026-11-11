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
    Cancelled,
}

#[derive(Default)]
struct Domain {
    closed: bool,
    permanent: bool,
    active: usize,
    operation: Option<String>,
}
struct Registration {
    keys: Vec<ScopeKey>,
    cancel: Arc<AtomicBool>,
    label: String,
}
#[derive(Default)]
struct State {
    domains: HashMap<ScopeKey, Domain>,
    jobs: HashMap<u64, Registration>,
    next: u64,
    shutdown: bool,
    shutdown_request: Option<String>,
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
    started: std::time::Instant,
    operation: String,
}

impl ScopeCoordinator {
    pub fn is_shutdown(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .shutdown
    }

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
                .any(|key| state.domains.get(key).is_some_and(|d| d.permanent))
        {
            return Err(ScopeChangeError::Closed);
        }
        if keys
            .iter()
            .any(|key| state.domains.get(key).is_some_and(|d| d.closed))
        {
            return Err(ScopeChangeError::Busy);
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
                label: "unlabelled".into(),
            },
        );
        Ok(ExecutionLease {
            coordinator: self.clone(),
            id,
        })
    }

    pub async fn wait_open(&self, keys: &[ScopeKey]) -> Result<(), ScopeChangeError> {
        let keys = Self::keys(keys)?;
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                if state.shutdown
                    || keys
                        .iter()
                        .any(|key| state.domains.get(key).is_some_and(|d| d.permanent))
                {
                    return Err(ScopeChangeError::Closed);
                }
                if keys
                    .iter()
                    .all(|key| state.domains.get(key).is_none_or(|d| !d.closed))
                {
                    return Ok(());
                }
            }
            tokio::select! {
                _ = &mut notified => {},
                _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {
                    let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                    let fences: Vec<_> = keys.iter().filter_map(|key| {
                        state.domains.get(key).filter(|domain| domain.closed)
                            .map(|domain| (key, &domain.operation))
                    }).collect();
                    tracing::info!(?fences, "tool admission waiting for scope fence release");
                }
            }
        }
    }

    pub async fn acquire_wait(
        self: &Arc<Self>,
        keys: &[ScopeKey],
        cancel: Arc<AtomicBool>,
    ) -> Result<ExecutionLease, ScopeChangeError> {
        loop {
            if self.is_shutdown() {
                return Err(ScopeChangeError::Closed);
            }
            if cancel.load(Ordering::Acquire) {
                return Err(ScopeChangeError::Cancelled);
            }
            match self.acquire(keys, cancel.clone()) {
                Err(ScopeChangeError::Busy) => {}
                result => return result,
            }
            let waiting = self.wait_open(keys);
            tokio::pin!(waiting);
            loop {
                tokio::select! {
                    result = &mut waiting => { result?; break; },
                    _ = tokio::time::sleep(refact_work_core::coordination_settings().lock_poll_interval()) => {
                        if cancel.load(Ordering::Acquire) {
                            return Err(ScopeChangeError::Cancelled);
                        }
                    }
                }
            }
        }
    }

    pub fn log_blockers(&self, keys: Option<&[ScopeKey]>, operation: &str) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let blockers: Vec<_> = state
            .jobs
            .iter()
            .filter(|(_, job)| {
                keys.is_none_or(|keys| job.keys.iter().any(|key| keys.contains(key)))
            })
            .map(|(id, job)| (id, &job.label, &job.keys))
            .collect();
        tracing::warn!(operation, ?keys, ?blockers, request = ?state.shutdown_request,
            "scope drain waiting for execution owners");
    }

    pub async fn begin_scope_change(
        self: &Arc<Self>,
        keys: &[ScopeKey],
    ) -> Result<ScopeChangeGuard, ScopeChangeError> {
        self.begin_scope_change_named(keys, "scope_change").await
    }

    pub async fn begin_scope_change_named(
        self: &Arc<Self>,
        keys: &[ScopeKey],
        operation: &str,
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
                        let domain = state.domains.entry(key.clone()).or_default();
                        domain.closed = true;
                        domain.operation = Some(operation.into());
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
            started: std::time::Instant::now(),
            operation: operation.into(),
        };
        tracing::info!(operation, keys = ?guard.keys, "scope fence begin");
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
            tokio::select! {
                _ = &mut notified => {},
                _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {
                    self.log_blockers(Some(&guard.keys), operation);
                }
            }
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
            tokio::select! {
                _ = &mut notified => {},
                _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {
                    self.log_blockers(None, "shutdown");
                }
            }
        }
    }

    pub fn shutdown_description(&self) -> String {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        match &state.shutdown_request {
            Some(request) => format!("Engine shutdown requested {request}; tool calls are refused. Restart the engine if it does not exit"),
            None => "Tool execution domain permanently closed".into(),
        }
    }

    pub fn begin_shutdown(&self) {
        self.begin_shutdown_requested("internal shutdown");
    }

    pub fn begin_shutdown_requested(&self, requester: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.shutdown {
            let request = format!("by {requester} at {}", chrono::Utc::now().to_rfc3339());
            tracing::warn!(%request, "engine shutdown requested; scope admission closed");
            state.shutdown_request = Some(request);
        }
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
    pub fn label(&self, label: String) {
        let mut state = self
            .coordinator
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(job) = state.jobs.get_mut(&self.id) {
            job.label = label;
        }
    }

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
        tracing::info!(operation = %self.operation, keys = ?self.keys,
            held_ms = self.started.elapsed().as_millis(), "scope fence release");
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
                    domain.operation = None;
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
    async fn wait_then_admit_and_dropped_waiter_leaves_no_registration() {
        let coordinator = Arc::new(ScopeCoordinator::default());
        let keys = [key()];
        let guard = coordinator.begin_scope_change(&keys).await.unwrap();
        let mut dropped =
            Box::pin(coordinator.acquire_wait(&keys, Arc::new(AtomicBool::new(false))));
        assert!(futures::poll!(&mut dropped).is_pending());
        drop(dropped);
        assert!(coordinator.state.lock().unwrap().jobs.is_empty());
        let mut waiting =
            Box::pin(coordinator.acquire_wait(&keys, Arc::new(AtomicBool::new(false))));
        assert!(futures::poll!(&mut waiting).is_pending());
        drop(guard);
        let lease = waiting.await.unwrap();
        assert_eq!(coordinator.state.lock().unwrap().jobs.len(), 1);
        lease.settle();
        assert!(coordinator.state.lock().unwrap().jobs.is_empty());
    }

    #[tokio::test]
    async fn shutdown_wakes_fenced_waiter_with_closed() {
        let coordinator = Arc::new(ScopeCoordinator::default());
        let keys = [key()];
        let _guard = coordinator.begin_scope_change(&keys).await.unwrap();
        let mut waiting =
            Box::pin(coordinator.acquire_wait(&keys, Arc::new(AtomicBool::new(false))));
        assert!(futures::poll!(&mut waiting).is_pending());
        coordinator.begin_shutdown_requested("test shutdown");
        assert!(matches!(waiting.await, Err(ScopeChangeError::Closed)));
        assert!(coordinator.shutdown_description().contains("test shutdown"));
        assert!(coordinator.state.lock().unwrap().jobs.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn abort_while_waiting_leaves_no_registration() {
        let coordinator = Arc::new(ScopeCoordinator::default());
        let keys = [key()];
        let _guard = coordinator.begin_scope_change(&keys).await.unwrap();
        let abort = Arc::new(AtomicBool::new(false));
        let mut waiting = Box::pin(coordinator.acquire_wait(&keys, abort.clone()));
        assert!(futures::poll!(&mut waiting).is_pending());
        abort.store(true, Ordering::Release);
        assert!(matches!(waiting.await, Err(ScopeChangeError::Cancelled)));
        assert!(coordinator.state.lock().unwrap().jobs.is_empty());
    }

    #[tokio::test]
    async fn abandoned_shutdown_keeps_truthful_restart_diagnostic() {
        let coordinator = Arc::new(ScopeCoordinator::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let lease = coordinator.acquire(&[key()], cancel.clone()).unwrap();
        coordinator.begin_shutdown_requested("HTTP /v1/graceful-shutdown");
        let mut shutdown = Box::pin(coordinator.drain_shutdown());
        assert!(futures::poll!(&mut shutdown).is_pending());
        drop(shutdown);
        assert!(cancel.load(Ordering::Acquire));
        lease.settle();
        assert!(matches!(
            coordinator.acquire(&[key()], cancel),
            Err(ScopeChangeError::Closed)
        ));
        let message = coordinator.shutdown_description();
        assert!(message.contains("HTTP /v1/graceful-shutdown"));
        assert!(message.contains("tool calls are refused"));
        assert!(message.contains("Restart the engine"));
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
            Err(ScopeChangeError::Busy)
        ));
        lease.settle();
        let guard = transition.await.unwrap();
        assert!(matches!(
            coordinator.acquire(&keys, cancel.clone()),
            Err(ScopeChangeError::Busy)
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
