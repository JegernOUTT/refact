use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScopeKey {
    PythonRun {
        chat_id: String,
        tool_call_id: String,
    },
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
    Unavailable {
        operation: String,
        waited: std::time::Duration,
        blockers: Vec<ScopeBlocker>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeBlocker {
    pub registration_id: u64,
    pub label: String,
    pub keys: Vec<ScopeKey>,
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
                ScopeKey::PythonRun {
                    chat_id,
                    tool_call_id,
                } => chat_id.is_empty() || tool_call_id.is_empty(),
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
        let blockers = Self::blockers(&state, keys);
        tracing::warn!(operation, ?keys, ?blockers, request = ?state.shutdown_request,
            "scope drain waiting for execution owners");
    }

    fn blockers(state: &State, keys: Option<&[ScopeKey]>) -> Vec<ScopeBlocker> {
        let mut blockers = state
            .jobs
            .iter()
            .filter(|(_, job)| {
                keys.is_none_or(|keys| job.keys.iter().any(|key| keys.contains(key)))
            })
            .map(|(id, job)| ScopeBlocker {
                registration_id: *id,
                label: job.label.clone(),
                keys: job.keys.clone(),
            })
            .collect::<Vec<_>>();
        blockers.sort_by_key(|blocker| blocker.registration_id);
        blockers
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
        self.begin_scope_change_inner(keys, operation, None).await
    }

    pub async fn begin_scope_change_drain_bounded(
        self: &Arc<Self>,
        keys: &[ScopeKey],
        operation: &str,
        drain_timeout: std::time::Duration,
    ) -> Result<ScopeChangeGuard, ScopeChangeError> {
        self.begin_scope_change_inner(keys, operation, Some(drain_timeout))
            .await
    }

    async fn begin_scope_change_inner(
        self: &Arc<Self>,
        keys: &[ScopeKey],
        operation: &str,
        drain_timeout: Option<std::time::Duration>,
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
        let drain_started = tokio::time::Instant::now();
        let deadline = drain_timeout.map(|timeout| drain_started + timeout);
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let timed_out = {
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
                deadline
                    .filter(|deadline| tokio::time::Instant::now() >= *deadline)
                    .map(|_| ScopeChangeError::Unavailable {
                        operation: operation.into(),
                        waited: drain_started.elapsed(),
                        blockers: Self::blockers(&state, Some(&guard.keys)),
                    })
            };
            if let Some(error) = timed_out {
                tracing::warn!(%error, "scope fence drain timed out; reopening scope");
                drop(guard);
                return Err(error);
            }
            let deadline_reached = async {
                match deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                _ = &mut notified => {},
                _ = deadline_reached => {},
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
        match self {
            Self::Unavailable {
                operation,
                waited,
                blockers,
            } => {
                write!(
                    f,
                    "{operation}: executions in this scope did not stop within {:.3}s after being asked to stop:",
                    waited.as_secs_f64()
                )?;
                for blocker in blockers {
                    write!(
                        f,
                        " #{} {} keys={:?};",
                        blocker.registration_id, blocker.label, blocker.keys
                    )?;
                }
                Ok(())
            }
            _ => write!(f, "{self:?}"),
        }
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
    use std::sync::atomic::AtomicUsize;
    use std::task::{Context, Wake, Waker};

    fn key() -> ScopeKey {
        ScopeKey::Chat {
            project: "/project".into(),
            chat_id: "chat".into(),
        }
    }

    const DRAIN: std::time::Duration = std::time::Duration::from_secs(120);

    #[tokio::test(start_paused = true)]
    async fn drain_timeout_names_every_blocker_and_reopens_domains() {
        let coordinator = Arc::new(ScopeCoordinator::default());
        let keys = [key()];
        let other = ScopeKey::Worktree {
            project: "/project".into(),
            root: "/project/wt".into(),
        };
        let first = coordinator
            .acquire(&keys, Arc::new(AtomicBool::new(false)))
            .unwrap();
        first.label("tool=shell first-monitor".into());
        let second = coordinator
            .acquire(&[key(), other.clone()], Arc::new(AtomicBool::new(false)))
            .unwrap();
        second.label("tool=python second-run".into());
        let unrelated = coordinator
            .acquire(&[other.clone()], Arc::new(AtomicBool::new(false)))
            .unwrap();
        unrelated.label("tool=shell unrelated".into());
        let mut fence =
            Box::pin(coordinator.begin_scope_change_drain_bounded(&keys, "test mutation", DRAIN));
        assert!(futures::poll!(&mut fence).is_pending());
        tokio::time::advance(DRAIN).await;
        let error = match futures::poll!(&mut fence) {
            std::task::Poll::Ready(Err(error)) => error,
            other => panic!(
                "drain deadline must fail immediately: {:?}",
                other.map(|r| r.is_ok())
            ),
        };
        let mut second_keys = vec![key(), other];
        second_keys.sort();
        assert_eq!(
            error,
            ScopeChangeError::Unavailable {
                operation: "test mutation".into(),
                waited: DRAIN,
                blockers: vec![
                    ScopeBlocker {
                        registration_id: first.id,
                        label: "tool=shell first-monitor".into(),
                        keys: keys.to_vec(),
                    },
                    ScopeBlocker {
                        registration_id: second.id,
                        label: "tool=python second-run".into(),
                        keys: second_keys,
                    },
                ],
            }
        );
        let text = error.to_string();
        assert!(text.contains("tool=shell first-monitor"), "{text}");
        assert!(text.contains("tool=python second-run"), "{text}");
        assert!(!text.contains("unrelated"), "{text}");
        assert!(!text.contains("another scope fence"), "{text}");
        coordinator
            .acquire(&keys, Arc::new(AtomicBool::new(false)))
            .unwrap()
            .settle();
        first.settle();
        second.settle();
        unrelated.settle();
        assert!(coordinator.state.lock().unwrap().jobs.is_empty());
    }

    #[derive(Default)]
    struct WakeCount(AtomicUsize);

    impl Wake for WakeCount {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn drain_timeout_wakes_all_parked_waiters_before_they_are_polled() {
        let coordinator = Arc::new(ScopeCoordinator::default());
        let keys = [key()];
        let lease = coordinator
            .acquire(&keys, Arc::new(AtomicBool::new(false)))
            .unwrap();
        let mut fence =
            Box::pin(coordinator.begin_scope_change_drain_bounded(&keys, "test mutation", DRAIN));
        assert!(futures::poll!(&mut fence).is_pending());
        tokio::time::advance(DRAIN).await;
        let mut waiters = std::array::from_fn::<_, 2, _>(|_| {
            let mut waiter =
                Box::pin(coordinator.acquire_wait(&keys, Arc::new(AtomicBool::new(false))));
            let wakes = Arc::new(WakeCount::default());
            let waker = Waker::from(wakes.clone());
            assert!(
                std::future::Future::poll(waiter.as_mut(), &mut Context::from_waker(&waker))
                    .is_pending()
            );
            assert_eq!(wakes.0.load(Ordering::Relaxed), 0);
            (waiter, wakes)
        });
        assert!(matches!(
            futures::poll!(&mut fence),
            std::task::Poll::Ready(Err(ScopeChangeError::Unavailable { .. }))
        ));
        for (index, (_, wakes)) in waiters.iter().enumerate() {
            assert_ne!(
                wakes.0.load(Ordering::Relaxed),
                0,
                "waiter {index} was not woken by the timed-out fence"
            );
        }
        for (waiter, _) in &mut waiters {
            match futures::poll!(waiter) {
                std::task::Poll::Ready(result) => result.unwrap().settle(),
                std::task::Poll::Pending => panic!("waiter must be admitted after the timeout"),
            }
        }
        lease.settle();
    }

    #[tokio::test(start_paused = true)]
    async fn lease_released_at_drain_deadline_yields_guard() {
        let coordinator = Arc::new(ScopeCoordinator::default());
        let keys = [key()];
        let lease = coordinator
            .acquire(&keys, Arc::new(AtomicBool::new(false)))
            .unwrap();
        let mut fence =
            Box::pin(coordinator.begin_scope_change_drain_bounded(&keys, "test mutation", DRAIN));
        assert!(futures::poll!(&mut fence).is_pending());
        tokio::time::advance(DRAIN).await;
        lease.settle();
        let guard = match futures::poll!(&mut fence) {
            std::task::Poll::Ready(result) => result.expect("success must win the deadline tie"),
            std::task::Poll::Pending => panic!("released lease must complete the fence"),
        };
        assert!(matches!(
            coordinator.acquire(&keys, Arc::new(AtomicBool::new(false))),
            Err(ScopeChangeError::Busy)
        ));
        drop(guard);
    }

    #[tokio::test(start_paused = true)]
    async fn drain_bounded_fence_waits_for_another_fence_without_limit() {
        let coordinator = Arc::new(ScopeCoordinator::default());
        let keys = [key()];
        let guard = coordinator.begin_scope_change(&keys).await.unwrap();
        let mut fence =
            Box::pin(coordinator.begin_scope_change_drain_bounded(&keys, "second mutation", DRAIN));
        assert!(futures::poll!(&mut fence).is_pending());
        tokio::time::advance(DRAIN * 3).await;
        assert!(futures::poll!(&mut fence).is_pending());
        drop(guard);
        let second = tokio::time::timeout(std::time::Duration::from_secs(1), fence)
            .await
            .expect("fence must proceed once the other fence releases")
            .expect("drain with no executions must succeed");
        assert!(matches!(
            coordinator.acquire(&keys, Arc::new(AtomicBool::new(false))),
            Err(ScopeChangeError::Busy)
        ));
        drop(second);
    }

    #[tokio::test(start_paused = true)]
    async fn drain_timeout_budget_starts_after_acquiring_all_keys() {
        let coordinator = Arc::new(ScopeCoordinator::default());
        let a = key();
        let b = ScopeKey::Worktree {
            project: "/project".into(),
            root: "/project/wt".into(),
        };
        let guard = coordinator.begin_scope_change(&[a.clone()]).await.unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let lease = coordinator.acquire(&[b.clone()], cancel.clone()).unwrap();
        let keys = [a, b];
        let mut fence =
            Box::pin(coordinator.begin_scope_change_drain_bounded(&keys, "second mutation", DRAIN));
        assert!(futures::poll!(&mut fence).is_pending());
        tokio::time::advance(DRAIN * 3).await;
        assert!(futures::poll!(&mut fence).is_pending());
        assert!(!cancel.load(Ordering::Acquire));

        drop(guard);
        let after_acquiring = futures::poll!(&mut fence);
        assert!(
            cancel.load(Ordering::Acquire),
            "B's lease was not asked to stop"
        );
        assert!(
            after_acquiring.is_pending(),
            "waiting on A must not consume B's drain budget"
        );
        let one_second = std::time::Duration::from_secs(1);
        tokio::time::advance(DRAIN - one_second).await;
        assert!(futures::poll!(&mut fence).is_pending());
        tokio::time::advance(one_second).await;
        assert!(matches!(
            futures::poll!(&mut fence),
            std::task::Poll::Ready(Err(ScopeChangeError::Unavailable { waited, .. })) if waited == DRAIN
        ));
        lease.settle();
    }

    #[tokio::test(start_paused = true)]
    async fn drain_timeout_lists_only_leases_still_active_at_deadline() {
        let coordinator = Arc::new(ScopeCoordinator::default());
        let keys = [key()];
        let released = coordinator.acquire(&keys, Arc::default()).unwrap();
        released.label("settled before deadline".into());
        let active = coordinator.acquire(&keys, Arc::default()).unwrap();
        active.label("still active at deadline".into());
        let mut fence =
            Box::pin(coordinator.begin_scope_change_drain_bounded(&keys, "test mutation", DRAIN));
        assert!(futures::poll!(&mut fence).is_pending());
        tokio::time::advance(DRAIN / 2).await;
        released.settle();
        assert!(futures::poll!(&mut fence).is_pending());
        tokio::time::advance(DRAIN / 2).await;
        let std::task::Poll::Ready(Err(ScopeChangeError::Unavailable {
            blockers, ..
        })) = futures::poll!(&mut fence)
        else {
            panic!("drain must time out with the second lease still active");
        };
        assert_eq!(
            blockers,
            vec![ScopeBlocker {
                registration_id: active.id,
                label: "still active at deadline".into(),
                keys: keys.to_vec(),
            }]
        );
        active.settle();
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

#[cfg(test)]
mod python_fence_tests {
    use super::*;
    #[tokio::test]
    async fn python_outer_fences_worktree_but_nested_key_stays_open() {
        let coordinator = Arc::new(ScopeCoordinator::default());
        let worktree = ScopeKey::Worktree {
            project: "/p".into(),
            root: "/w".into(),
        };
        let python = ScopeKey::PythonRun {
            chat_id: "chat".into(),
            tool_call_id: "call".into(),
        };
        let owner = coordinator
            .acquire(
                &[worktree.clone(), python.clone()],
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap();
        let keys = [worktree];
        let mut fence = Box::pin(coordinator.begin_scope_change(&keys));
        assert!(futures::poll!(&mut fence).is_pending());
        let nested = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            coordinator.acquire_wait(&[python], Arc::new(AtomicBool::new(false))),
        )
        .await
        .unwrap()
        .unwrap();
        drop(nested);
        assert!(futures::poll!(&mut fence).is_pending());
        drop(owner);
        tokio::time::timeout(std::time::Duration::from_secs(1), fence)
            .await
            .unwrap()
            .unwrap();
    }
}
