use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::process::Command;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;

use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::tasks::storage;

static GIT_MERGE_LOCK: OnceLock<AMutex<()>> = OnceLock::new();
fn git_merge_lock() -> &'static AMutex<()> {
    GIT_MERGE_LOCK.get_or_init(|| AMutex::new(()))
}

pub struct ToolTaskMergeAgent;

impl ToolTaskMergeAgent {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskMergeAgent {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_merge_agent".to_string(),
            display_name: "Task Merge Agent".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Merge an agent's work back to the main branch and cleanup the worktree. The agent must have completed work on a card with an associated git branch and worktree.".to_string(),
            input_schema: json_schema_from_params(&[("card_id", "string", "Card ID whose agent branch to merge"), ("strategy", "string", "Merge strategy: 'merge' (default) or 'squash'"), ("delete_worktree", "boolean", "Delete worktree and branch after merge (default: true)")], &["card_id"]),
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
        let ccx_lock = ccx.lock().await;

        let is_planner = ccx_lock
            .task_meta
            .as_ref()
            .map(|m| m.role == "planner")
            .unwrap_or(false);

        if !is_planner {
            return Err("task_merge_agent can only be called by the task planner. \
                 Switch to the planner chat to merge agent work."
                .to_string());
        }

        let task_id = if let Some(id) = args.get("task_id").and_then(|v| v.as_str()) {
            id.to_string()
        } else if let Some(ref meta) = ccx_lock.task_meta {
            meta.task_id.clone()
        } else {
            return Err("Missing 'task_id' (and chat is not bound to a task)".to_string());
        };

        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'card_id'")?;

        let strategy = args
            .get("strategy")
            .and_then(|v| v.as_str())
            .unwrap_or("merge");

        let delete_worktree = match args.get("delete_worktree") {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => s.to_lowercase() == "true",
            _ => true,
        };

        if strategy != "merge" && strategy != "squash" {
            return Err(format!(
                "Invalid strategy '{}', must be 'merge' or 'squash'",
                strategy
            ));
        }

        let gcx = ccx_lock.global_context.clone();
        drop(ccx_lock);

        let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
        let workspace_root = project_dirs.first().ok_or("No workspace folder found")?;

        let is_git_repo = Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(workspace_root)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !is_git_repo {
            return Err("Workspace is not a git repository".to_string());
        }

        let board = storage::load_board(gcx.clone(), &task_id).await?;
        let card = board
            .get_card(card_id)
            .ok_or(format!("Card {} not found", card_id))?;

        let agent_branch = card
            .agent_branch
            .as_ref()
            .ok_or(format!("Card {} has no agent branch", card_id))?;
        let agent_worktree = card
            .agent_worktree
            .as_ref()
            .ok_or(format!("Card {} has no agent worktree", card_id))?;

        let task_meta = storage::load_task_meta(gcx.clone(), &task_id).await?;
        let base_branch = task_meta
            .base_branch
            .as_ref()
            .ok_or("Task has no base branch set")?;

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

        let commits_ahead_result = run_git(&[
            "rev-list",
            "--count",
            &format!("{}..{}", base_branch, agent_branch),
        ]);
        let commits_ahead = match commits_ahead_result {
            Ok(output) => output
                .trim()
                .parse::<u32>()
                .map_err(|e| format!("Failed to parse commits ahead count: {}", e))?,
            Err(e) => {
                return Err(format!(
                    "Failed to count commits ahead (base: {}, agent: {}): {}. \
                    Check that both branches exist and are valid.",
                    base_branch, agent_branch, e
                ));
            }
        };

        if commits_ahead == 0 {
            let worktree_status = if let Some(wt) = card.agent_worktree.as_ref() {
                Command::new("git")
                    .args(["status", "--porcelain"])
                    .current_dir(wt)
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            };

            let main_status = run_git(&["status", "--porcelain"]).unwrap_or_default();
            let main_dirty = !main_status.trim().is_empty();
            let worktree_dirty = !worktree_status.is_empty();

            let diagnostic = if main_dirty && !worktree_dirty {
                "Main workspace has uncommitted changes but agent worktree is clean. Agent likely edited files in the wrong directory."
            } else if worktree_dirty {
                "Agent worktree has uncommitted changes. The agent may have forgotten to commit, or task_agent_finish auto-commit failed."
            } else {
                "Both main workspace and agent worktree are clean. Agent may not have made any changes."
            };

            let mut cleanup_status = Vec::new();
            if delete_worktree && agent_branch != base_branch {
                crate::files_in_workspace::remove_folder(
                    gcx.clone(),
                    &std::path::PathBuf::from(agent_worktree),
                )
                .await;
                let _guard = git_merge_lock().lock().await;
                let worktree_removed =
                    run_git(&["worktree", "remove", agent_worktree, "--force"]).is_ok();
                let branch_deleted = run_git(&["branch", "-D", agent_branch]).is_ok();
                drop(_guard);

                if worktree_removed {
                    cleanup_status.push("worktree removed");
                }
                if branch_deleted {
                    cleanup_status.push("branch deleted");
                }

                if worktree_removed || branch_deleted {
                    let card_id_owned = card_id.to_string();
                    let clear_worktree = worktree_removed;
                    let clear_branch = branch_deleted;
                    let _ = storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
                        if let Some(card) = board.get_card_mut(&card_id_owned) {
                            if clear_branch {
                                card.agent_branch = None;
                            }
                            if clear_worktree {
                                card.agent_worktree = None;
                                card.agent_worktree_name = None;
                            }
                        }
                        Ok(())
                    })
                    .await;
                }
            }

            let cleanup_msg = if cleanup_status.is_empty() {
                "No cleanup performed.".to_string()
            } else {
                format!("Cleanup: {}.", cleanup_status.join(", "))
            };

            return Ok((false, vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(format!(
                    "# Nothing to Merge\n\n**Card:** {}\n**Branch:** {}\n**Commits ahead of base:** 0\n\n**Diagnostic:** {}\n\n{}",
                    card_id, agent_branch, diagnostic, cleanup_msg
                )),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })]));
        }

        let merge_in_progress = run_git(&["rev-parse", "-q", "--verify", "MERGE_HEAD"]).is_ok();
        if merge_in_progress {
            let status = run_git(&["status", "--porcelain"]).unwrap_or_default();
            let conflict_files: Vec<String> = status
                .lines()
                .filter(|l| {
                    let bytes = l.as_bytes();
                    bytes.len() >= 2
                        && (bytes[0] == b'U'
                            || bytes[1] == b'U'
                            || (bytes[0] == b'A' && bytes[1] == b'A')
                            || (bytes[0] == b'D' && bytes[1] == b'D'))
                })
                .filter_map(|l| l.get(3..).map(|s| s.to_string()))
                .collect();

            let conflict_msg = format!(
                r#"# Merge Already In Progress

A previous merge is still in progress with unresolved conflicts.

## Conflicting Files
{}

## How to Resolve

### Option 1: Resolve conflicts
1. Use `cat <file>` to see conflict markers (`<<<<<<<`, `=======`, `>>>>>>>`)
2. Use `update_textdoc` to resolve each conflict
3. Stage and commit: `git add -A && git commit -m "Resolved conflicts"`

### Option 2: Abort and retry
```
git merge --abort
```
Then call `task_merge_agent` again."#,
                if conflict_files.is_empty() {
                    "None detected (check `git status`)".to_string()
                } else {
                    conflict_files
                        .iter()
                        .map(|f| format!("- {}", f))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            );

            return Ok((
                false,
                vec![ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(conflict_msg),
                    tool_calls: None,
                    tool_call_id: tool_call_id.clone(),
                    ..Default::default()
                })],
            ));
        }

        let main_status = run_git(&["status", "--porcelain"]).unwrap_or_default();
        if !main_status.trim().is_empty() {
            return Err(
                "Main workspace has uncommitted changes. Please commit or stash before merging."
                    .to_string(),
            );
        }

        // Generate commit message before acquiring the lock: git diff base...agent is read-only
        // and produces the same content as git diff --cached after a squash merge.
        let diff =
            run_git(&["diff", &format!("{}...{}", base_branch, agent_branch)]).unwrap_or_default();
        let commit_msg =
            match crate::agentic::generate_commit_message::generate_commit_message_by_diff(
                gcx.clone(),
                &diff,
                &Some(card.title.clone()),
            )
            .await
            {
                Ok(msg) if !msg.trim().is_empty() => msg,
                _ => format!("Card {}: {}", card_id, card.title),
            };

        let _guard = git_merge_lock().lock().await;

        run_git(&["checkout", base_branch])
            .map_err(|e| format!("Failed to checkout base branch: {}", e))?;

        let merge_result = if strategy == "squash" {
            run_git(&["merge", "--squash", agent_branch])
        } else {
            run_git(&["merge", "--no-ff", agent_branch, "-m", &commit_msg])
        };

        if let Err(e) = merge_result {
            let status = run_git(&["status", "--porcelain"]).unwrap_or_default();
            let is_conflict_line = |l: &str| {
                let bytes = l.as_bytes();
                bytes.len() >= 2
                    && (bytes[0] == b'U'
                        || bytes[1] == b'U'
                        || (bytes[0] == b'A' && bytes[1] == b'A')
                        || (bytes[0] == b'D' && bytes[1] == b'D'))
            };
            let has_conflicts = status.lines().any(is_conflict_line);

            if has_conflicts {
                let conflict_files: Vec<String> = status
                    .lines()
                    .filter(|l| is_conflict_line(l))
                    .filter_map(|l| l.get(3..).map(|s| s.to_string()))
                    .collect();

                let conflict_msg = format!(
                    r#"# Merge Conflicts Detected

**Card:** {}
**Branch:** {} → {}
**Strategy:** {}

## Conflicting Files
{}

## Current State
The merge is **in progress** with conflict markers in the files above.

## How to Resolve

### Option 1: Resolve manually
1. Open each conflicting file
2. Look for conflict markers: `<<<<<<<`, `=======`, `>>>>>>>`
3. Edit to keep the correct code (remove markers)
4. Stage resolved files: `git add <file>`
5. Complete merge: `git commit -m "Resolved conflicts from {}"`

### Option 2: Accept one side entirely
- Keep base branch version: `git checkout --ours <file>`
- Keep agent branch version: `git checkout --theirs <file>`
- Then: `git add <file>` and `git commit`

### Option 3: Abort and retry
```
git merge --abort
```
Then investigate why conflicts occurred and create a fix card.

## Conflict Details
Use `cat <file>` to see conflict markers in each file."#,
                    card_id,
                    agent_branch,
                    base_branch,
                    strategy,
                    conflict_files
                        .iter()
                        .map(|f| format!("- {}", f))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    agent_branch
                );

                return Ok((
                    false,
                    vec![ContextEnum::ChatMessage(ChatMessage {
                        role: "tool".to_string(),
                        content: ChatContent::SimpleText(conflict_msg),
                        tool_calls: None,
                        tool_call_id: tool_call_id.clone(),
                        ..Default::default()
                    })],
                ));
            }
            return Err(format!("Merge failed: {}", e));
        }

        if strategy == "squash" {
            let commit_result = run_git(&["commit", "-m", &commit_msg]);
            if let Err(e) = commit_result {
                if !e.contains("nothing to commit") {
                    return Err(format!("Failed to commit squash merge: {}", e));
                }
            }
        }

        let (worktree_removed, branch_deleted) = if delete_worktree && agent_branch != base_branch {
            crate::files_in_workspace::remove_folder(
                gcx.clone(),
                &std::path::PathBuf::from(agent_worktree),
            )
            .await;
            let wr = run_git(&["worktree", "remove", agent_worktree, "--force"]).is_ok();
            let bd = run_git(&["branch", "-D", agent_branch]).is_ok();
            (wr, bd)
        } else {
            (false, false)
        };

        drop(_guard);

        if worktree_removed || branch_deleted {
            let card_id_owned = card_id.to_string();
            let clear_worktree = worktree_removed;
            let clear_branch = branch_deleted;
            let _ = storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
                if let Some(card) = board.get_card_mut(&card_id_owned) {
                    if clear_branch {
                        card.agent_branch = None;
                    }
                    if clear_worktree {
                        card.agent_worktree = None;
                        card.agent_worktree_name = None;
                    }
                }
                Ok(())
            })
            .await?;
        }

        let cleanup_info = if delete_worktree && agent_branch != base_branch {
            match (worktree_removed, branch_deleted) {
                (true, true) => "Worktree and branch cleaned up.".to_string(),
                (true, false) => "Worktree removed, branch deletion failed.".to_string(),
                (false, true) => "Worktree removal failed, branch deleted.".to_string(),
                (false, false) => "Cleanup failed (worktree and branch still exist).".to_string(),
            }
        } else {
            "No cleanup requested.".to_string()
        };

        let result_message = format!(
            r#"# Agent Work Merged

**Card:** {}
**Strategy:** {}
**Branch:** {} → {}
**Cleanup:** {}

The agent's work has been successfully merged back to the main branch."#,
            card_id, strategy, agent_branch, base_branch, cleanup_info
        );

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
