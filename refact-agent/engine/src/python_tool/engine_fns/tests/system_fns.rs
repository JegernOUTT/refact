use super::*;

async fn block(app: &AppState, path: &std::path::Path) {
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.blocked = vec![path.to_string_lossy().into_owned()];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
}

fn engine_log(dir: &tempfile::TempDir, text: &str) {
    let logs = dir.path().join("cache/logs");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(logs.join("refact.log"), text).unwrap();
}

#[tokio::test]
async fn reads_report_resources_perf_setup_and_browser_settings() {
    let (dir, app, _) = fixture().await;
    std::fs::write(
        dir.path().join("config/browser.yaml"),
        "launch:\n  proxy_server: http://user:pw-zone@proxy.local:3128\n",
    )
    .unwrap();
    let m = run(
        &app,
        "r = await engine('system.resources')\np = await engine('system.perf', components=['none.such'])\ns = await engine('system.setup')\nb = await engine('system.browser_settings')\n[r['data'], p['data'], s['data'], b['data'], b['text']]",
    )
    .await;
    let out = result(&m);
    assert!(out.is_array(), "{}", m.content.content_text_only());
    assert_eq!(out[0]["pid"], std::process::id());
    assert!(out[0]["rss_bytes"].as_u64().unwrap() > 0);
    assert_eq!(out[1]["components"], json!([]));
    assert!(out[1]["rollout_switches"]["trajectory_writer_enabled"].is_boolean());
    assert!(out[1]["queue_diagnostics"]["coverage"].is_object());
    assert_eq!(out[2]["configured"], false);
    let reasons = out[2]["reasons"].as_array().unwrap();
    assert!(
        reasons.contains(&json!("missing_agents_md")) && reasons.contains(&json!("no_knowledge")),
        "{out}"
    );
    assert_eq!(out[3]["source"], "file");
    assert_eq!(
        out[3]["settings"]["launch"]["proxy_server"],
        "http://***@proxy.local:3128"
    );
    assert!(!out.to_string().contains("pw-zone"));
}

#[test]
fn proxy_userinfo_is_masked() {
    let mask = super::super::system::mask_proxy_for_test;
    assert_eq!(mask("http://a:b@h:1/p@x"), "http://***@h:1/p@x");
    assert_eq!(mask("socks5://h:1080"), "socks5://h:1080");
    assert_eq!(mask("u:p@h:1"), "***@h:1");
}

#[tokio::test]
async fn logs_and_errors_are_redacted_and_drop_privacy_blocked_paths() {
    let (dir, app, _) = fixture().await;
    let blocked = dir.path().join("project/zone_a.txt");
    block(&app, &blocked).await;
    engine_log(
        &dir,
        &format!(
            "000001.000 INFO src/a.rs:1: started api_key=sk-abcdefghijklmnopqrstuvwxyz0123\n000002.000 ERROR src/b.rs:2: cannot read {}\n000003.000 ERROR src/c.rs:3: failed 12 times\n000004.000 ERROR src/c.rs:3: failed 13 times\n",
            blocked.display()
        ),
    );
    let m = run(
        &app,
        "l = await engine('system.logs', tail=50)\ne = await engine('system.errors')\nbad = []\ntry:\n    await engine('system.logs', source='other')\nexcept TypeError as x:\n    bad.append(str(x))\n[l['data'], e['data'], bad]",
    )
    .await;
    let out = result(&m);
    assert!(out.is_array(), "{}", m.content.content_text_only());
    let lines = out[0]["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 3, "{out}");
    assert_eq!(out[0]["dropped_lines"], 1);
    assert!(!out
        .to_string()
        .contains("sk-abcdefghijklmnopqrstuvwxyz0123"));
    assert!(!out.to_string().contains("zone_a.txt"));
    let errors: Vec<_> = out[1]["errors"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["source"] == "engine")
        .collect();
    assert_eq!(errors.len(), 1, "{out}");
    assert_eq!(errors[0]["count"], 2);
    assert!(out[1]["dropped"].as_u64().unwrap() >= 1);
    assert!(out[2][0].as_str().unwrap().contains("source"), "{out}");
}

#[tokio::test]
async fn logs_refuse_when_the_privacy_policy_cannot_be_loaded() {
    let (dir, app, ccx) = fixture().await;
    engine_log(&dir, "000001.000 INFO src/a.rs:1: hello\n");
    std::fs::write(dir.path().join("config/privacy.yaml"), "zones: [unclosed\n").unwrap();
    app.gcx.privacy_policy_load.write().unwrap().error = Some("broken".into());
    let ctx = super::super::EngineCtx {
        app: app.clone(),
        ccx,
        chat_id: "python-test".into(),
        run_id: "run".into(),
        outer_tool_call_id: "outer".into(),
        project_root: Some(dir.path().join("project")),
        settings: crate::runtime_settings::current().coordination,
        llm: None,
        run_privacy: Default::default(),
        nested: super::super::NestedCalls {
            used: 0,
            limit: 1,
            parallel_limit: 1,
        },
        plan: Default::default(),
        run: None,
    };
    for name in ["system.logs", "system.errors"] {
        let function = super::super::lookup(name).unwrap();
        let error = (function.handler)(&ctx, &json!({})).await.unwrap_err();
        assert!(
            matches!(&error, super::super::EngineError::Failed(m) if m.contains("privacy policy")),
            "{error:?}"
        );
    }
}

#[tokio::test]
#[serial_test::serial(perf_telemetry)]
async fn set_perf_previews_applies_and_refuses_a_moved_state() {
    let (_dir, app, _) = fixture().await;
    let telemetry = app.gcx.performance_telemetry.clone();
    telemetry.set_enabled(false);
    let task = spawn(
        &app,
        "r = await engine('system.set_perf', enabled=True, reset=True)\nr['data']",
    );
    let request = next_request(&app, &task).await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(engine.title, "change performance telemetry");
    assert_eq!(engine.scope.as_deref(), Some("engine"));
    assert_eq!(engine.items[0].label, "telemetry");
    assert_eq!(engine.items[0].old, Some(json!("off")));
    assert_eq!(engine.items[0].new, Some(json!("on")));
    assert_eq!(engine.items[1].label, "counters");
    assert_eq!(engine.items[1].new, Some(json!("reset")));
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let m = finish(task).await;
    assert_eq!(result(&m), json!({"enabled":true,"reset":true}));
    assert!(telemetry.enabled());

    let task = spawn(&app, "await engine('system.set_perf', enabled=False)");
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    let m = finish(task).await;
    assert_eq!(m.extra["python"]["calls"][0]["approval"], "denied");
    assert!(telemetry.enabled());

    let task = spawn(
        &app,
        "r = await engine('system.set_perf', enabled=False)\n[r['ok'], r['error']]",
    );
    let request = next_request(&app, &task).await;
    telemetry.set_enabled(false);
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let out = result(&finish(task).await);
    assert_eq!(out[0], false, "{out}");
    assert!(
        out[1]
            .as_str()
            .unwrap()
            .contains("changed since the preview"),
        "{out}"
    );
    telemetry.set_enabled(false);
    let m = run(
        &app,
        "try:\n    await engine('system.set_perf')\n    r = 'ran'\nexcept TypeError as e:\n    r = str(e)\nr",
    )
    .await;
    assert!(
        result(&m).as_str().unwrap().contains("enabled or reset"),
        "{}",
        result(&m)
    );
    assert!(session(&app)
        .await
        .lock()
        .await
        .runtime
        .python_approvals
        .is_empty());
}

#[tokio::test]
async fn reload_config_counts_modes_and_invalidates_caps() {
    let (dir, app, _) = fixture().await;
    let modes = dir.path().join("project/.refact/modes");
    std::fs::create_dir_all(&modes).unwrap();
    std::fs::write(modes.join("zone_mode.yaml"), "id: [broken\n").unwrap();
    let generations = |app: &AppState| {
        let g = &app.gcx.tool_catalog_generations;
        (
            g.customization.load(std::sync::atomic::Ordering::SeqCst),
            g.capabilities.load(std::sync::atomic::Ordering::SeqCst),
        )
    };
    let before = generations(&app);
    let task = spawn(
        &app,
        "r = await engine('system.reload_config')\n[r['data'], r['text']]",
    );
    let request = next_request(&app, &task).await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(engine.title, "reload engine configuration");
    assert_eq!(engine.scope.as_deref(), Some("engine (all chats)"));
    let labels: Vec<_> = engine.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["customization", "capabilities"]);
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let out = result(&finish(task).await);
    assert!(
        out[0]["customization"]["modes_loaded"].as_u64().unwrap() > 0,
        "{out}"
    );
    let errors = out[0]["customization"]["errors"].as_array().unwrap();
    assert!(
        errors
            .iter()
            .any(|e| e["file_path"].as_str().unwrap().contains("zone_mode")),
        "{out}"
    );
    assert_eq!(out[0]["caps_invalidated"], true);
    assert!(out[1].as_str().unwrap().contains("not reloaded"));
    let after = generations(&app);
    assert!(after.0 > before.0 && after.1 > before.1);
    assert!(app.gcx.caps_state.read().await.caps.is_none());

    let task = spawn(&app, "await engine('system.reload_config', parts=['caps'])");
    let request = next_request(&app, &task).await;
    assert_eq!(request.engine.as_ref().unwrap().items.len(), 1);
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    assert_eq!(
        finish(task).await.extra["python"]["calls"][0]["approval"],
        "denied"
    );
}

#[tokio::test]
async fn bug_bundle_writes_a_redacted_zip_under_the_cache_dir() {
    let (dir, app, _) = fixture().await;
    engine_log(
        &dir,
        "000001.000 INFO src/a.rs:1: api_key=sk-abcdefghijklmnopqrstuvwxyz0123\n",
    );
    let task = spawn(&app, "r = await engine('system.bug_bundle')\nr['data']");
    let request = next_request(&app, &task).await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(engine.title, "write a diagnostics bundle");
    assert_eq!(engine.scope.as_deref(), Some("local disk"));
    assert!(engine.items[0]
        .new
        .as_ref()
        .unwrap()
        .as_str()
        .unwrap()
        .contains("bug-reports"));
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let out = result(&finish(task).await);
    let path = std::path::PathBuf::from(out["path"].as_str().unwrap());
    assert!(path.starts_with(dunce::canonicalize(dir.path().join("cache/bug-reports")).unwrap()));
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let mut log = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("engine.log").unwrap(), &mut log).unwrap();
    assert!(!log.contains("sk-abcdefghijklmnopqrstuvwxyz0123"), "{log}");

    let task = spawn(&app, "await engine('system.bug_bundle')");
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    finish(task).await;
    assert_eq!(
        std::fs::read_dir(dir.path().join("cache/bug-reports"))
            .unwrap()
            .count(),
        1
    );
}

#[tokio::test]
async fn session_status_and_runs_describe_this_chat() {
    let (_dir, app, _) = fixture().await;
    run(&app, "token = 'sk-abcdefghijklmnopqrstuvwxyz0123'\nn = 1").await;
    run(&app, "n = 2").await;
    let m = run(
        &app,
        "s = await engine('session.status')\nq = await engine('session.status', variables=False)\nr = await engine('session.runs', limit=1)\n[s['data'], q['data'], r['data']]",
    )
    .await;
    let out = result(&m);
    assert!(out.is_array(), "{}", m.content.content_text_only());
    assert_eq!(out[0]["status"], "running");
    assert!(out[0]["active_run_ids"].as_array().unwrap().len() == 1);
    let names: Vec<_> = out[0]["variables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"token"), "{out}");
    assert!(
        !out.to_string()
            .contains("sk-abcdefghijklmnopqrstuvwxyz0123"),
        "{out}"
    );
    assert!(out[1].get("variables").is_none());
    assert_eq!(out[2]["runs"].as_array().unwrap().len(), 1);
    assert!(out[2]["total"].as_u64().unwrap() >= 2, "{out}");
    assert_eq!(out[2]["truncated"], true);
}
