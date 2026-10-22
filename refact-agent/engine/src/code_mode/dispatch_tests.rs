use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ChatMessage;
use crate::chat::types::ThreadParams;
use crate::code_mode::session::AuthorizedCall;
use crate::subchat::ToolsPolicy;

use super::dispatch::dispatch_authorized_calls;

fn wait_call(arguments: Value) -> AuthorizedCall {
    AuthorizedCall {
        op: "wait",
        tool: "agent_wait",
        arguments,
    }
}

async fn fixture() -> (
    crate::app_state::AppState,
    Arc<AMutex<AtCommandsContext>>,
    ThreadParams,
    tempfile::TempDir,
) {
    let temp = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        temp.path().to_path_buf(),
        config.path().to_path_buf(),
    )
    .await;
    let workspace = tempfile::tempdir().unwrap();
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.path().into()];
    let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    let thread = ThreadParams {
        id: "code-mode-dispatch".into(),
        model: "model".into(),
        mode: "agent".into(),
        ..Default::default()
    };
    let parent = Arc::new(AMutex::new(crate::chat::types::ChatSession::new(
        thread.id.clone(),
    )));
    parent.lock().await.thread = thread.clone();
    app.chat
        .sessions
        .write()
        .await
        .insert(thread.id.clone(), parent);
    let ccx = Arc::new(AMutex::new(
        AtCommandsContext::new_from_app(
            app.clone(),
            4096,
            20,
            false,
            vec![],
            thread.id.clone(),
            Some(thread.id.clone()),
            thread.model.clone(),
            None,
            None,
        )
        .await,
    ));
    ccx.lock().await.child_tool_ceiling = Some(ToolsPolicy::Only(vec!["agent_wait".into()]));
    (app, ccx, thread, workspace)
}

async fn dispatch(
    app: crate::app_state::AppState,
    ccx: Arc<AMutex<AtCommandsContext>>,
    thread: &ThreadParams,
    calls: &[AuthorizedCall],
) -> Result<Vec<ChatMessage>, String> {
    dispatch_authorized_calls(
        app,
        ccx,
        thread,
        &thread.mode,
        Some(&thread.model),
        &thread.id,
        calls,
    )
    .await
}

async fn write_pretool_hook(app: &crate::app_state::AppState, command: &str) {
    tokio::fs::create_dir_all(&app.gcx.config_dir)
        .await
        .unwrap();
    tokio::fs::write(
        app.gcx.config_dir.join("hooks.yaml"),
        format!(
            "hooks:\n  PreToolUse:\n    - hooks:\n        - type: command\n          command: {}\n",
            serde_json::to_string(command).unwrap()
        ),
    )
    .await
    .unwrap();
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
}

#[tokio::test]
async fn authorized_wait_call_reaches_the_real_agent_wait_tool() {
    let (app, ccx, thread, _workspace) = fixture().await;

    let result = dispatch(app, ccx, &thread, &[wait_call(json!({"agent_id": "nope"}))]).await;

    let failure = result.unwrap_err();
    assert!(failure.contains("agent_wait"), "{failure}");
    assert!(
        failure.contains("Background agent 'nope' not found in this chat."),
        "{failure}"
    );
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn pretooluse_deny_blocks_the_call() {
    let (app, ccx, thread, workspace) = fixture().await;
    let marker = workspace.path().join("wait-ran");
    write_pretool_hook(
        &app,
        &format!(
            "echo ran >> '{}'; echo policy-block >&2; exit 2",
            marker.display()
        ),
    )
    .await;

    let error = dispatch(app, ccx, &thread, &[wait_call(json!({"agent_id": "nope"}))])
        .await
        .unwrap_err();

    assert!(error.contains("Blocked by hook"), "{error}");
    assert!(error.contains("policy-block"), "{error}");
    assert!(marker.exists(), "the denying hook did not run");
    assert!(
        !error.contains("not found"),
        "the real tool ran despite a denying hook"
    );
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn rewrite_adding_undeclared_argument_is_refused() {
    let (app, ccx, thread, _workspace) = fixture().await;
    write_pretool_hook(
        &app,
        "printf '%s' '{\"decision\":\"rewrite\",\"tool_input\":{\"agent_id\":\"nope\",\"extra\":true}}'",
    )
    .await;

    let error = dispatch(app, ccx, &thread, &[wait_call(json!({"agent_id": "nope"}))])
        .await
        .unwrap_err();

    assert!(error.contains("rewrite refused"), "{error}");
    assert!(error.contains("has no argument `extra`"), "{error}");
}

#[tokio::test]
async fn call_outside_ceiling_is_refused_by_the_dispatcher() {
    let (app, ccx, thread, _workspace) = fixture().await;
    ccx.lock().await.child_tool_ceiling = Some(ToolsPolicy::None);

    let error = dispatch(app, ccx, &thread, &[wait_call(json!({"agent_id": "nope"}))])
        .await
        .unwrap_err();

    assert!(error.contains("code mode call 0 (agent_wait) is outside the caller's authority"));
    assert!(!error.contains("Tool is outside accepted child authority"));
}

#[tokio::test]
async fn confirmation_required_call_fails_closed() {
    let (app, ccx, thread, _workspace) = fixture().await;
    let mode_dir = app.gcx.config_dir.join("modes");
    tokio::fs::create_dir_all(&mode_dir).await.unwrap();
    tokio::fs::write(
        mode_dir.join("agent.yaml"),
        "schema_version: 1\nid: agent\ntitle: Confirmation test\ndescription: test\nspecific: false\nprompt: ''\nplan_template: ''\ntools:\n  - agent_wait\nallow_integrations: false\nallow_mcp: false\nallow_subagents: false\ntool_confirm:\n  rules:\n    - match: agent_wait\n      action: ask\n",
    )
    .await
    .unwrap();
    crate::yaml_configs::customization_registry::invalidate_all_registry_caches(app.gcx.clone())
        .await;

    let error = dispatch(app, ccx, &thread, &[wait_call(json!({"agent_id": "nope"}))])
        .await
        .unwrap_err();

    assert!(error.contains("code mode call 0 (agent_wait) requires confirmation"));
    assert!(error.contains("code mode cannot pause for approval"));
}

#[test]
fn rewritten_arguments_reject_host_operation_extras() {
    let operation = crate::code_mode::hosts::operation("wait").unwrap();
    assert!(operation
        .validate(&json!({"agent_id": "nope", "extra": true}))
        .unwrap_err()
        .contains("has no argument `extra`"));
}
