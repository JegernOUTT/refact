use super::*;
use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use crate::scheduler::CronStore;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tower::ServiceExt;

async fn fixture() -> (tempfile::TempDir, AppState) {
    let root = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.path().to_path_buf()];
    let app = AppState::from_gcx(gcx).await;
    (root, app)
}

async fn persist(app: &AppState, session: &ChatSession) {
    crate::chat::trajectories::save_trajectory_snapshot(
        app.gcx.clone(),
        crate::chat::trajectories::trajectory_snapshot_from_session(session),
    )
    .await
    .unwrap();
}

fn cold_file(directory: &Path, id: &str, mode: &str, link_type: Option<&str>, recovery: Value) {
    std::fs::create_dir_all(directory).unwrap();
    let value = json!({
        "id": id, "title": id, "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:01Z", "model": "test-model",
        "mode": mode, "tool_use": "agent", "messages": [],
        "root_chat_id": id, "link_type": link_type, "recovery": recovery
    });
    std::fs::write(
        directory.join(format!("{id}.json")),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
}

async fn request(app: AppState) -> (StatusCode, Value) {
    let response = super::super::chats_routes()
        .with_state(app)
        .oneshot(
            Request::builder()
                .uri("/chats/activity")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

fn job(id: &str, chat_id: &str, trigger: Trigger) -> Job {
    let mut job = Job::new_cron_agent_chat(
        "0 * * * *".into(),
        "secret prompt".into(),
        "secret description".into(),
        true,
        true,
        1,
    );
    job.id = id.into();
    job.trigger = trigger;
    job.set_existing_chat(Some(chat_id.into()));
    job
}

fn recovery(state: RecoveryState) -> refact_core::recovery::RecoveryRecord {
    serde_json::from_value(json!({
        "operation_id":"op", "run_id":"run", "revision":1,
        "state":state, "policy":refact_core::recovery::RecoveryPolicy::default(),
        "policy_source":"global", "manual":false, "retries_used":0,
        "latest_error":{"category":"Network", "title":"Unavailable", "explanation":"Unavailable", "suggested_action":"Retry", "is_retryable":true, "raw_error":"secret diagnostic"},
        "attempts":[]
    })).unwrap()
}

#[cfg(unix)]
#[tokio::test]
async fn activity_unopened_work_and_cold_schedules_are_deduplicated() {
    let (root, app) = fixture().await;
    let mut live = ChatSession::new("unopened".into());
    live.work_project_root = Some(root.path().to_path_buf());
    live.thread.title = "Live title".into();
    live.runtime.state = SessionState::Generating;
    persist(&app, &live).await;
    live.thread.title = "Authoritative live title".into();
    app.chat
        .sessions
        .write()
        .await
        .insert(live.chat_id.clone(), Arc::new(Mutex::new(live)));
    let cold = ChatSession::new("cold".into());
    persist(&app, &cold).await;
    let store = crate::scheduler::JsonFileCronStore::new(root.path()).unwrap();
    store
        .add(job(
            "cron",
            "unopened",
            Trigger::Cron {
                expr: "0 * * * *".into(),
                tz: None,
            },
        ))
        .await
        .unwrap();
    let mut interval = job(
        "interval",
        "cold",
        Trigger::Interval {
            every_ms: 60_000,
        },
    );
    interval.enabled = false;
    store.add(interval).await.unwrap();
    let mut once = job(
        "once",
        "cold",
        Trigger::Once {
            at_ms: 100,
        },
    );
    once.paused_at_ms = Some(1);
    store.add(once).await.unwrap();
    let (status, value) = request(app).await;
    assert_eq!(status, StatusCode::OK);
    let chats = value["chats"].as_array().unwrap();
    assert_eq!(chats.len(), 2);
    let live = chats.iter().find(|c| c["id"] == "unopened").unwrap();
    assert_eq!(live["title"], "Authoritative live title");
    assert_eq!(live["session_state"], "generating");
    assert_eq!(live["schedule_count"], 1);
    let cold = chats.iter().find(|c| c["id"] == "cold").unwrap();
    assert_eq!(cold["title"], "New Chat");
    assert_eq!(cold["session_state"], Value::Null);
    assert_eq!(cold["schedule_count"], 2);
    assert_eq!(cold["paused_schedule_count"], 2);
    assert_eq!(cold.as_object().unwrap().len(), 7);
    assert!(!value.to_string().contains("secret"));
}

#[tokio::test]
async fn activity_cold_recovery_and_live_clearing_follow_current_state() {
    let (root, app) = fixture().await;
    for (id, state) in [
        ("waiting", RecoveryState::Waiting),
        ("exhausted", RecoveryState::Exhausted),
        ("succeeded", RecoveryState::Succeeded),
    ] {
        let mut session = ChatSession::new(id.into());
        session.runtime.recovery = Some(recovery(state));
        persist(&app, &session).await;
    }
    let mut cleared = ChatSession::new("cleared".into());
    cleared.runtime.recovery = Some(recovery(RecoveryState::NeedsAction));
    persist(&app, &cleared).await;
    cleared.runtime.recovery = None;
    cleared.work_project_root = Some(root.path().to_path_buf());
    app.chat
        .sessions
        .write()
        .await
        .insert(cleared.chat_id.clone(), Arc::new(Mutex::new(cleared)));
    for (id, state) in [
        ("live-waiting", RecoveryState::Waiting),
        ("live-attempting", RecoveryState::Attempting),
    ] {
        let mut session = ChatSession::new(id.into());
        session.work_project_root = Some(root.path().to_path_buf());
        session.runtime.recovery = Some(recovery(state));
        app.chat
            .sessions
            .write()
            .await
            .insert(id.into(), Arc::new(Mutex::new(session)));
    }
    let (status, value) = request(app).await;
    assert_eq!(status, StatusCode::OK);
    let chats = value["chats"].as_array().unwrap();
    let ids: HashSet<_> = chats.iter().map(|c| c["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        HashSet::from(["waiting", "exhausted", "live-waiting", "live-attempting"])
    );
    for chat in chats {
        let id = chat["id"].as_str().unwrap();
        let expected = match id {
            "exhausted" => "exhausted",
            "live-waiting" => "waiting",
            "live-attempting" => "attempting",
            _ => "needs_action",
        };
        assert_eq!(chat["recovery_state"], expected);
        assert_eq!(
            chat["session_state"],
            if id.starts_with("live-") {
                json!("idle")
            } else {
                Value::Null
            }
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn activity_excludes_other_project_hidden_chats_and_non_time_targets() {
    let (root, app) = fixture().await;
    let mut other = ChatSession::new("other-project".into());
    other.work_project_root = Some(root.path().join("other"));
    other.runtime.state = SessionState::Generating;
    app.chat
        .sessions
        .write()
        .await
        .insert(other.chat_id.clone(), Arc::new(Mutex::new(other)));
    persist(&app, &ChatSession::new("idle".into())).await;
    let store = crate::scheduler::JsonFileCronStore::new(root.path()).unwrap();
    store
        .add(job("manual", "idle", Trigger::Manual))
        .await
        .unwrap();
    store
        .add(job(
            "webhook",
            "idle",
            Trigger::Webhook {
                hook_id: "hook".into(),
            },
        ))
        .await
        .unwrap();
    let mut isolated = job(
        "isolated",
        "idle",
        Trigger::Once {
            at_ms: 100,
        },
    );
    isolated.action = crate::scheduler::Action::AgentTurn {
        prompt: "secret".into(),
        target: crate::scheduler::AgentTarget::Isolated,
        mode: None,
        model: None,
        tools: None,
    };
    store.add(isolated).await.unwrap();
    store
        .add(job(
            "missing",
            "missing",
            Trigger::Once {
                at_ms: 100,
            },
        ))
        .await
        .unwrap();
    let (status, value) = request(app).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value, json!({"chats":[]}));
}

#[tokio::test]
async fn activity_storage_errors_are_visible() {
    let (root, app) = fixture().await;
    std::fs::create_dir_all(root.path().join(".refact")).unwrap();
    std::fs::write(root.path().join(".refact/scheduled_tasks.json"), "not json").unwrap();
    let (status, value) = request(app).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(value["detail"]
        .as_str()
        .unwrap()
        .contains("Failed to parse scheduled tasks"));
    assert!(value.get("chats").is_none());
}

#[tokio::test]
async fn activity_discovers_cold_task_agent_but_not_buddy_or_internal() {
    let (root, app) = fixture().await;
    let directory = root
        .path()
        .join(".refact/tasks/task/trajectories/agents/card");
    for (id, mode, link_type) in [
        ("task-agent", "agent", None),
        ("buddy", "buddy", None),
        ("internal", "agent", Some("internal:test")),
    ] {
        cold_file(
            &directory,
            id,
            mode,
            link_type,
            serde_json::to_value(recovery(RecoveryState::NeedsAction)).unwrap(),
        );
    }
    let path = directory.join("task-agent.json");
    let mut value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value["task_meta"] =
        json!({"task_id": "task", "role": "agent", "agent_id": "worker", "card_id": "card"});
    std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    let (status, value) = request(app).await;
    assert_eq!(status, StatusCode::OK);
    let chats = value["chats"].as_array().unwrap();
    assert_eq!(chats.len(), 1);
    assert_eq!(chats[0]["id"], "task-agent");
    assert_eq!(chats[0]["session_state"], Value::Null);
}

#[tokio::test]
async fn activity_rejected_live_sessions_remove_eligible_persisted_rows() {
    let (root, app) = fixture().await;
    for id in ["foreign", "hidden", "closed"] {
        let mut session = ChatSession::new(id.into());
        session.runtime.recovery = Some(recovery(RecoveryState::NeedsAction));
        persist(&app, &session).await;
        session.work_project_root = Some(root.path().to_path_buf());
        match id {
            "foreign" => session.work_project_root = Some(root.path().join("other")),
            "hidden" => session.thread.link_type = Some("internal:test".into()),
            "closed" => session.closed = true,
            _ => unreachable!(),
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(id.into(), Arc::new(Mutex::new(session)));
    }
    let (status, value) = request(app).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value, json!({"chats": []}));
}

#[tokio::test]
async fn activity_malformed_cold_recovery_is_unavailable_until_valid_live_override() {
    let (root, app) = fixture().await;
    cold_file(
        &root.path().join(".refact/trajectories"),
        "malformed",
        "agent",
        None,
        json!({"state": "waiting"}),
    );
    let (status, value) = request(app.clone()).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        value["detail"],
        "Chat activity is unavailable because saved recovery metadata is invalid"
    );
    assert!(value.get("chats").is_none());
    let mut session = ChatSession::new("malformed".into());
    session.work_project_root = Some(root.path().to_path_buf());
    session.runtime.state = SessionState::Generating;
    app.chat
        .sessions
        .write()
        .await
        .insert(session.chat_id.clone(), Arc::new(Mutex::new(session)));
    let (status, value) = request(app).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["chats"].as_array().unwrap().len(), 1);
    assert_eq!(value["chats"][0]["id"], "malformed");
    assert_eq!(value["chats"][0]["recovery_state"], Value::Null);
}

#[cfg(unix)]
#[tokio::test]
async fn activity_accepts_workspace_alias_for_live_project_membership() {
    let (root, app) = fixture().await;
    let aliases = tempfile::tempdir().unwrap();
    let alias = aliases.path().join("workspace");
    std::os::unix::fs::symlink(root.path(), &alias).unwrap();
    *app.gcx.documents_state.workspace_folders.lock().unwrap() = vec![alias];
    let mut session = ChatSession::new("aliased".into());
    session.work_project_root = Some(root.path().to_path_buf());
    session.runtime.state = SessionState::Generating;
    app.chat
        .sessions
        .write()
        .await
        .insert(session.chat_id.clone(), Arc::new(Mutex::new(session)));
    let (status, value) = request(app).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["chats"].as_array().unwrap().len(), 1);
    assert_eq!(value["chats"][0]["id"], "aliased");
}

#[cfg(unix)]
#[tokio::test]
async fn activity_missing_storage_is_read_only_and_symlink_storage_is_rejected() {
    let (root, app) = fixture().await;
    let (status, value) = request(app.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value, json!({"chats": []}));
    assert!(!root.path().join(".refact").exists());
    let external = tempfile::tempdir().unwrap();
    let trajectories = external.path().join("trajectories");
    cold_file(
        &trajectories,
        "external",
        "agent",
        None,
        serde_json::to_value(recovery(RecoveryState::NeedsAction)).unwrap(),
    );
    std::os::unix::fs::symlink(external.path(), root.path().join(".refact")).unwrap();
    let (status, value) = request(app).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        value["detail"],
        "Chat activity storage must be a real directory"
    );
    let names: Vec<_> = std::fs::read_dir(&trajectories)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("external.json")]);
}
