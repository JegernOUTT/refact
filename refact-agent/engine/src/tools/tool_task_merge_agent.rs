use std::collections::HashMap;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::global_context::GlobalContext;
use crate::tasks::storage;
use crate::worktrees::service::{worktree_merge_lock, WorktreeService};
use crate::worktrees::types::{MergeWorktreeRequest, MergeWorktreeResponse, WorktreeMergeStrategy};

fn git_merge_lock() -> &'static AMutex<()> {
    worktree_merge_lock()
}

fn strategy_from_str(strategy: &str) -> Result<WorktreeMergeStrategy, String> {
    match strategy {
        "merge" => Ok(WorktreeMergeStrategy::Merge),
        "squash" => Ok(WorktreeMergeStrategy::Squash),
        _ => Err(format!(
            "Invalid strategy '{}', must be 'merge' or 'squash'",
            strategy
        )),
    }
}

fn cleanup_summary(response: &MergeWorktreeResponse) -> String {
    match response.cleanup.as_ref() {
        Some(cleanup) => format!(
            "worktree_deleted={}, branch_deleted={}, registry_deleted={}, affected_references={}",
            cleanup.worktree_deleted,
            cleanup.branch_deleted,
            cleanup.registry_deleted,
            response.affected_reference_count
        ),
        None => format!(
            "no cleanup requested, affected_references={}",
            response.affected_reference_count
        ),
    }
}

fn append_post_merge_check_message(
    message: &mut String,
    result: &crate::chat::post_merge_check::PostMergeCheckResult,
) {
    if !result.checked {
        if let Some(reason) = result.skipped_reason.as_deref() {
            message.push_str(&format!("\n\n**Post-merge check:** skipped ({})", reason));
        }
        return;
    }
    if result.auto_reverted {
        message.push_str(&format!(
            "\n\n## Post-merge regression detected\n\n**Verification:** `{}` failed{}\n**Auto-reverted:** true\n**Merge commit:** {}\n**Revert commit:** {}\n**Fix card:** {}\n\n{}",
            result.command.as_deref().unwrap_or("unknown"),
            result
                .exit_code
                .map(|code| format!(" with exit code {}", code))
                .unwrap_or_default(),
            result.merge_commit.as_deref().unwrap_or("unknown"),
            result.revert_commit.as_deref().unwrap_or("unknown"),
            result.fix_card_id.as_deref().unwrap_or("created"),
            crate::chat::post_merge_check::first_error_line(&result.output_tail)
        ));
    } else if let Some(reason) = result.revert_skipped_reason.as_deref() {
        message.push_str(&format!(
            "\n\n## Post-merge regression detected\n\n**Verification:** `{}` failed{}\n**Auto-reverted:** false ({})\n**Merge commit:** {}\n**Fix card:** {}\n\n{}",
            result.command.as_deref().unwrap_or("unknown"),
            result
                .exit_code
                .map(|code| format!(" with exit code {}", code))
                .unwrap_or_default(),
            reason,
            result.merge_commit.as_deref().unwrap_or("unknown"),
            result.fix_card_id.as_deref().unwrap_or("created"),
            crate::chat::post_merge_check::first_error_line(&result.output_tail)
        ));
    } else {
        message.push_str(&format!(
            "\n\n**Post-merge check:** `{}` passed",
            result.command.as_deref().unwrap_or("unknown")
        ));
    }
}

fn merge_response_message(
    card_id: &str,
    response: &MergeWorktreeResponse,
    post_merge_check: Option<&crate::chat::post_merge_check::PostMergeCheckResult>,
) -> String {
    if response.conflict.is_some() || response.status == "nothing_to_merge" {
        let _ = post_merge_check;
    }
    if let Some(conflict) = response.conflict.as_ref() {
        return format!(
            "# Merge Conflicts Detected\n\n**Card:** {}\n**Branch:** {} → {}\n**Strategy:** {}\n**Aborted:** {}\n\n## Conflicting Files\n{}\n\n{}",
            card_id,
            response.source_branch,
            response.target_branch,
            response.strategy,
            conflict.aborted,
            if conflict.files.is_empty() {
                "None detected".to_string()
            } else {
                conflict
                    .files
                    .iter()
                    .map(|file| format!("- {}", file))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            conflict.instructions
        );
    }
    if response.status == "nothing_to_merge" {
        return format!(
            "# Nothing to Merge\n\n**Card:** {}\n**Branch:** {} → {}\n**Commits ahead of base:** 0\n\nCleanup: {}.",
            card_id,
            response.source_branch,
            response.target_branch,
            cleanup_summary(response)
        );
    }
    let mut message = format!(
        "# Agent Work Merged\n\n**Card:** {}\n**Strategy:** {}\n**Branch:** {} → {}\n**Cleanup:** {}\n\nThe agent's work has been successfully merged back to the target branch.",
        card_id,
        response.strategy,
        response.source_branch,
        response.target_branch,
        cleanup_summary(response)
    );
    if let Some(result) = post_merge_check {
        append_post_merge_check_message(&mut message, result);
    }
    message
}

fn parse_bool_arg(args: &HashMap<String, Value>, name: &str) -> Result<bool, String> {
    match args.get(name) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(Value::String(value)) => Ok(value.eq_ignore_ascii_case("true")),
        Some(_) => Err(format!("{} must be a boolean", name)),
        None => Ok(false),
    }
}

fn parse_force(args: &HashMap<String, Value>) -> Result<bool, String> {
    parse_bool_arg(args, "force")
}

fn parse_auto_revert(args: &HashMap<String, Value>) -> Result<bool, String> {
    parse_bool_arg(args, "auto_revert")
}

fn parse_auto_revert_timeout(args: &HashMap<String, Value>) -> Result<Duration, String> {
    let Some(value) = args.get("auto_revert_timeout_secs") else {
        return Ok(Duration::from_secs(
            crate::chat::post_merge_check::DEFAULT_POST_MERGE_CHECK_TIMEOUT_SECS,
        ));
    };
    let secs = match value {
        Value::Number(number) => number
            .as_u64()
            .ok_or_else(|| "auto_revert_timeout_secs must be a positive integer".to_string())?,
        Value::String(text) => text
            .parse::<u64>()
            .map_err(|_| "auto_revert_timeout_secs must be a positive integer".to_string())?,
        _ => return Err("auto_revert_timeout_secs must be a positive integer".to_string()),
    };
    if secs == 0 {
        return Err("auto_revert_timeout_secs must be greater than zero".to_string());
    }
    Ok(Duration::from_secs(secs))
}

fn verifier_merge_block_message(card_id: &str, concerns: &[String]) -> String {
    let rendered = if concerns.is_empty() {
        "- verifier failed without concerns".to_string()
    } else {
        concerns
            .iter()
            .map(|concern| format!("- {}", concern))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "Card {} verifier failed. Refusing merge unless force=true. Concerns:\n{}",
        card_id, rendered
    )
}

fn verifier_force_warning(card_id: &str, concerns: &[String]) -> String {
    let rendered = if concerns.is_empty() {
        "- verifier failed without concerns".to_string()
    } else {
        concerns
            .iter()
            .map(|concern| format!("- {}", concern))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "Card {} verifier failed and requires human review. Merge is continuing only because force=true. Concerns:\n{}",
        card_id, rendered
    )
}

fn append_verifier_warning(message: &mut String, warning: Option<&str>) {
    if let Some(warning) = warning {
        message.push_str("\n\n## Verifier Warning\n\n");
        message.push_str(warning);
    }
}

fn ensure_verifier_allows_merge(
    card: &crate::tasks::types::BoardCard,
    force: bool,
) -> Result<Option<String>, String> {
    let Some(report) = card.verifier_report.as_ref() else {
        return Ok(None);
    };
    if report.passed {
        return Ok(None);
    }
    if force {
        return Ok((report.recommendation == "human-review")
            .then(|| verifier_force_warning(&card.id, &report.concerns)));
    }
    Err(verifier_merge_block_message(&card.id, &report.concerns))
}

async fn clear_board_mirrors_after_registered_merge(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
    response: &MergeWorktreeResponse,
) -> Result<(), String> {
    let Some(cleanup) = response.cleanup.as_ref() else {
        return Ok(());
    };
    if !cleanup.worktree_deleted && !cleanup.branch_deleted && !cleanup.registry_deleted {
        return Ok(());
    }
    let card_id_owned = card_id.to_string();
    let clear_worktree = cleanup.worktree_deleted || cleanup.registry_deleted;
    let clear_branch = cleanup.branch_deleted;
    storage::update_board_atomic(gcx, task_id, move |board| {
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
    .await
    .map(|_| ())
}

async fn merge_registered_task_worktree(
    gcx: Arc<GlobalContext>,
    workspace_root: &std::path::Path,
    task_id: &str,
    card_id: &str,
    strategy: &str,
    tool_call_id: &str,
    commit_message_override: Option<String>,
    force: bool,
    auto_revert: bool,
    auto_revert_timeout: Duration,
) -> Result<Option<(bool, Vec<ContextEnum>)>, String> {
    let board = storage::load_board(gcx.clone(), task_id).await?;
    let Some(card) = board.get_card(card_id) else {
        return Err(format!("Card {} not found", card_id));
    };
    let verifier_warning = ensure_verifier_allows_merge(card, force)?;
    let Some(worktree_id) = card.agent_worktree_name.clone() else {
        return Ok(None);
    };
    let task_meta = storage::load_task_meta(gcx.clone(), task_id).await?;
    let target_branch = task_meta
        .base_branch
        .clone()
        .ok_or("Task has no base branch set")?;
    let cache_dir = gcx.cache_dir.clone();
    let service = WorktreeService::new(cache_dir, workspace_root.to_path_buf())?;
    let diff = service.diff_worktree(&worktree_id).await.ok();
    let changed_files = diff
        .as_ref()
        .map(|diff| {
            diff.files
                .iter()
                .map(|file| file.path.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let commit_message = match commit_message_override {
        Some(message) if !message.trim().is_empty() => message,
        _ => {
            let diff_text = diff
                .as_ref()
                .map(|diff| diff.patch.clone())
                .unwrap_or_default();
            match crate::agentic::generate_commit_message::generate_commit_message_by_diff(
                gcx.clone(),
                &diff_text,
                &Some(card.title.clone()),
            )
            .await
            {
                Ok(message) if !message.trim().is_empty() => message,
                _ => format!("Card {}: {}", card_id, card.title),
            }
        }
    };
    let response = service
        .merge_worktree(
            &worktree_id,
            MergeWorktreeRequest {
                strategy: strategy_from_str(strategy)?,
                delete_after_merge: true,
                include_uncommitted: false,
                target_branch: Some(target_branch),
                commit_message: Some(commit_message),
                generate_commit_message: false,
            },
        )
        .await?;
    if response.merged && !changed_files.is_empty() {
        let _ = crate::chat::task_agent_monitor::append_card_target_files(
            crate::app_state::AppState::from_gcx(gcx.clone()).await,
            task_id,
            card_id,
            changed_files,
        )
        .await;
    }
    clear_board_mirrors_after_registered_merge(gcx.clone(), task_id, card_id, &response).await?;
    let post_merge_check = match response.merge_commit.as_ref() {
        Some(merge_commit) if response.merged && auto_revert => Some(
            crate::chat::post_merge_check::post_merge_check(
                gcx,
                crate::chat::post_merge_check::PostMergeCheckRequest {
                    task_id: task_id.to_string(),
                    card_id: card_id.to_string(),
                    workspace_root: workspace_root.to_path_buf(),
                    enabled: auto_revert,
                    timeout: auto_revert_timeout,
                    expected_merge_commit: merge_commit.clone(),
                },
            )
            .await?,
        ),
        _ => None,
    };
    let mut message = merge_response_message(card_id, &response, post_merge_check.as_ref());
    append_verifier_warning(&mut message, verifier_warning.as_deref());
    Ok(Some((
        false,
        vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(message),
            tool_calls: None,
            tool_call_id: tool_call_id.to_string(),
            ..Default::default()
        })],
    )))
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
            description: "Merge an agent's work back to the main branch and always cleanup the worktree after a successful merge or no-op merged state. The agent must have completed work on a card with an associated git branch and worktree.".to_string(),
            input_schema: json_schema_from_params(&[("card_id", "string", "Card ID whose agent branch to merge"), ("strategy", "string", "Merge strategy: 'merge' (default) or 'squash'"), ("force", "boolean", "Override a failed verifier_report and merge anyway"), ("auto_revert", "boolean", "Opt in to post-merge verification and automatic git revert on deterministic regression"), ("auto_revert_timeout_secs", "integer", "Timeout for post-merge verification and revert commands, default 300 seconds")], &["card_id"]),
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
        let force = parse_force(args)?;
        let auto_revert = parse_auto_revert(args)?;
        let auto_revert_timeout = parse_auto_revert_timeout(args)?;

        if strategy != "merge" && strategy != "squash" {
            return Err(format!(
                "Invalid strategy '{}', must be 'merge' or 'squash'",
                strategy
            ));
        }

        let gcx = ccx_lock.app.gcx.clone();
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
        let verifier_warning = ensure_verifier_allows_merge(card, force)?;

        let task_meta = storage::load_task_meta(gcx.clone(), &task_id).await?;
        let base_branch = task_meta
            .base_branch
            .as_ref()
            .ok_or("Task has no base branch set")?;

        if card.agent_worktree_name.is_some() {
            if let Some(result) = merge_registered_task_worktree(
                gcx.clone(),
                workspace_root,
                &task_id,
                card_id,
                strategy,
                tool_call_id,
                None,
                force,
                auto_revert,
                auto_revert_timeout,
            )
            .await?
            {
                return Ok(result);
            }
        }

        let agent_branch = card
            .agent_branch
            .as_ref()
            .ok_or(format!("Card {} has no agent branch", card_id))?;
        let agent_worktree = card
            .agent_worktree
            .as_ref()
            .ok_or(format!("Card {} has no agent worktree", card_id))?;

        let changed_files = crate::chat::task_agent_monitor::git_diff_name_only(
            std::path::Path::new(agent_worktree),
            base_branch,
            agent_branch,
        );

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
            if agent_branch != base_branch {
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

            let mut message = format!(
                "# Nothing to Merge\n\n**Card:** {}\n**Branch:** {}\n**Commits ahead of base:** 0\n\n**Diagnostic:** {}\n\n{}",
                card_id, agent_branch, diagnostic, cleanup_msg
            );
            append_verifier_warning(&mut message, verifier_warning.as_deref());

            return Ok((
                false,
                vec![ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(message),
                    tool_calls: None,
                    tool_call_id: tool_call_id.clone(),
                    ..Default::default()
                })],
            ));
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

        let merge_commit = run_git(&["rev-parse", "HEAD"])
            .map(|head| head.trim().to_string())
            .unwrap_or_default();

        let (worktree_removed, branch_deleted) = if agent_branch != base_branch {
            let wr = run_git(&["worktree", "remove", agent_worktree, "--force"]).is_ok();
            let bd = run_git(&["branch", "-D", agent_branch]).is_ok();
            (wr, bd)
        } else {
            (false, false)
        };

        drop(_guard);

        let post_merge_check = if auto_revert && !merge_commit.is_empty() {
            Some(
                crate::chat::post_merge_check::post_merge_check(
                    gcx.clone(),
                    crate::chat::post_merge_check::PostMergeCheckRequest {
                        task_id: task_id.clone(),
                        card_id: card_id.to_string(),
                        workspace_root: workspace_root.to_path_buf(),
                        enabled: true,
                        timeout: auto_revert_timeout,
                        expected_merge_commit: merge_commit,
                    },
                )
                .await?,
            )
        } else {
            None
        };

        let _ = crate::chat::task_agent_monitor::append_card_target_files(
            crate::app_state::AppState::from_gcx(gcx.clone()).await,
            &task_id,
            card_id,
            changed_files,
        )
        .await;

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

        let cleanup_info = if agent_branch != base_branch {
            match (worktree_removed, branch_deleted) {
                (true, true) => "Worktree and branch cleaned up.".to_string(),
                (true, false) => "Worktree removed, branch deletion failed.".to_string(),
                (false, true) => "Worktree removal failed, branch deleted.".to_string(),
                (false, false) => "Cleanup failed (worktree and branch still exist).".to_string(),
            }
        } else {
            "Cleanup skipped because agent branch matches base branch.".to_string()
        };

        let mut result_message = format!(
            r#"# Agent Work Merged

**Card:** {}
**Strategy:** {}
**Branch:** {} → {}
**Cleanup:** {}

The agent's work has been successfully merged back to the main branch."#,
            card_id, strategy, agent_branch, base_branch, cleanup_info
        );
        append_verifier_warning(&mut result_message, verifier_warning.as_deref());
        if let Some(post_merge_check) = post_merge_check.as_ref() {
            append_post_merge_check_message(&mut result_message, post_merge_check);
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

#[cfg(test)]
mod worktree_merge_tool_tests {
    use super::*;
    use crate::tasks::types::{BoardCard, TaskBoard, TaskMeta, TaskStatus, VerifierReport};
    use crate::worktrees::types::CreateWorktreeRequest;
    use std::path::Path;

    fn run_git(cwd: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap_or_else(|e| panic!("failed to run git {:?}: {}", args, e));
        if !output.status.success() {
            panic!(
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    fn init_repo(root: &Path) {
        run_git(root, &["init"]);
        run_git(root, &["checkout", "-b", "main"]);
        run_git(root, &["config", "core.autocrlf", "false"]);
        run_git(root, &["config", "user.email", "test@example.com"]);
        run_git(root, &["config", "user.name", "Test User"]);
        std::fs::write(root.join("file.txt"), "hello\n").unwrap();
        std::fs::write(root.join(".gitignore"), ".refact/\n").unwrap();
        run_git(root, &["add", "file.txt", ".gitignore"]);
        run_git(root, &["commit", "-m", "initial"]);
    }

    fn commit_file(root: &Path, file: &str, content: &str, message: &str) {
        std::fs::write(root.join(file), content).unwrap();
        run_git(root, &["add", file]);
        run_git(root, &["commit", "-m", message]);
    }

    async fn set_workspace(gcx: Arc<GlobalContext>, root: &Path) {
        let root = root.canonicalize().unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root];
    }

    fn test_card(worktree_id: &str, branch: &str, root: &Path) -> BoardCard {
        BoardCard {
            id: "T-1".to_string(),
            title: "Card T-1".to_string(),
            column: "done".to_string(),
            priority: "P1".to_string(),
            depends_on: vec![],
            instructions: String::new(),
            assignee: Some("agent-1".to_string()),
            agent_chat_id: Some("agent-chat-1".to_string()),
            status_updates: vec![],
            comments: vec![],
            final_report: Some("done".to_string()),
            final_report_structured: None,
            verifier_report: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            started_at: None,
            last_heartbeat_at: None,
            completed_at: Some(chrono::Utc::now().to_rfc3339()),
            agent_branch: Some(branch.to_string()),
            agent_worktree: Some(root.to_string_lossy().to_string()),
            agent_worktree_name: Some(worktree_id.to_string()),
            ab_variants: None,
            team_members: vec![],
            target_files: vec![],
            scope_guard_mode: Default::default(),
        }
    }

    fn launch_failure_verifier_report() -> VerifierReport {
        VerifierReport {
            passed: false,
            concerns: vec![
                "Verifier failed to launch; human review recommended: no verifier model"
                    .to_string(),
            ],
            recommendation: "human-review".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn merge_agent_refuses_when_verifier_failed_without_force() {
        let mut card = test_card("wt", "agent", Path::new("/tmp/worktree"));
        card.verifier_report = Some(VerifierReport {
            passed: false,
            concerns: vec!["cargo test failed".to_string()],
            recommendation: "fix-needed".to_string(),
            ..Default::default()
        });

        let err = ensure_verifier_allows_merge(&card, false).unwrap_err();

        assert!(err.contains("verifier failed"));
        assert!(err.contains("cargo test failed"));
        assert!(err.contains("force=true"));
    }

    #[test]
    fn merge_agent_proceeds_when_force_true() {
        let mut card = test_card("wt", "agent", Path::new("/tmp/worktree"));
        card.verifier_report = Some(VerifierReport {
            passed: false,
            concerns: vec!["cargo test failed".to_string()],
            recommendation: "fix-needed".to_string(),
            ..Default::default()
        });

        assert!(ensure_verifier_allows_merge(&card, true).is_ok());
    }

    #[test]
    fn merge_agent_blocks_when_verifier_launch_failed() {
        let mut card = test_card("wt", "agent", Path::new("/tmp/worktree"));
        card.verifier_report = Some(launch_failure_verifier_report());

        let err = ensure_verifier_allows_merge(&card, false).unwrap_err();

        assert!(err.contains("verifier failed"));
        assert!(err.contains("Verifier failed to launch"));
        assert!(err.contains("force=true"));
    }

    #[test]
    fn merge_agent_allows_card_without_verifier_report() {
        let card = test_card("wt", "agent", Path::new("/tmp/worktree"));

        assert!(ensure_verifier_allows_merge(&card, false).is_ok());
    }

    #[test]
    fn merge_agent_auto_revert_schema_and_args_are_opt_in() {
        let schema = ToolTaskMergeAgent::new().tool_description().input_schema;
        assert!(schema["properties"].get("auto_revert").is_some());
        assert!(schema["properties"]
            .get("auto_revert_timeout_secs")
            .is_some());
        let empty = HashMap::new();
        assert!(!parse_auto_revert(&empty).unwrap());
        assert_eq!(
            parse_auto_revert_timeout(&empty).unwrap(),
            Duration::from_secs(
                crate::chat::post_merge_check::DEFAULT_POST_MERGE_CHECK_TIMEOUT_SECS
            )
        );
        let args = HashMap::from([
            ("auto_revert".to_string(), Value::Bool(true)),
            (
                "auto_revert_timeout_secs".to_string(),
                Value::Number(120.into()),
            ),
        ]);
        assert!(parse_auto_revert(&args).unwrap());
        assert_eq!(
            parse_auto_revert_timeout(&args).unwrap(),
            Duration::from_secs(120)
        );
    }

    #[tokio::test]
    async fn merge_agent_allows_with_force_when_launch_failed() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        let cache_dir = gcx.cache_dir.clone();
        let service = WorktreeService::new(cache_dir, source.canonicalize().unwrap()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/task/task-1/card/T-1/agent".to_string()),
                kind: Some("task_agent".to_string()),
                task_id: Some("task-1".to_string()),
                card_id: Some("T-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                chat_id: Some("agent-chat-1".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let branch = created.worktree.meta.branch.clone().unwrap();
        let root = created.worktree.meta.root.clone();
        commit_file(
            &root,
            "file.txt",
            "merged after human review\n",
            "agent change",
        );
        let mut card = test_card(&created.worktree.meta.id, &branch, &root);
        card.verifier_report = Some(launch_failure_verifier_report());
        write_task(gcx.clone(), &source, card).await;

        let result = merge_registered_task_worktree(
            gcx,
            &source.canonicalize().unwrap(),
            "task-1",
            "T-1",
            "squash",
            "tool-call",
            Some("task merge".to_string()),
            true,
            false,
            Duration::from_secs(5),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "merged after human review\n"
        );
        let ContextEnum::ChatMessage(message) = &result.1[0] else {
            panic!("expected chat message")
        };
        let ChatContent::SimpleText(text) = &message.content else {
            panic!("expected simple text")
        };
        assert!(text.contains("Verifier Warning"));
        assert!(text.contains("requires human review"));
        assert!(text.contains("force=true"));
        assert!(text.contains("Verifier failed to launch"));
    }

    async fn write_task(gcx: Arc<GlobalContext>, source: &Path, card: BoardCard) {
        let task_dir = source.join(".refact").join("tasks").join("task-1");
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let meta = TaskMeta {
            schema_version: 1,
            id: "task-1".to_string(),
            name: "Task".to_string(),
            status: TaskStatus::Active,
            created_at: now.clone(),
            updated_at: now,
            cards_total: 1,
            cards_done: 1,
            cards_failed: 0,
            agents_active: 0,
            base_branch: Some("main".to_string()),
            base_commit: None,
            default_agent_model: None,
            is_name_generated: false,
            last_agents_summary_at: None,
            planner_session_state: None,
        };
        let mut board = TaskBoard::default();
        board.cards.push(card);
        tokio::fs::write(
            task_dir.join("meta.yaml"),
            serde_yaml::to_string(&meta).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(
            task_dir.join("board.yaml"),
            serde_yaml::to_string(&board).unwrap(),
        )
        .await
        .unwrap();
        set_workspace(gcx, source).await;
    }

    #[tokio::test]
    async fn worktree_merge_task_merge_agent_uses_service_and_clears_board_mirrors() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        let cache_dir = gcx.cache_dir.clone();
        let service = WorktreeService::new(cache_dir, source.canonicalize().unwrap()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/task/task-1/card/T-1/agent".to_string()),
                kind: Some("task_agent".to_string()),
                task_id: Some("task-1".to_string()),
                card_id: Some("T-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                chat_id: Some("agent-chat-1".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let branch = created.worktree.meta.branch.clone().unwrap();
        let root = created.worktree.meta.root.clone();
        commit_file(&root, "file.txt", "merged by task tool\n", "agent change");
        write_task(
            gcx.clone(),
            &source,
            test_card(&created.worktree.meta.id, &branch, &root),
        )
        .await;

        let result = merge_registered_task_worktree(
            gcx.clone(),
            &source.canonicalize().unwrap(),
            "task-1",
            "T-1",
            "squash",
            "tool-call",
            Some("task merge".to_string()),
            false,
            false,
            Duration::from_secs(5),
        )
        .await
        .unwrap();

        assert!(result.is_some());
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "merged by task tool\n"
        );
        assert!(!root.exists());
        assert!(service
            .get_worktree(&created.worktree.meta.id)
            .await
            .is_err());
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert!(card.agent_branch.is_none());
        assert!(card.agent_worktree.is_none());
        assert!(card.agent_worktree_name.is_none());
        assert!(card.target_files.contains(&"file.txt".to_string()));
    }
}
