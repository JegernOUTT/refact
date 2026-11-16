use super::*;

fn project(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("project")
}

async fn install(app: &AppState) {
    let mut caps = crate::caps::CodeAssistantCaps::default();
    for id in ["model", "zone/model-a", "zone/model-b", "other/model-c"] {
        caps.chat_models.insert(
            id.into(),
            Arc::new(crate::caps::ChatModelRecord {
                base: crate::caps::BaseModelRecord {
                    id: id.into(),
                    n_ctx: 64_000,
                    enabled: true,
                    ..Default::default()
                },
                supports_tools: true,
                supports_agent: true,
                ..Default::default()
            }),
        );
    }
    caps.defaults.chat_default_model = "model".into();
    let mut state = app.gcx.caps_state.write().await;
    state.caps = Some(Arc::new(caps));
    state.last_attempted_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
}

async fn approve(
    app: &AppState,
    code: &str,
) -> (refact_chat_api::PythonApprovalRequest, ChatMessage) {
    let task = spawn(app, code);
    let request = next_request(app, &task).await;
    decide(app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    (request, finish(task).await)
}

async fn denied(app: &AppState, code: &str) -> refact_chat_api::PythonApprovalRequest {
    let task = spawn(app, code);
    let request = next_request(app, &task).await;
    decide(app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    finish(task).await;
    request
}

fn error_type(m: &ChatMessage) -> Value {
    m.extra["python"]["error"]["type"].clone()
}

#[tokio::test]
async fn models_set_defaults_merges_one_slot_and_invalidates_caps() {
    let (dir, app, _) = fixture().await;
    install(&app).await;
    std::fs::write(
        project(&dir).join(".refact/model_defaults.yaml"),
        "chat:\n  model: zone/model-a\n",
    )
    .unwrap();
    let request = denied(&app, "await engine('models.set_defaults', scope='project', slots={'chat_light': {'model': 'zone/model-b'}})").await;
    assert_eq!(
        request.engine.as_ref().unwrap().title,
        "change default models"
    );
    let untouched =
        std::fs::read_to_string(project(&dir).join(".refact/model_defaults.yaml")).unwrap();
    assert!(!untouched.contains("chat_light"));
    let (request, m) = approve(&app, "(await engine('models.set_defaults', scope='project', slots={'chat_light': {'model': 'zone/model-b'}}))['data']").await;
    let engine = request.engine.unwrap();
    assert!(engine
        .items
        .iter()
        .any(|i| i.label == "chat_light.model" && i.new == Some(json!("zone/model-b"))));
    assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Risky);
    assert!(
        result(&m)["changed"]["chat_light"].is_object(),
        "{}",
        result(&m)
    );
    let saved = std::fs::read_to_string(project(&dir).join(".refact/model_defaults.yaml")).unwrap();
    assert!(
        saved.contains("zone/model-a") && saved.contains("zone/model-b"),
        "{saved}"
    );
    assert!(
        app.gcx.caps_state.read().await.caps.is_none(),
        "caps were not invalidated"
    );
    install(&app).await;
    let (request, _) = approve(&app, "await engine('models.set_defaults', scope='project', slots={'chat': {'model': 'other/model-c'}})").await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Risky);
    assert!(engine.warning.unwrap().contains("different provider"));
    install(&app).await;
    let bad = run(&app, "await engine('models.set_defaults', scope='project', slots={'chat': {'model': 'nowhere/none'}})").await;
    assert_eq!(error_type(&bad), "TypeError");
}

#[tokio::test]
async fn models_reload_rebuilds_the_list() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let (request, m) = approve(
        &app,
        "r = await engine('models.reload')\n[r['ok'], r['error']]",
    )
    .await;
    assert_eq!(request.engine.unwrap().title, "reload the model list");
    let out = result(&m);
    assert!(out.is_array(), "{out}");
}

async fn create_provider(app: &AppState, name: &str, body: Value) {
    let response = crate::providers::http::handle_v1_provider_update(
        axum::extract::State(app.clone()),
        axum::extract::Path(serde_json::from_value(json!({"name":name})).unwrap()),
        hyper::body::Bytes::from(serde_json::to_vec(&body).unwrap()),
    )
    .await
    .unwrap();
    assert!(response.status().is_success());
}

#[tokio::test]
async fn providers_custom_models_and_model_toggle() {
    let (_dir, app, _) = fixture().await;
    create_provider(
        &app,
        "openai_2",
        json!({"base_provider":"openai","api_key":"sk-zonesecret","enabled":true}),
    )
    .await;
    let (request, m) = approve(&app, "(await engine('providers.add_custom_model', name='openai_2', id='zone-model', config={'n_ctx': 32000, 'supports_tools': True}))['data']").await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.title, "add a custom model");
    assert!(engine
        .items
        .iter()
        .any(|i| i.label == "n_ctx" && i.new == Some(json!(32000))));
    assert_eq!(result(&m)["replaced"], false);
    let has = |app: &AppState| {
        let app = app.clone();
        async move {
            app.gcx
                .providers
                .read()
                .await
                .get("openai_2")
                .unwrap()
                .custom_models()
                .contains_key("zone-model")
        }
    };
    assert!(has(&app).await);
    let (request, _) = approve(&app, "await engine('providers.set_model_enabled', name='openai_2', model='zone-model', enabled=True)").await;
    assert_eq!(request.engine.unwrap().title, "enable a model");
    assert!(app
        .gcx
        .providers
        .read()
        .await
        .get("openai_2")
        .unwrap()
        .enabled_models()
        .iter()
        .any(|m| m == "zone-model"));
    denied(
        &app,
        "await engine('providers.remove_custom_model', name='openai_2', model='zone-model')",
    )
    .await;
    assert!(has(&app).await, "a denied removal removed the model");
    let (request, _) = approve(
        &app,
        "await engine('providers.remove_custom_model', name='openai_2', model='zone-model')",
    )
    .await;
    assert_eq!(request.engine.unwrap().items[0].new, Some(json!("removed")));
    assert!(!has(&app).await);
    let bad = run(&app, "await engine('providers.add_custom_model', name='openai_2', id='x', config={'zone_unknown': 1})").await;
    assert_eq!(error_type(&bad), "TypeError");
}

#[tokio::test]
async fn modes_save_merges_classifies_and_refuses_widening_this_chats_mode() {
    let (dir, app, _) = fixture().await;
    let modes = project(&dir).join(".refact/modes");
    std::fs::create_dir_all(&modes).unwrap();
    std::fs::write(
        modes.join("zone_mode.yaml"),
        "schema_version: 1\nid: zone_mode\ntitle: Zone\ntools: [cat]\nzone_extra: keep\n",
    )
    .unwrap();
    let (request, m) = approve(&app, "(await engine('modes.save', kind='modes', id='zone_mode', config={'title': 'Zone two'}))['data']").await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Change);
    assert_eq!(engine.title, "change a mode");
    assert_eq!(result(&m)["created"], false, "{}", result(&m));
    let saved = std::fs::read_to_string(modes.join("zone_mode.yaml")).unwrap();
    assert!(
        saved.contains("Zone two") && saved.contains("zone_extra"),
        "{saved}"
    );
    let request = denied(&app, "await engine('modes.save', kind='modes', id='zone_mode', config={'tools': ['cat', 'shell']})").await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Risky);
    assert!(engine.items.iter().any(|i| i.label == "tools added"));
    assert!(!std::fs::read_to_string(modes.join("zone_mode.yaml"))
        .unwrap()
        .contains("shell"));
    session(&app).await.lock().await.thread.mode = "zone_mode".into();
    let own = run(&app, "await engine('modes.save', kind='modes', id='zone_mode', config={'tools': ['cat', 'shell']})").await;
    assert_eq!(error_type(&own), "PermissionError", "{:?}", own.extra);
    let stale = run(&app, "await engine('modes.save', kind='modes', id='zone_mode', config={'title': 'x'}, expected_version='0000')").await;
    assert_eq!(error_type(&stale), "RuntimeError", "{:?}", stale.extra);
}

#[tokio::test]
async fn modes_project_info_and_skills_trigger() {
    let (dir, app, _) = fixture().await;
    let (request, _) = approve(
        &app,
        "await engine('modes.set_project_info', sections={'git_info': {'enabled': False}})",
    )
    .await;
    assert_eq!(request.engine.unwrap().items[0].label, "git_info");
    let out = result(&run(&app, "[b['enabled'] for b in (await engine('modes.project_info'))['data']['blocks'] if b['section'] == 'git_info']").await);
    assert!(out.as_array().unwrap().iter().all(|v| v == false), "{out}");
    let (request, _) = approve(
        &app,
        "await engine('modes.set_skills_trigger', auto_trigger='inject_full')",
    )
    .await;
    assert!(request
        .engine
        .unwrap()
        .items
        .iter()
        .any(|i| i.label == "prompt size"));
    let saved = std::fs::read_to_string(project(&dir).join(".refact/skills.yaml")).unwrap();
    assert!(saved.contains("inject_full"), "{saved}");
    let same = result(
        &run(
            &app,
            "(await engine('modes.set_skills_trigger', auto_trigger='inject_full'))['text']",
        )
        .await,
    );
    assert_eq!(same, "nothing to change");
}

#[tokio::test]
async fn tools_set_enabled_and_refresh() {
    let (_dir, app, _) = fixture().await;
    std::fs::write(app.gcx.config_dir.join("builtin_tools.yaml"), "tools: {}\n").unwrap();
    let python = run(
        &app,
        "await engine('tools.set_enabled', tool='python', enabled=False)",
    )
    .await;
    assert_eq!(error_type(&python), "PermissionError");
    let (request, _) = approve(
        &app,
        "await engine('tools.set_enabled', tool='cat', enabled=False)",
    )
    .await;
    assert_eq!(
        request.engine.unwrap().class,
        refact_chat_api::PythonEngineClass::Change
    );
    let config =
        std::fs::read_to_string(app.gcx.config_dir.join("builtin_tools.yaml")).unwrap_or_default();
    assert!(config.contains("cat"), "{config}");
    let task = spawn(
        &app,
        "await engine('tools.set_enabled', tool='cat', enabled=True)",
    );
    let request = next_request(&app, &task).await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Risky);
    assert!(engine.warning.unwrap().contains("use this tool again"));
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    finish(task).await;
    let this = run(&app, "await engine('tools.refresh', chat_id='python-test')").await;
    assert_eq!(error_type(&this), "TypeError");
    let other = Arc::new(tokio::sync::Mutex::new(
        crate::chat::types::ChatSession::new("zone-refresh".into()),
    ));
    other.lock().await.thread.frozen_request_prefix = Some(refact_chat_api::FrozenRequestPrefix {
        schema_version: 1,
        created_at: "zone-frozen".into(),
        system_prompt: None,
        tools_canonical: None,
    });
    app.chat
        .sessions
        .write()
        .await
        .insert("zone-refresh".into(), other.clone());
    let (request, _) = approve(
        &app,
        "await engine('tools.refresh', chat_id='zone-refresh')",
    )
    .await;
    assert!(request.engine.unwrap().items[0]
        .old
        .as_ref()
        .unwrap()
        .to_string()
        .contains("zone-frozen"));
    assert!(other.lock().await.thread.frozen_request_prefix.is_none());
}

#[tokio::test]
async fn tools_check_with_an_alias_does_not_run_the_shell_gate() {
    let (dir, app, _) = fixture().await;
    let mut catalog = (*app
        .tool_registry
        .acquire_tool_catalog("agent", Some("model"), None)
        .await)
        .clone();
    if !catalog.index.tools.iter().any(|t| t.name == "shell") {
        let mut shell = catalog.index.tools[0].clone();
        shell.name = "shell".into();
        catalog.index.tools.push(shell);
    }
    session(&app).await.lock().await.tool_catalog = Some(Arc::new(catalog));
    let out = result(&run(&app, "(await engine('tools.check', tool='t_shell', args={'command': 'sudo ls', 'description': 'x'}))['data']").await);
    assert_eq!(out["tool"], "shell", "{out}");
    assert_eq!(out["layers"]["tool_rule"], "deny");
    assert!(
        !project(&dir).join(".refact/shell_audit.jsonl").exists(),
        "the alias went through the real shell gate"
    );
}

#[tokio::test]
async fn privacy_tighten_only_tightens_and_keeps_other_keys() {
    let (_dir, app, _) = fixture().await;
    let path = app.gcx.config_dir.join("privacy.yaml");
    let before = std::fs::read_to_string(&path).unwrap_or_default();
    std::fs::write(&path, format!("{before}\nhooks:\n  enabled: true\n")).unwrap();
    let (request, m) = approve(&app, "(await engine('privacy.tighten', block=['zone_secrets/**'], new_zone={'name': 'zone_b', 'patterns': ['zone_b/**'], 'send_to': ['zone']}))['data']").await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.title, "tighten privacy rules");
    assert!(engine
        .items
        .iter()
        .any(|i| i.label == "blocked" && i.new == Some(json!("zone_secrets/**"))));
    assert!(result(&m)["changed"].as_array().unwrap().len() >= 2);
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        saved.contains("zone_secrets/**") && saved.contains("hooks"),
        "{saved}"
    );
    let out = result(&run(&app, "out = []\nfor kw in [dict(block=['.']), dict(new_zone={'name': 'zone_c', 'patterns': ['x/**'], 'send_to': ['*']}), dict(zone_add_patterns={'zone_missing': ['y']})]:\n    try:\n        await engine('privacy.tighten', **kw)\n        out.append('ok')\n    except TypeError:\n        out.append('TypeError')\nout").await);
    assert_eq!(out, json!(["TypeError", "TypeError", "TypeError"]));
    let (_, _) = approve(
        &app,
        "await engine('privacy.tighten', zone_remove_destinations={'zone_b': ['zone']})",
    )
    .await;
    let effective = app.gcx.privacy_policy_load.read().unwrap().policy.clone();
    assert!(effective
        .zones
        .iter()
        .any(|z| z.name == "zone_b" && z.send_to.is_empty()));
}
