//! Board-agent adapter over the shared `refact_work_core::AdmissionScheduler`.
//! Keys tickets/permits as `RuntimeKind::Trajectory` + `WorkKey::trajectory(agent_chat_id)`,
//! admitted through `app.work_scheduler` — the same scheduler chat agents use.
//! `WorkScope.project` is the planner's owning project; `WorkScope.root` is the
//! owning planner/root chat, shared with its chat-runtime descendants, not the
//! per-card worktree (which doesn't exist yet at admission time).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Mutex as AMutex;
use tokio::task::JoinHandle;

use refact_work_core::{
    AdmissionError, AdmissionRequest, CheckoutKey, ProjectKey, RunKey, Ticket, WorkKey, WorkScope,
};

use crate::app_state::AppState;

enum Phase {
    Queued(Ticket),
    Acquiring,
    Active(Arc<crate::agents::execution::RunExecution>),
}

struct Entry {
    phase: Phase,
    attempt: Arc<()>,
    scheduler: Arc<refact_work_core::AdmissionScheduler>,
    executions: Arc<crate::agents::execution::ExecutionRegistry>,
}

#[derive(Default)]
struct BoardAdmission {
    // No application locks or awaits inside this short critical section. A
    // synchronous lock lets dropped acquisition futures clean up without
    // spawning an unowned cleanup task from Drop.
    holders: Mutex<HashMap<String, Entry>>,
    tasks: Mutex<HashMap<String, Arc<SetupTask>>>,
}

struct SetupTask {
    handle: AMutex<JoinHandle<()>>,
    finished: Arc<AtomicBool>,
    executions: Arc<crate::agents::execution::ExecutionRegistry>,
    attempt: Arc<()>,
}

struct MarkSetupFinished(Arc<AtomicBool>);

impl Drop for MarkSetupFinished {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

impl BoardAdmission {
    fn holders(&self) -> MutexGuard<'_, HashMap<String, Entry>> {
        self.holders.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn tasks(&self) -> MutexGuard<'_, HashMap<String, Arc<SetupTask>>> {
        self.tasks.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn global() -> &'static BoardAdmission {
    static SLOT: OnceLock<BoardAdmission> = OnceLock::new();
    SLOT.get_or_init(BoardAdmission::default)
}

struct AcquisitionRegistration {
    agent_chat_id: String,
    attempt: Arc<()>,
}

impl Drop for AcquisitionRegistration {
    fn drop(&mut self) {
        let mut holders = global().holders();
        if holders.get(&self.agent_chat_id).is_some_and(|entry| {
            Arc::ptr_eq(&entry.attempt, &self.attempt) && matches!(entry.phase, Phase::Acquiring)
        }) {
            // Ticket::acquire owns the scheduler guard; dropping that future
            // releases its reservation/permit. Remove only our adapter entry.
            holders.remove(&self.agent_chat_id);
        }
    }
}

pub fn board_scope(project_identity: &str, root_chat_id: &str) -> WorkScope {
    WorkScope::new(
        ProjectKey::new(project_identity.to_string()),
        CheckoutKey::new(root_chat_id.to_string()),
    )
}

fn run_key(agent_chat_id: &str) -> RunKey {
    RunKey::new(WorkKey::trajectory(agent_chat_id.to_string()), 1)
}

/// Reserve a ticket before any worktree side effect or board claim. Never
/// blocks on capacity; the caller must still call [`wait_for_permit`] before
/// touching the filesystem.
pub async fn reserve(
    app: &AppState,
    agent_chat_id: &str,
    project_identity: &str,
    root_chat_id: &str,
    depth: usize,
) -> Result<(), AdmissionError> {
    let request = AdmissionRequest::new(
        run_key(agent_chat_id),
        board_scope(project_identity, root_chat_id),
    )
    .with_depth(depth);
    let mut holders = global().holders();
    if app
        .agent_executions
        .board_admission_closed
        .load(Ordering::Acquire)
    {
        return Err(AdmissionError::Disabled);
    }
    // Do not reuse a name while old setup/rollback still owns filesystem work,
    // even if that setup has already removed its admission holder.
    if holders.contains_key(agent_chat_id)
        || global()
            .tasks()
            .get(agent_chat_id)
            .is_some_and(|task| !task.finished.load(Ordering::Acquire))
    {
        return Err(AdmissionError::Duplicate {
            run: run_key(agent_chat_id),
        });
    }
    let ticket = app.work_scheduler.reserve(request)?;
    holders.insert(
        agent_chat_id.to_string(),
        Entry {
            phase: Phase::Queued(ticket),
            attempt: Arc::new(()),
            scheduler: app.work_scheduler.clone(),
            executions: app.agent_executions.clone(),
        },
    );
    Ok(())
}

pub async fn is_tracked(agent_chat_id: &str) -> bool {
    global().holders().contains_key(agent_chat_id)
}

/// Capture before an external wait. Delayed terminalization must not affect a
/// later reservation that happens to use the same legacy chat identifier.
#[derive(Clone, Debug)]
pub struct Attempt {
    chat_id: String,
    token: Arc<()>,
}

pub fn current_attempt(chat_id: &str) -> Option<Attempt> {
    global().holders().get(chat_id).map(|entry| Attempt {
        chat_id: chat_id.into(),
        token: entry.attempt.clone(),
    })
}

impl Attempt {
    pub(crate) fn capture_for_app(app: &AppState, chat_id: &str) -> Result<Option<Self>, String> {
        let holders = global().holders();
        let tasks = global().tasks();
        let task = tasks
            .get(chat_id)
            .filter(|task| !task.finished.load(Ordering::Acquire));
        if holders.get(chat_id).is_some_and(|entry| {
            !Arc::ptr_eq(&entry.executions, &app.agent_executions)
                || !Arc::ptr_eq(&entry.scheduler, &app.work_scheduler)
        }) || task.is_some_and(|task| !Arc::ptr_eq(&task.executions, &app.agent_executions))
        {
            return Err("Board attempt belongs to another runtime".into());
        }
        let token = holders
            .get(chat_id)
            .map(|entry| entry.attempt.clone())
            .or_else(|| task.map(|task| task.attempt.clone()));
        if let Some(token) = &token {
            if task.is_some_and(|task| !Arc::ptr_eq(&task.attempt, token)) {
                return Err("Board setup and admission attempts disagree".into());
            }
        }
        Ok(token.map(|token| Self {
            chat_id: chat_id.into(),
            token,
        }))
    }

    pub(crate) fn same_attempt(&self, other: &Self) -> bool {
        self.chat_id == other.chat_id && Arc::ptr_eq(&self.token, &other.token)
    }

    pub(crate) fn is_current(&self) -> bool {
        global()
            .holders()
            .get(&self.chat_id)
            .is_some_and(|entry| Arc::ptr_eq(&entry.attempt, &self.token))
    }

    pub(crate) fn belongs_to(&self, app: &AppState) -> bool {
        let holders = global().holders();
        if let Some(entry) = holders.get(&self.chat_id) {
            return Arc::ptr_eq(&entry.attempt, &self.token)
                && Arc::ptr_eq(&entry.executions, &app.agent_executions)
                && Arc::ptr_eq(&entry.scheduler, &app.work_scheduler);
        }
        global().tasks().get(&self.chat_id).is_some_and(|task| {
            Arc::ptr_eq(&task.attempt, &self.token)
                && Arc::ptr_eq(&task.executions, &app.agent_executions)
        })
    }

    pub(crate) async fn cancel_setup_and_wait(
        &self,
        timeout: std::time::Duration,
    ) -> Result<(), String> {
        let task = {
            let mut holders = global().holders();
            if holders
                .get(&self.chat_id)
                .is_some_and(|entry| !Arc::ptr_eq(&entry.attempt, &self.token))
            {
                return Err("Board admission attempt changed; workspace retained".into());
            }
            let task = global().tasks().get(&self.chat_id).cloned();
            if task
                .as_ref()
                .is_some_and(|task| !Arc::ptr_eq(&task.attempt, &self.token))
            {
                return Err("Board setup attempt changed; workspace retained".into());
            }
            request_setup_cancel_locked(&mut holders, &self.chat_id);
            task
        };
        if let Some(task) = task {
            let wait = async {
                let mut handle = task.handle.lock().await;
                if !task.finished.load(Ordering::Acquire) {
                    let _ = (&mut *handle).await;
                }
            };
            if tokio::time::timeout(timeout, wait).await.is_err() {
                return Err(
                    "Timed out waiting for accepted board setup; workspace retained".into(),
                );
            }
            let mut tasks = global().tasks();
            if tasks
                .get(&self.chat_id)
                .is_some_and(|current| Arc::ptr_eq(current, &task))
            {
                tasks.remove(&self.chat_id);
            }
        }
        Ok(())
    }

    pub(crate) fn owns_setup(&self) -> bool {
        let holders = global().holders();
        if holders
            .get(&self.chat_id)
            .is_some_and(|entry| !Arc::ptr_eq(&entry.attempt, &self.token))
        {
            return false;
        }
        if holders
            .get(&self.chat_id)
            .is_some_and(|entry| Arc::ptr_eq(&entry.attempt, &self.token))
        {
            return true;
        }
        global().tasks().get(&self.chat_id).is_some_and(|task| {
            Arc::ptr_eq(&task.attempt, &self.token) && !task.finished.load(Ordering::Acquire)
        })
    }

    pub(crate) fn chat_id(&self) -> &str {
        &self.chat_id
    }

    pub fn end(&self) {
        end_attempt(&self.chat_id, Some(&self.token));
    }
}

/// Cancel while still queued (no worktree exists yet): drops the ticket
/// immediately. In-flight acquisitions are invalidated before cancellation,
/// so even an already-granted permit cannot activate setup. No-op once active.
pub async fn cancel_queued(agent_chat_id: &str) -> bool {
    let mut holders = global().holders();
    let is_queued = matches!(
        holders.get(agent_chat_id).map(|e| &e.phase),
        Some(Phase::Queued(_))
    );
    if is_queued {
        if let Some(Entry {
            phase: Phase::Queued(ticket),
            ..
        }) = holders.remove(agent_chat_id)
        {
            ticket.cancel();
        }
        return true;
    }
    if matches!(
        holders.get(agent_chat_id).map(|e| &e.phase),
        Some(Phase::Acquiring)
    ) {
        if let Some(entry) = holders.remove(agent_chat_id) {
            entry.scheduler.cancel(&run_key(agent_chat_id));
        }
        return true;
    }
    false
}

/// Wait (intentionally, no timeout) for the reserved ticket to become an
/// active permit. The acquisition registration is cancellation-safe, including
/// when its future is dropped. Its attempt token fences late results/cleanup
/// from a later reservation using the same chat ID.
pub async fn wait_for_permit(agent_chat_id: &str) -> Result<(), AdmissionError> {
    let (ticket, registration) = {
        let mut holders = global().holders();
        let Some(entry) = holders.get_mut(agent_chat_id) else {
            return Err(AdmissionError::Gone);
        };
        match &entry.phase {
            Phase::Active(_) => return Ok(()),
            Phase::Acquiring => return Err(AdmissionError::Gone),
            Phase::Queued(_) => {}
        }
        let registration = AcquisitionRegistration {
            agent_chat_id: agent_chat_id.to_string(),
            attempt: entry.attempt.clone(),
        };
        let Phase::Queued(ticket) = std::mem::replace(&mut entry.phase, Phase::Acquiring) else {
            unreachable!("checked above")
        };
        (ticket, registration)
    };

    let result = ticket.acquire().await;

    let mut holders = global().holders();
    let Some(entry) = holders.get_mut(agent_chat_id) else {
        if let Ok(permit) = result {
            permit.release();
        }
        return Err(AdmissionError::Cancelled);
    };
    if !Arc::ptr_eq(&entry.attempt, &registration.attempt) {
        if let Ok(permit) = result {
            permit.release();
        }
        return Err(AdmissionError::Cancelled);
    }
    match result {
        Ok(permit) => match entry.executions.install(agent_chat_id, permit) {
            Ok(execution) => {
                entry.phase = Phase::Active(execution);
                Ok(())
            }
            Err(_) => {
                holders.remove(agent_chat_id);
                Err(AdmissionError::Cancelled)
            }
        },
        Err(err) => {
            holders.remove(agent_chat_id);
            Err(err)
        }
    }
}

/// Invalidate and remove the entry immediately. In-flight acquisitions own
/// their scheduler guard until they settle/drop; active operations retain the
/// execution guard until they truly stop. Does not abort setup — see [`abort`].
pub async fn end(agent_chat_id: &str) {
    end_attempt(agent_chat_id, None);
}

fn end_attempt(agent_chat_id: &str, expected: Option<&Arc<()>>) {
    let mut holders = global().holders();
    if expected.is_some_and(|expected| {
        !holders
            .get(agent_chat_id)
            .is_some_and(|entry| Arc::ptr_eq(&entry.attempt, expected))
    }) {
        return;
    }
    match holders.remove(agent_chat_id) {
        Some(Entry {
            phase: Phase::Queued(ticket),
            ..
        }) => ticket.cancel(),
        Some(Entry {
            phase: Phase::Active(execution),
            executions,
            ..
        }) => {
            execution.cancel();
            executions.finish(agent_chat_id);
        }
        Some(
            entry @ Entry {
                phase: Phase::Acquiring,
                ..
            },
        ) => {
            entry.scheduler.cancel(&run_key(agent_chat_id));
        }
        None => {}
    }
}

/// Track a setup task. Completed handles are reaped on synchronization; shutdown
/// and abort join live handles before releasing their execution ownership.
pub async fn spawn_tracked<F>(agent_chat_id: &str, future: F) -> Result<(), AdmissionError>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let id = agent_chat_id.to_string();
    // There is intentionally no suspension between checking ownership and
    // storing the handle. Reserve + spawn_tracked can precede the first claim
    // await without a dropped caller stranding a globally owned reservation.
    let holders = global().holders();
    let entry = holders.get(agent_chat_id).ok_or(AdmissionError::Gone)?;
    if entry
        .executions
        .board_admission_closed
        .load(Ordering::Acquire)
    {
        return Err(AdmissionError::Disabled);
    }
    let mut tasks = global().tasks();
    tasks.retain(|_, task| !task.finished.load(Ordering::Acquire));
    if tasks.contains_key(agent_chat_id) {
        return Err(AdmissionError::Duplicate {
            run: run_key(agent_chat_id),
        });
    }
    let finished = Arc::new(AtomicBool::new(false));
    let flag = MarkSetupFinished(finished.clone());
    let actor = crate::file_activity::current_actor();
    let handle = tokio::spawn(async move {
        let _finished = flag;
        match actor {
            Some(actor) => crate::file_activity::TOOL_ACTOR.scope(actor, future).await,
            None => future.await,
        }
    });
    tasks.insert(
        id,
        Arc::new(SetupTask {
            handle: AMutex::new(handle),
            finished,
            executions: entry.executions.clone(),
            attempt: entry.attempt.clone(),
        }),
    );
    Ok(())
}

pub async fn is_queued(agent_chat_id: &str) -> bool {
    matches!(
        global()
            .holders()
            .get(agent_chat_id)
            .map(|entry| &entry.phase),
        Some(Phase::Queued(_) | Phase::Acquiring)
    )
}

pub fn execution_state(agent_chat_id: &str) -> Option<crate::agents::execution::ExecutionState> {
    let holders = global().holders();
    holders.get(agent_chat_id).map(|entry| match &entry.phase {
        Phase::Queued(_) | Phase::Acquiring => crate::agents::execution::ExecutionState::Acquiring,
        Phase::Active(execution) => execution.state(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StaleReason {
    RunnerConfigMismatch,
    SessionClosed,
    SourceRootMissing,
    SourceRootUnresolved,
    TaskDirUnavailable,
    TaskDirForeign,
    BoardUnreadable,
    PlannerUnbound,
    PlannerNotInCustody,
    CardNotDoing,
    AgentUnbound,
    CardBindingMismatch,
    ClaimHeldByOther,
    SessionReplaced,
    SessionIdentityDrift,
    ExecutionReplaced,
}

impl StaleReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::RunnerConfigMismatch => "runner_config_mismatch",
            Self::SessionClosed => "session_closed",
            Self::SourceRootMissing => "source_root_missing",
            Self::SourceRootUnresolved => "source_root_unresolved",
            Self::TaskDirUnavailable => "task_dir_unavailable",
            Self::TaskDirForeign => "task_dir_foreign",
            Self::BoardUnreadable => "board_unreadable",
            Self::PlannerUnbound => "planner_unbound",
            Self::PlannerNotInCustody => "planner_not_in_custody",
            Self::CardNotDoing => "card_not_doing",
            Self::AgentUnbound => "agent_unbound",
            Self::CardBindingMismatch => "card_binding_mismatch",
            Self::ClaimHeldByOther => "claim_held_by_other",
            Self::SessionReplaced => "session_replaced",
            Self::SessionIdentityDrift => "session_identity_drift",
            Self::ExecutionReplaced => "execution_replaced",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecutionUnavailable {
    Missing,
    Queued,
    Acquiring,
    Cancelled,
    StaleBinding(StaleReason),
    PendingToolReconciliation,
    Held,
}

impl ExecutionUnavailable {
    pub(crate) fn persistent(self) -> bool {
        match self {
            Self::StaleBinding(StaleReason::TaskDirUnavailable | StaleReason::BoardUnreadable) => {
                false
            }
            Self::StaleBinding(_)
            | Self::Missing
            | Self::Cancelled
            | Self::PendingToolReconciliation => true,
            Self::Queued | Self::Acquiring | Self::Held => false,
        }
    }

    pub(crate) fn condition(self) -> String {
        match self {
            Self::Missing => "needs_fresh_admission".into(),
            Self::Queued => "admission_queued".into(),
            Self::Acquiring => "admission_acquiring".into(),
            Self::Cancelled => "admission_cancelled".into(),
            Self::StaleBinding(reason) => format!("stale_board_attempt/{}", reason.as_str()),
            Self::PendingToolReconciliation => "needs_explicit_tool_reconciliation".into(),
            Self::Held => "execution_held".into(),
        }
    }

    fn stale_recovery_message(self, reason: StaleReason) -> String {
        let base = Self::Missing.recovery_message();
        base.replacen(
            "needs_fresh_admission",
            &format!("stale_board_attempt/{}", reason.as_str()),
            1,
        )
    }

    pub(crate) fn recovery_message(self) -> String {
        let condition = match self {
            Self::Missing => "needs_fresh_admission",
            Self::Queued => "admission_queued",
            Self::Acquiring => "admission_acquiring",
            Self::Cancelled => "admission_cancelled",
            Self::StaleBinding(reason) => return self.stale_recovery_message(reason),
            Self::PendingToolReconciliation => "needs_explicit_tool_reconciliation",
            Self::Held => "execution_held",
        };
        format!(
            "Interrupted: {condition}. No model or tool operation was started. In the owning task planner, inspect check_agents(format=\"detail\") and the current card, then explicitly call restart_agent(card_id=\"<current card id>\", mode=\"resume\"). This creates a NEW agent/chat on the SAME card and retained checkout through scheduler admission; it does not continue this chat. Use force=true only to stop an actually live owned attempt. Pauses, denied approvals and unknown tool outcomes must be resolved by their authorized controls first; restart does not clear them or replay pending tools. Opening this chat or approving an old request does not resume it. Retain the checkout and WIP."
        )
    }
}

pub(crate) fn current_execution(
    app: &AppState,
    chat_id: &str,
) -> Result<Arc<crate::agents::execution::RunExecution>, ExecutionUnavailable> {
    let holders = global().holders();
    let entry = holders.get(chat_id).ok_or(ExecutionUnavailable::Missing)?;
    if !Arc::ptr_eq(&entry.executions, &app.agent_executions)
        || !Arc::ptr_eq(&entry.scheduler, &app.work_scheduler)
    {
        return Err(ExecutionUnavailable::Missing);
    }
    let execution = match &entry.phase {
        Phase::Queued(_) => return Err(ExecutionUnavailable::Queued),
        Phase::Acquiring => return Err(ExecutionUnavailable::Acquiring),
        Phase::Active(execution) => execution,
    };
    if execution.is_cancelled()
        || execution.state() == crate::agents::execution::ExecutionState::Gone
    {
        return Err(ExecutionUnavailable::Cancelled);
    }
    let registered = app
        .agent_executions
        .get(chat_id)
        .ok_or(ExecutionUnavailable::Missing)?;
    if !Arc::ptr_eq(execution, &registered) {
        return Err(ExecutionUnavailable::StaleBinding(
            StaleReason::ExecutionReplaced,
        ));
    }
    Ok(execution.clone())
}

pub async fn synchronize(app: &AppState, agent_chat_id: &str) {
    let attempt = {
        let holders = global().holders();
        let Some(entry) = holders.get(agent_chat_id) else {
            return;
        };
        if !Arc::ptr_eq(&entry.executions, &app.agent_executions)
            || !matches!(entry.phase, Phase::Active(_))
        {
            return;
        }
        entry.attempt.clone()
    };
    if global()
        .tasks()
        .get(agent_chat_id)
        .is_some_and(|task| !task.finished.load(Ordering::Acquire))
    {
        return;
    }
    let session = app.chat.sessions.read().await.get(agent_chat_id).cloned();
    let Some(session) = session else {
        end_attempt(agent_chat_id, Some(&attempt));
        return;
    };
    let (busy, terminal) = {
        let session = session.lock().await;
        (
            session.turn_depth > 0
                || matches!(
                    session.runtime.state,
                    crate::chat::types::SessionState::Generating
                        | crate::chat::types::SessionState::ExecutingTools
                ),
            session.closed
                || matches!(
                    session.runtime.state,
                    crate::chat::types::SessionState::Completed
                        | crate::chat::types::SessionState::Error
                ),
        )
    };
    if busy {
        return;
    }
    if terminal {
        end_attempt(agent_chat_id, Some(&attempt));
    } else {
        let holders = global().holders();
        if let Some(entry) = holders
            .get(agent_chat_id)
            .filter(|entry| Arc::ptr_eq(&entry.attempt, &attempt))
        {
            if let Phase::Active(execution) = &entry.phase {
                let _ = execution.park_now();
            }
        }
    }
}

pub async fn synchronize_all(app: &AppState) {
    global()
        .tasks()
        .retain(|_, task| !task.finished.load(Ordering::Acquire));
    let ids: Vec<_> = global()
        .holders()
        .iter()
        .filter(|(_, entry)| Arc::ptr_eq(&entry.executions, &app.agent_executions))
        .map(|(id, _)| id.clone())
        .collect();
    for id in ids {
        synchronize(app, &id).await;
    }
}

pub async fn shutdown(app: &AppState) {
    app.agent_executions.begin_shutdown();
    let (ids, tasks) = {
        let holders = global().holders();
        // Same lock as reservation/registration: no late launch escapes the
        // shutdown snapshot. The latch belongs to this AppState, not the process.
        app.agent_executions
            .board_admission_closed
            .store(true, Ordering::Release);
        let ids: Vec<_> = holders
            .iter()
            .filter(|(_, entry)| Arc::ptr_eq(&entry.executions, &app.agent_executions))
            .map(|(id, entry)| (id.clone(), entry.attempt.clone()))
            .collect();
        let tasks: Vec<_> = global()
            .tasks()
            .iter()
            .filter(|(_, task)| Arc::ptr_eq(&task.executions, &app.agent_executions))
            .map(|(id, task)| (id.clone(), task.clone()))
            .collect();
        (ids, tasks)
    };
    // Cancel every dependency before joining any of them.
    for (id, _) in &ids {
        request_setup_cancel(id);
    }
    for (id, task) in tasks {
        // Cancel first, then join even if setup needs time to finish an OS/git
        // operation. Dropping setup can detach spawn_blocking filesystem work.
        let mut handle = task.handle.lock().await;
        if !task.finished.load(Ordering::Acquire) {
            let _ = (&mut *handle).await;
        }
        end_attempt(&id, Some(&task.attempt));
        let mut tracked = global().tasks();
        if tracked
            .get(&id)
            .is_some_and(|current| Arc::ptr_eq(current, &task))
        {
            tracked.remove(&id);
        }
    }
    for (id, attempt) in ids {
        end_attempt(&id, Some(&attempt));
    }
}

/// Request cooperative cancellation and join setup without dropping a future
/// that may own an in-flight filesystem operation. A timeout keeps the handle
/// and permit tracked; callers must retain the workspace in that case.
pub async fn cancel_setup_and_wait(
    agent_chat_id: &str,
    timeout: std::time::Duration,
) -> Result<(), String> {
    request_setup_cancel(agent_chat_id);
    let task = global().tasks().get(agent_chat_id).cloned();
    if let Some(task) = task {
        let wait = async {
            let mut handle = task.handle.lock().await;
            if !task.finished.load(Ordering::Acquire) {
                let _ = (&mut *handle).await;
            }
        };
        if tokio::time::timeout(timeout, wait).await.is_err() {
            return Err(format!(
                "Timed out waiting for board setup {agent_chat_id} to stop; workspace retained"
            ));
        }
        let mut tasks = global().tasks();
        if tasks
            .get(agent_chat_id)
            .is_some_and(|current| Arc::ptr_eq(current, &task))
        {
            tasks.remove(agent_chat_id);
        }
    }
    // The live generation supervisor still owns execution until its turn ends.
    // Only setup-only entries can be released here; synchronize handles sessions.
    Ok(())
}

fn request_setup_cancel(agent_chat_id: &str) {
    let mut holders = global().holders();
    request_setup_cancel_locked(&mut holders, agent_chat_id);
}

fn request_setup_cancel_locked(holders: &mut HashMap<String, Entry>, agent_chat_id: &str) {
    if let Some(entry) = holders.get(agent_chat_id) {
        match &entry.phase {
            Phase::Active(execution) => execution.cancel(),
            Phase::Acquiring => {
                entry.scheduler.cancel(&run_key(agent_chat_id));
            }
            Phase::Queued(_) => {
                if let Some(Entry {
                    phase: Phase::Queued(ticket),
                    ..
                }) = holders.remove(agent_chat_id)
                {
                    ticket.cancel();
                }
            }
        }
    }
}

/// Abort the tracked setup task for `agent_chat_id` (if any) and wait for the
/// abort to actually land before releasing its admission slot, so the permit
/// is never freed while the task might still be using it.
pub async fn abort(agent_chat_id: &str) {
    let attempt = global()
        .holders()
        .get(agent_chat_id)
        .map(|entry| entry.attempt.clone());
    let task = global().tasks().get(agent_chat_id).cloned();
    if let Some(task) = task {
        let mut handle = task.handle.lock().await;
        if !task.finished.load(Ordering::Acquire) {
            handle.abort();
            let _ = (&mut *handle).await;
        }
        let mut tasks = global().tasks();
        if tasks
            .get(agent_chat_id)
            .is_some_and(|current| Arc::ptr_eq(current, &task))
        {
            tasks.remove(agent_chat_id);
        }
    }
    if let Some(attempt) = attempt {
        end_attempt(agent_chat_id, Some(&attempt));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_work_core::{AdmissionScheduler, CoordinationSettings};
    use std::sync::Arc;
    use uuid::Uuid;

    fn unique_id(tag: &str) -> String {
        format!("{tag}-{}", Uuid::new_v4())
    }

    async fn poll_pending<F: std::future::Future>(mut future: std::pin::Pin<&mut F>) {
        std::future::poll_fn(|cx| {
            assert!(future.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
    }

    async fn app_with_settings(settings: CoordinationSettings) -> AppState {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut app = AppState::from_gcx(gcx).await;
        app.work_scheduler = Arc::new(AdmissionScheduler::new(settings));
        app
    }

    async fn app_with_cap(cap: usize) -> AppState {
        app_with_settings(CoordinationSettings {
            max_active_engine: cap,
            max_active_project: cap,
            max_active_root: cap,
            ..CoordinationSettings::default()
        })
        .await
    }

    #[tokio::test]
    async fn refused_reservation_leaves_no_phantom_child_and_allocates_no_setup_task() {
        let app = app_with_settings(CoordinationSettings {
            max_active_engine: 1,
            max_active_project: 1,
            max_active_root: 1,
            max_pending: 1,
            max_outstanding: 8,
            ..CoordinationSettings::default()
        })
        .await;
        let owner = unique_id("owner");
        let queued = unique_id("queued");
        let refused = unique_id("refused");
        reserve(&app, &owner, "proj", "root", 0).await.unwrap();
        wait_for_permit(&owner).await.unwrap();
        reserve(&app, &queued, "proj", "root", 0).await.unwrap();

        let err = reserve(&app, &refused, "proj", "root", 0)
            .await
            .unwrap_err();
        assert!(matches!(err, AdmissionError::QueueFull { .. }), "{err:?}");
        assert!(!is_tracked(&refused).await);
        assert!(matches!(
            current_execution(&app, &refused),
            Err(ExecutionUnavailable::Missing)
        ));
        assert!(matches!(
            current_execution(&app, &queued),
            Err(ExecutionUnavailable::Queued)
        ));
        assert!(Arc::ptr_eq(
            &current_execution(&app, &owner).unwrap(),
            &app.agent_executions.get(&owner).unwrap()
        ));
        assert!(matches!(
            spawn_tracked(&refused, async {}).await,
            Err(AdmissionError::Gone)
        ));
        assert!(!global().tasks().contains_key(&refused));
        assert_eq!(app.work_scheduler.snapshot().outstanding, 2);

        end(&queued).await;
        end(&owner).await;
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
    }

    #[tokio::test]
    async fn residency_eviction_checkpoints_a_parked_board_run_instead_of_cancelling_it() {
        let app = app_with_settings(CoordinationSettings {
            max_active_engine: 1,
            max_active_project: 1,
            max_active_root: 1,
            max_parked_residency: 1,
            ..CoordinationSettings::default()
        })
        .await;
        let first = unique_id("first");
        let second = unique_id("second");
        let session = Arc::new(tokio::sync::Mutex::new(
            crate::chat::types::ChatSession::new(first.clone()),
        ));
        app.chat
            .sessions
            .write()
            .await
            .insert(first.clone(), session.clone());
        for id in [&first, &second] {
            reserve(&app, id, "proj", "root", 0).await.unwrap();
            wait_for_permit(id).await.unwrap();
            app.agent_executions.get(id).unwrap().park().await.unwrap();
        }

        let snapshot = app.work_scheduler.snapshot();
        assert_eq!((snapshot.parked, snapshot.evicted), (1, 1));
        let evicted = snapshot
            .tickets
            .iter()
            .find(|ticket| ticket.state == refact_work_core::TicketState::Evicted)
            .expect("the run parked longest is checkpointed");
        assert_eq!(evicted.run.work.id, first);
        assert!(
            is_tracked(&first).await,
            "an evicted run is checkpointed, not cancelled"
        );
        let execution = app.agent_executions.get(&first).unwrap();
        assert!(!execution.is_cancelled());
        assert!(Arc::ptr_eq(
            &app.chat.sessions.read().await[&first],
            &session
        ));

        execution.resume().await.unwrap();
        assert!(execution.holds_permit());
        assert_eq!(app.work_scheduler.snapshot().evicted, 0);
        assert!(Arc::ptr_eq(
            &app.chat.sessions.read().await[&first],
            &session
        ));

        drop(execution);
        end(&first).await;
        end(&second).await;
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
    }

    #[tokio::test]
    async fn shutdown_joins_setup_after_holder_removed_and_closes_only_its_app() {
        let app = app_with_cap(1).await;
        let other_app = app_with_cap(1).await;
        let id = unique_id("cleanup");
        reserve(&app, &id, "proj", "root", 0).await.unwrap();
        wait_for_permit(&id).await.unwrap();
        let (entered, entry) = tokio::sync::oneshot::channel();
        let (release, gate) = tokio::sync::oneshot::channel();
        let child = id.clone();
        spawn_tracked(&id, async move {
            end(&child).await;
            entered.send(()).unwrap();
            gate.await.unwrap();
        })
        .await
        .unwrap();
        entry.await.unwrap();
        assert!(!is_tracked(&id).await);
        let mut stopping = Box::pin(shutdown(&app));
        poll_pending(stopping.as_mut()).await;
        assert_eq!(
            reserve(&app, &unique_id("late"), "proj", "root", 0).await,
            Err(AdmissionError::Disabled)
        );
        let other = unique_id("other");
        reserve(&other_app, &other, "other", "root", 0)
            .await
            .unwrap();
        release.send(()).unwrap();
        stopping.await;
        assert!(!global().tasks().contains_key(&id));
        assert!(is_tracked(&other).await);
        end(&other).await;
    }

    #[tokio::test]
    async fn live_setup_cannot_be_overwritten_or_reused_after_early_end() {
        let app = app_with_cap(1).await;
        let id = unique_id("reuse");
        reserve(&app, &id, "proj", "root", 0).await.unwrap();
        let (release, gate) = tokio::sync::oneshot::channel();
        spawn_tracked(&id, async move {
            let _ = gate.await;
        })
        .await
        .unwrap();
        assert!(matches!(
            spawn_tracked(&id, async {}).await,
            Err(AdmissionError::Duplicate { .. })
        ));
        end(&id).await;
        assert!(matches!(
            reserve(&app, &id, "proj", "root", 0).await,
            Err(AdmissionError::Duplicate { .. })
        ));
        release.send(()).unwrap();
        cancel_setup_and_wait(&id, std::time::Duration::from_secs(5))
            .await
            .unwrap();
        reserve(&app, &id, "proj", "root", 0).await.unwrap();
        end(&id).await;
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
    }

    #[tokio::test]
    async fn cancel_queued_drops_reservation_without_touching_active_permits() {
        let app = app_with_cap(1).await;
        let a = unique_id("agent-a");
        let b = unique_id("agent-b");
        reserve(&app, &a, "proj", "task", 0).await.unwrap();
        wait_for_permit(&a).await.unwrap();

        reserve(&app, &b, "proj", "task", 0).await.unwrap();
        assert!(cancel_queued(&b).await);
        assert!(!is_tracked(&b).await);
        assert!(
            !cancel_queued(&a).await,
            "active permit is not cancel_queued's job"
        );

        end(&a).await;
    }

    #[tokio::test]
    async fn dropped_acquisition_removes_adapter_and_scheduler_reservations() {
        for grant_before_drop in [false, true] {
            let app = app_with_cap(1).await;
            let holder = unique_id("holder");
            let child = unique_id("child");
            reserve(&app, &holder, "proj", "root", 0).await.unwrap();
            wait_for_permit(&holder).await.unwrap();
            reserve(&app, &child, "proj", "root", 0).await.unwrap();
            let mut waiting = Box::pin(wait_for_permit(&child));
            poll_pending(waiting.as_mut()).await;
            assert_eq!(app.work_scheduler.snapshot().pending, 1);
            if grant_before_drop {
                end(&holder).await;
                assert_eq!(app.work_scheduler.snapshot().active, 1);
            }
            drop(waiting);
            assert!(!is_tracked(&child).await);
            assert!(app.agent_executions.get(&child).is_none());
            assert_eq!(app.work_scheduler.snapshot().pending, 0);
            assert_eq!(
                app.work_scheduler.snapshot().outstanding,
                usize::from(!grant_before_drop)
            );
            end(&holder).await;
            assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
        }
    }

    #[tokio::test]
    async fn late_acquisition_and_drop_cannot_remove_a_replacement_attempt() {
        for drop_old in [false, true] {
            let app = app_with_cap(1).await;
            let holder = unique_id("holder");
            let child = unique_id("child");
            reserve(&app, &holder, "proj", "root", 0).await.unwrap();
            wait_for_permit(&holder).await.unwrap();
            reserve(&app, &child, "proj", "root", 0).await.unwrap();
            let mut old = Box::pin(wait_for_permit(&child));
            poll_pending(old.as_mut()).await;
            assert!(cancel_queued(&child).await);
            reserve(&app, &child, "proj", "root", 0).await.unwrap();
            let mut replacement = Box::pin(wait_for_permit(&child));
            poll_pending(replacement.as_mut()).await;
            if drop_old {
                drop(old);
            } else {
                assert_eq!(old.await, Err(AdmissionError::Cancelled));
            }
            assert!(is_tracked(&child).await);
            assert_eq!(app.work_scheduler.snapshot().pending, 1);
            end(&holder).await;
            replacement.await.unwrap();
            end(&child).await;
            assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
        }
    }

    #[tokio::test]
    async fn abort_pending_setup_joins_and_removes_acquiring_entry() {
        let app = app_with_cap(1).await;
        let holder = unique_id("holder");
        let child = unique_id("child");
        reserve(&app, &holder, "proj", "root", 0).await.unwrap();
        wait_for_permit(&holder).await.unwrap();
        reserve(&app, &child, "proj", "root", 0).await.unwrap();
        let (ready, ready_rx) = tokio::sync::oneshot::channel();
        let id = child.clone();
        spawn_tracked(&child, async move {
            let mut waiting = Box::pin(wait_for_permit(&id));
            poll_pending(waiting.as_mut()).await;
            ready.send(()).unwrap();
            waiting
                .await
                .expect("setup should be aborted before admission");
            panic!("cancelled setup must not execute side effects");
        })
        .await
        .unwrap();
        ready_rx.await.unwrap();
        abort(&child).await;
        assert!(!is_tracked(&child).await);
        assert!(!global().tasks().contains_key(&child));
        assert_eq!(app.work_scheduler.snapshot().outstanding, 1);
        assert_eq!(app.work_scheduler.snapshot().pending, 0);
        end(&holder).await;
    }

    #[tokio::test]
    async fn end_preserves_capacity_until_executable_operations_drain() {
        let app = app_with_cap(1).await;
        let child = unique_id("child");
        reserve(&app, &child, "proj", "root", 0).await.unwrap();
        wait_for_permit(&child).await.unwrap();
        let execution = app.agent_executions.get(&child).unwrap();
        let operation = execution.begin_operation().await.unwrap();
        end(&child).await;
        assert!(matches!(
            current_execution(&app, &child),
            Err(ExecutionUnavailable::Missing)
        ));
        assert!(execution.is_cancelled());
        assert!(execution.begin_operation().await.is_err());
        drop(execution);
        assert_eq!(app.work_scheduler.snapshot().active, 1);
        drop(operation);
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
    }

    #[tokio::test]
    async fn board_and_chat_descendants_share_project_and_root_capacity() {
        for limit_project in [false, true] {
            let mut app = app_with_cap(4).await;
            app.work_scheduler = Arc::new(AdmissionScheduler::new(CoordinationSettings {
                max_active_engine: 4,
                max_active_project: if limit_project { 1 } else { 4 },
                max_active_root: if limit_project { 4 } else { 1 },
                ..CoordinationSettings::default()
            }));
            let root = unique_id("root");
            let project = crate::coordination::roots::project_identity(std::path::Path::new(
                "/tmp/shared-project",
            ));
            let chat = app
                .work_scheduler
                .reserve(AdmissionRequest::new(
                    WorkKey::background_agent(unique_id("chat")).run(1),
                    board_scope(&project, &root),
                ))
                .unwrap()
                .acquire()
                .await
                .unwrap();
            let child = unique_id("board-child");
            let board_root = if limit_project {
                unique_id("other-root")
            } else {
                root
            };
            reserve(&app, &child, &project, &board_root, 0)
                .await
                .unwrap();
            let mut waiting = Box::pin(wait_for_permit(&child));
            poll_pending(waiting.as_mut()).await;
            assert_eq!(app.work_scheduler.snapshot().active, 1);
            chat.release();
            waiting.await.unwrap();
            end(&child).await;
            assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
        }
    }

    #[tokio::test]
    async fn end_during_in_flight_wait_for_permit_releases_instead_of_activating() {
        let app = app_with_cap(1).await;
        let holder = unique_id("agent-holder");
        let a = unique_id("agent-a");
        // Fill capacity so `a`'s wait_for_permit actually parks.
        reserve(&app, &holder, "proj", "task", 0).await.unwrap();
        wait_for_permit(&holder).await.unwrap();

        reserve(&app, &a, "proj", "task", 0).await.unwrap();
        let mut waiting = Box::pin(wait_for_permit(&a));
        poll_pending(waiting.as_mut()).await;
        end(&a).await;
        // Now free capacity: the parked ticket may or may not still exist
        // depending on scheduler internals, but either way the caller must
        // observe cancellation, not an activated permit it never learns about.
        end(&holder).await;
        let outcome = waiting.await;
        assert!(
            outcome.is_err(),
            "cancelled wait_for_permit must not report success"
        );
        assert!(!is_tracked(&a).await);
    }

    #[tokio::test]
    async fn cooperative_setup_cancel_timeout_retains_handle_and_capacity() {
        let app = app_with_cap(1).await;
        let child = unique_id("setup");
        reserve(&app, &child, "proj", "root", 0).await.unwrap();
        wait_for_permit(&child).await.unwrap();
        let (release, waiting) = tokio::sync::oneshot::channel::<()>();
        let id = child.clone();
        spawn_tracked(&child, async move {
            waiting.await.unwrap();
            end(&id).await;
        })
        .await
        .unwrap();
        let error = cancel_setup_and_wait(&child, std::time::Duration::ZERO)
            .await
            .unwrap_err();
        assert!(error.contains("workspace retained"));
        assert_eq!(app.work_scheduler.snapshot().active, 1);
        assert!(global().tasks().contains_key(&child));
        release.send(()).unwrap();
        cancel_setup_and_wait(&child, std::time::Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
        assert!(!global().tasks().contains_key(&child));
    }

    #[tokio::test]
    async fn dropping_cancel_waiter_keeps_setup_visible_until_joined() {
        let app = app_with_cap(1).await;
        let child = unique_id("setup-drop");
        reserve(&app, &child, "proj", "root", 0).await.unwrap();
        wait_for_permit(&child).await.unwrap();
        let (release, waiting) = tokio::sync::oneshot::channel::<()>();
        let id = child.clone();
        spawn_tracked(&child, async move {
            waiting.await.unwrap();
            end(&id).await;
        })
        .await
        .unwrap();
        let mut cancel = Box::pin(cancel_setup_and_wait(
            &child,
            std::time::Duration::from_secs(30),
        ));
        poll_pending(cancel.as_mut()).await;
        drop(cancel);
        assert!(global().tasks().contains_key(&child));
        assert_eq!(app.work_scheduler.snapshot().active, 1);
        release.send(()).unwrap();
        cancel_setup_and_wait(&child, std::time::Duration::from_secs(5))
            .await
            .unwrap();
        assert!(!global().tasks().contains_key(&child));
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
    }
}
