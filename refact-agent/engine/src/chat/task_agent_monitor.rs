// Task agent failure detection and automatic cleanup
//
// This module monitors task agents and automatically marks them as failed when:
// - Streaming errors occur (network, model, timeout)
// - Agent becomes stuck (no activity beyond threshold)
// - Session ends in Error state without calling task_agent_finish

use std::sync::Arc;
use std::time::Duration;
use std::process::Command;
use std::path::Path;
use tokio::time::sleep;
use chrono::Utc;

use crate::app_state::AppState;
use crate::tasks::storage;
use crate::tasks::types::{BoardCard, StatusUpdate};
use crate::chat::retry_policy::{
    RetryDecision, UserErrorCategory, classify_llm_error_for_retry, classify_user_error,
    user_error_info,
};
use crate::chat::types::{ChatSession, SessionState, TaskMeta};
use crate::chat::{get_or_create_session_with_trajectory, process_command_queue};
use crate::chat::types::{CommandRequest, ChatCommand};
use crate::worktrees::service::WorktreeService;
use refact_buddy_core::types::BuddyRuntimeEvent;
use uuid::Uuid;

/// Timeout for agent inactivity before considering it stuck (20 minutes)
const AGENT_STUCK_TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// How often to check for stuck agents (5 minutes)
const MONITOR_INTERVAL: Duration = Duration::from_secs(5 * 60);

const MAX_IDLE_AGENT_NUDGES_PER_CARD: usize = 2;
const IDLE_AGENT_NUDGE_GRACE: Duration = Duration::from_secs(60);
const IDLE_AGENT_NUDGE_COOLDOWN_SECONDS: i64 = 300;
const IDLE_AGENT_NUDGE_STATUS_PREFIX: &str = "Auto-nudged idle agent:";
const IDLE_AGENT_REMINDER_MESSAGE: &str = concat!(
    "Automatic reminder: this task card is still marked as doing, but your chat stopped without calling `task_agent_finish`.\n",
    "Continue working if more changes are needed. If the task is complete, call `task_agent_finish(success=true, report=\"...\")`. ",
    "If it cannot be completed, call `task_agent_finish(success=false, report=\"...\")`."
);

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
        progress: None,
        dedupe_key: Some(dedupe_key.to_string()),
        priority: priority.unwrap_or("normal").to_string(),
        created_at: Utc::now().to_rfc3339(),
        ttl_ms: None,
        speech_text: None,
        scene: None,
        duration_hint: None,
        persistent: false,
        controls: Vec::new(),
        chat_id: None,
        dismissed: false,
    }
}

fn idle_agent_nudge_updates(
    card: &BoardCard,
) -> (usize, Option<chrono::DateTime<Utc>>) {
    let mut count = 0usize;
    let mut latest = None;

    for update in &card.status_updates {
        if !update.message.starts_with(IDLE_AGENT_NUDGE_STATUS_PREFIX) {
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

fn idle_agent_nudge_allowed_at(card: &BoardCard, now: chrono::DateTime<Utc>) -> bool {
    let (count, latest) = idle_agent_nudge_updates(card);
    if count >= MAX_IDLE_AGENT_NUDGES_PER_CARD {
        return false;
    }
    if let Some(latest) = latest {
        let since = now.signed_duration_since(latest).num_seconds();
        if since < IDLE_AGENT_NUDGE_COOLDOWN_SECONDS {
            return false;
        }
    }
    true
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

fn idle_agent_session_can_be_nudged(
    session: &ChatSession,
    task_id: &str,
    card: &BoardCard,
    now: chrono::DateTime<Utc>,
) -> bool {
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
    if session.last_activity.elapsed() < IDLE_AGENT_NUDGE_GRACE {
        return false;
    }
    idle_agent_nudge_allowed_at(card, now)
}

async fn record_idle_agent_nudge(
    app: AppState,
    task_id: &str,
    card_id: &str,
    agent_chat_id: &str,
    idle_for: Duration,
) -> Result<bool, String> {
    let card_id_owned = card_id.to_string();
    let agent_chat_id_owned = agent_chat_id.to_string();
    let now = Utc::now();
    let timestamp = now.to_rfc3339();
    let message = format!(
        "{} {} (idle for {})",
        IDLE_AGENT_NUDGE_STATUS_PREFIX,
        agent_chat_id,
        humantime::format_duration(idle_for)
    );

    storage::update_board_atomic(app.gcx.clone(), task_id, move |board| {
        let card = board
            .get_card_mut(&card_id_owned)
            .ok_or_else(|| format!("Card {} not found", card_id_owned))?;
        if card.column != "doing"
            || card.agent_chat_id.as_deref() != Some(agent_chat_id_owned.as_str())
            || card.assignee.is_none()
            || !idle_agent_nudge_allowed_at(card, now)
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

fn make_idle_agent_nudge_request() -> CommandRequest {
    CommandRequest {
        client_request_id: format!("idle-agent-nudge-{}", uuid::Uuid::new_v4()),
        priority: true,
        command: ChatCommand::UserMessage {
            content: serde_json::Value::String(IDLE_AGENT_REMINDER_MESSAGE.to_string()),
            attachments: vec![],
            context_files: vec![],
            suppress_auto_enrichment: false,
        },
    }
}

async fn enqueue_idle_agent_nudge_command(
    app: AppState,
    session_arc: Arc<tokio::sync::Mutex<ChatSession>>,
) -> Result<(), String> {
    let processor_flag = {
        let mut session = session_arc.lock().await;
        if session.closed {
            return Err(format!("Session {} is closed", session.chat_id));
        }
        session.command_queue.push_back(make_idle_agent_nudge_request());
        session.emit_queue_update();
        session.touch();
        session.queue_notify.notify_one();
        session.queue_processor_running.clone()
    };

    if !processor_flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
        tokio::spawn(process_command_queue(
            app.clone(),
            session_arc.clone(),
            processor_flag,
        ));
    }

    Ok(())
}

async fn nudge_idle_agent(
    app: AppState,
    task_id: &str,
    card_id: &str,
    agent_chat_id: &str,
    session_arc: Arc<tokio::sync::Mutex<ChatSession>>,
    idle_for: Duration,
) -> Result<bool, String> {
    if !record_idle_agent_nudge(app.clone(), task_id, card_id, agent_chat_id, idle_for).await? {
        return Ok(false);
    }

    enqueue_idle_agent_nudge_command(app, session_arc).await?;
    tracing::info!(
        "Auto-nudged idle task agent for card {} in chat {}",
        card_id,
        agent_chat_id
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
        matches!(self, AgentFailureKind::Permanent)
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
            AgentFailureKind::Permanent => "Worktree cleanup may run after this report.",
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
        "compact" => "Compact the chat, reduce attached context, or switch to a larger-context model.",
        "check_auth" => "Check provider credentials, OAuth login, token scope, and provider configuration.",
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
            card.column = "failed".to_string();
            card.completed_at = Some(Utc::now().to_rfc3339());
            card.status_updates.push(StatusUpdate {
                timestamp: Utc::now().to_rfc3339(),
                message: format!("Automatic failure detection: {}", reason_clone),
            });

            let _ = card;

            let agents_active_after = board
                .cards
                .iter()
                .filter(|c| c.column == "doing" && c.assignee.is_some())
                .count();
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

    if failure_kind.should_cleanup_worktree() {
        if let Some(card) = board.get_card(card_id) {
            if let (Some(ref wt), Some(ref branch)) = (&card.agent_worktree, &card.agent_branch) {
                let diff_report = cleanup_failed_agent_worktree(
                    app.clone(),
                    wt,
                    branch,
                    card.agent_worktree_name.as_deref(),
                )
                .await;
                let card_id_for_cleanup = card_id.to_string();
                let _ = storage::update_board_atomic(app.gcx.clone(), task_id, move |board| {
                    if let Some(c) = board.get_card_mut(&card_id_for_cleanup) {
                        if !diff_report.is_empty() {
                            if let Some(ref mut report) = c.final_report {
                                report.push_str(&diff_report);
                            }
                        }
                        c.agent_worktree = None;
                        c.agent_branch = None;
                        c.agent_worktree_name = None;
                    }
                    Ok(())
                })
                .await;
            }
        }
    } else {
        let card_id_for_retain = card_id.to_string();
        let _ = storage::update_board_atomic(app.gcx.clone(), task_id, move |board| {
            if let Some(c) = board.get_card_mut(&card_id_for_retain) {
                if let Some(ref mut report) = c.final_report {
                    report
                        .push_str("\n\nWorktree and branch were retained for inspection or retry.");
                }
            }
            Ok(())
        })
        .await;
    }

    if let Err(e) =
        notify_planner_agents_finished(app.clone(), task_id, &board, all_finished, planner_chat_id)
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
        "Run `task_board_get(card_id)` to see full details for any card."
    } else {
        "Other agents may still be running. Run `task_check_agents` to see live status or `task_board_get(card_id)` for full details."
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
            return Err(format!(
                "Cannot notify task planner {}: trajectory is missing or deleted",
                planner_chat_id
            ));
        }
    }

    let request = CommandRequest {
        client_request_id: format!("task-agent-finished-{}", uuid::Uuid::new_v4()),
        priority: true,
        command: ChatCommand::UserMessage {
            content: serde_json::Value::String(planner_message),
            attachments: vec![],
            context_files: vec![],
            suppress_auto_enrichment: false,
        },
    };

    let processor_flag = {
        let mut session = planner_session.lock().await;
        session.command_queue.push_back(request);
        session.emit_queue_update();
        session.queue_notify.notify_one();
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

pub(crate) async fn remove_agent_worktree_and_branch(
    app: AppState,
    agent_worktree: &str,
    agent_branch: &str,
    agent_worktree_name: Option<&str>,
) -> (bool, bool) {
    if let Some(worktree_id) = agent_worktree_name {
        let cache_dir = app.paths.cache_dir.clone();
        let project_dirs = crate::files_correction::get_project_dirs(app.gcx.clone()).await;
        if let Some(source_root) = project_dirs.first() {
            if let Ok(service) = WorktreeService::new(cache_dir, source_root.clone()) {
                match service.delete_worktree(worktree_id, true).await {
                    Ok(deleted) => {
                        return (
                            deleted.deleted && !Path::new(agent_worktree).exists(),
                            deleted.branch_deleted,
                        );
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
    }

    let project_dirs = crate::files_correction::get_project_dirs(app.gcx.clone()).await;
    if let Some(workspace_root) = project_dirs.first() {
        let worktree_removed = Command::new("git")
            .args(["worktree", "remove", agent_worktree, "--force"])
            .current_dir(workspace_root)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        let branch_deleted = Command::new("git")
            .args(["branch", "-D", agent_branch])
            .current_dir(workspace_root)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        (worktree_removed, branch_deleted)
    } else {
        (false, false)
    }
}

pub(crate) async fn cleanup_failed_agent_worktree(
    app: AppState,
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

    let _ = remove_agent_worktree_and_branch(
        app.clone(),
        agent_worktree,
        agent_branch,
        _agent_worktree_name,
    )
    .await;

    let parent = Path::new(agent_worktree).parent();
    if let Some(p) = parent {
        if p.exists()
            && p.read_dir()
                .map(|mut d| d.next().is_none())
                .unwrap_or(false)
        {
            let _ = std::fs::remove_dir(p);
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
                    None,
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
            let can_nudge = idle_agent_session_can_be_nudged(&session, task_id, card, Utc::now());

            if can_nudge {
                drop(session);

                let _ = nudge_idle_agent(
                    app.clone(),
                    task_id,
                    &card.id,
                    agent_chat_id,
                    session_arc.clone(),
                    elapsed,
                )
                .await?;
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
                    None,
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

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::types::{
        BoardCard, StatusUpdate, TaskBoard, TaskMeta as StoredTaskMeta, TaskStatus,
    };
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
            final_report: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            started_at: Some(chrono::Utc::now().to_rfc3339()),
            last_heartbeat_at: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
            target_files: vec![],
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
        session.last_activity = Instant::now()
            .checked_sub(idle_for)
            .unwrap_or_else(Instant::now);
        session.queue_processor_running.store(true, Ordering::SeqCst);
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
    ) {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        *app.workspace.documents_state.workspace_folders.lock().unwrap() =
            vec![temp.path().to_path_buf()];
        let task = crate::tasks::storage::create_task(gcx.clone(), "Nudge task")
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

        (temp, app, task.id, agent_chat_id, session_arc)
    }

    #[tokio::test]
    async fn idle_doing_agent_without_finish_gets_nudged() {
        let (_temp, app, task_id, agent_chat_id, session_arc) = setup_monitor_case(
            "doing",
            SessionState::Idle,
            Duration::from_secs(90),
            vec![],
        )
        .await;

        check_for_stuck_agents(app.clone()).await.unwrap();

        let board = storage::load_board(app.gcx.clone(), &task_id).await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert!(card.status_updates.iter().any(|update| {
            update.message.starts_with(IDLE_AGENT_NUDGE_STATUS_PREFIX)
                && update.message.contains(&agent_chat_id)
        }));

        let session = session_arc.lock().await;
        assert_eq!(session.command_queue.len(), 1);
        let queued = session.command_queue.front().unwrap();
        assert!(queued.priority);
        match &queued.command {
            ChatCommand::UserMessage { content, .. } => {
                let text = content.as_str().unwrap();
                assert!(text.contains("task_agent_finish(success=true, report=\"...\")"));
                assert!(text.contains("task_agent_finish(success=false, report=\"...\")"));
            }
            _ => panic!("expected user message nudge"),
        }
    }

    #[tokio::test]
    async fn active_agent_is_not_nudged() {
        for state in [
            SessionState::Generating,
            SessionState::ExecutingTools,
            SessionState::WaitingUserInput,
            SessionState::WaitingIde,
        ] {
            let (_temp, app, task_id, _agent_chat_id, session_arc) = setup_monitor_case(
                "doing",
                state,
                Duration::from_secs(90),
                vec![],
            )
            .await;

            check_for_stuck_agents(app.clone()).await.unwrap();

            let board = storage::load_board(app.gcx.clone(), &task_id).await.unwrap();
            let card = board.get_card("T-1").unwrap();
            assert!(card.status_updates.iter().all(|update| {
                !update.message.starts_with(IDLE_AGENT_NUDGE_STATUS_PREFIX)
            }));
            assert!(session_arc.lock().await.command_queue.is_empty());
        }
    }

    #[tokio::test]
    async fn done_or_failed_card_is_not_nudged() {
        for column in ["done", "failed"] {
            let (_temp, app, task_id, _agent_chat_id, session_arc) = setup_monitor_case(
                column,
                SessionState::Idle,
                Duration::from_secs(90),
                vec![],
            )
            .await;

            check_for_stuck_agents(app.clone()).await.unwrap();

            let board = storage::load_board(app.gcx.clone(), &task_id).await.unwrap();
            let card = board.get_card("T-1").unwrap();
            assert!(card.status_updates.iter().all(|update| {
                !update.message.starts_with(IDLE_AGENT_NUDGE_STATUS_PREFIX)
            }));
            assert!(session_arc.lock().await.command_queue.is_empty());
        }
    }

    #[tokio::test]
    async fn nudge_rate_limit_prevents_spam() {
        let recent = StatusUpdate {
            timestamp: Utc::now().to_rfc3339(),
            message: format!("{} agent-T-1", IDLE_AGENT_NUDGE_STATUS_PREFIX),
        };
        let (_temp, app, task_id, _agent_chat_id, session_arc) = setup_monitor_case(
            "doing",
            SessionState::Idle,
            Duration::from_secs(90),
            vec![recent],
        )
        .await;

        check_for_stuck_agents(app.clone()).await.unwrap();

        let board = storage::load_board(app.gcx.clone(), &task_id).await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert_eq!(idle_agent_nudge_updates(card).0, 1);
        assert!(session_arc.lock().await.command_queue.is_empty());

        let old_timestamp = (Utc::now() - chrono::Duration::seconds(600)).to_rfc3339();
        let maxed_updates = (0..MAX_IDLE_AGENT_NUDGES_PER_CARD)
            .map(|_| StatusUpdate {
                timestamp: old_timestamp.clone(),
                message: format!("{} agent-T-1", IDLE_AGENT_NUDGE_STATUS_PREFIX),
            })
            .collect::<Vec<_>>();
        let (_temp, app, task_id, _agent_chat_id, session_arc) = setup_monitor_case(
            "doing",
            SessionState::Idle,
            Duration::from_secs(90),
            maxed_updates,
        )
        .await;

        check_for_stuck_agents(app.clone()).await.unwrap();

        let board = storage::load_board(app.gcx.clone(), &task_id).await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert_eq!(idle_agent_nudge_updates(card).0, MAX_IDLE_AGENT_NUDGES_PER_CARD);
        assert!(session_arc.lock().await.command_queue.is_empty());
    }

    #[tokio::test]
    async fn nudge_triggers_regeneration_or_command_queue() {
        let (_temp, app, task_id, _agent_chat_id, session_arc) = setup_monitor_case(
            "doing",
            SessionState::Completed,
            Duration::from_secs(90),
            vec![],
        )
        .await;
        check_for_stuck_agents(app.clone()).await.unwrap();

        let board = storage::load_board(app.gcx.clone(), &task_id).await.unwrap();
        assert_eq!(idle_agent_nudge_updates(board.get_card("T-1").unwrap()).0, 1);
        let session = session_arc.lock().await;
        assert_eq!(session.command_queue.len(), 1);
        assert_eq!(session.runtime.queue_size, 1);
        assert!(matches!(
            &session.command_queue.front().unwrap().command,
            ChatCommand::UserMessage { .. }
        ));
    }

    #[test]
    fn test_agent_stuck_timeout_constant() {
        assert_eq!(AGENT_STUCK_TIMEOUT.as_secs(), 20 * 60);
    }

    #[test]
    fn test_monitor_interval_constant() {
        assert_eq!(MONITOR_INTERVAL.as_secs(), 5 * 60);
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
    fn task_agent_monitor_permanent_failure_can_cleanup_worktree() {
        let kind = AgentFailureKind::from_error("LLM error (401 Unauthorized): invalid api key");

        assert_eq!(kind, AgentFailureKind::Permanent);
        assert!(kind.should_cleanup_worktree());
    }

    #[test]
    fn task_agent_monitor_context_limit_is_distinct_from_transient() {
        let kind = AgentFailureKind::from_error(
            "LLM error (413 Payload Too Large): context length exceeded",
        );
        let report = kind.final_report_reason(
            "LLM error (413 Payload Too Large): context length exceeded",
        );

        assert_eq!(kind, AgentFailureKind::ContextLimit);
        assert!(!kind.should_cleanup_worktree());
        assert!(report.contains("context limit"));
        assert!(report.contains("Context too large"));
        assert!(report.contains("Suggested action: Compact the chat"));
    }
}
