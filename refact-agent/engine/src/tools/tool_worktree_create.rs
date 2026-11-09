use std::collections::HashMap;
use std::sync::{Arc, atomic::Ordering};
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::types::{ChatEvent, ChatSession, SessionState, TrajectoryCommitIntent};
use crate::chat::worktree_lifecycle::{
    check_ordinary_worktree_ancestry, is_ordinary_worktree_lineage, check_unchanged_attachment,
    transition_lock,
};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::worktrees::{scope::ExecutionScope, service::WorktreeService};

pub struct ToolWorktreeCreate;

fn check_caller(context: &AtCommandsContext, session: &ChatSession) -> Result<(), String> {
    if !is_ordinary_worktree_lineage(&context.chat_id, &context.root_chat_id, &session.thread)
        || context.subchat_depth != 0
        || context.background_agent_id.is_some()
        || context.task_meta.is_some()
    {
        return Err("worktree_create is available only to an ordinary root chat, not task or child executions".into());
    }
    if session.closed
        || session.chat_id != context.chat_id
        || session.runtime.state != SessionState::ExecutingTools
        || !Arc::ptr_eq(&session.abort_flag, &context.abort_flag)
        || context.abort_flag.load(Ordering::Relaxed)
        || context.execution_scope_worktree() != session.thread.worktree
    {
        return Err(
            "worktree_create requires the current live tool execution and unchanged chat scope"
                .into(),
        );
    }
    Ok(())
}

#[async_trait]
impl Tool for ToolWorktreeCreate {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "worktree_create".into(), display_name: "Create and Attach Worktree".into(),
            source: ToolSource { source_type: ToolSourceType::Builtin, config_path: String::new() },
            experimental: false, allow_parallel: false,
            description: "Create a registered isolated worktree and immediately attach it to this ordinary root chat. Later tools in this batch and future turns use it. Reuses this chat's existing active worktree; never nests worktrees. Uses the trusted current project and default refact/chat branch convention. No confirmation is needed by default. Not available to task, delegated, or background child executions. No shell commands are needed.".into(),
            input_schema: json!({"type":"object","properties":{},"required":[],"additionalProperties":false}),
            output_schema: None,
            annotations: Some(json!({"readOnlyHint":false,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false})),
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<Mutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        if !args.is_empty() {
            return Err("worktree_create takes no arguments; project, path and chat identity are runtime-owned".into());
        }
        let (app, chat_id) = {
            let context = ccx.lock().await;
            (context.app.clone(), context.chat_id.clone())
        };
        let session_arc = app
            .chat
            .sessions
            .read()
            .await
            .get(&chat_id)
            .cloned()
            .ok_or("Current chat session is unavailable")?;
        let _transition = transition_lock(&session_arc).await;
        if let Some(reason) = crate::agents::seal::seal_reason(&app, &chat_id).await {
            return Err(reason.message());
        }
        if app
            .agents
            .find_agent_id_by_child_chat_id(&chat_id)
            .await
            .is_some()
        {
            return Err("Background child chats cannot change worktree scope".into());
        }
        let (before, source) = {
            let context = ccx.lock().await;
            let session = session_arc.lock().await;
            check_caller(&context, &session)?;
            (
                session.thread.clone(),
                session
                    .work_project_root
                    .clone()
                    .ok_or("Trusted current project is unavailable")?,
            )
        };
        let root = ccx.lock().await.root_chat_id.clone();
        check_ordinary_worktree_ancestry(&app, &chat_id, &root, &before).await?;
        let service = crate::worktrees::service::fenced_service(
            WorktreeService::new_async(app.gcx.cache_dir.clone(), source.clone()).await?,
            app.runtime.tool_jobs.scopes.clone(),
        );
        let (worktree, reused, reattached) = if let Some(worktree) = before.worktree.as_ref() {
            let view = service.get_worktree(&worktree.id).await?;
            if view.meta != *worktree {
                return Err(
                    "Active worktree registry identity changed; refresh chat before retrying"
                        .into(),
                );
            }
            (view.meta, true, false)
        } else if let Some(worktree) = service.recover_chat_worktree(&chat_id).await? {
            (worktree, false, true)
        } else {
            let created = service.create_chat_worktree(&chat_id).await?;
            (created.worktree.meta, false, false)
        };
        let records = crate::privacy::records::declared_file_records(
            &app.gcx,
            [
                worktree.root.clone(),
                worktree.source_workspace_root.clone(),
                worktree.repo_root.clone(),
            ],
        )
        .map_err(|error| {
            format!(
                "{error}; registered worktree {} was retained for recovery",
                worktree.id
            )
        })?;
        let update = if reattached {
            crate::chat::queue::resolve_recovered_worktree_setparams_update(
                app.clone(),
                &chat_id,
                &before,
                &worktree,
            )
            .await
        } else {
            crate::chat::queue::resolve_worktree_setparams_update(
                app.clone(),
                &chat_id,
                &before,
                &json!({"worktree_id":worktree.id}),
            )
            .await
        }
        .map_err(|error| {
            format!(
                "{error}; registered worktree {} was retained for recovery",
                worktree.id
            )
        })?;
        {
            let mut context = ccx.lock().await;
            let mut session = session_arc.lock().await;
            check_caller(&context, &session)
                .and_then(|_| check_unchanged_attachment(&session, &before))
                .and_then(|_| {
                    if session.work_project_root.as_ref() != Some(&source) {
                        return Err(
                            "Trusted current project changed during worktree creation".into()
                        );
                    }
                    Ok(())
                })
                .map_err(|error| {
                    format!(
                        "{error}; registered worktree {} was retained for recovery",
                        worktree.id
                    )
                })?;
            if !reused {
                let update = update.ok_or("Worktree attachment produced no update")?;
                session.thread.worktree = update.worktree;
                context.execution_scope = Some(ExecutionScope::from_worktree(&worktree));
                session.release_turn_only_state();
                session.emit(ChatEvent::ThreadUpdated {
                    params: json!({"worktree":worktree}),
                });
                let mut activation = crate::chat::queue::worktree_activation_message(&worktree);
                crate::privacy::records::merge_records(&mut activation, records.clone());
                session.add_message(activation);
                session.increment_version();
                session.touch();
            }
        }
        crate::chat::trajectories::try_save_trajectory_with_intent(
            app.clone(),
            session_arc,
            TrajectoryCommitIntent::Required,
        )
        .await
        .map_err(|error| {
            format!(
                "Worktree {} is attached in memory but persistence failed: {error}",
                worktree.id
            )
        })?;
        let mut message = ChatMessage {
            role: "tool".into(), tool_call_id: tool_call_id.clone(),
            content: ChatContent::SimpleText(json!({"status":if reused {"already_attached"} else if reattached {"reattached"} else {"attached"},"worktree":worktree}).to_string()),
            ..Default::default()
        };
        crate::privacy::records::merge_records(&mut message, records);
        Ok((false, vec![ContextEnum::ChatMessage(message)]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fixture() -> (
        tempfile::TempDir,
        Arc<Mutex<AtCommandsContext>>,
        Arc<Mutex<ChatSession>>,
        WorktreeService,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir(&source).unwrap();
        let git = |args: &[&str]| crate::worktrees::git::run_git(&source, args).unwrap();
        git(&["init"]);
        git(&["checkout", "-b", "main"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "Test"]);
        std::fs::write(source.join("tracked"), "committed\n").unwrap();
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
        let mut session = ChatSession::new("worktree-create-root".into());
        session.work_project_root = Some(source.clone());
        session.set_runtime_state(SessionState::ExecutingTools, None);
        let abort = session.abort_flag.clone();
        let session = Arc::new(Mutex::new(session));
        app.chat
            .sessions
            .write()
            .await
            .insert("worktree-create-root".into(), session.clone());
        let context = AtCommandsContext::new_with_abort(
            app,
            4096,
            10,
            false,
            vec![],
            "worktree-create-root".into(),
            None,
            String::new(),
            None,
            None,
            Some(abort),
        )
        .await;
        let service = WorktreeService::new(cache, source).unwrap();
        (temp, Arc::new(Mutex::new(context)), session, service)
    }

    #[tokio::test]
    async fn worktree_create_attaches_scopes_following_write_and_persists() {
        let (_temp, context, session, service) = fixture().await;
        let mut events = session.lock().await.event_tx.subscribe();
        let mut tool = ToolWorktreeCreate;
        let decision = tool
            .match_against_confirm_deny(context.clone(), &HashMap::new())
            .await
            .unwrap();
        assert_eq!(
            decision.result,
            refact_tool_api::MatchConfirmDenyResult::PASS
        );
        let app = context.lock().await.app.clone();
        let call = crate::call_validation::ChatToolCall {
            id: "create".into(),
            index: None,
            function: crate::call_validation::ChatToolFunction {
                name: "worktree_create".into(),
                arguments: "{}".into(),
            },
            tool_type: "function".into(),
            extra_content: None,
            started_at_ms: None,
            completed_at_ms: None,
        };
        for mode in ["agent", "solo"] {
            let mut thread = session.lock().await.thread.clone();
            thread.mode = mode.into();
            thread.auto_approve_editing_tools = false;
            thread.auto_approve_dangerous_commands = false;
            thread.autonomous_no_confirm = false;
            let catalog = app
                .tool_registry
                .acquire_tool_catalog(mode, None, None)
                .await;
            assert!(catalog
                .index
                .tools
                .iter()
                .any(|tool| tool.name == "worktree_create"));
            let (confirmations, denials) = crate::chat::tools::check_tools_confirmation(
                app.clone(),
                std::slice::from_ref(&call),
                &[],
                &thread,
                Some(&catalog),
                mode,
                None,
                &[],
                "",
            )
            .await;
            assert!(confirmations.is_empty(), "{mode}: {confirmations:?}");
            assert!(denials.is_empty(), "{mode}: {denials:?}");
        }
        tool.tool_execute(context.clone(), &"create".into(), &HashMap::new())
            .await
            .unwrap();
        let attached = session.lock().await.thread.worktree.clone().unwrap();
        let scope = context.lock().await.execution_scope.clone().unwrap();
        let resolved = scope
            .resolve_creatable_path(std::path::Path::new("following.txt"))
            .unwrap();
        assert_eq!(resolved.path, attached.root.join("following.txt"));
        std::fs::write(&resolved.path, "isolated").unwrap();
        assert!(!service
            .source_workspace_root()
            .join("following.txt")
            .exists());
        let raw = std::fs::read_to_string(
            service
                .source_workspace_root()
                .join(".refact/trajectories/worktree-create-root/worktree-create-root.json"),
        )
        .unwrap();
        let saved: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(saved["worktree"]["id"], attached.id);
        let mut attached_event = false;
        while let Ok(event) = events.try_recv() {
            let event: Value = serde_json::from_str(&event).unwrap();
            if event["type"] == "thread_updated" && event["worktree"]["id"] == attached.id {
                attached_event = true;
            }
        }
        assert!(attached_event, "attachment must reach the UI event stream");
        tool.tool_execute(context.clone(), &"again".into(), &HashMap::new())
            .await
            .unwrap();
        assert_eq!(
            service.load_registry().await.unwrap().records.len(),
            1,
            "repeated calls must not create nested duplicates"
        );
        assert_eq!(
            session.lock().await.thread.worktree.as_ref().unwrap().id,
            attached.id
        );
        let app = context.lock().await.app.clone();
        let before = session.lock().await.thread.clone();
        let detached = crate::chat::queue::resolve_worktree_setparams_update(
            app,
            "worktree-create-root",
            &before,
            &json!({"worktree": null}),
        )
        .await
        .unwrap()
        .unwrap();
        session.lock().await.thread.worktree = detached.worktree;
        context.lock().await.execution_scope = None;
        assert!(service.load_registry().await.unwrap().records[0]
            .references
            .is_empty());
        let mut recovery_events = session.lock().await.event_tx.subscribe();
        let before_messages = session.lock().await.messages.len();
        let (_, result) = tool
            .tool_execute(context.clone(), &"recover".into(), &HashMap::new())
            .await
            .expect("detached chat must recover its exact registered checkout");
        let ContextEnum::ChatMessage(message) = &result[0] else {
            panic!("tool result missing")
        };
        let result: Value = serde_json::from_str(&message.content.content_text_only()).unwrap();
        assert_eq!(result["status"], "reattached");
        assert_eq!(
            session.lock().await.thread.worktree.as_ref(),
            Some(&attached)
        );
        assert_eq!(
            context.lock().await.execution_scope_worktree(),
            Some(attached.clone())
        );
        assert_eq!(
            std::fs::read_to_string(attached.root.join("following.txt")).unwrap(),
            "isolated"
        );
        let session_guard = session.lock().await;
        let activation = session_guard.messages.last().unwrap();
        assert_eq!(session_guard.messages.len(), before_messages + 1);
        assert_eq!(activation.tool_call_id, "worktree_enabled");
        assert!(activation.extra.get("privacy").is_some());
        drop(session_guard);
        let mut recovered_event = false;
        while let Ok(event) = recovery_events.try_recv() {
            let event: Value = serde_json::from_str(&event).unwrap();
            if event["type"] == "thread_updated" && event["worktree"]["id"] == attached.id {
                recovered_event = true;
            }
        }
        assert!(recovered_event, "recovery must emit attachment to the UI");
        let registry = service.load_registry().await.unwrap();
        assert_eq!(registry.records.len(), 1);
        assert_eq!(registry.records[0].references.len(), 1);
        let saved: Value = serde_json::from_str(
            &std::fs::read_to_string(
                service
                    .source_workspace_root()
                    .join(".refact/trajectories/worktree-create-root/worktree-create-root.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(saved["worktree"]["id"], attached.id);
    }

    #[tokio::test]
    async fn worktree_create_continuation_refuses_incoherent_and_child_callers() {
        let (_temp, context, session, _service) = fixture().await;
        let mut context = context.lock().await;
        let mut session = session.lock().await;
        context.root_chat_id = "original-root".into();
        session.thread.root_chat_id = Some("original-root".into());
        session.thread.parent_id = Some("previous-chat".into());
        session.thread.link_type = Some("handoff".into());
        let thread = session.thread.clone();
        for case in [
            "mismatched-root",
            "missing-parent",
            "unknown-link",
            "depth",
            "background",
            "context-task",
            "session-task",
        ] {
            session.thread = thread.clone();
            context.root_chat_id = "original-root".into();
            context.subchat_depth = 0;
            context.background_agent_id = None;
            context.task_meta = None;
            session.closed = false;
            session.set_runtime_state(SessionState::ExecutingTools, None);
            session.abort_flag.store(false, Ordering::Relaxed);
            context.abort_flag = session.abort_flag.clone();
            let task = crate::chat::types::TaskMeta {
                task_id: "task".into(),
                role: "planner".into(),
                agent_id: None,
                card_id: None,
                planner_chat_id: None,
            };
            match case {
                "mismatched-root" => session.thread.root_chat_id = Some("other-root".into()),
                "missing-parent" => session.thread.parent_id = None,
                "unknown-link" => session.thread.link_type = Some("subagent".into()),
                "depth" => context.subchat_depth = 1,
                "background" => context.background_agent_id = Some("agent".into()),
                "context-task" => context.task_meta = Some(task),
                "session-task" => session.thread.task_meta = Some(task),
                _ => unreachable!(),
            }
            let expected = "worktree_create is available only to an ordinary root chat, not task or child executions";
            assert_eq!(
                check_caller(&context, &session),
                Err(expected.into()),
                "{case}"
            );
        }
    }

    #[tokio::test]
    async fn worktree_create_recovery_refuses_foreign_reference() {
        let (_temp, context, session, service) = fixture().await;
        let created = service
            .create_chat_worktree("worktree-create-root")
            .await
            .unwrap();
        let expected = service
            .recover_chat_worktree("worktree-create-root")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(expected, created.worktree.meta);
        let foreign = crate::worktrees::types::WorktreeReference {
            kind: "chat".into(),
            chat_id: Some("foreign-chat".into()),
            ..Default::default()
        };
        service
            .add_reference(&created.worktree.meta.id, foreign.clone())
            .await
            .unwrap();
        let before = service.load_registry().await.unwrap();
        assert!(before.records[0].references.contains(&foreign));
        let own = crate::worktrees::types::WorktreeReference {
            kind: "chat".into(),
            chat_id: Some("worktree-create-root".into()),
            ..Default::default()
        };
        assert_eq!(
            service
                .add_recovered_chat_reference(&expected.id, "worktree-create-root", &expected, own)
                .await
                .unwrap_err(),
            "Chat worktree recovery refused: checkout ownership does not match"
        );
        assert_eq!(service.load_registry().await.unwrap(), before);
        let error = ToolWorktreeCreate
            .tool_execute(context.clone(), &"foreign".into(), &HashMap::new())
            .await
            .unwrap_err();
        assert_eq!(
            error,
            "Chat worktree recovery refused: checkout ownership does not match"
        );
        assert_eq!(service.load_registry().await.unwrap(), before);
        assert!(session.lock().await.thread.worktree.is_none());
        assert!(context.lock().await.execution_scope.is_none());
        assert!(created.worktree.meta.root.join("tracked").exists());
    }

    #[tokio::test]
    async fn worktree_create_recovery_refuses_unverified_request_identity() {
        let (_temp, context, session, service) = fixture().await;
        let created = service
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                kind: Some("chat".into()),
                chat_id: Some("worktree-create-root".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let before = service.load_registry().await.unwrap();
        assert_eq!(before.records.len(), 1);
        assert_eq!(
            before.records[0].references[0].chat_id.as_deref(),
            Some("worktree-create-root")
        );
        let error = ToolWorktreeCreate
            .tool_execute(context.clone(), &"unverified".into(), &HashMap::new())
            .await
            .unwrap_err();
        assert!(error.contains("already exists"), "wrong refusal: {error}");
        assert_eq!(service.load_registry().await.unwrap(), before);
        assert!(session.lock().await.thread.worktree.is_none());
        assert!(context.lock().await.execution_scope.is_none());
        assert!(created.worktree.meta.root.join("tracked").exists());
    }

    #[tokio::test]
    async fn worktree_create_refuses_child_identity_before_filesystem_mutation() {
        let (_temp, context, session, service) = fixture().await;
        context.lock().await.root_chat_id = "different-parent".into();
        let error = ToolWorktreeCreate
            .tool_execute(context.clone(), &"child".into(), &HashMap::new())
            .await
            .unwrap_err();
        assert_eq!(error, "worktree_create is available only to an ordinary root chat, not task or child executions");
        assert!(session.lock().await.thread.worktree.is_none());
        assert!(
            service.load_registry().await.unwrap().records.is_empty(),
            "refused child must not register a checkout"
        );
    }

    #[tokio::test]
    async fn worktree_create_privacy_failure_leaves_attachment_unchanged() {
        let (_temp, context, session, service) = fixture().await;
        let gcx = context.lock().await.app.gcx.clone();
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
            zones: vec![refact_privacy::Zone {
                name: "normal".into(),
                patterns: vec!["[".into()],
                send_to: vec!["*".into()],
                on_shell_read: refact_privacy::ShellBehavior::Withhold,
            }],
            ..Default::default()
        });
        let expected_error = gcx
            .privacy_policy_load
            .read()
            .unwrap()
            .policy
            .compile()
            .err()
            .expect("the fixture must contain an invalid policy")
            .to_string();
        let before_version = session.lock().await.trajectory_version;
        let mut events = session.lock().await.event_tx.subscribe();
        let error = ToolWorktreeCreate
            .tool_execute(context.clone(), &"privacy".into(), &HashMap::new())
            .await
            .unwrap_err();
        assert!(
            error.starts_with(&format!(
                "privacy policy failed to compile: {expected_error}"
            )),
            "wrong failure: {error}"
        );
        let registry = service.load_registry().await.unwrap();
        assert_eq!(
            registry.records.len(),
            1,
            "the privacy check must run after creation"
        );
        let id = &registry.records[0].meta.id;
        assert!(
            error.contains(id),
            "the retained checkout must be recoverable: {error}"
        );
        assert!(context.lock().await.execution_scope.is_none());
        let session = session.lock().await;
        assert!(session.thread.worktree.is_none());
        assert!(session.messages.is_empty());
        assert_eq!(session.trajectory_version, before_version);
        assert!(matches!(
            events.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
    }
}
