use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::types::{ChatEvent, ChatSession, SessionState, TrajectoryCommitIntent};
use crate::chat::worktree_lifecycle::{
    check_ordinary_worktree_ancestry, is_ordinary_worktree_lineage, check_unchanged_attachment,
    transition_lock,
};
use crate::tools::tools_description::{
    json_schema_from_params, Tool, ToolDesc, ToolSource, ToolSourceType,
};
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::{MergeWorktreeRequest, MergeWorktreeResponse, WorktreeMergeStrategy};

fn strategy_from_str(strategy: &str) -> Result<WorktreeMergeStrategy, String> {
    match strategy {
        "merge" => Ok(WorktreeMergeStrategy::Merge),
        "squash" => Ok(WorktreeMergeStrategy::Squash),
        other => Err(format!(
            "Invalid strategy '{}', must be 'merge' or 'squash'",
            other
        )),
    }
}

fn strategy_arg(args: &HashMap<String, Value>) -> Result<WorktreeMergeStrategy, String> {
    match args.get("strategy") {
        None => Ok(WorktreeMergeStrategy::Squash),
        Some(Value::String(strategy)) => strategy_from_str(strategy),
        Some(_) => Err("'strategy' must be a string".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fixture() -> (
        tempfile::TempDir,
        Arc<AMutex<AtCommandsContext>>,
        Arc<AMutex<ChatSession>>,
        WorktreeService,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir(&source).unwrap();
        let source = dunce::canonicalize(&source).unwrap();
        let git = |args: &[&str]| crate::worktrees::git::run_git(&source, args).unwrap();
        git(&["init"]);
        git(&["config", "core.autocrlf", "false"]);
        git(&["checkout", "-b", "main"]);
        git(&["config", "user.email", "test@example.invalid"]);
        git(&["config", "user.name", "Test"]);
        std::fs::write(source.join("tracked"), "initial\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "initial"]);
        let cache = temp.path().join("cache");
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            cache.clone(),
            temp.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let service = WorktreeService::new(cache, source.clone()).unwrap();
        let worktree = service
            .create_chat_worktree("merge-root")
            .await
            .unwrap()
            .worktree
            .meta;
        let mut session = ChatSession::new("merge-root".into());
        session.work_project_root = Some(source);
        session.thread.worktree = Some(worktree.clone());
        session.set_runtime_state(SessionState::ExecutingTools, None);
        let abort = session.abort_flag.clone();
        let session = Arc::new(AMutex::new(session));
        app.chat
            .sessions
            .write()
            .await
            .insert("merge-root".into(), session.clone());
        let context = AtCommandsContext::new_with_abort(
            app,
            4096,
            10,
            false,
            vec![],
            "merge-root".into(),
            None,
            String::new(),
            None,
            Some(worktree),
            Some(abort),
        )
        .await;
        (temp, Arc::new(AMutex::new(context)), session, service)
    }

    async fn execute(
        context: &Arc<AMutex<AtCommandsContext>>,
        name: &str,
        args: Value,
    ) -> Result<refact_runtime_api::ToolExecutionResult, String> {
        let registry = context.lock().await.app.tool_registry.clone();
        let catalog = registry
            .acquire_tool_catalog("agent", Some("test/model"), None)
            .await;
        let pool = registry
            .acquire_turn_tool_pool("agent", Some("test/model"), None, &catalog)
            .await;
        let pool = pool.expect("shipping pooled execution must be available");
        registry
            .execute_tool_with_catalog_and_pool(
                context,
                &catalog,
                Some(&pool),
                "agent",
                Some("test/model"),
                name,
                name,
                args.as_object().unwrap().clone(),
            )
            .await?
            .ok_or_else(|| format!("{name} not registered"))
    }

    fn merge_args() -> Value {
        serde_json::json!({"commit_message":"land isolated changes", "acknowledge_dirty_baseline":true})
    }

    #[tokio::test]
    async fn self_merge_handoff_allows_bookkeeping_and_persists_project_scope() {
        let (_temp, context, session, service) = fixture().await;
        let worktree = session.lock().await.thread.worktree.clone().unwrap();
        std::fs::write(worktree.root.join("landed"), "isolated dirty change\n").unwrap();
        std::fs::write(
            service.source_workspace_root().join("source-wip"),
            "preserve me\n",
        )
        .unwrap();
        execute(
            &context,
            "set_goal",
            serde_json::json!({"content":"Finish fixture bookkeeping"}),
        )
        .await
        .unwrap();
        assert_eq!(
            session.lock().await.runtime.goal_status,
            Some(crate::chat::types::GoalStatus::Active)
        );
        let app = context.lock().await.app.clone();
        let mut other = ChatSession::new("merge-child".into());
        other.thread.worktree = Some(worktree.clone());
        other.thread.parent_id = Some("merge-root".into());
        let other = Arc::new(AMutex::new(other));
        app.chat
            .sessions
            .write()
            .await
            .insert("merge-child".into(), other.clone());
        service
            .add_reference(
                &worktree.id,
                crate::worktrees::types::WorktreeReference {
                    kind: "chat".into(),
                    chat_id: Some("merge-child".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let mut events = session.lock().await.event_tx.subscribe();
        let merge_result = execute(&context, "worktree_merge", merge_args())
            .await
            .unwrap();
        {
            let mut session = session.lock().await;
            assert!(!session
                .messages
                .iter()
                .any(|message| message.tool_call_id == "worktree_disabled"));
            for message in merge_result.messages {
                session.add_message(message);
            }
            session.drain_post_tool_side_effects();
            let result_index = session
                .messages
                .iter()
                .position(|message| {
                    message.role == "tool" && message.tool_call_id == "worktree_merge"
                })
                .unwrap();
            let disabled_index = session
                .messages
                .iter()
                .position(|message| message.tool_call_id == "worktree_disabled")
                .unwrap();
            assert!(result_index < disabled_index);
        }
        assert!(
            !worktree.root.exists(),
            "successful self merge must actually remove checkout"
        );
        assert_eq!(
            std::fs::read_to_string(service.source_workspace_root().join("landed")).unwrap(),
            "isolated dirty change\n"
        );
        assert_eq!(
            std::fs::read_to_string(service.source_workspace_root().join("source-wip")).unwrap(),
            "preserve me\n"
        );
        execute(
            &context,
            "pause",
            serde_json::json!({"target":{"kind":"goal"},"reason":"fixture finished"}),
        )
        .await
        .expect("same-batch goal bookkeeping must run after self merge");
        assert_eq!(
            session.lock().await.runtime.goal_status,
            Some(crate::chat::types::GoalStatus::Paused)
        );
        assert_eq!(other.lock().await.thread.worktree.as_ref(), Some(&worktree));
        let other_worktree = other.lock().await.thread.worktree.clone();
        let other_context = Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app.clone(),
                4096,
                10,
                false,
                vec![],
                "merge-child".into(),
                Some("merge-root".into()),
                String::new(),
                None,
                other_worktree,
            )
            .await,
        ));
        let other_error = execute(&other_context, "get_goal", serde_json::json!({}))
            .await
            .err()
            .expect("other attached chat must remain fail closed");
        assert!(
            other_error.contains("Do not continue editing"),
            "{other_error}"
        );
        assert!(context.lock().await.execution_scope.is_none());
        let app = context.lock().await.app.clone();
        let (abort, attachment) = {
            let session = session.lock().await;
            assert!(session.thread.worktree.is_none());
            assert!(session
                .messages
                .iter()
                .any(|message| message.tool_call_id == "worktree_disabled"));
            (session.abort_flag.clone(), session.thread.worktree.clone())
        };
        let fresh = Arc::new(AMutex::new(
            AtCommandsContext::new_with_abort(
                app,
                4096,
                10,
                false,
                vec![],
                "merge-root".into(),
                None,
                String::new(),
                None,
                attachment,
                Some(abort),
            )
            .await,
        ));
        execute(&fresh, "get_goal", serde_json::json!({}))
            .await
            .expect("future turn must not inherit deleted scope");
        let saved: Value = serde_json::from_str(
            &std::fs::read_to_string(
                service
                    .source_workspace_root()
                    .join(".refact/trajectories/merge-root/merge-root.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(saved.get("worktree").is_none_or(Value::is_null));
        let mut detached = false;
        while let Ok(event) = events.try_recv() {
            let event: Value = serde_json::from_str(&event).unwrap();
            detached |=
                event["type"] == "thread_updated" && event.get("worktree") == Some(&Value::Null);
        }
        assert!(detached, "worktree:null must reach SSE consumers");
    }

    #[tokio::test]
    async fn self_merge_continuation_without_owner_reference_returns_to_project() {
        let (_temp, context, session, service) = fixture().await;
        context.lock().await.root_chat_id = "original-root".into();
        {
            let mut session = session.lock().await;
            session.thread.root_chat_id = Some("original-root".into());
            session.thread.parent_id = Some("previous-chat".into());
            session.thread.link_type = Some("mode_transition".into());
        }
        let app = context.lock().await.app.clone();
        app.chat.sessions.write().await.insert(
            "original-root".into(),
            Arc::new(AMutex::new(ChatSession::new("original-root".into()))),
        );
        let mut previous = ChatSession::new("previous-chat".into());
        previous.thread.root_chat_id = Some("original-root".into());
        previous.thread.parent_id = Some("original-root".into());
        previous.thread.link_type = Some("handoff".into());
        app.chat
            .sessions
            .write()
            .await
            .insert("previous-chat".into(), Arc::new(AMutex::new(previous)));
        let before = session.lock().await.thread.clone();
        let worktree = before.worktree.as_ref().unwrap();
        service
            .add_reference(
                &worktree.id,
                crate::worktrees::types::WorktreeReference {
                    kind: "chat".into(),
                    chat_id: Some("original-root".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        service
            .remove_reference(
                &worktree.id,
                &crate::worktrees::types::WorktreeReference {
                    kind: "chat".into(),
                    chat_id: Some("merge-root".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let registered = service.get_worktree(&worktree.id).await.unwrap();
        assert_eq!(registered.references.len(), 1);
        assert_eq!(
            registered.references[0].chat_id.as_deref(),
            Some("original-root")
        );
        std::fs::write(worktree.root.join("landed"), "continuation change\n").unwrap();
        execute(&context, "worktree_merge", merge_args())
            .await
            .expect("ordinary continuation must not need the original owner's registry reference");
        assert!(!worktree.root.exists());
        assert_eq!(
            std::fs::read_to_string(service.source_workspace_root().join("landed")).unwrap(),
            "continuation change\n"
        );
        assert!(context.lock().await.execution_scope.is_none());
        let thread = session.lock().await.thread.clone();
        assert!(thread.worktree.is_none());
        assert_eq!(thread.root_chat_id, before.root_chat_id);
        assert_eq!(thread.parent_id, before.parent_id);
        assert_eq!(thread.link_type, before.link_type);
    }

    #[tokio::test]
    async fn self_merge_retained_checkout_keeps_attachment() {
        let (_temp, context, session, _service) = fixture().await;
        let before = session.lock().await.thread.worktree.clone().unwrap();
        let mut args = merge_args();
        args["delete_after_merge"] = Value::Bool(false);
        execute(&context, "worktree_merge", args).await.unwrap();
        assert!(before.root.exists());
        assert_eq!(session.lock().await.thread.worktree.as_ref(), Some(&before));
        assert_eq!(
            context.lock().await.execution_scope_worktree().as_ref(),
            Some(&before)
        );
    }

    #[tokio::test]
    async fn self_merge_dirty_target_refusal_keeps_attachment() {
        let (_temp, context, session, service) = fixture().await;
        let before = session.lock().await.thread.worktree.clone().unwrap();
        std::fs::write(before.root.join("landed"), "isolated\n").unwrap();
        std::fs::write(
            service.source_workspace_root().join("source-wip"),
            "dirty\n",
        )
        .unwrap();
        let result = execute(
            &context,
            "worktree_merge",
            serde_json::json!({"commit_message":"refused"}),
        )
        .await
        .unwrap();
        assert!(result.messages[0]
            .content
            .content_text_only()
            .contains("Merge Refused: Dirty Target"));
        assert!(before.root.exists());
        assert_eq!(session.lock().await.thread.worktree.as_ref(), Some(&before));
        assert_eq!(
            context.lock().await.execution_scope_worktree().as_ref(),
            Some(&before)
        );
    }

    #[tokio::test]
    async fn foreign_merge_does_not_detach_active_checkout() {
        let (_temp, context, session, service) = fixture().await;
        let before = session.lock().await.thread.worktree.clone().unwrap();
        let foreign = service
            .create_chat_worktree("foreign-root")
            .await
            .unwrap()
            .worktree
            .meta;
        let mut args = merge_args();
        args["worktree_id"] = Value::String(foreign.id);
        execute(&context, "worktree_merge", args).await.unwrap();
        assert!(!foreign.root.exists());
        assert!(before.root.exists());
        assert_eq!(session.lock().await.thread.worktree.as_ref(), Some(&before));
        assert_eq!(
            context.lock().await.execution_scope_worktree().as_ref(),
            Some(&before)
        );
    }

    #[tokio::test]
    async fn child_self_merge_refuses_before_checkout_deletion() {
        let (_temp, context, session, _service) = fixture().await;
        let before = session.lock().await.thread.worktree.clone().unwrap();
        context.lock().await.root_chat_id = "parent".into();
        let error = execute(&context, "worktree_merge", merge_args())
            .await
            .err()
            .expect("child must be refused");
        assert_eq!(
            error,
            "Destructive self merge requires an ordinary root chat's own checkout"
        );
        assert!(before.root.exists());
        assert_eq!(session.lock().await.thread.worktree.as_ref(), Some(&before));
        assert_eq!(
            context.lock().await.execution_scope_worktree().as_ref(),
            Some(&before)
        );
    }

    #[tokio::test]
    async fn self_merge_noop_cleanup_returns_to_project() {
        let (_temp, context, session, _service) = fixture().await;
        let before = session.lock().await.thread.worktree.clone().unwrap();
        let result = execute(&context, "worktree_merge", merge_args())
            .await
            .unwrap();
        assert!(result.messages[0]
            .content
            .content_text_only()
            .starts_with("# Nothing to Merge"));
        assert!(!before.root.exists());
        assert!(session.lock().await.thread.worktree.is_none());
        assert!(context.lock().await.execution_scope.is_none());
    }

    #[tokio::test]
    async fn task_self_merge_refuses_before_checkout_deletion() {
        let (_temp, context, session, _service) = fixture().await;
        let before = session.lock().await.thread.worktree.clone().unwrap();
        session.lock().await.thread.task_meta = Some(crate::chat::types::TaskMeta {
            task_id: "fixture-task".into(),
            role: "planner".into(),
            agent_id: None,
            card_id: None,
            planner_chat_id: None,
        });
        let error = execute(&context, "worktree_merge", merge_args())
            .await
            .err()
            .expect("task must be refused");
        assert_eq!(
            error,
            "Destructive self merge requires an ordinary root chat's own checkout"
        );
        assert!(before.root.exists());
        assert_eq!(session.lock().await.thread.worktree.as_ref(), Some(&before));
        assert_eq!(
            context.lock().await.execution_scope_worktree().as_ref(),
            Some(&before)
        );
    }

    #[tokio::test]
    async fn self_merge_finalizes_after_same_execution_cancellation() {
        let (_temp, context, session, service) = fixture().await;
        let transition = transition_lock(&session).await;
        let before = session.lock().await.thread.clone();
        let worktree = before.worktree.as_ref().unwrap().clone();
        let app = context.lock().await.app.clone();
        let mut barrier = session.lock().await;
        let receiver = start_self_merge(
            context.clone(),
            session.clone(),
            before,
            service.clone(),
            worktree.id.clone(),
            MergeWorktreeRequest {
                commit_message: Some("fixture cleanup".into()),
                ..Default::default()
            },
            transition,
            app.clone(),
        )
        .unwrap();
        drop(receiver);
        barrier
            .abort_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);
        barrier.set_runtime_state(SessionState::Idle, None);
        drop(barrier);
        app.agent_executions.shutdown().await;
        assert!(!worktree.root.exists());
        assert!(session.lock().await.thread.worktree.is_none());
        assert!(context.lock().await.execution_scope.is_none());
        let saved: Value = serde_json::from_str(
            &std::fs::read_to_string(
                service
                    .source_workspace_root()
                    .join(".refact/trajectories/merge-root/merge-root.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(saved.get("worktree").is_none_or(Value::is_null));
    }

    #[test]
    fn bool_arg_accepts_model_spellings_and_rejects_garbage() {
        let args = HashMap::from([(
            "delete_after_merge".to_string(),
            Value::String("false".to_string()),
        )]);

        assert_eq!(bool_arg(&args, "delete_after_merge", true), Ok(false));

        let args = HashMap::from([(
            "include_uncommitted".to_string(),
            Value::String("True".to_string()),
        )]);

        assert_eq!(bool_arg(&args, "include_uncommitted", false), Ok(true));

        let args = HashMap::from([(
            "include_uncommitted".to_string(),
            Value::String("maybe".to_string()),
        )]);

        assert!(bool_arg(&args, "include_uncommitted", true)
            .unwrap_err()
            .contains("must be a boolean"));
    }
}

fn bool_arg(args: &HashMap<String, Value>, name: &str, default: bool) -> Result<bool, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(default),
        Some(value) => refact_tool_api::coerce_bool(value)
            .ok_or_else(|| format!("'{}' must be a boolean", name)),
    }
}

fn string_arg(args: &HashMap<String, Value>, name: &str) -> Result<Option<String>, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed.to_string()))
            }
        }
        Some(_) => Err(format!("'{}' must be a string", name)),
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

fn append_warnings(message: &mut String, warnings: &[String]) {
    if warnings.is_empty() {
        return;
    }
    message.push_str("\n\n## Warnings");
    for warning in warnings {
        message.push_str(&format!("\n- {}", warning));
    }
}

fn response_warnings(response: &MergeWorktreeResponse) -> Vec<String> {
    let mut warnings = response.warnings.clone();
    if let Some(cleanup) = response.cleanup.as_ref() {
        warnings.extend(cleanup.warnings.clone());
    }
    warnings
}

fn merge_response_message(response: &MergeWorktreeResponse) -> String {
    if let Some(conflict) = response.conflict.as_ref() {
        let files = if conflict.files.is_empty() {
            "None detected".to_string()
        } else {
            conflict
                .files
                .iter()
                .map(|file| format!("- {}", file))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let mut message = format!(
            "# Worktree Merge Conflicts\n\n**Worktree:** {}\n**Branch:** {} → {}\n**Strategy:** {}\n**Aborted:** {}\n\n## Conflicting Files\n{}\n\n{}",
            response.id,
            response.source_branch,
            response.target_branch,
            response.strategy,
            conflict.aborted,
            files,
            conflict.instructions
        );
        append_warnings(&mut message, &response_warnings(response));
        message.push_str("\n\nUse shell Git in the source worktree to merge the target branch, resolve and stage conflicts, and commit the result. Reverify before calling worktree_merge again.");
        return message;
    }

    if let Some(baseline) = response.dirty_baseline.as_ref() {
        let files = baseline
            .files
            .iter()
            .map(|file| format!("- {}", file))
            .collect::<Vec<_>>()
            .join("\n");
        let mut message = format!(
            "# Merge Refused: Dirty Target Checkout\n\n**Worktree:** {}\n**Branch:** {} → {}\n**Target branch checked out:** {}\n**Staged/unstaged/untracked:** {}/{}/{}\n\n## Uncommitted Files{}\n{}\n\nNothing was merged and nothing was discarded. Commit or stash these changes, or retry with acknowledge_dirty_baseline=true to stash and restore them around the merge.",
            response.id,
            response.source_branch,
            response.target_branch,
            baseline.branch.as_deref().unwrap_or("detached HEAD"),
            baseline.staged_count,
            baseline.unstaged_count,
            baseline.untracked_count,
            if baseline.files_truncated { " (truncated)" } else { "" },
            files
        );
        append_warnings(&mut message, &response_warnings(response));
        return message;
    }

    if response.status == "nothing_to_merge" {
        let mut message = format!(
            "# Nothing to Merge\n\n**Worktree:** {}\n**Branch:** {} → {}\n\nCleanup: {}.",
            response.id,
            response.source_branch,
            response.target_branch,
            cleanup_summary(response)
        );
        append_warnings(&mut message, &response_warnings(response));
        return message;
    }

    let mut message = format!(
        "# Worktree Merged\n\n**Worktree:** {}\n**Strategy:** {}\n**Branch:** {} → {}\n**Merge commit:** {}\n**Cleanup:** {}\n\nThe worktree changes have been merged into the target branch.",
        response.id,
        response.strategy,
        response.source_branch,
        response.target_branch,
        response.merge_commit.as_deref().unwrap_or("unknown"),
        cleanup_summary(response)
    );
    append_warnings(&mut message, &response_warnings(response));
    message
}

async fn service_from_gcx(
    app: &crate::app_state::AppState,
    requested_source_root: Option<String>,
) -> Result<WorktreeService, String> {
    let cache_dir = app.paths.cache_dir.clone();
    let project_dirs = crate::files_correction::get_project_dirs(app.gcx.clone()).await;
    if project_dirs.is_empty() {
        return Err("No project root available".to_string());
    }
    let source_root = if let Some(requested) = requested_source_root {
        let requested = std::path::PathBuf::from(requested);
        let requested = tokio::fs::canonicalize(&requested).await.map_err(|e| {
            format!(
                "Failed to resolve source workspace root '{}': {}",
                requested.display(),
                e
            )
        })?;
        let requested = dunce::simplified(&requested).to_path_buf();
        let mut matches = false;
        for dir in &project_dirs {
            if tokio::fs::canonicalize(dir)
                .await
                .map(|canonical| dunce::simplified(&canonical).to_path_buf() == requested)
                .unwrap_or(false)
            {
                matches = true;
                break;
            }
        }
        if !matches {
            return Err("Worktree source root is not a current workspace directory".to_string());
        }
        requested
    } else {
        project_dirs[0].clone()
    };
    Ok(crate::worktrees::service::fenced_service(
        WorktreeService::new_async(cache_dir, source_root).await?,
        app.runtime.tool_jobs.scopes.clone(),
    ))
}

pub struct ToolWorktreeMerge;

fn check_self_merge_caller(
    context: &AtCommandsContext,
    session: &ChatSession,
    require_live: bool,
) -> Result<(), String> {
    let worktree = context
        .execution_scope_worktree()
        .ok_or("Self merge scope is unavailable")?;
    if !is_ordinary_worktree_lineage(&context.chat_id, &context.root_chat_id, &session.thread)
        || context.subchat_depth != 0
        || context.background_agent_id.is_some()
        || context.task_meta.is_some()
        || crate::buddy::supervision_authority::native_edit_scope_for_chat(&context.chat_id)
            .is_some()
        || worktree.kind != "chat"
        || worktree.task_id.is_some()
        || worktree.card_id.is_some()
        || worktree.agent_id.is_some()
    {
        return Err("Destructive self merge requires an ordinary root chat's own checkout".into());
    }
    if session.closed
        || session.chat_id != context.chat_id
        || (require_live && session.runtime.state != SessionState::ExecutingTools)
        || !Arc::ptr_eq(&session.abort_flag, &context.abort_flag)
        || (require_live
            && context
                .abort_flag
                .load(std::sync::atomic::Ordering::Relaxed))
        || session.thread.worktree.as_ref() != Some(&worktree)
        || session.work_project_root.as_ref() != Some(&worktree.source_workspace_root)
    {
        return Err(
            "Self merge requires the current live tool execution and unchanged chat scope".into(),
        );
    }
    Ok(())
}

async fn check_self_merge_authority(
    app: &crate::app_state::AppState,
    chat_id: &str,
) -> Result<(), String> {
    if let Some(reason) = crate::agents::seal::seal_reason(app, chat_id).await {
        return Err(reason.message());
    }
    if crate::tasks::admission::is_tracked(chat_id).await
        || app
            .agents
            .find_agent_id_by_child_chat_id(chat_id)
            .await
            .is_some()
    {
        return Err("Task and background executions cannot hand off self merge scope".into());
    }
    Ok(())
}

async fn finish_self_merge(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    session_arc: &Arc<AMutex<ChatSession>>,
    before: &crate::chat::types::ThreadParams,
    response: &MergeWorktreeResponse,
) -> Result<(), String> {
    let (app, chat_id) = {
        let context = ccx.lock().await;
        (context.app.clone(), context.chat_id.clone())
    };
    let worktree = before.worktree.as_ref().ok_or("Self merge scope changed")?;
    let removed = response.id == worktree.id
        && (response.merged || response.status == "nothing_to_merge")
        && response.conflict.is_none()
        && response.dirty_baseline.is_none()
        && response
            .cleanup
            .as_ref()
            .is_some_and(|cleanup| cleanup.worktree_deleted && cleanup.registry_deleted)
        && matches!(tokio::fs::symlink_metadata(&worktree.root).await,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound);
    if removed {
        check_self_merge_authority(&app, &chat_id).await?;
        {
            let mut context = ccx.lock().await;
            let mut session = session_arc.lock().await;
            check_self_merge_caller(&context, &session, false)?;
            check_unchanged_attachment(&session, before)?;
            session.thread.worktree = None;
            context.execution_scope = None;
            session.release_turn_only_state();
            session.emit(ChatEvent::ThreadUpdated {
                params: serde_json::json!({"worktree": null}),
            });
            session.queue_post_tool_side_effect(crate::chat::queue::worktree_disabled_message(
                Some(worktree),
            ));
            session.increment_version();
            session.touch();
        }
        crate::chat::trajectories::try_save_trajectory_with_intent(
                    app.clone(), session_arc.clone(), TrajectoryCommitIntent::Required,
                ).await.map_err(|error| format!("Worktree removed and scope returned to project in memory, but persistence failed: {error}"))?;
    }
    Ok(())
}

fn start_self_merge(
    ccx: Arc<AMutex<AtCommandsContext>>,
    session: Arc<AMutex<ChatSession>>,
    before: crate::chat::types::ThreadParams,
    service: WorktreeService,
    worktree_id: String,
    request: MergeWorktreeRequest,
    transition: crate::chat::worktree_lifecycle::AttachmentTransitionGuard,
    app: crate::app_state::AppState,
) -> Result<tokio::sync::oneshot::Receiver<Result<MergeWorktreeResponse, String>>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.agent_executions.clone().spawn_owned(
        format!("self-merge:{}", uuid::Uuid::new_v4()),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
        Arc::new(tokio::sync::Notify::new()),
        async move {
            let _transition = transition;
            let result = async {
                let mut request = request;
                if request.target_branch.is_none() {
                    request.target_branch = Some(
                        service
                            .get_worktree(&worktree_id)
                            .await?
                            .meta
                            .base_branch
                            .ok_or("Merge target branch is required")?,
                    );
                }
                let (response, _, _) =
                    crate::tools::tool_task_merge_agent::merge_under_available_lease(
                        &app,
                        &service,
                        &worktree_id,
                        request,
                        None,
                        |_, _| async { Err("No post-merge check requested".into()) },
                    )
                    .await?;
                finish_self_merge(&ccx, &session, &before, &response).await?;
                Ok(response)
            }
            .await;
            if let Err(error) = &result {
                tracing::error!(%error, "Owned self merge failed");
            }
            let _ = sender.send(result);
        },
    )?;
    Ok(receiver)
}

impl ToolWorktreeMerge {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolWorktreeMerge {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "worktree_merge".to_string(),
            display_name: "Merge Worktree".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Merge any registered worktree in the current project back into its target branch. Use shell Git in the source worktree to merge target changes and resolve conflicts when needed, then reverify before retrying. Generates a commit message unless supplied. By default uses squash and deletes the worktree/branch after successful merge or no-op cleanup.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("worktree_id", "string", "Registered worktree id to merge. Defaults to the active chat worktree when omitted."),
                    ("source_workspace_root", "string", "Source workspace root for repositories with multiple workspace folders."),
                    ("target_branch", "string", "Target branch to merge into. Defaults to the worktree base branch."),
                    ("strategy", "string", "Merge strategy: 'squash' (default) or 'merge'."),
                    ("commit_message", "string", "Optional commit message override. If omitted, Refact generates one from the diff."),
                    ("delete_after_merge", "boolean", "Delete the worktree and source branch after a successful merge or nothing-to-merge cleanup. Defaults to true."),
                    ("include_uncommitted", "boolean", "Auto-commit dirty worktree changes before merging. Defaults to true."),
                    ("acknowledge_dirty_baseline", "boolean", "Merge even though the target checkout has uncommitted changes; they are stashed and restored around the merge. Defaults to false, which refuses the merge and reports the dirty files."),
                ],
                &[],
            ),
            output_schema: None,
            annotations: Some(serde_json::json!({
                "destructiveHint": true,
                "idempotentHint": false,
                "openWorldHint": false,
            })),
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, active_worktree) = {
            let ccx = ccx.lock().await;
            (ccx.app.gcx.clone(), ccx.execution_scope_worktree())
        };

        let worktree_id = string_arg(args, "worktree_id")?
            .or_else(|| active_worktree.as_ref().map(|worktree| worktree.id.clone()))
            .ok_or_else(|| {
                "Missing 'worktree_id' and this chat has no active worktree".to_string()
            })?;
        let source_workspace_root = string_arg(args, "source_workspace_root")?.or_else(|| {
            active_worktree
                .as_ref()
                .map(|worktree| worktree.source_workspace_root.to_string_lossy().to_string())
        });
        let strategy = strategy_arg(args)?;
        let delete_after_merge = bool_arg(args, "delete_after_merge", true)?;
        let include_uncommitted = bool_arg(args, "include_uncommitted", true)?;
        let acknowledge_dirty_baseline = bool_arg(args, "acknowledge_dirty_baseline", false)?;
        let target_branch = string_arg(args, "target_branch")?;

        let parent_chat_id = ccx.lock().await.chat_id.clone();
        let app = ccx.lock().await.app.clone();
        let service = service_from_gcx(&app, source_workspace_root).await?;
        let self_session = if delete_after_merge
            && active_worktree
                .as_ref()
                .is_some_and(|worktree| worktree.id == worktree_id)
        {
            Some(
                app.chat
                    .sessions
                    .read()
                    .await
                    .get(&parent_chat_id)
                    .cloned()
                    .ok_or("Current chat session is unavailable for self merge")?,
            )
        } else {
            None
        };
        let _transition = match &self_session {
            Some(session) => Some(transition_lock(session).await),
            None => None,
        };
        let before = if let Some(session) = &self_session {
            check_self_merge_authority(&app, &parent_chat_id).await?;
            let context = ccx.lock().await;
            let session = session.lock().await;
            check_self_merge_caller(&context, &session, true)?;
            let worktree = active_worktree.as_ref().ok_or("Self merge scope changed")?;
            if context.execution_scope_worktree().as_ref() != Some(worktree)
                || service.source_workspace_root() != worktree.source_workspace_root
            {
                return Err("Self merge source or checkout changed".into());
            }
            Some(session.thread.clone())
        } else {
            None
        };
        if let Some(before) = &before {
            let root = ccx.lock().await.root_chat_id.clone();
            check_ordinary_worktree_ancestry(&app, &parent_chat_id, &root, before).await?;
            let worktree = before.worktree.as_ref().ok_or("Self merge scope changed")?;
            let view = service.get_worktree(&worktree_id).await?;
            if view.meta != *worktree {
                return Err("Self merge registered checkout identity does not match".into());
            }
        }
        let commit_message = match string_arg(args, "commit_message")? {
            Some(message) => Some(message),
            None => {
                service
                    .validate_registered_worktree_checkout(&worktree_id)
                    .await?;
                match service.diff_worktree(&worktree_id).await {
                    Ok(diff) if !diff.files.is_empty() => {
                        match crate::agentic::generate_commit_message::generate_commit_message_by_diff(
                            gcx.clone(),
                            &diff.patch,
                            &target_branch,
                            Some(&parent_chat_id),
                        )
                        .await
                        {
                            Ok(message) if !message.trim().is_empty() => Some(message),
                            _ => None,
                        }
                    }
                    _ => None,
                }
            }
        };

        let target_branch = Some(match target_branch {
            Some(target) => target,
            None => service
                .get_worktree(&worktree_id)
                .await?
                .meta
                .base_branch
                .ok_or("Merge target branch is required")?,
        });

        if let (Some(session), Some(before)) = (&self_session, &before) {
            check_self_merge_authority(&app, &parent_chat_id).await?;
            let context = ccx.lock().await;
            let session = session.lock().await;
            check_self_merge_caller(&context, &session, true)?;
            check_unchanged_attachment(&session, before)?;
        }
        if let (Some(session), Some(before), Some(transition)) = (self_session, before, _transition)
        {
            let request = MergeWorktreeRequest {
                strategy,
                delete_after_merge,
                include_uncommitted,
                target_branch,
                commit_message,
                generate_commit_message: false,
                acknowledge_dirty_baseline,
            };
            let response = start_self_merge(
                ccx.clone(),
                session,
                before,
                service,
                worktree_id,
                request,
                transition,
                app.clone(),
            )?
            .await
            .map_err(|error| format!("Self merge completion task failed: {error}"))??;
            return Ok((
                false,
                vec![ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(merge_response_message(&response)),
                    tool_calls: None,
                    tool_call_id: tool_call_id.clone(),
                    ..Default::default()
                })],
            ));
        }

        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let (response, _, _) = crate::tools::tool_task_merge_agent::merge_under_available_lease(
            &app,
            &service,
            &worktree_id,
            MergeWorktreeRequest {
                strategy,
                delete_after_merge,
                include_uncommitted,
                target_branch,
                commit_message,
                generate_commit_message: false,
                acknowledge_dirty_baseline,
            },
            None,
            |_, _| async { Err("No post-merge check requested".into()) },
        )
        .await?;

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(merge_response_message(&response)),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }
}
