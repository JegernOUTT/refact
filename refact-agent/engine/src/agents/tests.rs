use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use serde_json::json;
use tempfile::tempdir;

use crate::agents::registry::{normalize_path_for_overlap, BackgroundAgentRegistry, InboxMessage};
use crate::agents::storage::{load_all, save_record};
use crate::agents::types::{
    AgentCompletion, AgentListFilter, BackgroundAgent, BgAgentKind, BgAgentStatus,
    CreateAgentRequest, NO_TEXT_RESULT_SUMMARY,
};
use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::types::{ChatEvent, ChatSession};
use crate::subchat::{SubchatConfig, SubchatResult};
use crate::tools::tools_description::Tool;
use serial_test::serial;

fn create_request(parent_chat_id: &str, kind: BgAgentKind) -> CreateAgentRequest {
    CreateAgentRequest {
        parent_chat_id: parent_chat_id.to_string(),
        parent_root_chat_id: Some("root-chat".to_string()),
        parent_tool_call_id: Some("tool-call".to_string()),
        kind,
        config_name: "subagent".to_string(),
        title: "Investigate frogs".to_string(),
        prompt: "Find the frog problem".to_string(),
        target_files: vec!["src/frog.rs".to_string()],
        model: "test-model".to_string(),
        model_type: Some("thinking".to_string()),
        goal_summary: Some("Fix the frog problem".to_string()),
        plan_present: true,
        worktree_id: Some("worktree-frog".to_string()),
        worktree_branch: Some("refact/subagent/frog".to_string()),
    }
}

fn completion(child_chat_id: &str) -> AgentCompletion {
    AgentCompletion {
        result_summary: "fixed frog".to_string(),
        edited_files: vec!["src/frog.rs".to_string()],
        diff_summary: Some("one frog changed".to_string()),
        conflict_summary: None,
        child_chat_id: Some(child_chat_id.to_string()),
        deliverable_status: None,
        handoff: None,
    }
}

async fn registry() -> (tempfile::TempDir, std::sync::Arc<BackgroundAgentRegistry>) {
    let temp = tempdir().expect("tempdir");
    let registry = BackgroundAgentRegistry::new(temp.path().to_path_buf())
        .await
        .expect("registry");
    (temp, registry)
}

async fn create_agent(
    registry: &BackgroundAgentRegistry,
    parent_chat_id: &str,
    kind: BgAgentKind,
) -> BackgroundAgent {
    registry
        .create(create_request(parent_chat_id, kind))
        .await
        .expect("create")
        .0
}

pub(super) async fn app_with_parent_session(
    parent_chat_id: &str,
) -> (
    std::sync::Arc<crate::global_context::GlobalContext>,
    AppState,
    Arc<tokio::sync::Mutex<ChatSession>>,
    tempfile::TempDir,
) {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let workspace = tempdir().expect("workspace");
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.path().to_path_buf()];
    let (root, _) = crate::caps::project_model_defaults_state(gcx.clone()).await;
    {
        let mut caps = crate::caps::CodeAssistantCaps::default();
        caps.chat_models.insert(
            "model".into(),
            Arc::new(crate::caps::ChatModelRecord {
                base: crate::caps::BaseModelRecord {
                    id: "model".into(),
                    ..Default::default()
                },
                supports_agent: true,
                supports_tools: true,
                ..Default::default()
            }),
        );
        let mut state = gcx.caps_state.write().await;
        state.caps = Some(Arc::new(caps));
        state.loaded_project_root = Some(root);
        state.last_attempted_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
    let app = AppState::from_gcx(gcx.clone()).await;
    let session = Arc::new(tokio::sync::Mutex::new(ChatSession::new(
        parent_chat_id.to_string(),
    )));
    session.lock().await.work_project_root = Some(workspace.path().canonicalize().unwrap());
    app.chat
        .sessions
        .write()
        .await
        .insert(parent_chat_id.to_string(), session.clone());
    (gcx, app, session, workspace)
}

async fn tool_context(app: AppState, chat_id: &str) -> Arc<tokio::sync::Mutex<AtCommandsContext>> {
    Arc::new(tokio::sync::Mutex::new(
        AtCommandsContext::new_from_app(
            app,
            4096,
            20,
            false,
            vec![],
            chat_id.to_string(),
            None,
            "model".to_string(),
            None,
            None,
        )
        .await,
    ))
}

fn output_text(result: (bool, Vec<ContextEnum>)) -> String {
    match result.1.into_iter().next().expect("tool output") {
        ContextEnum::ChatMessage(message) => match message.content {
            ChatContent::SimpleText(text) => text,
            _ => panic!("expected text output"),
        },
        _ => panic!("expected chat message"),
    }
}

#[tokio::test]
async fn legacy_record_without_introspection_fields_deserializes() {
    let (_temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Subagent).await;
    let mut legacy = serde_json::to_value(record).expect("serialize record");
    let object = legacy.as_object_mut().expect("record object");
    for key in [
        "stop_hook_generation",
        "model_type",
        "current_tool",
        "goal_summary",
        "plan_present",
        "worktree_id",
        "worktree_branch",
        "merge_status",
        "questions",
        "tokens_used",
        "cost_usd",
    ] {
        object.remove(key);
    }

    let parsed: BackgroundAgent = serde_json::from_value(legacy).expect("deserialize legacy");

    assert_eq!(parsed.model_type, None);
    assert_eq!(parsed.stop_hook_generation, None);
    assert_eq!(parsed.current_tool, None);
    assert_eq!(parsed.goal_summary, None);
    assert!(!parsed.plan_present);
    assert_eq!(parsed.worktree_id, None);
    assert_eq!(parsed.worktree_branch, None);
    assert_eq!(parsed.merge_status, None);
    assert!(parsed.questions.is_empty());
    assert_eq!(parsed.tokens_used, 0);
    assert_eq!(parsed.cost_usd, None);
}

#[tokio::test]
async fn terminal_agents_ignore_late_activity_and_progress_updates() {
    let (_temp, registry) = registry().await;
    let activity_record = create_agent(&registry, "parent", BgAgentKind::Subagent).await;
    let progress_record = create_agent(&registry, "parent", BgAgentKind::Subagent).await;

    let completed = registry
        .mark_completed(&activity_record.agent_id, completion("child-activity"))
        .await
        .expect("complete activity record");
    let late_activity = registry
        .update_activity(
            &activity_record.agent_id,
            Some("late activity".to_string()),
            Some(7),
            Some(Some("shell: cargo test".to_string())),
        )
        .await
        .expect("late activity is ignored");

    let failed = registry
        .mark_failed(&progress_record.agent_id, "failed".to_string())
        .await
        .expect("fail progress record");
    let late_progress = registry
        .update_progress(&progress_record.agent_id, "late progress".to_string(), 8)
        .await
        .expect("late progress is ignored");

    assert_eq!(late_activity, completed);
    assert_eq!(late_activity.current_tool, None);
    assert_eq!(late_progress, failed);
    assert_eq!(late_progress.current_tool, None);
}

#[tokio::test]
async fn mark_interrupted_preserves_first_terminal_transition() {
    let (_temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Subagent).await;
    let completed = registry
        .mark_completed(&record.agent_id, completion("child-completed"))
        .await
        .expect("complete");

    let interrupted = registry
        .mark_interrupted(&record.agent_id, "late interruption".to_string())
        .await
        .expect("late interruption is ignored");

    assert_eq!(interrupted, completed);
    assert_eq!(interrupted.status, BgAgentStatus::Completed);
}

#[tokio::test]
async fn mark_completed_writes_result_payload_sets_finished_and_persists() {
    let (temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Delegate).await;

    let completed = registry
        .mark_completed(&record.agent_id, completion("child-chat"))
        .await
        .expect("completed");

    assert_eq!(completed.status, BgAgentStatus::Completed);
    assert_eq!(completed.finished_at, Some(completed.last_update_at));
    assert_eq!(completed.result_summary.as_deref(), Some("fixed frog"));
    assert_eq!(completed.child_chat_id.as_deref(), Some("child-chat"));
    assert_eq!(completed.edited_files, vec!["src/frog.rs"]);
    let payload_path = completed
        .result_payload_path
        .as_ref()
        .expect("result payload path");
    assert!(payload_path.exists());
    let payload: serde_json::Value = serde_json::from_str(
        &tokio::fs::read_to_string(payload_path)
            .await
            .expect("payload"),
    )
    .expect("json");
    assert_eq!(payload["result_summary"], json!("fixed frog"));
    let records = load_all(temp.path()).await.expect("load");
    assert_eq!(records.get(&record.agent_id), Some(&completed));
}

#[tokio::test]
async fn mark_failed_writes_result_payload() {
    let (_temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Delegate).await;

    let failed = registry
        .mark_failed(&record.agent_id, "boom".to_string())
        .await
        .expect("failed");

    let payload_path = failed
        .result_payload_path
        .as_ref()
        .expect("result payload path");
    let payload: serde_json::Value = serde_json::from_str(
        &tokio::fs::read_to_string(payload_path)
            .await
            .expect("payload"),
    )
    .expect("json");
    assert_eq!(payload["status"], json!("failed"));
    assert_eq!(payload["error"], json!("boom"));
    assert_eq!(payload["edited_files"], json!([]));
    assert_eq!(payload["diff_summary"], serde_json::Value::Null);
    assert_eq!(payload["conflict_summary"], serde_json::Value::Null);
}

#[tokio::test]
async fn mark_cancelled_writes_result_payload() {
    let (_temp, registry) = registry().await;
    let default_reason = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    let custom_reason = create_agent(&registry, "parent", BgAgentKind::Delegate).await;

    let cancelled = registry
        .mark_cancelled(&default_reason.agent_id, None)
        .await
        .expect("cancelled");
    let custom_cancelled = registry
        .mark_cancelled(&custom_reason.agent_id, Some("stop".to_string()))
        .await
        .expect("custom cancelled");

    let payload_path = cancelled
        .result_payload_path
        .as_ref()
        .expect("result payload path");
    let payload: serde_json::Value = serde_json::from_str(
        &tokio::fs::read_to_string(payload_path)
            .await
            .expect("payload"),
    )
    .expect("json");
    assert_eq!(payload["status"], json!("cancelled"));
    assert_eq!(payload["error"], json!("Agent was cancelled."));
    assert!(cancelled.error.is_none());

    let custom_payload_path = custom_cancelled
        .result_payload_path
        .as_ref()
        .expect("custom result payload path");
    let custom_payload: serde_json::Value = serde_json::from_str(
        &tokio::fs::read_to_string(custom_payload_path)
            .await
            .expect("custom payload"),
    )
    .expect("json");
    assert_eq!(custom_payload["status"], json!("cancelled"));
    assert_eq!(custom_payload["error"], json!("stop"));
}

#[tokio::test]
async fn mark_failed_cancelled_and_waiting_for_approval_transition_and_persist() {
    let (temp, registry) = registry().await;
    let waiting_record = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    let failed_record = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    let cancelled_record = create_agent(&registry, "parent", BgAgentKind::Delegate).await;

    let waiting = registry
        .mark_waiting_for_approval(&waiting_record.agent_id)
        .await
        .expect("waiting");
    let failed = registry
        .mark_failed(&failed_record.agent_id, "boom".to_string())
        .await
        .expect("failed");
    let cancelled = registry
        .mark_cancelled(&cancelled_record.agent_id, Some("stop".to_string()))
        .await
        .expect("cancelled");

    assert_eq!(waiting.status, BgAgentStatus::WaitingForApproval);
    assert_eq!(failed.status, BgAgentStatus::Failed);
    assert_eq!(failed.error.as_deref(), Some("boom"));
    assert!(failed.finished_at.is_some());
    assert_eq!(cancelled.status, BgAgentStatus::Cancelled);
    assert_eq!(cancelled.error.as_deref(), Some("stop"));
    assert!(cancelled.finished_at.is_some());

    let records = load_all(temp.path()).await.expect("load");
    assert_eq!(records.get(&waiting.agent_id), Some(&waiting));
    assert_eq!(records.get(&failed.agent_id), Some(&failed));
    assert_eq!(records.get(&cancelled.agent_id), Some(&cancelled));
}

#[tokio::test]
async fn cancelled_agent_ignores_late_mark_completed() {
    let (temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    let running = registry
        .mark_running(&record.agent_id, "child-running".to_string())
        .await
        .expect("running");
    let cancelled = registry
        .cancel("parent", &record.agent_id, Some("stop".to_string()))
        .await
        .expect("cancelled");

    let late_completed = registry
        .mark_completed(&record.agent_id, completion("child-late"))
        .await
        .expect("late completed");
    let final_record = registry.get("parent", &record.agent_id).await.expect("get");

    assert_eq!(cancelled.status, BgAgentStatus::Cancelled);
    assert_eq!(cancelled.change_seq, running.change_seq + 1);
    assert_eq!(late_completed, cancelled);
    assert_eq!(final_record, cancelled);
    assert!(late_completed.result_payload_path.is_some());
    let records = load_all(temp.path()).await.expect("load");
    assert_eq!(records.get(&record.agent_id), Some(&cancelled));
}

#[tokio::test]
async fn cancelled_agent_ignores_late_mark_failed() {
    let (_temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    registry
        .mark_running(&record.agent_id, "child-running".to_string())
        .await
        .expect("running");
    let cancelled = registry
        .cancel("parent", &record.agent_id, Some("stop".to_string()))
        .await
        .expect("cancelled");

    let late_failed = registry
        .mark_failed(&record.agent_id, "boom".to_string())
        .await
        .expect("late failed");
    let final_record = registry.get("parent", &record.agent_id).await.expect("get");

    assert_eq!(late_failed, cancelled);
    assert_eq!(final_record.status, BgAgentStatus::Cancelled);
    assert_eq!(final_record.error.as_deref(), Some("stop"));
    assert_eq!(final_record.change_seq, cancelled.change_seq);
}

#[tokio::test]
async fn cancelled_agent_ignores_late_running_and_waiting_transitions() {
    let (_temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    let cancelled = registry
        .mark_cancelled(&record.agent_id, Some("stop".to_string()))
        .await
        .expect("cancelled");

    let running = registry
        .mark_running(&record.agent_id, "child-late".to_string())
        .await
        .expect("late running");
    let waiting = registry
        .mark_waiting_for_approval(&record.agent_id)
        .await
        .expect("late waiting");

    assert_eq!(running, cancelled);
    assert_eq!(waiting, cancelled);
}

#[tokio::test]
async fn mark_completed_on_completed_is_no_op() {
    let (_temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    let completed = registry
        .mark_completed(&record.agent_id, completion("child-first"))
        .await
        .expect("completed");

    let late_completed = registry
        .mark_completed(&record.agent_id, completion("child-late"))
        .await
        .expect("late completed");
    let final_record = registry.get("parent", &record.agent_id).await.expect("get");

    assert_eq!(late_completed, completed);
    assert_eq!(final_record, completed);
    assert_eq!(final_record.child_chat_id.as_deref(), Some("child-first"));
}

#[tokio::test]
async fn mark_cancelled_and_cancel_on_completed_are_no_ops() {
    let (_temp, registry) = registry().await;
    let (record, abort_flag, _) = registry
        .create(create_request("parent", BgAgentKind::Delegate))
        .await
        .expect("create");
    let completed = registry
        .mark_completed(&record.agent_id, completion("child-first"))
        .await
        .expect("completed");

    let mark_cancelled = registry
        .mark_cancelled(&record.agent_id, Some("too late".to_string()))
        .await
        .expect("mark cancelled");
    let cancel = registry
        .cancel(
            "parent",
            &record.agent_id,
            Some("also too late".to_string()),
        )
        .await
        .expect("cancel");

    assert_eq!(mark_cancelled, completed);
    assert_eq!(cancel, completed);
    assert!(!abort_flag.load(Ordering::SeqCst));
}

#[tokio::test]
async fn cancel_flips_abort_flag_and_marks_cancelled() {
    let (_temp, registry) = registry().await;
    let (record, abort_flag, _) = registry
        .create(create_request("parent", BgAgentKind::Delegate))
        .await
        .expect("create");

    let cancelled = registry
        .cancel("parent", &record.agent_id, Some("nope".to_string()))
        .await
        .expect("cancel");

    assert!(abort_flag.load(Ordering::SeqCst));
    assert_eq!(cancelled.status, BgAgentStatus::Cancelled);
    assert_eq!(cancelled.error.as_deref(), Some("nope"));
}

#[tokio::test]
async fn parent_scoping_hides_get_wait_and_cancel_from_other_parents() {
    let (_temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Delegate).await;

    assert_eq!(
        registry
            .get("other-parent", &record.agent_id)
            .await
            .expect_err("get err"),
        "agent not found"
    );
    assert_eq!(
        registry
            .wait("other-parent", &record.agent_id, Duration::from_millis(1))
            .await
            .expect_err("wait err"),
        "agent not found"
    );
    assert_eq!(
        registry
            .cancel("other-parent", &record.agent_id, None)
            .await
            .expect_err("cancel err"),
        "agent not found"
    );
}

#[tokio::test]
async fn restart_recovery_interrupts_active_records() {
    let temp = tempdir().expect("tempdir");
    let registry = BackgroundAgentRegistry::new(temp.path().to_path_buf())
        .await
        .expect("registry");
    let running = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    let waiting = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    let queued = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    let completed = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    registry
        .mark_running(&running.agent_id, "child-running".to_string())
        .await
        .expect("running");
    registry
        .mark_waiting_for_approval(&waiting.agent_id)
        .await
        .expect("waiting");
    registry
        .mark_completed(&completed.agent_id, completion("child-completed"))
        .await
        .expect("completed");
    drop(registry);

    let restarted = BackgroundAgentRegistry::new(temp.path().to_path_buf())
        .await
        .expect("restart");

    for agent_id in [&running.agent_id, &waiting.agent_id, &queued.agent_id] {
        let record = restarted.get("parent", agent_id).await.expect("record");
        assert_eq!(record.status, BgAgentStatus::Interrupted);
        assert_eq!(
            record.error.as_deref(),
            Some("Engine restarted before agent finished. Inspect the saved child before explicit recovery; no work was replayed.")
        );
        assert!(record.finished_at.is_some());
    }
    let completed_after = restarted
        .get("parent", &completed.agent_id)
        .await
        .expect("completed");
    assert_eq!(completed_after.status, BgAgentStatus::Completed);
}

#[tokio::test]
async fn set_completion_message_id_is_idempotent() {
    let (temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    let record = registry
        .mark_failed(&record.agent_id, "finished".into())
        .await
        .unwrap();

    registry
        .set_completion_message_id(&record.agent_id, "message-one".to_string())
        .await
        .expect("first");
    registry
        .set_completion_message_id(&record.agent_id, "message-two".to_string())
        .await
        .expect("second");

    let updated = registry.get("parent", &record.agent_id).await.expect("get");
    assert_eq!(
        updated.completion_message_id.as_deref(),
        Some("message-one")
    );
    assert!(updated.completion_pushed_at.is_some());
    assert_eq!(updated.change_seq, record.change_seq + 1);
    let records = load_all(temp.path()).await.expect("load");
    assert_eq!(
        records
            .get(&record.agent_id)
            .and_then(|record| record.completion_message_id.as_deref()),
        Some("message-one")
    );
}

#[tokio::test]
async fn set_completion_message_id_allows_pending_and_deferred_retry_markers_to_advance() {
    let (_temp, registry) = registry().await;
    let first = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    let second = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    registry
        .mark_failed(&first.agent_id, "finished".into())
        .await
        .unwrap();
    registry
        .mark_failed(&second.agent_id, "finished".into())
        .await
        .unwrap();

    registry
        .set_completion_message_id(&first.agent_id, "pending".to_string())
        .await
        .expect("pending");
    registry
        .set_completion_message_id(&first.agent_id, "message-one".to_string())
        .await
        .expect("message");
    registry
        .set_completion_message_id(&second.agent_id, "deferred".to_string())
        .await
        .expect("deferred");
    registry
        .set_completion_message_id(&second.agent_id, "pending".to_string())
        .await
        .expect("pending ignored");
    registry
        .set_completion_message_id(&second.agent_id, "message-two".to_string())
        .await
        .expect("message");

    let first = registry
        .get("parent", &first.agent_id)
        .await
        .expect("first");
    let second = registry
        .get("parent", &second.agent_id)
        .await
        .expect("second");
    assert_eq!(first.completion_message_id.as_deref(), Some("message-one"));
    assert_eq!(second.completion_message_id.as_deref(), Some("message-two"));
    assert!(first.deferred_at.is_none());
    assert!(second.deferred_at.is_none());
}

#[tokio::test]
async fn background_completion_burst_pushes_every_notice_without_queue_growth() {
    let (_gcx, app, session_arc, _workspace) = app_with_parent_session("parent-burst").await;
    session_arc
        .lock()
        .await
        .queue_processor_running
        .store(true, Ordering::SeqCst);
    let mut completed = Vec::new();
    for index in 0..6 {
        let record = create_agent(&app.agents, "parent-burst", BgAgentKind::Delegate).await;
        completed.push(
            app.agents
                .mark_completed(
                    &record.agent_id,
                    completion(&format!("child-burst-{index}")),
                )
                .await
                .expect("completed"),
        );
    }

    for record in &completed {
        crate::agents::push::push_completion_to_parent(app.clone(), record)
            .await
            .expect("push");
    }

    let (notice_count, wake_count) = {
        let session = session_arc.lock().await;
        (
            agents_spawn_system_notice_count(&session),
            session.delivery_wake_sources.len(),
        )
    };
    assert_eq!(notice_count, 6);
    assert_eq!(wake_count, 1);
    assert!(session_arc.lock().await.command_queue.is_empty());
    for record in completed {
        let updated = app
            .agents
            .get("parent-burst", &record.agent_id)
            .await
            .expect("updated");
        assert_ne!(updated.completion_message_id.as_deref(), Some("deferred"));
        assert_ne!(updated.completion_message_id.as_deref(), Some("pending"));
        assert!(updated.completion_pushed_at.is_some());
        assert!(updated.deferred_at.is_none());
    }
}

#[serial(test_runner)]
#[tokio::test]
async fn spawn_and_wait_times_out_when_runner_hangs() {
    let runner_started = Arc::new(tokio::sync::Notify::new());
    let _runner = {
        let runner_started = runner_started.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, _messages, _config| {
            let runner_started = runner_started.clone();
            Box::pin(async move {
                runner_started.notify_one();
                std::future::pending::<Result<SubchatResult, String>>().await
            })
        }))
    };
    let (_gcx, app, _session_arc, _workspace) =
        app_with_parent_session("parent-wait-timeout").await;
    let mut req = subagent_spawn_request("parent-wait-timeout", "src/frog.rs");
    req.notify_parent = crate::agents::spawn::NotifyParent::Silent;
    let wait = crate::agents::spawn::spawn_and_wait(app.clone(), req, Some(Duration::from_secs(5)));
    tokio::pin!(wait);

    tokio::select! {
        result = &mut wait => panic!("spawn finished before the runner started: {result:?}"),
        _ = runner_started.notified() => {}
    }
    tokio::time::pause();
    assert!(futures::poll!(&mut wait).is_pending());
    tokio::time::advance(Duration::from_secs(5)).await;
    let err = wait.await.expect_err("spawn should time out");

    assert_eq!(err, "background agent timed out");
    let records = app
        .agents
        .list_for_parent("parent-wait-timeout", AgentListFilter::default())
        .await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, BgAgentStatus::Running);
}

#[tokio::test]
async fn spawn_rejects_missing_configuration_before_admission() {
    let (_gcx, app, _session_arc, _workspace) = app_with_parent_session("parent-timeout").await;
    let req = crate::agents::spawn::SpawnRequest {
        native_supervision_guard: None,
        kind: BgAgentKind::Subagent,
        parent_chat_id: "parent-timeout".to_string(),
        parent_root_chat_id: None,
        parent_tool_call_id: None,
        config_name: "missing-subagent-config".to_string(),
        title: "Missing".to_string(),
        prompt: "prompt".to_string(),
        initial_context_messages: Vec::new(),
        history_source_model: None,
        custom_instructions: None,
        tools: None,
        target_files: vec![],
        max_steps: 1,
        model: "model".to_string(),
        model_type: None,
        context_tokens_cap: None,
        goal: None,
        plan: None,
        deliverable: None,

        profile: None,
        worktree_mode: crate::agents::spawn::SpawnWorktreeMode::Inherit,
        parent_subchat_tx: None,
        parent_worktree: None,
        parent_task_meta: None,
        subchat_depth: 0,
        notify_parent: crate::agents::spawn::NotifyParent::Silent,
        completion_push: Default::default(),
    };

    let err =
        crate::agents::spawn::spawn_and_wait(app.clone(), req, Some(Duration::from_millis(1)))
            .await
            .expect_err("missing config should error before waiting");
    assert_eq!(err, "subagent config 'missing-subagent-config' not found");
    assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
    assert!(app.agents.list_all().await.is_empty());
}

pub(super) fn subagent_spawn_request(
    parent_chat_id: &str,
    target_file: &str,
) -> crate::agents::spawn::SpawnRequest {
    crate::agents::spawn::SpawnRequest {
        native_supervision_guard: None,
        kind: BgAgentKind::Subagent,
        parent_chat_id: parent_chat_id.to_string(),
        parent_root_chat_id: Some(parent_chat_id.to_string()),
        parent_tool_call_id: None,
        config_name: "test_spawn".to_string(),
        title: format!("Edit {target_file}"),
        prompt: format!("Edit {target_file}"),
        initial_context_messages: Vec::new(),
        history_source_model: None,
        custom_instructions: None,
        tools: None,
        target_files: vec![target_file.to_string()],
        max_steps: 1,
        model: "model".to_string(),
        model_type: None,
        context_tokens_cap: None,
        goal: None,
        plan: None,
        deliverable: None,

        profile: None,
        worktree_mode: crate::agents::spawn::SpawnWorktreeMode::Inherit,
        parent_subchat_tx: None,
        parent_worktree: None,
        parent_task_meta: None,
        subchat_depth: 0,
        notify_parent: crate::agents::spawn::NotifyParent::Auto,
        completion_push: Default::default(),
    }
}

#[serial(test_runner)]
#[tokio::test]
async fn spawn_seed_installs_hidden_plan_goal_and_caps_steps() {
    let observed = Arc::new(tokio::sync::Mutex::new(None));
    let _runner = {
        let observed = observed.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, messages, config| {
            let observed = observed.clone();
            Box::pin(async move {
                *observed.lock().await = Some((messages.clone(), config.max_steps));
                let mut messages = messages;
                messages.push(ChatMessage::new(
                    "assistant".to_string(),
                    "done".to_string(),
                ));
                Ok(SubchatResult {
                    messages,
                    metering: serde_json::Map::new(),
                    chat_id: config.chat_id,
                    aborted: false,
                })
            })
        }))
    };
    let (_gcx, app, _session, _workspace) = app_with_parent_session("parent-hidden-goal").await;
    let mut req = subagent_spawn_request("parent-hidden-goal", "src/frog.rs");
    req.max_steps = 10;
    req.plan = Some("Plan body".to_string());
    req.goal = Some(crate::agents::spawn::SpawnGoal {
        content: "Goal body".to_string(),
        criteria: vec![crate::chat::types::GoalCriterion {
            id: "C1".to_string(),
            text: "Criterion".to_string(),
            verify_hint: None,
            verify_command: None,
        }],
        budget: Some(crate::chat::types::GoalBudget {
            max_turns: Some(2),
            ..Default::default()
        }),
    });
    req.notify_parent = crate::agents::spawn::NotifyParent::Silent;

    let completed = crate::agents::spawn::spawn_and_wait(app, req, Some(Duration::from_secs(2)))
        .await
        .expect("spawn completed");
    let (messages, max_steps) = observed.lock().await.take().expect("runner observed seed");

    assert_eq!(completed.goal_summary.as_deref(), Some("Goal body"));
    assert!(completed.plan_present);
    assert_eq!(max_steps, 2);
    let plan = refact_chat_api::goal_role::current_goal_plan(
        &messages,
        crate::runtime_settings::plan_projection_limits(),
        false,
    )
    .unwrap();
    assert_eq!(plan.mode, "agent");
    assert_eq!(plan.version, 1);
    assert_eq!(plan.content, "Plan body");
    let goal = messages
        .iter()
        .find(|message| message.role == "goal")
        .unwrap();
    assert_eq!(goal.extra["goal"]["active"], true);
    assert_eq!(goal.extra["goal"]["budget"]["max_turns"], 2);
    assert_eq!(goal.extra["goal"]["criteria"][0]["id"], "C1");
}

fn agents_spawn_system_notices(session: &ChatSession) -> Vec<&ChatMessage> {
    session
        .messages
        .iter()
        .filter(|message| {
            message.role == "event"
                && message
                    .extra
                    .get("event")
                    .and_then(|event| event.get("subkind"))
                    .and_then(|subkind| subkind.as_str())
                    == Some("system_notice")
                && message
                    .extra
                    .get("event")
                    .and_then(|event| event.get("source"))
                    .and_then(|source| source.as_str())
                    == Some("agents.spawn")
        })
        .collect()
}

fn agents_spawn_system_notice_count(session: &ChatSession) -> usize {
    agents_spawn_system_notices(session).len()
}

fn install_spawn_runner(abort_seen: Arc<AtomicBool>) -> crate::agents::spawn::TestRunnerGuard {
    install_spawn_runner_with_start(abort_seen, Arc::new(AtomicBool::new(false)))
}

fn install_spawn_runner_with_start(
    abort_seen: Arc<AtomicBool>,
    started: Arc<AtomicBool>,
) -> crate::agents::spawn::TestRunnerGuard {
    crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, messages, config| {
        let abort_seen = abort_seen.clone();
        started.store(true, Ordering::SeqCst);
        Box::pin(async move { stub_spawn_runner(abort_seen, messages, config).await })
    }))
}

async fn stub_spawn_runner(
    abort_seen: Arc<AtomicBool>,
    mut messages: Vec<ChatMessage>,
    config: SubchatConfig,
) -> Result<SubchatResult, String> {
    if config.title.as_deref() == Some("Cancel me") {
        loop {
            if config
                .abort_flag
                .as_ref()
                .map_or(false, |flag| flag.load(Ordering::SeqCst))
            {
                abort_seen.store(true, Ordering::SeqCst);
                return Err("Aborted by test".to_string());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    tokio::time::sleep(Duration::from_millis(30)).await;

    let target = messages
        .iter()
        .rev()
        .find(|message| message.role == "user" || message.role == "event")
        .map(|message| message.content.content_text_only())
        .unwrap_or_else(|| "background work".to_string());
    messages.push(ChatMessage::new(
        "assistant".to_string(),
        format!("Status: DONE\nCompleted {target}"),
    ));
    Ok(SubchatResult {
        messages,
        metering: serde_json::Map::new(),
        chat_id: config.chat_id,
        aborted: false,
    })
}

#[serial(test_runner)]
#[tokio::test]
async fn background_agent_final_integration_spawn_push_list_cancel_and_restart() {
    let abort_seen = Arc::new(AtomicBool::new(false));
    let runner_started = Arc::new(AtomicBool::new(false));
    let _runner = install_spawn_runner_with_start(abort_seen.clone(), runner_started.clone());
    let (_gcx, app, session_arc, _workspace) = app_with_parent_session("parent-final").await;
    session_arc
        .lock()
        .await
        .queue_processor_running
        .store(true, Ordering::SeqCst);

    let first = crate::agents::spawn::spawn_background_agent(
        app.clone(),
        subagent_spawn_request("parent-final", "src/frog.rs"),
    )
    .await
    .expect("first spawn");
    let warning = app
        .agents
        .overlap_warning("parent-final", &["src/frog.rs".to_string()])
        .await
        .expect("overlap warning");
    assert!(warning.contains(&first.agent_id));

    let second = crate::agents::spawn::spawn_background_agent(
        app.clone(),
        subagent_spawn_request("parent-final", "src/frog.rs"),
    )
    .await
    .expect("second spawn");

    let completed_first = first.completion_rx.await.expect("first completion");
    let completed_second = second.completion_rx.await.expect("second completion");
    assert_eq!(completed_first.status, BgAgentStatus::Completed);
    assert_eq!(completed_second.status, BgAgentStatus::Completed);

    {
        let session = session_arc.lock().await;
        assert_eq!(agents_spawn_system_notice_count(&session), 2);
    }

    let listed = app
        .agents
        .list_for_parent("parent-final", AgentListFilter::default())
        .await;
    assert_eq!(listed.len(), 2);
    assert!(listed
        .iter()
        .all(|record| record.status == BgAgentStatus::Completed));

    let mut cancel_req = subagent_spawn_request("parent-final", "src/toad.rs");
    cancel_req.title = "Cancel me".to_string();
    cancel_req.prompt = "wait until cancelled".to_string();
    cancel_req.notify_parent = crate::agents::spawn::NotifyParent::Silent;
    runner_started.store(false, Ordering::SeqCst);
    let cancel_handle = crate::agents::spawn::spawn_background_agent(app.clone(), cancel_req)
        .await
        .expect("cancel spawn");
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if runner_started.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("cancel target started running");
    let cancelled = app
        .agents
        .cancel(
            "parent-final",
            &cancel_handle.agent_id,
            Some("stop".to_string()),
        )
        .await
        .expect("cancel");
    assert_eq!(cancelled.status, BgAgentStatus::Cancelled);
    let cancelled_final = cancel_handle
        .completion_rx
        .await
        .expect("cancel completion");
    assert_eq!(cancelled_final.status, BgAgentStatus::Cancelled);
    assert!(abort_seen.load(Ordering::SeqCst));

    let temp = tempdir().expect("tempdir");
    let registry = BackgroundAgentRegistry::new(temp.path().to_path_buf())
        .await
        .expect("registry");
    let active = create_agent(&registry, "restart-parent", BgAgentKind::Delegate).await;
    registry
        .mark_running(&active.agent_id, "restart-child".to_string())
        .await
        .expect("running");
    drop(registry);
    let restarted = BackgroundAgentRegistry::new(temp.path().to_path_buf())
        .await
        .expect("restart");
    let interrupted = restarted
        .get("restart-parent", &active.agent_id)
        .await
        .expect("interrupted");
    assert_eq!(interrupted.status, BgAgentStatus::Interrupted);
    let app = AppState {
        agents: restarted,
        ..app.clone()
    };
    app.chat
        .sessions
        .write()
        .await
        .insert("restart-parent".to_string(), session_arc.clone());
    crate::agents::push::push_completion_to_parent(app, &interrupted)
        .await
        .expect("recovery push");
    let session = session_arc.lock().await;
    assert_eq!(agents_spawn_system_notice_count(&session), 3);
}

#[tokio::test]
async fn runner_pending_recovery_retains_followups_until_explicit_continue() {
    let (_gcx, app, session, _workspace) = app_with_parent_session("runner-recovery").await;
    let record = create_agent(&app.agents, "parent", BgAgentKind::Subagent).await;
    app.agents
        .mark_running(&record.agent_id, "runner-recovery".to_string())
        .await
        .unwrap();
    let delivery = refact_core::chat_types::PendingDelivery::with_id(
        "recover-id",
        vec![ChatMessage::new("user".into(), "recover me".into())],
        refact_core::chat_types::PushMode::WhenIdle,
        "test",
        true,
    );
    app.agents
        .enqueue_delivery(&record.agent_id, delivery)
        .await
        .unwrap();
    assert_eq!(
        crate::agents::delivery::recover_runner_deliveries(&app, "runner-recovery")
            .await
            .unwrap(),
        0
    );
    app.agents
        .mark_interrupted(&record.agent_id, "restart".into())
        .await
        .unwrap();
    assert_eq!(
        crate::agents::delivery::recover_runner_deliveries(&app, "runner-recovery")
            .await
            .unwrap(),
        0
    );
    assert!(app
        .agents
        .pending_deliveries(&record.agent_id)
        .await
        .is_empty());
    let record = app.agents.get_any(&record.agent_id).await.unwrap();
    assert_eq!(record.followups.len(), 1);
    assert_eq!(record.followups[0].id, "recover-id");
    let session = session.lock().await;
    assert!(!session.messages.iter().any(
        |message| refact_core::chat_types::delivery_id_of_message(message) == Some("recover-id")
    ));
    assert!(session
        .pending_deliveries
        .iter()
        .all(|delivery| !delivery.wake));
}
