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
    tempfile::TempDir,
) {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let workspace = tempdir().expect("workspace");
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.path().to_path_buf()];
    let app = AppState::from_gcx(gcx.clone()).await;
    let session = Arc::new(tokio::sync::Mutex::new(ChatSession::new(
        parent_chat_id.to_string(),
    )));
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
async fn subagent_create_retains_target_files() {
    let (_temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Subagent).await;

    assert_eq!(record.target_files, vec!["src/frog.rs"]);
}

#[tokio::test]
async fn legacy_record_without_introspection_fields_deserializes() {
    let (_temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Subagent).await;
    let mut legacy = serde_json::to_value(record).expect("serialize record");
    let object = legacy.as_object_mut().expect("record object");
    for key in [
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
        .update_progress(&record.agent_id, "reading files".to_string(), 7)
        .await
        .expect("progress");

    assert_eq!(updated.progress.as_deref(), Some("reading files"));
    assert_eq!(updated.step_count, 7);
    assert!(
        chrono::DateTime::parse_from_rfc3339(updated.last_activity.as_deref().unwrap()).is_ok()
    );
    assert_eq!(updated.change_seq, record.change_seq + 1);
}

#[tokio::test]
async fn update_activity_changes_only_requested_fields() {
    let (_temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Subagent).await;

    let first = registry
        .update_activity(
            &record.agent_id,
            Some("reading files".to_string()),
            Some(7),
            Some(Some("cat: src/frog.rs".to_string())),
        )
        .await
        .expect("first activity update");
    let second = registry
        .update_activity(&record.agent_id, None, None, Some(None))
        .await
        .expect("clear tool");

    assert_eq!(first.progress.as_deref(), Some("reading files"));
    assert_eq!(first.step_count, 7);
    assert_eq!(first.current_tool.as_deref(), Some("cat: src/frog.rs"));
    assert_eq!(second.progress, first.progress);
    assert_eq!(second.step_count, first.step_count);
    assert_eq!(second.current_tool, None);
    assert_eq!(second.change_seq, first.change_seq + 1);
}

#[tokio::test]
async fn terminal_transitions_clear_current_tool() {
    let (_temp, registry) = registry().await;
    let completed = create_agent(&registry, "parent", BgAgentKind::Subagent).await;
    let failed = create_agent(&registry, "parent", BgAgentKind::Subagent).await;
    let cancelled = create_agent(&registry, "parent", BgAgentKind::Subagent).await;
    let interrupted = create_agent(&registry, "parent", BgAgentKind::Subagent).await;

    for record in [&completed, &failed, &cancelled, &interrupted] {
        registry
            .update_activity(
                &record.agent_id,
                None,
                None,
                Some(Some("shell: cargo test".to_string())),
            )
            .await
            .expect("set current tool");
    }

    let completed = registry
        .mark_completed(&completed.agent_id, completion("child-completed"))
        .await
        .expect("complete");
    let failed = registry
        .mark_failed(&failed.agent_id, "failed".to_string())
        .await
        .expect("fail");
    let cancelled = registry
        .mark_cancelled(&cancelled.agent_id, Some("cancelled".to_string()))
        .await
        .expect("cancel");
    let interrupted = registry
        .mark_interrupted(&interrupted.agent_id, "interrupted".to_string())
        .await
        .expect("interrupt");

    for record in [completed, failed, cancelled, interrupted] {
        assert_eq!(record.current_tool, None);
    }
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
async fn find_agent_id_by_child_chat_id_returns_only_the_matched_id() {
    let (_temp, registry) = registry().await;
    let first = create_agent(&registry, "parent", BgAgentKind::Subagent).await;
    let second = create_agent(&registry, "parent", BgAgentKind::Subagent).await;
    registry
        .mark_running(&first.agent_id, "subchat-first".to_string())
        .await
        .expect("start first");
    registry
        .mark_running(&second.agent_id, "subchat-second".to_string())
        .await
        .expect("start second");

    assert_eq!(
        registry
            .find_agent_id_by_child_chat_id("subchat-second")
            .await,
        Some(second.agent_id)
    );
    assert_eq!(
        registry
            .find_agent_id_by_child_chat_id("subchat-missing")
            .await,
        None
    );
}

#[tokio::test]
async fn usage_question_and_inbox_lifecycle_persist_and_notify() {
    let (_temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Subagent).await;

    registry
        .add_usage(&record.agent_id, 10, Some(0.25))
        .await
        .expect("first usage");
    let usage = registry
        .add_usage(&record.agent_id, 5, None)
        .await
        .expect("second usage");
    let (questioned, question_id) = registry
        .add_question(&record.agent_id, "Can I edit the pond?".to_string())
        .await
        .expect("add question");
    let answered = registry
        .answer_question(&record.agent_id, &question_id, "Yes".to_string())
        .await
        .expect("answer question");
    let duplicate = registry
        .answer_question(&record.agent_id, &question_id, "Again".to_string())
        .await
        .expect_err("duplicate answer fails");

    assert_eq!(usage.tokens_used, 15);
    assert_eq!(usage.cost_usd, Some(0.25));
    assert_eq!(question_id.len(), 8);
    assert_eq!(questioned.questions.len(), 1);
    assert_eq!(answered.questions[0].answer.as_deref(), Some("Yes"));
    assert!(answered.questions[0].answered_at.is_some());
    assert_eq!(duplicate, "question already answered");

    let inbox = registry.inbox_for(&record.agent_id).await.expect("inbox");
    registry
        .push_inbox(
            &record.agent_id,
            InboxMessage {
                from: "parent".to_string(),
                text: "Please prioritize tests".to_string(),
                queued_at: Utc::now(),
            },
        )
        .await
        .expect("push inbox");
    let messages = inbox.lock().await;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].from, "parent");
    drop(messages);
    let missing = registry
        .push_inbox(
            "missing",
            InboxMessage {
                from: "parent".to_string(),
                text: "hello".to_string(),
                queued_at: Utc::now(),
            },
        )
        .await
        .expect_err("missing runtime errors");
    assert_eq!(missing, "agent not found");
}

#[tokio::test]
async fn terminal_agent_rejects_late_question() {
    let (_temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Subagent).await;
    registry
        .mark_completed(&record.agent_id, completion("child-question"))
        .await
        .expect("complete");

    assert_eq!(
        registry
            .add_question(&record.agent_id, "Can I edit the pond?".to_string())
            .await
            .expect_err("terminal agent rejects questions"),
        "agent already finished"
    );
}

#[tokio::test]
async fn list_descendants_walks_two_levels() {
    let (_temp, registry) = registry().await;
    let root = create_agent(&registry, "parent", BgAgentKind::Subagent).await;
    let root = registry
        .mark_running(&root.agent_id, "child-chat-1".to_string())
        .await
        .expect("root running");
    let child = create_agent(&registry, "child-chat-1", BgAgentKind::Subagent).await;
    let child = registry
        .mark_running(&child.agent_id, "child-chat-2".to_string())
        .await
        .expect("child running");
    let grandchild = create_agent(&registry, "child-chat-2", BgAgentKind::Subagent).await;

    let descendants = registry.list_descendants(&root.agent_id).await;

    assert_eq!(
        descendants
            .iter()
            .map(|record| record.agent_id.as_str())
            .collect::<Vec<_>>(),
        vec![child.agent_id.as_str(), grandchild.agent_id.as_str()]
    );
}

#[test]
fn summary_maps_introspection_and_question_state() {
    let now = Utc::now();
    let record = BackgroundAgent {
        schema_version: 1,
        agent_id: "agent".to_string(),
        parent_chat_id: "parent".to_string(),
        parent_root_chat_id: None,
        parent_tool_call_id: None,
        child_chat_id: Some("child".to_string()),
        kind: BgAgentKind::Subagent,
        config_name: "subagent".to_string(),
        title: "Fix frogs".to_string(),
        prompt: "Fix frogs".to_string(),
        target_files: vec!["src/frog.rs".to_string()],
        status: BgAgentStatus::Running,
        progress: Some("testing".to_string()),
        step_count: 3,
        last_activity: None,
        result_summary: None,
        result_payload_path: None,
        error: None,
        edited_files: Vec::new(),
        diff_summary: None,
        conflict_summary: None,
        completion_message_id: None,
        completion_pushed_at: None,
        completion_push: Default::default(),
        pending_deliveries: Vec::new(),
        delivery_ids: Vec::new(),
        deferred_at: None,
        model: "model".to_string(),
        model_type: Some("thinking".to_string()),
        current_tool: Some("cargo test".to_string()),
        goal_summary: Some("Fix all frogs".to_string()),
        plan_present: true,
        worktree_id: Some("worktree".to_string()),
        worktree_branch: Some("refact/subagent/frogs".to_string()),
        merge_status: Some("pending".to_string()),
        questions: vec![
            crate::agents::types::AgentQuestion {
                id: "11111111".to_string(),
                text: "Question one".to_string(),
                asked_at: now,
                answer: None,
                answered_at: None,
            },
            crate::agents::types::AgentQuestion {
                id: "22222222".to_string(),
                text: "Question two".to_string(),
                asked_at: now,
                answer: Some("Answer".to_string()),
                answered_at: Some(now),
            },
        ],
        tokens_used: 123,
        cost_usd: Some(0.42),
        created_at: now,
        started_at: Some(now),
        finished_at: None,
        last_update_at: now,
        change_seq: 4,
    };

    let summary = crate::agents::types::BackgroundAgentSummary::from(&record);

    assert_eq!(summary.model, "model");
    assert_eq!(summary.model_type.as_deref(), Some("thinking"));
    assert_eq!(summary.current_tool.as_deref(), Some("cargo test"));
    assert_eq!(summary.goal_summary.as_deref(), Some("Fix all frogs"));
    assert!(summary.plan_present);
    assert_eq!(
        summary.worktree_branch.as_deref(),
        Some("refact/subagent/frogs")
    );
    assert_eq!(summary.merge_status.as_deref(), Some("pending"));
    assert_eq!(summary.pending_questions, 1);
    assert_eq!(summary.tokens_used, 123);
    assert_eq!(summary.cost_usd, Some(0.42));
    assert_eq!(summary.questions.len(), 2);
    assert_eq!(summary.questions[1].answer.as_deref(), Some("Answer"));
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
async fn agent_result_falls_back_to_payload_when_summary_missing() {
    let (_gcx, app, _session_arc, _workspace) =
        app_with_parent_session("parent-result-fallback").await;
    let record = create_agent(&app.agents, "parent-result-fallback", BgAgentKind::Delegate).await;
    let completed = app
        .agents
        .mark_completed(&record.agent_id, completion("child-result-fallback"))
        .await
        .expect("completed");
    app.agents
        .clear_result_summary_for_test(&completed.agent_id)
        .await
        .expect("clear summary");
    let ccx = tool_context(app, "parent-result-fallback").await;
    let mut args = HashMap::new();
    args.insert("agent_id".to_string(), json!(completed.agent_id));

    let output = output_text(
        crate::tools::tool_background_agents::ToolAgentResult {
            config_path: String::new(),
        }
        .tool_execute(ccx, &"call".to_string(), &args)
        .await
        .expect("tool result"),
    );

    assert!(output.contains("fixed frog"));
    assert!(output.contains("- Edited files: src/frog.rs"));
    assert!(!output.contains("No result summary was recorded."));
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
async fn first_terminal_transition_bumps_change_seq_once() {
    let (_temp, registry) = registry().await;
    let record = create_agent(&registry, "parent", BgAgentKind::Delegate).await;

    let completed = registry
        .mark_completed(&record.agent_id, completion("child-first"))
        .await
        .expect("completed");
    let late_cancelled = registry
        .mark_cancelled(&record.agent_id, Some("too late".to_string()))
        .await
        .expect("late cancelled");

    assert_eq!(completed.change_seq, record.change_seq + 1);
    assert_eq!(late_cancelled.change_seq, completed.change_seq);
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
async fn overlap_warning_reports_running_agent_file_overlaps() {
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
    assert!(warning.contains(&subagent.agent_id));
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

#[test]
fn overlaps_normalize_equivalent_paths() {
    let requested: HashSet<String> = ["src/a.rs".to_string()].into_iter().collect();

    assert!(requested.contains(&normalize_path_for_overlap("./src/a.rs")));
    assert!(requested.contains(&normalize_path_for_overlap("src\\a.rs")));
    assert!(requested.contains(&normalize_path_for_overlap("src//a.rs")));
}

#[tokio::test]
async fn overlap_warning_normalizes_equivalent_paths() {
    let (_temp, registry) = registry().await;
    let delegate = create_agent(&registry, "parent", BgAgentKind::Delegate).await;
    registry
        .mark_running(&delegate.agent_id, "child-running".to_string())
        .await
        .expect("running");

    let dot_slash_warning = registry
        .overlap_warning("parent", &["./src/frog.rs".to_string()])
        .await
        .expect("dot slash warning");
    let backslash_warning = registry
        .overlap_warning("parent", &["src\\frog.rs".to_string()])
        .await
        .expect("backslash warning");
    let repeated_slash_warning = registry
        .overlap_warning("parent", &["src//frog.rs".to_string()])
        .await
        .expect("repeated slash warning");

    assert!(dot_slash_warning.contains(&delegate.agent_id));
    assert!(backslash_warning.contains(&delegate.agent_id));
    assert!(repeated_slash_warning.contains(&delegate.agent_id));
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
    assert!(first.deferred_at.is_none());
    assert!(second.deferred_at.is_none());
}

#[tokio::test]
async fn push_completion_to_parent_is_idempotent() {
    let (_gcx, app, session_arc, _workspace) = app_with_parent_session("parent-push").await;
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
    assert_eq!(agents_spawn_system_notice_count(&session), 1);
    let notice = agents_spawn_system_notices(&session).pop().unwrap();
    assert!(notice
        .content
        .content_text_only()
        .contains("[background subagent finished]"));
    let event = notice.extra.get("event").unwrap();
    assert_eq!(event["subkind"], json!("system_notice"));
    assert_eq!(event["source"], json!("agents.spawn"));
    assert_eq!(event["payload"]["agent_id"], json!(record.agent_id));
    assert_eq!(event["payload"]["status"], json!("completed"));
}

#[tokio::test]
async fn push_completion_to_parent_marks_pending_when_session_not_loaded_and_flush_retries() {
    let (_gcx, app, _session_arc, _workspace) = app_with_parent_session("parent-flush").await;
    _session_arc.lock().await.add_message(ChatMessage::new(
        "user".to_string(),
        "Persist this parent before unloading it".to_string(),
    ));
    crate::chat::trajectories::try_save_trajectory_with_intent(
        app.clone(),
        _session_arc.clone(),
        crate::chat::types::TrajectoryCommitIntent::Required,
    )
    .await
    .expect("persist parent before unloading");
    app.chat.sessions.write().await.remove("parent-flush");
    let record = create_agent(&app.agents, "parent-flush", BgAgentKind::Subagent).await;
    let completed = app
        .agents
        .mark_completed(&record.agent_id, completion("child-flush"))
        .await
        .unwrap();
    crate::agents::push::push_completion_to_parent(app.clone(), &completed)
        .await
        .unwrap();
    let updated = app
        .agents
        .get("parent-flush", &record.agent_id)
        .await
        .unwrap();
    assert!(updated.completion_pushed_at.is_some());
    assert_eq!(
        crate::agents::push::flush_pending_pushes_for_parent(app.clone(), "parent-flush")
            .await
            .unwrap(),
        0
    );
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get("parent-flush")
        .cloned()
        .unwrap();
    assert_eq!(agents_spawn_system_notice_count(&*session.lock().await), 1);
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
async fn spawn_background_agent_returns_immediately_with_child_chat_id_and_emits_transitions() {
    let runner_started = Arc::new(tokio::sync::Notify::new());
    let finish_runner = Arc::new(tokio::sync::Notify::new());
    let _runner = {
        let runner_started = runner_started.clone();
        let finish_runner = finish_runner.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, mut messages, config| {
            let runner_started = runner_started.clone();
            let finish_runner = finish_runner.clone();
            Box::pin(async move {
                runner_started.notify_one();
                finish_runner.notified().await;
                messages.push(ChatMessage::new(
                    "assistant".to_string(),
                    "Status: DONE\nCompleted spawn".to_string(),
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
    let (_gcx, app, session_arc, _workspace) =
        app_with_parent_session("parent-spawn-immediate").await;
    let mut rx = session_arc.lock().await.subscribe();
    let mut req = subagent_spawn_request("parent-spawn-immediate", "src/frog.rs");
    req.notify_parent = crate::agents::spawn::NotifyParent::Silent;

    let handle = tokio::time::timeout(
        Duration::from_secs(2),
        crate::agents::spawn::spawn_background_agent(app.clone(), req),
    )
    .await
    .expect("spawn returned before runner finished")
    .expect("spawn handle");

    assert!(handle.child_chat_id.starts_with("subchat-"));
    tokio::time::timeout(Duration::from_secs(1), runner_started.notified())
        .await
        .expect("runner started");

    let mut statuses = Vec::new();
    while !statuses.iter().any(|status| status == "running") {
        let json = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("background update before running")
            .expect("event");
        let value: serde_json::Value = serde_json::from_str(json.as_str()).expect("event json");
        let event: ChatEvent = serde_json::from_value(value).expect("event");
        if let ChatEvent::BackgroundAgentUpdated { agent, .. } = event {
            if agent.agent_id == handle.agent_id {
                statuses.push(agent.status);
            }
        }
    }

    let running = app
        .agents
        .get("parent-spawn-immediate", &handle.agent_id)
        .await
        .expect("running record");
    assert_eq!(running.status, BgAgentStatus::Running);
    assert_eq!(
        running.child_chat_id.as_deref(),
        Some(handle.child_chat_id.as_str())
    );

    finish_runner.notify_one();
    let completed = tokio::time::timeout(Duration::from_secs(1), handle.completion_rx)
        .await
        .expect("completion received")
        .expect("completion record");
    assert_eq!(completed.status, BgAgentStatus::Completed);

    while !statuses.iter().any(|status| status == "completed") {
        let json = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("background update before completed")
            .expect("event");
        let value: serde_json::Value = serde_json::from_str(json.as_str()).expect("event json");
        let event: ChatEvent = serde_json::from_value(value).expect("event");
        if let ChatEvent::BackgroundAgentUpdated { agent, .. } = event {
            if agent.agent_id == completed.agent_id {
                statuses.push(agent.status);
            }
        }
    }

    assert_eq!(statuses, vec!["queued", "running", "completed"]);
}

#[tokio::test]
async fn background_agent_updates_reach_parent_and_root_sessions() {
    let (_gcx, app, parent_session, _workspace) = app_with_parent_session("child-parent").await;
    let root_session = Arc::new(tokio::sync::Mutex::new(ChatSession::new(
        "root-parent".to_string(),
    )));
    app.chat
        .sessions
        .write()
        .await
        .insert("root-parent".to_string(), root_session.clone());
    let mut request = create_request("child-parent", BgAgentKind::Subagent);
    request.parent_root_chat_id = Some("root-parent".to_string());
    let (record, _, _) = app.agents.create(request).await.expect("create");

    crate::agents::spawn::emit_background_agent_update(app, &record).await;

    assert!(parent_session
        .lock()
        .await
        .background_agents
        .contains_key(&record.agent_id));
    assert!(root_session
        .lock()
        .await
        .background_agents
        .contains_key(&record.agent_id));
}

#[serial(test_runner)]
#[tokio::test]
async fn more_than_eight_background_agents_can_run_for_one_parent() {
    const AGENT_COUNT: usize = 12;
    let barrier = Arc::new(tokio::sync::Barrier::new(AGENT_COUNT + 1));
    let _runner = {
        let barrier = barrier.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, mut messages, config| {
            let barrier = barrier.clone();
            Box::pin(async move {
                barrier.wait().await;
                messages.push(ChatMessage::new(
                    "assistant".to_string(),
                    "Status: DONE\nCompleted concurrent spawn".to_string(),
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
    let (_gcx, app, _session_arc, _workspace) =
        app_with_parent_session("parent-unbounded-spawn").await;
    let mut handles = Vec::new();

    for index in 0..AGENT_COUNT {
        let mut request =
            subagent_spawn_request("parent-unbounded-spawn", &format!("src/frog-{index}.rs"));
        request.notify_parent = crate::agents::spawn::NotifyParent::Silent;
        handles.push(
            crate::agents::spawn::spawn_background_agent(app.clone(), request)
                .await
                .expect("spawn beyond former active-agent cap"),
        );
    }

    let active = app
        .agents
        .count_active_for_parent_root("parent-unbounded-spawn")
        .await;
    assert_eq!(active, AGENT_COUNT);

    barrier.wait().await;
    for handle in handles {
        let completed = handle.completion_rx.await.expect("completion");
        assert_eq!(completed.status, BgAgentStatus::Completed);
    }
}

#[serial(test_runner)]
#[tokio::test]
async fn spawn_and_wait_returns_terminal_record_within_timeout() {
    let _runner = install_spawn_runner(Arc::new(AtomicBool::new(false)));
    let (_gcx, app, _session_arc, _workspace) =
        app_with_parent_session("parent-wait-terminal").await;
    let mut req = subagent_spawn_request("parent-wait-terminal", "src/frog.rs");
    req.notify_parent = crate::agents::spawn::NotifyParent::Silent;

    let completed =
        crate::agents::spawn::spawn_and_wait(app.clone(), req, Some(Duration::from_secs(2)))
            .await
            .expect("spawn completed");

    assert_eq!(completed.status, BgAgentStatus::Completed);
    assert_eq!(completed.parent_chat_id, "parent-wait-terminal");
    assert!(completed
        .child_chat_id
        .as_deref()
        .is_some_and(|id| id.starts_with("subchat-")));
    assert!(completed
        .result_summary
        .as_deref()
        .is_some_and(|summary| summary.contains("Completed Edit src/frog.rs")));
    let persisted = app
        .agents
        .get("parent-wait-terminal", &completed.agent_id)
        .await
        .expect("persisted");
    assert_eq!(persisted, completed);
}

#[serial(test_runner)]
#[tokio::test]
async fn spawn_and_wait_times_out_when_runner_hangs() {
    tokio::time::pause();
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
    let wait_task = tokio::spawn(crate::agents::spawn::spawn_and_wait(
        app.clone(),
        req,
        Some(Duration::from_secs(5)),
    ));

    runner_started.notified().await;
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(5)).await;
    let err = wait_task
        .await
        .expect("spawn task joined")
        .expect_err("spawn should time out");

    assert_eq!(err, "background agent timed out");
    let records = app
        .agents
        .list_for_parent("parent-wait-timeout", AgentListFilter::default())
        .await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, BgAgentStatus::Running);
}

#[tokio::test]
async fn spawn_and_wait_timeout_returns_error() {
    let (_gcx, app, _session_arc, _workspace) = app_with_parent_session("parent-timeout").await;
    let req = crate::agents::spawn::SpawnRequest {
        kind: BgAgentKind::Subagent,
        parent_chat_id: "parent-timeout".to_string(),
        parent_root_chat_id: None,
        parent_tool_call_id: None,
        config_name: "missing-subagent-config".to_string(),
        title: "Missing".to_string(),
        prompt: "prompt".to_string(),
        tools: None,
        target_files: vec![],
        max_steps: 1,
        model: "model".to_string(),
        model_type: None,
        goal: None,
        plan: None,
        worktree_mode: crate::agents::spawn::SpawnWorktreeMode::Inherit,
        parent_subchat_tx: None,
        parent_worktree: None,
        parent_task_meta: None,
        subchat_depth: 0,
        notify_parent: crate::agents::spawn::NotifyParent::Silent,
        completion_push: Default::default(),
    };

    let err = crate::agents::spawn::spawn_and_wait(app, req, Some(Duration::from_millis(1)))
        .await
        .expect_err("missing config should error before waiting");
    assert!(err.contains("not found") || err.contains("missing"));
}

#[serial(test_runner)]
#[tokio::test]
async fn spawn_with_empty_assistant_response_uses_no_text_summary() {
    let _runner =
        crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, mut messages, config| {
            Box::pin(async move {
                messages.push(ChatMessage::new("assistant".to_string(), "   ".to_string()));
                Ok(SubchatResult {
                    messages,
                    metering: serde_json::Map::new(),
                    chat_id: config.chat_id,
                    aborted: false,
                })
            })
        }));
    let (_gcx, app, _session_arc, _workspace) =
        app_with_parent_session("parent-empty-summary").await;
    let mut req = subagent_spawn_request("parent-empty-summary", "src/frog.rs");
    req.notify_parent = crate::agents::spawn::NotifyParent::Silent;

    let completed = crate::agents::spawn::spawn_and_wait(app, req, Some(Duration::from_secs(2)))
        .await
        .expect("spawn completed");

    assert_eq!(completed.status, BgAgentStatus::Completed);
    assert_eq!(
        completed.result_summary.as_deref(),
        Some(NO_TEXT_RESULT_SUMMARY)
    );
}

fn subagent_spawn_request(
    parent_chat_id: &str,
    target_file: &str,
) -> crate::agents::spawn::SpawnRequest {
    crate::agents::spawn::SpawnRequest {
        kind: BgAgentKind::Subagent,
        parent_chat_id: parent_chat_id.to_string(),
        parent_root_chat_id: Some(parent_chat_id.to_string()),
        parent_tool_call_id: None,
        config_name: "test_spawn".to_string(),
        title: format!("Edit {target_file}"),
        prompt: format!("Edit {target_file}"),
        tools: None,
        target_files: vec![target_file.to_string()],
        max_steps: 1,
        model: "model".to_string(),
        model_type: None,
        goal: None,
        plan: None,
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
    let plan = messages
        .iter()
        .find(|message| message.role == "plan")
        .unwrap();
    assert_eq!(plan.extra["plan"]["mode"], "agent");
    assert_eq!(plan.extra["plan"]["version"], 1);
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
    crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, messages, config| {
        let abort_seen = abort_seen.clone();
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
    let _runner = install_spawn_runner(abort_seen.clone());
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
    let cancel_handle = crate::agents::spawn::spawn_background_agent(app.clone(), cancel_req)
        .await
        .expect("cancel spawn");
    tokio::time::sleep(Duration::from_millis(20)).await;
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

#[tokio::test]
async fn runner_pending_recovery_preserves_id_and_never_wakes() {
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
        1
    );
    assert!(app
        .agents
        .pending_deliveries(&record.agent_id)
        .await
        .is_empty());
    let session = session.lock().await;
    assert!(session.messages.iter().any(
        |message| refact_core::chat_types::delivery_id_of_message(message) == Some("recover-id")
    ));
    assert!(session
        .pending_deliveries
        .iter()
        .all(|delivery| !delivery.wake));
}
