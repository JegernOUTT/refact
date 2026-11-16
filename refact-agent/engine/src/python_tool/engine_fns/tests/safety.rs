use super::*;

fn tools_app(app: AppState) -> AppState {
    crate::tools::tool_python::tests::with_tools(app)
}

fn path_of(dir: &tempfile::TempDir, name: &str) -> String {
    dir.path().join(name).to_string_lossy().into_owned()
}

async fn card_gone(app: &AppState, request_id: &str) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while session(app)
            .await
            .lock()
            .await
            .runtime
            .python_approvals
            .iter()
            .any(|r| r.request_id == request_id)
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the answered card stayed pending");
}

#[tokio::test]
async fn the_card_waits_for_running_calls_and_new_calls_wait_for_the_answer() {
    let (dir, app, _) = fixture().await;
    let app = tools_app(app);
    let finished = path_of(&dir, "slow_finished");
    let started = path_of(&dir, "late_started");
    let code = format!(
        "import asyncio\nasync def late():\n    await fixture(ms=300)\n    return await fixture(ms=1, started={started:?})\nr = await asyncio.gather(fixture(ms=1500, finished={finished:?}), engine('testing.touch', n=3), late())\n[x['ok'] for x in r]"
    );
    let task = spawn(&app, &code);
    let request = next_request(&app, &task).await;
    assert!(
        std::path::Path::new(&finished).exists(),
        "the card appeared while another call of the run was still running"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !std::path::Path::new(&started).exists(),
        "a call started while the card was pending ran before the answer"
    );
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let m = finish(task).await;
    assert_eq!(result(&m), json!([true, true, true]));
    assert!(std::path::Path::new(&started).exists());
}

#[tokio::test]
async fn stop_ends_a_run_whose_calls_wait_for_a_pending_card() {
    let (_dir, app, _) = fixture().await;
    let task = spawn(
        &app,
        "import asyncio\nawait asyncio.gather(engine('testing.touch', n=1), engine('session.limits'))",
    );
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::Stop)
        .await
        .unwrap();
    let m = finish(task).await;
    assert_eq!(m.extra["python"]["status"], "cancelled");
}

#[tokio::test]
async fn two_changes_in_a_gather_both_take_effect() {
    let (dir, app, _) = fixture().await;
    let a = path_of(&dir, "a");
    let b = path_of(&dir, "b");
    let task = spawn(
        &app,
        &format!("import asyncio\nr = await asyncio.gather(engine('testing.mark', path={a:?}), engine('testing.mark', path={b:?}))\n[x['ok'] for x in r]"),
    );
    for _ in 0..2 {
        let request = next_request(&app, &task).await;
        assert_eq!(
            session(&app)
                .await
                .lock()
                .await
                .runtime
                .python_approvals
                .len(),
            1,
            "two cards of one run were pending at once"
        );
        decide(&app, &request.request_id, Decision::AllowOnce)
            .await
            .unwrap();
        card_gone(&app, &request.request_id).await;
    }
    let m = finish(task).await;
    assert_eq!(result(&m), json!([true, true]));
    assert!(std::path::Path::new(&a).exists() && std::path::Path::new(&b).exists());
    assert_eq!(m.extra["python"]["engine_changes_total"], 2);
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn two_settings_changes_in_a_gather_are_both_on_disk() {
    crate::runtime_settings::reset_for_test();
    let (dir, app, _) = fixture().await;
    let task = spawn(
        &app,
        "import asyncio\nr = await asyncio.gather(engine('settings.set', values={'max_resume_burst': 7}), engine('settings.set', values={'max_priority_burst': 9}))\n[x['ok'] for x in r]",
    );
    for _ in 0..2 {
        let request = next_request(&app, &task).await;
        decide(&app, &request.request_id, Decision::AllowOnce)
            .await
            .unwrap();
        card_gone(&app, &request.request_id).await;
    }
    let m = finish(task).await;
    assert_eq!(result(&m), json!([true, true]));
    let text =
        std::fs::read_to_string(dir.path().join("project/.refact/coordination.yaml")).unwrap();
    assert!(
        text.contains("max_resume_burst: 7") && text.contains("max_priority_burst: 9"),
        "{text}"
    );
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn a_chat_that_runs_commands_without_asking_cannot_change_unless_opted_in() {
    crate::runtime_settings::reset_for_test();
    let (dir, app, _) = fixture().await;
    session(&app)
        .await
        .lock()
        .await
        .thread
        .auto_approve_dangerous_commands = true;
    let m = run(
        &app,
        "try:\n    await engine('testing.touch', n=1)\n    r = 'applied'\nexcept PermissionError as e:\n    r = str(e)\nr",
    )
    .await;
    let text = result(&m).as_str().unwrap().to_string();
    assert!(text.contains("auto_approve_dangerous_commands"), "{text}");
    assert!(text.contains("approve the change itself"), "{text}");
    assert!(
        text.contains("python_engine_change_with_auto_commands"),
        "{text}"
    );
    assert_eq!(m.extra["python"]["calls"][0]["approval"], "denied");

    coordination(&dir, "python_engine_change_with_auto_commands: true\n");
    let m = run(&app, "try:\n    await engine('testing.touch', n=1)\n    r = 'applied'\nexcept PermissionError as e:\n    r = str(e)\nr").await;
    assert!(
        result(&m)
            .as_str()
            .unwrap()
            .contains("auto_approve_dangerous_commands"),
        "a project file widened the setting"
    );

    let mut settings = crate::runtime_settings::current();
    settings
        .coordination
        .python_engine_change_with_auto_commands = true;
    crate::runtime_settings::install_live(&settings);
    let task = spawn(&app, "(await engine('testing.touch', n=1))['ok']");
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    assert_eq!(result(&finish(task).await), true);
    crate::runtime_settings::reset_for_test();
}

#[tokio::test]
async fn a_function_argument_called_name_reaches_the_function() {
    let (dir, app, _) = fixture().await;
    let path = path_of(&dir, "named");
    let task = spawn(
        &app,
        &format!("(await engine('testing.mark', path={path:?}, name='alpha'))['data']"),
    );
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    assert_eq!(result(&finish(task).await), json!({"name":"alpha"}));
}

#[tokio::test]
async fn a_plan_that_changes_nothing_raises_no_card_and_an_empty_plan_is_refused() {
    let (_dir, app, _) = fixture().await;
    let m = run(
        &app,
        "a = await engine('testing.noop')\ntry:\n    await engine('testing.noop', broken=True)\n    b = 'ran'\nexcept RuntimeError as e:\n    b = str(e)\n[a['ok'], a['text'], b]",
    )
    .await;
    let out = result(&m);
    assert_eq!(out[0], true);
    assert_eq!(out[1], "nothing to change");
    assert!(out[2].as_str().unwrap().contains("internal error"), "{out}");
    assert!(m.extra["python"].get("engine_changes").is_none());
    assert!(session(&app)
        .await
        .lock()
        .await
        .runtime
        .python_approvals
        .is_empty());
}

#[tokio::test]
async fn refusals_before_the_card_are_denied_and_reads_stay_not_needed() {
    let (dir, app, _) = fixture().await;
    coordination(&dir, "python_engine_change_enabled: false\n");
    let m = run(
        &app,
        "await engine('session.limits')\ntry:\n    await engine('testing.touch', n=1)\nexcept PermissionError:\n    pass\n1",
    )
    .await;
    let calls = m.extra["python"]["calls"].as_array().unwrap();
    assert_eq!(calls[0]["approval"], "not_needed");
    assert_eq!(calls[1]["approval"], "denied");
    assert_eq!(calls[1]["status"], "denied");
}

#[tokio::test]
async fn a_too_large_result_after_an_applied_change_is_ok_and_truncated() {
    let (dir, app, _) = fixture().await;
    coordination(&dir, "python_max_nested_result_bytes: 65536\n");
    let path = path_of(&dir, "big");
    let task = spawn(
        &app,
        &format!("r = await engine('testing.mark', path={path:?}, mode='big')\n[r['ok'], r['data'], r['truncated']]"),
    );
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let m = finish(task).await;
    assert_eq!(result(&m), json!([true, null, true]));
    assert_eq!(m.extra["python"]["calls"][0]["status"], "ok");
    assert_eq!(m.extra["python"]["engine_changes_total"], 1);
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn settings_moved_between_card_and_answer_are_refused_and_nothing_is_applied() {
    crate::runtime_settings::reset_for_test();
    let (dir, app, _) = fixture().await;
    coordination(&dir, "max_resume_burst: 8\n");
    let task = spawn(
        &app,
        "r = await engine('settings.clear_project_override')\n[r['ok'], r['error']]",
    );
    let request = next_request(&app, &task).await;
    coordination(
        &dir,
        "max_resume_burst: 8\nboard_allow_self_review: false\n",
    );
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let m = finish(task).await;
    let out = result(&m);
    assert_eq!(out[0], false);
    let error = out[1].as_str().unwrap();
    assert!(error.contains("changed since the preview"), "{error}");
    assert!(error.contains("nothing was applied"), "{error}");
    let text =
        std::fs::read_to_string(dir.path().join("project/.refact/coordination.yaml")).unwrap();
    assert!(
        text.contains("board_allow_self_review") && text.contains("max_resume_burst"),
        "{text}"
    );
    assert!(m.extra["python"].get("engine_changes").is_none());
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn a_keyless_reset_lists_every_key_and_a_guarded_one_makes_it_risky() {
    crate::runtime_settings::reset_for_test();
    let (dir, app, _) = fixture().await;
    coordination(&dir, "max_resume_burst: 8\nboard_allow_self_review: true\n");
    let task = spawn(&app, "await engine('settings.reset')");
    let request = next_request(&app, &task).await;
    let engine = request.engine.clone().unwrap();
    let mut labels: Vec<_> = engine.items.iter().map(|i| i.label.as_str()).collect();
    labels.sort();
    assert_eq!(labels, ["board_allow_self_review", "max_resume_burst"]);
    assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Risky);
    assert!(engine.warning.unwrap().contains("review their own work"));
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    finish(task).await;
    let m = run(
        &app,
        "(await engine('settings.reset', keys=['max_queue_size'], scope='global'))['text']",
    )
    .await;
    assert_eq!(result(&m), "nothing to change");
}

async fn mark_with(app: &AppState, path: &str, mode: &str) -> ChatMessage {
    let task = spawn(
        app,
        &format!(
            "r = await engine('testing.mark', path={path:?}, mode={mode:?})\n[r['ok'], r['error']]"
        ),
    );
    let request = next_request(app, &task).await;
    decide(app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    finish(task).await
}

#[tokio::test]
async fn every_exit_after_the_handler_started_records_the_change() {
    let (dir, app, _) = fixture().await;
    for (mode, outcome) in [
        ("ok", None),
        ("panic", Some("unknown")),
        ("hang", Some("unknown")),
    ] {
        let path = path_of(&dir, mode);
        let m = mark_with(&app, &path, mode).await;
        assert!(std::path::Path::new(&path).exists(), "{mode}");
        let change = &m.extra["python"]["engine_changes"][0];
        assert_eq!(
            change["function"], "testing.mark",
            "{mode}: {}",
            m.extra["python"]
        );
        assert_eq!(
            change.get("outcome").and_then(Value::as_str),
            outcome,
            "{mode}"
        );
        assert_eq!(m.extra["python"]["calls"][0]["approval"], "allowed");
        if outcome.is_some() {
            assert_eq!(result(&m)[0], false, "{mode}");
        }
    }
}

#[tokio::test]
async fn stop_after_approval_does_not_interrupt_the_handler() {
    let (dir, app, _) = fixture().await;
    let path = path_of(&dir, "stopped");
    let task = spawn(
        &app,
        &format!("await engine('testing.mark', path={path:?}, mode='hang')"),
    );
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        while !std::path::Path::new(&path).exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("handler did not start");
    app.python_tool
        .stop_run("python-test", &request.tool_call_id)
        .unwrap();
    let m = finish(task).await;
    let change = &m.extra["python"]["engine_changes"][0];
    assert_eq!(change["outcome"], "unknown", "{}", m.extra["python"]);
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn a_post_hook_deny_keeps_the_change_listed_and_says_it_is_in_place() {
    let (dir, app, _) = fixture().await;
    std::fs::write(
        app.gcx.config_dir.join("hooks.yaml"),
        "hooks:\n  PostToolUse:\n    - hooks:\n        - type: command\n          command: \"echo post-denied >&2; exit 2\"\n",
    )
    .unwrap();
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
    let path = path_of(&dir, "hooked");
    let m = mark_with(&app, &path, "ok").await;
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
    let out = result(&m);
    assert_eq!(out[0], false);
    let error = out[1].as_str().unwrap();
    assert!(
        error.contains("hook rejected") && error.contains("in place"),
        "{error}"
    );
    let change = &m.extra["python"]["engine_changes"][0];
    assert!(change.get("outcome").is_none(), "{change}");
    assert_eq!(m.extra["python"]["engine_changes_total"], 1);
    let summary = m.extra["python"]["calls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["tool"] == "engine.testing.mark")
        .unwrap()["summary"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        summary.contains("hook rejected") && summary.contains("in place"),
        "{summary}"
    );
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn a_secret_argument_is_masked_in_the_record_file_hook_and_request() {
    let (dir, app, _) = fixture().await;
    let seen = path_of(&dir, "hook_seen.json");
    std::fs::write(
        app.gcx.config_dir.join("hooks.yaml"),
        format!("hooks:\n  PreToolUse:\n    - hooks:\n        - type: command\n          command: \"cat > {seen}\"\n"),
    )
    .unwrap();
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
    let path = path_of(&dir, "secret");
    let task = spawn(
        &app,
        &format!("await engine('testing.mark', path={path:?}, secret_token='sk-live-77', name='postgres://u:pw-55@db/x')"),
    );
    let request = next_request(&app, &task).await;
    let wire = serde_json::to_string(&request).unwrap();
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let m = finish(task).await;
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
    let call = &m.extra["python"]["calls"][0];
    let run_dir = std::path::PathBuf::from(m.extra["python"]["run_dir"].as_str().unwrap());
    let file = std::fs::read_to_string(run_dir.join(format!("calls/{}_engine.json", call["seq"])))
        .unwrap();
    let hook = std::fs::read_to_string(&seen).unwrap();
    for (place, text) in [
        ("record", call["args_preview"].to_string()),
        ("file", file),
        ("hook", hook),
        ("request", wire),
    ] {
        assert!(!text.contains("sk-live-77"), "{place}: {text}");
        assert!(!text.contains("pw-55"), "{place}: {text}");
    }
}

#[test]
fn the_scrubber_masks_new_field_names_and_url_credentials_of_any_scheme() {
    let mut value = json!({
        "auth": "a1", "pat": "p1", "dsn": "d1", "db_connection_string": "c1",
        "list": ["redis://:r-secret@cache:6379", "git+ssh://me:g-secret@host/repo", "https://h/x"],
        "port": 5432, "path": "/tmp/pattern"
    });
    super::super::mask::scrub(&mut value);
    for key in ["auth", "pat", "dsn", "db_connection_string"] {
        assert_eq!(value[key], "***", "{key}: {value}");
    }
    let text = value["list"].to_string();
    assert!(
        !text.contains("r-secret") && !text.contains("g-secret"),
        "{text}"
    );
    assert!(text.contains("https://h/x"));
    assert_eq!(value["port"], 5432);
    assert_eq!(value["path"], "/tmp/pattern");
}

#[tokio::test]
async fn a_restricted_child_has_no_engine_and_an_unrestricted_one_keeps_reads() {
    let (_dir, app, ccx) = fixture().await;
    ccx.lock().await.child_tool_ceiling =
        Some(crate::subchat::ToolsPolicy::Only(vec!["python".into()]));
    let m = run_ccx(
        ccx.clone(),
        "try:\n    engine\n    r = 'bound'\nexcept NameError:\n    r = 'unbound'\n[r, 'engine' in tools(), 'engine(' in help()]",
    )
    .await;
    assert_eq!(result(&m), json!(["unbound", false, false]));
    ccx.lock().await.child_tool_ceiling = Some(crate::subchat::ToolsPolicy::All);
    let m = run_ccx(ccx, "(await engine('session.limits'))['ok']").await;
    assert_eq!(result(&m), true);
    drop(app);
}

#[tokio::test]
async fn a_project_file_cannot_switch_engine_functions_back_on() {
    let global = refact_work_core::CoordinationSettings {
        python_tool_enabled: false,
        python_engine_read_enabled: false,
        python_engine_change_enabled: false,
        python_engine_risky_enabled: false,
        ..Default::default()
    };
    let overrides: crate::runtime_settings::CoordinationOverride = serde_yaml::from_str(
        "python_tool_enabled: true\npython_engine_read_enabled: true\npython_engine_change_enabled: true\npython_engine_risky_enabled: true\npython_engine_change_with_auto_commands: true\n",
    )
    .unwrap();
    let resolved = crate::runtime_settings::resolve_coordination(&global, &overrides).unwrap();
    assert!(!resolved.python_tool_enabled);
    assert!(!resolved.python_engine_read_enabled);
    assert!(!resolved.python_engine_change_enabled);
    assert!(!resolved.python_engine_risky_enabled);
    assert!(!resolved.python_engine_change_with_auto_commands);
    let narrowing: crate::runtime_settings::CoordinationOverride =
        serde_yaml::from_str("python_engine_change_enabled: false\n").unwrap();
    let resolved = crate::runtime_settings::resolve_coordination(
        &refact_work_core::CoordinationSettings::default(),
        &narrowing,
    )
    .unwrap();
    assert!(!resolved.python_engine_change_enabled);
    assert!(resolved.python_engine_risky_enabled);
}

#[test]
fn every_settings_key_is_classified_and_unlisted_keys_are_guarded() {
    use super::super::settings::{guarded_group, GUARDED, LOCKED, ORDINARY, OTHER_GUARDED};
    let fields = crate::http::routers::v1::settings_config::trajectory_settings_fields();
    let prefix = format!("{}.", crate::runtime_settings::COORDINATION_SETTINGS_KEY);
    let names: Vec<String> = fields
        .iter()
        .map(|f| f.name.strip_prefix(&prefix).unwrap_or(&f.name).to_string())
        .collect();
    let mut unlisted = Vec::new();
    for name in &names {
        let locked = LOCKED.contains(&name.as_str());
        let ordinary = ORDINARY.iter().any(|(key, _)| key == name);
        let grouped = GUARDED
            .iter()
            .any(|(_, keys, _)| keys.contains(&name.as_str()));
        assert!(
            [locked, ordinary, grouped].iter().filter(|x| **x).count() <= 1,
            "{name} is in more than one list"
        );
        if !(locked || ordinary || grouped) {
            unlisted.push(name.clone());
        }
    }
    for name in &unlisted {
        assert_eq!(guarded_group(name), Some(OTHER_GUARDED), "{name}");
    }
    for (key, reason) in ORDINARY {
        assert!(
            names.iter().any(|n| n == key),
            "ordinary key {key} does not exist"
        );
        assert!(!reason.trim().is_empty(), "{key}");
    }
    for (group, keys, warning) in GUARDED {
        assert!(!warning.trim().is_empty(), "{group}");
        for key in *keys {
            assert!(
                names.iter().any(|n| n == key),
                "guarded key {key} does not exist"
            );
        }
    }
    assert_eq!(
        guarded_group("a_key_nobody_classified"),
        Some(OTHER_GUARDED)
    );
    for key in [
        "memory_supervisor_override",
        "deadlock_detection_enabled",
        "enabled",
        "subagent_max_steps",
        "provider_pressure_enabled",
        "provider_breaker_failure_threshold",
        "no_progress_repeat_threshold",
        "deadlock_scan_interval_ms",
        "metrics_enabled",
        "disagreement_max_open",
        "board_max_reviewers",
        "stream_idle_timeout_secs",
        "lease_ttl_ms",
    ] {
        assert!(guarded_group(key).is_some(), "{key}");
    }
    assert!(
        unlisted.is_empty(),
        "keys in no list (guarded by default, but list them): {unlisted:?}"
    );
}

#[tokio::test]
async fn the_context_carries_the_run_facts_and_an_egress_check() {
    let (dir, app, _) = fixture().await;
    coordination(
        &dir,
        "python_max_nested_calls: 40\npython_max_parallel_calls: 3\npython_max_llm_calls: 5\n",
    );
    let m = run(
        &app,
        "await engine('session.limits')\n(await engine('testing.ctx'))['data']",
    )
    .await;
    let facts = result(&m);
    assert!(!facts["outer"].as_str().unwrap().is_empty(), "{facts}");
    assert_eq!(facts["used"], 2);
    assert_eq!(facts["limit"], 40);
    assert_eq!(facts["parallel"], 3);
    assert_eq!(facts["llm_calls"], 5);
    assert_eq!(facts["egress"], Value::Null, "{facts}");
}

#[tokio::test]
async fn a_change_in_a_turn_started_by_a_delivery_is_refused() {
    let (_dir, app, ccx) = fixture().await;
    let delivery = refact_chat_api::PendingDelivery::new(
        vec![crate::call_validation::ChatMessage::new(
            "user".into(),
            "scheduled prompt".into(),
        )],
        refact_chat_api::PushMode::Append,
        "scheduler.cron",
        true,
    );
    ccx.lock().await.messages = delivery.stamp_messages();
    let m = run_ccx(
        ccx.clone(),
        "try:\n    await engine('testing.touch', n=1)\n    r = 'applied'\nexcept PermissionError as e:\n    r = str(e)\n[r, (await engine('session.limits'))['ok']]",
    )
    .await;
    let out = result(&m);
    assert!(out[0].as_str().unwrap().contains("scheduler.cron"), "{out}");
    assert_eq!(out[1], true);
    for source in [
        "daemon.hooks",
        "agents.completion",
        "chat.goal_monitor",
        "tool_job.completed",
    ] {
        let wake = refact_chat_api::PendingDelivery::new(
            vec![crate::chat::internal_roles::event(
                crate::chat::internal_roles::EventSubkind::SystemNotice,
                source,
                json!({}),
                "woken",
            )],
            refact_chat_api::PushMode::Append,
            source,
            true,
        );
        let mut messages = vec![crate::call_validation::ChatMessage::new(
            "user".into(),
            "typed earlier".into(),
        )];
        messages.extend(wake.stamp_messages());
        ccx.lock().await.messages = messages;
        let m = run_ccx(
            ccx.clone(),
            "try:\n    await engine('testing.touch', n=1)\n    r = 'applied'\nexcept PermissionError as e:\n    r = str(e)\nr",
        )
        .await;
        assert!(
            result(&m).as_str().unwrap().contains(source),
            "{source}: {}",
            result(&m)
        );
    }
    ccx.lock().await.messages = vec![crate::call_validation::ChatMessage::new(
        "user".into(),
        "typed".into(),
    )];
    let task = {
        let ccx = ccx.clone();
        tokio::spawn(
            async move { run_ccx(ccx, "(await engine('testing.touch', n=1))['ok']").await },
        )
    };
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    assert_eq!(result(&finish(task).await), true);
}

#[test]
fn a_replaced_run_guard_still_drops_its_permit() {
    let runtime = crate::python_tool::PythonToolRuntime::default();
    let first = runtime.register_run("chat", "call", "run-1");
    runtime
        .approvals
        .run_permits
        .lock()
        .unwrap()
        .insert(("chat".into(), "run-1".into()));
    let second = runtime.register_run("chat", "call", "run-2");
    drop(first);
    assert!(!runtime.approvals.run_permitted("chat", "run-1"));
    assert_eq!(runtime.active_run_ids("chat"), ["run-2"]);
    drop(second);
}

#[tokio::test]
async fn a_restricted_chat_is_not_told_about_engine_outside_the_run() {
    let (_dir, app, _) = fixture().await;
    let catalog = app
        .tool_registry
        .acquire_tool_catalog("agent", Some("model"), None)
        .await;
    let described = |catalog: &refact_runtime_api::ToolCatalogSnapshot| {
        catalog
            .index
            .tools
            .iter()
            .find(|tool| tool.name == "python")
            .map(|tool| tool.description.contains("async def engine("))
    };
    let open = crate::tools::tools_list::filter_tool_catalog_for_chat(
        &app,
        "python-test",
        catalog.clone(),
    )
    .await;
    assert_eq!(described(&open), Some(true));
    let prompt = |app: AppState| async move {
        crate::chat::prompts::system_prompt_add_extra_instructions(
            app,
            "%CODE_MODE%".into(),
            std::collections::HashSet::from(["python".to_string()]),
            &crate::call_validation::ChatMeta {
                chat_id: "python-test".into(),
                ..Default::default()
            },
            &None,
            "agent",
        )
        .await
    };
    assert!(prompt(app.clone()).await.contains("await engine("));
    session(&app)
        .await
        .lock()
        .await
        .active_command
        .allowed_tools = vec!["python".into()];
    let restricted =
        crate::tools::tools_list::filter_tool_catalog_for_chat(&app, "python-test", catalog).await;
    assert_eq!(described(&restricted), Some(false));
    assert!(!prompt(app.clone()).await.contains("engine("));
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(permission_decision_hooks)]
async fn a_hook_rewrite_that_echoes_a_mask_keeps_the_real_secret() {
    let (dir, app, _) = fixture().await;
    let path = path_of(&dir, "rewritten");
    let rewrite = json!({"decision":"rewrite","tool_input":{"path":path,"secret_token":"***","name":"changed"}});
    std::fs::write(
        app.gcx.config_dir.join("hooks.yaml"),
        format!(
            "hooks:\n  PreToolUse:\n    - hooks:\n        - type: command\n          command: {:?}\n",
            format!("echo '{rewrite}'")
        ),
    )
    .unwrap();
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
    let task = spawn(
        &app,
        &format!("(await engine('testing.mark', path={path:?}, secret_token='sk-live-77', name='first'))['data']"),
    );
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let m = finish(task).await;
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
    assert_eq!(result(&m), json!({"name":"changed"}));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "sk-live-77");
}

#[test]
fn unmasking_restores_only_fields_the_hook_left_masked() {
    let original =
        json!({"token":"real","nested":{"password":"p","keep":1},"list":["--api-key","k"]});
    let sent = super::super::dispatch::masked(&original);
    let mut rewrite = json!({"token":"***","nested":{"password":"new","keep":2},"list":["--api-key","***"],"extra":"***"});
    super::super::dispatch::unmask_rewrite(&mut rewrite, &sent, &original);
    assert_eq!(rewrite["token"], "real");
    assert_eq!(rewrite["nested"]["password"], "new");
    assert_eq!(rewrite["nested"]["keep"], 2);
    assert_eq!(rewrite["list"], json!(["--api-key", "k"]));
    assert_eq!(rewrite["extra"], "***");
}

#[tokio::test]
async fn a_handler_that_ignores_the_approved_plan_is_an_internal_error() {
    let (_dir, app, _) = fixture().await;
    let task = spawn(
        &app,
        "r = await engine('testing.unplanned', n=1)\n[r['ok'], r['error']]",
    );
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let m = finish(task).await;
    let out = result(&m);
    assert_eq!(out[0], false);
    assert!(
        out[1]
            .as_str()
            .unwrap()
            .contains("did not apply the approved plan"),
        "{out}"
    );
    assert_eq!(m.extra["python"]["engine_changes"][0]["outcome"], "unknown");
}

#[tokio::test]
async fn a_change_does_not_wait_forever_for_other_calls() {
    let (_dir, app, _) = fixture().await;
    let app = crate::tools::tool_python::tests::with_tools(app);
    let m = run(
        &app,
        "import asyncio\nasync def change():\n    try:\n        await engine('testing.touch', n=1)\n        return 'ran'\n    except RuntimeError as e:\n        return str(e)\nr = await asyncio.gather(fixture(ms=6000), change())\nr[1]",
    )
    .await;
    let text = result(&m).as_str().unwrap().to_string();
    assert!(text.contains("await the other calls first"), "{text}");
    assert!(session(&app)
        .await
        .lock()
        .await
        .runtime
        .python_approvals
        .is_empty());
}

#[tokio::test]
async fn a_change_stopped_before_its_card_is_recorded_cancelled_not_as_a_read() {
    let (_dir, app, _) = fixture().await;
    let app = crate::tools::tool_python::tests::with_tools(app);
    let task = spawn(
        &app,
        "import asyncio\nr = await asyncio.gather(fixture(ms=20000), engine('testing.touch', n=1))\nr",
    );
    let tool_call_id = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let found = session(&app)
                .await
                .lock()
                .await
                .runtime
                .python_runs
                .iter()
                .find(|r| r.calls.iter().any(|c| c.tool == "engine.testing.touch"))
                .map(|r| r.tool_call_id.clone());
            if let Some(id) = found {
                break id;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the change call did not start");
    app.python_tool
        .stop_run("python-test", &tool_call_id)
        .unwrap();
    let m = finish(task).await;
    let call = m.extra["python"]["calls"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["tool"] == "engine.testing.touch")
        .cloned()
        .unwrap();
    assert_eq!(call["approval"], "cancelled", "{call}");
}

#[tokio::test]
async fn error_texts_are_masked_before_the_record_and_the_script() {
    let (_dir, app, _) = fixture().await;
    let m = run(
        &app,
        "try:\n    await engine('settings.set', values={'max_queue_size': 'postgres://u:pw-77@db/x'}, scope='global')\n    r = 'ran'\nexcept TypeError as e:\n    r = str(e)\nr",
    )
    .await;
    let text = result(&m).as_str().unwrap().to_string();
    assert!(!text.contains("pw-77"), "{text}");
    let summary = m.extra["python"]["calls"][0]["summary"].as_str().unwrap();
    assert!(!summary.contains("pw-77"), "{summary}");
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn a_settings_route_save_waits_for_an_engine_change_in_progress() {
    crate::runtime_settings::reset_for_test();
    let (_dir, app, _) = fixture().await;
    let held = crate::python_tool::engine_fns::ENGINE_CHANGE_LOCK
        .lock()
        .await;
    let body =
        serde_json::to_vec(&crate::runtime_settings::TrajectoryRuntimeSettings::default()).unwrap();
    let route = tokio::spawn({
        let app = app.clone();
        async move {
            crate::http::routers::v1::settings_config::handle_v1_trajectory_settings_post(
                axum::extract::State(app),
                hyper::body::Bytes::from(body),
            )
            .await
            .is_ok()
        }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !route.is_finished(),
        "the route wrote while an engine change held the lock"
    );
    drop(held);
    assert!(tokio::time::timeout(Duration::from_secs(15), route)
        .await
        .unwrap()
        .unwrap());
    crate::runtime_settings::reset_for_test();
}

#[tokio::test]
async fn a_card_never_carries_a_masked_or_cut_new_value() {
    let (_dir, app, _) = fixture().await;
    let m = run(
        &app,
        "out = []\nfor label, value in [('api_token', 'zone-tok-1'), ('note', 'Authorization: Bearer zone-bearer-2'), ('big', 'x' * 20000)]:\n    try:\n        await engine('testing.set', label=label, value=value)\n        out.append('asked')\n    except PermissionError as e:\n        out.append(str(e))\nout",
    )
    .await;
    let out = result(&m);
    let texts: Vec<&str> = out
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        texts[0].contains("api_token") && texts[0].contains("looks like a secret"),
        "{out}"
    );
    assert!(
        texts[1].contains("note") && texts[1].contains("looks like a secret"),
        "{out}"
    );
    assert!(
        texts[2].contains("big") && texts[2].contains("20002 bytes") && texts[2].contains("16384"),
        "{out}"
    );
    assert!(!out.to_string().contains("zone-tok-1"), "{out}");
    let calls = m.extra["python"]["calls"].as_array().unwrap();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|c| c["approval"] == "denied"), "{calls:?}");
    assert!(m.extra["python"].get("engine_changes").is_none());
    assert!(session(&app)
        .await
        .lock()
        .await
        .runtime
        .python_approvals
        .is_empty());

    let task = spawn(
        &app,
        "(await engine('testing.set', label='note', value='plain words'))['ok']",
    );
    let request = next_request(&app, &task).await;
    assert_eq!(
        request.engine.clone().unwrap().items[0].new,
        Some(json!("plain words"))
    );
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    assert_eq!(result(&finish(task).await), true);
}

#[tokio::test]
async fn a_handler_listed_to_run_unlocked_does_not_block_other_changes() {
    let (dir, app, _) = fixture().await;
    let path = path_of(&dir, "unlocked");
    let task = spawn(
        &app,
        &format!("(await engine('testing.unlocked', path={path:?}))['ok']"),
    );
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        while !std::path::Path::new(&format!("{path}.started")).exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("handler did not start");
    let other_change = tokio::time::timeout(
        Duration::from_secs(5),
        crate::python_tool::engine_fns::ENGINE_CHANGE_LOCK.lock(),
    )
    .await;
    let blocked = other_change.is_err();
    drop(other_change);
    std::fs::write(format!("{path}.release"), "").unwrap();
    assert!(
        !blocked,
        "the engine-wide lock was held while the unlocked handler ran"
    );
    assert_eq!(result(&finish(task).await), true);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "applied");
}
