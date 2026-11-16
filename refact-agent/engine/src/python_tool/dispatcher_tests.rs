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
        d.dispatch(1, "fixture", json!({"n":1}), false).await,
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
        d.dispatch(1, "fixture", json!({"n":1}), false).await,
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
    d.dispatch(7, "fixture", json!({"n":1}), false).await;
    let value: Value = serde_json::from_slice(&std::fs::read(payload).unwrap()).unwrap();
    assert_eq!(value["python_run_id"], "run");
    assert_eq!(value["python_parent_tool_call_id"], "outer");
    assert_eq!(value["tool_call_id"], "py_run_7");
}

async fn set_mode_rule(d: &mut NestedDispatcher, action: &str) {
    let mut registry =
        crate::yaml_configs::customization_registry::get_project_registry(d.config.app.gcx.clone())
            .await
            .unwrap();
    let mut mode = registry.modes["agent"].clone();
    mode.id = "nested_fixture".into();
    mode.tools = vec!["fixture".into()];
    mode.tool_confirm.rules = vec![crate::yaml_configs::customization_types::ToolConfirmRule {
        match_pattern: "fixture".into(),
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
        d.dispatch(1, "fixture", json!({"n":1}), false).await,
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
        d.dispatch(1, "fixture", json!({"n":1}), false),
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
            tokio::spawn(async move { d.dispatch(1, "fixture", json!({"n":1}), false).await });
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
        d.dispatch(1, "fixture", json!({"n":1}), false),
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
        (0..4).map(|seq| d.dispatch(seq, "fixture", json!({"n":seq}), false)),
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
        tokio::spawn(async move { d.dispatch(1, "fixture", json!({"n":-30}), false).await });
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
        tokio::spawn(async move { d.dispatch(1, "fixture", json!({"n":-60}), false).await });
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
    let result = d.dispatch(1, "fixture", json!({"n":1}), false).await;
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
    match d.dispatch(1, "fixture", json!({"n":1}), false).await.result {
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
    match d.dispatch(1, "fixture", json!({"n":1}), false).await.result {
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
    d.dispatch(1, "fixture", json!({"n":1}), false).await;
    assert_eq!(session.lock().await.messages.len(), before);
}
#[tokio::test]
async fn unknown_function_raises_runtime_error() {
    let (d, _) = fixture().await;
    assert!(exception(
        d.dispatch(1, "unknown", json!({}), false).await,
        "RuntimeError"
    )
    .contains("unknown function"));
}
#[tokio::test]
async fn truncated_kwargs_raise_value_error() {
    let (d, _) = fixture().await;
    assert_eq!(
        exception(
            d.dispatch(1, "fixture", json!({}), true).await,
            "ValueError"
        ),
        "arguments too large"
    );
}
#[tokio::test]
async fn invalid_kwargs_raise_type_error_with_schema_message() {
    let (d, _) = fixture().await;
    assert!(exception(
        d.dispatch(1, "fixture", json!({"n":"x"}), false).await,
        "TypeError"
    )
    .contains("must be integer"));
}
#[tokio::test]
async fn call_outside_authority_raises_permission_error() {
    let (d, _) = fixture().await;
    exception(
        d.dispatch(1, "fixture", json!({"n":1}), false).await,
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
        d.dispatch(1, "fixture", json!({"n":1}), false).await,
        "PermissionError"
    )
    .contains("child tool authority"));
}

#[tokio::test]
async fn nested_call_budget_exhausted_raises_runtime_error() {
    let (d, _) = fixture().await;
    d.dispatch(1, "fixture", json!({"n":1}), false).await;
    assert!(exception(
        d.dispatch(2, "fixture", json!({"n":1}), false).await,
        "RuntimeError"
    )
    .contains("budget exhausted"));
}
#[tokio::test]
async fn call_file_is_written_to_run_dir() {
    let (d, dir) = fixture().await;
    d.dispatch(1, "fixture", json!({"n":1}), false).await;
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
    let outcome = d.dispatch(1, "fixture", json!({"n":1}), false).await;
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
        d.dispatch(1, "fixture", json!({"n":null}), false),
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
        d.dispatch(1, "fixture", json!({"n":1}), false),
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
        d.dispatch(1, "fixture", json!({"n":1}), false),
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
            d.dispatch(1, "fixture", json!({"n":1}), false),
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
