use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use chrono::{TimeDelta, Utc};
use serde_json::json;
use tempfile::tempdir;

use crate::agents::registry::BackgroundAgentRegistry;
use crate::agents::storage::{load_all, save_record};
use crate::agents::types::{
    AgentCompletion, AgentListFilter, BackgroundAgent, BgAgentKind, BgAgentStatus,
    CreateAgentRequest,
};
use crate::app_state::AppState;
use crate::call_validation::ChatMessage;
use crate::chat::types::{ChatCommand, ChatSession};

fn create_request(parent_chat_id: &str, kind: BgAgentKind) -> CreateAgentRequest {
    CreateAgentRequest {
        parent_chat_id: parent_chat_id.to_string(),
        parent_root_chat_id: Some("root-chat".to_string()),
        parent_tool_call_id: Some("tool-call".to_string()),
        kind,
        config_name: match kind {
            BgAgentKind::Subagent => "subagent".to_string(),
            BgAgentKind::Delegate => "delegate_with_editing".to_string(),
        },
        title: "Investigate frogs".to_string(),
        prompt: "Find the frog problem".to_string(),
        target_files: vec!["src/frog.rs".to_string()],
        model: "test-model".to_string(),
    }
}

fn completion(child_chat_id: &str) -> AgentCompletion {
    AgentCompletion {
        result_summary: "fixed frog".to_string(),
        edited_files: vec!["src/frog.rs".to_string()],
        diff_summary: Some("one frog changed".to_string()),
        conflict_summary: None,
        child_chat_id: Some(child_chat_id.to_string()),
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

async fn app_with_parent_session(
    parent_chat_id: &str,
) -> (
    std::sync::Arc<crate::global_context::GlobalContext>,
    AppState,
    Arc<tokio::sync::Mutex<ChatSession>>,
) {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let app = AppState::from_gcx(gcx.clone()).await;
    let session = Arc::new(tokio::sync::Mutex::new(ChatSession::new(
        parent_chat_id.to_string(),
    )));
    app.chat
        .sessions
        .write()
        .await
        .insert(parent_chat_id.to_string(), session.clone());
    (gcx, app, session)
}

#[tokio::test]
async fn create_returns_queued_unique_persisted_records() {
    let (temp, registry) = registry().await;
    let (first, _, _) = registry
        .create(create_request("parent", BgAgentKind::Delegate))
        .await
        .expect("create first");
    let (second, _, _) = registry
        .create(create_request("parent", BgAgentKind::Delegate))
        .await
        .expect("create second");

    assert_eq!(first.status, BgAgentStatus::Queued);
    assert!(first.agent_id.starts_with("bgagent-"));
    assert_ne!(first.agent_id, second.agent_id);
    assert_eq!(first.change_seq, 1);
    assert_eq!(first.target_files, vec!["src/frog.rs"]);

    let records = load_all(temp.path()).await.expect("load");
    assert_eq!(records.get(&first.agent_id), Some(&first));
    assert_eq!(records.get(&second.agent_id), Some(&second));
}

#[tokio::test]
async fn subagent_create_discards_target_files() {
    let (_temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Subagent).await;

    assert!(record.target_files.is_empty());
}

#[tokio::test]
async fn mark_running_transitions_sets_started_bumps_and_persists() {
    let (temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Delegate).await;

    let running = registry
        .mark_running(&record.agent_id, "child-chat".to_string())
        .await
        .expect("running");

    assert_eq!(running.status, BgAgentStatus::Running);
    assert_eq!(running.child_chat_id.as_deref(), Some("child-chat"));
    assert!(running.started_at.is_some());
    assert_eq!(running.change_seq, record.change_seq + 1);
    let records = load_all(temp.path()).await.expect("load");
    assert_eq!(records.get(&record.agent_id), Some(&running));
}

#[tokio::test]
async fn update_progress_bumps_step_count_and_sets_last_activity() {
    let (_temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Delegate).await;

    let updated = registry
        .update_progress(
            &record.agent_id,
            "reading files".to_string(),
            7,
            Some("cat".to_string()),
        )
        .await
        .expect("progress");

    assert_eq!(updated.progress.as_deref(), Some("reading files"));
    assert_eq!(updated.step_count, 7);
    assert_eq!(updated.last_activity.as_deref(), Some("cat"));
    assert_eq!(updated.change_seq, record.change_seq + 1);
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
async fn wait_returns_immediately_when_status_is_terminal() {
    let (_temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    registry
        .mark_completed(&record.agent_id, completion("child-chat"))
        .await
        .expect("completed");

    let waited = registry
        .wait("parent", &record.agent_id, Duration::from_secs(10))
        .await
        .expect("wait");

    assert_eq!(waited.status, BgAgentStatus::Completed);
}

#[tokio::test]
async fn wait_returns_after_parallel_mark_completed() {
    let (_temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    let registry_clone = registry.clone();
    let agent_id = record.agent_id.clone();

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        registry_clone
            .mark_completed(&agent_id, completion("child-chat"))
            .await
            .expect("completed");
    });

    let waited = registry
        .wait("parent", &record.agent_id, Duration::from_secs(2))
        .await
        .expect("wait");

    assert_eq!(waited.status, BgAgentStatus::Completed);
}

#[tokio::test]
async fn wait_times_out_and_returns_current_status() {
    let (_temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    registry
        .mark_running(&record.agent_id, "child-chat".to_string())
        .await
        .expect("running");

    let waited = registry
        .wait("parent", &record.agent_id, Duration::from_millis(20))
        .await
        .expect("wait");

    assert_eq!(waited.status, BgAgentStatus::Running);
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
async fn list_for_parent_filters_by_status_kind_terminal_window_and_limit() {
    let (_temp, registry) = registry().await;
    let running_delegate = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    registry
        .mark_running(&running_delegate.agent_id, "child-running".to_string())
        .await
        .expect("running");
    let completed_delegate = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    registry
        .mark_completed(&completed_delegate.agent_id, completion("child-completed"))
        .await
        .expect("completed");
    let subagent = create_agent(&registry, "parent", BgAgentKind::Subagent).await;
    let other_parent = create_agent(&registry, "other", BgAgentKind::Delegate).await;
    registry
        .mark_running(&other_parent.agent_id, "other-child".to_string())
        .await
        .expect("running other");

    let running = registry
        .list_for_parent(
            "parent",
            AgentListFilter {
                status: Some(vec![BgAgentStatus::Running]),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].agent_id, running_delegate.agent_id);

    let delegates = registry
        .list_for_parent(
            "parent",
            AgentListFilter {
                kind: Some(BgAgentKind::Delegate),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(delegates.len(), 2);
    assert!(delegates
        .iter()
        .all(|record| record.kind == BgAgentKind::Delegate));

    let no_terminals = registry
        .list_for_parent(
            "parent",
            AgentListFilter {
                include_terminal_within_hours: Some(0),
                ..Default::default()
            },
        )
        .await;
    assert!(no_terminals
        .iter()
        .all(|record| record.status != BgAgentStatus::Completed));
    assert!(no_terminals
        .iter()
        .any(|record| record.agent_id == running_delegate.agent_id));
    assert!(no_terminals
        .iter()
        .any(|record| record.agent_id == subagent.agent_id));

    let limited = registry
        .list_for_parent(
            "parent",
            AgentListFilter {
                limit: Some(1),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(limited.len(), 1);
}

#[tokio::test]
async fn persistence_round_trip_save_load_equal() {
    let temp = tempdir().expect("tempdir");
    let registry = BackgroundAgentRegistry::new(temp.path().to_path_buf())
        .await
        .expect("registry");
    let created = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    let completed = registry
        .mark_completed(&created.agent_id, completion("child-chat"))
        .await
        .expect("completed");

    let loaded = load_all(temp.path()).await.expect("load");

    assert_eq!(loaded.get(&completed.agent_id), Some(&completed));
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
            Some("Engine restarted before agent finished. True resume is not supported.")
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
async fn overlap_warning_reports_running_delegate_file_overlap_only() {
    let (_temp, registry) = registry().await;
    let delegate = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    registry
        .mark_running(&delegate.agent_id, "child-running".to_string())
        .await
        .expect("running");
    let subagent = create_agent(&registry, "parent", BgAgentKind::Subagent).await;
    registry
        .mark_running(&subagent.agent_id, "child-subagent".to_string())
        .await
        .expect("subagent running");

    let warning = registry
        .overlap_warning(
            "parent",
            &["src/frog.rs".to_string(), "src/pond.rs".to_string()],
        )
        .await
        .expect("warning");
    assert!(warning.contains(&delegate.agent_id));
    assert!(warning.contains("src/frog.rs"));

    assert!(registry
        .overlap_warning("parent", &["src/toad.rs".to_string()])
        .await
        .is_none());
    assert!(registry
        .overlap_warning("other-parent", &["src/frog.rs".to_string()])
        .await
        .is_none());
}

#[tokio::test]
async fn set_completion_message_id_is_idempotent() {
    let (temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Delegate).await;

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
}

#[tokio::test]
async fn push_completion_to_parent_is_idempotent() {
    let (_gcx, app, session_arc) = app_with_parent_session("parent-push").await;
    let record = create_agent(&app.agents, "parent-push", BgAgentKind::Delegate).await;
    let completed = app
        .agents
        .mark_completed(&record.agent_id, completion("child-push"))
        .await
        .expect("completed");

    crate::agents::push::push_completion_to_parent(app.clone(), &completed)
        .await
        .expect("first push");
    let pushed = app
        .agents
        .get("parent-push", &record.agent_id)
        .await
        .unwrap();
    crate::agents::push::push_completion_to_parent(app, &pushed)
        .await
        .expect("second push");

    let session = session_arc.lock().await;
    assert_eq!(session.command_queue.len(), 1);
    match &session.command_queue.front().unwrap().command {
        ChatCommand::UserMessage { content, .. } => {
            assert!(content
                .as_str()
                .unwrap()
                .contains("[background delegate finished]"));
        }
        _ => panic!("expected UserMessage"),
    }
}

#[tokio::test]
async fn push_completion_to_parent_marks_pending_when_session_not_loaded_and_flush_retries() {
    let (_gcx, app, _session_arc) = app_with_parent_session("parent-flush").await;
    app.chat.sessions.write().await.remove("parent-flush");
    let record = create_agent(&app.agents, "parent-flush", BgAgentKind::Subagent).await;
    let completed = app
        .agents
        .mark_completed(&record.agent_id, completion("child-flush"))
        .await
        .expect("completed");

    crate::agents::push::push_completion_to_parent(app.clone(), &completed)
        .await
        .expect("pending push");
    let pending = app
        .agents
        .get("parent-flush", &record.agent_id)
        .await
        .unwrap();
    assert_eq!(pending.completion_message_id.as_deref(), Some("pending"));

    let session = Arc::new(tokio::sync::Mutex::new(ChatSession::new(
        "parent-flush".to_string(),
    )));
    app.chat
        .sessions
        .write()
        .await
        .insert("parent-flush".to_string(), session.clone());
    let count = crate::agents::push::flush_pending_pushes_for_parent(app.clone(), "parent-flush")
        .await
        .expect("flush");

    assert_eq!(count, 1);
    assert_eq!(session.lock().await.command_queue.len(), 1);
    let updated = app
        .agents
        .get("parent-flush", &record.agent_id)
        .await
        .unwrap();
    assert_ne!(updated.completion_message_id.as_deref(), Some("pending"));
}


#[tokio::test]
async fn spawn_and_wait_timeout_returns_error() {
    let (_gcx, app, _session_arc) = app_with_parent_session("parent-timeout").await;
    let req = crate::agents::spawn::SpawnRequest {
        kind: BgAgentKind::Subagent,
        parent_chat_id: "parent-timeout".to_string(),
        parent_root_chat_id: None,
        parent_tool_call_id: None,
        config_name: "missing-subagent-config".to_string(),
        title: "Missing".to_string(),
        prompt: "prompt".to_string(),
        target_files: vec![],
        max_steps: 1,
        model: "model".to_string(),
        parent_subchat_tx: None,
        parent_worktree: None,
        parent_task_meta: None,
        subchat_depth: 0,
        notify_parent: crate::agents::spawn::NotifyParent::Silent,
    };

    let err = crate::agents::spawn::spawn_and_wait(app, req, Some(Duration::from_millis(1)))
        .await
        .expect_err("missing config should error before waiting");
    assert!(err.contains("not found") || err.contains("missing"));
}

fn spawn_request(parent_chat_id: &str) -> crate::agents::spawn::SpawnRequest {
    crate::agents::spawn::SpawnRequest {
        kind: BgAgentKind::Subagent,
        parent_chat_id: parent_chat_id.to_string(),
        parent_root_chat_id: Some(parent_chat_id.to_string()),
        parent_tool_call_id: None,
        config_name: "test_spawn".to_string(),
        title: "Test spawn".to_string(),
        prompt: "prompt".to_string(),
        target_files: vec![],
        max_steps: 1,
        model: "model".to_string(),
        parent_subchat_tx: None,
        parent_worktree: None,
        parent_task_meta: None,
        subchat_depth: 0,
        notify_parent: crate::agents::spawn::NotifyParent::Silent,
    }
}




#[tokio::test]
async fn storage_save_record_preserves_existing_records() {
    let temp = tempdir().expect("tempdir");
    let registry = BackgroundAgentRegistry::new(temp.path().to_path_buf())
        .await
        .expect("registry");
    let first = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    let second = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    let mut changed = first.clone();
    changed.status = BgAgentStatus::Failed;
    changed.error = Some("manual".to_string());
    changed.finished_at = Some(Utc::now() + TimeDelta::seconds(1));
    changed.last_update_at = changed.finished_at.expect("finished");
    changed.change_seq += 1;

    save_record(temp.path(), &changed).await.expect("save");
    let records = load_all(temp.path()).await.expect("load");

    assert_eq!(records.get(&changed.agent_id), Some(&changed));
    assert_eq!(records.get(&second.agent_id), Some(&second));
}
