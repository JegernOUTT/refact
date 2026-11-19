use super::*;
use refact_python_catalog::CatalogFunction;

use crate::call_validation::{ChatContent, ContextEnum};
use crate::tools::tools_description::Tool;
use refact_tool_api::{MatchConfirmDeny, MatchConfirmDenyResult, ToolDesc};
use std::collections::HashMap;

struct FixtureTool {
    fail: bool,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
    confirms: Arc<AtomicUsize>,
}
#[async_trait::async_trait]
impl Tool for FixtureTool {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        if args.get("n").and_then(Value::as_i64) == Some(-30) {
            let abort = ccx.lock().await.abort_flag.clone();
            tokio::time::timeout(std::time::Duration::from_secs(30), async {
                while !abort.load(Ordering::Acquire) {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
                self.peak.store(100, Ordering::Release);
            })
            .await
            .map_err(|_| "abort not observed".to_string())?;
        } else if args.get("n").and_then(Value::as_i64) == Some(-60) {
            // Deliberately ignore cancellation beyond the old five-second grace period.
            tokio::time::sleep(std::time::Duration::from_secs(6)).await;
            self.peak.store(100, Ordering::Release);
        } else {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        if self.fail {
            return Err("fixture failure".into());
        }
        let mut message = ChatMessage {
            role: "tool".into(),
            tool_call_id: id.clone(),
            content: ChatContent::SimpleText("fixture output".into()),
            ..Default::default()
        };
        message
            .extra
            .insert("tool_output".into(), json!({"answer":42}));
        Ok((false, vec![ContextEnum::ChatMessage(message)]))
    }
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "fixture".into(),
            experimental: false,
            allow_parallel: true,
            description: "fixture".into(),
            input_schema: json!({"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]}),
            output_schema: None,
            annotations: None,
            display_name: "fixture".into(),
            source: refact_tool_api::ToolSource {
                source_type: refact_tool_api::ToolSourceType::Builtin,
                config_path: String::new(),
            },
        }
    }
    async fn match_against_confirm_deny(
        &self,
        _: Arc<AMutex<AtCommandsContext>>,
        _: &HashMap<String, Value>,
    ) -> Result<MatchConfirmDeny, String> {
        self.confirms.fetch_add(1, Ordering::SeqCst);
        Ok(MatchConfirmDeny {
            result: MatchConfirmDenyResult::PASS,
            command: String::new(),
            rule: "fixture".into(),
        })
    }
}

fn register_fixture(d: &mut NestedDispatcher, fail: bool) -> Arc<AtomicUsize> {
    register_counting_fixture(d, fail).0
}
fn register_counting_fixture(
    d: &mut NestedDispatcher,
    fail: bool,
) -> (Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let confirms = Arc::new(AtomicUsize::new(0));
    let return_peak = peak.clone();
    let return_confirms = confirms.clone();
    d.config.app = AppState {
        tool_registry: Arc::new(
            crate::app_state::AppToolRegistry::with_fixture_tool_factory(
                d.config.app.gcx.clone(),
                Arc::new(move || {
                    vec![Box::new(FixtureTool {
                        fail,
                        active: active.clone(),
                        peak: peak.clone(),
                        confirms: confirms.clone(),
                    }) as Box<dyn Tool + Send>]
                }),
            ),
        ),
        ..d.config.app.clone()
    };
    d.config.allowed_tools.clear();
    (return_peak, return_confirms)
}

#[cfg(unix)]
async fn hook_fixture(command: &str) -> (NestedDispatcher, tempfile::TempDir, tempfile::TempDir) {
    let (mut d, run) = fixture().await;
    let config = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        config.path().join("cache"),
        config.path().into(),
    )
    .await;
    d.config.app = AppState::from_gcx(gcx).await;
    std::fs::write(config.path().join("hooks.yaml"),format!("hooks:\n  PreToolUse:\n    - hooks:\n        - type: command\n          command: {command:?}\n")).unwrap();
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
    register_fixture(&mut d, false);
    (d, run, config)
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn pretooluse_deny_raises_permission_error() {
    let (d, _run, _config) = hook_fixture("echo nested-denied >&2; exit 2").await;
    assert!(exception(
        d.dispatch(1, "fixture", json!({"n":1}), None).await,
        "PermissionError"
    )
    .contains("nested-denied"));
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn pretooluse_rewrite_is_revalidated() {
    let (d, _run, _config) =
        hook_fixture("echo '{\"decision\":\"rewrite\",\"tool_input\":{\"n\":\"bad\"}}'").await;
    exception(
        d.dispatch(1, "fixture", json!({"n":1}), None).await,
        "TypeError",
    );
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn nested_hook_payload_carries_python_parent_ids() {
    let target = tempfile::tempdir().unwrap();
    let payload = target.path().join("payload.json");
    let (d, _run, _config) = hook_fixture(&format!("cat > '{}'", payload.display())).await;
    d.dispatch(7, "fixture", json!({"n":1}), None).await;
    let value: Value = serde_json::from_slice(&std::fs::read(payload).unwrap()).unwrap();
    assert_eq!(value["python_run_id"], "run");
    assert_eq!(value["python_parent_tool_call_id"], "outer");
    assert_eq!(value["tool_call_id"], "py_run_7");
}

async fn set_mode_rule(d: &mut NestedDispatcher, action: &str) {
    set_mode_rule_for(d, "fixture", action).await;
}

async fn set_mode_rule_for(d: &mut NestedDispatcher, tool: &str, action: &str) {
    let mut registry =
        crate::yaml_configs::customization_registry::get_project_registry(d.config.app.gcx.clone())
            .await
            .unwrap();
    let mut mode = registry.modes["agent"].clone();
    mode.id = "nested_fixture".into();
    mode.tools = vec![tool.into()];
    mode.tool_confirm.rules = vec![crate::yaml_configs::customization_types::ToolConfirmRule {
        match_pattern: tool.into(),
        action: action.into(),
    }];
    registry.modes.insert(mode.id.clone(), mode);
    let root = crate::files_correction::get_project_dirs(d.config.app.gcx.clone())
        .await
        .into_iter()
        .next()
        .unwrap_or_else(|| d.config.app.gcx.config_dir.clone());
    d.config
        .app
        .gcx
        .project_registry_cache
        .write()
        .unwrap()
        .insert(root, registry);
    d.config
        .app
        .gcx
        .tool_catalog_generations
        .advance_customization();
    d.config.mode_id = "nested_fixture".into();
}

#[tokio::test]
async fn mode_deny_rule_raises_permission_error() {
    let (mut d, _) = fixture().await;
    register_fixture(&mut d, false);
    set_mode_rule(&mut d, "deny").await;
    assert!(exception(
        d.dispatch(1, "fixture", json!({"n":1}), None).await,
        "PermissionError"
    )
    .contains("mode policy: deny"));
}

#[tokio::test]
async fn deny_rule_still_raises_permission_error_under_run_permit() {
    let (mut d, _dir) = fixture().await;
    register_fixture(&mut d, false);
    set_mode_rule(&mut d, "deny").await;
    d.config
        .app
        .python_tool
        .approvals
        .run_permits
        .lock()
        .unwrap()
        .insert((d.config.chat_id.clone(), d.config.run_id.clone()));
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        d.dispatch(1, "fixture", json!({"n":1}), None),
    )
    .await
    .unwrap();
    assert!(exception(outcome, "PermissionError").contains("mode policy: deny"));
}

#[tokio::test]
async fn ask_rule_requests_approval_and_waits() {
    use refact_chat_api::PythonApprovalDecisionKind as Decision;
    for decision in [Decision::AllowOnce, Decision::Deny, Decision::Stop] {
        let (mut d, _dir) = fixture().await;
        register_fixture(&mut d, false);
        set_mode_rule(&mut d, "ask").await;
        let mut session = crate::chat::types::ChatSession::new(d.config.chat_id.clone());
        session.thread = d.config.thread.clone();
        d.config
            .app
            .chat
            .sessions
            .write()
            .await
            .insert(d.config.chat_id.clone(), Arc::new(AMutex::new(session)));
        let _run = d
            .config
            .app
            .python_tool
            .register_run(&d.config.chat_id, "outer", "run");
        let app = d.config.app.clone();
        let chat = d.config.chat_id.clone();
        let waiter =
            tokio::spawn(async move { d.dispatch(1, "fixture", json!({"n":1}), None).await });
        let id = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let session = app.chat.sessions.read().await[&chat].clone();
                if let Some(request) = session.lock().await.runtime.python_approvals.first() {
                    break request.request_id.clone();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!waiter.is_finished());
        app.python_tool
            .approvals
            .decide(&app, &chat, &id, decision)
            .await
            .unwrap();
        let outcome = waiter.await.unwrap();
        match decision {
            Decision::AllowOnce => assert!(matches!(outcome.result, HostResult::Ok(_))),
            Decision::Deny => {
                exception(outcome, "PermissionError");
            }
            Decision::Stop => assert!(outcome.stop_run),
            _ => unreachable!(),
        }
    }
}

#[tokio::test]
async fn auto_approve_dangerous_commands_skips_approval() {
    let (mut d, _dir) = fixture().await;
    register_fixture(&mut d, false);
    set_mode_rule(&mut d, "ask").await;
    d.config.thread.auto_approve_dangerous_commands = true;
    let mut session = crate::chat::types::ChatSession::new(d.config.chat_id.clone());
    session.thread = d.config.thread.clone();
    d.config
        .app
        .chat
        .sessions
        .write()
        .await
        .insert(d.config.chat_id.clone(), Arc::new(AMutex::new(session)));
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        d.dispatch(1, "fixture", json!({"n":1}), None),
    )
    .await
    .unwrap();
    assert!(matches!(outcome.result, HostResult::Ok(_)));
}

#[tokio::test]
async fn parallel_calls_respect_semaphore() {
    let (mut d, _dir) = fixture().await;
    let peak = register_fixture(&mut d, false);
    d.config.max_nested_calls = 8;
    let results = futures::future::join_all(
        (0..4).map(|seq| d.dispatch(seq, "fixture", json!({"n":seq}), None)),
    )
    .await;
    for result in results {
        assert!(matches!(result.result, HostResult::Ok(_)));
    }
    assert_eq!(peak.load(Ordering::SeqCst), 1);
}

async fn cancellation_case() {
    let (mut d, _dir) = fixture().await;
    let observed = register_fixture(&mut d, false);
    let session = Arc::new(AMutex::new(crate::chat::types::ChatSession::new(
        d.config.chat_id.clone(),
    )));
    d.config
        .app
        .chat
        .sessions
        .write()
        .await
        .insert(d.config.chat_id.clone(), session.clone());
    let outer_abort = d.config.ccx.lock().await.abort_flag.clone();
    let cancel = d.config.cancel.clone();
    let waiter =
        tokio::spawn(async move { d.dispatch(1, "fixture", json!({"n":-30}), None).await });
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while observed.load(Ordering::Acquire) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    cancel.cancel();
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
        .await
        .unwrap()
        .unwrap();
    assert!(result.stop_run);
    assert_eq!(observed.load(Ordering::Acquire), 100);
    assert!(!outer_abort.load(Ordering::Acquire));
    assert!(session.lock().await.messages.is_empty());
}

#[tokio::test]
async fn cancellation_waits_past_grace_for_nested_execution_terminal() {
    let (mut d, _dir) = fixture().await;
    let observed = register_fixture(&mut d, false);
    let cancel = d.config.cancel.clone();
    let app = d.config.app.clone();
    let chat = d.config.chat_id.clone();
    let mut waiter =
        tokio::spawn(async move { d.dispatch(1, "fixture", json!({"n":-60}), None).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while observed.load(Ordering::Acquire) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    cancel.cancel();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(5500), &mut waiter)
            .await
            .is_err(),
        "dispatch must not return while a cancellation-ignoring tool still executes"
    );
    assert_eq!(
        app.python_tool
            .nested_parents
            .lock()
            .unwrap()
            .get(&(chat, "py_run_1".into())),
        Some(&"outer".to_string())
    );
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
        .await
        .unwrap()
        .unwrap();
    assert!(result.stop_run);
    assert_eq!(observed.load(Ordering::Acquire), 100);
    assert!(app.python_tool.nested_parents.lock().unwrap().is_empty());
}

#[tokio::test]
async fn successful_execution_does_not_set_outer_abort_flag() {
    let (mut d, _dir) = fixture().await;
    register_fixture(&mut d, false);
    let outer = d.config.ccx.lock().await.abort_flag.clone();
    let result = d.dispatch(1, "fixture", json!({"n":1}), None).await;
    assert!(matches!(result.result, HostResult::Ok(_)));
    assert!(!outer.load(Ordering::Acquire));
}

#[tokio::test]
async fn stopping_run_does_not_set_outer_abort_flag() {
    cancellation_case().await;
}

#[tokio::test]
async fn stop_cancels_in_flight_nested_tool_promptly() {
    cancellation_case().await;
}

struct Sink;
impl CallSink for Sink {
    fn call_started(&self, _: &PythonCallRecord) {}
    fn call_finished(&self, _: &PythonCallRecord) {}
}

async fn fixture() -> (NestedDispatcher, tempfile::TempDir) {
    let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
    let thread = ThreadParams {
        id: "nested-test".into(),
        model: "model".into(),
        ..Default::default()
    };
    let ccx = Arc::new(AMutex::new(
        AtCommandsContext::new_from_app(
            app.clone(),
            16384,
            1,
            false,
            vec![],
            thread.id.clone(),
            thread.root_chat_id.clone(),
            thread.model.clone(),
            None,
            None,
        )
        .await,
    ));
    let dir = tempfile::tempdir().unwrap();
    let dispatcher = NestedDispatcher::new(NestedDispatchConfig {
        app,
        ccx,
        chat_id: thread.id.clone(),
        outer_tool_call_id: "outer".into(),
        run_id: "run".into(),
        catalog: Arc::new(PythonCatalog {
            functions: vec![CatalogFunction {
                py_name: "fixture".into(),
                tool_name: "fixture".into(),
                summary: String::new(),
                description: String::new(),
                input_schema: json!({"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]}),
                output_schema: None,
                is_async: true,
            }],
            helpers: vec![],
            description: String::new(),
            engine: vec![],
        }),
        thread,
        mode_id: "agent".into(),
        model_id: None,
        allowed_tools: vec!["cat".into()],
        tool_catalog: None,
        turn_tool_pool: None,
        max_nested_calls: 1,
        max_parallel_calls: 1,
        max_nested_result_bytes: 4096,
        run_dir: dir.path().into(),
        cancel: CancellationToken::new(),
        sink: Arc::new(Sink),
        llm: None,
        run_privacy: Default::default(),
        engine: None,
        project_root: dir.path().into(),
        workspace_folders: vec![dir.path().into()],
        deny_read: Default::default(),
        workspace_dir: dir.path().into(),
    });
    (dispatcher, dir)
}
fn exception(outcome: NestedOutcome, expected: &str) -> String {
    match outcome.result {
        HostResult::Err {
            exc_type,
            message,
        } => {
            assert_eq!(exc_type, expected);
            message
        }
        _ => panic!("expected exception"),
    }
}
#[tokio::test]
async fn nested_call_returns_envelope_with_structured_data() {
    let (mut d, _) = fixture().await;
    register_fixture(&mut d, false);
    match d.dispatch(1, "fixture", json!({"n":1}), None).await.result {
        HostResult::Ok(value) => {
            assert_eq!(value["ok"], true);
            assert_eq!(value["data"]["answer"], 42);
        }
        other => panic!("{other:?}"),
    }
}
#[tokio::test]
async fn failed_tool_returns_ok_false_not_exception() {
    let (mut d, _) = fixture().await;
    register_fixture(&mut d, true);
    match d.dispatch(1, "fixture", json!({"n":1}), None).await.result {
        HostResult::Ok(value) => assert_eq!(value["ok"], false),
        other => panic!("{other:?}"),
    }
}
#[tokio::test]
async fn nested_messages_are_not_added_to_session() {
    let (mut d, _) = fixture().await;
    register_fixture(&mut d, false);
    let session = Arc::new(AMutex::new(crate::chat::types::ChatSession::new(
        d.config.chat_id.clone(),
    )));
    d.config
        .app
        .chat
        .sessions
        .write()
        .await
        .insert(d.config.chat_id.clone(), session.clone());
    let before = session.lock().await.messages.len();
    d.dispatch(1, "fixture", json!({"n":1}), None).await;
    assert_eq!(session.lock().await.messages.len(), before);
}
#[tokio::test]
async fn unknown_function_raises_runtime_error() {
    let (d, _) = fixture().await;
    assert!(exception(
        d.dispatch(1, "unknown", json!({}), None).await,
        "RuntimeError"
    )
    .contains("unknown function"));
}
#[tokio::test]
async fn truncated_kwargs_raise_value_error() {
    let (d, _) = fixture().await;
    assert_eq!(
        exception(
            d.dispatch(
                1,
                "fixture",
                json!({}),
                Some("at least 200 bytes, over the 128-byte limit".into())
            )
            .await,
            "ValueError"
        ),
        "fixture() arguments are too large to send: at least 200 bytes, over the 128-byte limit; pass a file path or a smaller value"
    );
}
#[tokio::test]
async fn invalid_kwargs_raise_type_error_with_schema_message() {
    let (d, _) = fixture().await;
    assert!(exception(
        d.dispatch(1, "fixture", json!({"n":"x"}), None).await,
        "TypeError"
    )
    .contains("must be integer"));
}
#[tokio::test]
async fn call_outside_authority_raises_permission_error() {
    let (d, _) = fixture().await;
    exception(
        d.dispatch(1, "fixture", json!({"n":1}), None).await,
        "PermissionError",
    );
}
#[tokio::test]
async fn child_authority_ceiling_is_enforced() {
    let (mut d, _dir) = fixture().await;
    register_fixture(&mut d, false);
    d.config.ccx.lock().await.child_tool_ceiling =
        Some(crate::subchat::ToolsPolicy::Only(vec!["cat".into()]));
    assert!(exception(
        d.dispatch(1, "fixture", json!({"n":1}), None).await,
        "PermissionError"
    )
    .contains("child tool authority"));
}

#[tokio::test]
async fn nested_call_budget_exhausted_raises_runtime_error() {
    let (d, _) = fixture().await;
    d.dispatch(1, "fixture", json!({"n":1}), None).await;
    assert!(exception(
        d.dispatch(2, "fixture", json!({"n":1}), None).await,
        "RuntimeError"
    )
    .contains("budget exhausted"));
}
#[tokio::test]
async fn call_file_is_written_to_run_dir() {
    let (d, dir) = fixture().await;
    d.dispatch(1, "fixture", json!({"n":1}), None).await;
    let value: Value = serde_json::from_slice(
        &tokio::fs::read(dir.path().join("calls/1_fixture.json"))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(value["call_id"], "py_run_1");
}
#[tokio::test]
async fn cancelled_run_stops_waiting_call() {
    let (d, _) = fixture().await;
    d.config.cancel.cancel();
    let outcome = d.dispatch(1, "fixture", json!({"n":1}), None).await;
    assert!(outcome.stop_run);
    exception(outcome, "RuntimeError");
}

#[tokio::test]
async fn optional_none_is_not_received_by_tool() {
    let (mut d, _dir) = fixture().await;
    register_fixture(&mut d, false);
    Arc::make_mut(&mut d.config.catalog).functions[0].input_schema["required"] = json!([]);
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        d.dispatch(1, "fixture", json!({"n":null}), None),
    )
    .await
    .unwrap();
    assert!(
        matches!(outcome.result, HostResult::Ok(_)),
        "{:?}",
        outcome.record
    );
}
#[tokio::test]
async fn autonomous_no_confirm_records_auto_without_card() {
    let (mut d, _dir) = fixture().await;
    register_fixture(&mut d, false);
    set_mode_rule(&mut d, "ask").await;
    d.config.thread.autonomous_no_confirm = true;
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        d.dispatch(1, "fixture", json!({"n":1}), None),
    )
    .await
    .unwrap();
    assert_eq!(outcome.record.approval, PythonCallApproval::Auto);
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn authority_refusal_precedes_hooks_and_approval() {
    let (mut d, _run, config) = hook_fixture("touch authority-hook-ran").await;
    d.config.ccx.lock().await.child_tool_ceiling =
        Some(crate::subchat::ToolsPolicy::Only(vec!["cat".into()]));
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        d.dispatch(1, "fixture", json!({"n":1}), None),
    )
    .await
    .unwrap();
    assert!(exception(outcome, "PermissionError").contains("authority"));
    assert!(!config.path().join("authority-hook-ran").exists());
}

#[tokio::test]
async fn auto_label_comes_from_a_single_confirmation_check() {
    for (autonomous, rule, expected) in [
        (true, "ask", PythonCallApproval::Auto),
        (false, "auto", PythonCallApproval::NotNeeded),
    ] {
        let (mut d, _dir) = fixture().await;
        let (_, confirms) = register_counting_fixture(&mut d, false);
        set_mode_rule(&mut d, rule).await;
        d.config.thread.autonomous_no_confirm = autonomous;
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            d.dispatch(1, "fixture", json!({"n":1}), None),
        )
        .await
        .unwrap();
        assert!(matches!(outcome.result, HostResult::Ok(_)));
        assert_eq!(outcome.record.approval, expected);
        assert_eq!(
            confirms.load(Ordering::SeqCst),
            1,
            "autonomous={autonomous}"
        );
    }
}

#[test]
fn optional_null_through_ref_follows_the_referenced_schema() {
    let schema = json!({
        "type":"object",
        "$defs":{"Nullable":{"type":["string","null"]},"Plain":{"type":"string"}},
        "properties":{"a":{"$ref":"#/$defs/Nullable"},"b":{"$ref":"#/$defs/Plain"}}
    });
    let normalized = normalize_optional_nulls(&schema, &json!({"a":null,"b":null}));
    assert!(normalized.get("a").is_some_and(Value::is_null));
    assert!(normalized.get("b").is_none());
    assert!(validate(&schema, &normalized).is_ok());
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum ShellEnd {
    Exited,
    Long,
    Unobserved,
    Withheld,
    AskPending,
    NoReceipt,
    ObservationIncomplete,
    ForeignProcess,
    StillRunning,
    Detached,
    TextOnlyAfterStart,
    RefusedBeforeStart,
}

/// A result shaped like `tool_shell`'s, for a process in the exec registry that printed
/// "a\nerror: boom\n" and ended with exit code 3 unless `end` says otherwise.
async fn shell_result(
    registry: &crate::exec::ExecRegistry,
    chat_id: &str,
    call_id: &str,
    command: &str,
    end: ShellEnd,
) -> Result<ChatMessage, String> {
    let text_only = |text: &str| ChatMessage {
        role: "tool".into(),
        tool_call_id: call_id.into(),
        content: ChatContent::SimpleText(text.into()),
        tool_failed: Some(true),
        ..Default::default()
    };
    if end == ShellEnd::RefusedBeforeStart {
        return Ok(text_only("the workdir is outside the chat's worktree"));
    }
    let owner = if end == ShellEnd::ForeignProcess {
        "another-call"
    } else {
        call_id
    };
    let meta =
        crate::exec::ExecProcessMeta::new(crate::exec::ExecMode::Foreground, command.to_string())
            .with_chat_id(chat_id)
            .with_tool_call_id(owner);
    let process_id = registry.register(meta, 1 << 20).await.meta.process_id;
    let long = (0..2000)
        .map(|i| format!("line {i:04}\n"))
        .collect::<String>();
    for (stream, text) in [
        (crate::exec::ExecOutputStream::Stdout, "a\n"),
        (
            crate::exec::ExecOutputStream::Stdout,
            if end == ShellEnd::Long {
                long.as_str()
            } else {
                ""
            },
        ),
        (crate::exec::ExecOutputStream::Stderr, "error: boom\n"),
    ] {
        if text.is_empty() {
            continue;
        }
        registry
            .append_output(&process_id, stream, text.into())
            .await?;
    }
    if end != ShellEnd::StillRunning {
        registry.mark_exited(&process_id, Some(3)).await?;
    }
    if end == ShellEnd::TextOnlyAfterStart {
        return Ok(text_only("a PostToolUse hook refused the result"));
    }
    let mut message = ChatMessage {
        role: "tool".into(),
        tool_call_id: call_id.into(),
        content: ChatContent::SimpleText("shell output".into()),
        ..Default::default()
    };
    message.extra.insert(
        "exec".into(),
        json!({"process_id":process_id.as_str(),"status":"exited","exit_code":3,"duration_ms":1200,"detached":end == ShellEnd::Detached}),
    );
    if end != ShellEnd::Unobserved {
        message.extra.insert(
            "privacy".into(),
            json!({"files":[{"path":"/secret/notes.txt","zone":"restricted","attribution":"observed"}]}),
        );
    }
    if end != ShellEnd::NoReceipt {
        let withheld = matches!(
            end,
            ShellEnd::Withheld | ShellEnd::AskPending | ShellEnd::Detached
        );
        message.extra.insert(
            "tool_output".into(),
            json!({"status":"exited","exit_code":3,"stdout":"","stderr":"","output_withheld":withheld}),
        );
    }
    let local = json!("a\nerror: boom\n");
    match end {
        ShellEnd::Withheld => {
            message.extra.insert(
                "privacy_shell".into(),
                json!({"withheld":true,"ask_pending":false,"local_only_output":local}),
            );
        }
        ShellEnd::AskPending => {
            message.extra.insert(
                "privacy_shell".into(),
                json!({"withheld":false,"ask_pending":true,"local_only_output":local}),
            );
        }
        ShellEnd::ObservationIncomplete => {
            message.extra.insert(
                "privacy_observation".into(),
                json!({"status":"incomplete","degraded":false,"incomplete":true}),
            );
        }
        _ => {}
    }
    Ok(message)
}

fn shell_schema() -> Value {
    json!({"type":"object","properties":{"command":{"type":"string"},"description":{"type":"string"},"timeout":{"type":"integer"},"workdir":{"type":"string"},"run_in_background":{"type":"boolean"}},"required":["command","description"]})
}

struct ShellFixture {
    end: ShellEnd,
    calls: Arc<AtomicUsize>,
    registry: Arc<crate::exec::ExecRegistry>,
}
#[async_trait::async_trait]
impl Tool for ShellFixture {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let chat_id = ccx.lock().await.chat_id.clone();
        let command = args
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let message = shell_result(&self.registry, &chat_id, id, command, self.end).await?;
        Ok((false, vec![ContextEnum::ChatMessage(message)]))
    }
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "shell".into(),
            experimental: false,
            allow_parallel: true,
            description: "shell".into(),
            input_schema: shell_schema(),
            output_schema: None,
            annotations: None,
            display_name: "shell".into(),
            source: refact_tool_api::ToolSource {
                source_type: refact_tool_api::ToolSourceType::Builtin,
                config_path: String::new(),
            },
        }
    }
    async fn match_against_confirm_deny(
        &self,
        _: Arc<AMutex<AtCommandsContext>>,
        _: &HashMap<String, Value>,
    ) -> Result<MatchConfirmDeny, String> {
        Ok(MatchConfirmDeny {
            result: MatchConfirmDenyResult::PASS,
            command: String::new(),
            rule: "shell".into(),
        })
    }
}

fn register_shell_fixture(d: &mut NestedDispatcher, end: ShellEnd) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let registry = d.config.app.runtime.exec_registry.clone();
    d.config.app = AppState {
        tool_registry: Arc::new(
            crate::app_state::AppToolRegistry::with_fixture_tool_factory(
                d.config.app.gcx.clone(),
                Arc::new(move || {
                    vec![Box::new(ShellFixture {
                        end,
                        calls: counter.clone(),
                        registry: registry.clone(),
                    }) as Box<dyn Tool + Send>]
                }),
            ),
        ),
        ..d.config.app.clone()
    };
    Arc::make_mut(&mut d.config.catalog).functions = vec![CatalogFunction {
        py_name: "shell".into(),
        tool_name: "shell".into(),
        summary: String::new(),
        description: String::new(),
        input_schema: shell_schema(),
        output_schema: None,
        is_async: true,
    }];
    d.config.allowed_tools.clear();
    calls
}

fn run_value(outcome: &NestedOutcome) -> Value {
    match &outcome.result {
        HostResult::Ok(value) => value.clone(),
        other => panic!("{other:?}"),
    }
}

async fn dispatch_run(d: &NestedDispatcher, seq: u32, kwargs: Value) -> NestedOutcome {
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        d.dispatch_run(seq, kwargs, None, (None, None)),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn run_projects_the_shell_result_into_a_labeled_log() {
    let (mut d, dir) = fixture().await;
    let calls = register_shell_fixture(&mut d, ShellEnd::Exited);
    let outcome = dispatch_run(&d, 1, json!({"cmd":"make test","tail":1})).await;
    let value = run_value(&outcome);
    let log = dir.path().join("logs/1_run.log");
    assert_eq!(value["ok"], false);
    assert_eq!(value["status"], "exited");
    assert_eq!(value["exit_code"], 3);
    assert_eq!(value["duration_s"], 1.2);
    assert_eq!(value["tail"], "error: boom");
    assert_eq!(value["problems"], json!(["error: boom"]));
    assert_eq!(value["truncated"], false);
    assert_eq!(value["error"], "the command exited with code 3");
    assert_eq!(
        value["log"],
        json!(refact_python_kernel::hostfs::virtual_path(
            &log,
            if cfg!(windows) {
                refact_python_kernel::hostfs::Platform::Windows
            } else {
                refact_python_kernel::hostfs::Platform::Unix
            }
        ))
    );
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "a\nerror: boom\n");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(outcome.record.tool, "shell");
    assert_eq!(outcome.record.status, PythonCallStatus::Failed);
    assert_eq!(outcome.record.summary, "exit 3 · 1.2 s · 1 problem");
    assert!(outcome
        .privacy
        .iter()
        .any(|record| record["files"][0]["zone"] == "restricted"));
    let derived = d.config.ccx.lock().await.derived_privacy_zones.clone();
    assert_eq!(
        crate::privacy::records::derived_zone_for_path(&d.config.app.gcx, &log, &derived)
            .unwrap()
            .zone,
        "restricted"
    );
    let ledger: Vec<(PathBuf, crate::privacy::records::DerivedZone)> =
        serde_json::from_slice(&std::fs::read(dir.path().join("privacy_ledger.json")).unwrap())
            .unwrap();
    assert!(ledger
        .iter()
        .any(|(path, zone)| path == &log && zone.zone == "restricted"));
    let artifact: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("calls/1_shell.json")).unwrap())
            .unwrap();
    assert_eq!(artifact["tool"], "shell");
    assert_eq!(artifact["kwargs"]["command"], "make test");
    assert_eq!(artifact["kwargs"]["description"], "run(): make test");
    assert_eq!(artifact["run_kwargs"]["cmd"], "make test");
    assert_eq!(artifact["envelope"]["status"], "exited");
}

#[tokio::test]
async fn run_without_a_cleared_result_of_its_own_ended_process_is_withheld_without_a_log() {
    for end in [
        ShellEnd::Withheld,
        ShellEnd::AskPending,
        ShellEnd::NoReceipt,
        ShellEnd::ObservationIncomplete,
        ShellEnd::ForeignProcess,
        ShellEnd::StillRunning,
        ShellEnd::Detached,
        ShellEnd::TextOnlyAfterStart,
    ] {
        let (mut d, dir) = fixture().await;
        let calls = register_shell_fixture(&mut d, end);
        let outcome = dispatch_run(&d, 1, json!({"cmd":"make test"})).await;
        let value = run_value(&outcome);
        assert_eq!(value["status"], "withheld", "{end:?}: {value}");
        assert_eq!(value["ok"], false, "{end:?}");
        assert_eq!(value["tail"], "", "{end:?}");
        assert_eq!(value["problems"], json!([]), "{end:?}");
        assert_eq!(value["log"], Value::Null, "{end:?}");
        assert!(
            value["error"].as_str().unwrap().contains("withheld")
                || value["error"].as_str().unwrap().contains("detached"),
            "{end:?}: {value}"
        );
        assert!(
            outcome.record.summary.starts_with("withheld · "),
            "{end:?}: {}",
            outcome.record.summary
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1, "{end:?}");
        assert!(!dir.path().join("logs").exists(), "{end:?}");
    }
    let (mut d, dir) = fixture().await;
    register_shell_fixture(&mut d, ShellEnd::RefusedBeforeStart);
    let value = run_value(&dispatch_run(&d, 1, json!({"cmd":"make test"})).await);
    assert_eq!(value["status"], "failed");
    assert_eq!(
        value["error"],
        "the command failed: the workdir is outside the chat's worktree"
    );
    assert!(!dir.path().join("logs").exists());
}

#[tokio::test]
async fn a_log_of_a_shell_run_without_observation_gets_the_strictest_zone_of_the_policy() {
    let (mut d, dir) = fixture().await;
    register_shell_fixture(&mut d, ShellEnd::Unobserved);
    d.config.max_nested_calls = 8;
    let label = |seq: u32| {
        let log = dir.path().join(format!("logs/{seq}_run.log"));
        let derived = d
            .config
            .ccx
            .try_lock()
            .unwrap()
            .derived_privacy_zones
            .clone();
        crate::privacy::records::derived_zone_for_path(&d.config.app.gcx, &log, &derived)
            .map(|zone| zone.zone)
    };
    assert_eq!(
        run_value(&dispatch_run(&d, 1, json!({"cmd":"make test"})).await)["status"],
        "exited"
    );
    assert_eq!(label(1), None);
    let mut policy = (*d.config.app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.zones.push(refact_privacy::Zone {
        name: "internal".into(),
        patterns: vec!["/internal/**".into()],
        send_to: vec!["model".into()],
        on_shell_read: refact_privacy::ShellBehavior::Withhold,
    });
    crate::privacy::save_privacy_policy(d.config.app.gcx.clone(), policy)
        .await
        .unwrap();
    assert_eq!(
        run_value(&dispatch_run(&d, 2, json!({"cmd":"make test"})).await)["status"],
        "exited"
    );
    assert_eq!(label(2).as_deref(), Some("internal"));
}

#[tokio::test]
async fn stop_during_the_run_projection_publishes_no_log() {
    let (d, dir) = fixture().await;
    let registry = d.config.app.runtime.exec_registry.clone();
    let message = shell_result(
        &registry,
        &d.config.chat_id,
        "py_run_1",
        "make test",
        ShellEnd::Exited,
    )
    .await
    .unwrap();
    let log = dir.path().join("logs/1_run.log");
    let staged = || {
        std::fs::read_dir(dir.path().join("logs"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".1_run.log.")
            })
            .count()
    };
    let held = d.config.ccx.lock().await;
    let projection = d.project_run(1, "py_run_1", std::slice::from_ref(&message), 60);
    tokio::pin!(projection);
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while staged() == 0 {
            tokio::select! {
                biased;
                result = &mut projection => panic!("the projection ended before labeling the log: {result:?}"),
                _ = tokio::time::sleep(std::time::Duration::from_millis(5)) => {}
            }
        }
    })
    .await
    .unwrap();
    assert!(std::fs::read(&log).unwrap().is_empty());
    d.config.cancel.cancel();
    drop(held);
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), projection)
        .await
        .unwrap();
    assert_eq!(
        result.unwrap_err(),
        ("RuntimeError", "python run cancelled".to_string(), true)
    );
    assert!(!log.exists());
    assert_eq!(staged(), 0);
}

#[cfg(unix)]
#[tokio::test]
async fn a_run_log_is_published_only_after_the_ledger_folder_is_synced() {
    let (mut d, dir) = fixture().await;
    register_shell_fixture(&mut d, ShellEnd::Exited);
    d.config.max_nested_calls = 8;
    crate::python_tool::workspace::arm_save_fault(&d.config.workspace_dir, "dir_sync_eio");
    let message = exception(
        dispatch_run(&d, 1, json!({"cmd":"make test"})).await,
        "RuntimeError",
    );
    assert!(
        message.contains("its log could not be written") && message.contains("not durable"),
        "{message}"
    );
    assert!(!dir.path().join("logs/1_run.log").exists());
    crate::python_tool::workspace::arm_save_fault(&d.config.workspace_dir, "dir_sync_einval");
    let value = run_value(&dispatch_run(&d, 2, json!({"cmd":"make test"})).await);
    assert_eq!(value["status"], "exited");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("logs/2_run.log")).unwrap(),
        "a\nerror: boom\n"
    );
}

#[tokio::test]
async fn a_run_result_is_fitted_to_the_nested_result_budget_before_its_log_is_published() {
    use refact_python_kernel::convert::{fits_result, json_size, result_nodes_for};
    let (mut d, dir) = fixture().await;
    register_shell_fixture(&mut d, ShellEnd::Long);
    d.config.max_nested_result_bytes = 2048;
    let value = run_value(&dispatch_run(&d, 1, json!({"cmd":"make test","tail":2000})).await);
    let size = json_size(&value);
    assert!(
        fits_result(&size, result_nodes_for(2048), 2048),
        "{} bytes, {} values",
        size.bytes,
        size.nodes
    );
    assert_eq!(value["status"], "exited");
    assert_eq!(value["exit_code"], 3);
    assert!(value["log"].as_str().unwrap().ends_with("1_run.log"));
    assert_eq!(value["problems"], json!(["error: boom"]));
    let tail = value["tail"].as_str().unwrap();
    assert!(tail.ends_with("line 1999\nerror: boom"), "{tail}");
    assert!(tail.lines().count() > 20 && tail.lines().count() < 2000);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("logs/1_run.log"))
            .unwrap()
            .lines()
            .count(),
        2002
    );
}

#[tokio::test]
async fn git_blocking_work_keeps_its_parallel_permit_when_its_waiter_is_dropped() {
    let (d, dir) = fixture().await;
    let repo = git2::Repository::init(dir.path()).unwrap();
    let mut index = repo.index().unwrap();
    for i in 0..4000 {
        let name = format!("files/{i:04}.txt");
        std::fs::create_dir_all(dir.path().join("files")).unwrap();
        std::fs::write(dir.path().join(&name), "1\n").unwrap();
        index.add_path(std::path::Path::new(&name)).unwrap();
    }
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let signature = git2::Signature::now("Test User", "test@example.com").unwrap();
    repo.commit(Some("HEAD"), &signature, &signature, "init", &tree, &[])
        .unwrap();
    for i in 0..4000 {
        std::fs::write(dir.path().join(format!("files/{i:04}.txt")), "2\n").unwrap();
    }
    let d = Arc::new(d);
    let call = tokio::spawn({
        let d = d.clone();
        async move {
            d.dispatch_git(1, json!({"subcommand":"diff"}), None, (None, None))
                .await
        }
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while d.parallel.available_permits() > 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    call.abort();
    let _ = call.await;
    assert_eq!(
        d.parallel.available_permits(),
        0,
        "the blocking work still holds the permit"
    );
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        while d.parallel.available_permits() == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn run_argument_errors_raise_type_error_and_start_nothing() {
    let (mut d, _dir) = fixture().await;
    let calls = register_shell_fixture(&mut d, ShellEnd::Exited);
    d.config.max_nested_calls = 8;
    for (kwargs, text) in [
        (json!({"cmd":""}), "run() cmd must be a non-empty string"),
        (
            json!({"cmd":"x","tail":-1}),
            "run() tail must be an integer between 0 and 2000",
        ),
        (
            json!({"cmd":"x","timeout":0}),
            "run() timeout must be whole seconds between 1 and 3600, or None",
        ),
        (
            json!({"cmd":"x","shell":true}),
            "run() got an unexpected keyword argument 'shell'",
        ),
    ] {
        let outcome = dispatch_run(&d, 1, kwargs).await;
        assert_eq!(outcome.record.tool, "run");
        assert_eq!(exception(outcome, "TypeError"), text);
    }
    Arc::make_mut(&mut d.config.catalog).functions.clear();
    assert!(
        exception(dispatch_run(&d, 2, json!({"cmd":"x"})).await, "TypeError")
            .contains("this chat has no shell tool")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn run_follows_the_shell_rules_approvals_and_authority() {
    let (mut d, _dir) = fixture().await;
    let calls = register_shell_fixture(&mut d, ShellEnd::Exited);
    d.config.max_nested_calls = 8;
    set_mode_rule_for(&mut d, "shell", "deny").await;
    let from_run = exception(
        dispatch_run(&d, 1, json!({"cmd":"make test"})).await,
        "PermissionError",
    );
    let from_shell = exception(
        d.dispatch(
            2,
            "shell",
            json!({"command":"make test","description":"run(): make test"}),
            None,
        )
        .await,
        "PermissionError",
    );
    assert!(from_run.contains("mode policy: deny"), "{from_run}");
    assert_eq!(from_run, from_shell);

    let (mut d, _dir) = fixture().await;
    let denied_calls = register_shell_fixture(&mut d, ShellEnd::Exited);
    d.config.allowed_tools = vec!["cat".into()];
    assert_eq!(
        exception(
            dispatch_run(&d, 1, json!({"cmd":"make test"})).await,
            "PermissionError"
        ),
        "tool is outside allowed tools"
    );
    d.config.allowed_tools.clear();
    d.config.max_nested_calls = 8;
    d.config.ccx.lock().await.child_tool_ceiling =
        Some(crate::subchat::ToolsPolicy::Only(vec!["cat".into()]));
    assert!(exception(
        dispatch_run(&d, 2, json!({"cmd":"make test"})).await,
        "PermissionError"
    )
    .contains("child tool authority"));

    let (mut d, _dir) = fixture().await;
    let asked_calls = register_shell_fixture(&mut d, ShellEnd::Exited);
    set_mode_rule_for(&mut d, "shell", "ask").await;
    let mut session = crate::chat::types::ChatSession::new(d.config.chat_id.clone());
    session.thread = d.config.thread.clone();
    d.config
        .app
        .chat
        .sessions
        .write()
        .await
        .insert(d.config.chat_id.clone(), Arc::new(AMutex::new(session)));
    let _run = d
        .config
        .app
        .python_tool
        .register_run(&d.config.chat_id, "outer", "run");
    let app = d.config.app.clone();
    let chat = d.config.chat_id.clone();
    let waiter = tokio::spawn(async move { dispatch_run(&d, 1, json!({"cmd":"make test"})).await });
    let id = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let session = app.chat.sessions.read().await[&chat].clone();
            if let Some(request) = session.lock().await.runtime.python_approvals.first() {
                break request.request_id.clone();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    app.python_tool
        .approvals
        .decide(
            &app,
            &chat,
            &id,
            refact_chat_api::PythonApprovalDecisionKind::Deny,
        )
        .await
        .unwrap();
    let outcome = waiter.await.unwrap();
    assert_eq!(outcome.record.approval, PythonCallApproval::Denied);
    assert_eq!(
        exception(outcome, "PermissionError"),
        "rejected by the user"
    );
    for counter in [calls, denied_calls, asked_calls] {
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn run_hooks_see_a_shell_call_with_the_command() {
    let target = tempfile::tempdir().unwrap();
    let payload = target.path().join("payload.json");
    let (mut d, _run, _config) = hook_fixture(&format!(
        "cat > '{}'; echo run-denied >&2; exit 2",
        payload.display()
    ))
    .await;
    let calls = register_shell_fixture(&mut d, ShellEnd::Exited);
    assert!(exception(
        dispatch_run(&d, 3, json!({"cmd":"make test"})).await,
        "PermissionError"
    )
    .contains("run-denied"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let value: Value = serde_json::from_slice(&std::fs::read(payload).unwrap()).unwrap();
    assert_eq!(value["tool_name"], "shell");
    assert_eq!(value["tool_input"]["command"], "make test");
    assert_eq!(value["tool_call_id"], "py_run_3");
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn run_refuses_a_hook_that_moves_the_command_to_the_background() {
    let (mut d, run_dir, _config) = hook_fixture(
        "echo '{\"decision\":\"rewrite\",\"tool_input\":{\"command\":\"make test\",\"description\":\"run(): make test\",\"run_in_background\":true}}'",
    )
    .await;
    let calls = register_shell_fixture(&mut d, ShellEnd::Exited);
    assert!(exception(
        dispatch_run(&d, 1, json!({"cmd":"make test"})).await,
        "RuntimeError"
    )
    .contains("turned the command into a background one"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(!run_dir.path().join("logs").exists());
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn run_refuses_a_background_rewrite_before_any_approval_card() {
    let (mut d, run_dir, _config) = hook_fixture(
        "echo '{\"decision\":\"rewrite\",\"tool_input\":{\"command\":\"make test\",\"description\":\"run(): make test\",\"run_in_background\":true}}'",
    )
    .await;
    let calls = register_shell_fixture(&mut d, ShellEnd::Exited);
    set_mode_rule_for(&mut d, "shell", "ask").await;
    let mut session = crate::chat::types::ChatSession::new(d.config.chat_id.clone());
    session.thread = d.config.thread.clone();
    d.config
        .app
        .chat
        .sessions
        .write()
        .await
        .insert(d.config.chat_id.clone(), Arc::new(AMutex::new(session)));
    let _run = d
        .config
        .app
        .python_tool
        .register_run(&d.config.chat_id, "outer", "run");
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        d.dispatch_run(1, json!({"cmd":"make test"}), None, (None, None)),
    )
    .await
    .expect("run() asked to approve a command it can never start");
    assert!(exception(outcome, "RuntimeError").contains("turned the command into a background one"));
    let session = d.config.app.chat.sessions.read().await[&d.config.chat_id].clone();
    assert!(session.lock().await.runtime.python_approvals.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(!run_dir.path().join("logs").exists());
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn run_honours_other_hook_rewrites() {
    let (mut d, run_dir, _config) = hook_fixture(
        "echo '{\"decision\":\"rewrite\",\"tool_input\":{\"command\":\"make check\",\"description\":\"run(): make check\"}}'",
    )
    .await;
    let calls = register_shell_fixture(&mut d, ShellEnd::Exited);
    let outcome = dispatch_run(&d, 1, json!({"cmd":"make test"})).await;
    assert_eq!(run_value(&outcome)["status"], "exited");
    assert!(outcome.record.args_preview.contains("make check"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(run_dir.path().join("logs/1_run.log").exists());
}
