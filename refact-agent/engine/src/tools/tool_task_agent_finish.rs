use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::path::{Path, PathBuf};
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;
use async_trait::async_trait;
use chrono::Utc;

use crate::global_context::GlobalContext;
use crate::agentic::generate_commit_message::generate_commit_message_by_diff;

use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::tasks::storage;
use crate::tasks::types::{BoardCard, StatusUpdate};
use crate::worktrees::types::WorktreeMeta;

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

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedAgentWorktree {
    root: PathBuf,
    branch: Option<String>,
    name: Option<String>,
}

fn resolve_agent_worktree(
    thread_worktree: Option<WorktreeMeta>,
    card: &BoardCard,
) -> Option<ResolvedAgentWorktree> {
    if let Some(meta) = thread_worktree {
        return Some(ResolvedAgentWorktree {
            root: meta.root,
            branch: meta.branch,
            name: Some(meta.id),
        });
    }
    card.agent_worktree
        .as_ref()
        .map(|root| ResolvedAgentWorktree {
            root: PathBuf::from(root),
            branch: card.agent_branch.clone(),
            name: card.agent_worktree_name.clone(),
        })
}

static FINISH_LOCKS: OnceLock<AMutex<HashMap<String, Arc<AMutex<()>>>>> = OnceLock::new();

fn get_finish_locks() -> &'static AMutex<HashMap<String, Arc<AMutex<()>>>> {
    FINISH_LOCKS.get_or_init(|| AMutex::new(HashMap::new()))
}

async fn get_finish_lock(task_id: &str, card_id: &str) -> Arc<AMutex<()>> {
    let mut locks = get_finish_locks().lock().await;
    locks
        .entry(format!("{}:{}", task_id, card_id))
        .or_insert_with(|| Arc::new(AMutex::new(())))
        .clone()
}

fn git_failure_details(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    match (stderr.is_empty(), stdout.is_empty()) {
        (false, false) => format!("{}\n{}", stderr, stdout),
        (false, true) => stderr,
        (true, false) => stdout,
        (true, true) => format!("exit status {}", output.status),
    }
}

async fn git_output_checked(
    worktree_path: &Path,
    args: &[&str],
    action: &str,
) -> Result<std::process::Output, String> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(worktree_path)
        .output()
        .await
        .map_err(|e| {
            format!(
                "Failed to run git {} in worktree '{}': {}",
                action,
                worktree_path.display(),
                e
            )
        })?;

    if !output.status.success() {
        return Err(format!(
            "git {} failed in worktree '{}': {}",
            action,
            worktree_path.display(),
            git_failure_details(&output)
        ));
    }

    Ok(output)
}

async fn validate_git_worktree(worktree_path: &Path) -> Result<(), String> {
    if !worktree_path.exists() {
        return Err(format!(
            "Assigned worktree path '{}' does not exist",
            worktree_path.display()
        ));
    }
    if !worktree_path.is_dir() {
        return Err(format!(
            "Assigned worktree path '{}' is not a directory",
            worktree_path.display()
        ));
    }

    let output = tokio::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(worktree_path)
        .output()
        .await
        .map_err(|e| {
            format!(
                "Failed to validate git worktree '{}': {}",
                worktree_path.display(),
                e
            )
        })?;

    if !output.status.success() {
        return Err(format!(
            "Assigned worktree path '{}' is not a git worktree/repo: {}",
            worktree_path.display(),
            git_failure_details(&output)
        ));
    }

    let inside_work_tree = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if inside_work_tree != "true" {
        return Err(format!(
            "Assigned worktree path '{}' is not inside a git worktree",
            worktree_path.display()
        ));
    }

    Ok(())
}

async fn auto_commit_worktree(
    gcx: Arc<GlobalContext>,
    worktree_path: &Path,
    card_id: &str,
    card_title: &str,
) -> Result<Option<String>, String> {
    auto_commit_worktree_with_message(gcx, worktree_path, card_id, card_title, None).await
}

async fn auto_commit_worktree_with_message(
    gcx: Arc<GlobalContext>,
    worktree_path: &Path,
    card_id: &str,
    card_title: &str,
    commit_msg_override: Option<String>,
) -> Result<Option<String>, String> {
    validate_git_worktree(worktree_path).await?;

    let status_output =
        git_output_checked(worktree_path, &["status", "--porcelain"], "status").await?;

    let status = String::from_utf8_lossy(&status_output.stdout);
    if status.trim().is_empty() {
        return Ok(None);
    }

    git_output_checked(worktree_path, &["add", "-A"], "add").await?;

    let diff_output =
        git_output_checked(worktree_path, &["diff", "--cached"], "diff --cached").await?;
    let diff = String::from_utf8_lossy(&diff_output.stdout).to_string();

    let commit_msg = match commit_msg_override {
        Some(msg) if !msg.trim().is_empty() => msg,
        _ => match generate_commit_message_by_diff(gcx, &diff, &Some(card_title.to_string())).await
        {
            Ok(msg) if !msg.trim().is_empty() => msg,
            _ => format!("Card {}: {}", card_id, card_title),
        },
    };

    let commit_output = tokio::process::Command::new("git")
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
        .await
        .map_err(|e| {
            format!(
                "Failed to commit in worktree '{}': {}",
                worktree_path.display(),
                e
            )
        })?;

    if !commit_output.status.success() {
        let stderr = String::from_utf8_lossy(&commit_output.stderr);
        if stderr.contains("nothing to commit") {
            return Ok(None);
        }
        return Err(format!(
            "git commit failed in worktree '{}': {}",
            worktree_path.display(),
            git_failure_details(&commit_output)
        ));
    }

    let rev_output =
        git_output_checked(worktree_path, &["rev-parse", "HEAD"], "rev-parse HEAD").await?;

    let commit_hash = String::from_utf8_lossy(&rev_output.stdout)
        .trim()
        .to_string();
    if commit_hash.is_empty() {
        return Err(format!(
            "git rev-parse HEAD returned empty output in worktree '{}'",
            worktree_path.display()
        ));
    }
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
        let planner_chat_id = ccx
            .lock()
            .await
            .task_meta
            .as_ref()
            .and_then(|meta| meta.planner_chat_id.clone());

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

        let gcx = ccx.lock().await.app.gcx.clone();
        let finish_lock = get_finish_lock(&task_id, &card_id).await;
        let _finish_guard = finish_lock.lock().await;

        let _ =
            crate::chat::task_agent_monitor::update_card_heartbeat(crate::app_state::AppState::from_gcx(gcx.clone()).await, &task_id, &card_id)
                .await;

        let board_pre = storage::load_board(gcx.clone(), &task_id).await?;
        let card_pre = board_pre
            .get_card(&card_id)
            .ok_or(format!("Card {} not found", card_id))?;
        if card_pre.column == "done" || card_pre.column == "failed" {
            return Err(format!(
                "Card {} is already in '{}' column. Cannot finish twice.",
                card_id, card_pre.column
            ));
        }
        let thread_worktree = ccx.lock().await.execution_scope_worktree();
        let resolved_worktree = resolve_agent_worktree(thread_worktree, card_pre);
        let card_title_for_commit = card_pre.title.clone();

        let commit_result = if success {
            if let Some(ref worktree) = resolved_worktree {
                match auto_commit_worktree(
                    gcx.clone(),
                    &worktree.root,
                    &card_id,
                    &card_title_for_commit,
                )
                .await
                {
                    Ok(hash) => hash,
                    Err(e) => {
                        return Err(format!(
                            "Auto-commit failed in worktree '{}': {}. Please ensure your changes are committed before calling task_agent_finish(success=true). \
                            You can run `git add -A && git commit -m 'your message'` in the worktree, or investigate the error.",
                            worktree.root.display(),
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
            if let Some(ref worktree) = resolved_worktree {
                if let Some(branch) = worktree.branch.clone() {
                    let worktree_root = worktree.root.to_string_lossy().to_string();
                    let _diff = crate::chat::task_agent_monitor::cleanup_failed_agent_worktree(
                        crate::app_state::AppState::from_gcx(gcx.clone()).await,
                        &worktree_root,
                        &branch,
                        worktree.name.as_deref(),
                    )
                    .await;
                    let card_id_clear = card_id.clone();
                    let _ = storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
                        if let Some(c) = board.get_card_mut(&card_id_clear) {
                            c.agent_worktree = None;
                            c.agent_branch = None;
                            c.agent_worktree_name = None;
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
                    "✅ **Card Completed: {}**\n\n**Report:**\n{}\n\nPlanner notified. Other agents are still running.",
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
                    "❌ **Card Failed: {}**\n\n**Reason:**\n{}\n\nPlanner notified. Other agents are still running.",
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

        let notify_error = crate::chat::task_agent_monitor::notify_planner_agents_finished(
            crate::app_state::AppState::from_gcx(gcx.clone()).await,
            &task_id,
            &board,
            all_finished,
            planner_chat_id.as_deref(),
        )
        .await
        .err();
        if let Some(ref error) = notify_error {
            tracing::warn!(
                "Agent finished card {}, but planner notification failed: {}",
                card_id,
                error
            );
        }

        {
            let ccx_lock = ccx.lock().await;
            ccx_lock.abort_flag.store(true, Ordering::SeqCst);
        }

        let result_message = if let Some(error) = notify_error {
            format!(
                "{}\n\n⚠️ Planner notification failed: {}",
                result_message, error
            )
        } else {
            result_message
        };

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
mod tests {
    use super::*;
    use crate::tasks::types::BoardCard;

    fn run_git(cwd: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
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
        run_git(root, &["config", "user.email", "test@example.com"]);
        run_git(root, &["config", "user.name", "Test User"]);
        std::fs::write(root.join("file.txt"), "hello\n").unwrap();
        run_git(root, &["add", "file.txt"]);
        run_git(root, &["commit", "-m", "initial"]);
    }

    fn test_card(worktree: Option<String>) -> BoardCard {
        BoardCard {
            id: "T-1".to_string(),
            title: "Card T-1".to_string(),
            column: "doing".to_string(),
            priority: "P1".to_string(),
            depends_on: vec![],
            instructions: String::new(),
            assignee: Some("agent-1".to_string()),
            agent_chat_id: Some("agent-chat-1".to_string()),
            status_updates: vec![],
            final_report: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: Some(Utc::now().to_rfc3339()),
            last_heartbeat_at: None,
            completed_at: None,
            agent_branch: Some("legacy-branch".to_string()),
            agent_worktree: worktree,
            agent_worktree_name: Some("legacy-id".to_string()),
            target_files: vec![],
        }
    }

    fn sample_worktree_meta(temp: &Path) -> WorktreeMeta {
        let root = temp.join("thread-worktree");
        let source = temp.join("source");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&source).unwrap();
        WorktreeMeta {
            id: "thread-id".to_string(),
            kind: "task_agent".to_string(),
            root,
            source_workspace_root: source.clone(),
            repo_root: source,
            branch: Some("thread-branch".to_string()),
            base_branch: Some("main".to_string()),
            base_commit: Some("base".to_string()),
            task_id: Some("task-1".to_string()),
            card_id: Some("T-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            enforce: true,
        }
    }

    async fn test_gcx() -> Arc<GlobalContext> {
        crate::global_context::tests::make_test_gcx().await
    }

    #[test]
    fn task_spawn_agent_finish_prefers_thread_worktree_over_board_mirror() {
        let temp = tempfile::tempdir().unwrap();
        let meta = sample_worktree_meta(temp.path());
        let legacy_root = temp.path().join("legacy-root");
        let card = test_card(Some(legacy_root.to_string_lossy().to_string()));

        let resolved = resolve_agent_worktree(Some(meta.clone()), &card).unwrap();
        assert_eq!(resolved.root, meta.root);
        assert_eq!(resolved.branch.as_deref(), Some("thread-branch"));
        assert_eq!(resolved.name.as_deref(), Some("thread-id"));

        let legacy = resolve_agent_worktree(None, &card).unwrap();
        assert_eq!(legacy.root, legacy_root);
        assert_eq!(legacy.branch.as_deref(), Some("legacy-branch"));
        assert_eq!(legacy.name.as_deref(), Some("legacy-id"));
    }

    #[tokio::test]
    async fn task_spawn_agent_finish_missing_worktree_returns_error() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing-worktree");
        let result = auto_commit_worktree_with_message(
            test_gcx().await,
            &missing,
            "T-1",
            "Card T-1",
            Some("test commit".to_string()),
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    #[tokio::test]
    async fn task_spawn_agent_finish_non_git_worktree_returns_error() {
        let temp = tempfile::tempdir().unwrap();
        let non_git = temp.path().join("non-git");
        std::fs::create_dir_all(&non_git).unwrap();
        let result = auto_commit_worktree_with_message(
            test_gcx().await,
            &non_git,
            "T-1",
            "Card T-1",
            Some("test commit".to_string()),
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not a git worktree/repo"));
    }

    #[tokio::test]
    async fn task_spawn_agent_finish_clean_worktree_returns_no_commit() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);

        let commit = auto_commit_worktree_with_message(
            test_gcx().await,
            &repo,
            "T-1",
            "Card T-1",
            Some("test commit".to_string()),
        )
        .await
        .unwrap();

        assert!(commit.is_none());
        assert!(run_git(&repo, &["status", "--porcelain"]).trim().is_empty());
    }

    #[tokio::test]
    async fn task_spawn_agent_finish_auto_commits_from_worktree_root() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let worktree = temp.path().join("agent-worktree");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        run_git(
            &source,
            &[
                "worktree",
                "add",
                "-b",
                "refact/task/task-1/card/T-1/agent",
                worktree.to_str().unwrap(),
            ],
        );
        std::fs::write(worktree.join("file.txt"), "changed in worktree\n").unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;

        let commit = auto_commit_worktree_with_message(
            gcx,
            &worktree,
            "T-1",
            "Card T-1",
            Some("test commit".to_string()),
        )
        .await
        .unwrap();

        assert!(commit.is_some());
        assert!(run_git(&worktree, &["status", "--porcelain"])
            .trim()
            .is_empty());
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "hello\n"
        );
        assert_eq!(
            std::fs::read_to_string(worktree.join("file.txt")).unwrap(),
            "changed in worktree\n"
        );
    }
}
