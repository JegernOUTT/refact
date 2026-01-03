use std::collections::HashMap;
use std::sync::Arc;
use std::process::Command;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;

use crate::tools::tools_description::{Tool, ToolDesc, ToolParam, ToolSource, ToolSourceType};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::tasks::storage;

pub struct ToolTaskMergeAgent;

impl ToolTaskMergeAgent {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl Tool for ToolTaskMergeAgent {
    fn as_any(&self) -> &dyn std::any::Any { self }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_merge_agent".to_string(),
            display_name: "Task Merge Agent".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            agentic: true,
            experimental: false,
            description: "Merge an agent's work back to the main branch and cleanup the worktree. The agent must have completed work on a card with an associated git branch and worktree.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "card_id".to_string(),
                    param_type: "string".to_string(),
                    description: "Card ID whose agent branch to merge".to_string(),
                },
                ToolParam {
                    name: "strategy".to_string(),
                    param_type: "string".to_string(),
                    description: "Merge strategy: 'merge' (default) or 'squash'".to_string(),
                },
                ToolParam {
                    name: "delete_worktree".to_string(),
                    param_type: "boolean".to_string(),
                    description: "Delete worktree and branch after merge (default: true)".to_string(),
                },
            ],
            parameters_required: vec!["card_id".to_string()],
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let ccx_lock = ccx.lock().await;
        
        let is_planner = ccx_lock.task_meta.as_ref()
            .map(|m| m.role == "planner")
            .unwrap_or(false);

        if !is_planner {
            return Err(
                "task_merge_agent can only be called by the task planner. \
                 Switch to the planner chat to merge agent work.".to_string()
            );
        }
        
        let task_id = if let Some(id) = args.get("task_id").and_then(|v| v.as_str()) {
            id.to_string()
        } else if let Some(ref meta) = ccx_lock.task_meta {
            meta.task_id.clone()
        } else {
            return Err("Missing 'task_id' (and chat is not bound to a task)".to_string());
        };

        let card_id = args.get("card_id").and_then(|v| v.as_str())
            .ok_or("Missing 'card_id'")?;

        let strategy = args.get("strategy")
            .and_then(|v| v.as_str())
            .unwrap_or("merge");

        let delete_worktree = match args.get("delete_worktree") {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => s.to_lowercase() == "true",
            _ => true,
        };

        if strategy != "merge" && strategy != "squash" {
            return Err(format!("Invalid strategy '{}', must be 'merge' or 'squash'", strategy));
        }

        let gcx = ccx_lock.global_context.clone();
        drop(ccx_lock);

        let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
        let workspace_root = project_dirs.first().ok_or("No workspace folder found")?;

        // Verify it's a git repo
        if !workspace_root.join(".git").exists() {
            return Err("Workspace is not a git repository".to_string());
        }

        let board = storage::load_board(gcx.clone(), &task_id).await?;
        let card = board.get_card(card_id)
            .ok_or(format!("Card {} not found", card_id))?;

        let agent_branch = card.agent_branch.as_ref()
            .ok_or(format!("Card {} has no agent branch", card_id))?;
        let agent_worktree = card.agent_worktree.as_ref()
            .ok_or(format!("Card {} has no agent worktree", card_id))?;

        let task_meta = storage::load_task_meta(gcx.clone(), &task_id).await?;
        let base_branch = task_meta.base_branch.as_ref()
            .ok_or("Task has no base branch set")?;

        // Helper to run git commands
        let run_git = |args: &[&str]| -> Result<String, String> {
            let output = Command::new("git")
                .args(args)
                .current_dir(workspace_root)
                .output()
                .map_err(|e| format!("Failed to run git: {}", e))?;

            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).to_string())
            } else {
                Err(String::from_utf8_lossy(&output.stderr).to_string())
            }
        };

        // Checkout base branch
        run_git(&["checkout", base_branch])
            .map_err(|e| format!("Failed to checkout base branch: {}", e))?;

        let merge_result = if strategy == "squash" {
            run_git(&["merge", "--squash", agent_branch])
        } else {
            run_git(&["merge", agent_branch, "-m", &format!("Merge agent work from {}", agent_branch)])
        };

        if let Err(e) = merge_result {
            let status = run_git(&["status", "--porcelain"]).unwrap_or_default();
            let has_conflicts = status.lines().any(|l| {
                let chars: Vec<char> = l.chars().take(2).collect();
                chars.len() >= 2 && (chars[0] == 'U' || chars[1] == 'U' ||
                    (chars[0] == 'A' && chars[1] == 'A') ||
                    (chars[0] == 'D' && chars[1] == 'D'))
            });

            if has_conflicts {
                let _ = run_git(&["merge", "--abort"]);
                let _ = run_git(&["reset", "--merge"]);

                let conflict_files: Vec<String> = status.lines()
                    .filter(|l| {
                        let chars: Vec<char> = l.chars().take(2).collect();
                        chars.len() >= 2 && (chars[0] == 'U' || chars[1] == 'U' ||
                            (chars[0] == 'A' && chars[1] == 'A') ||
                            (chars[0] == 'D' && chars[1] == 'D'))
                    })
                    .filter_map(|l| l.get(3..).map(|s| s.to_string()))
                    .collect();

                let error_msg = format!(
                    "Merge conflicts detected:\n{}\n\nMerge aborted. Please resolve conflicts manually or retry.",
                    conflict_files.join("\n")
                );

                return Ok((false, vec![ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(error_msg),
                    tool_calls: None,
                    tool_call_id: tool_call_id.clone(),
                    ..Default::default()
                })]));
            }
            return Err(format!("Merge failed: {}", e));
        }

        if strategy == "squash" {
            let commit_result = run_git(&["commit", "-m", &format!("Squash merge agent work from {}", agent_branch)]);
            if let Err(e) = commit_result {
                if !e.contains("nothing to commit") {
                    return Err(format!("Failed to commit squash merge: {}", e));
                }
            }
        }

        // Cleanup worktree and branch if requested
        if delete_worktree {
            let worktree_removed = run_git(&["worktree", "remove", agent_worktree, "--force"]).is_ok();
            let branch_deleted = run_git(&["branch", "-D", agent_branch]).is_ok();

            if worktree_removed || branch_deleted {
                let card_id_owned = card_id.to_string();
                let (_board, _) = storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
                    if let Some(card) = board.get_card_mut(&card_id_owned) {
                        card.agent_branch = None;
                        card.agent_worktree = None;
                        card.agent_worktree_name = None;
                    }
                    Ok(())
                }).await?;
            }
        }

        let result_message = format!(
            r#"# Agent Work Merged

**Card:** {}
**Strategy:** {}
**Branch:** {}
**Worktree Deleted:** {}

The agent's work has been successfully merged back to the main branch."#,
            card_id, strategy, agent_branch, delete_worktree
        );

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
