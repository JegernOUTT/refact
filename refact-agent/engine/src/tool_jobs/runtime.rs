use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinHandle;

use crate::call_validation::{ChatMessage, ContextFile};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobBinding {
    /// Backend-only canonical domains; never serialize execution roots to clients.
    #[serde(skip)]
    pub scope_keys: Vec<super::ScopeKey>,
    pub chat_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub model: String,
    /// Frozen turn identity, not the mutable chat's current turn.
    pub turn_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Accepted,
    Running,
    CancelRequested,
    Succeeded,
    Failed,
    /// Executor explicitly established quiescence without successful completion.
    Cancelled,
    /// Effects may have happened. Never replay automatically.
    Unknown,
}

impl JobStatus {
    pub fn is_terminal(self) -> bool {
        self.terminal()
    }

    fn terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Unknown
        )
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct JobResult {
    pub had_corrections: bool,
    pub messages: Vec<ChatMessage>,
    pub context_files: Vec<ContextFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobReceipt {
    pub job_id: String,
    #[serde(flatten)]
    pub binding: JobBinding,
    pub status: JobStatus,
    pub result_url: String,
    pub cancel_url: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct JobSnapshot {
    pub receipt: JobReceipt,
    pub status: JobStatus,
    pub revision: u64,
    pub result: Option<JobResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_unavailable_reason: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub result_released: bool,
}

/// Budget for retained results, measured as their serialized JSON size, not process memory.
pub const RESULT_RETENTION_BYTES: usize = 256 * 1024 * 1024;
/// Terminal records beyond this are evicted oldest first, consumed records only.
pub const TERMINAL_RECORD_CAP: usize = 4096;
/// Last-resort cap on undelivered terminal records, evicted oldest first.
pub const UNDELIVERED_RECORD_CAP: usize = 65_536;
pub const RESULT_NOT_KEPT: &str =
    "result no longer kept: released to bound engine memory; the call was not retried";

struct ByteCount(usize);

impl std::io::Write for ByteCount {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(buf.len());
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn serialized_size(result: &JobResult) -> usize {
    let mut count = ByteCount(0);
    serde_json::to_writer(&mut count, result).map_or(0, |()| count.0)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobEvent {
    pub job_id: String,
    #[serde(flatten)]
    pub binding: JobBinding,
    pub status: JobStatus,
    pub revision: u64,
    pub result_available: bool,
}

pub enum Submission {
    Inline(JobResult),
    Accepted(JobReceipt),
}

/// Delivery candidate without the native payload; landing loads it through its checked path.
#[derive(Clone)]
pub struct TerminalRef {
    pub receipt: JobReceipt,
    pub revision: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Owner {
    Undecided,
    Inline,
    Background,
    InternalReadiness,
}

struct Record {
    receipt: JobReceipt,
    status: JobStatus,
    revision: u64,
    result: Option<JobResult>,
    owner: Owner,
    cancel: Arc<AtomicBool>,
    task: Option<JoinHandle<()>>,
    consumed: bool,
    unavailable: Option<String>,
    result_bytes: usize,
    released: bool,
    finished: u64,
    claiming: bool,
}

struct State {
    records: HashMap<String, Record>,
    invocation_ids: HashMap<String, String>,
    closed: bool,
    retained_bytes: usize,
    finished: u64,
}

pub struct JobRuntime {
    #[cfg(test)]
    test_inline_budget: std::sync::Mutex<Option<Option<Duration>>>,
    pub scopes: Arc<super::ScopeCoordinator>,
    namespace: std::path::PathBuf,
    state: Mutex<State>,
    retention_bytes: usize,
    record_cap: usize,
    undelivered_cap: usize,
    events: broadcast::Sender<JobEvent>,
    changed: tokio::sync::Notify,
    transition: tokio::sync::Mutex<()>,
    bridge: Mutex<Option<(tokio::sync::watch::Sender<bool>, JoinHandle<()>)>>,
    bridge_shutdown: AtomicBool,
}

struct ForegroundClaim {
    runtime: std::sync::Weak<JobRuntime>,
    job_id: String,
}

impl Drop for ForegroundClaim {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.upgrade() {
            let mut state = runtime.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(record) = state.records.get_mut(&self.job_id) {
                record.claiming = false;
                if record.owner == Owner::Undecided {
                    record.owner = Owner::Background;
                    if record.status.terminal() {
                        let _ = runtime.events.send(JobRuntime::event(record));
                    }
                }
            }
            runtime.enforce_retention(&mut state);
        }
    }
}

impl JobRuntime {
    pub fn new(event_capacity: usize, retention_bytes: usize) -> Arc<Self> {
        Self::with_namespace(
            std::path::PathBuf::from("/isolated-tool-job-runtime"),
            event_capacity,
            retention_bytes,
        )
    }

    pub fn with_namespace(
        namespace: std::path::PathBuf,
        event_capacity: usize,
        retention_bytes: usize,
    ) -> Arc<Self> {
        Self::with_limits(
            namespace,
            event_capacity,
            retention_bytes,
            TERMINAL_RECORD_CAP,
            UNDELIVERED_RECORD_CAP,
        )
    }

    pub(crate) fn with_limits(
        namespace: std::path::PathBuf,
        event_capacity: usize,
        retention_bytes: usize,
        record_cap: usize,
        undelivered_cap: usize,
    ) -> Arc<Self> {
        let (events, _) = broadcast::channel(event_capacity.max(1));
        Arc::new(Self {
            #[cfg(test)]
            test_inline_budget: std::sync::Mutex::new(None),
            scopes: Arc::new(super::ScopeCoordinator::default()),
            namespace,
            state: Mutex::new(State {
                records: HashMap::new(),
                invocation_ids: HashMap::new(),
                closed: false,
                retained_bytes: 0,
                finished: 0,
            }),
            retention_bytes,
            record_cap,
            undelivered_cap,
            events,
            changed: tokio::sync::Notify::new(),
            transition: tokio::sync::Mutex::new(()),
            bridge: Mutex::new(None),
            bridge_shutdown: AtomicBool::new(false),
        })
    }

    pub fn install_completion_bridge(
        &self,
        factory: impl FnOnce(tokio::sync::watch::Receiver<bool>) -> JoinHandle<()>,
    ) {
        let mut bridge = self.bridge.lock().unwrap_or_else(|e| e.into_inner());
        if !self.bridge_shutdown.load(Ordering::Acquire) && bridge.is_none() {
            let (stop, receiver) = tokio::sync::watch::channel(false);
            *bridge = Some((stop, factory(receiver)));
        }
    }

    fn enforce_retention(&self, state: &mut State) {
        self.enforce_retention_within(state, self.retention_bytes);
    }

    #[cfg(test)]
    pub(crate) fn release_results_over(&self, budget: usize) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.enforce_retention_within(&mut state, budget);
    }

    fn enforce_retention_within(&self, state: &mut State, budget: usize) {
        let order = |state: &State, consumed: bool, payload: bool| {
            let mut ids = state
                .records
                .iter()
                .filter(|(_, record)| {
                    record.status.terminal()
                        && !record.claiming
                        && record.consumed == consumed
                        && (!payload || record.result.is_some())
                })
                .map(|(id, record)| (record.finished, id.clone()))
                .collect::<Vec<_>>();
            ids.sort_unstable();
            ids.into_iter().map(|(_, id)| id).collect::<Vec<_>>()
        };
        if state.retained_bytes > budget {
            for consumed in [true, false] {
                for id in order(state, consumed, true) {
                    if state.retained_bytes <= budget {
                        break;
                    }
                    if let Some(record) = state.records.get_mut(&id) {
                        record.result = None;
                        record.released = true;
                        record.unavailable = Some(RESULT_NOT_KEPT.into());
                        state.retained_bytes =
                            state.retained_bytes.saturating_sub(record.result_bytes);
                        if !record.consumed && record.owner == Owner::Background {
                            let _ = self.events.send(Self::event(record));
                        }
                    }
                }
            }
        }
        let (consumed, undelivered) = state
            .records
            .values()
            .filter(|record| record.status.terminal() && !record.claiming)
            .fold((0usize, 0usize), |(consumed, undelivered), record| {
                if record.consumed {
                    (consumed + 1, undelivered)
                } else {
                    (consumed, undelivered + 1)
                }
            });
        let consumed_excess = (consumed + undelivered)
            .saturating_sub(self.record_cap)
            .min(consumed);
        let undelivered_excess = undelivered.saturating_sub(self.undelivered_cap);
        let mut evicted = Vec::new();
        if consumed_excess > 0 {
            evicted.extend(order(state, true, false).into_iter().take(consumed_excess));
        }
        if undelivered_excess > 0 {
            evicted.extend(
                order(state, false, false)
                    .into_iter()
                    .take(undelivered_excess),
            );
        }
        for id in evicted {
            if let Some(record) = state.records.remove(&id) {
                if record.result.is_some() {
                    state.retained_bytes = state.retained_bytes.saturating_sub(record.result_bytes);
                }
            }
        }
    }

    pub async fn get_result(
        &self,
        chat_id: &str,
        job_id: &str,
    ) -> Result<Option<JobSnapshot>, String> {
        Ok(self.get(chat_id, job_id))
    }

    pub async fn ack_delivered(
        &self,
        chat_id: &str,
        job_id: &str,
        revision: u64,
    ) -> Result<(), String> {
        let _transition = self.transition.lock().await;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let record = state
            .records
            .get_mut(job_id)
            .filter(|record| record.receipt.binding.chat_id == chat_id)
            .ok_or("tool job not found")?;
        if !record.status.terminal() || record.revision != revision {
            return Err("terminal revision mismatch".into());
        }
        record.consumed = true;
        self.enforce_retention(&mut state);
        Ok(())
    }

    fn terminal_ref(record: &Record) -> TerminalRef {
        TerminalRef {
            receipt: record.receipt.clone(),
            revision: record.revision,
        }
    }

    pub fn chat_scope_key(&self, chat_id: &str) -> super::ScopeKey {
        super::ScopeKey::Chat {
            project: self.namespace.clone(),
            chat_id: chat_id.to_string(),
        }
    }

    pub async fn begin_scope_change(
        self: &Arc<Self>,
        keys: &[super::ScopeKey],
    ) -> Result<super::ScopeChangeGuard, super::ScopeChangeError> {
        self.scopes.begin_scope_change(keys).await
    }

    pub fn subscribe(&self) -> broadcast::Receiver<JobEvent> {
        self.events.subscribe()
    }

    fn event(record: &Record) -> JobEvent {
        JobEvent {
            job_id: record.receipt.job_id.clone(),
            binding: record.receipt.binding.clone(),
            status: record.status,
            revision: record.revision,
            result_available: record.result.is_some(),
        }
    }

    /// Moves ONE invocation into runtime ownership before polling it. The inline
    /// timeout borrows only its completion receiver; it cannot drop/recreate work.
    /// `work` must retain its validated execution lease until it settles.
    #[cfg(test)]
    pub(crate) fn set_test_inline_budget(&self, budget: Option<Duration>) {
        *self.test_inline_budget.lock().unwrap() = Some(budget);
    }

    pub(crate) async fn submit<F, Make>(
        self: &Arc<Self>,
        binding: JobBinding,
        budget: Duration,
        make: Make,
    ) -> Result<Submission, String>
    where
        F: Future<Output = (JobStatus, JobResult)> + Send + 'static,
        Make: FnOnce(Arc<AtomicBool>) -> F,
    {
        #[cfg(test)]
        let budget = self
            .test_inline_budget
            .lock()
            .unwrap()
            .unwrap_or(Some(budget));
        #[cfg(not(test))]
        let budget = Some(budget);
        self.submit_owned(binding, budget, false, None, make).await
    }

    pub(crate) async fn submit_with_abort<F, Make>(
        self: &Arc<Self>,
        binding: JobBinding,
        budget: Duration,
        abort: Arc<AtomicBool>,
        shutdown: Arc<AtomicBool>,
        make: Make,
    ) -> Result<Submission, String>
    where
        F: Future<Output = (JobStatus, JobResult)> + Send + 'static,
        Make: FnOnce(Arc<AtomicBool>) -> F,
    {
        #[cfg(test)]
        let budget = self
            .test_inline_budget
            .lock()
            .unwrap()
            .unwrap_or(Some(budget));
        #[cfg(not(test))]
        let budget = Some(budget);
        self.submit_owned(binding, budget, false, Some((abort, shutdown)), make)
            .await
    }

    pub(crate) async fn submit_background<F, Make>(
        self: &Arc<Self>,
        binding: JobBinding,
        make: Make,
    ) -> Result<JobReceipt, String>
    where
        F: Future<Output = (JobStatus, JobResult)> + Send + 'static,
        Make: FnOnce(Arc<AtomicBool>) -> F,
    {
        match self.submit_owned(binding, None, false, None, make).await? {
            Submission::Accepted(receipt) => Ok(receipt),
            Submission::Inline(_) => unreachable!("background submission never claims inline"),
        }
    }

    pub(crate) async fn submit_readiness<F, Make>(
        self: &Arc<Self>,
        binding: JobBinding,
        make: Make,
    ) -> Result<JobReceipt, String>
    where
        F: Future<Output = (JobStatus, JobResult)> + Send + 'static,
        Make: FnOnce(Arc<AtomicBool>) -> F,
    {
        match self.submit_owned(binding, None, true, None, make).await? {
            Submission::Accepted(receipt) => Ok(receipt),
            Submission::Inline(_) => unreachable!(),
        }
    }

    pub(crate) async fn ack_readiness(
        &self,
        chat_id: &str,
        job_id: &str,
        revision: u64,
    ) -> Result<(), String> {
        let internal = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .records
            .get(job_id)
            .is_some_and(|record| record.owner == Owner::InternalReadiness);
        if !internal {
            return Err("not a private readiness owner".into());
        }
        self.ack_delivered(chat_id, job_id, revision).await
    }

    fn check_admission_abort(
        &self,
        abort: Option<&(Arc<AtomicBool>, Arc<AtomicBool>)>,
    ) -> Result<(), String> {
        if let Some((abort, shutdown)) = abort {
            if self.scopes.is_shutdown() {
                return Err(self.scopes.shutdown_description());
            }
            if shutdown.load(Ordering::Relaxed) {
                return Err("Engine shutdown requested; tool calls are refused. Restart the engine if it does not exit".into());
            }
            if abort.load(Ordering::Relaxed) {
                return Err("Aborted while waiting for tool admission".into());
            }
        }
        Ok(())
    }

    fn scope_error(&self, error: super::scope::ScopeChangeError) -> String {
        if error == super::scope::ScopeChangeError::Closed {
            self.scopes.shutdown_description()
        } else {
            format!("Tool execution scope unavailable: {error}")
        }
    }

    async fn submit_owned<F, Make>(
        self: &Arc<Self>,
        binding: JobBinding,
        budget: Option<Duration>,
        internal_readiness: bool,
        abort: Option<(Arc<AtomicBool>, Arc<AtomicBool>)>,
        make: Make,
    ) -> Result<Submission, String>
    where
        F: Future<Output = (JobStatus, JobResult)> + Send + 'static,
        Make: FnOnce(Arc<AtomicBool>) -> F,
    {
        let mut binding = binding;
        let chat_key = self.chat_scope_key(&binding.chat_id);
        if !binding.scope_keys.contains(&chat_key) {
            binding.scope_keys.push(chat_key);
        }
        let invocation_id = serde_json::to_string(&binding).map_err(|error| error.to_string())?;
        let job_id = format!("tooljob_{}", uuid::Uuid::new_v4());
        let base = format!("/v1/chats/{}/tool-jobs/{job_id}", binding.chat_id);
        let receipt = JobReceipt {
            job_id: job_id.clone(),
            binding,
            status: JobStatus::Accepted,
            result_url: base.clone(),
            cancel_url: format!("{base}/cancel"),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, mut rx) = oneshot::channel();
        let (transition, lease) = loop {
            let waiting = self.scopes.wait_open(&receipt.binding.scope_keys);
            tokio::pin!(waiting);
            loop {
                self.check_admission_abort(abort.as_ref())?;
                tokio::select! {
                    result = &mut waiting => {
                        result.map_err(|error| self.scope_error(error))?;
                        break;
                    },
                    _ = tokio::time::sleep(refact_work_core::coordination_settings().lock_poll_interval()) => {}
                }
            }
            let transition = self.transition.lock().await;
            self.check_admission_abort(abort.as_ref())?;
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return Err("tool job runtime is shut down".into());
            }
            if state.invocation_ids.contains_key(&invocation_id) {
                return Err(
                    "tool invocation already owned; inspect existing result, do not replay".into(),
                );
            }
            let lease = match self
                .scopes
                .acquire(&receipt.binding.scope_keys, cancel.clone())
            {
                Ok(lease) => lease,
                Err(super::scope::ScopeChangeError::Busy) => continue,
                Err(error) => return Err(self.scope_error(error)),
            };
            lease.label(format!(
                "tool={} chat={} job={}",
                receipt.binding.tool_name, receipt.binding.chat_id, job_id
            ));
            state.invocation_ids.insert(invocation_id, job_id.clone());
            state.records.insert(
                job_id.clone(),
                Record {
                    receipt: receipt.clone(),
                    status: JobStatus::Running,
                    revision: 1,
                    result: None,
                    owner: if internal_readiness {
                        Owner::InternalReadiness
                    } else {
                        Owner::Undecided
                    },
                    cancel: cancel.clone(),
                    task: None,
                    consumed: false,
                    unavailable: None,
                    result_bytes: 0,
                    released: false,
                    finished: 0,
                    claiming: true,
                },
            );
            break (transition, lease);
        };
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let runtime = Arc::downgrade(self);
            let id = job_id.clone();
            // Construction itself may perform effects and panic. Keep that
            // failure inside the same owned terminal path as a polling panic.
            let future = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| make(cancel)));
            let task = tokio::spawn(async move {
                use futures::FutureExt;
                let completed = match future {
                    Ok(future) => std::panic::AssertUnwindSafe(future).catch_unwind().await,
                    Err(panic) => Err(panic),
                };
                drop(lease);
                let Some(runtime) = runtime.upgrade() else {
                    return;
                };
                let (mut status, result) = match completed {
                    Ok(value) => value,
                    Err(_) => (
                        JobStatus::Unknown,
                        JobResult {
                            had_corrections: false,
                            messages: vec![],
                            context_files: vec![],
                        },
                    ),
                };
                if !status.terminal() {
                    status = JobStatus::Unknown;
                }
                let result_bytes = serialized_size(&result);
                let _transition = runtime.transition.lock().await;
                let mut state = runtime.state.lock().unwrap_or_else(|e| e.into_inner());
                state.finished += 1;
                let finished = state.finished;
                let mut retained = 0;
                if let Some(record) = state.records.get_mut(&id) {
                    if !record.status.terminal() {
                        record.status = status;
                        record.result = Some(result);
                        record.result_bytes = result_bytes;
                        record.finished = finished;
                        retained = result_bytes;
                        record.revision += 1;
                        if record.owner == Owner::Background {
                            let _ = runtime.events.send(Self::event(record));
                        }
                    }
                }
                state.retained_bytes = state.retained_bytes.saturating_add(retained);
                runtime.enforce_retention(&mut state);
                drop(state);
                runtime.changed.notify_waiters();
                let _ = tx.send(());
            });
            state.records.get_mut(&job_id).expect("inserted job").task = Some(task);
        }
        drop(transition);
        let _claim = ForegroundClaim {
            runtime: Arc::downgrade(self),
            job_id: job_id.clone(),
        };
        let completed_inline = match budget {
            Some(budget) => tokio::time::timeout(budget, &mut rx).await.is_ok(),
            None => false,
        };
        let _transition = self.transition.lock().await;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let record = state.records.get_mut(&job_id).ok_or("job ownership lost")?;
        record.claiming = false;
        // Completion may race timeout. Claim under the same mutex used by completion.
        let inline = match record.result.clone() {
            Some(result) if completed_inline => {
                record.owner = Owner::Inline;
                record.consumed = true;
                Some(result)
            }
            _ => {
                if !internal_readiness {
                    record.owner = Owner::Background;
                }
                if record.status.terminal() && record.owner == Owner::Background {
                    let _ = self.events.send(Self::event(record));
                }
                None
            }
        };
        self.enforce_retention(&mut state);
        Ok(match inline {
            Some(result) => Submission::Inline(result),
            None => Submission::Accepted(receipt),
        })
    }

    #[cfg(test)]
    pub(crate) fn jobs_for_chat(&self, chat: &str) -> usize {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .records
            .values()
            .filter(|record| record.receipt.binding.chat_id == chat)
            .count()
    }

    #[cfg(test)]
    pub(crate) fn evict_record_for_test(&self, job_id: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(record) = state.records.remove(job_id) {
            if record.result.is_some() {
                state.retained_bytes = state.retained_bytes.saturating_sub(record.result_bytes);
            }
        }
    }

    pub(crate) fn knows(&self, chat_id: &str, job_id: &str) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .records
            .get(job_id)
            .is_some_and(|record| record.receipt.binding.chat_id == chat_id)
    }

    pub fn get(&self, chat_id: &str, job_id: &str) -> Option<JobSnapshot> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let record = state.records.get(job_id)?;
        if record.receipt.binding.chat_id != chat_id {
            return None;
        }
        Some(JobSnapshot {
            receipt: record.receipt.clone(),
            status: record.status,
            revision: record.revision,
            result: record.result.clone(),
            result_unavailable_reason: record.unavailable.clone(),
            result_released: record.released,
        })
    }

    /// Race-free completion wait; ownership is checked on every read. Waiting
    /// never consumes a result, changes delivery ownership or replays work.
    pub async fn wait_terminal(&self, chat_id: &str, job_id: &str) -> Option<JobSnapshot> {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let snapshot = self.get(chat_id, job_id)?;
            if snapshot.status.terminal() {
                return Some(snapshot);
            }
            notified.await;
        }
    }

    /// Broadcast is only a wake hint. Retained terminal records are the source
    /// of truth, including completion-before-subscribe and lagged receivers.
    pub(crate) fn background_terminal_refs(&self) -> Vec<TerminalRef> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .records
            .values()
            .filter(|record| {
                record.owner == Owner::Background && record.status.terminal() && !record.consumed
            })
            .map(Self::terminal_ref)
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn background_terminals(&self) -> Vec<JobSnapshot> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .records
            .values()
            .filter(|record| {
                record.owner == Owner::Background && record.status.terminal() && !record.consumed
            })
            .map(|record| JobSnapshot {
                receipt: record.receipt.clone(),
                status: record.status,
                revision: record.revision,
                result: record.result.clone(),
                result_unavailable_reason: record.unavailable.clone(),
                result_released: record.released,
            })
            .collect()
    }

    pub fn cancel(&self, chat_id: &str, job_id: &str) -> Option<JobStatus> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let record = state.records.get_mut(job_id)?;
        if record.receipt.binding.chat_id != chat_id {
            return None;
        }
        if !record.status.terminal() && record.status != JobStatus::CancelRequested {
            record.cancel.store(true, Ordering::Release);
            record.status = JobStatus::CancelRequested;
        }
        Some(record.status)
    }

    pub async fn shutdown(&self) {
        let bridge = {
            let mut bridge = self.bridge.lock().unwrap_or_else(|e| e.into_inner());
            self.bridge_shutdown.store(true, Ordering::Release);
            bridge.take()
        };
        if let Some((stop, mut task)) = bridge {
            let _ = stop.send(true);
            loop {
                tokio::select! {
                    _ = &mut task => break,
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {
                        self.scopes.log_blockers(None, "shutdown completion bridge");
                    }
                }
            }
        }
        let transition = self.transition.lock().await;
        self.scopes.begin_shutdown();
        let tasks = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.closed = true;
            state
                .records
                .values_mut()
                .filter_map(|record| {
                    record.cancel.store(true, Ordering::Release);
                    record.task.take()
                })
                .collect::<Vec<_>>()
        };
        drop(transition);
        // Cooperative cancellation only: joining retains spawn_blocking/owned
        // effect lifetimes. Abort is not proof that their side effects ceased.
        for mut task in tasks {
            loop {
                tokio::select! {
                    _ = &mut task => break,
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {
                        self.scopes.log_blockers(None, "shutdown tool jobs");
                    }
                }
            }
        }
        let _ = self.scopes.drain_shutdown().await;
    }
}

impl Drop for JobRuntime {
    fn drop(&mut self) {
        if let Some((stop, task)) = self
            .bridge
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let _ = stop.send(true);
            task.abort();
        }
        let state = self.state.get_mut().unwrap_or_else(|e| e.into_inner());
        for record in state.records.values_mut() {
            record.cancel.store(true, Ordering::Release);
            if let Some(task) = record.task.take() {
                task.abort();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn binding() -> JobBinding {
        JobBinding {
            scope_keys: vec![super::super::ScopeKey::Chat {
                project: "/project".into(),
                chat_id: "chat".into(),
            }],
            chat_id: "chat".into(),
            tool_call_id: "call".into(),
            tool_name: "tool".into(),
            model: "provider/model".into(),
            turn_id: "turn".into(),
        }
    }
    fn result() -> JobResult {
        JobResult {
            had_corrections: false,
            messages: vec![ChatMessage::new("tool".into(), "plain, not JSON".into())],
            context_files: vec![],
        }
    }

    #[tokio::test(start_paused = true)]
    async fn aborted_turn_waiter_never_constructs_execution() {
        let runtime = JobRuntime::new(4, 65536);
        let binding = binding();
        let _guard = runtime
            .scopes
            .begin_scope_change(&binding.scope_keys)
            .await
            .unwrap();
        let abort = Arc::new(AtomicBool::new(false));
        let mut pending = Box::pin(runtime.submit_with_abort(
            binding,
            Duration::ZERO,
            abort.clone(),
            Arc::new(AtomicBool::new(false)),
            |_| async {
                panic!("aborted admission must not execute");
            },
        ));
        assert!(futures::poll!(&mut pending).is_pending());
        abort.store(true, Ordering::Release);
        assert!(pending
            .await
            .err()
            .unwrap()
            .contains("Aborted while waiting for tool admission"));
        assert!(runtime.state.lock().unwrap().records.is_empty());
        assert!(runtime.state.lock().unwrap().invocation_ids.is_empty());
    }

    #[tokio::test]
    async fn abort_after_admission_returns_owned_inline_result_only() {
        let runtime = JobRuntime::new(4, 65536);
        runtime.set_test_inline_budget(Some(Duration::from_secs(3600)));
        let abort = Arc::new(AtomicBool::new(false));
        let (release, released) = oneshot::channel();
        let mut pending = Box::pin(runtime.submit_with_abort(
            binding(),
            Duration::ZERO,
            abort.clone(),
            Arc::new(AtomicBool::new(false)),
            |_| async move {
                released.await.unwrap();
                (JobStatus::Succeeded, result())
            },
        ));
        assert!(futures::poll!(&mut pending).is_pending());
        assert_eq!(runtime.state.lock().unwrap().records.len(), 1);
        abort.store(true, Ordering::Release);
        release.send(()).unwrap();
        assert!(matches!(pending.await.unwrap(), Submission::Inline(_)));
        assert!(runtime
            .state
            .lock()
            .unwrap()
            .records
            .values()
            .all(|record| record.consumed));
        assert!(runtime.background_terminal_refs().is_empty());
    }

    #[tokio::test]
    async fn fence_cancellation_unwinds_nested_admission_without_deadlock() {
        let runtime = JobRuntime::new(4, 65536);
        let (enter, entered) = oneshot::channel();
        let nested_runtime = runtime.clone();
        let outer = runtime
            .submit_background(binding(), move |outer_cancel| async move {
                entered.await.unwrap();
                let mut nested_binding = binding();
                nested_binding.tool_call_id = "nested".into();
                let nested = nested_runtime
                    .submit_with_abort(
                        nested_binding,
                        Duration::ZERO,
                        outer_cancel,
                        Arc::new(AtomicBool::new(false)),
                        |_| async {
                            panic!("cancelled nested admission must not execute");
                        },
                    )
                    .await;
                assert!(nested
                    .err()
                    .unwrap()
                    .contains("Aborted while waiting for tool admission"));
                (JobStatus::Cancelled, result())
            })
            .await
            .unwrap();
        let keys = binding().scope_keys;
        let mut fence = Box::pin(runtime.scopes.begin_scope_change(&keys));
        assert!(futures::poll!(&mut fence).is_pending());
        enter.send(()).unwrap();
        let guard = fence.await.unwrap();
        runtime.wait_terminal("chat", &outer.job_id).await.unwrap();
        assert_eq!(runtime.state.lock().unwrap().records.len(), 1);
        drop(guard);
    }

    #[tokio::test]
    async fn fenced_duplicate_submissions_execute_once_and_drop_leaves_no_record() {
        let runtime = JobRuntime::new(4, 65536);
        let binding = binding();
        let guard = runtime
            .scopes
            .begin_scope_change(&binding.scope_keys)
            .await
            .unwrap();
        let runs = Arc::new(AtomicUsize::new(0));
        let first_runs = runs.clone();
        let mut first = Box::pin(
            runtime.submit_background(binding.clone(), move |_| async move {
                first_runs.fetch_add(1, Ordering::SeqCst);
                (JobStatus::Succeeded, result())
            }),
        );
        let second_runs = runs.clone();
        let mut second = Box::pin(runtime.submit_background(
            binding.clone(),
            move |_| async move {
                second_runs.fetch_add(1, Ordering::SeqCst);
                (JobStatus::Succeeded, result())
            },
        ));
        let mut dropped = Box::pin(runtime.submit_background(binding.clone(), |_| async {
            panic!("dropped waiter must never execute");
        }));
        assert!(futures::poll!(&mut first).is_pending());
        assert!(futures::poll!(&mut second).is_pending());
        assert!(futures::poll!(&mut dropped).is_pending());
        drop(dropped);
        assert!(runtime.state.lock().unwrap().records.is_empty());
        assert!(runtime.state.lock().unwrap().invocation_ids.is_empty());
        drop(guard);
        let receipt = first.await.unwrap();
        assert!(second.await.unwrap_err().contains("already owned"));
        runtime
            .wait_terminal("chat", &receipt.job_id)
            .await
            .unwrap();
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(runtime.state.lock().unwrap().records.len(), 1);
    }

    #[tokio::test]
    async fn readiness_owner_is_private_and_external_same_name_is_not() {
        let runtime = JobRuntime::new(4, 65536);
        let mut private = binding();
        private.tool_name = "review_readiness".into();
        let receipt = runtime
            .submit_readiness(private.clone(), |_| async {
                (JobStatus::Succeeded, result())
            })
            .await
            .unwrap();
        let terminal = runtime
            .wait_terminal("chat", &receipt.job_id)
            .await
            .unwrap();
        assert!(runtime.background_terminals().is_empty());
        private.tool_call_id = "external-call".into();
        let external = runtime
            .submit_background(private, |_| async { (JobStatus::Succeeded, result()) })
            .await
            .unwrap();
        runtime
            .wait_terminal("chat", &external.job_id)
            .await
            .unwrap();
        assert_eq!(runtime.background_terminals().len(), 1);
        assert_eq!(
            runtime.background_terminals()[0].receipt.job_id,
            external.job_id
        );
        assert!(runtime
            .ack_readiness("chat", &external.job_id, terminal.revision)
            .await
            .is_err());
        runtime
            .ack_readiness("chat", &receipt.job_id, terminal.revision)
            .await
            .unwrap();
        assert!(runtime.state.lock().unwrap().records[&receipt.job_id].consumed);
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn completion_bridge_installs_once_and_shutdown_joins() {
        let runtime = JobRuntime::new(2, 65536);
        let stopped = Arc::new(AtomicBool::new(false));
        let done = stopped.clone();
        runtime.install_completion_bridge(move |mut stop| {
            tokio::spawn(async move {
                while !*stop.borrow_and_update() {
                    if stop.changed().await.is_err() {
                        break;
                    }
                }
                done.store(true, Ordering::Release);
            })
        });
        runtime.install_completion_bridge(|_| panic!("bridge installed twice"));
        runtime.shutdown().await;
        assert!(stopped.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn completion_bridge_cannot_reinstall_during_or_after_shutdown() {
        let runtime = JobRuntime::new(2, 65536);
        let (stopping, stopped) = tokio::sync::oneshot::channel();
        let (release, released) = tokio::sync::oneshot::channel();
        runtime.install_completion_bridge(move |mut stop| {
            tokio::spawn(async move {
                stop.changed().await.unwrap();
                stopping.send(()).unwrap();
                released.await.unwrap();
            })
        });
        let shutting_down = runtime.clone();
        let shutdown = tokio::spawn(async move { shutting_down.shutdown().await });
        tokio::time::timeout(Duration::from_secs(10), stopped)
            .await
            .unwrap()
            .unwrap();
        runtime.install_completion_bridge(|_| panic!("bridge restarted during shutdown"));
        assert!(!shutdown.is_finished());
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(10), shutdown)
            .await
            .unwrap()
            .unwrap();
        runtime.install_completion_bridge(|_| panic!("bridge restarted after shutdown"));
        assert!(runtime.bridge.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn fast_results_reclaim_active_capacity_without_losing_dedup() {
        let runtime = JobRuntime::new(2, 65536);
        for index in 0..300 {
            let mut call = binding();
            call.tool_call_id = format!("fast-{index}");
            assert!(matches!(
                runtime
                    .submit(call, Duration::from_secs(60), |_| async {
                        (JobStatus::Succeeded, result())
                    })
                    .await
                    .unwrap(),
                Submission::Inline(_)
            ));
        }
        assert_eq!(
            runtime
                .state
                .lock()
                .unwrap()
                .records
                .values()
                .filter(|r| !r.consumed)
                .count(),
            0
        );
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn factory_construction_panic_records_unknown_without_poisoning_scope() {
        let runtime = JobRuntime::new(2, 65536);
        let effects = Arc::new(AtomicUsize::new(0));
        let counter = effects.clone();
        let receipt = runtime
            .submit_background(binding(), move |_| {
                counter.fetch_add(1, Ordering::SeqCst);
                // This is construction, not a panic while polling an async block.
                panic!("factory performed an effect before constructing its future");
                #[allow(unreachable_code)]
                std::future::ready((JobStatus::Succeeded, result()))
            })
            .await
            .unwrap();
        let terminal = runtime
            .wait_terminal("chat", &receipt.job_id)
            .await
            .unwrap();
        assert_eq!(terminal.status, JobStatus::Unknown);
        assert_eq!(runtime.background_terminals().len(), 1);
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert!(runtime.state.lock().unwrap().records[&receipt.job_id]
            .task
            .is_some());
        drop(
            runtime
                .begin_scope_change(&receipt.binding.scope_keys)
                .await
                .unwrap(),
        );
        assert!(runtime
            .submit_background(receipt.binding.clone(), |_| async {
                panic!("must not replay")
            })
            .await
            .is_err());
        runtime
            .scopes
            .acquire(
                &receipt.binding.scope_keys,
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap()
            .settle();
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn inline_is_once_and_never_emits_background_completion() {
        let runtime = JobRuntime::new(2, 65536);
        let mut events = runtime.subscribe();
        let output = runtime
            .submit(binding(), Duration::from_secs(60), |_| async {
                (JobStatus::Succeeded, result())
            })
            .await
            .unwrap();
        let Submission::Inline(output) = output else {
            panic!("expected inline");
        };
        assert_eq!(
            output.messages[0].content.content_text_only(),
            "plain, not JSON"
        );
        assert!(matches!(
            events.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        assert!(runtime
            .submit(binding(), Duration::ZERO, |_| async {
                panic!("duplicate must never run");
            })
            .await
            .is_err());
        let mut same_invocation = binding();
        same_invocation.scope_keys = vec![super::super::ScopeKey::Worktree {
            project: "/other-project".into(),
            root: "/other-worktree".into(),
        }];
        assert!(runtime
            .submit(same_invocation, Duration::ZERO, |_| async {
                panic!("scope provenance must not change invocation identity");
            })
            .await
            .is_err());
        assert_eq!(runtime.state.lock().unwrap().invocation_ids.len(), 1);
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn timeout_preserves_one_future_and_cancel_is_not_success_or_quiescence() {
        let runtime = JobRuntime::new(2, 65536);
        let mut events = runtime.subscribe();
        let calls = Arc::new(AtomicUsize::new(0));
        let (release, released) = oneshot::channel();
        let counter = calls.clone();
        let output = runtime
            .submit(binding(), Duration::ZERO, move |cancel| async move {
                counter.fetch_add(1, Ordering::SeqCst);
                released.await.unwrap();
                assert!(cancel.load(Ordering::Acquire));
                (JobStatus::Unknown, result())
            })
            .await
            .unwrap();
        let Submission::Accepted(receipt) = output else {
            panic!("expected receipt");
        };
        assert!(runtime.get("other-chat", &receipt.job_id).is_none());
        assert_eq!(
            runtime.cancel("chat", &receipt.job_id),
            Some(JobStatus::CancelRequested)
        );
        release.send(()).unwrap();
        let event = events.recv().await.unwrap();
        assert_eq!(event.status, JobStatus::Unknown);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            runtime.cancel("chat", &receipt.job_id),
            Some(JobStatus::Unknown)
        );
        assert!(matches!(
            events.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn active_jobs_have_no_ceiling_and_shutdown_and_restart_never_replay() {
        let runtime = JobRuntime::new(1, 65536);
        let effects = Arc::new(AtomicUsize::new(0));
        let mut receipts = Vec::new();
        for call_id in ["call", "other"] {
            let mut call = binding();
            call.tool_call_id = call_id.into();
            let (started, ready) = oneshot::channel();
            let counter = effects.clone();
            let output = runtime
                .submit(call, Duration::ZERO, move |cancel| {
                    counter.fetch_add(1, Ordering::SeqCst);
                    async move {
                        started.send(()).unwrap();
                        while !cancel.load(Ordering::Acquire) {
                            tokio::task::yield_now().await;
                        }
                        (JobStatus::Unknown, result())
                    }
                })
                .await
                .unwrap();
            ready.await.unwrap();
            let Submission::Accepted(receipt) = output else {
                panic!("expected receipt");
            };
            receipts.push(receipt);
        }
        assert_eq!(effects.load(Ordering::SeqCst), 2);
        for receipt in &receipts {
            assert!(runtime
                .submit(receipt.binding.clone(), Duration::ZERO, |_| async {
                    panic!("pending duplicate must never run");
                })
                .await
                .is_err());
        }
        runtime.shutdown().await;
        for receipt in &receipts {
            assert_eq!(
                runtime.get("chat", &receipt.job_id).unwrap().status,
                JobStatus::Unknown
            );
        }
        assert!(runtime
            .submit(binding(), Duration::ZERO, |_| async {
                panic!("closed runtime must not run");
            })
            .await
            .is_err());
        let mut fresh = binding();
        fresh.tool_call_id = "fresh-after-shutdown".into();
        assert!(runtime
            .submit(fresh, Duration::ZERO, |_| async {
                panic!("closed runtime must not run");
            })
            .await
            .is_err());
        assert_eq!(effects.load(Ordering::SeqCst), 2);
        let restarted = JobRuntime::new(1, 65536);
        for receipt in &receipts {
            assert!(restarted.get("chat", &receipt.job_id).is_none());
        }
        restarted.shutdown().await;
    }

    #[tokio::test]
    async fn admission_accepts_over_256_pending_jobs_and_lag_recovers_from_records() {
        const JOBS: usize = 300;
        let runtime = JobRuntime::new(4, 65536);
        let mut events = runtime.subscribe();
        let effects = Arc::new(AtomicUsize::new(0));
        let mut releases = Vec::with_capacity(JOBS);
        let mut receipts = Vec::with_capacity(JOBS);
        for index in 0..JOBS {
            let mut call = binding();
            call.tool_call_id = format!("pending-{index}");
            let (started, ready) = oneshot::channel();
            let (release, released) = oneshot::channel::<()>();
            let counter = effects.clone();
            let receipt = runtime
                .submit_background(call, move |_| {
                    counter.fetch_add(1, Ordering::SeqCst);
                    async move {
                        started.send(()).unwrap();
                        released.await.unwrap();
                        (
                            JobStatus::Succeeded,
                            JobResult {
                                had_corrections: false,
                                messages: vec![ChatMessage::new(
                                    "tool".into(),
                                    format!("result-{index}"),
                                )],
                                context_files: vec![],
                            },
                        )
                    }
                })
                .await
                .unwrap();
            ready.await.unwrap();
            releases.push(release);
            receipts.push(receipt);
        }
        assert_eq!(effects.load(Ordering::SeqCst), JOBS);
        let ids = receipts
            .iter()
            .map(|receipt| receipt.job_id.clone())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(ids.len(), JOBS);
        {
            let state = runtime.state.lock().unwrap();
            assert_eq!(state.records.len(), JOBS);
            assert!(state
                .records
                .values()
                .all(|record| !record.consumed && !record.status.terminal()));
        }
        assert!(runtime.background_terminals().is_empty());
        for receipt in [&receipts[0], &receipts[JOBS - 1]] {
            assert!(runtime
                .submit_background(receipt.binding.clone(), |_| async {
                    panic!("pending duplicate must never run")
                })
                .await
                .is_err());
        }
        let mut extra = binding();
        extra.tool_call_id = format!("pending-{JOBS}");
        let extra = runtime
            .submit_background(extra, |_| async { (JobStatus::Succeeded, result()) })
            .await
            .unwrap();
        let extra = runtime.wait_terminal("chat", &extra.job_id).await.unwrap();
        runtime
            .ack_delivered("chat", &extra.receipt.job_id, extra.revision)
            .await
            .unwrap();
        for release in releases {
            release.send(()).unwrap();
        }
        for receipt in &receipts {
            let terminal = runtime
                .wait_terminal("chat", &receipt.job_id)
                .await
                .unwrap();
            assert_eq!(terminal.status, JobStatus::Succeeded);
        }
        assert!(matches!(
            events.try_recv(),
            Err(broadcast::error::TryRecvError::Lagged(_))
        ));
        let notices = runtime.background_terminals();
        assert_eq!(notices.len(), JOBS);
        assert_eq!(
            notices
                .iter()
                .map(|notice| notice.receipt.job_id.clone())
                .collect::<std::collections::HashSet<_>>(),
            ids
        );
        for notice in &notices {
            let index = notice
                .receipt
                .binding
                .tool_call_id
                .strip_prefix("pending-")
                .unwrap();
            assert_eq!(
                notice.receipt.job_id,
                receipts[index.parse::<usize>().unwrap()].job_id
            );
            assert_eq!(
                notice.result.as_ref().unwrap().messages[0]
                    .content
                    .content_text_only(),
                format!("result-{index}")
            );
            runtime
                .ack_delivered("chat", &notice.receipt.job_id, notice.revision)
                .await
                .unwrap();
        }
        assert!(runtime.background_terminals().is_empty());
        assert_eq!(effects.load(Ordering::SeqCst), JOBS);
        assert_eq!(runtime.state.lock().unwrap().records.len(), JOBS + 1);
        runtime.shutdown().await;
    }

    fn sized(call: &str, bytes: usize) -> (JobBinding, JobResult) {
        let mut call_binding = binding();
        call_binding.tool_call_id = call.into();
        let result = JobResult {
            had_corrections: false,
            messages: vec![ChatMessage::new("tool".into(), "x".repeat(bytes))],
            context_files: vec![],
        };
        (call_binding, result)
    }

    async fn finish_background(runtime: &Arc<JobRuntime>, call: &str, bytes: usize) -> JobSnapshot {
        let (call, result) = sized(call, bytes);
        let receipt = runtime
            .submit_background(call, move |_| async move { (JobStatus::Succeeded, result) })
            .await
            .unwrap();
        runtime
            .wait_terminal("chat", &receipt.job_id)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn retention_releases_consumed_payloads_before_undelivered_ones() {
        let runtime = JobRuntime::with_limits(
            "/project".into(),
            64,
            3000,
            TERMINAL_RECORD_CAP,
            UNDELIVERED_RECORD_CAP,
        );
        let consumed = finish_background(&runtime, "consumed", 1000).await;
        runtime
            .ack_delivered("chat", &consumed.receipt.job_id, consumed.revision)
            .await
            .unwrap();
        let older = finish_background(&runtime, "older-undelivered", 1000).await;
        let newer = finish_background(&runtime, "newer-undelivered", 1000).await;
        assert!(
            runtime
                .get("chat", &consumed.receipt.job_id)
                .unwrap()
                .result_released
        );
        assert!(runtime
            .get("chat", &older.receipt.job_id)
            .unwrap()
            .result
            .is_some());
        assert!(runtime
            .get("chat", &newer.receipt.job_id)
            .unwrap()
            .result
            .is_some());
        let latest = finish_background(&runtime, "latest-undelivered", 1000).await;
        let older = runtime.get("chat", &older.receipt.job_id).unwrap();
        assert!(older.result.is_none() && older.result_released);
        assert_eq!(
            older.result_unavailable_reason.as_deref(),
            Some(RESULT_NOT_KEPT)
        );
        assert!(runtime
            .get("chat", &newer.receipt.job_id)
            .unwrap()
            .result
            .is_some());
        assert!(runtime
            .get("chat", &latest.receipt.job_id)
            .unwrap()
            .result
            .is_some());
        assert!(runtime.state.lock().unwrap().retained_bytes <= 3000);
        let refs = runtime
            .background_terminal_refs()
            .into_iter()
            .map(|job| job.receipt.job_id)
            .collect::<std::collections::HashSet<_>>();
        assert!(refs.contains(&older.receipt.job_id));
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn retention_never_releases_an_inline_result_before_it_is_returned() {
        let runtime = JobRuntime::with_limits(
            "/project".into(),
            8,
            10,
            TERMINAL_RECORD_CAP,
            UNDELIVERED_RECORD_CAP,
        );
        let (call, result) = sized("big-inline", 1000);
        let Submission::Inline(output) = runtime
            .submit(call, Duration::from_secs(60), move |_| async move {
                (JobStatus::Succeeded, result)
            })
            .await
            .unwrap()
        else {
            panic!("expected inline result");
        };
        assert_eq!(output.messages[0].content.content_text_only().len(), 1000);
        assert_eq!(runtime.state.lock().unwrap().retained_bytes, 0);
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn record_cap_evicts_only_consumed_records_and_undelivered_have_their_own_cap() {
        let runtime = JobRuntime::with_limits("/project".into(), 64, usize::MAX, 3, 4);
        let first = finish_background(&runtime, "first", 1).await;
        let second = finish_background(&runtime, "second", 1).await;
        runtime
            .ack_delivered("chat", &second.receipt.job_id, second.revision)
            .await
            .unwrap();
        let third = finish_background(&runtime, "third", 1).await;
        runtime
            .ack_delivered("chat", &third.receipt.job_id, third.revision)
            .await
            .unwrap();
        let fourth = finish_background(&runtime, "fourth", 1).await;
        assert!(runtime.get("chat", &second.receipt.job_id).is_none());
        assert!(runtime.get("chat", &third.receipt.job_id).is_some());
        assert!(runtime.get("chat", &first.receipt.job_id).is_some());
        let fifth = finish_background(&runtime, "fifth", 1).await;
        assert!(runtime.get("chat", &third.receipt.job_id).is_none());
        let sixth = finish_background(&runtime, "sixth", 1).await;
        assert!(
            runtime.knows("chat", &first.receipt.job_id),
            "the ordinary cap must never evict an undelivered record"
        );
        assert_eq!(runtime.state.lock().unwrap().records.len(), 4);
        let seventh = finish_background(&runtime, "seventh", 1).await;
        assert!(!runtime.knows("chat", &first.receipt.job_id));
        for kept in [&fourth, &fifth, &sixth, &seventh] {
            assert!(runtime.get("chat", &kept.receipt.job_id).is_some());
        }
        assert_eq!(runtime.state.lock().unwrap().records.len(), 4);
        assert_eq!(runtime.state.lock().unwrap().invocation_ids.len(), 7);
        assert!(runtime
            .submit_background(first.receipt.binding.clone(), |_| async {
                panic!("an evicted record must not make its invocation replayable")
            })
            .await
            .is_err());
        runtime.shutdown().await;
    }

    #[test]
    fn retention_size_is_the_serialized_size_without_a_copy() {
        let (_, result) = sized("measured", 1234);
        assert_eq!(
            serialized_size(&result),
            serde_json::to_vec(&result).unwrap().len()
        );
        assert!(serialized_size(&result) > 1234);
    }

    #[tokio::test]
    async fn knows_checks_existence_and_chat_without_a_payload() {
        let runtime = JobRuntime::with_limits(
            "/project".into(),
            8,
            usize::MAX,
            TERMINAL_RECORD_CAP,
            UNDELIVERED_RECORD_CAP,
        );
        let done = finish_background(&runtime, "known", 10).await;
        assert!(runtime.knows("chat", &done.receipt.job_id));
        assert!(!runtime.knows("other-chat", &done.receipt.job_id));
        assert!(!runtime.knows("chat", "missing"));
        runtime.shutdown().await;
    }
}
