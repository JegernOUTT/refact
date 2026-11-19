use super::*;
#[test]
fn render_respects_result_char_budget() {
    let extra = json!({"error":{"type":"Error","message":"x".repeat(5000),"traceback":"trace"},"state":{"persisted":true}});
    let text = crate::python_tool::render::render(
        &"é".repeat(10000),
        &"out".repeat(10000),
        &extra,
        "",
        "",
        1000,
    );
    assert!(text.chars().count() <= 1000);
    assert!(text.contains("more chars"));
}

pub(crate) async fn fixture() -> (tempfile::TempDir, AppState, Arc<AMutex<AtCommandsContext>>) {
    fixture_with_secrets_file(None).await
}
/// `secrets_file` points the engine at a custom secrets source (`--secrets-yaml`), so tests
/// never create or load a file named `secrets.yaml`.
async fn fixture_with_secrets_file(
    secrets_file: Option<&str>,
) -> (tempfile::TempDir, AppState, Arc<AMutex<AtCommandsContext>>) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(root.join(".refact")).unwrap();
    let mut gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        dir.path().join("cache"),
        dir.path().join("config"),
    )
    .await;
    if let Some(name) = secrets_file {
        let path = dir.path().join(name);
        Arc::get_mut(&mut gcx)
            .expect("fresh test gcx")
            .cmdline
            .secrets_yaml = path.to_string_lossy().into_owned();
    }
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root];
    let mut app = AppState::from_gcx(gcx).await;
    app.python_tool = Arc::new(crate::python_tool::PythonToolRuntime {
        registry: crate::python_tool::registry::KernelRegistry::new(
            crate::python_tool::registry::WorkerBackend::InThread,
        ),
        ..Default::default()
    });
    let session = Arc::new(AMutex::new(crate::chat::types::ChatSession::new(
        "python-test".into(),
    )));
    app.chat
        .sessions
        .write()
        .await
        .insert("python-test".into(), session);
    let ccx = Arc::new(AMutex::new(
        AtCommandsContext::new_from_app(
            app.clone(),
            16384,
            20,
            false,
            vec![],
            "python-test".into(),
            None,
            "model".into(),
            None,
            None,
        )
        .await,
    ));
    (dir, app, ccx)
}
async fn run(ccx: Arc<AMutex<AtCommandsContext>>, code: &str, restart: bool) -> ChatMessage {
    let (_, messages) = ToolPython::default()
        .tool_execute(
            ccx,
            &uuid::Uuid::new_v4().to_string(),
            &HashMap::from([
                ("code".into(), json!(code)),
                ("restart".into(), json!(restart)),
            ]),
        )
        .await
        .unwrap();
    match messages.into_iter().next().unwrap() {
        ContextEnum::ChatMessage(m) => m,
        _ => panic!("expected message"),
    }
}
#[tokio::test]
async fn runs_code_and_returns_result_text_and_extra() {
    let (_dir, _app, ccx) = fixture().await;
    let m = run(ccx, "print('hello')\n1 + 2", false).await;
    assert_eq!(m.tool_failed, Some(false));
    assert!(m.content.content_text_only().contains("Result:\n3"));
    assert!(m.content.content_text_only().contains("hello"));
    assert_eq!(m.extra["python"]["status"], "completed");
}
#[tokio::test]
async fn exception_marks_tool_failed_with_traceback() {
    let (_dir, _app, ccx) = fixture().await;
    let m = run(ccx, "1 / 0", false).await;
    assert_eq!(m.tool_failed, Some(true));
    assert_eq!(m.extra["python"]["error"]["type"], "ZeroDivisionError");
    assert!(!m.extra["python"]["error"]["traceback"]
        .as_str()
        .unwrap()
        .is_empty());
}
#[tokio::test]
async fn print_only_run_gets_the_unused_result_budget() {
    let (_dir, _app, ccx) = fixture().await;
    let m = run(ccx, "print('x' * 50000)", false).await;
    let text = m.content.content_text_only();
    assert!(text.chars().count() <= 20_000, "{}", text.chars().count());
    assert!(
        text.matches('x').count() >= 19_000,
        "{}",
        text.matches('x').count()
    );
}
#[tokio::test]
async fn long_output_never_pushes_out_error_and_state_lines() {
    let (dir, _app, ccx) = fixture().await;
    std::fs::write(
        dir.path().join("project/.refact/coordination.yaml"),
        "python_max_result_chars: 1000\npython_max_state_bytes: 1048576\n",
    )
    .unwrap();
    let message = format!("boom {}", "z".repeat(400));
    let m = run(
        ccx,
        &format!("big = 'b' * 2000000\nprint('y' * 50000)\nraise ValueError('{message}')"),
        false,
    )
    .await;
    let text = m.content.content_text_only();
    assert!(text.chars().count() <= 1000, "{}", text.chars().count());
    assert!(
        text.starts_with(&format!(
            "Error: ValueError: {message}\nState: not saved (state is 2.0 MB, over the limit 1.0 MB); variables stay in memory until this session is restarted or evicted; set large variables to None and the next run saves again\n\nPrinted output:\nyyy"
        )),
        "{text}"
    );
}
#[tokio::test]
async fn display_notes_count_in_the_result_budget() {
    let (dir, _app, ccx) = fixture().await;
    std::fs::write(
        dir.path().join("project/.refact/coordination.yaml"),
        "python_max_result_chars: 1000\n",
    )
    .unwrap();
    let m = run(
        ccx,
        "for i in range(14):\n    show([{'a': i}], title=f'table {i}')\nprint('y' * 50000)\n'r' * 50000",
        false,
    )
    .await;
    let text = m.content.content_text_only();
    assert!(
        text.chars().count() <= 1000,
        "{}: {text}",
        text.chars().count()
    );
    assert!(text.contains("shown to the user: table"), "{text}");
    assert!(text.contains("Printed output:\nyyy"), "{text}");
}
#[tokio::test]
async fn results_show_non_json_values_readably_without_a_limit_note() {
    let (_dir, _app, ccx) = fixture().await;
    let m = run(
        ccx,
        "from pathlib import Path\nimport datetime\n[Path('a/b'), {1, 2}, b'abc', {1: 'a'}, datetime.date(2024, 1, 2)]",
        false,
    )
    .await;
    let text = m.content.content_text_only();
    for part in [
        "PosixPath('a/b')",
        "[\n    1,\n    2\n  ]",
        "b'abc'",
        "{1: 'a'}",
        "datetime.date(2024, 1, 2)",
    ] {
        assert!(text.contains(part), "{part}: {text}");
    }
    assert!(!text.contains("over the result limit"), "{text}");
}
#[tokio::test]
async fn oversized_helper_arguments_report_the_measured_size() {
    let (dir, _app, ccx) = fixture().await;
    std::fs::write(
        dir.path().join("project/.refact/coordination.yaml"),
        "python_max_nested_result_bytes: 65536\n",
    )
    .unwrap();
    let m = run(ccx, "help('x' * 100000)", false).await;
    let message = m.extra["python"]["error"]["message"].as_str().unwrap();
    assert!(
        message.starts_with("help() arguments are too large to send: at least "),
        "{message}"
    );
    assert!(
        message.ends_with(" bytes, over the 65536-byte limit; pass a file path or a smaller value"),
        "{message}"
    );
}
#[tokio::test]
async fn a_refused_binding_fails_the_run_and_keeps_the_session() {
    let (_dir, _app, ccx) = fixture().await;
    run(ccx.clone(), "x = 41", false).await;
    let m = run(ccx.clone(), "print('ran')\nPROJECT_ROOT = 'x'", false).await;
    assert_eq!(m.extra["python"]["status"], "failed");
    assert_eq!(m.extra["python"]["error"]["type"], "SyntaxError");
    assert_eq!(m.extra["python"]["error"]["line"], 2);
    assert!(m.extra["python"]["error"]["message"]
        .as_str()
        .unwrap()
        .starts_with("line 2 binds 'PROJECT_ROOT', a constant set before every run."));
    assert_eq!(m.extra["python"]["stdout_preview"], "");
    let m = run(ccx, "x + 1", false).await;
    assert!(m.content.content_text_only().contains("Result:\n42"));
}
#[tokio::test]
async fn known_interpreter_gaps_get_a_hint_under_the_error() {
    let (_dir, _app, ccx) = fixture().await;
    let m = run(ccx.clone(), "x = 1\ndel x", false).await;
    let hint = m.extra["python"]["error"]["hint"].as_str().unwrap();
    assert!(hint.starts_with("del is not supported"), "{hint}");
    let text = m.content.content_text_only();
    assert!(
        text.starts_with("Error: NotImplementedError: ")
            && text.contains(&format!("'del' statement\nHint: {hint}")),
        "{text}"
    );
    let m = run(ccx, "1 / 0", false).await;
    assert!(m.extra["python"]["error"].get("hint").is_none());
    assert!(!m.content.content_text_only().contains("Hint:"));
}
#[tokio::test]
async fn variables_persist_between_runs() {
    let (_dir, _app, ccx) = fixture().await;
    run(ccx.clone(), "x = 41", false).await;
    let m = run(ccx, "x + 1", false).await;
    assert!(m.content.content_text_only().contains("Result:\n42"));
}
#[tokio::test]
async fn restart_clears_variables_but_keeps_artifacts() {
    let (_dir, _app, ccx) = fixture().await;
    let m = run(
        ccx.clone(),
        "x = 41\nfrom pathlib import Path\nPath(ARTIFACTS_DIR + '/keep.txt').write_text('kept')",
        false,
    )
    .await;
    assert_eq!(m.tool_failed, Some(false));
    let m = run(ccx.clone(), "x", true).await;
    assert_eq!(m.extra["python"]["error"]["type"], "NameError");
    let m = run(
        ccx,
        "from pathlib import Path\nPath(ARTIFACTS_DIR + '/keep.txt').read_text()",
        false,
    )
    .await;
    assert!(m.content.content_text_only().contains("kept"));
}
#[tokio::test]
async fn disabled_for_project_returns_clear_error() {
    let (dir, _app, ccx) = fixture().await;
    std::fs::write(
        dir.path().join("project/.refact/coordination.yaml"),
        "python_tool_enabled: false\n",
    )
    .unwrap();
    let e = ToolPython::default()
        .tool_execute(
            ccx,
            &"disabled".into(),
            &HashMap::from([("code".into(), json!("1"))]),
        )
        .await
        .err()
        .unwrap();
    assert!(e.contains("disabled"));
}

struct FixtureTool {
    serial: bool,
}
#[async_trait]
impl Tool for FixtureTool {
    async fn tool_execute(
        &mut self,
        _: Arc<AMutex<AtCommandsContext>>,
        id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        if let Some(path) = args.get("started").and_then(Value::as_str) {
            tokio::fs::write(path, "started").await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(
            args.get("ms").and_then(Value::as_u64).unwrap_or(30),
        ))
        .await;
        if let Some(path) = args.get("finished").and_then(Value::as_str) {
            tokio::fs::write(path, "finished").await.unwrap();
        }
        let mut m = ChatMessage {
            role: "tool".into(),
            tool_call_id: id.clone(),
            content: ChatContent::SimpleText("fixture output".into()),
            ..Default::default()
        };
        m.extra.insert("tool_output".into(), json!({"answer":42}));
        if args.get("images") == Some(&Value::Bool(true)) {
            m.content = ChatContent::Multimodal(vec![crate::call_validation::MultimodalElement {
                m_type: "image/png".into(),
                m_content: args.get("ms").map(Value::to_string).unwrap_or_default(),
            }]);
        }
        if let Some(privacy) = args.get("privacy") {
            m.extra.insert("privacy".into(), privacy.clone());
        }
        if let Some(observation) = args.get("observation") {
            m.extra
                .insert("privacy_observation".into(), observation.clone());
        }
        Ok((false, vec![ContextEnum::ChatMessage(m)]))
    }
    fn tool_description(&self) -> ToolDesc {
        let mut d = ToolPython::default().tool_description();
        d.name = if self.serial {
            "serial_fixture"
        } else {
            "fixture"
        }
        .into();
        d.allow_parallel = !self.serial;
        d.input_schema = json!({"type":"object","properties":{"ms":{"type":"integer"},"images":{"type":"boolean"},"privacy":{"type":"object"},"observation":{"type":"object"},"started":{"type":"string"},"finished":{"type":"string"}}});
        d
    }
}
pub(crate) fn with_tools(mut app: AppState) -> AppState {
    app.tool_registry = Arc::new(
        crate::app_state::AppToolRegistry::with_fixture_tool_factory(
            app.gcx.clone(),
            Arc::new(|| {
                vec![
                    Box::new(ToolPython::default()),
                    Box::new(FixtureTool {
                        serial: false,
                    }),
                    Box::new(FixtureTool {
                        serial: true,
                    }),
                ]
            }),
        ),
    );
    app
}
pub(crate) fn call(id: &str, code: &str, background: bool) -> crate::call_validation::ChatToolCall {
    serde_json::from_value(json!({"id":id,"type":"function","function":{"name":"python","arguments":json!({"code":code,"run_in_background":background}).to_string()}})).unwrap()
}
pub(crate) async fn execute(
    app: AppState,
    calls: &[crate::call_validation::ChatToolCall],
) -> Vec<ChatMessage> {
    let thread = app
        .chat
        .sessions
        .read()
        .await
        .get("python-test")
        .unwrap()
        .lock()
        .await
        .thread
        .clone();
    tokio::time::timeout(
        Duration::from_secs(20),
        crate::chat::tools::execute_tools(
            app,
            calls,
            &[],
            &thread,
            "agent",
            Some("model"),
            crate::chat::tools::ExecuteToolsOptions {
                structured_output: true,
                ..Default::default()
            },
        ),
    )
    .await
    .expect("python fixture execution exceeded 20 seconds")
    .0
}
#[tokio::test]
async fn nested_call_result_is_usable_in_script() {
    let (_dir, app, _) = fixture().await;
    let app = with_tools(app);
    let m = execute(
        app,
        &[call(
            "nested",
            "r = await fixture()\nr['data']['answer'] + 1",
            false,
        )],
    )
    .await;
    assert!(
        m[0].content.content_text_only().contains("43"),
        "{:?}",
        m[0].content
    );
    assert_eq!(m[0].extra["python"]["calls_total"], 1);
}
#[tokio::test]
async fn gather_runs_nested_calls_in_parallel() {
    let (_dir, app, _) = fixture().await;
    let m = execute(
        with_tools(app),
        &[call(
            "gather",
            "import asyncio\nawait asyncio.gather(fixture(ms=200), fixture(ms=200))",
            false,
        )],
    )
    .await;
    let calls = m[0].extra["python"]["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 2);
    let a = calls[0]["started_at_ms"].as_u64().unwrap();
    let b = calls[1]["started_at_ms"].as_u64().unwrap();
    assert!(a.abs_diff(b) < 150);
    assert_eq!(m[0].tool_failed, Some(false));
}
#[tokio::test]
async fn two_calls_in_one_message_run_in_order() {
    let (_dir, app, _) = fixture().await;
    let m = execute(
        with_tools(app),
        &[call("first", "x = 41", false), call("second", "x+1", false)],
    )
    .await;
    assert_eq!(m.len(), 2);
    assert!(m[1].content.content_text_only().contains("42"));
}
#[tokio::test]
async fn python_job_holds_scope_but_does_not_hold_dispatch_gate() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let (dir, app, _) = fixture().await;
        let app = with_tools(app);
        let messages = execute(
            app.clone(),
            &[call(
                "serial",
                "r=await serial_fixture(ms=100)\nr['data']['answer']",
                true,
            )],
        )
        .await;
        let job = messages[0].extra["tool_job"]["job_id"].as_str().unwrap();
        let snapshot = app.runtime.tool_jobs.get("python-test", job).unwrap();
        let outer = snapshot.receipt.binding;
        let root = dunce::canonicalize(dir.path().join("project")).unwrap();
        assert!(outer
            .scope_keys
            .contains(&app.runtime.tool_jobs.chat_scope_key("python-test")));
        assert!(outer
            .scope_keys
            .contains(&crate::tool_jobs::ScopeKey::Worktree {
                project: root.clone(),
                root
            }));
        assert!(outer
            .scope_keys
            .contains(&crate::tool_jobs::ScopeKey::PythonRun {
                chat_id: "python-test".into(),
                tool_call_id: "serial".into()
            }));
        let terminal = app
            .runtime
            .tool_jobs
            .wait_terminal("python-test", job)
            .await
            .unwrap();
        let result = terminal.result.unwrap();
        assert_eq!(result.messages[0].tool_failed, Some(false));
        assert!(result.messages[0]
            .content
            .content_text_only()
            .contains("42"));
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn run_in_background_returns_receipt_immediately() {
    let (_dir, app, _) = fixture().await;
    let app = with_tools(app);
    let t = std::time::Instant::now();
    let m = execute(
        app.clone(),
        &[call("background", "await fixture(ms=5000)", true)],
    )
    .await;
    assert!(t.elapsed() < Duration::from_secs(3));
    let id = m[0].extra["tool_job"]["job_id"].as_str().unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        app.runtime.tool_jobs.wait_terminal("python-test", id),
    )
    .await
    .expect("job exceeded 20 seconds")
    .unwrap();
    assert!(result.result.unwrap().messages[0]
        .extra
        .contains_key("python"));
}
#[tokio::test]
async fn detaches_after_inline_budget_and_result_is_delivered() {
    let (_dir, app, _) = fixture().await;
    let app = with_tools(app);
    app.runtime
        .tool_jobs
        .set_test_inline_budget(Some(Duration::from_millis(50)));
    let m = execute(
        app.clone(),
        &[call("detach", "await fixture(ms=300)", false)],
    )
    .await;
    let id = m[0].extra["tool_job"]["job_id"].as_str().unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        app.runtime.tool_jobs.wait_terminal("python-test", id),
    )
    .await
    .expect("job exceeded 20 seconds")
    .unwrap();
    assert_eq!(
        result.result.unwrap().messages[0].extra["python"]["background"],
        true
    );
}
#[tokio::test]
async fn second_run_while_background_run_is_refused_with_job_id() {
    let (_dir, app, _) = fixture().await;
    let app = with_tools(app);
    let m = execute(
        app.clone(),
        &[call("background", "await fixture(ms=1000)", true)],
    )
    .await;
    let id = m[0].extra["tool_job"]["job_id"]
        .as_str()
        .unwrap()
        .to_owned();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let m = execute(app.clone(), &[call("refused", "1", false)]).await;
    assert!(m[0].content.content_text_only().contains(&id));
    tokio::time::timeout(
        Duration::from_secs(20),
        app.runtime.tool_jobs.wait_terminal("python-test", &id),
    )
    .await
    .expect("background job exceeded 20 seconds")
    .unwrap();
}
#[tokio::test]
async fn child_chat_gets_parent_artifacts_dir() {
    let (_dir, app, ccx) = fixture().await;
    app.chat
        .sessions
        .read()
        .await
        .get("python-test")
        .unwrap()
        .lock()
        .await
        .thread
        .parent_id = Some("parent".into());
    app.chat.sessions.read().await["python-test"]
        .lock()
        .await
        .thread
        .link_type = Some("subagent".into());
    app.chat.sessions.write().await.insert(
        "parent".into(),
        Arc::new(AMutex::new(crate::chat::types::ChatSession::new(
            "parent".into(),
        ))),
    );
    let m = run(ccx.clone(), "PARENT_ARTIFACTS_DIR", false).await;
    assert!(m.content.content_text_only().contains("parent/artifacts"));
    let parent = PythonWorkspace::for_chat(&app, "parent").await.unwrap();
    parent.ensure_dirs().await.unwrap();
    let artifact = parent.artifacts_dir().join("derived.txt");
    std::fs::write(&artifact, "parent secret").unwrap();
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.zones.insert(
        0,
        refact_privacy::Zone {
            name: "parent-restricted".into(),
            patterns: vec!["source-only.txt".into()],
            send_to: vec!["trusted".into()],
            ..Default::default()
        },
    );
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
    std::fs::write(
        parent.dir().join("privacy_ledger.json"),
        serde_json::to_vec(&vec![(
            artifact.clone(),
            crate::privacy::records::DerivedZone {
                zone: "parent-restricted".into(),
                origin: Some("source-only.txt".into()),
            },
        )])
        .unwrap(),
    )
    .unwrap();
    ccx.lock().await.derived_privacy_zones = crate::privacy::records::new_derived_privacy_zones();
    let message = run(
        ccx.clone(),
        "open(PARENT_ARTIFACTS_DIR + '/derived.txt').read()",
        false,
    )
    .await;
    assert_eq!(message.tool_failed, Some(false), "{message:?}");
    assert!(message.extra["privacy"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|record| record["zone"] == "parent-restricted"));
    let derived = ccx.lock().await.derived_privacy_zones.clone();
    assert_eq!(
        crate::privacy::records::derived_zone_for_path(&app.gcx, &artifact, &derived)
            .unwrap()
            .zone,
        "parent-restricted"
    );
}

#[tokio::test]
async fn runtime_snapshot_lists_active_run() {
    let (_dir, app, _) = fixture().await;
    let app = with_tools(app);
    let worker_app = app.clone();
    let worker = tokio::spawn(async move {
        execute(worker_app, &[call("live", "await fixture(ms=500)", false)]).await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let session = app
                .chat
                .sessions
                .read()
                .await
                .get("python-test")
                .unwrap()
                .clone();
            if !session.lock().await.runtime.python_runs.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    worker.await.unwrap();
    assert!(app
        .chat
        .sessions
        .read()
        .await
        .get("python-test")
        .unwrap()
        .lock()
        .await
        .runtime
        .python_runs
        .is_empty());
}
#[tokio::test]
async fn stop_control_cancels_running_job_and_turn_continues() {
    let (_dir, app, _) = fixture().await;
    let app = with_tools(app);
    let worker_app = app.clone();
    let worker = tokio::spawn(async move {
        execute(worker_app, &[call("stop", "await fixture(ms=2000)", false)]).await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if app.python_tool.stop_run("python-test", "stop").is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let messages = worker.await.unwrap();
    assert_eq!(messages[0].extra["python"]["status"], "cancelled");
    assert!(!app
        .chat
        .sessions
        .read()
        .await
        .get("python-test")
        .unwrap()
        .lock()
        .await
        .abort_flag
        .load(Ordering::Acquire));
    let next = execute(app, &[call("after-stop", "42", false)]).await;
    assert_eq!(next[0].tool_failed, Some(false));
}
#[tokio::test]
async fn run_timeout_reports_timed_out() {
    let (dir, app, _) = fixture().await;
    std::fs::write(
        dir.path().join("project/.refact/coordination.yaml"),
        "python_max_run_ms: 1000\n",
    )
    .unwrap();
    let m = execute(
        with_tools(app),
        &[call("timeout", "await fixture(ms=2000)", false)],
    )
    .await;
    assert_eq!(m[0].extra["python"]["status"], "timed_out");
    assert_eq!(m[0].tool_failed, Some(true));
}
#[tokio::test]
async fn progress_events_emitted_for_nested_calls() {
    let (_dir, app, _) = fixture().await;
    let app = with_tools(app);
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get("python-test")
        .unwrap()
        .clone();
    let mut events = session.lock().await.event_tx.subscribe();
    execute(app, &[call("progress", "await fixture(ms=100)", false)]).await;
    let mut kinds = Vec::new();
    while let Ok(e) = events.try_recv() {
        kinds.push(e.to_string());
    }
    let events = kinds.join("\n");
    assert!(events.contains("call_started"));
    assert!(events.contains("call_finished"));
    assert!(events.contains("finished"));
}

#[tokio::test]
async fn project_root_is_the_chat_worktree_when_scoped() {
    let (dir, _app, ccx) = fixture().await;
    let root = dir.path().join("worktree");
    std::fs::create_dir_all(&root).unwrap();
    let source = dir.path().join("project");
    let meta = crate::worktrees::types::WorktreeMeta {
        id: "python-worktree".into(),
        kind: "chat".into(),
        root: root.clone(),
        source_workspace_root: source.clone(),
        repo_root: source,
        branch: Some("feature".into()),
        base_branch: Some("main".into()),
        base_commit: Some("base".into()),
        task_id: None,
        card_id: None,
        agent_id: None,
        enforce: true,
    };
    ccx.lock().await.execution_scope = Some(
        crate::worktrees::scope::ExecutionScope::from_worktree(&meta),
    );
    let m = run(ccx, "PROJECT_ROOT", false).await;
    let expected = python_path(&dunce::canonicalize(&root).unwrap(), native_platform());
    assert!(m.content.content_text_only().contains(&expected));
}

#[tokio::test]
async fn worker_loss_reports_failure_and_next_run_reports_reset() {
    let (_dir, app, ccx) = fixture().await;
    run(ccx.clone(), "x = 42", false).await;
    app.python_tool
        .registry
        .lose_worker_for_test("python-test")
        .await;
    let failed = run(ccx.clone(), "x", false).await;
    assert_eq!(failed.tool_failed, Some(true));
    let restored = run(ccx, "x", false).await;
    assert_eq!(restored.extra["python"]["state"]["restored"], true);
    assert!(restored.extra["python"]["state"]["reset_reason"].is_string());
    assert!(restored.content.content_text_only().contains("42"));
}

#[tokio::test]
async fn protected_path_rules_cover_engine_configuration() {
    let (dir, app, _) = fixture().await;
    let root = dir.path().join("project");
    let ws = PythonWorkspace::for_chat(&app, "python-test")
        .await
        .unwrap();
    let (write, read) = protected_paths(&app, &root, &ws).await;
    assert!(write.files.contains(&root.join(".refact/hooks.yaml")));
    assert!(write.files.contains(
        &dunce::canonicalize(root.join(".refact"))
            .unwrap()
            .join("privacy.yaml")
    ));
    assert!(write.prefixes.contains(
        &dunce::canonicalize(root.join(".refact"))
            .unwrap()
            .join("integrations.d")
    ));
    assert!(write.prefixes.contains(&app.paths.config_dir));
    assert!(read
        .files
        .contains(&app.paths.config_dir.join("python_state.key")));
}

#[tokio::test]
async fn configure_environment_equals_shell_environment() {
    let (_dir, app, ccx) = fixture().await;
    std::fs::write(
        app.paths.config_dir.join("variables.yaml"),
        "PYTHON_ENV_FIXTURE: parity\n",
    )
    .unwrap();
    let mut errors = vec![];
    let replacements =
        crate::integrations::setting_up_integrations::get_public_vars_for_replacements(
            app.gcx.clone(),
            &mut errors,
        )
        .await;
    let expected = refact_exec::build_child_env(
        &refact_exec::ExecEnvPolicy::Scrubbed {
            passthrough: app.gcx.terminal_security_config.env_passthrough.clone(),
        },
        &replacements,
    );
    let message = run(ccx, "import os\ndict(os.environ)", false).await;
    let result_path = std::path::Path::new(message.extra["python"]["run_dir"].as_str().unwrap())
        .join("result.json");
    let actual: std::collections::HashMap<String, String> =
        serde_json::from_slice(&std::fs::read(result_path).unwrap()).unwrap();
    assert_eq!(actual, expected.into_iter().collect());
}

#[tokio::test]
async fn nested_catalog_does_not_preapprove_mode_ask() {
    use refact_chat_api::PythonApprovalDecisionKind;
    let (_dir, app, _) = fixture().await;
    let app = with_tools(app);
    let mut registry =
        crate::yaml_configs::customization_registry::get_project_registry(app.gcx.clone())
            .await
            .unwrap();
    registry.modes.get_mut("agent").unwrap().tool_confirm.rules =
        vec![crate::yaml_configs::customization_types::ToolConfirmRule {
            match_pattern: "fixture".into(),
            action: "ask".into(),
        }];
    let root = crate::files_correction::get_project_dirs(app.gcx.clone())
        .await
        .remove(0);
    app.gcx
        .project_registry_cache
        .write()
        .unwrap()
        .insert(root, registry);
    app.gcx.tool_catalog_generations.advance_customization();
    let task = {
        let app = app.clone();
        tokio::spawn(async move { execute(app, &[call("ask", "await fixture()", false)]).await })
    };
    let request = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let session = app.chat.sessions.read().await["python-test"].clone();
            if let Some(request) = session.lock().await.runtime.python_approvals.first() {
                break request.request_id.clone();
            }
            assert!(
                !task.is_finished(),
                "catalog must not auto-approve nested calls"
            );
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    app.python_tool
        .approvals
        .decide(
            &app,
            "python-test",
            &request,
            PythonApprovalDecisionKind::AllowOnce,
        )
        .await
        .unwrap();
    assert_eq!(task.await.unwrap()[0].tool_failed, Some(false));
}

#[tokio::test]
async fn outer_message_merges_nested_and_direct_read_provenance() {
    let (dir, app, ccx) = fixture().await;
    let file = dir.path().join("project/secret.txt");
    std::fs::write(&file, "secret").unwrap();
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.blocked = vec![file.to_string_lossy().into_owned()];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
    let app = with_tools(app);
    ccx.lock().await.app = app.clone();
    let message = run(
        ccx,
        "from pathlib import Path\nx = Path('secret.txt').read_text()\nr = await fixture(privacy={'files':[{'path':'nested.txt','zone':'blocked','attribution':'observed'}]})\nx",
        false,
    )
    .await;
    let files = message.extra["privacy"]["files"].as_array().unwrap();
    assert!(files.iter().any(|f| f["path"] == "nested.txt"));
    let expected = dunce::canonicalize(&file).unwrap();
    assert!(files.iter().any(|f| f["path"]
        .as_str()
        .is_some_and(|p| dunce::canonicalize(p).ok().as_ref() == Some(&expected))));
}

#[tokio::test]
async fn session_taint_survives_restart_via_state_json() {
    let (dir, app, ccx) = fixture().await;
    let file = dir.path().join("project/secret.txt");
    std::fs::write(&file, "secret").unwrap();
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.blocked = vec![file.to_string_lossy().into_owned()];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
    run(
        ccx.clone(),
        "from pathlib import Path\nx = Path('secret.txt').read_text()",
        false,
    )
    .await;
    app.python_tool.registry.drop_chat("python-test").await;
    let message = run(ccx, "x", false).await;
    assert_eq!(message.extra["privacy"]["files"][0]["zone"], "blocked");
}

#[tokio::test]
async fn direct_writes_register_derived_privacy_zones() {
    let (dir, app, ccx) = fixture().await;
    let file = dir.path().join("project/secret.txt");
    std::fs::write(&file, "secret").unwrap();
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.blocked = vec![file.to_string_lossy().into_owned()];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
    run(
        ccx.clone(),
        "from pathlib import Path\nx = Path('secret.txt').read_text()",
        false,
    )
    .await;
    run(ccx.clone(), "Path('derived.txt').write_text(x)", false).await;
    let derived = ccx.lock().await.derived_privacy_zones.clone();
    assert_eq!(
        crate::privacy::records::derived_zone_for_path(
            &app.gcx,
            &dir.path().join("project/derived.txt"),
            &derived
        )
        .unwrap()
        .zone,
        "blocked"
    );
    let workspace = PythonWorkspace::for_chat(&app, "python-test")
        .await
        .unwrap();
    assert!(workspace.dir().join("privacy_ledger.json").exists());
    let message = run(ccx, "Path('derived.txt').write_text(x * 50000)", false).await;
    assert_eq!(message.tool_failed, Some(false), "{message:?}");
    let run_dir = workspace.run_dir(message.extra["python"]["run_id"].as_str().unwrap());
    let files = crate::python_tool::run_output::run_data_files(&run_dir)
        .await
        .unwrap();
    assert!(files
        .iter()
        .any(|path| path.parent() == Some(run_dir.join("captures").as_path())));
    assert!(files.iter().any(|path| path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("diff-")));
    let ledger: Vec<(std::path::PathBuf, crate::privacy::records::DerivedZone)> =
        serde_json::from_slice(
            &std::fs::read(workspace.dir().join("privacy_ledger.json")).unwrap(),
        )
        .unwrap();
    for path in files {
        assert_eq!(
            ledger
                .iter()
                .find(|(entry, _)| entry == &path)
                .unwrap()
                .1
                .zone,
            "blocked",
            "{}",
            path.display()
        );
    }
}

#[tokio::test]
async fn files_written_carry_diffs_within_budget() {
    let (_dir, _app, ccx) = fixture().await;
    let message = run(
        ccx,
        "from pathlib import Path\nPath('new.txt').write_text('new text\\n')",
        false,
    )
    .await;
    let diffs = &message.extra["python"]["files_written"][0]["diffs"];
    assert_eq!(diffs[0]["file_action"], "add");
    assert!(serde_json::to_vec(diffs).unwrap().len() <= 256 * 1024);
}

#[tokio::test]
async fn images_from_nested_calls_are_attached_in_order() {
    let (_dir, app, _) = fixture().await;
    let messages = execute(with_tools(app), &[call("images", "import asyncio\nawait asyncio.gather(fixture(ms=80, images=True), fixture(ms=10, images=True))", false)]).await;
    let ChatContent::Multimodal(parts) = &messages[0].content else {
        panic!("missing multimodal parts");
    };
    assert_eq!(parts[0].m_type, "text");
    let run_dir = std::path::Path::new(messages[0].extra["python"]["run_dir"].as_str().unwrap());
    let mut indexed = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(run_dir.join("calls")).unwrap() {
        let value: Value =
            serde_json::from_slice(&std::fs::read(entry.unwrap().path()).unwrap()).unwrap();
        indexed.insert(
            value["envelope"]["images"][0]["index"].as_u64().unwrap(),
            value["kwargs"]["ms"].to_string(),
        );
    }
    assert_eq!(
        parts
            .iter()
            .skip(1)
            .map(|p| p.m_content.clone())
            .collect::<Vec<_>>(),
        indexed.into_values().collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn nested_tool_outside_slash_command_authority_is_refused() {
    let (_dir, app, _) = fixture().await;
    app.chat.sessions.read().await["python-test"]
        .lock()
        .await
        .active_command
        .allowed_tools = vec!["python".into(), "cat".into()];
    let messages = execute(
        with_tools(app),
        &[call(
            "slash",
            "try:\n    await fixture()\nexcept PermissionError:\n    answer = 42\nanswer",
            false,
        )],
    )
    .await;
    assert_eq!(messages[0].tool_failed, Some(false));
    assert!(messages[0].content.content_text_only().contains("42"));
    assert_eq!(messages[0].extra["python"]["calls"][0]["status"], "denied");
}

#[tokio::test]
async fn nested_shell_needs_confirmation_requests_approval() {
    let (_dir, mut app, _) = fixture().await;
    app.tool_registry = Arc::new(
        crate::app_state::AppToolRegistry::with_fixture_tool_factory(
            app.gcx.clone(),
            Arc::new(|| {
                vec![
                    Box::new(ToolPython::default()),
                    Box::new(crate::tools::tool_shell::ToolShell {
                        cfg: Default::default(),
                        config_path: String::new(),
                    }),
                ]
            }),
        ),
    );
    let task = {
        let app = app.clone();
        tokio::spawn(async move {
            execute(app, &[call("shell-ask", "await shell(command='echo safe', description='test approval', needs_confirmation=True)", false)]).await
        })
    };
    let id = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let session = app.chat.sessions.read().await["python-test"].clone();
            if let Some(request) = session.lock().await.runtime.python_approvals.first() {
                break request.request_id.clone();
            }
            assert!(!task.is_finished(), "shell confirmation was bypassed");
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    app.python_tool
        .approvals
        .decide(
            &app,
            "python-test",
            &id,
            refact_chat_api::PythonApprovalDecisionKind::Deny,
        )
        .await
        .unwrap();
    assert_eq!(
        task.await.unwrap()[0].extra["python"]["calls"][0]["status"],
        "denied"
    );
}

#[tokio::test]
async fn invalid_python_call_releases_reserved_turn_promptly() {
    let (_dir, app, _) = fixture().await;
    let app = with_tools(app);
    // A leaked ticket keeps "valid" off the turn until it detaches; the budget only bounds that failure.
    app.runtime
        .tool_jobs
        .set_test_inline_budget(Some(Duration::from_secs(15)));
    let messages = execute(
        app,
        &[call("invalid", "", false), call("valid", "42", false)],
    )
    .await;
    assert_eq!(
        messages[1]
            .extra
            .get("python")
            .and_then(|p| p.get("status")),
        Some(&json!("completed")),
        "{messages:?}"
    );
}
#[tokio::test]
async fn secrets_are_not_in_python_environment() {
    let (dir, app, ccx) = fixture_with_secrets_file(Some("private-values.yaml")).await;
    std::fs::write(
        app.paths.config_dir.join("variables.yaml"),
        "PYCM_PUBLIC: visible\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("private-values.yaml"),
        "PYCM_SECRET: hidden\n",
    )
    .unwrap();
    let m = run(
        ccx,
        "import os\n[os.environ.get('PYCM_PUBLIC'), os.environ.get('PYCM_SECRET')]",
        false,
    )
    .await;
    assert!(m.content.content_text_only().contains("visible"));
    assert!(!m.content.content_text_only().contains("hidden"));
}
#[tokio::test]
async fn saved_stdout_matches_byte_count_and_output_limit() {
    let (_dir, _app, ccx) = fixture().await;
    for (size, status) in [(900_000, "completed"), (1_100_000, "limit_exceeded")] {
        let m = run(ccx.clone(), &format!("print('x' * {size})"), false).await;
        assert_eq!(m.extra["python"]["status"], status);
        let path =
            std::path::Path::new(m.extra["python"]["run_dir"].as_str().unwrap()).join("stdout.txt");
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(m.extra["python"]["stdout_bytes"], bytes.len());
        if size == 900_000 {
            assert_eq!(bytes.len(), size + 1);
        }
    }
}

#[tokio::test]
async fn engine_state_protected_but_artifacts_and_project_writes_open() {
    let (dir, app, ccx) = fixture().await;
    let ws = PythonWorkspace::for_chat(&app, "python-test")
        .await
        .unwrap();
    let paths = [
        dir.path().join("project/.claude/settings.json"),
        dir.path().join("project/.claude/settings.local.json"),
        dir.path().join("project/.refact/shell_policy.yaml"),
        dir.path().join("project/.refact/modes/x.yaml"),
        ws.dir().join("state.json"),
        ws.dir().join("privacy_ledger.json"),
        ws.runs_dir().join("old/run.json"),
    ];
    for path in paths {
        let m = run(
            ccx.clone(),
            &format!(
                "open({}, 'w')",
                json!(python_path(&path, native_platform()))
            ),
            false,
        )
        .await;
        assert_eq!(
            m.extra["python"]["error"]["type"], "PermissionError",
            "{path:?}"
        );
        assert!(m.extra["python"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("file-edit tools"));
    }
    let m = run(ccx, "from pathlib import Path\nimport os\nPath(PROJECT_ROOT).mkdir(parents=True, exist_ok=True)\nos.makedirs(ARTIFACTS_DIR, exist_ok=True)\nPath('root.txt').write_text('ok')\nPath(ARTIFACTS_DIR + '/out.txt').write_text('ok')", false).await;
    assert_eq!(m.tool_failed, Some(false), "{:?}", m.content);
    assert!(ws.artifacts_dir().join("out.txt").exists());
    let (rules, _) = protected_paths(&app, &dir.path().join("project"), &ws).await;
    if let Some(home) = home::home_dir() {
        assert!(rules.prefixes.contains(&home.join(".claude")));
    }
}

#[tokio::test]
async fn engine_secret_reads_denied_but_runs_readable() {
    let (dir, app, ccx) = fixture().await;
    let ws = PythonWorkspace::for_chat(&app, "python-test")
        .await
        .unwrap();
    ws.ensure_dirs().await.unwrap();
    std::fs::write(ws.runs_dir().join("readable"), "allowed").unwrap();
    for path in [
        app.paths.config_dir.join("providers.d/x.yaml"),
        app.paths.config_dir.join("secrets.yaml"),
        app.paths.config_dir.join("python_state.key"),
        dir.path().join("project/.refact/integrations.d/x.yaml"),
    ] {
        for code in [
            format!(
                "open({}).read()",
                json!(python_path(&path, native_platform()))
            ),
            format!(
                "from pathlib import Path\nPath({}).read_text()",
                json!(python_path(&path, native_platform()))
            ),
        ] {
            let m = run(ccx.clone(), &code, false).await;
            assert_eq!(
                m.extra["python"]["error"]["type"], "PermissionError",
                "{path:?}"
            );
        }
    }
    let m = run(ccx, "open(RUNS_DIR + '/readable').read()", false).await;
    assert!(m.content.content_text_only().contains("allowed"));
}

#[tokio::test]
async fn late_privacy_enumeration_failure_leaves_failed_metadata_without_summary() {
    let temp = tempfile::tempdir().unwrap();
    let run = temp.path().join("runs/late-privacy-failure");
    std::fs::create_dir_all(&run).unwrap();
    let guard = crate::python_tool::workspace::FailedRunGuard::new(
        run.clone(),
        "late-privacy-failure".into(),
        1,
        false,
    );
    crate::python_tool::workspace::write_running_run_json(
        &run,
        "late-privacy-failure",
        1,
        false,
        &Value::Null,
    )
    .await
    .unwrap();
    let metadata = || -> Value {
        serde_json::from_slice(&std::fs::read(run.join("run.json")).unwrap()).unwrap()
    };
    assert_eq!(metadata()["status"], "running");
    std::fs::create_dir_all(run.join("too/deep/for/a/scan")).unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let error = crate::python_tool::run_output::persist_run_privacy(
        &gcx,
        temp.path(),
        &run,
        &[],
        vec![],
        &crate::privacy::records::new_derived_privacy_zones(),
    )
    .await
    .unwrap_err();
    assert!(error.contains("enumeration limit exceeded"), "{error}");
    assert_eq!(metadata()["status"], "running");
    drop(guard);
    assert_eq!(metadata()["status"], "failed");
    assert_eq!(metadata()["error"]["type"], "RunAbandoned");
    assert!(!run
        .join(crate::python_tool::workspace::RUN_SUMMARY_FILE)
        .exists());
    assert!(!temp.path().join("privacy_ledger.json").exists());
}

#[tokio::test]
async fn corrupt_privacy_ledger_fails_closed_without_erasing_provenance() {
    let (_dir, app, ccx) = fixture().await;
    run(ccx.clone(), "x = 42", false).await;
    let ws = PythonWorkspace::for_chat(&app, "python-test")
        .await
        .unwrap();
    std::fs::write(ws.dir().join("privacy_ledger.json"), "not json").unwrap();
    let before = std::fs::read(ws.dir().join("state.json")).unwrap();
    let artifact = ws.artifacts_dir().join("not-created");
    let error = ToolPython::default()
        .tool_execute(
            ccx,
            &"corrupt-ledger".into(),
            &HashMap::from([(
                "code".into(),
                json!(format!("open({}, 'w').write('bad')", json!(artifact))),
            )]),
        )
        .await
        .unwrap_err();
    assert!(error.contains("privacy ledger"), "{error}");
    assert_eq!(
        std::fs::read(ws.dir().join("privacy_ledger.json")).unwrap(),
        b"not json"
    );
    assert_eq!(std::fs::read(ws.dir().join("state.json")).unwrap(), before);
    assert!(!artifact.exists());
}

#[tokio::test]
async fn python_scope_change_waits_and_nested_call_does_not_deadlock() {
    let (dir, app, _) = fixture().await;
    let app = with_tools(app);
    let root = dunce::canonicalize(dir.path().join("project")).unwrap();
    // Exercise every key on the outer Python owner, not merely an unrelated fence.
    let keys = [
        crate::tool_jobs::ScopeKey::Worktree {
            project: root.clone(),
            root,
        },
        app.runtime.tool_jobs.chat_scope_key("python-test"),
        crate::tool_jobs::ScopeKey::PythonRun {
            chat_id: "python-test".into(),
            tool_call_id: "scoped-2".into(),
        },
    ];
    for (index, key) in keys.into_iter().enumerate() {
        let call_id = format!("scoped-{index}");
        let started = dir.path().join(format!("started-{index}"));
        let finished = dir.path().join(format!("finished-{index}"));
        let ms = if index == 0 { 6500 } else { 500 };
        let code = format!(
            "await serial_fixture(ms={ms}, started={}, finished={})",
            json!(started),
            json!(finished)
        );
        let running = {
            let app = app.clone();
            tokio::spawn(async move { execute(app, &[call(&call_id, &code, false)]).await })
        };
        tokio::time::timeout(Duration::from_secs(5), async {
            while !started.exists() {
                assert!(!running.is_finished(), "nested fixture never started");
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let scope = app.runtime.tool_jobs.scopes.clone();
        let change = tokio::spawn(async move {
            let guard = scope.begin_scope_change(&[key]).await.unwrap();
            assert!(
                finished.exists(),
                "scope mutation passed a live nested execution"
            );
            guard
        });
        if index == 0 {
            tokio::time::sleep(Duration::from_millis(5500)).await;
            assert!(
                !change.is_finished(),
                "scope fence escaped at the old cancellation grace timeout"
            );
            assert!(
                !running.is_finished(),
                "outer Python owner settled before nested terminal"
            );
        }
        let result = tokio::time::timeout(Duration::from_secs(10), running)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            result[0].extra["python"]["status"], "cancelled",
            "{result:?}"
        );
        let guard = tokio::time::timeout(Duration::from_secs(5), change)
            .await
            .unwrap()
            .unwrap();
        drop(guard);
    }
}

#[test]
fn stale_cancelled_event_is_not_current_run_terminal() {
    assert_eq!(
        event_run_id(&KernelEvent::Cancelled {
            run_id: "old".into()
        }),
        Some("old")
    );
    assert_ne!(
        event_run_id(&KernelEvent::Cancelled {
            run_id: "old".into()
        }),
        Some("current")
    );
    assert_eq!(
        event_run_id(&KernelEvent::Completed {
            run_id: "current".into(),
            value: Value::Null,
            repr: "None".into(),
            truncated: false
        }),
        Some("current")
    );
}
#[test]
fn unknown_need_results_id_gets_runtime_error() {
    assert!(
        matches!(unknown_call_result(42), HostResult::Err {exc_type, message} if exc_type == "RuntimeError" && message.contains("42"))
    );
}
#[tokio::test]
async fn failure_after_feed_cleans_runtime_and_discards_worker() {
    let (_dir, app, ccx) = fixture().await;
    let failed = FAIL_AFTER_FEED
        .scope(
            true,
            ToolPython::default().tool_execute(
                ccx.clone(),
                &"injected".into(),
                &HashMap::from([("code".into(), json!("x=1"))]),
            ),
        )
        .await;
    assert!(failed.unwrap_err().contains("injected"));
    let workspace = PythonWorkspace::for_chat(&app, "python-test")
        .await
        .unwrap();
    let run_dir = std::fs::read_dir(workspace.runs_dir())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let metadata: Value =
        serde_json::from_slice(&std::fs::read(run_dir.join("run.json")).unwrap()).unwrap();
    assert_eq!(metadata["status"], "failed");
    let session = app.chat.sessions.read().await["python-test"].clone();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if session.lock().await.runtime.python_runs.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(app.python_tool.detached.lock().unwrap().is_empty());
    assert_eq!(run(ccx, "42", false).await.tool_failed, Some(false));
}
#[tokio::test]
async fn workspace_view_uses_registry_reset_reason() {
    let (_dir, app, ccx) = fixture().await;
    FAIL_AFTER_FEED
        .scope(
            true,
            ToolPython::default().tool_execute(
                ccx,
                &"injected".into(),
                &HashMap::from([("code".into(), json!("1"))]),
            ),
        )
        .await
        .unwrap_err();
    let view = crate::python_tool::workspace_view::workspace_view(&app, "python-test")
        .await
        .unwrap();
    assert!(view.session.reset_reason.unwrap().contains("cleanup"));
}

#[tokio::test]
async fn python_does_not_inherit_unread_history_privacy() {
    let (_dir, _app, ccx) = fixture().await;
    let mut history = ChatMessage::default();
    history.extra.insert(
        "privacy".into(),
        json!({"files":[{"path":"unread-secret","zone":"blocked","attribution":"observed"}]}),
    );
    ccx.lock().await.messages.push(history);
    let result = run(ccx, "42", false).await;
    assert!(!result
        .extra
        .get("privacy")
        .is_some_and(|p| p.to_string().contains("unread-secret")));
}
#[tokio::test]
async fn sync_helpers_accept_keyword_arguments() {
    let (_dir, _app, ccx) = fixture().await;
    let m = run(
        ccx,
        "[help(name='cat'), host_path(p='/tmp/a'), virtual_path(p='/tmp/a')]",
        false,
    )
    .await;
    assert_eq!(m.tool_failed, Some(false), "{m:?}");
}
#[tokio::test]
async fn help_without_name_lists_functions_and_help_with_name_prints_typed_stub() {
    let (_dir, _app, ccx) = fixture().await;
    let m = run(
        ccx,
        "index = help()\nassert index == help(name=None)\nassert index.startswith('help(\"name\") prints one function')\nassert '\\ncat: ' in index and '\\nshow: ' in index\nassert 'async def cat(*, paths: str' in help('cat')\n'checked'",
        false,
    )
    .await;
    assert_eq!(m.tool_failed, Some(false), "{m:?}");
    assert!(m.extra["python"]["error"].is_null(), "{m:?}");
    assert!(m.content.content_text_only().contains("checked"));
}
#[tokio::test]
async fn branched_root_has_no_parent_artifacts() {
    let (_dir, app, ccx) = fixture().await;
    app.chat.sessions.read().await["python-test"]
        .lock()
        .await
        .thread
        .parent_id = Some("parent".into());
    assert!(run(ccx, "PARENT_ARTIFACTS_DIR", false)
        .await
        .content
        .content_text_only()
        .contains("null"));
}

#[tokio::test]
async fn foreground_python_nested_serial_background_note_and_limits() {
    let (dir, app, ccx) = fixture().await;
    let app = with_tools(app);
    // This path is a supervised chat whose native tools deliberately stay foreground.
    ccx.lock().await.chat_id = "subchat-supervised-python".into();
    let thread = refact_chat_api::ThreadParams {
        id: "subchat-supervised-python".into(),
        ..Default::default()
    };
    let execute =
        |code: &str| crate::call_validation::ChatToolCall::from(call("foreground", code, true));
    let messages = tokio::time::timeout(
        Duration::from_secs(20),
        crate::chat::tools::execute_tools(
            app.clone(),
            &[execute("await serial_fixture()")],
            &[],
            &thread,
            "agent",
            Some("model"),
            crate::chat::tools::ExecuteToolsOptions {
                execution_context: Some(ccx.clone()),
                structured_output: true,
                ..Default::default()
            },
        ),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(messages[0].tool_failed, Some(false));
    assert!(messages[0]
        .content
        .content_text_only()
        .starts_with("Background runs are not available in this chat."));
    assert_eq!(messages[0].extra["python"]["background"], false);
    let saved: Value = serde_json::from_slice(
        &std::fs::read(
            std::path::Path::new(messages[0].extra["python"]["run_dir"].as_str().unwrap())
                .join("run.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(saved["background"], false);
    std::fs::write(
        dir.path().join("project/.refact/coordination.yaml"),
        "python_max_run_ms: 1000\n",
    )
    .unwrap();
    let messages = tokio::time::timeout(
        Duration::from_secs(20),
        crate::chat::tools::execute_tools(
            app.clone(),
            &[call("fg-timeout", "await fixture(ms=2000)", false)],
            &[],
            &thread,
            "agent",
            Some("model"),
            crate::chat::tools::ExecuteToolsOptions {
                execution_context: Some(ccx.clone()),
                structured_output: true,
                ..Default::default()
            },
        ),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(messages[0].extra["python"]["status"], "timed_out");
}

#[test]
fn python_constants_use_windows_virtual_paths() {
    assert_eq!(
        python_path(
            std::path::Path::new("C:\\project\\artifacts"),
            refact_python_kernel::hostfs::Platform::Windows
        ),
        "/C:/project/artifacts"
    );
}
#[test]
fn python_input_schema_and_description_contract_are_pinned() {
    let description = ToolPython::default().tool_description();
    assert_eq!(description.name, "python");
    assert!(description.allow_parallel);
    assert!(!description.experimental);
    assert!(description.output_schema.is_none());
    assert_eq!(description.input_schema["additionalProperties"], false);
    assert_eq!(description.input_schema["required"], json!(["code"]));
    assert_eq!(
        description.input_schema["properties"]["code"]["minLength"],
        1
    );
    assert_eq!(
        description.input_schema["properties"]["description"],
        json!({"type":"string","description":"What this script does, in one short sentence. The user sees it as the title of the run."})
    );
    assert_eq!(
        description.input_schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["code", "description", "restart", "run_in_background"]
    );
}
#[tokio::test]
async fn config_contained_workspace_can_read_and_overwrite_artifacts() {
    let (_dir, app, _) = fixture().await;
    let config = app.paths.config_dir.clone();
    let workspace = PythonWorkspace::from_trajectory(
        &config.join("trajectories/chat.json"),
        "chat",
        config.join("python_state.key"),
    );
    workspace.ensure_dirs().await.unwrap();
    std::fs::write(workspace.artifacts_dir().join("out"), "before").unwrap();
    std::fs::write(workspace.runs_dir().join("earlier"), "saved").unwrap();
    let (deny_write, deny_read) = protected_paths(&app, &config, &workspace).await;
    assert!(deny_read
        .allow_prefixes
        .contains(&workspace.dir().to_owned()));
    assert!(deny_write
        .allow_prefixes
        .contains(&workspace.artifacts_dir()));
    assert!(deny_read.prefixes.contains(&config));
}

#[tokio::test]
async fn python_is_admitted_as_owned_job() {
    let (_dir, app, _) = fixture().await;
    let messages = execute(with_tools(app.clone()), &[call("owned", "42", false)]).await;
    assert_eq!(messages[0].extra["tool_job_postprocessed"], true);
    assert_eq!(messages[0].extra["python"]["status"], "completed");
}
#[tokio::test]
async fn outer_call_cap_and_limit_status() {
    let (_dir, app, _) = fixture().await;
    let messages = execute(
        with_tools(app),
        &[call(
            "cap",
            "for i in range(205):\n    await fixture(ms=0)\nprint('x' * 1100000)",
            false,
        )],
    )
    .await;
    let extra = &messages[0].extra["python"];
    assert_eq!(extra["status"], "limit_exceeded");
    assert_eq!(extra["calls"].as_array().unwrap().len(), 200);
    assert_eq!(extra["calls_truncated"], true);
    let seqs: Vec<_> = extra["calls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["seq"].as_u64().unwrap())
        .collect();
    assert!(seqs.windows(2).all(|w| w[0] < w[1]));
}

#[tokio::test]
async fn limit_failure_rolls_back_to_last_snapshot() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let (_dir, _app, ccx) = fixture().await;
        run(ccx.clone(), "x=42", false).await;
        let failed = run(ccx.clone(), "x=0\nlost=1\nprint('x'*1100000)", false).await;
        assert_eq!(failed.extra["python"]["status"], "limit_exceeded");
        let reason = failed.extra["python"]["state"]["reset_reason"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            reason.contains("restored to the state saved before this run")
                && reason.contains("variables set in this run are lost"),
            "{reason}"
        );
        let text = failed.content.content_text_only();
        assert!(text.contains(&reason), "{text}");
        assert!(!text.contains("not persisted"), "{text}");
        let next = run(ccx.clone(), "x", false).await;
        assert!(
            next.content.content_text_only().contains("Result:\n42"),
            "{:?}",
            next.content
        );
        assert!(next.extra["python"]["state"]["reset_reason"].is_null());
        let lost = run(ccx, "lost", false).await;
        assert_eq!(lost.extra["python"]["error"]["type"], "NameError");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn limit_failure_without_snapshot_says_session_reset() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let (_dir, _app, ccx) = fixture().await;
        let failed = run(ccx.clone(), "x=0\nprint('x'*1100000)", false).await;
        assert_eq!(failed.extra["python"]["status"], "limit_exceeded");
        let reason = failed.extra["python"]["state"]["reset_reason"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            reason.contains("session was reset and has no variables"),
            "{reason}"
        );
        assert!(!reason.contains("restored"), "{reason}");
        let next = run(ccx.clone(), "x", false).await;
        assert_eq!(next.extra["python"]["error"]["type"], "NameError");
        assert!(next.extra["python"]["state"]["reset_reason"].is_null());
        let third = run(ccx, "1", false).await;
        assert!(third.extra["python"]["state"]["reset_reason"].is_null());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn foreground_python_stop_cancels_run() {
    let (_dir, app, ccx) = fixture().await;
    let app = with_tools(app);
    ccx.lock().await.chat_id = "subchat-supervised-stop".into();
    let thread = refact_chat_api::ThreadParams {
        id: "subchat-supervised-stop".into(),
        ..Default::default()
    };
    let task = {
        let app = app.clone();
        tokio::spawn(async move {
            tokio::time::timeout(
                Duration::from_secs(20),
                crate::chat::tools::execute_tools(
                    app,
                    &[call("fg-stop", "await fixture(ms=5000)", false)],
                    &[],
                    &thread,
                    "agent",
                    Some("model"),
                    crate::chat::tools::ExecuteToolsOptions {
                        execution_context: Some(ccx),
                        structured_output: true,
                        ..Default::default()
                    },
                ),
            )
            .await
            .unwrap()
            .0
        })
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if app
                .python_tool
                .stop_run("subchat-supervised-stop", "fg-stop")
                .is_ok()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let messages = tokio::time::timeout(Duration::from_secs(20), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(messages[0].extra["python"]["status"], "cancelled");
}

#[tokio::test]
async fn configured_state_limit_reaches_worker_on_restore() {
    let (dir, app, ccx) = fixture().await;
    let settings = dir.path().join("project/.refact/coordination.yaml");
    std::fs::write(&settings, "python_max_state_bytes: 2097152\n").unwrap();
    let m = run(ccx.clone(), "large = 'x' * 1200000", false).await;
    assert_eq!(m.extra["python"]["state"]["persisted"], true);
    app.python_tool.registry.drop_chat("python-test").await;
    std::fs::write(&settings, "python_max_state_bytes: 1048576\n").unwrap();
    let m = run(ccx, "42", false).await;
    assert!(m.extra["python"]["state"]["reset_reason"]
        .as_str()
        .unwrap()
        .contains("configured state limit"));
}

#[tokio::test]
async fn auto_approve_editing_tools_skips_nested_card_and_records_auto() {
    let (_dir, mut app, _) = fixture().await;
    app.tool_registry = Arc::new(
        crate::app_state::AppToolRegistry::with_fixture_tool_factory(
            app.gcx.clone(),
            Arc::new(|| {
                vec![
                    Box::new(ToolPython::default()),
                    Box::new(
                        crate::tools::file_edit::tool_create_textdoc::ToolCreateTextDoc {
                            config_path: String::new(),
                        },
                    ),
                ]
            }),
        ),
    );
    app.chat.sessions.read().await["python-test"]
        .lock()
        .await
        .thread
        .auto_approve_editing_tools = true;
    let messages = execute(
        app.clone(),
        &[call(
            "edit-auto",
            "await create_textdoc(path=PROJECT_ROOT + '/auto.txt', content='ok')",
            false,
        )],
    )
    .await;
    assert_eq!(messages[0].tool_failed, Some(false), "{messages:?}");
    assert_eq!(messages[0].extra["python"]["calls"][0]["approval"], "auto");
    assert!(app.chat.sessions.read().await["python-test"]
        .lock()
        .await
        .runtime
        .python_approvals
        .is_empty());
}

async fn next_approval(
    app: &AppState,
    task: &tokio::task::JoinHandle<Vec<ChatMessage>>,
) -> refact_chat_api::PythonApprovalRequest {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let session = app.chat.sessions.read().await["python-test"].clone();
            if let Some(request) = session.lock().await.runtime.python_approvals.first() {
                break request.clone();
            }
            assert!(!task.is_finished(), "run ended without asking");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("no approval request")
}

#[tokio::test]
async fn allow_for_run_skips_later_edit_confirmations_of_the_same_run_only() {
    tokio::time::timeout(Duration::from_secs(90), async {
        let (dir, mut app, _) = fixture().await;
        app.tool_registry = Arc::new(
            crate::app_state::AppToolRegistry::with_fixture_tool_factory(
                app.gcx.clone(),
                Arc::new(|| {
                    vec![
                        Box::new(ToolPython::default()),
                        Box::new(
                            crate::tools::file_edit::tool_create_textdoc::ToolCreateTextDoc {
                                config_path: String::new(),
                            },
                        ),
                        Box::new(crate::tools::tool_shell::ToolShell {
                            cfg: Default::default(),
                            config_path: String::new(),
                        }),
                    ]
                }),
            ),
        );
        use refact_chat_api::PythonApprovalDecisionKind as Decision;
        let code = "await create_textdoc(path=PROJECT_ROOT + '/zone_a.txt', content='a')\nawait create_textdoc(path=PROJECT_ROOT + '/zone_b.txt', content='b')\ntry:\n    await shell(command='echo safe', description='still asks', needs_confirmation=True)\nexcept PermissionError:\n    pass\n'done'";
        let task = {
            let app = app.clone();
            tokio::spawn(async move { execute(app, &[call("edit-run", code, false)]).await })
        };
        let first = next_approval(&app, &task).await;
        assert!(first.editing_tool);
        app.python_tool
            .approvals
            .decide(&app, "python-test", &first.request_id, Decision::AllowForRun)
            .await
            .unwrap();
        let shell = next_approval(&app, &task).await;
        assert_eq!(shell.tool_name, "shell");
        assert!(!shell.editing_tool);
        app.python_tool
            .approvals
            .decide(&app, "python-test", &shell.request_id, Decision::Deny)
            .await
            .unwrap();
        let messages = task.await.unwrap();
        let python = &messages[0].extra["python"];
        assert_eq!(python["status"], "completed", "{python}");
        let approvals: Vec<_> = python["calls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["approval"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(approvals, ["allowed_for_run", "allowed_for_run", "denied"]);
        assert!(dir.path().join("project/zone_b.txt").exists());
        let session = app.chat.sessions.read().await["python-test"].clone();
        assert!(!session.lock().await.thread.auto_approve_editing_tools);
        assert!(app.python_tool.approvals.run_permits.lock().unwrap().is_empty());

        let task = {
            let app = app.clone();
            tokio::spawn(async move {
                execute(
                    app,
                    &[call(
                        "edit-next",
                        "await create_textdoc(path=PROJECT_ROOT + '/zone_c.txt', content='c')",
                        false,
                    )],
                )
                .await
            })
        };
        let again = next_approval(&app, &task).await;
        app.python_tool
            .approvals
            .decide(&app, "python-test", &again.request_id, Decision::Deny)
            .await
            .unwrap();
        let messages = task.await.unwrap();
        assert_eq!(messages[0].extra["python"]["calls"][0]["approval"], "denied");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn workspace_active_run_reports_live_call_count() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (_dir, app, _) = fixture().await;
        let app = with_tools(app);
        let worker_app = app.clone();
        let worker = tokio::spawn(async move {
            execute(
                worker_app,
                &[call("live-count", "await fixture(ms=1000)", false)],
            )
            .await
        });
        loop {
            let view = crate::python_tool::workspace_view::workspace_view(&app, "python-test")
                .await
                .unwrap();
            if let Some(run) = view.runs.iter().find(|run| run.calls_total == 1) {
                assert!(matches!(run.status.as_str(), "running" | "background"));
                assert!(run.finished_at_ms.is_none());
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        worker.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn reset_reason_is_only_for_the_run_that_reset_tool_and_view() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let (_dir, app, ccx) = fixture().await;
        run(ccx.clone(), "x = 41", false).await;
        app.python_tool
            .registry
            .lose_worker_for_test("python-test")
            .await;
        let lost = run(ccx.clone(), "x", false).await;
        assert_eq!(lost.tool_failed, Some(true));
        let reset = run(ccx.clone(), "x", false).await;
        assert!(reset.extra["python"]["state"]["reset_reason"].is_string());
        assert!(
            crate::python_tool::workspace_view::workspace_view(&app, "python-test")
                .await
                .unwrap()
                .session
                .reset_reason
                .is_some()
        );
        for _ in 0..2 {
            let normal = run(ccx.clone(), "x + 1", false).await;
            assert!(normal.content.content_text_only().contains("42"));
            assert!(normal.extra["python"]["state"]["reset_reason"].is_null());
            let view = crate::python_tool::workspace_view::workspace_view(&app, "python-test")
                .await
                .unwrap();
            assert!(view.session.reset_reason.is_none());
        }
    })
    .await
    .unwrap();
}

#[test]
fn custom_configuration_paths_use_engine_cwd_and_deny_reads_and_writes() {
    let mut write = PathDenyRules::default();
    let mut read = Vec::new();
    protect_custom_configuration(
        &mut write,
        &mut read,
        "custom-secrets.yaml",
        "custom-integrations.yaml",
    );
    for name in ["custom-secrets.yaml", "custom-integrations.yaml"] {
        let path = crate::files_correction::canonical_path(name);
        assert!(path.is_absolute());
        assert!(
            write.files.contains(&path),
            "missing write denial for {name}"
        );
        assert!(read.contains(&path), "missing read denial for {name}");
    }
}

#[tokio::test]
async fn queued_python_turn_observes_cancellation_promptly() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (_dir, app, ccx) = fixture().await;
        let first = app
            .python_tool
            .registry
            .reserve_turn("python-test", "holding")
            .acquire()
            .await
            .unwrap();
        let abort = ccx.lock().await.abort_flag.clone();
        let id = "queued-cancel".to_owned();
        let args = HashMap::from([("code".into(), json!("1"))]);
        let mut tool = ToolPython::default();
        let mut queued = Box::pin(tool.tool_execute(ccx, &id, &args));
        assert!(futures::poll!(&mut queued).is_pending());
        abort.store(true, Ordering::Release);
        // The tool returns an error on cancellation, so avoid the run helper's unwrap.
        let result = tokio::time::timeout(Duration::from_millis(500), queued).await;
        drop(first);
        assert!(
            result.is_ok(),
            "cancelled queued Python turn did not return promptly"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn restart_reasons_clear_on_normal_runs() {
    tokio::time::timeout(Duration::from_secs(30), async {
        for _ in 0..2 {
            let (_dir, app, ccx) = fixture().await;
            run(ccx.clone(), "old = 1", false).await;
            let reset = run(ccx.clone(), "x = 41", true).await;
            assert!(reset.extra["python"]["state"]["reset_reason"].is_string());
            assert!(
                crate::python_tool::workspace_view::workspace_view(&app, "python-test")
                    .await
                    .unwrap()
                    .session
                    .reset_reason
                    .is_some()
            );
            for _ in 0..2 {
                let normal = run(ccx.clone(), "x+1", false).await;
                assert!(normal.content.content_text_only().contains("42"));
                assert!(normal.extra["python"]["state"]["reset_reason"].is_null());
                assert!(
                    crate::python_tool::workspace_view::workspace_view(&app, "python-test")
                        .await
                        .unwrap()
                        .session
                        .reset_reason
                        .is_none()
                );
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn observation_markers_stay_on_the_run_that_produced_them() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let (_dir, app, _) = fixture().await;
        let app = with_tools(app);
        let first = execute(
            app.clone(),
            &[call(
                "observed",
                "r = await fixture(observation={'status':'incomplete'})\n1",
                false,
            )],
        )
        .await;
        assert_eq!(
            first[0].extra["privacy_observation"]["status"], "incomplete",
            "{:?}",
            first[0].extra
        );
        let second = execute(app.clone(), &[call("plain", "2", false)]).await;
        assert!(!second[0].extra.contains_key("privacy_observation"));
        app.python_tool.registry.drop_chat("python-test").await;
        let restored = execute(app, &[call("restored", "3", false)]).await;
        assert_eq!(restored[0].extra["python"]["state"]["restored"], true);
        assert!(!restored[0].extra.contains_key("privacy_observation"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn running_run_json_is_written_when_a_run_starts() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let (_dir, app, _) = fixture().await;
        let app = with_tools(app);
        let workspace = PythonWorkspace::for_chat(&app, "python-test")
            .await
            .unwrap();
        let worker_app = app.clone();
        let worker = tokio::spawn(async move {
            execute(
                worker_app,
                &[call("starting", "await fixture(ms=1500)", false)],
            )
            .await
        });
        let running = loop {
            let found = std::fs::read_dir(workspace.runs_dir())
                .ok()
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|entry| std::fs::read(entry.path().join("run.json")).ok())
                .filter_map(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .find(|value| value["status"] == "running");
            if let Some(value) = found {
                break value;
            }
            assert!(
                !worker.is_finished(),
                "run finished without running run.json"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(running["schema"], 1);
        assert!(running["started_at_ms"].as_u64().is_some());
        assert!(running["finished_at_ms"].is_null());
        let messages = worker.await.unwrap();
        let saved: Value = serde_json::from_slice(
            &std::fs::read(
                std::path::Path::new(messages[0].extra["python"]["run_dir"].as_str().unwrap())
                    .join("run.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(saved["status"], "completed");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn stop_drain_reports_waiting_once_and_engine_shutdown_ends_it() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let (dir, app, _) = fixture().await;
        let app = with_tools(app);
        let session = app.chat.sessions.read().await["python-test"].clone();
        let mut events = session.lock().await.event_tx.subscribe();
        let marker = dir.path().join("drain-started");
        let code = format!("await fixture(ms=25000, started={})", json!(marker));
        let worker_app = app.clone();
        let worker =
            tokio::spawn(async move { execute(worker_app, &[call("drain", &code, false)]).await });
        while !marker.exists() {
            assert!(!worker.is_finished(), "nested fixture never started");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        app.python_tool.stop_run("python-test", "drain").unwrap();
        let started = std::time::Instant::now();
        let mut seen = String::new();
        while !seen.contains("Waiting for fixture to stop") {
            match tokio::time::timeout(Duration::from_secs(10), events.recv()).await {
                Ok(Ok(event)) => seen.push_str(&event.to_string()),
                Ok(Err(_)) => continue,
                Err(_) => panic!("no waiting notice after Stop: {seen}"),
            }
        }
        assert!(started.elapsed() >= Duration::from_secs(4));
        assert!(seen.contains("stopping…"), "{seen}");
        assert!(!worker.is_finished());
        app.gcx.shutdown_flag.store(true, Ordering::Release);
        let messages = tokio::time::timeout(Duration::from_secs(5), worker)
            .await
            .expect("engine shutdown did not end the drain")
            .unwrap();
        assert_eq!(messages[0].extra["python"]["status"], "cancelled");
        let mut rest = String::new();
        while let Ok(event) = events.try_recv() {
            rest.push_str(&event.to_string());
        }
        assert!(!rest.contains("Waiting for"), "{rest}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn small_sessions_are_not_marked_truncated_and_totals_are_reported() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let (_dir, app, ccx) = fixture().await;
        run(ccx.clone(), "r = {'k': 'v' * 500}\nimport json", false).await;
        let view = crate::python_tool::workspace_view::workspace_view(&app, "python-test")
            .await
            .unwrap();
        assert!(!view.session.variables_truncated, "{:?}", view.session);
        let small = view.session.variables.len() as u64;
        assert_eq!(view.session.variables_total, Some(small));
        assert!(!view.session.last_run_restored);
        let code: String = (0..230).map(|i| format!("v{i} = {i}\n")).collect();
        run(ccx.clone(), &code, false).await;
        let view = crate::python_tool::workspace_view::workspace_view(&app, "python-test")
            .await
            .unwrap();
        assert!(view.session.variables_truncated);
        assert_eq!(view.session.variables.len(), 200);
        assert_eq!(view.session.variables_total, Some(small + 230));
        app.python_tool.registry.drop_chat("python-test").await;
        run(ccx, "1", false).await;
        let view = crate::python_tool::workspace_view::workspace_view(&app, "python-test")
            .await
            .unwrap();
        assert!(view.session.last_run_restored);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn run_description_and_code_preview_are_recorded_everywhere() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (_dir, app, _) = fixture().await;
        let app = with_tools(app);
        let workspace = PythonWorkspace::for_chat(&app, "python-test")
            .await
            .unwrap();
        let titled = |id: &str, code: &str, description: Option<&str>| {
            let mut args = json!({"code":code});
            if let Some(d) = description {
                args["description"] = json!(d);
            }
            serde_json::from_value::<crate::call_validation::ChatToolCall>(json!({"id":id,"type":"function","function":{"name":"python","arguments":args.to_string()}})).unwrap()
        };
        let read_run = |dir: &str| -> Value {
            serde_json::from_slice(&std::fs::read(std::path::Path::new(dir).join("run.json")).unwrap())
                .unwrap()
        };
        let worker_app = app.clone();
        let worker = tokio::spawn(async move {
            execute(
                worker_app,
                &[titled(
                    "titled",
                    "\n   \nawait fixture(ms=1500)  \n1",
                    Some("  Count   TODO\nmarkers "),
                )],
            )
            .await
        });
        let running = loop {
            let found = std::fs::read_dir(workspace.runs_dir())
                .ok()
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|entry| std::fs::read(entry.path().join("run.json")).ok())
                .filter_map(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .find(|value| value["status"] == "running");
            if let Some(value) = found {
                break value;
            }
            assert!(!worker.is_finished(), "run finished without running run.json");
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(running["description"], "Count TODO markers");
        assert_eq!(running["code_preview"], "await fixture(ms=1500)");
        let messages = worker.await.unwrap();
        let python = &messages[0].extra["python"];
        assert_eq!(python["status"], "completed");
        assert_eq!(python["description"], "Count TODO markers");
        assert_eq!(python["code_preview"], "await fixture(ms=1500)");
        let saved = read_run(python["run_dir"].as_str().unwrap());
        assert_eq!(saved["description"], "Count TODO markers");
        assert_eq!(saved["code_preview"], "await fixture(ms=1500)");
        let view = serde_json::to_value(
            crate::python_tool::workspace_view::workspace_view(&app, "python-test")
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(view["runs"][0]["description"], "Count TODO markers");
        assert_eq!(view["runs"][0]["code_preview"], "await fixture(ms=1500)");

        let messages = execute(app.clone(), &[titled("untitled", "x = 2\nx", None)]).await;
        let python = &messages[0].extra["python"];
        assert!(python.get("description").is_none());
        assert_eq!(python["code_preview"], "x = 2");
        let saved = read_run(python["run_dir"].as_str().unwrap());
        assert!(saved.get("description").is_none());
        assert_eq!(saved["code_preview"], "x = 2");
        let view = serde_json::to_value(
            crate::python_tool::workspace_view::workspace_view(&app, "python-test")
                .await
                .unwrap(),
        )
        .unwrap();
        let entry = view["runs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["run_id"] == python["run_id"])
            .unwrap();
        assert!(entry.get("description").is_none());
        assert_eq!(entry["code_preview"], "x = 2");

        let long = "a".repeat(400);
        let messages = execute(app.clone(), &[titled("long", "1", Some(&long))]).await;
        let description = messages[0].extra["python"]["description"].as_str().unwrap();
        assert_eq!(description.chars().count(), 160);
        assert!(description.ends_with('…'));
        assert_eq!(description, format!("{}…", "a".repeat(159)));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nested_call_records_and_errors_carry_script_positions() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (_dir, app, _) = fixture().await;
        let app = with_tools(app);
        let session = app
            .chat
            .sessions
            .read()
            .await
            .get("python-test")
            .unwrap()
            .clone();
        let mut events = session.lock().await.event_tx.subscribe();
        let code = "a = await fixture(ms=1)\n\nif a:\n    b = await fixture(ms=1)\nc = 'é'; d = await fixture(ms=1)";
        let messages = execute(app.clone(), &[call("sites", code, false)]).await;
        let python = &messages[0].extra["python"];
        assert_eq!(python["status"], "completed", "{python}");
        let at = |line: &Value, col: &Value| -> String {
            code.lines()
                .nth(line.as_u64().unwrap() as usize - 1)
                .unwrap()
                .chars()
                .skip(col.as_u64().unwrap() as usize)
                .collect()
        };
        let sites: Vec<_> = python["calls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| (c["line"].clone(), c["col"].clone()))
            .collect();
        assert_eq!(
            sites,
            vec![(json!(1), json!(10)), (json!(4), json!(14)), (json!(5), json!(19))]
        );
        for (line, col) in &sites {
            assert!(at(line, col).starts_with("fixture("));
        }
        let mut live = vec![];
        fn started(v: &Value, out: &mut Vec<(Value, Value)>) {
            if v["kind"] == "call_started" {
                out.push((v["record"]["line"].clone(), v["record"]["col"].clone()));
            } else if let Some(o) = v.as_object() {
                o.values().for_each(|v| started(v, out));
            }
        }
        while let Ok(e) = events.try_recv() {
            started(&serde_json::from_str(&e).unwrap(), &mut live);
        }
        assert_eq!(live, sites);
        let saved: Value = serde_json::from_slice(
            &std::fs::read(
                std::path::Path::new(python["run_dir"].as_str().unwrap())
                    .join("calls/2_fixture.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!((saved["record"]["line"].clone(), saved["record"]["col"].clone()), sites[1]);

        let messages = execute(app, &[call("raise", "x = 1\ny = x / 0\nz = 2", false)]).await;
        assert_eq!(messages[0].extra["python"]["error"]["line"], 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn module_imports_go_through_the_read_guard_and_are_recorded() {
    let (dir, app, ccx) = fixture().await;
    let root = dir.path().join("project");
    std::fs::write(root.join("zone_mod.py"), "VALUE = 7\n").unwrap();
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.zones = vec![refact_privacy::Zone {
        name: "zone_a".into(),
        patterns: vec!["zone_mod.py".into()],
        send_to: vec!["model".into()],
        on_shell_read: refact_privacy::ShellBehavior::Withhold,
    }];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
    let m = run(ccx.clone(), "import zone_mod\nzone_mod.VALUE", false).await;
    assert_eq!(m.tool_failed, Some(false), "{:?}", m.content);
    let module = root.join("zone_mod.py");
    let read: Vec<String> = m.extra["python"]["files_read"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    assert!(read.iter().any(|p| p.ends_with("zone_mod.py")), "{read:?}");
    let privacy = m
        .extra
        .get("privacy")
        .map(|v| v.to_string())
        .unwrap_or_default();
    assert!(
        privacy.contains("zone_mod.py") && privacy.contains("zone_a"),
        "{privacy} / {module:?}"
    );
    let guarded = app.paths.config_dir.join("zone_dir");
    std::fs::create_dir_all(&guarded).unwrap();
    std::fs::write(guarded.join("zone_b.py"), "VALUE = 8\n").unwrap();
    let open = run(
        ccx.clone(),
        &format!(
            "open({}).read()",
            json!(python_path(&guarded.join("zone_b.py"), native_platform()))
        ),
        false,
    )
    .await;
    assert_eq!(open.extra["python"]["error"]["type"], "PermissionError");
    let code = format!(
        "import os\nos.chdir({})\nimport zone_b\nzone_b.VALUE",
        json!(python_path(&guarded, native_platform()))
    );
    let m = run(ccx, &code, false).await;
    assert_eq!(
        m.extra["python"]["error"]["type"], "PermissionError",
        "{:?}",
        m.content
    );
    assert!(!m.content.content_text_only().contains("VALUE = 8"));
}

async fn show_session(app: &AppState) -> Arc<AMutex<crate::chat::types::ChatSession>> {
    app.chat
        .sessions
        .read()
        .await
        .get("python-test")
        .unwrap()
        .clone()
}

#[tokio::test]
async fn show_reaches_live_updates_extra_run_json_and_model_text() {
    let (_dir, app, _) = fixture().await;
    let app = with_tools(app);
    let session = show_session(&app).await;
    let mut events = session.lock().await.event_tx.subscribe();
    let code = "rows = [{'test': 'cell_marker_' + str(i), 'ms': i * 3} for i in range(250)]\nshow(rows, title='Slowest tests')\nshow({'module': ['a', 'b'], 'failures': [3, 1], 'flaky': [0, 2]}, kind='bar', title='Failures by module')\nshow('# Notes\\nbody_marker')\n'done'";
    let messages = execute(app.clone(), &[call("show-run", code, false)]).await;
    let python = &messages[0].extra["python"];
    assert_eq!(python["status"], "completed", "{python}");
    assert_eq!(python["displays_total"], 3);
    assert_eq!(python["displays_truncated"], false);
    let displays = python["displays"].as_array().unwrap();
    assert_eq!(displays.len(), 3);
    assert_eq!(displays[0]["index"], 1);
    assert_eq!(displays[0]["kind"], "table");
    assert_eq!(displays[0]["rows"].as_array().unwrap().len(), 200);
    assert_eq!(
        (
            displays[0]["rows_total"].clone(),
            displays[0]["truncated"].clone()
        ),
        (json!(250), json!(true))
    );
    assert_eq!(displays[1]["x"], "module");
    assert_eq!(displays[1]["y"], json!(["failures", "flaky"]));
    assert_eq!(displays[2]["kind"], "markdown");
    let run_dir = std::path::PathBuf::from(python["run_dir"].as_str().unwrap());
    let file_path = run_dir.join("displays").join("1.json");
    assert_eq!(displays[0]["path"], json!(file_path.to_string_lossy()));
    let file: Value = serde_json::from_slice(&std::fs::read(&file_path).unwrap()).unwrap();
    assert_eq!(
        (
            file["schema"].clone(),
            file["rows"].as_array().unwrap().len()
        ),
        (json!(1), 250)
    );
    let saved: Value =
        serde_json::from_slice(&std::fs::read(run_dir.join("run.json")).unwrap()).unwrap();
    assert_eq!(saved["displays_total"], 3);
    let text = messages[0].content.content_text_only();
    assert!(
        text.contains("shown to the user: table \"Slowest tests\" (250 rows × 2 columns)"),
        "{text}"
    );
    assert!(text.contains("shown to the user: bar chart \"Failures by module\" (2 rows; x=module; y=failures, flaky)"), "{text}");
    assert!(
        text.contains("shown to the user: markdown (19 characters)"),
        "{text}"
    );
    assert!(
        !text.contains("cell_marker_") && !text.contains("body_marker"),
        "{text}"
    );
    for value in python["files_written"]
        .as_array()
        .unwrap()
        .iter()
        .chain(python["artifacts"].as_array().unwrap())
    {
        assert!(!value.to_string().contains("displays"), "{value}");
    }
    let mut live = vec![];
    while let Ok(event) = events.try_recv() {
        let Some(event) = serde_json::from_str::<Value>(&event).ok().or_else(|| {
            event
                .find('{')
                .and_then(|i| serde_json::from_str::<Value>(event[i..].trim_end()).ok())
        }) else {
            continue;
        };
        let event = if event.get("update").is_some() {
            event
        } else {
            event["event"].clone()
        };
        if event["update"]["kind"] == "display" {
            live.push(event["update"]["display"].clone());
        }
    }
    assert_eq!(live, displays.to_vec());
    let view = crate::python_tool::workspace_view::workspace_view(&app, "python-test")
        .await
        .unwrap();
    let entry = view
        .runs
        .iter()
        .find(|r| Some(r.run_id.as_str()) == python["run_id"].as_str())
        .unwrap();
    assert_eq!(entry.displays_total, Some(3));
    let shown: Vec<u32> = view.displays.iter().map(|d| d.display.index).collect();
    assert_eq!(shown, [3, 2, 1]);
    let messages = execute(app.clone(), &[call("show-none", "1", false)]).await;
    let python = &messages[0].extra["python"];
    assert!(python.get("displays").is_none() && python.get("displays_total").is_none());
}

#[tokio::test]
async fn show_snapshot_is_current_while_the_run_is_suspended() {
    let (_dir, app, _) = fixture().await;
    let app = with_tools(app);
    let session = show_session(&app).await;
    let watcher = tokio::spawn({
        let session = session.clone();
        async move {
            tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    if let Some(run) = session.lock().await.runtime.python_runs.first() {
                        if !run.displays.is_empty() {
                            return run.displays.clone();
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("display must appear in the run snapshot")
        }
    });
    execute(
        app,
        &[call(
            "show-snap",
            "show([1, 2], title='Live')\nawait fixture(ms=1500)",
            false,
        )],
    )
    .await;
    let displays = watcher.await.unwrap();
    assert_eq!(displays[0].title.as_deref(), Some("Live"));
    assert_eq!(displays[0].rows.as_ref().unwrap().len(), 2);
}

#[tokio::test]
async fn show_errors_raise_type_error_and_displays_survive_failures() {
    let (_dir, app, _) = fixture().await;
    let app = with_tools(app);
    let code = "show([1], title='kept')\nerrors = []\nfor kw in [dict(kind='heatmap'), dict(kind='bar'), dict(kind='markdown')]:\n    try:\n        show([['a', 'b']], **kw)\n    except TypeError as e:\n        errors.append(str(e))\ntry:\n    show([list(range(201))])\nexcept TypeError as e:\n    errors.append(str(e))\nprint(len(errors))\n1 / 0";
    let messages = execute(app, &[call("show-fail", code, false)]).await;
    let python = &messages[0].extra["python"];
    assert_eq!(python["status"], "failed");
    assert!(
        python["stdout_preview"].as_str().unwrap().starts_with('4'),
        "{python}"
    );
    assert_eq!(python["displays_total"], 1);
    assert_eq!(python["displays"][0]["title"], "kept");
}

#[tokio::test]
async fn show_beyond_twelve_displays_is_disk_only() {
    let (_dir, app, _) = fixture().await;
    let app = with_tools(app);
    let messages = execute(
        app,
        &[call(
            "show-many",
            "for i in range(14):\n    show([i], title='t' + str(i))",
            false,
        )],
    )
    .await;
    let python = &messages[0].extra["python"];
    assert_eq!(
        (
            python["displays_total"].clone(),
            python["displays_truncated"].clone()
        ),
        (json!(14), json!(true))
    );
    assert_eq!(python["displays"].as_array().unwrap().len(), 12);
    let run_dir = std::path::PathBuf::from(python["run_dir"].as_str().unwrap());
    assert!(run_dir.join("displays/14.json").exists());
    let text = messages[0].content.content_text_only();
    assert!(text.contains("… and 2 more"), "{text}");
}

#[tokio::test]
async fn show_passes_a_10000_by_30_table() {
    let (_dir, app, _) = fixture().await;
    let app = with_tools(app);
    let code =
        "rows = [[i * 30 + j for j in range(30)] for i in range(10000)]\nshow(rows, title='Big')";
    let messages = execute(app, &[call("show-big", code, false)]).await;
    let python = &messages[0].extra["python"];
    assert_eq!(python["status"], "completed", "{python}");
    let run_dir = std::path::PathBuf::from(python["run_dir"].as_str().unwrap());
    let file: Value =
        serde_json::from_slice(&std::fs::read(run_dir.join("displays/1.json")).unwrap()).unwrap();
    assert_eq!(file["rows"].as_array().unwrap().len(), 10000);
    assert_eq!(file["rows"][9999][29], 299_999);
    assert_eq!(python["displays"][0]["rows_total"], 10000);
}

#[tokio::test]
async fn show_is_not_a_variable_and_provider_requests_omit_display_data() {
    let (_dir, app, ccx) = fixture().await;
    let m = run(ccx, "show([{'cell': 'provider_marker'}])\nx = 1", false).await;
    assert!(m.extra["python"]["displays"]
        .to_string()
        .contains("provider_marker"));
    let view = crate::python_tool::workspace_view::workspace_view(&app, "python-test")
        .await
        .unwrap();
    let names: Vec<_> = view
        .session
        .variables
        .iter()
        .map(|v| v.name.as_str())
        .collect();
    assert!(
        names.contains(&"x") && !names.contains(&"show"),
        "{names:?}"
    );
    let mut request_message = m.clone();
    request_message.tool_call_id = "call_1".into();
    let assistant = crate::call_validation::ChatMessage {
        role: "assistant".into(),
        tool_calls: Some(vec![serde_json::from_value(
            json!({"id":"call_1","type":"function","function":{"name":"python","arguments":"{}"}}),
        )
        .unwrap()]),
        ..Default::default()
    };
    let messages = vec![
        crate::call_validation::ChatMessage::new("user".into(), "go".into()),
        assistant,
        request_message,
    ];
    for format in [
        crate::llm::WireFormat::OpenaiChatCompletions,
        crate::llm::WireFormat::OpenaiResponses,
        crate::llm::WireFormat::AnthropicMessages,
    ] {
        let request = crate::llm::LlmRequest::new("m".into(), messages.clone());
        let settings = refact_llm::adapter::AdapterSettings {
            api_key: "k".into(),
            auth_token: String::new(),
            endpoint: "https://example.invalid/v1".into(),
            extra_headers: Default::default(),
            model_name: "m".into(),
            supports_tools: true,
            supports_reasoning: false,
            reasoning_type: None,
            supports_temperature: true,
            supports_max_completion_tokens: false,
            eof_is_done: false,
            native_web_search_enabled: false,
            supports_cache_control: false,
        };
        let body = crate::llm::get_adapter(format)
            .build_http(&refact_privacy::testing::cleared(request), &settings)
            .unwrap()
            .body
            .to_string();
        assert!(body.contains("shown to the user"), "{format:?}: {body}");
        assert!(!body.contains("provider_marker"), "{format:?}: {body}");
    }
}

struct LargeOutputTool;
#[async_trait]
impl Tool for LargeOutputTool {
    async fn tool_execute(
        &mut self,
        _: Arc<AMutex<AtCommandsContext>>,
        id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let text_bytes = args.get("text_bytes").and_then(Value::as_u64).unwrap_or(0) as usize;
        let items = args.get("items").and_then(Value::as_u64).unwrap_or(0);
        let mut m = ChatMessage {
            role: "tool".into(),
            tool_call_id: id.clone(),
            content: ChatContent::SimpleText(format!("head{}", "t".repeat(text_bytes - 4))),
            tool_failed: Some(args.get("fail") == Some(&Value::Bool(true))),
            ..Default::default()
        };
        m.extra.insert(
            "tool_output".into(),
            json!({"items": (0..items).map(|i| json!({"i": i, "name": format!("n{i}")})).collect::<Vec<_>>()}),
        );
        let mut out = vec![ContextEnum::ChatMessage(m)];
        if let Some(lines) = args.get("file_lines").and_then(Value::as_u64) {
            out.push(ContextEnum::ContextFile(
                crate::call_validation::ContextFile {
                    file_name: "virtual://large.txt".into(),
                    file_content: (0..lines)
                        .map(|i| format!("line {i} of the large file\n"))
                        .collect(),
                    line1: 1,
                    line2: lines as usize,
                    skip_pp: true,
                    ..Default::default()
                },
            ));
        }
        Ok((false, out))
    }
    fn tool_description(&self) -> ToolDesc {
        let mut d = ToolPython::default().tool_description();
        d.name = "large_fixture".into();
        d.input_schema = json!({"type":"object","properties":{"text_bytes":{"type":"integer"},"items":{"type":"integer"},"fail":{"type":"boolean"},"file_lines":{"type":"integer"}}});
        d
    }
}
fn with_large_tool(mut app: AppState) -> AppState {
    app.tool_registry = Arc::new(
        crate::app_state::AppToolRegistry::with_fixture_tool_factory(
            app.gcx.clone(),
            Arc::new(|| vec![Box::new(ToolPython::default()), Box::new(LargeOutputTool)]),
        ),
    );
    app
}
fn saved_result(python: &Value) -> Value {
    let dir = std::path::Path::new(python["run_dir"].as_str().unwrap());
    serde_json::from_slice(&std::fs::read(dir.join("result.json")).unwrap()).unwrap()
}

#[tokio::test]
async fn nested_result_of_1_mib_text_and_50000_items_arrives_intact() {
    let (_dir, app, _) = fixture().await;
    let m = execute(
        with_large_tool(app),
        &[call(
            "large",
            "r = await large_fixture(text_bytes=1048576, items=50000)\n[r['ok'], r['truncated'], len(r['data']['items']), r['data']['items'][-1]['name'], len(r['text'])]",
            false,
        )],
    )
    .await;
    let python = &m[0].extra["python"];
    assert_eq!(python["status"], "completed", "{python}");
    assert_eq!(
        saved_result(python),
        json!([true, false, 50000, "n49999", 1048576])
    );
}

#[tokio::test]
async fn nested_result_over_the_cap_arrives_as_a_truncated_dict() {
    let (dir, app, _) = fixture().await;
    std::fs::write(
        dir.path().join("project/.refact/coordination.yaml"),
        "python_max_nested_result_bytes: 65536\n",
    )
    .unwrap();
    let m = execute(
        with_large_tool(app),
        &[call(
            "over-cap",
            "out = []\nfor fail in [False, True]:\n    r = await large_fixture(text_bytes=300000, items=20000, fail=fail)\n    note, rest = r['text'].split('\\n', 1)\n    out.append([r['ok'], r['truncated'], r['data'], 'python_max_nested_result_bytes' in note, rest[:4], len(r['text']) > 1000, r['error'] is None])\nout",
            false,
        )],
    )
    .await;
    let python = &m[0].extra["python"];
    assert_eq!(python["status"], "completed", "{python}");
    assert_eq!(
        saved_result(python),
        json!([
            [true, true, null, true, "head", true, true],
            [false, true, null, true, "head", true, false]
        ])
    );
}

#[tokio::test]
async fn truncated_last_expression_is_flagged_to_the_model() {
    let (_dir, _app, ccx) = fixture().await;
    let m = run(ccx, "[{'i': i} for i in range(300000)]", false).await;
    assert_eq!(
        m.extra["python"]["result_truncated"], true,
        "{}",
        m.extra["python"]
    );
    let text = m.content.content_text_only();
    assert!(text.contains("over the result limit"), "{text:.2000}");
    let (_dir, _app, ccx) = fixture().await;
    let m = run(ccx, "[{'i': i} for i in range(1000)]", false).await;
    assert!(m.extra["python"].get("result_truncated").is_none());
}

#[tokio::test]
#[serial_test::serial(python_llm_backend)]
async fn llm_answer_of_300_kb_returns_whole() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (_dir, app, _) = fixture().await;
        let mut caps = crate::caps::CodeAssistantCaps::default();
        for id in ["model", "cheap/light"] {
            caps.chat_models.insert(
                id.into(),
                Arc::new(crate::caps::ChatModelRecord {
                    base: crate::caps::BaseModelRecord {
                        id: id.into(),
                        tokenizer: "fake".into(),
                        n_ctx: 32_000,
                        endpoint: "https://example.invalid/v1".into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            );
        }
        caps.defaults.chat_default_model = "model".into();
        caps.defaults.chat_light_model = "cheap/light".into();
        {
            let mut state = app.gcx.caps_state.write().await;
            state.caps = Some(Arc::new(caps));
            state.last_attempted_ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600;
        }
        let answer = format!("{}end", "a".repeat(300 * 1024));
        crate::subchat::install_model_fixture(
            "python-test-pyllm",
            vec![crate::chat::stream_core::ChoiceFinal {
                content: answer.clone(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            }],
            None,
        );
        let m = execute(
            with_tools(app),
            &[call(
                "llm-big",
                "r = await llm('long please')\n[r['ok'], len(r['text']), r['text'][-3:]]",
                false,
            )],
        )
        .await;
        crate::subchat::remove_model_fixture("python-test-pyllm");
        let python = &m[0].extra["python"];
        assert_eq!(python["status"], "completed", "{python}");
        assert_eq!(saved_result(python), json!([true, answer.len(), "end"]));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn large_context_file_reaches_the_script_cut_only_with_the_flag() {
    let (_dir, app, _) = fixture().await;
    let m = execute(
        with_large_tool(app),
        &[call(
            "large-file",
            "out = []\nfor n in [10, 200000]:\n    r = await large_fixture(text_bytes=8, file_lines=n)\n    c = r['files'][0]['content'] if r['files'] else ''\n    out.append([len(r['files']), c.count('of the large file'), r['truncated']])\nout",
            false,
        )],
    )
    .await;
    let python = &m[0].extra["python"];
    assert_eq!(python["status"], "completed", "{python}");
    let out = saved_result(python);
    assert_eq!(out[0], json!([1, 10, false]), "{out}");
    let big = out[1].as_array().unwrap();
    assert_eq!(big[2], true, "a cut file must set truncated: {out}");
    assert!(big[1].as_u64().unwrap() < 200000, "{out}");
}

#[tokio::test]
async fn a_save_without_an_answer_retires_the_worker_and_says_where_variables_come_from() {
    let (_dir, app, ccx) = fixture().await;
    run(ccx.clone(), "x = 1", false).await;
    let ws = PythonWorkspace::for_chat(&app, "python-test")
        .await
        .unwrap();
    crate::python_tool::workspace::arm_save_fault(ws.dir(), "snapshot_timeout");
    let failed = run(ccx.clone(), "x = 2", false).await;
    let text = failed.content.content_text_only();
    assert!(
        text.contains("State: not saved (Python worker gave no answer within ")
            && text.contains("); variables come from the last successful save"),
        "{text}"
    );
    let next = run(ccx.clone(), "x", false).await;
    assert!(
        next.content.content_text_only().contains("Result:\n1"),
        "{:?}",
        next.content
    );
    assert_eq!(next.extra["python"]["state"]["restored"], true);
    assert!(next.extra["python"]["state"]["reset_reason"].is_null());
}

#[tokio::test]
async fn a_large_state_names_its_largest_variables_until_it_grows_again() {
    tokio::time::timeout(Duration::from_secs(120), async {
        let (_dir, _app, ccx) = fixture().await;
        let first = run(ccx.clone(), "big = 'x' * 17000000", false).await;
        let text = first.content.content_text_only();
        assert!(
            text.contains("State: the session holds 17.0 MB of variables. Largest: big str[17000000] ~17.0 MB. Set variables you no longer need to None (del is not supported), or start over with restart=true."),
            "{text}"
        );
        let quiet = run(ccx.clone(), "y = 1", false).await;
        let text = quiet.content.content_text_only();
        assert!(!text.contains("the session holds"), "{text}");
        let grown = run(ccx.clone(), "more = 'z' * 9000000", false).await;
        let text = grown.content.content_text_only();
        assert!(
            text.contains("Largest: big str[17000000] ~17.0 MB, more str[9000000] ~9.0 MB"),
            "{text}"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_session_whose_variables_cannot_be_listed_says_so_in_its_metadata() {
    let (_dir, app, ccx) = fixture().await;
    run(ccx.clone(), "def locals():\n    return {}", false).await;
    let ws = PythonWorkspace::for_chat(&app, "python-test")
        .await
        .unwrap();
    let meta: Value =
        serde_json::from_slice(&std::fs::read(ws.dir().join("state.json")).unwrap()).unwrap();
    assert_eq!(meta["variables_unknown"], true, "{meta}");
    assert_eq!(meta["variables"], json!([]), "{meta}");
}

#[tokio::test]
async fn optional_state_fields_are_sent_only_when_they_say_something() {
    let (dir, _app, ccx) = fixture().await;
    let ordinary = run(ccx.clone(), "x = 1", false).await;
    let state = ordinary.extra["python"]["state"]
        .as_object()
        .unwrap()
        .clone();
    for key in ["not_saved", "note", "notes"] {
        assert!(!state.contains_key(key), "{key}: {state:?}");
    }
    std::fs::write(
        dir.path().join("project/.refact/coordination.yaml"),
        "python_max_state_bytes: 1048576\n",
    )
    .unwrap();
    let refused = run(ccx, "big = 'b' * 2000000", false).await;
    let state = &refused.extra["python"]["state"];
    assert_eq!(state["persisted"], false);
    assert!(
        state["not_saved"]
            .as_str()
            .unwrap()
            .starts_with("not saved (state is 2.0 MB, over the limit 1.0 MB)"),
        "{state}"
    );
    assert!(
        state.get("note").is_none() && state.get("notes").is_none(),
        "{state}"
    );
    assert!(!refused
        .content
        .content_text_only()
        .contains("State: not persisted"));
}

#[tokio::test]
async fn a_fork_note_reaches_the_next_run_of_the_destination_and_is_not_a_reset() {
    let (_dir, app, ccx) = fixture().await;
    app.chat.sessions.write().await.insert(
        "fork-source".into(),
        Arc::new(AMutex::new(crate::chat::types::ChatSession::new(
            "fork-source".into(),
        ))),
    );
    let source = PythonWorkspace::for_chat(&app, "fork-source")
        .await
        .unwrap();
    source.ensure_dirs().await.unwrap();
    std::fs::write(source.dir().join("state.json"), "{not json").unwrap();
    let outcome = PythonWorkspace::fork(&app, "fork-source", "python-test")
        .await
        .unwrap();
    assert_eq!(
        outcome.summary().as_deref(),
        Some("Python variables were not copied (missing or invalid Python state metadata).")
    );
    let m = run(ccx, "1", false).await;
    let text = m.content.content_text_only();
    assert!(
        text.contains("State: variables were not copied from chat fork-source: missing or invalid Python state metadata"),
        "{text}"
    );
    assert!(!text.contains("State: reset"), "{text}");
    assert!(m.extra["python"]["state"]["reset_reason"].is_null());
    assert_eq!(
        m.extra["python"]["state"]["notes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_during_the_grep_listing_keeps_the_session() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let (_dir, _app, ccx) = fixture().await;
        let first = run(ccx.clone(), "x = 41\nPROJECT_ROOT", false).await;
        let root = saved_result(&first.extra["python"]);
        let pause = crate::python_tool::workspace::arm_blocking_pause(
            std::path::Path::new(root.as_str().unwrap()),
            "grep_listing",
        );
        let abort = ccx.lock().await.abort_flag.clone();
        let stopped = tokio::spawn({
            let ccx = ccx.clone();
            async move { run(ccx, "grep('TODO', '.')", false).await }
        });
        tokio::task::spawn_blocking({
            let abort = abort.clone();
            move || {
                pause.reached();
                abort.store(true, Ordering::Release);
                pause.release();
            }
        })
        .await
        .unwrap();
        let stopped = stopped.await.unwrap();
        assert_eq!(stopped.extra["python"]["status"], "cancelled");
        abort.store(false, Ordering::Release);
        let next = run(ccx, "x + 1", false).await;
        let text = next.content.content_text_only();
        assert!(text.contains("Result:\n42"), "{text}");
        assert_eq!(next.extra["python"]["state"]["restored"], false);
        assert!(next.extra["python"]["state"]["reset_reason"].is_null());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn script_writes_follow_saved_child_tools_a_tool_wide_deny_rule_and_mv() {
    let (_dir, app, ccx) = fixture().await;
    let agent = app
        .tool_registry
        .acquire_tool_catalog("agent", Some("model"), None)
        .await;
    for (tools, writable) in [
        (vec!["cat", "python"], false),
        (vec!["cat", "python", "mv"], true),
        (vec!["cat", "python", "rm"], true),
    ] {
        let mut config = crate::subchat::test_support::minimal_config("python-test", "parent");
        config.tools =
            crate::subchat::ToolsPolicy::Only(tools.iter().map(|tool| tool.to_string()).collect());
        app.agent_executions.remember_config("saved-agent", config);
        ccx.lock().await.background_agent_id = Some("saved-agent".into());
        assert_eq!(
            super::script_writes(&app, &ccx, &agent, &[], "agent", "model").await,
            writable,
            "{tools:?}"
        );
    }
    ccx.lock().await.background_agent_id = Some("agent-without-saved-tools".into());
    assert!(!super::script_writes(&app, &ccx, &agent, &[], "agent", "model").await);
    ccx.lock().await.background_agent_id = None;
    let only_mv = vec!["cat".to_string(), "python".into(), "mv".into()];
    assert!(super::script_writes(&app, &ccx, &agent, &only_mv, "agent", "model").await);
    let rule =
        |pattern: &str, action: &str| crate::yaml_configs::customization_types::ToolConfirmRule {
            match_pattern: pattern.into(),
            action: action.into(),
        };
    let root = crate::files_correction::get_project_dirs(app.gcx.clone())
        .await
        .remove(0);
    for (rules, writable) in [
        (vec![rule("mv", "auto"), rule("*", "deny")], true),
        (vec![rule("*", "deny")], false),
    ] {
        let mut registry =
            crate::yaml_configs::customization_registry::get_project_registry(app.gcx.clone())
                .await
                .unwrap();
        registry.modes.get_mut("agent").unwrap().tool_confirm.rules = rules;
        app.gcx
            .project_registry_cache
            .write()
            .unwrap()
            .insert(root.clone(), registry);
        app.gcx.tool_catalog_generations.advance_customization();
        assert_eq!(
            super::script_writes(&app, &ccx, &agent, &[], "agent", "model").await,
            writable
        );
    }
}

#[tokio::test]
async fn file_helpers_report_reads_and_writes_with_a_diff_and_make_no_call_records() {
    let (dir, _app, ccx) = fixture().await;
    let notes = dir.path().join("project/notes.txt");
    std::fs::write(&notes, "alpha\nbeta\n").unwrap();
    let m = run(
        ccx,
        "t = read('notes.txt', numbered=False)\nreplace('notes.txt', 'beta', 'gamma')\nt",
        false,
    )
    .await;
    let python = &m.extra["python"];
    assert_eq!(python["status"], "completed", "{python}");
    assert_eq!(saved_result(python), json!("alpha\nbeta\n"));
    assert_eq!(std::fs::read_to_string(&notes).unwrap(), "alpha\ngamma\n");
    let read = python["files_read"].as_array().unwrap();
    assert!(
        read.iter()
            .any(|path| path.as_str().unwrap().ends_with("notes.txt")),
        "{python}"
    );
    let written = python["files_written"].as_array().unwrap();
    assert_eq!(written.len(), 1, "{python}");
    assert!(written[0]["path"].as_str().unwrap().ends_with("notes.txt"));
    let diff = serde_json::to_string(&written[0]["diffs"]).unwrap();
    assert!(diff.contains("beta") && diff.contains("gamma"), "{diff}");
    assert_eq!(python["calls"], json!([]));
    assert_eq!(python["calls_total"], 0);
}

#[tokio::test]
async fn audited_scenario_file_helper_ranges_context_and_replace_mismatch() {
    let (dir, _app, ccx) = fixture().await;
    let root = dir.path().join("project");
    let notes = root.join("notes.txt");
    let original = "header\nbefore\nneedle\ncafé after\ntail needle";
    std::fs::write(&notes, original).unwrap();
    std::fs::write(root.join("other.txt"), "needle\n").unwrap();

    let m = run(
        ccx.clone(),
        "span = read('notes.txt', start=2, end=3)\ntail = read('notes.txt', -2, numbered=False)\nhits = grep('^needle$', 'notes.txt', context=1)\n[span, tail, hits]",
        false,
    )
    .await;
    let python = &m.extra["python"];
    assert_eq!(python["status"], "completed", "{python}");
    assert_eq!(
        saved_result(python),
        json!([
            "   2 | before\n   3 | needle\n",
            "café after\ntail needle",
            "notes.txt-2- before\nnotes.txt:3: needle\nnotes.txt-4- café after"
        ])
    );

    let m = run(
        ccx,
        "replace('notes.txt', 'needle', 'thread', count=1)",
        false,
    )
    .await;
    let python = &m.extra["python"];
    assert_eq!(m.tool_failed, Some(true), "{python}");
    assert_eq!(python["error"]["type"], "ValueError", "{python}");
    assert!(python["error"]["message"].as_str().unwrap().starts_with(
        "replace(): found 2 occurrences of old in notes.txt, expected 1; the file is unchanged."
    ), "{python}");
    assert_eq!(std::fs::read(&notes).unwrap(), original.as_bytes());
    assert_eq!(python["files_written"], json!([]), "{python}");
    assert_eq!(saved_result(python), Value::Null);
    let run_dir = std::path::Path::new(python["run_dir"].as_str().unwrap());
    let files = crate::python_tool::run_output::run_data_files(run_dir)
        .await
        .unwrap();
    assert!(
        !files
            .iter()
            .any(|path| path.starts_with(run_dir.join("captures"))
                || path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("diff-")),
        "unexpected capture or diff: {files:?}"
    );
}

#[tokio::test]
async fn audited_scenario_capped_output_keeps_error_hint_and_state_notice() {
    let (dir, _app, ccx) = fixture().await;
    std::fs::write(
        dir.path().join("project/.refact/coordination.yaml"),
        "python_max_result_chars: 1000\n",
    )
    .unwrap();
    let m = run(ccx, "print('y' * 50000)\nstr.maketrans('a', 'b')", true).await;
    let python = &m.extra["python"];
    assert_eq!(m.tool_failed, Some(true), "{python}");
    assert_eq!(python["error"]["type"], "AttributeError", "{python}");
    assert_eq!(python["state"]["reset_reason"], "requested restart");
    assert_eq!(python["stdout_bytes"], 50_001);
    let hint = "Translate with a dict: ''.join(table.get(c, c) for c in s).";
    assert_eq!(python["error"]["hint"], hint);
    let text = m.content.content_text_only();
    assert!(text.chars().count() <= 1000, "{text}");
    assert!(text.starts_with(&format!(
        "Error: AttributeError: type object 'str' has no attribute 'maketrans'\nHint: {hint}\nState: reset (requested restart)\n\nPrinted output:\nyyy"
    )), "{text}");
    assert!(text.contains("more chars"), "{text}");
}

#[tokio::test]
async fn audited_scenario_production_bindings_reject_shadowing_without_losing_state() {
    let (_dir, _app, ccx) = fixture().await;
    let m = run(
        ccx.clone(),
        "import os\nimport os.path\nimport re\nimport json as json\nimport asyncio\nfrom pathlib import Path\nkept = json.loads('41')\n[kept, os.path.basename('a/b'), re.sub('a', 'b', 'a'), Path('a/b').name]",
        false,
    )
    .await;
    assert_eq!(m.tool_failed, Some(false), "{m:?}");
    assert_eq!(saved_result(&m.extra["python"]), json!([41, "b", "b", "b"]));

    for (code, diagnostic, recovery) in [
        (
            "def show(value):\n    return value",
            "line 2 defines 'show', a helper bound by this session.",
            "Rename it (for example 'show_').",
        ),
        (
            "rm = []",
            "line 2 binds 'rm', an async function bound by this session (a tool or helper).",
            "Rename it (for example 'rm_').",
        ),
        (
            "import glob",
            "line 2 imports glob, but glob is the file-search tool (await glob(pattern=...));",
            "for local patterns use Path(dir).glob(...) or .rglob(...). Remove the import.",
        ),
    ] {
        let m = run(ccx.clone(), &format!("print('ran')\n{code}"), false).await;
        let python = &m.extra["python"];
        assert_eq!(m.tool_failed, Some(true), "{code}: {python}");
        assert_eq!(python["status"], "failed", "{code}: {python}");
        assert_eq!(python["error"]["type"], "SyntaxError", "{code}: {python}");
        assert_eq!(python["error"]["line"], 2, "{code}: {python}");
        let message = python["error"]["message"].as_str().unwrap();
        assert!(message.starts_with(diagnostic), "{code}: {message}");
        assert!(
            message.ends_with(&format!("{recovery} Nothing in this script ran.")),
            "{code}: {message}"
        );
        assert_eq!(python["stdout_preview"], "", "{code}: {python}");
        assert_eq!(python["stdout_bytes"], 0, "{code}: {python}");
    }

    let m = run(ccx, "kept + 1", false).await;
    assert_eq!(m.tool_failed, Some(false), "{m:?}");
    assert_eq!(saved_result(&m.extra["python"]), json!(42));
    assert_eq!(m.extra["python"]["state"]["restored"], false);
    assert!(m.extra["python"]["state"]["reset_reason"].is_null());
}

#[tokio::test]
async fn audited_scenario_interpreter_hints_and_stringio_round_trip() {
    let (dir, _app, ccx) = fixture().await;
    let original = "café\nsecond line\n";
    std::fs::write(dir.path().join("project/labels.txt"), original).unwrap();
    for (code, exception, diagnostic, hint) in [
        (
            "next(item for item in ['pear'])",
            "TypeError",
            "'list' object is not an iterator",
            "Generator expressions and map/filter/zip/enumerate/reversed return lists here: use next(iter(...), None) or index the list.",
        ),
        (
            "re.subn('a', 'b', 'aaa')",
            "AttributeError",
            "module 're' has no attribute 'subn'",
            "Use new = re.sub(p, r, s) and n = len(re.findall(p, s)).",
        ),
        (
            "Path('labels.txt').read_text(encoding='utf-8')",
            "TypeError",
            "read_text() takes no",
            "Files are always UTF-8 here: call p.read_text() or p.write_text(s) without encoding or errors; for undecodable data use p.read_bytes().decode('utf-8', errors='replace').",
        ),
    ] {
        let m = run(ccx.clone(), code, false).await;
        let error = &m.extra["python"]["error"];
        assert_eq!(m.tool_failed, Some(true), "{code}: {error}");
        assert_eq!(error["type"], exception, "{code}: {error}");
        let message = error["message"].as_str().unwrap();
        assert!(message.contains(diagnostic), "{code}: {message}");
        assert_eq!(error["hint"], hint, "{code}: {error}");
        let text = m.content.content_text_only();
        assert!(text.starts_with(&format!("Error: {exception}: {message}\nHint: {hint}\n")), "{code}: {text}");
    }

    let m = run(
        ccx,
        "import io\nbuf = io.StringIO()\nbuf.write(Path('labels.txt').read_text())\nbuf.seek(0)\nbuf.read()",
        false,
    )
    .await;
    assert_eq!(m.tool_failed, Some(false), "{m:?}");
    assert_eq!(saved_result(&m.extra["python"]), json!(original));
}

#[tokio::test]
async fn grep_leaves_out_ignored_and_blocked_files_and_records_a_derived_zone_it_matched() {
    let (dir, app, ccx) = fixture().await;
    let project = dir.path().join("project/src");
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    std::fs::write(
        dir.path().join("config/indexing.yaml"),
        "blocklist:\n  - ignored\nadditional_indexing_dirs: []\n",
    )
    .unwrap();
    crate::files_blocklist::reload_indexing_everywhere_now(app.gcx.clone()).await;
    std::fs::create_dir_all(project.join("ignored")).unwrap();
    std::fs::write(project.join("ignored/hidden.txt"), "needle ignored\n").unwrap();
    std::fs::write(project.join("blocked.txt"), "needle blocked\n").unwrap();
    std::fs::write(project.join("derived.txt"), "needle derived\n").unwrap();
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.blocked = vec![project.join("blocked.txt").to_string_lossy().into_owned()];
    policy.zones = vec![refact_privacy::Zone {
        name: "restricted".into(),
        patterns: vec!["no-file-is-named-like-this".into()],
        send_to: vec!["model".into()],
        ..Default::default()
    }];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
    let derived = ccx.lock().await.derived_privacy_zones.clone();
    derived.write().unwrap().insert(
        project.join("derived.txt"),
        crate::privacy::records::DerivedZone {
            zone: "restricted".into(),
            origin: Some("source.txt".into()),
        },
    );
    let m = run(ccx, "grep('needle', 'src', data=True)", false).await;
    let python = &m.extra["python"];
    assert_eq!(python["status"], "completed", "{python}");
    let out = saved_result(python);
    let matched: Vec<_> = out["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["text"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(matched, ["needle derived"], "{out}");
    assert!(out["files_skipped"].as_u64().unwrap() >= 1, "{out}");
    let text = m.content.content_text_only();
    assert!(
        !text.contains("needle blocked") && !text.contains("needle ignored"),
        "{text}"
    );
    let records = m.extra["privacy"]["files"].as_array().unwrap();
    assert!(
        records.iter().any(
            |record| record["path"].as_str().unwrap().ends_with("derived.txt")
                && record["zone"] == "restricted"
        ),
        "{records:?}"
    );
}

#[tokio::test]
async fn grep_discards_results_when_the_admission_snapshot_changes() {
    let (dir, app, ccx) = fixture().await;
    let root = crate::files_correction::get_active_project_path(app.gcx.clone())
        .await
        .unwrap();
    std::fs::write(dir.path().join("project/fixture.txt"), "apple sample\n").unwrap();
    let pause = crate::python_tool::workspace::arm_pause(&root, "grep_scanned");
    let (message, ()) = tokio::time::timeout(Duration::from_secs(60), async {
        tokio::join!(
            run(ccx, "print(grep('apple', 'fixture.txt', data=True))", false),
            async {
                pause.reached().await;
                let blocked = vec!["fixture.txt".into()];
                crate::privacy::save_privacy_policy(
                    app.gcx.clone(),
                    refact_privacy::PrivacyPolicy {
                        blocked: blocked.clone(),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
                let load = app.gcx.privacy_policy_load.read().unwrap();
                assert!(load.error.is_none(), "{:?}", load.error);
                assert_eq!(load.policy.blocked, blocked);
                pause.release();
            }
        )
    })
    .await
    .unwrap();
    let python = &message.extra["python"];
    assert_eq!(message.tool_failed, Some(true), "{python}");
    assert_eq!(python["status"], "failed", "{python}");
    assert_eq!(python["error"]["type"], "RuntimeError", "{python}");
    assert_eq!(
        python["error"]["message"], "grep(): results unavailable",
        "{python}"
    );
    assert_eq!(python["stdout_bytes"], 0, "{python}");
    assert_eq!(saved_result(python), Value::Null, "{python}");
    assert_eq!(python["files_read"], json!([]), "{python}");
    assert!(
        !message.extra.contains_key("privacy"),
        "{:?}",
        message.extra
    );
}

#[tokio::test]
async fn grep_blocks_project_relative_paths_before_reading() {
    let (dir, app, ccx) = fixture().await;
    let src = dir.path().join("project/src");
    std::fs::create_dir(&src).unwrap();
    std::fs::write(src.join("blocked.txt"), "fruit plum\n").unwrap();
    std::fs::write(src.join("allowed.txt"), "fruit pear\n").unwrap();
    crate::privacy::save_privacy_policy(
        app.gcx.clone(),
        refact_privacy::PrivacyPolicy {
            blocked: vec!["src/blocked*.txt".into()],
            zones: vec![refact_privacy::Zone {
                name: "allowed-fixture".into(),
                patterns: vec!["src/allowed.txt".into()],
                send_to: vec!["model".into()],
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let explicit = run(
        ccx.clone(),
        "[grep('fruit', 'src/blocked.txt', data=True), grep('fruit', 'src/blocked.txt')]",
        false,
    )
    .await;
    let python = &explicit.extra["python"];
    assert_eq!(explicit.tool_failed, Some(false), "{python}");
    assert_eq!(
        saved_result(python),
        json!([
            {"matches": [], "total": 0, "truncated": false, "files_searched": 0, "files_skipped": 1},
            "no matches (searched 0 files)\nskipped: 1 privacy-blocked file"
        ])
    );
    assert_eq!(python["files_read"], json!([]), "{python}");
    assert!(
        !explicit.extra.contains_key("privacy"),
        "{:?}",
        explicit.extra
    );

    let directory = run(ccx, "grep('fruit', 'src', data=True)", false).await;
    let python = &directory.extra["python"];
    assert_eq!(directory.tool_failed, Some(false), "{python}");
    assert_eq!(
        saved_result(python),
        json!({
            "matches": [{"path": "src/allowed.txt", "line": 1, "text": "fruit pear"}],
            "total": 1, "truncated": false, "files_searched": 1, "files_skipped": 1
        })
    );
    let reads = python["files_read"].as_array().unwrap();
    assert_eq!(reads.len(), 1, "{reads:?}");
    let read = reads[0].as_str().unwrap();
    assert!(
        std::path::Path::new(read).ends_with("src/allowed.txt"),
        "{reads:?}"
    );
    assert_eq!(
        directory.extra["privacy"]["files"],
        json!([{"path": read.replace('\\', "/"), "zone": "allowed-fixture", "attribution": "observed"}])
    );
}

fn listed(answer: Result<(Value, Arc<Snapshot>), String>) -> Value {
    answer.unwrap().0
}

#[tokio::test]
async fn the_grep_listing_walks_only_the_requested_folder_and_globs_before_the_privacy_partition() {
    let (dir, app, _) = fixture().await;
    let src = dir.path().join("project/src");
    let extra = dir.path().join("extra");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&extra).unwrap();
    for name in ["a.rs", "b.rs", "secret.env"] {
        std::fs::write(src.join(name), "x").unwrap();
    }
    for n in 0..5 {
        std::fs::write(extra.join(format!("{n}.rs")), "x").unwrap();
    }
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    std::fs::write(
        dir.path().join("config/indexing.yaml"),
        format!(
            "blocklist: []\nadditional_indexing_dirs:\n  - {}\n",
            extra.display()
        ),
    )
    .unwrap();
    crate::files_blocklist::reload_indexing_everywhere_now(app.gcx.clone()).await;
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.blocked = vec![src.join("secret.env").to_string_lossy().into_owned()];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
    let abort = std::sync::atomic::AtomicBool::new(false);
    let request = |glob: Option<&str>| json!([{"path": src, "max_entries": 4, "glob": glob}]);
    let all = listed(super::grep_files(app.gcx.clone(), &request(None), &abort).await);
    assert_eq!(
        (
            all["files"].as_array().unwrap().len(),
            &all["blocked"],
            &all["truncated"]
        ),
        (2, &json!(1), &json!(false)),
        "{all}"
    );
    let rust = listed(super::grep_files(app.gcx.clone(), &request(Some("*.rs")), &abort).await);
    assert_eq!(
        (rust["files"].as_array().unwrap().len(), &rust["blocked"]),
        (2, &json!(0)),
        "{rust}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn the_grep_listing_leaves_out_and_counts_names_that_are_not_utf8() {
    use std::os::unix::ffi::OsStrExt;
    let (dir, app, _) = fixture().await;
    let src = dir.path().join("project/src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join(std::ffi::OsStr::from_bytes(b"caf\xe9.txt")), "x").unwrap();
    std::fs::write(src.join("plain.txt"), "x").unwrap();
    let abort = std::sync::atomic::AtomicBool::new(false);
    let list = listed(
        super::grep_files(
            app.gcx.clone(),
            &json!([{"path": src, "max_entries": 100}]),
            &abort,
        )
        .await,
    );
    assert_eq!(list["unlisted"], 1, "{list}");
    assert_eq!(list["files"].as_array().unwrap().len(), 1, "{list}");
}

#[tokio::test]
async fn the_grep_listing_returns_at_most_twenty_thousand_files_whatever_is_asked() {
    let (dir, app, _) = fixture().await;
    let many = dir.path().join("project/many");
    std::fs::create_dir_all(&many).unwrap();
    for n in 0..20_001 {
        std::fs::File::create(many.join(n.to_string())).unwrap();
    }
    let abort = std::sync::atomic::AtomicBool::new(false);
    let list = listed(
        super::grep_files(
            app.gcx.clone(),
            &json!([{"path": many, "max_entries": 1_000_000}]),
            &abort,
        )
        .await,
    );
    assert_eq!(list["files"].as_array().unwrap().len(), 20_000);
    assert_eq!(list["truncated"], true);
}
