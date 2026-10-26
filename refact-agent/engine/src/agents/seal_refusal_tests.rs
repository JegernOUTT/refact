use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use hyper::{Body, Response, StatusCode};
use serde_json::{json, Value};
use serial_test::serial;
use tokio::sync::{Mutex, Notify};

use crate::agents::seal::seal_reason;
use crate::agents::spawn::{install_test_runner, TestRunnerGuard};
use crate::agents::spawn_lifecycle_tests::{await_completion, repo_fixture, spawn_request, RepoFixture};
use crate::agents::types::{BackgroundAgent, BgAgentKind, BgAgentStatus};
use crate::app_state::AppState;
use crate::call_validation::ChatMessage;
use crate::chat::types::{ChatSession, DeliveryOutcome, DeliveryStatus, PendingDelivery, PushMode};
use crate::subchat::{SubchatConfig, SubchatResult};

async fn fixture() -> RepoFixture {
    let mut fixture = repo_fixture().await;
    let service = crate::worktrees::service::WorktreeService::new(
        fixture.cache.clone(),
        fixture.source.clone(),
    )
    .unwrap();
    fixture.parent_worktree = service
        .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
            branch: Some("refact/chat/seal-refusal".into()),
            kind: Some("chat".into()),
            ..Default::default()
        })
        .await
        .unwrap()
        .worktree
        .meta;
    let mut parent = ChatSession::new("seal-refusal-parent".into());
    parent.work_project_root = Some(fixture.source.clone());
    parent.thread.worktree = Some(fixture.parent_worktree.clone());
    fixture
        .app
        .chat
        .sessions
        .write()
        .await
        .insert(parent.chat_id.clone(), Arc::new(Mutex::new(parent)));
    fixture
}

async fn persist_result(
    gcx: Arc<crate::global_context::GlobalContext>,
    mut messages: Vec<ChatMessage>,
    config: SubchatConfig,
) -> Result<SubchatResult, String> {
    let app = AppState::from_gcx(gcx.clone()).await;
    let child_id = config.chat_id.as_deref().unwrap().to_string();
    let child = app
        .chat
        .sessions
        .read()
        .await
        .get(&child_id)
        .cloned()
        .unwrap();
    let thread = child.lock().await.thread.clone();
    messages.push(ChatMessage::new("assistant".into(), "done".into()));
    crate::chat::trajectories::save_trajectory_as(gcx, &thread, &messages).await;
    crate::subchat::mirror_subchat_messages_into_session(&app, &child_id, &messages).await;
    crate::subchat::finish_stateful_subchat_session(&app, &child_id, &config).await;
    Ok(SubchatResult {
        messages,
        metering: serde_json::Map::new(),
        chat_id: Some(child_id),
        aborted: false,
    })
}

fn completed_runner() -> TestRunnerGuard {
    install_test_runner(Arc::new(|gcx, messages, config| {
        Box::pin(persist_result(gcx, messages, config))
    }))
}

async fn spawn(fixture: &RepoFixture) -> crate::agents::spawn::SpawnHandle {
    let mut request = spawn_request("seal-refusal-parent", BgAgentKind::Subagent);
    request.parent_worktree = Some(fixture.parent_worktree.clone());
    crate::agents::spawn::spawn_background_agent(fixture.app.clone(), request)
        .await
        .unwrap()
}

async fn completed(fixture: &RepoFixture) -> BackgroundAgent {
    let record = await_completion(spawn(fixture).await).await;
    assert_eq!(
        record.status,
        BgAgentStatus::Completed,
        "{:?}",
        record.error
    );
    assert!(
        seal_reason(&fixture.app, record.child_chat_id.as_deref().unwrap())
            .await
            .is_some()
    );
    record
}

async fn child(fixture: &RepoFixture, child_id: &str) -> Arc<Mutex<ChatSession>> {
    fixture
        .app
        .chat
        .sessions
        .read()
        .await
        .get(child_id)
        .cloned()
        .unwrap()
}

async fn command(app: &AppState, child_id: &str, id: &str, command: Value) -> Response<Body> {
    let mut request = command;
    request["client_request_id"] = json!(id);
    crate::chat::handle_v1_chat_command(
        State(app.clone()),
        Path(child_id.to_string()),
        serde_json::to_vec(&request).unwrap().into(),
    )
    .await
    .unwrap()
}

fn delivery(id: &str, push: PushMode, wake: bool) -> PendingDelivery {
    PendingDelivery::with_id(
        id,
        vec![ChatMessage::new("user".into(), id.into())],
        push,
        "seal-refusal-test",
        wake,
    )
}

#[serial(test_runner)]
#[tokio::test]
async fn sealed_child_refuses_user_messages() {
    let fixture = fixture().await;
    let _runner = completed_runner();
    let record = completed(&fixture).await;
    let child_id = record.child_chat_id.as_deref().unwrap();
    let session = child(&fixture, child_id).await;
    let reason = seal_reason(&fixture.app, child_id).await.unwrap().message();
    let (count, state, mut events) = {
        let mut session = session.lock().await;
        session.thread.checkpoints_enabled = false;
        (
            session.messages.len(),
            session.runtime.state,
            session.subscribe(),
        )
    };

    command(
        &fixture.app,
        child_id,
        "sealed-user-message",
        json!({"type": "user_message", "content": "do not restart this child"}),
    )
    .await;
    let event = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let event: Value = serde_json::from_str(&events.recv().await.unwrap()).unwrap();
            assert_ne!(event["type"], "stream_started");
            if event["type"] == "message_added"
                || (event["client_request_id"] == "sealed-user-message"
                    && event["accepted"] == false)
            {
                break event;
            }
        }
    })
    .await
    .unwrap();

    let session = session.lock().await;
    assert_eq!(
        session.messages.len(),
        count,
        "sealed UserMessage must not append"
    );
    assert_eq!(session.runtime.state, state);
    assert!(session.draft_message.is_none());
    assert_eq!(event["accepted"], false);
    assert_eq!(event["result"]["error"], reason);
    while let Ok(event) = events.try_recv() {
        let event: Value = serde_json::from_str(&event).unwrap();
        assert_ne!(event["type"], "stream_started");
    }
}

#[serial(test_runner)]
#[tokio::test]
async fn sealed_child_refuses_worktree_unbinding() {
    let fixture = fixture().await;
    let _runner = completed_runner();
    let record = completed(&fixture).await;
    let child_id = record.child_chat_id.as_deref().unwrap();
    let session = child(&fixture, child_id).await;
    let worktree = session.lock().await.thread.worktree.clone();
    assert!(worktree.is_some());
    let reason = seal_reason(&fixture.app, child_id).await.unwrap().message();

    for (id, patch) in [
        ("unbind", json!({"worktree": null})),
        ("rebind-id", json!({"worktree_id": "different-worktree"})),
        (
            "rebind-meta",
            json!({"worktree": {"id": "different-worktree"}}),
        ),
    ] {
        let response = command(
            &fixture.app,
            child_id,
            id,
            json!({"type": "set_params", "patch": patch}),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body: Value =
            serde_json::from_slice(&hyper::body::to_bytes(response.into_body()).await.unwrap())
                .unwrap();
        assert_eq!(body["error"], reason);
        assert_eq!(session.lock().await.thread.worktree, worktree);
    }

    let response = command(
        &fixture.app,
        child_id,
        "model-only",
        json!({"type": "set_params", "patch": {"model": "openai/updated-model"}}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let session = session.lock().await;
    assert_eq!(session.thread.model, "openai/updated-model");
    assert_eq!(session.thread.worktree, worktree);
}

#[serial(test_runner)]
#[tokio::test]
async fn wake_delivery_into_sealed_child_does_not_start_generation() {
    let fixture = fixture().await;
    let _runner = completed_runner();
    let record = completed(&fixture).await;
    let child_id = record.child_chat_id.as_deref().unwrap();
    let session = child(&fixture, child_id).await;
    let (count, state, mut events) = {
        let session = session.lock().await;
        (
            session.messages.len(),
            session.runtime.state,
            session.subscribe(),
        )
    };

    for (id, push) in [
        ("sealed-append", PushMode::Append),
        ("sealed-preempt", PushMode::Preempt),
        ("sealed-idle", PushMode::WhenIdle),
    ] {
        assert_eq!(
            crate::chat::deliver_to_chat(fixture.app.clone(), child_id, delivery(id, push, true))
                .await
                .unwrap(),
            DeliveryOutcome::Sealed
        );
        let session = session.lock().await;
        assert_eq!(session.delivery_status(id), DeliveryStatus::Sealed);
        assert!(session.dropped_deliveries.iter().any(|dropped| {
            dropped.id == id && serde_json::to_value(dropped).unwrap()["status"] == "sealed"
        }));
        assert_eq!(session.messages.len(), count);
        assert_eq!(session.runtime.state, state);
        assert!(session.pending_deliveries.is_empty());
        assert!(session.delivery_wake_sources.is_empty());
        assert!(!session.queue_processor_running.load(Ordering::SeqCst));
        assert!(session.draft_message.is_none());
    }
    assert_eq!(
        crate::chat::deliver_to_chat(
            fixture.app.clone(),
            child_id,
            delivery("sealed-append", PushMode::Append, true)
        )
        .await
        .unwrap(),
        DeliveryOutcome::Duplicate
    );
    let saved =
        crate::chat::trajectories::load_trajectory_for_chat(fixture.app.gcx.clone(), child_id)
            .await
            .unwrap();
    assert!(saved
        .dropped_deliveries
        .iter()
        .any(|dropped| dropped.id == "sealed-append" && dropped.status == DeliveryStatus::Sealed));
    while let Ok(event) = events.try_recv() {
        let event: Value = serde_json::from_str(&event).unwrap();
        assert_ne!(event["type"], "stream_started");
    }
}

#[serial(test_runner)]
#[tokio::test]
async fn informational_delivery_into_sealed_child_is_appended() {
    let fixture = fixture().await;
    let _runner = completed_runner();
    let record = completed(&fixture).await;
    let child_id = record.child_chat_id.as_deref().unwrap();
    let session = child(&fixture, child_id).await;
    let count = session.lock().await.messages.len();

    assert_eq!(
        crate::chat::deliver_to_chat(
            fixture.app.clone(),
            child_id,
            delivery("informational", PushMode::Append, false)
        )
        .await
        .unwrap(),
        DeliveryOutcome::Delivered
    );
    let session = session.lock().await;
    assert_eq!(session.messages.len(), count + 1);
    assert_eq!(
        session.messages.last().unwrap().content.content_text_only(),
        "informational"
    );
    assert_eq!(
        session.delivery_status("informational"),
        DeliveryStatus::Delivered
    );
    assert!(session.delivery_wake_sources.is_empty());
    assert!(!session.queue_processor_running.load(Ordering::SeqCst));
    drop(session);
    assert!(seal_reason(&fixture.app, child_id).await.is_some());
}

#[serial(test_runner)]
#[tokio::test]
async fn running_child_accepts_messages_after_continue() {
    let fixture = fixture().await;
    let attempts = Arc::new(AtomicUsize::new(0));
    let resumed = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let _runner = {
        let attempts = attempts.clone();
        let resumed = resumed.clone();
        let release = release.clone();
        install_test_runner(Arc::new(move |gcx, messages, config| {
            let attempt = attempts.fetch_add(1, Ordering::SeqCst);
            let resumed = resumed.clone();
            let release = release.clone();
            Box::pin(async move {
                if attempt == 1 {
                    let app = AppState::from_gcx(gcx.clone()).await;
                    let child_id = config.chat_id.as_deref().unwrap();
                    assert_eq!(seal_reason(&app, child_id).await, None);
                    assert!(messages.iter().any(|message| message.role == "user"
                        && message.content.content_text_only() == "continue with generation two"));
                    assert_eq!(
                        crate::chat::deliver_to_chat(
                            app.clone(),
                            child_id,
                            delivery("resumed-message", PushMode::Append, true)
                        )
                        .await
                        .unwrap(),
                        DeliveryOutcome::Queued
                    );
                    let agent_id = app
                        .agents
                        .find_agent_id_by_child_chat_id(child_id)
                        .await
                        .unwrap();
                    let delivered =
                        crate::agents::delivery::drain_authorized(&app, &agent_id, false)
                            .await
                            .unwrap();
                    assert!(delivered.iter().any(|delivery| delivery
                        .messages
                        .iter()
                        .any(|message| message.content.content_text_only() == "resumed-message")));
                    app.agents
                        .consume_delivery(&agent_id, "resumed-message")
                        .await
                        .unwrap();
                    resumed.notify_one();
                    release.notified().await;
                }
                persist_result(gcx, messages, config).await
            })
        }))
    };
    let record = completed(&fixture).await;
    let reopened = crate::agents::spawn::resume_agent(
        fixture.app.clone(),
        "seal-refusal-parent",
        &record.agent_id,
        None,
        record.change_seq,
        Some("continue with generation two".into()),
        true,
    )
    .await
    .unwrap();
    assert_eq!(reopened.run_generation, 2);
    tokio::time::timeout(Duration::from_secs(20), resumed.notified())
        .await
        .unwrap();
    let child_id = record.child_chat_id.as_deref().unwrap();
    assert_eq!(seal_reason(&fixture.app, child_id).await, None);
    release.notify_one();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let latest = fixture.app.agents.get_any(&record.agent_id).await.unwrap();
            if latest.status.is_terminal() && latest.run_generation == 2 {
                assert_eq!(
                    latest.status,
                    BgAgentStatus::Completed,
                    "{:?}",
                    latest.error
                );
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[serial(test_runner)]
#[tokio::test]
async fn running_child_refuses_worktree_rebinding() {
    let fixture = fixture().await;
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let _runner = {
        let started = started.clone();
        let release = release.clone();
        install_test_runner(Arc::new(move |gcx, messages, config| {
            let started = started.clone();
            let release = release.clone();
            Box::pin(async move {
                started.notify_one();
                release.notified().await;
                persist_result(gcx, messages, config).await
            })
        }))
    };
    let handle = spawn(&fixture).await;
    tokio::time::timeout(Duration::from_secs(20), started.notified())
        .await
        .unwrap();
    assert_eq!(seal_reason(&fixture.app, &handle.child_chat_id).await, None);
    let session = child(&fixture, &handle.child_chat_id).await;
    let worktree = session.lock().await.thread.worktree.clone().unwrap();
    for (id, patch) in [
        ("running-unbind", json!({"worktree": null})),
        (
            "running-rebind",
            json!({"worktree_id": "different-worktree"}),
        ),
    ] {
        let response = command(
            &fixture.app,
            &handle.child_chat_id,
            id,
            json!({"type": "set_params", "patch": patch}),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body: Value =
            serde_json::from_slice(&hyper::body::to_bytes(response.into_body()).await.unwrap())
                .unwrap();
        assert_eq!(
            body["error"],
            "Background agent child chats cannot change their worktree"
        );
        assert_eq!(
            session.lock().await.thread.worktree.as_ref(),
            Some(&worktree)
        );
    }
    let response = command(
        &fixture.app,
        &handle.child_chat_id,
        "same-worktree",
        json!({"type": "set_params", "patch": {"worktree_id": worktree.id, "temperature": 0.25}}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(session.lock().await.thread.temperature, Some(0.25));
    release.notify_one();
    assert_eq!(
        await_completion(handle).await.status,
        BgAgentStatus::Completed
    );
}
