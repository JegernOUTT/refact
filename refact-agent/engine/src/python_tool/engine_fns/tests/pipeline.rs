use super::*;

#[tokio::test]
async fn unknown_name_suggests_close_matches_and_bad_arguments_raise_type_error() {
    let (_dir, app, _) = fixture().await;
    let m = run(
        &app,
        "out = []\nfor call in [lambda: engine('settings.gte'), lambda: engine('nothing.here'), lambda: engine('settings.get', keys=5), lambda: engine('settings.get', bogus=1), lambda: engine()]:\n    try:\n        await call()\n        out.append('ok')\n    except TypeError as e:\n        out.append(str(e))\nout",
    )
    .await;
    let out = result(&m);
    assert!(
        out[0]
            .as_str()
            .unwrap()
            .contains("unknown engine function 'settings.gte'"),
        "{out}"
    );
    assert!(out[0].as_str().unwrap().contains("settings.get"), "{out}");
    assert!(
        out[1].as_str().unwrap().contains("help(\"engine\")"),
        "{out}"
    );
    assert!(out[2].as_str().unwrap().contains("keys"), "{out}");
    assert!(out[3].as_str().unwrap().contains("bogus"), "{out}");
    assert!(out[4].as_str().unwrap().contains("missing"), "{out}");
}

#[tokio::test]
async fn reads_off_unbind_and_undescribe_engine() {
    let (dir, app, _) = fixture().await;
    coordination(&dir, "python_engine_read_enabled: false\n");
    let m = run(
        &app,
        "try:\n    engine\n    r = 'bound'\nexcept NameError:\n    r = 'unbound'\nr",
    )
    .await;
    assert_eq!(result(&m), "unbound");
    let settings = crate::runtime_settings::coordination_for_project(&dir.path().join("project"))
        .await
        .unwrap();
    let options = crate::python_tool::catalog_binding::BindOptions::from_settings(&settings);
    assert!(!options.engine);
    let catalog = crate::python_tool::catalog_binding::catalog_with_options(&[], false, options);
    assert!(!catalog.description.contains("engine("));
    assert!(catalog.resolve("engine").is_none());
    let on = crate::python_tool::catalog_binding::catalog_with_options(
        &[],
        false,
        crate::python_tool::catalog_binding::BindOptions {
            engine: true,
            ..options
        },
    );
    assert!(on
        .description
        .contains("async def engine(name: str, **kwargs)"));
    let mut areas: Vec<&str> = super::super::functions().iter().map(|f| f.area()).collect();
    areas.dedup();
    assert!(on
        .description
        .contains(&format!("# engine areas: {};", areas.join(", "))));
}

#[tokio::test]
async fn change_and_risky_switches_refuse_before_asking() {
    let (dir, app, _) = fixture().await;
    coordination(
        &dir,
        "python_engine_change_enabled: false\npython_engine_risky_enabled: false\n",
    );
    let m = run(
        &app,
        "out = []\nfor name in ['testing.touch', 'testing.wipe']:\n    try:\n        await engine(name, n=2)\n    except PermissionError as e:\n        out.append(str(e))\nr = await engine('settings.get', keys=['python_max_llm_calls'])\nout.append(r['ok'])\nout",
    )
    .await;
    let out = result(&m);
    assert!(
        out[0]
            .as_str()
            .unwrap()
            .contains("python_engine_change_enabled=false"),
        "{out}"
    );
    assert!(
        out[1]
            .as_str()
            .unwrap()
            .contains("python_engine_risky_enabled=false"),
        "{out}"
    );
    assert_eq!(out[2], true);
    assert!(session(&app)
        .await
        .lock()
        .await
        .runtime
        .python_approvals
        .is_empty());
    coordination(&dir, "python_engine_risky_enabled: false\n");
    let m = run(
        &app,
        "try:\n    await engine('testing.wipe')\nexcept PermissionError as e:\n    r = str(e)\nr",
    )
    .await;
    assert!(result(&m)
        .as_str()
        .unwrap()
        .contains("python_engine_risky_enabled=false"));
}

#[tokio::test]
async fn nested_call_budget_raises_runtime_error() {
    let (dir, app, _) = fixture().await;
    coordination(&dir, "python_max_nested_calls: 1\n");
    let m = run(
        &app,
        "await engine('session.limits')\ntry:\n    await engine('session.limits')\n    r = 'no'\nexcept RuntimeError as e:\n    r = str(e)\nr",
    )
    .await;
    assert!(result(&m).as_str().unwrap().contains("budget"), "{m:?}");
}

#[tokio::test]
async fn a_change_is_refused_in_child_and_autonomous_chats_while_reads_work() {
    for setup in ["child", "autonomous"] {
        let (_dir, app, _) = fixture().await;
        {
            let session = session(&app).await;
            let mut session = session.lock().await;
            match setup {
                "child" => {
                    session.thread.parent_id = Some("parent".into());
                    session.thread.link_type = Some("subagent".into());
                }
                _ => session.thread.autonomous_no_confirm = true,
            }
        }
        let m = run(
            &app,
            "try:\n    await engine('testing.touch', n=2)\n    r = 'applied'\nexcept PermissionError as e:\n    r = str(e)\nok = (await engine('session.limits'))['ok']\n[r, ok]",
        )
        .await;
        let out = result(&m);
        assert!(
            out[0].as_str().unwrap().contains("need a person"),
            "{setup}: {out}"
        );
        assert_eq!(out[1], true, "{setup}");
        assert!(m.extra["python"].get("engine_changes").is_none(), "{setup}");
        assert!(session(&app)
            .await
            .lock()
            .await
            .runtime
            .python_approvals
            .is_empty());
        assert_eq!(m.extra["python"]["calls"][0]["status"], "denied");
    }
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn a_change_always_asks_despite_every_auto_approval_and_records_it() {
    crate::runtime_settings::reset_for_test();
    let mut settings = crate::runtime_settings::current();
    settings
        .coordination
        .python_engine_change_with_auto_commands = true;
    crate::runtime_settings::install_live(&settings);
    let (dir, app, _) = fixture().await;
    {
        let session = session(&app).await;
        let mut session = session.lock().await;
        session.thread.auto_approve_dangerous_commands = true;
        session.thread.auto_approve_editing_tools = true;
    }
    set_mode_rule(&app, "engine.*", "auto").await;
    let task = spawn(
        &app,
        "r1 = await engine('session.limits')\nr2 = await engine('testing.touch', n=7)\n[r1['ok'], r2['ok'], r2['data']]",
    );
    let request = next_request(&app, &task).await;
    app.python_tool
        .approvals
        .run_permits
        .lock()
        .unwrap()
        .insert(("python-test".into(), request.run_id.clone()));
    assert_eq!(request.tool_name, "engine.testing.touch");
    assert!(!request.editing_tool);
    assert!(request.needs_person);
    let engine = request.engine.clone().unwrap();
    assert_eq!(engine.function, "testing.touch");
    assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Change);
    assert_eq!(engine.title, "touch a test value");
    assert_eq!(engine.scope.as_deref(), Some("project"));
    assert_eq!(engine.items[0].new, Some(json!(7)));
    assert_eq!(engine.items[1].old, Some(json!("***")));
    assert_eq!(engine.items[1].new, None);
    assert!(engine.items[2].truncated);
    assert!(serde_json::to_string(&engine.items[2].old).unwrap().len() <= 16 * 1024);
    assert_eq!(engine.items[2].new, Some(json!(0)));
    for rejected in [Decision::AllowForRun, Decision::AllowForChat] {
        assert!(decide(&app, &request.request_id, rejected).await.is_err());
    }
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let m = finish(task).await;
    assert_eq!(result(&m), json!([true, true, {"n":7}]));
    let python = &m.extra["python"];
    let calls = python["calls"].as_array().unwrap();
    assert_eq!(calls[0]["tool"], "engine.session.limits");
    assert_eq!(calls[0]["approval"], "not_needed");
    assert_eq!(calls[1]["tool"], "engine.testing.touch");
    assert_eq!(calls[1]["approval"], "allowed");
    assert!(calls[1]["line"].is_u64() && calls[1]["col"].is_u64());
    assert!(
        calls[1]["summary"]
            .as_str()
            .unwrap()
            .starts_with("n: 1 → 7"),
        "{}",
        calls[1]
    );
    assert_eq!(python["engine_changes_total"], 1);
    let change = &python["engine_changes"][0];
    assert_eq!(change["seq"], calls[1]["seq"]);
    assert_eq!(change["function"], "testing.touch");
    assert_eq!(change["class"], "change");
    assert!(change["at_ms"].is_u64());
    let run_dir = std::path::PathBuf::from(python["run_dir"].as_str().unwrap());
    let log: Value = serde_json::from_slice(
        &std::fs::read(run_dir.join(format!("calls/{}_engine.json", calls[1]["seq"]))).unwrap(),
    )
    .unwrap();
    assert_eq!(log["tool"], "engine.testing.touch");
    assert_eq!(log["record"]["approval"], "allowed");
    assert!(!log.to_string().contains("old-secret"));
    let meta: Value =
        serde_json::from_slice(&std::fs::read(run_dir.join("run.json")).unwrap()).unwrap();
    assert_eq!(meta["engine_changes_total"], 1);
    assert!(meta.get("engine_changes").is_none());
    let summary: Value =
        serde_json::from_slice(&std::fs::read(run_dir.join("summary.json")).unwrap()).unwrap();
    assert_eq!(summary["status"], meta["status"]);
    assert_eq!(summary["calls_total"], meta["calls_total"]);
    assert_eq!(summary["engine_changes_total"], 1);
    assert!(summary.get("calls").is_none());
    let view = crate::python_tool::workspace_view::workspace_view(&app, "python-test")
        .await
        .unwrap();
    let entry = serde_json::to_value(&view.runs[0]).unwrap();
    assert_eq!(entry["engine_changes"], 1);
    let warned = spawn(
        &app,
        "await engine('testing.touch', n=8, warn='Zone warning.')",
    );
    let request = next_request(&app, &warned).await;
    let card = request.engine.clone().unwrap();
    assert_eq!(card.class, refact_chat_api::PythonEngineClass::Risky);
    assert_eq!(card.warning.as_deref(), Some("Zone warning."));
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    finish(warned).await;
    drop(dir);
    crate::runtime_settings::reset_for_test();
}

#[tokio::test]
async fn deny_raises_permission_error_and_stop_cancels_the_run() {
    let (_dir, app, _) = fixture().await;
    let task = spawn(
        &app,
        "try:\n    await engine('testing.touch', n=1)\n    r = 'applied'\nexcept PermissionError as e:\n    r = str(e)\nr",
    );
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    let m = finish(task).await;
    assert_eq!(result(&m), "rejected by the user");
    assert_eq!(m.extra["python"]["calls"][0]["approval"], "denied");
    assert!(m.extra["python"].get("engine_changes").is_none());
    let view = crate::python_tool::workspace_view::workspace_view(&app, "python-test")
        .await
        .unwrap();
    assert!(serde_json::to_value(&view.runs[0])
        .unwrap()
        .get("engine_changes")
        .is_none());

    let task = spawn(&app, "await engine('testing.touch', n=1)\n'after'");
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::Stop)
        .await
        .unwrap();
    let m = finish(task).await;
    assert_eq!(m.extra["python"]["status"], "cancelled");
    assert_eq!(m.extra["python"]["calls"][0]["approval"], "cancelled");
}

#[tokio::test]
async fn destructive_preview_cannot_lower_its_class_and_carries_the_warning() {
    let (_dir, app, _) = fixture().await;
    let task = spawn(&app, "(await engine('testing.wipe'))['ok']");
    let request = next_request(&app, &task).await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(
        engine.class,
        refact_chat_api::PythonEngineClass::Destructive
    );
    assert_eq!(
        engine.warning.as_deref(),
        Some("The test value cannot be restored.")
    );
    assert_eq!(engine.items[0].new, None);
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let m = finish(task).await;
    assert_eq!(
        m.extra["python"]["engine_changes"][0]["class"],
        "destructive"
    );
    assert!(m.extra["python"]["calls"][0]["summary"]
        .as_str()
        .unwrap()
        .contains("n: 1 → (removed)"));
}

#[tokio::test]
async fn timeout_and_failure_return_ok_false_and_outputs_are_masked() {
    let (_dir, app, _) = fixture().await;
    let m = run(
        &app,
        "a = await engine('testing.slow')\nb = await engine('testing.fails')\nc = await engine('testing.secrets')\n[a['ok'], a['error'], b['ok'], b['error'], c]",
    )
    .await;
    let out = result(&m);
    assert_eq!(out[0], false);
    assert!(out[1].as_str().unwrap().contains("timed out"), "{out}");
    assert_eq!(out[2], false);
    assert_eq!(out[3], "broken on purpose");
    let c = &out[4];
    let data = &c["data"];
    assert_eq!(data["api_key"], "***");
    assert_eq!(data["nested"]["Authorization"], "***");
    assert_eq!(data["nested"]["cookie"], "***");
    assert_eq!(data["nested"]["count_tokens"], 7);
    assert_eq!(data["args"], json!(["--token", "***", "--port", "8001"]));
    assert!(!data["url"].as_str().unwrap().contains("pw@"), "{data}");
    assert!(!data["url"].as_str().unwrap().contains("q1"), "{data}");
    assert_eq!(data["plain"], "visible");
    assert!(!c["text"].as_str().unwrap().contains("abc.def"));
    let calls = m.extra["python"]["calls"].as_array().unwrap();
    assert_eq!(calls[0]["status"], "failed");
    assert_eq!(calls[0]["approval"], "not_needed");
    let run_dir = std::path::PathBuf::from(m.extra["python"]["run_dir"].as_str().unwrap());
    let log =
        std::fs::read_to_string(run_dir.join(format!("calls/{}_engine.json", calls[2]["seq"])))
            .unwrap();
    assert!(
        !log.contains("sk-live-1") && !log.contains("abc.def"),
        "{log}"
    );
}

#[tokio::test]
async fn privacy_records_of_a_read_reach_the_outer_message() {
    let (_dir, app, _) = fixture().await;
    let m = run(&app, "(await engine('testing.content'))['data']['content']").await;
    let files = m.extra["privacy"]["files"].as_array().unwrap();
    assert!(files.iter().any(|f| f["path"] == "zone_a.txt"), "{files:?}");
}

async fn set_mode_rule(app: &AppState, pattern: &str, action: &str) {
    let gcx = app.gcx.clone();
    let mut registry =
        crate::yaml_configs::customization_registry::get_project_registry(gcx.clone())
            .await
            .unwrap();
    let mode = registry.modes.get_mut("agent").unwrap();
    mode.tool_confirm.rules.insert(
        0,
        crate::yaml_configs::customization_types::ToolConfirmRule {
            match_pattern: pattern.into(),
            action: action.into(),
        },
    );
    let root = crate::files_correction::get_project_dirs(gcx.clone())
        .await
        .into_iter()
        .next()
        .unwrap_or_else(|| gcx.config_dir.clone());
    gcx.project_registry_cache
        .write()
        .unwrap()
        .insert(root, registry);
    gcx.tool_catalog_generations.advance_customization();
}

#[tokio::test]
async fn deny_rule_refuses_before_asking() {
    let (_dir, app, _) = fixture().await;
    set_mode_rule(&app, "engine.testing.touch", "deny").await;
    let m = run(&app, "try:\n    await engine('testing.touch', n=1)\nexcept PermissionError as e:\n    r = str(e)\nr").await;
    assert!(
        result(&m).as_str().unwrap().contains("mode policy: deny"),
        "{m:?}"
    );
    assert!(session(&app)
        .await
        .lock()
        .await
        .runtime
        .python_approvals
        .is_empty());
}

#[cfg(unix)]
async fn hook_app(command: &str) -> (tempfile::TempDir, AppState) {
    let (dir, app, _) = fixture().await;
    let config = app.gcx.config_dir.clone();
    std::fs::write(
        config.join("hooks.yaml"),
        format!("hooks:\n  PreToolUse:\n    - hooks:\n        - type: command\n          command: {command:?}\n"),
    )
    .unwrap();
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
    (dir, app)
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn pre_tool_use_hook_can_deny_and_its_rewrite_is_revalidated() {
    let (_dir, app) = hook_app("echo engine-denied >&2; exit 2").await;
    let m = run(
        &app,
        "try:\n    await engine('session.limits')\nexcept PermissionError as e:\n    r = str(e)\nr",
    )
    .await;
    assert!(
        result(&m).as_str().unwrap().contains("engine-denied"),
        "{m:?}"
    );
    crate::ext::hooks_runner::clear_hook_cache_for_tests();

    let (_dir, app) =
        hook_app("echo '{\"decision\":\"rewrite\",\"tool_input\":{\"keys\":5}}'").await;
    let m = run(
        &app,
        "try:\n    await engine('settings.get')\nexcept TypeError as e:\n    r = str(e)\nr",
    )
    .await;
    assert!(result(&m).as_str().unwrap().contains("keys"), "{m:?}");
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
}

#[test]
fn who_may_change_covers_task_agent_scheduled_and_automation_chats() {
    use super::super::dispatch::unattended_kind;
    let root = refact_chat_api::ThreadParams::default();
    assert_eq!(unattended_kind(false, false, &root, "chat-1"), None);
    assert!(unattended_kind(true, false, &root, "chat-1").is_some());
    assert!(unattended_kind(false, true, &root, "chat-1").is_some());
    let mut task = root.clone();
    task.task_meta = Some(refact_chat_api::TaskMeta {
        task_id: "t".into(),
        role: "agents".into(),
        ..Default::default()
    });
    assert_eq!(
        unattended_kind(false, false, &task, "chat-1"),
        Some("a task-agent chat")
    );
    let mut planner = root.clone();
    planner.task_meta = Some(refact_chat_api::TaskMeta {
        task_id: "t".into(),
        role: "planner".into(),
        ..Default::default()
    });
    assert_eq!(unattended_kind(false, false, &planner, "chat-1"), None);
    for link in ["subagent", "delegate", "task_agent"] {
        let mut child = root.clone();
        child.link_type = Some(link.into());
        assert_eq!(
            unattended_kind(false, false, &child, "chat-1"),
            Some("a child chat")
        );
    }
    let mut branch = root.clone();
    branch.link_type = Some("branch".into());
    assert_eq!(unattended_kind(false, false, &branch, "chat-1"), None);
    assert!(unattended_kind(false, false, &root, "cron_job_3").is_some());
    let mut buddy = root.clone();
    buddy.buddy_meta = Some(
        serde_json::from_value(
            json!({"is_buddy_chat":true,"buddy_chat_kind":"workflow","workflow_id":"w"}),
        )
        .unwrap(),
    );
    assert_eq!(
        unattended_kind(false, false, &buddy, "chat-1"),
        Some("a scheduled or automation chat")
    );
    let mut auto = root;
    auto.autonomous_no_confirm = true;
    assert!(unattended_kind(false, false, &auto, "chat-1").is_some());
}

#[tokio::test]
async fn help_levels_work_in_scripts_and_engine_is_not_a_variable() {
    let (_dir, app, _) = fixture().await;
    let m = run(
        &app,
        "a = help('engine')\nb = help('engine.settings')\nc = help('engine.settings.set')\nnames = tools()\nx = 1\n[a, b, c, 'engine' in names]",
    )
    .await;
    let out = result(&m);
    assert!(
        out[0]
            .as_str()
            .unwrap()
            .contains("settings: clear_project_override, diff, get, reset, schema, set"),
        "{out}"
    );
    assert!(
        out[1].as_str().unwrap().contains("settings.set [change]"),
        "{out}"
    );
    assert!(
        out[2]
            .as_str()
            .unwrap()
            .contains("await engine(\"settings.set\", values=..., scope=None)"),
        "{out}"
    );
    assert_eq!(out[3], true);
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
        names.contains(&"x") && !names.contains(&"engine"),
        "{names:?}"
    );
}
