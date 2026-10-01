use std::sync::Arc;

use tokio::sync::Mutex as AMutex;

use crate::app_state::AppState;
use crate::call_validation::{ChatContent, ChatMessage, ChatToolCall, ChatToolFunction};
use crate::chat::permission_decision_hooks::apply_confirmation_permission_hooks;
use crate::chat::types::{ChatSession, PauseReason, SessionState, ToolDecisionItem};
use crate::ext::hooks_runner::clear_hook_cache_for_tests;

async fn fixture(enabled: bool) -> (tempfile::TempDir, AppState, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let config_dir = dir.path().join("config");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        dir.path().join("cache"),
        config_dir.clone(),
    )
    .await;
    Arc::get_mut(&mut gcx).unwrap().hooks_config.enabled = enabled;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.clone()];
    let app = AppState::from_gcx(gcx).await;
    (dir, app, workspace)
}

fn install_hook(config_dir: &std::path::Path, command: &str) {
    std::fs::create_dir_all(config_dir).unwrap();
    std::fs::write(
        config_dir.join("hooks.yaml"),
        format!(
            "hooks:\n  PermissionDecision:\n    - hooks:\n        - type: command\n          command: {command:?}\n"
        ),
    )
    .unwrap();
    clear_hook_cache_for_tests();
}

fn call(id: &str, name: &str, arguments: &str) -> ChatToolCall {
    ChatToolCall {
        id: id.into(),
        index: None,
        function: ChatToolFunction {
            name: name.into(),
            arguments: arguments.into(),
        },
        tool_type: "function".into(),
        extra_content: None,
        started_at_ms: None,
        completed_at_ms: None,
    }
}

async fn paused_session(
    chat_id: &str,
    tool_call: ChatToolCall,
    shell_privacy: bool,
) -> Arc<AMutex<ChatSession>> {
    let mut session = ChatSession::new(chat_id.into());
    session.add_message(ChatMessage {
        role: "assistant".into(),
        tool_calls: Some(vec![tool_call.clone()]),
        ..Default::default()
    });
    if shell_privacy {
        let mut output = ChatMessage {
            role: "tool".into(),
            tool_call_id: tool_call.id.clone(),
            content: ChatContent::SimpleText(
                "Output awaiting user approval — this command read guarded files.".into(),
            ),
            ..Default::default()
        };
        output.extra.insert(
            "privacy_shell".into(),
            serde_json::json!({"ask_pending": true, "local_only_output": "guarded output"}),
        );
        session.add_message(output);
    }
    session.runtime.pause_reasons.push(PauseReason {
        reason_type: "confirmation".into(),
        tool_name: tool_call.function.name.clone(),
        command: tool_call.function.name.clone(),
        rule: if shell_privacy {
            crate::chat::tools::SHELL_PRIVACY_APPROVAL_RULE.into()
        } else {
            "ask".into()
        },
        tool_call_id: tool_call.id,
        integr_config_path: None,
    });
    session.runtime.paused_message_index = Some(0);
    session.set_runtime_state(SessionState::Paused, None);
    Arc::new(AMutex::new(session))
}

fn invocation_count(path: &std::path::Path) -> usize {
    std::fs::read_to_string(path)
        .map(|contents| contents.lines().count())
        .unwrap_or_default()
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn approved_confirmation_denied_by_hook_is_not_executed_and_the_model_sees_why() {
    let (dir, app, workspace) = fixture(true).await;
    let config_dir = app.paths.config_dir.clone();
    let count = dir.path().join("confirmation-hook-count");
    let tool_marker = dir.path().join("tool-ran");
    install_hook(
        &config_dir,
        &format!(
            "echo invoked >> {}; echo policy-block >&2; exit 2",
            shell_quote(&count.display().to_string())
        ),
    );
    let session = paused_session(
        "confirmation-denied",
        call(
            "call-1",
            "shell",
            &serde_json::json!({"command": format!("touch {}", shell_quote(&tool_marker.display().to_string()))}).to_string(),
        ),
        false,
    )
    .await;
    session
        .lock()
        .await
        .user_interrupt_flag
        .store(true, std::sync::atomic::Ordering::SeqCst);

    crate::chat::handle_tool_decisions_guarded(
        app,
        session.clone(),
        &[ToolDecisionItem {
            tool_call_id: "call-1".into(),
            accepted: true,
        }],
        None,
        "test",
    )
    .await;

    let session = session.lock().await;
    assert!(!tool_marker.exists());
    let denial = session
        .messages
        .iter()
        .find(|message| message.role == "tool" && message.tool_call_id == "call-1")
        .unwrap()
        .content
        .content_text_only();
    assert!(denial.starts_with("Tool call denied by PermissionDecision hook:"));
    assert!(denial.contains("policy-block"));
    assert_eq!(invocation_count(&count), 1);
    assert!(workspace.exists());
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn approved_shell_privacy_output_denied_by_hook_stays_withheld() {
    let (dir, app, _) = fixture(true).await;
    let count = dir.path().join("privacy-hook-count");
    install_hook(
        &app.paths.config_dir,
        &format!(
            "echo invoked >> {}; echo policy-block >&2; exit 2",
            shell_quote(&count.display().to_string())
        ),
    );
    let session = paused_session(
        "privacy-denied",
        call("privacy-call", "shell", r#"{"command":"cat secret"}"#),
        true,
    )
    .await;
    session
        .lock()
        .await
        .user_interrupt_flag
        .store(true, std::sync::atomic::Ordering::SeqCst);

    crate::chat::handle_tool_decisions_guarded(
        app,
        session.clone(),
        &[ToolDecisionItem {
            tool_call_id: "privacy-call".into(),
            accepted: true,
        }],
        None,
        "test",
    )
    .await;

    let session = session.lock().await;
    let output = session
        .messages
        .iter()
        .find(|message| message.role == "tool" && message.tool_call_id == "privacy-call")
        .unwrap();
    assert_eq!(
        output.content.content_text_only(),
        crate::privacy::records::SHELL_WITHHELD_MESSAGE
    );
    assert_eq!(output.extra["privacy_shell"]["ask_pending"], false);
    assert_eq!(output.extra["privacy_shell"]["approved"], false);
    assert_eq!(invocation_count(&count), 1);
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn agent_approval_wrapper_confirmation_emits_nothing_at_the_queue_site() {
    let (dir, app, _) = fixture(true).await;
    let count = dir.path().join("wrapper-hook-count");
    install_hook(
        &app.paths.config_dir,
        &format!(
            "echo invoked >> {} ; exit 2",
            shell_quote(&count.display().to_string())
        ),
    );
    let session = paused_session(
        "wrapper-confirmation",
        call("wrapper-call", "agent_approval", r#"{"action":"reject"}"#),
        false,
    )
    .await;

    crate::chat::handle_tool_decisions_guarded(
        app,
        session,
        &[ToolDecisionItem {
            tool_call_id: "wrapper-call".into(),
            accepted: false,
        }],
        None,
        "test",
    )
    .await;

    assert_eq!(invocation_count(&count), 0);
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn rejected_confirmation_with_allow_hook_stays_rejected_with_the_user_wording() {
    let (dir, app, _) = fixture(true).await;
    let count = dir.path().join("reject-hook-count");
    install_hook(
        &app.paths.config_dir,
        &format!(
            "echo invoked >> {}; exit 0",
            shell_quote(&count.display().to_string())
        ),
    );
    let session = paused_session(
        "confirmation-rejected",
        call("call-reject", "shell", r#"{"command":"echo not-run"}"#),
        false,
    )
    .await;

    crate::chat::handle_tool_decisions_guarded(
        app,
        session.clone(),
        &[ToolDecisionItem {
            tool_call_id: "call-reject".into(),
            accepted: false,
        }],
        None,
        "test",
    )
    .await;

    let session = session.lock().await;
    let denial = session
        .messages
        .iter()
        .find(|message| message.role == "tool" && message.tool_call_id == "call-reject")
        .unwrap()
        .content
        .content_text_only();
    assert_eq!(denial, "Tool call denied by user.");
    assert_eq!(invocation_count(&count), 1);
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn hooks_disabled_changes_nothing() {
    let (dir, app, _) = fixture(false).await;
    let count = dir.path().join("disabled-hook-count");
    install_hook(
        &app.paths.config_dir,
        &format!(
            "echo invoked >> {}; exit 2",
            shell_quote(&count.display().to_string())
        ),
    );
    let session = paused_session(
        "confirmation-hooks-disabled",
        call("call-disabled", "shell", r#"{"command":"echo no"}"#),
        false,
    )
    .await;
    let decisions = [ToolDecisionItem {
        tool_call_id: "call-disabled".into(),
        accepted: false,
    }];

    let outcome = apply_confirmation_permission_hooks(&app, &session, &decisions).await;

    assert!(!outcome.decisions[0].accepted);
    assert!(outcome.denial_reasons.is_empty());
    assert_eq!(invocation_count(&count), 0);
}
