// Task agent failure detection and automatic cleanup
//
// This module monitors task agents and automatically marks them as failed when:
// - Streaming errors occur (network, model, timeout)
// - Agent becomes stuck (no activity beyond threshold)
// - Session ends in Error state without calling agent_finish

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use chrono::Utc;
use serde_json::json;
use tokio::time::sleep;

use crate::app_state::AppState;
use crate::tasks::storage;
use crate::tasks::types::{BoardCard, StatusUpdate};
use crate::call_validation::{ChatMessage, ChatUsage};
use crate::chat::internal_roles::{event, EventSubkind};
use crate::chat::retry_policy::{
    RetryDecision, UserErrorCategory, classify_llm_error_for_retry, classify_user_error,
    user_error_info,
};
use crate::chat::types::{ChatSession, SessionState, TaskMeta};
use crate::chat::{get_or_create_session_with_trajectory, process_command_queue};
use crate::chat::types::{CommandRequest, ChatCommand, EnqueueCommandOutcome};
use crate::worktrees::service::WorktreeService;
use crate::worktrees::git;
use refact_buddy_core::types::BuddyRuntimeEvent;
use uuid::Uuid;

/// Timeout for agent inactivity before considering it stuck (20 minutes)
const AGENT_STUCK_TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// How often to check for stuck agents (2 minutes)
const MONITOR_INTERVAL: Duration = Duration::from_secs(2 * 60);

/// Timeout for in-flight stream stall (Generating with no token activity)
const STREAM_STALL_TIMEOUT: Duration = Duration::from_secs(4 * 60);

const TOOL_STALL_TIMEOUT: Duration = Duration::from_secs(8 * 60);

const MAX_STALL_PLANNER_NOTIFICATIONS_PER_CARD: usize = 2;
const STALL_PLANNER_NOTIFY_GRACE: Duration = Duration::from_secs(60);
const STALL_PLANNER_NOTIFY_COOLDOWN_SECONDS: i64 = 5 * 60;
const STALL_PLANNER_NOTIFY_STATUS_PREFIX: &str = "Planner notified about stall:";
const TASK_AGENT_MONITOR_SOURCE: &str = "chat.task_agent_monitor";

fn task_agent_monitor_notice(payload: serde_json::Value, content: String) -> ChatMessage {
    event(
        EventSubkind::SystemNotice,
        TASK_AGENT_MONITOR_SOURCE,
        payload,
        content,
    )
}

fn regenerate_request(client_request_prefix: &str) -> CommandRequest {
    CommandRequest {
        client_request_id: format!("{}-{}", client_request_prefix, uuid::Uuid::new_v4()),
        priority: true,
        command: ChatCommand::Regenerate {},
    }
}

fn make_runtime_event(
    signal_type: &str,
    title: &str,
    source: &str,
    dedupe_key: &str,
    status: &str,
    priority: Option<&str>,
) -> BuddyRuntimeEvent {
    BuddyRuntimeEvent {
        id: Uuid::new_v4().to_string(),
        signal_type: signal_type.to_string(),
        title: title.to_string(),
        description: None,
        source: source.to_string(),
        status: status.to_string(),
        failure_category: None,
        failure_summary: None,
        progress: None,
        dedupe_key: Some(dedupe_key.to_string()),
        priority: priority.unwrap_or("normal").to_string(),
        created_at: Utc::now().to_rfc3339(),
        ttl_ms: None,
        bubble_policy: None,
        speech_text: None,
        scene: None,
        duration_hint: None,
        persistent: false,
        controls: Vec::new(),
        chat_id: None,
        dismissed: false,
    }
}

fn stall_planner_notifications(card: &BoardCard) -> (usize, Option<chrono::DateTime<Utc>>) {
    let mut count = 0usize;
    let mut latest = None;

    for update in &card.status_updates {
        if !update
            .message
            .starts_with(STALL_PLANNER_NOTIFY_STATUS_PREFIX)
        {
            continue;
        }
        count += 1;
        if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(&update.timestamp) {
            let parsed = parsed.with_timezone(&Utc);
            latest = Some(match latest {
                Some(existing) if existing > parsed => existing,
                _ => parsed,
            });
        }
    }

    (count, latest)
}

fn stall_planner_notify_allowed(card: &BoardCard, now: chrono::DateTime<Utc>) -> bool {
    let (count, latest) = stall_planner_notifications(card);
    if count >= MAX_STALL_PLANNER_NOTIFICATIONS_PER_CARD {
        return false;
    }
    if let Some(latest) = latest {
        let since = now.signed_duration_since(latest).num_seconds();
        if since < STALL_PLANNER_NOTIFY_COOLDOWN_SECONDS {
            return false;
        }
    }
    true
}

fn stalled_agent_should_fail_after_notify_failure(stall_elapsed: Duration) -> bool {
    stall_elapsed >= AGENT_STUCK_TIMEOUT
}

fn linked_agent_session_matches(session: &ChatSession, task_id: &str, card: &BoardCard) -> bool {
    let Some(meta) = session.thread.task_meta.as_ref() else {
        return false;
    };

    if meta.role != "agents" || meta.task_id != task_id {
        return false;
    }
    if meta.card_id.as_deref() != Some(card.id.as_str()) {
        return false;
    }
    if card.assignee.as_deref() != meta.agent_id.as_deref() {
        return false;
    }
    true
}

fn agent_session_idle_stall_ready(session: &ChatSession, task_id: &str, card: &BoardCard) -> bool {
    if card.column != "doing" || card.agent_chat_id.is_none() {
        return false;
    }
    if !linked_agent_session_matches(session, task_id, card) {
        return false;
    }
    if !matches!(
        session.runtime.state,
        SessionState::Idle | SessionState::Completed
    ) {
        return false;
    }
    if session.closed
        || session.draft_message.is_some()
        || session.pending_browser_message.is_some()
        || !session.command_queue.is_empty()
        || !session.runtime.pause_reasons.is_empty()
    {
        return false;
    }
    if session
        .user_interrupt_flag
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        return false;
    }
    session.last_activity.elapsed() >= STALL_PLANNER_NOTIFY_GRACE
}

fn usage_summary(usage: Option<&ChatUsage>) -> String {
    usage
        .map(|usage| {
            let mut parts = vec![
                format!("prompt={}", usage.prompt_tokens),
                format!("completion={}", usage.completion_tokens),
                format!("total={}", usage.total_tokens),
            ];
            if let Some(cache_read) = usage.cache_read_tokens {
                parts.push(format!("cache_read={cache_read}"));
            }
            if let Some(cache_creation) = usage.cache_creation_tokens {
                parts.push(format!("cache_creation={cache_creation}"));
            }
            parts.join(", ")
        })
        .unwrap_or_else(|| "unavailable".to_string())
}

fn reasoning_token_limit_resume_prompt(
    card_id: &str,
    finish_reason: &str,
    usage: Option<&ChatUsage>,
) -> String {
    format!(
        "Agent `{card_id}` stopped while reasoning before producing an answer or tool call. \
Finish reason: `{finish_reason}`. Token usage: {}.\n\n\
Please inspect this agent, compact its chat if needed, then resume it with a short concrete message. \
A good recovery path is: run `agent_pulse(card_id=\"{card_id}\")`, use `agent_steer` with a concise continuation prompt, or cancel/mark the card failed if it cannot continue.",
        usage_summary(usage)
    )
}

async fn notify_planner_about_reasoning_token_limit(
    app: AppState,
    task_id: &str,
    card_id: &str,
    agent_chat_id: &str,
    planner_chat_id: &str,
    prompt: String,
    finish_reason: &str,
    usage: Option<&ChatUsage>,
    message_id: &str,
) -> Result<bool, String> {
    let notice = task_agent_monitor_notice(
        json!({
            "kind": "reasoning_token_limit",
            "task_id": task_id,
            "card_id": card_id,
            "agent_chat_id": agent_chat_id,
            "finish_reason": finish_reason,
            "usage": usage.map(|usage| json!({
                "prompt_tokens": usage.prompt_tokens,
                "completion_tokens": usage.completion_tokens,
                "total_tokens": usage.total_tokens,
                "cache_read_tokens": usage.cache_read_tokens,
                "cache_creation_tokens": usage.cache_creation_tokens,
            })),
            "message_id": message_id,
        }),
        prompt,
    );

    let sessions = app.chat.sessions.clone();
    let planner_session =
        get_or_create_session_with_trajectory(app.clone(), &sessions, planner_chat_id).await;

    {
        let session = planner_session.lock().await;
        if session.thread.task_meta.is_none() {
            tracing::warn!(
                "Cannot notify task planner {}: trajectory is missing or deleted",
                planner_chat_id
            );
            return Ok(false);
        }
    }

    if !record_stall_planner_notification(
        app.clone(),
        task_id,
        card_id,
        agent_chat_id,
        "reasoning_token_limit",
    )
    .await?
    {
        return Ok(false);
    }

    let processor_flag = {
        let mut session = planner_session.lock().await;
        let request = regenerate_request("task-agent-reasoning-token-limit");
        if session.enqueue_priority_command(request) != EnqueueCommandOutcome::Accepted {
            return Ok(false);
        }
        session.add_message(notice);
        session.queue_processor_running.clone()
    };

    if !processor_flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
        tokio::spawn(process_command_queue(
            app.clone(),
            planner_session.clone(),
            processor_flag,
        ));
    }

    Ok(true)
}

pub async fn handle_agent_reasoning_token_limit_stop(
    app: AppState,
    task_meta: TaskMeta,
    finish_reason: String,
    usage: Option<ChatUsage>,
    message_id: String,
    agent_chat_id: String,
) -> Result<(), String> {
    if task_meta.role != "agents" {
        return Ok(());
    }
    let Some(card_id) = task_meta.card_id.clone() else {
        return Ok(());
    };
    let Some(planner_chat_id) = task_meta.planner_chat_id.clone() else {
        return Ok(());
    };

    let prompt = reasoning_token_limit_resume_prompt(&card_id, &finish_reason, usage.as_ref());
    let planner_notified = notify_planner_about_reasoning_token_limit(
        app.clone(),
        &task_meta.task_id,
        &card_id,
        &agent_chat_id,
        &planner_chat_id,
        prompt,
        &finish_reason,
        usage.as_ref(),
        &message_id,
    )
    .await?;

    let notification_status = if planner_notified {
        "Planner notified for compaction/resume."
    } else {
        "Planner notification skipped because the card/session changed or a duplicate notice was already sent."
    };
    let update_message = format!(
        "Agent stopped while reasoning before producing an answer/tool call (finish_reason={finish_reason}, usage={}). {notification_status} message_id={message_id}",
        usage_summary(usage.as_ref())
    );
    let _ = storage::update_board_atomic(app.gcx.clone(), &task_meta.task_id, move |board| {
        if let Some(card) = board.get_card_mut(&card_id) {
            card.status_updates.push(StatusUpdate {
                timestamp: Utc::now().to_rfc3339(),
                message: update_message.clone(),
            });
        }
        Ok(())
    })
    .await;

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StallKind {
    IdleNoFinish,
    Completed,
    GeneratingNoTokens,
    ExecutingToolsNoProgress,
}

impl StallKind {
    fn short_reason(self) -> &'static str {
        match self {
            StallKind::IdleNoFinish => "idle, stream finished without agent_finish",
            StallKind::Completed => "Completed state but card still in doing",
            StallKind::GeneratingNoTokens => "stream appears stalled, no token activity",
            StallKind::ExecutingToolsNoProgress => "tool execution stalled, no tool progress",
        }
    }
}

async fn record_stall_planner_notification(
    app: AppState,
    task_id: &str,
    card_id: &str,
    agent_chat_id: &str,
    reason: &str,
) -> Result<bool, String> {
    let card_id_owned = card_id.to_string();
    let agent_chat_id_owned = agent_chat_id.to_string();
    let now = Utc::now();
    let timestamp = now.to_rfc3339();
    let message = format!(
        "{} {} ({})",
        STALL_PLANNER_NOTIFY_STATUS_PREFIX, agent_chat_id, reason
    );

    storage::update_board_atomic(app.gcx.clone(), task_id, move |board| {
        let card = board
            .get_card_mut(&card_id_owned)
            .ok_or_else(|| format!("Card {} not found", card_id_owned))?;
        if card.column != "doing"
            || card.agent_chat_id.as_deref() != Some(agent_chat_id_owned.as_str())
            || card.assignee.is_none()
            || !stall_planner_notify_allowed(card, now)
        {
            return Ok(false);
        }
        card.last_heartbeat_at = Some(timestamp.clone());
        card.status_updates.push(StatusUpdate {
            timestamp: timestamp.clone(),
            message: message.clone(),
        });
        Ok(true)
    })
    .await
    .map(|(_, recorded)| recorded)
}

fn build_stalled_agent_planner_message(
    card_id: &str,
    card_title: &str,
    agent_chat_id: &str,
    assignee: Option<&str>,
    kind: StallKind,
    stalled_for: Duration,
) -> String {
    let header = match kind {
        StallKind::GeneratingNoTokens => "**Task agent stream stalled.**",
        StallKind::ExecutingToolsNoProgress => "**Task agent tool execution stalled.**",
        StallKind::IdleNoFinish => "**Task agent appears stalled.**",
        StallKind::Completed => {
            "**Task agent reached Completed state but the card is still in `doing`.**"
        }
    };

    let state_line = match kind {
        StallKind::GeneratingNoTokens => format!(
            "State: Generating — no tokens received for {}.",
            humantime::format_duration(stalled_for)
        ),
        StallKind::ExecutingToolsNoProgress => format!(
            "State: ExecutingTools — no tool progress for {}.",
            humantime::format_duration(stalled_for)
        ),
        StallKind::IdleNoFinish => format!(
            "State: Idle — stream finished but `agent_finish` was not called. Idle for {}.",
            humantime::format_duration(stalled_for)
        ),
        StallKind::Completed => format!(
            "State: Completed — finish-like tool reached the agent but the card was not updated. Idle for {}.",
            humantime::format_duration(stalled_for)
        ),
    };

    let context_line = match kind {
        StallKind::GeneratingNoTokens => {
            "The model is connected but is not producing output. This often happens when a reasoning model gets stuck without progress, hits a thinking budget, or the provider stops sending data without an error."
        }
        StallKind::ExecutingToolsNoProgress => {
            "A tool started but has not reported progress for a long time. The tool process may be hung, waiting on input, or otherwise unresponsive."
        }
        StallKind::IdleNoFinish => {
            "The agent stopped producing output without calling `agent_finish`. This often happens when the model hits a token/thinking budget or returns an empty response."
        }
        StallKind::Completed => {
            "The agent emitted a finish-like tool (e.g. `agent_finish`) but the card was not moved out of `doing`. The agent's work may be effectively done already, or the transition may have failed."
        }
    };

    let assignee_line = assignee
        .map(|a| format!(" (assignee: `{}`)", a))
        .unwrap_or_default();

    format!(
        "{}\n\nCard: `{}` ({}){}\nAgent chat: `{}`\n{}\n\n{}\n\nOptions:\n\
1. Send a continue/guidance message to the agent.\n\
2. Restart the agent with `restart_agent(card_id=\"{}\", instructions=\"...\")`.\n\
3. Inspect via `agent_pulse(card_id=\"{}\")` or `board_get(card_id=\"{}\")`.\n\
4. If the task cannot proceed, move the card or mark it accordingly.",
        header,
        card_id,
        card_title,
        assignee_line,
        agent_chat_id,
        state_line,
        context_line,
        card_id,
        card_id,
        card_id,
    )
}

async fn notify_planner_about_stalled_agent(
    app: AppState,
    task_id: &str,
    card: &BoardCard,
    agent_chat_id: &str,
    planner_chat_id: Option<&str>,
    kind: StallKind,
    stalled_for: Duration,
) -> Result<bool, String> {
    let card_id = card.id.as_str();
    let short_reason = kind.short_reason();

    let Some(planner_chat_id) = planner_chat_id else {
        tracing::warn!(
            "Cannot notify planner about stalled agent for card {} in task {}: no planner_chat_id",
            card_id,
            task_id
        );
        return Ok(false);
    };

    let message = build_stalled_agent_planner_message(
        card_id,
        card.title.as_str(),
        agent_chat_id,
        card.assignee.as_deref(),
        kind,
        stalled_for,
    );
    let notice = task_agent_monitor_notice(
        json!({
            "kind": "stalled_agent",
            "task_id": task_id,
            "card_id": card_id,
            "card_title": card.title.as_str(),
            "agent_chat_id": agent_chat_id,
            "assignee": card.assignee.as_deref(),
            "stall_kind": format!("{:?}", kind),
            "reason": short_reason,
            "stalled_for_secs": stalled_for.as_secs(),
        }),
        message,
    );

    let sessions = app.chat.sessions.clone();
    let planner_session =
        get_or_create_session_with_trajectory(app.clone(), &sessions, planner_chat_id).await;

    {
        let session = planner_session.lock().await;
        if session.thread.task_meta.is_none() {
            tracing::warn!(
                "Cannot notify task planner {}: trajectory is missing or deleted",
                planner_chat_id
            );
            return Ok(false);
        }
    }

    if !record_stall_planner_notification(
        app.clone(),
        task_id,
        card_id,
        agent_chat_id,
        short_reason,
    )
    .await?
    {
        return Ok(false);
    }

    let processor_flag = {
        let mut session = planner_session.lock().await;
        let request = regenerate_request("task-agent-stall");
        if session.enqueue_priority_command(request) != EnqueueCommandOutcome::Accepted {
            return Ok(false);
        }
        session.add_message(notice);
        session.queue_processor_running.clone()
    };

    if !processor_flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
        tokio::spawn(process_command_queue(
            app.clone(),
            planner_session.clone(),
            processor_flag,
        ));
    }

    tracing::info!(
        "Notified planner {} about stalled agent for card {} ({}): {}",
        planner_chat_id,
        card_id,
        agent_chat_id,
        short_reason
    );

    Ok(true)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentFailureKind {
    TransientExhausted,
    ContextLimit,
    Permanent,
    Cancelled,
}

impl AgentFailureKind {
    fn from_error(error_message: &str) -> Self {
        match classify_llm_error_for_retry(error_message) {
            RetryDecision::Retry { .. } => AgentFailureKind::TransientExhausted,
            RetryDecision::ContextLimit { .. } => AgentFailureKind::ContextLimit,
            RetryDecision::DoNotRetry { .. } => AgentFailureKind::Permanent,
            RetryDecision::UserCancelled { .. } => AgentFailureKind::Cancelled,
        }
    }

    fn should_cleanup_worktree(self) -> bool {
        matches!(self, AgentFailureKind::Cancelled)
    }

    fn final_report_reason(self, error_message: &str) -> String {
        let category = match self {
            AgentFailureKind::Cancelled => UserErrorCategory::Unknown,
            _ => classify_user_error(error_message),
        };
        let info = user_error_info(category);
        let retry_note = if info.is_retryable {
            "This error is usually retryable."
        } else {
            "This error is not expected to succeed by retrying unchanged."
        };
        let retention_note = match self {
            AgentFailureKind::TransientExhausted => {
                "Retries were exhausted; worktree retained for inspection or retry."
            }
            AgentFailureKind::ContextLimit => {
                "Provider context limit was reached; worktree retained so the task can be retried after compaction or with a smaller history."
            }
            AgentFailureKind::Permanent => "Worktree retained for inspection or retry via restart_agent.",
            AgentFailureKind::Cancelled => "The task was cancelled before the agent finished.",
        };

        format!(
            "{}\n\n{}\n\nSuggested action: {}\n{}\n{}\n\nRaw error: {}",
            info.title,
            info.explanation,
            format_user_error_action(info.suggested_action),
            retry_note,
            retention_note,
            error_message
        )
    }
}

fn format_user_error_action(action: &str) -> &'static str {
    match action {
        "retry" => "Retry the task or wait for the provider/network to recover.",
        "compact" => {
            "Compact the chat, reduce attached context, or switch to a larger-context model."
        }
        "check_auth" => {
            "Check provider credentials, OAuth login, token scope, and provider configuration."
        }
        "switch_model" => "Select an available model or update the provider route configuration.",
        "check_billing" => "Check provider billing, credits, quota, and usage limits.",
        _ => "Review the raw provider error and adjust the request or task before retrying.",
    }
}

/// Detect if a session error should cause task agent failure
pub async fn handle_agent_streaming_error(
    app: AppState,
    task_meta: &TaskMeta,
    error_message: &str,
) {
    let Some(ref card_id) = task_meta.card_id else {
        tracing::warn!("Agent has no card_id in task_meta, cannot mark as failed");
        return;
    };

    tracing::error!(
        "Task agent streaming error detected for card {}: {}",
        card_id,
        error_message
    );

    let failure_kind = AgentFailureKind::from_error(error_message);
    let failure_reason = failure_kind.final_report_reason(error_message);

    if let Err(e) = mark_agent_as_failed(
        app.clone(),
        &task_meta.task_id,
        card_id,
        task_meta.agent_id.as_deref(),
        task_meta.planner_chat_id.as_deref(),
        &failure_reason,
        failure_kind,
    )
    .await
    {
        tracing::error!("Failed to mark agent as failed: {}", e);
    }
}

/// Mark a task card as failed and notify planner
async fn mark_agent_as_failed(
    app: AppState,
    task_id: &str,
    card_id: &str,
    expected_agent_id: Option<&str>,
    planner_chat_id: Option<&str>,
    reason: &str,
    failure_kind: AgentFailureKind,
) -> Result<(), String> {
    let _ = update_card_heartbeat(app.clone(), task_id, card_id).await;

    let card_id_owned = card_id.to_string();
    let reason_clone = reason.to_string();
    let expected_agent_id_owned = expected_agent_id.map(|s| s.to_string());

    // The closure returns (card_title, actually_failed, all_finished).
    // actually_failed is true only when the card transitioned from "doing" to "failed".
    let (board, (card_title, actually_failed, all_finished)) =
        storage::update_board_atomic(app.gcx.clone(), task_id, move |board| {
            let card = board
                .get_card_mut(&card_id_owned)
                .ok_or(format!("Card {} not found", card_id_owned))?;

            if card.column == "done" || card.column == "failed" {
                let card_title = card.title.clone();
                return Ok((card_title, false, false));
            }

            if card.column != "doing" {
                let card_title = card.title.clone();
                return Ok((card_title, false, false));
            }

            if let Some(ref expected_id) = expected_agent_id_owned {
                if card.assignee.as_ref() != Some(expected_id) {
                    tracing::warn!(
                        "Card {} assignee mismatch: expected {}, got {:?}. Skipping auto-fail.",
                        card_id_owned,
                        expected_id,
                        card.assignee
                    );
                    let card_title = card.title.clone();
                    return Ok((card_title, false, false));
                }
            }

            let card_title = card.title.clone();

            card.final_report = Some(format!("FAILED (automatic): {}", reason_clone));
            card.final_report_structured = None;
            card.column = "failed".to_string();
            card.completed_at = Some(Utc::now().to_rfc3339());
            card.status_updates.push(StatusUpdate {
                timestamp: Utc::now().to_rfc3339(),
                message: format!("Automatic failure detection: {}", reason_clone),
            });

            let _ = card;

            let agents_active_after = storage::active_agent_count(&board.cards);
            let all_finished = agents_active_after == 0;

            Ok((card_title, true, all_finished))
        })
        .await?;

    if !actually_failed {
        return Ok(());
    }

    storage::update_task_stats(app.gcx.clone(), task_id).await?;

    tracing::info!("Marked agent for card {} as failed: {}", card_id, reason);

    {
        let ev = make_runtime_event(
            "task_failed",
            &format!("Agent failed: {}", card_title),
            "task_agent",
            &format!("task_agent_{}", card_id),
            "failed",
            Some("high"),
        );
        app.buddy_event_sink.enqueue_event(ev).await;
    }

    {
        let (worktree, branch, worktree_name) = if let Some(card) = board.get_card(card_id) {
            (
                card.agent_worktree.clone(),
                card.agent_branch.clone(),
                card.agent_worktree_name.clone(),
            )
        } else {
            (None, None, None)
        };
        let diff_report = if let (Some(ref wt), Some(ref br)) = (&worktree, &branch) {
            capture_failed_agent_diff(app.clone(), wt, br, worktree_name.as_deref()).await
        } else {
            String::new()
        };
        let cleanup_result = if failure_kind.should_cleanup_worktree() {
            if let (Some(ref wt), Some(ref br)) = (&worktree, &branch) {
                let result = remove_agent_worktree_and_branch(
                    app.clone(),
                    task_id,
                    card_id,
                    wt,
                    br,
                    worktree_name.as_deref(),
                )
                .await;
                tracing::info!(
                    "Automatic cleanup for failed agent card {}: worktree_removed={}, branch_deleted={}",
                    card_id,
                    result.0,
                    result.1
                );
                Some(result)
            } else {
                Some((false, false))
            }
        } else {
            None
        };
        let retention_or_cleanup_note = match cleanup_result {
            Some((worktree_removed, branch_deleted)) => {
                let worktree_note = if worktree_removed {
                    "worktree removed"
                } else if worktree.is_some() {
                    "worktree removal not confirmed"
                } else {
                    "no worktree recorded"
                };
                let branch_note = if branch_deleted {
                    "branch deleted"
                } else if branch.is_some() {
                    "branch deletion not confirmed"
                } else {
                    "no branch recorded"
                };
                format!(
                    "\n\nAutomatic cleanup attempted: {}; {}.",
                    worktree_note, branch_note
                )
            }
            None => "\n\nWorktree and branch retained for inspection or retry via `restart_agent`."
                .to_string(),
        };
        let card_id_for_report = card_id.to_string();
        let clear_worktree_fields = cleanup_result
            .map(|(worktree_removed, _)| worktree_removed)
            .unwrap_or(false);
        let clear_branch_field = cleanup_result
            .map(|(_, branch_deleted)| branch_deleted)
            .unwrap_or(false);
        if let Err(e) = storage::update_board_atomic(app.gcx.clone(), task_id, move |board| {
            if let Some(c) = board.get_card_mut(&card_id_for_report) {
                if let Some(ref mut report) = c.final_report {
                    if !diff_report.is_empty() {
                        report.push_str(&diff_report);
                    }
                    report.push_str(&retention_or_cleanup_note);
                }
                if clear_worktree_fields {
                    c.agent_worktree = None;
                    c.agent_worktree_name = None;
                }
                if clear_branch_field {
                    c.agent_branch = None;
                }
            } else {
                tracing::warn!(
                    "Cannot update final report for failed agent card {}: card disappeared",
                    card_id_for_report
                );
            }
            Ok(())
        })
        .await
        {
            tracing::warn!(
                "Failed to update final report for failed agent card {}: {}",
                card_id,
                e
            );
        }
    }

    let notification_board = storage::load_board(app.gcx.clone(), task_id)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(
                "Failed to reload task board {} before planner notification: {}",
                task_id,
                e
            );
            board.clone()
        });

    if let Err(e) = notify_planner_agents_finished(
        app.clone(),
        task_id,
        &notification_board,
        all_finished,
        planner_chat_id,
    )
    .await
    {
        tracing::warn!(
            "Marked agent for card {} as failed, but planner notification failed: {}",
            card_id,
            e
        );
    }

    Ok(())
}

/// Notify planner about newly finished agents without waiting for the full batch to end.
pub(crate) async fn notify_planner_agents_finished(
    app: AppState,
    task_id: &str,
    board: &crate::tasks::types::TaskBoard,
    all_finished: bool,
    planner_chat_id: Option<&str>,
) -> Result<(), String> {
    let since = match storage::load_task_meta(app.gcx.clone(), task_id).await {
        Ok(meta) => meta
            .last_agents_summary_at
            .as_deref()
            .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
            .map(|dt| dt.with_timezone(&Utc)),
        Err(_) => None,
    };

    let mut results = Vec::new();
    for card in &board.cards {
        if let Some(variants) = card.ab_variants.as_ref() {
            for (variant_key, variant) in [("a", &variants.a), ("b", &variants.b)] {
                let Some(finish) = variant.finish.as_ref() else {
                    continue;
                };
                if let Some(ref since_dt) = since {
                    let Some(completed_at) =
                        chrono::DateTime::parse_from_rfc3339(&finish.completed_at)
                            .ok()
                            .map(|dt| dt.with_timezone(&Utc))
                    else {
                        continue;
                    };
                    if completed_at < *since_dt {
                        continue;
                    }
                }
                let status = if finish.success {
                    "✅ done"
                } else {
                    "❌ failed"
                };
                let report_preview: String = finish.final_report.chars().take(200).collect();
                results.push(format!(
                    "**{}/{} ({} [A/B {}])**: {}\n{}",
                    card.id,
                    variant_key,
                    card.title,
                    variant_key.to_ascii_uppercase(),
                    status,
                    report_preview
                ));
            }
            if card.agent_chat_id.is_none() {
                continue;
            }
        }
        if card.agent_chat_id.is_none() {
            continue;
        }
        if let Some(ref since_dt) = since {
            let Some(completed_at) = card
                .completed_at
                .as_deref()
                .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
                .map(|dt| dt.with_timezone(&Utc))
            else {
                continue;
            };
            if completed_at < *since_dt {
                continue;
            }
        }
        let status = if card.column == "done" {
            "✅ done"
        } else if card.column == "failed" {
            "❌ failed"
        } else {
            continue;
        };
        let report_preview: String = card
            .final_report
            .as_deref()
            .unwrap_or("")
            .chars()
            .take(200)
            .collect();
        results.push(format!(
            "**{} ({})**: {}\n{}",
            card.id, card.title, status, report_preview
        ));
    }

    let heading = if all_finished {
        "**All agents have completed.**"
    } else {
        "**Task agent finished.**"
    };
    let footer = if all_finished {
        "Run `board_get(card_id)` to see full details for any card."
    } else {
        "Other agents may still be running. Run `check_agents` to see live status or `board_get(card_id)` for full details."
    };

    let planner_message = format!(
        "{}\n\n{}\n\n{}",
        heading,
        if results.is_empty() {
            let note = since
                .as_ref()
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_else(|| "(unknown)".to_string());
            format!("_(No newly-finished cards detected since {}.)_", note)
        } else {
            results.join("\n\n")
        },
        footer
    );
    let notice = task_agent_monitor_notice(
        json!({
            "kind": "agents_finished",
            "task_id": task_id,
            "all_finished": all_finished,
            "results": results,
            "since": since.as_ref().map(|dt| dt.to_rfc3339()),
        }),
        planner_message,
    );

    let sessions = app.chat.sessions.clone();

    let planner_chat_id = if let Some(id) = planner_chat_id {
        id.to_string()
    } else {
        let agent_session_arcs = {
            let sessions_guard = sessions.read().await;
            board
                .cards
                .iter()
                .filter(|card| card.column == "done" || card.column == "failed")
                .filter_map(|card| card.agent_chat_id.as_deref())
                .filter_map(|agent_chat_id| sessions_guard.get(agent_chat_id).cloned())
                .collect::<Vec<_>>()
        };
        let mut planner_chat_id = None;
        for session_arc in agent_session_arcs {
            let session = session_arc.lock().await;
            planner_chat_id = session
                .thread
                .task_meta
                .as_ref()
                .and_then(|meta| meta.planner_chat_id.clone());
            if planner_chat_id.is_some() {
                break;
            }
        }
        planner_chat_id.ok_or_else(|| {
            format!(
                "Cannot notify task planner for task {}: finished agent has no planner_chat_id",
                task_id
            )
        })?
    };
    let planner_session =
        get_or_create_session_with_trajectory(app.clone(), &sessions, &planner_chat_id).await;
    {
        let session = planner_session.lock().await;
        if session.thread.task_meta.is_none() {
            tracing::warn!(
                "Cannot notify task planner {}: trajectory is missing or deleted",
                planner_chat_id
            );
            return Ok(());
        }
    }

    let processor_flag = {
        let mut session = planner_session.lock().await;
        let request = regenerate_request("task-agent-finished");
        if session.enqueue_priority_command(request) != EnqueueCommandOutcome::Accepted {
            return Ok(());
        }
        session.add_message(notice);
        session.queue_processor_running.clone()
    };

    if !processor_flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
        tokio::spawn(process_command_queue(
            app.clone(),
            planner_session.clone(),
            processor_flag,
        ));
    }

    // Best-effort: mark summary as emitted.
    if let Ok(mut meta) = storage::load_task_meta(app.gcx.clone(), task_id).await {
        meta.last_agents_summary_at = Some(Utc::now().to_rfc3339());
        let _ = storage::save_task_meta(app.gcx.clone(), task_id, &meta).await;
    }

    Ok(())
}

fn expected_agent_branch_prefix(task_id: &str, card_id: &str) -> String {
    format!("refact/task/{}/card/{}/", task_id, card_id)
}

fn validate_agent_branch_for_cleanup(
    task_id: &str,
    card_id: &str,
    branch: &str,
) -> Result<(), String> {
    let expected_prefix = expected_agent_branch_prefix(task_id, card_id);
    if branch.starts_with(&expected_prefix) {
        Ok(())
    } else {
        Err(format!(
            "Refusing to delete unsafe branch '{}'; expected prefix '{}'.",
            branch, expected_prefix
        ))
    }
}

fn canonical_existing_path(path: &Path) -> Result<PathBuf, String> {
    dunce::canonicalize(path)
        .map_err(|e| format!("Failed to canonicalize '{}': {}", path.display(), e))
}

fn validate_agent_worktree_path_for_cleanup(
    cache_dir: &Path,
    workspace_roots: &[PathBuf],
    worktree: &Path,
) -> Result<PathBuf, String> {
    let worktree = canonical_existing_path(worktree)?;
    let cache_dir = canonical_existing_path(cache_dir)?;
    let cache_root = cache_dir.join("worktrees");
    let cache_root = if cache_root.exists() {
        canonical_existing_path(&cache_root)?
    } else {
        return Err(format!(
            "Refusing to delete worktree path '{}': worktree cache '{}' does not exist.",
            worktree.display(),
            cache_root.display()
        ));
    };

    if worktree == Path::new("/") || worktree == cache_dir || worktree == cache_root {
        return Err(format!(
            "Refusing to delete unsafe worktree path '{}'.",
            worktree.display()
        ));
    }

    for workspace_root in workspace_roots {
        if let Ok(workspace_root) = canonical_existing_path(workspace_root) {
            if worktree == workspace_root {
                return Err(format!(
                    "Refusing to delete workspace root '{}'.",
                    worktree.display()
                ));
            }
        }
    }

    if !worktree.starts_with(&cache_root) {
        return Err(format!(
            "Refusing to delete worktree path '{}' outside Refact worktree cache '{}'.",
            worktree.display(),
            cache_root.display()
        ));
    }

    Ok(worktree)
}

fn fallback_remove_worktree_and_branch(
    cache_dir: PathBuf,
    workspace_root: PathBuf,
    workspace_roots: Vec<PathBuf>,
    task_id: String,
    card_id: String,
    agent_worktree: String,
    agent_branch: String,
    remove_worktree: bool,
    remove_branch: bool,
) -> (bool, bool) {
    let branch_is_safe = match validate_agent_branch_for_cleanup(&task_id, &card_id, &agent_branch)
    {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("Skipping fallback agent branch cleanup: {}", e);
            false
        }
    };

    let mut worktree_removed = false;
    if remove_worktree {
        match validate_agent_worktree_path_for_cleanup(
            &cache_dir,
            &workspace_roots,
            Path::new(&agent_worktree),
        ) {
            Ok(worktree) => {
                worktree_removed = !worktree.exists();
                if !worktree_removed {
                    let warnings = git::remove_worktree_path(&workspace_root, &worktree);
                    for warning in warnings {
                        tracing::warn!("Fallback agent worktree cleanup warning: {}", warning);
                    }
                    worktree_removed = !worktree.exists();
                }
            }
            Err(e) => tracing::warn!("Skipping fallback agent worktree cleanup: {}", e),
        }
    }

    let mut branch_deleted = false;
    if remove_branch && branch_is_safe {
        match git::delete_branch(&workspace_root, &agent_branch) {
            Ok(deleted) => branch_deleted = deleted,
            Err(e) => tracing::warn!("Fallback agent branch cleanup failed: {}", e),
        }
    }

    (worktree_removed, branch_deleted)
}

pub(crate) async fn remove_agent_worktree_and_branch(
    app: AppState,
    task_id: &str,
    card_id: &str,
    agent_worktree: &str,
    agent_branch: &str,
    agent_worktree_name: Option<&str>,
) -> (bool, bool) {
    let cache_dir = app.paths.cache_dir.clone();
    let project_dirs = crate::files_correction::get_project_dirs(app.gcx.clone()).await;
    let Some(workspace_root) = project_dirs.first().cloned() else {
        tracing::warn!(
            "Cannot cleanup failed agent worktree for card {}: no workspace folder found",
            card_id
        );
        return (false, false);
    };

    let mut worktree_removed = !Path::new(agent_worktree).exists();
    let mut branch_deleted = false;

    if let Some(worktree_id) = agent_worktree_name {
        if let Ok(service) = WorktreeService::new(cache_dir.clone(), workspace_root.clone()) {
            match service.delete_worktree(worktree_id, true, true).await {
                Ok(deleted) => {
                    worktree_removed = deleted.deleted && !Path::new(agent_worktree).exists();
                    branch_deleted = deleted.branch_deleted;
                    for warning in deleted.warnings {
                        tracing::warn!(
                            "Registered agent worktree cleanup warning for '{}': {}",
                            worktree_id,
                            warning
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to delete registered worktree '{}': {}",
                        worktree_id,
                        e
                    );
                }
            }
        }
    }

    if worktree_removed && branch_deleted {
        return (true, true);
    }

    let remove_worktree = !worktree_removed;
    let remove_branch = !branch_deleted;
    let task_id = task_id.to_string();
    let card_id = card_id.to_string();
    let agent_worktree = agent_worktree.to_string();
    let agent_branch = agent_branch.to_string();
    tokio::task::spawn_blocking(move || {
        fallback_remove_worktree_and_branch(
            cache_dir,
            workspace_root,
            project_dirs,
            task_id,
            card_id,
            agent_worktree,
            agent_branch,
            remove_worktree,
            remove_branch,
        )
    })
    .await
    .map(|(fallback_worktree_removed, fallback_branch_deleted)| {
        (
            worktree_removed || fallback_worktree_removed,
            branch_deleted || fallback_branch_deleted,
        )
    })
    .unwrap_or_else(|e| {
        tracing::warn!("Fallback agent cleanup task failed: {}", e);
        (worktree_removed, branch_deleted)
    })
}

pub(crate) async fn capture_failed_agent_diff(
    _app: AppState,
    agent_worktree: &str,
    agent_branch: &str,
    _agent_worktree_name: Option<&str>,
) -> String {
    let worktree_path = Path::new(agent_worktree);
    let mut diff_report = String::new();

    if worktree_path.exists() {
        let uncommitted_diff = Command::new("git")
            .args(["diff", "HEAD"])
            .current_dir(worktree_path)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();

        let committed_diff = Command::new("git")
            .args(["log", "--patch", "--reverse", "HEAD@{upstream}..HEAD"])
            .current_dir(worktree_path)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();

        let committed_diff = if committed_diff.is_empty() {
            Command::new("git")
                .args(["diff", &format!("{}~1..HEAD", agent_branch)])
                .current_dir(worktree_path)
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default()
        } else {
            committed_diff
        };

        if !committed_diff.is_empty() || !uncommitted_diff.is_empty() {
            diff_report.push_str("\n\n## Changes made before failure\n\n");
            if !committed_diff.is_empty() {
                diff_report.push_str("### Committed changes\n```diff\n");
                let truncated: String = committed_diff.chars().take(2000).collect();
                diff_report.push_str(&truncated);
                if committed_diff.len() > 2000 {
                    diff_report.push_str("\n... (truncated)");
                }
                diff_report.push_str("\n```\n");
            }
            if !uncommitted_diff.is_empty() {
                diff_report.push_str("### Uncommitted changes\n```diff\n");
                let truncated: String = uncommitted_diff.chars().take(2000).collect();
                diff_report.push_str(&truncated);
                if uncommitted_diff.len() > 2000 {
                    diff_report.push_str("\n... (truncated)");
                }
                diff_report.push_str("\n```\n");
            }
        }
    }

    diff_report
}

pub async fn update_card_heartbeat(
    app: AppState,
    task_id: &str,
    card_id: &str,
) -> Result<(), String> {
    let card_id_owned = card_id.to_string();
    let heartbeat = Utc::now().to_rfc3339();
    storage::update_board_atomic(app.gcx.clone(), task_id, move |board| {
        let card = board
            .get_card_mut(&card_id_owned)
            .ok_or_else(|| format!("Card {} not found", card_id_owned))?;
        card.last_heartbeat_at = Some(heartbeat.clone());
        Ok(())
    })
    .await
    .map(|_| ())
}

pub fn git_diff_name_only(worktree_path: &Path, base_ref: &str, head_ref: &str) -> Vec<String> {
    let range = format!("{}..{}", base_ref, head_ref);
    let output = Command::new("git")
        .args(["diff", "--name-only", &range])
        .current_dir(worktree_path)
        .output();
    match output {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        Ok(output) => {
            tracing::warn!(
                "git diff --name-only failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            vec![]
        }
        Err(err) => {
            tracing::warn!("git diff --name-only failed: {}", err);
            vec![]
        }
    }
}

pub async fn append_card_target_files(
    app: AppState,
    task_id: &str,
    card_id: &str,
    files: Vec<String>,
) -> Result<(), String> {
    if files.is_empty() {
        return Ok(());
    }
    let card_id_owned = card_id.to_string();
    storage::update_board_atomic(app.gcx.clone(), task_id, move |board| {
        let card = board
            .get_card_mut(&card_id_owned)
            .ok_or_else(|| format!("Card {} not found", card_id_owned))?;
        for file in &files {
            if !card.target_files.contains(file) {
                card.target_files.push(file.clone());
            }
        }
        Ok(())
    })
    .await
    .map(|_| ())
}

/// Return the wall-clock timestamp of the last recorded activity for the
/// given agent chat session. Returns `None` when the session no longer exists.
pub async fn get_last_agent_heartbeat(
    app: AppState,
    agent_chat_id: &str,
) -> Option<chrono::DateTime<Utc>> {
    let sessions = app.chat.sessions.clone();
    let sessions_read = sessions.read().await;
    let session_arc = sessions_read.get(agent_chat_id)?.clone();
    drop(sessions_read);
    let session = session_arc.lock().await;
    let elapsed = session.last_activity.elapsed();
    drop(session);
    let heartbeat = Utc::now() - chrono::Duration::from_std(elapsed).ok()?;
    Some(heartbeat)
}

/// Background task that monitors for stuck agents
pub async fn start_agent_monitor(app: AppState) {
    tracing::info!("Starting task agent monitor");

    loop {
        let shutdown_flag = app.runtime.shutdown_flag.clone();
        tokio::select! {
            _ = sleep(MONITOR_INTERVAL) => {}
            _ = async {
                while !shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                }
            } => {
                tracing::info!("Task agent monitor: shutdown detected, stopping");
                return;
            }
        }

        if let Err(e) = check_for_stuck_agents(app.clone()).await {
            tracing::error!("Agent monitor error: {}", e);
        }
    }
}

async fn sweep_planner_wake_ups(app: AppState) -> Result<(), String> {
    let now = Utc::now();
    let task_metas = storage::list_tasks(app.gcx.clone()).await?;

    for task_meta in task_metas {
        if task_meta.status != crate::tasks::types::TaskStatus::Active {
            continue;
        }
        let task_id = task_meta.id.clone();

        let all_session_arcs: Vec<_> = {
            let sessions_read = app.chat.sessions.read().await;
            sessions_read.values().cloned().collect()
        };

        for session_arc in all_session_arcs {
            let (should_wake, chat_id) = {
                let session = session_arc.lock().await;
                let is_planner_for_task = session
                    .thread
                    .task_meta
                    .as_ref()
                    .map(|m| m.role == "planner" && m.task_id == task_id)
                    .unwrap_or(false);
                if !is_planner_for_task {
                    (false, String::new())
                } else {
                    let is_waiting = session.runtime.state == SessionState::WaitingUserInput;
                    let past_deadline = session.wake_up_at.map(|t| now >= t).unwrap_or(false);
                    let chat_id = session.chat_id.clone();
                    (is_waiting && past_deadline, chat_id)
                }
            };

            if !should_wake {
                continue;
            }

            {
                let mut session = session_arc.lock().await;
                session.wake_up_at = None;
                session.mark_persisted_runtime_changed();
            }

            let statuses = crate::tools::tool_task_check_agents::get_agent_statuses(
                app.gcx.clone(),
                app.chat.facade.clone(),
                &task_id,
            )
            .await
            .unwrap_or_default();

            let empty_args: std::collections::HashMap<String, serde_json::Value> =
                std::collections::HashMap::new();
            let query = crate::tools::tool_task_check_agents::parse_agent_status_query(&empty_args)
                .unwrap_or_else(|_| {
                    crate::tools::tool_task_check_agents::parse_agent_status_query(
                        &std::collections::HashMap::new(),
                    )
                    .unwrap()
                });
            let status_text =
                crate::tools::tool_task_check_agents::format_agent_statuses(&statuses, &query)
                    .unwrap_or_else(|_| "(status unavailable)".to_string());

            let message = format!(
                "[AUTO WAKE] Your requested wait window expired. Current agent status:\n\n{}\n\nDecide whether to continue waiting, merge ready cards, or move on.",
                status_text
            );
            let notice = task_agent_monitor_notice(
                json!({
                    "kind": "planner_auto_wake",
                    "task_id": task_id,
                    "planner_chat_id": chat_id,
                    "status_text": status_text,
                }),
                message,
            );

            let processor_flag = {
                let mut session = session_arc.lock().await;
                let request = regenerate_request("planner-wake-up");
                if session.enqueue_priority_command(request) != EnqueueCommandOutcome::Accepted {
                    continue;
                }
                session.add_message(notice);
                session.queue_processor_running.clone()
            };

            if !processor_flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
                tokio::spawn(process_command_queue(
                    app.clone(),
                    session_arc.clone(),
                    processor_flag,
                ));
            }

            tracing::info!(
                "Auto-woke planner {} for task {} (wake_up_at deadline passed)",
                chat_id,
                task_id
            );
        }
    }

    Ok(())
}

/// Check all active tasks for stuck agents
async fn check_for_stuck_agents(app: AppState) -> Result<(), String> {
    let task_metas = storage::list_tasks(app.gcx.clone()).await?;

    for task_meta in task_metas {
        if task_meta.status != crate::tasks::types::TaskStatus::Active {
            continue;
        }

        let task_id = &task_meta.id;
        let board = storage::load_board(app.gcx.clone(), task_id).await?;
        let sessions = app.chat.sessions.clone();

        for card in &board.cards {
            if card.column != "doing" || card.assignee.is_none() {
                continue;
            }
            if card.ab_variants.is_some() && card.agent_chat_id.is_none() {
                continue;
            }

            let last_activity_timestamp = card
                .status_updates
                .last()
                .map(|u| u.timestamp.as_str())
                .or(card.started_at.as_deref())
                .unwrap_or(&card.created_at);

            let agent_chat_id = match &card.agent_chat_id {
                Some(id) => id,
                None => {
                    if let Ok(last_time) =
                        chrono::DateTime::parse_from_rfc3339(last_activity_timestamp)
                    {
                        let elapsed =
                            Utc::now().signed_duration_since(last_time.with_timezone(&Utc));
                        if elapsed.num_seconds() as u64 > AGENT_STUCK_TIMEOUT.as_secs() {
                            tracing::warn!(
                                "Agent for card {} has no agent_chat_id but is doing, stuck for {} ago",
                                card.id,
                                humantime::format_duration(Duration::from_secs(elapsed.num_seconds() as u64))
                            );

                            mark_agent_as_failed(
                                app.clone(),
                                task_id,
                                &card.id,
                                card.assignee.as_deref(),
                                None,
                                &format!(
                                    "Agent appears stuck (no agent_chat_id, no activity for {})",
                                    humantime::format_duration(AGENT_STUCK_TIMEOUT)
                                ),
                                AgentFailureKind::Permanent,
                            )
                            .await?;
                        }
                    }
                    continue;
                }
            };

            let session_arc = {
                let sessions_read = sessions.read().await;
                sessions_read.get(agent_chat_id).cloned()
            };

            let Some(session_arc) = session_arc else {
                if let Ok(last_time) = chrono::DateTime::parse_from_rfc3339(last_activity_timestamp)
                {
                    let elapsed = Utc::now().signed_duration_since(last_time.with_timezone(&Utc));
                    if elapsed.num_seconds() as u64 > AGENT_STUCK_TIMEOUT.as_secs() {
                        tracing::warn!(
                            "Agent for card {} appears stuck (no session, last update {} ago)",
                            card.id,
                            humantime::format_duration(Duration::from_secs(
                                elapsed.num_seconds() as u64
                            ))
                        );

                        mark_agent_as_failed(
                            app.clone(),
                            task_id,
                            &card.id,
                            card.assignee.as_deref(),
                            None,
                            &format!(
                                "Agent appears stuck (no activity for {})",
                                humantime::format_duration(AGENT_STUCK_TIMEOUT)
                            ),
                            AgentFailureKind::Permanent,
                        )
                        .await?;
                    }
                }
                continue;
            };

            let session = session_arc.lock().await;

            // Check if session is in Error state
            if session.runtime.state == SessionState::Error {
                let error_msg = session
                    .runtime
                    .error
                    .as_deref()
                    .unwrap_or("Unknown error")
                    .to_string();
                let planner_chat_id = session
                    .thread
                    .task_meta
                    .as_ref()
                    .and_then(|meta| meta.planner_chat_id.clone());

                drop(session);

                tracing::warn!(
                    "Agent for card {} is in Error state: {}",
                    card.id,
                    error_msg
                );

                let failure_kind = AgentFailureKind::from_error(&error_msg);
                let failure_reason = failure_kind.final_report_reason(&error_msg);
                mark_agent_as_failed(
                    app.clone(),
                    task_id,
                    &card.id,
                    card.assignee.as_deref(),
                    planner_chat_id.as_deref(),
                    &failure_reason,
                    failure_kind,
                )
                .await?;
                continue;
            }

            let elapsed = session.last_activity.elapsed();
            let planner_chat_id = session
                .thread
                .task_meta
                .as_ref()
                .and_then(|meta| meta.planner_chat_id.clone());

            let now = Utc::now();
            let stall_info: Option<(StallKind, Duration)> =
                if agent_session_idle_stall_ready(&session, task_id, card) {
                    let kind = match session.runtime.state {
                        SessionState::Completed => StallKind::Completed,
                        _ => StallKind::IdleNoFinish,
                    };
                    Some((kind, elapsed))
                } else if session.runtime.state == SessionState::Generating
                    && session
                        .last_stream_delta_at
                        .map(|t| t.elapsed() > STREAM_STALL_TIMEOUT)
                        .unwrap_or(elapsed > STREAM_STALL_TIMEOUT)
                {
                    let stall_elapsed = session
                        .last_stream_delta_at
                        .map(|t| t.elapsed())
                        .unwrap_or(elapsed);
                    Some((StallKind::GeneratingNoTokens, stall_elapsed))
                } else if session.runtime.state == SessionState::ExecutingTools
                    && session
                        .last_tool_progress_at
                        .or(session.last_tool_started_at)
                        .map(|t| t.elapsed() > TOOL_STALL_TIMEOUT)
                        .unwrap_or(false)
                {
                    let stall_elapsed = session
                        .last_tool_progress_at
                        .or(session.last_tool_started_at)
                        .map(|t| t.elapsed())
                        .unwrap_or(elapsed);
                    Some((StallKind::ExecutingToolsNoProgress, stall_elapsed))
                } else {
                    None
                };

            if let Some((kind, stall_elapsed)) = stall_info {
                let (notify_count, _) = stall_planner_notifications(card);
                drop(session);

                if notify_count >= MAX_STALL_PLANNER_NOTIFICATIONS_PER_CARD {
                    mark_agent_as_failed(
                        app.clone(),
                        task_id,
                        &card.id,
                        card.assignee.as_deref(),
                        planner_chat_id.as_deref(),
                        &format!(
                            "{} for {}, planner-notify retries exhausted",
                            kind.short_reason(),
                            humantime::format_duration(stall_elapsed)
                        ),
                        AgentFailureKind::TransientExhausted,
                    )
                    .await?;
                } else if stall_planner_notify_allowed(card, now) {
                    let notify_result = notify_planner_about_stalled_agent(
                        app.clone(),
                        task_id,
                        card,
                        agent_chat_id,
                        planner_chat_id.as_deref(),
                        kind,
                        stall_elapsed,
                    )
                    .await;
                    let planner_notified = match notify_result {
                        Ok(notified) => notified,
                        Err(error) => {
                            tracing::warn!(
                                "Failed to notify planner about stalled agent for card {} in task {}: {}",
                                card.id,
                                task_id,
                                error
                            );
                            false
                        }
                    };
                    if !planner_notified
                        && stalled_agent_should_fail_after_notify_failure(stall_elapsed)
                    {
                        mark_agent_as_failed(
                            app.clone(),
                            task_id,
                            &card.id,
                            card.assignee.as_deref(),
                            planner_chat_id.as_deref(),
                            &format!(
                                "{} for {}, planner notification unavailable",
                                kind.short_reason(),
                                humantime::format_duration(stall_elapsed)
                            ),
                            AgentFailureKind::TransientExhausted,
                        )
                        .await?;
                    }
                }
                continue;
            }

            // If agent is idle and hasn't done anything in a long time, might be stuck
            if session.runtime.state == SessionState::Idle
                && session.command_queue.is_empty()
                && elapsed > AGENT_STUCK_TIMEOUT
            {
                drop(session);

                tracing::warn!(
                    "Agent for card {} appears stuck (idle for {:?})",
                    card.id,
                    elapsed
                );

                mark_agent_as_failed(
                    app.clone(),
                    task_id,
                    &card.id,
                    card.assignee.as_deref(),
                    planner_chat_id.as_deref(),
                    &format!(
                        "Agent stuck (idle with no activity for {})",
                        humantime::format_duration(elapsed)
                    ),
                    AgentFailureKind::Permanent,
                )
                .await?;
            }
        }
    }

    sweep_planner_wake_ups(app).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::types::{
        BoardCard, StatusUpdate, TaskBoard, TaskMeta as StoredTaskMeta, TaskStatus,
    };
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::time::Instant;

    fn create_test_card(id: &str, column: &str, assignee: Option<String>) -> BoardCard {
        let agent_chat_id = assignee.as_ref().map(|a| format!("agent-{}", a));
        BoardCard {
            id: id.to_string(),
            title: format!("Card {}", id),
            column: column.to_string(),
            priority: "P1".to_string(),
            depends_on: vec![],
            instructions: "Test instructions".to_string(),
            assignee,
            agent_chat_id,
            status_updates: vec![],
            comments: vec![],
            final_report: None,
            final_report_structured: None,
            verifier_report: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            started_at: Some(chrono::Utc::now().to_rfc3339()),
            last_heartbeat_at: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
            ab_variants: None,
            team_members: vec![],
            target_files: vec![],
            scope_guard_mode: Default::default(),
        }
    }

    fn make_test_agent_session(
        task_id: &str,
        card_id: &str,
        agent_id: &str,
        agent_chat_id: &str,
        state: SessionState,
        idle_for: Duration,
    ) -> ChatSession {
        let mut session = ChatSession::new(agent_chat_id.to_string());
        session.thread.task_meta = Some(TaskMeta {
            task_id: task_id.to_string(),
            role: "agents".to_string(),
            agent_id: Some(agent_id.to_string()),
            card_id: Some(card_id.to_string()),
            planner_chat_id: Some("planner-test".to_string()),
        });
        session.runtime.state = state;
        let activity_at = Instant::now()
            .checked_sub(idle_for)
            .unwrap_or_else(Instant::now);
        session.last_activity = activity_at;
        match state {
            SessionState::Generating => session.last_stream_delta_at = Some(activity_at),
            SessionState::ExecutingTools => session.last_tool_started_at = Some(activity_at),
            _ => {}
        }
        session
            .queue_processor_running
            .store(true, Ordering::SeqCst);
        session
    }

    async fn setup_monitor_case(
        column: &str,
        state: SessionState,
        idle_for: Duration,
        status_updates: Vec<StatusUpdate>,
    ) -> (
        tempfile::TempDir,
        AppState,
        String,
        String,
        Arc<tokio::sync::Mutex<ChatSession>>,
        Arc<tokio::sync::Mutex<ChatSession>>,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        *app.workspace
            .documents_state
            .workspace_folders
            .lock()
            .unwrap() = vec![temp.path().to_path_buf()];
        let task = crate::tasks::storage::create_task(gcx.clone(), "Stall task")
            .await
            .unwrap();
        let now = Utc::now().to_rfc3339();
        let meta = StoredTaskMeta {
            schema_version: 1,
            id: task.id.clone(),
            name: task.name.clone(),
            status: TaskStatus::Active,
            created_at: now.clone(),
            updated_at: now,
            cards_total: 1,
            cards_done: 0,
            cards_failed: 0,
            agents_active: if column == "doing" { 1 } else { 0 },
            base_branch: None,
            base_commit: None,
            default_agent_model: None,
            is_name_generated: true,
            last_agents_summary_at: None,
            planner_session_state: None,
        };
        crate::tasks::storage::save_task_meta(gcx.clone(), &task.id, &meta)
            .await
            .unwrap();

        let card_id = "T-1".to_string();
        let agent_id = "agent-1".to_string();
        let agent_chat_id = "agent-T-1".to_string();
        let mut card = create_test_card(&card_id, column, Some(agent_id.clone()));
        card.agent_chat_id = Some(agent_chat_id.clone());
        card.status_updates = status_updates;
        crate::tasks::storage::save_board(
            gcx.clone(),
            &task.id,
            &TaskBoard {
                cards: vec![card],
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let session = make_test_agent_session(
            &task.id,
            &card_id,
            &agent_id,
            &agent_chat_id,
            state,
            idle_for,
        );
        let session_arc = Arc::new(tokio::sync::Mutex::new(session));
        app.chat
            .sessions
            .write()
            .await
            .insert(agent_chat_id.clone(), session_arc.clone());

        let planner_chat_id = "planner-test".to_string();
        let mut planner_session = ChatSession::new(planner_chat_id.clone());
        planner_session.thread.task_meta = Some(TaskMeta {
            task_id: task.id.clone(),
            role: "planner".to_string(),
            agent_id: None,
            card_id: None,
            planner_chat_id: None,
        });
        planner_session.runtime.state = SessionState::WaitingUserInput;
        planner_session
            .queue_processor_running
            .store(true, Ordering::SeqCst);
        let planner_arc = Arc::new(tokio::sync::Mutex::new(planner_session));
        app.chat
            .sessions
            .write()
            .await
            .insert(planner_chat_id, planner_arc.clone());

        (temp, app, task.id, agent_chat_id, session_arc, planner_arc)
    }

    #[tokio::test]
    async fn idle_doing_agent_without_finish_notifies_planner() {
        let (_temp, app, task_id, agent_chat_id, agent_arc, planner_arc) =
            setup_monitor_case("doing", SessionState::Idle, Duration::from_secs(90), vec![]).await;

        check_for_stuck_agents(app.clone()).await.unwrap();

        let board = storage::load_board(app.gcx.clone(), &task_id)
            .await
            .unwrap();
        let card = board.get_card("T-1").unwrap();
        assert!(card.status_updates.iter().any(|update| {
            update
                .message
                .starts_with(STALL_PLANNER_NOTIFY_STATUS_PREFIX)
                && update.message.contains(&agent_chat_id)
        }));

        assert!(
            agent_arc.lock().await.command_queue.is_empty(),
            "agent itself should NOT receive a message"
        );

        let planner = planner_arc.lock().await;
        assert_eq!(planner.command_queue.len(), 1);
        let queued = planner.command_queue.front().unwrap();
        assert!(queued.priority);
        assert!(matches!(&queued.command, ChatCommand::Regenerate {}));
        let notice = planner.messages.last().unwrap();
        assert_eq!(notice.role, "event");
        assert_eq!(
            notice.extra["event"]["source"],
            json!(TASK_AGENT_MONITOR_SOURCE)
        );
        assert_eq!(
            notice.extra["event"]["payload"]["kind"],
            json!("stalled_agent")
        );
        let text = notice.content.content_text_only();
        assert!(text.contains("appears stalled"));
        assert!(text.contains("T-1"));
        assert!(text.contains("restart_agent"));
        assert!(text.contains(&agent_chat_id));
    }

    #[tokio::test]
    async fn active_agent_does_not_notify_planner() {
        for state in [SessionState::WaitingUserInput, SessionState::WaitingIde] {
            let (_temp, app, task_id, _agent_chat_id, _agent_arc, planner_arc) =
                setup_monitor_case("doing", state, Duration::from_secs(90), vec![]).await;

            check_for_stuck_agents(app.clone()).await.unwrap();

            let board = storage::load_board(app.gcx.clone(), &task_id)
                .await
                .unwrap();
            let card = board.get_card("T-1").unwrap();
            assert!(card.status_updates.iter().all(|update| {
                !update
                    .message
                    .starts_with(STALL_PLANNER_NOTIFY_STATUS_PREFIX)
            }));
            assert!(planner_arc.lock().await.command_queue.is_empty());
        }
    }

    #[tokio::test]
    async fn done_or_failed_card_does_not_notify_planner() {
        for column in ["done", "failed"] {
            let (_temp, app, task_id, _agent_chat_id, _agent_arc, planner_arc) =
                setup_monitor_case(column, SessionState::Idle, Duration::from_secs(90), vec![])
                    .await;

            check_for_stuck_agents(app.clone()).await.unwrap();

            let board = storage::load_board(app.gcx.clone(), &task_id)
                .await
                .unwrap();
            let card = board.get_card("T-1").unwrap();
            assert!(card.status_updates.iter().all(|update| {
                !update
                    .message
                    .starts_with(STALL_PLANNER_NOTIFY_STATUS_PREFIX)
            }));
            assert!(planner_arc.lock().await.command_queue.is_empty());
        }
    }

    #[tokio::test]
    async fn planner_notify_cooldown_prevents_spam() {
        let recent = StatusUpdate {
            timestamp: Utc::now().to_rfc3339(),
            message: format!("{} agent-T-1", STALL_PLANNER_NOTIFY_STATUS_PREFIX),
        };
        let (_temp, app, task_id, _agent_chat_id, _agent_arc, planner_arc) = setup_monitor_case(
            "doing",
            SessionState::Idle,
            Duration::from_secs(90),
            vec![recent],
        )
        .await;

        check_for_stuck_agents(app.clone()).await.unwrap();

        let board = storage::load_board(app.gcx.clone(), &task_id)
            .await
            .unwrap();
        let card = board.get_card("T-1").unwrap();
        assert_eq!(stall_planner_notifications(card).0, 1);
        assert!(planner_arc.lock().await.command_queue.is_empty());
    }

    #[tokio::test]
    async fn completed_state_on_doing_card_notifies_planner() {
        let (_temp, app, task_id, _agent_chat_id, _agent_arc, planner_arc) = setup_monitor_case(
            "doing",
            SessionState::Completed,
            Duration::from_secs(90),
            vec![],
        )
        .await;
        check_for_stuck_agents(app.clone()).await.unwrap();

        let board = storage::load_board(app.gcx.clone(), &task_id)
            .await
            .unwrap();
        assert_eq!(
            stall_planner_notifications(board.get_card("T-1").unwrap()).0,
            1
        );
        let planner = planner_arc.lock().await;
        assert_eq!(planner.command_queue.len(), 1);
        assert!(matches!(
            &planner.command_queue.front().unwrap().command,
            ChatCommand::Regenerate {}
        ));
        let text = planner.messages.last().unwrap().content.content_text_only();
        assert!(text.contains("Completed state but the card is still in `doing`"));
    }

    #[test]
    fn test_agent_stuck_timeout_constant() {
        assert_eq!(AGENT_STUCK_TIMEOUT.as_secs(), 20 * 60);
    }

    #[test]
    fn test_monitor_interval_constant() {
        assert_eq!(MONITOR_INTERVAL.as_secs(), 2 * 60);
    }

    #[test]
    fn test_tool_stall_timeout_constant() {
        assert_eq!(TOOL_STALL_TIMEOUT.as_secs(), 8 * 60);
    }

    #[test]
    fn stalled_agent_fail_after_notify_failure_waits_for_hard_timeout() {
        assert!(!stalled_agent_should_fail_after_notify_failure(
            AGENT_STUCK_TIMEOUT - Duration::from_secs(1)
        ));
        assert!(stalled_agent_should_fail_after_notify_failure(
            AGENT_STUCK_TIMEOUT
        ));
    }

    #[test]
    fn test_agents_active_count() {
        let cards = vec![
            create_test_card("T-1", "doing", Some("agent-1".to_string())),
            create_test_card("T-2", "doing", Some("agent-2".to_string())),
            create_test_card("T-3", "done", Some("agent-3".to_string())),
            create_test_card("T-4", "planned", None),
        ];

        let active_count = cards
            .iter()
            .filter(|c| c.column == "doing" && c.assignee.is_some())
            .count();

        assert_eq!(active_count, 2);
    }

    #[test]
    fn test_timestamp_fallback_logic() {
        let mut card = create_test_card("T-1", "doing", Some("agent-1".to_string()));

        card.status_updates.push(StatusUpdate {
            timestamp: "2024-01-01T10:00:00Z".to_string(),
            message: "Test update".to_string(),
        });

        let last_activity = card
            .status_updates
            .last()
            .map(|u| u.timestamp.as_str())
            .or(card.started_at.as_deref())
            .unwrap_or(&card.created_at);

        assert_eq!(last_activity, "2024-01-01T10:00:00Z");
    }

    #[test]
    fn test_timestamp_fallback_to_started_at() {
        let card = create_test_card("T-1", "doing", Some("agent-1".to_string()));

        let last_activity = card
            .status_updates
            .last()
            .map(|u| u.timestamp.as_str())
            .or(card.started_at.as_deref())
            .unwrap_or(&card.created_at);

        assert_eq!(last_activity, card.started_at.as_ref().unwrap());
    }

    #[test]
    fn test_timestamp_fallback_to_created_at() {
        let mut card = create_test_card("T-1", "doing", Some("agent-1".to_string()));
        card.started_at = None;

        let last_activity = card
            .status_updates
            .last()
            .map(|u| u.timestamp.as_str())
            .or(card.started_at.as_deref())
            .unwrap_or(&card.created_at);

        assert_eq!(last_activity, &card.created_at);
    }

    #[test]
    fn test_all_finished_when_no_doing_cards() {
        let cards = vec![
            create_test_card("T-1", "done", Some("agent-1".to_string())),
            create_test_card("T-2", "failed", Some("agent-2".to_string())),
        ];

        let agents_active = cards
            .iter()
            .filter(|c| c.column == "doing" && c.assignee.is_some())
            .count();

        assert_eq!(agents_active, 0);
    }

    #[test]
    fn test_elapsed_time_calculation() {
        let old_time = chrono::Utc::now() - chrono::Duration::seconds(25 * 60);
        let old_timestamp = old_time.to_rfc3339();

        if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(&old_timestamp) {
            let elapsed =
                chrono::Utc::now().signed_duration_since(parsed.with_timezone(&chrono::Utc));
            let elapsed_secs = elapsed.num_seconds() as u64;

            assert!(elapsed_secs >= 25 * 60 - 5);
            assert!(elapsed_secs > AGENT_STUCK_TIMEOUT.as_secs());
        } else {
            panic!("Failed to parse timestamp");
        }
    }

    #[test]
    fn test_assignee_mismatch_detection() {
        let card = create_test_card("T-1", "doing", Some("agent-123".to_string()));
        let expected_agent = "agent-456";

        let mismatch = card.assignee.as_ref() != Some(&expected_agent.to_string());
        assert!(mismatch);
    }

    #[test]
    fn test_assignee_match() {
        let card = create_test_card("T-1", "doing", Some("agent-123".to_string()));
        let expected_agent = "agent-123";

        let matches = card.assignee.as_ref() == Some(&expected_agent.to_string());
        assert!(matches);
    }

    #[test]
    fn task_agent_monitor_transient_exhaustion_retains_worktree() {
        let kind = AgentFailureKind::from_error("LLM error (503 Service Unavailable): overloaded");
        let report = kind.final_report_reason("LLM error (503 Service Unavailable): overloaded");

        assert_eq!(kind, AgentFailureKind::TransientExhausted);
        assert!(!kind.should_cleanup_worktree());
        assert!(report.contains("worktree retained"));
        assert!(report.contains("Provider temporarily unavailable"));
        assert!(report.contains("Suggested action: Retry"));
    }

    #[test]
    fn task_agent_monitor_permanent_failure_retains_worktree() {
        let kind = AgentFailureKind::from_error("LLM error (401 Unauthorized): invalid api key");
        let report = kind.final_report_reason("LLM error (401 Unauthorized): invalid api key");

        assert_eq!(kind, AgentFailureKind::Permanent);
        assert!(!kind.should_cleanup_worktree());
        assert!(report.contains("retained"));
    }

    #[test]
    fn task_agent_monitor_only_cancelled_failure_cleans_up_worktree() {
        for kind in [
            AgentFailureKind::TransientExhausted,
            AgentFailureKind::ContextLimit,
            AgentFailureKind::Permanent,
        ] {
            assert!(
                !kind.should_cleanup_worktree(),
                "{:?} should retain worktree",
                kind
            );
        }
        assert!(AgentFailureKind::Cancelled.should_cleanup_worktree());
    }

    #[test]
    fn task_agent_monitor_context_limit_is_distinct_from_transient() {
        let kind = AgentFailureKind::from_error(
            "LLM error (413 Payload Too Large): context length exceeded",
        );
        let report =
            kind.final_report_reason("LLM error (413 Payload Too Large): context length exceeded");

        assert_eq!(kind, AgentFailureKind::ContextLimit);
        assert!(!kind.should_cleanup_worktree());
        assert!(report.contains("context limit"));
        assert!(report.contains("Context too large"));
        assert!(report.contains("Suggested action: Compact the chat"));
    }

    #[tokio::test]
    async fn cancelled_agent_failure_attempts_cleanup() {
        let (_temp, app, task_id, _agent_chat_id, _agent_arc, _planner_arc) =
            setup_monitor_case("doing", SessionState::Idle, Duration::from_secs(90), vec![]).await;

        mark_agent_as_failed(
            app.clone(),
            &task_id,
            "T-1",
            Some("agent-1"),
            None,
            "cancelled by user",
            AgentFailureKind::Cancelled,
        )
        .await
        .unwrap();

        let board = storage::load_board(app.gcx.clone(), &task_id)
            .await
            .unwrap();
        let card = board.get_card("T-1").unwrap();
        assert_eq!(card.column, "failed");
        let report = card.final_report.as_deref().unwrap_or("");
        assert!(report.contains("Automatic cleanup attempted"));
        assert!(report.contains("no worktree recorded"));
        assert!(report.contains("no branch recorded"));
        assert!(!report.contains("retained for inspection or retry"));
    }

    #[tokio::test]
    async fn stream_stall_notifies_planner() {
        let (_temp, app, task_id, agent_chat_id, agent_arc, planner_arc) = setup_monitor_case(
            "doing",
            SessionState::Generating,
            STREAM_STALL_TIMEOUT + Duration::from_secs(10),
            vec![],
        )
        .await;

        check_for_stuck_agents(app.clone()).await.unwrap();

        let board = storage::load_board(app.gcx.clone(), &task_id)
            .await
            .unwrap();
        let card = board.get_card("T-1").unwrap();
        assert!(
            card.status_updates.iter().any(|u| {
                u.message.starts_with(STALL_PLANNER_NOTIFY_STATUS_PREFIX)
                    && u.message.contains(&agent_chat_id)
                    && u.message.contains("stream appears stalled")
            }),
            "expected stream-stall status update"
        );

        assert!(
            agent_arc.lock().await.command_queue.is_empty(),
            "agent itself should NOT receive a message"
        );

        let planner = planner_arc.lock().await;
        assert_eq!(planner.command_queue.len(), 1, "planner should be notified");
        assert!(matches!(
            &planner.command_queue.front().unwrap().command,
            ChatCommand::Regenerate {}
        ));
        let text = planner.messages.last().unwrap().content.content_text_only();
        assert!(text.contains("stream stalled"));
        assert!(text.contains("Generating"));
    }

    #[tokio::test]
    async fn tool_stall_notifies_planner() {
        let (_temp, app, task_id, agent_chat_id, agent_arc, planner_arc) = setup_monitor_case(
            "doing",
            SessionState::ExecutingTools,
            TOOL_STALL_TIMEOUT + Duration::from_secs(10),
            vec![],
        )
        .await;

        check_for_stuck_agents(app.clone()).await.unwrap();

        let board = storage::load_board(app.gcx.clone(), &task_id)
            .await
            .unwrap();
        let card = board.get_card("T-1").unwrap();
        assert!(
            card.status_updates.iter().any(|u| {
                u.message.starts_with(STALL_PLANNER_NOTIFY_STATUS_PREFIX)
                    && u.message.contains(&agent_chat_id)
                    && u.message.contains("tool execution stalled")
            }),
            "expected tool-stall status update"
        );

        assert!(agent_arc.lock().await.command_queue.is_empty());
        let planner = planner_arc.lock().await;
        assert_eq!(planner.command_queue.len(), 1);
        assert!(matches!(
            &planner.command_queue.front().unwrap().command,
            ChatCommand::Regenerate {}
        ));
        let text = planner.messages.last().unwrap().content.content_text_only();
        assert!(text.contains("tool execution stalled"));
    }

    #[tokio::test]
    async fn tool_state_with_recent_progress_is_not_notified() {
        let (_temp, app, task_id, _agent_chat_id, agent_arc, planner_arc) = setup_monitor_case(
            "doing",
            SessionState::ExecutingTools,
            TOOL_STALL_TIMEOUT + Duration::from_secs(10),
            vec![],
        )
        .await;
        {
            let mut session = agent_arc.lock().await;
            session.last_tool_progress_at = Some(Instant::now());
        }

        check_for_stuck_agents(app.clone()).await.unwrap();

        let board = storage::load_board(app.gcx.clone(), &task_id)
            .await
            .unwrap();
        let card = board.get_card("T-1").unwrap();
        assert!(card
            .status_updates
            .iter()
            .all(|u| { !u.message.starts_with(STALL_PLANNER_NOTIFY_STATUS_PREFIX) }));
        assert!(planner_arc.lock().await.command_queue.is_empty());
    }

    #[tokio::test]
    async fn stream_stall_after_max_planner_notifications_marks_failed_transient() {
        let old_ts = (Utc::now() - chrono::Duration::seconds(600)).to_rfc3339();
        let maxed = (0..MAX_STALL_PLANNER_NOTIFICATIONS_PER_CARD)
            .map(|_| StatusUpdate {
                timestamp: old_ts.clone(),
                message: format!(
                    "{} agent-T-1 (stream appears stalled, no token activity)",
                    STALL_PLANNER_NOTIFY_STATUS_PREFIX
                ),
            })
            .collect::<Vec<_>>();

        let (_temp, app, task_id, _agent_chat_id, _agent_arc, planner_arc) = setup_monitor_case(
            "doing",
            SessionState::Generating,
            STREAM_STALL_TIMEOUT + Duration::from_secs(10),
            maxed,
        )
        .await;

        check_for_stuck_agents(app.clone()).await.unwrap();

        let board = storage::load_board(app.gcx.clone(), &task_id)
            .await
            .unwrap();
        let card = board.get_card("T-1").unwrap();
        assert_eq!(card.column, "failed", "card should be failed");
        assert!(
            card.final_report
                .as_deref()
                .unwrap_or("")
                .contains("stalled"),
            "report should mention stalled"
        );

        // Failure path goes through notify_planner_agents_finished which posts
        // the standard "agent finished" planner summary message.
        let planner = planner_arc.lock().await;
        assert!(
            !planner.command_queue.is_empty(),
            "planner should receive a failure summary, not a stall notification"
        );
    }

    async fn setup_planner_wake_case(
        state: SessionState,
        wake_up_at: Option<chrono::DateTime<Utc>>,
        with_agent_card: bool,
    ) -> (
        tempfile::TempDir,
        AppState,
        String,
        Arc<tokio::sync::Mutex<ChatSession>>,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        *app.workspace
            .documents_state
            .workspace_folders
            .lock()
            .unwrap() = vec![temp.path().to_path_buf()];

        let task = crate::tasks::storage::create_task(gcx.clone(), "Planner wake task")
            .await
            .unwrap();
        let now = Utc::now().to_rfc3339();
        let meta = crate::tasks::types::TaskMeta {
            schema_version: 1,
            id: task.id.clone(),
            name: task.name.clone(),
            status: crate::tasks::types::TaskStatus::Active,
            created_at: now.clone(),
            updated_at: now,
            cards_total: if with_agent_card { 1 } else { 0 },
            cards_done: 0,
            cards_failed: 0,
            agents_active: if with_agent_card { 1 } else { 0 },
            base_branch: None,
            base_commit: None,
            default_agent_model: None,
            is_name_generated: true,
            last_agents_summary_at: None,
            planner_session_state: None,
        };
        crate::tasks::storage::save_task_meta(gcx.clone(), &task.id, &meta)
            .await
            .unwrap();

        let cards = if with_agent_card {
            let mut c = create_test_card("T-1", "doing", Some("agent-1".to_string()));
            c.agent_chat_id = Some("agent-T-1".to_string());
            vec![c]
        } else {
            vec![]
        };
        crate::tasks::storage::save_board(
            gcx.clone(),
            &task.id,
            &crate::tasks::types::TaskBoard {
                cards,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let planner_chat_id = "planner-wake-chat".to_string();
        let mut planner_session = ChatSession::new(planner_chat_id.clone());
        planner_session.thread.task_meta = Some(TaskMeta {
            task_id: task.id.clone(),
            role: "planner".to_string(),
            agent_id: None,
            card_id: None,
            planner_chat_id: None,
        });
        planner_session.runtime.state = state;
        planner_session.wake_up_at = wake_up_at;
        planner_session
            .queue_processor_running
            .store(true, std::sync::atomic::Ordering::SeqCst);

        let planner_arc = Arc::new(tokio::sync::Mutex::new(planner_session));
        app.chat
            .sessions
            .write()
            .await
            .insert(planner_chat_id.clone(), planner_arc.clone());

        (temp, app, task.id, planner_arc)
    }

    #[tokio::test]
    async fn planner_wake_up_fires_when_deadline_passes() {
        let past = Utc::now() - chrono::Duration::seconds(10);
        let (_temp, app, _task_id, planner_arc) =
            setup_planner_wake_case(SessionState::WaitingUserInput, Some(past), false).await;

        check_for_stuck_agents(app.clone()).await.unwrap();

        let session = planner_arc.lock().await;
        assert_eq!(
            session.command_queue.len(),
            1,
            "wake-up message should be queued"
        );
        assert!(
            session.wake_up_at.is_none(),
            "wake_up_at cleared after fire"
        );
        let cmd = session.command_queue.front().unwrap();
        assert!(cmd.priority);
        assert!(matches!(&cmd.command, ChatCommand::Regenerate {}));
        assert!(
            session
                .messages
                .last()
                .unwrap()
                .content
                .content_text_only()
                .contains("[AUTO WAKE]"),
            "message should contain AUTO WAKE"
        );
    }

    #[tokio::test]
    async fn planner_wake_up_does_not_double_fire() {
        let past = Utc::now() - chrono::Duration::seconds(10);
        let (_temp, app, _task_id, planner_arc) =
            setup_planner_wake_case(SessionState::WaitingUserInput, Some(past), false).await;

        check_for_stuck_agents(app.clone()).await.unwrap();
        check_for_stuck_agents(app.clone()).await.unwrap();

        let session = planner_arc.lock().await;
        assert_eq!(
            session.command_queue.len(),
            1,
            "only one wake-up message after two sweeps"
        );
    }

    #[tokio::test]
    async fn planner_wake_up_skipped_when_state_changed() {
        let past = Utc::now() - chrono::Duration::seconds(10);
        let (_temp, app, _task_id, planner_arc) =
            setup_planner_wake_case(SessionState::Generating, Some(past), false).await;

        check_for_stuck_agents(app.clone()).await.unwrap();

        let session = planner_arc.lock().await;
        assert!(
            session.command_queue.is_empty(),
            "no wake message when state is not WaitingUserInput"
        );
    }

    #[tokio::test]
    async fn planner_wake_up_message_contains_agent_status_snapshot() {
        let past = Utc::now() - chrono::Duration::seconds(10);
        let (_temp, app, _task_id, planner_arc) =
            setup_planner_wake_case(SessionState::WaitingUserInput, Some(past), true).await;

        check_for_stuck_agents(app.clone()).await.unwrap();

        let session = planner_arc.lock().await;
        assert_eq!(session.command_queue.len(), 1);
        assert!(matches!(
            &session.command_queue.front().unwrap().command,
            ChatCommand::Regenerate {}
        ));
        let text = session.messages.last().unwrap().content.content_text_only();
        assert!(text.contains("T-1"), "message should contain card T-1");
    }
}
