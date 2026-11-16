use super::*;
use crate::chat::types::ChatSession;

async fn install(app: &AppState) {
    let mut caps = crate::caps::CodeAssistantCaps::default();
    for id in ["model", "zone/model-a", "zone/model-b"] {
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

async fn other(
    app: &AppState,
    id: &str,
    f: impl FnOnce(&mut ChatSession),
) -> Arc<tokio::sync::Mutex<ChatSession>> {
    let mut s = ChatSession::new(id.into());
    s.thread.title = format!("{id} title");
    s.thread.model = "zone/model-a".into();
    s.thread.mode = "explore".into();
    let mut user = ChatMessage::new("user".into(), "first".into());
    user.message_id = format!("{id}-m1");
    let mut tool = ChatMessage::new("tool".into(), "zone file text".into());
    tool.message_id = format!("{id}-m2");
    tool.tool_call_id = "t1".into();
    crate::privacy::records::attach_record(
        &mut tool,
        refact_privacy::FileRecord {
            path: "/zone/secret.txt".into(),
            zone: "zone_secret".into(),
            attribution: refact_privacy::Attribution::Declared,
        },
    );
    let mut last = ChatMessage::new("assistant".into(), "done".into());
    last.message_id = format!("{id}-m3");
    s.messages = vec![user, tool, last];
    s.trajectory_version = 1;
    f(&mut s);
    crate::chat::trajectories::save_trajectory_snapshot(
        app.gcx.clone(),
        crate::chat::trajectories::trajectory_snapshot_from_session(&s),
    )
    .await
    .unwrap();
    let arc = Arc::new(tokio::sync::Mutex::new(s));
    app.chat
        .sessions
        .write()
        .await
        .insert(id.into(), arc.clone());
    arc
}

async fn until<F: Fn(&ChatSession) -> bool>(
    session: &Arc<tokio::sync::Mutex<ChatSession>>,
    done: F,
) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !done(&*session.lock().await) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the other chat did not reach the expected state");
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

fn error_type(m: &ChatMessage) -> Value {
    m.extra["python"]["error"]["type"].clone()
}

#[tokio::test]
async fn set_params_applies_to_an_idle_chat_and_refuses_busy_unknown_and_this_chat() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-idle", |s| {
        s.thread.root_chat_id = Some("python-test".into())
    })
    .await;
    let (request, m) = approve(&app, "(await engine('chats.set_params', chat_id='zone-idle', model='zone/model-b', mode='agent'))['data']").await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.title, "change another chat's settings");
    assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Risky);
    assert!(engine.warning.unwrap().starts_with("zone-idle title:"));
    assert!(engine.scope.unwrap().contains("zone-idle"));
    assert!(result(&m)["changed"]["model"].is_object(), "{}", result(&m));
    until(&target, |s| {
        s.thread.model == "zone/model-b" && s.thread.mode == "agent"
    })
    .await;
    let busy = other(&app, "zone-busy", |s| {
        s.runtime.state = refact_chat_api::SessionState::Generating
    })
    .await;
    let _ = busy;
    let before: Vec<String> = app.chat.sessions.read().await.keys().cloned().collect();
    for (id, kind) in [
        ("zone-busy", "PermissionError"),
        ("zone-unknown-chat", "TypeError"),
        ("python-test", "TypeError"),
    ] {
        let m = run(
            &app,
            &format!("await engine('chats.set_params', chat_id='{id}', temperature=0.3)"),
        )
        .await;
        assert_eq!(error_type(&m), kind, "{id}: {:?}", m.extra);
    }
    let after: Vec<String> = app.chat.sessions.read().await.keys().cloned().collect();
    assert_eq!(before.len(), after.len(), "an unknown id created a session");
}

#[tokio::test]
async fn set_title_renames_even_a_busy_chat_and_deny_leaves_it() {
    let (_dir, app, _) = fixture().await;
    let target = other(&app, "zone-title", |s| {
        s.runtime.state = refact_chat_api::SessionState::Generating
    })
    .await;
    let task = spawn(
        &app,
        "await engine('chats.set_title', chat_id='zone-title', title='zone renamed')",
    );
    let request = next_request(&app, &task).await;
    assert_eq!(
        request.engine.as_ref().unwrap().items[0].new,
        Some(json!("zone renamed"))
    );
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    finish(task).await;
    assert!(target.lock().await.command_queue.is_empty());
    approve(
        &app,
        "await engine('chats.set_title', chat_id='zone-title', title='zone renamed')",
    )
    .await;
    {
        let mut s = target.lock().await;
        s.runtime.state = refact_chat_api::SessionState::Idle;
        s.queue_notify.notify_one();
    }
    until(&target, |s| s.thread.title == "zone renamed").await;
    let bad = run(
        &app,
        "await engine('chats.set_title', chat_id='zone-title', title='two\\nlines')",
    )
    .await;
    assert_eq!(error_type(&bad), "TypeError");
}

#[tokio::test]
async fn send_delivers_with_run_records_and_is_risky_for_autonomous_chats() {
    let (dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-send", |s| {
        s.thread.auto_approve_editing_tools = true
    })
    .await;
    std::fs::write(dir.path().join("project/run_secret.txt"), "zone note").unwrap();
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.zones = vec![refact_privacy::Zone {
        name: "zone_note".into(),
        patterns: vec!["run_secret.txt".into()],
        send_to: vec!["*".into()],
        on_shell_read: refact_privacy::ShellBehavior::Withhold,
    }];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
    let (request, m) = approve(
        &app,
        "open('run_secret.txt').read()\n(await engine('chats.send', chat_id='zone-send', text='zone hello'))['data']",
    )
    .await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Risky);
    assert!(engine.warning.unwrap().contains("without asking"));
    assert_eq!(result(&m)["queued"], true, "{}", result(&m));
    until(&target, |s| {
        s.messages
            .iter()
            .any(|m| m.content.content_text_only() == "zone hello")
            || s.pending_deliveries.iter().any(|d| {
                d.messages
                    .iter()
                    .any(|m| m.content.content_text_only() == "zone hello")
            })
    })
    .await;
    let s = target.lock().await;
    let sent = s
        .messages
        .iter()
        .chain(s.pending_deliveries.iter().flat_map(|d| d.messages.iter()))
        .find(|m| m.content.content_text_only() == "zone hello")
        .unwrap();
    assert!(
        sent.extra
            .get("privacy")
            .is_some_and(|p| p.to_string().contains("zone_note")),
        "{:?}",
        sent.extra
    );
}

#[tokio::test]
async fn send_is_refused_when_the_run_data_may_not_reach_that_chats_model() {
    let (dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-egress", |_| {}).await;
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.zones = vec![refact_privacy::Zone {
        name: "zone_a".into(),
        patterns: vec!["zone_a.txt".into()],
        send_to: vec!["model".into()],
        on_shell_read: refact_privacy::ShellBehavior::Withhold,
    }];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
    std::fs::write(dir.path().join("project/zone_a.txt"), "guarded").unwrap();
    let m = run(&app, "x = open('zone_a.txt').read()\nawait engine('chats.send', chat_id='zone-egress', text='zone ' + x)").await;
    assert_eq!(error_type(&m), "PermissionError", "{:?}", m.extra);
    assert!(session(&app)
        .await
        .lock()
        .await
        .runtime
        .python_approvals
        .is_empty());
    let s = target.lock().await;
    assert!(
        s.pending_deliveries.is_empty() && s.messages.len() == 3,
        "something was sent"
    );
}

#[tokio::test]
async fn abort_stops_a_busy_chat_and_is_nothing_for_an_idle_one() {
    let (_dir, app, _) = fixture().await;
    let idle = other(&app, "zone-abort-idle", |_| {}).await;
    let _ = idle;
    let out = result(
        &run(
            &app,
            "(await engine('chats.abort', chat_id='zone-abort-idle'))['text']",
        )
        .await,
    );
    assert_eq!(out, "nothing to change");
    let busy = other(&app, "zone-abort-busy", |s| {
        s.runtime.state = refact_chat_api::SessionState::Generating
    })
    .await;
    let (request, m) = approve(
        &app,
        "(await engine('chats.abort', chat_id='zone-abort-busy'))['data']",
    )
    .await;
    assert_eq!(
        request.engine.unwrap().items[0].old,
        Some(json!("generating"))
    );
    assert_eq!(result(&m)["aborted"], true);
    assert!(busy
        .lock()
        .await
        .abort_flag
        .load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn fork_copies_history_and_keeps_privacy_records() {
    let (_dir, app, _) = fixture().await;
    other(&app, "zone-fork", |_| {}).await;
    let missing = run(
        &app,
        "await engine('chats.fork', chat_id='zone-fork', up_to_message_id='zone-nope')",
    )
    .await;
    assert_eq!(error_type(&missing), "TypeError");
    let (request, m) = approve(&app, "(await engine('chats.fork', chat_id='zone-fork', up_to_message_id='zone-fork-m2'))['data']").await;
    assert_eq!(request.engine.unwrap().items[1].new, Some(json!(2)));
    let out = result(&m);
    assert_eq!(out["messages_copied"], 2, "{out}");
    let new_id = out["new_chat_id"].as_str().unwrap().to_string();
    let forked = app.chat.sessions.read().await[&new_id].clone();
    let s = forked.lock().await;
    assert_eq!(s.thread.parent_id.as_deref(), Some("zone-fork"));
    let records = crate::privacy::records::records_to_carry(&s.messages).unwrap();
    assert!(
        records.iter().any(|r| r.zone == "zone_secret"),
        "the fork dropped the source's records"
    );
}

#[tokio::test]
async fn compact_preview_reports_and_refuses_ineligible_chats() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    other(&app, "zone-compact", |_| {}).await;
    let out = result(
        &run(
            &app,
            "(await engine('chats.compact_preview', chat_id='zone-compact'))['data']",
        )
        .await,
    );
    assert!(out.get("eligible").is_some(), "{out}");
    assert!(session(&app)
        .await
        .lock()
        .await
        .runtime
        .python_approvals
        .is_empty());
    if out["eligible"] == false {
        let m = run(
            &app,
            "r = await engine('chats.compact', chat_id='zone-compact')\n[r['ok'], r['error']]",
        )
        .await;
        let out = result(&m);
        assert_eq!(out[0], false, "{out}");
    }
}
