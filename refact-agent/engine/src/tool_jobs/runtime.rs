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
    archived: bool,
}

struct State {
    records: HashMap<String, Record>,
    closed: bool,
}

/// Capacity counts only active/unconsumed ownership. Durable identities are retained
/// separately under a hard admission bound; no active/unacknowledged record is evicted.
pub struct JobRuntime {
    #[cfg(test)]
    test_inline_budget: std::sync::Mutex<Option<Option<Duration>>>,
    pub scopes: Arc<super::ScopeCoordinator>,
    namespace: std::path::PathBuf,
    state: Mutex<State>,
    capacity: usize,
    max_result_bytes: usize,
    events: broadcast::Sender<JobEvent>,
    changed: tokio::sync::Notify,
    journal: Option<Arc<super::journal::Journal>>,
    transition: tokio::sync::Mutex<()>,
    bridge: Mutex<Option<(tokio::sync::watch::Sender<bool>, JoinHandle<()>)>>,
    bridge_shutdown: AtomicBool,
}

struct ForegroundClaim {
    runtime: std::sync::Weak<JobRuntime>,
    job_id: String,
}

/// Owns the admission I/O even if the submitting future is dropped. No effect
/// factory has been called while this guard is armed.
struct AdmissionClaim {
    runtime: Arc<JobRuntime>,
    job_id: String,
    chat_id: String,
    lease: Option<super::ExecutionLease>,
    persist: Option<JoinHandle<Result<(), String>>>,
}

impl Drop for AdmissionClaim {
    fn drop(&mut self) {
        let Some(lease) = self.lease.take() else {
            return;
        };
        let runtime = self.runtime.clone();
        let id = self.job_id.clone();
        let chat_id = self.chat_id.clone();
        let persist = self.persist.take();
        tokio::spawn(async move {
            // Joining is essential: a detached SQLite write must not land after
            // our terminal write and resurrect a Running record.
            if let Some(persist) = persist {
                let _ = persist.await;
            }
            let _transition = runtime.transition.lock().await;
            let mut snapshot = runtime.get(&chat_id, &id).expect("admitted job");
            snapshot.status = JobStatus::Cancelled;
            snapshot.revision += 1;
            snapshot.result_unavailable_reason = Some(
                "submission dropped before effect construction; invocation never started".into(),
            );
            let saved = runtime.persist(snapshot.clone(), true, false).await;
            lease.settle(); // Proven safe: factory was never called.
            let mut state = runtime.state.lock().unwrap_or_else(|e| e.into_inner());
            if saved.is_err() {
                state.closed = true;
            }
            let record = state.records.get_mut(&id).expect("admitted job");
            record.status = snapshot.status;
            record.revision = snapshot.revision;
            record.unavailable = snapshot.result_unavailable_reason;
            if record.owner != Owner::InternalReadiness {
                record.owner = Owner::Background;
            }
            record.consumed = saved.is_ok();
            let _ = runtime.events.send(JobRuntime::event(record));
            runtime.changed.notify_waiters();
        });
    }
}

impl Drop for ForegroundClaim {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.upgrade() {
            let mut state = runtime.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(record) = state.records.get_mut(&self.job_id) {
                if record.owner == Owner::Undecided {
                    record.owner = Owner::Background;
                    if record.status.terminal() {
                        let _ = runtime.events.send(JobRuntime::event(record));
                    }
                }
            }
        }
    }
}

impl JobRuntime {
    pub fn new(capacity: usize, max_result_bytes: usize) -> Arc<Self> {
        Self::with_namespace(
            std::path::PathBuf::from("/isolated-tool-job-runtime"),
            capacity,
            max_result_bytes,
        )
    }

    pub fn with_namespace(
        namespace: std::path::PathBuf,
        capacity: usize,
        max_result_bytes: usize,
    ) -> Arc<Self> {
        let (events, _) = broadcast::channel(capacity.max(1));
        Arc::new(Self {
            #[cfg(test)]
            test_inline_budget: std::sync::Mutex::new(None),
            scopes: Arc::new(super::ScopeCoordinator::default()),
            namespace,
            state: Mutex::new(State {
                records: HashMap::new(),
                closed: false,
            }),
            capacity,
            max_result_bytes,
            events,
            changed: tokio::sync::Notify::new(),
            journal: None,
            transition: tokio::sync::Mutex::new(()),
            bridge: Mutex::new(None),
            bridge_shutdown: AtomicBool::new(false),
        })
    }

    pub async fn open_durable(
        namespace: std::path::PathBuf,
        capacity: usize,
        max_result_bytes: usize,
    ) -> Result<Arc<Self>, String> {
        let (journal, entries) =
            super::journal::Journal::open(namespace.join("tool-jobs/ownership.sqlite")).await?;
        // Legacy terminal payloads are no safer than active entries: without
        // original domains, attachment checks cannot establish read authority.
        if entries.iter().any(|entry| entry.scope_keys.is_empty()) {
            return Err("tool job journal lacks original scope provenance".into());
        }
        let mut runtime = Self::with_namespace(namespace, capacity, max_result_bytes);
        let inner = Arc::get_mut(&mut runtime).expect("new runtime uniquely owned");
        inner.journal = Some(journal);
        let state = inner.state.get_mut().unwrap_or_else(|e| e.into_inner());
        for entry in entries {
            let mut snapshot = entry.snapshot;
            snapshot.receipt.binding.scope_keys = entry.scope_keys;
            if snapshot.status == JobStatus::Unknown {
                inner
                    .scopes
                    .mark_unknown(&snapshot.receipt.binding.scope_keys);
            }
            state.records.insert(
                snapshot.receipt.job_id.clone(),
                Record {
                    receipt: snapshot.receipt,
                    status: snapshot.status,
                    revision: snapshot.revision,
                    archived: snapshot.result.is_some(),
                    result: None,
                    owner: if entry.internal_readiness {
                        Owner::InternalReadiness
                    } else if entry.inline {
                        Owner::Inline
                    } else {
                        Owner::Background
                    },
                    cancel: Arc::new(AtomicBool::new(false)),
                    task: None,
                    consumed: entry.consumed,
                    unavailable: snapshot.result_unavailable_reason,
                },
            );
        }
        Ok(runtime)
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

    async fn persist(
        &self,
        snapshot: JobSnapshot,
        consumed: bool,
        inline: bool,
    ) -> Result<(), String> {
        if let Some(journal) = &self.journal {
            let internal_readiness = self
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .records
                .get(&snapshot.receipt.job_id)
                .is_some_and(|record| record.owner == Owner::InternalReadiness);
            journal
                .save(super::journal::Entry {
                    internal_readiness,
                    scope_keys: snapshot.receipt.binding.scope_keys.clone(),
                    snapshot,
                    consumed,
                    inline,
                })
                .await?;
        }
        Ok(())
    }

    pub async fn get_result(
        &self,
        chat_id: &str,
        job_id: &str,
    ) -> Result<Option<JobSnapshot>, String> {
        let Some(mut snapshot) = self.get(chat_id, job_id) else {
            return Ok(None);
        };
        if snapshot.result.is_none() {
            if let Some(journal) = &self.journal {
                let stored = journal.load(job_id.to_owned()).await?;
                snapshot.result = stored.snapshot.result;
            }
        }
        Ok(Some(snapshot))
    }

    pub async fn ack_delivered(
        &self,
        chat_id: &str,
        job_id: &str,
        revision: u64,
    ) -> Result<(), String> {
        let _transition = self.transition.lock().await;
        let snapshot = self
            .get_result(chat_id, job_id)
            .await?
            .ok_or("tool job not found")?;
        if !snapshot.status.terminal() || snapshot.revision != revision {
            return Err("terminal revision mismatch".into());
        }
        self.persist(snapshot, true, false).await?;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let record = state.records.get_mut(job_id).ok_or("tool job not found")?;
        record.consumed = true;
        if self.journal.is_some() {
            record.archived |= record.result.is_some();
            record.result = None;
        }
        Ok(())
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
            result_available: record.result.is_some() || record.archived,
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
        self.submit_owned(binding, budget, false, make).await
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
        match self.submit_owned(binding, None, false, make).await? {
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
        match self.submit_owned(binding, None, true, make).await? {
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

    async fn submit_owned<F, Make>(
        self: &Arc<Self>,
        binding: JobBinding,
        budget: Option<Duration>,
        internal_readiness: bool,
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
        let transition = self.transition.lock().await;
        let lease;
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.closed {
                return Err("tool job runtime is shut down".into());
            }
            if state.records.values().filter(|r| !r.consumed).count() >= self.capacity {
                return Err("tool job capacity exhausted".into());
            }
            if state.records.len() >= super::journal::RETAINED_IDENTITIES {
                return Err(
                    "tool job retained identity limit reached; explicit archival required".into(),
                );
            }
            if state.records.values().any(|record| {
                serde_json::to_value(&record.receipt.binding).ok()
                    == serde_json::to_value(&receipt.binding).ok()
            }) {
                return Err(
                    "tool invocation already owned; inspect existing result, do not replay".into(),
                );
            }
            lease = self
                .scopes
                .acquire(&receipt.binding.scope_keys, cancel.clone())
                .map_err(|error| format!("tool job scope unavailable: {error:?}"))?;
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
                    archived: false,
                },
            );
        }
        // Commit durable ownership before constructing or polling any effect.
        let initial = self
            .get(&receipt.binding.chat_id, &job_id)
            .expect("inserted job");
        let runtime = self.clone();
        let mut admission = AdmissionClaim {
            runtime: self.clone(),
            job_id: job_id.clone(),
            chat_id: receipt.binding.chat_id.clone(),
            lease: Some(lease),
            persist: Some(tokio::spawn(async move {
                runtime.persist(initial, false, false).await
            })),
        };
        let persistence = admission
            .persist
            .as_mut()
            .expect("admission I/O")
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result);
        admission.persist.take();
        if let Err(error) = persistence {
            self.state.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
            return Err(format!(
                "tool ownership journal unavailable; invocation not started: {error}"
            ));
        }
        let lease = admission.lease.take().expect("admission lease");
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let runtime = Arc::downgrade(self);
            let id = job_id.clone();
            let chat_id = receipt.binding.chat_id.clone();
            // Construction itself may perform effects and panic. Keep that
            // failure inside the same owned terminal path as a polling panic.
            let future = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| make(cancel)));
            let task = tokio::spawn(async move {
                use futures::FutureExt;
                let completed = match future {
                    Ok(future) => std::panic::AssertUnwindSafe(future).catch_unwind().await,
                    Err(panic) => Err(panic),
                };
                // A panic or explicitly unknown outcome cannot prove quiescence.
                if completed
                    .as_ref()
                    .is_ok_and(|(status, _)| status.terminal() && *status != JobStatus::Unknown)
                {
                    lease.settle();
                } else {
                    drop(lease);
                }
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
                let _transition = runtime.transition.lock().await;
                let mut snapshot = runtime.get(&chat_id, &id).expect("owned job");
                snapshot.status = status;
                snapshot.result = Some(result.clone());
                snapshot.revision += 1;
                let persistence = runtime.persist(snapshot, false, false).await;
                let oversized = serde_json::to_vec(&result)
                    .map_or(true, |bytes| bytes.len() > runtime.max_result_bytes);
                let mut state = runtime.state.lock().unwrap_or_else(|e| e.into_inner());
                if persistence.is_err() {
                    state.closed = true;
                }
                if let Some(record) = state.records.get_mut(&id) {
                    if !record.status.terminal() {
                        record.status = status;
                        record.result = Some(result);
                        if let Err(error) = persistence {
                            record.unavailable = Some(format!("terminal journal commit failed; exact result retained in process only: {error}"));
                        } else if oversized
                            && runtime.journal.is_some()
                            && record.owner == Owner::Background
                        {
                            record.archived = true;
                            record.result = None;
                        }
                        record.revision += 1;
                        if record.owner == Owner::Background {
                            let _ = runtime.events.send(Self::event(record));
                        }
                    }
                }
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
        let snapshot = self
            .get_result(&receipt.binding.chat_id, &job_id)
            .await?
            .ok_or("job ownership lost")?;
        if completed_inline && snapshot.result.is_some() {
            // A cancelled SQLite await may still commit. Persist a pending
            // inline handoff, never a consumed result before the return. On
            // restart this becomes background delivery (stable-ID duplicates
            // are safe); only explicit delivery acknowledgement consumes it.
            self.persist(snapshot.clone(), false, true).await?;
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let record = state.records.get_mut(&job_id).ok_or("job ownership lost")?;
        // Completion may race timeout. Claim under the same mutex used by completion.
        if completed_inline && snapshot.result.is_some() {
            record.owner = Owner::Inline;
            record.consumed = true;
            let result = snapshot.result.expect("checked result");
            if self.journal.is_some() {
                record.archived = true;
                record.result = None;
            }
            Ok(Submission::Inline(result))
        } else {
            if !internal_readiness {
                record.owner = Owner::Background;
            }
            if record.status.terminal() && record.owner == Owner::Background {
                let _ = self.events.send(Self::event(record));
            }
            Ok(Submission::Accepted(receipt))
        }
    }

    pub(crate) fn proposal_job(&self, chat: &str, call: &str) -> Option<JobReceipt> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut jobs = state.records.values().filter(|r| {
            r.receipt.binding.chat_id == chat
                && r.receipt.binding.tool_call_id == call
                && r.receipt.binding.tool_name == "review_update"
        });
        let job = jobs.next()?.receipt.clone();
        jobs.next().is_none().then_some(job)
    }

    pub(crate) fn proposal_boundary_clear(&self, chat: &str, own_job: Option<&str>) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .records
            .values()
            .filter(|r| r.receipt.binding.chat_id == chat)
            .all(|r| {
                if Some(r.receipt.job_id.as_str()) == own_job {
                    r.status == JobStatus::Succeeded
                } else {
                    r.status.terminal() && r.consumed
                }
            })
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
        if let Some((stop, task)) = bridge {
            let _ = stop.send(true);
            let _ = task.await;
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
        for task in tasks {
            let _ = task.await;
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
    async fn readiness_admission_v13_private_durable_success_and_interrupted_roundtrip() {
        for interrupted in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let runtime = JobRuntime::open_durable(dir.path().into(), 2, 65536)
                .await
                .unwrap();
            let mut private = binding();
            private.tool_name = "review_readiness".into();
            let path = dir.path().join("tool-jobs/ownership.sqlite");
            let (entered, worker_entered) = oneshot::channel();
            let (release, released) = oneshot::channel();
            let receipt = runtime
                .submit_readiness(private, move |_| {
                    let payload: String = rusqlite::Connection::open(path)
                        .unwrap()
                        .query_row("SELECT payload FROM tool_jobs", [], |row| row.get(0))
                        .unwrap();
                    let initial: super::super::journal::Entry =
                        serde_json::from_str(&payload).unwrap();
                    assert!(initial.internal_readiness);
                    assert!(!initial.consumed);
                    assert!(!initial.scope_keys.is_empty());
                    assert_eq!(initial.snapshot.status, JobStatus::Running);
                    assert_eq!(initial.snapshot.revision, 1);
                    async move {
                        entered.send(()).unwrap();
                        released.await.unwrap();
                        (JobStatus::Succeeded, result())
                    }
                })
                .await
                .unwrap();
            worker_entered.await.unwrap();
            assert!(runtime.background_terminals().is_empty());
            assert!(runtime
                .ack_readiness("chat", &receipt.job_id, 1)
                .await
                .is_err());
            let expected_status;
            let expected_revision;
            if interrupted {
                let task = runtime
                    .state
                    .lock()
                    .unwrap()
                    .records
                    .get_mut(&receipt.job_id)
                    .unwrap()
                    .task
                    .take()
                    .unwrap();
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
                drop(release);
                let stored = runtime
                    .journal
                    .as_ref()
                    .unwrap()
                    .load(receipt.job_id.clone())
                    .await
                    .unwrap();
                assert!(stored.internal_readiness);
                assert!(!stored.consumed);
                assert_eq!(stored.snapshot.status, JobStatus::Running);
                expected_status = JobStatus::Unknown;
                expected_revision = stored.snapshot.revision + 1;
            } else {
                release.send(()).unwrap();
                let terminal = runtime
                    .wait_terminal("chat", &receipt.job_id)
                    .await
                    .unwrap();
                assert_eq!(terminal.status, JobStatus::Succeeded);
                let stored = runtime
                    .journal
                    .as_ref()
                    .unwrap()
                    .load(receipt.job_id.clone())
                    .await
                    .unwrap();
                assert!(stored.internal_readiness);
                assert!(!stored.consumed);
                assert_eq!(stored.snapshot.status, terminal.status);
                assert_eq!(stored.snapshot.revision, terminal.revision);
                expected_status = terminal.status;
                expected_revision = terminal.revision;
            }
            runtime.shutdown().await;
            drop(runtime);
            for _ in 0..2 {
                let reopened = JobRuntime::open_durable(dir.path().into(), 2, 65536)
                    .await
                    .unwrap();
                let snapshot = reopened.get("chat", &receipt.job_id).unwrap();
                assert_eq!(snapshot.status, expected_status);
                assert_eq!(snapshot.revision, expected_revision);
                assert_eq!(
                    snapshot.receipt.binding.scope_keys,
                    receipt.binding.scope_keys
                );
                {
                    let state = reopened.state.lock().unwrap();
                    let record = &state.records[&receipt.job_id];
                    assert!(record.owner == Owner::InternalReadiness);
                    assert!(record.consumed);
                }
                assert!(reopened.background_terminals().is_empty());
                assert!(reopened
                    .ack_readiness("chat", &receipt.job_id, expected_revision + 1)
                    .await
                    .is_err());
                assert!(reopened
                    .ack_readiness("other-chat", &receipt.job_id, expected_revision)
                    .await
                    .is_err());
                reopened
                    .ack_readiness("chat", &receipt.job_id, expected_revision)
                    .await
                    .unwrap();
                let stored = reopened
                    .journal
                    .as_ref()
                    .unwrap()
                    .load(receipt.job_id.clone())
                    .await
                    .unwrap();
                assert!(stored.internal_readiness);
                assert!(stored.consumed);
                assert_eq!(stored.snapshot.status, expected_status);
                assert_eq!(stored.snapshot.revision, expected_revision);
                if interrupted {
                    assert!(matches!(
                        reopened
                            .begin_scope_change(&receipt.binding.scope_keys)
                            .await,
                        Err(super::super::ScopeChangeError::UnknownEffects)
                    ));
                    assert!(reopened
                        .scopes
                        .acquire(
                            &receipt.binding.scope_keys,
                            Arc::new(AtomicBool::new(false))
                        )
                        .is_err());
                } else {
                    reopened
                        .scopes
                        .acquire(
                            &receipt.binding.scope_keys,
                            Arc::new(AtomicBool::new(false)),
                        )
                        .unwrap()
                        .settle();
                }
                assert!(reopened.background_terminals().is_empty());
                reopened.shutdown().await;
                drop(reopened);
            }
        }
    }

    #[tokio::test]
    async fn readiness_admission_v13_legacy_absent_marker_preserves_public_owner() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = JobRuntime::open_durable(dir.path().into(), 2, 65536)
            .await
            .unwrap();
        let mut public = binding();
        public.tool_name = "review_readiness".into();
        let receipt = runtime
            .submit_background(public, |_| async { (JobStatus::Succeeded, result()) })
            .await
            .unwrap();
        let terminal = runtime
            .wait_terminal("chat", &receipt.job_id)
            .await
            .unwrap();
        runtime.shutdown().await;
        drop(runtime);
        let path = dir.path().join("tool-jobs/ownership.sqlite");
        let id = receipt.job_id.clone();
        tokio::task::spawn_blocking(move || {
            let connection = rusqlite::Connection::open(path).unwrap();
            let payload: String = connection
                .query_row(
                    "SELECT payload FROM tool_jobs WHERE job_id=?1",
                    [&id],
                    |row| row.get(0),
                )
                .unwrap();
            let mut legacy: serde_json::Value = serde_json::from_str(&payload).unwrap();
            assert_eq!(legacy["internal_readiness"], false);
            assert!(!legacy["scope_keys"].as_array().unwrap().is_empty());
            assert_eq!(legacy["consumed"], false);
            assert_eq!(
                legacy.as_object_mut().unwrap().remove("internal_readiness"),
                Some(false.into())
            );
            assert_eq!(
                connection
                    .execute(
                        "UPDATE tool_jobs SET payload=?1 WHERE job_id=?2",
                        rusqlite::params![legacy.to_string(), id],
                    )
                    .unwrap(),
                1
            );
        })
        .await
        .unwrap();
        for consumed in [false, true] {
            let reopened = JobRuntime::open_durable(dir.path().into(), 2, 65536)
                .await
                .unwrap();
            let snapshot = reopened.get("chat", &receipt.job_id).unwrap();
            assert_eq!(snapshot.status, terminal.status);
            assert_eq!(snapshot.revision, terminal.revision);
            assert_eq!(
                snapshot.receipt.binding.scope_keys,
                receipt.binding.scope_keys
            );
            {
                let state = reopened.state.lock().unwrap();
                let record = &state.records[&receipt.job_id];
                assert!(record.owner == Owner::Background);
                assert_eq!(record.consumed, consumed);
            }
            let notices = reopened.background_terminals();
            assert_eq!(notices.len(), usize::from(!consumed));
            if !consumed {
                assert_eq!(notices[0].receipt.job_id, receipt.job_id);
            }
            assert!(reopened
                .ack_readiness("chat", &receipt.job_id, terminal.revision)
                .await
                .is_err());
            assert_eq!(
                reopened.state.lock().unwrap().records[&receipt.job_id].consumed,
                consumed
            );
            reopened
                .ack_delivered("chat", &receipt.job_id, terminal.revision)
                .await
                .unwrap();
            let stored = reopened
                .journal
                .as_ref()
                .unwrap()
                .load(receipt.job_id.clone())
                .await
                .unwrap();
            assert!(!stored.internal_readiness);
            assert!(stored.consumed);
            assert_eq!(stored.scope_keys, receipt.binding.scope_keys);
            assert!(reopened.background_terminals().is_empty());
            reopened.shutdown().await;
            drop(reopened);
        }
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
    async fn durable_restart_unknown_duplicate_and_result_ack_reload() {
        let dir = tempfile::tempdir().unwrap();
        let namespace = dir.path().to_path_buf();
        let runtime = JobRuntime::open_durable(namespace.clone(), 2, 1)
            .await
            .unwrap();
        let receipt = runtime
            .submit_background(binding(), |_| async { (JobStatus::Succeeded, result()) })
            .await
            .unwrap();
        let terminal = runtime
            .wait_terminal("chat", &receipt.job_id)
            .await
            .unwrap();
        runtime
            .ack_delivered("chat", &receipt.job_id, terminal.revision)
            .await
            .unwrap();
        assert!(runtime.background_terminals().is_empty());
        runtime.shutdown().await;
        drop(runtime);
        let runtime = JobRuntime::open_durable(namespace.clone(), 2, 1)
            .await
            .unwrap();
        let reloaded = runtime
            .get_result("chat", &receipt.job_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            reloaded.result.unwrap().messages[0]
                .content
                .content_text_only(),
            "plain, not JSON"
        );
        assert!(runtime.background_terminals().is_empty());
        assert!(runtime
            .submit_background(binding(), |_| async { panic!("duplicate replay") })
            .await
            .is_err());
        let mut pending_binding = binding();
        pending_binding.tool_call_id = "pending".into();
        let pending = JobSnapshot {
            receipt: JobReceipt {
                job_id: "pending-job".into(),
                binding: pending_binding,
                status: JobStatus::Accepted,
                result_url: "local".into(),
                cancel_url: "local".into(),
            },
            status: JobStatus::Running,
            revision: 1,
            result: None,
            result_unavailable_reason: None,
        };
        runtime.persist(pending, false, false).await.unwrap();
        runtime.shutdown().await;
        drop(runtime);
        let restarted = JobRuntime::open_durable(namespace, 2, 1).await.unwrap();
        let pending = restarted.get("chat", "pending-job").unwrap();
        assert_eq!(pending.status, JobStatus::Unknown);
        assert!(pending
            .result_unavailable_reason
            .unwrap()
            .contains("never replay"));
        assert!(!pending.receipt.binding.scope_keys.is_empty());
        assert!(matches!(
            restarted
                .begin_scope_change(&pending.receipt.binding.scope_keys)
                .await,
            Err(super::super::ScopeChangeError::UnknownEffects)
        ));
        assert!(restarted
            .scopes
            .acquire(
                &pending.receipt.binding.scope_keys,
                Arc::new(AtomicBool::new(false))
            )
            .is_err());
        assert!(restarted
            .submit_background(pending.receipt.binding, |_| async {
                panic!("pending replay")
            })
            .await
            .is_err());
        restarted.shutdown().await;
    }

    #[tokio::test]
    async fn second_live_journal_owner_is_refused_without_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = JobRuntime::open_durable(dir.path().into(), 2, 65536)
            .await
            .unwrap();
        let receipt = runtime
            .submit_background(binding(), |_| async {
                futures::future::pending::<()>().await;
                (JobStatus::Succeeded, result())
            })
            .await
            .unwrap();
        assert!(JobRuntime::open_durable(dir.path().into(), 2, 65536)
            .await
            .is_err());
        assert_eq!(
            runtime
                .journal
                .as_ref()
                .unwrap()
                .load(receipt.job_id.clone())
                .await
                .unwrap()
                .snapshot
                .status,
            JobStatus::Running
        );
        // Simulate process loss, then the next exclusive owner may recover.
        drop(runtime);
        let restarted = JobRuntime::open_durable(dir.path().into(), 2, 65536)
            .await
            .unwrap();
        assert_eq!(
            restarted.get("chat", &receipt.job_id).unwrap().status,
            JobStatus::Unknown
        );
        restarted.shutdown().await;
    }

    #[tokio::test]
    async fn dropped_durable_admission_joins_io_never_constructs_effect_and_recovers_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = JobRuntime::open_durable(dir.path().into(), 1, 65536)
            .await
            .unwrap();
        let (entered, admission_entered) = oneshot::channel();
        let (release, released) = oneshot::channel();
        *runtime
            .journal
            .as_ref()
            .unwrap()
            .admission_gate
            .lock()
            .unwrap() = Some((entered, released));
        let effects = Arc::new(AtomicUsize::new(0));
        let counter = effects.clone();
        let mut submission = Box::pin(runtime.submit(binding(), Duration::ZERO, move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            async { (JobStatus::Succeeded, result()) }
        }));
        assert!(futures::poll!(&mut submission).is_pending());
        admission_entered.await.unwrap();
        let id = runtime
            .state
            .lock()
            .unwrap()
            .records
            .keys()
            .next()
            .unwrap()
            .clone();
        let keys = runtime.get("chat", &id).unwrap().receipt.binding.scope_keys;
        drop(submission);
        assert_eq!(effects.load(Ordering::SeqCst), 0);
        assert_eq!(runtime.get("chat", &id).unwrap().status, JobStatus::Running);
        release.send(()).unwrap();
        let terminal =
            tokio::time::timeout(Duration::from_secs(5), runtime.wait_terminal("chat", &id))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(terminal.status, JobStatus::Cancelled);
        assert!(terminal
            .result_unavailable_reason
            .unwrap()
            .contains("never started"));
        assert_eq!(effects.load(Ordering::SeqCst), 0);
        assert_eq!(
            runtime
                .journal
                .as_ref()
                .unwrap()
                .load(id.clone())
                .await
                .unwrap()
                .snapshot
                .status,
            JobStatus::Cancelled
        );
        // No future was constructed, so releasing the fence is provably safe.
        drop(runtime.begin_scope_change(&keys).await.unwrap());
        let mut next = binding();
        next.tool_call_id = "next-after-dropped-admission".into();
        assert!(matches!(
            runtime
                .submit(next, Duration::from_secs(5), |_| async {
                    (JobStatus::Succeeded, result())
                })
                .await
                .unwrap(),
            Submission::Inline(_)
        ));
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn legacy_entries_without_original_provenance_refuse_all_statuses() {
        for status in [
            JobStatus::Accepted,
            JobStatus::Running,
            JobStatus::CancelRequested,
            JobStatus::Succeeded,
            JobStatus::Failed,
            JobStatus::Cancelled,
            JobStatus::Unknown,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let runtime = JobRuntime::open_durable(dir.path().into(), 2, 65536)
                .await
                .unwrap();
            runtime.shutdown().await;
            drop(runtime);
            let snapshot = JobSnapshot {
                receipt: JobReceipt {
                    job_id: "legacy".into(),
                    binding: binding(),
                    status,
                    result_url: "local".into(),
                    cancel_url: "local".into(),
                },
                status,
                revision: 1,
                result: Some(result()),
                result_unavailable_reason: None,
            };
            let identity = super::super::journal::identity(&snapshot).unwrap();
            let mut payload = serde_json::to_value(super::super::journal::Entry {
                snapshot,
                consumed: true,
                internal_readiness: false,
                inline: true,
                scope_keys: vec![],
            })
            .unwrap();
            // Exercise the legacy serde(default), not merely an explicit [].
            payload.as_object_mut().unwrap().remove("scope_keys");
            let path = dir.path().join("tool-jobs/ownership.sqlite");
            tokio::task::spawn_blocking(move || {
                rusqlite::Connection::open(path)
                    .unwrap()
                    .execute(
                        "INSERT INTO tool_jobs VALUES (?1, ?2, ?3)",
                        rusqlite::params!["legacy", identity, payload.to_string()],
                    )
                    .unwrap();
            })
            .await
            .unwrap();
            let reopened = JobRuntime::open_durable(dir.path().into(), 2, 65536).await;
            assert!(matches!(reopened, Err(error) if error.contains("provenance")));
        }
    }

    #[tokio::test]
    async fn dropped_inline_claim_during_blocking_write_retains_restart_notice() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = JobRuntime::open_durable(dir.path().into(), 2, 65536)
            .await
            .unwrap();
        let (entered, write_entered) = oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        *runtime
            .journal
            .as_ref()
            .unwrap()
            .inline_write_gate
            .lock()
            .unwrap() = Some((entered, released));
        let effects = Arc::new(AtomicUsize::new(0));
        let counter = effects.clone();
        let mut submission =
            Box::pin(
                runtime.submit(binding(), Duration::from_secs(60), move |_| {
                    counter.fetch_add(1, Ordering::SeqCst);
                    async { (JobStatus::Succeeded, result()) }
                }),
            );
        tokio::select! {
            entered = write_entered => entered.unwrap(),
            _ = &mut submission => panic!("inline write must block before return"),
            _ = tokio::time::sleep(Duration::from_secs(5)) => panic!("inline writer did not reach barrier"),
        }
        // The real spawn_blocking writer owns the SQLite connection mutex.
        // Dropping its waiter cannot stop that pending durable commit.
        drop(submission);
        let terminal = runtime.background_terminals().pop().unwrap();
        let id = terminal.receipt.job_id;
        assert_eq!(terminal.status, JobStatus::Succeeded);
        release.send(()).unwrap();
        let stored = runtime
            .journal
            .as_ref()
            .unwrap()
            .load(id.clone())
            .await
            .unwrap();
        assert!(stored.inline);
        assert!(!stored.consumed);
        runtime.shutdown().await;
        drop(runtime);
        let restarted = JobRuntime::open_durable(dir.path().into(), 2, 65536)
            .await
            .unwrap();
        let notices = restarted.background_terminals();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].receipt.job_id, id);
        assert_eq!(notices[0].status, JobStatus::Succeeded);
        assert_eq!(notices[0].revision, terminal.revision);
        assert_eq!(
            restarted
                .get_result("chat", &id)
                .await
                .unwrap()
                .unwrap()
                .result
                .unwrap()
                .messages[0]
                .content
                .content_text_only(),
            "plain, not JSON"
        );
        assert!(restarted
            .submit_background(binding(), |_| async {
                panic!("must never replay the future")
            })
            .await
            .is_err());
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        restarted
            .ack_delivered("chat", &id, terminal.revision)
            .await
            .unwrap();
        restarted.shutdown().await;
        drop(restarted);
        let acknowledged = JobRuntime::open_durable(dir.path().into(), 2, 65536)
            .await
            .unwrap();
        assert!(acknowledged.background_terminals().is_empty());
        acknowledged.shutdown().await;
    }

    #[tokio::test]
    async fn factory_construction_panic_records_unknown_and_retains_fence() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = JobRuntime::open_durable(dir.path().into(), 2, 65536)
            .await
            .unwrap();
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
        assert!(matches!(
            runtime
                .begin_scope_change(&receipt.binding.scope_keys)
                .await,
            Err(super::super::ScopeChangeError::UnknownEffects)
        ));
        runtime.shutdown().await;
        drop(runtime);
        let restarted = JobRuntime::open_durable(dir.path().into(), 2, 65536)
            .await
            .unwrap();
        assert_eq!(
            restarted.background_terminals()[0].status,
            JobStatus::Unknown
        );
        assert!(restarted
            .scopes
            .acquire(
                &receipt.binding.scope_keys,
                Arc::new(AtomicBool::new(false))
            )
            .is_err());
        restarted.shutdown().await;
    }

    #[tokio::test]
    async fn malformed_journal_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tool-jobs/ownership.sqlite");
        let runtime = JobRuntime::open_durable(dir.path().into(), 2, 1)
            .await
            .unwrap();
        runtime.shutdown().await;
        drop(runtime);
        tokio::task::spawn_blocking(move || {
            rusqlite::Connection::open(path)
                .unwrap()
                .execute(
                    "INSERT INTO tool_jobs VALUES ('bad', 'bad', 'not json')",
                    [],
                )
                .unwrap();
        })
        .await
        .unwrap();
        assert!(JobRuntime::open_durable(dir.path().into(), 2, 1)
            .await
            .is_err());
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
    async fn capacity_shutdown_and_restart_never_replay() {
        let runtime = JobRuntime::new(1, 65536);
        let (started, ready) = oneshot::channel();
        let output = runtime
            .submit(binding(), Duration::ZERO, |cancel| async move {
                started.send(()).unwrap();
                while !cancel.load(Ordering::Acquire) {
                    tokio::task::yield_now().await;
                }
                (JobStatus::Unknown, result())
            })
            .await
            .unwrap();
        ready.await.unwrap();
        let Submission::Accepted(receipt) = output else {
            panic!("expected receipt");
        };
        let mut other = binding();
        other.tool_call_id = "other".into();
        assert!(runtime
            .submit(other, Duration::ZERO, |_| async {
                panic!("full runtime must not start work");
            })
            .await
            .is_err());
        runtime.shutdown().await;
        assert_eq!(
            runtime.get("chat", &receipt.job_id).unwrap().status,
            JobStatus::Unknown
        );
        assert!(runtime
            .submit(binding(), Duration::ZERO, |_| async {
                panic!("closed runtime must not run");
            })
            .await
            .is_err());
        let restarted = JobRuntime::new(1, 65536);
        assert!(restarted.get("chat", &receipt.job_id).is_none());
        restarted.shutdown().await;
    }
}
