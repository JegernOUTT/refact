use crate::ext::hooks_runner::{
    clear_hook_cache_for_tests, hook_surface, run_control_hooks, ControlVerdict, HookPayload,
};
use crate::ext::hooks::HookEvent;
use crate::global_context::tests::make_test_gcx_with_dirs;
use crate::app_state::AppState;

fn marker_command(marker: &std::path::Path) -> String {
    #[cfg(windows)]
    {
        use base64::Engine;
        // cmd /c is the production shell. Windows argv escaping is not cmd
        // escaping; encode the script rather than passing embedded path quotes.
        let script = format!(
            "[System.IO.File]::WriteAllText('{}', 'ran')",
            marker.to_string_lossy().replace('\'', "''")
        );
        let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
        format!(
            "powershell.exe -NoProfile -NonInteractive -EncodedCommand {}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )
    }
    #[cfg(not(windows))]
    {
        format!(
            "printf ran > '{}'",
            marker.to_string_lossy().replace('\'', "'\\''")
        )
    }
}

#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn different_config_dirs_never_share_cached_hooks() {
    clear_hook_cache_for_tests();
    let root = tempfile::tempdir().unwrap();
    let config_a = root.path().join("config-a");
    let config_b = root.path().join("config-b");
    let marker_a = root.path().join("hook-a-ran");
    let marker_b = root.path().join("hook-b-ran");
    let command_a = marker_command(&marker_a);
    let command_b = marker_command(&marker_b);
    tokio::fs::create_dir_all(&config_a).await.unwrap();
    tokio::fs::create_dir_all(&config_b).await.unwrap();
    tokio::fs::write(
        config_a.join("hooks.yaml"),
        serde_yaml::to_string(&serde_json::json!({
            "hooks": {"PreToolUse": [{"hooks": [{"type": "command", "command": command_a}]}]}
        }))
        .unwrap(),
    )
    .await
    .unwrap();
    tokio::fs::write(
        config_b.join("hooks.yaml"),
        serde_yaml::to_string(&serde_json::json!({
            "hooks": {"PreToolUse": [{"hooks": [{"type": "command", "command": command_b}]}]}
        }))
        .unwrap(),
    )
    .await
    .unwrap();

    let gcx_a = make_test_gcx_with_dirs(root.path().join("cache-a"), config_a).await;
    let gcx_b = make_test_gcx_with_dirs(root.path().join("cache-b"), config_b).await;
    let app_a = AppState::from_gcx(gcx_a).await;
    let app_b = AppState::from_gcx(gcx_b).await;
    let event = HookEvent::PreToolUse;

    let surface_a = hook_surface(app_a.clone(), 0).await;
    assert_eq!(surface_a.registrations.len(), 1);
    assert_eq!(surface_a.registrations[0].command, command_a);
    assert_eq!(surface_a.runnable, 1);
    assert!(matches!(
        run_control_hooks(
            app_a,
            event.clone(),
            HookPayload::new(&event, "session-a".into(), String::new()),
        )
        .await,
        ControlVerdict::Allow
    ));
    assert_eq!(std::fs::read_to_string(&marker_a).unwrap(), "ran");
    assert!(!marker_b.exists());

    let surface_b = hook_surface(app_b.clone(), 0).await;
    assert_eq!(surface_b.registrations.len(), 1);
    assert_eq!(surface_b.registrations[0].command, command_b);
    assert_eq!(surface_b.runnable, 1);
    assert!(matches!(
        run_control_hooks(
            app_b,
            event.clone(),
            HookPayload::new(&event, "session-b".into(), String::new()),
        )
        .await,
        ControlVerdict::Allow
    ));
    assert_eq!(std::fs::read_to_string(&marker_b).unwrap(), "ran");
}
