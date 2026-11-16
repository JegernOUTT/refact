use super::*;

fn stats_event(id: &str, chat: &str, day: &str, success: bool) -> String {
    json!({
        "id": id, "ts_start": format!("{day}T10:00:00Z"), "ts_end": format!("{day}T10:00:01Z"),
        "duration_ms": 1000, "chat_id": chat, "root_chat_id": null, "mode": "agent",
        "task_id": null, "task_role": null, "agent_id": null, "card_id": null,
        "model_id": "zone/model-a", "provider": "zone", "model": "model-a",
        "messages_count": 2, "tools_count": 0, "max_tokens": 100, "temperature": null,
        "success": success,
        "error_message": if success { Value::Null } else { json!(format!("{chat} said Bearer abcdefghijklmnop")) },
        "finish_reason": null, "attempt_n": 1, "retry_reason": null,
        "prompt_tokens": 10, "completion_tokens": 5, "cache_read_tokens": null,
        "cache_creation_tokens": null, "total_tokens": 15, "cost_usd": 0.5
    })
    .to_string()
}

fn write_stats(dir: &tempfile::TempDir, lines: &[String]) {
    let stats = dir.path().join("project/.refact/stats");
    std::fs::create_dir_all(&stats).unwrap();
    std::fs::write(stats.join("1.jsonl"), lines.join("\n") + "\n").unwrap();
}

#[tokio::test]
async fn stats_usage_totals_and_date_validation() {
    let (dir, app, _) = fixture().await;
    write_stats(
        &dir,
        &[
            stats_event("e1", "python-test", "2026-01-02", true),
            stats_event("e2", "other-chat", "2026-01-03", false),
            stats_event("e3", "other-chat", "2026-02-01", true),
        ],
    );
    let m = run(
        &app,
        "u = await engine('stats.usage', **{'from': '2026-01-01', 'to': '2026-01-31'})\nbad = []\nfor kw in [{'from': '2026-13-01'}, {'from': '2026-02-01', 'to': '2026-01-01'}]:\n    try:\n        await engine('stats.usage', **kw)\n    except TypeError as e:\n        bad.append(str(e))\n[u['data']['totals'], bad]",
    )
    .await;
    let out = result(&m);
    assert!(out.is_array(), "{}", m.content.content_text_only());
    assert_eq!(out[0]["total_calls"], 2, "{out}");
    assert_eq!(out[0]["failed_calls"], 1, "{out}");
    assert_eq!(out[1].as_array().unwrap().len(), 2, "{out}");
}

#[tokio::test]
async fn stats_events_default_to_this_chat_and_strip_foreign_errors() {
    let (dir, app, _) = fixture().await;
    let mut lines = vec![
        stats_event("own-1", "python-test", "2026-01-02", false),
        stats_event("foreign-1", "other-chat", "2026-01-03", false),
    ];
    for i in 0..5 {
        lines.push(stats_event(
            &format!("bulk-{i}"),
            "other-chat",
            "2026-01-04",
            true,
        ));
    }
    write_stats(&dir, &lines);
    let m = run(
        &app,
        "a = await engine('stats.events')\nb = await engine('stats.events', chat_id='*', success=False)\nc = await engine('stats.events', chat_id='*', limit=2, offset=1)\n[a['data'], b['data'], c['data']]",
    )
    .await;
    let out = result(&m);
    assert!(out.is_array(), "{}", m.content.content_text_only());
    let own = out[0]["events"].as_array().unwrap();
    assert_eq!(own.len(), 1);
    assert_eq!(own[0]["id"], "own-1");
    assert!(own[0]["error_message"]
        .as_str()
        .unwrap()
        .contains("python-test"));
    assert!(!out.to_string().contains("abcdefghijklmnop"), "{out}");
    let failed = out[1]["events"].as_array().unwrap();
    assert_eq!(failed.len(), 2);
    let foreign = failed.iter().find(|e| e["id"] == "foreign-1").unwrap();
    assert!(foreign.get("error_message").is_none(), "{foreign}");
    assert_eq!(out[1]["truncated"], false);
    assert_eq!(out[2]["events"].as_array().unwrap().len(), 2);
    assert_eq!(out[2]["truncated"], true);
}

#[tokio::test]
async fn stats_estimate_cost_with_and_without_pricing() {
    let (_dir, app, _) = fixture().await;
    let mut caps = crate::caps::CodeAssistantCaps::default();
    let mut model_caps = std::collections::HashMap::new();
    model_caps.insert(
        "zone/model-a".to_string(),
        refact_core::model_caps::ModelCapabilities {
            pricing: Some(refact_core::provider_types::ModelPricing {
                prompt: 2.0,
                generated: 10.0,
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    caps.model_caps = std::sync::Arc::new(model_caps);
    {
        let mut state = app.gcx.caps_state.write().await;
        state.caps = Some(std::sync::Arc::new(caps));
        state.last_attempted_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
    let m = run(
        &app,
        "a = await engine('stats.estimate_cost', model='zone/model-a', prompt_tokens=1000000, completion_tokens=100000)\nb = await engine('stats.estimate_cost', model='zone/nothing', prompt_tokens=1, completion_tokens=1)\n[a['data'], b['data']]",
    )
    .await;
    let out = result(&m);
    assert!(out.is_array(), "{}", m.content.content_text_only());
    assert_eq!(out[0]["pricing_known"], true);
    assert_eq!(out[0]["cost"]["total_usd"], 3.0);
    assert_eq!(out[1]["pricing_known"], false);
    assert!(out[1]["cost"].is_null());
}

#[tokio::test]
async fn scheduler_config_reports_startup_state() {
    let (_dir, app, _) = fixture().await;
    let m = run(&app, "(await engine('scheduler.config'))['data']").await;
    let out = result(&m);
    assert_eq!(out["enabled"], true, "{out}");
    assert_eq!(out["disabled_by"], json!([]));
    assert_eq!(out["retry"]["max_attempts"], 3);
    assert_eq!(out["restart_required_to_change"], true);
    assert!(out.get("max_jobs").is_none());
}

#[cfg(unix)]
#[test]
fn scheduler_credentials_list_metadata_without_values() {
    let project = crate::test_paths::tempdir();
    let private = crate::test_paths::tempdir();
    let store = crate::scheduler::credentials::AutomationCredentialStore::open(
        project.path(),
        Some(private.path()),
    )
    .unwrap();
    store
        .set(
            crate::scheduler::credentials::CredentialIdentity {
                id: "fixture-token".into(),
                origin: "https://example.com".into(),
                header: "authorization".into(),
                tenant: "fixture-tenant".into(),
                principal: "fixture-reader".into(),
                scope: vec!["read".into()],
            },
            "fixture-value-xyz".into(),
            None,
        )
        .unwrap();
    let out =
        super::super::scheduler::credentials_for(project.path(), Some(private.path())).unwrap();
    assert_eq!(out["supported"], true);
    assert_eq!(out["credentials"][0]["id"], "fixture-token");
    assert!(out["credentials"][0]["secret_revision"].is_string());
    let text = out.to_string();
    assert!(!text.contains("fixture-value-xyz") && !text.contains("\"secret\":"));
}

#[tokio::test]
async fn scheduler_credentials_refused_in_agent_chats() {
    let (_dir, _app, ccx) = fixture().await;
    ccx.lock().await.subchat_depth = 1;
    let m = run_ccx(
        ccx,
        "try:\n    await engine('scheduler.credentials')\n    r = 'ran'\nexcept PermissionError as e:\n    r = str(e)\nr",
    )
    .await;
    assert!(
        result(&m).as_str().unwrap().contains("root chat"),
        "{}",
        result(&m)
    );
}

#[tokio::test]
async fn gui_voice_reports_build_state() {
    let (_dir, app, _) = fixture().await;
    let m = run(&app, "(await engine('gui.voice'))['data']").await;
    let out = result(&m);
    assert_eq!(out["enabled"], crate::voice::VoiceService::is_enabled());
    assert_eq!(out["model_loaded"], false);
}

async fn install_buddy(
    app: &AppState,
    dir: &tempfile::TempDir,
) -> tokio::sync::broadcast::Receiver<crate::buddy::events::BuddyEvent> {
    let (tx, rx) = tokio::sync::broadcast::channel(16);
    let service = crate::buddy::actor::BuddyService::new(
        dir.path().join("project"),
        crate::buddy::state::default_buddy_state(),
        crate::buddy::settings::BuddySettings::default(),
        vec![],
        crate::buddy::runtime_queue::RuntimeQueue::new(),
        tx,
        None,
    );
    *app.buddy.buddy.lock().await = Some(service);
    rx
}

#[tokio::test]
async fn gui_open_page_previews_sends_and_refuses() {
    let (dir, app, _) = fixture().await;
    let m = run(
        &app,
        "try:\n    await engine('gui.open_page', page='stats')\n    r = 'ran'\nexcept RuntimeError as e:\n    r = str(e)\nr",
    )
    .await;
    assert!(
        result(&m)
            .as_str()
            .unwrap()
            .contains("buddy service not initialized"),
        "{}",
        result(&m)
    );
    let mut rx = install_buddy(&app, &dir).await;
    let m = run(
        &app,
        "out = []\nfor kw in [{'page': 'task_workspace'}, {'page': 'stats', 'task_id': 'x'}, {'page': 'task_workspace', 'task_id': 'no-such'}, {'page': 'settings'}]:\n    try:\n        await engine('gui.open_page', **kw)\n        out.append('ran')\n    except TypeError as e:\n        out.append(str(e))\nout",
    )
    .await;
    let out = result(&m);
    assert!(out[0].as_str().unwrap().contains("needs task_id"), "{out}");
    assert!(
        out[1].as_str().unwrap().contains("only to task_workspace"),
        "{out}"
    );
    assert!(out[2].as_str().unwrap().contains("no task"), "{out}");
    assert!(out[3].as_str().unwrap().contains("page"), "{out}");

    let task = spawn(
        &app,
        "(await engine('gui.open_page', page='knowledge_graph'))['data']",
    );
    let request = next_request(&app, &task).await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(engine.title, "open a page in the GUI");
    assert_eq!(engine.items[0].new, Some(json!("knowledge_graph")));
    assert_eq!(engine.scope.as_deref(), Some("GUI of this engine"));
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    assert_eq!(
        result(&finish(task).await),
        json!({"page":"knowledge_graph","sent":true})
    );
    let event = tokio::time::timeout(Duration::from_secs(15), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            event,
            crate::buddy::events::BuddyEvent::NavigationRequest {
                page: crate::buddy::types::BuddyPage::KnowledgeGraph
            }
        ),
        "{event:?}"
    );

    let task = spawn(
        &app,
        "r = await engine('gui.open_page', page='stats')\n[r['ok'], r['error']]",
    );
    let request = next_request(&app, &task).await;
    *app.buddy.buddy.lock().await = None;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let out = result(&finish(task).await);
    assert_eq!(out[0], false, "{out}");
    assert!(rx.try_recv().is_err());
}
