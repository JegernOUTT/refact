use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use serde_json::json;
use tokio::sync::{broadcast, Mutex as AMutex, Notify, RwLock as ARwLock};
use tracing::{info, warn};
use uuid::Uuid;

use crate::agents::types::AgentListFilter;
use crate::app_state::AppState;
use crate::call_validation::{ChatContent, ChatMessage, ChatUsage};
use crate::chat::diagnostics::make_ui_only_error_message;
use crate::chat::internal_roles::{event, EventSubkind, GOAL_ROLE};
use crate::exec::{ExecMode, ExecProcessFilter, ExecProcessSnapshot, ExecStatusKind};
use crate::ext::hooks::HookEvent;
use crate::ext::hooks_runner::{HookPayload, get_project_dir_string, run_hooks};

use super::types::*;
use super::types::{session_idle_timeout, session_cleanup_interval};
use super::config::limits;
use super::trajectories::{
    first_system_prompt, task_context_from_task_meta, trajectory_meta_title, TrajectoryEvent,
};

pub(super) fn has_displayable_assistant_content(message: &ChatMessage) -> bool {
    let has_text_content = match &message.content {
        ChatContent::SimpleText(s) => !s.trim().is_empty(),
        ChatContent::Multimodal(v) => !v.is_empty(),
        ChatContent::ContextFiles(v) => !v.is_empty(),
    };
    let has_structured_data = message
        .tool_calls
        .as_ref()
        .map_or(false, |tc| !tc.is_empty())
        || message
            .reasoning_content
            .as_ref()
            .map_or(false, |r| !r.trim().is_empty())
        || message
            .thinking_blocks
            .as_ref()
            .map_or(false, |tb| !tb.is_empty())
        || !message.citations.is_empty()
        || !message.server_content_blocks.is_empty()
        || (!message.extra.is_empty() && has_non_metadata_extra(message));

    has_text_content || has_structured_data
}

fn has_non_metadata_extra(message: &ChatMessage) -> bool {
    message
        .extra
        .keys()
        .any(|key| !key.starts_with('_') && key != "openai_response_id")
}

fn is_background_agent_terminal(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "cancelled" | "interrupted")
}

fn is_terminal_runtime_state(state: SessionState) -> bool {
    matches!(
        state,
        SessionState::Idle
            | SessionState::Completed
            | SessionState::Error
            | SessionState::WaitingUserInput
    )
}

fn is_active_compression_phase(phase: Option<CompressionPhase>) -> bool {
    matches!(
        phase,
        Some(CompressionPhase::Checking | CompressionPhase::Running)
    )
}

fn epoch_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn goal_event_subkind(message: &ChatMessage) -> Option<&str> {
    if message.role != crate::chat::internal_roles::EVENT_ROLE {
        return None;
    }
    message
        .extra
        .get("event")
        .and_then(|event| event.get("subkind"))
        .and_then(|subkind| subkind.as_str())
}

fn is_goal_projection_event(message: &ChatMessage) -> bool {
    matches!(
        goal_event_subkind(message),
        Some("goal_delta" | "goal_pursuit")
    )
}

fn message_affects_goal_projection(message: &ChatMessage) -> bool {
    message.role == GOAL_ROLE || is_goal_projection_event(message)
}

fn goal_runtime_projection(
    goal: Option<&GoalSnapshot>,
) -> (bool, Option<GoalStatus>, u32, u64, u32) {
    goal.map(|goal| {
        (
            goal.active,
            Some(goal.status),
            goal.progress.turns_used,
            goal.progress.tokens_used,
            goal.progress.no_progress_turns,
        )
    })
    .unwrap_or((false, None, 0, 0, 0))
}

fn apply_goal_runtime_projection(runtime: &mut RuntimeState, goal: Option<&GoalSnapshot>) {
    let (goal_active, goal_status, goal_turns_used, goal_tokens_used, goal_no_progress_turns) =
        goal_runtime_projection(goal);
    runtime.goal_active = goal_active;
    runtime.goal_status = goal_status;
    runtime.goal_turns_used = goal_turns_used;
    runtime.goal_tokens_used = goal_tokens_used;
    runtime.goal_no_progress_turns = goal_no_progress_turns;
}

fn synthesized_goal_content(messages: &[ChatMessage], base: &ChatMessage) -> String {
    let base = base.content.content_text_only();
    let notes = messages
        .iter()
        .filter(|message| goal_event_subkind(message) == Some("goal_delta"))
        .map(|message| message.content.content_text_only())
        .collect::<Vec<_>>();
    if notes.is_empty() {
        base
    } else {
        format!("{base}\n\n---\n\n## Goal updates\n\n{}", notes.join("\n\n"))
    }
}

fn goal_events_from_messages(messages: &[ChatMessage]) -> Vec<GoalEvent> {
    messages
        .iter()
        .filter_map(|message| {
            let subkind = goal_event_subkind(message)?;
            if !matches!(subkind, "goal_delta" | "goal_pursuit") {
                return None;
            }
            let payload = message
                .extra
                .get("event")
                .and_then(|event| event.get("payload"));
            let at_ms = payload
                .and_then(|payload| payload.get("at_ms"))
                .or_else(|| payload.and_then(|payload| payload.get("created_at_ms")))
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            Some(GoalEvent {
                at_ms,
                kind: subkind.to_string(),
                text: message.content.content_text_only(),
            })
        })
        .collect()
}

pub(crate) fn goal_snapshot_from_messages(
    messages: &[ChatMessage],
    existing: Option<&GoalSnapshot>,
) -> Option<GoalSnapshot> {
    let (base_index, version, base) = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            crate::chat::goal_role::goal_version(message).map(|version| (index, version, message))
        })
        .max_by_key(|(index, version, _)| (*version, *index))?;
    let _ = base_index;
    let meta = base.extra.get("goal");
    let prior = existing.filter(|goal| goal.version == version);
    let has_prior = prior.is_some();
    let active = meta
        .and_then(|meta| meta.get("active"))
        .and_then(|value| value.as_bool())
        .or_else(|| prior.map(|goal| goal.active))
        .unwrap_or(true);
    let meta_status = meta
        .and_then(|meta| meta.get("status"))
        .and_then(|value| serde_json::from_value(value.clone()).ok());
    let status = meta_status
        .or_else(|| prior.map(|goal| goal.status))
        .unwrap_or(if active {
            GoalStatus::Active
        } else {
            GoalStatus::Paused
        });
    let budget = meta
        .and_then(|meta| meta.get("budget"))
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .or_else(|| prior.map(|goal| goal.budget.clone()))
        .unwrap_or_default();
    let budget = budget.migrate_legacy_default_hard_limits();
    let progress = meta
        .and_then(|meta| meta.get("progress"))
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .or_else(|| prior.map(|goal| goal.progress.clone()))
        .unwrap_or_default();
    let derived_events = goal_events_from_messages(messages);
    let events = meta
        .and_then(|meta| meta.get("events"))
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .or_else(|| prior.map(|goal| goal.events.clone()))
        .filter(|events: &Vec<GoalEvent>| !events.is_empty())
        .unwrap_or(derived_events);
    let mut snapshot = GoalSnapshot {
        content: synthesized_goal_content(messages, base),
        version,
        active,
        status,
        budget,
        progress,
        attempts: meta
            .and_then(|meta| meta.get("attempts"))
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .or_else(|| prior.map(|goal| goal.attempts.clone()))
            .unwrap_or_default(),
        events,
        transferred_from: meta
            .and_then(|meta| meta.get("transferred_from"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or_else(|| prior.and_then(|goal| goal.transferred_from.clone())),
        transferred_to: meta
            .and_then(|meta| meta.get("transferred_to"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or_else(|| prior.and_then(|goal| goal.transferred_to.clone())),
    };
    if (meta_status.is_some() || !has_prior)
        && matches!(
            snapshot.status,
            GoalStatus::BudgetExhausted | GoalStatus::NoProgress
        )
        && snapshot
            .goal_budget_exhaustion_status_at(epoch_ms_now())
            .is_none()
    {
        snapshot.status = if snapshot.active {
            GoalStatus::Active
        } else {
            GoalStatus::Paused
        };
    }
    Some(snapshot)
}

fn should_replace_background_agent(
    existing: Option<&BackgroundAgentSummary>,
    incoming: &BackgroundAgentSummary,
) -> bool {
    match existing {
        None => true,
        Some(existing) if incoming.change_seq > existing.change_seq => true,
        Some(existing) if incoming.change_seq < existing.change_seq => false,
        Some(existing) => {
            is_background_agent_terminal(&incoming.status)
                && !is_background_agent_terminal(&existing.status)
        }
    }
}

pub type SessionsMap = Arc<ARwLock<HashMap<String, Arc<AMutex<ChatSession>>>>>;

pub fn create_sessions_map() -> SessionsMap {
    Arc::new(ARwLock::new(HashMap::new()))
}

pub struct ToolDecisionOutcome {
    pub accepted_ids: Vec<String>,
    pub denied_ids: Vec<String>,
}

fn tool_decision_message(decision: &str, tool_call_ids: Vec<String>, scope: &str) -> ChatMessage {
    let count = tool_call_ids.len();
    let verb = if decision == "approve" {
        "approved"
    } else {
        "rejected"
    };
    let noun = if count == 1 {
        "tool call"
    } else {
        "tool calls"
    };
    event(
        EventSubkind::ToolDecision,
        "chat.session",
        json!({
            "tool_call_ids": tool_call_ids,
            "decision": decision,
            "scope": scope,
        }),
        format!("User {verb} {count} {noun} ({scope})"),
    )
}

fn background_process_cleanup_notice(killed_count: usize) -> ChatMessage {
    event(
        EventSubkind::SystemNotice,
        "chat.session",
        json!({ "killed_count": killed_count }),
        format!("Cleared {killed_count} background processes from this chat"),
    )
}

fn background_process_cleanup_modes(include_services: bool) -> Vec<ExecMode> {
    if include_services {
        vec![ExecMode::Background, ExecMode::Service]
    } else {
        vec![ExecMode::Background]
    }
}

pub async fn clean_background_processes_for_chat(
    app: AppState,
    chat_id: &str,
    include_services: bool,
) -> Result<Vec<ExecProcessSnapshot>, String> {
    let mut killed = Vec::new();
    for mode in background_process_cleanup_modes(include_services) {
        for status in [ExecStatusKind::Starting, ExecStatusKind::Running] {
            killed.extend(
                app.runtime
                    .exec_registry
                    .remove_by_owner(ExecProcessFilter {
                        chat_id: Some(chat_id.to_string()),
                        mode: Some(mode.clone()),
                        status: Some(status),
                        ..ExecProcessFilter::default()
                    })
                    .await?,
            );
        }
    }
    killed.sort_by(|a, b| a.meta.process_id.as_str().cmp(b.meta.process_id.as_str()));
    Ok(killed)
}

impl ChatSession {
    pub fn add_background_process_cleanup_notice(&mut self, killed_count: usize) {
        self.add_message(background_process_cleanup_notice(killed_count));
    }
}

impl ChatSession {
    pub fn new(chat_id: String) -> Self {
        let (event_tx, _) = broadcast::channel(limits().event_channel_capacity);
        Self {
            chat_id: chat_id.clone(),
            thread: ThreadParams {
                id: chat_id,
                ..Default::default()
            },
            messages: Vec::new(),
            runtime: RuntimeState::default(),
            goal: None,
            goal_active: false,
            goal_status: None,
            goal_turns_used: 0,
            goal_tokens_used: 0,
            goal_no_progress_turns: 0,
            is_compressing: false,
            compression_phase: None,
            compression_reason: None,
            compression_attempt_generation: 0,
            active_compression_attempt: None,
            compression_attempt_started_at_ms: None,
            draft_message: None,
            draft_usage: None,
            command_queue: VecDeque::new(),
            event_seq: 0,
            event_tx,
            trajectory_events_tx: None,
            recent_request_ids: VecDeque::with_capacity(limits().recent_request_ids_capacity),
            recent_request_ids_set: HashSet::new(),
            abort_flag: Arc::new(AtomicBool::new(false)),
            abort_notify: Arc::new(Notify::new()),
            user_interrupt_flag: Arc::new(AtomicBool::new(false)),
            queue_processor_running: Arc::new(AtomicBool::new(false)),
            queue_notify: Arc::new(Notify::new()),
            last_activity: Instant::now(),
            last_stream_delta_at: None,
            last_tool_started_at: None,
            last_tool_progress_at: None,
            trajectory_dirty: false,
            trajectory_version: 0,
            trajectory_save_in_flight: false,
            trajectory_save_queued: false,
            trajectory_save_mutex: Arc::new(AMutex::new(())),
            created_at: chrono::Utc::now().to_rfc3339(),
            closed: false,
            closed_flag: Arc::new(AtomicBool::new(false)),
            external_reload_pending: None,
            last_prompt_messages: Vec::new(),
            tier1_compact_attempts: 0,
            tier1_compaction_disabled: false,
            compression_insufficient_hashes: HashSet::new(),
            pending_max_new_tokens_boost: None,
            cache_guard_snapshot: None,
            cache_guard_force_next: false,
            provider_usage_stale: false,
            task_agent_error: None,
            pending_browser_message: None,
            post_tool_side_effects: VecDeque::new(),
            active_command: ActiveCommandContext::default(),
            skills_available_count: 0,
            skills_included: Vec::new(),
            pending_skill_deactivation: None,
            stop_hook_handle: None,
            openai_codex_websocket: Default::default(),
            suppress_auto_enrichment_for_next_turn: false,
            wake_up_at: None,
            waiting_for_card_ids: Vec::new(),
            background_completion_burst: BurstGuard::new(),
            background_agents: HashMap::new(),
            goal_stopped_by_abort: false,
        }
    }

    pub fn new_with_trajectory(
        chat_id: String,
        messages: Vec<ChatMessage>,
        mut thread: ThreadParams,
        created_at: String,
        wake_up_at: Option<chrono::DateTime<chrono::Utc>>,
        waiting_for_card_ids: Vec<String>,
        goal: Option<GoalSnapshot>,
    ) -> Self {
        // active_skill is runtime state — if the server restarted mid-skill, the compaction
        // anchor (started_at_index) is lost. Clear it so the session starts cleanly rather
        // than leaving the user locked into a ghost skill that can never be deactivated.
        thread.active_skill = None;
        let goal = goal_snapshot_from_messages(&messages, goal.as_ref()).or(goal);
        let (goal_active, goal_status, goal_turns_used, goal_tokens_used, goal_no_progress_turns) =
            goal_runtime_projection(goal.as_ref());
        let mut runtime = RuntimeState::default();
        runtime.goal_active = goal_active;
        runtime.goal_status = goal_status;
        runtime.goal_turns_used = goal_turns_used;
        runtime.goal_tokens_used = goal_tokens_used;
        runtime.goal_no_progress_turns = goal_no_progress_turns;
        let (event_tx, _) = broadcast::channel(limits().event_channel_capacity);
        Self {
            chat_id,
            thread,
            messages,
            runtime,
            goal,
            goal_active,
            goal_status,
            goal_turns_used,
            goal_tokens_used,
            goal_no_progress_turns,
            is_compressing: false,
            compression_phase: None,
            compression_reason: None,
            compression_attempt_generation: 0,
            active_compression_attempt: None,
            compression_attempt_started_at_ms: None,
            draft_message: None,
            draft_usage: None,
            command_queue: VecDeque::new(),
            event_seq: 0,
            event_tx,
            trajectory_events_tx: None,
            recent_request_ids: VecDeque::with_capacity(limits().recent_request_ids_capacity),
            recent_request_ids_set: HashSet::new(),
            abort_flag: Arc::new(AtomicBool::new(false)),
            abort_notify: Arc::new(Notify::new()),
            user_interrupt_flag: Arc::new(AtomicBool::new(false)),
            queue_processor_running: Arc::new(AtomicBool::new(false)),
            queue_notify: Arc::new(Notify::new()),
            last_activity: Instant::now(),
            last_stream_delta_at: None,
            last_tool_started_at: None,
            last_tool_progress_at: None,
            external_reload_pending: None,
            trajectory_dirty: false,
            trajectory_version: 0,
            trajectory_save_in_flight: false,
            trajectory_save_queued: false,
            trajectory_save_mutex: Arc::new(AMutex::new(())),
            created_at,
            closed: false,
            closed_flag: Arc::new(AtomicBool::new(false)),
            last_prompt_messages: Vec::new(),
            tier1_compact_attempts: 0,
            tier1_compaction_disabled: false,
            compression_insufficient_hashes: HashSet::new(),
            pending_max_new_tokens_boost: None,
            cache_guard_snapshot: None,
            cache_guard_force_next: false,
            provider_usage_stale: false,
            task_agent_error: None,
            pending_browser_message: None,
            post_tool_side_effects: VecDeque::new(),
            active_command: ActiveCommandContext::default(),
            skills_available_count: 0,
            skills_included: Vec::new(),
            pending_skill_deactivation: None,
            stop_hook_handle: None,
            openai_codex_websocket: Default::default(),
            suppress_auto_enrichment_for_next_turn: false,
            wake_up_at,
            waiting_for_card_ids,
            background_completion_burst: BurstGuard::new(),
            background_agents: HashMap::new(),
            goal_stopped_by_abort: false,
        }
    }

    pub fn increment_version(&mut self) {
        self.trajectory_version += 1;
        self.trajectory_dirty = true;
        if self.trajectory_save_in_flight {
            self.trajectory_save_queued = true;
        }
    }

    pub(crate) fn refresh_goal_runtime_mirror(&mut self) {
        let (goal_active, goal_status, goal_turns_used, goal_tokens_used, goal_no_progress_turns) =
            goal_runtime_projection(self.goal.as_ref());
        self.goal_active = goal_active;
        self.goal_status = goal_status;
        self.goal_turns_used = goal_turns_used;
        self.goal_tokens_used = goal_tokens_used;
        self.goal_no_progress_turns = goal_no_progress_turns;
        apply_goal_runtime_projection(&mut self.runtime, self.goal.as_ref());
    }

    pub(crate) fn set_goal_projection(&mut self, goal: Option<GoalSnapshot>) {
        self.goal = goal;
        self.refresh_goal_runtime_mirror();
    }

    pub fn goal_budget_exhausted(&self) -> bool {
        self.goal
            .as_ref()
            .is_some_and(GoalSnapshotBudgetExt::goal_budget_exhausted)
    }

    pub fn goal_can_pursue(&self) -> bool {
        self.goal
            .as_ref()
            .is_some_and(GoalSnapshotBudgetExt::goal_can_pursue)
    }

    pub fn goal_nudge_ready_at(&self, now_ms: u64) -> bool {
        self.goal
            .as_ref()
            .is_some_and(|goal| goal.goal_nudge_ready_at(now_ms))
    }

    pub fn goal_record_progress(&mut self, tokens: u64, made_progress: bool) -> bool {
        let Some(goal) = self.goal.as_mut() else {
            return false;
        };
        goal.goal_record_progress(tokens, made_progress);
        self.mark_persisted_runtime_changed();
        self.emit_goal_status();
        true
    }

    pub fn goal_record_progress_from_usage(&mut self, usage: &ChatUsage) -> bool {
        let Some(goal) = self.goal.as_ref() else {
            return false;
        };
        let made_progress =
            usage.completion_tokens as u64 >= goal.budget.no_progress_token_threshold;
        self.goal_record_progress(usage.total_tokens as u64, made_progress)
    }

    pub fn goal_record_verifier_attempt(&mut self, tokens: u64) -> bool {
        let Some(goal) = self.goal.as_mut() else {
            return false;
        };
        goal.goal_record_verifier_attempt(tokens);
        self.mark_persisted_runtime_changed();
        self.emit_goal_status();
        true
    }

    pub fn goal_note_no_progress_turn(&mut self) -> bool {
        let Some(goal) = self.goal.as_mut() else {
            return false;
        };
        goal.goal_note_no_progress_turn();
        self.mark_persisted_runtime_changed();
        self.emit_goal_status();
        true
    }

    pub fn goal_record_nudge(&mut self, at_ms: u64) -> bool {
        let Some(goal) = self.goal.as_mut() else {
            return false;
        };
        goal.goal_record_nudge(at_ms);
        self.mark_persisted_runtime_changed();
        self.emit_goal_status();
        true
    }

    pub fn goal_reset_no_progress(&mut self) -> bool {
        let Some(goal) = self.goal.as_mut() else {
            return false;
        };
        let before = (goal.progress.no_progress_turns, goal.status);
        goal.goal_reset_no_progress();
        if before == (goal.progress.no_progress_turns, goal.status) {
            return false;
        }
        self.mark_persisted_runtime_changed();
        self.emit_goal_status();
        true
    }

    pub fn goal_set_status(&mut self, status: GoalStatus) -> bool {
        let Some(goal) = self.goal.as_mut() else {
            return false;
        };
        if goal.status == status {
            self.emit_goal_status();
            return true;
        }
        goal.status = status;
        self.mark_persisted_runtime_changed();
        self.emit_goal_status();
        true
    }

    pub fn stop_goal_on_manual_abort(&mut self) -> bool {
        let should_stop = self.goal.as_ref().is_some_and(|goal| {
            goal.active
                && matches!(
                    goal.status,
                    GoalStatus::Active
                        | GoalStatus::Verifying
                        | GoalStatus::BudgetExhausted
                        | GoalStatus::NoProgress
                )
        });
        if !should_stop {
            return false;
        }
        self.goal_set_status(GoalStatus::Stopped);
        self.goal_stopped_by_abort = true;
        self.add_message(event(
            EventSubkind::GoalPursuit,
            "chat.session",
            json!({"kind": "stopped", "trigger": "manual_abort"}),
            "Goal pursuit stopped: chat aborted.".to_string(),
        ));
        true
    }

    pub fn reactivate_goal_stopped_by_manual_abort(&mut self) -> bool {
        if !self.goal_stopped_by_abort {
            return false;
        }
        self.goal_stopped_by_abort = false;
        let can_reactivate = self
            .goal
            .as_ref()
            .is_some_and(|goal| goal.active && goal.status == GoalStatus::Stopped);
        if !can_reactivate {
            return false;
        }
        self.goal_set_status(GoalStatus::Active);
        self.add_message(event(
            EventSubkind::GoalPursuit,
            "chat.session",
            json!({"kind": "resumed", "trigger": "manual_abort_recovery"}),
            "Goal pursuit resumed.".to_string(),
        ));
        true
    }

    pub fn clear_goal_stopped_by_abort_marker(&mut self) {
        self.goal_stopped_by_abort = false;
    }

    pub fn goal_push_attempt(&mut self, attempt: GoalAttempt) -> bool {
        let Some(goal) = self.goal.as_mut() else {
            return false;
        };
        goal.goal_push_attempt(attempt);
        self.mark_persisted_runtime_changed();
        self.emit_goal_status();
        true
    }

    pub fn goal_push_event(&mut self, event: GoalEvent) -> bool {
        let Some(goal) = self.goal.as_mut() else {
            return false;
        };
        goal.goal_push_event(event);
        self.mark_persisted_runtime_changed();
        self.emit_goal_status();
        true
    }

    pub(crate) fn rebuild_goal_projection_from_messages(&mut self) {
        let existing = self.goal.clone();
        self.goal = goal_snapshot_from_messages(&self.messages, existing.as_ref());
        self.refresh_goal_runtime_mirror();
    }

    pub(crate) fn runtime_update_event(
        &self,
        state: SessionState,
        error: Option<String>,
        is_compressing: bool,
        compression_phase: Option<CompressionPhase>,
        compression_reason: Option<CompressionReason>,
    ) -> ChatEvent {
        ChatEvent::RuntimeUpdated {
            goal_active: self.goal_active,
            goal_status: self.goal_status,
            goal_turns_used: self.goal_turns_used,
            goal_tokens_used: self.goal_tokens_used,
            goal_no_progress_turns: self.goal_no_progress_turns,
            state,
            error,
            is_compressing,
            compression_phase,
            compression_reason,
        }
    }

    pub(crate) fn emit_goal_status(&mut self) {
        self.refresh_goal_runtime_mirror();
        let event = self.runtime_update_event(
            self.runtime.state,
            self.runtime.error.clone(),
            self.is_compressing,
            self.compression_phase,
            self.compression_reason,
        );
        self.emit(event);
    }

    pub fn reset_compaction_runtime_state(&mut self) {
        self.last_prompt_messages.clear();
        self.tier1_compact_attempts = 0;
        self.tier1_compaction_disabled = false;
        self.compression_insufficient_hashes.clear();
        self.pending_max_new_tokens_boost = None;
        self.thread.reactive_compact_attempts = None;
        self.thread.previous_response_id = None;
        self.cache_guard_force_next = true;
        self.provider_usage_stale = true;
        self.is_compressing = false;
        self.runtime.is_compressing = false;
        self.compression_phase = None;
        self.runtime.compression_phase = None;
        self.compression_reason = None;
        self.runtime.compression_reason = None;
        self.active_compression_attempt = None;
        self.compression_attempt_started_at_ms = None;
        self.refresh_goal_runtime_mirror();
    }

    pub fn replace_messages(&mut self, messages: Vec<ChatMessage>) {
        self.messages = messages;
        self.rebuild_goal_projection_from_messages();
        self.reset_compaction_runtime_state();
        self.increment_version();
        self.touch();
    }

    pub fn set_active_skill(&mut self, name: String) {
        self.thread.active_skill = Some(name);
        self.increment_version();
    }

    pub fn clear_active_skill(&mut self) {
        self.thread.active_skill = None;
        self.increment_version();
    }

    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    pub fn mark_tool_started(&mut self) {
        let now = Instant::now();
        self.last_tool_started_at = Some(now);
        self.last_tool_progress_at = None;
        self.last_activity = now;
    }

    pub fn mark_tool_progress(&mut self) {
        let now = Instant::now();
        self.last_tool_progress_at = Some(now);
        self.last_activity = now;
    }

    fn mark_stream_delta(&mut self) {
        let now = Instant::now();
        self.last_stream_delta_at = Some(now);
        self.last_activity = now;
    }

    pub(crate) fn mark_persisted_runtime_changed(&mut self) {
        self.increment_version();
        self.touch();
    }

    pub fn is_pending_wake_up(&self) -> bool {
        self.runtime.state == SessionState::WaitingUserInput
            && self
                .wake_up_at
                .as_ref()
                .is_some_and(|deadline| *deadline > chrono::Utc::now())
    }

    pub fn is_idle(&self) -> bool {
        matches!(
            self.runtime.state,
            SessionState::Idle | SessionState::WaitingUserInput
        )
    }

    pub fn is_idle_for_cleanup(&self) -> bool {
        let is_idle_like = matches!(
            self.runtime.state,
            SessionState::Idle | SessionState::Completed | SessionState::WaitingUserInput
        );
        is_idle_like
            && !self.is_pending_wake_up()
            && self.command_queue.is_empty()
            && self.last_activity.elapsed() > session_idle_timeout()
    }

    pub fn close_event_channel(&mut self) {
        self.closed = true;
        self.closed_flag.store(true, Ordering::Relaxed);
        if let Some(h) = self.stop_hook_handle.take() {
            h.abort();
        }
        let (new_tx, _) = broadcast::channel(limits().event_channel_capacity);
        self.event_tx = new_tx;
    }

    pub fn emit(&mut self, event: ChatEvent) {
        self.event_seq += 1;
        let envelope = EventEnvelope {
            chat_id: self.chat_id.clone(),
            seq: self.event_seq,
            event,
        };
        match serde_json::to_string(&envelope) {
            Ok(json) => {
                let _ = self.event_tx.send(Arc::new(json));
            }
            Err(e) => tracing::error!("Failed to serialize SSE event for {}: {}", self.chat_id, e),
        }
    }

    pub fn upsert_background_agent(&mut self, agent: BackgroundAgentSummary) {
        if should_replace_background_agent(self.background_agents.get(&agent.agent_id), &agent) {
            self.background_agents.insert(agent.agent_id.clone(), agent);
        }
    }

    pub fn upsert_background_agents<I>(&mut self, agents: I)
    where
        I: IntoIterator<Item = BackgroundAgentSummary>,
    {
        for agent in agents {
            self.upsert_background_agent(agent);
        }
    }

    pub fn snapshot(&self) -> ChatEvent {
        let mut background_agents: Vec<_> = self.background_agents.values().cloned().collect();
        background_agents.sort_by(|a, b| {
            b.change_seq
                .cmp(&a.change_seq)
                .then(a.agent_id.cmp(&b.agent_id))
        });
        self.snapshot_with_background_agents(background_agents)
    }

    pub fn snapshot_with_background_agents(
        &self,
        background_agents: Vec<BackgroundAgentSummary>,
    ) -> ChatEvent {
        let mut messages = self.messages.clone();
        if self.runtime.state == SessionState::Generating {
            if let Some(ref draft) = self.draft_message {
                if has_displayable_assistant_content(draft) {
                    messages.push(draft.clone());
                }
            }
        }
        let mut runtime = self.runtime.clone();
        runtime.queue_size = self.command_queue.len();
        apply_goal_runtime_projection(&mut runtime, self.goal.as_ref());
        runtime.is_compressing = self.is_compressing;
        runtime.compression_phase = self.compression_phase;
        runtime.compression_reason = self.compression_reason;
        runtime.queued_items = self.build_queued_items();
        ChatEvent::Snapshot {
            goal: self.goal.clone(),
            thread: self.thread.clone(),
            runtime,
            messages,
            background_agents,
            browser: None,
        }
    }

    pub fn snapshot_with_agents(
        app: AppState,
        session: &ChatSession,
    ) -> impl std::future::Future<Output = (ChatEvent, Vec<BackgroundAgentSummary>)> + Send + 'static
    {
        let chat_id = session.chat_id.clone();
        let base_background_agents: HashMap<String, BackgroundAgentSummary> =
            session.background_agents.clone();
        let mut snapshot = session.snapshot();
        async move {
            let browser = crate::integrations::browser_runtime::browser_snapshot_for_chat(
                app.clone(),
                &chat_id,
            )
            .await;
            let mut background_agents = base_background_agents;
            let agents = app
                .agents
                .list_for_parent(&chat_id, AgentListFilter::default())
                .await;
            for agent in agents.iter().map(BackgroundAgentSummary::from) {
                if should_replace_background_agent(background_agents.get(&agent.agent_id), &agent) {
                    background_agents.insert(agent.agent_id.clone(), agent);
                }
            }
            let mut background_agents: Vec<_> = background_agents.into_values().collect();
            background_agents.sort_by(|a, b| {
                b.change_seq
                    .cmp(&a.change_seq)
                    .then(a.agent_id.cmp(&b.agent_id))
            });
            if let ChatEvent::Snapshot {
                background_agents: snapshot_background_agents,
                browser: snapshot_browser,
                ..
            } = &mut snapshot
            {
                *snapshot_background_agents = background_agents.clone();
                *snapshot_browser = browser;
            }
            (snapshot, background_agents)
        }
    }

    pub fn is_duplicate_request(&mut self, request_id: &str) -> bool {
        if self.recent_request_ids_set.contains(request_id) {
            return true;
        }
        self.remember_request_id(request_id);
        false
    }

    pub fn has_seen_request(&self, request_id: &str) -> bool {
        self.recent_request_ids_set.contains(request_id)
    }

    pub fn remember_accepted_request(&mut self, request_id: &str) {
        if !self.has_seen_request(request_id) {
            self.remember_request_id(request_id);
        }
    }

    fn remember_request_id(&mut self, request_id: &str) {
        if self.recent_request_ids.len() >= limits().recent_request_ids_capacity {
            if let Some(evicted) = self.recent_request_ids.pop_front() {
                self.recent_request_ids_set.remove(&evicted);
            }
        }
        self.recent_request_ids.push_back(request_id.to_string());
        self.recent_request_ids_set.insert(request_id.to_string());
    }

    fn command_is_critical_for_queue(&self, command: &ChatCommand) -> bool {
        matches!(command, ChatCommand::Abort {})
            || (self.runtime.state == SessionState::Paused
                && matches!(
                    command,
                    ChatCommand::ToolDecision { .. } | ChatCommand::ToolDecisions { .. }
                ))
            || (self.runtime.state == SessionState::WaitingIde
                && matches!(command, ChatCommand::IdeToolResult { .. }))
    }

    fn command_interrupts_active_loop(command: &ChatCommand) -> bool {
        matches!(
            command,
            ChatCommand::UserMessage { .. }
                | ChatCommand::RetryFromIndex { .. }
                | ChatCommand::Regenerate {}
                | ChatCommand::UpdateMessage {
                    regenerate: true,
                    ..
                }
                | ChatCommand::RemoveMessage {
                    regenerate: true,
                    ..
                }
                | ChatCommand::Abort {}
        )
    }

    pub fn enqueue_command(&mut self, request: CommandRequest) -> EnqueueCommandOutcome {
        if self
            .recent_request_ids_set
            .contains(&request.client_request_id)
        {
            return EnqueueCommandOutcome::Duplicate;
        }
        let request_id = request.client_request_id.clone();
        let outcome = self.enqueue_accepted_command(request);
        if outcome == EnqueueCommandOutcome::Accepted {
            self.remember_request_id(&request_id);
        }
        outcome
    }

    pub fn enqueue_accepted_command(&mut self, request: CommandRequest) -> EnqueueCommandOutcome {
        if self.command_queue.len() >= max_queue_size()
            && !self.command_is_critical_for_queue(&request.command)
        {
            return EnqueueCommandOutcome::Full;
        }

        if request.priority {
            self.enqueue_accepted_priority_command(request)
        } else {
            self.command_queue.push_back(request);
            self.touch();
            self.emit_queue_update();
            self.queue_notify.notify_one();
            EnqueueCommandOutcome::Accepted
        }
    }

    pub fn add_message(&mut self, mut message: ChatMessage) {
        if message.message_id.is_empty() {
            message.message_id = Uuid::new_v4().to_string();
        }
        let affects_goal = message_affects_goal_projection(&message);
        let index = self.messages.len();
        self.messages.push(message.clone());
        if affects_goal {
            self.rebuild_goal_projection_from_messages();
        }
        self.tier1_compact_attempts = 0;
        self.tier1_compaction_disabled = false;
        self.emit(ChatEvent::MessageAdded { message, index });
        self.increment_version();
        self.touch();
    }

    pub fn queue_post_tool_side_effect(&mut self, message: ChatMessage) {
        self.post_tool_side_effects.push_back(message);
        self.touch();
    }

    fn latest_assistant_tool_call_window(
        &self,
        tool_call_id: Option<&str>,
    ) -> Option<(usize, Vec<String>)> {
        self.messages
            .iter()
            .enumerate()
            .rev()
            .find_map(|(idx, message)| {
                if message.role != "assistant" {
                    return None;
                }
                let tool_calls = message.tool_calls.as_ref()?;
                if tool_calls.is_empty() {
                    None
                } else if tool_call_id.map_or(true, |id| tool_calls.iter().any(|tc| tc.id == id)) {
                    Some((idx, tool_calls.iter().map(|tc| tc.id.clone()).collect()))
                } else {
                    None
                }
            })
    }

    fn result_ids_after_assistant(&self, assistant_index: usize) -> HashSet<String> {
        let mut result_ids = HashSet::new();
        for message in self.messages.iter().skip(assistant_index + 1) {
            match message.role.as_str() {
                "tool" | "diff" if !message.tool_call_id.is_empty() => {
                    result_ids.insert(message.tool_call_id.clone());
                }
                role if role == "assistant"
                    || role == "user"
                    || role == crate::chat::internal_roles::EVENT_ROLE
                    || role == crate::chat::internal_roles::PLAN_ROLE
                    || role == crate::chat::internal_roles::GOAL_ROLE =>
                {
                    break;
                }
                _ => {}
            }
        }
        result_ids
    }

    fn all_tool_call_ids_have_results_after(
        &self,
        assistant_index: usize,
        tool_call_ids: &[String],
    ) -> bool {
        let result_ids = self.result_ids_after_assistant(assistant_index);
        tool_call_ids.iter().all(|id| result_ids.contains(id))
    }

    fn has_pending_tool_result_window(&self) -> bool {
        self.latest_assistant_tool_call_window(None)
            .map(|(assistant_index, tool_call_ids)| {
                !self.all_tool_call_ids_have_results_after(assistant_index, &tool_call_ids)
            })
            .unwrap_or(false)
    }

    fn tool_result_window_closed_for_tool_call(&self, tool_call_id: &str) -> bool {
        if let Some((assistant_index, tool_call_ids)) =
            self.latest_assistant_tool_call_window(Some(tool_call_id))
        {
            self.all_tool_call_ids_have_results_after(assistant_index, &tool_call_ids)
        } else {
            !self.has_pending_tool_result_window()
        }
    }

    pub fn drain_post_tool_side_effects(&mut self) {
        if self.has_pending_tool_result_window() {
            return;
        }
        let side_effects = std::mem::take(&mut self.post_tool_side_effects);
        for message in side_effects {
            self.add_message(message);
        }
    }

    pub fn clear_post_tool_side_effects(&mut self) {
        self.post_tool_side_effects.clear();
        self.touch();
    }

    pub fn record_ide_tool_result(
        &mut self,
        tool_call_id: String,
        content: String,
        tool_failed: bool,
    ) -> bool {
        let ok = !tool_failed;
        self.add_message(ChatMessage {
            message_id: Uuid::new_v4().to_string(),
            role: "tool".to_string(),
            content: ChatContent::SimpleText(content.clone()),
            tool_call_id: tool_call_id.clone(),
            tool_failed: Some(tool_failed),
            ..Default::default()
        });
        self.queue_post_tool_side_effect(crate::chat::internal_roles::event(
            crate::chat::internal_roles::EventSubkind::IdeCallback,
            "ide.bridge",
            json!({"tool_call_id": tool_call_id.clone(), "ok": ok, "summary": content.clone()}),
            content,
        ));
        let completed = self.tool_result_window_closed_for_tool_call(&tool_call_id);
        if completed {
            self.drain_post_tool_side_effects();
        }
        completed
    }

    pub fn install_plan(
        &mut self,
        mode: &str,
        body: &str,
    ) -> crate::chat::plan_role::PlanInstallReport {
        crate::chat::plan_role::install_plan(self, mode, body)
    }

    pub fn install_goal(
        &mut self,
        mode: &str,
        body: &str,
        active: bool,
        budget: GoalBudget,
    ) -> crate::chat::goal_role::GoalInstallReport {
        let report = crate::chat::goal_role::install_goal(self, mode, body, active, budget);
        self.rebuild_goal_projection_from_messages();
        self.goal_stopped_by_abort = false;
        self.emit_goal_status();
        report
    }

    pub fn insert_message(&mut self, index: usize, mut message: ChatMessage) {
        if message.message_id.is_empty() {
            message.message_id = Uuid::new_v4().to_string();
        }
        let affects_goal = message_affects_goal_projection(&message);
        let insert_idx = index.min(self.messages.len());
        self.messages.insert(insert_idx, message.clone());
        if affects_goal {
            self.rebuild_goal_projection_from_messages();
        }
        self.tier1_compact_attempts = 0;
        self.tier1_compaction_disabled = false;
        self.emit(ChatEvent::MessageAdded {
            message,
            index: insert_idx,
        });
        self.increment_version();
        self.touch();
    }

    pub fn update_message(&mut self, message_id: &str, message: ChatMessage) -> Option<usize> {
        if let Some(idx) = self
            .messages
            .iter()
            .position(|m| m.message_id == message_id)
        {
            let affects_goal = message_affects_goal_projection(&self.messages[idx])
                || message_affects_goal_projection(&message);
            self.messages[idx] = message.clone();
            if affects_goal {
                self.rebuild_goal_projection_from_messages();
            }
            self.tier1_compact_attempts = 0;
            self.tier1_compaction_disabled = false;
            self.compression_insufficient_hashes.clear();
            self.thread.previous_response_id = None;
            self.emit(ChatEvent::MessageUpdated {
                message_id: message_id.to_string(),
                message,
            });
            self.goal_reset_no_progress();
            self.invalidate_summaries_referencing(message_id);
            self.increment_version();
            self.touch();
            return Some(idx);
        }
        None
    }

    fn invalidate_summaries_referencing(&mut self, source_message_id: &str) {
        let stale: Vec<(String, Option<String>)> = self
            .messages
            .iter()
            .filter(|message| crate::chat::summarization::is_segment_summary(message))
            .filter(|message| {
                message
                    .extra
                    .get("compression")
                    .and_then(|c| c.get("summarized_source_message_ids"))
                    .and_then(|ids| ids.as_array())
                    .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(source_message_id)))
            })
            .map(|message| {
                let source_hash = message
                    .extra
                    .get("compression")
                    .and_then(|c| c.get("source_hash"))
                    .and_then(|h| h.as_str())
                    .map(ToString::to_string);
                (message.message_id.clone(), source_hash)
            })
            .collect();
        self.remove_stale_summaries(stale);
    }

    fn invalidate_orphaned_summaries(&mut self) {
        let existing_ids: HashSet<String> = self
            .messages
            .iter()
            .filter(|message| !message.message_id.is_empty())
            .map(|message| message.message_id.clone())
            .collect();
        let stale: Vec<(String, Option<String>)> = self
            .messages
            .iter()
            .filter(|message| crate::chat::summarization::is_segment_summary(message))
            .filter(|message| {
                message
                    .extra
                    .get("compression")
                    .and_then(|c| c.get("summarized_source_message_ids"))
                    .and_then(|ids| ids.as_array())
                    .is_some_and(|ids| {
                        ids.iter()
                            .filter_map(|id| id.as_str())
                            .any(|id| !id.is_empty() && !existing_ids.contains(id))
                    })
            })
            .map(|message| {
                let source_hash = message
                    .extra
                    .get("compression")
                    .and_then(|c| c.get("source_hash"))
                    .and_then(|h| h.as_str())
                    .map(ToString::to_string);
                (message.message_id.clone(), source_hash)
            })
            .collect();
        self.remove_stale_summaries(stale);
    }

    fn remove_stale_summaries(&mut self, stale: Vec<(String, Option<String>)>) {
        for (summary_id, source_hash) in stale {
            let report_ids: Vec<String> = self
                .messages
                .iter()
                .filter(|message| {
                    source_hash.is_some()
                        && message
                            .extra
                            .get("compression_report")
                            .and_then(|r| r.get("source_hash"))
                            .and_then(|h| h.as_str())
                            .map(ToString::to_string)
                            == source_hash
                })
                .map(|message| message.message_id.clone())
                .collect();
            for stale_id in report_ids.into_iter().chain(std::iter::once(summary_id)) {
                if let Some(stale_idx) = self.messages.iter().position(|m| m.message_id == stale_id)
                {
                    self.messages.remove(stale_idx);
                    self.emit(ChatEvent::MessageRemoved {
                        message_id: stale_id,
                    });
                }
            }
        }
    }

    pub fn remove_message(&mut self, message_id: &str) -> Option<usize> {
        if let Some(idx) = self
            .messages
            .iter()
            .position(|m| m.message_id == message_id)
        {
            let msg = &self.messages[idx];
            let affects_goal = message_affects_goal_projection(msg);
            let role = msg.role.clone();
            let tool_call_ids: Vec<String> = msg
                .tool_calls
                .as_ref()
                .map(|tcs| tcs.iter().map(|tc| tc.id.clone()).collect())
                .unwrap_or_default();

            self.messages.remove(idx);
            if affects_goal {
                self.rebuild_goal_projection_from_messages();
            }
            self.tier1_compact_attempts = 0;
            self.tier1_compaction_disabled = false;
            self.compression_insufficient_hashes.clear();
            self.thread.previous_response_id = None;
            self.emit(ChatEvent::MessageRemoved {
                message_id: message_id.to_string(),
            });

            if role == "assistant" && !tool_call_ids.is_empty() {
                let tool_msg_ids: Vec<String> = self
                    .messages
                    .iter()
                    .filter(|m| m.role == "tool" && tool_call_ids.contains(&m.tool_call_id))
                    .map(|m| m.message_id.clone())
                    .collect();

                for tid in tool_msg_ids {
                    if let Some(tool_idx) = self.messages.iter().position(|m| m.message_id == tid) {
                        self.messages.remove(tool_idx);
                        self.emit(ChatEvent::MessageRemoved { message_id: tid });
                    }
                }
            }

            self.invalidate_orphaned_summaries();
            self.increment_version();
            self.touch();
            return Some(idx);
        }
        None
    }

    pub fn truncate_messages(&mut self, from_index: usize) {
        if from_index < self.messages.len() {
            let affects_goal = self.messages[from_index..]
                .iter()
                .any(message_affects_goal_projection);
            self.messages.truncate(from_index);
            if affects_goal {
                self.rebuild_goal_projection_from_messages();
            }
            self.tier1_compact_attempts = 0;
            self.tier1_compaction_disabled = false;
            self.compression_insufficient_hashes.clear();
            self.thread.previous_response_id = None;
            self.emit(ChatEvent::MessagesTruncated { from_index });
            self.invalidate_orphaned_summaries();
            self.increment_version();
            self.touch();
        }
    }

    pub fn perform_skill_deactivation_cleanup(&mut self) {
        let Some(pending) = self.pending_skill_deactivation.take() else {
            return;
        };

        if pending.start_index > self.messages.len() {
            warn!(
                "Skill deactivation cleanup: start_index {} is beyond messages.len() {} for skill '{}', skipping compaction",
                pending.start_index, self.messages.len(), pending.skill_name
            );
            return;
        }

        let activation_tool_message =
            pending
                .activation_tool_call_id
                .as_ref()
                .and_then(|tool_id| {
                    self.messages
                        .iter()
                        .skip(pending.start_index)
                        .find(|msg| msg.role == "tool" && msg.tool_call_id == *tool_id)
                        .cloned()
                });

        if pending.start_index > self.messages.len() {
            warn!(
                "Skill deactivation cleanup: start_index {} is beyond messages.len() {} for skill '{}', skipping compaction",
                pending.start_index, self.messages.len(), pending.skill_name
            );
            return;
        }

        info!(
            "Skill deactivation cleanup: compacting messages from index {} for skill '{}'",
            pending.start_index, pending.skill_name
        );
        self.truncate_messages(pending.start_index);

        if let Some(tool_message) = activation_tool_message {
            self.add_message(tool_message);
        }

        let report_content = format!(
            "## Skill Report: {}\n\n✅ Skill '{}' executed successfully.\n\nHere is the compactified result. The full skill conversation was compactified and removed from the thread.\n\n{}",
            pending.skill_name,
            pending.skill_name,
            pending.report
        );
        let report_message = ChatMessage {
            role: "plain_text".to_string(),
            content: ChatContent::SimpleText(report_content),
            ..Default::default()
        };
        self.add_message(report_message);
    }

    pub fn set_runtime_state(&mut self, state: SessionState, error: Option<String>) {
        let old_state = self.runtime.state;
        let old_error = self.runtime.error.clone();
        let should_clear_terminal_compression = is_terminal_runtime_state(state)
            && (self.is_compressing
                || self.runtime.is_compressing
                || self.active_compression_attempt.is_some()
                || is_active_compression_phase(self.compression_phase)
                || is_active_compression_phase(self.runtime.compression_phase));
        if old_state == state && old_error == error && !should_clear_terminal_compression {
            return;
        }

        let was_paused = old_state == SessionState::Paused;
        let had_pause_reasons = !self.runtime.pause_reasons.is_empty();

        if state == SessionState::ExecutingTools {
            self.mark_tool_started();
        } else if old_state == SessionState::ExecutingTools {
            self.last_tool_started_at = None;
            self.last_tool_progress_at = None;
        }
        if state == SessionState::Generating && old_state != SessionState::Generating {
            self.last_stream_delta_at = None;
        }

        self.runtime.state = state;
        self.runtime.paused = state == SessionState::Paused;
        self.runtime.error = error.clone();
        self.runtime.queue_size = self.command_queue.len();
        self.runtime.queued_items = self.build_queued_items();
        if should_clear_terminal_compression {
            self.is_compressing = false;
            self.runtime.is_compressing = false;
            self.active_compression_attempt = None;
            if is_active_compression_phase(self.compression_phase) {
                self.compression_phase = None;
                self.compression_reason = None;
            }
        }
        self.runtime.is_compressing = self.is_compressing;
        self.runtime.compression_phase = self.compression_phase;
        self.runtime.compression_reason = self.compression_reason;
        self.refresh_goal_runtime_mirror();
        self.touch();

        if state != SessionState::Paused && (was_paused || had_pause_reasons) {
            self.runtime.pause_reasons.clear();
            self.runtime.auto_approved_tool_ids.clear();
            self.runtime.accepted_tool_ids.clear();
            self.runtime.paused_message_index = None;
            self.emit(ChatEvent::PauseCleared {});
        }

        if old_state == SessionState::WaitingUserInput && state != SessionState::WaitingUserInput {
            let mut changed = false;
            if self.wake_up_at.is_some() {
                self.wake_up_at = None;
                changed = true;
            }
            if !self.waiting_for_card_ids.is_empty() {
                self.waiting_for_card_ids.clear();
                changed = true;
            }
            if changed {
                self.increment_version();
            }
        }

        let event = self.runtime_update_event(
            state,
            error.clone(),
            self.is_compressing,
            self.compression_phase,
            self.compression_reason,
        );
        self.emit(event);
        self.emit_trajectory_state_change();
    }

    fn emit_trajectory_state_change(&self) {
        if self.thread.task_meta.is_some() {
            return;
        }
        if let Some(ref tx) = self.trajectory_events_tx {
            let state_str = match self.runtime.state {
                SessionState::Idle => "idle",
                SessionState::Generating => "generating",
                SessionState::ExecutingTools => "executing_tools",
                SessionState::Paused => "paused",
                SessionState::WaitingIde => "waiting_ide",
                SessionState::WaitingUserInput => "waiting_user_input",
                SessionState::Completed => "completed",
                SessionState::Error => "error",
            };
            let effective_root = self
                .thread
                .root_chat_id
                .clone()
                .unwrap_or_else(|| self.chat_id.clone());
            let (task_id, task_role, agent_id, card_id) =
                task_context_from_task_meta(self.thread.task_meta.as_ref());
            let event = TrajectoryEvent {
                event_type: "updated".to_string(),
                id: self.chat_id.clone(),
                updated_at: None,
                title: None,
                is_title_generated: None,
                session_state: Some(state_str.to_string()),
                error: self.runtime.error.clone(),
                message_count: Some(self.messages.len()),
                parent_id: self.thread.parent_id.clone(),
                link_type: self.thread.link_type.clone(),
                root_chat_id: Some(effective_root),
                task_id,
                task_role,
                agent_id,
                card_id,
                model: Some(self.thread.model.clone()),
                mode: Some(self.thread.mode.clone()),
                worktree: self.thread.worktree.clone(),
                total_lines_added: None,
                total_lines_removed: None,
                tasks_total: None,
                tasks_done: None,
                tasks_failed: None,
                total_prompt_tokens: None,
                total_completion_tokens: None,
                total_tokens: None,
                total_cache_read_tokens: None,
                total_cache_creation_tokens: None,
                total_cost_usd: None,
            };
            let _ = tx.send(event);
        }
    }

    pub fn build_queued_items(&self) -> Vec<QueuedItem> {
        self.command_queue
            .iter()
            .map(|r| r.to_queued_item())
            .collect()
    }

    pub fn emit_queue_update(&mut self) {
        self.runtime.queue_size = self.command_queue.len();
        self.runtime.queued_items = self.build_queued_items();
        self.emit(ChatEvent::QueueUpdated {
            queue_size: self.runtime.queue_size,
            queued_items: self.runtime.queued_items.clone(),
        });
    }

    /// Try to insert a priority command before non-priority queued work.
    ///
    /// Accepted interrupting commands abort any active generation/tool loop before
    /// insertion. Duplicate or full-queue rejections leave the active loop and
    /// side effects untouched, so callers that add messages should do so only
    /// after receiving `Accepted`.
    pub fn enqueue_priority_command(
        &mut self,
        mut request: CommandRequest,
    ) -> EnqueueCommandOutcome {
        request.priority = true;
        self.enqueue_command(request)
    }

    pub fn enqueue_accepted_priority_command(
        &mut self,
        mut request: CommandRequest,
    ) -> EnqueueCommandOutcome {
        request.priority = true;
        if self.command_queue.len() >= max_queue_size()
            && !self.command_is_critical_for_queue(&request.command)
        {
            return EnqueueCommandOutcome::Full;
        }
        let interrupts_active_loop = Self::command_interrupts_active_loop(&request.command);
        let active = matches!(
            self.runtime.state,
            SessionState::Generating | SessionState::ExecutingTools
        );
        if interrupts_active_loop && active {
            self.abort_stream();
            self.clear_pending_tool_calls_for_interruption();
        }
        let insert_pos = self
            .command_queue
            .iter()
            .position(|r| !r.priority)
            .unwrap_or(self.command_queue.len());
        self.command_queue.insert(insert_pos, request);
        self.touch();
        self.emit_queue_update();
        self.queue_notify.notify_one();
        EnqueueCommandOutcome::Accepted
    }

    pub fn set_paused_with_reasons_and_auto_approved(
        &mut self,
        reasons: Vec<PauseReason>,
        auto_approved_ids: Vec<String>,
        message_index: Option<usize>,
    ) {
        self.runtime.pause_reasons = reasons.clone();
        self.runtime.auto_approved_tool_ids = auto_approved_ids;
        self.runtime.accepted_tool_ids.clear();
        self.runtime.paused_message_index = message_index;
        self.emit(ChatEvent::PauseRequired { reasons });
        self.set_runtime_state(SessionState::Paused, None);
    }

    pub fn start_stream(&mut self) -> Option<(String, Arc<AtomicBool>)> {
        if self.runtime.state == SessionState::ExecutingTools || self.draft_message.is_some() {
            warn!("Attempted to start stream while already executing tools or draft exists");
            return None;
        }
        self.abort_flag.store(false, Ordering::SeqCst);
        self.user_interrupt_flag.store(false, Ordering::SeqCst);
        let message_id = Uuid::new_v4().to_string();
        self.draft_message = Some(ChatMessage {
            message_id: message_id.clone(),
            role: "assistant".to_string(),
            ..Default::default()
        });
        self.draft_usage = None;
        self.set_runtime_state(SessionState::Generating, None);
        self.emit(ChatEvent::StreamStarted {
            message_id: message_id.clone(),
        });
        self.touch();
        Some((message_id, self.abort_flag.clone()))
    }

    pub fn emit_stream_delta(&mut self, ops: Vec<DeltaOp>) {
        let (message_id, applied) = match &mut self.draft_message {
            Some(draft) => {
                let mut applied = false;
                for op in &ops {
                    match op {
                        DeltaOp::AppendContent { text } => match &mut draft.content {
                            ChatContent::SimpleText(s) => {
                                if !text.is_empty() {
                                    s.push_str(text);
                                    applied = true;
                                }
                            }
                            _ => {
                                if !text.is_empty() {
                                    draft.content = ChatContent::SimpleText(text.clone());
                                    applied = true;
                                }
                            }
                        },
                        DeltaOp::AppendReasoning { text } => {
                            if !text.is_empty() {
                                let r = draft.reasoning_content.get_or_insert_with(String::new);
                                r.push_str(text);
                                applied = true;
                            }
                        }
                        DeltaOp::SetToolCalls { tool_calls } => {
                            let had_tool_calls = draft
                                .tool_calls
                                .as_ref()
                                .map_or(false, |calls| !calls.is_empty());
                            if !tool_calls.is_empty() || had_tool_calls {
                                if let Ok(parsed) = serde_json::from_value(json!(tool_calls)) {
                                    draft.tool_calls = Some(parsed);
                                    applied = true;
                                }
                            }
                        }
                        DeltaOp::SetThinkingBlocks { blocks } => {
                            let had_blocks = draft
                                .thinking_blocks
                                .as_ref()
                                .map_or(false, |current| !current.is_empty());
                            if !blocks.is_empty() || had_blocks {
                                draft.thinking_blocks = Some(blocks.clone());
                                applied = true;
                            }
                        }
                        DeltaOp::AddCitation { citation } => {
                            if !citation.is_null() {
                                draft.citations.push(citation.clone());
                                applied = true;
                            }
                        }
                        DeltaOp::AddServerContentBlock { block } => {
                            if !block.is_null() {
                                draft.server_content_blocks.push(block.clone());
                                applied = true;
                            }
                        }
                        DeltaOp::SetUsage { usage } => {
                            if let Ok(u) = serde_json::from_value(usage.clone()) {
                                draft.usage = Some(u);
                                applied = true;
                            }
                        }
                        DeltaOp::MergeExtra { extra } => {
                            if !extra.is_empty() {
                                draft.extra.extend(extra.clone());
                                applied = true;
                            }
                        }
                    }
                }
                (draft.message_id.clone(), applied)
            }
            None => return,
        };
        self.emit(ChatEvent::StreamDelta { message_id, ops });
        if applied {
            self.mark_stream_delta();
        }
    }

    pub fn finish_stream(&mut self, finish_reason: Option<String>) {
        self.finish_stream_with_next_state(finish_reason, SessionState::Idle);
    }

    pub fn finish_stream_with_next_state(
        &mut self,
        finish_reason: Option<String>,
        next_state: SessionState,
    ) {
        if let Some(mut draft) = self.draft_message.take() {
            let should_keep_draft = has_displayable_assistant_content(&draft);

            self.emit(ChatEvent::StreamFinished {
                message_id: draft.message_id.clone(),
                finish_reason: finish_reason.clone(),
            });

            if should_keep_draft {
                draft.finish_reason = finish_reason;
                if let Some(usage) = self.draft_usage.take() {
                    draft.usage = Some(usage);
                }
                self.add_message(draft);
            } else {
                tracing::warn!("Discarding empty assistant message");
                self.emit(ChatEvent::MessageRemoved {
                    message_id: draft.message_id,
                });
            }
        }
        self.set_runtime_state(next_state, None);
        self.touch();
    }

    pub fn clear_stream_for_retry(&mut self) {
        if let Some(draft) = self.draft_message.take() {
            self.emit(ChatEvent::MessageRemoved {
                message_id: draft.message_id,
            });
        }
        self.draft_usage = None;
        self.set_runtime_state(SessionState::Idle, None);
        self.touch();
    }

    pub fn finish_stream_with_error(&mut self, error: String) {
        if let Some(mut draft) = self.draft_message.take() {
            if has_displayable_assistant_content(&draft) {
                self.emit(ChatEvent::StreamFinished {
                    message_id: draft.message_id.clone(),
                    finish_reason: Some("error".to_string()),
                });
                draft.finish_reason = Some("error".to_string());
                if let Some(usage) = self.draft_usage.take() {
                    draft.usage = Some(usage);
                }
                self.add_message(draft);
            } else {
                self.emit(ChatEvent::MessageRemoved {
                    message_id: draft.message_id,
                });
            }
        }
        self.add_message(make_ui_only_error_message(&error));
        self.set_runtime_state(SessionState::Error, Some(error.clone()));
        self.touch();

        // Store task_meta for async notification (need to clone before async)
        self.task_agent_error = Some(error);
    }

    pub fn abort_stream(&mut self) {
        self.abort_flag.store(true, Ordering::SeqCst);
        self.user_interrupt_flag.store(true, Ordering::SeqCst);
        self.abort_notify.notify_waiters();
        self.refresh_goal_runtime_mirror();
        if let Some(draft) = self.draft_message.take() {
            self.emit(ChatEvent::StreamFinished {
                message_id: draft.message_id.clone(),
                finish_reason: Some("abort".to_string()),
            });
            self.emit(ChatEvent::MessageRemoved {
                message_id: draft.message_id,
            });
        }
        self.draft_usage = None;
        self.set_runtime_state(SessionState::Idle, None);
        self.touch();
        self.queue_notify.notify_one();
    }

    pub fn clear_pending_tool_calls_for_interruption(&mut self) {
        let latest_window = self.latest_assistant_tool_call_window(None);
        let mut updated_message = None;
        if let Some((assistant_index, _)) = latest_window {
            let answered_ids = self.result_ids_after_assistant(assistant_index);
            let Some(message) = self.messages.get_mut(assistant_index) else {
                return;
            };
            let Some(tool_calls) = message.tool_calls.as_ref() else {
                return;
            };
            let retained_tool_calls: Vec<_> = tool_calls
                .iter()
                .filter(|tool_call| answered_ids.contains(&tool_call.id))
                .cloned()
                .collect();

            if retained_tool_calls.len() != tool_calls.len() {
                message.tool_calls = if retained_tool_calls.is_empty() {
                    None
                } else {
                    Some(retained_tool_calls)
                };
                updated_message = Some(message.clone());
            }
        }

        if let Some(message) = updated_message {
            self.increment_version();
            self.emit(ChatEvent::MessageUpdated {
                message_id: message.message_id.clone(),
                message,
            });
        }
    }

    pub fn discard_draft_for_pause(&mut self) {
        if let Some(draft) = self.draft_message.take() {
            self.emit(ChatEvent::MessageRemoved {
                message_id: draft.message_id,
            });
        }
        self.draft_usage = None;
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<String>> {
        self.event_tx.subscribe()
    }

    pub fn set_title(&mut self, title: String, is_generated: bool) {
        self.thread.title = title.clone();
        self.thread.is_title_generated = is_generated;
        self.increment_version();
        self.touch();
        self.emit_trajectory_title_change(title);
    }

    fn emit_trajectory_title_change(&self, title: String) {
        if self.thread.task_meta.is_some() {
            return;
        }
        if let Some(ref tx) = self.trajectory_events_tx {
            let effective_root = self
                .thread
                .root_chat_id
                .clone()
                .unwrap_or_else(|| self.chat_id.clone());
            let (task_id, task_role, agent_id, card_id) =
                task_context_from_task_meta(self.thread.task_meta.as_ref());
            let event = TrajectoryEvent {
                event_type: "updated".to_string(),
                id: self.chat_id.clone(),
                updated_at: Some(chrono::Utc::now().to_rfc3339()),
                title: Some(trajectory_meta_title(&title)),
                is_title_generated: Some(self.thread.is_title_generated),
                session_state: Some(self.runtime.state.to_string()),
                error: self.runtime.error.clone(),
                message_count: Some(self.messages.len()),
                parent_id: self.thread.parent_id.clone(),
                link_type: self.thread.link_type.clone(),
                root_chat_id: Some(effective_root),
                task_id,
                task_role,
                agent_id,
                card_id,
                model: Some(self.thread.model.clone()),
                mode: Some(self.thread.mode.clone()),
                worktree: self.thread.worktree.clone(),
                total_lines_added: None,
                total_lines_removed: None,
                tasks_total: None,
                tasks_done: None,
                tasks_failed: None,
                total_prompt_tokens: None,
                total_completion_tokens: None,
                total_tokens: None,
                total_cache_read_tokens: None,
                total_cache_creation_tokens: None,
                total_cost_usd: None,
            };
            let _ = tx.send(event);
        }
    }

    pub fn validate_tool_decision(&self, tool_call_id: &str) -> bool {
        self.runtime
            .pause_reasons
            .iter()
            .any(|r| r.tool_call_id == tool_call_id)
    }

    pub(super) fn add_tool_decision_event(
        &mut self,
        decision: &str,
        tool_call_ids: Vec<String>,
        scope: &str,
    ) {
        if tool_call_ids.is_empty() {
            return;
        }
        self.queue_post_tool_side_effect(tool_decision_message(decision, tool_call_ids, scope));
    }

    pub fn process_tool_decisions(
        &mut self,
        decisions: &[ToolDecisionItem],
    ) -> ToolDecisionOutcome {
        let mut accepted_ids = Vec::new();
        let mut denied_ids = Vec::new();

        for decision in decisions {
            if !self.validate_tool_decision(&decision.tool_call_id) {
                warn!(
                    "Tool decision for unknown tool_call_id: {}",
                    decision.tool_call_id
                );
                continue;
            }
            if decision.accepted {
                accepted_ids.push(decision.tool_call_id.clone());
            } else {
                denied_ids.push(decision.tool_call_id.clone());
            }
        }

        let before_len = self.runtime.pause_reasons.len();
        self.runtime.pause_reasons.retain(|r| {
            !accepted_ids.contains(&r.tool_call_id) && !denied_ids.contains(&r.tool_call_id)
        });
        let after_len = self.runtime.pause_reasons.len();

        for denied_id in &denied_ids {
            let has_matching_tool_call = self
                .messages
                .iter()
                .rev()
                .find(|m| m.role == "assistant")
                .and_then(|m| m.tool_calls.as_ref())
                .map_or(false, |tcs| tcs.iter().any(|tc| &tc.id == denied_id));
            if !has_matching_tool_call {
                warn!(
                    "Denied tool_call_id {} not found in last assistant message, skipping synthesis",
                    denied_id
                );
                continue;
            }
            self.add_message(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText("Tool call denied by user.".to_string()),
                tool_call_id: denied_id.clone(),
                ..Default::default()
            });
        }

        self.add_tool_decision_event("approve", accepted_ids.clone(), "once");
        self.add_tool_decision_event("reject", denied_ids.clone(), "once");
        self.drain_post_tool_side_effects();

        if before_len != after_len {
            self.touch();
            if self.runtime.pause_reasons.is_empty() {
                // The caller knows whether a cleared pause will continue into tool execution,
                // follow-up generation, or true idle. Keep the paused runtime state intact here
                // so the caller can publish exactly one non-idle/idle transition.
            } else {
                self.emit(ChatEvent::PauseRequired {
                    reasons: self.runtime.pause_reasons.clone(),
                });
                let event = self.runtime_update_event(
                    self.runtime.state,
                    self.runtime.error.clone(),
                    self.is_compressing,
                    self.compression_phase,
                    self.compression_reason,
                );
                self.emit(event);
            }
        }

        ToolDecisionOutcome {
            accepted_ids,
            denied_ids,
        }
    }
}

async fn migrate_legacy_frozen_prefix_on_open(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
) {
    let installed = {
        let mut session = session_arc.lock().await;
        let prefix = session.thread.frozen_request_prefix.as_ref();
        let needs_prefix = prefix.is_none();
        let needs_legacy_save = prefix
            .map(|prefix| prefix.tools_canonical.is_none() && prefix.system_prompt.is_some())
            .unwrap_or(false);
        if needs_prefix {
            let system_prompt = first_system_prompt(&session.messages);
            super::trajectories::ensure_frozen_prefix(&mut session, system_prompt, None).is_some()
        } else if needs_legacy_save {
            session.increment_version();
            true
        } else {
            false
        }
    };
    if installed {
        super::trajectories::maybe_save_trajectory(app, session_arc).await;
    }
}

pub async fn get_or_create_session_with_trajectory(
    app: AppState,
    sessions: &SessionsMap,
    chat_id: &str,
) -> Arc<AMutex<ChatSession>> {
    let gcx = app.gcx.clone();
    let maybe_existing = {
        let sessions_read = sessions.read().await;
        sessions_read.get(chat_id).cloned()
    };

    if let Some(session_arc) = maybe_existing {
        let is_closed = {
            let session = session_arc.lock().await;
            session.closed
        };
        if !is_closed {
            return session_arc;
        }
        let mut sessions_write = sessions.write().await;
        if let Some(current) = sessions_write.get(chat_id) {
            if Arc::ptr_eq(current, &session_arc) {
                sessions_write.remove(chat_id);
            }
        }
    }

    let trajectory_events_tx = app.chat.trajectory_events_tx.clone();

    let (mut session, is_new, transition_repair_patch) = if let Some(mut loaded) =
        super::trajectories::load_trajectory_for_chat(gcx.clone(), chat_id).await
    {
        info!(
            "Loaded trajectory for chat {} with {} messages",
            chat_id,
            loaded.messages.len()
        );
        super::trajectories::apply_mode_defaults_to_thread(
            gcx.clone(),
            &mut loaded.thread,
            loaded.auto_approve_editing_tools_present,
            loaded.auto_approve_dangerous_commands_present,
        )
        .await;
        let transition_identity_repaired = loaded.transition_identity_repaired;
        let transition_repair_patch = transition_identity_repaired.then(|| loaded.repair_patch());
        let mut session = ChatSession::new_with_trajectory(
            chat_id.to_string(),
            loaded.messages,
            loaded.thread,
            loaded.created_at,
            loaded.wake_up_at,
            loaded.waiting_for_card_ids,
            loaded.goal,
        );
        if transition_identity_repaired {
            session.increment_version();
        }
        (session, false, transition_repair_patch)
    } else {
        let mut s = ChatSession::new(chat_id.to_string());
        s.increment_version();
        (s, true, None)
    };

    let background_agents = app
        .agents
        .list_for_parent(chat_id, AgentListFilter::default())
        .await;
    session.upsert_background_agents(background_agents.iter().map(BackgroundAgentSummary::from));

    if is_new {
        session.thread.auto_enrichment_enabled = Some(true);
        if let Some(mode_config) = crate::yaml_configs::customization_registry::get_mode_config(
            gcx.clone(),
            &session.thread.mode,
            None,
        )
        .await
        {
            let defaults = &mode_config.thread_defaults;
            if let Some(v) = defaults.include_project_info {
                session.thread.include_project_info = v;
            }
            if let Some(v) = defaults.checkpoints_enabled {
                session.thread.checkpoints_enabled = v;
            }
            if let Some(v) = defaults.auto_approve_editing_tools {
                session.thread.auto_approve_editing_tools = v;
            }
            if let Some(v) = defaults.auto_approve_dangerous_commands {
                session.thread.auto_approve_dangerous_commands = v;
            }
        }
    }

    session.trajectory_events_tx = Some(trajectory_events_tx.clone());

    let (session_arc, inserted) = {
        let mut sessions_write = sessions.write().await;
        match sessions_write.entry(chat_id.to_string()) {
            std::collections::hash_map::Entry::Vacant(e) => {
                let arc = Arc::new(AMutex::new(session));
                e.insert(arc.clone());
                (arc, true)
            }
            std::collections::hash_map::Entry::Occupied(e) => (e.get().clone(), false),
        }
    };

    if inserted && is_new {
        let app_hook = AppState::from_gcx(gcx.clone()).await;
        let chat_id_clone = chat_id.to_string();
        tokio::spawn(async move {
            let project_dir = get_project_dir_string(app_hook.clone()).await;
            let payload = HookPayload {
                hook_event_name: "SessionStart".to_string(),
                session_id: chat_id_clone,
                project_dir,
                tool_name: None,
                tool_input: None,
                tool_output: None,
                user_prompt: None,
                extra: std::collections::HashMap::new(),
            };
            run_hooks(app_hook, HookEvent::SessionStart, payload).await;
        });
    }

    if inserted && !is_new {
        if transition_repair_patch.is_none() {
            migrate_legacy_frozen_prefix_on_open(app.clone(), session_arc.clone()).await;
        }
        if let Some(repair_patch) = transition_repair_patch {
            let repaired_version = {
                let session = session_arc.lock().await;
                session.trajectory_version
            };
            if let Err(e) = super::trajectories::persist_loaded_trajectory_repair_raw(
                gcx.clone(),
                &repair_patch,
            )
            .await
            {
                warn!(
                    "Failed to persist repaired trajectory for {}: {}",
                    chat_id, e
                );
            } else {
                let mut session = session_arc.lock().await;
                if session.trajectory_version == repaired_version {
                    session.trajectory_dirty = false;
                }
            }
        }
    }

    session_arc
}

pub async fn try_restore_session_if_trajectory_exists(
    app: AppState,
    sessions: &SessionsMap,
    chat_id: &str,
) -> bool {
    let maybe_existing = {
        let sessions_read = sessions.read().await;
        sessions_read.get(chat_id).cloned()
    };

    if let Some(session_arc) = maybe_existing {
        let is_closed = session_arc.lock().await.closed;
        if !is_closed {
            return true;
        }
        let mut sessions_write = sessions.write().await;
        if let Some(current) = sessions_write.get(chat_id) {
            if Arc::ptr_eq(current, &session_arc) {
                sessions_write.remove(chat_id);
            }
        }
    }

    if super::trajectories::load_trajectory_for_chat(app.gcx.clone(), chat_id)
        .await
        .is_none()
    {
        return false;
    }

    get_or_create_session_with_trajectory(app, sessions, chat_id).await;
    true
}

pub async fn close_all_chat_sessions(app: AppState) {
    let sessions = app.chat.sessions.clone();
    let session_arcs: Vec<Arc<AMutex<ChatSession>>> = {
        let sessions_read = sessions.read().await;
        sessions_read.values().cloned().collect()
    };
    for session_arc in session_arcs {
        let lock_result =
            tokio::time::timeout(std::time::Duration::from_millis(500), session_arc.lock()).await;
        match lock_result {
            Ok(mut session) => {
                session.abort_stream();
                session.close_event_channel(); // sets closed + closed_flag
                session.queue_notify.notify_waiters();
            }
            Err(_) => {
                // Could not acquire lock within timeout — notify_waiters best-effort
                // so the queue processor can eventually notice the shutdown flag.
                warn!(
                    "close_all_chat_sessions: session lock timeout, notifying waiters without lock"
                );
                session_arc
                    .try_lock()
                    .map(|s| s.queue_notify.notify_waiters())
                    .ok();
            }
        }
    }
}

async fn cleanup_idle_session(
    app: AppState,
    sessions: &SessionsMap,
    chat_id: &str,
    session_arc: Arc<AMutex<ChatSession>>,
) -> bool {
    if !save_idle_session_for_cleanup(app, session_arc.clone()).await {
        return false;
    }
    remove_idle_session_if_safe(sessions, chat_id, session_arc).await
}

async fn save_idle_session_for_cleanup(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
) -> bool {
    match super::trajectories::try_save_trajectory(app, session_arc).await {
        Ok(saved) => saved,
        Err(error) => {
            warn!("{}", error);
            false
        }
    }
}

async fn remove_idle_session_if_safe(
    sessions: &SessionsMap,
    chat_id: &str,
    session_arc: Arc<AMutex<ChatSession>>,
) -> bool {
    const MAX_REMOVAL_STRONG_REFS: usize = 3;

    let mut sessions_write = sessions.write().await;
    let current_matches = sessions_write
        .get(chat_id)
        .map_or(false, |current| Arc::ptr_eq(current, &session_arc));
    if !current_matches {
        return false;
    }

    let removal_snapshot = {
        let Ok(mut session) = session_arc.try_lock() else {
            return false;
        };
        let max_refs = MAX_REMOVAL_STRONG_REFS
            + usize::from(session.queue_processor_running.load(Ordering::SeqCst));
        if Arc::strong_count(&session_arc) > max_refs {
            return false;
        }
        if !close_idle_session_for_cleanup(&mut session) {
            return false;
        }
        CleanupRemovalSnapshot::capture(&session)
    };

    let safe_to_remove = {
        let Ok(session) = session_arc.try_lock() else {
            return false;
        };
        removal_snapshot.still_matches(&session)
    };
    if safe_to_remove {
        sessions_write.remove(chat_id);
        return true;
    }
    false
}

struct CleanupRemovalSnapshot {
    runtime_state: SessionState,
    runtime_paused: bool,
    runtime_error: Option<String>,
    pause_reasons_len: usize,
    messages_len: usize,
    command_queue_len: usize,
    event_seq: u64,
    trajectory_version: u64,
    last_activity: Instant,
    draft_message_present: bool,
    pending_browser_message_present: bool,
    post_tool_side_effects_len: usize,
}

impl CleanupRemovalSnapshot {
    fn capture(session: &ChatSession) -> Self {
        Self {
            runtime_state: session.runtime.state,
            runtime_paused: session.runtime.paused,
            runtime_error: session.runtime.error.clone(),
            pause_reasons_len: session.runtime.pause_reasons.len(),
            messages_len: session.messages.len(),
            command_queue_len: session.command_queue.len(),
            event_seq: session.event_seq,
            trajectory_version: session.trajectory_version,
            last_activity: session.last_activity,
            draft_message_present: session.draft_message.is_some(),
            pending_browser_message_present: session.pending_browser_message.is_some(),
            post_tool_side_effects_len: session.post_tool_side_effects.len(),
        }
    }

    fn still_matches(&self, session: &ChatSession) -> bool {
        session.closed
            && session.closed_flag.load(Ordering::Relaxed)
            && session.is_idle_for_cleanup()
            && session.runtime.state == self.runtime_state
            && session.runtime.paused == self.runtime_paused
            && session.runtime.error == self.runtime_error
            && session.runtime.pause_reasons.len() == self.pause_reasons_len
            && session.messages.len() == self.messages_len
            && session.command_queue.len() == self.command_queue_len
            && session.event_seq == self.event_seq
            && session.trajectory_version == self.trajectory_version
            && session.last_activity == self.last_activity
            && session.draft_message.is_some() == self.draft_message_present
            && session.pending_browser_message.is_some() == self.pending_browser_message_present
            && session.post_tool_side_effects.len() == self.post_tool_side_effects_len
    }
}

fn close_idle_session_for_cleanup(session: &mut ChatSession) -> bool {
    if !session.is_idle_for_cleanup() {
        return false;
    }
    if !session.closed {
        session.close_event_channel();
    }
    session.queue_notify.notify_waiters();
    true
}

pub fn start_session_cleanup_task(app: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(session_cleanup_interval());
        let shutdown_flag = app.runtime.shutdown_flag.clone();
        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = async {
                    while !shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                    }
                } => {
                    tracing::info!("Session cleanup: shutdown detected, stopping");
                    return;
                }
            }

            let sessions = app.chat.sessions.clone();

            let candidates: Vec<(String, Arc<AMutex<ChatSession>>)> = {
                let sessions_read = sessions.read().await;
                sessions_read
                    .iter()
                    .map(|(chat_id, session_arc)| (chat_id.clone(), session_arc.clone()))
                    .collect()
            };

            let mut to_cleanup = Vec::new();
            for (chat_id, session_arc) in candidates {
                let session = session_arc.lock().await;
                if session.is_pending_wake_up() {
                    continue;
                }
                if session.is_idle_for_cleanup() {
                    drop(session);
                    to_cleanup.push((chat_id, session_arc));
                }
            }

            if to_cleanup.is_empty() {
                continue;
            }

            info!("Cleaning up {} idle sessions", to_cleanup.len());

            for (chat_id, session_arc) in &to_cleanup {
                if !cleanup_idle_session(app.clone(), &sessions, chat_id, session_arc.clone()).await
                {
                    continue;
                }

                let app_hook = app.clone();
                let chat_id_hook = chat_id.clone();
                tokio::spawn(async move {
                    let project_dir = get_project_dir_string(app_hook.clone()).await;
                    let payload = HookPayload {
                        hook_event_name: "SessionEnd".to_string(),
                        session_id: chat_id_hook,
                        project_dir,
                        tool_name: None,
                        tool_input: None,
                        tool_output: None,
                        user_prompt: None,
                        extra: std::collections::HashMap::new(),
                    };
                    run_hooks(app_hook, HookEvent::SessionEnd, payload).await;
                });

                info!("Saved trajectory for closed session {}", chat_id);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::{ChatCommand, CommandRequest};
    use crate::call_validation::{ChatToolCall, ChatToolFunction};
    use crate::exec::{ExecMode, ExecProcessId, ExecProcessMeta};
    use serde_json::json;
    use std::sync::atomic::Ordering;
    use std::time::Instant;

    fn make_session() -> ChatSession {
        ChatSession::new("test-chat".to_string())
    }

    mod goal_budget {
        use super::*;

        fn budget() -> GoalBudget {
            GoalBudget {
                max_turns: Some(5),
                max_minutes: Some(2),
                max_tokens: Some(100),
                cooldown_ms: 1_500,
                no_progress_token_threshold: 10,
                no_progress_turns: Some(2),
                explicit: false,
            }
        }

        fn snapshot() -> GoalSnapshot {
            GoalSnapshot {
                content: "ship it".to_string(),
                version: 1,
                active: true,
                status: GoalStatus::Active,
                budget: budget(),
                progress: GoalProgress {
                    started_at_ms: 1_000,
                    ..Default::default()
                },
                attempts: Vec::new(),
                events: Vec::new(),
                transferred_from: None,
                transferred_to: None,
            }
        }

        fn usage(total_tokens: usize, completion_tokens: usize) -> ChatUsage {
            ChatUsage {
                prompt_tokens: total_tokens.saturating_sub(completion_tokens),
                completion_tokens,
                total_tokens,
                cache_creation_tokens: None,
                cache_read_tokens: None,
                metering_usd: None,
            }
        }

        #[test]
        fn budget_exhaustion_by_turns_tokens_minutes_and_no_progress() {
            let mut by_turns = snapshot();
            by_turns.progress.turns_used = by_turns.budget.max_turns.unwrap();
            assert_eq!(
                by_turns.goal_budget_exhaustion_status_at(1_000),
                Some(GoalStatus::BudgetExhausted)
            );

            let mut by_tokens = snapshot();
            by_tokens.progress.tokens_used = by_tokens.budget.max_tokens.unwrap();
            assert_eq!(
                by_tokens.goal_budget_exhaustion_status_at(1_000),
                Some(GoalStatus::BudgetExhausted)
            );

            let by_minutes = snapshot();
            assert_eq!(
                by_minutes.goal_budget_exhaustion_status_at(121_000),
                Some(GoalStatus::BudgetExhausted)
            );

            let mut by_no_progress = snapshot();
            by_no_progress.progress.no_progress_turns =
                by_no_progress.budget.no_progress_turns.unwrap();
            assert_eq!(
                by_no_progress.goal_budget_exhaustion_status_at(1_000),
                Some(GoalStatus::NoProgress)
            );
        }

        #[test]
        fn default_budget_never_exhausts() {
            let mut goal = snapshot();
            goal.budget = GoalBudget::default();
            goal.progress.turns_used = u32::MAX;
            goal.progress.tokens_used = u64::MAX;
            goal.progress.no_progress_turns = u32::MAX;
            goal.progress.started_at_ms = 1;

            assert_eq!(goal.goal_budget_exhaustion_status_at(u64::MAX), None);
            assert!(goal.goal_can_pursue_at(u64::MAX));
        }

        #[test]
        fn zero_budget_limits_are_disabled() {
            let mut goal = snapshot();
            goal.budget.max_turns = Some(0);
            goal.budget.max_minutes = Some(0);
            goal.budget.max_tokens = Some(0);
            goal.budget.no_progress_turns = Some(0);
            goal.progress.turns_used = u32::MAX;
            goal.progress.tokens_used = u64::MAX;
            goal.progress.no_progress_turns = u32::MAX;
            goal.progress.started_at_ms = 1;

            assert_eq!(goal.goal_budget_exhaustion_status_at(u64::MAX), None);
            assert!(goal.goal_can_pursue_at(u64::MAX));
        }

        #[test]
        fn cooldown_gate_uses_last_nudge_plus_budget_cooldown() {
            let mut goal = snapshot();
            assert!(goal.goal_nudge_ready_at(1_000));

            goal.goal_record_nudge(2_000);

            assert!(!goal.goal_nudge_ready_at(3_499));
            assert!(goal.goal_nudge_ready_at(3_500));
        }

        #[test]
        fn goal_can_pursue_matrix_across_statuses() {
            let mut goal = snapshot();
            assert!(goal.goal_can_pursue_at(1_000));

            for status in [
                GoalStatus::Verifying,
                GoalStatus::Paused,
                GoalStatus::Completed,
                GoalStatus::Stopped,
                GoalStatus::BudgetExhausted,
                GoalStatus::NoProgress,
                GoalStatus::Transferred,
            ] {
                goal.status = status;
                assert!(!goal.goal_can_pursue_at(1_000), "{status:?}");
            }

            goal.status = GoalStatus::Active;
            goal.active = false;
            assert!(!goal.goal_can_pursue_at(1_000));

            goal.active = true;
            goal.progress.turns_used = goal.budget.max_turns.unwrap();
            assert!(!goal.goal_can_pursue_at(1_000));
        }

        #[test]
        fn no_progress_counts_low_output_and_resets_on_progress_or_edit() {
            let mut session = make_session();
            session.install_goal("agent", "ship it", true, budget());

            assert!(session.goal_record_progress_from_usage(&usage(20, 3)));
            assert_eq!(session.goal.as_ref().unwrap().progress.no_progress_turns, 1);
            assert_eq!(session.goal_status, Some(GoalStatus::Active));

            session.goal_record_progress_from_usage(&usage(20, 4));
            assert_eq!(session.goal.as_ref().unwrap().progress.no_progress_turns, 2);
            assert_eq!(session.goal_status, Some(GoalStatus::NoProgress));
            assert!(!session.goal_can_pursue());

            session.goal_record_progress_from_usage(&usage(20, 12));
            assert_eq!(session.goal.as_ref().unwrap().progress.no_progress_turns, 0);
            assert_eq!(session.goal_status, Some(GoalStatus::Active));

            session.goal_record_progress_from_usage(&usage(20, 1));
            assert_eq!(session.goal.as_ref().unwrap().progress.no_progress_turns, 1);
            let user = ChatMessage {
                message_id: "user-edit".to_string(),
                role: "user".to_string(),
                content: ChatContent::SimpleText("before".to_string()),
                ..Default::default()
            };
            session.add_message(user.clone());
            let mut updated = user;
            updated.content = ChatContent::SimpleText("after".to_string());
            session.update_message("user-edit", updated);

            assert_eq!(session.goal.as_ref().unwrap().progress.no_progress_turns, 0);
        }

        #[test]
        fn manual_no_progress_reset_restores_active_gate() {
            let mut session = make_session();
            session.install_goal("agent", "ship it", true, budget());
            session.goal_record_progress_from_usage(&usage(20, 1));
            session.goal_record_progress_from_usage(&usage(20, 1));
            assert_eq!(session.goal_status, Some(GoalStatus::NoProgress));

            assert!(session.goal_reset_no_progress());

            assert_eq!(session.goal.as_ref().unwrap().progress.no_progress_turns, 0);
            assert_eq!(session.goal_status, Some(GoalStatus::Active));
            assert!(session.goal_can_pursue());
        }

        #[test]
        fn stop_goal_on_manual_abort_is_noop_without_goal() {
            let mut session = make_session();
            assert!(!session.stop_goal_on_manual_abort());
        }

        #[test]
        fn stop_goal_on_manual_abort_stops_pursuable_statuses() {
            for status in [
                GoalStatus::Active,
                GoalStatus::Verifying,
                GoalStatus::BudgetExhausted,
                GoalStatus::NoProgress,
            ] {
                let mut session = make_session();
                session.install_goal("agent", "ship it", true, budget());
                session.goal_set_status(status);

                assert!(session.stop_goal_on_manual_abort(), "{status:?}");

                assert_eq!(session.goal_status, Some(GoalStatus::Stopped));
                assert!(session.messages.iter().any(|message| {
                    message.role == "event"
                        && message
                            .extra
                            .get("event")
                            .and_then(|event| event.get("payload"))
                            .and_then(|payload| payload.get("kind"))
                            .and_then(|kind| kind.as_str())
                            == Some("stopped")
                }));
            }
        }

        #[test]
        fn stop_goal_on_manual_abort_leaves_held_or_terminal_statuses_untouched() {
            for status in [
                GoalStatus::Paused,
                GoalStatus::Stopped,
                GoalStatus::Completed,
                GoalStatus::Transferred,
            ] {
                let mut session = make_session();
                session.install_goal("agent", "ship it", true, budget());
                session.goal_set_status(status);

                assert!(!session.stop_goal_on_manual_abort(), "{status:?}");
                assert_eq!(session.goal_status, Some(status));
            }
        }

        #[test]
        fn reactivate_goal_stopped_by_manual_abort_restores_active_status() {
            let mut session = make_session();
            session.install_goal("agent", "ship it", true, budget());
            assert!(session.stop_goal_on_manual_abort());

            assert!(session.reactivate_goal_stopped_by_manual_abort());

            assert_eq!(session.goal_status, Some(GoalStatus::Active));
            assert!(!session.goal_stopped_by_abort);
            assert!(session.messages.iter().any(|message| {
                message.role == "event"
                    && message
                        .extra
                        .get("event")
                        .and_then(|event| event.get("payload"))
                        .and_then(|payload| payload.get("kind"))
                        .and_then(|kind| kind.as_str())
                        == Some("resumed")
            }));
        }

        #[test]
        fn reactivate_goal_stopped_by_manual_abort_is_noop_without_marker() {
            let mut session = make_session();
            session.install_goal("agent", "ship it", true, budget());
            session.goal_set_status(GoalStatus::Stopped);

            assert!(!session.reactivate_goal_stopped_by_manual_abort());
            assert_eq!(session.goal_status, Some(GoalStatus::Stopped));
        }

        #[test]
        fn explicit_goal_control_clears_manual_abort_marker() {
            let mut session = make_session();
            session.install_goal("agent", "ship it", true, budget());
            assert!(session.stop_goal_on_manual_abort());
            assert!(session.goal_stopped_by_abort);

            session.clear_goal_stopped_by_abort_marker();

            assert!(!session.goal_stopped_by_abort);
            assert!(!session.reactivate_goal_stopped_by_manual_abort());
            assert_eq!(session.goal_status, Some(GoalStatus::Stopped));
        }

        #[test]
        fn verifier_attempt_counts_turn_and_tokens_without_no_progress() {
            let mut session = make_session();
            session.install_goal("agent", "ship it", true, budget());

            assert!(session.goal_record_verifier_attempt(42));

            let goal = session.goal.as_ref().unwrap();
            assert_eq!(goal.progress.turns_used, 1);
            assert_eq!(goal.progress.tokens_used, 42);
            assert_eq!(goal.progress.no_progress_turns, 0);
        }

        #[test]
        fn attempts_and_events_append_through_helpers() {
            let mut session = make_session();
            session.install_goal("agent", "ship it", true, budget());

            session.goal_push_attempt(GoalAttempt {
                at_ms: 10,
                trigger: "done".to_string(),
                verdict: "needs_work".to_string(),
                gaps: vec!["tests".to_string()],
                verifier_reply: "run tests".to_string(),
            });
            session.goal_push_event(GoalEvent {
                at_ms: 11,
                kind: "nudge".to_string(),
                text: "keep going".to_string(),
            });

            let goal = session.goal.as_ref().unwrap();
            assert_eq!(goal.attempts.len(), 1);
            assert_eq!(goal.events.len(), 1);
            assert_eq!(goal.attempts[0].gaps, vec!["tests".to_string()]);
            assert_eq!(goal.events[0].text, "keep going");
        }
    }

    /// Creates a session with a small broadcast channel capacity, useful for
    /// triggering `RecvError::Lagged` quickly in tests without emitting
    /// thousands of events.
    fn make_session_with_capacity(capacity: usize) -> ChatSession {
        let (event_tx, _) = broadcast::channel::<Arc<String>>(capacity);
        let mut session = ChatSession::new("test-chat-small".to_string());
        session.event_tx = event_tx;
        session
    }

    async fn test_app_with_workspace(
        workspace: &std::path::Path,
    ) -> (crate::app_state::AppState, tempfile::TempDir) {
        let config_dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            workspace.join("cache"),
            config_dir.path().to_path_buf(),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.to_path_buf()];
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        (app, config_dir)
    }

    fn background_completion_meta(chat_id: &str) -> ExecProcessMeta {
        ExecProcessMeta::new(ExecMode::Background, "true".to_string())
            .with_process_id(ExecProcessId("exec_cleanup_order".to_string()))
            .with_chat_id(chat_id)
            .with_short_description("cleanup order".to_string())
    }

    #[test]
    fn test_new_session_initial_state() {
        let session = make_session();
        assert_eq!(session.chat_id, "test-chat");
        assert_eq!(session.thread.id, "test-chat");
        assert_eq!(session.runtime.state, SessionState::Idle);
        assert!(session.messages.is_empty());
        assert!(session.draft_message.is_none());
        assert_eq!(session.event_seq, 0);
        assert!(!session.trajectory_dirty);
    }

    #[test]
    fn test_new_with_trajectory() {
        let msg = ChatMessage {
            role: "user".into(),
            content: ChatContent::SimpleText("hello".into()),
            ..Default::default()
        };
        let thread = ThreadParams {
            id: "traj-1".into(),
            title: "Old Chat".into(),
            ..Default::default()
        };
        let session = ChatSession::new_with_trajectory(
            "traj-1".into(),
            vec![msg.clone()],
            thread,
            "2024-01-01T00:00:00Z".into(),
            None,
            Vec::new(),
            None,
        );
        assert_eq!(session.chat_id, "traj-1");
        assert_eq!(session.thread.title, "Old Chat");
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.created_at, "2024-01-01T00:00:00Z");
    }

    #[test]
    fn install_goal_populates_snapshot_and_runtime_fields() {
        let mut session = make_session();
        let budget = GoalBudget {
            max_turns: Some(7),
            max_minutes: Some(8),
            max_tokens: Some(9),
            cooldown_ms: 10,
            no_progress_token_threshold: 11,
            no_progress_turns: Some(12),
            explicit: false,
        };

        session.install_goal("agent", "ship the card", true, budget.clone());

        let goal = session.goal.clone().expect("goal projection");
        assert_eq!(goal.content, "ship the card");
        assert_eq!(goal.status, GoalStatus::Active);
        assert_eq!(goal.budget, budget);
        assert!(session.goal_active);
        assert_eq!(session.goal_status, Some(GoalStatus::Active));
        assert_eq!(session.goal_turns_used, 0);
        match session.snapshot() {
            ChatEvent::Snapshot {
                goal: snapshot_goal,
                runtime,
                ..
            } => {
                assert_eq!(snapshot_goal, Some(goal));
                assert!(runtime.goal_active);
                assert_eq!(runtime.goal_status, Some(GoalStatus::Active));
            }
            other => panic!("expected snapshot, got {other:?}"),
        }
    }

    #[test]
    fn new_with_trajectory_rehydrates_goal_without_transfer() {
        let goal_message = crate::chat::internal_roles::goal(
            "agent",
            1,
            "finish the migration",
            None,
            true,
            GoalBudget::default(),
        );
        let persisted_goal = GoalSnapshot {
            content: "persisted content".to_string(),
            version: 1,
            active: true,
            status: GoalStatus::Active,
            budget: GoalBudget::default(),
            progress: GoalProgress {
                turns_used: 4,
                tokens_used: 1234,
                started_at_ms: 55,
                no_progress_turns: 1,
                last_nudge_at_ms: 77,
            },
            attempts: Vec::new(),
            events: Vec::new(),
            transferred_from: None,
            transferred_to: None,
        };

        let session = ChatSession::new_with_trajectory(
            "goal-reload".to_string(),
            vec![goal_message],
            ThreadParams::default(),
            "2024-01-01T00:00:00Z".to_string(),
            None,
            Vec::new(),
            Some(persisted_goal.clone()),
        );

        let goal = session.goal.expect("goal projection");
        assert_eq!(goal.content, "finish the migration");
        assert_eq!(goal.status, GoalStatus::Active);
        assert_eq!(goal.progress, persisted_goal.progress);
        assert_eq!(goal.transferred_from, None);
        assert_eq!(goal.transferred_to, None);
        assert!(session.goal_active);
        assert_eq!(session.goal_status, Some(GoalStatus::Active));
        assert_eq!(session.goal_turns_used, 4);
        assert_eq!(session.goal_tokens_used, 1234);
        assert_eq!(session.goal_no_progress_turns, 1);
        assert_eq!(session.event_seq, 0);
    }

    #[test]
    fn restored_active_goal_without_progress_keeps_unstarted_clock() {
        let goal_message = crate::chat::internal_roles::goal(
            "agent",
            1,
            "finish the migration",
            None,
            true,
            GoalBudget::default(),
        );

        let session = ChatSession::new_with_trajectory(
            "goal-unstarted-reload".to_string(),
            vec![goal_message],
            ThreadParams::default(),
            "2024-01-01T00:00:00Z".to_string(),
            None,
            Vec::new(),
            None,
        );

        let goal = session.goal.expect("goal projection");
        assert_eq!(goal.status, GoalStatus::Active);
        assert_eq!(goal.progress.started_at_ms, 0);
    }

    #[test]
    fn restored_legacy_default_budget_heals_terminal_status() {
        let mut goal_message = crate::chat::internal_roles::goal(
            "agent",
            1,
            "finish the migration",
            None,
            true,
            GoalBudget::legacy_default_hard_limits(),
        );
        let goal_meta = goal_message.extra.get_mut("goal").unwrap();
        goal_meta["status"] = json!(GoalStatus::BudgetExhausted);

        let session = ChatSession::new_with_trajectory(
            "goal-legacy-budget-reload".to_string(),
            vec![goal_message],
            ThreadParams::default(),
            "2024-01-01T00:00:00Z".to_string(),
            None,
            Vec::new(),
            None,
        );

        let goal = session.goal.expect("goal projection");
        assert_eq!(goal.budget, GoalBudget::default());
        assert_eq!(goal.status, GoalStatus::Active);
        assert_eq!(goal.progress.started_at_ms, 0);
    }

    #[test]
    fn restored_explicit_finite_exhausted_goal_stays_terminal() {
        let mut goal_message = crate::chat::internal_roles::goal(
            "agent",
            1,
            "finish the migration",
            None,
            true,
            GoalBudget {
                max_turns: Some(3),
                max_minutes: None,
                max_tokens: None,
                cooldown_ms: 1_500,
                no_progress_token_threshold: 50,
                no_progress_turns: None,
                explicit: true,
            },
        );
        let goal_meta = goal_message.extra.get_mut("goal").unwrap();
        goal_meta["status"] = json!(GoalStatus::BudgetExhausted);
        goal_meta["progress"] = json!(GoalProgress {
            turns_used: 3,
            ..Default::default()
        });

        let session = ChatSession::new_with_trajectory(
            "goal-explicit-budget-reload".to_string(),
            vec![goal_message],
            ThreadParams::default(),
            "2024-01-01T00:00:00Z".to_string(),
            None,
            Vec::new(),
            None,
        );

        let goal = session.goal.expect("goal projection");
        assert_eq!(goal.status, GoalStatus::BudgetExhausted);
        assert_eq!(goal.budget.max_turns, Some(3));
    }

    #[test]
    fn abort_preserves_goal_projection_and_runtime_mirror() {
        let mut session = make_session();
        session.install_goal("agent", "keep goal", true, GoalBudget::default());
        session.goal.as_mut().unwrap().progress.turns_used = 2;
        session.refresh_goal_runtime_mirror();

        session.abort_stream();

        assert!(session.goal.is_some());
        assert!(session.goal_active);
        assert_eq!(session.goal_status, Some(GoalStatus::Active));
        assert_eq!(session.goal_turns_used, 2);
        assert_eq!(session.runtime.goal_turns_used, 2);
    }

    #[test]
    fn test_emit_increments_seq() {
        let mut session = make_session();
        assert_eq!(session.event_seq, 0);
        session.emit(ChatEvent::PauseCleared {});
        assert_eq!(session.event_seq, 1);
        session.emit(ChatEvent::PauseCleared {});
        assert_eq!(session.event_seq, 2);
    }

    #[test]
    fn test_emit_sends_correct_envelope() {
        let mut session = make_session();
        let mut rx = session.subscribe();
        session.emit(ChatEvent::PauseCleared {});
        let json = rx.try_recv().unwrap();
        let envelope: EventEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(envelope.chat_id, "test-chat");
        assert_eq!(envelope.seq, 1);
        assert!(matches!(envelope.event, ChatEvent::PauseCleared {}));
    }

    #[test]
    fn test_snapshot_without_draft() {
        let mut session = make_session();
        session.messages.push(ChatMessage {
            role: "user".into(),
            content: ChatContent::SimpleText("hi".into()),
            ..Default::default()
        });
        let snap = session.snapshot();
        match snap {
            ChatEvent::Snapshot { messages, .. } => {
                assert_eq!(messages.len(), 1);
            }
            _ => panic!("Expected Snapshot"),
        }
    }

    #[test]
    fn test_snapshot_includes_draft_when_generating() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::AppendContent {
            text: "partial".into(),
        }]);
        let snap = session.snapshot();
        match snap {
            ChatEvent::Snapshot {
                messages, runtime, ..
            } => {
                assert_eq!(runtime.state, SessionState::Generating);
                assert_eq!(messages.len(), 1);
                match &messages[0].content {
                    ChatContent::SimpleText(s) => assert_eq!(s, "partial"),
                    _ => panic!("Expected SimpleText"),
                }
            }
            _ => panic!("Expected Snapshot"),
        }
    }

    #[test]
    fn test_snapshot_omits_empty_draft_when_generating() {
        let mut session = make_session();
        session.messages.push(ChatMessage {
            role: "user".into(),
            content: ChatContent::SimpleText("hi".into()),
            ..Default::default()
        });
        session.start_stream();

        let snap = session.snapshot();

        match snap {
            ChatEvent::Snapshot {
                messages, runtime, ..
            } => {
                assert_eq!(runtime.state, SessionState::Generating);
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].role, "user");
            }
            _ => panic!("Expected Snapshot"),
        }
    }

    #[test]
    fn test_snapshot_omits_metadata_only_draft_when_generating() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![
            DeltaOp::SetUsage {
                usage: json!({
                    "prompt_tokens": 10,
                    "completion_tokens": 0,
                    "total_tokens": 10,
                }),
            },
            DeltaOp::MergeExtra {
                extra: serde_json::Map::from_iter([(
                    "openai_response_id".to_string(),
                    json!("resp_123"),
                )]),
            },
        ]);

        let snap = session.snapshot();

        match snap {
            ChatEvent::Snapshot { messages, .. } => {
                assert!(messages.is_empty());
            }
            _ => panic!("Expected Snapshot"),
        }
    }

    #[test]
    fn test_is_duplicate_request_detects_duplicates() {
        let mut session = make_session();
        assert!(!session.is_duplicate_request("req-1"));
        assert!(session.is_duplicate_request("req-1"));
        assert!(!session.is_duplicate_request("req-2"));
        assert!(session.is_duplicate_request("req-2"));
    }

    #[test]
    fn test_is_duplicate_request_caps_at_100() {
        let mut session = make_session();
        for i in 0..100 {
            session.is_duplicate_request(&format!("req-{}", i));
        }
        assert_eq!(session.recent_request_ids.len(), 100);
        session.is_duplicate_request("req-100");
        assert_eq!(session.recent_request_ids.len(), 100);
        assert!(!session.recent_request_ids.contains(&"req-0".to_string()));
        assert!(session.recent_request_ids.contains(&"req-100".to_string()));
    }

    #[test]
    fn test_add_message_generates_id_if_empty() {
        let mut session = make_session();
        let msg = ChatMessage {
            role: "user".into(),
            content: ChatContent::SimpleText("hi".into()),
            ..Default::default()
        };
        session.add_message(msg);
        assert!(!session.messages[0].message_id.is_empty());
        assert!(session.trajectory_dirty);
    }

    #[test]
    fn test_add_message_preserves_existing_id() {
        let mut session = make_session();
        let msg = ChatMessage {
            message_id: "custom-id".into(),
            role: "user".into(),
            content: ChatContent::SimpleText("hi".into()),
            ..Default::default()
        };
        session.add_message(msg);
        assert_eq!(session.messages[0].message_id, "custom-id");
    }

    #[test]
    fn test_update_message_returns_index() {
        let mut session = make_session();
        let msg = ChatMessage {
            message_id: "m1".into(),
            role: "user".into(),
            content: ChatContent::SimpleText("original".into()),
            ..Default::default()
        };
        session.messages.push(msg);
        let updated = ChatMessage {
            message_id: "m1".into(),
            role: "user".into(),
            content: ChatContent::SimpleText("updated".into()),
            ..Default::default()
        };
        let idx = session.update_message("m1", updated);
        assert_eq!(idx, Some(0));
        match &session.messages[0].content {
            ChatContent::SimpleText(s) => assert_eq!(s, "updated"),
            _ => panic!("Expected SimpleText"),
        }
    }

    #[test]
    fn test_update_message_unknown_id_returns_none() {
        let mut session = make_session();
        let msg = ChatMessage::default();
        assert!(session.update_message("unknown", msg).is_none());
    }

    #[test]
    fn test_remove_message_returns_index() {
        let mut session = make_session();
        session.messages.push(ChatMessage {
            message_id: "m1".into(),
            ..Default::default()
        });
        session.messages.push(ChatMessage {
            message_id: "m2".into(),
            ..Default::default()
        });
        let idx = session.remove_message("m1");
        assert_eq!(idx, Some(0));
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].message_id, "m2");
    }

    #[test]
    fn test_remove_message_unknown_id_returns_none() {
        let mut session = make_session();
        assert!(session.remove_message("unknown").is_none());
    }

    #[test]
    fn test_truncate_messages() {
        let mut session = make_session();
        for i in 0..5 {
            session.messages.push(ChatMessage {
                message_id: format!("m{}", i),
                ..Default::default()
            });
        }
        session.truncate_messages(2);
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[1].message_id, "m1");
    }

    #[test]
    fn test_truncate_beyond_length_is_noop() {
        let mut session = make_session();
        session.messages.push(ChatMessage::default());
        let version_before = session.trajectory_version;
        session.truncate_messages(10);
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.trajectory_version, version_before);
    }

    #[test]
    fn test_start_stream_returns_message_id() {
        let mut session = make_session();
        let result = session.start_stream();
        assert!(result.is_some());
        let (msg_id, abort_flag) = result.unwrap();
        assert!(!msg_id.is_empty());
        assert!(!abort_flag.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(session.runtime.state, SessionState::Generating);
        assert!(session.draft_message.is_some());
    }

    #[test]
    fn test_start_stream_fails_if_already_generating() {
        let mut session = make_session();
        session.start_stream();
        let result = session.start_stream();
        assert!(result.is_none());
    }

    #[test]
    fn test_start_stream_fails_if_executing_tools() {
        let mut session = make_session();
        session.set_runtime_state(SessionState::ExecutingTools, None);
        let result = session.start_stream();
        assert!(result.is_none());
    }

    #[test]
    fn test_emit_stream_delta_appends_content() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::AppendContent {
            text: "Hello".into(),
        }]);
        session.emit_stream_delta(vec![DeltaOp::AppendContent {
            text: " World".into(),
        }]);
        let draft = session.draft_message.as_ref().unwrap();
        match &draft.content {
            ChatContent::SimpleText(s) => assert_eq!(s, "Hello World"),
            _ => panic!("Expected SimpleText"),
        }
    }

    #[test]
    fn test_emit_stream_delta_appends_reasoning() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::AppendReasoning {
            text: "think".into(),
        }]);
        session.emit_stream_delta(vec![DeltaOp::AppendReasoning { text: "ing".into() }]);
        let draft = session.draft_message.as_ref().unwrap();
        assert_eq!(draft.reasoning_content.as_ref().unwrap(), "thinking");
    }

    #[test]
    fn test_emit_stream_delta_sets_tool_calls() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::SetToolCalls {
            tool_calls: vec![
                json!({"id":"tc1","type":"function","function":{"name":"test","arguments":"{}"}}),
            ],
        }]);
        let draft = session.draft_message.as_ref().unwrap();
        assert!(draft.tool_calls.is_some());
        assert_eq!(draft.tool_calls.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_clear_pending_tool_calls_for_interruption_updates_last_assistant() {
        let mut session = make_session();
        session.add_message(ChatMessage {
            message_id: "assistant-with-tool".to_string(),
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("I'll use a tool".to_string()),
            tool_calls: Some(vec![crate::call_validation::ChatToolCall {
                id: "call_1".to_string(),
                index: Some(0),
                tool_type: "function".to_string(),
                function: crate::call_validation::ChatToolFunction {
                    name: "shell".to_string(),
                    arguments: "{}".to_string(),
                },
                extra_content: None,
            }]),
            ..Default::default()
        });

        session.clear_pending_tool_calls_for_interruption();

        assert!(session.messages[0].tool_calls.is_none());
    }

    #[test]
    fn test_emit_stream_delta_without_draft_is_noop() {
        let mut session = make_session();
        session.emit_stream_delta(vec![DeltaOp::AppendContent { text: "x".into() }]);
        assert!(session.draft_message.is_none());
    }

    #[test]
    fn test_finish_stream_adds_message() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::AppendContent {
            text: "done".into(),
        }]);
        session.finish_stream(Some("stop".into()));
        assert!(session.draft_message.is_none());
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].finish_reason, Some("stop".into()));
        assert_eq!(session.runtime.state, SessionState::Idle);
    }

    #[test]
    fn test_finish_stream_can_continue_to_tools_without_idle() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::SetToolCalls {
            tool_calls: vec![json!({
                "id": "call_123",
                "type": "function",
                "function": {"name": "cat", "arguments": "{}"}
            })],
        }]);

        session
            .finish_stream_with_next_state(Some("tool_calls".into()), SessionState::ExecutingTools);

        assert!(session.draft_message.is_none());
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.runtime.state, SessionState::ExecutingTools);
        assert!(session.messages[0]
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty()));
    }

    #[test]
    fn test_finish_stream_with_error_keeps_content() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::AppendContent {
            text: "partial".into(),
        }]);
        session.finish_stream_with_error("timeout".into());
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[0].finish_reason, Some("error".into()));
        assert_eq!(session.messages[1].role, "error");
        assert!(crate::chat::diagnostics::is_ui_only_message(
            &session.messages[1]
        ));
        assert_eq!(session.runtime.state, SessionState::Error);
        assert_eq!(session.runtime.error, Some("timeout".into()));
        assert_eq!(
            session.messages[1]
                .extra
                .get("error_info")
                .and_then(|info| info.get("category"))
                .and_then(|category| category.as_str()),
            Some("ProviderTransient")
        );
    }

    #[test]
    fn test_finish_stream_with_error_keeps_structured_data() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::SetToolCalls {
            tool_calls: vec![
                json!({"id":"tc1","type":"function","function":{"name":"test","arguments":"{}"}}),
            ],
        }]);
        session.finish_stream_with_error("error".into());
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[0].role, "assistant");
        assert_eq!(session.messages[1].role, "error");
    }

    #[test]
    fn test_finish_stream_with_error_removes_empty_draft() {
        let mut session = make_session();
        let mut rx = session.subscribe();
        session.start_stream();
        session.finish_stream_with_error("error".into());
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].role, "error");
        assert_eq!(session.messages[0].content.content_text_only(), "error");
        let mut found_removed = false;
        while let Ok(json) = rx.try_recv() {
            if let Ok(env) = serde_json::from_str::<EventEnvelope>(&json) {
                if matches!(env.event, ChatEvent::MessageRemoved { .. }) {
                    found_removed = true;
                }
            }
        }
        assert!(found_removed);
    }

    #[test]
    fn test_finish_stream_with_error_trims_empty_draft() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::AppendContent {
            text: "   \n".into(),
        }]);
        session.finish_stream_with_error("network failed".into());
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].role, "error");
        assert_eq!(
            session.messages[0].content.content_text_only(),
            "network failed"
        );
    }

    #[test]
    fn test_abort_stream() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::AppendContent {
            text: "partial".into(),
        }]);
        session.abort_stream();
        assert!(session.draft_message.is_none());
        assert!(session.messages.is_empty());
        assert!(session.abort_flag.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(session.runtime.state, SessionState::Idle);
    }

    #[test]
    fn enqueue_priority_command_interrupts_active_generation() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::AppendContent {
            text: "partial".into(),
        }]);
        session.enqueue_priority_command(CommandRequest {
            client_request_id: "priority-regenerate".into(),
            priority: false,
            command: ChatCommand::Regenerate {},
        });

        assert!(session.draft_message.is_none());
        assert!(session.abort_flag.load(std::sync::atomic::Ordering::SeqCst));
        assert!(session
            .user_interrupt_flag
            .load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(session.runtime.state, SessionState::Idle);
        assert_eq!(session.command_queue.len(), 1);
        assert!(session.command_queue[0].priority);
        assert!(matches!(
            session.command_queue[0].command,
            ChatCommand::Regenerate {}
        ));
    }

    #[test]
    fn duplicate_priority_command_does_not_interrupt_active_generation() {
        let mut session = make_session();
        session.is_duplicate_request("priority-regenerate");
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::AppendContent {
            text: "partial".into(),
        }]);

        let outcome = session.enqueue_priority_command(CommandRequest {
            client_request_id: "priority-regenerate".into(),
            priority: false,
            command: ChatCommand::Regenerate {},
        });

        assert_eq!(outcome, EnqueueCommandOutcome::Duplicate);
        assert!(!session.abort_flag.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(session.runtime.state, SessionState::Generating);
        assert!(session.command_queue.is_empty());
    }

    #[test]
    fn full_priority_command_does_not_interrupt_active_generation() {
        let mut session = make_session();
        for i in 0..max_queue_size() {
            session.command_queue.push_back(CommandRequest {
                client_request_id: format!("queued-{i}"),
                priority: true,
                command: ChatCommand::SetParams {
                    patch: json!({"temperature": i}),
                },
            });
        }
        session.emit_queue_update();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::AppendContent {
            text: "partial".into(),
        }]);

        let outcome = session.enqueue_priority_command(CommandRequest {
            client_request_id: "priority-regenerate-full".into(),
            priority: false,
            command: ChatCommand::Regenerate {},
        });

        assert_eq!(outcome, EnqueueCommandOutcome::Full);
        assert!(!session.abort_flag.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(session.runtime.state, SessionState::Generating);
        assert_eq!(session.command_queue.len(), max_queue_size());
        assert!(!session
            .recent_request_ids_set
            .contains("priority-regenerate-full"));
    }

    #[test]
    fn full_queue_abort_still_interrupts_active_generation() {
        let mut session = make_session();
        for i in 0..max_queue_size() {
            session.command_queue.push_back(CommandRequest {
                client_request_id: format!("queued-{i}"),
                priority: true,
                command: ChatCommand::SetParams {
                    patch: json!({"temperature": i}),
                },
            });
        }
        session.emit_queue_update();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::AppendContent {
            text: "partial".into(),
        }]);

        let outcome = session.enqueue_priority_command(CommandRequest {
            client_request_id: "priority-abort-full".into(),
            priority: false,
            command: ChatCommand::Abort {},
        });

        assert_eq!(outcome, EnqueueCommandOutcome::Accepted);
        assert!(session.abort_flag.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(session.runtime.state, SessionState::Idle);
        assert_eq!(session.command_queue.len(), max_queue_size() + 1);
        assert!(session
            .recent_request_ids_set
            .contains("priority-abort-full"));
    }

    #[test]
    fn enqueue_priority_abort_interrupts_active_generation() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::AppendContent {
            text: "partial".into(),
        }]);

        session.enqueue_priority_command(CommandRequest {
            client_request_id: "priority-abort".into(),
            priority: false,
            command: ChatCommand::Abort {},
        });

        assert!(session.draft_message.is_none());
        assert!(session.abort_flag.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(session.runtime.state, SessionState::Idle);
        assert!(matches!(
            session.command_queue[0].command,
            ChatCommand::Abort {}
        ));
    }

    #[test]
    fn enqueue_priority_command_clears_unanswered_tool_calls() {
        let mut session = make_session();
        session.set_runtime_state(SessionState::ExecutingTools, None);
        session.messages.push(ChatMessage {
            message_id: "assistant-with-tools".into(),
            role: "assistant".into(),
            tool_calls: Some(vec![ChatToolCall {
                id: "tool-pending".into(),
                index: Some(0),
                function: ChatToolFunction {
                    name: "cat".into(),
                    arguments: "{}".into(),
                },
                tool_type: "function".into(),
                extra_content: None,
            }]),
            ..Default::default()
        });

        session.enqueue_priority_command(CommandRequest {
            client_request_id: "priority-user".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("interrupt"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
            },
        });

        assert!(session.abort_flag.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(session.runtime.state, SessionState::Idle);
        assert!(session.messages[0].tool_calls.is_none());
        assert!(session.command_queue[0].priority);
    }

    #[test]
    fn enqueue_priority_command_preserves_answered_tool_calls() {
        let mut session = make_session();
        session.set_runtime_state(SessionState::ExecutingTools, None);
        session.messages.push(ChatMessage {
            message_id: "assistant-with-tools".into(),
            role: "assistant".into(),
            tool_calls: Some(vec![
                ChatToolCall {
                    id: "tool-answered".into(),
                    index: Some(0),
                    function: ChatToolFunction {
                        name: "cat".into(),
                        arguments: "{}".into(),
                    },
                    tool_type: "function".into(),
                    extra_content: None,
                },
                ChatToolCall {
                    id: "tool-pending".into(),
                    index: Some(1),
                    function: ChatToolFunction {
                        name: "tree".into(),
                        arguments: "{}".into(),
                    },
                    tool_type: "function".into(),
                    extra_content: None,
                },
            ]),
            ..Default::default()
        });
        session.messages.push(ChatMessage {
            role: "tool".into(),
            tool_call_id: "tool-answered".into(),
            content: ChatContent::SimpleText("done".into()),
            ..Default::default()
        });

        session.enqueue_priority_command(CommandRequest {
            client_request_id: "priority-regenerate".into(),
            priority: false,
            command: ChatCommand::Regenerate {},
        });

        let tool_calls = session.messages[0].tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "tool-answered");
    }

    #[test]
    fn snapshot_includes_cached_background_agents() {
        let mut session = make_session();
        let agent = BackgroundAgentSummary {
            agent_id: "bgagent-cached".into(),
            parent_chat_id: session.chat_id.clone(),
            child_chat_id: Some("child-chat".into()),
            kind: "subagent".into(),
            status: "completed".into(),
            title: "Cached agent".into(),
            progress: None,
            step_count: 2,
            last_activity: None,
            target_files: vec![],
            edited_files: vec![],
            diff_summary: None,
            conflict_summary: None,
            result_summary: Some("done".into()),
            error: None,
            started_at: None,
            finished_at: Some("2026-05-28T00:00:00Z".into()),
            change_seq: 7,
        };
        session
            .background_agents
            .insert(agent.agent_id.clone(), agent.clone());

        let snapshot = session.snapshot();

        match snapshot {
            ChatEvent::Snapshot {
                background_agents, ..
            } => assert_eq!(background_agents, vec![agent]),
            _ => panic!("Expected Snapshot"),
        }
    }

    #[test]
    fn stale_background_agent_update_does_not_replace_terminal_status() {
        let mut session = make_session();
        let completed = BackgroundAgentSummary {
            agent_id: "bgagent-cached".into(),
            parent_chat_id: session.chat_id.clone(),
            child_chat_id: Some("child-chat".into()),
            kind: "subagent".into(),
            status: "completed".into(),
            title: "Cached agent".into(),
            progress: None,
            step_count: 2,
            last_activity: None,
            target_files: vec![],
            edited_files: vec![],
            diff_summary: None,
            conflict_summary: None,
            result_summary: Some("done".into()),
            error: None,
            started_at: None,
            finished_at: Some("2026-05-28T00:00:00Z".into()),
            change_seq: 7,
        };
        let stale_running = BackgroundAgentSummary {
            status: "running".into(),
            finished_at: None,
            result_summary: None,
            change_seq: 6,
            ..completed.clone()
        };
        session.upsert_background_agent(completed.clone());
        session.upsert_background_agent(stale_running);

        let cached = session.background_agents.get("bgagent-cached").unwrap();
        assert_eq!(cached.status, "completed");
        assert_eq!(cached.change_seq, 7);
    }

    #[test]
    fn same_sequence_background_agent_update_keeps_non_terminal_existing() {
        let mut session = make_session();
        let running = BackgroundAgentSummary {
            agent_id: "bgagent-cached".into(),
            parent_chat_id: session.chat_id.clone(),
            child_chat_id: Some("child-chat".into()),
            kind: "subagent".into(),
            status: "running".into(),
            title: "Cached agent".into(),
            progress: Some("Applying patch".into()),
            step_count: 2,
            last_activity: None,
            target_files: vec![],
            edited_files: vec![],
            diff_summary: None,
            conflict_summary: None,
            result_summary: None,
            error: None,
            started_at: None,
            finished_at: None,
            change_seq: 7,
        };
        let stale_running = BackgroundAgentSummary {
            progress: Some("Queued".into()),
            ..running.clone()
        };
        session.upsert_background_agent(running.clone());
        session.upsert_background_agent(stale_running);

        let cached = session.background_agents.get("bgagent-cached").unwrap();
        assert_eq!(cached.progress.as_deref(), Some("Applying patch"));
    }

    #[test]
    fn tool_execution_progress_updates_last_tool_progress_at() {
        let mut session = make_session();
        session.set_runtime_state(SessionState::ExecutingTools, None);
        let started = session.last_tool_started_at.unwrap();
        assert!(session.last_tool_progress_at.is_none());

        std::thread::sleep(std::time::Duration::from_millis(10));
        session.mark_tool_progress();

        let progress = session.last_tool_progress_at.unwrap();
        assert!(progress > started);
        assert_eq!(session.last_activity, progress);

        session.set_runtime_state(SessionState::Idle, None);
        assert!(session.last_tool_started_at.is_none());
        assert!(session.last_tool_progress_at.is_none());
    }

    #[test]
    fn set_runtime_state_updates_last_activity_on_change() {
        let mut session = make_session();
        let before = session.last_activity;
        std::thread::sleep(std::time::Duration::from_millis(10));

        session.set_runtime_state(SessionState::Generating, None);

        assert!(session.last_activity > before);
    }

    #[test]
    fn set_runtime_state_no_op_does_not_touch_activity() {
        let mut session = make_session();
        let before = session.last_activity;
        std::thread::sleep(std::time::Duration::from_millis(10));

        session.set_runtime_state(SessionState::Idle, None);

        assert_eq!(session.last_activity, before);
        assert_eq!(session.event_seq, 0);
    }

    #[test]
    fn set_runtime_state_idle_clears_active_compression_same_state() {
        let mut session = make_session();
        session.is_compressing = false;
        session.runtime.is_compressing = false;
        session.compression_phase = Some(CompressionPhase::Checking);
        session.runtime.compression_phase = Some(CompressionPhase::Checking);
        session.compression_attempt_generation = 1;
        session.active_compression_attempt = Some(1);
        let mut rx = session.subscribe();
        let before = session.last_activity;
        std::thread::sleep(std::time::Duration::from_millis(10));

        session.set_runtime_state(SessionState::Idle, None);

        assert!(!session.is_compressing);
        assert!(!session.runtime.is_compressing);
        assert_eq!(session.compression_phase, None);
        assert_eq!(session.runtime.compression_phase, None);
        assert_eq!(session.active_compression_attempt, None);
        assert!(session.last_activity > before);

        let json = rx.try_recv().unwrap();
        let envelope: EventEnvelope = serde_json::from_str(&json).unwrap();
        match envelope.event {
            ChatEvent::RuntimeUpdated {
                state,
                is_compressing,
                compression_phase,
                ..
            } => {
                assert_eq!(state, SessionState::Idle);
                assert!(!is_compressing);
                assert_eq!(compression_phase, None);
            }
            other => panic!("expected RuntimeUpdated, got {other:?}"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn set_runtime_state_terminal_clears_active_running_compression() {
        let mut session = make_session();
        session.runtime.state = SessionState::Generating;
        session.is_compressing = true;
        session.runtime.is_compressing = true;
        session.compression_phase = Some(CompressionPhase::Running);
        session.runtime.compression_phase = Some(CompressionPhase::Running);
        session.compression_attempt_generation = 7;
        session.active_compression_attempt = Some(7);
        let mut rx = session.subscribe();

        session.set_runtime_state(SessionState::Completed, None);

        assert_eq!(session.runtime.state, SessionState::Completed);
        assert!(!session.is_compressing);
        assert!(!session.runtime.is_compressing);
        assert_eq!(session.compression_phase, None);
        assert_eq!(session.runtime.compression_phase, None);
        assert_eq!(session.active_compression_attempt, None);

        let json = rx.try_recv().unwrap();
        let envelope: EventEnvelope = serde_json::from_str(&json).unwrap();
        match envelope.event {
            ChatEvent::RuntimeUpdated {
                state,
                is_compressing,
                compression_phase,
                ..
            } => {
                assert_eq!(state, SessionState::Completed);
                assert!(!is_compressing);
                assert_ne!(compression_phase, Some(CompressionPhase::Running));
                assert_ne!(compression_phase, Some(CompressionPhase::Checking));
            }
            other => panic!("expected RuntimeUpdated, got {other:?}"),
        }
    }

    #[test]
    fn set_runtime_state_terminal_preserves_terminal_compression_phase() {
        let mut session = make_session();
        session.runtime.state = SessionState::Generating;
        session.compression_phase = Some(CompressionPhase::Applied);
        session.compression_reason = None;
        session.runtime.compression_phase = Some(CompressionPhase::Applied);
        session.runtime.compression_reason = None;
        let mut rx = session.subscribe();

        session.set_runtime_state(SessionState::Idle, None);

        assert!(!session.is_compressing);
        assert!(!session.runtime.is_compressing);
        assert_eq!(session.compression_phase, Some(CompressionPhase::Applied));
        assert_eq!(
            session.runtime.compression_phase,
            Some(CompressionPhase::Applied)
        );

        let json = rx.try_recv().unwrap();
        let envelope: EventEnvelope = serde_json::from_str(&json).unwrap();
        match envelope.event {
            ChatEvent::RuntimeUpdated {
                is_compressing,
                compression_phase,
                ..
            } => {
                assert!(!is_compressing);
                assert_eq!(compression_phase, Some(CompressionPhase::Applied));
            }
            other => panic!("expected RuntimeUpdated, got {other:?}"),
        }
    }

    #[test]
    fn set_runtime_state_terminal_clears_active_compression_attempt_token() {
        let mut session = make_session();
        session.compression_attempt_generation = 3;
        session.active_compression_attempt = Some(3);
        let mut rx = session.subscribe();

        session.set_runtime_state(SessionState::Idle, None);

        assert_eq!(session.compression_attempt_generation, 3);
        assert_eq!(session.active_compression_attempt, None);
        let json = rx.try_recv().unwrap();
        let envelope: EventEnvelope = serde_json::from_str(&json).unwrap();
        match envelope.event {
            ChatEvent::RuntimeUpdated {
                state,
                is_compressing,
                ..
            } => {
                assert_eq!(state, SessionState::Idle);
                assert!(!is_compressing);
            }
            other => panic!("expected RuntimeUpdated, got {other:?}"),
        }
    }

    #[test]
    fn test_set_runtime_state_clears_pause_on_transition() {
        let mut session = make_session();
        session.runtime.pause_reasons.push(PauseReason {
            reason_type: "test".into(),
            tool_name: "test_tool".into(),
            command: "cmd".into(),
            rule: "rule".into(),
            tool_call_id: "tc1".into(),
            integr_config_path: None,
        });
        session.set_runtime_state(SessionState::Paused, None);
        assert!(!session.runtime.pause_reasons.is_empty());
        session.set_runtime_state(SessionState::Idle, None);
        assert!(session.runtime.pause_reasons.is_empty());
    }

    #[test]
    fn test_set_runtime_state_clears_wake_up_at_and_marks_dirty() {
        let mut session = make_session();
        session.runtime.state = SessionState::WaitingUserInput;
        session.wake_up_at = Some(chrono::Utc::now());
        session.trajectory_dirty = false;

        session.set_runtime_state(SessionState::Idle, None);

        assert!(session.wake_up_at.is_none());
        assert!(session.trajectory_dirty);
    }

    #[test]
    fn replace_messages_resets_compaction_runtime_state() {
        let mut session = make_session();
        session.last_prompt_messages =
            vec![ChatMessage::new("user".to_string(), "old".to_string())];
        session.tier1_compact_attempts = 2;
        session.tier1_compaction_disabled = true;
        session.thread.previous_response_id = Some("resp-old".to_string());
        session.cache_guard_force_next = false;
        session.trajectory_dirty = false;

        session.replace_messages(vec![ChatMessage::new(
            "user".to_string(),
            "new".to_string(),
        )]);

        assert!(session.last_prompt_messages.is_empty());
        assert_eq!(session.tier1_compact_attempts, 0);
        assert!(!session.tier1_compaction_disabled);
        assert!(session.thread.previous_response_id.is_none());
        assert!(session.cache_guard_force_next);
        assert!(session.trajectory_dirty);
    }

    #[test]
    fn leaving_waiting_user_input_clears_waiting_for_card_ids() {
        let mut session = make_session();
        session.runtime.state = SessionState::WaitingUserInput;
        session.waiting_for_card_ids = vec!["T-1".to_string(), "T-2".to_string()];
        session.trajectory_dirty = false;

        session.set_runtime_state(SessionState::Idle, None);

        assert!(session.waiting_for_card_ids.is_empty());
        assert!(session.trajectory_dirty);
    }

    #[test]
    fn waiting_planner_with_future_wake_up_survives_cleanup() {
        let mut session = make_session();
        session.runtime.state = SessionState::WaitingUserInput;
        session.wake_up_at = Some(chrono::Utc::now() + chrono::Duration::minutes(10));
        session.last_activity =
            Instant::now() - session_idle_timeout() - std::time::Duration::from_secs(1);

        assert!(session.is_pending_wake_up());
        assert!(!session.is_idle_for_cleanup());
    }

    #[test]
    fn waiting_planner_with_past_wake_up_can_be_cleaned_up() {
        let mut session = make_session();
        session.runtime.state = SessionState::WaitingUserInput;
        session.wake_up_at = Some(chrono::Utc::now() - chrono::Duration::minutes(10));
        session.last_activity =
            Instant::now() - session_idle_timeout() - std::time::Duration::from_secs(1);

        assert!(!session.is_pending_wake_up());
        assert!(session.is_idle_for_cleanup());
    }

    #[tokio::test]
    async fn cleanup_saves_before_session_removal() {
        let workspace = tempfile::tempdir().unwrap();
        let (app, _config_dir) = test_app_with_workspace(workspace.path()).await;
        let chat_id = "cleanup-saves-before-removal";
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.add_message(ChatMessage::new("user".to_string(), "hello".to_string()));
            session.last_activity =
                Instant::now() - session_idle_timeout() - std::time::Duration::from_secs(1);
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        assert!(
            cleanup_idle_session(
                app.clone(),
                &app.chat.sessions,
                chat_id,
                session_arc.clone(),
            )
            .await
        );

        assert!(app.chat.sessions.read().await.get(chat_id).is_none());
        assert!(
            super::super::trajectories::load_trajectory_for_chat(app.gcx.clone(), chat_id)
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn cleanup_save_failure_keeps_session_mapped_and_open() {
        let workspace = tempfile::tempdir().unwrap();
        let (app, _config_dir) = test_app_with_workspace(workspace.path()).await;
        let chat_id = "cleanup-save-failure-keeps-session";
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.task_meta = Some(TaskMeta {
                task_id: "missing-task-for-save-failure".to_string(),
                role: "agents".to_string(),
                agent_id: Some("agent".to_string()),
                card_id: Some("card".to_string()),
                planner_chat_id: None,
            });
            session.add_message(ChatMessage::new("user".to_string(), "hello".to_string()));
            session.last_activity =
                Instant::now() - session_idle_timeout() - std::time::Duration::from_secs(1);
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        assert!(
            !cleanup_idle_session(
                app.clone(),
                &app.chat.sessions,
                chat_id,
                session_arc.clone(),
            )
            .await
        );

        assert!(app.chat.sessions.read().await.get(chat_id).is_some());
        let session = session_arc.lock().await;
        assert!(!session.closed);
        assert!(session.trajectory_dirty);
    }

    #[tokio::test]
    async fn cleanup_keeps_session_when_completion_arrives_during_save() {
        let workspace = tempfile::tempdir().unwrap();
        let (app, _config_dir) = test_app_with_workspace(workspace.path()).await;
        let chat_id = "cleanup-keeps-completion-during-save";
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.add_message(ChatMessage::new("user".to_string(), "hello".to_string()));
            session
                .queue_processor_running
                .store(true, Ordering::SeqCst);
            session.last_activity =
                Instant::now() - session_idle_timeout() - std::time::Duration::from_secs(1);
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        super::super::trajectories::maybe_save_trajectory(app.clone(), session_arc.clone()).await;
        let process_id = app
            .runtime
            .exec_registry
            .register(background_completion_meta(chat_id), 4096)
            .await
            .meta
            .process_id;
        app.runtime
            .exec_registry
            .mark_started(&process_id)
            .await
            .unwrap();
        let mut completion_rx = app.runtime.exec_registry.subscribe_completion();
        app.runtime
            .exec_registry
            .mark_exited(&process_id, Some(0))
            .await
            .unwrap();
        let completion = completion_rx.recv().await.unwrap();
        crate::chat::notifications::handle_process_completion(app.gcx.clone(), completion).await;

        assert!(
            !cleanup_idle_session(
                app.clone(),
                &app.chat.sessions,
                chat_id,
                session_arc.clone(),
            )
            .await
        );

        let session = session_arc.lock().await;
        assert!(!session.closed);
        assert!(session
            .command_queue
            .iter()
            .any(|request| matches!(request.command, ChatCommand::Regenerate {})));
        assert!(session.last_activity.elapsed() <= session_idle_timeout());
        session
            .queue_processor_running
            .store(false, Ordering::SeqCst);
        drop(session);
        assert!(app.chat.sessions.read().await.get(chat_id).is_some());
        app.runtime.shutdown_flag.store(true, Ordering::Relaxed);
    }

    #[test]
    fn cleanup_close_helper_rechecks_idle_state() {
        let mut session = make_session();
        session.last_activity =
            Instant::now() - session_idle_timeout() - std::time::Duration::from_secs(1);
        assert!(close_idle_session_for_cleanup(&mut session));
        assert!(session.closed);

        let mut already_closed = make_session();
        already_closed.close_event_channel();
        already_closed.last_activity =
            Instant::now() - session_idle_timeout() - std::time::Duration::from_secs(1);
        assert!(close_idle_session_for_cleanup(&mut already_closed));
        assert!(already_closed.closed);

        let mut touched = make_session();
        touched.last_activity = Instant::now();
        assert!(!close_idle_session_for_cleanup(&mut touched));
        assert!(!touched.closed);
    }

    #[tokio::test]
    async fn cleanup_final_removal_recheck_keeps_queued_command() {
        let sessions = create_sessions_map();
        let chat_id = "cleanup-final-recheck-queued";
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.add_message(ChatMessage::new("user".to_string(), "hello".to_string()));
            session.last_activity =
                Instant::now() - session_idle_timeout() - std::time::Duration::from_secs(1);
        }
        sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        {
            let mut session = session_arc.lock().await;
            session.enqueue_priority_command(CommandRequest {
                client_request_id: "queued-before-removal".to_string(),
                priority: false,
                command: ChatCommand::Regenerate {},
            });
        }

        assert!(!remove_idle_session_if_safe(&sessions, chat_id, session_arc.clone(),).await);
        assert!(sessions.read().await.get(chat_id).is_some());
        let session = session_arc.lock().await;
        assert!(session
            .command_queue
            .iter()
            .any(|request| request.client_request_id == "queued-before-removal"));
    }

    #[tokio::test]
    async fn cleanup_final_removal_recheck_keeps_outstanding_session_ref() {
        let sessions = create_sessions_map();
        let chat_id = "cleanup-final-recheck-ref";
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.add_message(ChatMessage::new("user".to_string(), "hello".to_string()));
            session.last_activity =
                Instant::now() - session_idle_timeout() - std::time::Duration::from_secs(1);
        }
        sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());
        let delayed_enqueue_ref = session_arc.clone();

        assert!(!remove_idle_session_if_safe(&sessions, chat_id, session_arc.clone(),).await);

        {
            let mut session = delayed_enqueue_ref.lock().await;
            session.enqueue_priority_command(CommandRequest {
                client_request_id: "queued-from-outstanding-ref".to_string(),
                priority: false,
                command: ChatCommand::Regenerate {},
            });
        }

        let session = sessions.read().await.get(chat_id).cloned().unwrap();
        let session = session.lock().await;
        assert!(session
            .command_queue
            .iter()
            .any(|request| request.client_request_id == "queued-from-outstanding-ref"));
    }

    #[test]
    fn test_set_paused_with_reasons_and_auto_approved() {
        let mut session = make_session();
        let mut rx = session.subscribe();
        let reasons = vec![PauseReason {
            reason_type: "confirmation".into(),
            tool_name: "shell".into(),
            command: "shell".into(),
            rule: "ask".into(),
            tool_call_id: "tc1".into(),
            integr_config_path: None,
        }];
        session.set_paused_with_reasons_and_auto_approved(
            reasons.clone(),
            vec!["tc2".into()],
            Some(0),
        );
        assert_eq!(session.runtime.state, SessionState::Paused);
        assert_eq!(session.runtime.pause_reasons.len(), 1);
        assert_eq!(
            session.runtime.auto_approved_tool_ids,
            vec!["tc2".to_string()]
        );
        assert_eq!(session.runtime.paused_message_index, Some(0));
        let mut found_pause_required = false;
        while let Ok(json) = rx.try_recv() {
            if let Ok(env) = serde_json::from_str::<EventEnvelope>(&json) {
                if matches!(env.event, ChatEvent::PauseRequired { .. }) {
                    found_pause_required = true;
                }
            }
        }
        assert!(found_pause_required);
    }

    #[test]
    fn test_set_title() {
        let mut session = make_session();
        session.set_title("New Title".into(), true);
        assert_eq!(session.thread.title, "New Title");
        assert!(session.thread.is_title_generated);
        assert!(session.trajectory_dirty);
    }

    #[test]
    fn test_validate_tool_decision() {
        let mut session = make_session();
        session.runtime.pause_reasons.push(PauseReason {
            reason_type: "test".into(),
            tool_name: "test_tool".into(),
            command: "cmd".into(),
            rule: "rule".into(),
            tool_call_id: "tc1".into(),
            integr_config_path: None,
        });
        assert!(session.validate_tool_decision("tc1"));
        assert!(!session.validate_tool_decision("unknown"));
    }

    #[test]
    fn test_process_tool_decisions_accepts() {
        let mut session = make_session();
        session.runtime.pause_reasons.push(PauseReason {
            reason_type: "test".into(),
            tool_name: "test_tool".into(),
            command: "cmd".into(),
            rule: "rule".into(),
            tool_call_id: "tc1".into(),
            integr_config_path: None,
        });
        session.runtime.pause_reasons.push(PauseReason {
            reason_type: "test".into(),
            tool_name: "test_tool".into(),
            command: "cmd".into(),
            rule: "rule".into(),
            tool_call_id: "tc2".into(),
            integr_config_path: None,
        });
        session.set_runtime_state(SessionState::Paused, None);
        let outcome = session.process_tool_decisions(&[ToolDecisionItem {
            tool_call_id: "tc1".into(),
            accepted: true,
        }]);
        assert_eq!(outcome.accepted_ids, vec!["tc1"]);
        assert_eq!(session.runtime.pause_reasons.len(), 1);
        assert_eq!(session.runtime.state, SessionState::Paused);
    }

    #[test]
    fn ide_tool_result_emits_event_not_user_message() {
        let mut session = make_session();
        session.runtime.state = SessionState::WaitingIde;
        let mut rx = session.subscribe();

        session.record_ide_tool_result(
            "call_ide_1".to_string(),
            "The user accepted the changes.".to_string(),
            false,
        );

        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[0].role, "tool");
        assert_eq!(session.messages[0].tool_call_id, "call_ide_1");
        assert_eq!(session.messages[0].tool_failed, Some(false));
        assert_eq!(
            session.messages[1].role,
            crate::chat::internal_roles::EVENT_ROLE
        );
        assert!(!session.messages.iter().any(|message| {
            message.role == "user"
                && message
                    .content
                    .content_text_only()
                    .contains("accepted the changes")
        }));
        assert_eq!(
            session.messages[1].extra["event"]["subkind"],
            json!("ide_callback")
        );
        assert_eq!(
            session.messages[1].extra["event"]["source"],
            json!("ide.bridge")
        );
        assert_eq!(
            session.messages[1].extra["event"]["payload"],
            json!({
                "tool_call_id": "call_ide_1",
                "ok": true,
                "summary": "The user accepted the changes."
            })
        );
        assert_eq!(
            session.messages[1].content.content_text_only(),
            "The user accepted the changes."
        );
        assert_eq!(session.runtime.state, SessionState::WaitingIde);

        let mut event_roles = Vec::new();
        while let Ok(json) = rx.try_recv() {
            let envelope: EventEnvelope = serde_json::from_str(&json).unwrap();
            if let ChatEvent::MessageAdded { message, .. } = envelope.event {
                event_roles.push(message.role);
            }
        }
        assert_eq!(
            event_roles,
            vec!["tool", crate::chat::internal_roles::EVENT_ROLE]
        );
    }

    #[test]
    fn process_tool_decisions_emits_runtime_updated_when_partial() {
        let mut session = make_session();
        session.runtime.pause_reasons.push(PauseReason {
            reason_type: "test".into(),
            tool_name: "test_tool".into(),
            command: "cmd".into(),
            rule: "rule".into(),
            tool_call_id: "tc1".into(),
            integr_config_path: None,
        });
        session.runtime.pause_reasons.push(PauseReason {
            reason_type: "test".into(),
            tool_name: "test_tool".into(),
            command: "cmd".into(),
            rule: "rule".into(),
            tool_call_id: "tc2".into(),
            integr_config_path: None,
        });
        session.set_runtime_state(SessionState::Paused, None);
        let mut rx = session.subscribe();
        let before = session.last_activity;
        std::thread::sleep(std::time::Duration::from_millis(10));

        session.process_tool_decisions(&[ToolDecisionItem {
            tool_call_id: "tc1".into(),
            accepted: true,
        }]);

        assert_eq!(session.runtime.pause_reasons.len(), 1);
        assert!(session.last_activity > before);
        let mut saw_runtime_updated = false;
        let mut saw_pause_required = false;
        while let Ok(json) = rx.try_recv() {
            let env: EventEnvelope = serde_json::from_str(&json).unwrap();
            match env.event {
                ChatEvent::RuntimeUpdated { state, error, .. } => {
                    assert_eq!(state, SessionState::Paused);
                    assert_eq!(error, None);
                    saw_runtime_updated = true;
                }
                ChatEvent::PauseRequired { reasons } => {
                    assert_eq!(reasons.len(), 1);
                    assert_eq!(reasons[0].tool_call_id, "tc2");
                    saw_pause_required = true;
                }
                _ => {}
            }
        }
        assert!(saw_runtime_updated);
        assert!(saw_pause_required);
    }

    #[test]
    fn process_tool_decisions_no_change_no_event() {
        let mut session = make_session();
        session.runtime.pause_reasons.push(PauseReason {
            reason_type: "test".into(),
            tool_name: "test_tool".into(),
            command: "cmd".into(),
            rule: "rule".into(),
            tool_call_id: "tc1".into(),
            integr_config_path: None,
        });
        session.set_runtime_state(SessionState::Paused, None);
        let mut rx = session.subscribe();
        let before = session.last_activity;
        std::thread::sleep(std::time::Duration::from_millis(10));

        let outcome = session.process_tool_decisions(&[ToolDecisionItem {
            tool_call_id: "unknown".into(),
            accepted: true,
        }]);

        assert!(outcome.accepted_ids.is_empty());
        assert_eq!(session.runtime.pause_reasons.len(), 1);
        assert_eq!(session.last_activity, before);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_process_tool_decisions_denies() {
        let mut session = make_session();
        session.runtime.pause_reasons.push(PauseReason {
            reason_type: "test".into(),
            tool_name: "test_tool".into(),
            command: "cmd".into(),
            rule: "rule".into(),
            tool_call_id: "tc1".into(),
            integr_config_path: None,
        });
        session.set_runtime_state(SessionState::Paused, None);
        let outcome = session.process_tool_decisions(&[ToolDecisionItem {
            tool_call_id: "tc1".into(),
            accepted: false,
        }]);
        assert!(outcome.accepted_ids.is_empty());
        assert_eq!(outcome.denied_ids, vec!["tc1"]);
        assert!(session.runtime.pause_reasons.is_empty());
        assert_eq!(session.runtime.state, SessionState::Paused);
    }

    #[test]
    fn test_process_tool_decisions_ignores_unknown() {
        let mut session = make_session();
        session.runtime.pause_reasons.push(PauseReason {
            reason_type: "test".into(),
            tool_name: "test_tool".into(),
            command: "cmd".into(),
            rule: "rule".into(),
            tool_call_id: "tc1".into(),
            integr_config_path: None,
        });
        session.set_runtime_state(SessionState::Paused, None);
        let outcome = session.process_tool_decisions(&[ToolDecisionItem {
            tool_call_id: "unknown".into(),
            accepted: true,
        }]);
        assert!(outcome.accepted_ids.is_empty());
        assert_eq!(session.runtime.pause_reasons.len(), 1);
    }

    #[test]
    fn test_process_tool_decisions_leaves_final_state_to_caller() {
        let mut session = make_session();
        session.runtime.pause_reasons.push(PauseReason {
            reason_type: "test".into(),
            tool_name: "test_tool".into(),
            command: "cmd".into(),
            rule: "rule".into(),
            tool_call_id: "tc1".into(),
            integr_config_path: None,
        });
        session.set_runtime_state(SessionState::Paused, None);
        session.process_tool_decisions(&[ToolDecisionItem {
            tool_call_id: "tc1".into(),
            accepted: true,
        }]);
        assert!(session.runtime.pause_reasons.is_empty());
        assert_eq!(session.runtime.state, SessionState::Paused);
    }

    #[test]
    fn test_increment_version() {
        let mut session = make_session();
        assert_eq!(session.trajectory_version, 0);
        assert!(!session.trajectory_dirty);
        session.increment_version();
        assert_eq!(session.trajectory_version, 1);
        assert!(session.trajectory_dirty);
    }

    #[test]
    fn test_create_sessions_map() {
        let map = create_sessions_map();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let read = map.read().await;
            assert!(read.is_empty());
        });
    }

    #[test]
    fn test_build_queued_items() {
        let mut session = make_session();
        session.command_queue.push_back(CommandRequest {
            client_request_id: "req-1".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("hello"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
            },
        });
        session.command_queue.push_back(CommandRequest {
            client_request_id: "req-2".into(),
            priority: true,
            command: ChatCommand::Abort {},
        });
        let items = session.build_queued_items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].client_request_id, "req-1");
        assert!(!items[0].priority);
        assert_eq!(items[0].command_type, "user_message");
        assert_eq!(items[1].client_request_id, "req-2");
        assert!(items[1].priority);
        assert_eq!(items[1].command_type, "abort");
    }

    #[test]
    fn test_emit_queue_update_syncs_runtime() {
        let mut session = make_session();
        session.command_queue.push_back(CommandRequest {
            client_request_id: "req-1".into(),
            priority: false,
            command: ChatCommand::Abort {},
        });
        session.emit_queue_update();
        assert_eq!(session.runtime.queue_size, 1);
        assert_eq!(session.runtime.queued_items.len(), 1);
    }

    #[test]
    fn test_set_runtime_state_syncs_queued_items() {
        let mut session = make_session();
        session.command_queue.push_back(CommandRequest {
            client_request_id: "req-1".into(),
            priority: true,
            command: ChatCommand::Abort {},
        });
        session.set_runtime_state(SessionState::Generating, None);
        assert_eq!(session.runtime.queued_items.len(), 1);
        assert_eq!(session.runtime.queued_items[0].client_request_id, "req-1");
    }

    #[test]
    fn test_snapshot_includes_queued_items() {
        let mut session = make_session();
        session.command_queue.push_back(CommandRequest {
            client_request_id: "req-1".into(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("test"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
            },
        });
        let snap = session.snapshot();
        match snap {
            ChatEvent::Snapshot { runtime, .. } => {
                assert_eq!(runtime.queue_size, 1);
                assert_eq!(runtime.queued_items.len(), 1);
                assert_eq!(runtime.queued_items[0].client_request_id, "req-1");
            }
            _ => panic!("Expected Snapshot"),
        }
    }

    #[test]
    fn test_snapshot_includes_is_compressing() {
        let mut session = make_session();
        session.is_compressing = true;

        let snap = session.snapshot();

        match snap {
            ChatEvent::Snapshot { runtime, .. } => {
                assert!(runtime.is_compressing);
            }
            _ => panic!("Expected Snapshot"),
        }
    }

    #[test]
    fn test_touch_updates_last_activity() {
        let mut session = make_session();
        let before = session.last_activity;
        std::thread::sleep(std::time::Duration::from_millis(10));
        session.touch();
        assert!(session.last_activity > before);
    }

    #[tokio::test]
    async fn stream_delta_updates_last_activity() {
        let mut session = make_session();
        session.start_stream();
        let before = session.last_activity;

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        session.emit_stream_delta(vec![DeltaOp::AppendContent {
            text: "hello".into(),
        }]);

        assert!(session.last_activity > before);
    }

    #[tokio::test]
    async fn stream_delta_only_resets_stream_timestamp_not_tool_timestamp() {
        let mut session = make_session();
        session.set_runtime_state(SessionState::ExecutingTools, None);
        let tool_started = session.last_tool_started_at;
        session.mark_tool_progress();
        let tool_progress = session.last_tool_progress_at;
        session.set_runtime_state(SessionState::Idle, None);
        session.last_tool_started_at = tool_started;
        session.last_tool_progress_at = tool_progress;

        session.start_stream();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        session.emit_stream_delta(vec![DeltaOp::AppendContent {
            text: "hello".into(),
        }]);

        assert!(session.last_stream_delta_at.is_some());
        assert_eq!(session.last_tool_started_at, tool_started);
        assert_eq!(session.last_tool_progress_at, tool_progress);
    }

    #[tokio::test]
    async fn empty_stream_delta_does_not_update_last_activity() {
        let mut session = make_session();
        session.start_stream();
        let before = session.last_activity;

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        session.emit_stream_delta(vec![]);

        assert_eq!(session.last_activity, before);
    }

    #[test]
    fn test_finish_stream_keeps_server_content_blocks_only_message() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![
            DeltaOp::AddServerContentBlock {
                block: json!({
                    "type": "server_tool_use",
                    "id": "srvtoolu_test",
                    "name": "web_search",
                    "input": {"query": "test"}
                }),
            },
            DeltaOp::AddServerContentBlock {
                block: json!({
                    "type": "web_search_tool_result",
                    "tool_use_id": "srvtoolu_test",
                    "content": [{"type": "web_search_result", "title": "Result", "url": "https://example.com"}]
                }),
            },
        ]);
        session.finish_stream(Some("stop".to_string()));

        assert_eq!(
            session.messages.len(),
            1,
            "Server-blocks-only assistant message should be preserved"
        );
        assert_eq!(session.messages[0].server_content_blocks.len(), 2);
        assert_eq!(session.messages[0].role, "assistant");
    }

    #[test]
    fn test_finish_stream_discards_truly_empty_message() {
        let mut session = make_session();
        session.start_stream();
        // No deltas at all
        session.finish_stream(Some("stop".to_string()));

        assert_eq!(
            session.messages.len(),
            0,
            "Truly empty assistant message should be discarded"
        );
    }

    #[test]
    fn test_finish_stream_discards_usage_only_message() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::SetUsage {
            usage: json!({
                "prompt_tokens": 10,
                "completion_tokens": 0,
                "total_tokens": 10,
            }),
        }]);

        session.finish_stream(Some("stop".to_string()));

        assert!(session.messages.is_empty());
    }

    #[test]
    fn test_finish_stream_discards_extra_only_message() {
        let mut session = make_session();
        session.start_stream();
        session.emit_stream_delta(vec![DeltaOp::MergeExtra {
            extra: serde_json::Map::from_iter([(
                "openai_response_id".to_string(),
                json!("resp_123"),
            )]),
        }]);

        session.finish_stream(Some("stop".to_string()));

        assert!(session.messages.is_empty());
    }

    #[test]
    fn test_finish_stream_emits_removal_for_metadata_only_message() {
        let mut session = make_session();
        let mut rx = session.subscribe();
        let (message_id, _) = session.start_stream().unwrap();
        session.emit_stream_delta(vec![DeltaOp::MergeExtra {
            extra: serde_json::Map::from_iter([(
                "openai_response_id".to_string(),
                json!("resp_123"),
            )]),
        }]);

        session.finish_stream(Some("stop".to_string()));

        assert!(session.messages.is_empty());
        let mut found_removed = false;
        while let Ok(json) = rx.try_recv() {
            if let Ok(env) = serde_json::from_str::<EventEnvelope>(&json) {
                if matches!(env.event, ChatEvent::MessageRemoved { message_id: id } if id == message_id)
                {
                    found_removed = true;
                }
            }
        }
        assert!(found_removed);
    }

    /// Regression test: after a broadcast::Receiver lags, the handler must
    /// re-subscribe (`rx = session.subscribe()`) before capturing `event_seq`
    /// for the recovery snapshot.  Without re-subscribing, the old receiver
    /// resumes from the oldest ring-buffer entry whose seq is *lower* than the
    /// snapshot seq, causing the frontend to silently drop every subsequent event.
    ///
    /// This test simulates the handler's Lagged recovery path and asserts that
    /// the first event received after the snapshot has seq == snapshot_seq + 1.
    #[tokio::test]
    async fn test_lagged_recovery_seq_monotonicity() {
        use tokio::sync::broadcast::error::RecvError;

        // Use a tiny channel capacity so we only need to emit a handful of
        // events to trigger Lagged rather than the default 4096+.
        const SMALL_CAP: usize = 8;
        let mut session = make_session_with_capacity(SMALL_CAP);

        // Subscribe a "slow" receiver that we will intentionally lag.
        let mut slow_rx = session.subscribe();

        // Emit capacity+1 events so slow_rx is guaranteed to lag.
        let overflow_count = SMALL_CAP + 1;
        for _ in 0..overflow_count {
            session.emit(ChatEvent::QueueUpdated {
                queue_size: 0,
                queued_items: vec![],
            });
        }

        // Confirm that slow_rx is lagged.
        assert!(
            matches!(slow_rx.recv().await, Err(RecvError::Lagged(_))),
            "slow_rx should be lagged after overflow"
        );

        // --- Simulate the handler's recovery path ---
        // After Lagged, the handler must:
        //   1. Lock the session
        //   2. Re-subscribe to get a fresh receiver
        //   3. Capture event_seq for the recovery snapshot
        //   4. Drop the lock
        //   5. Emit one more event (from some background task)
        //   6. Assert first recv() on fresh_rx has seq == snapshot_seq + 1

        // Step 2-3: re-subscribe while holding the "lock" (single-threaded here).
        let mut fresh_rx = session.subscribe();
        let snapshot_seq = session.event_seq;

        // Step 5: emit one more event (e.g. a RuntimeUpdated broadcast).
        session.emit(ChatEvent::QueueUpdated {
            queue_size: 0,
            queued_items: vec![],
        });

        // Step 6: the first event from fresh_rx must have seq == snapshot_seq + 1.
        let json = fresh_rx
            .recv()
            .await
            .expect("fresh_rx should receive an event");

        let envelope: EventEnvelope = serde_json::from_str(&json).unwrap();

        assert_eq!(
            envelope.seq,
            snapshot_seq + 1,
            "First event after re-subscribe must have seq == snapshot_seq + 1, \
             got {} (snapshot_seq={}). \
             If seq < snapshot_seq the frontend drops all events forever.",
            envelope.seq,
            snapshot_seq
        );
    }

    #[test]
    #[ignore]
    fn stress_emit_and_snapshot_large_history_baseline() {
        const MESSAGE_COUNT: usize = 2_000;
        const MESSAGE_SIZE: usize = 2_048;
        const SNAPSHOT_RUNS: usize = 200;

        let mut session = make_session();

        for i in 0..MESSAGE_COUNT {
            session.add_message(ChatMessage {
                message_id: format!("m{}", i),
                role: if i % 2 == 0 {
                    "user".to_string()
                } else {
                    "assistant".to_string()
                },
                content: ChatContent::SimpleText("x".repeat(MESSAGE_SIZE)),
                ..Default::default()
            });
        }

        let emit_start = Instant::now();
        for _ in 0..1_500 {
            session.emit(ChatEvent::QueueUpdated {
                queue_size: 0,
                queued_items: vec![],
            });
        }
        let emit_elapsed = emit_start.elapsed();

        let snapshot_start = Instant::now();
        for _ in 0..SNAPSHOT_RUNS {
            let snapshot = session.snapshot();
            if let ChatEvent::Snapshot { messages, .. } = snapshot {
                assert_eq!(messages.len(), MESSAGE_COUNT);
            } else {
                panic!("Expected Snapshot event");
            }
        }
        let snapshot_elapsed = snapshot_start.elapsed();

        println!(
            "STRESS_BASELINE session_emit_snapshot: messages={}, msg_size={}, emits=1500, snapshots={}, emit_ms={}, snapshot_ms={}",
            MESSAGE_COUNT,
            MESSAGE_SIZE,
            SNAPSHOT_RUNS,
            emit_elapsed.as_millis(),
            snapshot_elapsed.as_millis(),
        );
    }

    #[test]
    #[ignore]
    fn stress_broadcast_lag_recovery_baseline() {
        let event_count = limits().event_channel_capacity * 3;

        let mut session = make_session();
        let mut slow_rx = session.subscribe();

        let emit_start = Instant::now();
        for i in 0..event_count {
            session.emit(ChatEvent::MessageAdded {
                message: ChatMessage {
                    message_id: format!("lag-{}", i),
                    role: "assistant".to_string(),
                    content: ChatContent::SimpleText("delta".to_string()),
                    ..Default::default()
                },
                index: i,
            });
        }
        let emit_elapsed = emit_start.elapsed();

        let recv_start = Instant::now();
        let mut received = 0usize;
        let mut lagged = 0usize;
        loop {
            match slow_rx.try_recv() {
                Ok(_envelope) => {
                    received += 1;
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_skipped)) => {
                    lagged += 1;
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Empty)
                | Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                    break;
                }
            }
        }
        let recv_elapsed = recv_start.elapsed();

        assert!(lagged > 0, "Expected lagged receiver under saturation");

        println!(
            "STRESS_BASELINE broadcast_lag: emitted={}, received={}, lagged_events={}, emit_ms={}, drain_ms={}, channel_capacity={}",
            event_count,
            received,
            lagged,
            emit_elapsed.as_millis(),
            recv_elapsed.as_millis(),
            limits().event_channel_capacity,
        );
    }

    #[test]
    fn test_active_command_initial_default() {
        let session = make_session();
        assert!(session.active_command.context_fork.is_none());
        assert!(session.active_command.model_override.is_none());
        assert!(session.active_command.allowed_tools.is_empty());
        assert!(session.active_command.name.is_empty());
    }

    #[test]
    fn test_active_command_stored_and_cleared() {
        let mut session = make_session();
        session.active_command = ActiveCommandContext {
            name: "my-agent".to_string(),
            allowed_tools: vec!["cat".to_string()],
            model_override: Some("gpt-4".to_string()),
            context_fork: Some("subagent".to_string()),
            started_at_index: None,
            activation_tool_call_id: None,
        };
        assert_eq!(
            session.active_command.context_fork,
            Some("subagent".to_string())
        );
        assert_eq!(session.active_command.name, "my-agent");
        session.active_command = ActiveCommandContext::default();
        assert!(session.active_command.context_fork.is_none());
        assert!(session.active_command.name.is_empty());
    }

    #[test]
    fn test_new_with_trajectory_active_command_default() {
        use crate::call_validation::{ChatContent};
        let msg = ChatMessage {
            role: "user".into(),
            content: ChatContent::SimpleText("hello".into()),
            ..Default::default()
        };
        let thread = ThreadParams {
            id: "traj-fork".into(),
            ..Default::default()
        };
        let session = ChatSession::new_with_trajectory(
            "traj-fork".into(),
            vec![msg],
            thread,
            "2024-01-01T00:00:00Z".into(),
            None,
            Vec::new(),
            None,
        );
        assert!(session.active_command.context_fork.is_none());
        assert!(session.active_command.model_override.is_none());
    }

    #[test]
    fn test_set_clear_active_skill() {
        let mut session = make_session();
        assert!(session.thread.active_skill.is_none());
        session.set_active_skill("test-skill".to_string());
        assert_eq!(session.thread.active_skill, Some("test-skill".to_string()));
        assert!(session.trajectory_dirty);
        session.clear_active_skill();
        assert!(session.thread.active_skill.is_none());
    }

    fn make_user_message(text: &str) -> ChatMessage {
        ChatMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            role: "user".to_string(),
            content: crate::call_validation::ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn test_skill_deactivation_cleanup_compacts_messages() {
        let mut session = make_session();

        // Add 2 pre-skill messages
        session.add_message(make_user_message("pre-skill message 1"));
        session.add_message(make_user_message("pre-skill message 2"));
        let anchor = session.messages.len(); // = 2

        // Add skill-run messages that should be removed
        session.add_message(make_user_message("skill run message A"));
        session.add_message(make_user_message("skill run message B"));
        session.add_message(make_user_message("skill run message C"));
        assert_eq!(session.messages.len(), 5);

        session.pending_skill_deactivation = Some(crate::chat::types::PendingSkillDeactivation {
            start_index: anchor,
            skill_name: "my-skill".to_string(),
            report: "Did useful things.".to_string(),
            activation_tool_call_id: None,
        });

        session.perform_skill_deactivation_cleanup();

        // 2 pre-skill messages + 1 report = 3 total
        assert_eq!(session.messages.len(), 3, "Expected 2 pre-skill + 1 report");
        let last = session.messages.last().unwrap();
        assert_eq!(last.role, "plain_text");
        if let crate::call_validation::ChatContent::SimpleText(ref text) = last.content {
            assert!(
                text.contains("## Skill Report: my-skill"),
                "Report header missing: {}",
                text
            );
            assert!(
                text.contains("Skill 'my-skill' executed successfully"),
                "Report preface missing: {}",
                text
            );
            assert!(
                text.contains("Did useful things."),
                "Report body missing: {}",
                text
            );
        } else {
            panic!("Expected SimpleText content in report message");
        }
        // pending is consumed
        assert!(session.pending_skill_deactivation.is_none());
    }

    #[test]
    fn test_skill_deactivation_cleanup_noop_when_no_pending() {
        let mut session = make_session();
        session.add_message(make_user_message("msg1"));
        session.add_message(make_user_message("msg2"));

        session.perform_skill_deactivation_cleanup();
        // Nothing changed
        assert_eq!(session.messages.len(), 2);
    }

    #[test]
    fn test_skill_deactivation_keeps_activation_tool_message() {
        let mut session = make_session();

        session.add_message(make_user_message("pre-skill"));
        let anchor = session.messages.len();

        let tool_message = ChatMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            role: "tool".to_string(),
            content: ChatContent::SimpleText("Skill activated".to_string()),
            tool_call_id: "call_activate_skill".to_string(),
            tool_failed: Some(false),
            ..Default::default()
        };
        session.add_message(tool_message);

        session.add_message(ChatMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            role: "context_file".to_string(),
            content: ChatContent::SimpleText("Skill body".to_string()),
            ..Default::default()
        });
        session.add_message(make_user_message("skill run"));

        session.pending_skill_deactivation = Some(crate::chat::types::PendingSkillDeactivation {
            start_index: anchor,
            skill_name: "tool-skill".to_string(),
            report: "Wrapped up".to_string(),
            activation_tool_call_id: Some("call_activate_skill".to_string()),
        });

        session.perform_skill_deactivation_cleanup();

        assert_eq!(
            session.messages.len(),
            3,
            "Expected pre-skill + tool + report"
        );
        assert_eq!(
            session.messages[1].role, "tool",
            "Activation tool message must remain"
        );
        assert_eq!(session.messages[1].tool_call_id, "call_activate_skill");
        assert_eq!(session.messages.last().unwrap().role, "plain_text");
    }

    #[test]
    fn test_skill_deactivation_skips_exact_activation_tool_call_id() {
        let mut session = make_session();

        session.add_message(make_user_message("pre-skill"));
        let anchor = session.messages.len();

        let unrelated_tool = ChatMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            role: "tool".to_string(),
            content: ChatContent::SimpleText("Unrelated tool".to_string()),
            tool_call_id: "call_other_tool".to_string(),
            tool_failed: Some(false),
            ..Default::default()
        };
        session.add_message(unrelated_tool);

        let activation_tool = ChatMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            role: "tool".to_string(),
            content: ChatContent::SimpleText("Skill activated".to_string()),
            tool_call_id: "call_activate_skill".to_string(),
            tool_failed: Some(false),
            ..Default::default()
        };
        session.add_message(activation_tool);

        session.add_message(ChatMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            role: "cd_instruction".to_string(),
            content: ChatContent::SimpleText("Skill body".to_string()),
            ..Default::default()
        });
        session.add_message(make_user_message("skill run"));

        session.pending_skill_deactivation = Some(crate::chat::types::PendingSkillDeactivation {
            start_index: anchor,
            skill_name: "tool-skill".to_string(),
            report: "Wrapped up".to_string(),
            activation_tool_call_id: Some("call_activate_skill".to_string()),
        });

        session.perform_skill_deactivation_cleanup();

        assert_eq!(
            session.messages.len(),
            3,
            "Expected pre-skill + activation tool + report"
        );
        assert_eq!(session.messages[1].tool_call_id, "call_activate_skill");
    }

    #[test]
    fn test_skill_deactivation_without_anchor_still_records_report() {
        let mut session = make_session();
        session.add_message(make_user_message("pre-skill"));

        session.pending_skill_deactivation = Some(crate::chat::types::PendingSkillDeactivation {
            start_index: session.messages.len(),
            skill_name: "no-anchor".to_string(),
            report: "All done".to_string(),
            activation_tool_call_id: None,
        });

        session.perform_skill_deactivation_cleanup();

        assert_eq!(session.messages.len(), 2, "Expected pre-skill + report");
        let last = session.messages.last().unwrap();
        assert_eq!(last.role, "plain_text");
        if let crate::call_validation::ChatContent::SimpleText(ref text) = last.content {
            assert!(text.contains("## Skill Report: no-anchor"));
            assert!(text.contains("All done"));
        }
    }

    #[test]
    fn test_skill_deactivation_cleanup_rejects_out_of_range_index() {
        let mut session = make_session();
        session.add_message(make_user_message("only message"));
        assert_eq!(session.messages.len(), 1);

        session.pending_skill_deactivation = Some(crate::chat::types::PendingSkillDeactivation {
            start_index: 99, // beyond messages.len()
            skill_name: "bad-skill".to_string(),
            report: "report".to_string(),
            activation_tool_call_id: None,
        });

        session.perform_skill_deactivation_cleanup();

        // No truncation, no report added — skipped with warning
        assert_eq!(
            session.messages.len(),
            1,
            "Messages must not be modified on bad index"
        );
        assert!(
            session.pending_skill_deactivation.is_none(),
            "pending must be consumed even on skip"
        );
    }

    #[test]
    fn test_new_with_trajectory_clears_active_skill() {
        let thread = ThreadParams {
            id: "t1".into(),
            active_skill: Some("leftover-skill".to_string()),
            ..Default::default()
        };
        let session = ChatSession::new_with_trajectory(
            "t1".into(),
            vec![],
            thread,
            "2024-01-01T00:00:00Z".into(),
            None,
            Vec::new(),
            None,
        );
        assert!(
            session.thread.active_skill.is_none(),
            "active_skill must be cleared on restore: compaction anchor is lost after restart"
        );
    }

    #[test]
    fn test_emit_broadcast_is_valid_json() {
        let mut session = make_session();
        let mut rx = session.subscribe();
        session.emit(ChatEvent::PauseCleared {});
        let json = rx.try_recv().unwrap();
        let envelope: EventEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(envelope.chat_id, "test-chat");
        assert_eq!(envelope.seq, 1);
        assert!(matches!(envelope.event, ChatEvent::PauseCleared {}));
    }

    #[test]
    fn test_emit_broadcast_multiple_subscribers_identical_payload() {
        let mut session = make_session();
        let mut rx1 = session.subscribe();
        let mut rx2 = session.subscribe();
        session.emit(ChatEvent::PauseCleared {});
        let j1 = rx1.try_recv().unwrap();
        let j2 = rx2.try_recv().unwrap();
        assert_eq!(j1, j2);
    }

    #[test]
    fn test_duplicate_request_hashset_stays_in_sync() {
        let mut session = make_session();
        assert!(!session.is_duplicate_request("req-x"));
        assert!(session.recent_request_ids.contains(&"req-x".to_string()));
        assert!(session.recent_request_ids_set.contains("req-x"));
        assert!(session.is_duplicate_request("req-x"));
    }

    #[test]
    fn test_duplicate_request_hashset_eviction_in_sync() {
        let mut session = make_session();
        for i in 0..100 {
            session.is_duplicate_request(&format!("req-{}", i));
        }
        session.is_duplicate_request("req-100");
        assert!(!session.recent_request_ids_set.contains("req-0"));
        assert!(session.recent_request_ids_set.contains("req-100"));
        assert_eq!(
            session.recent_request_ids.len(),
            session.recent_request_ids_set.len()
        );
    }

    fn make_assistant_with_tool_calls(ids: &[&str]) -> ChatMessage {
        use crate::call_validation::{ChatToolCall, ChatToolFunction};
        ChatMessage {
            role: "assistant".to_string(),
            tool_calls: Some(
                ids.iter()
                    .map(|id| ChatToolCall {
                        id: id.to_string(),
                        index: None,
                        function: ChatToolFunction {
                            name: "shell".to_string(),
                            arguments: "{}".to_string(),
                        },
                        tool_type: "function".to_string(),
                        extra_content: None,
                    })
                    .collect(),
            ),
            ..Default::default()
        }
    }

    fn make_pause_reason(tool_call_id: &str) -> PauseReason {
        PauseReason {
            reason_type: "confirmation".into(),
            tool_name: "shell".into(),
            command: "shell".into(),
            rule: "ask".into(),
            tool_call_id: tool_call_id.into(),
            integr_config_path: None,
        }
    }

    fn make_tool_result(tool_call_id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            tool_call_id: tool_call_id.to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            tool_failed: Some(false),
            ..Default::default()
        }
    }

    fn role_sequence(session: &ChatSession) -> Vec<&str> {
        session
            .messages
            .iter()
            .map(|message| message.role.as_str())
            .collect()
    }

    fn default_openai_settings() -> crate::llm::adapter::AdapterSettings {
        crate::llm::adapter::AdapterSettings {
            api_key: "test-key".to_string(),
            auth_token: String::new(),
            endpoint: "https://api.openai.com/v1/chat/completions".to_string(),
            extra_headers: Default::default(),
            model_name: "gpt-4.1".to_string(),
            supports_tools: true,
            supports_reasoning: true,
            reasoning_type: None,
            supports_temperature: true,
            supports_max_completion_tokens: false,
            eof_is_done: false,
            supports_web_search: false,
            supports_cache_control: false,
        }
    }

    fn assert_openai_tool_results_follow_assistant(messages: Vec<ChatMessage>) {
        use crate::llm::adapter::LlmWireAdapter;
        use crate::llm::adapters::openai_chat::OpenAiChatAdapter;

        let req = crate::llm::canonical::LlmRequest::new("gpt-4.1".to_string(), messages);
        let body = OpenAiChatAdapter
            .build_http(&req, &default_openai_settings())
            .unwrap()
            .body;
        let wire_messages = body["messages"].as_array().unwrap();
        for (idx, message) in wire_messages.iter().enumerate() {
            if message["role"] != "assistant" || message.get("tool_calls").is_none() {
                continue;
            }
            let tool_calls = message["tool_calls"].as_array().unwrap();
            let expected_ids: HashSet<String> = tool_calls
                .iter()
                .filter_map(|tool_call| tool_call["id"].as_str().map(str::to_string))
                .collect();
            let actual_ids: HashSet<String> = wire_messages
                .iter()
                .skip(idx + 1)
                .take(tool_calls.len())
                .map(|tool_result| {
                    assert_eq!(
                        tool_result["role"], "tool",
                        "wire messages: {wire_messages:?}"
                    );
                    tool_result["tool_call_id"].as_str().unwrap().to_string()
                })
                .collect();
            assert_eq!(actual_ids, expected_ids, "wire messages: {wire_messages:?}");
        }
    }

    #[test]
    fn denied_tool_call_produces_synthetic_tool_result_message() {
        let mut session = make_session();
        session.add_message(make_assistant_with_tool_calls(&["tc1"]));
        session.runtime.pause_reasons.push(make_pause_reason("tc1"));
        session.set_runtime_state(SessionState::Paused, None);

        let outcome = session.process_tool_decisions(&[ToolDecisionItem {
            tool_call_id: "tc1".into(),
            accepted: false,
        }]);

        assert_eq!(outcome.denied_ids, vec!["tc1"]);
        assert!(outcome.accepted_ids.is_empty());
        let tool_msgs: Vec<_> = session
            .messages
            .iter()
            .filter(|m| m.role == "tool")
            .collect();
        assert_eq!(tool_msgs.len(), 1);
        assert_eq!(tool_msgs[0].tool_call_id, "tc1");
        assert_eq!(
            tool_msgs[0].content,
            ChatContent::SimpleText("Tool call denied by user.".to_string())
        );
    }

    #[test]
    fn denied_then_accepted_in_same_decision_batch_keeps_accepted_running() {
        let mut session = make_session();
        session.add_message(make_assistant_with_tool_calls(&["tc1", "tc2"]));
        session.runtime.pause_reasons.push(make_pause_reason("tc1"));
        session.runtime.pause_reasons.push(make_pause_reason("tc2"));
        session.set_runtime_state(SessionState::Paused, None);

        let outcome = session.process_tool_decisions(&[
            ToolDecisionItem {
                tool_call_id: "tc1".into(),
                accepted: false,
            },
            ToolDecisionItem {
                tool_call_id: "tc2".into(),
                accepted: true,
            },
        ]);

        assert_eq!(outcome.accepted_ids, vec!["tc2"]);
        assert_eq!(outcome.denied_ids, vec!["tc1"]);
        let tool_msgs: Vec<_> = session
            .messages
            .iter()
            .filter(|m| m.role == "tool")
            .collect();
        assert_eq!(tool_msgs.len(), 1);
        assert_eq!(tool_msgs[0].tool_call_id, "tc1");
    }

    #[test]
    fn all_denied_synthesizes_tool_result_and_leaves_state_to_caller() {
        let mut session = make_session();
        session.add_message(make_assistant_with_tool_calls(&["tc1"]));
        session.runtime.pause_reasons.push(make_pause_reason("tc1"));
        session.set_runtime_state(SessionState::Paused, None);

        let outcome = session.process_tool_decisions(&[ToolDecisionItem {
            tool_call_id: "tc1".into(),
            accepted: false,
        }]);

        assert_eq!(outcome.denied_ids, vec!["tc1"]);
        assert!(session.runtime.pause_reasons.is_empty());
        assert_eq!(session.runtime.state, SessionState::Paused);
        assert_eq!(
            session.messages.iter().filter(|m| m.role == "tool").count(),
            1
        );
    }

    #[test]
    fn denied_tool_call_unmatched_id_logs_warning_but_does_not_panic() {
        let mut session = make_session();
        session.runtime.pause_reasons.push(make_pause_reason("tc1"));
        session.set_runtime_state(SessionState::Paused, None);

        let outcome = session.process_tool_decisions(&[ToolDecisionItem {
            tool_call_id: "tc1".into(),
            accepted: false,
        }]);

        assert_eq!(outcome.denied_ids, vec!["tc1"]);
        assert!(session.messages.iter().all(|m| m.role != "tool"));
        assert!(session.runtime.pause_reasons.is_empty());
        assert_eq!(session.runtime.state, SessionState::Paused);
    }

    #[test]
    fn synthesized_tool_result_message_has_correct_tool_call_id() {
        let mut session = make_session();
        session.add_message(make_assistant_with_tool_calls(&["unique-call-abc"]));
        session
            .runtime
            .pause_reasons
            .push(make_pause_reason("unique-call-abc"));
        session.set_runtime_state(SessionState::Paused, None);

        session.process_tool_decisions(&[ToolDecisionItem {
            tool_call_id: "unique-call-abc".into(),
            accepted: false,
        }]);

        let tool_msg = session
            .messages
            .iter()
            .find(|m| m.role == "tool")
            .expect("no tool message synthesized");
        assert_eq!(tool_msg.tool_call_id, "unique-call-abc");
    }

    #[test]
    fn rejected_tool_decision_event_after_synthetic_tool_result() {
        let mut session = make_session();
        session.add_message(make_assistant_with_tool_calls(&["tc1"]));
        session.runtime.pause_reasons.push(make_pause_reason("tc1"));
        session.set_runtime_state(SessionState::Paused, None);

        session.process_tool_decisions(&[ToolDecisionItem {
            tool_call_id: "tc1".into(),
            accepted: false,
        }]);

        assert_eq!(role_sequence(&session), vec!["assistant", "tool", "event"]);
        assert_eq!(session.messages[1].tool_call_id, "tc1");
        assert_eq!(
            session.messages[2].extra["event"]["subkind"],
            json!("tool_decision")
        );
        assert_eq!(
            session.messages[2].extra["event"]["payload"]["decision"],
            json!("reject")
        );
    }

    #[test]
    fn mixed_approve_reject_defers_event_until_accepted_tool_result() {
        let mut session = make_session();
        session.add_message(make_assistant_with_tool_calls(&["tc1", "tc2"]));
        session.runtime.pause_reasons.push(make_pause_reason("tc1"));
        session.runtime.pause_reasons.push(make_pause_reason("tc2"));
        session.set_runtime_state(SessionState::Paused, None);

        session.process_tool_decisions(&[
            ToolDecisionItem {
                tool_call_id: "tc1".into(),
                accepted: false,
            },
            ToolDecisionItem {
                tool_call_id: "tc2".into(),
                accepted: true,
            },
        ]);

        assert_eq!(role_sequence(&session), vec!["assistant", "tool"]);
        assert_eq!(session.post_tool_side_effects.len(), 2);

        session.add_message(make_tool_result("tc2", "accepted result"));
        session.drain_post_tool_side_effects();

        assert_eq!(
            role_sequence(&session),
            vec!["assistant", "tool", "tool", "event", "event"]
        );
        assert_eq!(
            session.messages[3].extra["event"]["payload"]["decision"],
            json!("approve")
        );
        assert_eq!(
            session.messages[4].extra["event"]["payload"]["decision"],
            json!("reject")
        );
    }

    #[test]
    fn ide_callback_event_deferred_until_all_ide_tool_results_present() {
        let mut session = make_session();
        session.add_message(make_assistant_with_tool_calls(&["tc1", "tc2"]));
        session.runtime.state = SessionState::WaitingIde;

        let completed =
            session.record_ide_tool_result("tc1".to_string(), "first".to_string(), false);

        assert!(!completed);
        assert_eq!(role_sequence(&session), vec!["assistant", "tool"]);
        assert_eq!(session.runtime.state, SessionState::WaitingIde);
        assert_eq!(session.post_tool_side_effects.len(), 1);

        let completed =
            session.record_ide_tool_result("tc2".to_string(), "second".to_string(), false);

        assert!(completed);
        assert_eq!(
            role_sequence(&session),
            vec!["assistant", "tool", "tool", "event", "event"]
        );
        assert_eq!(session.runtime.state, SessionState::WaitingIde);
        assert!(session.post_tool_side_effects.is_empty());
        assert_eq!(
            session.messages[3].extra["event"]["subkind"],
            json!("ide_callback")
        );
        assert_eq!(
            session.messages[4].extra["event"]["subkind"],
            json!("ide_callback")
        );
    }

    #[test]
    fn ide_tool_completion_ignores_stale_reused_tool_result_ids() {
        let mut session = make_session();
        session.add_message(make_assistant_with_tool_calls(&["tc1"]));
        session.add_message(make_tool_result("tc1", "old result"));
        session.add_message(ChatMessage::new("user".to_string(), "again".to_string()));
        session.add_message(make_assistant_with_tool_calls(&["tc1", "tc2"]));
        session.runtime.state = SessionState::WaitingIde;

        let completed =
            session.record_ide_tool_result("tc2".to_string(), "second".to_string(), false);

        assert!(!completed);
        assert_eq!(session.post_tool_side_effects.len(), 1);
    }

    #[test]
    fn interruption_cleanup_ignores_stale_reused_tool_result_ids() {
        let mut session = make_session();
        session.add_message(make_assistant_with_tool_calls(&["tc1"]));
        session.add_message(make_tool_result("tc1", "old result"));
        session.add_message(ChatMessage::new("user".to_string(), "again".to_string()));
        session.add_message(make_assistant_with_tool_calls(&["tc1", "tc2"]));
        session.add_message(make_tool_result("tc2", "current second result"));

        session.clear_pending_tool_calls_for_interruption();

        let latest_assistant = session
            .messages
            .iter()
            .rev()
            .find(|message| message.role == "assistant")
            .unwrap();
        let ids: Vec<_> = latest_assistant
            .tool_calls
            .as_ref()
            .unwrap()
            .iter()
            .map(|tool_call| tool_call.id.as_str())
            .collect();
        assert_eq!(ids, vec!["tc2"]);
    }

    #[test]
    fn openai_wire_order_valid_after_tool_decision() {
        let mut session = make_session();
        session.add_message(make_assistant_with_tool_calls(&["tc1", "tc2"]));
        session.runtime.pause_reasons.push(make_pause_reason("tc1"));
        session.runtime.pause_reasons.push(make_pause_reason("tc2"));
        session.set_runtime_state(SessionState::Paused, None);

        session.process_tool_decisions(&[
            ToolDecisionItem {
                tool_call_id: "tc1".into(),
                accepted: false,
            },
            ToolDecisionItem {
                tool_call_id: "tc2".into(),
                accepted: true,
            },
        ]);
        session.add_message(make_tool_result("tc2", "accepted result"));
        session.drain_post_tool_side_effects();

        assert_openai_tool_results_follow_assistant(session.messages.clone());
    }

    #[test]
    fn openai_wire_order_valid_after_multi_ide_callbacks() {
        let mut session = make_session();
        session.add_message(make_assistant_with_tool_calls(&["tc1", "tc2"]));
        session.runtime.state = SessionState::WaitingIde;

        session.record_ide_tool_result("tc1".to_string(), "first".to_string(), false);
        session.record_ide_tool_result("tc2".to_string(), "second".to_string(), false);

        assert_openai_tool_results_follow_assistant(session.messages.clone());
    }
    #[test]
    fn update_message_invalidates_summaries_referencing_edited_source() {
        let mut session = make_session();
        let mut source = ChatMessage::new("assistant".to_string(), "original answer".to_string());
        source.message_id = "source-id".to_string();
        let mut summary = ChatMessage::new("assistant".to_string(), "summary".to_string());
        summary.message_id = "summary-id".to_string();
        summary.summarization_tier = Some("llm_segment_summary".to_string());
        summary.extra.insert(
            "compression".to_string(),
            json!({
                "kind": "llm_segment_summary",
                "insert_mode": "source_preserving",
                "source_hash": "hash-1",
                "summarized_source_message_ids": ["source-id"],
            }),
        );
        let mut report = ChatMessage::new("compression_report".to_string(), "report".to_string());
        report.message_id = "report-id".to_string();
        report.extra.insert(
            "compression_report".to_string(),
            json!({ "kind": "chat_compression_report", "source_hash": "hash-1" }),
        );
        let mut unrelated_summary =
            ChatMessage::new("assistant".to_string(), "other summary".to_string());
        unrelated_summary.message_id = "other-summary-id".to_string();
        unrelated_summary.summarization_tier = Some("llm_segment_summary".to_string());
        unrelated_summary.extra.insert(
            "compression".to_string(),
            json!({
                "kind": "llm_segment_summary",
                "source_hash": "hash-2",
                "summarized_source_message_ids": ["unrelated-id"],
            }),
        );
        session.messages = vec![
            ChatMessage::new("user".to_string(), "question".to_string()),
            source.clone(),
            report,
            summary,
            unrelated_summary,
        ];

        let mut edited = source;
        edited.content = ChatContent::SimpleText("edited answer".to_string());
        session.update_message("source-id", edited);

        let ids: Vec<&str> = session
            .messages
            .iter()
            .map(|message| message.message_id.as_str())
            .collect();
        assert!(!ids.contains(&"summary-id"));
        assert!(!ids.contains(&"report-id"));
        assert!(ids.contains(&"other-summary-id"));
        assert_eq!(
            session.messages[1].content.content_text_only(),
            "edited answer"
        );
    }

    fn summary_fixture_messages() -> Vec<ChatMessage> {
        let mut source = ChatMessage::new("assistant".to_string(), "original answer".to_string());
        source.message_id = "source-id".to_string();
        let mut summary = ChatMessage::new("assistant".to_string(), "summary".to_string());
        summary.message_id = "summary-id".to_string();
        summary.summarization_tier = Some("llm_segment_summary".to_string());
        summary.extra.insert(
            "compression".to_string(),
            json!({
                "kind": "llm_segment_summary",
                "insert_mode": "source_preserving",
                "source_hash": "hash-1",
                "summarized_source_message_ids": ["source-id"],
            }),
        );
        let mut report = ChatMessage::new("compression_report".to_string(), "report".to_string());
        report.message_id = "report-id".to_string();
        report.extra.insert(
            "compression_report".to_string(),
            json!({ "kind": "chat_compression_report", "source_hash": "hash-1" }),
        );
        let mut unrelated_summary =
            ChatMessage::new("assistant".to_string(), "other summary".to_string());
        unrelated_summary.message_id = "other-summary-id".to_string();
        unrelated_summary.summarization_tier = Some("llm_segment_summary".to_string());
        unrelated_summary.extra.insert(
            "compression".to_string(),
            json!({
                "kind": "llm_segment_summary",
                "source_hash": "hash-2",
                "summarized_source_message_ids": ["other-source-id"],
            }),
        );
        let mut other_source =
            ChatMessage::new("assistant".to_string(), "other answer".to_string());
        other_source.message_id = "other-source-id".to_string();
        let mut user_msg = ChatMessage::new("user".to_string(), "question".to_string());
        user_msg.message_id = "user-id".to_string();
        vec![
            user_msg,
            other_source,
            unrelated_summary,
            report,
            summary,
            source,
        ]
    }

    #[test]
    fn remove_message_drops_summaries_referencing_removed_source() {
        let mut session = make_session();
        session.messages = summary_fixture_messages();

        session.remove_message("source-id");

        let ids: Vec<&str> = session
            .messages
            .iter()
            .map(|message| message.message_id.as_str())
            .collect();
        assert!(!ids.contains(&"source-id"));
        assert!(!ids.contains(&"summary-id"));
        assert!(!ids.contains(&"report-id"));
        assert!(ids.contains(&"other-summary-id"));
        assert!(ids.contains(&"other-source-id"));
    }

    #[test]
    fn truncate_messages_drops_summaries_referencing_truncated_sources() {
        let mut session = make_session();
        session.messages = summary_fixture_messages();

        // Truncate away only the trailing source; its summary and report sit earlier
        // in the transcript and must be invalidated.
        session.truncate_messages(5);

        let ids: Vec<&str> = session
            .messages
            .iter()
            .map(|message| message.message_id.as_str())
            .collect();
        assert!(!ids.contains(&"source-id"));
        assert!(!ids.contains(&"summary-id"));
        assert!(!ids.contains(&"report-id"));
        assert!(ids.contains(&"other-summary-id"));
        assert!(ids.contains(&"other-source-id"));
    }

    #[test]
    fn reset_compaction_runtime_state_clears_reactive_breaker_and_hashes() {
        let mut session = make_session();
        session.thread.reactive_compact_attempts = Some(2);
        session
            .compression_insufficient_hashes
            .insert("hash".to_string());
        session.pending_max_new_tokens_boost = Some(16_000);

        session.reset_compaction_runtime_state();

        assert_eq!(session.thread.reactive_compact_attempts, None);
        assert!(session.compression_insufficient_hashes.is_empty());
        assert!(session.pending_max_new_tokens_boost.is_none());
    }
}
