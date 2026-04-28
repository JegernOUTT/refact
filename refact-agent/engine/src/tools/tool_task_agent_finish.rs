use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::process::Command;
use std::path::Path;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;
use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use crate::global_context::GlobalContext;
use crate::agentic::generate_commit_message::generate_commit_message_by_diff;

use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::tasks::storage;
use crate::tasks::types::StatusUpdate;
use crate::chat::{get_or_create_session_with_trajectory, process_command_queue};
use crate::chat::types::{ChatCommand, CommandRequest};

async fn get_task_id(ccx: &Arc<AMutex<AtCommandsContext>>) -> Result<String, String> {
    let ccx_lock = ccx.lock().await;
    ccx_lock
        .task_meta
        .as_ref()
        .map(|m| m.task_id.clone())
        .ok_or_else(|| {
            "This tool can only be used by task agents (chat not bound to a task)".to_string()
        })
}

async fn get_card_id(ccx: &Arc<AMutex<AtCommandsContext>>) -> Result<String, String> {
    let ccx_lock = ccx.lock().await;
    ccx_lock
        .task_meta
        .as_ref()
        .and_then(|m| m.card_id.clone())
        .ok_or_else(|| {
            "This tool can only be used by task agents (no card_id in task_meta)".to_string()
        })
}

async fn auto_commit_worktree(
    gcx: Arc<ARwLock<GlobalContext>>,
    worktree_path: &Path,
    card_id: &str,
    card_title: &str,
) -> Result<Option<String>, String> {
    if !worktree_path.exists() {
        return Ok(None);
    }

    let git_dir = worktree_path.join(".git");
    if !git_dir.exists() && !worktree_path.join("..").join(".git").exists() {
        return Ok(None);
    }

    let status_output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(worktree_path)
        .output()
        .map_err(|e| format!("Failed to check git status: {}", e))?;

    if !status_output.status.success() {
        return Err(format!(
            "git status failed: {}",
            String::from_utf8_lossy(&status_output.stderr)
        ));
    }

    let status = String::from_utf8_lossy(&status_output.stdout);
    if status.trim().is_empty() {
        return Ok(None);
    }

    let add_output = Command::new("git")
        .args(["add", "-A"])
        .current_dir(worktree_path)
        .output()
        .map_err(|e| format!("Failed to stage changes: {}", e))?;

    if !add_output.status.success() {
        return Err(format!(
            "git add failed: {}",
            String::from_utf8_lossy(&add_output.stderr)
        ));
    }

    let diff_output = Command::new("git")
        .args(["diff", "--cached"])
        .current_dir(worktree_path)
        .output()
        .map_err(|e| format!("Failed to get diff: {}", e))?;
    let diff = String::from_utf8_lossy(&diff_output.stdout).to_string();

    let commit_msg =
        match generate_commit_message_by_diff(gcx, &diff, &Some(card_title.to_string())).await {
            Ok(msg) if !msg.trim().is_empty() => msg,
            _ => format!("Card {}: {}", card_id, card_title),
        };

    let commit_output = Command::new("git")
        .args([
            "-c",
            "user.name=Refact Agent",
            "-c",
            "user.email=agent@refact.ai",
            "commit",
            "-m",
            &commit_msg,
            "--no-gpg-sign",
        ])
        .current_dir(worktree_path)
        .output()
        .map_err(|e| format!("Failed to commit: {}", e))?;

    if !commit_output.status.success() {
        let stderr = String::from_utf8_lossy(&commit_output.stderr);
        if stderr.contains("nothing to commit") {
            return Ok(None);
        }
        return Err(format!("git commit failed: {}", stderr));
    }

    let rev_output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(worktree_path)
        .output()
        .map_err(|e| format!("Failed to get commit hash: {}", e))?;

    let commit_hash = String::from_utf8_lossy(&rev_output.stdout)
        .trim()
        .to_string();
    Ok(Some(commit_hash))
}

pub struct ToolTaskAgentFinish;

impl ToolTaskAgentFinish {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskAgentFinish {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_agent_finish".to_string(),
            display_name: "Task Agent Finish".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Mark the current card as completed or failed. Task agents MUST call this exactly once when finished. This updates the task board and notifies the planner.".to_string(),
            input_schema: json_schema_from_params(&[("success", "boolean", "true if the card was completed successfully, false if it failed"), ("report", "string", "Summary of what was done (if success) or why it failed (if failure)")], &["success", "report"]),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let task_id = get_task_id(&ccx).await?;
        let card_id = get_card_id(&ccx).await?;

        let success = match args.get("success") {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => s.to_lowercase() == "true",
            _ => return Err("Missing or invalid 'success' parameter (must be boolean)".to_string()),
        };

        let report = args
            .get("report")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'report' parameter")?
            .to_string();

        let gcx = ccx.lock().await.global_context.clone();

        let _ =
            crate::chat::task_agent_monitor::update_card_heartbeat(gcx.clone(), &task_id, &card_id)
                .await;

        let board_pre = storage::load_board(gcx.clone(), &task_id).await?;
        let card_pre = board_pre
            .get_card(&card_id)
            .ok_or(format!("Card {} not found", card_id))?;
        let worktree_path = card_pre.agent_worktree.clone();
        let card_title_for_commit = card_pre.title.clone();

        let commit_result = if success {
            if let Some(ref wt) = worktree_path {
                let wt_path = Path::new(wt);
                match auto_commit_worktree(gcx.clone(), wt_path, &card_id, &card_title_for_commit)
                    .await
                {
                    Ok(hash) => hash,
                    Err(e) => {
                        return Err(format!(
                            "Auto-commit failed: {}. Please ensure your changes are committed before calling task_agent_finish(success=true). \
                            You can run `git add -A && git commit -m 'your message'` in the worktree, or investigate the error.",
                            e
                        ));
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        let card_id_owned = card_id.clone();
        let report_clone = report.clone();
        let success_clone = success;
        let commit_hash = commit_result.clone();

        let (board, (card_title, _agent_branch, all_finished)) =
            storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
                let card = board
                    .get_card_mut(&card_id_owned)
                    .ok_or(format!("Card {} not found in task", card_id_owned))?;

                if card.column == "done" || card.column == "failed" {
                    return Err(format!(
                        "Card {} is already in '{}' column. Cannot finish twice.",
                        card_id_owned, card.column
                    ));
                }

                let card_title = card.title.clone();
                let agent_branch = card.agent_branch.clone();

                if success_clone {
                    card.final_report = Some(report_clone.clone());
                    card.column = "done".to_string();
                    card.completed_at = Some(Utc::now().to_rfc3339());
                    if let Some(ref hash) = commit_hash {
                        card.status_updates.push(StatusUpdate {
                            timestamp: Utc::now().to_rfc3339(),
                            message: format!("Auto-committed: {}", hash),
                        });
                    }
                    card.status_updates.push(StatusUpdate {
                        timestamp: Utc::now().to_rfc3339(),
                        message: "Agent completed successfully".to_string(),
                    });
                } else {
                    card.final_report = Some(format!("FAILED: {}", report_clone));
                    card.column = "failed".to_string();
                    card.completed_at = Some(Utc::now().to_rfc3339());
                    card.status_updates.push(StatusUpdate {
                        timestamp: Utc::now().to_rfc3339(),
                        message: format!("Agent failed: {}", report_clone),
                    });
                }

                let agents_active = board
                    .cards
                    .iter()
                    .filter(|c| c.column == "doing" && c.assignee.is_some())
                    .count();
                let all_finished = agents_active == 0;

                Ok((card_title, agent_branch, all_finished))
            })
            .await?;

        storage::update_task_stats(gcx.clone(), &task_id).await?;

        if !success {
            if let Some(ref wt) = worktree_path {
                if let Some(branch) = board
                    .get_card(&card_id)
                    .and_then(|c| c.agent_branch.clone())
                {
                    let _diff = crate::chat::task_agent_monitor::cleanup_failed_agent_worktree(
                        gcx.clone(),
                        wt,
                        &branch,
                        None,
                    )
                    .await;
                    let card_id_clear = card_id.clone();
                    let _ = storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
                        if let Some(c) = board.get_card_mut(&card_id_clear) {
                            c.agent_worktree = None;
                            c.agent_branch = None;
                        }
                        Ok(())
                    })
                    .await;
                }
            }
        }

        let result_message = if success {
            if all_finished {
                format!(
                    "✅ **Card Completed: {}**\n\n**Report:**\n{}\n\nAll agents have completed. Planner notified.",
                    card_title, report
                )
            } else {
                format!(
                    "✅ **Card Completed: {}**\n\n**Report:**\n{}\n\nPlanner will be notified when all agents complete.",
                    card_title, report
                )
            }
        } else {
            if all_finished {
                format!(
                    "❌ **Card Failed: {}**\n\n**Reason:**\n{}\n\nAll agents have completed. Planner notified.",
                    card_title, report
                )
            } else {
                format!(
                    "❌ **Card Failed: {}**\n\n**Reason:**\n{}\n\nPlanner will be notified when all agents complete.",
                    card_title, report
                )
            }
        };

        tracing::info!(
            "Agent finished card {} ({}): {}",
            card_id,
            if success { "success" } else { "failed" },
            report.chars().take(100).collect::<String>()
        );

        if all_finished {
            let since = match storage::load_task_meta(gcx.clone(), &task_id).await {
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

            let planner_chat_id = storage::get_planner_chat_id(gcx.clone(), &task_id).await?;
            let planner_session =
                get_or_create_session_with_trajectory(gcx.clone(), &sessions, &planner_chat_id)
                    .await;

            let request = CommandRequest {
                client_request_id: format!("task-all-finished-{}", Uuid::new_v4()),
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

            if !processor_flag.swap(true, Ordering::SeqCst) {
                tokio::spawn(process_command_queue(
                    gcx.clone(),
                    planner_session.clone(),
                    processor_flag,
                ));
            }

            // Best-effort: mark summary as emitted to avoid repeating historical cards.
            if let Ok(mut meta) = storage::load_task_meta(gcx.clone(), &task_id).await {
                meta.last_agents_summary_at = Some(Utc::now().to_rfc3339());
                let _ = storage::save_task_meta(gcx.clone(), &task_id, &meta).await;
            }
        }

        {
            let ccx_lock = ccx.lock().await;
            ccx_lock.abort_flag.store(true, Ordering::SeqCst);
        }

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result_message),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}
