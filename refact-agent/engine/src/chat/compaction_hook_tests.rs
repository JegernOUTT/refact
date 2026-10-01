use crate::chat::context_rebuild::{
    commit_report_then_observe, postcompact_observation, precompact_control, rebuild_session,
    status, PendingContextRebuild,
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
    let thread = session.thread.clone();
    let session = Arc::new(Mutex::new(session));

    commit_report_then_observe(
        gcx.clone(),
        &session,
        report(),
        version,
        1,
        &thread,
        &[],
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
    let version = session.trajectory_version;
    let thread = session.thread.clone();
    let session = Arc::new(Mutex::new(session));

    assert!(commit_report_then_observe(
        gcx,
        &session,
        report(),
        version,
        1,
        &thread,
        &[],
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
