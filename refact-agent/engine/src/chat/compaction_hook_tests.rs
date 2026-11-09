use crate::chat::context_rebuild::{
    commit_report_then_observe, postcompact_observation, precompact_control, rebuild_session,
    status, PendingContextRebuild, RebuildSource,
};
use crate::call_validation::ChatMessage;
use crate::chat::types::{ChatSession, CompressionPhase, CompressionReason};
use crate::ext::hooks_runner::ControlVerdict;
use crate::ext::hooks::HooksConfig;
use crate::global_context::GlobalContext;
use refact_core::active_context::make_reconstruction_report;
use refact_core::active_context::ReconstructionMetadata;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::Mutex;

async fn test_context() -> (tempfile::TempDir, Arc<GlobalContext>) {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("project");
    tokio::fs::create_dir_all(&project_dir).await.unwrap();
    let mut gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        dir.path().join("cache"),
        dir.path().join("config"),
    )
    .await;
    gcx.documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .push(project_dir);
    Arc::get_mut(&mut gcx).unwrap().hooks_config = HooksConfig::default();
    (dir, gcx)
}

async fn write_hooks(gcx: &GlobalContext, precompact: Option<&str>, postcompact: Option<&str>) {
    let mut hooks = Vec::new();
    if let Some(command) = precompact {
        hooks.push(format!(
            "  PreCompact:\n    - hooks:\n        - type: command\n          command: \"{command}\""
        ));
    }
    if let Some(command) = postcompact {
        hooks.push(format!(
            "  PostCompact:\n    - hooks:\n        - type: command\n          command: \"{command}\""
        ));
    }
    tokio::fs::write(
        gcx.config_dir.join("hooks.yaml"),
        format!("hooks:\n{}\n", hooks.join("\n")),
    )
    .await
    .unwrap();
}

fn rebuild_fixture() -> (ChatSession, serde_json::Value) {
    let mut session = ChatSession::new("compact-hooks".into());
    session.add_message(ChatMessage::new(
        "user".into(),
        "history that must remain unchanged".into(),
    ));
    let before = serde_json::to_value(&session.messages).unwrap();
    (session, before)
}

fn report() -> ChatMessage {
    let mut message = ChatMessage::new("user".into(), "rebuilt context".into());
    message.message_id = "reconstructed-context".into();
    make_reconstruction_report(vec![message], ReconstructionMetadata::default()).unwrap()
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn precompact_deny_preserves_history_records_reason_and_suppresses_automatic_retry() {
    let (dir, gcx) = test_context().await;
    let marker = dir.path().join("precompact-runs");
    let command = format!(
        "echo denied >> {}; echo policy-block >&2; exit 2",
        marker.display()
    );
    write_hooks(&gcx, Some(&command), None).await;
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
    let (session, before) = rebuild_fixture();
    let session = Arc::new(Mutex::new(session));
    let request = PendingContextRebuild {
        model: None,
        trigger: "automatic".into(),
    };

    let error = rebuild_session(gcx.clone(), &session, request.clone(), None, false)
        .await
        .unwrap_err();
    assert!(error.contains("blocked by PreCompact hook: policy-block"));
    {
        let session = session.lock().await;
        assert_eq!(serde_json::to_value(&session.messages).unwrap(), before);
        assert_eq!(session.trajectory_version, 1);
        assert_eq!(session.last_rebuild_attempt_version, Some(1));
        assert_eq!(session.compression_phase, Some(CompressionPhase::Skipped));
        assert_eq!(
            session.compression_reason,
            Some(CompressionReason::PreCompactDenied)
        );
    }

    assert!(rebuild_session(gcx, &session, request, None, false)
        .await
        .unwrap_err()
        .contains("already attempted"));
    assert_eq!(
        tokio::fs::read_to_string(marker)
            .await
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn successful_commit_emits_postcompact_once_with_before_and_after_counts() {
    let (dir, gcx) = test_context().await;
    let marker = dir.path().join("postcompact-payloads");
    let command = format!("cat >> {}", marker.display());
    write_hooks(&gcx, None, Some(&command)).await;
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
    let mut session = ChatSession::new("postcompact-session".into());
    session.add_message(ChatMessage::new("user".into(), "old context".into()));
    session.active_compression_attempt = Some(1);
    status(&mut session, CompressionPhase::Running, None);
    let version = session.trajectory_version;
    let source = RebuildSource::capture(&session).unwrap();
    let thread = session.thread.clone();
    let session = Arc::new(Mutex::new(session));

    commit_report_then_observe(
        gcx.clone(),
        &session,
        report(),
        &source,
        1,
        &thread,
        &AtomicBool::new(false),
        "test-trigger",
        1,
        |snapshot| async move {
            assert_eq!(snapshot.version, version + 1);
            Ok(())
        },
    )
    .await
    .unwrap();

    let payloads = tokio::fs::read_to_string(marker).await.unwrap();
    assert_eq!(payloads.lines().count(), 1);
    let payload: serde_json::Value = serde_json::from_str(payloads.trim()).unwrap();
    assert_eq!(payload["hook_event_name"], "PostCompact");
    assert_eq!(payload["session_id"], "postcompact-session");
    assert_eq!(payload["trigger"], "test-trigger");
    assert_eq!(payload["messages_before"], 1);
    assert_eq!(payload["outcome"], "applied");
    assert_eq!(payload["messages_after"], 1);
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn failed_commit_never_emits_postcompact() {
    let (dir, gcx) = test_context().await;
    let marker = dir.path().join("postcompact-payloads");
    let command = format!("cat >> {}", marker.display());
    write_hooks(&gcx, None, Some(&command)).await;
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
    let mut session = ChatSession::new("failed-postcompact".into());
    session.add_message(ChatMessage::new("user".into(), "old context".into()));
    session.active_compression_attempt = Some(1);
    status(&mut session, CompressionPhase::Running, None);
    let source = RebuildSource::capture(&session).unwrap();
    let thread = session.thread.clone();
    let session = Arc::new(Mutex::new(session));

    assert!(commit_report_then_observe(
        gcx,
        &session,
        report(),
        &source,
        1,
        &thread,
        &AtomicBool::new(false),
        "test-trigger",
        1,
        |_| async { Err("persistence failed".into()) },
    )
    .await
    .is_err());
    assert!(!marker.exists());
}

#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn disabled_hooks_allow_and_do_not_spawn_processes() {
    let (dir, mut gcx) = test_context().await;
    Arc::get_mut(&mut gcx).unwrap().hooks_config.enabled = false;
    let marker = dir.path().join("should-not-exist");
    let command = format!("touch {}", marker.display());
    write_hooks(&gcx, Some(&command), Some(&command)).await;
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
    let thread = ChatSession::new("hooks-disabled".into()).thread;
    assert!(matches!(
        precompact_control(gcx.clone(), &thread, "disabled-test", 0).await,
        ControlVerdict::Allow
    ));
    postcompact_observation(gcx, &thread, "disabled-test", 0, 0).await;
    assert!(!marker.exists());
}

#[cfg(unix)]
async fn blocked_precompact_finalization(drop_future: bool) {
    use refact_core::chat_types::{DeliveryOutcome, PendingDelivery, PushMode};
    use std::sync::atomic::Ordering;

    let (dir, gcx) = test_context().await;
    let entered = dir.path().join("entered");
    let release = dir.path().join("release");
    let post = dir.path().join("post");
    write_hooks(
        &gcx,
        Some(&format!(
            "touch {}; while [ ! -f {} ]; do sleep 0.01; done; echo policy-block >&2; exit 2",
            entered.display(),
            release.display()
        )),
        Some(&format!("touch {}", post.display())),
    )
    .await;
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
    let (session, before) = rebuild_fixture();
    let session = Arc::new(Mutex::new(session));
    let mut rebuild = Box::pin(rebuild_session(
        gcx.clone(),
        &session,
        PendingContextRebuild {
            model: None,
            trigger: "automatic".into(),
        },
        None,
        false,
    ));
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::select! {
            result = &mut rebuild => panic!("hook must remain blocked: {result:?}"),
            _ = async {
                while !entered.exists() {
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
            } => {}
        }
    })
    .await
    .expect("PreCompact must reach controlled barrier");
    {
        let mut s = session.lock().await;
        assert_eq!(s.compression_phase, Some(CompressionPhase::Checking));
        assert!(s.active_compression_attempt.is_some());
        for (id, push) in [
            ("append", PushMode::Append),
            ("idle", PushMode::WhenIdle),
            ("preempt", PushMode::Preempt),
        ] {
            let delivery = PendingDelivery::with_id(
                id,
                vec![ChatMessage::new("event".into(), id.into())],
                push,
                "test",
                false,
            );
            assert_eq!(
                s.enqueue_delivery(delivery).unwrap(),
                DeliveryOutcome::Queued
            );
        }
        assert_eq!(serde_json::to_value(&s.messages).unwrap(), before);
        assert!(!s.abort_flag.load(Ordering::SeqCst));
    }
    let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    assert!(
        !crate::chat::delivery::drain_deliveries_at_boundary(app.clone(), session.clone()).await
    );
    assert_eq!(session.lock().await.pending_deliveries.len(), 3);
    if drop_future {
        drop(rebuild);
        tokio::fs::write(&release, "release").await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while Arc::strong_count(&session) != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("dropped wrapper must finish its finalizer");
    } else {
        tokio::fs::write(&release, "release").await.unwrap();
        let error = tokio::time::timeout(std::time::Duration::from_secs(10), rebuild)
            .await
            .expect("denied wrapper must finish")
            .unwrap_err();
        assert!(error.contains("blocked by PreCompact hook: policy-block"));
    }
    {
        let s = session.lock().await;
        assert!(s.active_compression_attempt.is_none());
        assert_eq!(
            s.compression_phase,
            Some(if drop_future {
                CompressionPhase::Failed
            } else {
                CompressionPhase::Skipped
            })
        );
        assert!(s.pending_deliveries.is_empty());
        assert_eq!(s.messages.len(), 4);
        assert_eq!(serde_json::to_value(&s.messages[..1]).unwrap(), before);
        let tail: Vec<_> = s.messages[1..]
            .iter()
            .map(|m| m.content.content_text_only())
            .collect();
        assert_eq!(tail, ["append", "idle", "preempt"]);
        assert!(s.command_queue.is_empty());
        assert!(!s.queue_processor_running.load(Ordering::SeqCst));
    }
    assert!(!post.exists());
    assert!(!crate::chat::delivery::drain_deliveries_at_boundary(app, session.clone()).await);
    assert_eq!(session.lock().await.messages.len(), 4);
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn checking_hook_denial_wrapper_flushes_passive_events_once_without_waking() {
    blocked_precompact_finalization(false).await;
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn dropped_checking_wrapper_flushes_passive_events_once_without_waking() {
    blocked_precompact_finalization(true).await;
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn blocked_postcompact_cannot_let_new_delivery_or_user_command_overtake_held_event() {
    use crate::chat::types::{ChatCommand, CommandRequest};
    use refact_core::chat_types::{DeliveryOutcome, PendingDelivery, PushMode};

    let (dir, gcx) = test_context().await;
    let entered = dir.path().join("post-entered");
    let release = dir.path().join("post-release");
    write_hooks(
        &gcx,
        None,
        Some(&format!(
            "touch {}; while [ ! -f {} ]; do sleep 0.01; done",
            entered.display(),
            release.display()
        )),
    )
    .await;
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
    let (mut s, _) = rebuild_fixture();
    s.active_compression_attempt = Some(1);
    status(&mut s, CompressionPhase::Running, None);
    let source = RebuildSource::capture(&s).unwrap();
    let thread = s.thread.clone();
    assert_eq!(
        s.enqueue_delivery(PendingDelivery::with_id(
            "A",
            vec![ChatMessage::new("event".into(), "A".into())],
            PushMode::Append,
            "test",
            false,
        ))
        .unwrap(),
        DeliveryOutcome::Queued
    );
    s.command_queue.push_back(CommandRequest {
        native_approval_guard: None,
        supervision_guard: None,
        client_request_id: "queued-user".into(),
        priority: true,
        command: ChatCommand::UserMessage {
            content: serde_json::json!("command"),
            attachments: vec![],
            context_files: vec![],
            suppress_auto_enrichment: true,
            client_message_id: None,
            app_context_ids: Vec::new(),
        },
    });
    let session = Arc::new(Mutex::new(s));
    let abort = AtomicBool::new(false);
    let mut commit = Box::pin(commit_report_then_observe(
        gcx.clone(),
        &session,
        report(),
        &source,
        1,
        &thread,
        &abort,
        "test",
        1,
        |_| async { Ok(()) },
    ));
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::select! {
            result = &mut commit => panic!("PostCompact must block: {result:?}"),
            _ = async { while !entered.exists() { tokio::time::sleep(std::time::Duration::from_millis(1)).await; } } => {}
        }
    }).await.unwrap();
    {
        let mut s = session.lock().await;
        assert_eq!(
            s.enqueue_delivery(PendingDelivery::with_id(
                "B",
                vec![ChatMessage::new("event".into(), "B".into())],
                PushMode::Append,
                "test",
                false,
            ))
            .unwrap(),
            DeliveryOutcome::Delivered
        );
    }
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    assert!(crate::chat::queue::inject_priority_messages_if_any(app, session.clone()).await);
    let s = session.lock().await;
    let tail: Vec<_> = s.messages[2..]
        .iter()
        .map(|m| m.content.content_text_only())
        .collect();
    assert_eq!(tail, ["A", "B", "command"]);
    assert!(s.pending_deliveries.is_empty());
    drop(s);
    tokio::fs::write(release, "release").await.unwrap();
    commit.await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn cancelled_terminal_flush_finishes_persistence_after_writer_barrier() {
    use refact_core::chat_types::{PendingDelivery, PushMode};

    let (dir, gcx) = test_context().await;
    let release = dir.path().join("deny-release");
    write_hooks(
        &gcx,
        Some(&format!(
            "while [ ! -f {} ]; do sleep 0.01; done; echo denied >&2; exit 2",
            release.display()
        )),
        None,
    )
    .await;
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
    let (s, _) = rebuild_fixture();
    let session = Arc::new(Mutex::new(s));
    let writer = session.lock().await.trajectory_save_mutex.clone();
    let barrier = writer.lock().await;
    let mut rebuild = Box::pin(rebuild_session(
        gcx.clone(),
        &session,
        PendingContextRebuild {
            model: None,
            trigger: "test".into(),
        },
        None,
        false,
    ));
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::select! {
            result = &mut rebuild => panic!("flush must block on writer: {result:?}"),
            _ = async {
                loop {
                    let mut s = session.lock().await;
                    if s.compression_phase == Some(CompressionPhase::Checking) {
                        s.enqueue_delivery(PendingDelivery::with_id(
                            "held", vec![ChatMessage::new("event".into(), "held".into())], PushMode::Append, "test", false,
                        )).unwrap();
                        tokio::fs::write(&release, "release").await.unwrap();
                    }
                    if s.compression_phase == Some(CompressionPhase::Skipped) { break; }
                    drop(s);
                    tokio::task::yield_now().await;
                }
            } => {}
        }
    }).await.unwrap();
    let lock = session.lock().await;
    drop(rebuild);
    drop(barrier);
    drop(lock);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while Arc::strong_count(&session) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let s = session.lock().await;
    assert!(s.pending_deliveries.is_empty());
    assert_eq!(s.messages.len(), 2);
    assert_eq!(s.messages[1].content.content_text_only(), "held");
    assert!(!s.trajectory_dirty);
    drop(s);
    let saved = crate::chat::trajectories::load_trajectory_for_chat(gcx, "compact-hooks")
        .await
        .unwrap();
    assert!(saved.pending_deliveries.is_empty());
    assert_eq!(saved.messages.len(), 2);
}
