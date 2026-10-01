use crate::chat::notifications::deliver_process_completion;
use crate::chat::types::{ChatSession, DeliveryOutcome, PushMode};
use crate::exec::{ExecMode, ExecProcessId, ExecStatus, ProcessCompletionEvent};
use crate::ext::hooks::HooksConfig;
use crate::global_context::{GlobalContext, SharedGlobalContext};
use std::sync::Arc;
use tokio::sync::Mutex;

async fn fixture(
    enabled: bool,
) -> (
    tempfile::TempDir,
    SharedGlobalContext,
    crate::app_state::AppState,
    Arc<Mutex<ChatSession>>,
) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("project");
    let config = dir.path().join("config");
    tokio::fs::create_dir_all(&workspace).await.unwrap();
    tokio::fs::create_dir_all(&config).await.unwrap();
    let mut gcx =
        crate::global_context::tests::make_test_gcx_with_dirs(dir.path().join("cache"), config)
            .await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace];
    Arc::get_mut(&mut gcx).unwrap().hooks_config = HooksConfig {
        enabled,
        ..HooksConfig::default()
    };
    let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    let chat_id = "notification-hook-chat";
    let session = Arc::new(Mutex::new(ChatSession::new(chat_id.to_string())));
    gcx.chat_sessions
        .write()
        .await
        .insert(chat_id.to_string(), session.clone());
    (dir, gcx, app, session)
}

async fn write_notification_hook(gcx: &GlobalContext, command: &str) {
    tokio::fs::write(
        gcx.config_dir.join("hooks.yaml"),
        format!(
            "hooks:\n  Notification:\n    - hooks:\n        - type: command\n          command: {command:?}\n"
        ),
    )
    .await
    .unwrap();
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
}

fn completion_event() -> ProcessCompletionEvent {
    ProcessCompletionEvent {
        process_id: ExecProcessId("exec_notification_hook".to_string()),
        chat_id: "notification-hook-chat".to_string(),
        status: ExecStatus::Exited { exit_code: Some(0) },
        exit_code: Some(0),
        duration_ms: Some(7),
        short_description: "notification hook test".to_string(),
        mode: ExecMode::Background,
        push: PushMode::Append,
    }
}

fn marker_command(marker: &std::path::Path) -> String {
    format!(
        "cat >> '{}'; echo >> '{}'",
        marker.display(),
        marker.display()
    )
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn delivered_process_completion_emits_one_notification_hook() {
    let (dir, gcx, app, _) = fixture(true).await;
    let marker = dir.path().join("notification-payloads");
    write_notification_hook(&gcx, &marker_command(&marker)).await;

    let event = completion_event();
    assert_eq!(
        deliver_process_completion(app, &event).await.unwrap(),
        DeliveryOutcome::Delivered
    );

    let payloads = tokio::fs::read_to_string(marker).await.unwrap();
    assert_eq!(payloads.lines().count(), 1);
    let payload: serde_json::Value = serde_json::from_str(payloads.trim()).unwrap();
    assert_eq!(payload["hook_event_name"], "Notification");
    assert_eq!(payload["kind"], "process_completion");
    assert_eq!(payload["source"], "exec.registry");
    assert_eq!(payload["chat_id"], event.chat_id);
    assert_eq!(
        payload["summary"],
        "Background process 'notification hook test' exited (exit 0)"
    );
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn rejected_process_completion_delivery_does_not_emit_notification_hook() {
    let (dir, gcx, app, session) = fixture(true).await;
    let marker = dir.path().join("notification-payloads");
    write_notification_hook(&gcx, &marker_command(&marker)).await;
    session.lock().await.closed = true;

    assert!(deliver_process_completion(app, &completion_event())
        .await
        .unwrap_err()
        .contains("closed"));
    assert!(!marker.exists());
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn disabled_hooks_do_not_create_notification_marker() {
    let (dir, gcx, app, _) = fixture(false).await;
    let marker = dir.path().join("notification-payloads");
    write_notification_hook(&gcx, &marker_command(&marker)).await;

    assert_eq!(
        deliver_process_completion(app, &completion_event())
            .await
            .unwrap(),
        DeliveryOutcome::Delivered
    );
    assert!(!marker.exists());
}
