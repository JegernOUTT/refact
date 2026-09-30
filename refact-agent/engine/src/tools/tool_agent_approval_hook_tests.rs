use super::ToolAgentApproval;
use crate::agents::approval::tests::{fixture, Fixture};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ContextEnum;
use crate::ext::hooks_runner::clear_hook_cache_for_tests;
use crate::tools::tools_description::Tool;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use tokio::sync::Mutex;
use std::sync::Arc;

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn install_hook(fixture: &Fixture, command: &str, timeout_secs: Option<u64>) {
    let config_dir = fixture.dir.path().join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    let timeout = timeout_secs
        .map(|secs| format!("          timeout: {secs}\n"))
        .unwrap_or_default();
    std::fs::write(
        config_dir.join("hooks.yaml"),
        format!(
            "hooks:\n  PermissionDecision:\n    - hooks:\n        - type: command\n          command: {command:?}\n{timeout}"
        ),
    )
    .unwrap();
    clear_hook_cache_for_tests();
}

async fn parent_context(fixture: &Fixture) -> Arc<Mutex<AtCommandsContext>> {
    Arc::new(Mutex::new(
        AtCommandsContext::new_from_app(
            fixture.app.clone(),
            4096,
            10,
            false,
            vec![],
            "parent".into(),
            None,
            "test/model".into(),
            None,
            None,
        )
        .await,
    ))
}

async fn execute_decision(fixture: &Fixture, pending_id: &str, action: &str) -> Value {
    let args = serde_json::json!({"action": action, "request_id": pending_id});
    let args: HashMap<String, Value> = args.as_object().unwrap().clone().into_iter().collect();
    let ccx = parent_context(fixture).await;
    if action == "approve" {
        fixture
            .app
            .agent_approvals
            .grant_user_once(
                "parent",
                pending_id,
                fixture.generation,
                "approval-call",
                crate::agents::approval::now_ms(),
            )
            .unwrap();
    }
    let mut tool = ToolAgentApproval {
        config_path: "test".into(),
    };
    let (_, contexts) = tool
        .tool_execute(ccx, &"approval-call".into(), &args)
        .await
        .unwrap();
    let ContextEnum::ChatMessage(message) = &contexts[0] else {
        panic!("approval response was not a chat message");
    };
    serde_json::from_str(&message.content.content_text_only()).unwrap()
}

fn invocation_count(path: &std::path::Path) -> usize {
    std::fs::read_to_string(path)
        .map(|contents| contents.lines().count())
        .unwrap_or_default()
}

async fn wait_for_request<F: Future<Output = Result<bool, String>>>(
    fixture: &Fixture,
    future: Pin<&mut F>,
) -> refact_work_core::PendingApproval {
    fixture.waiting(future).await
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn approve_plus_deny_hook_rejects_and_the_child_sees_the_hook_reason() {
    let fixture = fixture().await;
    let counter = fixture.dir.path().join("deny-count");
    install_hook(
        &fixture,
        &format!(
            "echo invoked >> {}; echo blocked >&2; exit 2",
            shell_quote(&counter.display().to_string())
        ),
        None,
    );
    let call = fixture.call("shell");
    let mut child_request = Box::pin(crate::agents::approval::request(
        &fixture.app,
        &fixture.ccx,
        &call,
        "needs decision",
    ));
    let pending = wait_for_request(&fixture, child_request.as_mut()).await;

    let response = execute_decision(&fixture, &pending.id, "approve").await;
    assert_eq!(
        fixture
            .app
            .agent_approvals
            .wait_now(&pending.id, crate::agents::approval::now_ms()),
        Ok(refact_work_core::ApprovalDecision::Rejected)
    );
    let child_error = child_request.await.unwrap_err();
    assert!(
        child_error.contains("PermissionDecision hook"),
        "{child_error}"
    );
    assert_eq!(invocation_count(&counter), 1);
    assert_eq!(response["hook_overrode_approval"], true);
    assert!(response["denial_reason"]
        .as_str()
        .unwrap()
        .contains("PermissionDecision hook"));
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn reject_plus_allow_hook_stays_rejected_and_the_hook_ran_once() {
    let fixture = fixture().await;
    let counter = fixture.dir.path().join("reject-count");
    install_hook(
        &fixture,
        &format!(
            "echo invoked >> {}; exit 0",
            shell_quote(&counter.display().to_string())
        ),
        None,
    );
    let call = fixture.call("shell");
    let mut child_request = Box::pin(crate::agents::approval::request(
        &fixture.app,
        &fixture.ccx,
        &call,
        "needs decision",
    ));
    let pending = wait_for_request(&fixture, child_request.as_mut()).await;

    let response = execute_decision(&fixture, &pending.id, "reject").await;
    assert_eq!(response["decision"], "rejected");
    assert_eq!(
        fixture
            .app
            .agent_approvals
            .wait_now(&pending.id, crate::agents::approval::now_ms()),
        Ok(refact_work_core::ApprovalDecision::Rejected)
    );
    assert!(!child_request.await.unwrap());
    assert_eq!(invocation_count(&counter), 1);
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn approve_plus_allow_hook_approves_and_the_hook_ran_once() {
    let fixture = fixture().await;
    let counter = fixture.dir.path().join("allow-count");
    install_hook(
        &fixture,
        &format!(
            "echo invoked >> {}; exit 0",
            shell_quote(&counter.display().to_string())
        ),
        None,
    );
    let call = fixture.call("shell");
    let mut child_request = Box::pin(crate::agents::approval::request(
        &fixture.app,
        &fixture.ccx,
        &call,
        "needs decision",
    ));
    let pending = wait_for_request(&fixture, child_request.as_mut()).await;

    let response = execute_decision(&fixture, &pending.id, "approve").await;
    assert_eq!(response["decision"], "approved");
    assert!(child_request.await.unwrap());
    assert_eq!(invocation_count(&counter), 1);
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn approve_plus_hook_timeout_rejects() {
    let fixture = fixture().await;
    let counter = fixture.dir.path().join("timeout-count");
    install_hook(
        &fixture,
        &format!(
            "echo invoked >> {}; sleep 5",
            shell_quote(&counter.display().to_string())
        ),
        Some(1),
    );
    let call = fixture.call("shell");
    let mut child_request = Box::pin(crate::agents::approval::request(
        &fixture.app,
        &fixture.ccx,
        &call,
        "needs decision",
    ));
    let pending = wait_for_request(&fixture, child_request.as_mut()).await;

    let response = execute_decision(&fixture, &pending.id, "approve").await;
    assert_eq!(response["decision"], "rejected");
    assert!(response["denial_reason"]
        .as_str()
        .unwrap()
        .contains("PermissionDecision hook"));
    assert!(child_request
        .await
        .unwrap_err()
        .contains("PermissionDecision hook"));
    assert_eq!(invocation_count(&counter), 1);
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn hooks_disabled_spawns_no_hook_and_changes_nothing() {
    let fixture = crate::agents::approval::tests::fixture_with_hooks(false).await;
    let counter = fixture.dir.path().join("disabled-count");
    install_hook(
        &fixture,
        &format!(
            "echo invoked >> {}; exit 2",
            shell_quote(&counter.display().to_string())
        ),
        None,
    );
    let call = fixture.call("shell");
    let mut child_request = Box::pin(crate::agents::approval::request(
        &fixture.app,
        &fixture.ccx,
        &call,
        "needs decision",
    ));
    let pending = wait_for_request(&fixture, child_request.as_mut()).await;

    let response = execute_decision(&fixture, &pending.id, "approve").await;
    assert_eq!(response["decision"], "approved");
    assert!(response.get("hook_overrode_approval").is_none());
    assert!(response.get("denial_reason").is_none());
    assert!(child_request.await.unwrap());
    assert_eq!(invocation_count(&counter), 0);
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn reject_hook_receives_reject_decision_payload() {
    let fixture = fixture().await;
    let counter = fixture.dir.path().join("payload-count");
    let payload = fixture.dir.path().join("payload.json");
    install_hook(
        &fixture,
        &format!(
            "cat > {}; echo invoked >> {}; exit 0",
            shell_quote(&payload.display().to_string()),
            shell_quote(&counter.display().to_string())
        ),
        None,
    );
    let call = fixture.call("shell");
    let mut child_request = Box::pin(crate::agents::approval::request(
        &fixture.app,
        &fixture.ccx,
        &call,
        "needs decision",
    ));
    let pending = wait_for_request(&fixture, child_request.as_mut()).await;

    execute_decision(&fixture, &pending.id, "reject").await;
    assert!(!child_request.await.unwrap());
    let payload: Value = serde_json::from_slice(&std::fs::read(payload).unwrap()).unwrap();
    assert_eq!(payload["decision"], "reject");
    assert_eq!(payload["source"], "child_approval");
    assert_eq!(invocation_count(&counter), 1);
}
