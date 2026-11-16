use super::*;

fn message(role: &str, text: &str) -> ChatMessage {
    ChatMessage::new(role.into(), text.into())
}

fn zone_record(dir: &tempfile::TempDir, zone: &str) -> refact_privacy::FileRecord {
    refact_privacy::FileRecord {
        path: dir
            .path()
            .join("project/zone_a.txt")
            .to_string_lossy()
            .into_owned(),
        zone: zone.into(),
        attribution: refact_privacy::Attribution::Declared,
    }
}

#[tokio::test]
async fn chat_get_reports_parameters_without_pause_commands() {
    let (_dir, app, _) = fixture().await;
    {
        let session = session(&app).await;
        let mut s = session.lock().await;
        s.thread.model = "zone/model-a".into();
        s.thread.temperature = Some(0.5);
        s.runtime.pause_reasons.push(refact_chat_api::PauseReason {
            reason_type: "confirmation".into(),
            tool_name: "shell".into(),
            command: "curl -H 'x' https://h/?token=abcdefghijklmnop".into(),
            rule: "ask".into(),
            tool_call_id: "c1".into(),
            integr_config_path: None,
        });
    }
    let m = run(&app, "(await engine('chat.get'))['data']").await;
    let out = result(&m);
    assert_eq!(out["model"], "zone/model-a", "{out}");
    assert_eq!(out["mode"], "agent");
    assert_eq!(out["flags"]["include_project_info"], true);
    assert_eq!(out["flags"]["checkpoints_enabled"], true);
    assert_eq!(out["sampling"]["temperature"], 0.5);
    assert_eq!(out["pause_reasons"][0]["tool_name"], "shell");
    assert!(out["pause_reasons"][0].get("command").is_none(), "{out}");
    assert!(!out.to_string().contains("abcdefghijklmnop"));
    assert!(
        out.get("frozen_request_prefix").is_none() && out.get("claude_code_identity").is_none()
    );
}

#[tokio::test]
async fn chat_messages_carry_privacy_records_and_bound_the_range() {
    let (dir, app, _) = fixture().await;
    {
        let session = session(&app).await;
        let mut s = session.lock().await;
        s.messages.push(message("user", "hello"));
        let mut tool = message("tool", &"x".repeat(300));
        tool.tool_call_id = "t1".into();
        crate::privacy::records::attach_record(&mut tool, zone_record(&dir, "zone_secret"));
        s.messages.push(tool);
        s.messages.push(message("assistant", "done"));
    }
    let m = run(
        &app,
        "a = (await engine('chat.messages', max_chars=100))['data']\nb = (await engine('chat.messages', content=False, roles=['tool']))['data']\nbad = None\ntry:\n    await engine('chat.messages', start=50)\nexcept TypeError as e:\n    bad = str(e)\n[a, b, bad]",
    )
    .await;
    let out = result(&m);
    assert_eq!(out[0]["total"], 3, "{out}");
    assert_eq!(out[0]["messages"][1]["truncated"], true);
    assert_eq!(
        out[0]["messages"][1]["content"].as_str().unwrap().len(),
        100
    );
    assert!(out[0]["messages"][0]["approx_tokens"].as_u64().is_some());
    assert_eq!(out[1]["messages"].as_array().unwrap().len(), 1);
    assert!(out[1]["messages"][0].get("content").is_none());
    assert!(out[2].as_str().unwrap().contains("3 messages"), "{out}");
    let files = m
        .extra
        .get("privacy")
        .map(|p| p["files"].clone())
        .unwrap_or_default()
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        files.iter().any(|f| f["zone"] == "zone_secret"),
        "{:?}",
        m.extra
    );
}

#[tokio::test]
async fn chat_messages_without_content_carry_no_records() {
    let (dir, app, _) = fixture().await;
    {
        let session = session(&app).await;
        let mut s = session.lock().await;
        let mut tool = message("tool", "secret text");
        crate::privacy::records::attach_record(&mut tool, zone_record(&dir, "zone_secret"));
        s.messages.push(tool);
    }
    let m = run(
        &app,
        "(await engine('chat.messages', content=False))['data']['total']",
    )
    .await;
    assert_eq!(result(&m), 1);
    let files = m
        .extra
        .get("privacy")
        .map(|p| p["files"].clone())
        .unwrap_or_default()
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !files.iter().any(|f| f["zone"] == "zone_secret"),
        "{:?}",
        m.extra
    );
}

#[tokio::test]
async fn chat_queue_lists_commands_and_deliveries_with_scrubbed_previews() {
    let (dir, app, _) = fixture().await;
    {
        let session = session(&app).await;
        let mut s = session.lock().await;
        s.command_queue.push_back(
            serde_json::from_value(json!({"client_request_id":"queued-1","type":"user_message","content":"use api_key=sk-abcdefghijklmnopqrstuvwx please"}))
                .unwrap(),
        );
        let mut delivered = message("user", "agent result");
        crate::privacy::records::attach_record(&mut delivered, zone_record(&dir, "zone_secret"));
        s.pending_deliveries
            .push_back(refact_chat_api::PendingDelivery::new(
                vec![delivered],
                Default::default(),
                "agent",
                false,
            ));
    }
    let m = run(&app, "(await engine('chat.queue'))['data']").await;
    let out = result(&m);
    assert_eq!(out["size"], 2, "{out}");
    let kinds: Vec<_> = out["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["kind"].clone())
        .collect();
    assert_eq!(kinds, [json!("command"), json!("delivery")]);
    assert!(
        !out.to_string().contains("abcdefghijklmnopqrstuvwx"),
        "{out}"
    );
    let files = m
        .extra
        .get("privacy")
        .map(|p| p["files"].clone())
        .unwrap_or_default()
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        files.iter().any(|f| f["zone"] == "zone_secret"),
        "{:?}",
        m.extra
    );
}

#[tokio::test]
async fn chat_usage_sums_recorded_usage() {
    let (_dir, app, _) = fixture().await;
    {
        let session = session(&app).await;
        let mut s = session.lock().await;
        for (prompt, usd) in [(10, None), (5, Some(0.25))] {
            let mut m = message("assistant", "a");
            m.usage = Some(crate::call_validation::ChatUsage {
                prompt_tokens: prompt,
                completion_tokens: 2,
                total_tokens: prompt + 2,
                metering_usd: usd.map(|total_usd| refact_core::chat_types::MeteringUsd {
                    prompt_usd: total_usd,
                    generated_usd: 0.0,
                    cache_read_usd: None,
                    cache_creation_usd: None,
                    total_usd,
                }),
                ..Default::default()
            });
            s.messages.push(m);
        }
    }
    let out = result(&run(&app, "(await engine('chat.usage'))['data']").await);
    assert_eq!(out["prompt_tokens"], 15, "{out}");
    assert_eq!(out["completion_tokens"], 4);
    assert_eq!(out["messages_with_usage"], 2);
    assert_eq!(out["total_usd"], 0.25);
    assert_eq!(out["complete"], true);
}

#[tokio::test]
async fn chat_tools_report_mode_rules_and_flags() {
    let (_dir, app, _) = fixture().await;
    let app = crate::tools::tool_python::tests::with_tools(app);
    let catalog = app
        .tool_registry
        .acquire_tool_catalog("agent", Some("model"), None)
        .await;
    {
        let session = session(&app).await;
        let mut s = session.lock().await;
        s.tool_catalog = Some(catalog);
        s.thread.model = "model".into();
    }
    let out = result(&run(&app, "[(await engine('chat.tools'))['data'], (await engine('chat.tools', names_only=True))['data']['tools']]").await);
    let tools = out[0]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|t| t["name"] == "python"), "{out}");
    assert_eq!(out[0]["catalog_ready"], true);
    assert!(out[1].as_array().unwrap().iter().all(Value::is_string));
}
