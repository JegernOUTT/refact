use super::*;

fn chat_model(id: &str, n_ctx: usize) -> Arc<crate::caps::ChatModelRecord> {
    Arc::new(crate::caps::ChatModelRecord {
        base: crate::caps::BaseModelRecord {
            id: id.into(),
            n_ctx,
            enabled: true,
            ..Default::default()
        },
        supports_tools: true,
        supports_agent: true,
        reasoning_effort_options: Some(vec!["low".into(), "high".into()]),
        max_output_tokens: Some(8000),
        supports_temperature: id != "zone/cold",
        ..Default::default()
    })
}

/// Caps with two models of provider `zone`, one of `other` and one without temperature; the fixture
/// chat runs `zone/model-a` and looks like a chat in the middle of its turn.
async fn busy_chat(app: &AppState) {
    let mut caps = crate::caps::CodeAssistantCaps::default();
    for (id, n_ctx) in [
        ("zone/model-a", 64_000),
        ("zone/model-b", 32_000),
        ("other/model-c", 128_000),
        ("zone/cold", 16_000),
    ] {
        caps.chat_models.insert(id.into(), chat_model(id, n_ctx));
    }
    caps.defaults.chat_default_model = "zone/model-a".into();
    {
        let mut state = app.gcx.caps_state.write().await;
        state.caps = Some(Arc::new(caps));
        state.last_attempted_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;
    }
    let session = session(app).await;
    let mut s = session.lock().await;
    s.thread.model = "zone/model-a".into();
    s.thread.mode = "explore".into();
    s.runtime.state = refact_chat_api::SessionState::ExecutingTools;
    s.turn_depth = 1;
}

/// Ends the simulated turn and waits until the queue processor ran everything queued.
async fn end_turn(app: &AppState) {
    let session = session(app).await;
    {
        let mut s = session.lock().await;
        s.runtime.state = refact_chat_api::SessionState::Idle;
        s.turn_depth = 0;
        s.queue_notify.notify_one();
    }
    tokio::time::timeout(Duration::from_secs(20), async {
        while !session.lock().await.command_queue.is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the queued command did not run");
}

async fn thread(app: &AppState) -> crate::chat::types::ThreadParams {
    session(app).await.lock().await.thread.clone()
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

#[tokio::test]
async fn set_model_queues_and_applies_after_the_turn() {
    let (_dir, app, _) = fixture().await;
    busy_chat(&app).await;
    let (request, m) = approve(
        &app,
        "(await engine('chat.set_model', model='zone/model-b'))",
    )
    .await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.title, "switch this chat's model");
    assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Change);
    assert_eq!(
        engine.scope.as_deref(),
        Some("this chat, from its next turn")
    );
    assert_eq!(engine.items[0].label, "model");
    assert_eq!(engine.items[0].old, Some(json!("zone/model-a")));
    assert_eq!(engine.items[0].new, Some(json!("zone/model-b")));
    assert!(engine
        .items
        .iter()
        .any(|i| i.label == "context window" && i.new == Some(json!(32000))));
    let out = result(&m);
    assert_eq!(out["data"]["new_model"], "zone/model-b", "{out}");
    assert!(out["text"].as_str().unwrap().contains("after this turn"));
    assert_eq!(
        thread(&app).await.model,
        "zone/model-a",
        "applied before the turn ended"
    );
    let queued = session(&app).await.lock().await.command_queue.len();
    assert_eq!(queued, 1);
    end_turn(&app).await;
    assert_eq!(thread(&app).await.model, "zone/model-b");
}

#[tokio::test]
async fn set_model_raises_for_another_provider_and_refuses_unknown_models() {
    let (_dir, app, _) = fixture().await;
    busy_chat(&app).await;
    let task = spawn(
        &app,
        "await engine('chat.set_model', model='other/model-c')",
    );
    let request = next_request(&app, &task).await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Risky);
    assert!(engine.warning.unwrap().contains("different provider"));
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    finish(task).await;
    assert!(
        session(&app).await.lock().await.command_queue.is_empty(),
        "a denied card queued something"
    );
    let m = run(&app, "r = None\ntry:\n    await engine('chat.set_model', model='nowhere/none')\nexcept TypeError as e:\n    r = str(e)\nr").await;
    assert!(
        result(&m).as_str().is_some(),
        "{}",
        m.content.content_text_only()
    );
    assert!(session(&app).await.lock().await.command_queue.is_empty());
    let same = result(
        &run(
            &app,
            "(await engine('chat.set_model', model='zone/model-a'))['text']",
        )
        .await,
    );
    assert_eq!(same, "nothing to change");
}

#[tokio::test]
async fn set_mode_applies_mode_defaults_refuses_bound_and_unknown_modes() {
    let (_dir, app, _) = fixture().await;
    busy_chat(&app).await;
    let (request, _) = approve(
        &app,
        "await engine('chat.set_mode', mode='agent', reason='zone switch')",
    )
    .await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.title, "switch this chat's mode");
    assert_eq!(
        engine.class,
        refact_chat_api::PythonEngineClass::Risky,
        "agent turns editing auto-approval on"
    );
    assert!(engine.warning.unwrap().contains("without asking"));
    assert!(engine
        .items
        .iter()
        .any(|i| i.label == "auto_approve_editing_tools" && i.new == Some(json!(true))));
    assert!(engine.items.iter().any(|i| i.label == "system prompt"));
    end_turn(&app).await;
    let t = thread(&app).await;
    assert_eq!(t.mode, "agent");
    assert!(
        t.auto_approve_editing_tools,
        "the mode's thread defaults were not applied"
    );
    let out = result(&run(&app, "out = []\nfor m in ['task_planner', 'agent!!']:\n    try:\n        await engine('chat.set_mode', mode=m)\n        out.append('ok')\n    except (TypeError, PermissionError) as e:\n        out.append(type(e).__name__)\nout").await);
    assert_eq!(out, json!(["PermissionError", "TypeError"]));
}

#[tokio::test]
async fn queued_mode_switch_clears_the_frozen_request_prefix() {
    let (_dir, app, _) = fixture().await;
    busy_chat(&app).await;
    session(&app)
        .await
        .lock()
        .await
        .thread
        .frozen_request_prefix = Some(refact_chat_api::FrozenRequestPrefix {
        schema_version: 1,
        created_at: "zone-frozen".into(),
        system_prompt: Some("old prompt".into()),
        tools_canonical: Some(json!([{"name":"old_tool"}])),
    });
    approve(&app, "await engine('chat.set_mode', mode='ask')").await;
    assert!(thread(&app).await.frozen_request_prefix.is_some());
    end_turn(&app).await;
    let t = thread(&app).await;
    assert_eq!(t.mode, "ask");
    assert!(
        t.frozen_request_prefix.is_none(),
        "the next request would still carry the old tools"
    );
}

#[tokio::test]
async fn reasoning_sampling_context_flags_and_recovery_are_checked() {
    let (_dir, app, _) = fixture().await;
    busy_chat(&app).await;
    let code = "out = []\nfor name, kw in [('chat.set_reasoning', dict(reasoning_effort='medium')), ('chat.set_reasoning', dict()), ('chat.set_sampling', dict(temperature=2.5)), ('chat.set_context', dict(context_tokens_cap=100000)), ('chat.set_flags', dict(zone_flag=True)), ('chat.set_recovery', dict(max_retries=101)), ('chat.set_recovery', dict(reset=True, max_retries=1)), ('chat.set_goal_budget', dict(max_turns=3))]:\n    try:\n        await engine(name, **kw)\n        out.append('ok')\n    except TypeError as e:\n        out.append('TypeError')\nout";
    let out = result(&run(&app, code).await);
    assert_eq!(out, json!(vec!["TypeError"; 8]), "{out}");
    let (request, _) = approve(
        &app,
        "await engine('chat.set_reasoning', reasoning_effort='high')",
    )
    .await;
    assert_eq!(request.engine.unwrap().items[0].new, Some(json!("high")));
    let (request, _) = approve(
        &app,
        "await engine('chat.set_flags', checkpoints_enabled=False)",
    )
    .await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Risky);
    assert!(engine.warning.unwrap().contains("checkpoints"));
    let (request, _) = approve(&app, "await engine('chat.set_recovery', max_retries=5)").await;
    assert!(request
        .engine
        .unwrap()
        .warning
        .unwrap()
        .contains("retried more"));
    let (request, _) = approve(
        &app,
        "await engine('chat.set_context', context_tokens_cap=8000, auto_compression_cap=None)",
    )
    .await;
    assert!(request
        .engine
        .unwrap()
        .items
        .iter()
        .any(|i| i.label == "context_tokens_cap"));
    end_turn(&app).await;
    let t = thread(&app).await;
    assert_eq!(t.reasoning_effort.as_deref(), Some("high"));
    assert!(!t.checkpoints_enabled);
    assert_eq!(t.recovery_policy.and_then(|p| p.max_retries), Some(5));
    assert_eq!(t.context_tokens_cap, Some(8000));
    assert!(t.auto_compression_cap_pending);
}

#[tokio::test]
async fn temperature_is_refused_for_a_model_without_it() {
    let (_dir, app, _) = fixture().await;
    busy_chat(&app).await;
    session(&app).await.lock().await.thread.model = "zone/cold".into();
    let out = result(&run(&app, "try:\n    await engine('chat.set_sampling', temperature=0.5)\n    r = 'ok'\nexcept TypeError as e:\n    r = str(e)\nr").await);
    assert!(out.as_str().unwrap().contains("temperature"), "{out}");
}

#[tokio::test]
async fn a_change_moved_after_the_preview_is_not_applied() {
    let (_dir, app, _) = fixture().await;
    busy_chat(&app).await;
    let task = spawn(
        &app,
        "r = await engine('chat.set_model', model='zone/model-b')\n[r['ok'], r['error']]",
    );
    let request = next_request(&app, &task).await;
    session(&app).await.lock().await.thread.model = "other/model-c".into();
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let out = result(&finish(task).await);
    assert_eq!(out[0], false, "{out}");
    assert!(out[1]
        .as_str()
        .unwrap()
        .contains("changed since the preview"));
    assert!(session(&app).await.lock().await.command_queue.is_empty());
}

#[tokio::test]
async fn goal_budget_and_stop_goal() {
    let (_dir, app, _) = fixture().await;
    busy_chat(&app).await;
    let none = result(&run(&app, "try:\n    await engine('chat.stop_goal')\n    r = 'ok'\nexcept TypeError as e:\n    r = str(e)\nr").await);
    assert!(none.as_str().unwrap().contains("no goal"), "{none}");
    session(&app)
        .await
        .lock()
        .await
        .install_goal("agent", "zone goal", true, Default::default());
    let (request, _) = approve(
        &app,
        "await engine('chat.set_goal_budget', cooldown_ms=1234)",
    )
    .await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.title, "change this chat's goal budget");
    assert_eq!(engine.items[0].new, Some(json!(1234)));
    let (request, m) = approve(&app, "await engine('chat.stop_goal')").await;
    assert_eq!(request.engine.unwrap().items[0].new, Some(json!("stopped")));
    assert!(result(&m)["text"]
        .as_str()
        .unwrap()
        .contains("after this turn"));
    end_turn(&app).await;
    let session = session(&app).await;
    let s = session.lock().await;
    assert_eq!(s.goal.as_ref().unwrap().budget.cooldown_ms, 1234);
    assert_eq!(s.goal_status, Some(refact_chat_api::GoalStatus::Stopped));
    assert!(s.messages.iter().any(|m| m
        .content
        .content_text_only()
        .contains("Goal pursuit stopped")));
}

#[tokio::test]
async fn cancel_queued_removes_one_item_and_is_destructive_for_user_messages() {
    let (_dir, app, _) = fixture().await;
    busy_chat(&app).await;
    {
        let session = session(&app).await;
        let mut s = session.lock().await;
        for (id, command) in [
            (
                "zone-q1",
                json!({"type":"set_params","patch":{"temperature":0.1}}),
            ),
            (
                "zone-q2",
                json!({"type":"set_params","patch":{"temperature":0.2}}),
            ),
            (
                "zone-q3",
                json!({"type":"user_message","content":"zone typed text"}),
            ),
        ] {
            let mut request = command;
            request["client_request_id"] = json!(id);
            s.command_queue
                .push_back(serde_json::from_value(request).unwrap());
        }
    }
    let (request, m) = approve(
        &app,
        "(await engine('chat.cancel_queued', id='zone-q1'))['data']",
    )
    .await;
    assert_eq!(
        request.engine.unwrap().class,
        refact_chat_api::PythonEngineClass::Change
    );
    assert_eq!(result(&m)["removed"], true);
    let ids: Vec<String> = session(&app)
        .await
        .lock()
        .await
        .command_queue
        .iter()
        .map(|r| r.client_request_id.clone())
        .collect();
    assert_eq!(ids, ["zone-q2", "zone-q3"]);
    let task = spawn(&app, "await engine('chat.cancel_queued', id='zone-q3')");
    let request = next_request(&app, &task).await;
    let engine = request.engine.unwrap();
    assert_eq!(
        engine.class,
        refact_chat_api::PythonEngineClass::Destructive
    );
    assert!(engine.warning.unwrap().contains("cannot be restored"));
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    finish(task).await;
    assert_eq!(session(&app).await.lock().await.command_queue.len(), 2);
    let unknown = result(&run(&app, "try:\n    await engine('chat.cancel_queued', id='zone-missing')\n    r = 'ok'\nexcept TypeError as e:\n    r = 'TypeError'\nr").await);
    assert_eq!(unknown, "TypeError");
}
