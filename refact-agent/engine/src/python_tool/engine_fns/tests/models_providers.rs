use super::*;

async fn install_caps(app: &AppState) {
    let mut caps = crate::caps::CodeAssistantCaps::default();
    let mut model = crate::caps::ChatModelRecord {
        base: crate::caps::BaseModelRecord {
            id: "zone/model-a".into(),
            n_ctx: 64_000,
            enabled: true,
            endpoint: "https://u:p@host.invalid/v1?key=zonesecretendpoint".into(),
            api_key: "sk-zonesecretkeyvalue".into(),
            ..Default::default()
        },
        supports_tools: true,
        max_output_tokens: Some(4096),
        ..Default::default()
    };
    model
        .base
        .extra_headers
        .insert("Authorization".into(), "Bearer zonesecretheader".into());
    caps.chat_models
        .insert("zone/model-a".into(), Arc::new(model));
    caps.chat_models.insert(
        "zone/model-b".into(),
        Arc::new(crate::caps::ChatModelRecord {
            base: crate::caps::BaseModelRecord {
                id: "zone/model-b".into(),
                n_ctx: 8_000,
                enabled: true,
                ..Default::default()
            },
            ..Default::default()
        }),
    );
    caps.defaults.chat_default_model = "zone/model-a".into();
    caps.defaults.chat_light_model = "zone/model-b".into();
    let mut state = app.gcx.caps_state.write().await;
    state.caps = Some(Arc::new(caps));
    state.last_attempted_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
}

#[tokio::test]
async fn models_list_and_get_pick_fields_without_secrets() {
    let (_dir, app, _) = fixture().await;
    install_caps(&app).await;
    let m = run(
        &app,
        "l = (await engine('models.list'))['data']\ng = (await engine('models.get', model='zone/model-a'))['data']\nbad = None\ntry:\n    await engine('models.get', model='nowhere/none-such-model-xyz')\nexcept TypeError as e:\n    bad = str(e)\n[l, g, bad]",
    )
    .await;
    let out = result(&m);
    let models = out[0]["models"].as_array().unwrap();
    assert_eq!(models.len(), 2, "{out}");
    let a = models.iter().find(|m| m["id"] == "zone/model-a").unwrap();
    assert_eq!(a["slots"], json!(["chat"]));
    assert_eq!(a["max_output_tokens"], 4096);
    let b = models.iter().find(|m| m["id"] == "zone/model-b").unwrap();
    assert_eq!(b["slots"], json!(["light"]));
    assert_eq!(out[1]["in_caps"], true);
    assert_eq!(out[1]["n_ctx"], 64000);
    assert!(out[2].as_str().is_some(), "{out}");
    let text = Value::Object(m.extra.clone()).to_string() + &out.to_string();
    for secret in ["zonesecret", "endpoint", "extra_headers", "api_key"] {
        assert!(!out.to_string().contains(secret), "{secret} in {out}");
    }
    assert!(!text.contains("zonesecret"));
}

#[tokio::test]
async fn models_defaults_merge_the_project_slot() {
    let (dir, app, _) = fixture().await;
    install_caps(&app).await;
    std::fs::create_dir_all(dir.path().join("config/providers.d")).unwrap();
    std::fs::write(
        dir.path().join("config/providers.d/defaults.yaml"),
        "chat:\n  model: zone/model-b\n  temperature: 0.2\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("project/.refact/model_defaults.yaml"),
        "chat:\n  model: zone/model-a\n",
    )
    .unwrap();
    let out = result(&run(&app, "(await engine('models.defaults'))['data']").await);
    assert_eq!(out["global"]["chat"]["model"], "zone/model-b", "{out}");
    assert_eq!(out["project"]["available"], true);
    assert_eq!(out["project"]["defaults"]["chat"]["model"], "zone/model-a");
    assert_eq!(out["effective"]["chat"]["model"], "zone/model-a");
    assert!(
        out["effective"]["chat"].get("temperature").is_none(),
        "slot replaced wholesale: {out}"
    );
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
async fn providers_list_and_get_mask_every_secret() {
    let (_dir, app, _) = fixture().await;
    create_provider(&app, "openai_2", json!({"base_provider":"openai","api_key":"sk-zonesecretopenai","enabled":true,"enabled_models":["gpt-4.1"]})).await;
    create_provider(&app, "custom_2", json!({"base_provider":"custom","api_key":"zonesecretcustom","chat_endpoint":"https://u:zonesecretpass@host.invalid/v1?api_key=zonesecretquery","extra_headers":{"Authorization":"Bearer zonesecretheader"},"enabled":true})).await;
    let m = run(
        &app,
        "l = (await engine('providers.list'))['data']\na = (await engine('providers.get', provider='openai_2'))['data']\nb = (await engine('providers.get', provider='custom_2'))['data']\nbad = None\ntry:\n    await engine('providers.get', provider='Bad Name!')\nexcept TypeError as e:\n    bad = str(e)\n[l, a, b, bad]",
    )
    .await;
    let out = result(&m);
    let rows = out[0]["providers"]
        .as_array()
        .unwrap_or_else(|| panic!("{out}"));
    let row = rows.iter().find(|p| p["name"] == "openai_2").unwrap();
    assert!(
        row["has_credentials"] == true && row["status"] != "not_configured",
        "{row}"
    );
    assert!(rows.iter().any(|p| p["status"] == "not_configured"));
    assert_eq!(out[1]["base_provider"], "openai");
    assert!(out[2]["settings"].is_object());
    assert!(out[3].as_str().is_some(), "{out}");
    let all = m.content.content_text_only()
        + &out.to_string()
        + &Value::Object(m.extra.clone()).to_string();
    assert!(!all.contains("zonesecret"), "{all}");
}

#[tokio::test]
async fn providers_available_models_falls_back_without_network() {
    let (_dir, app, _) = fixture().await;
    create_provider(
        &app,
        "openai_2",
        json!({"base_provider":"openai","api_key":"sk-zonesecretopenai","enabled":true}),
    )
    .await;
    let out = result(
        &run(
            &app,
            "(await engine('providers.available_models', provider='openai_2'))['data']",
        )
        .await,
    );
    assert!(out["source"].is_string(), "{out}");
    assert!(out["models"].is_array());
    assert!(!out.to_string().contains("zonesecret"));
}

#[tokio::test]
async fn providers_status_uses_a_fresh_cached_quota_and_redacts_blobs() {
    let (_dir, app, _) = fixture().await;
    create_provider(
        &app,
        "openai_2",
        json!({"base_provider":"openai","api_key":"sk-zonesecretopenai","enabled":true}),
    )
    .await;
    let mut snapshot =
        crate::providers::quota::ProviderQuotaSnapshot::unavailable("openai_2", "openai");
    snapshot.available = true;
    snapshot.plan = Some("zone plan".into());
    snapshot.litellm = Some(json!({"api_key":"zonesecretblob","spend":3}));
    app.gcx
        .quota_cache
        .lock()
        .await
        .insert("openai_2".into(), snapshot);
    let out = result(
        &run(
            &app,
            "(await engine('providers.status', provider='openai_2'))['data']",
        )
        .await,
    );
    assert_eq!(out["quotas"][0]["plan"], "zone plan", "{out}");
    assert_eq!(out["quotas"][0]["litellm"]["spend"], 3);
    assert!(!out.to_string().contains("zonesecret"), "{out}");
    assert!(out.get("health").is_none());
}

#[tokio::test]
#[serial_test::serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn providers_pressure_shows_buckets_without_credentials() {
    let (_dir, app, _) = fixture().await;
    refact_providers::pressure::uninstall_pressure_registry();
    let off = result(
        &run(
            &app,
            "(await engine('providers.pressure'))['data']['enabled']",
        )
        .await,
    );
    assert_eq!(off, false);
    let registry = refact_providers::pressure::install_pressure_registry(
        refact_providers::pressure::PressureConfig::default(),
    )
    .unwrap();
    let bucket = refact_providers::pressure::bucket_key("zone", "sk-zonesecretpressure");
    let permit = registry
        .try_admit(
            &bucket,
            refact_providers::pressure::RequestPriority::Interactive,
        )
        .unwrap();
    let out = result(&run(&app, "(await engine('providers.pressure'))['data']").await);
    drop(permit);
    refact_providers::pressure::uninstall_pressure_registry();
    let row = out["buckets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["bucket"] == bucket.as_str())
        .cloned();
    assert_eq!(row.unwrap()["in_flight"], 1, "{out}");
    assert!(!out.to_string().contains("zonesecret"));
}
