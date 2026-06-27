use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, Notify, Mutex as AMutex};

use crate::call_validation::{ChatMessage, ChatUsage};
use crate::git::checkpoints::Checkpoint;
use super::config::{limits, timeouts};

pub use refact_chat_api::{
    ActiveCommandContext, BackgroundAgentSummary, BrowserMeta, BrowserSnapshot, BrowserTabInfo,
    BuddyThreadMeta, ChatCommand, ChatEvent, CommandRequest, CompressionPhase, CompressionReason,
    DeltaOp, DiffBox, EventEnvelope, GoalAttempt, GoalBudget, GoalEvent, GoalProgress,
    GoalSnapshot, GoalStatus, PauseReason, PendingSkillDeactivation, QueuedItem, RuntimeState,
    SessionState, TaskMeta, ThreadParams, TimelineEntry, ToolDecisionItem, WindowBounds,
    WorktreeMeta,
};

pub fn max_queue_size() -> usize {
    limits().max_queue_size
}
pub fn session_idle_timeout() -> std::time::Duration {
    timeouts().session_idle
}
pub fn session_cleanup_interval() -> std::time::Duration {
    timeouts().session_cleanup_interval
}
pub fn stream_idle_timeout() -> std::time::Duration {
    timeouts().stream_idle
}
pub fn stream_total_timeout() -> std::time::Duration {
    timeouts().stream_total
}
pub fn stream_heartbeat() -> std::time::Duration {
    timeouts().stream_heartbeat
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueCommandOutcome {
    Accepted,
    Duplicate,
    Full,
}

#[derive(Debug)]
pub struct BurstGuard {
    inner: tokio::sync::Mutex<BurstGuardInner>,
}

#[derive(Debug, Default)]
struct BurstGuardInner {
    recent: VecDeque<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BurstGuardDecision {
    Allow,
    Defer,
}

impl BurstGuard {
    pub fn new() -> Self {
        Self {
            inner: tokio::sync::Mutex::new(BurstGuardInner::default()),
        }
    }

    pub async fn record_and_check(&self) -> BurstGuardDecision {
        let now = chrono::Utc::now();
        let mut guard = self.inner.lock().await;
        while let Some(front) = guard.recent.front() {
            if now.signed_duration_since(*front).num_seconds() > 10 {
                guard.recent.pop_front();
            } else {
                break;
            }
        }
        if guard.recent.len() >= 5 {
            BurstGuardDecision::Defer
        } else {
            guard.recent.push_back(now);
            BurstGuardDecision::Allow
        }
    }
}

impl Default for BurstGuard {
    fn default() -> Self {
        Self::new()
    }
}

fn epoch_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub trait GoalSnapshotBudgetExt {
    fn goal_budget_exhaustion_status_at(&self, now_ms: u64) -> Option<GoalStatus>;
    fn goal_budget_exhausted_at(&self, now_ms: u64) -> bool;
    fn goal_budget_exhausted(&self) -> bool;
    fn goal_can_pursue_at(&self, now_ms: u64) -> bool;
    fn goal_can_pursue(&self) -> bool;
    fn goal_nudge_ready_at(&self, now_ms: u64) -> bool;
    fn goal_record_progress(&mut self, tokens: u64, made_progress: bool);
    fn goal_record_verifier_attempt(&mut self, tokens: u64);
    fn goal_record_nudge(&mut self, at_ms: u64);
    fn goal_reset_no_progress(&mut self);
    fn goal_push_attempt(&mut self, attempt: GoalAttempt);
    fn goal_push_event(&mut self, event: GoalEvent);
}

impl GoalSnapshotBudgetExt for GoalSnapshot {
    fn goal_budget_exhaustion_status_at(&self, now_ms: u64) -> Option<GoalStatus> {
        if let Some(no_progress_turns) = self.budget.no_progress_turns {
            if no_progress_turns > 0 && self.progress.no_progress_turns >= no_progress_turns {
                return Some(GoalStatus::NoProgress);
            }
        }
        if let Some(max_turns) = self.budget.max_turns {
            if max_turns > 0 && self.progress.turns_used >= max_turns {
                return Some(GoalStatus::BudgetExhausted);
            }
        }
        if let Some(max_tokens) = self.budget.max_tokens {
            if max_tokens > 0 && self.progress.tokens_used >= max_tokens {
                return Some(GoalStatus::BudgetExhausted);
            }
        }
        if let Some(max_minutes) = self.budget.max_minutes {
            let max_ms = u64::from(max_minutes).saturating_mul(60_000);
            if max_minutes > 0
                && self.progress.started_at_ms > 0
                && now_ms.saturating_sub(self.progress.started_at_ms) >= max_ms
            {
                return Some(GoalStatus::BudgetExhausted);
            }
        }
        None
    }

    fn goal_budget_exhausted_at(&self, now_ms: u64) -> bool {
        self.goal_budget_exhaustion_status_at(now_ms).is_some()
    }

    fn goal_budget_exhausted(&self) -> bool {
        self.goal_budget_exhausted_at(epoch_ms_now())
    }

    fn goal_can_pursue_at(&self, now_ms: u64) -> bool {
        self.active && self.status == GoalStatus::Active && !self.goal_budget_exhausted_at(now_ms)
    }

    fn goal_can_pursue(&self) -> bool {
        self.goal_can_pursue_at(epoch_ms_now())
    }

    fn goal_nudge_ready_at(&self, now_ms: u64) -> bool {
        self.progress.last_nudge_at_ms == 0
            || now_ms
                >= self
                    .progress
                    .last_nudge_at_ms
                    .saturating_add(self.budget.cooldown_ms)
    }

    fn goal_record_progress(&mut self, tokens: u64, made_progress: bool) {
        if self.progress.started_at_ms == 0 {
            self.progress.started_at_ms = epoch_ms_now();
        }
        self.progress.turns_used = self.progress.turns_used.saturating_add(1);
        self.progress.tokens_used = self.progress.tokens_used.saturating_add(tokens);
        if made_progress {
            self.progress.no_progress_turns = 0;
        } else {
            self.progress.no_progress_turns = self.progress.no_progress_turns.saturating_add(1);
        }
        match self.goal_budget_exhaustion_status_at(epoch_ms_now()) {
            Some(status) => self.status = status,
            None if matches!(
                self.status,
                GoalStatus::BudgetExhausted | GoalStatus::NoProgress
            ) =>
            {
                self.status = GoalStatus::Active;
            }
            None => {}
        }
    }

    fn goal_record_verifier_attempt(&mut self, tokens: u64) {
        if self.progress.started_at_ms == 0 {
            self.progress.started_at_ms = epoch_ms_now();
        }
        self.progress.turns_used = self.progress.turns_used.saturating_add(1);
        self.progress.tokens_used = self.progress.tokens_used.saturating_add(tokens);
        match self.goal_budget_exhaustion_status_at(epoch_ms_now()) {
            Some(status) => self.status = status,
            None if matches!(
                self.status,
                GoalStatus::BudgetExhausted | GoalStatus::NoProgress
            ) =>
            {
                self.status = GoalStatus::Active;
            }
            None => {}
        }
    }

    fn goal_record_nudge(&mut self, at_ms: u64) {
        self.progress.last_nudge_at_ms = at_ms;
    }

    fn goal_reset_no_progress(&mut self) {
        self.progress.no_progress_turns = 0;
        if matches!(
            self.status,
            GoalStatus::BudgetExhausted | GoalStatus::NoProgress
        ) {
            self.status = self
                .goal_budget_exhaustion_status_at(epoch_ms_now())
                .unwrap_or(GoalStatus::Active);
        }
    }

    fn goal_push_attempt(&mut self, attempt: GoalAttempt) {
        self.attempts.push(attempt);
    }

    fn goal_push_event(&mut self, event: GoalEvent) {
        self.events.push(event);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum TrajectorySourceIdentity {
    #[default]
    Normal,
    Task {
        task_id: String,
        role: String,
        agent_id: Option<String>,
        card_id: Option<String>,
        planner_chat_id: Option<String>,
    },
    Buddy,
}

impl TrajectorySourceIdentity {
    pub(crate) fn task(
        task_id: String,
        role: String,
        agent_id: Option<String>,
        card_id: Option<String>,
        planner_chat_id: Option<String>,
    ) -> Self {
        Self::Task {
            task_id,
            role,
            agent_id,
            card_id,
            planner_chat_id,
        }
    }

    pub(crate) fn from_task_meta(task_meta: &TaskMeta) -> Self {
        Self::task(
            task_meta.task_id.clone(),
            task_meta.role.clone(),
            task_meta.agent_id.clone(),
            task_meta.card_id.clone(),
            task_meta.planner_chat_id.clone(),
        )
    }

    pub(crate) fn from_extra(
        extra: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<Self, String> {
        let buddy_meta_present = extra
            .get("buddy_meta")
            .is_some_and(|value| !value.is_null());
        let task_meta_value = extra.get("task_meta").filter(|value| !value.is_null());

        if buddy_meta_present && task_meta_value.is_some() {
            return Err("trajectory cannot contain both task_meta and buddy_meta".to_string());
        }
        if buddy_meta_present {
            return Ok(Self::Buddy);
        }
        if let Some(value) = task_meta_value {
            let task_meta = serde_json::from_value::<TaskMeta>(value.clone())
                .map_err(|e| format!("invalid task_meta: {}", e))?;
            return Ok(Self::from_task_meta(&task_meta));
        }
        Ok(Self::Normal)
    }

    pub(crate) fn from_json(json: &serde_json::Value) -> Result<Self, String> {
        let Some(root) = json.as_object() else {
            return Err("trajectory JSON root must be an object".to_string());
        };
        Self::from_extra(root)
    }

    pub(crate) fn from_session_parts(thread: &ThreadParams) -> Self {
        if thread.buddy_meta.is_some() {
            Self::Buddy
        } else if let Some(task_meta) = thread.task_meta.as_ref() {
            Self::from_task_meta(task_meta)
        } else {
            Self::Normal
        }
    }

    pub(crate) fn from_session(session: &ChatSession) -> Self {
        Self::from_session_parts(&session.thread)
    }

    pub(crate) fn emits_generic_event(&self) -> bool {
        !matches!(self, Self::Buddy)
    }

    pub(crate) fn matches_session(&self, session: &ChatSession) -> bool {
        &Self::from_session(session) == self
    }

    pub(crate) fn matches_session_for_delete(&self, session: &ChatSession) -> bool {
        let active_source = Self::from_session(session);
        match (self, active_source) {
            (
                Self::Task {
                    task_id,
                    role,
                    agent_id,
                    card_id,
                    planner_chat_id,
                },
                Self::Task {
                    task_id: active_task_id,
                    role: active_role,
                    agent_id: active_agent_id,
                    card_id: active_card_id,
                    planner_chat_id: active_planner_chat_id,
                },
            ) => {
                task_id == &active_task_id
                    && role == &active_role
                    && agent_id
                        .as_ref()
                        .is_none_or(|agent_id| Some(agent_id) == active_agent_id.as_ref())
                    && card_id
                        .as_ref()
                        .is_none_or(|card_id| Some(card_id) == active_card_id.as_ref())
                    && planner_chat_id.as_ref().is_none_or(|planner_chat_id| {
                        Some(planner_chat_id) == active_planner_chat_id.as_ref()
                    })
            }
            (left, right) => left == &right,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ExternalReloadPending {
    Update { source: TrajectorySourceIdentity },
    Delete { source: TrajectorySourceIdentity },
}

impl ExternalReloadPending {
    pub(crate) fn update(source: TrajectorySourceIdentity) -> Self {
        Self::Update { source }
    }

    pub(crate) fn delete(source: TrajectorySourceIdentity) -> Self {
        Self::Delete { source }
    }
}

pub struct ChatSession {
    pub chat_id: String,
    pub thread: ThreadParams,
    pub messages: Vec<ChatMessage>,
    pub runtime: RuntimeState,
    pub goal: Option<GoalSnapshot>,
    pub goal_active: bool,
    pub goal_status: Option<GoalStatus>,
    pub goal_turns_used: u32,
    pub goal_tokens_used: u64,
    pub goal_no_progress_turns: u32,
    pub is_compressing: bool,
    pub compression_phase: Option<CompressionPhase>,
    pub compression_reason: Option<CompressionReason>,
    pub(crate) compression_attempt_generation: u64,
    pub(crate) active_compression_attempt: Option<u64>,
    pub(crate) compression_attempt_started_at_ms: Option<u64>,
    pub draft_message: Option<ChatMessage>,
    pub draft_usage: Option<ChatUsage>,
    pub command_queue: VecDeque<CommandRequest>,
    pub event_seq: u64,
    pub event_tx: broadcast::Sender<Arc<String>>,
    pub trajectory_events_tx: Option<broadcast::Sender<super::trajectories::TrajectoryEvent>>,
    pub recent_request_ids: VecDeque<String>,
    pub recent_request_ids_set: HashSet<String>,
    pub abort_flag: Arc<AtomicBool>,
    pub abort_notify: Arc<Notify>,
    pub user_interrupt_flag: Arc<AtomicBool>,
    pub queue_processor_running: Arc<AtomicBool>,
    pub queue_notify: Arc<Notify>,
    pub last_activity: Instant,
    pub last_stream_delta_at: Option<Instant>,
    pub last_tool_started_at: Option<Instant>,
    pub last_tool_progress_at: Option<Instant>,
    pub trajectory_dirty: bool,
    pub trajectory_version: u64,
    pub trajectory_save_in_flight: bool,
    pub trajectory_save_queued: bool,
    pub trajectory_save_mutex: Arc<AMutex<()>>,
    pub created_at: String,
    pub closed: bool,
    pub closed_flag: Arc<AtomicBool>,
    pub external_reload_pending: Option<ExternalReloadPending>,
    pub last_prompt_messages: Vec<ChatMessage>,
    pub tier1_compact_attempts: usize,
    pub tier1_compaction_disabled: bool,
    pub compression_insufficient_hashes: HashSet<String>,
    pub pending_max_new_tokens_boost: Option<usize>,
    pub cache_guard_snapshot: Option<serde_json::Value>,
    pub cache_guard_force_next: bool,
    pub provider_usage_stale: bool,
    pub task_agent_error: Option<String>,
    pub pending_browser_message: Option<PendingBrowserMessage>,
    pub post_tool_side_effects: VecDeque<ChatMessage>,
    pub active_command: ActiveCommandContext,
    pub skills_available_count: usize,
    pub skills_included: Vec<String>,
    pub pending_skill_deactivation: Option<PendingSkillDeactivation>,
    pub stop_hook_handle: Option<tokio::task::JoinHandle<()>>,
    pub(crate) openai_codex_websocket: super::openai_codex_ws::OpenAICodexWebSocketSession,
    pub suppress_auto_enrichment_for_next_turn: bool,
    pub wake_up_at: Option<chrono::DateTime<chrono::Utc>>,
    pub waiting_for_card_ids: Vec<String>,
    pub background_completion_burst: BurstGuard,
    /// Latest known background agent summaries for this parent chat, keyed by `agent_id`.
    /// Kept in sync by `emit_background_agent_update` and snapshot enrichment paths so
    /// every `ChatEvent::Snapshot` carries the current agent set instead of an empty list.
    pub background_agents: HashMap<String, BackgroundAgentSummary>,
}

#[derive(Debug, Clone)]
pub struct PendingBrowserMessage {
    pub pending_message_id: String,
    pub content: serde_json::Value,
    pub attachments: Vec<serde_json::Value>,
    pub checkpoints: Vec<Checkpoint>,
    pub context_files: Vec<serde_json::Value>,
    pub suppress_auto_enrichment: bool,
    pub skill_activation_name: Option<String>,
    pub skill_context_msg: Option<ChatMessage>,
}
