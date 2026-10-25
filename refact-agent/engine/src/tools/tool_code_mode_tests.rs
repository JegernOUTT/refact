use std::collections::HashMap;
use std::sync::Arc;

use refact_sandbox::{BwrapProvider, Enforcement, SandboxProvider};
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ChatToolCall, ChatToolFunction};
use crate::chat::types::{ChatSession, ThreadParams};
use crate::code_mode::session::AuthorizedCall;
use crate::global_context::TerminalSecurityMode;
use crate::subchat::ToolsPolicy;
use crate::tools::tool_code_mode::ToolCodeMode;
use crate::tools::tools_description::{Tool, ToolSourceType};
use crate::tools::tools_list::{get_available_tools, get_tools_for_mode};

fn args(script: &str) -> HashMap<String, Value> {
    HashMap::from([("script".to_string(), json!(script))])
}

async fn fixture(
    terminal_mode: TerminalSecurityMode,
) -> (
    AppState,
    Arc<AMutex<AtCommandsContext>>,
    Arc<AMutex<ChatSession>>,
) {
    let mut gcx = crate::global_context::tests::make_test_gcx().await;
    crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(&gcx.config_dir)
        .await
        .unwrap();
    let project = gcx.config_dir.join("workspace");
    std::fs::create_dir_all(&project).unwrap();
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![project];
    Arc::get_mut(&mut gcx)
        .unwrap()
        .terminal_security_config
        .mode = terminal_mode;
    let app = AppState::from_gcx(gcx).await;
    let chat_id = format!("code-mode-{}", uuid::Uuid::new_v4());
    let thread = ThreadParams {
        id: chat_id.clone(),
        model: "test/default".to_string(),
        mode: "agent".to_string(),
        ..Default::default()
    };
    let session = Arc::new(AMutex::new(ChatSession::new(chat_id.clone())));
    session.lock().await.thread = thread;
    app.chat
        .sessions
        .write()
        .await
        .insert(chat_id.clone(), session.clone());
    let ccx = Arc::new(AMutex::new(
        AtCommandsContext::new_from_app(
            app.clone(),
            4096,
            20,
            false,
            Vec::new(),
            chat_id,
            None,
            "test/default".to_string(),
            None,
            None,
        )
        .await,
    ));
    ccx.lock().await.child_tool_ceiling = Some(ToolsPolicy::Only(vec!["wait".into()]));
    (app, ccx, session)
}

fn test_call(name: &str) -> ChatToolCall {
    ChatToolCall {
        id: "confirmation-call".into(),
        index: Some(0),
        function: ChatToolFunction {
            name: name.into(),
            arguments: "{}".into(),
        },
        tool_type: "function".into(),
        extra_content: None,
        started_at_ms: None,
        completed_at_ms: None,
    }
}

fn authorized_calls() -> Vec<AuthorizedCall> {
    vec![
        AuthorizedCall {
            op: "wait",
            tool: "wait",
            arguments: json!({"agent_id": "a"}),
        },
        AuthorizedCall {
            op: "wait",
            tool: "wait",
            arguments: json!({"agent_id": "b"}),
        },
        AuthorizedCall {
            op: "wait",
            tool: "wait",
            arguments: json!({"agent_id": "c"}),
        },
    ]
}

fn tool_message(index: usize, failed: Option<bool>, text: &str) -> ChatMessage {
    ChatMessage {
        tool_call_id: format!("codemode_{index}_test"),
        tool_failed: failed,
        content: ChatContent::SimpleText(text.to_string()),
        ..Default::default()
    }
}

#[tokio::test]
async fn code_mode_is_registered_and_allow_listed() {
    let (app, _, _) = fixture(TerminalSecurityMode::SandboxRequired).await;
    let available = get_available_tools(app.gcx.clone()).await;
    let registered = available
        .iter()
        .filter(|tool| tool.tool_description().name == "code_mode")
        .collect::<Vec<_>>();
    assert_eq!(registered.len(), 1);
    let description = registered[0].tool_description();
    assert!(matches!(
        description.source.source_type,
        ToolSourceType::Builtin
    ));
    assert!(!description.allow_parallel);
    assert_eq!(description.input_schema["required"], json!(["script"]));
    assert_eq!(
        description.input_schema["properties"]["script"]["minLength"],
        1
    );

    let allowed = get_tools_for_mode(app.gcx.clone(), "agent", Some("test/default"))
        .await
        .into_iter()
        .filter(|tool| tool.tool_description().name == "code_mode")
        .count();
    assert_eq!(allowed, 1);
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn default_settings_refuse_before_any_spawn() {
    let (app, ccx, _) = fixture(TerminalSecurityMode::SandboxRequired).await;
    let mut tool = ToolCodeMode {
        config_path: String::new(),
    };
    let error = tool
        .tool_execute(ccx, &"default-off".into(), &args("pass"))
        .await
        .unwrap_err();
    assert!(error.contains("code_mode_enabled"), "{error}");
    assert!(app
        .runtime
        .exec_registry
        .list(Default::default())
        .await
        .is_empty());
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn refusal_happens_before_ceiling_computation() {
    let (app, ccx, _) = fixture(TerminalSecurityMode::ApprovalOnly).await;
    let _settings = crate::tools::settings_guard::SettingsGuard::install(|settings| {
        settings.coordination.code_mode_enabled = true;
    });
    let mut tool = ToolCodeMode {
        config_path: String::new(),
    };
    let error = tool
        .tool_execute(ccx, &"sandbox-refusal".into(), &args("pass"))
        .await
        .unwrap_err();
    assert!(error.contains("never confines"), "{error}");
    assert!(!error.contains("capabilities"), "{error}");
    assert!(app
        .runtime
        .exec_registry
        .list(Default::default())
        .await
        .is_empty());

    let simulated = crate::code_mode::status(
        true,
        TerminalSecurityMode::SandboxRequired,
        refact_sandbox::SandboxStatus {
            provider: "fixture-noop",
            enforcement: Enforcement::Unusable,
        },
    )
    .require_available()
    .unwrap_err();
    assert!(simulated.contains("fixture-noop"), "{simulated}");
    assert!(
        simulated.contains("no usable sandbox provider"),
        "{simulated}"
    );
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn enabled_with_sandbox_dispatches_wait_call() {
    if crate::code_mode::interpreter().is_none() || BwrapProvider.probe() != Enforcement::Full {
        return;
    }
    let (app, ccx, _) = fixture(TerminalSecurityMode::SandboxRequired).await;
    let _settings = crate::tools::settings_guard::SettingsGuard::install(|settings| {
        settings.coordination.code_mode_enabled = true;
    });
    let script = "import json\nprint(json.dumps({'op':'wait','arguments':{'agent_id':'code-mode-no-such-agent','timeout_ms':1}}))\n";
    let mut tool = ToolCodeMode {
        config_path: String::new(),
    };
    let output = tool
        .tool_execute(ccx, &"confined-run".into(), &args(script))
        .await
        .unwrap();
    let crate::call_validation::ContextEnum::ChatMessage(message) = &output.1[0] else {
        panic!("expected tool result")
    };
    let ChatContent::SimpleText(summary) = &message.content else {
        panic!("expected text")
    };
    assert!(
        summary.contains("wait") && summary.contains("err"),
        "{summary}"
    );
    assert!(summary.contains("not found in this chat"), "{summary}");
    assert!(app
        .runtime
        .exec_registry
        .list(Default::default())
        .await
        .is_empty());
}

#[tokio::test]
async fn code_mode_requires_user_confirmation() {
    let (app, _, session) = fixture(TerminalSecurityMode::SandboxRequired).await;
    let thread = session.lock().await.thread.clone();
    let (confirmations, denials) = crate::chat::tools::check_tools_confirmation(
        app,
        &[test_call("code_mode")],
        &[],
        &thread,
        None,
        "agent",
        Some("test/default"),
        &[],
        "",
    )
    .await;
    assert!(denials.is_empty(), "{denials:?}");
    assert_eq!(confirmations.len(), 1);
    assert_eq!(confirmations[0].tool_name, "code_mode");
    assert_eq!(confirmations[0].rule, "mode policy: ask");
}

#[test]
fn summarize_dispatch_never_reports_unknown_or_failed_calls_as_ok() {
    let calls = authorized_calls();
    let all_messages = vec![
        tool_message(0, Some(false), "first"),
        tool_message(1, Some(true), "x".repeat(250).as_str()),
        tool_message(2, Some(false), "third"),
    ];
    let ok = crate::tools::tool_code_mode::summarize_dispatch(&calls, &Ok(all_messages));
    assert!(ok[0].contains("ok"));
    assert!(ok[1].contains("err: "));
    assert!(ok[1].chars().count() < 280);
    assert!(ok[2].contains("ok"));

    let missing = crate::tools::tool_code_mode::summarize_dispatch(
        &calls,
        &Ok(vec![
            tool_message(0, Some(false), "first"),
            tool_message(1, Some(false), "second"),
        ]),
    );
    assert!(missing[2].contains("no result"), "{missing:?}");

    let indexed_error = crate::tools::tool_code_mode::summarize_dispatch(
        &calls,
        &Err("code mode call 1 (wait) failed: x".to_string()),
    );
    assert!(indexed_error[0].contains("ran (output not returned after a later failure)"));
    assert!(indexed_error[1].contains("err: code mode call 1 (wait) failed: x"));
    assert!(indexed_error[2].contains("not run"));
    assert!(indexed_error.iter().all(|line| !line.contains("— ok")));

    let unknown_error = crate::tools::tool_code_mode::summarize_dispatch(
        &calls,
        &Err("something else".to_string()),
    );
    assert_eq!(unknown_error.len(), calls.len() + 1);
    assert!(unknown_error[0].contains("dispatch stopped: something else"));
    assert!(unknown_error[1..]
        .iter()
        .all(|line| line.contains("status unknown")));
    assert!(unknown_error.iter().all(|line| !line.contains("— ok")));
}
