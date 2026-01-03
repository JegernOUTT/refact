use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::process::Command;
use std::path::Path;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use crate::tools::tools_description::{Tool, ToolDesc, ToolParam, ToolSource, ToolSourceType};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::tasks::storage;
use crate::tasks::types::StatusUpdate;
use crate::chat::{get_or_create_session_with_trajectory, process_command_queue};
use crate::chat::types::{ChatCommand, CommandRequest};

async fn get_task_id(ccx: &Arc<AMutex<AtCommandsContext>>) -> Result<String, String> {
    let ccx_lock = ccx.lock().await;
    ccx_lock.task_meta.as_ref()
        .map(|m| m.task_id.clone())
        .ok_or_else(|| "This tool can only be used by task agents (chat not bound to a task)".to_string())
}

async fn get_card_id(ccx: &Arc<AMutex<AtCommandsContext>>) -> Result<String, String> {
    let ccx_lock = ccx.lock().await;
    ccx_lock.task_meta.as_ref()
        .and_then(|m| m.card_id.clone())
        .ok_or_else(|| "This tool can only be used by task agents (no card_id in task_meta)".to_string())
}

fn auto_commit_worktree(worktree_path: &Path, card_id: &str, card_title: &str) -> Result<Option<String>, String> {
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
        return Err(format!("git add failed: {}", String::from_utf8_lossy(&add_output.stderr)));
    }

    let commit_msg = format!("Card {}: {}", card_id, card_title);
    let commit_output = Command::new("git")
        .args([
            "-c", "user.name=Refact Agent",
            "-c", "user.email=agent@refact.ai",
            "commit",
            "-m", &commit_msg,
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

    let commit_hash = String::from_utf8_lossy(&rev_output.stdout).trim().to_string();
    Ok(Some(commit_hash))
}

pub struct ToolTaskAgentFinish;

impl ToolTaskAgentFinish {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl Tool for ToolTaskAgentFinish {
    fn as_any(&self) -> &dyn std::any::Any { self }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_agent_finish".to_string(),
            display_name: "Task Agent Finish".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            agentic: false,
            experimental: false,
            description: "Mark the current card as completed or failed. Task agents MUST call this exactly once when finished. This updates the task board and notifies the planner.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "success".to_string(),
                    param_type: "boolean".to_string(),
                    description: "true if the card was completed successfully, false if it failed".to_string(),
                },
                ToolParam {
                    name: "report".to_string(),
                    param_type: "string".to_string(),
                    description: "Summary of what was done (if success) or why it failed (if failure)".to_string(),
                },
            ],
            parameters_required: vec!["success".to_string(), "report".to_string()],
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

        let report = args.get("report")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'report' parameter")?
            .to_string();

        let gcx = ccx.lock().await.global_context.clone();

        let board_pre = storage::load_board(gcx.clone(), &task_id).await?;
        let card_pre = board_pre.get_card(&card_id)
            .ok_or(format!("Card {} not found", card_id))?;
        let worktree_path = card_pre.agent_worktree.clone();
        let card_title_for_commit = card_pre.title.clone();

        let commit_result = if success {
            if let Some(ref wt) = worktree_path {
                let wt_path = Path::new(wt);
                match auto_commit_worktree(wt_path, &card_id, &card_title_for_commit) {
                    Ok(hash) => hash,
                    Err(e) => {
                        tracing::warn!("Auto-commit failed for card {}: {}", card_id, e);
                        None
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

        let (board, (card_title, _agent_branch, all_finished)) = storage::update_board_atomic(
            gcx.clone(),
            &task_id,
            move |board| {
                let card = board.get_card_mut(&card_id_owned)
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

                let agents_active = board.cards.iter()
                    .filter(|c| c.column == "doing" && c.assignee.is_some())
                    .count();
                let all_finished = agents_active == 0;

                Ok((card_title, agent_branch, all_finished))
            },
        ).await?;

        storage::update_task_stats(gcx.clone(), &task_id).await?;

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
            let mut done_cards = Vec::new();
            let mut failed_cards = Vec::new();

            for card in &board.cards {
                if card.column == "done" {
                    let branch_info = card.agent_branch.as_deref().unwrap_or("no branch");
                    let report_preview: String = card.final_report
                        .as_deref()
                        .unwrap_or("")
                        .chars()
                        .take(100)
                        .collect();
                    done_cards.push(format!("- **{}**: {} (branch: {})\n  Report: {}", 
                        card.id, card.title, branch_info, report_preview));
                } else if card.column == "failed" {
                    let report_preview: String = card.final_report
                        .as_deref()
                        .unwrap_or("")
                        .chars()
                        .take(100)
                        .collect();
                    failed_cards.push(format!("- **{}**: {}\n  Reason: {}", 
                        card.id, card.title, report_preview));
                }
            }

            let mut planner_message = String::from("✅ **All agents have completed!**\n\n");
            
            if !done_cards.is_empty() {
                planner_message.push_str(&format!("**Completed ({}):**\n{}\n\n", 
                    done_cards.len(), done_cards.join("\n")));
            }
            
            if !failed_cards.is_empty() {
                planner_message.push_str(&format!("**Failed ({}):**\n{}\n\n", 
                    failed_cards.len(), failed_cards.join("\n")));
            }
            
            planner_message.push_str("Run `task_board_get` to review full results and decide next steps.");

            let sessions = {
                let gcx_locked = gcx.read().await;
                gcx_locked.chat_sessions.clone()
            };

            let planner_chat_id = storage::get_planner_chat_id(gcx.clone(), &task_id).await?;
            let planner_session = get_or_create_session_with_trajectory(gcx.clone(), &sessions, &planner_chat_id).await;

            let request = CommandRequest {
                client_request_id: format!("task-all-finished-{}", Uuid::new_v4()),
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

            if !processor_flag.swap(true, Ordering::SeqCst) {
                tokio::spawn(process_command_queue(
                    gcx.clone(),
                    planner_session.clone(),
                    processor_flag,
                ));
            }
        }

        Ok((false, vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(result_message),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        })]))
    }

    fn tool_depends_on(&self) -> Vec<String> { vec![] }
}
