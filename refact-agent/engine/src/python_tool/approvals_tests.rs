use super::*;
use crate::app_state::AppState;
use crate::chat::types::ChatSession;
use refact_chat_api::PythonApprovalDecisionKind as Decision;
use std::sync::Arc;
use tokio::sync::Mutex;

async fn fixture() -> AppState {
    let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
    app.chat.sessions.write().await.insert(
        "chat".into(),
        Arc::new(Mutex::new(ChatSession::new("chat".into()))),
    );
    app
}

fn ask(editing_tool: bool) -> ApprovalAsk {
    ApprovalAsk {
        chat_id: "chat".into(),
        tool_call_id: "python".into(),
        run_id: "run".into(),
        call_id: "nested".into(),
        tool_name: "cat".into(),
        args_preview: "{}".into(),
        command: None,
        rule: "confirm".into(),
        editing_tool,
        engine: None,
    }
}

async fn pending(app: &AppState) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let session = app.chat.sessions.read().await["chat"].clone();
            if let Some(request) = session.lock().await.runtime.python_approvals.first() {
                return request.request_id.clone();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

async fn decision_case(decision: Decision, expected: ApprovalOutcome, editing: bool) {
    let app = fixture().await;
    let run = app.python_tool.register_run("chat", "python", "run");
    let waiter = {
        let app = app.clone();
        tokio::spawn(async move { app.python_tool.approvals.request(&app, ask(editing)).await })
    };
    let id = pending(&app).await;
    app.python_tool
        .approvals
        .decide(&app, "chat", &id, decision)
        .await
        .unwrap();
    assert_eq!(waiter.await.unwrap(), expected);
    assert_eq!(
        run.cancellation_token().is_cancelled(),
        decision == Decision::Stop
    );
    assert!(app.chat.sessions.read().await["chat"]
        .lock()
        .await
        .runtime
        .python_approvals
        .is_empty());
    if decision == Decision::AllowForChat {
        assert!(
            app.chat.sessions.read().await["chat"]
                .lock()
                .await
                .thread
                .auto_approve_editing_tools
        );
    }
}

#[tokio::test]
async fn allow_once_resumes_waiter() {
    decision_case(
        Decision::AllowOnce,
        ApprovalOutcome::Allowed {
            for_chat: false,
        },
        false,
    )
    .await;
}
#[tokio::test]
async fn deny_resolves_denied() {
    decision_case(Decision::Deny, ApprovalOutcome::Denied, false).await;
}
#[tokio::test]
async fn stop_resolves_and_cancels_run() {
    decision_case(Decision::Stop, ApprovalOutcome::Stopped, false).await;
}
#[tokio::test]
async fn allow_for_chat_sets_auto_approve_editing_tools() {
    decision_case(
        Decision::AllowForChat,
        ApprovalOutcome::Allowed {
            for_chat: true,
        },
        true,
    )
    .await;
}
#[tokio::test]
async fn allow_for_chat_rejected_for_non_editing_tool() {
    let app = fixture().await;
    let _run = app.python_tool.register_run("chat", "python", "run");
    let waiter = {
        let app = app.clone();
        tokio::spawn(async move { app.python_tool.approvals.request(&app, ask(false)).await })
    };
    let id = pending(&app).await;
    assert!(app
        .python_tool
        .approvals
        .decide(&app, "chat", &id, Decision::AllowForChat)
        .await
        .is_err());
    assert!(!waiter.is_finished());
    app.python_tool
        .approvals
        .decide(&app, "chat", &id, Decision::Deny)
        .await
        .unwrap();
    assert_eq!(waiter.await.unwrap(), ApprovalOutcome::Denied);
}
#[tokio::test]
async fn pending_approvals_appear_in_snapshot_runtime() {
    let app = fixture().await;
    let _run = app.python_tool.register_run("chat", "python", "run");
    let waiter = {
        let app = app.clone();
        tokio::spawn(async move { app.python_tool.approvals.request(&app, ask(false)).await })
    };
    let id = pending(&app).await;
    let snapshot = app.chat.sessions.read().await["chat"]
        .lock()
        .await
        .snapshot();
    assert_eq!(
        serde_json::to_value(snapshot).unwrap()["runtime"]["python_approvals"][0]["request_id"],
        id
    );
    app.python_tool
        .approvals
        .decide(&app, "chat", &id, Decision::Deny)
        .await
        .unwrap();
    waiter.await.unwrap();
}
#[tokio::test]
async fn cancel_run_resolves_pending_as_cancelled() {
    let app = fixture().await;
    let run = app.python_tool.register_run("chat", "python", "run");
    let waiter = {
        let app = app.clone();
        tokio::spawn(async move { app.python_tool.approvals.request(&app, ask(false)).await })
    };
    pending(&app).await;
    drop(run);
    assert_eq!(waiter.await.unwrap(), ApprovalOutcome::Cancelled);
    assert!(app.chat.sessions.read().await["chat"]
        .lock()
        .await
        .runtime
        .python_approvals
        .is_empty());
}
#[tokio::test]
async fn unknown_request_and_unknown_run_report_errors() {
    let app = fixture().await;
    assert!(app.python_tool.stop_run("chat", "python").is_err());
    assert!(app
        .python_tool
        .approvals
        .decide(&app, "chat", "missing", Decision::Deny)
        .await
        .is_err());
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn permission_decision_hook_deny_overrides_allow() {
    let workspace = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        workspace.path().join("cache"),
        config.path().into(),
    )
    .await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.path().into()];
    std::fs::write(config.path().join("hooks.yaml"), "hooks:\n  PermissionDecision:\n    - hooks:\n        - type: command\n          command: 'exit 2'\n").unwrap();
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
    let app = AppState::from_gcx(gcx).await;
    app.chat.sessions.write().await.insert(
        "chat".into(),
        Arc::new(Mutex::new(ChatSession::new("chat".into()))),
    );
    let _run = app.python_tool.register_run("chat", "python", "run");
    let waiter = {
        let app = app.clone();
        tokio::spawn(async move { app.python_tool.approvals.request(&app, ask(true)).await })
    };
    let id = pending(&app).await;
    app.python_tool
        .approvals
        .decide(&app, "chat", &id, Decision::AllowForChat)
        .await
        .unwrap();
    assert_eq!(waiter.await.unwrap(), ApprovalOutcome::Denied);
    assert!(
        !app.chat.sessions.read().await["chat"]
            .lock()
            .await
            .thread
            .auto_approve_editing_tools
    );
}

async fn pending_ids(app: &AppState, count: usize) -> Vec<(String, bool)> {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let session = app.chat.sessions.read().await["chat"].clone();
            let list: Vec<_> = session
                .lock()
                .await
                .runtime
                .python_approvals
                .iter()
                .map(|r| (r.request_id.clone(), r.editing_tool))
                .collect();
            if list.len() >= count {
                return list;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

fn spawn_ask(app: &AppState, request: ApprovalAsk) -> tokio::task::JoinHandle<ApprovalOutcome> {
    let app = app.clone();
    tokio::spawn(async move { app.python_tool.approvals.request(&app, request).await })
}

#[tokio::test]
async fn allow_for_run_rejected_for_non_editing_tool() {
    let app = fixture().await;
    let _run = app.python_tool.register_run("chat", "python", "run");
    let waiter = spawn_ask(&app, ask(false));
    let id = pending(&app).await;
    assert!(app
        .python_tool
        .approvals
        .decide(&app, "chat", &id, Decision::AllowForRun)
        .await
        .is_err());
    assert!(!waiter.is_finished());
    assert!(!app.python_tool.approvals.run_permitted("chat", "run"));
    app.python_tool
        .approvals
        .decide(&app, "chat", &id, Decision::Deny)
        .await
        .unwrap();
    assert_eq!(waiter.await.unwrap(), ApprovalOutcome::Denied);
}

#[tokio::test]
async fn allow_for_run_resolves_pending_edits_and_skips_later_edits_of_the_run() {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let app = fixture().await;
        let _run = app.python_tool.register_run("chat", "python", "run");
        let session = app.chat.sessions.read().await["chat"].clone();
        let mut events = session.lock().await.event_tx.subscribe();
        let edit_a = spawn_ask(&app, ask(true));
        let edit_b = spawn_ask(&app, ask(true));
        let read = spawn_ask(&app, ask(false));
        let ids = pending_ids(&app, 3).await;
        let first_edit = ids.iter().find(|(_, editing)| *editing).unwrap().0.clone();
        let read_id = ids.iter().find(|(_, editing)| !*editing).unwrap().0.clone();
        app.python_tool
            .approvals
            .decide(&app, "chat", &first_edit, Decision::AllowForRun)
            .await
            .unwrap();
        assert_eq!(edit_a.await.unwrap(), ApprovalOutcome::AllowedForRun);
        assert_eq!(edit_b.await.unwrap(), ApprovalOutcome::AllowedForRun);
        assert!(!read.is_finished());
        let left: Vec<_> = session
            .lock()
            .await
            .runtime
            .python_approvals
            .iter()
            .map(|r| r.request_id.clone())
            .collect();
        assert_eq!(left, vec![read_id.clone()]);
        let mut wire = String::new();
        while let Ok(event) = events.try_recv() {
            wire.push_str(&event.to_string());
        }
        assert_eq!(
            wire.matches("\"decision\":\"allow_for_run\"").count(),
            2,
            "{wire}"
        );
        assert!(!session.lock().await.thread.auto_approve_editing_tools);
        assert_eq!(
            app.python_tool.approvals.request(&app, ask(true)).await,
            ApprovalOutcome::AllowedForRun
        );
        assert_eq!(session.lock().await.runtime.python_approvals.len(), 1);
        let second_read = spawn_ask(&app, ask(false));
        assert_eq!(pending_ids(&app, 2).await.len(), 2);
        for (id, _) in pending_ids(&app, 2).await {
            app.python_tool
                .approvals
                .decide(&app, "chat", &id, Decision::Deny)
                .await
                .unwrap();
        }
        assert_eq!(read.await.unwrap(), ApprovalOutcome::Denied);
        assert_eq!(second_read.await.unwrap(), ApprovalOutcome::Denied);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn allow_for_run_permit_ends_with_the_run() {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let app = fixture().await;
        let run = app.python_tool.register_run("chat", "python", "run");
        let waiter = spawn_ask(&app, ask(true));
        let id = pending(&app).await;
        app.python_tool
            .approvals
            .decide(&app, "chat", &id, Decision::AllowForRun)
            .await
            .unwrap();
        assert_eq!(waiter.await.unwrap(), ApprovalOutcome::AllowedForRun);
        assert!(app.python_tool.approvals.run_permitted("chat", "run"));
        app.python_tool.approvals.cancel_run_requests("chat", "run");
        drop(run);
        assert!(!app.python_tool.approvals.run_permitted("chat", "run"));
        let _next = app.python_tool.register_run("chat", "python", "next");
        let mut next_ask = ask(true);
        next_ask.run_id = "next".into();
        let waiter = spawn_ask(&app, next_ask);
        let id = pending(&app).await;
        app.python_tool
            .approvals
            .decide(&app, "chat", &id, Decision::Deny)
            .await
            .unwrap();
        assert_eq!(waiter.await.unwrap(), ApprovalOutcome::Denied);
    })
    .await
    .unwrap();
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn permission_decision_hook_deny_overrides_allow_for_run() {
    let workspace = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        workspace.path().join("cache"),
        config.path().into(),
    )
    .await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.path().into()];
    std::fs::write(config.path().join("hooks.yaml"), "hooks:\n  PermissionDecision:\n    - hooks:\n        - type: command\n          command: 'exit 2'\n").unwrap();
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
    let app = AppState::from_gcx(gcx).await;
    app.chat.sessions.write().await.insert(
        "chat".into(),
        Arc::new(Mutex::new(ChatSession::new("chat".into()))),
    );
    let _run = app.python_tool.register_run("chat", "python", "run");
    let waiter = spawn_ask(&app, ask(true));
    let id = pending(&app).await;
    app.python_tool
        .approvals
        .decide(&app, "chat", &id, Decision::AllowForRun)
        .await
        .unwrap();
    assert_eq!(waiter.await.unwrap(), ApprovalOutcome::Denied);
    assert!(!app.python_tool.approvals.run_permitted("chat", "run"));
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
}

#[tokio::test]
async fn decision_command_handled_while_generation_active() {
    let app = fixture().await;
    let session = app.chat.sessions.read().await["chat"].clone();
    session
        .lock()
        .await
        .set_runtime_state(crate::chat::types::SessionState::Generating, None);
    let _run = app.python_tool.register_run("chat", "python", "run");
    let waiter = {
        let app = app.clone();
        tokio::spawn(async move { app.python_tool.approvals.request(&app, ask(false)).await })
    };
    let id = pending(&app).await;
    let request = refact_chat_api::CommandRequest {
        native_approval_guard: None,
        supervision_guard: None,
        client_request_id: "decide".into(),
        priority: false,
        command: refact_chat_api::ChatCommand::PythonApprovalDecision {
            request_id: id,
            decision: Decision::AllowOnce,
        },
    };
    let response = crate::chat::handle_v1_chat_command(
        axum::extract::State(app.clone()),
        axum::extract::Path("chat".into()),
        serde_json::to_vec(&request).unwrap().into(),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert_eq!(
        waiter.await.unwrap(),
        ApprovalOutcome::Allowed {
            for_chat: false
        }
    );
    assert!(session.lock().await.command_queue.is_empty());
    assert_eq!(
        session.lock().await.runtime.state,
        crate::chat::types::SessionState::Generating
    );
}

#[tokio::test]
async fn context_free_child_request_is_refused() {
    let fixture = crate::agents::approval::tests::fixture().await;
    let mut child_ask = ask(false);
    child_ask.chat_id = fixture.ccx.lock().await.chat_id.clone();
    assert_eq!(
        fixture
            .app
            .python_tool
            .approvals
            .request(&fixture.app, child_ask)
            .await,
        ApprovalOutcome::Denied
    );
}

#[tokio::test]
async fn replaced_run_guard_does_not_unregister_new_run() {
    let runtime = PythonToolRuntime::default();
    let old = runtime.register_run("chat", "python", "old");
    let current = runtime.register_run("chat", "python", "new");
    assert!(old.cancellation_token().is_cancelled());
    drop(old);
    runtime.stop_run("chat", "python").unwrap();
    assert!(current.cancellation_token().is_cancelled());
    drop(current);
    assert!(runtime.stop_run("chat", "python").is_err());
}

#[tokio::test]
async fn cancel_run_requests_resolves_without_dropping_guard() {
    let app = fixture().await;
    let _run = app.python_tool.register_run("chat", "python", "run");
    let waiter = {
        let app = app.clone();
        tokio::spawn(async move { app.python_tool.approvals.request(&app, ask(false)).await })
    };
    pending(&app).await;
    app.python_tool.approvals.cancel_run_requests("chat", "run");
    assert_eq!(waiter.await.unwrap(), ApprovalOutcome::Cancelled);
    assert!(app.chat.sessions.read().await["chat"]
        .lock()
        .await
        .runtime
        .python_approvals
        .is_empty());
}

#[tokio::test]
async fn cancelled_request_resolves_as_stop_on_wire() {
    let app = fixture().await;
    let _run = app.python_tool.register_run("chat", "python", "run");
    let session = app.chat.sessions.read().await["chat"].clone();
    let mut events = session.lock().await.event_tx.subscribe();
    let task = {
        let app = app.clone();
        tokio::spawn(async move { app.python_tool.approvals.request(&app, ask(false)).await })
    };
    let _ = pending(&app).await;
    app.python_tool.approvals.cancel_run_requests("chat", "run");
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap(),
        ApprovalOutcome::Cancelled
    );
    let mut wire = String::new();
    while let Ok(event) = events.try_recv() {
        wire.push_str(&event.to_string());
    }
    assert!(wire.contains("\"decision\":\"stop\""), "{wire}");
}

#[tokio::test]
async fn child_chat_request_routes_through_child_approval_flow() {
    let fixture = crate::agents::approval::tests::fixture().await;
    let call = fixture.call("shell");
    let chat = fixture.ccx.lock().await.chat_id.clone();
    let _run = fixture.app.python_tool.register_run(&chat, "outer", "run");
    let request = ApprovalAsk {
        chat_id: chat.clone(),
        tool_call_id: "outer".into(),
        run_id: "run".into(),
        call_id: call.id.clone(),
        tool_name: call.function.name.clone(),
        args_preview: call.function.arguments.clone(),
        command: None,
        rule: "needs decision".into(),
        editing_tool: false,
        engine: None,
    };
    let mut future = Box::pin(async {
        Ok::<bool, String>(
            fixture
                .app
                .python_tool
                .approvals
                .request_with_context(&fixture.app, request, &fixture.ccx, &call)
                .await
                == ApprovalOutcome::Allowed {
                    for_chat: false,
                },
        )
    });
    let pending = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        fixture.waiting(future.as_mut()),
    )
    .await
    .unwrap();
    fixture
        .app
        .agent_approvals
        .decide(
            "parent",
            fixture.generation,
            &pending.id,
            true,
            crate::agents::approval::now_ms(),
        )
        .unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(10), future)
            .await
            .unwrap()
            .unwrap()
    );
    assert!(fixture.app.chat.sessions.read().await[&chat]
        .lock()
        .await
        .runtime
        .python_approvals
        .is_empty());
}

#[tokio::test]
async fn wire_preview_is_bounded_but_broker_retains_full_arguments() {
    let app = fixture().await;
    let _run = app.python_tool.register_run("chat", "python", "run");
    let mut request = ask(false);
    request.args_preview = serde_json::json!({"command":"é".repeat(5000)}).to_string();
    request.command = Some("x".repeat(20000));
    let full = request.args_preview.clone();
    let task = {
        let app = app.clone();
        tokio::spawn(async move { app.python_tool.approvals.request(&app, request).await })
    };
    let id = pending(&app).await;
    let session = app.chat.sessions.read().await["chat"].clone();
    let wire = session.lock().await.runtime.python_approvals[0].clone();
    assert!(wire.args_preview.chars().count() <= 2000);
    let preview: serde_json::Value = serde_json::from_str(&wire.args_preview).unwrap();
    assert_eq!(
        preview["command"],
        format!("{}… (+4600 chars)", "é".repeat(400))
    );
    assert_eq!(wire.command.unwrap().chars().count(), 16000);
    assert_eq!(
        app.python_tool.approvals.pending.lock().unwrap()[&id].full_args,
        full
    );
    app.python_tool
        .approvals
        .decide(&app, "chat", &id, Decision::Deny)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
}

async fn approved_child_diagnostics(app: &AppState) -> String {
    let mut diagnostics = Vec::new();
    let inspection = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        let session = app
            .chat
            .sessions
            .read()
            .await
            .get("subchat-approval-test")
            .cloned();
        if let Some(session) = session {
            let session = session.lock().await;
            diagnostics.push(format!(
                "session={:?} pending_python_approvals={} python_runs={}",
                session.runtime.state,
                session.runtime.python_approvals.len(),
                session.runtime.python_runs.len()
            ));
            for run in session.runtime.python_runs.iter().take(4) {
                diagnostics.push(format!(
                    "run={} status={:?} calls={}",
                    run.run_id,
                    run.status,
                    run.calls.len()
                ));
                for call in run.calls.iter().take(4) {
                    diagnostics.push(format!(
                        "call={} status={:?} approval={:?} duration_ms={:?}",
                        call.call_id, call.status, call.approval, call.duration_ms
                    ));
                }
            }
        } else {
            diagnostics.push("session=missing".into());
        }
        diagnostics.push(format!(
            "pending_parent_approvals={}",
            app.agent_approvals
                .list_for_parent("parent", crate::agents::approval::now_ms())
                .len()
        ));
        let registry = &app.runtime.exec_registry;
        let snapshots = registry
            .list(crate::exec::ExecProcessFilter {
                chat_id: Some("subchat-approval-test".into()),
                ..Default::default()
            })
            .await;
        diagnostics.push(format!("owned_processes={}", snapshots.len()));
        for snapshot in snapshots.iter().take(4) {
            let id = &snapshot.meta.process_id;
            let exit = match &snapshot.status {
                crate::exec::ExecStatus::Exited {
                    exit_code,
                } => *exit_code,
                crate::exec::ExecStatus::SandboxLauncherFailed {
                    exit_code,
                } => Some(*exit_code),
                _ => None,
            };
            diagnostics.push(format!(
                "process={id} status={:?} exit={exit:?}",
                snapshot.status.kind()
            ));
            let raw = registry.read_raw_capture(id).await;
            diagnostics.push(format!(
                "raw_bytes={:?} eof=unavailable",
                raw.map(|raw| (raw.stdout_captured_bytes, raw.stderr_captured_bytes))
            ));
        }
    })
    .await;
    format!(
        "diagnostics_complete={} {diagnostics:?}",
        inspection.is_ok()
    )
}

#[tokio::test]
async fn foreground_child_script_with_nested_approval_completes() {
    use crate::tools::tools_description::Tool;
    let mut fixture = crate::agents::approval::tests::fixture().await;
    fixture.app.python_tool = Arc::new(PythonToolRuntime {
        registry: super::registry::KernelRegistry::new(super::registry::WorkerBackend::InThread),
        ..Default::default()
    });
    fixture.ccx.lock().await.app = fixture.app.clone();
    let diagnostic_app = fixture.app.clone();
    let code="await shell(command='echo approved', description='Nested child approval', needs_confirmation=True)";
    let mut future = Box::pin(async {
        let result = crate::tools::tool_python::ToolPython::default()
            .tool_execute(
                fixture.ccx.clone(),
                &"python-child".into(),
                &std::collections::HashMap::from([("code".into(), serde_json::json!(code))]),
            )
            .await?;
        Ok::<bool,String>(result.1.iter().all(|m| !matches!(m,crate::call_validation::ContextEnum::ChatMessage(m) if m.tool_failed == Some(true))))
    });
    let pending = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        fixture.waiting(future.as_mut()),
    )
    .await
    .unwrap();
    fixture
        .app
        .agent_approvals
        .decide(
            "parent",
            fixture.generation,
            &pending.id,
            true,
            crate::agents::approval::now_ms(),
        )
        .unwrap();
    let result = match tokio::time::timeout(std::time::Duration::from_secs(20), future).await {
        Ok(result) => result,
        Err(error) => {
            let diagnostics = approved_child_diagnostics(&diagnostic_app).await;
            panic!("approved nested call timed out: {error}; {diagnostics}");
        }
    };
    assert!(result.unwrap());
}

#[tokio::test]
async fn approval_has_no_command_when_policy_repeats_tool_name() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let app = fixture().await;
        let _run = app.python_tool.register_run("chat", "python", "run");
        let mut request = ask(false);
        request.command = Some(request.tool_name.clone());
        let task = {
            let app = app.clone();
            tokio::spawn(async move { app.python_tool.approvals.request(&app, request).await })
        };
        let id = pending(&app).await;
        assert!(app.chat.sessions.read().await["chat"]
            .lock()
            .await
            .runtime
            .python_approvals[0]
            .command
            .is_none());
        app.python_tool
            .approvals
            .decide(&app, "chat", &id, Decision::Deny)
            .await
            .unwrap();
        task.await.unwrap();
    })
    .await
    .unwrap();
}

async fn child_python_waiting_for_parent(
    fixture: &crate::agents::approval::tests::Fixture,
) -> (
    tokio::task::JoinHandle<Result<Vec<crate::call_validation::ContextEnum>, String>>,
    crate::agents::execution::OperationGuard,
    refact_work_core::PendingApproval,
) {
    use crate::tools::tools_description::Tool;
    // The runner admits the outer python call as an operation on the child's execution.
    let operation = fixture.execution.register_operation().unwrap();
    let ccx = fixture.ccx.clone();
    let code = "await shell(command='echo approved', description='Nested child approval', needs_confirmation=True)";
    let task = tokio::spawn(async move {
        crate::tools::tool_python::ToolPython::default()
            .tool_execute(
                ccx,
                &"python-child".into(),
                &std::collections::HashMap::from([("code".into(), serde_json::json!(code))]),
            )
            .await
            .map(|result| result.1)
    });
    let pending = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(pending) = fixture
                .app
                .agent_approvals
                .list_for_parent("parent", crate::agents::approval::now_ms())
                .first()
                .cloned()
            {
                break pending;
            }
            assert!(!task.is_finished(), "child call ended before asking");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("no parent approval request");
    (task, operation, pending)
}

fn python_extra(messages: &[crate::call_validation::ContextEnum]) -> serde_json::Value {
    messages
        .iter()
        .find_map(|m| match m {
            crate::call_validation::ContextEnum::ChatMessage(m) => m.extra.get("python").cloned(),
            _ => None,
        })
        .unwrap()
}

async fn python_fixture() -> crate::agents::approval::tests::Fixture {
    let mut fixture = crate::agents::approval::tests::fixture().await;
    fixture.app.python_tool = Arc::new(PythonToolRuntime {
        registry: super::registry::KernelRegistry::new(super::registry::WorkerBackend::InThread),
        ..Default::default()
    });
    fixture.ccx.lock().await.app = fixture.app.clone();
    fixture
}

#[tokio::test]
async fn running_child_python_call_proceeds_after_parent_approval() {
    let fixture = python_fixture().await;
    let (task, operation, pending) = child_python_waiting_for_parent(&fixture).await;
    fixture
        .app
        .agent_approvals
        .decide_with_reason(
            "parent",
            fixture.generation,
            &pending.id,
            true,
            None,
            crate::agents::approval::now_ms(),
        )
        .unwrap();
    let mut task = task;
    let messages = match tokio::time::timeout(std::time::Duration::from_secs(20), &mut task).await {
        Ok(result) => result.unwrap().unwrap(),
        Err(error) => {
            let diagnostics = approved_child_diagnostics(&fixture.app).await;
            fixture.execution.cancel();
            task.abort();
            let _ = task.await;
            panic!("approved nested call never ran: {error}; {diagnostics}");
        }
    };
    drop(operation);
    let extra = python_extra(&messages);
    assert_eq!(extra["status"], "completed", "{extra}");
    assert_eq!(extra["calls"][0]["status"], "ok", "{extra}");
    assert_eq!(extra["calls"][0]["approval"], "allowed", "{extra}");
    assert!(!fixture.execution.is_cancelled());
}

#[tokio::test]
async fn child_python_waiting_for_parent_ends_on_stop_and_on_agent_cancel() {
    for cancel_agent in [false, true] {
        let fixture = python_fixture().await;
        let (task, operation, pending) = child_python_waiting_for_parent(&fixture).await;
        if cancel_agent {
            fixture.execution.cancel();
        } else {
            fixture
                .app
                .python_tool
                .stop_run("subchat-approval-test", "python-child")
                .unwrap();
        }
        let messages = tokio::time::timeout(std::time::Duration::from_secs(10), task)
            .await
            .expect("waiting nested call did not end")
            .unwrap()
            .unwrap();
        drop(operation);
        let extra = python_extra(&messages);
        assert_eq!(extra["status"], "cancelled", "{extra}");
        assert_eq!(extra["calls"][0]["approval"], "cancelled", "{extra}");
        assert!(matches!(
            fixture
                .app
                .agent_approvals
                .wait_now(&pending.id, crate::agents::approval::now_ms()),
            Ok(refact_work_core::ApprovalDecision::Cancelled)
        ));
    }
}

fn protected_ask() -> ApprovalAsk {
    let mut request = ask(true);
    request.rule = crate::chat::tools::PROTECTED_CONFIG_APPROVAL.into();
    request
}

#[tokio::test]
async fn run_permit_never_covers_a_later_request_that_needs_a_person() {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let app = fixture().await;
        let _run = app.python_tool.register_run("chat", "python", "run");
        let waiter = spawn_ask(&app, ask(true));
        let id = pending(&app).await;
        app.python_tool
            .approvals
            .decide(&app, "chat", &id, Decision::AllowForRun)
            .await
            .unwrap();
        assert_eq!(waiter.await.unwrap(), ApprovalOutcome::AllowedForRun);
        let protected = spawn_ask(&app, protected_ask());
        let id = pending(&app).await;
        assert!(!protected.is_finished());
        app.python_tool
            .approvals
            .decide(&app, "chat", &id, Decision::Deny)
            .await
            .unwrap();
        assert_eq!(protected.await.unwrap(), ApprovalOutcome::Denied);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn run_permit_never_releases_a_pending_request_that_needs_a_person() {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let app = fixture().await;
        let _run = app.python_tool.register_run("chat", "python", "run");
        let protected = spawn_ask(&app, protected_ask());
        pending_ids(&app, 1).await;
        let edit = spawn_ask(&app, ask(true));
        let ids = pending_ids(&app, 2).await;
        let session = app.chat.sessions.read().await["chat"].clone();
        let protected_id = session
            .lock()
            .await
            .runtime
            .python_approvals
            .iter()
            .find(|r| crate::chat::tools::requires_actual_human_approval(&r.rule))
            .unwrap()
            .request_id
            .clone();
        let plain_id = ids
            .iter()
            .find(|(id, _)| *id != protected_id)
            .unwrap()
            .0
            .clone();
        app.python_tool
            .approvals
            .decide(&app, "chat", &plain_id, Decision::AllowForRun)
            .await
            .unwrap();
        assert_eq!(edit.await.unwrap(), ApprovalOutcome::AllowedForRun);
        assert!(!protected.is_finished());
        assert_eq!(pending_ids(&app, 1).await[0].0, protected_id);
        assert!(app
            .python_tool
            .approvals
            .decide(&app, "chat", &protected_id, Decision::AllowForRun)
            .await
            .is_err());
        app.python_tool
            .approvals
            .decide(&app, "chat", &protected_id, Decision::AllowOnce)
            .await
            .unwrap();
        assert_eq!(
            protected.await.unwrap(),
            ApprovalOutcome::Allowed {
                for_chat: false
            }
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn run_permit_is_dropped_with_the_run_guard_on_every_exit() {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let app = fixture().await;
        let run = app.python_tool.register_run("chat", "python", "run");
        let waiter = spawn_ask(&app, ask(true));
        let id = pending(&app).await;
        app.python_tool
            .approvals
            .decide(&app, "chat", &id, Decision::AllowForRun)
            .await
            .unwrap();
        assert_eq!(waiter.await.unwrap(), ApprovalOutcome::AllowedForRun);
        assert!(app.python_tool.approvals.run_permitted("chat", "run"));
        drop(run);
        assert!(!app.python_tool.approvals.run_permitted("chat", "run"));
    })
    .await
    .unwrap();
}

use crate::python_tool::approvals::{needs_person, run_permit_covers, ENGINE_CHANGE_APPROVAL};

fn engine_ask(editing_tool: bool) -> ApprovalAsk {
    let mut request = ask(editing_tool);
    request.tool_name = "engine.settings.set".into();
    request.rule = ENGINE_CHANGE_APPROVAL.into();
    request.engine = Some(refact_chat_api::PythonEngineRequest {
        function: "settings.set".into(),
        class: refact_chat_api::PythonEngineClass::Change,
        title: "change engine settings".into(),
        items: vec![],
        scope: Some("project".into()),
        warning: None,
    });
    request
}

#[tokio::test]
async fn engine_change_request_is_never_covered_by_a_run_permit() {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        for editing in [false, true] {
            let app = fixture().await;
            let _run = app.python_tool.register_run("chat", "python", "run");
            app.python_tool
                .approvals
                .run_permits
                .lock()
                .unwrap()
                .insert(("chat".into(), "run".into()));
            let waiter = spawn_ask(&app, engine_ask(editing));
            let id = pending(&app).await;
            let request = app.chat.sessions.read().await["chat"]
                .lock()
                .await
                .runtime
                .python_approvals[0]
                .clone();
            assert_eq!(request.engine.as_ref().unwrap().function, "settings.set");
            assert!(request.needs_person);
            assert!(!run_permit_covers(&request));
            for rejected in [Decision::AllowForRun, Decision::AllowForChat] {
                assert!(app
                    .python_tool
                    .approvals
                    .decide(&app, "chat", &id, rejected)
                    .await
                    .is_err());
            }
            assert!(
                !app.chat.sessions.read().await["chat"]
                    .lock()
                    .await
                    .thread
                    .auto_approve_editing_tools
            );
            app.python_tool
                .approvals
                .decide(&app, "chat", &id, Decision::AllowOnce)
                .await
                .unwrap();
            assert_eq!(
                waiter.await.unwrap(),
                ApprovalOutcome::Allowed {
                    for_chat: false
                }
            );
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn request_that_needs_a_person_says_so_and_refuses_both_permits() {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let app = fixture().await;
        let _run = app.python_tool.register_run("chat", "python", "run");
        let edit = spawn_ask(&app, ask(true));
        let id = pending(&app).await;
        let session = app.chat.sessions.read().await["chat"].clone();
        assert!(!session.lock().await.runtime.python_approvals[0].needs_person);
        app.python_tool
            .approvals
            .decide(&app, "chat", &id, Decision::AllowOnce)
            .await
            .unwrap();
        edit.await.unwrap();
        let protected = spawn_ask(&app, protected_ask());
        let id = pending(&app).await;
        let snapshot = serde_json::to_value(session.lock().await.snapshot()).unwrap();
        assert_eq!(
            snapshot["runtime"]["python_approvals"][0]["needs_person"],
            true
        );
        for (decision, scope) in [
            (Decision::AllowForRun, "this run"),
            (Decision::AllowForChat, "this chat"),
        ] {
            let error = app
                .python_tool
                .approvals
                .decide(&app, "chat", &id, decision)
                .await
                .unwrap_err();
            assert!(
                error.contains(scope) && error.contains("needs a person's approval"),
                "{error}"
            );
        }
        assert!(!protected.is_finished());
        assert!(!session.lock().await.thread.auto_approve_editing_tools);
        app.python_tool
            .approvals
            .decide(&app, "chat", &id, Decision::Deny)
            .await
            .unwrap();
        assert_eq!(protected.await.unwrap(), ApprovalOutcome::Denied);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn engine_change_rule_alone_marks_a_request_as_needing_a_person() {
    let mut request = refact_chat_api::PythonApprovalRequest {
        request_id: "r".into(),
        tool_call_id: "t".into(),
        run_id: "run".into(),
        call_id: "c".into(),
        tool_name: "engine.settings.set".into(),
        args_preview: "{}".into(),
        command: None,
        rule: ENGINE_CHANGE_APPROVAL.into(),
        editing_tool: true,
        created_at_ms: 0,
        engine: None,
        needs_person: false,
    };
    assert!(needs_person(&request) && !run_permit_covers(&request));
    request.rule = "confirm".into();
    assert!(!needs_person(&request) && run_permit_covers(&request));
    request.engine = engine_ask(true).engine;
    assert!(needs_person(&request) && !run_permit_covers(&request));
}

#[tokio::test]
async fn allow_for_run_from_a_child_chat_has_no_effect() {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let app = fixture().await;
        let mut child = ChatSession::new("child".into());
        child.thread.link_type = Some("subagent".into());
        app.chat
            .sessions
            .write()
            .await
            .insert("child".into(), Arc::new(Mutex::new(child)));
        let _child_run = app.python_tool.register_run("child", "python", "child-run");
        let mut child_ask = ask(true);
        child_ask.chat_id = "child".into();
        child_ask.run_id = "child-run".into();
        assert_eq!(
            app.python_tool.approvals.request(&app, child_ask).await,
            ApprovalOutcome::Denied
        );
        assert!(app.chat.sessions.read().await["child"]
            .lock()
            .await
            .runtime
            .python_approvals
            .is_empty());
        let _run = app.python_tool.register_run("chat", "python", "run");
        let root_edit = spawn_ask(&app, ask(true));
        let root_id = pending(&app).await;
        assert!(app
            .python_tool
            .approvals
            .decide(&app, "child", &root_id, Decision::AllowForRun)
            .await
            .is_err());
        assert!(!app
            .python_tool
            .approvals
            .run_permitted("child", "child-run"));
        assert!(!app.python_tool.approvals.run_permitted("child", "run"));
        assert!(!app.python_tool.approvals.run_permitted("chat", "run"));
        assert!(!root_edit.is_finished());
        let mut later = ask(true);
        later.chat_id = "child".into();
        later.run_id = "child-run".into();
        assert_eq!(
            app.python_tool.approvals.request(&app, later).await,
            ApprovalOutcome::Denied
        );
        app.python_tool
            .approvals
            .decide(&app, "chat", &root_id, Decision::Deny)
            .await
            .unwrap();
        assert_eq!(root_edit.await.unwrap(), ApprovalOutcome::Denied);
    })
    .await
    .unwrap();
}

fn files_containing(root: &std::path::Path, needle: &str) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if std::fs::read(&path)
                .is_ok_and(|bytes| String::from_utf8_lossy(&bytes).contains(needle))
            {
                found.push(path);
            }
        }
    }
    found
}

#[tokio::test]
async fn run_permit_is_never_written_to_disk_or_to_saved_parameters() {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().join("project")];
        std::fs::create_dir_all(dir.path().join("project")).unwrap();
        let app = AppState::from_gcx(gcx).await;
        let session = Arc::new(Mutex::new(ChatSession::new("permit-chat".into())));
        app.chat
            .sessions
            .write()
            .await
            .insert("permit-chat".into(), session.clone());
        let run_id = "run-permit-7f3a9c";
        let _run = app
            .python_tool
            .register_run("permit-chat", "python", run_id);
        let mut edit = ask(true);
        edit.chat_id = "permit-chat".into();
        edit.run_id = run_id.into();
        let waiter = spawn_ask(&app, edit);
        let id = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(request) = session.lock().await.runtime.python_approvals.first() {
                    return request.request_id.clone();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        app.python_tool
            .approvals
            .decide(&app, "permit-chat", &id, Decision::AllowForRun)
            .await
            .unwrap();
        assert_eq!(waiter.await.unwrap(), ApprovalOutcome::AllowedForRun);
        assert!(app
            .python_tool
            .approvals
            .run_permitted("permit-chat", run_id));
        {
            let mut session = session.lock().await;
            session.add_message(crate::call_validation::ChatMessage::new(
                "user".into(),
                "edit the files".into(),
            ));
            session.increment_version();
        }
        crate::chat::trajectories::maybe_save_trajectory(app.clone(), session.clone()).await;
        let saved =
            crate::chat::trajectories::load_trajectory_for_chat(app.gcx.clone(), "permit-chat")
                .await
                .expect("the trajectory was saved");
        assert!(!saved.thread.auto_approve_editing_tools);
        let thread = serde_json::to_string(&saved.thread).unwrap();
        assert!(
            !thread.contains(run_id) && !thread.contains("for_run"),
            "{thread}"
        );
        assert_eq!(
            files_containing(dir.path(), run_id),
            Vec::<std::path::PathBuf>::new()
        );
        assert!(app
            .python_tool
            .approvals
            .run_permitted("permit-chat", run_id));
    })
    .await
    .unwrap();
}
