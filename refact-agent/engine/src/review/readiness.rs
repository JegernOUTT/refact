//! Request-local Review preflight. Receipts are progress, never permission to publish.
//! The second lifecycle lease is intentionally retained after the public job settles:
//! a completed private batch is admitted exactly once or discarded before releasing it.
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::Notify;
use crate::{
    app_state::AppState,
    chat::types::ChatSession,
    privacy::records::{DerivedZone, ReviewPrivacyInputs},
    tool_jobs::{ExecutionLease, JobBinding, JobReceipt, JobResult, JobStatus, ScopeKey},
};
use super::context::{PreparedBinding, resolve_binding};

type Key = (String, String);
type Labels = Vec<(PathBuf, DerivedZone)>;
const CAPACITY: usize = 64;
const RETENTION: Duration = Duration::from_secs(300);

async fn joined_blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| format!("Review preflight worker failed: {e}"))?
}

#[derive(Clone)]
pub(crate) struct AdmissionIdentity {
    frontier: String,
    messages_len: usize,
    nonce: String,
    abort: Arc<AtomicBool>,
    scope_cancel: Option<Arc<AtomicBool>>,
}
impl AdmissionIdentity {
    fn capture(session: &ChatSession, nonce: &str) -> Result<Self, String> {
        Ok(Self {
            frontier: frontier(session, session.messages.len())?,
            messages_len: session.messages.len(),
            nonce: nonce.into(),
            abort: session.abort_flag.clone(),
            scope_cancel: None,
        })
    }
    pub(crate) fn matches(&self, session: &ChatSession) -> bool {
        if session.closed
            || self.abort.load(Ordering::Acquire)
            || self
                .scope_cancel
                .as_ref()
                .is_some_and(|cancel| cancel.load(Ordering::Acquire))
            || !Arc::ptr_eq(&self.abort, &session.abort_flag)
        {
            return false;
        }
        if session.messages.len() == self.messages_len {
            return frontier(session, self.messages_len)
                .is_ok_and(|current| current == self.frontier);
        }
        // Only a private verifier witness may authorize its own paired tool result.
        // The complete original prefix and scope must still match this job's input.
        session.messages.len() == self.messages_len + 1
            && crate::chat::goal_verifier::readiness_allowed_tool_result(session, &self.nonce)
                .is_some_and(|id| {
                    session
                        .messages
                        .last()
                        .is_some_and(|message| message.role == "tool" && message.tool_call_id == id)
                })
            && frontier(session, self.messages_len).is_ok_and(|current| current == self.frontier)
    }
}
fn frontier(session: &ChatSession, len: usize) -> Result<String, String> {
    serde_json::to_vec(&(
        &session.messages[..len],
        &session.thread,
        &session.work_project_root,
    ))
    .map(|bytes| refact_review_core::content_hash(&bytes))
    .map_err(|e| e.to_string())
}
struct Batch {
    binding: PreparedBinding,
    inputs: ReviewPrivacyInputs,
    labels: Labels,
}
struct Entry {
    submission: Arc<AtomicBool>,
    registered: bool,
    receipt: Option<JobReceipt>,
    cancel: Option<Arc<AtomicBool>>,
    identity: AdmissionIdentity,
    completed: Option<Result<Batch, String>>,
    lease: Option<ExecutionLease>,
    born: Instant,
}
#[derive(Default)]
struct State {
    entries: HashMap<Key, Entry>,
    consumed: HashSet<Key>,
    failed: HashMap<Key, (String, String)>,
    closed: bool,
    owners: Vec<Arc<tokio::sync::Mutex<OwnerRecord>>>,
}
struct OwnerRecord {
    task: tokio::task::JoinHandle<()>,
    outcome: Option<Result<(), String>>,
}
impl State {
    fn readiness_result(&mut self, key: &Key, job_id: &str) -> Result<bool, String> {
        if let Some((failed_job, error)) = self.failed.get(key) {
            return if failed_job == job_id {
                Err(error.clone())
            } else {
                Err("Review readiness job changed".into())
            };
        }
        let entry = self
            .entries
            .get(key)
            .ok_or("Review readiness invalidated; retry preparation")?;
        if entry
            .receipt
            .as_ref()
            .is_some_and(|receipt| receipt.job_id != job_id)
        {
            return Err("Review readiness job changed".into());
        }
        match &entry.completed {
            None => Ok(false),
            Some(Ok(_)) => Ok(true),
            Some(Err(error)) => {
                let error = error.clone();
                if entry.receipt.is_some() {
                    let mut entry = self.entries.remove(key).expect("present entry");
                    if let Some(lease) = entry.lease.take() {
                        lease.settle();
                    }
                    self.consumed.insert(key.clone());
                    self.failed
                        .insert(key.clone(), (job_id.into(), error.clone()));
                }
                Err(error)
            }
        }
    }
}
#[derive(Default)]
pub struct ReadinessRegistry {
    state: Mutex<State>,
    changed: Notify,
}

struct SubmissionGuard {
    abandoned: Arc<AtomicBool>,
    registry: std::sync::Weak<ReadinessRegistry>,
    key: Key,
    armed: bool,
}
impl Drop for SubmissionGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.abandoned.store(true, Ordering::Release);
        if let Some(registry) = self.registry.upgrade() {
            let mut state = registry.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(cancel) = state
                .entries
                .get(&self.key)
                .filter(|entry| Arc::ptr_eq(&entry.submission, &self.abandoned))
                .and_then(|entry| entry.cancel.as_ref())
            {
                cancel.store(true, Ordering::Release);
            }
            if state.entries.get(&self.key).is_some_and(|entry| {
                Arc::ptr_eq(&entry.submission, &self.abandoned) && !entry.registered
            }) {
                state.entries.remove(&self.key);
            }
            registry.changed.notify_waiters();
        }
    }
}
impl ReadinessRegistry {
    /// Close first, cancel, then wait for the real blocking owners to join. Never abort.
    pub async fn shutdown(&self) -> Result<(), String> {
        let owners = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.closed = true;
            for entry in state.entries.values() {
                if let Some(cancel) = &entry.cancel {
                    cancel.store(true, Ordering::Release);
                }
            }
            state.owners.clone()
        };
        let mut failure = None;
        for owner in owners {
            let mut owner = owner.lock().await;
            if owner.outcome.is_none() {
                owner.outcome = Some(
                    (&mut owner.task)
                        .await
                        .map_err(|e| format!("Review admission owner failed: {e}")),
                );
            }
            if let Some(Err(error)) = &owner.outcome {
                failure = Some(error.clone());
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                let done = state
                    .entries
                    .iter()
                    .filter(|(_, e)| e.completed.is_some())
                    .map(|(k, _)| k.clone())
                    .collect::<Vec<_>>();
                for key in done {
                    if let Some(mut entry) = state.entries.remove(&key) {
                        if let Some(lease) = entry.lease.take() {
                            lease.settle();
                        }
                    }
                }
                if state.entries.is_empty() {
                    return Ok(());
                }
            }
            changed.await;
        }
    }
}

/// None means no Review or a freshly revalidated single-use batch was admitted.
/// This function never waits for the expensive per-origin filesystem preflight.
pub async fn prepare_readiness(
    app: &AppState,
    chat_id: &str,
    turn_nonce: &str,
) -> Result<Option<JobReceipt>, String> {
    if turn_nonce.is_empty() {
        return Err("Review preparation requires a turn nonce".into());
    }
    let registry = app.runtime.review_readiness.clone();
    let key = (chat_id.to_string(), turn_nonce.to_string());
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(chat_id)
        .cloned()
        .ok_or("Chat unavailable")?;
    // Remove a completed batch before any await: a second caller cannot publish it twice.
    let completed = {
        let mut state = registry.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.closed {
            return Err("Review readiness is shut down".into());
        }
        if state.consumed.contains(&key) {
            return Err("Review readiness nonce already consumed; start a fresh request".into());
        }
        if state.consumed.len() >= 4096 {
            return Err("Review readiness admission ledger capacity exhausted".into());
        }
        match state.entries.get(&key) {
            Some(entry) if entry.completed.is_none() => {
                return entry.receipt.clone().map(Some).ok_or_else(|| {
                    "Review submission is still being admitted; retry preparation".into()
                });
            }
            Some(entry) if matches!(entry.completed, Some(Err(_))) => {
                let error = entry
                    .completed
                    .as_ref()
                    .unwrap()
                    .as_ref()
                    .err()
                    .unwrap()
                    .clone();
                let job_id = entry.receipt.as_ref().map(|receipt| receipt.job_id.clone());
                if let Some(job_id) = job_id {
                    state.readiness_result(&key, &job_id)?;
                }
                return Err(error);
            }
            Some(_) => {
                state.consumed.insert(key.clone());
                state.entries.remove(&key)
            }
            None => None,
        }
    };
    if let Some(mut entry) = completed {
        let outcome: Result<bool, String> = async {
            let batch = entry.completed.take().expect("completed entry")?;
            if entry
                .cancel
                .as_ref()
                .is_some_and(|c| c.load(Ordering::Acquire))
                || !entry.identity.matches(&*session.lock().await)
            {
                return Err("Review readiness invalidated by a newer turn or scope change".into());
            }
            let current = resolve_binding(app, chat_id)
                .await?
                .ok_or("Review disappeared during readiness")?;
            if !batch.binding.same_identity(&current) {
                return Err("Review binding changed during readiness".into());
            }
            // Drift is a normal restart, not permission to use old labels.
            if !batch.inputs.still_current(app.gcx.clone()).await {
                return Ok(false);
            }
            super::context::publish_binding_checked(
                app,
                chat_id,
                &batch.binding,
                batch.labels,
                Some(&entry.identity),
            )
            .await?;
            Ok(true)
        }
        .await;
        if let Some(lease) = entry.lease.take() {
            lease.settle();
        }
        registry.changed.notify_waiters();
        if outcome? {
            return Ok(None);
        }
        // Policy drift deliberately redoes this same parked request from scratch.
        registry
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .consumed
            .remove(&key);
    }
    // Strict checkout validation is cheap relative to full origin scans and has no publication effects.
    let Some(binding) = resolve_binding(app, chat_id).await? else {
        return Ok(None);
    };
    let identity = AdmissionIdentity::capture(&*session.lock().await, turn_nonce)?;
    let mut keys = vec![
        app.runtime.tool_jobs.chat_scope_key(chat_id),
        ScopeKey::Worktree {
            project: binding.worktree.source_workspace_root.clone(),
            root: binding.worktree.root.clone(),
        },
    ];
    if let Some(previous) = &binding.previous {
        if previous != &binding.worktree {
            keys.push(ScopeKey::Worktree {
                project: previous.source_workspace_root.clone(),
                root: previous.root.clone(),
            });
        }
    }
    let abandoned = Arc::new(AtomicBool::new(false));
    {
        let mut state = registry.state.lock().unwrap_or_else(|e| e.into_inner());
        use futures::FutureExt;
        let mut owner_failed = false;
        state.owners.retain(|record| {
            let Ok(mut owner) = record.try_lock() else {
                return true;
            };
            if owner.outcome.is_none() && owner.task.is_finished() {
                owner.outcome = (&mut owner.task)
                    .now_or_never()
                    .map(|result| result.map_err(|e| e.to_string()));
            }
            owner_failed |= matches!(owner.outcome, Some(Err(_)));
            !matches!(owner.outcome, Some(Ok(())))
        });
        if owner_failed {
            state.closed = true;
        }
        if state.owners.len() >= CAPACITY {
            return Err("Review readiness owner capacity exhausted; retry after cleanup".into());
        }
        if state.closed {
            return Err("Review readiness is shut down".into());
        }
        if state.consumed.contains(&key) {
            return Err("Review readiness nonce already consumed; start a fresh request".into());
        }
        if state.consumed.len() + state.entries.len() >= 4096 {
            return Err("Review readiness admission ledger capacity exhausted".into());
        }
        if state.entries.len() >= CAPACITY {
            return Err(
                "Review readiness capacity exhausted; retry after pending work completes".into(),
            );
        }
        if state.entries.contains_key(&key) {
            return Err("Review readiness already submitted".into());
        }
        state.entries.insert(
            key.clone(),
            Entry {
                submission: abandoned.clone(),
                registered: false,
                receipt: None,
                cancel: None,
                identity,
                completed: None,
                lease: None,
                born: Instant::now(),
            },
        );
    }
    let mut guard = SubmissionGuard {
        abandoned: abandoned.clone(),
        registry: Arc::downgrade(&registry),
        key: key.clone(),
        armed: true,
    };
    let (reply, received) = tokio::sync::oneshot::channel();
    let job_binding = JobBinding {
        scope_keys: keys.clone(),
        chat_id: chat_id.into(),
        tool_call_id: format!("review-readiness-{}", uuid::Uuid::new_v4()),
        tool_name: "review_readiness".into(),
        model: String::new(),
        turn_id: turn_nonce.into(),
    };
    let worker_app = app.clone();
    let worker_registry = registry.clone();
    let worker_key = key.clone();
    let weak = Arc::downgrade(&registry);
    let jobs = app.runtime.tool_jobs.clone();
    let worker_abandoned = abandoned.clone();
    #[cfg(test)]
    tests::admission_hook("reserved").await;
    {
        let mut registration = registry.state.lock().unwrap_or_else(|e| e.into_inner());
        if registration.closed || registration.owners.len() >= CAPACITY {
            if registration
                .entries
                .get(&key)
                .is_some_and(|entry| Arc::ptr_eq(&entry.submission, &abandoned))
            {
                registration.entries.remove(&key);
            }
            registry.changed.notify_waiters();
            return Err("Review readiness is shut down".into());
        }
        if let Some(entry) = registration
            .entries
            .get_mut(&key)
            .filter(|entry| Arc::ptr_eq(&entry.submission, &abandoned))
        {
            entry.registered = true;
        }
        let owner_token = abandoned.clone();
        let owner = tokio::spawn(async move {
            #[cfg(test)]
            tests::admission_hook("before_submit").await;
            let receipt = jobs
                .submit_readiness(job_binding, move |cancel| {
                    // Register the admission lease synchronously before the owner can be polled.
                    let lease = worker_app
                        .runtime
                        .tool_jobs
                        .scopes
                        .acquire(&keys, cancel.clone());
                    {
                        let mut state = worker_registry
                            .state
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        if let Some(entry) = state
                            .entries
                            .get_mut(&worker_key)
                            .filter(|entry| Arc::ptr_eq(&entry.submission, &worker_abandoned))
                        {
                            entry.cancel = Some(cancel.clone());
                            entry.identity.scope_cancel = Some(cancel.clone());
                        }
                        if state.closed || worker_abandoned.load(Ordering::Acquire) {
                            cancel.store(true, Ordering::Release);
                        }
                    }
                    async move {
                        let result = match lease {
                            Ok(lease) => {
                                use futures::FutureExt;
                                let result = std::panic::AssertUnwindSafe(build_batch(
                                    &worker_app,
                                    binding,
                                    cancel.clone(),
                                ))
                                .catch_unwind()
                                .await
                                .unwrap_or_else(|_| {
                                    Err("Review preflight panicked; retry explicitly".into())
                                });
                                let mut state = worker_registry
                                    .state
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner());
                                if result.is_err() {
                                    // The owner joined the actual blocking closure: a denied
                                    // batch has no publication rights and must not pin writers.
                                    lease.settle();
                                } else if let Some(entry) =
                                    state.entries.get_mut(&worker_key).filter(|entry| {
                                        Arc::ptr_eq(&entry.submission, &worker_abandoned)
                                    })
                                {
                                    entry.lease = Some(lease);
                                } else {
                                    lease.settle();
                                }
                                result
                            }
                            Err(e) => Err(format!("Review admission scope unavailable: {e}")),
                        };
                        let status = if cancel.load(Ordering::Acquire) {
                            JobStatus::Cancelled
                        } else if result.is_ok() {
                            JobStatus::Succeeded
                        } else {
                            JobStatus::Failed
                        };
                        {
                            let mut state = worker_registry
                                .state
                                .lock()
                                .unwrap_or_else(|e| e.into_inner());
                            if let Some(entry) = state
                                .entries
                                .get_mut(&worker_key)
                                .filter(|entry| Arc::ptr_eq(&entry.submission, &worker_abandoned))
                            {
                                entry.completed = Some(result);
                            }
                        }
                        worker_registry.changed.notify_waiters();
                        (
                            status,
                            JobResult {
                                had_corrections: false,
                                messages: vec![],
                                context_files: vec![],
                            },
                        )
                    }
                })
                .await;
            let receipt = match receipt {
                Ok(receipt) => receipt,
                Err(error) => {
                    let Some(registry) = weak.upgrade() else {
                        return;
                    };
                    let mut state = registry.state.lock().unwrap_or_else(|e| e.into_inner());
                    if state
                        .entries
                        .get(&key)
                        .is_some_and(|entry| Arc::ptr_eq(&entry.submission, &owner_token))
                    {
                        state.entries.remove(&key);
                    }
                    registry.changed.notify_waiters();
                    let _ = reply.send(Err(error));
                    return;
                }
            };
            let Some(registry) = weak.upgrade() else {
                return;
            };
            {
                let mut state = registry.state.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(entry) = state
                    .entries
                    .get_mut(&key)
                    .filter(|entry| Arc::ptr_eq(&entry.submission, &owner_token))
                {
                    entry.receipt = Some(receipt.clone());
                }
            }
            #[cfg(test)]
            tests::admission_hook("before_reply").await;
            let _ = reply.send(Ok(receipt.clone()));
            // Ready batches cannot pin a writer forever: cancellation, changed human frontier,
            // or bounded retention discards them, but only after build_batch actually joined.
            drop(registry);
            let completed_receipt = receipt.clone();
            loop {
                tokio::time::sleep(Duration::from_millis(50)).await;
                let Some(registry) = weak.upgrade() else {
                    break;
                };
                let invalid = {
                    let state = registry.state.lock().unwrap_or_else(|e| e.into_inner());
                    let Some(entry) = state
                        .entries
                        .get(&key)
                        .filter(|entry| Arc::ptr_eq(&entry.submission, &owner_token))
                    else {
                        break;
                    };
                    state.closed
                        || abandoned.load(Ordering::Acquire)
                        || entry.born.elapsed() >= RETENTION
                        || entry
                            .cancel
                            .as_ref()
                            .is_some_and(|c| c.load(Ordering::Acquire))
                };
                let human_changed = {
                    let s = session.lock().await;
                    let state = registry.state.lock().unwrap_or_else(|e| e.into_inner());
                    state
                        .entries
                        .get(&key)
                        .filter(|entry| Arc::ptr_eq(&entry.submission, &owner_token))
                        .is_some_and(|entry| !entry.identity.matches(&s))
                };
                if invalid || human_changed {
                    let mut state = registry.state.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(entry) = state
                        .entries
                        .get_mut(&key)
                        .filter(|entry| Arc::ptr_eq(&entry.submission, &owner_token))
                    {
                        if let Some(cancel) = &entry.cancel {
                            cancel.store(true, Ordering::Release);
                        }
                        if entry.completed.is_some() {
                            let mut entry = state.entries.remove(&key).expect("present entry");
                            if let Some(lease) = entry.lease.take() {
                                lease.settle();
                            }
                            registry.changed.notify_waiters();
                            break;
                        }
                    }
                }
            }
            if let Some(terminal) = jobs
                .wait_terminal(
                    &completed_receipt.binding.chat_id,
                    &completed_receipt.job_id,
                )
                .await
            {
                let _ = jobs
                    .ack_readiness(
                        &completed_receipt.binding.chat_id,
                        &completed_receipt.job_id,
                        terminal.revision,
                    )
                    .await;
            }
        });
        registration
            .owners
            .push(Arc::new(tokio::sync::Mutex::new(OwnerRecord {
                task: owner,
                outcome: None,
            })));
    }
    let receipt = received
        .await
        .map_err(|_| "Review admission owner unavailable".to_string())??;
    // A returned receipt transfers cancellation responsibility to normal lifecycle checks.
    guard.armed = false;
    Ok(Some(receipt))
}

async fn build_batch(
    app: &AppState,
    binding: PreparedBinding,
    cancel: Arc<AtomicBool>,
) -> Result<Batch, String> {
    let inputs = crate::privacy::records::prepare_review_privacy_inputs(app.gcx.clone()).await?;
    let checked = inputs.clone();
    let candidate = binding.candidate.clone();
    let worktree = binding.worktree.clone();
    let gcx = app.gcx.clone();
    // Always await the JoinHandle, including cooperative cancellation. A dropped owner
    // is not quiescence; its retained lease consequently drops Unknown (fail closed).
    #[cfg(test)]
    let barrier = tests::WORKER_BARRIER.with(|hook| hook.borrow_mut().take());
    let labels = joined_blocking(move || {
        #[cfg(test)]
        if let Some((entered, release)) = barrier {
            let _ = entered.send(());
            release.recv().unwrap();
        }
        let origins = candidate
            .files
            .iter()
            .map(|file| PathBuf::from(&candidate.repository).join(&file.path))
            .collect::<Vec<_>>();
        for path in &origins {
            if cancel.load(Ordering::Acquire) {
                return Err("Review readiness cancelled".to_string());
            }
            checked.check_origin(path)?;
        }
        let records = checked.records_cancellable(&gcx, origins, &cancel)?;
        let records = records
            .into_iter()
            .map(|r| (PathBuf::from(&r.path), r))
            .collect::<HashMap<_, _>>();
        candidate
            .files
            .iter()
            .map(|file| {
                let origin = PathBuf::from(&candidate.repository).join(&file.path);
                let record = records.get(&origin).ok_or("Review origin record missing")?;
                Ok((
                    worktree.root.join(&file.path),
                    DerivedZone {
                        zone: record.zone.clone(),
                        origin: Some(record.path.clone()),
                    },
                ))
            })
            .collect::<Result<Labels, String>>()
    })
    .await?;
    Ok(Batch {
        binding,
        inputs,
        labels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::ChatMessage;

    type WorkerBarrier = (
        tokio::sync::oneshot::Sender<()>,
        std::sync::mpsc::Receiver<()>,
    );
    thread_local! {
        pub(super) static WORKER_BARRIER: std::cell::RefCell<Option<WorkerBarrier>> = const { std::cell::RefCell::new(None) };
    }

    thread_local! {
        static ADMISSION_HOOK: std::cell::RefCell<Option<(&'static str, tokio::sync::oneshot::Sender<()>, tokio::sync::oneshot::Receiver<()>)>> = const { std::cell::RefCell::new(None) };
    }
    pub(super) async fn admission_hook(stage: &str) {
        let hook = ADMISSION_HOOK.with(|hook| {
            let mut hook = hook.borrow_mut();
            if hook
                .as_ref()
                .is_some_and(|(expected, _, _)| *expected == stage)
            {
                hook.take()
            } else {
                None
            }
        });
        if let Some((_, entered, release)) = hook {
            let _ = entered.send(());
            let _ = release.await;
        }
    }
    fn pause_admission(
        stage: &'static str,
    ) -> (
        tokio::sync::oneshot::Receiver<()>,
        tokio::sync::oneshot::Sender<()>,
    ) {
        let (entered, ready) = tokio::sync::oneshot::channel();
        let (release, blocked) = tokio::sync::oneshot::channel();
        ADMISSION_HOOK.with(|hook| *hook.borrow_mut() = Some((stage, entered, blocked)));
        (ready, release)
    }

    async fn drop_fixture() -> (tempfile::TempDir, AppState) {
        use crate::at_commands::at_commands::AtCommandsContext;
        use super::super::context::{set_context, SetContextRequest};
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir(&source).unwrap();
        let oid = {
            let repo = git2::Repository::init(&source).unwrap();
            let blob = repo.blob(b"original\n").unwrap();
            let mut tree = repo.treebuilder(None).unwrap();
            tree.insert("tracked", blob, 0o100644).unwrap();
            let tree_oid = tree.write().unwrap();
            let signature = git2::Signature::now("Test", "test@localhost").unwrap();
            let oid = repo
                .commit(
                    Some("HEAD"),
                    &signature,
                    &signature,
                    "initial",
                    &repo.find_tree(tree_oid).unwrap(),
                    &[],
                )
                .unwrap();
            repo.checkout_head(None).unwrap();
            oid
        };
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            temp.path().join("cache"),
            temp.path().join("config"),
        )
        .await;
        crate::privacy::save_privacy_policy(gcx.clone(), crate::privacy::PrivacyPolicy::default())
            .await
            .unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        let mut app = AppState::from_gcx(gcx).await;
        app.runtime.tool_jobs = crate::tool_jobs::JobRuntime::with_namespace(
            temp.path().join("jobs"),
            8,
            crate::tool_jobs::RESULT_RETENTION_BYTES,
        );
        let mut session = ChatSession::new("drop-v14".into());
        session.work_project_root = Some(source.clone());
        session.set_runtime_state(crate::chat::types::SessionState::ExecutingTools, None);
        let abort = session.abort_flag.clone();
        app.chat.sessions.write().await.insert(
            "drop-v14".into(),
            Arc::new(tokio::sync::Mutex::new(session)),
        );
        let context = AtCommandsContext::new_with_abort(
            app.clone(),
            4096,
            10,
            false,
            vec![],
            "drop-v14".into(),
            None,
            String::new(),
            None,
            None,
            Some(abort),
        )
        .await;
        set_context(
            Arc::new(tokio::sync::Mutex::new(context)),
            SetContextRequest {
                repository: source.to_string_lossy().into(),
                base_ref: oid.to_string(),
                candidate_ref: oid.to_string(),
                policy: refact_review_core::CapturePolicy::default(),
                title: "Review".into(),
                target_reference: "local".into(),
                review_id: None,
                expected_revision: None,
                request_key: "drop-fixture".into(),
            },
        )
        .await
        .unwrap();
        (temp, app)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn readiness_drop_v14_during_job_admission_cleans_and_shutdown_joins() {
        let (_temp, app) = drop_fixture().await;
        let (mut ready, release) = pause_admission("before_submit");
        let mut submitting = Box::pin(prepare_readiness(&app, "drop-v14", "initial"));
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                assert!(futures::poll!(&mut submitting).is_pending());
                if ready.try_recv().is_ok() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(app
            .runtime
            .review_readiness
            .state
            .lock()
            .unwrap()
            .entries
            .contains_key(&("drop-v14".into(), "initial".into())));
        assert!(app
            .runtime
            .review_readiness
            .state
            .lock()
            .unwrap()
            .entries
            .values()
            .all(|entry| entry.cancel.is_none()));
        drop(submitting);
        release.send(()).unwrap();
        tokio::time::timeout(
            Duration::from_secs(10),
            app.runtime.review_readiness.shutdown(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(app
            .runtime
            .review_readiness
            .state
            .lock()
            .unwrap()
            .entries
            .is_empty());
        app.runtime.tool_jobs.shutdown().await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn readiness_drop_v14_after_factory_retains_fence_until_real_join() {
        let (_temp, app) = drop_fixture().await;
        let (entered, ready) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        WORKER_BARRIER.with(|hook| *hook.borrow_mut() = Some((entered, blocked)));
        let (reply_ready, reply_release) = pause_admission("before_reply");
        let mut submitting = Box::pin(prepare_readiness(&app, "drop-v14", "worker"));
        // Stop polling the caller once reserved; its owned admission continues to the factory.
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                assert!(futures::poll!(&mut submitting).is_pending());
                if !app
                    .runtime
                    .review_readiness
                    .state
                    .lock()
                    .unwrap()
                    .entries
                    .is_empty()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(10), ready)
            .await
            .unwrap()
            .unwrap();
        reply_ready.await.unwrap();
        drop(submitting);
        let cancel = app
            .runtime
            .review_readiness
            .state
            .lock()
            .unwrap()
            .entries
            .values()
            .next()
            .unwrap()
            .cancel
            .clone()
            .unwrap();
        assert!(cancel.load(Ordering::Acquire));
        let scopes = app.runtime.tool_jobs.scopes.clone();
        let keys = [app.runtime.tool_jobs.chat_scope_key("drop-v14")];
        let mut transition = Box::pin(scopes.begin_scope_change(&keys));
        assert!(futures::poll!(&mut transition).is_pending());
        let mut shutdown = Box::pin(app.runtime.review_readiness.shutdown());
        assert!(futures::poll!(&mut shutdown).is_pending());
        drop(shutdown);
        assert_eq!(
            app.runtime
                .review_readiness
                .state
                .lock()
                .unwrap()
                .owners
                .len(),
            1
        );
        reply_release.send(()).unwrap();
        release.send(()).unwrap();
        tokio::time::timeout(
            Duration::from_secs(10),
            app.runtime.review_readiness.shutdown(),
        )
        .await
        .unwrap()
        .unwrap();
        drop(
            tokio::time::timeout(Duration::from_secs(10), transition)
                .await
                .unwrap()
                .unwrap(),
        );
        assert!(app
            .runtime
            .review_readiness
            .state
            .lock()
            .unwrap()
            .entries
            .is_empty());
        app.runtime.tool_jobs.shutdown().await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn readiness_drop_v14_shutdown_between_reservation_and_registration() {
        let (_temp, app) = drop_fixture().await;
        let (mut ready, release) = pause_admission("reserved");
        let mut submitting = Box::pin(prepare_readiness(&app, "drop-v14", "registration"));
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                assert!(futures::poll!(&mut submitting).is_pending());
                if ready.try_recv().is_ok() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let mut shutdown = Box::pin(app.runtime.review_readiness.shutdown());
        assert!(futures::poll!(&mut shutdown).is_pending());
        release.send(()).unwrap();
        assert!(submitting.await.is_err());
        tokio::time::timeout(Duration::from_secs(10), shutdown)
            .await
            .unwrap()
            .unwrap();
        assert!(app
            .runtime
            .review_readiness
            .state
            .lock()
            .unwrap()
            .owners
            .is_empty());
        app.runtime.tool_jobs.shutdown().await;
    }

    fn failed_entry(nonce: &str) -> Entry {
        Entry {
            submission: Arc::new(AtomicBool::new(false)),
            registered: true,
            receipt: Some(JobReceipt {
                job_id: nonce.into(),
                binding: JobBinding {
                    scope_keys: vec![],
                    chat_id: "failed".into(),
                    tool_call_id: nonce.into(),
                    tool_name: "review_readiness".into(),
                    model: String::new(),
                    turn_id: nonce.into(),
                },
                status: JobStatus::Failed,
                result_url: String::new(),
                cancel_url: String::new(),
            }),
            cancel: None,
            identity: AdmissionIdentity::capture(&ChatSession::new("failed".into()), nonce)
                .unwrap(),
            completed: Some(Err("preflight denied".into())),
            lease: None,
            born: Instant::now(),
        }
    }

    #[test]
    fn consumed_failures_recover_capacity_and_preserve_exact_job_errors() {
        let mut state = State::default();
        for index in 0..CAPACITY {
            let nonce = index.to_string();
            state
                .entries
                .insert(("failed".into(), nonce.clone()), failed_entry(&nonce));
        }
        assert_eq!(state.entries.len(), CAPACITY);
        let key = ("failed".into(), "0".into());
        assert_eq!(
            state.readiness_result(&key, "wrong"),
            Err("Review readiness job changed".into())
        );
        assert_eq!(state.entries.len(), CAPACITY);
        assert_eq!(
            state.readiness_result(&key, "0"),
            Err("preflight denied".into())
        );
        assert_eq!(state.entries.len(), CAPACITY - 1);
        assert!(state.consumed.contains(&key));
        assert_eq!(
            state.readiness_result(&key, "0"),
            Err("preflight denied".into())
        );
        assert_eq!(
            state.readiness_result(&key, "wrong"),
            Err("Review readiness job changed".into())
        );
        state
            .entries
            .insert(("failed".into(), "fresh".into()), failed_entry("fresh"));
        assert_eq!(state.entries.len(), CAPACITY);
        assert_eq!(state.failed.len(), 1);
    }

    #[tokio::test]
    async fn concurrent_failure_waiters_observe_same_error_after_entry_release() {
        let registry = Arc::new(ReadinessRegistry::default());
        let key = ("failed".into(), "nonce".into());
        registry
            .state
            .lock()
            .unwrap()
            .entries
            .insert(key.clone(), failed_entry("nonce"));
        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let mut waiters = Vec::new();
        for _ in 0..2 {
            let registry = registry.clone();
            let key = key.clone();
            let barrier = barrier.clone();
            waiters.push(tokio::spawn(async move {
                barrier.wait().await;
                let result = registry
                    .state
                    .lock()
                    .unwrap()
                    .readiness_result(&key, "nonce");
                result
            }));
        }
        barrier.wait().await;
        for waiter in waiters {
            assert_eq!(waiter.await.unwrap(), Err("preflight denied".into()));
        }
        let state = registry.state.lock().unwrap();
        assert!(state.entries.is_empty());
        assert_eq!(state.failed.len(), 1);
        assert_eq!(state.consumed.len(), 1);
    }

    #[test]
    fn admission_identity_rejects_human_scope_abort_and_replaced_abort() {
        for mutation in [
            "human",
            "scope",
            "abort",
            "replacement",
            "closed",
            "scope_cancel",
        ] {
            let mut session = ChatSession::new("identity".into());
            let mut identity = AdmissionIdentity::capture(&session, "test").unwrap();
            assert!(identity.matches(&session));
            match mutation {
                "human" => session
                    .messages
                    .push(ChatMessage::new("user".into(), "new request".into())),
                "scope" => session.work_project_root = Some("/different".into()),
                "abort" => session.abort_flag.store(true, Ordering::Release),
                "replacement" => session.abort_flag = Arc::new(AtomicBool::new(false)),
                "closed" => session.closed = true,
                _ => identity.scope_cancel = Some(Arc::new(AtomicBool::new(true))),
            }
            assert!(!identity.matches(&session), "{mutation}");
        }
    }

    #[tokio::test]
    async fn cancellation_does_not_release_lease_until_blocking_worker_joins() {
        let scopes = Arc::new(crate::tool_jobs::ScopeCoordinator::default());
        let key = ScopeKey::Chat {
            project: "/test".into(),
            chat_id: "joined".into(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let lease = scopes.acquire(&[key.clone()], cancel.clone()).unwrap();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let worker = tokio::spawn(async move {
            let result = joined_blocking(move || {
                started.send(()).unwrap();
                blocked.recv().unwrap();
                assert!(cancel.load(Ordering::Acquire));
                Err::<(), String>("cancelled".into())
            })
            .await;
            lease.settle();
            result
        });
        ready.await.unwrap();
        let keys = [key.clone()];
        let transition = scopes.begin_scope_change(&keys);
        tokio::pin!(transition);
        assert!(futures::poll!(&mut transition).is_pending());
        assert!(
            !worker.is_finished(),
            "cancellation must not detach blocking work"
        );
        release.send(()).unwrap();
        assert!(worker.await.unwrap().is_err());
        drop(transition.await.unwrap());
        scopes
            .acquire(&[key], Arc::new(AtomicBool::new(false)))
            .unwrap()
            .settle();
    }

    #[tokio::test]
    async fn shutdown_discards_ready_private_batch_and_releases_admission_lease() {
        let registry = ReadinessRegistry::default();
        let scopes = Arc::new(crate::tool_jobs::ScopeCoordinator::default());
        let key = ScopeKey::Chat {
            project: "/test".into(),
            chat_id: "ready".into(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let lease = scopes.acquire(&[key.clone()], cancel.clone()).unwrap();
        registry.state.lock().unwrap().entries.insert(
            ("ready".into(), "nonce".into()),
            Entry {
                submission: Arc::new(AtomicBool::new(false)),
                registered: true,
                receipt: None,
                cancel: Some(cancel.clone()),
                identity: AdmissionIdentity::capture(&ChatSession::new("ready".into()), "nonce")
                    .unwrap(),
                completed: Some(Err("discard ready batch".into())),
                lease: Some(lease),
                born: Instant::now(),
            },
        );
        registry.shutdown().await.unwrap();
        assert!(cancel.load(Ordering::Acquire));
        assert!(registry.state.lock().unwrap().entries.is_empty());
        assert!(registry.state.lock().unwrap().closed);
        let guard = scopes.begin_scope_change(&[key]).await.unwrap();
        drop(guard);
    }
}

/// Only background continuation tasks may wait here; preparation returns Pending.
pub async fn wait_for_readiness(
    app: &AppState,
    chat_id: &str,
    receipt: &JobReceipt,
    turn_nonce: &str,
) -> Result<(), String> {
    if receipt.binding.chat_id != chat_id
        || receipt.binding.turn_id != turn_nonce
        || receipt.binding.tool_name != "review_readiness"
    {
        return Err("Review readiness receipt identity mismatch".into());
    }
    let key = (chat_id.to_string(), turn_nonce.to_string());
    loop {
        let changed = app.runtime.review_readiness.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        {
            let mut state = app
                .runtime
                .review_readiness
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            match state.readiness_result(&key, &receipt.job_id) {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(error) => {
                    app.runtime.review_readiness.changed.notify_waiters();
                    return Err(error);
                }
            }
        }
        changed.await;
    }
}
