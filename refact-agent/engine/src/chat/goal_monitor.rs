use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::json;
use tokio::sync::Mutex as AMutex;
use tokio::time::sleep;

use crate::agents::execution::ExecutionState;
use crate::app_state::AppState;
use crate::chat::internal_roles::{self, EventSubkind};
use crate::chat::process_command_queue;
use crate::chat::trajectories::maybe_save_trajectory_background_with_intent;
use crate::chat::types::*;

pub const GOAL_MONITOR_INTERVAL: Duration = Duration::from_secs(30);
const GOAL_MONITOR_STALL_GRACE: Duration = Duration::from_secs(30);
const GOAL_MONITOR_DIAGNOSTIC_GRACE: Duration = Duration::from_secs(180);
const GOAL_MONITOR_SOURCE: &str = "chat.goal_monitor";
/// After this many consecutive no-progress nudges an active goal whose no-progress
/// budget is unlimited stops being nudged (goes quiescent) and resumes on the next
/// user message instead of looping forever.
pub(crate) const QUIESCENCE_NUDGES: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalNudgeTrigger {
    Monitor,
    InlineTurnEnd,
}

impl GoalNudgeTrigger {
    fn as_str(self) -> &'static str {
        match self {
            GoalNudgeTrigger::Monitor => "monitor",
            GoalNudgeTrigger::InlineTurnEnd => "inline_turn_end",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalNudgeReason {
    Idle,
    Error,
    Completed,
    TurnEnd,
}

impl GoalNudgeReason {
    fn as_str(self) -> &'static str {
        match self {
            GoalNudgeReason::Idle => "idle",
            GoalNudgeReason::Error => "error",
            GoalNudgeReason::Completed => "completed",
            GoalNudgeReason::TurnEnd => "turn_end",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalNudgeSkip {
    NoGoal,
    InactiveGoal,
    NonActiveGoalStatus,
    Snoozed,
    Waiting,
    Cooldown,
    Aborted,
    Closed,
    WaitingForInput,
    IntentionallyWaiting,
    Busy,
    UserMessageQueued,
    PendingCommand,
    NotStalled,
    QueueRejected,
    Quiescent,
    Sealed,
    OwnerUnavailable,
    ReadOnlyHistory,
    InvalidContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalNudgeOutcome {
    Nudged(GoalNudgeReason),
    BudgetExhausted(GoalStatus),
    WaitExpired,
    Skipped(GoalNudgeSkip),
}

impl GoalNudgeOutcome {
    fn changed(self) -> bool {
        matches!(
            self,
            GoalNudgeOutcome::Nudged(_)
                | GoalNudgeOutcome::BudgetExhausted(_)
                | GoalNudgeOutcome::WaitExpired
        )
    }

    fn nudged(self) -> bool {
        matches!(self, GoalNudgeOutcome::Nudged(_))
    }

    /// Both a nudge and a wait that just expired have queued a wake delivery the
    /// command processor has to pick up.
    fn wakes_queue(self) -> bool {
        self.nudged() || matches!(self, GoalNudgeOutcome::WaitExpired)
    }

    fn should_persist(self) -> bool {
        self.changed() || matches!(self, GoalNudgeOutcome::Skipped(GoalNudgeSkip::Quiescent))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GoalNudgeConfig {
    pub stall_grace: Duration,
    pub no_token_grace: Duration,
}

impl Default for GoalNudgeConfig {
    fn default() -> Self {
        Self {
            stall_grace: GOAL_MONITOR_STALL_GRACE,
            no_token_grace: GOAL_MONITOR_DIAGNOSTIC_GRACE,
        }
    }
}

fn epoch_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub async fn start_goal_monitor(app: AppState) {
    tracing::info!("Starting goal monitor");

    loop {
        let shutdown_flag = app.runtime.shutdown_flag.clone();
        tokio::select! {
            _ = sleep(GOAL_MONITOR_INTERVAL) => {}
            _ = async {
                while !shutdown_flag.load(Ordering::SeqCst) {
                    sleep(Duration::from_millis(200)).await;
                }
            } => {
                tracing::info!("Goal monitor: shutdown detected, stopping");
                return;
            }
        }

        if let Err(error) = check_goal_sessions(app.clone()).await {
            tracing::error!("Goal monitor error: {}", error);
        }
    }
}

async fn check_goal_sessions(app: AppState) -> Result<(), String> {
    let sessions = {
        let sessions_read = app.chat.sessions.read().await;
        sessions_read.values().cloned().collect::<Vec<_>>()
    };

    for session_arc in sessions {
        dispatch_goal_nudge(app.clone(), session_arc, GoalNudgeTrigger::Monitor).await;
    }

    Ok(())
}

pub async fn handle_goal_turn_end(app: AppState, session_arc: Arc<AMutex<ChatSession>>) -> bool {
    let outcome = dispatch_goal_nudge(
        app.clone(),
        session_arc.clone(),
        GoalNudgeTrigger::InlineTurnEnd,
    )
    .await;
    if outcome == GoalNudgeOutcome::Skipped(GoalNudgeSkip::ReadOnlyHistory) {
        return false;
    }
    if matches!(
        outcome,
        GoalNudgeOutcome::Skipped(GoalNudgeSkip::Sealed | GoalNudgeSkip::OwnerUnavailable)
    ) {
        return true;
    }
    if goal_turn_end_blocks_completion(outcome) {
        return true;
    }

    if let Err(error) =
        crate::tasks::goal::hydrate_session_task_goal_for_mutation(app.gcx.clone(), &session_arc)
            .await
    {
        tracing::warn!(%error, "goal turn end owner unavailable");
        return true;
    }
    let now_ms = epoch_ms_now();
    let (terminal, changed) = {
        let mut session = session_arc.lock().await;
        match record_terminal_goal_event_if_needed(
            &mut session,
            GoalNudgeTrigger::InlineTurnEnd,
            now_ms,
        ) {
            Some(changed) => (true, changed),
            None => (false, false),
        }
    };
    if changed {
        if let Err(error) =
            crate::tasks::goal::commit_session_task_goal(app.gcx.clone(), &session_arc).await
        {
            tracing::warn!(%error, "goal turn end commit rejected");
            return true;
        }
        maybe_save_trajectory_background_with_intent(
            app,
            session_arc,
            TrajectoryCommitIntent::Checkpoint,
        );
    }
    terminal
}

fn goal_turn_end_blocks_completion(outcome: GoalNudgeOutcome) -> bool {
    outcome.changed()
}

pub async fn dispatch_goal_nudge(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    trigger: GoalNudgeTrigger,
) -> GoalNudgeOutcome {
    let chat_id = {
        let session = session_arc.lock().await;
        if session.has_runner_lease() {
            return GoalNudgeOutcome::Skipped(GoalNudgeSkip::Busy);
        }
        session.chat_id.clone()
    };
    if crate::agents::seal::is_sealed(&app, &chat_id).await {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::Sealed);
    }
    if app.agents.child_chat_paused(&chat_id).await {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::IntentionallyWaiting);
    }
    match crate::tasks::goal::hydrate_session_task_goal_for_pursuit(app.gcx.clone(), &session_arc)
        .await
    {
        Ok(true) => {}
        Ok(false) => return GoalNudgeOutcome::Skipped(GoalNudgeSkip::ReadOnlyHistory),
        Err(error) => {
            tracing::warn!(%chat_id, %error, "goal nudge owner unavailable");
            return GoalNudgeOutcome::Skipped(GoalNudgeSkip::OwnerUnavailable);
        }
    }
    let now_ms = epoch_ms_now();
    let (outcome, processor_flag) = {
        let mut session = session_arc.lock().await;
        let execution = run_execution_state(&app, &session.chat_id);
        let outcome = try_apply_goal_nudge(
            &mut session,
            trigger,
            now_ms,
            Instant::now(),
            GoalNudgeConfig::default(),
            execution,
        );
        let processor_flag = if outcome.wakes_queue() {
            session.queue_notify.notify_one();
            Some(session.queue_processor_running.clone())
        } else {
            None
        };
        (outcome, processor_flag)
    };

    if outcome.should_persist() {
        if let Err(error) =
            crate::tasks::goal::commit_session_task_goal(app.gcx.clone(), &session_arc).await
        {
            tracing::warn!(%chat_id, %error, "goal nudge commit rejected");
            return GoalNudgeOutcome::Skipped(GoalNudgeSkip::OwnerUnavailable);
        }
        maybe_save_trajectory_background_with_intent(
            app.clone(),
            session_arc.clone(),
            TrajectoryCommitIntent::Checkpoint,
        );
    }

    if let Some(processor_flag) = processor_flag {
        if !processor_flag.swap(true, Ordering::SeqCst) {
            tokio::spawn(process_command_queue(app, session_arc, processor_flag));
        }
    }

    outcome
}

/// Admission is the fuller source: it reports a run that is still queued and has
/// never been admitted, which owns no `RunExecution` yet. The registry covers
/// chat-runtime children, which have an execution but no admission entry.
fn run_execution_state(app: &AppState, chat_id: &str) -> Option<ExecutionState> {
    crate::tasks::admission::execution_state(chat_id)
        .or_else(|| app.agent_executions.get(chat_id).map(|run| run.state()))
}

/// A run that released or never took its execution permit is waiting by design —
/// on a child, an approval, a question or a timer — not stalling. Same triple
/// `agents::monitor` and `tool_task_check_agents` already treat as waiting.
fn intentionally_waiting(execution: Option<ExecutionState>) -> bool {
    matches!(
        execution,
        Some(ExecutionState::Parked | ExecutionState::Acquiring | ExecutionState::Suspending)
    )
}

pub fn try_apply_goal_nudge(
    session: &mut ChatSession,
    trigger: GoalNudgeTrigger,
    now_ms: u64,
    now_instant: Instant,
    config: GoalNudgeConfig,
    execution: Option<ExecutionState>,
) -> GoalNudgeOutcome {
    if session.closed {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::Closed);
    }
    if session.has_runner_lease() {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::Busy);
    }

    // A private pre-model continuation owns this frontier. Appending a goal
    // nudge would invalidate it and repeatedly resubmit the same preflight.
    if session.review_readiness_pending.is_some() || session.verifier_readiness_pending.is_some() {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::IntentionallyWaiting);
    }

    let Some(goal) = session.goal.as_ref() else {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::NoGoal);
    };
    if !goal.active {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::InactiveGoal);
    }
    if goal.status != GoalStatus::Active {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::NonActiveGoalStatus);
    }
    if goal
        .snoozed_until_ms
        .is_some_and(|until_ms| now_ms < until_ms)
    {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::Snoozed);
    }
    match goal.wait.as_ref().map(|wait| wait.expired_at(now_ms)) {
        Some(false) => return GoalNudgeOutcome::Skipped(GoalNudgeSkip::Waiting),
        Some(true) => return expire_goal_wait(session, now_ms),
        None => {}
    }
    if session.abort_flag.load(Ordering::SeqCst)
        || session.user_interrupt_flag.load(Ordering::SeqCst)
    {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::Aborted);
    }
    if let Some(status) = goal.goal_budget_exhaustion_status_at(now_ms) {
        apply_goal_terminal_status(session, status, trigger, now_ms);
        return GoalNudgeOutcome::BudgetExhausted(status);
    }
    if crate::chat::context_rebuild::compression_attempt_active(session) {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::Busy);
    }
    if intentionally_waiting(execution) {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::IntentionallyWaiting);
    }
    if !goal.goal_nudge_ready_at_with_backoff(now_ms) {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::Cooldown);
    }
    if waiting_for_user_or_ide(session) {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::WaitingForInput);
    }
    if busy_without_stall(session, trigger, now_instant, config) {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::Busy);
    }
    if queued_user_message(session) {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::UserMessageQueued);
    }
    if !session.command_queue.is_empty()
        || !session.delivery_wake_sources.is_empty()
        || session
            .pending_deliveries
            .iter()
            .any(|delivery| delivery.wake)
    {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::PendingCommand);
    }

    let Some(reason) = nudge_reason(session, trigger, now_instant, config) else {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::NotStalled);
    };

    if goal_is_quiescent(session) {
        record_quiescent_event_if_needed(session, trigger, now_ms);
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::Quiescent);
    }

    let context = match nudge_context(session) {
        Ok(context) => context,
        Err(error) => {
            tracing::warn!(%error, "Goal nudge blocked by invalid control context");
            return GoalNudgeOutcome::Skipped(GoalNudgeSkip::InvalidContext);
        }
    };
    let delivery = PendingDelivery::new(
        vec![goal_nudge_event(trigger, reason, now_ms, &context)],
        PushMode::Append,
        GOAL_MONITOR_SOURCE,
        true,
    );
    match session.enqueue_delivery(delivery) {
        Ok(DeliveryOutcome::Delivered) => {
            session.queue_notify.notify_one();
        }
        Ok(_) => {}
        Err(_) => return GoalNudgeOutcome::Skipped(GoalNudgeSkip::QueueRejected),
    }
    if reason == GoalNudgeReason::Error {
        session.goal_note_no_progress_turn();
    }
    session.goal_record_nudge(now_ms);
    GoalNudgeOutcome::Nudged(reason)
}

/// An expired deadline is never a pass. The wait ends with an explicit expired
/// outcome naming its subject, and the goal is only then eligible for nudges
/// again.
fn expire_goal_wait(session: &mut ChatSession, now_ms: u64) -> GoalNudgeOutcome {
    match crate::chat::goal_wait::settle_goal_wait_in_session(
        session,
        None,
        GoalWaitResume::Expired,
        now_ms,
    ) {
        Some(_) => GoalNudgeOutcome::WaitExpired,
        None => GoalNudgeOutcome::Skipped(GoalNudgeSkip::Waiting),
    }
}

fn waiting_for_user_or_ide(session: &ChatSession) -> bool {
    if session
        .runtime
        .recovery
        .as_ref()
        .is_some_and(|record| record.holds_goal())
    {
        return true;
    }
    matches!(
        session.runtime.state,
        SessionState::Paused | SessionState::WaitingUserInput | SessionState::WaitingIde
    ) || !session.runtime.pause_reasons.is_empty()
        || session.pending_browser_message.is_some()
}

fn busy_without_stall(
    session: &ChatSession,
    trigger: GoalNudgeTrigger,
    now: Instant,
    config: GoalNudgeConfig,
) -> bool {
    match trigger {
        GoalNudgeTrigger::InlineTurnEnd => session.runtime.state != SessionState::Idle,
        GoalNudgeTrigger::Monitor => match session.runtime.state {
            SessionState::Generating => {
                if generating_quiet_for(session, now, config.no_token_grace).is_some() {
                    tracing::debug!(chat_id = %session.chat_id, "Goal monitor: stream silence; request watchdog owns recovery");
                }
                true
            }
            SessionState::ExecutingTools => true,
            _ => false,
        },
    }
}

fn queued_user_message(session: &ChatSession) -> bool {
    session
        .command_queue
        .iter()
        .any(|request| matches!(request.command, ChatCommand::UserMessage { .. }))
}

fn nudge_reason(
    session: &ChatSession,
    trigger: GoalNudgeTrigger,
    now: Instant,
    config: GoalNudgeConfig,
) -> Option<GoalNudgeReason> {
    match trigger {
        GoalNudgeTrigger::InlineTurnEnd => Some(GoalNudgeReason::TurnEnd),
        GoalNudgeTrigger::Monitor => match session.runtime.state {
            SessionState::Idle if session.last_activity + config.stall_grace <= now => {
                Some(GoalNudgeReason::Idle)
            }
            SessionState::Error if session.last_activity + config.stall_grace <= now => {
                Some(GoalNudgeReason::Error)
            }
            SessionState::Completed if session.last_activity + config.stall_grace <= now => {
                Some(GoalNudgeReason::Completed)
            }
            _ => None,
        },
    }
}

fn generating_quiet_for(session: &ChatSession, now: Instant, grace: Duration) -> Option<Duration> {
    let last_progress = session
        .last_stream_delta_at
        .unwrap_or(session.last_activity);
    if last_progress + grace <= now {
        Some(now.duration_since(last_progress))
    } else {
        None
    }
}

fn goal_nudge_event(
    trigger: GoalNudgeTrigger,
    reason: GoalNudgeReason,
    at_ms: u64,
    context: &str,
) -> crate::call_validation::ChatMessage {
    let mut content = format!(
        "Goal pursuit nudge: continue the active goal ({}, {}).",
        trigger.as_str(),
        reason.as_str()
    );
    if !context.is_empty() {
        content.push('\n');
        content.push_str(context);
    }
    internal_roles::event(
        EventSubkind::GoalPursuit,
        GOAL_MONITOR_SOURCE,
        json!({
            "kind": "nudge",
            "trigger": trigger.as_str(),
            "reason": reason.as_str(),
            "at_ms": at_ms,
            "account_progress": true,
        }),
        content,
    )
}

fn bounded_excerpt(text: &str, limit: usize) -> String {
    let mut chars = text.trim().chars();
    let mut out: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() && limit > 0 {
        out.pop();
        out.push('…');
    }
    out
}

fn nudge_context(session: &ChatSession) -> Result<String, String> {
    let projection = session.try_accepted_control_projection()?;
    let goal = session.goal.as_ref().ok_or("goal unavailable")?;
    let base = refact_chat_api::goal_role::current_base_goal(&projection)
        .ok_or("installed goal control unavailable")?;
    let start = projection
        .messages
        .iter()
        .position(|m| std::ptr::eq(m, base))
        .unwrap();
    let mut lines = Vec::new();
    let latest_goal = refact_chat_api::goal_role::goal_delta_events(&projection)
        .into_iter()
        .rev()
        .map(|m| m.content.content_text_only())
        .find(|text| !text.trim().is_empty());
    let (label, text) = match latest_goal {
        Some(text) => ("Latest goal update", text),
        None => ("Goal", base.content.content_text_only()),
    };
    lines.push(format!("{label}: {}", bounded_excerpt(&text, 500)));
    let mut plan = base.extra["goal"]["plan"]["content"]
        .as_str()
        .map(str::to_owned);
    let mut plan_label = "Plan";
    for message in projection.messages.iter().skip(start + 1) {
        let event = message
            .extra
            .get("event")
            .unwrap_or(&serde_json::Value::Null);
        let payload = &event["payload"];
        if payload["goal_id"]
            .as_str()
            .is_some_and(|id| id != base.message_id)
            || !(refact_chat_api::goal_role::is_plan_delta(message)
                || event["subkind"].as_str() == Some("plan_delta"))
        {
            continue;
        }
        let text = payload["plan"]["content"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| message.content.content_text_only());
        if !text.trim().is_empty() {
            plan_label = if payload.get("plan").is_some() {
                "Plan"
            } else {
                "Latest plan update"
            };
            plan = Some(text);
        }
    }
    if let Some(plan) = plan {
        lines.push(format!("{plan_label}: {}", bounded_excerpt(&plan, 500)));
    }
    lines.push(
        "Latest accepted changes; consult the installed goal/plan for full constraints.".into(),
    );
    if let Some(gaps) = goal
        .attempts
        .last()
        .filter(|attempt| attempt.verdict != "met" && !attempt.gaps.is_empty())
        .map(|attempt| attempt.gaps.join("; "))
    {
        lines.push(format!("Remaining gaps: {}", bounded_excerpt(&gaps, 300)));
    }
    let turns = match goal.budget.max_turns.filter(|limit| *limit > 0) {
        Some(limit) => format!("{}/{}", goal.progress.turns_used, limit),
        None => goal.progress.turns_used.to_string(),
    };
    lines.push(bounded_excerpt(&format!(
        "Progress: turns {}, no-progress turns {}. If nothing remains, call validate_goal or finish; use wait for running work, or pause with target {{kind: \"goal\"}} and an optional until_ms deadline to suspend pursuit.",
        turns, goal.progress.no_progress_turns
    ), 220));
    Ok(bounded_excerpt(&lines.join("\n"), 1600))
}

fn goal_is_quiescent(session: &ChatSession) -> bool {
    session.goal.as_ref().is_some_and(|goal| {
        goal.budget.no_progress_turns.is_none_or(|limit| limit == 0)
            && goal.progress.no_progress_turns >= QUIESCENCE_NUDGES
    })
}

pub fn mark_goal_blocked_on_context_limit(session: &mut ChatSession) -> bool {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let has_finite_limit = {
        let Some(goal) = session.goal.as_ref() else {
            return false;
        };
        if !goal.active || !matches!(goal.status, GoalStatus::Active) {
            return false;
        }
        goal.budget.no_progress_turns.is_some_and(|limit| limit > 0)
    };
    while session.goal.as_ref().is_some_and(|goal| {
        matches!(goal.status, GoalStatus::Active)
            && goal.progress.no_progress_turns < QUIESCENCE_NUDGES
    }) {
        if !session.goal_note_no_progress_turn() {
            break;
        }
    }
    if has_finite_limit
        && session
            .goal
            .as_ref()
            .is_some_and(|goal| goal.status == GoalStatus::Active)
    {
        session.goal_set_status_reason(GoalStatus::NoProgress, "context_limit");
    }
    if has_finite_limit {
        record_terminal_goal_event_if_needed(session, GoalNudgeTrigger::Monitor, now_ms);
    } else {
        record_quiescent_event_if_needed(session, GoalNudgeTrigger::Monitor, now_ms);
    }
    session.mark_persisted_runtime_changed();
    session.emit_goal_status();
    true
}

fn goal_pursuit_kind(message: &crate::call_validation::ChatMessage) -> Option<&str> {
    if message.role != internal_roles::EVENT_ROLE {
        return None;
    }
    let event = message.extra.get("event")?;
    if event.get("subkind").and_then(|value| value.as_str()) != Some("goal_pursuit") {
        return None;
    }
    event
        .get("payload")
        .and_then(|payload| payload.get("kind"))
        .and_then(|value| value.as_str())
}

fn quiescent_event_already_recorded(session: &ChatSession) -> bool {
    session
        .messages
        .iter()
        .chain(
            session
                .pending_deliveries
                .iter()
                .flat_map(|delivery| delivery.messages.iter()),
        )
        .rev()
        .find_map(goal_pursuit_kind)
        .is_some_and(|kind| kind == "pursuit_quiescent")
}

fn record_quiescent_event_if_needed(
    session: &mut ChatSession,
    trigger: GoalNudgeTrigger,
    at_ms: u64,
) -> bool {
    if quiescent_event_already_recorded(session) {
        return false;
    }
    session
        .enqueue_delivery(PendingDelivery::new(
            vec![goal_quiescent_event(trigger, at_ms)],
            PushMode::Append,
            GOAL_MONITOR_SOURCE,
            false,
        ))
        .is_ok()
}

fn goal_quiescent_event(
    trigger: GoalNudgeTrigger,
    at_ms: u64,
) -> crate::call_validation::ChatMessage {
    internal_roles::event(
        EventSubkind::GoalPursuit,
        GOAL_MONITOR_SOURCE,
        json!({
            "kind": "pursuit_quiescent",
            "trigger": trigger.as_str(),
            "at_ms": at_ms,
        }),
        "Automatic goal reminders are quiet after repeated no-progress attempts or a context limit. The goal status is unchanged; a new user message can reset the reminder throttle."
            .to_string(),
    )
}

fn record_terminal_goal_event_if_needed(
    session: &mut ChatSession,
    trigger: GoalNudgeTrigger,
    at_ms: u64,
) -> Option<bool> {
    let status = session.goal.as_ref()?.status;
    if !matches!(status, GoalStatus::BudgetExhausted | GoalStatus::NoProgress) {
        return None;
    }
    let kind = terminal_kind(status);
    let already_recorded = session.goal.as_ref().is_some_and(|goal| {
        goal.events
            .iter()
            .any(|event| event.kind == "goal_pursuit" && event.text.contains(kind))
    });
    if already_recorded
        || session
            .pending_deliveries
            .iter()
            .flat_map(|delivery| delivery.messages.iter())
            .any(|message| goal_pursuit_kind(message) == Some(kind))
    {
        return Some(false);
    }
    Some(
        session
            .enqueue_delivery(PendingDelivery::new(
                vec![goal_terminal_event(status, trigger, at_ms)],
                PushMode::Append,
                GOAL_MONITOR_SOURCE,
                false,
            ))
            .is_ok(),
    )
}

fn terminal_kind(status: GoalStatus) -> &'static str {
    match status {
        GoalStatus::NoProgress => "no_progress",
        _ => "budget_exhausted",
    }
}

fn apply_goal_terminal_status(
    session: &mut ChatSession,
    status: GoalStatus,
    trigger: GoalNudgeTrigger,
    at_ms: u64,
) {
    session.goal_set_status(status);
    let _ = session.enqueue_delivery(PendingDelivery::new(
        vec![goal_terminal_event(status, trigger, at_ms)],
        PushMode::Append,
        GOAL_MONITOR_SOURCE,
        false,
    ));
}

fn goal_terminal_event(
    status: GoalStatus,
    trigger: GoalNudgeTrigger,
    at_ms: u64,
) -> crate::call_validation::ChatMessage {
    let kind = terminal_kind(status);
    internal_roles::event(
        EventSubkind::GoalPursuit,
        GOAL_MONITOR_SOURCE,
        json!({
            "kind": kind,
            "trigger": trigger.as_str(),
            "at_ms": at_ms,
        }),
        format!("Goal pursuit stopped: {kind}."),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::execution::ExecutionState;
    use crate::call_validation::{ChatContent, ChatMessage, ChatUsage};

    fn config() -> GoalNudgeConfig {
        GoalNudgeConfig {
            stall_grace: Duration::from_secs(5),
            no_token_grace: Duration::from_secs(5),
        }
    }

    fn active_goal_session() -> ChatSession {
        let mut session = ChatSession::new("goal-monitor-test".to_string());
        session.install_goal(
            "agent",
            "ship the thing",
            true,
            GoalBudget {
                max_turns: Some(10),
                max_minutes: Some(60),
                max_tokens: Some(10_000),
                max_cost_cents: None,
                cooldown_ms: 1_000,
                no_progress_token_threshold: 50,
                no_progress_turns: Some(2),
                explicit: false,
            },
        );
        session
    }

    fn old_idle_session() -> (ChatSession, Instant) {
        let mut session = active_goal_session();
        let now = Instant::now();
        session.last_activity = now - Duration::from_secs(10);
        (session, now)
    }

    fn unlimited_no_progress_session() -> ChatSession {
        let mut session = ChatSession::new("goal-monitor-quiescent-test".to_string());
        session.install_goal(
            "agent",
            "ship the thing",
            true,
            GoalBudget {
                max_turns: None,
                max_minutes: None,
                max_tokens: None,
                max_cost_cents: None,
                cooldown_ms: 1_000,
                no_progress_token_threshold: 50,
                no_progress_turns: None,
                explicit: false,
            },
        );
        session
    }

    fn apply_monitor(session: &mut ChatSession, now_ms: u64, now: Instant) -> GoalNudgeOutcome {
        apply_monitor_with(session, now_ms, now, None)
    }

    fn apply_monitor_with(
        session: &mut ChatSession,
        now_ms: u64,
        now: Instant,
        execution: Option<ExecutionState>,
    ) -> GoalNudgeOutcome {
        try_apply_goal_nudge(
            session,
            GoalNudgeTrigger::Monitor,
            now_ms,
            now,
            config(),
            execution,
        )
    }

    async fn goal_app() -> (AppState, tempfile::TempDir) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let workspace = tempfile::tempdir().unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        (AppState::from_gcx(gcx).await, workspace)
    }

    async fn stalled_goal_session(app: &AppState, chat_id: &str) -> Arc<AMutex<ChatSession>> {
        let mut session = ChatSession::new(chat_id.to_string());
        session.install_goal(
            "agent",
            "ship the thing",
            true,
            GoalBudget {
                max_turns: None,
                max_minutes: None,
                max_tokens: None,
                max_cost_cents: None,
                cooldown_ms: 1_000,
                no_progress_token_threshold: 50,
                no_progress_turns: None,
                explicit: false,
            },
        );
        session.last_activity = Instant::now() - Duration::from_secs(600);
        session
            .queue_processor_running
            .store(true, Ordering::SeqCst);
        let session = Arc::new(AMutex::new(session));
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session.clone());
        session
    }

    fn unique_chat_id(tag: &str) -> String {
        format!("goal-monitor-{tag}-{}", uuid::Uuid::new_v4())
    }

    async fn admitted_execution(
        app: &AppState,
        chat_id: &str,
    ) -> Arc<crate::agents::execution::RunExecution> {
        crate::tasks::admission::reserve(app, chat_id, "project", "root", 0)
            .await
            .unwrap();
        crate::tasks::admission::wait_for_permit(chat_id)
            .await
            .unwrap();
        app.agent_executions.get(chat_id).unwrap()
    }

    async fn monitor_tick(app: &AppState, session: &Arc<AMutex<ChatSession>>) -> GoalNudgeOutcome {
        dispatch_goal_nudge(app.clone(), session.clone(), GoalNudgeTrigger::Monitor).await
    }

    /// Negative control: suppressing parked/queued must not disable stall
    /// detection. A run holding its permit and producing nothing is still stalled.
    #[tokio::test]
    async fn goal_monitor_still_nudges_a_running_run_with_no_progress() {
        let (app, _workspace) = goal_app().await;
        let chat_id = unique_chat_id("running");
        let session = stalled_goal_session(&app, &chat_id).await;
        let execution = admitted_execution(&app, &chat_id).await;
        assert_eq!(execution.state(), ExecutionState::Running);

        assert_eq!(
            monitor_tick(&app, &session).await,
            GoalNudgeOutcome::Nudged(GoalNudgeReason::Idle)
        );

        crate::tasks::admission::end(&chat_id).await;
        app.agent_executions.shutdown().await;
    }

    /// The scheduler's own view of this run's permit. Counted per run, not by
    /// the global active total, so a parallel test holding a permit elsewhere
    /// cannot decide this assertion.
    fn active_permits_for(app: &AppState, chat_id: &str) -> usize {
        let run = refact_work_core::RunKey::new(
            refact_work_core::WorkKey::trajectory(chat_id.to_string()),
            1,
        );
        app.work_scheduler
            .snapshot()
            .tickets
            .iter()
            .filter(|ticket| {
                ticket.run == run && ticket.state == refact_work_core::TicketState::Active
            })
            .count()
    }

    /// Waiting must not occupy capacity: the wait hands the permit back through
    /// the scheduler's park path and holds none for as long as it waits.
    #[tokio::test]
    async fn a_waiting_goal_holds_no_execution_permit() {
        let (app, _workspace) = goal_app().await;
        let chat_id = unique_chat_id("wait-permit");
        let session = stalled_goal_session(&app, &chat_id).await;
        let execution = admitted_execution(&app, &chat_id).await;
        assert_eq!(execution.state(), ExecutionState::Running);
        assert_eq!(active_permits_for(&app, &chat_id), 1);

        {
            let mut session = session.lock().await;
            assert!(session.goal_begin_wait(wait_on("proc-7", None), 1_000));
        }
        crate::chat::goal_wait::begin_goal_wait_park(app.clone(), session.clone()).await;

        assert_eq!(execution.state(), ExecutionState::Parked);
        assert_eq!(active_permits_for(&app, &chat_id), 0);
        for _ in 0..3 {
            assert_eq!(
                monitor_tick(&app, &session).await,
                GoalNudgeOutcome::Skipped(GoalNudgeSkip::Waiting)
            );
            assert_eq!(active_permits_for(&app, &chat_id), 0);
        }

        crate::tasks::admission::end(&chat_id).await;
        app.agent_executions.shutdown().await;
    }

    fn event_payload(message: &ChatMessage) -> &serde_json::Value {
        message
            .extra
            .get("event")
            .and_then(|event| event.get("payload"))
            .unwrap()
    }

    fn nudge_event_count(session: &ChatSession) -> usize {
        session
            .messages
            .iter()
            .filter(|message| {
                message.role == "event"
                    && event_payload(message).get("kind") == Some(&json!("nudge"))
            })
            .count()
    }

    #[test]
    fn goal_monitor_detects_error_and_no_token_stalls() {
        let (mut error_session, now) = old_idle_session();
        error_session.set_runtime_state(SessionState::Error, Some("provider failed".to_string()));
        error_session.last_activity = now - Duration::from_secs(10);
        assert_eq!(
            apply_monitor(&mut error_session, 10_000, now),
            GoalNudgeOutcome::Nudged(GoalNudgeReason::Error)
        );

        assert_eq!(
            error_session
                .goal
                .as_ref()
                .unwrap()
                .progress
                .no_progress_turns,
            1,
            "an error nudge must count as a no-progress turn"
        );

        let mut generating = active_goal_session();
        generating.set_runtime_state(SessionState::Generating, None);
        generating.last_activity = now - Duration::from_secs(10);
        assert_eq!(
            apply_monitor(&mut generating, 10_000, now),
            GoalNudgeOutcome::Skipped(GoalNudgeSkip::Busy)
        );
    }

    #[test]
    fn goal_monitor_error_loop_reaches_quiescence() {
        let mut session = unlimited_no_progress_session();
        let now = Instant::now();
        session.set_runtime_state(SessionState::Error, Some("permanent 400".to_string()));
        session.last_activity = now - Duration::from_secs(10);

        let mut now_ms = 10_000u64;
        let mut nudges = 0u32;
        let mut quiescent_seen = false;
        for _ in 0..20 {
            match apply_monitor(&mut session, now_ms, now) {
                GoalNudgeOutcome::Nudged(GoalNudgeReason::Error) => {
                    nudges += 1;
                    session.command_queue.clear();
                    session.delivery_wake_sources.clear();
                    session
                        .set_runtime_state(SessionState::Error, Some("permanent 400".to_string()));
                    session.last_activity = now - Duration::from_secs(10);
                }
                GoalNudgeOutcome::Skipped(GoalNudgeSkip::Cooldown) => {}
                GoalNudgeOutcome::Skipped(GoalNudgeSkip::Quiescent) => {
                    quiescent_seen = true;
                    break;
                }
                other => panic!("unexpected outcome: {:?}", other),
            }
            now_ms += 600_000;
        }

        assert_eq!(
            nudges, QUIESCENCE_NUDGES,
            "error loop must stop nudging after the quiescence threshold"
        );
        assert!(quiescent_seen, "error loop must reach quiescence");
        assert!(goal_is_quiescent(&session));
        assert_eq!(session.goal.as_ref().unwrap().status, GoalStatus::Active);
        let notice = session
            .messages
            .iter()
            .chain(
                session
                    .pending_deliveries
                    .iter()
                    .flat_map(|d| d.messages.iter()),
            )
            .find(|m| goal_pursuit_kind(m) == Some("pursuit_quiescent"))
            .unwrap();
        assert!(notice
            .content
            .content_text_only()
            .contains("goal status is unchanged"));
        assert!(!notice
            .content
            .content_text_only()
            .contains("pursuit paused"));
        assert!(session.delivery_wake_sources.is_empty());
        assert!(!record_quiescent_event_if_needed(
            &mut session,
            GoalNudgeTrigger::Monitor,
            now_ms
        ));
    }

    #[tokio::test]
    async fn goal_monitor_does_not_fire_when_user_message_queued() {
        let (app, _workspace) = goal_app().await;
        let chat_id = unique_chat_id("queued-user");
        let session_arc = stalled_goal_session(&app, &chat_id).await;
        let mut session = session_arc.lock().await;
        let ledger_before = session.goal_ledger.clone();
        session.command_queue.push_back(CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "queued-user".to_string(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("hello"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
                client_message_id: None,
                app_context_ids: Vec::new(),
            },
        });

        drop(session);

        let blocked = handle_goal_turn_end(app.clone(), session_arc.clone()).await;
        let mut session = session_arc.lock().await;
        let nudge_count = |session: &ChatSession| {
            session
                .messages
                .iter()
                .filter(|message| {
                    message.role == "event"
                        && event_payload(message).get("kind") == Some(&json!("nudge"))
                })
                .count()
        };
        assert_eq!(
            nudge_count(&session),
            0,
            "queued user must suppress actual nudge delivery"
        );
        assert!(!blocked);
        assert!(session.delivery_wake_sources.is_empty());
        assert!(session.pending_deliveries.is_empty());
        assert_eq!(session.goal_ledger, ledger_before);
        assert_eq!(
            session.command_queue.front().unwrap().client_request_id,
            "queued-user"
        );
        assert!(matches!(
            session.command_queue.front().unwrap().command,
            ChatCommand::UserMessage { .. }
        ));
        assert!(!session.has_runner_lease());

        assert_eq!(
            apply_monitor(&mut session, epoch_ms_now(), Instant::now()),
            GoalNudgeOutcome::Skipped(GoalNudgeSkip::UserMessageQueued)
        );
        assert_eq!(session.command_queue.len(), 1);

        session.command_queue.clear();
        drop(session);
        assert!(handle_goal_turn_end(app.clone(), session_arc.clone()).await);
        let session = session_arc.lock().await;
        assert_eq!(nudge_count(&session), 1);
        assert!(session.delivery_wake_sources.contains("goal"));
        assert!(session.goal.as_ref().unwrap().progress.last_nudge_at_ms > 0);
    }

    #[test]
    fn context_limit_block_large_limit_appends_cas_visible_status_change() {
        let mut session = ChatSession::new("goal-monitor-large-limit".to_string());
        session.install_goal(
            "agent",
            "ship the thing",
            true,
            GoalBudget {
                max_turns: None,
                max_minutes: None,
                max_tokens: None,
                max_cost_cents: None,
                cooldown_ms: 1_000,
                no_progress_token_threshold: 50,
                no_progress_turns: Some(10),
                explicit: false,
            },
        );
        let epoch = session.goal_ledger_last_seq();
        assert!(mark_goal_blocked_on_context_limit(&mut session));
        assert_eq!(
            session.goal.as_ref().unwrap().status,
            GoalStatus::NoProgress
        );
        assert!(session.goal_status_changed_since(epoch));
        let replayed = refact_chat_api::reduce_goal_ledger(&session.goal_ledger).unwrap();
        assert_eq!(replayed.status, GoalStatus::NoProgress);
    }

    #[test]
    fn goal_monitor_post_restart_dormant_goal_nudged_once_after_grace() {
        let (mut session, now) = old_idle_session();
        assert_eq!(session.goal.as_ref().unwrap().progress.last_nudge_at_ms, 0);

        assert_eq!(
            apply_monitor(&mut session, 10_000, now),
            GoalNudgeOutcome::Nudged(GoalNudgeReason::Idle)
        );
        assert_eq!(
            apply_monitor(&mut session, 11_500, now + Duration::from_secs(10)),
            GoalNudgeOutcome::Skipped(GoalNudgeSkip::PendingCommand)
        );
        assert!(session.command_queue.is_empty());
        assert_eq!(session.delivery_wake_sources.len(), 1);
        let nudge_events = session
            .messages
            .iter()
            .filter(|message| {
                message.role == "event"
                    && event_payload(message).get("kind") == Some(&json!("nudge"))
            })
            .count();
        assert_eq!(nudge_events, 1);
        let prefix = session.goal_ledger.clone();
        session.delivery_wake_sources.clear();
        session.last_activity = now - Duration::from_secs(10);
        assert_eq!(
            apply_monitor(&mut session, 30_000, now),
            GoalNudgeOutcome::Nudged(GoalNudgeReason::Idle)
        );
        assert!(session.goal_ledger.starts_with(&prefix));
        let nudges: Vec<_> = session
            .goal_ledger
            .iter()
            .filter(|entry| matches!(entry.op, refact_chat_api::GoalLedgerOp::NudgeRecorded))
            .collect();
        assert_eq!(nudges.len(), 2);
        assert_eq!(nudges[0].at_ms, 10_000);
        assert_eq!(nudges[1].at_ms, 30_000);
        assert!(nudges[1].seq > nudges[0].seq);
        assert_eq!(nudge_event_count(&session), 2);
        assert_eq!(
            refact_chat_api::reduce_goal_ledger(&session.goal_ledger)
                .unwrap()
                .progress
                .last_nudge_at_ms,
            30_000
        );
        let ledger = session.goal_ledger.clone();
        assert_eq!(
            apply_monitor(&mut session, 30_001, now),
            GoalNudgeOutcome::Skipped(GoalNudgeSkip::Cooldown)
        );
        assert_eq!(session.goal_ledger, ledger);
        assert_eq!(nudge_event_count(&session), 2);
    }

    #[test]
    fn goal_monitor_snoozed_goal_skips_until_expiry() {
        let (mut session, now) = old_idle_session();
        assert!(session.goal_set_snooze(Some(15_000)));

        assert_eq!(
            apply_monitor(&mut session, 10_000, now),
            GoalNudgeOutcome::Skipped(GoalNudgeSkip::Snoozed)
        );
        assert!(session.command_queue.is_empty());

        session.last_activity = now - Duration::from_secs(10);
        assert_eq!(
            apply_monitor(&mut session, 20_000, now),
            GoalNudgeOutcome::Nudged(GoalNudgeReason::Idle)
        );
    }

    fn wait_on(process_id: &str, deadline_ms: Option<u64>) -> GoalWait {
        GoalWait::new(
            GoalWaitSubject::Process {
                process_id: process_id.to_string(),
            },
            deadline_ms,
        )
    }

    fn goal_pursuit_event_count(session: &ChatSession, kind: &str) -> usize {
        session
            .messages
            .iter()
            .chain(
                session
                    .pending_deliveries
                    .iter()
                    .flat_map(|delivery| delivery.messages.iter()),
            )
            .filter(|message| {
                message.role == "event" && event_payload(message).get("kind") == Some(&json!(kind))
            })
            .count()
    }

    /// A goal that declared what it waits on costs nothing while it waits: over
    /// repeated monitor ticks it produces no nudge, so nothing ever reaches a
    /// model. The nudge count is the model-call count on this path — a nudge is
    /// the only thing here that queues work for the provider.
    #[test]
    fn goal_monitor_never_nudges_a_goal_that_is_waiting_on_a_subject() {
        let (mut session, now) = old_idle_session();
        assert!(session.goal_begin_wait(wait_on("proc-7", None), 1_000));

        let mut now_ms = 10_000u64;
        for tick in 0..10 {
            assert_eq!(
                apply_monitor(&mut session, now_ms, now + Duration::from_secs(tick * 30)),
                GoalNudgeOutcome::Skipped(GoalNudgeSkip::Waiting),
                "tick {tick} nudged a waiting goal"
            );
            now_ms += 600_000;
        }

        assert_eq!(nudge_event_count(&session), 0);
        assert_eq!(goal_pursuit_event_count(&session, "nudge"), 0);
        assert!(session.command_queue.is_empty());
        assert!(session.delivery_wake_sources.is_empty());
        assert_eq!(session.goal.as_ref().unwrap().progress.no_progress_turns, 0);
        assert_eq!(session.goal.as_ref().unwrap().progress.turns_used, 0);
        assert_eq!(session.goal_status, Some(GoalStatus::Active));
    }

    /// An expired deadline is an outcome of its own. It names the subject and it
    /// is never allowed to read as a pass.
    #[test]
    fn an_expired_wait_deadline_is_reported_expired_and_never_met() {
        let (mut session, now) = old_idle_session();
        assert!(session.goal_begin_wait(wait_on("proc-7", Some(15_000)), 1_000));

        assert_eq!(
            apply_monitor(&mut session, 14_999, now),
            GoalNudgeOutcome::Skipped(GoalNudgeSkip::Waiting)
        );
        assert_eq!(
            apply_monitor(&mut session, 15_000, now),
            GoalNudgeOutcome::WaitExpired
        );

        assert!(session.goal_wait().is_none());
        assert_ne!(session.goal_status, Some(GoalStatus::Completed));
        assert_eq!(session.goal_status, Some(GoalStatus::Active));
        assert_eq!(goal_pursuit_event_count(&session, "wait_expired"), 1);
        let expired = session
            .messages
            .iter()
            .chain(
                session
                    .pending_deliveries
                    .iter()
                    .flat_map(|delivery| delivery.messages.iter()),
            )
            .find(|message| {
                message.role == "event"
                    && event_payload(message).get("kind") == Some(&json!("wait_expired"))
            })
            .unwrap();
        assert_eq!(event_payload(expired)["subject"], json!("process proc-7"));
        assert!(expired
            .content
            .content_text_only()
            .contains("did not settle before its deadline"));
    }

    #[test]
    fn goal_monitor_nudge_content_carries_goal_context() {
        let (mut session, now) = old_idle_session();
        session.messages[0].content = ChatContent::SimpleText("obsolete scope ".repeat(80));
        let base_id = session.messages[0].message_id.clone();
        session.add_message(internal_roles::goal_delta(
            "test",
            json!({"goal_id":base_id,"seq":1}),
            "latest corrected outcome",
        ));
        session.add_message(internal_roles::goal_delta(
            "test",
            json!({"goal_id":base_id,"target":"plan","seq":1}),
            "latest corrected plan",
        ));
        session.add_message(internal_roles::goal_delta(
            "test",
            json!({"goal_id":"superseded","target":"plan","seq":2}),
            "superseded plan note",
        ));
        session.last_activity = now - Duration::from_secs(10);
        session.goal.as_mut().unwrap().attempts.push(GoalAttempt {
            at_ms: 1,
            trigger: "finish".to_string(),
            verdict: "unmet".to_string(),
            gaps: vec!["docs missing".to_string()],
            verifier_reply: String::new(),
            criteria_verdicts: Vec::new(),
        });

        assert_eq!(
            apply_monitor(&mut session, 10_000, now),
            GoalNudgeOutcome::Nudged(GoalNudgeReason::Idle)
        );

        let content = session.messages.last().unwrap().content.content_text_only();
        assert!(content.contains("Latest goal update: latest corrected outcome"));
        assert!(content.contains("Latest plan update: latest corrected plan"));
        assert!(!content.contains("obsolete scope"));
        assert!(!content.contains("superseded plan note"));
        assert!(content.chars().count() <= 1700);
        assert!(content.contains("Remaining gaps: docs missing"));
        assert!(content.contains("pause with target"));
    }

    #[test]
    fn goal_monitor_active_compression_is_busy_and_not_interrupted() {
        let now = Instant::now();
        for age_ms in [0, 16 * 60 * 1000] {
            let mut session = active_goal_session();
            let abort = Arc::new(std::sync::atomic::AtomicBool::new(false));
            // Rebuilds reserve an idle session, not a concurrently running stream.
            session.active_compression_attempt = Some(7);
            session.compression_abort_flag = Some(abort.clone());
            session.compression_attempt_started_at_ms = Some(epoch_ms_now().saturating_sub(age_ms));
            session.is_compressing = true;
            session.runtime.is_compressing = true;
            session.compression_phase = Some(CompressionPhase::Running);
            session.runtime.compression_phase = Some(CompressionPhase::Running);
            session.last_activity = now - Duration::from_secs(40);

            assert!(
                session.start_stream().is_none(),
                "live reservation must refuse generation"
            );
            assert_eq!(
                apply_monitor(&mut session, 10_000, now),
                GoalNudgeOutcome::Skipped(GoalNudgeSkip::Busy),
                "live reservation must stay busy even at age {age_ms}ms"
            );
            assert!(!abort.load(Ordering::SeqCst));
            assert!(!session.abort_flag.load(Ordering::SeqCst));
            assert!(!session.user_interrupt_flag.load(Ordering::SeqCst));
            assert_eq!(session.active_compression_attempt, Some(7));
            assert_eq!(session.runtime.state, SessionState::Idle);
            assert!(session.draft_message.is_none());
            assert!(session.command_queue.is_empty());
            assert!(session.delivery_wake_sources.is_empty());
            assert!(session.pending_deliveries.is_empty());
        }
    }

    #[test]
    fn goal_monitor_canceled_compression_waits_for_owner_cleanup_before_recovery() {
        let now = Instant::now();
        for age_ms in [0, 16 * 60 * 1000] {
            let mut session = active_goal_session();
            let abort = Arc::new(std::sync::atomic::AtomicBool::new(false));
            session.is_compressing = true;
            session.runtime.is_compressing = true;
            session.compression_phase = Some(CompressionPhase::Running);
            session.runtime.compression_phase = Some(CompressionPhase::Running);
            session.active_compression_attempt = Some(7);
            session.compression_abort_flag = Some(abort.clone());
            session.compression_attempt_started_at_ms = Some(epoch_ms_now().saturating_sub(age_ms));
            session.last_activity = now - Duration::from_secs(40);

            abort.store(true, Ordering::SeqCst);
            assert_eq!(
                apply_monitor(&mut session, 10_000, now),
                GoalNudgeOutcome::Skipped(GoalNudgeSkip::Busy)
            );
            assert!(session.start_stream().is_none());
            crate::chat::context_rebuild::status(
                &mut session,
                CompressionPhase::Failed,
                Some(crate::chat::types::CompressionReason::TransientFailure),
            );
            assert_eq!(
                apply_monitor(&mut session, 10_000, now),
                GoalNudgeOutcome::Nudged(GoalNudgeReason::Idle),
                "released reservation must recover at age {age_ms}ms"
            );
            assert!(!session.abort_flag.load(Ordering::SeqCst));
            assert!(!session.user_interrupt_flag.load(Ordering::SeqCst));
            assert!(!session.delivery_wake_sources.is_empty());
            assert!(
                session.start_stream().is_some(),
                "released reservation must allow generation"
            );
        }
    }

    #[test]
    fn goal_monitor_no_token_stall_preserves_running_generation() {
        let mut session = active_goal_session();
        let now = Instant::now();
        session.start_stream();
        session.last_activity = now - Duration::from_secs(10);
        session.last_stream_delta_at = Some(now - Duration::from_secs(10));
        session
            .queue_processor_running
            .store(true, Ordering::SeqCst);

        assert_eq!(
            apply_monitor(&mut session, 10_000, now),
            GoalNudgeOutcome::Skipped(GoalNudgeSkip::Busy)
        );
        assert!(!session.abort_flag.load(Ordering::SeqCst));
        assert!(!session.user_interrupt_flag.load(Ordering::SeqCst));
        assert_eq!(session.runtime.state, SessionState::Generating);
        assert!(session.delivery_wake_sources.is_empty());
        assert_eq!(session.goal.as_ref().unwrap().progress.no_progress_turns, 0);
        assert!(!session.messages.iter().any(|message| message
            .extra
            .get("event")
            .is_some_and(|event| event["subkind"] == "cancellation_note")));
    }

    #[test]
    fn goal_monitor_skips_runner_owned_idle_session() {
        let (mut session, now) = old_idle_session();
        let token = session
            .claim_runner(
                "goal-runner".into(),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
            )
            .unwrap();
        assert!(session.has_runner_lease());
        let ledger = session.goal_ledger.clone();
        assert_eq!(
            apply_monitor(&mut session, 10_000, now),
            GoalNudgeOutcome::Skipped(GoalNudgeSkip::Busy)
        );
        assert!(session.pending_deliveries.is_empty());
        assert!(session.delivery_wake_sources.is_empty());
        assert_eq!(session.goal_ledger, ledger);
        session.release_runner(&token);
    }
}
