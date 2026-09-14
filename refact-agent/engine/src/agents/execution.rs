use refact_work_core::{AdmissionError, Permit, Ticket};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionState {
    Running,
    Suspending,
    Acquiring,
    Parked,
    Gone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParkOutcome {
    Parked,
    AlreadyParked,
    Acquiring,
    Draining,
    NotHeld,
}

enum Slot {
    Running(Permit),
    Acquiring,
    Parked(Ticket),
    Gone,
}

struct Shared {
    slot: Slot,
    active_ops: usize,
    pending_waits: usize,
    cancelled: bool,
}

struct Inner {
    state: Mutex<Shared>,
    notify: Notify,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn wake(&self) {
        self.notify.notify_waiters();
    }
}

pub struct OperationGuard {
    inner: Arc<Inner>,
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        let drained = {
            let mut state = self.inner.lock();
            state.active_ops = state.active_ops.saturating_sub(1);
            state.active_ops == 0
        };
        if drained {
            self.inner.wake();
        }
    }
}

pub struct WaitGuard {
    inner: Arc<Inner>,
}

impl Drop for WaitGuard {
    fn drop(&mut self) {
        {
            let mut state = self.inner.lock();
            state.pending_waits = state.pending_waits.saturating_sub(1);
        }
        self.inner.wake();
    }
}

struct AcquireGuard {
    inner: Arc<Inner>,
    armed: bool,
}

impl Drop for AcquireGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        {
            let mut state = self.inner.lock();
            if matches!(state.slot, Slot::Acquiring) {
                state.slot = Slot::Gone;
            }
        }
        self.inner.wake();
    }
}

fn park_in_place(state: &mut Shared) -> Result<ParkOutcome, AdmissionError> {
    match std::mem::replace(&mut state.slot, Slot::Gone) {
        Slot::Running(permit) => match permit.park() {
            Ok(ticket) => {
                state.slot = Slot::Parked(ticket);
                Ok(ParkOutcome::Parked)
            }
            Err(err) => {
                state.slot = Slot::Gone;
                Err(err)
            }
        },
        Slot::Parked(ticket) => {
            state.slot = Slot::Parked(ticket);
            Ok(ParkOutcome::AlreadyParked)
        }
        Slot::Acquiring => {
            state.slot = Slot::Acquiring;
            Ok(ParkOutcome::Acquiring)
        }
        Slot::Gone => {
            state.slot = Slot::Gone;
            Ok(ParkOutcome::NotHeld)
        }
    }
}

pub struct RunExecution {
    inner: Arc<Inner>,
    admission_generation: u64,
}

impl RunExecution {
    pub fn new(permit: Permit) -> Self {
        Self {
            admission_generation: permit.id().0,
            inner: Arc::new(Inner {
                state: Mutex::new(Shared {
                    slot: Slot::Running(permit),
                    active_ops: 0,
                    pending_waits: 0,
                    cancelled: false,
                }),
                notify: Notify::new(),
            }),
        }
    }

    pub fn state(&self) -> ExecutionState {
        let state = self.inner.lock();
        if matches!(state.slot, Slot::Gone) {
            return ExecutionState::Gone;
        }
        if matches!(state.slot, Slot::Parked(_)) {
            return ExecutionState::Parked;
        }
        if matches!(state.slot, Slot::Acquiring) {
            return ExecutionState::Acquiring;
        }
        if state.pending_waits > 0 {
            ExecutionState::Suspending
        } else {
            ExecutionState::Running
        }
    }

    pub fn holds_permit(&self) -> bool {
        matches!(self.inner.lock().slot, Slot::Running(_))
    }

    pub fn admission_generation(&self) -> u64 {
        self.admission_generation
    }

    pub fn active_operations(&self) -> usize {
        self.inner.lock().active_ops
    }

    pub fn pending_waits(&self) -> usize {
        self.inner.lock().pending_waits
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.lock().cancelled
    }

    pub fn cancel(&self) {
        {
            let mut state = self.inner.lock();
            state.cancelled = true;
            if matches!(state.slot, Slot::Parked(_)) {
                if let Slot::Parked(ticket) = std::mem::replace(&mut state.slot, Slot::Gone) {
                    ticket.cancel();
                }
            }
        }
        self.inner.wake();
    }

    pub async fn park(&self) -> Result<(), AdmissionError> {
        self.suspend_for_wait().await.map(|_| ())
    }

    pub fn park_now(&self) -> Result<ParkOutcome, AdmissionError> {
        let outcome = {
            let mut state = self.inner.lock();
            if state.cancelled && !matches!(state.slot, Slot::Gone) {
                return Err(AdmissionError::Cancelled);
            }
            if state.active_ops > 0 {
                return Ok(ParkOutcome::Draining);
            }
            park_in_place(&mut state)
        };
        self.inner.wake();
        outcome
    }

    pub fn begin_wait(&self) -> WaitGuard {
        {
            let mut state = self.inner.lock();
            state.pending_waits += 1;
        }
        self.inner.wake();
        WaitGuard {
            inner: self.inner.clone(),
        }
    }

    pub fn register_operation(&self) -> Option<OperationGuard> {
        let mut state = self.inner.lock();
        if state.cancelled || state.pending_waits > 0 {
            return None;
        }
        if !matches!(state.slot, Slot::Running(_)) {
            return None;
        }
        state.active_ops += 1;
        Some(OperationGuard {
            inner: self.inner.clone(),
        })
    }

    pub async fn begin_tool_batch(
        &self,
        recognized_waits: &[bool],
    ) -> Result<Vec<Option<OperationGuard>>, AdmissionError> {
        let gate = self.begin_operation().await?;
        let mut state = self.inner.lock();
        if state.cancelled {
            return Err(AdmissionError::Cancelled);
        }
        let guards = recognized_waits
            .iter()
            .map(|is_wait| {
                if *is_wait {
                    None
                } else {
                    state.active_ops += 1;
                    Some(OperationGuard {
                        inner: self.inner.clone(),
                    })
                }
            })
            .collect();
        drop(state);
        drop(gate);
        Ok(guards)
    }

    pub async fn begin_operation(&self) -> Result<OperationGuard, AdmissionError> {
        loop {
            let notified = self.inner.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let reacquire = {
                let mut state = self.inner.lock();
                if state.cancelled {
                    return Err(AdmissionError::Cancelled);
                }
                if matches!(state.slot, Slot::Gone) {
                    return Err(AdmissionError::Gone);
                }
                if matches!(state.slot, Slot::Running(_)) && state.pending_waits == 0 {
                    state.active_ops += 1;
                    return Ok(OperationGuard {
                        inner: self.inner.clone(),
                    });
                }
                matches!(state.slot, Slot::Parked(_)) && state.pending_waits == 0
            };
            if reacquire {
                self.reacquire().await?;
                continue;
            }
            notified.await;
        }
    }

    pub async fn suspend_for_wait(&self) -> Result<ParkOutcome, AdmissionError> {
        let _request = self.begin_wait();
        loop {
            let notified = self.inner.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut state = self.inner.lock();
                if matches!(state.slot, Slot::Gone) {
                    return Ok(ParkOutcome::NotHeld);
                }
                if state.cancelled {
                    return Err(AdmissionError::Cancelled);
                }
                if matches!(state.slot, Slot::Parked(_)) {
                    return Ok(ParkOutcome::AlreadyParked);
                }
                if matches!(state.slot, Slot::Running(_)) {
                    if state.active_ops == 0 {
                        let outcome = park_in_place(&mut state);
                        drop(state);
                        self.inner.wake();
                        return outcome;
                    }
                }
            }
            notified.await;
        }
    }

    pub async fn resume(&self) -> Result<(), AdmissionError> {
        self.reacquire().await
    }

    pub async fn wait_until_running(&self) -> Result<(), AdmissionError> {
        loop {
            let notified = self.inner.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let state = self.inner.lock();
                if state.cancelled {
                    return Err(AdmissionError::Cancelled);
                }
                if matches!(state.slot, Slot::Gone) {
                    return Err(AdmissionError::Gone);
                }
                if matches!(state.slot, Slot::Running(_)) {
                    return Ok(());
                }
            }
            notified.await;
        }
    }

    async fn reacquire(&self) -> Result<(), AdmissionError> {
        loop {
            let notified = self.inner.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let ticket = {
                let mut state = self.inner.lock();
                if state.cancelled && !matches!(state.slot, Slot::Gone) {
                    return Err(AdmissionError::Cancelled);
                }
                if matches!(state.slot, Slot::Running(_)) {
                    return Ok(());
                }
                if matches!(state.slot, Slot::Gone) {
                    return Err(AdmissionError::Gone);
                }
                if matches!(state.slot, Slot::Parked(_)) {
                    if state.cancelled {
                        return Err(AdmissionError::Cancelled);
                    }
                    match std::mem::replace(&mut state.slot, Slot::Acquiring) {
                        Slot::Parked(ticket) => Some(ticket),
                        other => {
                            state.slot = other;
                            None
                        }
                    }
                } else {
                    None
                }
            };
            let ticket = match ticket {
                Some(ticket) => ticket,
                None => {
                    notified.await;
                    continue;
                }
            };
            let mut guard = AcquireGuard {
                inner: self.inner.clone(),
                armed: true,
            };
            let acquired = {
                let acquisition = ticket.acquire();
                tokio::pin!(acquisition);
                loop {
                    let cancelled = self.inner.notify.notified();
                    tokio::pin!(cancelled);
                    cancelled.as_mut().enable();
                    if self.inner.lock().cancelled {
                        break Err(AdmissionError::Cancelled);
                    }
                    tokio::select! {
                        result = &mut acquisition => break result,
                        _ = cancelled => {}
                    }
                }
            };
            guard.armed = false;
            let result = {
                let mut state = self.inner.lock();
                match acquired {
                    Ok(permit) => {
                        if state.cancelled {
                            permit.release();
                            state.slot = Slot::Gone;
                            Err(AdmissionError::Cancelled)
                        } else {
                            state.slot = Slot::Running(permit);
                            Ok(())
                        }
                    }
                    Err(err) => {
                        state.slot = Slot::Gone;
                        Err(err)
                    }
                }
            };
            self.inner.wake();
            return result;
        }
    }
}

#[derive(Default)]
pub struct ExecutionRegistry {
    runs: Mutex<HashMap<String, Arc<RunExecution>>>,
    tasks: Mutex<ExecutionTasks>,
    configs: Mutex<HashMap<String, crate::subchat::SubchatConfig>>,
    scopes: Mutex<HashMap<String, std::path::PathBuf>>,
    launch_provenance: Mutex<HashMap<String, (Vec<refact_privacy::FileRecord>, bool)>>,
    resume_intents: Mutex<std::collections::HashSet<String>>,
    pub(crate) board_admission_closed: std::sync::atomic::AtomicBool,
}

#[derive(Default)]
struct ExecutionTasks {
    closed: bool,
    entries: HashMap<String, Arc<ExecutionTask>>,
}

struct ExecutionTask {
    handle: tokio::sync::Mutex<Option<JoinHandle<()>>>,
    lifetime: tokio::task::AbortHandle,
    cancel: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl ExecutionTask {
    fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
        self.notify.notify_waiters();
    }

    async fn join(&self) {
        let mut handle = self.handle.lock().await;
        if let Some(task) = handle.as_mut() {
            let _ = task.await;
        }
        handle.take();
    }
}

pub struct ExecutionRegistration {
    registry: Arc<ExecutionRegistry>,
    chat_id: String,
    execution: Option<Arc<RunExecution>>,
}

pub struct ResumeIntent {
    registry: Arc<ExecutionRegistry>,
    agent_id: String,
}

impl Drop for ResumeIntent {
    fn drop(&mut self) {
        self.registry
            .resume_intents
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.agent_id);
    }
}

impl Drop for ExecutionRegistration {
    fn drop(&mut self) {
        let mut runs = self.registry.runs.lock().unwrap_or_else(|e| e.into_inner());
        if self
            .execution
            .as_ref()
            .zip(runs.get(&self.chat_id))
            .is_some_and(|(owned, current)| Arc::ptr_eq(owned, current))
        {
            runs.remove(&self.chat_id);
        }
    }
}

impl ExecutionRegistry {
    pub fn reserve_resume(self: &Arc<Self>, agent_id: &str) -> Result<ResumeIntent, String> {
        let mut intents = self
            .resume_intents
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if intents.contains(agent_id) {
            return Err("resume already being prepared for this child".into());
        }
        if intents.len() >= crate::file_activity::settings().max_outstanding {
            return Err("resume preparation capacity unavailable".into());
        }
        if self
            .board_admission_closed
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("execution registry is shutting down".into());
        }
        intents.insert(agent_id.into());
        Ok(ResumeIntent {
            registry: self.clone(),
            agent_id: agent_id.into(),
        })
    }

    pub fn remember_launch_provenance(
        &self,
        agent_id: &str,
        provenance: (Vec<refact_privacy::FileRecord>, bool),
    ) {
        let settings = crate::file_activity::settings();
        let mut provenance = provenance;
        let mut seen = std::collections::HashSet::new();
        provenance.0.retain(|record| seen.insert(record.clone()));
        if !provenance.1
            || provenance.0.len() > settings.max_work_provenance_records
            || provenance.0.iter().fold(0usize, |bytes, record| {
                bytes
                    .saturating_add(record.path.len())
                    .saturating_add(record.zone.len())
            }) > settings.max_work_provenance_bytes
        {
            provenance = (vec![], false);
        }
        let mut records = self
            .launch_provenance
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if records.len() < settings.max_outstanding {
            records.insert(agent_id.into(), provenance);
        }
    }

    pub fn launch_provenance(
        &self,
        agent_id: &str,
    ) -> Option<(Vec<refact_privacy::FileRecord>, bool)> {
        let settings = crate::file_activity::settings();
        let records = self
            .launch_provenance
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        records.get(agent_id).map(|(records, complete)| {
            if *complete
                && records.len() <= settings.max_work_provenance_records
                && records.iter().fold(0usize, |bytes, record| {
                    bytes
                        .saturating_add(record.path.len())
                        .saturating_add(record.zone.len())
                }) <= settings.max_work_provenance_bytes
            {
                (records.clone(), true)
            } else {
                (vec![], false)
            }
        })
    }

    pub fn forget_launch_provenance(&self, agent_id: &str) {
        self.launch_provenance
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(agent_id);
    }

    pub fn remember_scope(&self, agent_id: &str, root: std::path::PathBuf) {
        self.scopes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(agent_id.into(), root);
    }

    pub fn saved_scope(&self, agent_id: &str) -> Option<std::path::PathBuf> {
        self.scopes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(agent_id)
            .cloned()
    }

    pub fn registration(self: &Arc<Self>, chat_id: &str) -> ExecutionRegistration {
        ExecutionRegistration {
            registry: self.clone(),
            chat_id: chat_id.into(),
            execution: self.get(chat_id),
        }
    }

    pub fn remember_config(&self, agent_id: &str, config: crate::subchat::SubchatConfig) {
        self.configs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(agent_id.to_string(), config);
    }

    pub fn saved_config(&self, agent_id: &str) -> Option<crate::subchat::SubchatConfig> {
        self.configs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(agent_id)
            .cloned()
    }

    pub fn forget_config(&self, agent_id: &str) {
        self.configs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(agent_id);
    }

    pub fn install(&self, chat_id: &str, permit: Permit) -> Result<Arc<RunExecution>, String> {
        let tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        if tasks.closed || self.board_admission_closed.load(Ordering::Acquire) {
            return Err("execution registry is shutting down".into());
        }
        let mut runs = self.runs.lock().unwrap_or_else(|e| e.into_inner());
        if runs.contains_key(chat_id) {
            return Err("child execution is already registered".into());
        }
        let execution = Arc::new(RunExecution::new(permit));
        runs.insert(chat_id.to_string(), execution.clone());
        Ok(execution)
    }

    pub fn get(&self, chat_id: &str) -> Option<Arc<RunExecution>> {
        self.runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(chat_id)
            .cloned()
    }

    pub fn finish(&self, chat_id: &str) {
        self.runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(chat_id);
    }

    pub fn spawn_owned<F>(
        &self,
        task_id: String,
        cancel: Arc<AtomicBool>,
        notify: Arc<Notify>,
        future: F,
    ) -> Result<(), String>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        if tasks.closed || self.board_admission_closed.load(Ordering::Acquire) {
            return Err("execution registry is shutting down".into());
        }
        tasks.entries.retain(|_, task| !task.lifetime.is_finished());
        if tasks.entries.contains_key(&task_id) {
            return Err("child task is still finishing".into());
        }
        let (registered, ready) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            if ready.await.is_ok() {
                future.await;
            }
        });
        let lifetime = handle.abort_handle();
        tasks.entries.insert(
            task_id,
            Arc::new(ExecutionTask {
                handle: tokio::sync::Mutex::new(Some(handle)),
                lifetime,
                cancel,
                notify,
            }),
        );
        let _ = registered.send(());
        Ok(())
    }

    pub fn task_active(&self, task_id: &str) -> bool {
        self.tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .get(task_id)
            .is_some_and(|task| !task.lifetime.is_finished())
    }

    pub fn begin_shutdown(&self) {
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        tasks.closed = true;
        self.board_admission_closed.store(true, Ordering::Release);
        for task in tasks.entries.values() {
            task.cancel();
        }
        for execution in self.runs.lock().unwrap_or_else(|e| e.into_inner()).values() {
            execution.cancel();
        }
    }

    pub async fn shutdown(&self) {
        self.begin_shutdown();
        let tasks: Vec<_> = self
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .iter()
            .map(|(id, task)| (id.clone(), task.clone()))
            .collect();
        for (id, task) in tasks {
            task.join().await;
            let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
            if tasks
                .entries
                .get(&id)
                .is_some_and(|current| Arc::ptr_eq(current, &task))
            {
                tasks.entries.remove(&id);
            }
        }
        self.runs.lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.configs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.scopes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.launch_provenance
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    #[cfg(test)]
    pub(crate) async fn interrupt_tasks_for_test(&self) {
        self.begin_shutdown();
        for task in self
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .values()
        {
            task.lifetime.abort();
        }
        self.shutdown().await;
    }
}

impl Drop for ExecutionRegistry {
    fn drop(&mut self) {
        for task in self
            .tasks
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .entries
            .values()
        {
            task.cancel();
            task.lifetime.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_work_core::{
        AdmissionRequest, AdmissionScheduler, CoordinationSettings, RunKey, TicketState, WorkKey,
        WorkScope,
    };
    use std::future::Future;
    use std::pin::Pin;

    fn scheduler(active: usize) -> AdmissionScheduler {
        AdmissionScheduler::new(CoordinationSettings {
            max_active_engine: active,
            max_active_project: active,
            max_active_root: active,
            ..CoordinationSettings::default()
        })
    }

    fn request(id: &str) -> AdmissionRequest {
        AdmissionRequest::new(
            RunKey::new(WorkKey::background_agent(id), 1),
            WorkScope::single("project"),
        )
    }

    async fn permit_for(sched: &AdmissionScheduler, id: &str) -> Permit {
        sched.reserve(request(id)).unwrap().acquire().await.unwrap()
    }

    async fn stays_pending<F: Future>(mut fut: Pin<&mut F>) {
        std::future::poll_fn(|cx| {
            assert!(fut.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
    }

    async fn completes_now<F: Future>(mut fut: Pin<&mut F>) -> F::Output {
        std::future::poll_fn(|cx| match fut.as_mut().poll(cx) {
            std::task::Poll::Ready(value) => std::task::Poll::Ready(value),
            std::task::Poll::Pending => {
                panic!("future remained blocked after the barrier released")
            }
        })
        .await
    }

    fn park_count(sched: &AdmissionScheduler) -> u32 {
        sched
            .snapshot()
            .tickets
            .iter()
            .map(|t| t.park_count)
            .max()
            .unwrap_or(0)
    }

    #[tokio::test]
    async fn park_frees_the_slot_and_resume_takes_it_back() {
        let sched = scheduler(1);
        let execution = RunExecution::new(permit_for(&sched, "run").await);
        assert_eq!(execution.state(), ExecutionState::Running);
        assert_eq!(sched.snapshot().active, 1);

        execution.park().await.unwrap();
        assert_eq!(execution.state(), ExecutionState::Parked);
        assert!(!execution.holds_permit());
        assert_eq!(sched.snapshot().active, 0);
        assert_eq!(sched.snapshot().parked, 1);

        execution.resume().await.unwrap();
        assert_eq!(execution.state(), ExecutionState::Running);
        assert!(execution.holds_permit());
        assert_eq!(sched.snapshot().active, 1);
        assert_eq!(park_count(&sched), 1);
    }

    #[tokio::test]
    async fn registration_drop_releases_only_its_owned_attempt() {
        let sched = scheduler(2);
        let registry = Arc::new(ExecutionRegistry::default());
        registry
            .install("child", permit_for(&sched, "old").await)
            .unwrap();
        let old = registry.registration("child");
        registry.finish("child");
        registry
            .install("child", permit_for(&sched, "new").await)
            .unwrap();
        let new = registry.registration("child");
        drop(old);
        assert!(registry.get("child").is_some());
        drop(new);
        assert!(registry.get("child").is_none());
        assert_eq!(sched.snapshot().active, 0);
    }

    #[tokio::test]
    async fn park_is_idempotent() {
        let sched = scheduler(1);
        let execution = RunExecution::new(permit_for(&sched, "run").await);
        assert_eq!(execution.park_now().unwrap(), ParkOutcome::Parked);
        assert_eq!(execution.park_now().unwrap(), ParkOutcome::AlreadyParked);
        execution.park().await.unwrap();
        assert_eq!(sched.snapshot().parked, 1);
        assert_eq!(park_count(&sched), 1);
    }

    #[tokio::test]
    async fn dropped_reacquisition_never_reports_running() {
        let sched = scheduler(1);
        let execution = RunExecution::new(permit_for(&sched, "run").await);
        execution.park().await.unwrap();
        let _blocker = permit_for(&sched, "blocker").await;

        let mut resume = Box::pin(execution.resume());
        stays_pending(resume.as_mut()).await;
        assert_eq!(execution.state(), ExecutionState::Acquiring);
        drop(resume);

        assert_eq!(execution.state(), ExecutionState::Gone);
        assert!(!execution.holds_permit());
        assert!(matches!(
            execution.resume().await,
            Err(AdmissionError::Gone)
        ));
        assert!(matches!(
            execution.begin_operation().await,
            Err(AdmissionError::Gone)
        ));
        assert!(execution.register_operation().is_none());
    }

    #[tokio::test]
    async fn cancelled_reacquisition_reports_error_not_running() {
        let sched = scheduler(1);
        let execution = RunExecution::new(permit_for(&sched, "run").await);
        execution.park().await.unwrap();
        let _blocker = permit_for(&sched, "blocker").await;

        let mut resume = Box::pin(execution.resume());
        stays_pending(resume.as_mut()).await;
        assert!(sched.cancel(&RunKey::new(WorkKey::background_agent("run"), 1)));

        assert!(matches!(resume.await, Err(AdmissionError::Cancelled)));
        assert_eq!(execution.state(), ExecutionState::Gone);
        assert!(!execution.holds_permit());
    }

    #[tokio::test]
    async fn cancel_while_parked_makes_resume_fail() {
        let sched = scheduler(2);
        let execution = RunExecution::new(permit_for(&sched, "run").await);
        execution.park().await.unwrap();
        execution.cancel();
        assert_eq!(execution.state(), ExecutionState::Gone);
        assert!(matches!(
            execution.resume().await,
            Err(AdmissionError::Gone)
        ));
        assert_eq!(sched.snapshot().outstanding, 0);
    }

    #[tokio::test]
    async fn cancel_while_running_preserves_permit_and_refuses_to_park() {
        let sched = scheduler(2);
        let execution = RunExecution::new(permit_for(&sched, "run").await);
        execution.cancel();
        assert!(execution.holds_permit());
        assert!(matches!(
            execution.suspend_for_wait().await,
            Err(AdmissionError::Cancelled)
        ));
        assert!(matches!(
            execution.park_now(),
            Err(AdmissionError::Cancelled)
        ));
        assert!(execution.holds_permit());
        assert_eq!(sched.snapshot().active, 1);
    }

    #[tokio::test]
    async fn suspend_waits_for_non_wait_operations_to_drain() {
        let sched = scheduler(2);
        let execution = RunExecution::new(permit_for(&sched, "run").await);
        let operation = execution.register_operation().unwrap();
        assert_eq!(execution.active_operations(), 1);

        let mut suspend = Box::pin(execution.suspend_for_wait());
        stays_pending(suspend.as_mut()).await;
        assert_eq!(execution.state(), ExecutionState::Suspending);
        assert!(execution.holds_permit());
        assert_eq!(sched.snapshot().active, 1);

        drop(operation);
        assert_eq!(suspend.await.unwrap(), ParkOutcome::Parked);
        assert_eq!(execution.state(), ExecutionState::Parked);
        assert_eq!(sched.snapshot().active, 0);
    }

    #[tokio::test]
    async fn pending_wait_blocks_fresh_operations() {
        let sched = scheduler(2);
        let execution = RunExecution::new(permit_for(&sched, "run").await);
        let wait = execution.begin_wait();
        assert!(execution.register_operation().is_none());
        assert_eq!(
            execution.suspend_for_wait().await.unwrap(),
            ParkOutcome::Parked
        );

        let mut gated = Box::pin(execution.begin_operation());
        stays_pending(gated.as_mut()).await;
        assert_eq!(execution.state(), ExecutionState::Parked);

        drop(wait);
        let operation = gated.await.unwrap();
        assert_eq!(execution.state(), ExecutionState::Running);
        assert_eq!(execution.active_operations(), 1);
        assert_eq!(sched.snapshot().active, 1);
        assert_eq!(park_count(&sched), 1);
        drop(operation);
        assert_eq!(execution.active_operations(), 0);
    }

    #[tokio::test]
    async fn concurrent_resumes_reacquire_once() {
        let sched = scheduler(1);
        let execution = RunExecution::new(permit_for(&sched, "run").await);
        execution.park().await.unwrap();
        let blocker = permit_for(&sched, "blocker").await;

        let mut first = Box::pin(execution.resume());
        let mut second = Box::pin(execution.resume());
        stays_pending(first.as_mut()).await;
        stays_pending(second.as_mut()).await;
        assert_eq!(sched.snapshot().pending, 1);
        blocker.release();
        completes_now(first.as_mut()).await.unwrap();
        completes_now(second.as_mut()).await.unwrap();

        assert_eq!(execution.state(), ExecutionState::Running);
        assert!(execution.holds_permit());
        assert_eq!(sched.snapshot().active, 1);
        assert_eq!(park_count(&sched), 1);
    }

    #[tokio::test]
    async fn concurrent_park_and_resume_never_double_charge() {
        let sched = scheduler(1);
        let execution = RunExecution::new(permit_for(&sched, "run").await);
        execution.park().await.unwrap();
        let blocker = permit_for(&sched, "blocker").await;

        let mut resume = Box::pin(execution.resume());
        stays_pending(resume.as_mut()).await;
        assert_eq!(execution.park_now().unwrap(), ParkOutcome::Acquiring);
        let mut park = Box::pin(execution.park());
        stays_pending(park.as_mut()).await;
        blocker.release();
        completes_now(resume.as_mut()).await.unwrap();
        completes_now(park.as_mut()).await.unwrap();
        assert_eq!(sched.snapshot().active, 0);

        execution.resume().await.unwrap();
        assert!(execution.holds_permit());
        assert_eq!(sched.snapshot().active, 1);
        assert_eq!(sched.snapshot().outstanding, 1);
    }

    #[tokio::test]
    async fn cancel_during_acquire_refuses_park_and_resume() {
        let sched = scheduler(1);
        let execution = RunExecution::new(permit_for(&sched, "run").await);
        execution.park().await.unwrap();
        let blocker = permit_for(&sched, "blocker").await;

        let mut resume = Box::pin(execution.resume());
        stays_pending(resume.as_mut()).await;
        assert_eq!(execution.state(), ExecutionState::Acquiring);

        execution.cancel();
        assert!(matches!(
            execution.park_now(),
            Err(AdmissionError::Cancelled)
        ));
        assert!(!execution.holds_permit());

        blocker.release();
        assert!(matches!(resume.await, Err(AdmissionError::Cancelled)));
        assert_eq!(execution.state(), ExecutionState::Gone);
        assert!(!execution.holds_permit());
        assert_eq!(sched.snapshot().active, 0);
    }

    #[tokio::test]
    async fn concurrent_waits_park_once_and_resume_once() {
        let sched = scheduler(2);
        let execution = RunExecution::new(permit_for(&sched, "run").await);
        let (first, second) =
            tokio::join!(execution.suspend_for_wait(), execution.suspend_for_wait());
        let outcomes = [first.unwrap(), second.unwrap()];
        assert!(outcomes.contains(&ParkOutcome::Parked));
        assert!(outcomes.contains(&ParkOutcome::AlreadyParked));
        assert_eq!(execution.pending_waits(), 0);

        execution.resume().await.unwrap();
        assert_eq!(park_count(&sched), 1);
        assert_eq!(sched.snapshot().active, 1);
    }

    #[tokio::test]
    async fn parked_execution_reports_ticket_state_parked() {
        let sched = scheduler(2);
        let execution = RunExecution::new(permit_for(&sched, "run").await);
        execution.park().await.unwrap();
        let states: Vec<TicketState> = sched.snapshot().tickets.iter().map(|t| t.state).collect();
        assert_eq!(states, vec![TicketState::Parked]);
    }

    #[tokio::test]
    async fn wait_until_running_unblocks_after_resume() {
        let sched = scheduler(2);
        let execution = Arc::new(RunExecution::new(permit_for(&sched, "run").await));
        execution.park().await.unwrap();

        let observer = execution.clone();
        let mut waiting = Box::pin(observer.wait_until_running());
        stays_pending(waiting.as_mut()).await;
        execution.resume().await.unwrap();
        waiting.await.unwrap();
        assert!(execution.holds_permit());
    }

    #[tokio::test]
    async fn registry_install_get_and_finish() {
        let sched = scheduler(2);
        let registry = ExecutionRegistry::default();
        let execution = registry
            .install("chat", permit_for(&sched, "run").await)
            .unwrap();
        assert!(registry.get("chat").is_some());
        assert!(execution.holds_permit());
        registry.finish("chat");
        assert!(registry.get("chat").is_none());
        drop(execution);
        assert_eq!(sched.snapshot().active, 0);
    }

    #[tokio::test]
    async fn duplicate_execution_never_replaces_or_releases_the_owner() {
        let sched = scheduler(2);
        let registry = ExecutionRegistry::default();
        let execution = registry
            .install("child", permit_for(&sched, "first").await)
            .unwrap();
        assert!(registry
            .install("child", permit_for(&sched, "second").await)
            .is_err());
        assert!(Arc::ptr_eq(&registry.get("child").unwrap(), &execution));
        assert_eq!(sched.snapshot().active, 1);
        registry.finish("child");
        drop(execution);
        assert_eq!(sched.snapshot().outstanding, 0);
    }

    #[tokio::test]
    async fn owned_registration_refuses_duplicate_and_late_work_without_polling_it() {
        let registry = Arc::new(ExecutionRegistry::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let notify = Arc::new(Notify::new());
        let (release, wait) = tokio::sync::oneshot::channel();
        let (started, ready) = tokio::sync::oneshot::channel();
        let seen = registry.clone();
        registry
            .spawn_owned("child".into(), cancel.clone(), notify.clone(), async move {
                assert!(seen.task_active("child"));
                let _ = started.send(());
                let _ = wait.await;
            })
            .unwrap();
        ready.await.unwrap();
        assert!(registry
            .spawn_owned("child".into(), cancel.clone(), notify.clone(), async {
                panic!("duplicate task was polled");
            })
            .is_err());
        registry.begin_shutdown();
        assert!(cancel.load(Ordering::Acquire));
        assert!(registry
            .spawn_owned("late".into(), cancel, notify, async {
                panic!("post-shutdown task was polled");
            })
            .is_err());
        let sched = scheduler(1);
        assert!(registry
            .install("late", permit_for(&sched, "late").await)
            .is_err());
        assert_eq!(sched.snapshot().outstanding, 0);
        let mut shutdown = Box::pin(registry.shutdown());
        stays_pending(shutdown.as_mut()).await;
        release.send(()).unwrap();
        shutdown.await;
        assert!(!registry.task_active("child"));
    }

    #[tokio::test]
    async fn dropped_shutdown_waiter_retains_tasks_and_waits_for_all_cleanup() {
        let registry = Arc::new(ExecutionRegistry::default());
        let sched = scheduler(2);
        let mut releases = Vec::new();
        let mut cancellations = Vec::new();
        for id in ["one", "two"] {
            registry.install(id, permit_for(&sched, id).await).unwrap();
            let owner = registry.registration(id);
            let cancel = Arc::new(AtomicBool::new(false));
            cancellations.push(cancel.clone());
            let notify = Arc::new(Notify::new());
            let (release, wait) = tokio::sync::oneshot::channel();
            releases.push(release);
            registry
                .spawn_owned(id.into(), cancel, notify, async move {
                    let _owner = owner;
                    let _ = wait.await;
                })
                .unwrap();
        }
        let mut first = Box::pin(registry.shutdown());
        stays_pending(first.as_mut()).await;
        assert!(cancellations
            .iter()
            .all(|flag| flag.load(Ordering::Acquire)));
        drop(first);
        assert!(registry.task_active("one"));
        assert!(registry.task_active("two"));
        assert_eq!(sched.snapshot().active, 2);
        let mut second = Box::pin(registry.shutdown());
        let mut concurrent = Box::pin(registry.shutdown());
        stays_pending(second.as_mut()).await;
        stays_pending(concurrent.as_mut()).await;
        for release in releases {
            release.send(()).unwrap();
        }
        tokio::join!(second, concurrent);
        assert_eq!(sched.snapshot().outstanding, 0);
        assert!(!registry.task_active("one"));
        assert!(!registry.task_active("two"));
    }

    #[tokio::test]
    async fn registration_racing_shutdown_has_no_unowned_task() {
        for _ in 0..32 {
            let registry = Arc::new(ExecutionRegistry::default());
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let closing_registry = registry.clone();
            let closing_barrier = barrier.clone();
            let closer = std::thread::spawn(move || {
                closing_barrier.wait();
                closing_registry.begin_shutdown();
            });
            let cancel = Arc::new(AtomicBool::new(false));
            let notify = Arc::new(Notify::new());
            let flag = cancel.clone();
            let signal = notify.clone();
            let finished = Arc::new(AtomicBool::new(false));
            let done = finished.clone();
            barrier.wait();
            let accepted = registry
                .spawn_owned("racing".into(), cancel, notify, async move {
                    loop {
                        let notified = signal.notified();
                        tokio::pin!(notified);
                        notified.as_mut().enable();
                        if flag.load(Ordering::Acquire) {
                            break;
                        }
                        notified.await;
                    }
                    done.store(true, Ordering::Release);
                })
                .is_ok();
            closer.join().unwrap();
            registry.shutdown().await;
            assert_eq!(finished.load(Ordering::Acquire), accepted);
            assert!(!registry.task_active("racing"));
        }
    }

    #[tokio::test]
    async fn operations_running_during_park_keep_the_permit() {
        let sched = scheduler(2);
        let execution = RunExecution::new(permit_for(&sched, "run").await);
        let first = execution.register_operation().unwrap();
        let second = execution.register_operation().unwrap();
        assert_eq!(execution.active_operations(), 2);

        let mut suspend = Box::pin(execution.suspend_for_wait());
        stays_pending(suspend.as_mut()).await;
        drop(first);
        stays_pending(suspend.as_mut()).await;
        assert!(execution.holds_permit());
        drop(second);
        assert_eq!(suspend.await.unwrap(), ParkOutcome::Parked);
        assert!(!execution.holds_permit());
    }

    #[tokio::test]
    async fn batch_registers_unpolled_operations_before_wait_can_park() {
        let sched = scheduler(1);
        let execution = RunExecution::new(permit_for(&sched, "run").await);
        let mut guards = execution
            .begin_tool_batch(&[true, false, false])
            .await
            .unwrap();
        assert!(guards[0].is_none());
        assert_eq!(execution.active_operations(), 2);
        assert_eq!(execution.park_now().unwrap(), ParkOutcome::Draining);

        let mut wait = Box::pin(execution.park());
        stays_pending(wait.as_mut()).await;
        assert!(execution.register_operation().is_none());
        drop(guards[1].take());
        stays_pending(wait.as_mut()).await;
        assert!(execution.holds_permit());
        drop(guards[2].take());
        completes_now(wait.as_mut()).await.unwrap();
        assert_eq!(sched.snapshot().active, 0);
        assert_eq!(park_count(&sched), 1);
    }

    #[tokio::test]
    async fn recognized_wait_stays_pending_while_fresh_batch_reacquires() {
        let sched = scheduler(1);
        let execution = RunExecution::new(permit_for(&sched, "run").await);
        let guards = execution.begin_tool_batch(&[true, true]).await.unwrap();
        assert!(guards.iter().all(Option::is_none));
        let (ready, waiting) = tokio::sync::oneshot::channel::<()>();
        let mut wait = Box::pin(async {
            execution.suspend_for_wait().await.unwrap();
            waiting.await.unwrap();
        });
        stays_pending(wait.as_mut()).await;
        assert_eq!(sched.snapshot().parked, 1);
        let blocker = permit_for(&sched, "blocker").await;
        let mut batch = Box::pin(execution.begin_tool_batch(&[false]));
        let mut resume = Box::pin(execution.resume());
        stays_pending(batch.as_mut()).await;
        stays_pending(resume.as_mut()).await;
        assert_eq!(sched.snapshot().pending, 1);
        assert_eq!(execution.active_operations(), 0);
        blocker.release();
        let operations = completes_now(batch.as_mut()).await.unwrap();
        completes_now(resume.as_mut()).await.unwrap();
        stays_pending(wait.as_mut()).await;
        assert!(execution.holds_permit());
        assert_eq!(execution.active_operations(), 1);
        assert_eq!(sched.snapshot().active, 1);
        ready.send(()).unwrap();
        completes_now(wait.as_mut()).await;
        drop(operations);
        assert_eq!(execution.active_operations(), 0);
    }

    #[tokio::test]
    async fn local_cancel_interrupts_acquisition_without_freeing_blocker() {
        let sched = scheduler(1);
        let execution = RunExecution::new(permit_for(&sched, "run").await);
        execution.park().await.unwrap();
        let _blocker = permit_for(&sched, "blocker").await;
        let mut resume = Box::pin(execution.resume());
        stays_pending(resume.as_mut()).await;
        execution.cancel();
        assert_eq!(
            completes_now(resume.as_mut()).await,
            Err(AdmissionError::Cancelled)
        );
        assert_eq!(execution.state(), ExecutionState::Gone);
        assert_eq!(sched.snapshot().outstanding, 1);
        assert_eq!(sched.snapshot().active, 1);
        assert_eq!(sched.snapshot().pending, 0);
        assert!(execution.resume().await.is_err());
    }

    #[tokio::test]
    async fn dropping_granted_acquisition_wakes_followers_with_error() {
        let sched = scheduler(1);
        let execution = RunExecution::new(permit_for(&sched, "run").await);
        execution.park().await.unwrap();
        let blocker = permit_for(&sched, "blocker").await;
        let mut first = Box::pin(execution.resume());
        let mut second = Box::pin(execution.resume());
        stays_pending(first.as_mut()).await;
        stays_pending(second.as_mut()).await;
        blocker.release();
        assert_eq!(sched.snapshot().active, 1);
        drop(first);
        assert_eq!(
            completes_now(second.as_mut()).await,
            Err(AdmissionError::Gone)
        );
        assert_eq!(execution.state(), ExecutionState::Gone);
        assert_eq!(sched.snapshot().outstanding, 0);
        assert!(!execution.holds_permit());
    }

    #[tokio::test]
    async fn cancelled_running_execution_refuses_successful_resume_and_batches() {
        let sched = scheduler(1);
        let execution = RunExecution::new(permit_for(&sched, "run").await);
        execution.cancel();
        assert_eq!(execution.resume().await, Err(AdmissionError::Cancelled));
        assert!(matches!(
            execution.begin_tool_batch(&[false]).await,
            Err(AdmissionError::Cancelled)
        ));
        assert_eq!(execution.active_operations(), 0);
        assert!(execution.holds_permit());
    }

    #[tokio::test]
    async fn dropping_suspension_releases_gate_without_releasing_running_permit() {
        let sched = scheduler(1);
        let execution = RunExecution::new(permit_for(&sched, "run").await);
        let operation = execution.register_operation().unwrap();
        let mut park = Box::pin(execution.park());
        stays_pending(park.as_mut()).await;
        assert_eq!(execution.state(), ExecutionState::Suspending);
        drop(park);
        assert_eq!(execution.state(), ExecutionState::Running);
        assert_eq!(execution.pending_waits(), 0);
        drop(operation);
        let batch = execution.begin_tool_batch(&[false, true]).await.unwrap();
        assert_eq!(execution.active_operations(), 1);
        drop(batch);
        assert_eq!(execution.active_operations(), 0);
        assert_eq!(sched.snapshot().active, 1);
    }
}
