use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::json;
use tokio::sync::Mutex as AMutex;
use tokio::time::sleep;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::chat::internal_roles::{self, EventSubkind};
use crate::chat::process_command_queue;
use crate::chat::trajectories::maybe_save_trajectory_background;
use crate::chat::types::*;

pub const GOAL_MONITOR_INTERVAL: Duration = Duration::from_secs(30);
const GOAL_MONITOR_STALL_GRACE: Duration = Duration::from_secs(30);
const GOAL_MONITOR_NO_TOKEN_GRACE: Duration = Duration::from_secs(30);
const GOAL_MONITOR_SOURCE: &str = "chat.goal_monitor";
/// After this many consecutive no-progress nudges an active goal whose no-progress
/// budget is unlimited stops being nudged (goes quiescent) and resumes on the next
/// user message instead of looping forever.
const QUIESCENCE_NUDGES: u32 = 3;

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
    GeneratingNoTokens,
    TurnEnd,
}

impl GoalNudgeReason {
    fn as_str(self) -> &'static str {
        match self {
            GoalNudgeReason::Idle => "idle",
            GoalNudgeReason::Error => "error",
            GoalNudgeReason::Completed => "completed",
            GoalNudgeReason::GeneratingNoTokens => "generating_no_tokens",
            GoalNudgeReason::TurnEnd => "turn_end",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalNudgeSkip {
    NoGoal,
    InactiveGoal,
    NonActiveGoalStatus,
    Cooldown,
    Aborted,
    Closed,
    WaitingForInput,
    Busy,
    UserMessageQueued,
    PendingCommand,
    NotStalled,
    QueueRejected,
    Quiescent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalNudgeOutcome {
    Nudged(GoalNudgeReason),
    BudgetExhausted(GoalStatus),
    Skipped(GoalNudgeSkip),
}

impl GoalNudgeOutcome {
    fn changed(self) -> bool {
        matches!(
            self,
            GoalNudgeOutcome::Nudged(_) | GoalNudgeOutcome::BudgetExhausted(_)
        )
    }

    fn nudged(self) -> bool {
        matches!(self, GoalNudgeOutcome::Nudged(_))
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
            no_token_grace: GOAL_MONITOR_NO_TOKEN_GRACE,
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
    if goal_turn_end_blocks_completion(outcome) {
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
        maybe_save_trajectory_background(app, session_arc);
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
    let now_ms = epoch_ms_now();
    let (outcome, processor_flag) = {
        let mut session = session_arc.lock().await;
        let outcome = try_apply_goal_nudge(
            &mut session,
            trigger,
            now_ms,
            Instant::now(),
            GoalNudgeConfig::default(),
        );
        let processor_flag = if outcome.nudged() {
            Some(session.queue_processor_running.clone())
        } else {
            None
        };
        (outcome, processor_flag)
    };

    if outcome.should_persist() {
        maybe_save_trajectory_background(app.clone(), session_arc.clone());
    }

    if let Some(processor_flag) = processor_flag {
        if !processor_flag.swap(true, Ordering::SeqCst) {
            tokio::spawn(process_command_queue(app, session_arc, processor_flag));
        }
    }

    outcome
}

pub fn try_apply_goal_nudge(
    session: &mut ChatSession,
    trigger: GoalNudgeTrigger,
    now_ms: u64,
    now_instant: Instant,
    config: GoalNudgeConfig,
) -> GoalNudgeOutcome {
    if session.closed {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::Closed);
    }
    if session.abort_flag.load(Ordering::SeqCst)
        || session.user_interrupt_flag.load(Ordering::SeqCst)
    {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::Aborted);
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
    if let Some(status) = goal.goal_budget_exhaustion_status_at(now_ms) {
        apply_goal_terminal_status(session, status, trigger, now_ms);
        return GoalNudgeOutcome::BudgetExhausted(status);
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
    if !session.command_queue.is_empty() {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::PendingCommand);
    }

    let Some(reason) = nudge_reason(session, trigger, now_instant, config) else {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::NotStalled);
    };

    if goal_is_quiescent(session) {
        record_quiescent_event_if_needed(session, trigger, now_ms);
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::Quiescent);
    }

    let request = CommandRequest {
        client_request_id: format!("goal-nudge-{}", Uuid::new_v4()),
        priority: true,
        command: ChatCommand::Regenerate {},
    };
    if enqueue_regenerate_for_nudge(session, reason, request) != EnqueueCommandOutcome::Accepted {
        return GoalNudgeOutcome::Skipped(GoalNudgeSkip::QueueRejected);
    }

    session.add_message(goal_nudge_event(trigger, reason, now_ms));
    session.goal_record_nudge(now_ms);
    GoalNudgeOutcome::Nudged(reason)
}

fn enqueue_regenerate_for_nudge(
    session: &mut ChatSession,
    reason: GoalNudgeReason,
    request: CommandRequest,
) -> EnqueueCommandOutcome {
    if reason == GoalNudgeReason::GeneratingNoTokens
        && session.runtime.state == SessionState::Generating
    {
        if session.command_queue.len() >= max_queue_size() {
            return EnqueueCommandOutcome::Full;
        }
        session.abort_stream();
        session.clear_pending_tool_calls_for_interruption();
    }
    session.enqueue_priority_command(request)
}

fn waiting_for_user_or_ide(session: &ChatSession) -> bool {
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
                generating_quiet_for(session, now, config.no_token_grace).is_none()
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
            SessionState::Generating => generating_quiet_for(session, now, config.no_token_grace)
                .map(|_| GoalNudgeReason::GeneratingNoTokens),
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
) -> crate::call_validation::ChatMessage {
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
        format!(
            "Goal pursuit nudge: continue the active goal ({}, {}).",
            trigger.as_str(),
            reason.as_str()
        ),
    )
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
    {
        let Some(goal) = session.goal.as_mut() else {
            return false;
        };
        if !goal.active || !matches!(goal.status, GoalStatus::Active) {
            return false;
        }
        let floor = goal
            .budget
            .no_progress_turns
            .filter(|limit| *limit > 0)
            .map(|limit| limit.max(QUIESCENCE_NUDGES))
            .unwrap_or(QUIESCENCE_NUDGES);
        if goal.progress.no_progress_turns < floor {
            goal.progress.no_progress_turns = floor;
        }
        if goal.budget.no_progress_turns.is_some_and(|limit| limit > 0) {
            goal.status = GoalStatus::NoProgress;
        }
    }
    record_quiescent_event_if_needed(session, GoalNudgeTrigger::Monitor, now_ms);
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
    session.add_message(goal_quiescent_event(trigger, at_ms));
    true
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
        "Goal pursuit paused: agent idle with no progress; will resume on your next message."
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
    if already_recorded {
        return Some(false);
    }
    session.add_message(goal_terminal_event(status, trigger, at_ms));
    Some(true)
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
    session.add_message(goal_terminal_event(status, trigger, at_ms));
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
                cooldown_ms: 1_000,
                no_progress_token_threshold: 50,
                no_progress_turns: None,
                explicit: false,
            },
        );
        session
    }

    fn apply_monitor(session: &mut ChatSession, now_ms: u64, now: Instant) -> GoalNudgeOutcome {
        try_apply_goal_nudge(session, GoalNudgeTrigger::Monitor, now_ms, now, config())
    }

    fn event_payload(message: &ChatMessage) -> &serde_json::Value {
        message
            .extra
            .get("event")
            .and_then(|event| event.get("payload"))
            .unwrap()
    }

    #[test]
    fn goal_monitor_idle_stall_enqueues_regenerate_and_records_event() {
        let (mut session, now) = old_idle_session();
        let outcome = apply_monitor(&mut session, 10_000, now);

        assert_eq!(outcome, GoalNudgeOutcome::Nudged(GoalNudgeReason::Idle));
        assert_eq!(session.command_queue.len(), 1);
        let request = session.command_queue.front().unwrap();
        assert!(request.priority);
        assert!(matches!(request.command, ChatCommand::Regenerate {}));
        assert_eq!(
            session.goal.as_ref().unwrap().progress.last_nudge_at_ms,
            10_000
        );
        let message = session.messages.last().unwrap();
        assert_eq!(message.role, "event");
        let payload = event_payload(message);
        assert_eq!(payload["kind"], json!("nudge"));
        assert_eq!(payload["trigger"], json!("monitor"));
        assert_eq!(payload["reason"], json!("idle"));
        assert_eq!(payload["account_progress"], json!(true));
        assert!(!session.goal.as_ref().unwrap().events.is_empty());
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

        let mut generating = active_goal_session();
        generating.set_runtime_state(SessionState::Generating, None);
        generating.last_activity = now - Duration::from_secs(10);
        assert_eq!(
            apply_monitor(&mut generating, 10_000, now),
            GoalNudgeOutcome::Nudged(GoalNudgeReason::GeneratingNoTokens)
        );
    }

    #[test]
    fn goal_monitor_respects_cooldown_and_grace() {
        let mut recent = active_goal_session();
        let now = Instant::now();
        recent.last_activity = now - Duration::from_secs(2);
        assert_eq!(
            apply_monitor(&mut recent, 10_000, now),
            GoalNudgeOutcome::Skipped(GoalNudgeSkip::NotStalled)
        );
        assert!(recent.command_queue.is_empty());

        let (mut cooling_down, now) = old_idle_session();
        cooling_down
            .goal
            .as_mut()
            .unwrap()
            .progress
            .last_nudge_at_ms = 9_500;
        assert_eq!(
            apply_monitor(&mut cooling_down, 10_000, now),
            GoalNudgeOutcome::Skipped(GoalNudgeSkip::Cooldown)
        );
        assert!(cooling_down.command_queue.is_empty());
    }

    #[test]
    fn goal_monitor_ignores_paused_stopped_transferred_and_waiting_states() {
        for status in [
            GoalStatus::Paused,
            GoalStatus::Stopped,
            GoalStatus::Transferred,
        ] {
            let (mut session, now) = old_idle_session();
            session.goal_set_status(status);
            session.last_activity = now - Duration::from_secs(10);
            assert_eq!(
                apply_monitor(&mut session, 10_000, now),
                GoalNudgeOutcome::Skipped(GoalNudgeSkip::NonActiveGoalStatus)
            );
            assert!(session.command_queue.is_empty());
        }

        for state in [
            SessionState::Paused,
            SessionState::WaitingUserInput,
            SessionState::WaitingIde,
        ] {
            let (mut session, now) = old_idle_session();
            session.set_runtime_state(state, None);
            session.last_activity = now - Duration::from_secs(10);
            assert_eq!(
                apply_monitor(&mut session, 10_000, now),
                GoalNudgeOutcome::Skipped(GoalNudgeSkip::WaitingForInput)
            );
            assert!(session.command_queue.is_empty());
        }
    }

    #[test]
    fn goal_monitor_does_not_fire_when_user_message_queued() {
        let (mut session, now) = old_idle_session();
        session.command_queue.push_back(CommandRequest {
            client_request_id: "queued-user".to_string(),
            priority: false,
            command: ChatCommand::UserMessage {
                content: json!("hello"),
                attachments: vec![],
                context_files: vec![],
                suppress_auto_enrichment: false,
            },
        });

        assert_eq!(
            apply_monitor(&mut session, 10_000, now),
            GoalNudgeOutcome::Skipped(GoalNudgeSkip::UserMessageQueued)
        );
        assert_eq!(session.command_queue.len(), 1);
    }

    #[test]
    fn goal_monitor_budget_exhaustion_sets_terminal_status() {
        let (mut session, now) = old_idle_session();
        session.goal.as_mut().unwrap().progress.turns_used = 10;

        assert_eq!(
            apply_monitor(&mut session, 10_000, now),
            GoalNudgeOutcome::BudgetExhausted(GoalStatus::BudgetExhausted)
        );
        assert_eq!(session.goal_status, Some(GoalStatus::BudgetExhausted));
        assert!(session.command_queue.is_empty());
        let payload = event_payload(session.messages.last().unwrap());
        assert_eq!(payload["kind"], json!("budget_exhausted"));
    }

    #[test]
    fn goal_monitor_no_progress_sets_terminal_status() {
        let (mut session, now) = old_idle_session();
        session.goal.as_mut().unwrap().progress.no_progress_turns = 2;

        assert_eq!(
            apply_monitor(&mut session, 10_000, now),
            GoalNudgeOutcome::BudgetExhausted(GoalStatus::NoProgress)
        );
        assert_eq!(session.goal_status, Some(GoalStatus::NoProgress));
        let payload = event_payload(session.messages.last().unwrap());
        assert_eq!(payload["kind"], json!("no_progress"));
    }

    #[test]
    fn goal_monitor_goes_quiescent_after_threshold_without_terminal_status() {
        let mut session = unlimited_no_progress_session();
        let now = Instant::now();
        session.last_activity = now - Duration::from_secs(10);
        session.goal.as_mut().unwrap().progress.no_progress_turns = QUIESCENCE_NUDGES;

        let first = apply_monitor(&mut session, 10_000, now);
        assert_eq!(first, GoalNudgeOutcome::Skipped(GoalNudgeSkip::Quiescent));
        assert!(session.command_queue.is_empty());
        assert_eq!(session.goal_status, Some(GoalStatus::Active));

        let second = apply_monitor(&mut session, 20_000, now + Duration::from_secs(20));
        assert_eq!(second, GoalNudgeOutcome::Skipped(GoalNudgeSkip::Quiescent));
        assert!(session.command_queue.is_empty());
        assert_eq!(session.goal_status, Some(GoalStatus::Active));

        let quiescent_events = session
            .messages
            .iter()
            .filter(|message| {
                message.role == "event"
                    && event_payload(message).get("kind") == Some(&json!("pursuit_quiescent"))
            })
            .count();
        assert_eq!(quiescent_events, 1);
    }

    #[test]
    fn context_limit_block_quiesces_unlimited_goal() {
        let mut session = unlimited_no_progress_session();
        assert!(!goal_is_quiescent(&session));
        assert!(mark_goal_blocked_on_context_limit(&mut session));
        let goal = session.goal.as_ref().unwrap();
        assert!(goal.progress.no_progress_turns >= QUIESCENCE_NUDGES);
        assert_eq!(goal.status, GoalStatus::Active);
        assert!(goal_is_quiescent(&session));
    }

    #[test]
    fn context_limit_block_terminates_finite_no_progress_goal() {
        let mut session = active_goal_session();
        assert!(mark_goal_blocked_on_context_limit(&mut session));
        assert_eq!(
            session.goal.as_ref().unwrap().status,
            GoalStatus::NoProgress
        );
    }

    #[test]
    fn context_limit_block_without_goal_is_noop() {
        let mut session = ChatSession::new("no-goal".to_string());
        assert!(!mark_goal_blocked_on_context_limit(&mut session));
    }

    #[test]
    fn goal_monitor_backoff_extends_cooldown_with_no_progress_turns() {
        let mut session = unlimited_no_progress_session();
        let now = Instant::now();
        session.last_activity = now - Duration::from_secs(10);
        {
            let goal = session.goal.as_mut().unwrap();
            goal.progress.no_progress_turns = 1;
            goal.progress.last_nudge_at_ms = 5_000;
        }

        assert_eq!(
            apply_monitor(&mut session, 6_500, now),
            GoalNudgeOutcome::Skipped(GoalNudgeSkip::Cooldown)
        );
        assert!(session.command_queue.is_empty());

        assert_eq!(
            apply_monitor(&mut session, 7_000, now),
            GoalNudgeOutcome::Nudged(GoalNudgeReason::Idle)
        );
        assert_eq!(session.command_queue.len(), 1);
    }

    #[test]
    fn goal_monitor_fresh_no_progress_uses_base_cooldown() {
        let mut session = unlimited_no_progress_session();
        let now = Instant::now();
        session.last_activity = now - Duration::from_secs(10);
        {
            let goal = session.goal.as_mut().unwrap();
            goal.progress.no_progress_turns = 0;
            goal.progress.last_nudge_at_ms = 5_000;
        }

        assert_eq!(
            apply_monitor(&mut session, 5_999, now),
            GoalNudgeOutcome::Skipped(GoalNudgeSkip::Cooldown)
        );
        assert_eq!(
            apply_monitor(&mut session, 6_000, now),
            GoalNudgeOutcome::Nudged(GoalNudgeReason::Idle)
        );
    }

    #[test]
    fn goal_monitor_resume_after_quiescence_nudges_again() {
        let mut session = unlimited_no_progress_session();
        let now = Instant::now();
        session.last_activity = now - Duration::from_secs(10);
        session.goal.as_mut().unwrap().progress.no_progress_turns = QUIESCENCE_NUDGES;

        assert_eq!(
            apply_monitor(&mut session, 10_000, now),
            GoalNudgeOutcome::Skipped(GoalNudgeSkip::Quiescent)
        );

        assert!(session.goal_reset_no_progress());
        assert_eq!(session.goal.as_ref().unwrap().progress.no_progress_turns, 0);

        assert_eq!(
            apply_monitor(&mut session, 20_000, now + Duration::from_secs(20)),
            GoalNudgeOutcome::Nudged(GoalNudgeReason::Idle)
        );
        assert_eq!(session.command_queue.len(), 1);
    }

    #[test]
    fn goal_monitor_progress_resets_no_progress_and_prevents_quiescence() {
        let mut session = unlimited_no_progress_session();
        let now = Instant::now();
        session.goal.as_mut().unwrap().progress.no_progress_turns = QUIESCENCE_NUDGES - 1;

        assert!(session.goal_record_progress_from_usage(&ChatUsage {
            prompt_tokens: 10,
            completion_tokens: 100,
            total_tokens: 110,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            metering_usd: None,
        }));
        assert_eq!(session.goal.as_ref().unwrap().progress.no_progress_turns, 0);

        // Recording progress touches `last_activity`; mark the session stalled afterwards
        // so the monitor still sees an idle stall and nudges (no quiescence at np == 0).
        session.last_activity = now - Duration::from_secs(10);
        assert_eq!(
            apply_monitor(&mut session, 10_000, now),
            GoalNudgeOutcome::Nudged(GoalNudgeReason::Idle)
        );
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
        assert_eq!(session.command_queue.len(), 1);
        let nudge_events = session
            .messages
            .iter()
            .filter(|message| {
                message.role == "event"
                    && event_payload(message).get("kind") == Some(&json!("nudge"))
            })
            .count();
        assert_eq!(nudge_events, 1);
    }

    #[test]
    fn goal_monitor_inline_turn_end_bypasses_stall_grace_and_rate_limits_monitor() {
        let mut session = active_goal_session();
        let now = Instant::now();
        session.last_activity = now;

        assert_eq!(
            try_apply_goal_nudge(
                &mut session,
                GoalNudgeTrigger::InlineTurnEnd,
                10_000,
                now,
                config(),
            ),
            GoalNudgeOutcome::Nudged(GoalNudgeReason::TurnEnd)
        );
        assert_eq!(
            apply_monitor(&mut session, 10_100, now + Duration::from_secs(10)),
            GoalNudgeOutcome::Skipped(GoalNudgeSkip::Cooldown)
        );
        assert_eq!(session.command_queue.len(), 1);
    }

    #[test]
    fn goal_monitor_no_token_stall_requires_quiet_grace() {
        let mut session = active_goal_session();
        let now = Instant::now();
        session.set_runtime_state(SessionState::Generating, None);
        session.last_activity = now - Duration::from_secs(10);
        session.last_stream_delta_at = Some(now - Duration::from_secs(2));

        assert_eq!(
            apply_monitor(&mut session, 10_000, now),
            GoalNudgeOutcome::Skipped(GoalNudgeSkip::Busy)
        );
        assert!(session.command_queue.is_empty());
    }

    #[test]
    fn goal_monitor_no_token_stall_enqueues_regenerate() {
        let mut session = active_goal_session();
        let now = Instant::now();
        session.set_runtime_state(SessionState::Generating, None);
        session.last_activity = now - Duration::from_secs(10);

        assert_eq!(
            apply_monitor(&mut session, 10_000, now),
            GoalNudgeOutcome::Nudged(GoalNudgeReason::GeneratingNoTokens)
        );
        assert_eq!(session.command_queue.len(), 1);
    }

    #[test]
    fn goal_monitor_no_token_stall_interrupts_running_generation() {
        let mut session = active_goal_session();
        let now = Instant::now();
        session.start_stream();
        session.last_activity = now - Duration::from_secs(10);
        session
            .queue_processor_running
            .store(true, Ordering::SeqCst);

        assert_eq!(
            apply_monitor(&mut session, 10_000, now),
            GoalNudgeOutcome::Nudged(GoalNudgeReason::GeneratingNoTokens)
        );
        assert!(session.abort_flag.load(Ordering::SeqCst));
        assert!(session.user_interrupt_flag.load(Ordering::SeqCst));
        assert_eq!(session.runtime.state, SessionState::Idle);
        assert!(session.draft_message.is_none());
        assert_eq!(session.command_queue.len(), 1);
        assert!(matches!(
            session.command_queue.front().unwrap().command,
            ChatCommand::Regenerate {}
        ));
    }

    #[test]
    fn goal_monitor_pending_non_user_command_prevents_double_fire() {
        let (mut session, now) = old_idle_session();
        session.command_queue.push_back(CommandRequest {
            client_request_id: "queued-regenerate".to_string(),
            priority: true,
            command: ChatCommand::Regenerate {},
        });

        assert_eq!(
            apply_monitor(&mut session, 10_000, now),
            GoalNudgeOutcome::Skipped(GoalNudgeSkip::PendingCommand)
        );
        assert_eq!(session.command_queue.len(), 1);
    }

    #[test]
    fn goal_monitor_turn_end_budget_exhaustion_blocks_completion() {
        assert!(goal_turn_end_blocks_completion(
            GoalNudgeOutcome::BudgetExhausted(GoalStatus::BudgetExhausted,)
        ));
        assert!(goal_turn_end_blocks_completion(
            GoalNudgeOutcome::BudgetExhausted(GoalStatus::NoProgress,)
        ));
        assert!(goal_turn_end_blocks_completion(GoalNudgeOutcome::Nudged(
            GoalNudgeReason::TurnEnd,
        )));
        assert!(!goal_turn_end_blocks_completion(GoalNudgeOutcome::Skipped(
            GoalNudgeSkip::NoGoal,
        )));
    }

    #[test]
    fn goal_monitor_turn_end_skip_without_work_does_not_block_completion() {
        let (mut session, now) = old_idle_session();
        assert_eq!(
            apply_monitor(&mut session, 10_000, now),
            GoalNudgeOutcome::Nudged(GoalNudgeReason::Idle)
        );
        let cooldown_outcome = try_apply_goal_nudge(
            &mut session,
            GoalNudgeTrigger::InlineTurnEnd,
            10_100,
            now + Duration::from_secs(1),
            config(),
        );
        assert_eq!(
            cooldown_outcome,
            GoalNudgeOutcome::Skipped(GoalNudgeSkip::Cooldown)
        );
        assert!(!goal_turn_end_blocks_completion(cooldown_outcome));
        for skip in [
            GoalNudgeSkip::Busy,
            GoalNudgeSkip::PendingCommand,
            GoalNudgeSkip::UserMessageQueued,
            GoalNudgeSkip::WaitingForInput,
            GoalNudgeSkip::QueueRejected,
        ] {
            assert!(!goal_turn_end_blocks_completion(GoalNudgeOutcome::Skipped(
                skip,
            )));
        }
    }

    #[test]
    fn goal_monitor_records_terminal_event_when_turn_end_progress_exhausted_budget() {
        let mut session = active_goal_session();
        session.goal.as_mut().unwrap().status = GoalStatus::BudgetExhausted;

        assert_eq!(
            record_terminal_goal_event_if_needed(
                &mut session,
                GoalNudgeTrigger::InlineTurnEnd,
                10_000,
            ),
            Some(true)
        );
        assert_eq!(
            record_terminal_goal_event_if_needed(
                &mut session,
                GoalNudgeTrigger::InlineTurnEnd,
                10_100,
            ),
            Some(false)
        );
        assert_eq!(session.goal_status, Some(GoalStatus::BudgetExhausted));
        let terminal_events = session
            .messages
            .iter()
            .filter(|message| {
                message.role == "event"
                    && event_payload(message).get("kind") == Some(&json!("budget_exhausted"))
            })
            .count();
        assert_eq!(terminal_events, 1);
    }

    #[test]
    fn goal_monitor_event_before_assistant_marks_next_usage_for_accounting() {
        let (mut session, now) = old_idle_session();
        assert_eq!(
            apply_monitor(&mut session, 10_000, now),
            GoalNudgeOutcome::Nudged(GoalNudgeReason::Idle)
        );
        session.add_message(ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("worked".to_string()),
            usage: Some(ChatUsage {
                prompt_tokens: 10,
                completion_tokens: 60,
                total_tokens: 70,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                metering_usd: None,
            }),
            ..Default::default()
        });
        let assistant_index = session
            .messages
            .iter()
            .rposition(|message| message.role == "assistant")
            .unwrap();
        let marked_before_assistant = session.messages[..assistant_index]
            .iter()
            .rev()
            .take_while(|message| message.role != "assistant")
            .any(|message| {
                message.role == "event"
                    && message
                        .extra
                        .get("event")
                        .and_then(|event| event.get("subkind"))
                        .and_then(|subkind| subkind.as_str())
                        == Some("goal_pursuit")
                    && message
                        .extra
                        .get("event")
                        .and_then(|event| event.get("payload"))
                        .and_then(|payload| payload.get("account_progress"))
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
            });
        assert!(marked_before_assistant);
    }
}
