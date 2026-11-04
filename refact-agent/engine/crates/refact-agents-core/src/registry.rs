use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta, Utc};
use tokio::sync::{Mutex, Notify, RwLock};
use uuid::Uuid;
use refact_chat_api::{
    AskQuestion, DeliveryOutcome, DeliveryStatus, DeliveryStatusRecord, PendingDelivery, PushMode,
    QuestionKind, QuestionLimits, QuestionRegistry,
};

use crate::storage;
use crate::types::{
    AgentCompletion, AgentListFilter, AgentRunOutcome, BackgroundAgent, BgAgentStatus,
    CreateAgentRequest,
};
#[cfg(test)]
use crate::types::BgAgentKind;

/// Native steering notes are bounded by the mandate's own note capacity; a
/// full queue rejects instead of silently discarding an earlier authorization.
pub const MAX_NATIVE_NOTES: usize = 12;

#[derive(Debug, Clone)]
pub struct InboxMessage {
    pub from: String,
    pub text: String,
    pub queued_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct AgentMessageReceipt {
    pub outcome: DeliveryOutcome,
    pub finished_status: Option<BgAgentStatus>,
    pub dropped_followups: Vec<String>,
}

type NativeNoteAuthorizer = Arc<dyn Fn(&str, &str) -> Result<String, String> + Send + Sync>;

/// A queued native steering note. It carries no text of its own: the text is
/// produced only by `consume`, which re-runs the native authorization the
/// owner minted (epoch, expiry, chat and model binding) at delivery time. No
/// Deserialize implementation exists, so HTTP bodies or model output cannot
/// manufacture one. Ordinary `InboxMessage`s are untouched by this type.
#[derive(Clone)]
pub struct NativeNote {
    pub control_id: String,
    pub note_id: String,
    pub queued_at: DateTime<Utc>,
    authorize: NativeNoteAuthorizer,
}

impl std::fmt::Debug for NativeNote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeNote")
            .field("control_id", &self.control_id)
            .field("note_id", &self.note_id)
            .field("queued_at", &self.queued_at)
            .finish_non_exhaustive()
    }
}

impl NativeNote {
    pub fn new(
        control_id: String,
        note_id: String,
        authorize: impl Fn(&str, &str) -> Result<String, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            control_id,
            note_id,
            queued_at: Utc::now(),
            authorize: Arc::new(authorize),
        }
    }

    /// Re-authorize against the consuming chat and model; a revoked, expired,
    /// re-epoched, rebound, or model-changed grant yields no text at all.
    pub fn consume(&self, chat_id: &str, model: &str) -> Result<String, String> {
        (self.authorize)(chat_id, model)
    }
}

/// Cooperative pause barrier shared between the owner and the running loop.
/// `requested` is the owner's intent; `quiescent` is set only by the loop when
/// it is actually parked at a model/tool boundary, so a status reader can tell
/// "pause requested, dispatch still in flight" from "no further dispatch".
#[derive(Default)]
pub struct PauseGate {
    requested: AtomicBool,
    parent_requested: AtomicBool,
    quiescent: AtomicBool,
    wake: Notify,
    quiescence: Notify,
}

impl PauseGate {
    pub fn is_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst) || self.parent_requested.load(Ordering::SeqCst)
    }

    pub fn is_quiescent(&self) -> bool {
        self.quiescent.load(Ordering::SeqCst)
    }

    pub fn request(&self) {
        self.requested.store(true, Ordering::SeqCst);
    }

    pub fn release(&self) {
        self.requested.store(false, Ordering::SeqCst);
        self.wake.notify_waiters();
    }

    fn set_parent_requested(&self, requested: bool) {
        self.parent_requested.store(requested, Ordering::SeqCst);
        if !requested {
            self.wake.notify_waiters();
        }
    }

    /// Unblock a parked loop without releasing the pause; the loop observes
    /// the abort flag and exits instead of dispatching.
    pub fn wake_for_abort(&self) {
        self.wake.notify_waiters();
    }

    /// Resolves once the loop has actually parked, or immediately if it is
    /// already parked. Deterministic test hook; never used by the owner path.
    pub async fn quiesced(&self) {
        loop {
            let notified = self.quiescence.notified();
            if self.is_quiescent() {
                return;
            }
            notified.await;
        }
    }

    /// Park while a pause is requested. Returns `true` when the wait ended
    /// because of cancellation, so the caller must not dispatch. The in-flight
    /// outcome the caller already holds is untouched by this call.
    pub async fn hold(&self, abort: Option<&AtomicBool>) -> bool {
        let aborted = || abort.is_some_and(|flag| flag.load(Ordering::SeqCst));
        loop {
            // Register before checking state: Notify does not retain a
            // notify_waiters permit, so reversing this order can lose a
            // release/cancel wake between the check and await.
            let notified = self.wake.notified();
            if aborted() {
                self.quiescent.store(false, Ordering::SeqCst);
                return true;
            }
            if !self.is_requested() {
                self.quiescent.store(false, Ordering::SeqCst);
                return false;
            }
            if !self.quiescent.swap(true, Ordering::SeqCst) {
                self.quiescence.notify_waiters();
            }
            notified.await;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PauseStatus {
    pub requested: bool,
    /// The loop is parked at a boundary: no model or tool dispatch will start
    /// until release or cancellation.
    pub quiescent: bool,
    pub running_tool: Option<String>,
    /// Tools run to their next await point; cancellation never interrupts a
    /// synchronous tool body, it only prevents the next dispatch.
    pub running_tool_cancelable: bool,
    pub queued_native_notes: usize,
}

#[derive(Clone)]
pub struct AgentRuntime {
    pub abort_flag: Arc<AtomicBool>,
    pub interrupt_notify: Arc<Notify>,
    pub notify: Arc<Notify>,
    pub inbox: Arc<Mutex<VecDeque<InboxMessage>>>,
    pub pause: Arc<PauseGate>,
    pub native_notes: Arc<Mutex<VecDeque<NativeNote>>>,
    delivery_state: Arc<Mutex<RunnerDeliveryState>>,
}

#[derive(Debug, Clone)]
pub struct ParentRun {
    agent_id: String,
    generation: u64,
    abort_flag: Arc<AtomicBool>,
}

pub struct CheckoutSetup {
    record: BackgroundAgent,
    abort: Arc<AtomicBool>,
    parent: Option<ParentRun>,
    operation: uuid::Uuid,
}

impl CheckoutSetup {
    pub fn operation(&self) -> uuid::Uuid {
        self.operation
    }
}

#[derive(Debug)]
pub struct CheckoutSetupError {
    pub message: String,
    pub retain_checkout: bool,
}

#[derive(Default)]
struct RunnerDeliveryState {
    claimed: HashSet<String>,
    sealed: bool,
}

pub struct BackgroundAgentRegistry {
    records: RwLock<AgentRecords>,
    runtime: RwLock<HashMap<String, AgentRuntime>>,
    storage_root: PathBuf,
    write_state: Mutex<RegistryWriteState>,
    storage_write: Mutex<()>,
    questions: QuestionRegistry,
    quarantine: Option<storage::QuarantineReport>,
    #[cfg(any(test, feature = "test-support"))]
    list_all_calls: std::sync::atomic::AtomicUsize,
}

#[derive(Default)]
struct AgentRecords {
    by_id: HashMap<String, BackgroundAgent>,
    by_child: HashMap<String, String>,
    cancelled_subtrees: HashMap<String, u64>,
}

impl std::ops::Deref for AgentRecords {
    type Target = HashMap<String, BackgroundAgent>;

    fn deref(&self) -> &Self::Target {
        &self.by_id
    }
}

impl AgentRecords {
    fn insert(&mut self, agent_id: String, record: BackgroundAgent) {
        if let Some(previous) = self.by_id.insert(agent_id.clone(), record) {
            if let Some(child) = previous.child_chat_id {
                self.by_child.remove(&child);
            }
        }
        if let Some(child) = &self.by_id[&agent_id].child_chat_id {
            self.by_child.insert(child.clone(), agent_id);
        }
    }

    fn get_mut(&mut self, agent_id: &str) -> Option<&mut BackgroundAgent> {
        self.by_id.get_mut(agent_id)
    }
}

struct RegistryWriteState {
    pending: bool,
    last_write: Option<Instant>,
    flush_task: Option<tokio::task::JoinHandle<()>>,
}

impl Default for RegistryWriteState {
    fn default() -> Self {
        Self {
            pending: false,
            last_write: None,
            flush_task: None,
        }
    }
}

const WRITE_DEBOUNCE: Duration = Duration::from_secs(10);

impl BackgroundAgentRegistry {
    pub async fn new(storage_root: PathBuf) -> Result<Arc<Self>, String> {
        Self::with_question_limits(storage_root, QuestionLimits::default()).await
    }

    pub async fn with_question_limits(
        storage_root: PathBuf,
        question_limits: QuestionLimits,
    ) -> Result<Arc<Self>, String> {
        tokio::fs::create_dir_all(&storage_root)
            .await
            .map_err(|e| format!("Failed to create background agent registry directory: {e}"))?;
        let (mut records, quarantine) = storage::load_all_with_report(&storage_root).await?;
        normalize_delivery_state(&mut records);
        normalize_run_history(&mut records);
        reconcile_interrupted(&storage_root, &mut records).await?;
        let questions = QuestionRegistry::new(question_limits)?;
        let now = Utc::now();
        questions.restore(
            records
                .values()
                .flat_map(|record| record.questions.iter().cloned()),
            now,
        );
        for record in records
            .values_mut()
            .filter(|record| record.status.is_terminal())
        {
            let previous = record.clone();
            settle_terminal_record(&questions, record, now);
            if previous != *record {
                storage::save_record(&storage_root, record).await?;
            }
        }
        let mut indexed = AgentRecords::default();
        for (agent_id, record) in records {
            indexed.insert(agent_id, record);
        }
        Ok(Arc::new(Self {
            records: RwLock::new(indexed),
            runtime: RwLock::new(HashMap::new()),
            storage_root,
            write_state: Mutex::new(RegistryWriteState::default()),
            storage_write: Mutex::new(()),
            questions,
            quarantine,
            #[cfg(any(test, feature = "test-support"))]
            list_all_calls: std::sync::atomic::AtomicUsize::new(0),
        }))
    }

    pub fn quarantine_report(&self) -> Option<&storage::QuarantineReport> {
        self.quarantine.as_ref()
    }

    /// The single tracked question service. Every question family shares this
    /// instance, and agent records are its durable projection.
    pub fn questions(&self) -> &QuestionRegistry {
        &self.questions
    }

    pub async fn create(
        &self,
        req: CreateAgentRequest,
    ) -> Result<(BackgroundAgent, Arc<AtomicBool>, Arc<Notify>), String> {
        let agent_id = format!("bgagent-{}", Uuid::new_v4());
        self.create_with_id(req, agent_id).await
    }

    pub async fn create_with_id(
        &self,
        req: CreateAgentRequest,
        agent_id: impl Into<String>,
    ) -> Result<(BackgroundAgent, Arc<AtomicBool>, Arc<Notify>), String> {
        let parent = self.parent_run(&req.parent_chat_id).await?;
        self.create_with_id_for_parent_run(req, agent_id.into(), parent.as_ref())
            .await
    }

    pub async fn parent_run(&self, chat_id: &str) -> Result<Option<ParentRun>, String> {
        let records = self.records.read().await;
        let Some(agent_id) = records.by_child.get(chat_id) else {
            return Ok(None);
        };
        let record = &records.by_id[agent_id];
        if record.status.is_terminal() {
            return Err("parent agent run has stopped".into());
        }
        let runtime = self.runtime.read().await;
        let runtime = runtime
            .get(agent_id)
            .ok_or("parent agent run unavailable")?;
        let parent = ParentRun {
            agent_id: agent_id.clone(),
            generation: record.run_generation,
            abort_flag: runtime.abort_flag.clone(),
        };
        validate_parent_run(&records, chat_id, Some(&parent))?;
        Ok(Some(parent))
    }

    pub async fn validate_parent_run(
        &self,
        chat_id: &str,
        parent: Option<&ParentRun>,
    ) -> Result<(), String> {
        let records = self.records.read().await;
        validate_parent_run(&records, chat_id, parent)
    }

    pub async fn create_for_parent_run(
        &self,
        req: CreateAgentRequest,
        parent: Option<&ParentRun>,
    ) -> Result<(BackgroundAgent, Arc<AtomicBool>, Arc<Notify>), String> {
        self.create_with_id_for_parent_run(req, format!("bgagent-{}", Uuid::new_v4()), parent)
            .await
    }

    async fn create_with_id_for_parent_run(
        &self,
        req: CreateAgentRequest,
        agent_id: String,
        parent: Option<&ParentRun>,
    ) -> Result<(BackgroundAgent, Arc<AtomicBool>, Arc<Notify>), String> {
        let now = Utc::now();
        let record = BackgroundAgent {
            schema_version: storage::SCHEMA_VERSION,
            run_generation: 1,
            stop_hook_generation: None,
            agent_id: agent_id.clone(),
            parent_chat_id: req.parent_chat_id,
            parent_root_chat_id: req.parent_root_chat_id,
            parent_tool_call_id: req.parent_tool_call_id,
            child_chat_id: None,
            kind: req.kind,
            config_name: req.config_name,
            title: req.title,
            prompt: req.prompt,
            target_files: req.target_files,
            status: BgAgentStatus::Queued,
            paused_by: None,
            paused_at: None,
            progress: None,
            step_count: 0,
            last_activity: None,
            result_summary: None,
            result_payload_path: None,
            error: None,
            edited_files: Vec::new(),
            diff_summary: None,
            conflict_summary: None,
            completion_message_id: None,
            completion_pushed_at: None,
            completion_push: PushMode::Append,
            pending_deliveries: Vec::new(),
            followups: Vec::new(),
            delivery_ids: Vec::new(),
            dropped_deliveries: Vec::new(),
            deferred_at: None,
            close_reason: None,
            previous_runs: Vec::new(),
            model: req.model,
            model_type: req.model_type,
            current_tool: None,
            goal_summary: req.goal_summary,
            plan_present: req.plan_present,
            worktree_id: req.worktree_id,
            worktree_branch: req.worktree_branch,
            merge_status: None,
            questions: Vec::new(),
            tokens_used: 0,
            cost_usd: None,
            deliverable: None,
            deliverable_status: None,
            profile_name: None,
            handoff: None,
            created_at: now,
            started_at: None,
            finished_at: None,
            last_update_at: now,
            change_seq: 1,
        };
        let abort_flag = Arc::new(AtomicBool::new(false));
        let notify = Arc::new(Notify::new());
        let inbox = Arc::new(Mutex::new(VecDeque::new()));
        let pause = Arc::new(PauseGate::default());
        let native_notes = Arc::new(Mutex::new(VecDeque::new()));
        {
            let mut records = self.records.write().await;
            if records.contains_key(&agent_id) {
                return Err("agent id already exists".to_string());
            }
            validate_parent_run(&records, &record.parent_chat_id, parent)?;
            let mut runtime = self.runtime.write().await;
            records.insert(agent_id.clone(), record.clone());
            runtime.insert(
                agent_id.clone(),
                AgentRuntime {
                    abort_flag: abort_flag.clone(),
                    interrupt_notify: Arc::new(Notify::new()),
                    notify: notify.clone(),
                    inbox,
                    pause,
                    native_notes,
                    delivery_state: Arc::new(Mutex::new(RunnerDeliveryState::default())),
                },
            );
        }
        if let Err(error) = self.flush_records(Vec::new()).await {
            let mut records = self.records.write().await;
            let mut runtime = self.runtime.write().await;
            abort_flag.store(true, Ordering::SeqCst);
            if runtime
                .get(&agent_id)
                .is_some_and(|entry| Arc::ptr_eq(&entry.abort_flag, &abort_flag))
            {
                runtime.remove(&agent_id);
            }
            if let Some(record) = records.get_mut(&agent_id) {
                if record.run_generation == 1 && !record.status.is_terminal() {
                    record.status = BgAgentStatus::Failed;
                    record.error = Some(error.clone());
                    record.finished_at = Some(Utc::now());
                    self.settle_terminal(record, Utc::now());
                    touch_record(record, Utc::now());
                }
            }
            notify.notify_waiters();
            return Err(error);
        }
        if abort_flag.load(Ordering::SeqCst) {
            return Err("launch cancelled during registry insertion".into());
        }
        Ok((record, abort_flag, notify))
    }

    pub async fn save_accepted_launch(
        &self,
        agent_id: &str,
        payload: &storage::AcceptedLaunchPayload,
    ) -> Result<(), String> {
        let record = self.get_any(agent_id).await?;
        if record.status != BgAgentStatus::Queued || record.run_generation != 1 {
            return Err(
                "accepted launch can only be installed on the initial queued attempt".into(),
            );
        }
        storage::save_accepted_launch(&self.storage_root, agent_id, payload).await
    }

    pub async fn accepted_launch(
        &self,
        parent_chat_id: &str,
        agent_id: &str,
        max_bytes: usize,
    ) -> Result<storage::AcceptedLaunchPayload, String> {
        let record = self.get_any(agent_id).await?;
        if record.parent_chat_id != parent_chat_id {
            return Err("accepted launch is not visible".into());
        }
        storage::load_accepted_launch(&self.storage_root, agent_id, max_bytes).await
    }

    pub async fn mark_running(
        self: &Arc<Self>,
        agent_id: &str,
        child_chat_id: String,
    ) -> Result<BackgroundAgent, String> {
        self.update_record_if_not_terminal(agent_id, false, |record, now| {
            let starting = record.status == BgAgentStatus::Queued;
            record.status = BgAgentStatus::Running;
            record.child_chat_id = Some(child_chat_id);
            if starting || record.started_at.is_none() {
                record.started_at = Some(now);
            }
            record.finished_at = None;
            record.error = None;
            Ok(())
        })
        .await
    }

    pub async fn claim_stop_hook(&self, agent_id: &str, generation: u64) -> bool {
        let (previous, snapshot) = {
            let mut records = self.records.write().await;
            let Some(record) = records.get(agent_id) else {
                return false;
            };
            if record
                .stop_hook_generation
                .is_some_and(|previous| previous >= generation)
            {
                return false;
            }
            let previous = record.stop_hook_generation;
            records.get_mut(agent_id).unwrap().stop_hook_generation = Some(generation);
            (previous, records.values().cloned().collect())
        };
        if self.flush_records(snapshot).await.is_ok() {
            return true;
        }
        let mut records = self.records.write().await;
        if records
            .get(agent_id)
            .is_some_and(|record| record.stop_hook_generation == Some(generation))
        {
            records.get_mut(agent_id).unwrap().stop_hook_generation = previous;
        }
        false
    }

    pub async fn save_run_config(
        &self,
        agent_id: &str,
        generation: u64,
        payload: &storage::AcceptedLaunchPayload,
    ) -> Result<(), String> {
        let record = self.get_any(agent_id).await?;
        if record.status != BgAgentStatus::Queued || record.run_generation != generation {
            return Err("run configuration requires the matching queued generation".into());
        }
        storage::save_run_config(&self.storage_root, agent_id, generation, payload).await
    }

    pub async fn run_config(
        &self,
        parent_chat_id: &str,
        agent_id: &str,
        generation: u64,
        max_bytes: usize,
    ) -> Result<storage::AcceptedLaunchPayload, String> {
        let record = self.get_any(agent_id).await?;
        if record.parent_chat_id != parent_chat_id || record.run_generation != generation {
            return Err("run configuration is not visible at this generation".into());
        }
        storage::load_run_config(&self.storage_root, agent_id, generation, max_bytes).await
    }

    pub async fn recovery_run_config(
        &self,
        parent_chat_id: &str,
        agent_id: &str,
        current_generation: u64,
        generation: u64,
        max_bytes: usize,
    ) -> Result<
        (
            storage::AcceptedLaunchPayload,
            Option<storage::AcceptedLaunchPayload>,
        ),
        String,
    > {
        let record = self.get(parent_chat_id, agent_id).await?;
        if record.run_generation != current_generation
            || generation == 0
            || generation > current_generation
        {
            return Err("recovery configuration generation does not match".into());
        }
        let config =
            storage::load_run_config(&self.storage_root, agent_id, generation, max_bytes).await?;
        let setup =
            storage::load_checkout_setup(&self.storage_root, agent_id, generation, max_bytes)
                .await?;
        Ok((config, setup))
    }

    pub async fn begin_checkout_setup(
        &self,
        parent_chat_id: &str,
        agent_id: &str,
        generation: u64,
        change_seq: u64,
        abort: &Arc<AtomicBool>,
        parent: Option<&ParentRun>,
        operation: uuid::Uuid,
    ) -> Result<CheckoutSetup, String> {
        let setup = CheckoutSetup {
            record: self.get(parent_chat_id, agent_id).await?,
            abort: abort.clone(),
            parent: parent.cloned(),
            operation,
        };
        if setup.record.run_generation != generation || setup.record.change_seq != change_seq {
            return Err("child changed before checkout setup".into());
        }
        let records = self.records.read().await;
        self.validate_checkout_setup(&records, &setup).await?;
        Ok(setup)
    }

    pub async fn with_checkout_setup<T>(
        &self,
        setup: &CheckoutSetup,
        operation: impl FnOnce(&BackgroundAgent) -> T,
    ) -> Result<T, String> {
        let records = self.records.read().await;
        self.validate_checkout_setup(&records, setup).await?;
        Ok(operation(&setup.record))
    }

    pub async fn finish_queued_resume(
        &self,
        agent_id: &str,
        generation: u64,
        abort: &Arc<AtomicBool>,
        error: String,
        cancelled: bool,
    ) -> Result<BackgroundAgent, String> {
        let mut records = self.records.write().await;
        let mut runtime = self.runtime.write().await;
        let current = records.get(agent_id).ok_or("child unavailable")?;
        if current.run_generation != generation {
            return Err("child moved to another generation".into());
        }
        if current.status.is_terminal() {
            return Ok(current.clone());
        }
        if current.status != BgAgentStatus::Queued
            || !runtime
                .get(agent_id)
                .is_some_and(|run| Arc::ptr_eq(&run.abort_flag, abort))
        {
            return Err("queued child attempt identity changed".into());
        }
        let mut updated = current.clone();
        let now = Utc::now();
        updated.status = if cancelled || abort.load(Ordering::Acquire) {
            BgAgentStatus::Cancelled
        } else {
            BgAgentStatus::Failed
        };
        updated.error = Some(error);
        updated.finished_at = Some(now);
        updated.result_payload_path = None;
        self.settle_terminal(&mut updated, now);
        touch_record(&mut updated, now);
        records.insert(agent_id.to_string(), updated.clone());
        if let Some(run) = runtime.remove(agent_id) {
            run.notify.notify_waiters();
        }
        drop(runtime);
        drop(records);
        self.flush_records(Vec::new()).await?;
        Ok(updated)
    }

    pub async fn rollback_unaccepted_resume(
        &self,
        previous: &BackgroundAgent,
        queued: &BackgroundAgent,
        abort: &Arc<AtomicBool>,
    ) -> Result<bool, String> {
        if !storage::run_config_absent(&self.storage_root, &queued.agent_id, queued.run_generation)
            .await
        {
            return Ok(false);
        }
        let mut records = self.records.write().await;
        let mut runtime = self.runtime.write().await;
        if previous.run_generation.checked_add(1) != Some(queued.run_generation)
            || previous.agent_id != queued.agent_id
            || previous.parent_chat_id != queued.parent_chat_id
            || !previous.status.is_terminal()
            || records.get(&queued.agent_id).is_none_or(|current| {
                current.run_generation != queued.run_generation
                    || current.change_seq != queued.change_seq
                    || current.status != BgAgentStatus::Queued
            })
            || !runtime
                .get(&queued.agent_id)
                .is_some_and(|run| Arc::ptr_eq(&run.abort_flag, abort))
            || abort.load(Ordering::Acquire)
        {
            return Ok(false);
        }
        abort.store(true, Ordering::Release);
        if let Some(run) = runtime.remove(&queued.agent_id) {
            run.notify.notify_waiters();
        }
        let mut restored = previous.clone();
        restored.change_seq = queued.change_seq;
        touch_record(&mut restored, Utc::now());
        records.insert(restored.agent_id.clone(), restored);
        drop(runtime);
        drop(records);
        self.flush_records(Vec::new()).await?;
        Ok(true)
    }

    async fn validate_checkout_setup(
        &self,
        records: &AgentRecords,
        setup: &CheckoutSetup,
    ) -> Result<(), String> {
        validate_parent_run(records, &setup.record.parent_chat_id, setup.parent.as_ref())?;
        let current = records
            .get(&setup.record.agent_id)
            .ok_or("child unavailable")?;
        let runtime = self.runtime.read().await;
        if current.run_generation != setup.record.run_generation
            || current.change_seq != setup.record.change_seq
            || current.status != BgAgentStatus::Queued
            || current.worktree_id != setup.record.worktree_id
            || current.worktree_branch != setup.record.worktree_branch
            || setup.abort.load(Ordering::Acquire)
            || !runtime
                .get(&current.agent_id)
                .is_some_and(|run| Arc::ptr_eq(&run.abort_flag, &setup.abort))
        {
            return Err("child changed or cancelled during checkout setup".into());
        }
        Ok(())
    }

    pub async fn commit_checkout_setup(
        &self,
        setup: &CheckoutSetup,
        worktree_id: String,
        worktree_branch: Option<String>,
        payload: &storage::AcceptedLaunchPayload,
        remember: impl FnOnce(&BackgroundAgent),
    ) -> Result<BackgroundAgent, CheckoutSetupError> {
        let failure = |message, retain_checkout| CheckoutSetupError {
            message,
            retain_checkout,
        };
        {
            let records = self.records.read().await;
            self.validate_checkout_setup(&records, setup)
                .await
                .map_err(|error| failure(error, false))?;
        }
        storage::save_checkout_setup(
            &self.storage_root,
            &setup.record.agent_id,
            setup.record.run_generation,
            payload,
        )
        .await
        .map_err(|error| {
            failure(
                format!("{error}; checkout publication outcome unknown; inspect before recovery"),
                true,
            )
        })?;
        let mut records = self.records.write().await;
        self.validate_checkout_setup(&records, setup)
            .await
            .map_err(|error| {
                failure(
                    format!("{error}; published checkout setup retained; inspect before recovery"),
                    true,
                )
            })?;
        let mut updated = setup.record.clone();
        updated.worktree_id = Some(worktree_id);
        updated.worktree_branch = worktree_branch;
        touch_record(&mut updated, Utc::now());
        records.insert(updated.agent_id.clone(), updated.clone());
        remember(&updated);
        drop(records);
        self.flush_records(Vec::new()).await.map_err(|error| {
            failure(
                format!("{error}; checkout committed in memory; retained for recovery"),
                true,
            )
        })?;
        Ok(updated)
    }

    pub async fn update_progress(
        self: &Arc<Self>,
        agent_id: &str,
        progress: String,
        step_count: u32,
    ) -> Result<BackgroundAgent, String> {
        self.update_record_if_not_terminal(agent_id, true, |record, now| {
            record.progress = Some(progress);
            record.step_count = step_count;
            record.last_activity = Some(now.to_rfc3339());
            Ok(())
        })
        .await
    }

    pub async fn update_activity(
        self: &Arc<Self>,
        agent_id: &str,
        progress: Option<String>,
        step_count: Option<u32>,
        current_tool: Option<Option<String>>,
    ) -> Result<BackgroundAgent, String> {
        self.update_record_if_not_terminal(agent_id, true, |record, now| {
            if let Some(progress) = progress {
                record.progress = Some(progress);
            }
            if let Some(step_count) = step_count {
                record.step_count = step_count;
            }
            if let Some(current_tool) = current_tool {
                record.current_tool = current_tool;
            }
            record.last_activity = Some(now.to_rfc3339());
            Ok(())
        })
        .await
    }

    pub async fn add_usage(
        &self,
        agent_id: &str,
        tokens_delta: u64,
        cost_delta_usd: Option<f64>,
    ) -> Result<BackgroundAgent, String> {
        self.update_record(agent_id, |record, _| {
            record.tokens_used = record.tokens_used.saturating_add(tokens_delta);
            if let Some(cost_delta_usd) = cost_delta_usd {
                record.cost_usd = Some(record.cost_usd.unwrap_or_default() + cost_delta_usd);
            }
            Ok(())
        })
        .await
    }

    pub async fn set_worktree_metadata(
        self: &Arc<Self>,
        agent_id: &str,
        worktree_id: Option<String>,
        worktree_branch: Option<String>,
    ) -> Result<BackgroundAgent, String> {
        self.update_record_if_not_terminal(agent_id, false, |record, _| {
            record.worktree_id = worktree_id;
            record.worktree_branch = worktree_branch;
            Ok(())
        })
        .await
    }

    pub async fn reopen_capped(
        self: &Arc<Self>,
        agent_id: &str,
        parent_chat_id: &str,
        expected_change_seq: u64,
    ) -> Result<(BackgroundAgent, Arc<AtomicBool>, Arc<Notify>), String> {
        self.reopen(agent_id, parent_chat_id, expected_change_seq, true)
            .await
    }

    pub async fn reopen(
        self: &Arc<Self>,
        agent_id: &str,
        parent_chat_id: &str,
        expected_change_seq: u64,
        require_cap: bool,
    ) -> Result<(BackgroundAgent, Arc<AtomicBool>, Arc<Notify>), String> {
        let parent = self.parent_run(parent_chat_id).await?;
        self.reopen_for_parent_run(
            agent_id,
            parent_chat_id,
            expected_change_seq,
            require_cap,
            parent.as_ref(),
        )
        .await
    }

    pub async fn reopen_for_parent_run(
        self: &Arc<Self>,
        agent_id: &str,
        parent_chat_id: &str,
        expected_change_seq: u64,
        require_cap: bool,
        parent: Option<&ParentRun>,
    ) -> Result<(BackgroundAgent, Arc<AtomicBool>, Arc<Notify>), String> {
        let abort_flag = Arc::new(AtomicBool::new(false));
        let notify = Arc::new(Notify::new());
        let (previous, updated) = {
            // Keep the records -> runtime order used by terminal retirement. Publish
            // the claim and its cancellation handle together, before awaiting disk IO.
            let mut records = self.records.write().await;
            validate_parent_run(&records, parent_chat_id, parent)?;
            let previous = records
                .get(agent_id)
                .cloned()
                .ok_or_else(|| "agent not found".to_string())?;
            if previous.parent_chat_id != parent_chat_id {
                return Err("agent is not owned by this parent".into());
            }
            if previous.change_seq != expected_change_seq {
                return Err("agent changed; inspect its current state before resuming".into());
            }
            let recoverable = previous
                .error
                .as_deref()
                .and_then(super::types::CapStop::from_error)
                .is_some_and(|stop| stop.parent_can_raise_steps());
            if require_cap && (previous.status != BgAgentStatus::Failed || !recoverable) {
                return Err("agent is not stopped by a recoverable cap".into());
            }
            if !previous.status.is_terminal() {
                return Err("agent has not stopped".into());
            }
            let mut runtime = self.runtime.write().await;
            if runtime.contains_key(agent_id) {
                return Err("agent's previous runner is still finishing".into());
            }
            let mut updated = previous.clone();
            updated.run_generation = previous
                .run_generation
                .checked_add(1)
                .ok_or("agent run generation overflow")?;
            record_previous_run(&mut updated, &previous);
            updated.status = BgAgentStatus::Queued;
            updated.close_reason = None;
            updated.error = None;
            updated.finished_at = None;
            updated.completion_message_id = None;
            updated.completion_pushed_at = None;
            updated.deferred_at = None;
            updated.result_summary = None;
            updated.result_payload_path = None;
            updated.current_tool = None;
            updated.merge_status = None;
            let mut followups = std::mem::take(&mut updated.followups);
            followups.append(&mut updated.pending_deliveries);
            updated.pending_deliveries = followups;
            touch_record(&mut updated, Utc::now());
            let pause = Arc::new(PauseGate::default());
            pause.set_parent_requested(updated.paused_by.is_some());
            records.insert(agent_id.to_string(), updated.clone());
            runtime.insert(
                agent_id.to_string(),
                AgentRuntime {
                    abort_flag: abort_flag.clone(),
                    notify: notify.clone(),
                    interrupt_notify: Arc::new(Notify::new()),
                    inbox: Arc::new(Mutex::new(VecDeque::new())),
                    pause,
                    native_notes: Arc::new(Mutex::new(VecDeque::new())),
                    delivery_state: Arc::new(Mutex::new(RunnerDeliveryState::default())),
                },
            );
            (previous, updated)
        };
        if let Err(error) = self.flush_records(Vec::new()).await {
            let mut records = self.records.write().await;
            let mut runtime = self.runtime.write().await;
            abort_flag.store(true, Ordering::SeqCst);
            if runtime
                .get(agent_id)
                .is_some_and(|entry| Arc::ptr_eq(&entry.abort_flag, &abort_flag))
            {
                runtime.remove(agent_id);
            }
            // A cancellation or a newer claim wins over this failed disk write.
            if records
                .get(agent_id)
                .is_some_and(|record| record.change_seq == updated.change_seq)
            {
                let mut restored = previous;
                restored.change_seq = updated.change_seq;
                touch_record(&mut restored, Utc::now());
                records.insert(agent_id.to_string(), restored);
            }
            notify.notify_waiters();
            return Err(error);
        }
        // Cancellation may have won while the flush was awaiting its IO lock.
        let records = self.records.read().await;
        if abort_flag.load(Ordering::SeqCst)
            || records
                .get(agent_id)
                .is_none_or(|record| record.status != BgAgentStatus::Queued)
        {
            return Err("agent changed while reopening".into());
        }
        Ok((updated, abort_flag, notify))
    }

    pub async fn set_merge_status(
        &self,
        agent_id: &str,
        status: &str,
    ) -> Result<BackgroundAgent, String> {
        self.set_merge_outcome(agent_id, status, None, None).await
    }

    pub async fn install_deliverable(
        &self,
        agent_id: &str,
        contract: refact_chat_api::DeliverableContract,
    ) -> Result<BackgroundAgent, String> {
        self.update_record(agent_id, |record, _| {
            if record.status != BgAgentStatus::Queued || record.run_generation != 1 {
                return Err(
                    "deliverable contract can only be installed on the initial queued attempt"
                        .into(),
                );
            }
            if record.deliverable.is_some() {
                return Err("deliverable contract already installed".into());
            }
            record.deliverable = Some(contract);
            Ok(())
        })
        .await
    }

    /// Records which reusable profile this child was launched from. Grouping
    /// data only: it confers no authority and cannot change after acceptance.
    pub async fn install_profile_name(
        &self,
        agent_id: &str,
        profile_name: String,
    ) -> Result<BackgroundAgent, String> {
        self.update_record(agent_id, |record, _| {
            if record.status != BgAgentStatus::Queued || record.run_generation != 1 {
                return Err(
                    "profile name can only be installed on the initial queued attempt".into(),
                );
            }
            record.profile_name = Some(profile_name);
            Ok(())
        })
        .await
    }

    pub async fn set_merge_outcome(
        &self,
        agent_id: &str,
        status: &str,
        conflict_summary: Option<String>,
        error: Option<String>,
    ) -> Result<BackgroundAgent, String> {
        if !matches!(
            status,
            "pending" | "merged" | "conflict" | "skipped" | "failed"
        ) {
            return Err("invalid merge status".to_string());
        }
        let status = status.to_string();
        self.update_record(agent_id, |record, _| {
            record.merge_status = Some(status);
            if let Some(conflict_summary) = conflict_summary {
                record.conflict_summary = Some(conflict_summary);
            }
            if let Some(error) = error {
                record.error = Some(match record.error.take() {
                    Some(existing) => format!("{}\n{}", existing, error),
                    None => error,
                });
            }
            Ok(())
        })
        .await
    }

    pub async fn add_question(
        &self,
        agent_id: &str,
        text: String,
    ) -> Result<(BackgroundAgent, String), String> {
        let question_id = Uuid::new_v4().simple().to_string()[..8].to_string();
        let answerer = {
            let records = self.records.read().await;
            let record = records
                .get(agent_id)
                .ok_or_else(|| "agent not found".to_string())?;
            if record.status.is_terminal() {
                return Err("agent already finished".to_string());
            }
            record.parent_chat_id.clone()
        };
        let question = self
            .questions
            .ask(
                AskQuestion::new(
                    QuestionKind::AgentParent,
                    agent_id,
                    answerer,
                    format!("{agent_id}:{question_id}"),
                    text,
                )
                .with_id(&question_id),
                Utc::now(),
            )
            .map_err(|error| error.to_string())?;
        let updated = self
            .update_record(agent_id, |record, _| {
                if record.status.is_terminal() {
                    return Err("agent already finished".to_string());
                }
                record.questions.push(question);
                Ok(())
            })
            .await;
        match updated {
            Ok(updated) => Ok((updated, question_id)),
            Err(error) => {
                let _ = self.questions.cancel(&question_id, agent_id, Utc::now());
                Err(error)
            }
        }
    }

    pub async fn answer_question(
        &self,
        agent_id: &str,
        question_id: &str,
        answer: String,
    ) -> Result<BackgroundAgent, String> {
        let answerer = {
            let records = self.records.read().await;
            let record = records
                .get(agent_id)
                .ok_or_else(|| "agent not found".to_string())?;
            if record.status.is_terminal() {
                return Err(
                    "question was withdrawn when the agent finished; nothing was delivered".into(),
                );
            }
            record.parent_chat_id.clone()
        };
        let settled = match self
            .questions
            .answer(question_id, &answerer, answer, Utc::now())
        {
            Ok(settled) => settled,
            Err(refact_chat_api::QuestionError::AlreadyAnswered) => {
                return Err("question already answered".to_string())
            }
            Err(refact_chat_api::QuestionError::NotFound) => {
                return Err("question not found".to_string())
            }
            Err(error) => return Err(error.to_string()),
        };
        self.update_record(agent_id, |record, _| {
            let question = record
                .questions
                .iter_mut()
                .find(|question| question.id == settled.id)
                .ok_or_else(|| "question not found".to_string())?;
            *question = settled;
            Ok(())
        })
        .await
    }

    pub async fn mark_completed(
        &self,
        agent_id: &str,
        completion: AgentCompletion,
    ) -> Result<BackgroundAgent, String> {
        let updated = {
            let mut records = self.records.write().await;
            let current = records
                .get(agent_id)
                .cloned()
                .ok_or_else(|| "agent not found".to_string())?;
            if current.status.is_terminal() {
                return Ok(current);
            }
            let AgentCompletion {
                result_summary,
                edited_files,
                diff_summary,
                conflict_summary,
                child_chat_id,
                deliverable_status,
                handoff,
            } = completion;
            let payload = serde_json::json!({
                "agent_id": agent_id,
                "result_summary": result_summary.clone(),
                "edited_files": edited_files.clone(),
                "diff_summary": diff_summary.clone(),
                "conflict_summary": conflict_summary.clone(),
                "child_chat_id": child_chat_id.clone(),
                "deliverable_status": deliverable_status.clone(),
                "handoff": handoff.clone(),
            });
            let result_payload_path =
                storage::save_result_payload(&self.storage_root, agent_id, &payload).await?;
            let mut updated = current;
            let now = Utc::now();
            updated.status = BgAgentStatus::Completed;
            updated.result_summary = Some(result_summary);
            updated.result_payload_path = Some(result_payload_path);
            updated.edited_files = edited_files;
            updated.diff_summary = diff_summary;
            updated.conflict_summary = conflict_summary;
            updated.deliverable_status = deliverable_status;
            updated.handoff = handoff;
            updated.current_tool = None;
            if child_chat_id.is_some() {
                updated.child_chat_id = child_chat_id;
            }
            updated.error = None;
            updated.finished_at = Some(now);
            self.settle_terminal(&mut updated, now);
            touch_record(&mut updated, now);
            records.insert(agent_id.to_string(), updated.clone());
            let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
            drop(records);
            self.flush_records(snapshot).await?;
            updated
        };
        if let Some(notify) = self.notify_for(agent_id).await {
            notify.notify_waiters();
        }
        self.retire_runtime(agent_id, updated.run_generation).await;
        Ok(updated)
    }

    pub async fn mark_failed(
        &self,
        agent_id: &str,
        error: String,
    ) -> Result<BackgroundAgent, String> {
        let updated = {
            let mut records = self.records.write().await;
            let current = records
                .get(agent_id)
                .cloned()
                .ok_or_else(|| "agent not found".to_string())?;
            if current.status.is_terminal() {
                return Ok(current);
            }
            let payload = terminal_result_payload("failed", &error, &current);
            let result_payload_path =
                storage::save_result_payload(&self.storage_root, agent_id, &payload).await?;
            let mut updated = current;
            let now = Utc::now();
            updated.status = BgAgentStatus::Failed;
            if let Some(stop) = super::types::CapStop::from_error(&error) {
                if matches!(
                    stop.key,
                    super::types::CapKind::Steps | super::types::CapKind::GoalTurns
                ) {
                    updated.step_count = updated
                        .step_count
                        .max(u32::try_from(stop.used).unwrap_or(u32::MAX));
                }
            }
            updated.error = Some(error);
            updated.current_tool = None;
            updated.result_payload_path = Some(result_payload_path);
            updated.finished_at = Some(now);
            self.settle_terminal(&mut updated, now);
            touch_record(&mut updated, now);
            records.insert(agent_id.to_string(), updated.clone());
            let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
            drop(records);
            self.flush_records(snapshot).await?;
            updated
        };
        if let Some(notify) = self.notify_for(agent_id).await {
            notify.notify_waiters();
        }
        self.retire_runtime(agent_id, updated.run_generation).await;
        Ok(updated)
    }

    pub async fn mark_cancelled(
        &self,
        agent_id: &str,
        reason: Option<String>,
    ) -> Result<BackgroundAgent, String> {
        let updated = {
            let mut records = self.records.write().await;
            let current = records
                .get(agent_id)
                .cloned()
                .ok_or_else(|| "agent not found".to_string())?;
            if current.status.is_terminal() {
                return Ok(current);
            }
            let payload_error = reason
                .clone()
                .unwrap_or_else(|| "Agent was cancelled.".to_string());
            let payload = terminal_result_payload("cancelled", &payload_error, &current);
            let result_payload_path =
                storage::save_result_payload(&self.storage_root, agent_id, &payload).await?;
            let mut updated = current;
            let now = Utc::now();
            updated.status = BgAgentStatus::Cancelled;
            updated.error = reason;
            updated.current_tool = None;
            updated.result_payload_path = Some(result_payload_path);
            updated.finished_at = Some(now);
            self.settle_terminal(&mut updated, now);
            touch_record(&mut updated, now);
            records.insert(agent_id.to_string(), updated.clone());
            let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
            drop(records);
            self.flush_records(snapshot).await?;
            updated
        };
        if let Some(notify) = self.notify_for(agent_id).await {
            notify.notify_waiters();
        }
        self.retire_runtime(agent_id, updated.run_generation).await;
        Ok(updated)
    }

    pub async fn mark_interrupted(
        &self,
        agent_id: &str,
        reason: String,
    ) -> Result<BackgroundAgent, String> {
        let updated = {
            let mut records = self.records.write().await;
            let current = records
                .get(agent_id)
                .cloned()
                .ok_or_else(|| "agent not found".to_string())?;
            if current.status.is_terminal() {
                return Ok(current);
            }
            let mut updated = current;
            let now = Utc::now();
            updated.status = BgAgentStatus::Interrupted;
            updated.error = Some(reason);
            updated.current_tool = None;
            updated.finished_at = Some(now);
            self.settle_terminal(&mut updated, now);
            touch_record(&mut updated, now);
            records.insert(agent_id.to_string(), updated.clone());
            let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
            drop(records);
            self.flush_records(snapshot).await?;
            updated
        };
        if let Some(notify) = self.notify_for(agent_id).await {
            notify.notify_waiters();
        }
        self.retire_runtime(agent_id, updated.run_generation).await;
        Ok(updated)
    }

    /// Close this child: terminal, but its durable record and saved
    /// configuration stay resumable. Returns whether it was already closed.
    pub async fn mark_closed(
        &self,
        agent_id: &str,
        expected_run_generation: Option<u64>,
        reason: Option<String>,
    ) -> Result<(BackgroundAgent, bool), String> {
        self.mark_closed_inner(agent_id, None, expected_run_generation, reason, |_| {})
            .await
    }

    async fn mark_closed_inner(
        &self,
        agent_id: &str,
        parent_chat_id: Option<&str>,
        expected_run_generation: Option<u64>,
        reason: Option<String>,
        on_closed: impl FnOnce(&BackgroundAgent),
    ) -> Result<(BackgroundAgent, bool), String> {
        let updated = {
            let mut records = self.records.write().await;
            let current = records
                .get(agent_id)
                .filter(|record| {
                    parent_chat_id.is_none_or(|parent| record.parent_chat_id == parent)
                })
                .cloned()
                .ok_or_else(|| "agent not found".to_string())?;
            if expected_run_generation
                .is_some_and(|generation| generation != current.run_generation)
            {
                return Err(
                    "agent moved to another generation; inspect its current state before closing"
                        .to_string(),
                );
            }
            if current.status == BgAgentStatus::Closed {
                return Ok((current, true));
            }
            if current.status.is_terminal() {
                return Err(format!(
                    "agent already finished as {}; only a live agent can be closed",
                    current.status.as_str()
                ));
            }
            let runtime = self.runtime.read().await;
            let mut updated = current;
            let now = Utc::now();
            updated.status = BgAgentStatus::Closed;
            updated.close_reason = reason;
            updated.current_tool = None;
            updated.finished_at = Some(now);
            self.settle_terminal(&mut updated, now);
            touch_record(&mut updated, now);
            records.insert(agent_id.to_string(), updated.clone());
            if let Some(runtime) = runtime.get(agent_id) {
                runtime.abort_flag.store(true, Ordering::SeqCst);
                runtime.pause.wake_for_abort();
                runtime.notify.notify_waiters();
                runtime.interrupt_notify.notify_waiters();
            }
            on_closed(&updated);
            updated
        };
        let persisted = self.flush_records(Vec::new()).await;
        self.retire_runtime(agent_id, updated.run_generation).await;
        persisted?;
        Ok((updated, false))
    }

    /// Parent-scoped close: stops the runner, then retires the record as closed.
    pub async fn close(
        &self,
        parent_chat_id: &str,
        agent_id: &str,
        expected_run_generation: Option<u64>,
        reason: Option<String>,
    ) -> Result<(BackgroundAgent, bool), String> {
        self.close_with(
            parent_chat_id,
            agent_id,
            expected_run_generation,
            reason,
            |_| {},
        )
        .await
    }

    pub async fn close_with(
        &self,
        parent_chat_id: &str,
        agent_id: &str,
        expected_run_generation: Option<u64>,
        reason: Option<String>,
        on_closed: impl FnOnce(&BackgroundAgent),
    ) -> Result<(BackgroundAgent, bool), String> {
        self.mark_closed_inner(
            agent_id,
            Some(parent_chat_id),
            expected_run_generation,
            reason,
            on_closed,
        )
        .await
    }

    pub async fn mark_waiting_for_approval(
        self: &Arc<Self>,
        agent_id: &str,
    ) -> Result<BackgroundAgent, String> {
        self.update_record_if_not_terminal(agent_id, false, |record, _| {
            record.status = BgAgentStatus::WaitingForApproval;
            Ok(())
        })
        .await
    }

    pub async fn set_completion_message_id(
        &self,
        agent_id: &str,
        message_id: String,
    ) -> Result<(), String> {
        self.set_completion_message_id_for_run(agent_id, None, message_id)
            .await
    }

    pub async fn set_completion_message_id_for_run(
        &self,
        agent_id: &str,
        run_generation: Option<u64>,
        message_id: String,
    ) -> Result<(), String> {
        {
            let mut records = self.records.write().await;
            let current = records
                .get(agent_id)
                .cloned()
                .ok_or_else(|| "agent not found".to_string())?;
            // An old terminal delivery may finish after capped recovery. It must
            // not attach the previous attempt's correlation to the live attempt.
            if !current.status.is_terminal()
                || run_generation.is_some_and(|generation| generation != current.run_generation)
            {
                return Ok(());
            }
            if let Some(current_id) = current.completion_message_id.as_deref() {
                if current_id != "pending" && current_id != "deferred" {
                    return Ok(());
                }
                if current_id == message_id && current_id != "deferred" {
                    return Ok(());
                }
            }
            if matches!(
                (
                    current.completion_message_id.as_deref(),
                    message_id.as_str()
                ),
                (Some("deferred"), "pending")
            ) {
                return Ok(());
            }
            let mut updated = current;
            let now = Utc::now();
            let pushed = message_id != "pending" && message_id != "deferred";
            let deferred = message_id == "deferred";
            updated.completion_message_id = Some(message_id);
            updated.completion_pushed_at = pushed.then_some(now);
            updated.deferred_at = deferred.then_some(now);
            touch_record(&mut updated, now);
            records.insert(agent_id.to_string(), updated);
            let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
            drop(records);
            self.flush_records(snapshot).await?;
        }
        if let Some(notify) = self.notify_for(agent_id).await {
            notify.notify_waiters();
        }
        Ok(())
    }

    pub async fn count_active_for_parent_root(&self, parent_root_chat_id: &str) -> usize {
        self.records
            .read()
            .await
            .values()
            .filter(|record| {
                record.parent_root_chat_id.as_deref() == Some(parent_root_chat_id)
                    || record.parent_chat_id == parent_root_chat_id
            })
            .filter(|record| !record.status.is_terminal())
            .count()
    }

    pub async fn list_for_parent(
        &self,
        parent_chat_id: &str,
        filter: AgentListFilter,
    ) -> Vec<BackgroundAgent> {
        let cutoff = terminal_cutoff(filter.include_terminal_within_hours.unwrap_or(24));
        let mut records: Vec<BackgroundAgent> = self
            .records
            .read()
            .await
            .values()
            .filter(|record| record.parent_chat_id == parent_chat_id)
            .filter(|record| filter.kind.map_or(true, |kind| record.kind == kind))
            .filter(|record| {
                filter
                    .status
                    .as_ref()
                    .map_or(true, |statuses| statuses.contains(&record.status))
            })
            .filter(|record| should_include_record(record, cutoff))
            .cloned()
            .collect();
        records.sort_by(|a, b| {
            b.last_update_at
                .cmp(&a.last_update_at)
                .then(b.created_at.cmp(&a.created_at))
                .then(a.agent_id.cmp(&b.agent_id))
        });
        if let Some(limit) = filter.limit {
            records.truncate(limit);
        }
        records
    }

    pub async fn list_all(&self) -> Vec<BackgroundAgent> {
        #[cfg(any(test, feature = "test-support"))]
        self.list_all_calls.fetch_add(1, Ordering::Relaxed);
        let mut records: Vec<BackgroundAgent> =
            self.records.read().await.values().cloned().collect();
        records.sort_by(|a, b| {
            b.last_update_at
                .cmp(&a.last_update_at)
                .then(b.created_at.cmp(&a.created_at))
                .then(a.agent_id.cmp(&b.agent_id))
        });
        records
    }

    pub async fn find_agent_id_by_child_chat_id(&self, chat_id: &str) -> Option<String> {
        self.records.read().await.by_child.get(chat_id).cloned()
    }

    pub async fn rebind_parent(
        &self,
        child_chat_id: &str,
        expected_parent: &str,
        new_parent: &str,
    ) -> Result<(), String> {
        let Some(agent_id) = self.find_agent_id_by_child_chat_id(child_chat_id).await else {
            return Ok(());
        };
        self.update_record(&agent_id, |record, _| {
            if record.child_chat_id.as_deref() != Some(child_chat_id)
                || record.parent_chat_id != expected_parent
            {
                return Err("Agent parent changed during custody transfer".into());
            }
            record.parent_chat_id = new_parent.to_string();
            record.parent_root_chat_id = Some(new_parent.to_string());
            Ok(())
        })
        .await
        .map(|_| ())
    }

    pub async fn child_chat_status(&self, chat_id: &str) -> Option<(String, BgAgentStatus, u64)> {
        let records = self.records.read().await;
        let agent_id = records.by_child.get(chat_id)?;
        let record = records.by_id.get(agent_id)?;
        Some((agent_id.clone(), record.status, record.run_generation))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn list_all_calls_for_test(&self) -> usize {
        self.list_all_calls.load(Ordering::Relaxed)
    }

    pub fn try_child_chat_index(&self) -> Option<HashMap<String, String>> {
        self.records.try_read().ok().map(|records| {
            records
                .values()
                .filter_map(|record| {
                    record
                        .child_chat_id
                        .as_ref()
                        .map(|chat_id| (chat_id.clone(), record.agent_id.clone()))
                })
                .collect()
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn records_write_guard_for_test(
        &self,
    ) -> tokio::sync::RwLockMappedWriteGuard<'_, HashMap<String, BackgroundAgent>> {
        tokio::sync::RwLockWriteGuard::map(self.records.write().await, |records| &mut records.by_id)
    }

    pub async fn list_descendants(&self, agent_id: &str) -> Vec<BackgroundAgent> {
        let records = self.records.read().await;
        let Some(root) = records.get(agent_id) else {
            return Vec::new();
        };
        let Some(child_chat_id) = root.child_chat_id.clone() else {
            return Vec::new();
        };
        let mut pending_chat_ids = VecDeque::from([child_chat_id]);
        let mut visited_chat_ids = HashSet::new();
        let mut visited_agent_ids = HashSet::from([agent_id.to_string()]);
        let mut descendants = Vec::new();
        while let Some(parent_chat_id) = pending_chat_ids.pop_front() {
            if !visited_chat_ids.insert(parent_chat_id.clone()) {
                continue;
            }
            let mut children: Vec<BackgroundAgent> = records
                .values()
                .filter(|record| record.parent_chat_id == parent_chat_id)
                .filter(|record| visited_agent_ids.insert(record.agent_id.clone()))
                .cloned()
                .collect();
            children.sort_by(|a, b| {
                a.created_at
                    .cmp(&b.created_at)
                    .then(a.agent_id.cmp(&b.agent_id))
            });
            for child in children {
                if let Some(child_chat_id) = child.child_chat_id.clone() {
                    pending_chat_ids.push_back(child_chat_id);
                }
                descendants.push(child);
            }
        }
        descendants
    }

    pub async fn list_with_completion_message_id(&self, ids: &[&str]) -> Vec<BackgroundAgent> {
        let wanted: HashSet<&str> = ids.iter().copied().collect();
        let mut records: Vec<BackgroundAgent> = self
            .records
            .read()
            .await
            .values()
            .filter(|record| {
                record
                    .completion_message_id
                    .as_deref()
                    .map_or(false, |id| wanted.contains(id))
            })
            .cloned()
            .collect();
        records.sort_by(|a, b| {
            b.last_update_at
                .cmp(&a.last_update_at)
                .then(b.created_at.cmp(&a.created_at))
                .then(a.agent_id.cmp(&b.agent_id))
        });
        records
    }

    pub async fn has_runtime(&self, agent_id: &str) -> bool {
        self.runtime.read().await.contains_key(agent_id)
    }

    #[doc(hidden)]
    pub async fn set_last_update_at_for_test(
        &self,
        agent_id: &str,
        last_update_at: DateTime<Utc>,
    ) -> Result<(), String> {
        let mut records = self.records.write().await;
        let record = records
            .get_mut(agent_id)
            .ok_or_else(|| "agent not found".to_string())?;
        record.last_update_at = last_update_at;
        if record.finished_at.is_some() {
            record.finished_at = Some(last_update_at);
        }
        let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
        drop(records);
        self.flush_records(snapshot).await
    }

    #[doc(hidden)]
    pub async fn set_deferred_at_for_test(
        &self,
        agent_id: &str,
        deferred_at: DateTime<Utc>,
    ) -> Result<(), String> {
        let mut records = self.records.write().await;
        let record = records
            .get_mut(agent_id)
            .ok_or_else(|| "agent not found".to_string())?;
        record.deferred_at = Some(deferred_at);
        let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
        drop(records);
        self.flush_records(snapshot).await
    }

    #[doc(hidden)]
    pub async fn clear_result_summary_for_test(&self, agent_id: &str) -> Result<(), String> {
        let mut records = self.records.write().await;
        let record = records
            .get_mut(agent_id)
            .ok_or_else(|| "agent not found".to_string())?;
        record.result_summary = None;
        let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
        drop(records);
        self.flush_records(snapshot).await
    }

    pub async fn get(
        &self,
        parent_chat_id: &str,
        agent_id: &str,
    ) -> Result<BackgroundAgent, String> {
        let records = self.records.read().await;
        scoped_record(&records, parent_chat_id, agent_id)
    }

    pub async fn get_any(&self, agent_id: &str) -> Result<BackgroundAgent, String> {
        self.records
            .read()
            .await
            .get(agent_id)
            .cloned()
            .ok_or_else(|| "agent not found".to_string())
    }

    pub async fn wait(
        &self,
        parent_chat_id: &str,
        agent_id: &str,
        timeout: Duration,
    ) -> Result<BackgroundAgent, String> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(Instant::now);
        loop {
            let record = self.get(parent_chat_id, agent_id).await?;
            if record.status.is_terminal() || timeout.is_zero() {
                return Ok(record);
            }
            let Some(runtime) = self.runtime.read().await.get(agent_id).cloned() else {
                return Ok(record);
            };
            let notified = runtime.notify.notified();
            let record = self.get(parent_chat_id, agent_id).await?;
            if record.status.is_terminal() {
                return Ok(record);
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(record);
            }
            if tokio::time::timeout(deadline - now, notified)
                .await
                .is_err()
            {
                return self.get(parent_chat_id, agent_id).await;
            }
        }
    }

    pub async fn cancel(
        &self,
        parent_chat_id: &str,
        agent_id: &str,
        reason: Option<String>,
    ) -> Result<BackgroundAgent, String> {
        self.cancel_subtree(parent_chat_id, agent_id, false, reason)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| "agent not found".into())
    }

    pub async fn cancel_subtree(
        &self,
        parent_chat_id: &str,
        agent_id: &str,
        subtree: bool,
        reason: Option<String>,
    ) -> Result<Vec<BackgroundAgent>, String> {
        self.cancel_subtree_with_outcome(parent_chat_id, agent_id, subtree, reason)
            .await
            .map(|(records, _)| records)
    }

    pub async fn cancel_subtree_with_outcome(
        &self,
        parent_chat_id: &str,
        agent_id: &str,
        subtree: bool,
        reason: Option<String>,
    ) -> Result<(Vec<BackgroundAgent>, HashSet<String>), String> {
        let (mut updated, cancelled) = {
            let mut records = self.records.write().await;
            let root = scoped_record(&records, parent_chat_id, agent_id)?;
            let mut selected = vec![root];
            let mut seen = HashSet::from([agent_id.to_string()]);
            let mut index = 0;
            while subtree && index < selected.len() {
                if let Some(chat_id) = selected[index].child_chat_id.as_deref() {
                    let mut children: Vec<_> = records
                        .values()
                        .filter(|record| record.parent_chat_id == chat_id)
                        .filter(|record| seen.insert(record.agent_id.clone()))
                        .cloned()
                        .collect();
                    children.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
                    selected.extend(children);
                }
                index += 1;
            }
            let runtime = self.runtime.read().await;
            let mut cancelled = HashSet::new();
            for record in &mut selected {
                if subtree {
                    records
                        .cancelled_subtrees
                        .insert(record.agent_id.clone(), record.run_generation);
                }
                if record.status.is_terminal() {
                    continue;
                }
                if let Some(runtime) = runtime.get(&record.agent_id) {
                    runtime.abort_flag.store(true, Ordering::SeqCst);
                    runtime.pause.wake_for_abort();
                    runtime.notify.notify_waiters();
                    runtime.interrupt_notify.notify_waiters();
                }
                let now = Utc::now();
                record.status = BgAgentStatus::Cancelled;
                record.error = reason.clone();
                record.current_tool = None;
                record.finished_at = Some(now);
                self.settle_terminal(record, now);
                touch_record(record, now);
                records.insert(record.agent_id.clone(), record.clone());
                cancelled.insert(record.agent_id.clone());
            }
            (selected, cancelled)
        };
        let mut persistence_error = None;
        for record in &mut updated {
            if !cancelled.contains(&record.agent_id) {
                continue;
            }
            let error = reason.as_deref().unwrap_or("Agent was cancelled.");
            let payload = terminal_result_payload("cancelled", error, record);
            let path =
                match storage::save_result_payload(&self.storage_root, &record.agent_id, &payload)
                    .await
                {
                    Ok(path) => path,
                    Err(error) => {
                        persistence_error.get_or_insert(error);
                        continue;
                    }
                };
            let mut records = self.records.write().await;
            if let Some(current) = records.get_mut(&record.agent_id) {
                if current.run_generation == record.run_generation
                    && current.status == BgAgentStatus::Cancelled
                {
                    current.result_payload_path = Some(path.clone());
                    record.result_payload_path = Some(path);
                }
            }
        }
        if let Err(error) = self.flush_records(Vec::new()).await {
            persistence_error.get_or_insert(error);
        }
        {
            let records = self.records.read().await;
            let mut runtime = self.runtime.write().await;
            for record in &updated {
                if cancelled.contains(&record.agent_id)
                    && records.get(&record.agent_id).is_some_and(|current| {
                        current.run_generation == record.run_generation
                            && current.status == BgAgentStatus::Cancelled
                    })
                {
                    runtime.remove(&record.agent_id);
                }
            }
        }
        match persistence_error {
            Some(error) => Err(error),
            None => Ok((updated, cancelled)),
        }
    }

    pub async fn abort_flag(&self, agent_id: &str) -> Option<Arc<AtomicBool>> {
        self.runtime
            .read()
            .await
            .get(agent_id)
            .map(|runtime| runtime.abort_flag.clone())
    }

    pub async fn set_completion_push(&self, agent_id: &str, push: PushMode) -> Result<(), String> {
        self.update_record(agent_id, |record, _| {
            record.completion_push = push;
            Ok(())
        })
        .await
        .map(|_| ())
    }

    pub async fn interrupt_notify(&self, agent_id: &str) -> Option<Arc<Notify>> {
        self.runtime
            .read()
            .await
            .get(agent_id)
            .map(|runtime| runtime.interrupt_notify.clone())
    }

    pub async fn pending_deliveries(&self, agent_id: &str) -> Vec<PendingDelivery> {
        self.records
            .read()
            .await
            .get(agent_id)
            .map(|record| record.pending_deliveries.clone())
            .unwrap_or_default()
    }

    pub async fn has_preempt(&self, agent_id: &str) -> bool {
        let runtime = self.runtime.read().await.get(agent_id).cloned();
        let Some(runtime) = runtime else { return false };
        let state = runtime.delivery_state.lock().await;
        self.records
            .read()
            .await
            .get(agent_id)
            .map(|record| {
                record.pending_deliveries.iter().any(|delivery| {
                    delivery.push == PushMode::Preempt && !state.claimed.contains(&delivery.id)
                })
            })
            .unwrap_or(false)
    }

    pub async fn enqueue_delivery(
        &self,
        agent_id: &str,
        delivery: PendingDelivery,
    ) -> Result<DeliveryOutcome, String> {
        self.enqueue_delivery_inner(agent_id, delivery, false)
            .await
            .map(|receipt| receipt.outcome)
    }

    pub async fn enqueue_agent_message(
        &self,
        agent_id: &str,
        delivery: PendingDelivery,
    ) -> Result<AgentMessageReceipt, String> {
        self.enqueue_delivery_inner(agent_id, delivery, true).await
    }

    async fn enqueue_delivery_inner(
        &self,
        agent_id: &str,
        mut delivery: PendingDelivery,
        allow_followup: bool,
    ) -> Result<AgentMessageReceipt, String> {
        let runtime = self.runtime.read().await.get(agent_id).cloned();
        let state = match &runtime {
            Some(runtime) => Some(runtime.delivery_state.lock().await),
            None => None,
        };
        let mut receipt = AgentMessageReceipt {
            outcome: DeliveryOutcome::Queued,
            finished_status: None,
            dropped_followups: Vec::new(),
        };
        let preempt = delivery.push == PushMode::Preempt;
        let claimed = state
            .as_ref()
            .map(|state| state.claimed.clone())
            .unwrap_or_default();
        let settings = refact_work_core::coordination_settings();
        delivery.ensure_ttl_secs(now_ms(), settings.delivery_default_ttl_secs);
        self.update_record(agent_id, |record, now| {
            if record.delivery_ids.contains(&delivery.id)
                || record
                    .followups
                    .iter()
                    .any(|pending| pending.id == delivery.id)
                || record
                    .dropped_deliveries
                    .iter()
                    .any(|dropped| dropped.id == delivery.id)
                || record
                    .pending_deliveries
                    .iter()
                    .any(|pending| pending.id == delivery.id)
            {
                receipt.outcome = DeliveryOutcome::Duplicate;
                return Ok(());
            }
            let terminal = record.status.is_terminal();
            if terminal && !allow_followup {
                return Err("agent already finished".to_string());
            }
            if !terminal {
                let state = state
                    .as_ref()
                    .ok_or("agent is not running in this process")?;
                if state.sealed {
                    return Err("agent delivery turn already finished".to_string());
                }
            }
            if delivery.is_expired_at(now.timestamp_millis().max(0) as u64) {
                record_dropped_delivery(record, delivery.id.clone(), DeliveryStatus::Expired);
                receipt.outcome = DeliveryOutcome::Expired;
                return Ok(());
            }
            if terminal {
                if followup_bytes(&delivery) > settings.agent_followup_max_bytes {
                    return Err("message exceeds the configured follow-up byte limit".into());
                }
                record.followups.push(delivery);
                receipt.dropped_followups = trim_followups(record);
                receipt.finished_status = Some(record.status);
                return Ok(());
            }
            if delivery.supersede_key.is_some() {
                let superseded: Vec<String> = record
                    .pending_deliveries
                    .iter()
                    .filter(|pending| {
                        delivery.supersedes(pending) && !claimed.contains(&pending.id)
                    })
                    .map(|pending| pending.id.clone())
                    .collect();
                record
                    .pending_deliveries
                    .retain(|pending| !superseded.contains(&pending.id));
                for id in superseded {
                    record_dropped_delivery(record, id, DeliveryStatus::Superseded);
                }
            }
            if record.pending_deliveries.len() >= settings.delivery_max_pending {
                return Err("agent delivery queue is full".to_string());
            }
            record.pending_deliveries.push(delivery);
            Ok(())
        })
        .await?;
        if preempt
            && receipt.outcome == DeliveryOutcome::Queued
            && receipt.finished_status.is_none()
        {
            if let Some(runtime) = &runtime {
                runtime.interrupt_notify.notify_one();
            }
        }
        Ok(receipt)
    }

    pub async fn update_pending_delivery(
        &self,
        agent_id: &str,
        id: &str,
        push: Option<PushMode>,
        cancel: bool,
    ) -> Result<(), String> {
        let runtime = self.runtime.read().await.get(agent_id).cloned();
        let state = match runtime.as_ref() {
            Some(runtime) => Some(runtime.delivery_state.lock().await),
            None => None,
        };
        if state
            .as_ref()
            .is_some_and(|state| state.claimed.contains(id))
        {
            return Err("delivery is already being applied".to_string());
        }
        self.update_record(agent_id, |record, _| {
            let index = record
                .pending_deliveries
                .iter()
                .position(|delivery| delivery.id == id)
                .ok_or_else(|| "pending delivery not found".to_string())?;
            if cancel {
                let cancelled = record.pending_deliveries.remove(index);
                record_dropped_delivery(record, cancelled.id, DeliveryStatus::Failed);
            } else if let Some(push) = push {
                record.pending_deliveries[index].push = push;
            }
            Ok(())
        })
        .await?;
        if !cancel && push == Some(PushMode::Preempt) {
            if let Some(notify) = self.interrupt_notify(agent_id).await {
                notify.notify_one();
            }
        }
        Ok(())
    }

    pub async fn acknowledge_delivery(&self, agent_id: &str, id: &str) -> Result<(), String> {
        self.update_record(agent_id, |record, _| {
            record
                .pending_deliveries
                .retain(|delivery| delivery.id != id);
            record.followups.retain(|delivery| delivery.id != id);
            if !record.delivery_ids.iter().any(|delivered| delivered == id) {
                record.delivery_ids.push(id.to_string());
            }
            let retention = refact_work_core::coordination_settings().delivery_status_retention;
            if record.delivery_ids.len() > retention {
                record
                    .delivery_ids
                    .drain(..record.delivery_ids.len() - retention);
            }
            Ok(())
        })
        .await
        .map(|_| ())
    }

    pub async fn consume_delivery(&self, agent_id: &str, id: &str) -> Result<(), String> {
        self.update_record(agent_id, |record, _| {
            let was_pending = record
                .pending_deliveries
                .iter()
                .chain(&record.followups)
                .any(|delivery| delivery.id == id);
            let was_delivered = record.delivery_ids.iter().any(|delivered| delivered == id);
            if !was_pending && !was_delivered {
                return Err("delivery was not accepted by this agent".to_string());
            }
            record
                .pending_deliveries
                .retain(|delivery| delivery.id != id);
            record.followups.retain(|delivery| delivery.id != id);
            record.delivery_ids.retain(|delivered| delivered != id);
            record_dropped_delivery(record, id.to_string(), DeliveryStatus::Consumed);
            Ok(())
        })
        .await
        .map(|_| ())
    }

    /// Lifecycle of one delivery id for this runner: pending, consumed,
    /// dropped before consumption, or never seen.
    pub async fn delivery_status(&self, agent_id: &str, id: &str) -> DeliveryStatus {
        let records = self.records.read().await;
        let Some(record) = records.get(agent_id) else {
            return DeliveryStatus::Unknown;
        };
        if record
            .pending_deliveries
            .iter()
            .chain(&record.followups)
            .any(|pending| pending.id == id)
        {
            return DeliveryStatus::Pending;
        }
        if let Some(status) = record
            .dropped_deliveries
            .iter()
            .rev()
            .find(|dropped| dropped.id == id)
            .map(|dropped| dropped.status)
        {
            return status;
        }
        record
            .delivery_ids
            .iter()
            .any(|known| known == id)
            .then_some(DeliveryStatus::Delivered)
            .unwrap_or(DeliveryStatus::Unknown)
    }

    async fn claim_pending_deliveries(
        &self,
        agent_id: &str,
        seal: bool,
        filter: impl Fn(&PendingDelivery) -> bool,
    ) -> Result<Vec<PendingDelivery>, String> {
        let runtime = self
            .runtime
            .read()
            .await
            .get(agent_id)
            .cloned()
            .ok_or_else(|| "agent is not running in this process".to_string())?;
        let mut state = runtime.delivery_state.lock().await;
        let claimed = state.claimed.clone();
        self.expire_deliveries(agent_id, &claimed).await;
        let records = self.records.read().await;
        let record = records
            .get(agent_id)
            .ok_or_else(|| "agent not found".to_string())?;
        if record.status.is_terminal() {
            return Err("agent already finished".into());
        }
        let deliveries = record
            .pending_deliveries
            .iter()
            .filter(|delivery| filter(delivery) && !state.claimed.contains(&delivery.id))
            .cloned()
            .collect::<Vec<_>>();
        state
            .claimed
            .extend(deliveries.iter().map(|delivery| delivery.id.clone()));
        if seal {
            state.sealed = !deliveries.iter().any(|delivery| delivery.wake);
        }
        Ok(deliveries)
    }

    pub async fn record_delivery_status(
        &self,
        agent_id: &str,
        id: &str,
        status: DeliveryStatus,
    ) -> Result<(), String> {
        if !matches!(
            status,
            DeliveryStatus::Consumed
                | DeliveryStatus::Expired
                | DeliveryStatus::Superseded
                | DeliveryStatus::Sealed
                | DeliveryStatus::Failed
        ) {
            return Err("delivery status is not terminal".to_string());
        }
        self.update_record(agent_id, |record, _| {
            record
                .pending_deliveries
                .retain(|delivery| delivery.id != id);
            record.followups.retain(|delivery| delivery.id != id);
            record.delivery_ids.retain(|delivered| delivered != id);
            record_dropped_delivery(record, id.to_string(), status);
            Ok(())
        })
        .await
        .map(|_| ())
    }

    async fn expire_deliveries(&self, agent_id: &str, claimed: &HashSet<String>) {
        let now_ms = now_ms();
        let expired: Vec<String> = self
            .records
            .read()
            .await
            .get(agent_id)
            .map(|record| {
                record
                    .pending_deliveries
                    .iter()
                    .filter(|pending| {
                        !claimed.contains(&pending.id) && pending.is_expired_at(now_ms)
                    })
                    .map(|pending| pending.id.clone())
                    .collect()
            })
            .unwrap_or_default();
        if expired.is_empty() {
            return;
        }
        let _ = self
            .update_record(agent_id, |record, _| {
                record
                    .pending_deliveries
                    .retain(|pending| !expired.contains(&pending.id));
                for id in &expired {
                    record_dropped_delivery(record, id.clone(), DeliveryStatus::Expired);
                }
                Ok(())
            })
            .await;
    }

    pub async fn drain_deliveries(
        &self,
        agent_id: &str,
        idle: bool,
    ) -> Result<Vec<PendingDelivery>, String> {
        self.claim_pending_deliveries(agent_id, false, |delivery| {
            idle || delivery.push != PushMode::WhenIdle
        })
        .await
    }

    /// Claim the final batch and atomically stop accepting if it cannot rearm the runner.
    /// Claims are transient: pending payloads remain durable until trajectory acknowledgement.
    pub async fn finish_delivery_turn(
        &self,
        agent_id: &str,
    ) -> Result<Vec<PendingDelivery>, String> {
        self.claim_pending_deliveries(agent_id, true, |_| true)
            .await
    }

    pub async fn inbox_for(&self, agent_id: &str) -> Option<Arc<Mutex<VecDeque<InboxMessage>>>> {
        self.runtime
            .read()
            .await
            .get(agent_id)
            .map(|runtime| runtime.inbox.clone())
    }

    pub async fn push_inbox(&self, agent_id: &str, msg: InboxMessage) -> Result<(), String> {
        let record = self
            .records
            .read()
            .await
            .get(agent_id)
            .cloned()
            .ok_or_else(|| "agent not found".to_string())?;
        if record.status.is_terminal() {
            return Err("agent already finished".to_string());
        }
        let inbox = self
            .inbox_for(agent_id)
            .await
            .ok_or_else(|| "agent is not running in this process".to_string())?;
        let mut inbox = inbox.lock().await;
        if inbox.len() >= refact_work_core::coordination_settings().delivery_max_pending {
            return Err("agent inbox is full".to_string());
        }
        inbox.push_back(msg);
        Ok(())
    }

    pub async fn drain_inbox(&self, agent_id: &str) -> Vec<InboxMessage> {
        let Some(inbox) = self.inbox_for(agent_id).await else {
            return Vec::new();
        };
        let mut inbox = inbox.lock().await;
        inbox.drain(..).collect()
    }

    pub async fn pause_gate(&self, agent_id: &str) -> Option<Arc<PauseGate>> {
        self.runtime
            .read()
            .await
            .get(agent_id)
            .map(|runtime| runtime.pause.clone())
    }

    async fn live_runtime(&self, agent_id: &str) -> Result<AgentRuntime, String> {
        let record = self
            .records
            .read()
            .await
            .get(agent_id)
            .cloned()
            .ok_or_else(|| "agent not found".to_string())?;
        if record.status.is_terminal() {
            return Err("agent already finished".to_string());
        }
        self.runtime
            .read()
            .await
            .get(agent_id)
            .cloned()
            .ok_or_else(|| "agent is not running in this process".to_string())
    }

    /// Queue a native steering note. Returns `Ok(false)` when the same control
    /// identity is already queued, so a retried request never duplicates.
    pub async fn push_native_note(&self, agent_id: &str, note: NativeNote) -> Result<bool, String> {
        let runtime = self.live_runtime(agent_id).await?;
        let mut notes = runtime.native_notes.lock().await;
        if notes
            .iter()
            .any(|queued| queued.control_id == note.control_id)
        {
            return Ok(false);
        }
        if notes.len() >= MAX_NATIVE_NOTES {
            return Err("native note queue capacity reached".to_string());
        }
        notes.push_back(note);
        Ok(true)
    }

    pub async fn drain_native_notes(&self, agent_id: &str) -> Vec<NativeNote> {
        let Some(notes) = self
            .runtime
            .read()
            .await
            .get(agent_id)
            .map(|runtime| runtime.native_notes.clone())
        else {
            return Vec::new();
        };
        let mut notes = notes.lock().await;
        notes.drain(..).collect()
    }

    pub async fn request_pause(&self, agent_id: &str) -> Result<PauseStatus, String> {
        self.live_runtime(agent_id).await?.pause.request();
        self.pause_status(agent_id)
            .await
            .ok_or_else(|| "agent is not running in this process".to_string())
    }

    pub async fn set_parent_pause(
        &self,
        agent_id: &str,
        caller_chat_id: &str,
        paused: bool,
    ) -> Result<BackgroundAgent, String> {
        self.set_parent_pause_inner(agent_id, caller_chat_id, paused, None)
            .await
    }

    pub async fn set_parent_pause_checked(
        &self,
        agent_id: &str,
        caller_chat_id: &str,
        paused: bool,
        expected_run_generation: u64,
    ) -> Result<BackgroundAgent, String> {
        self.set_parent_pause_inner(
            agent_id,
            caller_chat_id,
            paused,
            Some(expected_run_generation),
        )
        .await
    }

    async fn set_parent_pause_inner(
        &self,
        agent_id: &str,
        caller_chat_id: &str,
        paused: bool,
        expected_run_generation: Option<u64>,
    ) -> Result<BackgroundAgent, String> {
        let updated = {
            let mut records = self.records.write().await;
            let record = records
                .get_mut(agent_id)
                .ok_or_else(|| "agent not found".to_string())?;
            if record.parent_chat_id != caller_chat_id
                && record.parent_root_chat_id.as_deref() != Some(caller_chat_id)
            {
                return Err("agent not found".to_string());
            }
            if expected_run_generation.is_some_and(|expected| expected != record.run_generation) {
                return Err("child moved to another generation".to_string());
            }
            if record.status.is_terminal() {
                return Err(refact_chat_api::SealReason::AgentTerminal {
                    agent_id: record.agent_id.clone(),
                    status: record.status.as_str().to_string(),
                    run_generation: record.run_generation,
                }
                .message());
            }
            let runtime = self.runtime.read().await;
            let runtime = runtime
                .get(agent_id)
                .ok_or_else(|| "agent is not running in this process".to_string())?;
            if paused != record.is_paused() {
                let now = Utc::now();
                record.paused_by = paused.then(|| caller_chat_id.to_string());
                record.paused_at = paused.then_some(now);
                touch_record(record, now);
            }
            runtime.pause.set_parent_requested(paused);
            record.clone()
        };
        self.flush_records(Vec::new()).await?;
        Ok(updated)
    }

    pub async fn child_chat_paused(&self, chat_id: &str) -> bool {
        let records = self.records.read().await;
        let Some(record) = records
            .by_child
            .get(chat_id)
            .and_then(|agent_id| records.by_id.get(agent_id))
        else {
            return false;
        };
        if record.status.is_terminal() {
            return false;
        }
        record.is_paused()
            || self
                .runtime
                .read()
                .await
                .get(&record.agent_id)
                .is_some_and(|runtime| runtime.pause.is_requested())
    }

    pub async fn release_pause(&self, agent_id: &str) -> Result<PauseStatus, String> {
        self.live_runtime(agent_id).await?.pause.release();
        self.pause_status(agent_id)
            .await
            .ok_or_else(|| "agent is not running in this process".to_string())
    }

    pub async fn pause_status(&self, agent_id: &str) -> Option<PauseStatus> {
        let runtime = self.runtime.read().await.get(agent_id).cloned()?;
        let running_tool = self
            .records
            .read()
            .await
            .get(agent_id)
            .and_then(|record| record.current_tool.clone());
        let queued_native_notes = runtime.native_notes.lock().await.len();
        Some(PauseStatus {
            requested: runtime.pause.is_requested(),
            quiescent: runtime.pause.is_quiescent(),
            running_tool,
            running_tool_cancelable: false,
            queued_native_notes,
        })
    }

    pub async fn overlap_warning(
        &self,
        parent_chat_id: &str,
        target_files: &[String],
    ) -> Option<String> {
        let requested: HashSet<String> = target_files
            .iter()
            .map(|path| normalize_path_for_overlap(path))
            .collect();
        if requested.is_empty() {
            return None;
        }
        let records = self.records.read().await;
        let mut overlaps = Vec::new();
        for record in records.values() {
            if record.parent_chat_id != parent_chat_id || record.status.is_terminal() {
                continue;
            }
            let shared: Vec<String> = record
                .target_files
                .iter()
                .filter(|path| requested.contains(&normalize_path_for_overlap(path)))
                .cloned()
                .collect();
            if !shared.is_empty() {
                overlaps.push(format!(
                    "{} ({}) overlaps on {}",
                    record.agent_id,
                    record.title,
                    shared.join(", ")
                ));
            }
        }
        if overlaps.is_empty() {
            None
        } else {
            Some(format!(
                "Running subagent target file overlap detected: {}",
                overlaps.join("; ")
            ))
        }
    }

    async fn update_record<F>(&self, agent_id: &str, update: F) -> Result<BackgroundAgent, String>
    where
        F: FnOnce(&mut BackgroundAgent, DateTime<Utc>) -> Result<(), String>,
    {
        let updated = {
            let mut records = self.records.write().await;
            let current = records
                .get(agent_id)
                .cloned()
                .ok_or_else(|| "agent not found".to_string())?;
            let mut updated = current;
            let now = Utc::now();
            update(&mut updated, now)?;
            touch_record(&mut updated, now);
            records.insert(agent_id.to_string(), updated.clone());
            let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
            drop(records);
            self.flush_records(snapshot).await?;
            updated
        };
        if let Some(notify) = self.notify_for(agent_id).await {
            notify.notify_waiters();
        }
        Ok(updated)
    }

    async fn update_record_if_not_terminal<F>(
        self: &Arc<Self>,
        agent_id: &str,
        debounce_write: bool,
        update: F,
    ) -> Result<BackgroundAgent, String>
    where
        F: FnOnce(&mut BackgroundAgent, DateTime<Utc>) -> Result<(), String>,
    {
        let updated = {
            let mut records = self.records.write().await;
            let current = records
                .get(agent_id)
                .cloned()
                .ok_or_else(|| "agent not found".to_string())?;
            if current.status.is_terminal() {
                return Ok(current);
            }
            let mut updated = current;
            let now = Utc::now();
            update(&mut updated, now)?;
            touch_record(&mut updated, now);
            records.insert(agent_id.to_string(), updated.clone());
            let snapshot: Vec<BackgroundAgent> = records.values().cloned().collect();
            drop(records);
            self.schedule_or_flush_records(snapshot, !debounce_write)
                .await?;
            updated
        };
        if let Some(notify) = self.notify_for(agent_id).await {
            notify.notify_waiters();
        }
        Ok(updated)
    }

    async fn notify_for(&self, agent_id: &str) -> Option<Arc<Notify>> {
        self.runtime
            .read()
            .await
            .get(agent_id)
            .map(|runtime| runtime.notify.clone())
    }

    async fn retire_runtime(&self, agent_id: &str, generation: u64) {
        let records = self.records.read().await;
        if records.get(agent_id).is_none_or(|record| {
            record.run_generation != generation || !record.status.is_terminal()
        }) {
            return;
        }
        if let Some(runtime) = self.runtime.write().await.remove(agent_id) {
            if runtime.pause.is_requested() {
                runtime.abort_flag.store(true, Ordering::SeqCst);
                runtime.pause.wake_for_abort();
            }
        }
    }

    fn settle_terminal(&self, record: &mut BackgroundAgent, now: DateTime<Utc>) {
        settle_terminal_record(&self.questions, record, now);
    }

    #[doc(hidden)]
    pub async fn flush_pending_writes_for_test(&self) -> Result<(), String> {
        let snapshot: Vec<BackgroundAgent> = self.records.read().await.values().cloned().collect();
        self.flush_records(snapshot).await
    }

    async fn flush_records(&self, _records: Vec<BackgroundAgent>) -> Result<(), String> {
        // Re-snapshot after serializing disk writes: older scheduled flushes must
        // never overwrite a newly accepted durable delivery.
        let _write = self.storage_write.lock().await;
        let records = self.records.read().await.values().cloned().collect();
        storage::save_all(&self.storage_root, records).await?;
        let mut write_state = self.write_state.lock().await;
        write_state.pending = false;
        write_state.last_write = Some(Instant::now());
        if let Some(handle) = write_state.flush_task.take() {
            handle.abort();
        }
        Ok(())
    }

    async fn schedule_or_flush_records(
        self: &Arc<Self>,
        snapshot: Vec<BackgroundAgent>,
        force_flush: bool,
    ) -> Result<(), String> {
        if force_flush {
            return self.flush_records(snapshot).await;
        }
        let mut write_state = self.write_state.lock().await;
        let now = Instant::now();
        let should_flush_now = write_state
            .last_write
            .map(|last| now.duration_since(last) >= WRITE_DEBOUNCE)
            .unwrap_or(true);
        if should_flush_now {
            drop(write_state);
            return self.flush_records(snapshot).await;
        }
        write_state.pending = true;
        if write_state.flush_task.is_none() {
            let registry = Arc::downgrade(self);
            let wait_for = WRITE_DEBOUNCE
                .checked_sub(now.duration_since(write_state.last_write.unwrap_or(now)))
                .unwrap_or(Duration::ZERO);
            write_state.flush_task = Some(tokio::spawn(async move {
                tokio::time::sleep(wait_for).await;
                loop {
                    let Some(registry) = registry.upgrade() else {
                        return;
                    };
                    match registry.flush_records(Vec::new()).await {
                        Ok(()) => return,
                        Err(error) => tracing::error!(
                            "Failed to save background agents at {}: {error}; retrying in {} seconds",
                            registry.storage_root.join("records.json").display(),
                            WRITE_DEBOUNCE.as_secs()
                        ),
                    }
                    drop(registry);
                    tokio::time::sleep(WRITE_DEBOUNCE).await;
                }
            }));
        }
        Ok(())
    }
}

fn normalize_delivery_state(records: &mut HashMap<String, BackgroundAgent>) {
    let settings = refact_work_core::coordination_settings();
    let now_ms = now_ms();
    for record in records.values_mut() {
        trim_followups(record);
        let mut superseded = Vec::new();
        let mut expired = Vec::new();
        let mut newest_by_key: HashMap<String, String> = HashMap::new();
        for delivery in &mut record.pending_deliveries {
            delivery.ensure_restored_ttl_secs(now_ms, settings.delivery_default_ttl_secs);
            if delivery.is_expired_at(now_ms) {
                expired.push(delivery.id.clone());
            }
            if let Some(key) = &delivery.supersede_key {
                if let Some(previous) = newest_by_key.insert(key.clone(), delivery.id.clone()) {
                    superseded.push(previous);
                }
            }
        }
        record.pending_deliveries.retain(|delivery| {
            !expired.contains(&delivery.id) && !superseded.contains(&delivery.id)
        });
        for id in expired {
            record_dropped_delivery(record, id, DeliveryStatus::Expired);
        }
        for id in superseded {
            record_dropped_delivery(record, id, DeliveryStatus::Superseded);
        }
        if record.pending_deliveries.len() > settings.delivery_max_pending {
            let overflow = record.pending_deliveries.len() - settings.delivery_max_pending;
            let dropped: Vec<_> = record.pending_deliveries.drain(..overflow).collect();
            for delivery in dropped {
                record_dropped_delivery(record, delivery.id, DeliveryStatus::Failed);
            }
        }
        if record.delivery_ids.len() > settings.delivery_status_retention {
            let overflow = record.delivery_ids.len() - settings.delivery_status_retention;
            record.delivery_ids.drain(..overflow);
        }
        if record.dropped_deliveries.len() > settings.delivery_status_retention {
            let overflow = record.dropped_deliveries.len() - settings.delivery_status_retention;
            record.dropped_deliveries.drain(..overflow);
        }
    }
}

fn settle_terminal_record(
    questions: &QuestionRegistry,
    record: &mut BackgroundAgent,
    now: DateTime<Utc>,
) {
    let now_ms = now.timestamp_millis().max(0) as u64;
    let pending = std::mem::take(&mut record.pending_deliveries);
    for delivery in pending {
        if delivery.is_expired_at(now_ms) {
            record_dropped_delivery(record, delivery.id, DeliveryStatus::Expired);
        } else if record.delivery_ids.contains(&delivery.id)
            || record
                .dropped_deliveries
                .iter()
                .any(|dropped| dropped.id == delivery.id)
            || record
                .followups
                .iter()
                .any(|followup| followup.id == delivery.id)
        {
            continue;
        } else {
            record.followups.push(delivery);
        }
    }
    let dropped = trim_followups(record);
    if !dropped.is_empty() {
        tracing::warn!(agent_id = %record.agent_id, ?dropped, "Oldest agent follow-ups dropped at capacity");
    }
    for pending in questions.pending_for_asker(&record.agent_id, now) {
        if pending.kind == QuestionKind::AgentParent
            && !record
                .questions
                .iter()
                .any(|question| question.id == pending.id)
        {
            record.questions.push(pending);
        }
    }
    for question in &mut record.questions {
        if question.kind != QuestionKind::AgentParent {
            continue;
        }
        if let Some(current) = questions.get(&question.id) {
            *question = current;
        }
        if question.state == refact_chat_api::QuestionState::Pending {
            if let Ok(withdrawn) = questions.withdraw(&question.id, &question.asker, now) {
                *question = withdrawn;
            }
        }
    }
}

fn followup_bytes(delivery: &PendingDelivery) -> usize {
    serde_json::to_vec(delivery).map_or(usize::MAX, |bytes| bytes.len())
}

fn trim_followups(record: &mut BackgroundAgent) -> Vec<String> {
    let settings = refact_work_core::coordination_settings();
    let mut bytes = record.followups.iter().fold(0usize, |total, delivery| {
        total.saturating_add(followup_bytes(delivery))
    });
    let mut dropped = Vec::new();
    while record.followups.len() > settings.agent_followup_max_count
        || bytes > settings.agent_followup_max_bytes
    {
        let delivery = record.followups.remove(0);
        bytes = bytes.saturating_sub(followup_bytes(&delivery));
        dropped.push(delivery.id.clone());
        record_dropped_delivery(record, delivery.id, DeliveryStatus::Failed);
    }
    dropped
}

fn record_previous_run(updated: &mut BackgroundAgent, previous: &BackgroundAgent) {
    updated.previous_runs.push(AgentRunOutcome {
        run_generation: previous.run_generation,
        status: previous.status,
        step_count: previous.step_count,
        result_summary: previous.result_summary.clone(),
        error: previous.error.clone(),
        finished_at: previous.finished_at,
        worktree_id: previous.worktree_id.clone(),
        worktree_branch: previous.worktree_branch.clone(),
        merge_status: previous.merge_status.clone(),
    });
    trim_previous_runs(updated);
}

fn trim_previous_runs(record: &mut BackgroundAgent) {
    let retained = refact_work_core::coordination_settings()
        .agent_closed_retained_max
        .max(1);
    if record.previous_runs.len() > retained {
        let overflow = record.previous_runs.len() - retained;
        record.previous_runs.drain(..overflow);
    }
}

fn normalize_run_history(records: &mut HashMap<String, BackgroundAgent>) {
    for record in records.values_mut() {
        trim_previous_runs(record);
        if record.status != BgAgentStatus::Closed {
            record.close_reason = None;
        }
    }
}

fn now_ms() -> u64 {
    Utc::now().timestamp_millis().max(0) as u64
}

fn record_dropped_delivery(record: &mut BackgroundAgent, id: String, status: DeliveryStatus) {
    record.dropped_deliveries.retain(|dropped| dropped.id != id);
    record
        .dropped_deliveries
        .push(DeliveryStatusRecord { id, status });
    let retention = refact_work_core::coordination_settings().delivery_status_retention;
    while record.dropped_deliveries.len() > retention {
        record.dropped_deliveries.remove(0);
    }
}

pub fn normalize_path_for_overlap(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let mut collapsed = String::with_capacity(normalized.len());
    let mut previous_slash = false;
    for ch in normalized.chars() {
        if ch == '/' {
            if !previous_slash {
                collapsed.push(ch);
            }
            previous_slash = true;
        } else {
            collapsed.push(ch);
            previous_slash = false;
        }
    }
    while let Some(stripped) = collapsed.strip_prefix("./") {
        collapsed = stripped.to_string();
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        collapsed.to_lowercase()
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        collapsed
    }
}

fn touch_record(record: &mut BackgroundAgent, now: DateTime<Utc>) {
    record.change_seq = record.change_seq.saturating_add(1);
    record.last_update_at = now;
}

fn terminal_result_payload(
    status: &str,
    error: &str,
    record: &BackgroundAgent,
) -> serde_json::Value {
    serde_json::json!({
        "status": status,
        "error": error,
        "edited_files": record.edited_files.clone(),
        "diff_summary": record.diff_summary.clone(),
        "conflict_summary": record.conflict_summary.clone(),
    })
}

fn scoped_record(
    records: &HashMap<String, BackgroundAgent>,
    parent_chat_id: &str,
    agent_id: &str,
) -> Result<BackgroundAgent, String> {
    records
        .get(agent_id)
        .filter(|record| record.parent_chat_id == parent_chat_id)
        .cloned()
        .ok_or_else(|| "agent not found".to_string())
}

fn validate_parent_run(
    records: &AgentRecords,
    chat_id: &str,
    parent: Option<&ParentRun>,
) -> Result<(), String> {
    match (records.by_child.get(chat_id), parent) {
        (None, None) => Ok(()),
        (Some(agent_id), Some(parent)) if agent_id == &parent.agent_id => {
            let record = &records.by_id[agent_id];
            if record.run_generation != parent.generation
                || (record.status.is_terminal() && record.status != BgAgentStatus::Completed)
                || parent.abort_flag.load(Ordering::SeqCst)
                || records.cancelled_subtrees.get(agent_id) == Some(&parent.generation)
            {
                Err("parent agent run changed or was cancelled".into())
            } else {
                Ok(())
            }
        }
        _ => Err("parent agent run identity changed".into()),
    }
}

fn terminal_cutoff(hours: i64) -> Option<DateTime<Utc>> {
    if hours < 0 {
        return None;
    }
    Utc::now().checked_sub_signed(TimeDelta::hours(hours))
}

fn should_include_record(record: &BackgroundAgent, cutoff: Option<DateTime<Utc>>) -> bool {
    if !record.status.is_terminal() {
        return true;
    }
    match (cutoff, record.finished_at) {
        (Some(cutoff), Some(finished_at)) => finished_at >= cutoff,
        _ => false,
    }
}

async fn reconcile_interrupted(
    storage_root: &Path,
    records: &mut HashMap<String, BackgroundAgent>,
) -> Result<(), String> {
    let now = Utc::now();
    let mut interrupted = Vec::new();
    for record in records.values_mut() {
        if matches!(
            record.status,
            BgAgentStatus::Queued | BgAgentStatus::Running | BgAgentStatus::WaitingForApproval
        ) {
            record.status = BgAgentStatus::Interrupted;
            record.finished_at = Some(now);
            record.last_update_at = now;
            record.change_seq = record.change_seq.saturating_add(1);
            record.error = Some(
                "Engine restarted before agent finished. Inspect the saved child before explicit recovery; no work was replayed.".to_string(),
            );
            interrupted.push(record.clone());
        }
    }
    for record in interrupted {
        storage::save_record(storage_root, &record).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;

    async fn registry() -> (tempfile::TempDir, Arc<BackgroundAgentRegistry>) {
        let temp = tempfile::tempdir().unwrap();
        let registry = BackgroundAgentRegistry::new(temp.path().join("agents"))
            .await
            .unwrap();
        (temp, registry)
    }

    fn request(parent_chat_id: &str) -> CreateAgentRequest {
        CreateAgentRequest {
            parent_chat_id: parent_chat_id.to_string(),
            parent_root_chat_id: None,
            parent_tool_call_id: None,
            kind: BgAgentKind::Subagent,
            config_name: "test".to_string(),
            title: "Test agent".to_string(),
            prompt: "Test prompt".to_string(),
            target_files: Vec::new(),
            model: "test-model".to_string(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
        }
    }

    #[tokio::test]
    async fn child_chat_index_tracks_binding_replacement_completion_and_restore() {
        let (temp, registry) = registry().await;
        let record = registry.create(request("parent")).await.unwrap().0;
        assert_eq!(registry.child_chat_status("child-a").await, None);
        registry
            .mark_running(&record.agent_id, "child-a".into())
            .await
            .unwrap();
        assert_eq!(
            registry.find_agent_id_by_child_chat_id("child-a").await,
            Some(record.agent_id.clone())
        );
        registry
            .mark_running(&record.agent_id, "child-b".into())
            .await
            .unwrap();
        assert_eq!(registry.child_chat_status("child-a").await, None);
        let mut finished = completion();
        finished.child_chat_id = Some("child-c".into());
        let completed = registry
            .mark_completed(&record.agent_id, finished)
            .await
            .unwrap();
        assert_eq!(registry.child_chat_status("child-b").await, None);
        assert_eq!(
            registry.child_chat_status("child-c").await,
            Some((record.agent_id.clone(), BgAgentStatus::Completed, 1))
        );
        registry
            .reopen(&record.agent_id, "parent", completed.change_seq, false)
            .await
            .unwrap();
        assert_eq!(
            registry.child_chat_status("child-c").await,
            Some((record.agent_id.clone(), BgAgentStatus::Queued, 2))
        );
        let restored = BackgroundAgentRegistry::new(temp.path().join("agents"))
            .await
            .unwrap();
        assert_eq!(
            restored.child_chat_status("child-c").await,
            Some((record.agent_id, BgAgentStatus::Interrupted, 2))
        );
    }

    #[tokio::test]
    async fn claim_stop_hook_is_once_per_generation() {
        let (_temp, registry) = registry().await;
        let record = registry.create(request("parent")).await.unwrap().0;

        assert!(registry.claim_stop_hook(&record.agent_id, 1).await);
        assert!(!registry.claim_stop_hook(&record.agent_id, 1).await);
        assert!(registry.claim_stop_hook(&record.agent_id, 2).await);
    }

    async fn capped(registry: &Arc<BackgroundAgentRegistry>) -> BackgroundAgent {
        let (record, _, _) = registry.create(request("parent")).await.unwrap();
        registry
            .update_progress(&record.agent_id, "capped".into(), 4)
            .await
            .unwrap();
        registry
            .add_usage(&record.agent_id, 123, Some(0.5))
            .await
            .unwrap();
        let error = super::super::types::CapStop {
            key: refact_work_core::CapKind::Steps,
            scope: refact_work_core::CapScope::Child,
            limit: 4,
            used: 4,
            resumable: true,
            run_generation: 0,
        }
        .as_error()
        .unwrap();
        registry.mark_failed(&record.agent_id, error).await.unwrap()
    }

    #[tokio::test]
    async fn create_with_id_rejects_duplicates_without_replacing_the_record() {
        let (_temp, registry) = registry().await;
        let (original, _, _) = registry
            .create_with_id(request("parent"), "stable-agent")
            .await
            .unwrap();
        let before = registry.get_any("stable-agent").await.unwrap();
        assert_eq!(before.schema_version, storage::SCHEMA_VERSION);

        assert_eq!(
            registry
                .create_with_id(request("other-parent"), "stable-agent")
                .await
                .unwrap_err(),
            "agent id already exists"
        );
        assert_eq!(registry.get_any("stable-agent").await.unwrap(), before);
        assert_eq!(original.parent_chat_id, "parent");

        let (generated, _, _) = registry.create(request("parent")).await.unwrap();
        assert!(generated.agent_id.starts_with("bgagent-"));
    }

    #[tokio::test]
    async fn capped_reopen_rejects_forged_caps_owner_and_stale_revision() {
        let (_temp, registry) = registry().await;
        let record = capped(&registry).await;
        assert!(registry
            .reopen_capped(&record.agent_id, "other", record.change_seq)
            .await
            .is_err());
        assert!(registry
            .reopen_capped(&record.agent_id, "parent", record.change_seq - 1)
            .await
            .is_err());
        for error in [
            "CAP_STOP:forged",
            "CAP_STOP:{\"key\":\"max_steps\",\"limit\":4,\"used\":4,\"resumable\":false}",
            "CAP_STOP:{\"key\":\"goal.max_turns\",\"limit\":4,\"used\":4,\"resumable\":true}",
        ] {
            let forged = registry
                .update_record(&record.agent_id, |r, _| {
                    r.error = Some(error.into());
                    Ok(())
                })
                .await
                .unwrap();
            assert!(registry
                .reopen_capped(&record.agent_id, "parent", forged.change_seq)
                .await
                .is_err());
            assert!(!registry.has_runtime(&record.agent_id).await);
        }
    }

    #[tokio::test]
    async fn capped_reopen_has_one_winner_and_preserves_accounting() {
        let (_temp, registry) = registry().await;
        let record = capped(&registry).await;
        registry
            .set_completion_message_id(&record.agent_id, "old".into())
            .await
            .unwrap();
        let record = registry.get_any(&record.agent_id).await.unwrap();
        let (one, two) = tokio::join!(
            registry.reopen_capped(&record.agent_id, "parent", record.change_seq),
            registry.reopen_capped(&record.agent_id, "parent", record.change_seq),
        );
        assert_ne!(one.is_ok(), two.is_ok());
        let (reopened, _, _) = one.or(two).unwrap();
        assert_eq!(reopened.step_count, 4);
        assert_eq!(reopened.tokens_used, 123);
        assert_eq!(reopened.cost_usd, Some(0.5));
        assert!(reopened.result_payload_path.is_none());
        registry
            .set_completion_message_id(&record.agent_id, "late-old".into())
            .await
            .unwrap();
        assert!(registry
            .get_any(&record.agent_id)
            .await
            .unwrap()
            .completion_message_id
            .is_none());
    }

    #[tokio::test]
    async fn cancellation_during_reopen_flush_cannot_resurrect_runtime() {
        let (_temp, registry) = registry().await;
        let record = capped(&registry).await;
        let disk = registry.storage_write.lock().await;
        let reopen = registry.reopen_capped(&record.agent_id, "parent", record.change_seq);
        tokio::pin!(reopen);
        // Poll exactly through publication and stop on the held disk lock.
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(reopen.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        let flag = registry.abort_flag(&record.agent_id).await.unwrap();
        let cancel = registry.cancel("parent", &record.agent_id, None);
        tokio::pin!(cancel);
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(cancel.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        assert!(flag.load(Ordering::SeqCst));
        drop(disk);
        let (resumed, cancelled) = tokio::join!(reopen, cancel);
        assert!(resumed.is_err());
        assert_eq!(cancelled.unwrap().status, BgAgentStatus::Cancelled);
        assert!(!registry.has_runtime(&record.agent_id).await);
    }

    #[tokio::test]
    async fn subtree_cancel_fences_insertion_before_flush_and_old_parent_generations() {
        let (_temp, registry) = registry().await;
        let (root, _, _) = registry.create(request("root-owner")).await.unwrap();
        registry
            .mark_running(&root.agent_id, "root-child".into())
            .await
            .unwrap();
        let parent = registry.parent_run("root-child").await.unwrap().unwrap();
        assert_eq!(parent.agent_id, root.agent_id);
        assert_eq!(parent.generation, 1);
        let disk = registry.storage_write.lock().await;
        let create = registry.create_with_id_for_parent_run(
            request("root-child"),
            "inserting-child".into(),
            Some(&parent),
        );
        tokio::pin!(create);
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(create.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        let inserted = registry.get_any("inserting-child").await.unwrap();
        assert_eq!(inserted.parent_chat_id, "root-child");
        assert_eq!(inserted.status, BgAgentStatus::Queued);
        let abort = registry.abort_flag(&inserted.agent_id).await.unwrap();
        let cancel = registry.cancel_subtree("root-owner", &root.agent_id, true, None);
        tokio::pin!(cancel);
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(cancel.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        assert!(abort.load(Ordering::SeqCst));
        assert_eq!(
            registry.get_any(&inserted.agent_id).await.unwrap().status,
            BgAgentStatus::Cancelled
        );
        assert!(registry
            .create_for_parent_run(request("root-child"), Some(&parent))
            .await
            .is_err());
        drop(disk);
        let (created, cancelled) = tokio::join!(create, cancel);
        assert!(created.is_err());
        let cancelled = cancelled.unwrap();
        assert_eq!(
            cancelled
                .iter()
                .map(|record| record.agent_id.as_str())
                .collect::<HashSet<_>>(),
            HashSet::from([root.agent_id.as_str(), "inserting-child"])
        );
        assert!(!registry.has_runtime(&inserted.agent_id).await);
        let stopped = registry.get_any(&root.agent_id).await.unwrap();
        let (_, next_abort, _) = registry
            .reopen(&root.agent_id, "root-owner", stopped.change_seq, false)
            .await
            .unwrap();
        let next = registry.parent_run("root-child").await.unwrap().unwrap();
        assert_eq!(next.generation, 2);
        assert!(!next_abort.load(Ordering::SeqCst));
        assert!(registry
            .create_for_parent_run(request("root-child"), Some(&parent))
            .await
            .is_err());
        let (fresh, _, _) = registry
            .create_for_parent_run(request("root-child"), Some(&next))
            .await
            .unwrap();
        assert_eq!(fresh.status, BgAgentStatus::Queued);
        assert_eq!(
            registry.get_any(&inserted.agent_id).await.unwrap().status,
            BgAgentStatus::Cancelled
        );
    }

    #[tokio::test]
    async fn failed_reopen_flush_releases_claim_without_reusing_revision() {
        let (_temp, registry) = registry().await;
        let record = capped(&registry).await;
        tokio::fs::remove_dir_all(&registry.storage_root)
            .await
            .unwrap();
        tokio::fs::write(&registry.storage_root, b"not a directory")
            .await
            .unwrap();
        assert!(registry
            .reopen_capped(&record.agent_id, "parent", record.change_seq)
            .await
            .is_err());
        let restored = registry.get_any(&record.agent_id).await.unwrap();
        assert_eq!(restored.status, BgAgentStatus::Failed);
        assert_eq!(restored.error, record.error);
        assert!(restored.change_seq > record.change_seq);
        assert!(!registry.has_runtime(&record.agent_id).await);
        assert!(registry
            .reopen_capped(&record.agent_id, "parent", record.change_seq)
            .await
            .is_err());
    }

    fn inbox_message(text: impl Into<String>) -> InboxMessage {
        InboxMessage {
            from: "sibling".to_string(),
            text: text.into(),
            queued_at: Utc::now(),
        }
    }

    fn completion() -> AgentCompletion {
        AgentCompletion {
            result_summary: "done".to_string(),
            edited_files: Vec::new(),
            diff_summary: None,
            conflict_summary: None,
            child_chat_id: None,
            deliverable_status: None,
            handoff: None,
        }
    }

    #[tokio::test]
    async fn list_descendants_stops_at_agent_and_chat_cycles() {
        let (_temp, registry) = registry().await;
        let (root, _, _) = registry.create(request("chat-a")).await.unwrap();
        registry
            .mark_running(&root.agent_id, "chat-b".to_string())
            .await
            .unwrap();
        let (child, _, _) = registry.create(request("chat-b")).await.unwrap();
        registry
            .mark_running(&child.agent_id, "chat-a".to_string())
            .await
            .unwrap();

        let descendants = registry.list_descendants(&root.agent_id).await;

        assert_eq!(descendants.len(), 1);
        assert_eq!(descendants[0].agent_id, child.agent_id);
    }

    #[tokio::test]
    async fn queued_inbox_accepts_messages_and_terminal_agents_reject_them() {
        let (_temp, registry) = registry().await;
        let (record, _, _) = registry.create(request("parent")).await.unwrap();

        registry
            .push_inbox(&record.agent_id, inbox_message("queued"))
            .await
            .unwrap();
        registry
            .mark_completed(&record.agent_id, completion())
            .await
            .unwrap();

        assert!(registry.inbox_for(&record.agent_id).await.is_none());
        assert_eq!(
            registry
                .push_inbox(&record.agent_id, inbox_message("late"))
                .await
                .unwrap_err(),
            "agent already finished"
        );
        assert_eq!(
            registry
                .push_inbox("missing", inbox_message("missing"))
                .await
                .unwrap_err(),
            "agent not found"
        );
    }

    #[tokio::test]
    async fn inbox_drops_oldest_message_after_capacity() {
        let (_temp, registry) = registry().await;
        let (record, _, _) = registry.create(request("parent")).await.unwrap();
        let capacity = refact_work_core::coordination_settings().delivery_max_pending;
        for index in 0..capacity {
            registry
                .push_inbox(&record.agent_id, inbox_message(index.to_string()))
                .await
                .unwrap();
        }
        assert!(registry
            .push_inbox(&record.agent_id, inbox_message("overflow"))
            .await
            .is_err());
        let inbox = registry.drain_inbox(&record.agent_id).await;
        assert_eq!(inbox.len(), capacity);
        assert_eq!(inbox[0].text, "0");
    }

    fn delivery(push: PushMode) -> PendingDelivery {
        PendingDelivery::new(Vec::new(), push, "registry-test".to_string(), true)
    }

    #[tokio::test]
    async fn terminal_effects_preserve_only_eligible_followups_for_every_status() {
        let (temp, registry) = registry().await;
        for status in [
            BgAgentStatus::Completed,
            BgAgentStatus::Failed,
            BgAgentStatus::Cancelled,
            BgAgentStatus::Interrupted,
            BgAgentStatus::Closed,
        ] {
            let (record, _, _) = registry.create(request("parent")).await.unwrap();
            let mut old = delivery(PushMode::Append);
            old.supersede_key = Some("steer".into());
            let mut newest = delivery(PushMode::Append);
            newest.supersede_key = Some("steer".into());
            newest.expires_at_ms = Some(u64::MAX);
            let mut expired = delivery(PushMode::WhenIdle);
            expired.expires_at_ms = Some(u64::MAX);
            for pending in [old.clone(), newest.clone(), expired.clone()] {
                registry
                    .enqueue_delivery(&record.agent_id, pending)
                    .await
                    .unwrap();
            }
            registry
                .update_record(&record.agent_id, |record, _| {
                    record
                        .pending_deliveries
                        .iter_mut()
                        .find(|pending| pending.id == expired.id)
                        .unwrap()
                        .expires_at_ms = Some(1);
                    Ok(())
                })
                .await
                .unwrap();
            let terminal = match status {
                BgAgentStatus::Completed => registry
                    .mark_completed(&record.agent_id, completion())
                    .await
                    .unwrap(),
                BgAgentStatus::Failed => registry
                    .mark_failed(&record.agent_id, "failed".into())
                    .await
                    .unwrap(),
                BgAgentStatus::Cancelled => registry
                    .mark_cancelled(&record.agent_id, None)
                    .await
                    .unwrap(),
                BgAgentStatus::Interrupted => registry
                    .mark_interrupted(&record.agent_id, "interrupted".into())
                    .await
                    .unwrap(),
                BgAgentStatus::Closed => {
                    registry
                        .mark_closed(&record.agent_id, None, None)
                        .await
                        .unwrap()
                        .0
                }
                _ => unreachable!(),
            };
            assert!(terminal.pending_deliveries.is_empty(), "{status:?}");
            assert_eq!(terminal.followups, vec![newest.clone()], "{status:?}");
            assert_eq!(
                registry
                    .delivery_status(&record.agent_id, &expired.id)
                    .await,
                DeliveryStatus::Expired
            );
            assert_eq!(
                registry.delivery_status(&record.agent_id, &old.id).await,
                DeliveryStatus::Superseded
            );
            let saved = storage::load_all(&temp.path().join("agents"))
                .await
                .unwrap();
            assert_eq!(saved[&record.agent_id].followups, terminal.followups);
        }
    }

    #[tokio::test]
    async fn terminal_followups_restore_reopen_atomically_and_return_on_failure() {
        let (temp, registry) = registry().await;
        let (record, _, _) = registry.create(request("parent")).await.unwrap();
        registry
            .mark_completed(&record.agent_id, completion())
            .await
            .unwrap();
        let mut pending = delivery(PushMode::WhenIdle);
        pending.expires_at_ms = Some(u64::MAX);
        let receipt = registry
            .enqueue_agent_message(&record.agent_id, pending.clone())
            .await
            .unwrap();
        assert_eq!(receipt.finished_status, Some(BgAgentStatus::Completed));
        let restored = BackgroundAgentRegistry::new(temp.path().join("agents"))
            .await
            .unwrap();
        let terminal = restored.get_any(&record.agent_id).await.unwrap();
        assert_eq!(terminal.followups, vec![pending.clone()]);
        let reopened = restored
            .reopen(&record.agent_id, "parent", terminal.change_seq, false)
            .await
            .unwrap()
            .0;
        assert!(reopened.followups.is_empty());
        assert_eq!(reopened.pending_deliveries, vec![pending.clone()]);
        let saved = storage::load_all(&temp.path().join("agents"))
            .await
            .unwrap();
        assert!(saved[&record.agent_id].followups.is_empty());
        assert_eq!(
            saved[&record.agent_id].pending_deliveries,
            vec![pending.clone()]
        );
        let failed = restored
            .mark_failed(&record.agent_id, "worktree preparation failed".into())
            .await
            .unwrap();
        assert!(failed.pending_deliveries.is_empty());
        assert_eq!(failed.followups, vec![pending.clone()]);
        assert_eq!(
            restored
                .enqueue_agent_message(&record.agent_id, pending)
                .await
                .unwrap()
                .outcome,
            DeliveryOutcome::Duplicate
        );
    }

    #[tokio::test]
    async fn racing_termination_never_strands_an_accepted_agent_message() {
        let (_temp, registry) = registry().await;
        for _ in 0..20 {
            let (record, _, _) = registry.create(request("parent")).await.unwrap();
            let mut pending = delivery(PushMode::Append);
            pending.expires_at_ms = Some(u64::MAX);
            let (receipt, terminal) = tokio::join!(
                registry.enqueue_agent_message(&record.agent_id, pending.clone()),
                registry.mark_completed(&record.agent_id, completion()),
            );
            assert!(receipt.is_ok(), "{receipt:?}");
            terminal.unwrap();
            let after = registry.get_any(&record.agent_id).await.unwrap();
            assert!(after.pending_deliveries.is_empty());
            assert_eq!(after.followups, vec![pending]);
        }
    }

    #[tokio::test]
    async fn oversized_followup_is_refused_without_dropping_existing_messages() {
        let (_temp, registry) = registry().await;
        let (record, _, _) = registry.create(request("parent")).await.unwrap();
        registry
            .mark_completed(&record.agent_id, completion())
            .await
            .unwrap();
        let mut pending = delivery(PushMode::Append);
        pending.expires_at_ms = Some(u64::MAX);
        registry
            .enqueue_agent_message(&record.agent_id, pending.clone())
            .await
            .unwrap();
        let mut oversized = delivery(PushMode::Append);
        oversized.source =
            "x".repeat(refact_work_core::coordination_settings().agent_followup_max_bytes);
        assert!(registry
            .enqueue_agent_message(&record.agent_id, oversized)
            .await
            .unwrap_err()
            .contains("byte limit"));
        assert_eq!(
            registry.get_any(&record.agent_id).await.unwrap().followups,
            vec![pending]
        );
    }

    #[tokio::test]
    async fn runner_deliveries_expire_and_supersede_without_reaching_the_agent() {
        let (_temp, registry) = registry().await;
        let (record, _cancel, _) = registry.create(request("parent")).await.unwrap();
        let now_ms = Utc::now().timestamp_millis() as u64;
        let mut stale = delivery(PushMode::Append);
        stale.expires_at_ms = Some(now_ms.saturating_sub(1));
        assert_eq!(
            registry
                .enqueue_delivery(&record.agent_id, stale.clone())
                .await
                .unwrap(),
            DeliveryOutcome::Expired
        );
        assert_eq!(
            registry.delivery_status(&record.agent_id, &stale.id).await,
            DeliveryStatus::Expired
        );
        let mut first = delivery(PushMode::Append);
        first.supersede_key = Some("steer:card".into());
        let mut second = delivery(PushMode::Append);
        second.supersede_key = Some("steer:card".into());
        let mut soon = delivery(PushMode::Append);
        soon.expires_at_ms = Some(u64::MAX);
        for pending in [first.clone(), second.clone(), soon.clone()] {
            assert_eq!(
                registry
                    .enqueue_delivery(&record.agent_id, pending)
                    .await
                    .unwrap(),
                DeliveryOutcome::Queued
            );
        }
        assert_eq!(
            registry.delivery_status(&record.agent_id, &first.id).await,
            DeliveryStatus::Superseded
        );
        assert_eq!(
            registry.delivery_status(&record.agent_id, &second.id).await,
            DeliveryStatus::Pending
        );
        assert_eq!(
            registry.delivery_status(&record.agent_id, &soon.id).await,
            DeliveryStatus::Pending
        );
        registry
            .update_record(&record.agent_id, |record, now| {
                record
                    .pending_deliveries
                    .iter_mut()
                    .find(|pending| pending.id == soon.id)
                    .unwrap()
                    .expires_at_ms = Some((now.timestamp_millis() as u64).saturating_sub(1));
                Ok(())
            })
            .await
            .unwrap();
        let drained = registry
            .drain_deliveries(&record.agent_id, true)
            .await
            .unwrap();
        assert_eq!(
            drained.iter().map(|d| d.id.clone()).collect::<Vec<_>>(),
            vec![second.id.clone()]
        );
        assert_eq!(
            registry.delivery_status(&record.agent_id, &soon.id).await,
            DeliveryStatus::Expired
        );
        registry
            .acknowledge_delivery(&record.agent_id, &second.id)
            .await
            .unwrap();
        assert_eq!(
            registry.delivery_status(&record.agent_id, &second.id).await,
            DeliveryStatus::Delivered
        );
        registry
            .consume_delivery(&record.agent_id, &second.id)
            .await
            .unwrap();
        assert_eq!(
            registry.delivery_status(&record.agent_id, &second.id).await,
            DeliveryStatus::Consumed
        );
        assert_eq!(
            registry.delivery_status(&record.agent_id, "never").await,
            DeliveryStatus::Unknown
        );
        let persisted = registry.get_any(&record.agent_id).await.unwrap();
        assert_eq!(persisted.dropped_deliveries.len(), 4);
        assert!(persisted.pending_deliveries.is_empty());
    }

    #[tokio::test]
    async fn runner_supersession_only_drops_the_same_logical_stream() {
        let (_temp, registry) = registry().await;
        let (record, _cancel, _) = registry.create(request("parent")).await.unwrap();
        let keyed = |key: Option<&str>| {
            let mut pending = delivery(PushMode::Append);
            pending.supersede_key = key.map(str::to_string);
            pending
        };
        let old_steer = keyed(Some("agent_steer:T-1"));
        let other_card = keyed(Some("agent_steer:T-2"));
        let plain = keyed(None);
        let question = keyed(Some("planner_qna:Q-7"));
        for pending in [
            old_steer.clone(),
            other_card.clone(),
            plain.clone(),
            question.clone(),
        ] {
            assert_eq!(
                registry
                    .enqueue_delivery(&record.agent_id, pending)
                    .await
                    .unwrap(),
                DeliveryOutcome::Queued
            );
        }
        let new_steer = keyed(Some("agent_steer:T-1"));
        assert_eq!(
            registry
                .enqueue_delivery(&record.agent_id, new_steer.clone())
                .await
                .unwrap(),
            DeliveryOutcome::Queued
        );

        assert_eq!(
            registry
                .delivery_status(&record.agent_id, &old_steer.id)
                .await,
            DeliveryStatus::Superseded
        );
        for survivor in [&other_card, &plain, &question, &new_steer] {
            assert_eq!(
                registry
                    .delivery_status(&record.agent_id, &survivor.id)
                    .await,
                DeliveryStatus::Pending,
                "a different logical stream must survive supersession"
            );
        }

        registry
            .enqueue_delivery(&record.agent_id, keyed(None))
            .await
            .unwrap();
        assert_eq!(
            registry.delivery_status(&record.agent_id, &plain.id).await,
            DeliveryStatus::Pending,
            "a keyless delivery supersedes nothing"
        );
    }

    #[tokio::test]
    async fn runner_expired_delivery_never_arms_a_preempt_wake() {
        let (_temp, registry) = registry().await;
        let (record, cancel, _) = registry.create(request("parent")).await.unwrap();
        let mut stale = delivery(PushMode::Preempt);
        stale.expires_at_ms = Some(now_ms().saturating_sub(1));
        assert_eq!(
            registry
                .enqueue_delivery(&record.agent_id, stale.clone())
                .await
                .unwrap(),
            DeliveryOutcome::Expired
        );
        assert!(
            !registry.has_preempt(&record.agent_id).await,
            "an expired delivery must not wake the runner"
        );
        assert!(!cancel.load(Ordering::SeqCst));

        let mut old_steer = delivery(PushMode::Preempt);
        old_steer.supersede_key = Some("agent_steer:T-1".into());
        let mut new_steer = delivery(PushMode::Preempt);
        new_steer.supersede_key = Some("agent_steer:T-1".into());
        new_steer.expires_at_ms = Some(u64::MAX);
        registry
            .enqueue_delivery(&record.agent_id, old_steer.clone())
            .await
            .unwrap();
        registry
            .enqueue_delivery(&record.agent_id, new_steer.clone())
            .await
            .unwrap();
        assert_eq!(
            registry
                .delivery_status(&record.agent_id, &old_steer.id)
                .await,
            DeliveryStatus::Superseded
        );
        assert_eq!(
            registry
                .delivery_status(&record.agent_id, &new_steer.id)
                .await,
            DeliveryStatus::Pending
        );
        registry
            .update_record(&record.agent_id, |record, now| {
                record
                    .pending_deliveries
                    .iter_mut()
                    .find(|pending| pending.id == new_steer.id)
                    .unwrap()
                    .expires_at_ms = Some((now.timestamp_millis() as u64).saturating_sub(1));
                Ok(())
            })
            .await
            .unwrap();
        assert!(registry
            .drain_deliveries(&record.agent_id, true)
            .await
            .unwrap()
            .is_empty());
        assert!(
            !registry.has_preempt(&record.agent_id).await,
            "a superseded or expired preempt must leave nothing to wake for"
        );
    }

    #[tokio::test]
    async fn runner_delivery_saturates_a_huge_ttl_and_heals_legacy_records() {
        let (_temp, registry) = registry().await;
        let (record, _cancel, _) = registry.create(request("parent")).await.unwrap();
        let mut forever = delivery(PushMode::Append);
        forever.set_ttl_secs(u64::MAX);
        assert_eq!(forever.expires_at_ms, Some(u64::MAX));
        assert_eq!(
            registry
                .enqueue_delivery(&record.agent_id, forever.clone())
                .await
                .unwrap(),
            DeliveryOutcome::Queued
        );
        assert_eq!(
            registry
                .delivery_status(&record.agent_id, &forever.id)
                .await,
            DeliveryStatus::Pending
        );

        let legacy: BackgroundAgent = serde_json::from_value(
            serde_json::to_value(&record)
                .map(|mut value| {
                    let object = value.as_object_mut().unwrap();
                    object.remove("dropped_deliveries");
                    object.remove("pending_deliveries");
                    object.insert(
                        "pending_deliveries".to_string(),
                        serde_json::json!([{"id": "legacy-pending"}]),
                    );
                    value
                })
                .unwrap(),
        )
        .unwrap();
        assert!(legacy.dropped_deliveries.is_empty());
        assert_eq!(legacy.pending_deliveries[0].enqueued_at_ms, 0);
        let mut records = HashMap::from([(legacy.agent_id.clone(), legacy)]);
        normalize_delivery_state(&mut records);
        let healed = &records[&record.agent_id];
        assert_eq!(
            healed.pending_deliveries.len(),
            1,
            "an unknown enqueue time must not be read as 1970 and expired away"
        );
        assert!(healed.pending_deliveries[0].enqueued_at_ms > 0);
        assert!(healed.dropped_deliveries.is_empty());
    }

    #[tokio::test]
    async fn delivery_terminal_statuses_survive_registry_restore_and_reject_id_reuse() {
        let (temp, registry) = registry().await;
        let (record, _cancel, _) = registry.create(request("parent")).await.unwrap();
        let mut message = delivery(PushMode::Append);
        message.supersede_key = Some("steer:one".to_string());
        registry
            .enqueue_delivery(&record.agent_id, message.clone())
            .await
            .unwrap();
        registry
            .consume_delivery(&record.agent_id, &message.id)
            .await
            .unwrap();
        let restored = BackgroundAgentRegistry::new(temp.path().join("agents"))
            .await
            .unwrap();
        assert_eq!(
            restored
                .delivery_status(&record.agent_id, &message.id)
                .await,
            DeliveryStatus::Consumed
        );
        let reopened = restored.get_any(&record.agent_id).await.unwrap();
        let (_runtime_record, _, _) = restored
            .reopen(&record.agent_id, "parent", reopened.change_seq, false)
            .await
            .unwrap();
        assert_eq!(
            restored
                .enqueue_delivery(&record.agent_id, message)
                .await
                .unwrap(),
            DeliveryOutcome::Duplicate
        );
    }

    #[tokio::test]
    async fn delivery_boundaries_dedupe_and_reprioritize() {
        let (_temp, registry) = registry().await;
        let (record, cancel, _) = registry.create(request("parent")).await.unwrap();
        let idle = delivery(PushMode::WhenIdle);
        let append = delivery(PushMode::Append);
        registry
            .enqueue_delivery(&record.agent_id, idle.clone())
            .await
            .unwrap();
        registry
            .enqueue_delivery(&record.agent_id, append.clone())
            .await
            .unwrap();
        assert_eq!(
            registry
                .enqueue_delivery(&record.agent_id, append.clone())
                .await
                .unwrap(),
            DeliveryOutcome::Duplicate
        );
        let safe = registry
            .drain_deliveries(&record.agent_id, false)
            .await
            .unwrap();
        assert_eq!(safe.len(), 1);
        assert_eq!(safe[0].id, append.id);
        assert_eq!(registry.pending_deliveries(&record.agent_id).await.len(), 2);
        registry
            .acknowledge_delivery(&record.agent_id, &append.id)
            .await
            .unwrap();
        registry
            .update_pending_delivery(&record.agent_id, &idle.id, Some(PushMode::Preempt), false)
            .await
            .unwrap();
        assert!(registry.has_preempt(&record.agent_id).await);
        assert!(
            !cancel.load(Ordering::SeqCst),
            "preempt must not cancel the runner"
        );
        registry
            .update_pending_delivery(&record.agent_id, &idle.id, None, true)
            .await
            .unwrap();
        assert!(registry
            .pending_deliveries(&record.agent_id)
            .await
            .is_empty());
        assert_eq!(
            registry
                .enqueue_delivery(&record.agent_id, idle)
                .await
                .unwrap(),
            DeliveryOutcome::Duplicate
        );
    }

    #[tokio::test]
    async fn claimed_delivery_survives_restart_until_acknowledged() {
        let (temp, registry) = registry().await;
        let (record, _, _) = registry.create(request("parent")).await.unwrap();
        let message = delivery(PushMode::Append);
        registry
            .enqueue_delivery(&record.agent_id, message.clone())
            .await
            .unwrap();
        assert_eq!(
            registry
                .drain_deliveries(&record.agent_id, false)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(registry
            .drain_deliveries(&record.agent_id, false)
            .await
            .unwrap()
            .is_empty());
        let restored = BackgroundAgentRegistry::new(temp.path().join("agents"))
            .await
            .unwrap();
        assert_eq!(
            restored.get_any(&record.agent_id).await.unwrap().followups[0].id,
            message.id
        );
        registry
            .acknowledge_delivery(&record.agent_id, &message.id)
            .await
            .unwrap();
        let saved = storage::load_all(&temp.path().join("agents"))
            .await
            .unwrap();
        assert!(saved[&record.agent_id].pending_deliveries.is_empty());
        assert!(saved[&record.agent_id].delivery_ids.contains(&message.id));
    }

    #[tokio::test]
    async fn final_boundary_rearms_wake_and_seals_non_wake() {
        let (_temp, registry) = registry().await;
        let (record, _, _) = registry.create(request("parent")).await.unwrap();
        let wake = delivery(PushMode::WhenIdle);
        registry
            .enqueue_delivery(&record.agent_id, wake)
            .await
            .unwrap();
        assert_eq!(
            registry
                .finish_delivery_turn(&record.agent_id)
                .await
                .unwrap()
                .len(),
            1
        );
        let mut quiet = delivery(PushMode::WhenIdle);
        quiet.wake = false;
        registry
            .enqueue_delivery(&record.agent_id, quiet)
            .await
            .unwrap();
        assert_eq!(
            registry
                .finish_delivery_turn(&record.agent_id)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(registry
            .enqueue_delivery(&record.agent_id, delivery(PushMode::Append))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn racing_final_boundary_never_strands_accepted_delivery() {
        let (_temp, registry) = registry().await;
        for _ in 0..20 {
            let (record, _, _) = registry.create(request("parent")).await.unwrap();
            let message = delivery(PushMode::WhenIdle);
            let (accepted, drained) = tokio::join!(
                registry.enqueue_delivery(&record.agent_id, message.clone()),
                registry.finish_delivery_turn(&record.agent_id),
            );
            let drained = drained.unwrap();
            if accepted.is_ok() {
                assert!(drained.iter().any(|delivery| delivery.id == message.id));
            } else {
                assert!(drained.is_empty());
            }
        }
    }

    #[tokio::test]
    async fn closing_is_terminal_resumable_idempotent_and_refuses_a_stale_generation() {
        let (_temp, registry) = registry().await;
        let (record, abort, _) = registry.create(request("parent")).await.unwrap();
        let record = registry
            .mark_running(&record.agent_id, "child-chat".into())
            .await
            .unwrap();
        let runtime = registry.runtime.read().await[&record.agent_id].clone();
        runtime.pause.request();
        let held = runtime.pause.hold(Some(&abort));
        tokio::pin!(held);
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(held.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        assert!(runtime.pause.is_quiescent());
        let pause_woken = runtime.pause.wake.notified();
        let notified = runtime.notify.notified();
        let interrupted = runtime.interrupt_notify.notified();
        tokio::pin!(pause_woken, notified, interrupted);
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(pause_woken.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(notified.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(interrupted.as_mut().poll(cx)))
                .await
                .is_pending()
        );

        assert!(registry
            .mark_closed(&record.agent_id, Some(7), None)
            .await
            .unwrap_err()
            .contains("another generation"));
        assert!(!abort.load(Ordering::SeqCst));
        assert!(registry
            .close("parent", &record.agent_id, Some(7), None)
            .await
            .unwrap_err()
            .contains("another generation"));
        assert!(registry
            .close_with("other-parent", &record.agent_id, None, None, |_| {
                panic!("foreign close released an execution")
            })
            .await
            .is_err());
        assert!(!abort.load(Ordering::SeqCst));
        assert_eq!(registry.get_any(&record.agent_id).await.unwrap(), record);
        assert!(Arc::ptr_eq(
            &registry.abort_flag(&record.agent_id).await.unwrap(),
            &abort
        ));
        assert!(Arc::ptr_eq(
            &registry.pause_gate(&record.agent_id).await.unwrap(),
            &runtime.pause
        ));
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(held.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(pause_woken.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(notified.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(interrupted.as_mut().poll(cx)))
                .await
                .is_pending()
        );

        let released = AtomicBool::new(false);
        let disk = registry.storage_write.lock().await;
        let close = registry.close_with(
            "parent",
            &record.agent_id,
            Some(record.run_generation),
            Some("held for later".into()),
            |closed| {
                assert_eq!(closed.run_generation, record.run_generation);
                assert_eq!(closed.child_chat_id, record.child_chat_id);
                assert!(abort.load(Ordering::SeqCst));
                released.store(true, Ordering::SeqCst);
            },
        );
        tokio::pin!(close);
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(close.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        assert!(released.load(Ordering::SeqCst));
        assert!(abort.load(Ordering::SeqCst));
        assert!(matches!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(held.as_mut().poll(cx))).await,
            std::task::Poll::Ready(true)
        ));
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(pause_woken.as_mut().poll(cx)))
                .await
                .is_ready()
        );
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(notified.as_mut().poll(cx)))
                .await
                .is_ready()
        );
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(interrupted.as_mut().poll(cx)))
                .await
                .is_ready()
        );
        assert!(registry.has_runtime(&record.agent_id).await);
        drop(disk);
        let (closed, already) = close.await.unwrap();
        assert!(!already);
        assert_eq!(closed.status, BgAgentStatus::Closed);
        assert!(closed.status.is_terminal());
        assert_eq!(closed.status.as_str(), "closed");
        assert_eq!(closed.close_reason.as_deref(), Some("held for later"));
        assert!(closed.finished_at.is_some());
        assert!(abort.load(Ordering::SeqCst));
        assert!(!registry.has_runtime(&record.agent_id).await);

        let (again, already) = registry
            .close_with(
                "parent",
                &record.agent_id,
                None,
                Some("second".into()),
                |_| panic!("idempotent close released an execution"),
            )
            .await
            .unwrap();
        assert!(already);
        assert_eq!(again.change_seq, closed.change_seq);
        assert_eq!(again.close_reason.as_deref(), Some("held for later"));

        for late in [
            registry
                .mark_cancelled(&record.agent_id, Some("too late".into()))
                .await
                .unwrap(),
            registry
                .mark_failed(&record.agent_id, "too late".into())
                .await
                .unwrap(),
            registry
                .mark_completed(&record.agent_id, completion())
                .await
                .unwrap(),
        ] {
            assert_eq!(late.status, BgAgentStatus::Closed);
            assert_eq!(late.change_seq, closed.change_seq);
        }

        assert!(registry
            .close("other-parent", &record.agent_id, None, None)
            .await
            .is_err());

        let (reopened, fresh_abort, _) = registry
            .reopen(&record.agent_id, "parent", closed.change_seq, false)
            .await
            .unwrap();
        assert_eq!(reopened.run_generation, record.run_generation + 1);
        assert!(!Arc::ptr_eq(&abort, &fresh_abort));
        assert!(registry
            .close_with(
                "parent",
                &record.agent_id,
                Some(record.run_generation),
                None,
                |_| panic!("stale close released a reopened execution"),
            )
            .await
            .unwrap_err()
            .contains("another generation"));
        assert!(!fresh_abort.load(Ordering::SeqCst));
        assert_eq!(registry.get_any(&record.agent_id).await.unwrap(), reopened);
        assert!(Arc::ptr_eq(
            &registry.abort_flag(&record.agent_id).await.unwrap(),
            &fresh_abort
        ));
    }

    #[tokio::test]
    async fn closing_a_finished_child_is_refused_and_reopen_keeps_its_outcome() {
        let (_temp, registry) = registry().await;
        let (record, _, _) = registry.create(request("parent")).await.unwrap();
        registry
            .set_worktree_metadata(
                &record.agent_id,
                Some("wt-first".into()),
                Some("child-first".into()),
            )
            .await
            .unwrap();
        registry
            .mark_running(&record.agent_id, "child-chat".into())
            .await
            .unwrap();
        registry
            .update_progress(&record.agent_id, "worked".into(), 3)
            .await
            .unwrap();
        registry
            .add_usage(&record.agent_id, 42, Some(0.25))
            .await
            .unwrap();
        registry
            .mark_completed(&record.agent_id, completion())
            .await
            .unwrap();
        let completed = registry
            .set_merge_status(&record.agent_id, "merged")
            .await
            .unwrap();

        assert!(registry
            .close("parent", &record.agent_id, None, None)
            .await
            .unwrap_err()
            .contains("already finished as completed"));
        assert_eq!(
            registry
                .get_any(&record.agent_id)
                .await
                .unwrap()
                .result_summary
                .as_deref(),
            Some("done")
        );

        let (continued, _, _) = registry
            .reopen(&record.agent_id, "parent", completed.change_seq, false)
            .await
            .unwrap();
        assert_eq!(continued.status, BgAgentStatus::Queued);
        assert_eq!(continued.run_generation, 2);
        assert!(continued.result_summary.is_none());
        assert_eq!(continued.step_count, 3);
        assert_eq!(continued.tokens_used, 42);
        assert!(continued.merge_status.is_none());
        assert_eq!(continued.previous_runs.len(), 1);
        assert_eq!(continued.previous_runs[0].run_generation, 1);
        assert_eq!(continued.previous_runs[0].status, BgAgentStatus::Completed);
        assert_eq!(
            continued.previous_runs[0].result_summary.as_deref(),
            Some("done")
        );
        assert_eq!(continued.previous_runs[0].step_count, 3);
        assert_eq!(
            continued.previous_runs[0].worktree_id.as_deref(),
            Some("wt-first")
        );
        assert_eq!(
            continued.previous_runs[0].worktree_branch.as_deref(),
            Some("child-first")
        );
        assert_eq!(
            continued.previous_runs[0].merge_status.as_deref(),
            Some("merged")
        );
        let restored: BackgroundAgent =
            serde_json::from_value(serde_json::to_value(&continued).unwrap()).unwrap();
        assert_eq!(restored.previous_runs, continued.previous_runs);
        let mut legacy = serde_json::to_value(&continued.previous_runs[0]).unwrap();
        for field in ["worktree_id", "worktree_branch", "merge_status"] {
            legacy.as_object_mut().unwrap().remove(field);
        }
        let legacy: AgentRunOutcome = serde_json::from_value(legacy).unwrap();
        assert!(legacy.worktree_id.is_none());
        assert!(legacy.worktree_branch.is_none());
        assert!(legacy.merge_status.is_none());
        let abort = registry.abort_flag(&record.agent_id).await.unwrap();
        let operation = uuid::Uuid::new_v4();
        assert!(registry
            .begin_checkout_setup(
                "other-parent",
                &record.agent_id,
                2,
                continued.change_seq,
                &abort,
                None,
                operation
            )
            .await
            .is_err());
        let setup = registry
            .begin_checkout_setup(
                "parent",
                &record.agent_id,
                2,
                continued.change_seq,
                &abort,
                None,
                operation,
            )
            .await
            .unwrap();
        let payload = storage::AcceptedLaunchPayload::serialize(
            &serde_json::json!({"operation": operation}),
            256,
        )
        .unwrap();
        let changed = registry
            .update_progress(&record.agent_id, "concurrent edit".into(), 3)
            .await
            .unwrap();
        let refused = registry
            .commit_checkout_setup(&setup, "wt-stale".into(), None, &payload, |_| {})
            .await
            .unwrap_err();
        assert!(!refused.retain_checkout);
        assert_eq!(registry.get_any(&record.agent_id).await.unwrap(), changed);
        assert!(
            storage::load_checkout_setup(&registry.storage_root, &record.agent_id, 2, 256)
                .await
                .unwrap()
                .is_none()
        );
        let setup = registry
            .begin_checkout_setup(
                "parent",
                &record.agent_id,
                2,
                changed.change_seq,
                &abort,
                None,
                operation,
            )
            .await
            .unwrap();
        let remembered = std::sync::atomic::AtomicBool::new(false);
        let disk = registry.storage_write.lock().await;
        let publication = registry.commit_checkout_setup(
            &setup,
            "wt-fresh".into(),
            Some("child-fresh".into()),
            &payload,
            |record| {
                assert_eq!(record.worktree_id.as_deref(), Some("wt-fresh"));
                remembered.store(true, Ordering::Release);
            },
        );
        tokio::pin!(publication);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                assert!(std::future::poll_fn(|cx| std::task::Poll::Ready(
                    std::future::Future::poll(publication.as_mut(), cx)
                ))
                .await
                .is_pending());
                if remembered.load(Ordering::Acquire) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            registry
                .get_any(&record.agent_id)
                .await
                .unwrap()
                .worktree_id
                .as_deref(),
            Some("wt-fresh")
        );
        assert_eq!(
            storage::load_checkout_setup(&registry.storage_root, &record.agent_id, 2, 256)
                .await
                .unwrap()
                .unwrap()
                .as_bytes(),
            payload.as_bytes()
        );
        assert!(registry
            .commit_checkout_setup(&setup, "duplicate".into(), None, &payload, |_| panic!(
                "duplicate publication remembered configuration"
            ))
            .await
            .is_err());
        drop(disk);
        let published = publication.await.unwrap();
        assert_eq!(published.worktree_id.as_deref(), Some("wt-fresh"));
        assert_eq!(published.previous_runs, continued.previous_runs);
        let result_path = completed.result_payload_path.as_ref().unwrap();
        let result = std::fs::read(result_path).unwrap();
        let failed = registry
            .finish_queued_resume(
                &record.agent_id,
                2,
                &abort,
                "setup interrupted".into(),
                false,
            )
            .await
            .unwrap();
        assert_eq!(failed.status, BgAgentStatus::Failed);
        assert!(failed.result_payload_path.is_none());
        assert_eq!(std::fs::read(result_path).unwrap(), result);
        let (newer, newer_abort, _) = registry
            .reopen(&record.agent_id, "parent", failed.change_seq, false)
            .await
            .unwrap();
        assert!(registry
            .finish_queued_resume(&record.agent_id, 2, &abort, "stale failure".into(), true)
            .await
            .is_err());
        assert!(!registry
            .rollback_unaccepted_resume(&completed, &continued, &abort)
            .await
            .unwrap());
        assert!(registry
            .commit_checkout_setup(&setup, "wt-stale".into(), None, &payload, |_| {})
            .await
            .is_err());
        assert_eq!(registry.get_any(&record.agent_id).await.unwrap(), newer);
        assert!(!newer_abort.load(Ordering::Acquire));
        assert_eq!(
            storage::load_checkout_setup(&registry.storage_root, &record.agent_id, 2, 256)
                .await
                .unwrap()
                .unwrap()
                .as_bytes(),
            payload.as_bytes()
        );
    }

    #[test]
    fn legacy_records_load_without_lifecycle_fields_and_history_stays_bounded() {
        let now = Utc::now();
        let legacy = serde_json::json!({
            "schema_version": 1,
            "agent_id": "bgagent-legacy",
            "parent_chat_id": "parent",
            "parent_root_chat_id": null,
            "parent_tool_call_id": null,
            "child_chat_id": "child",
            "kind": "subagent",
            "config_name": "test",
            "title": "Legacy",
            "prompt": "legacy prompt",
            "target_files": [],
            "status": "completed",
            "progress": null,
            "step_count": 2,
            "last_activity": null,
            "result_summary": "legacy answer",
            "result_payload_path": null,
            "error": null,
            "edited_files": [],
            "diff_summary": null,
            "conflict_summary": null,
            "completion_message_id": null,
            "completion_pushed_at": null,
            "model": "test-model",
            "created_at": now,
            "started_at": null,
            "finished_at": now,
            "last_update_at": now,
            "change_seq": 4,
        });
        let legacy: BackgroundAgent = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.run_generation, 1);
        assert!(legacy.close_reason.is_none());
        assert!(legacy.previous_runs.is_empty());
        assert!(legacy.followups.is_empty());

        let retained = refact_work_core::coordination_settings().agent_closed_retained_max;
        let mut overfull = legacy.clone();
        overfull.previous_runs = (0..retained as u64 + 6)
            .map(|generation| AgentRunOutcome {
                run_generation: generation,
                status: BgAgentStatus::Closed,
                step_count: 0,
                result_summary: None,
                error: None,
                finished_at: None,
                worktree_id: None,
                worktree_branch: None,
                merge_status: None,
            })
            .collect();
        overfull.close_reason = Some("stale reason on a completed record".into());
        let mut records = HashMap::from([(overfull.agent_id.clone(), overfull)]);
        normalize_run_history(&mut records);
        let healed = &records["bgagent-legacy"];
        assert_eq!(healed.previous_runs.len(), retained);
        assert_eq!(
            healed.previous_runs.last().unwrap().run_generation,
            retained as u64 + 5,
            "the newest generations survive the bound"
        );
        assert!(
            healed.close_reason.is_none(),
            "only a closed record carries a close reason"
        );

        assert_eq!(
            serde_json::to_value(BgAgentStatus::Closed).unwrap(),
            serde_json::json!("closed")
        );
        assert_eq!(
            serde_json::from_value::<BgAgentStatus>(serde_json::json!("closed")).unwrap(),
            BgAgentStatus::Closed
        );
    }

    #[tokio::test]
    async fn pause_gate_parks_until_release_and_cancel_always_wakes() {
        let (_temp, registry) = registry().await;
        let (record, abort, _) = registry.create(request("parent")).await.unwrap();
        let gate = registry.pause_gate(&record.agent_id).await.unwrap();
        assert!(!gate.hold(Some(&abort)).await);
        let status = registry.request_pause(&record.agent_id).await.unwrap();
        assert!(status.requested && !status.quiescent);
        let held = tokio::spawn({
            let gate = gate.clone();
            let abort = abort.clone();
            async move { gate.hold(Some(&abort)).await }
        });
        gate.quiesced().await;
        assert!(
            registry
                .pause_status(&record.agent_id)
                .await
                .unwrap()
                .quiescent
        );
        registry.release_pause(&record.agent_id).await.unwrap();
        assert!(!held.await.unwrap());
        assert!(!gate.is_quiescent());
        registry.request_pause(&record.agent_id).await.unwrap();
        let held = tokio::spawn({
            let gate = gate.clone();
            let abort = abort.clone();
            async move { gate.hold(Some(&abort)).await }
        });
        gate.quiesced().await;
        registry
            .cancel("parent", &record.agent_id, Some("cancel".into()))
            .await
            .unwrap();
        assert!(held.await.unwrap());
        assert!(registry.request_pause(&record.agent_id).await.is_err());
        assert!(registry.release_pause(&record.agent_id).await.is_err());
    }

    #[tokio::test]
    async fn native_notes_dedupe_by_control_identity_and_reject_at_capacity() {
        let (_temp, registry) = registry().await;
        let (record, _, _) = registry.create(request("parent")).await.unwrap();
        let note = |control: &str| {
            NativeNote::new(control.to_string(), "n1".to_string(), |_, _| {
                Ok("text".into())
            })
        };
        assert!(registry
            .push_native_note(&record.agent_id, note("c1"))
            .await
            .unwrap());
        assert!(!registry
            .push_native_note(&record.agent_id, note("c1"))
            .await
            .unwrap());
        for index in 1..MAX_NATIVE_NOTES {
            registry
                .push_native_note(&record.agent_id, note(&format!("c{index}-extra")))
                .await
                .unwrap();
        }
        assert!(registry
            .push_native_note(&record.agent_id, note("overflow"))
            .await
            .is_err());
        assert_eq!(
            registry
                .pause_status(&record.agent_id)
                .await
                .unwrap()
                .queued_native_notes,
            MAX_NATIVE_NOTES
        );
        let drained = registry.drain_native_notes(&record.agent_id).await;
        assert_eq!(drained.len(), MAX_NATIVE_NOTES);
        assert_eq!(drained[0].control_id, "c1");
        assert!(registry
            .drain_native_notes(&record.agent_id)
            .await
            .is_empty());
        registry
            .mark_completed(&record.agent_id, completion())
            .await
            .unwrap();
        assert!(registry
            .push_native_note(&record.agent_id, note("late"))
            .await
            .is_err());
    }
}
