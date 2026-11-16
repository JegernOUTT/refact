use super::*;

fn row<'a>(rows: &'a Value, key: &str) -> &'a Value {
    rows.get(key)
        .unwrap_or_else(|| panic!("{key} missing in {rows}"))
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn get_schema_diff_and_limits_read_the_fixture_project() {
    crate::runtime_settings::reset_for_test();
    let (dir, app, _) = fixture().await;
    coordination(&dir, "python_max_llm_calls: 7\n");
    let m = run(
        &app,
        "full = await engine('settings.get')\nassert full['ok'] and len(full['data']['settings']) > 200, full\ng = await engine('settings.get', keys=['python_max_llm_calls', 'max_queue_size', 'event_channel_capacity', 'python_tool_enabled'])\ns = await engine('settings.schema')\nd = await engine('settings.diff')\nl = await engine('session.limits')\ni = await engine('system.info')\n[g['data'], s['data'], d['data'], l['data'], i['data']]",
    )
    .await;
    let out = result(&m);
    assert!(out.is_array(), "{}", m.content.content_text_only());
    let get = &out[0]["settings"];
    let llm = row(get, "python_max_llm_calls");
    assert_eq!(
        (llm["value"].clone(), llm["default"].clone()),
        (json!(7), json!(200))
    );
    assert_eq!(llm["source"], "project");
    assert_eq!(llm["range"], "0..100000");
    assert!(llm.get("restart_only").is_none());
    assert_eq!(row(get, "max_queue_size")["source"], "default");
    assert_eq!(row(get, "event_channel_capacity")["restart_only"], true);
    assert_eq!(row(get, "python_tool_enabled")["value"], true);
    let schema = &out[1]["keys"];
    assert!(schema.as_object().unwrap().len() > 200);
    for key in [
        "python_engine_read_enabled",
        "python_engine_change_enabled",
        "python_engine_risky_enabled",
    ] {
        assert_eq!(row(schema, key)["treatment"], "locked");
        assert_eq!(row(schema, key)["default"], true);
    }
    assert_eq!(
        row(schema, "approval_timeout_secs")["group"],
        "approval timeout"
    );
    assert_eq!(row(schema, "python_max_llm_calls")["treatment"], "guarded");
    assert_eq!(row(schema, "max_subchat_depth")["treatment"], "guarded");
    assert_eq!(row(schema, "max_subchat_depth")["group"], "agent limits");
    assert_eq!(row(schema, "max_queue_size")["treatment"], "ordinary");
    assert_eq!(row(schema, "max_queue_size")["global_only"], true);
    assert_eq!(row(schema, "event_channel_capacity")["restart_only"], true);
    let diff = &out[2]["changed"];
    let changed = row(diff, "python_max_llm_calls");
    assert_eq!(
        (changed["project"].clone(), changed["global"].clone()),
        (json!(7), json!(200))
    );
    assert_eq!(diff.as_object().unwrap().len(), 1, "{diff}");
    assert_eq!(out[3]["limits"]["python_max_llm_calls"], 7);
    assert!(out[3]["limits"]
        .as_object()
        .unwrap()
        .keys()
        .all(|k| k.starts_with("python_")));
    assert_eq!(out[4]["pid"], std::process::id());
    assert!(out[4]["version"].is_string() && out[4]["args"].is_array());
}

#[test]
fn system_info_masks_secret_flags() {
    let masked = super::super::system::mask_args(
        [
            "refact-lsp",
            "--api-key",
            "sk-1",
            "--http-port",
            "8001",
            "--auth-token=t2",
            "--verbose",
        ]
        .map(String::from)
        .to_vec(),
    );
    assert_eq!(
        masked,
        [
            "refact-lsp",
            "--api-key",
            "***",
            "--http-port",
            "8001",
            "--auth-token=***",
            "--verbose"
        ]
    );
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn set_applies_live_and_is_visible_to_the_code_that_reads_it() {
    crate::runtime_settings::reset_for_test();
    let (dir, app, _) = fixture().await;
    let task = spawn(&app, "r = await engine('settings.set', values={'max_queue_size': 42}, scope='global')\nr['data']");
    let request = next_request(&app, &task).await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Change);
    assert_eq!(engine.title, "change engine settings");
    assert_eq!(engine.scope.as_deref(), Some("global"));
    assert_eq!(engine.items[0].label, "max_queue_size");
    let default = 100;
    assert_eq!(
        engine.items[0].old,
        Some(json!(format!("{default} (default)")))
    );
    assert_eq!(engine.items[0].new, Some(json!(42)));
    assert!(engine.warning.is_none());
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let m = finish(task).await;
    assert_eq!(result(&m)["applied"], json!(["max_queue_size"]));
    assert_eq!(crate::runtime_settings::current().max_queue_size, 42);
    assert_eq!(
        m.extra["python"]["calls"][0]["summary"],
        format!("max_queue_size: {default} (default) → 42")
    );
    assert_eq!(
        m.extra["python"]["engine_changes"][0]["items"][0]["old"],
        json!(format!("{default} (default)"))
    );
    let persisted =
        std::fs::read_to_string(dir.path().join("config/trajectory-settings.yaml")).unwrap();
    assert!(persisted.contains("max_queue_size: 42"), "{persisted}");

    let task = spawn(
        &app,
        "await engine('settings.set', values={'max_subchat_depth': 9})",
    );
    let request = next_request(&app, &task).await;
    assert_eq!(
        request.engine.as_ref().unwrap().scope.as_deref(),
        Some("project")
    );
    assert!(request.engine.as_ref().unwrap().items[0]
        .old
        .as_ref()
        .unwrap()
        .as_str()
        .unwrap()
        .ends_with("(global)"));
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    finish(task).await;
    let effective = crate::runtime_settings::coordination_for_project(&dir.path().join("project"))
        .await
        .unwrap();
    assert_eq!(effective.max_subchat_depth, 9);
    crate::runtime_settings::reset_for_test();
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn out_of_bounds_locked_and_restart_only_keys_are_refused_before_asking() {
    crate::runtime_settings::reset_for_test();
    let (_dir, app, _) = fixture().await;
    let m = run(
        &app,
        "out = []\nfor values, scope in [({'python_max_parallel_calls': 1000}, 'project'), ({'max_queue_size': 0}, 'global'), ({'no_such_key': 1}, 'project'), ({'max_queue_size': 5}, 'project')]:\n    try:\n        await engine('settings.set', values=values, scope=scope)\n        out.append('applied')\n    except TypeError as e:\n        out.append('TypeError: ' + str(e))\nfor values, scope in [({'python_tool_enabled': False}, 'project'), ({'python_engine_change_enabled': False}, 'project'), ({'python_engine_read_enabled': False}, 'global'), ({'event_channel_capacity': 64}, 'global')]:\n    try:\n        await engine('settings.set', values=values, scope=scope)\n        out.append('applied')\n    except PermissionError as e:\n        out.append('PermissionError: ' + str(e))\nout",
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
        texts[0].starts_with("TypeError") && texts[0].contains("python_max_parallel_calls"),
        "{texts:?}"
    );
    assert!(
        texts[1].starts_with("TypeError") && texts[1].contains("max_queue_size"),
        "{texts:?}"
    );
    assert!(
        texts[2].starts_with("TypeError") && texts[2].contains("unknown setting"),
        "{texts:?}"
    );
    assert!(
        texts[3].starts_with("TypeError") && texts[3].contains("scope=\"global\""),
        "{texts:?}"
    );
    for text in &texts[4..7] {
        assert!(
            text.starts_with("PermissionError") && text.contains("locked"),
            "{texts:?}"
        );
    }
    assert!(
        texts[7].starts_with("PermissionError") && texts[7].contains("restart"),
        "{texts:?}"
    );
    assert!(session(&app)
        .await
        .lock()
        .await
        .runtime
        .python_approvals
        .is_empty());
    assert_eq!(crate::runtime_settings::current().max_queue_size, 100);
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn guarded_key_makes_a_risky_request_with_a_warning() {
    crate::runtime_settings::reset_for_test();
    let (_dir, app, _) = fixture().await;
    let task = spawn(
        &app,
        "await engine('settings.set', values={'python_max_nested_calls': 5000, 'board_allow_self_review': True, 'max_subchat_depth': 6})",
    );
    let request = next_request(&app, &task).await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Risky);
    let warning = engine.warning.clone().unwrap();
    assert!(warning.contains("more tool and model calls"), "{warning}");
    assert!(warning.contains("review their own work"), "{warning}");
    assert!(warning.ends_with('.') && !warning.contains('\n'));
    assert_eq!(engine.items.len(), 3);
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    let m = finish(task).await;
    assert_eq!(m.extra["python"]["calls"][0]["approval"], "denied");
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn a_python_limit_change_says_it_applies_from_the_next_run() {
    crate::runtime_settings::reset_for_test();
    let (_dir, app, _) = fixture().await;
    let task = spawn(
        &app,
        "r = await engine('settings.set', values={'python_max_nested_calls': 7, 'max_resume_burst': 3})\nr['text']",
    );
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let text = result(&finish(task).await).as_str().unwrap().to_string();
    assert!(
        text.contains("python_max_nested_calls applies from the next script run"),
        "{text}"
    );
    assert!(!text.contains("max_resume_burst applies"), "{text}");
    let task = spawn(
        &app,
        "r = await engine('settings.set', values={'max_resume_burst': 4})\nr['text']",
    );
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let text = result(&finish(task).await).as_str().unwrap().to_string();
    assert!(!text.contains("next script run"), "{text}");
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn reset_and_clear_project_override() {
    crate::runtime_settings::reset_for_test();
    let (dir, app, _) = fixture().await;
    coordination(
        &dir,
        "max_subchat_depth: 9\nmax_resume_burst: 8\npython_tool_enabled: true\n",
    );
    let task = spawn(
        &app,
        "(await engine('settings.reset', keys=['max_subchat_depth']))['data']",
    );
    let request = next_request(&app, &task).await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(engine.title, "reset engine settings to their defaults");
    assert_eq!(engine.items[0].old, Some(json!(9)));
    assert_eq!(engine.items[0].new, Some(json!(5)));
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    finish(task).await;
    let path = dir.path().join("project/.refact/coordination.yaml");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("max_subchat_depth: 5"), "{text}");

    let task = spawn(
        &app,
        "(await engine('settings.clear_project_override'))['data']",
    );
    let request = next_request(&app, &task).await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(engine.title, "remove project setting overrides");
    let labels: Vec<_> = engine.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["max_resume_burst", "max_subchat_depth"]);
    assert!(engine.items.iter().all(|i| i.new.is_none()));
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let m = finish(task).await;
    assert_eq!(
        result(&m)["skipped"],
        json!([{"key":"python_tool_enabled","reason":"locked"}])
    );
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        !text.contains("max_subchat_depth") && text.contains("python_tool_enabled"),
        "{text}"
    );
}
