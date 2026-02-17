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
use tokio::sync::RwLock as ARwLock;
use tokio::time::sleep;
use chrono::Utc;

use crate::global_context::GlobalContext;
use crate::tasks::storage;
use crate::tasks::types::StatusUpdate;
use crate::chat::types::{SessionState, TaskMeta};
use crate::chat::{get_or_create_session_with_trajectory, process_command_queue};
use crate::chat::types::{CommandRequest, ChatCommand};

/// Timeout for agent inactivity before considering it stuck (20 minutes)
const AGENT_STUCK_TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// How often to check for stuck agents (5 minutes)
const MONITOR_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Detect if a session error should cause task agent failure
pub async fn handle_agent_streaming_error(
    gcx: Arc<ARwLock<GlobalContext>>,
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

    if let Err(e) = mark_agent_as_failed(
        gcx.clone(),
        &task_meta.task_id,
        card_id,
        task_meta.agent_id.as_deref(),
        &format!("Agent streaming error: {}", error_message),
    )
    .await
    {
        tracing::error!("Failed to mark agent as failed: {}", e);
    }
}

/// Mark a task card as failed and notify planner
async fn mark_agent_as_failed(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
    card_id: &str,
    expected_agent_id: Option<&str>,
    reason: &str,
) -> Result<(), String> {
    let card_id_owned = card_id.to_string();
    let reason_clone = reason.to_string();
    let expected_agent_id_owned = expected_agent_id.map(|s| s.to_string());

    let (board, (_card_title, all_finished)) =
        storage::update_board_atomic(gcx.clone(), task_id, move |board| {
            let card = board
                .get_card_mut(&card_id_owned)
                .ok_or(format!("Card {} not found", card_id_owned))?;

            if card.column == "done" || card.column == "failed" {
                let card_title = card.title.clone();
                let _ = card;
                let agents_active = board
                    .cards
                    .iter()
                    .filter(|c| c.column == "doing" && c.assignee.is_some())
                    .count();
                return Ok((card_title, agents_active == 0));
            }

            if card.column != "doing" {
                let card_title = card.title.clone();
                let _ = card;
                let agents_active = board
                    .cards
                    .iter()
                    .filter(|c| c.column == "doing" && c.assignee.is_some())
                    .count();
                return Ok((card_title, agents_active == 0));
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
                    let _ = card;
                    let agents_active = board
                        .cards
                        .iter()
                        .filter(|c| c.column == "doing" && c.assignee.is_some())
                        .count();
                    return Ok((card_title, agents_active == 0));
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

            Ok((card_title, all_finished))
        })
        .await?;

    storage::update_task_stats(gcx.clone(), task_id).await?;

    tracing::info!("Marked agent for card {} as failed: {}", card_id, reason);

    if let Some(card) = board.get_card(card_id) {
        if let (Some(ref wt), Some(ref branch)) = (&card.agent_worktree, &card.agent_branch) {
            let diff_report = cleanup_failed_agent_worktree(
                gcx.clone(), wt, branch, card.agent_worktree_name.as_deref()
            ).await;
            let card_id_for_cleanup = card_id.to_string();
            let _ = storage::update_board_atomic(gcx.clone(), task_id, move |board| {
                if let Some(c) = board.get_card_mut(&card_id_for_cleanup) {
                    if !diff_report.is_empty() {
                        if let Some(ref mut report) = c.final_report {
                            report.push_str(&diff_report);
                        }
                    }
                    c.agent_worktree = None;
                    c.agent_branch = None;
                }
                Ok(())
            }).await;
        }
    }

    if all_finished {
        notify_planner_all_agents_done(gcx, task_id, &board).await?;
    }

    Ok(())
}

/// Notify planner that all agents have completed (same logic as task_agent_finish)
async fn notify_planner_all_agents_done(
    gcx: Arc<ARwLock<GlobalContext>>,
    task_id: &str,
    board: &crate::tasks::types::TaskBoard,
) -> Result<(), String> {
    let since = match storage::load_task_meta(gcx.clone(), task_id).await {
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

    let planner_message = format!(
        "**All agents have completed.**\n\n{}\n\nRun `task_board_get(card_id)` to see full details for any card.",
        if results.is_empty() {
            let note = since
                .as_ref()
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_else(|| "(unknown)".to_string());
            format!("_(No newly-finished cards detected since {}.)_", note)
        } else {
            results.join("\n\n")
        }
    );

    let sessions = {
        let gcx_locked = gcx.read().await;
        gcx_locked.chat_sessions.clone()
    };

    let planner_chat_id = storage::get_planner_chat_id(gcx.clone(), task_id).await?;
    let planner_session =
        get_or_create_session_with_trajectory(gcx.clone(), &sessions, &planner_chat_id).await;

    let request = CommandRequest {
        client_request_id: format!("task-all-finished-{}", uuid::Uuid::new_v4()),
        priority: true,
        command: ChatCommand::UserMessage {
            content: serde_json::Value::String(planner_message),
            attachments: vec![],
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
            gcx.clone(),
            planner_session.clone(),
            processor_flag,
        ));
    }

    // Best-effort: mark summary as emitted.
    if let Ok(mut meta) = storage::load_task_meta(gcx.clone(), task_id).await {
        meta.last_agents_summary_at = Some(Utc::now().to_rfc3339());
        let _ = storage::save_task_meta(gcx.clone(), task_id, &meta).await;
    }

    Ok(())
}

pub(crate) async fn cleanup_failed_agent_worktree(
    gcx: Arc<ARwLock<GlobalContext>>,
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

    crate::files_in_workspace::remove_folder(gcx.clone(), &std::path::PathBuf::from(agent_worktree)).await;

    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    if let Some(workspace_root) = project_dirs.first() {
        let _ = Command::new("git")
            .args(["worktree", "remove", agent_worktree, "--force"])
            .current_dir(workspace_root)
            .output();

        let _ = Command::new("git")
            .args(["branch", "-D", agent_branch])
            .current_dir(workspace_root)
            .output();
    }

    let parent = Path::new(agent_worktree).parent();
    if let Some(p) = parent {
        if p.exists() && p.read_dir().map(|mut d| d.next().is_none()).unwrap_or(false) {
            let _ = std::fs::remove_dir(p);
        }
    }

    diff_report
}

/// Background task that monitors for stuck agents
pub async fn start_agent_monitor(gcx: Arc<ARwLock<GlobalContext>>) {
    tracing::info!("Starting task agent monitor");

    loop {
        sleep(MONITOR_INTERVAL).await;

        if let Err(e) = check_for_stuck_agents(gcx.clone()).await {
            tracing::error!("Agent monitor error: {}", e);
        }
    }
}

/// Check all active tasks for stuck agents
async fn check_for_stuck_agents(gcx: Arc<ARwLock<GlobalContext>>) -> Result<(), String> {
    let task_metas = storage::list_tasks(gcx.clone()).await?;

    for task_meta in task_metas {
        if task_meta.status != crate::tasks::types::TaskStatus::Active {
            continue;
        }

        let task_id = &task_meta.id;
        let board = storage::load_board(gcx.clone(), task_id).await?;
        let sessions = {
            let gcx_locked = gcx.read().await;
            gcx_locked.chat_sessions.clone()
        };

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
                                gcx.clone(),
                                task_id,
                                &card.id,
                                card.assignee.as_deref(),
                                &format!(
                                    "Agent appears stuck (no agent_chat_id, no activity for {})",
                                    humantime::format_duration(AGENT_STUCK_TIMEOUT)
                                ),
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
                            gcx.clone(),
                            task_id,
                            &card.id,
                            card.assignee.as_deref(),
                            &format!(
                                "Agent appears stuck (no activity for {})",
                                humantime::format_duration(AGENT_STUCK_TIMEOUT)
                            ),
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

                drop(session);

                tracing::warn!(
                    "Agent for card {} is in Error state: {}",
                    card.id,
                    error_msg
                );

                mark_agent_as_failed(
                    gcx.clone(),
                    task_id,
                    &card.id,
                    None,
                    &format!("Session error: {}", error_msg),
                )
                .await?;
                continue;
            }

            // Check for stuck agents (no activity for too long)
            let last_activity = session.last_activity;
            let elapsed = last_activity.elapsed();

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
                    gcx.clone(),
                    task_id,
                    &card.id,
                    None,
                    &format!(
                        "Agent stuck (idle with no activity for {})",
                        humantime::format_duration(elapsed)
                    ),
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
    use crate::tasks::types::{BoardCard, StatusUpdate};

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
            completed_at: None,
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
        }
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
}
