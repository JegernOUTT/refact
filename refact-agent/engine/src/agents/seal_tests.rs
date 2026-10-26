use std::sync::Arc;
use std::time::Duration;

use serial_test::serial;
use tokio::sync::{Mutex, Notify};

use crate::agents::seal::{seal_reason, SealReason};
use crate::agents::spawn::install_test_runner;
use crate::agents::spawn_lifecycle_tests::{await_completion, repo_fixture, spawn_request};
use crate::agents::types::BgAgentKind;
use crate::app_state::AppState;
use crate::call_validation::ChatMessage;
use crate::chat::types::ChatSession;
use crate::subchat::SubchatResult;

fn is_agent_sealed(reason: Option<SealReason>, status: &str, generation: u64) -> bool {
    matches!(reason, Some(SealReason::AgentTerminal { status: actual, run_generation, .. }) if actual == status && run_generation == generation)
}

fn parent(fixture: &super::spawn_lifecycle_tests::RepoFixture, id: &str) -> ChatSession {
    let mut session = ChatSession::new(id.into());
    session.work_project_root = Some(fixture.source.clone());
    session.set_runtime_state(crate::chat::types::SessionState::Generating, None);
    session
}

async fn install_parent(fixture: &super::spawn_lifecycle_tests::RepoFixture, id: &str) {
    fixture
        .app
        .chat
        .sessions
        .write()
        .await
        .insert(id.into(), Arc::new(Mutex::new(parent(fixture, id))));
}

async fn persist_result(
    gcx: Arc<crate::global_context::GlobalContext>,
    mut messages: Vec<ChatMessage>,
    config: crate::subchat::SubchatConfig,
    aborted: bool,
    text: &str,
) -> Result<SubchatResult, String> {
    let app = AppState::from_gcx(gcx.clone()).await;
    let child_id = config.chat_id.as_deref().unwrap().to_string();
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(&child_id)
        .cloned()
        .unwrap();
    let thread = session.lock().await.thread.clone();
    messages.push(ChatMessage::new("assistant".into(), text.to_string()));
    crate::chat::trajectories::save_trajectory_as(gcx, &thread, &messages).await;
    crate::subchat::mirror_subchat_messages_into_session(&app, &child_id, &messages).await;
    crate::subchat::finish_stateful_subchat_session(&app, &child_id, &config).await;
    Ok(SubchatResult {
        messages,
        metering: serde_json::Map::new(),
        chat_id: Some(child_id),
        aborted,
    })
}

fn completed_runner(text: &'static str) -> crate::agents::spawn::TestRunnerGuard {
    install_test_runner(Arc::new(move |gcx, messages, config| {
        Box::pin(async move { persist_result(gcx, messages, config, false, text).await })
    }))
}

#[serial(test_runner)]
#[tokio::test]
async fn running_child_is_not_sealed() {
    let fixture = repo_fixture().await;
    let parent_id = "seal-running-parent";
    install_parent(&fixture, parent_id).await;
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let _runner = {
        let started = started.clone();
        let release = release.clone();
        install_test_runner(Arc::new(move |_gcx, messages, config| {
            let started = started.clone();
            let release = release.clone();
            Box::pin(async move {
                started.notify_one();
                release.notified().await;
                Ok(SubchatResult {
                    messages,
                    metering: serde_json::Map::new(),
                    chat_id: config.chat_id,
                    aborted: false,
                })
            })
        }))
    };
    let handle = crate::agents::spawn::spawn_background_agent(
        fixture.app.clone(),
        spawn_request(parent_id, BgAgentKind::Subagent),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(20), started.notified())
        .await
        .unwrap();

    assert_eq!(seal_reason(&fixture.app, &handle.child_chat_id).await, None);

    release.notify_one();
    let _ = await_completion(handle).await;
}

#[serial(test_runner)]
#[tokio::test]
async fn continue_unseals_then_seals_again_at_generation_two() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let fixture = repo_fixture().await;
    let parent_id = "seal-continue-parent";
    install_parent(&fixture, parent_id).await;
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
                let app = AppState::from_gcx(gcx.clone()).await;
                let child_id = config.chat_id.as_deref().unwrap().to_string();
                if attempt == 1 {
                    assert_eq!(seal_reason(&app, &child_id).await, None);
                    assert!(messages.iter().any(|message| {
                        message.role == "user"
                            && message.content.content_text_only().contains("continue")
                    }));
                    resumed.notify_one();
                    release.notified().await;
                }
                persist_result(
                    gcx,
                    messages,
                    config,
                    false,
                    if attempt == 0 {
                        "generation one done"
                    } else {
                        "generation two done"
                    },
                )
                .await
            })
        }))
    };
    let completed = await_completion(
        crate::agents::spawn::spawn_background_agent(
            fixture.app.clone(),
            spawn_request(parent_id, BgAgentKind::Subagent),
        )
        .await
        .unwrap(),
    )
    .await;
    let child_id = completed.child_chat_id.clone().unwrap();
    assert!(is_agent_sealed(
        seal_reason(&fixture.app, &child_id).await,
        "completed",
        1
    ));

    let queued = crate::agents::spawn::resume_agent(
        fixture.app.clone(),
        parent_id,
        &completed.agent_id,
        None,
        completed.change_seq,
        Some("continue this".into()),
        true,
    )
    .await
    .unwrap();
    assert_eq!(queued.run_generation, 2);
    tokio::time::timeout(Duration::from_secs(20), resumed.notified())
        .await
        .unwrap();
    assert_eq!(seal_reason(&fixture.app, &child_id).await, None);

    release.notify_one();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let latest = fixture
                .app
                .agents
                .get_any(&completed.agent_id)
                .await
                .unwrap();
            if latest.status.is_terminal() && latest.run_generation == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(is_agent_sealed(
        seal_reason(&fixture.app, &child_id).await,
        "completed",
        2
    ));
}

#[serial(test_runner)]
#[tokio::test]
async fn seal_survives_reload() {
    let fixture = repo_fixture().await;
    let parent_id = "seal-reload-parent";
    install_parent(&fixture, parent_id).await;
    let _runner = completed_runner("done");
    let completed = await_completion(
        crate::agents::spawn::spawn_background_agent(
            fixture.app.clone(),
            spawn_request(parent_id, BgAgentKind::Subagent),
        )
        .await
        .unwrap(),
    )
    .await;
    let child_id = completed.child_chat_id.as_deref().unwrap().to_string();
    fixture
        .app
        .agents
        .flush_pending_writes_for_test()
        .await
        .unwrap();

    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        fixture.cache.clone(),
        fixture._temp.path().join("config"),
    )
    .await;
    let reloaded = AppState::from_gcx(gcx).await;
    assert!(is_agent_sealed(
        seal_reason(&reloaded, &child_id).await,
        "completed",
        1
    ));
}

#[tokio::test]
async fn unknown_or_top_level_chat_is_not_sealed() {
    let fixture = repo_fixture().await;
    assert_eq!(seal_reason(&fixture.app, "not-a-chat").await, None);
    fixture.app.chat.sessions.write().await.insert(
        "seal-top-level-chat".into(),
        Arc::new(Mutex::new(ChatSession::new("seal-top-level-chat".into()))),
    );
    assert_eq!(seal_reason(&fixture.app, "seal-top-level-chat").await, None);
}
