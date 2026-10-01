use crate::ext::hooks_runner::{
    clear_hook_cache_for_tests, run_control_hooks, ControlVerdict, HookPayload,
};
use crate::ext::hooks::HookEvent;
use crate::global_context::tests::make_test_gcx_with_dirs;
use crate::app_state::AppState;

#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn different_config_dirs_never_share_cached_hooks() {
    clear_hook_cache_for_tests();
    let root = tempfile::tempdir().unwrap();
    let config_a = root.path().join("config-a");
    let config_b = root.path().join("config-b");
    let marker_a = root.path().join("hook-a-ran");
    let marker_b = root.path().join("hook-b-ran");
    let command_a = if cfg!(windows) {
        format!("echo ran > \"{}\"", marker_a.display())
    } else {
        format!("echo ran > '{}'", marker_a.display())
    };
    let command_b = if cfg!(windows) {
        format!("echo ran > \"{}\"", marker_b.display())
    } else {
        format!("echo ran > '{}'", marker_b.display())
    };
    tokio::fs::create_dir_all(&config_a).await.unwrap();
    tokio::fs::create_dir_all(&config_b).await.unwrap();
    tokio::fs::write(
        config_a.join("hooks.yaml"),
        format!(
            "hooks:\n  PreToolUse:\n    - hooks:\n        - type: command\n          command: {:?}\n",
            command_a
        ),
    )
    .await
    .unwrap();
    tokio::fs::write(
        config_b.join("hooks.yaml"),
        format!(
            "hooks:\n  PreToolUse:\n    - hooks:\n        - type: command\n          command: {:?}\n",
            command_b
        ),
    )
    .await
    .unwrap();

    let gcx_a = make_test_gcx_with_dirs(root.path().join("cache-a"), config_a).await;
    let gcx_b = make_test_gcx_with_dirs(root.path().join("cache-b"), config_b).await;
    let app_a = AppState::from_gcx(gcx_a).await;
    let app_b = AppState::from_gcx(gcx_b).await;
    let event = HookEvent::PreToolUse;

    assert!(matches!(
        run_control_hooks(
            app_a,
            event.clone(),
            HookPayload::new(&event, "session-a".into(), String::new()),
        )
        .await,
        ControlVerdict::Allow
    ));
    assert!(marker_a.exists());

    assert!(matches!(
        run_control_hooks(
            app_b,
            event.clone(),
            HookPayload::new(&event, "session-b".into(), String::new()),
        )
        .await,
        ControlVerdict::Allow
    ));
    assert!(marker_b.exists());
}
