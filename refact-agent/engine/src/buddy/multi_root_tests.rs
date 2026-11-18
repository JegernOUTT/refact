use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Query, State};
use hyper::StatusCode;
use tokio::sync::Mutex;

use crate::app_state::AppState;
use crate::chat::types::ChatSession;
use crate::http::routers::v1::{
    buddy_attention, buddy_decisions, buddy_native_approvals, buddy_native_work,
};

struct MultiRoot {
    _temp: tempfile::TempDir,
    app: AppState,
    own: PathBuf,
    foreign: PathBuf,
}

async fn multi_root(with_service: bool) -> MultiRoot {
    workspace(with_service, true).await
}

async fn single_root() -> MultiRoot {
    workspace(true, false).await
}

async fn workspace(with_service: bool, include_foreign: bool) -> MultiRoot {
    let temp = tempfile::tempdir().unwrap();
    let base = dunce::canonicalize(temp.path()).unwrap();
    let own = base.join("own");
    let foreign = base.join("foreign");
    std::fs::create_dir_all(&own).unwrap();
    std::fs::create_dir_all(&foreign).unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        base.join("cache"),
        base.join("config"),
    )
    .await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = if include_foreign {
        vec![foreign.clone(), own.clone()]
    } else {
        vec![own.clone()]
    };
    let app = AppState::from_gcx(gcx).await;
    if with_service {
        let (tx, _) = tokio::sync::broadcast::channel(16);
        *app.buddy.buddy.lock().await = Some(super::actor::BuddyService::new(
            own.clone(),
            super::state::default_buddy_state(),
            super::settings::BuddySettings::default(),
            vec![],
            super::runtime_queue::RuntimeQueue::new(),
            tx,
            None,
        ));
    }
    MultiRoot {
        _temp: temp,
        app,
        own,
        foreign,
    }
}

fn question(id: &str) -> refact_chat_api::PendingQuestionSet {
    refact_chat_api::PendingQuestionSet {
        tool_call_id: id.into(),
        asked_at_ms: 1,
        questions: vec![refact_chat_api::PendingQuestion {
            id: "ok".into(),
            question_type: "yes_no".into(),
            text: "Continue?".into(),
            options: None,
        }],
        status: refact_chat_api::PendingQuestionStatus::Pending,
        answers: vec![],
        resolution_id: None,
        response_protocol_version: refact_chat_api::QUESTION_RESPONSE_PROTOCOL_VERSION,
        response: None,
    }
}

async fn insert(app: &AppState, id: &str, root: &Path) -> Arc<Mutex<ChatSession>> {
    insert_with_root(app, id, Some(root)).await
}

async fn insert_with_root(
    app: &AppState,
    id: &str,
    root: Option<&Path>,
) -> Arc<Mutex<ChatSession>> {
    let mut session = ChatSession::new(id.into());
    session.work_project_root = root.map(Path::to_path_buf);
    session
        .register_pending_questions(question(&format!("{id}-q")))
        .unwrap();
    session.runtime.state = refact_chat_api::SessionState::Error;
    session.runtime.error = Some("needs attention".into());
    let session = Arc::new(Mutex::new(session));
    app.chat
        .sessions
        .write()
        .await
        .insert(id.into(), session.clone());
    session
}

fn native_work_query(after: Option<&str>) -> Query<buddy_native_work::NativeWorkQuery> {
    Query(serde_json::from_value(serde_json::json!({ "after": after })).unwrap())
}

#[tokio::test]
async fn busy_foreign_session_does_not_make_buddy_snapshots_unavailable() {
    let fixture = multi_root(true).await;
    let app = &fixture.app;
    insert(app, "a-own", &fixture.own).await;
    let foreign = insert(app, "b-foreign", &fixture.foreign).await;
    let _busy = foreign.lock().await;

    let approvals = buddy_native_approvals::handle_current_approvals(State(app.clone()))
        .await
        .map_err(|e| e.status_code)
        .expect("approvals snapshot");
    let approvals = serde_json::to_value(&approvals.0).unwrap();
    assert_eq!(approvals["sessions"][0]["chat_id"], "a-own");
    assert_eq!(approvals["sessions"].as_array().unwrap().len(), 1);

    let decisions = buddy_decisions::handle_current_decisions(State(app.clone()))
        .await
        .map_err(|e| e.status_code)
        .expect("decisions snapshot");
    let decisions = serde_json::to_value(&decisions.0).unwrap();
    assert_eq!(decisions["questions"].as_array().unwrap().len(), 1);
    assert_eq!(decisions["questions"][0]["chat_id"], "a-own");

    let work = buddy_native_work::handle_native_work(State(app.clone()), native_work_query(None))
        .await
        .map_err(|e| e.status_code)
        .expect("native work snapshot");
    let work = serde_json::to_value(&work.0).unwrap();
    assert_eq!(work["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(work["sessions"][0]["chat_id"], "a-own");

    let attention = buddy_attention::handle_current_attention(State(app.clone()))
        .await
        .map_err(|e| e.status_code)
        .expect("attention snapshot");
    let attention = serde_json::to_value(&attention.0).unwrap();
    assert_eq!(attention["busy_chat_ids"], serde_json::json!([]));
}

#[tokio::test]
async fn busy_own_root_session_still_reports_busy() {
    let fixture = multi_root(true).await;
    let own = insert(&fixture.app, "a-own", &fixture.own).await;
    let _busy = own.lock().await;
    let error = buddy_native_approvals::handle_current_approvals(State(fixture.app.clone()))
        .await
        .err()
        .expect("a busy own-root session must not be silently skipped");
    assert_eq!(error.status_code, StatusCode::SERVICE_UNAVAILABLE);
    let attention = buddy_attention::handle_current_attention(State(fixture.app.clone()))
        .await
        .unwrap();
    let attention = serde_json::to_value(&attention.0).unwrap();
    assert_eq!(attention["busy_chat_ids"], serde_json::json!(["a-own"]));
}

#[tokio::test]
async fn foreign_sessions_never_truncate_own_rows_or_become_the_cursor() {
    let fixture = multi_root(true).await;
    let app = &fixture.app;
    for i in 0..250 {
        insert(app, &format!("a-foreign-{i:03}"), &fixture.foreign).await;
    }
    for i in 0..25 {
        insert(app, &format!("m-own-{i:02}"), &fixture.own).await;
    }
    insert(app, "z-foreign", &fixture.foreign).await;

    let approvals = buddy_native_approvals::handle_current_approvals(State(app.clone()))
        .await
        .unwrap();
    let approvals = serde_json::to_value(&approvals.0).unwrap();
    assert_eq!(approvals["sessions"].as_array().unwrap().len(), 25);
    assert_eq!(approvals["truncated"], false);

    let decisions = buddy_decisions::handle_current_decisions(State(app.clone()))
        .await
        .unwrap();
    let decisions = serde_json::to_value(&decisions.0).unwrap();
    assert_eq!(decisions["questions"].as_array().unwrap().len(), 25);
    assert_eq!(decisions["truncated"], false);

    let page = buddy_native_work::handle_native_work(State(app.clone()), native_work_query(None))
        .await
        .unwrap();
    let page = serde_json::to_value(&page.0).unwrap();
    let ids: Vec<_> = page["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["chat_id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids.len(), 20);
    assert!(ids.iter().all(|id| id.starts_with("m-own-")));
    assert_eq!(page["next_cursor"], "m-own-19");

    let page = buddy_native_work::handle_native_work(
        State(app.clone()),
        native_work_query(Some("m-own-19")),
    )
    .await
    .unwrap();
    let page = serde_json::to_value(&page.0).unwrap();
    let ids: Vec<_> = page["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["chat_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec!["m-own-20", "m-own-21", "m-own-22", "m-own-23", "m-own-24"]
    );
    assert_eq!(page["next_cursor"], serde_json::Value::Null);
}

#[tokio::test]
async fn attention_resolves_the_bound_root_in_multi_root_workspaces() {
    let fixture = multi_root(true).await;
    insert(&fixture.app, "own-chat", &fixture.own).await;
    insert(&fixture.app, "foreign-chat", &fixture.foreign).await;
    let snapshot = buddy_attention::handle_current_attention(State(fixture.app.clone()))
        .await
        .map_err(|e| (e.status_code, e.message))
        .expect("multi-root attention must resolve Buddy's bound root");
    let snapshot = serde_json::to_value(&snapshot.0).unwrap();
    assert_eq!(
        snapshot["project_root"],
        fixture.own.to_string_lossy().as_ref()
    );
    let chats: std::collections::BTreeSet<_> = snapshot["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["chat_id"].as_str().unwrap())
        .collect();
    assert_eq!(chats, std::collections::BTreeSet::from(["own-chat"]));
}

#[tokio::test]
async fn attention_keeps_rejecting_unscoped_workspaces() {
    let fixture = multi_root(false).await;
    let error = buddy_attention::handle_current_attention(State(fixture.app.clone()))
        .await
        .err()
        .expect("multiple roots without a bound Buddy are ambiguous");
    assert_eq!(error.status_code, StatusCode::SERVICE_UNAVAILABLE);

    let fixture = multi_root(true).await;
    *fixture
        .app
        .gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap() = vec![fixture.foreign.clone()];
    let detached = buddy_attention::handle_current_attention(State(fixture.app.clone()))
        .await
        .err()
        .expect("a detached Buddy root must not fall back to another root");
    assert_eq!(detached.status_code, StatusCode::SERVICE_UNAVAILABLE);

    *fixture
        .app
        .gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap() = vec![];
    let empty = buddy_attention::handle_current_attention(State(fixture.app.clone()))
        .await
        .err()
        .expect("zero roots must be rejected");
    assert_eq!(empty.status_code, StatusCode::SERVICE_UNAVAILABLE);
}

fn diagnostic(message: &str) -> super::diagnostics::DiagnosticContext {
    super::diagnostics::DiagnosticContext {
        error_type: "test".into(),
        error_message: message.into(),
        source_file: None,
        tool_name: None,
        chat_id: None,
        model_id: None,
        collected_at: chrono::Utc::now().to_rfc3339(),
        severity: super::diagnostics::DiagnosticSeverity::High,
        occurrences: None,
    }
}

#[tokio::test]
async fn diagnostics_list_and_resolve_use_buddys_bound_root() {
    let fixture = multi_root(true).await;
    super::storage::append_diagnostic(&fixture.own, &diagnostic("own failure"))
        .await
        .unwrap();
    super::storage::append_diagnostic(&fixture.foreign, &diagnostic("foreign failure"))
        .await
        .unwrap();
    assert_eq!(
        super::actor::buddy_project_root(fixture.app.clone())
            .await
            .unwrap(),
        fixture.own
    );
    let listed = crate::http::routers::v1::buddy::handle_v1_buddy_diagnostics_list(State(
        fixture.app.clone(),
    ))
    .await
    .map_err(|e| e.message)
    .unwrap()
    .0;
    let messages: Vec<_> = listed.iter().map(|d| d.error_message.as_str()).collect();
    assert_eq!(messages, vec!["own failure"]);
    let resolved = super::actor::resolve_diagnostic(fixture.app.clone(), Some(0), None, None, None)
        .await
        .unwrap();
    assert_eq!(resolved.error_message, "own failure");

    *fixture
        .app
        .gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap() = vec![fixture.foreign.clone()];
    assert!(super::actor::buddy_project_root(fixture.app.clone())
        .await
        .is_err());
}

fn retry_messages() -> Vec<crate::call_validation::ChatMessage> {
    ["actually no", "wait try again", "sorry undo"]
        .into_iter()
        .map(|text| crate::call_validation::ChatMessage {
            role: "user".into(),
            content: crate::call_validation::ChatContent::SimpleText(text.into()),
            ..Default::default()
        })
        .collect()
}

#[tokio::test]
async fn chat_pattern_observer_ignores_foreign_root_sessions() {
    use super::observers::BuddyObserver;
    let fixture = multi_root(true).await;
    for (id, root) in [
        ("own-chat", &fixture.own),
        ("foreign-chat", &fixture.foreign),
    ] {
        let mut session = ChatSession::new(id.into());
        session.work_project_root = Some(root.clone());
        session.messages = retry_messages();
        fixture
            .app
            .chat
            .sessions
            .write()
            .await
            .insert(id.into(), Arc::new(Mutex::new(session)));
    }
    let facts = super::observers::chat_pattern::ChatPatternObserver
        .observe(
            fixture.app.clone(),
            &super::observers::ObserverContext {
                project_root: fixture.own.clone(),
                now: chrono::Utc::now(),
            },
        )
        .await;
    let chats: Vec<_> = facts
        .iter()
        .map(|fact| fact.payload["chat_id"].as_str().unwrap())
        .collect();
    assert_eq!(chats, vec!["own-chat"]);
}

async fn snapshot_chat_ids(app: &AppState) -> Vec<Vec<String>> {
    fn ids(value: &serde_json::Value, list: &str) -> Vec<String> {
        let mut ids: Vec<String> = value[list]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["chat_id"].as_str().unwrap().to_string())
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }
    let approvals = buddy_native_approvals::handle_current_approvals(State(app.clone()))
        .await
        .map_err(|e| e.message)
        .unwrap();
    let decisions = buddy_decisions::handle_current_decisions(State(app.clone()))
        .await
        .map_err(|e| e.message)
        .unwrap();
    let work = buddy_native_work::handle_native_work(State(app.clone()), native_work_query(None))
        .await
        .map_err(|e| e.message)
        .unwrap();
    let attention = buddy_attention::handle_current_attention(State(app.clone()))
        .await
        .map_err(|e| e.message)
        .unwrap();
    vec![
        ids(&serde_json::to_value(&approvals.0).unwrap(), "sessions"),
        ids(&serde_json::to_value(&decisions.0).unwrap(), "questions"),
        ids(&serde_json::to_value(&work.0).unwrap(), "sessions"),
        ids(&serde_json::to_value(&attention.0).unwrap(), "rows"),
    ]
}

#[tokio::test]
async fn rootless_sessions_belong_to_buddy_only_in_a_single_root_workspace() {
    let single = single_root().await;
    insert(&single.app, "own", &single.own).await;
    insert_with_root(&single.app, "rootless", None).await;
    for ids in snapshot_chat_ids(&single.app).await {
        assert_eq!(ids, vec!["own", "rootless"]);
    }
    let attention = buddy_attention::handle_current_attention(State(single.app.clone()))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&attention.0).unwrap()["busy_chat_ids"],
        serde_json::json!([])
    );

    let multi = multi_root(true).await;
    insert(&multi.app, "own", &multi.own).await;
    insert_with_root(&multi.app, "rootless", None).await;
    for ids in snapshot_chat_ids(&multi.app).await {
        assert_eq!(ids, vec!["own"]);
    }
    let attention = buddy_attention::handle_current_attention(State(multi.app.clone()))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&attention.0).unwrap()["busy_chat_ids"],
        serde_json::json!([])
    );
}

#[tokio::test]
async fn chat_pattern_observer_applies_the_rootless_rule() {
    use super::observers::BuddyObserver;
    for (fixture, expected) in [
        (single_root().await, vec!["own-chat", "rootless-chat"]),
        (multi_root(true).await, vec!["own-chat"]),
    ] {
        for (id, root) in [("own-chat", Some(&fixture.own)), ("rootless-chat", None)] {
            let mut session = ChatSession::new(id.into());
            session.work_project_root = root.cloned();
            session.messages = retry_messages();
            fixture
                .app
                .chat
                .sessions
                .write()
                .await
                .insert(id.into(), Arc::new(Mutex::new(session)));
        }
        let facts = super::observers::chat_pattern::ChatPatternObserver
            .observe(
                fixture.app.clone(),
                &super::observers::ObserverContext {
                    project_root: fixture.own.clone(),
                    now: chrono::Utc::now(),
                },
            )
            .await;
        let mut chats: Vec<_> = facts
            .iter()
            .map(|fact| fact.payload["chat_id"].as_str().unwrap())
            .collect();
        chats.sort();
        assert_eq!(chats, expected);
    }
}

#[tokio::test]
async fn session_inserted_while_locked_is_busy_not_foreign() {
    let fixture = multi_root(true).await;
    let session = Arc::new(Mutex::new({
        let mut session = ChatSession::new("a-unknown".into());
        session.work_project_root = Some(fixture.foreign.clone());
        session
    }));
    let _busy = session.lock().await;
    fixture
        .app
        .chat
        .sessions
        .write()
        .await
        .insert("a-unknown".into(), session.clone());
    assert_eq!(
        fixture.app.chat.sessions.1.source_root_hint("a-unknown"),
        None
    );
    let error = buddy_native_approvals::handle_current_approvals(State(fixture.app.clone()))
        .await
        .err()
        .expect("an unpublished busy session must not be presumed foreign");
    assert_eq!(error.status_code, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn root_change_under_lock_flips_the_hint_through_the_publish_hook() {
    let fixture = multi_root(true).await;
    let session = insert(&fixture.app, "a-moving", &fixture.own).await;
    let sessions = &fixture.app.chat.sessions;
    assert_eq!(
        sessions.1.source_root_hint("a-moving"),
        Some(Some(fixture.own.clone()))
    );
    let mut guard = session.lock().await;
    guard.work_project_root = Some(fixture.foreign.clone());
    guard.increment_version();
    assert_eq!(
        sessions.1.source_root_hint("a-moving"),
        Some(Some(fixture.foreign.clone()))
    );
    let snapshot = buddy_native_approvals::handle_current_approvals(State(fixture.app.clone()))
        .await
        .map_err(|e| e.status_code)
        .expect("a session published as foreign is skipped while busy");
    assert!(serde_json::to_value(&snapshot.0).unwrap()["sessions"]
        .as_array()
        .unwrap()
        .is_empty());
    drop(guard);
}

#[tokio::test]
async fn removed_sessions_are_pruned_from_the_hint_map() {
    let fixture = multi_root(true).await;
    insert(&fixture.app, "gone", &fixture.foreign).await;
    insert(&fixture.app, "kept", &fixture.own).await;
    fixture.app.chat.sessions.write().await.remove("gone");
    assert_eq!(fixture.app.chat.sessions.1.source_root_hint("gone"), None);
    assert_eq!(
        fixture.app.chat.sessions.1.source_root_hint("kept"),
        Some(Some(fixture.own.clone()))
    );
}

#[tokio::test]
async fn own_root_caps_report_truncation_and_more_pages() {
    let fixture = multi_root(true).await;
    for i in 0..201 {
        insert(&fixture.app, &format!("own-{i:03}"), &fixture.own).await;
    }
    let approvals = buddy_native_approvals::handle_current_approvals(State(fixture.app.clone()))
        .await
        .unwrap();
    let approvals = serde_json::to_value(&approvals.0).unwrap();
    assert_eq!(approvals["sessions"].as_array().unwrap().len(), 200);
    assert_eq!(approvals["truncated"], true);
    let decisions = buddy_decisions::handle_current_decisions(State(fixture.app.clone()))
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&decisions.0).unwrap()["truncated"],
        true
    );

    let fixture = multi_root(true).await;
    for i in 0..21 {
        insert(&fixture.app, &format!("own-{i:02}"), &fixture.own).await;
    }
    let work =
        buddy_native_work::handle_native_work(State(fixture.app.clone()), native_work_query(None))
            .await
            .unwrap();
    let work = serde_json::to_value(&work.0).unwrap();
    assert_eq!(work["sessions"].as_array().unwrap().len(), 20);
    assert_eq!(work["next_cursor"], "own-19");
}

#[tokio::test]
async fn busy_own_session_past_the_cap_only_marks_more_rows() {
    let fixture = multi_root(true).await;
    for i in 0..20 {
        insert(&fixture.app, &format!("own-{i:02}"), &fixture.own).await;
    }
    let busy = insert(&fixture.app, "own-20", &fixture.own).await;
    let _busy = busy.lock().await;
    let work =
        buddy_native_work::handle_native_work(State(fixture.app.clone()), native_work_query(None))
            .await
            .map_err(|e| e.status_code)
            .expect("a busy row beyond the page must not 503 the page");
    let work = serde_json::to_value(&work.0).unwrap();
    assert_eq!(work["sessions"].as_array().unwrap().len(), 20);
    assert_eq!(work["next_cursor"], "own-19");

    let fixture = multi_root(true).await;
    for i in 0..200 {
        insert(&fixture.app, &format!("own-{i:03}"), &fixture.own).await;
    }
    let busy = insert(&fixture.app, "own-200", &fixture.own).await;
    let _busy = busy.lock().await;
    let approvals = buddy_native_approvals::handle_current_approvals(State(fixture.app.clone()))
        .await
        .map_err(|e| e.status_code)
        .expect("a busy row beyond the cap must not 503 the snapshot");
    assert_eq!(
        serde_json::to_value(&approvals.0).unwrap()["truncated"],
        true
    );
}

#[tokio::test]
async fn attention_finds_unopened_parents_through_equivalent_root_spellings() {
    let fixture = multi_root(true).await;
    let (record, _, _) = fixture
        .app
        .agents
        .create(crate::agents::types::CreateAgentRequest {
            parent_chat_id: "unopened-parent".into(),
            parent_root_chat_id: None,
            parent_tool_call_id: None,
            kind: crate::agents::types::BgAgentKind::Subagent,
            config_name: "subagent".into(),
            title: "child".into(),
            prompt: "work".into(),
            target_files: vec![],
            model: "test/model".into(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
        })
        .await
        .unwrap();
    let mut alias = fixture.own.as_os_str().to_os_string();
    alias.push(format!("{}.", std::path::MAIN_SEPARATOR));
    fixture
        .app
        .agent_executions
        .remember_scope(&record.agent_id, PathBuf::from(alias));
    let now = crate::agents::approval::now_ms();
    fixture
        .app
        .agent_approvals
        .register(
            refact_work_core::PendingApproval {
                id: "approval-1".into(),
                child_work_id: record.agent_id.clone(),
                parent_chat_id: "unopened-parent".into(),
                run_generation: 1,
                tool_call_id: "call-1".into(),
                tool_name: "shell".into(),
                args: serde_json::json!({}),
                requires_human: false,
                digest: None,
                created_at: now,
                expires_at: now + 600_000,
                execution: None,
            },
            now,
        )
        .unwrap();
    let snapshot = buddy_attention::handle_current_attention(State(fixture.app.clone()))
        .await
        .unwrap();
    let snapshot = serde_json::to_value(&snapshot.0).unwrap();
    assert!(
        snapshot["rows"].as_array().unwrap().iter().any(|row| {
            row["kind"] == "child_permission" && row["parent_chat_id"] == "unopened-parent"
        }),
        "{snapshot}"
    );
}

async fn sink_effects(app: &AppState) -> (usize, usize, String) {
    let lock = app.buddy.buddy.lock().await;
    let svc = lock.as_ref().unwrap();
    (
        svc.recent_diagnostics.len(),
        svc.runtime_queue.items.len(),
        svc.state.semantic.mood.clone(),
    )
}

#[tokio::test]
async fn foreign_chat_activity_never_reaches_buddy_through_the_event_sink() {
    let fixture = multi_root(true).await;
    insert(&fixture.app, "foreign-chat", &fixture.foreign).await;
    insert(&fixture.app, "own-chat", &fixture.own).await;
    let sink = fixture.app.buddy_event_sink.clone();
    let event = |chat: &str| {
        let mut event = super::actor::make_runtime_event(
            "chat_completed",
            &format!("Completed: {chat}"),
            "chat",
            &format!("chat_{chat}"),
            "completed",
            None,
        );
        event.chat_id = Some(chat.into());
        event
    };
    fixture
        .app
        .buddy
        .buddy
        .lock()
        .await
        .as_mut()
        .unwrap()
        .state
        .semantic
        .mood = "sentinel".into();
    let before = sink_effects(&fixture.app).await;
    sink.report_error_with_model(
        "llm_error",
        "foreign provider failure",
        Some("chat/generation.rs"),
        Some("foreign-chat"),
        Some("foreign/model"),
    )
    .await;
    super::actor::report_error_persisted(
        fixture.app.clone(),
        "browser_error",
        "foreign browser failure",
        None,
        Some("foreign-chat"),
    )
    .await;
    sink.apply_chat_completion(event("foreign-chat"), 4, "happy".into())
        .await;
    sink.enqueue_event(event("foreign-chat")).await;
    sink.mark_chat_error(event("foreign-chat")).await;
    assert_eq!(sink_effects(&fixture.app).await, before);

    sink.report_error_with_model(
        "llm_error",
        "own provider failure",
        Some("chat/generation.rs"),
        Some("own-chat"),
        Some("own/model"),
    )
    .await;
    sink.apply_chat_completion(event("own-chat"), 4, "happy".into())
        .await;
    let after = sink_effects(&fixture.app).await;
    assert_eq!(after.0, before.0 + 1);
    assert!(after.1 > before.1);
    assert_eq!(after.2, "happy");
    let lock = fixture.app.buddy.buddy.lock().await;
    let svc = lock.as_ref().unwrap();
    assert!(svc
        .recent_diagnostics
        .iter()
        .all(|diag| diag.chat_id.as_deref() != Some("foreign-chat")));
}

fn completed(chat: &str) -> refact_buddy_core::types::BuddyRuntimeEvent {
    let mut event = super::actor::make_runtime_event(
        "chat_completed",
        &format!("Completed: {chat}"),
        "chat",
        &format!("chat_{chat}"),
        "completed",
        None,
    );
    event.chat_id = Some(chat.into());
    event
}

async fn send_chat_activity(app: &AppState, chat: &str) {
    let sink = app.buddy_event_sink.clone();
    sink.report_error_with_model(
        "llm_error",
        &format!("{chat} provider failure"),
        Some("chat/generation.rs"),
        Some(chat),
        Some("model"),
    )
    .await;
    sink.apply_chat_completion(completed(chat), 4, "happy".into())
        .await;
    sink.enqueue_event(completed(chat)).await;
}

async fn reset_mood(app: &AppState) {
    app.buddy
        .buddy
        .lock()
        .await
        .as_mut()
        .unwrap()
        .state
        .semantic
        .mood = "sentinel".into();
}

#[tokio::test]
async fn sink_fails_closed_for_unknown_sources_even_in_a_single_root_workspace() {
    let fixture = single_root().await;
    let outside = tempfile::tempdir().unwrap();
    let unknown = Arc::new(Mutex::new({
        let mut session = ChatSession::new("unknown-chat".into());
        session.work_project_root = Some(outside.path().to_path_buf());
        session
    }));
    let _busy = unknown.lock().await;
    fixture
        .app
        .chat
        .sessions
        .write()
        .await
        .insert("unknown-chat".into(), unknown.clone());
    assert_eq!(
        fixture.app.chat.sessions.1.source_root_hint("unknown-chat"),
        None
    );
    reset_mood(&fixture.app).await;
    let before = sink_effects(&fixture.app).await;
    send_chat_activity(&fixture.app, "unknown-chat").await;
    send_chat_activity(&fixture.app, "never-loaded-chat").await;
    assert_eq!(sink_effects(&fixture.app).await, before);

    let rootless = insert_with_root(&fixture.app, "rootless-chat", None).await;
    assert_eq!(
        fixture
            .app
            .chat
            .sessions
            .1
            .source_root_hint("rootless-chat"),
        Some(None)
    );
    let _busy_rootless = rootless.lock().await;
    send_chat_activity(&fixture.app, "rootless-chat").await;
    let after = sink_effects(&fixture.app).await;
    assert_eq!(after.0, before.0 + 1);
    assert!(after.1 > before.1);
    assert_eq!(after.2, "happy");
}

#[cfg(unix)]
#[tokio::test]
async fn sink_attribution_is_cached_until_workspace_folders_change() {
    let fixture = single_root().await;
    let links = tempfile::tempdir().unwrap();
    let alias = links.path().join("own-alias");
    std::os::unix::fs::symlink(&fixture.own, &alias).unwrap();
    insert(&fixture.app, "own-chat", &alias).await;
    reset_mood(&fixture.app).await;
    send_chat_activity(&fixture.app, "own-chat").await;
    assert_eq!(sink_effects(&fixture.app).await.2, "happy");

    std::fs::remove_file(&alias).unwrap();
    reset_mood(&fixture.app).await;
    send_chat_activity(&fixture.app, "own-chat").await;
    assert_eq!(sink_effects(&fixture.app).await.2, "happy");

    *fixture
        .app
        .gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap() = vec![fixture.foreign.clone(), fixture.own.clone()];
    reset_mood(&fixture.app).await;
    send_chat_activity(&fixture.app, "own-chat").await;
    assert_eq!(sink_effects(&fixture.app).await.2, "sentinel");
}
