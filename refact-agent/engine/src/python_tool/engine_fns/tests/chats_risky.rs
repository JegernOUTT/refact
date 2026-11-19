use super::*;
use crate::chat::types::ChatSession;
use refact_chat_api::PythonEngineClass as Class;

pub(super) async fn install(app: &AppState) {
    let mut caps = crate::caps::CodeAssistantCaps::default();
    for id in ["model", "zone/model-a"] {
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

pub(super) fn message(id: &str, role: &str, text: &str) -> ChatMessage {
    let mut m = ChatMessage::new(role.into(), text.into());
    m.message_id = id.into();
    m
}

/// A saved chat with five messages: user, assistant, user, assistant, user.
pub(super) fn build(id: &str, f: impl FnOnce(&mut ChatSession)) -> ChatSession {
    let mut s = ChatSession::new(id.into());
    s.thread.title = format!("{id} title");
    s.thread.model = "zone/model-a".into();
    s.thread.mode = "explore".into();
    s.thread.root_chat_id = Some("python-test".into());
    s.messages = vec![
        message(&format!("{id}-m0"), "user", "zone first question"),
        message(&format!("{id}-m1"), "assistant", "zone first answer"),
        message(&format!("{id}-m2"), "user", "zone second question"),
        message(&format!("{id}-m3"), "assistant", "zone second answer"),
        message(&format!("{id}-m4"), "user", "zone third question"),
    ];
    s.trajectory_version = 1;
    f(&mut s);
    s
}

pub(super) async fn save(app: &AppState, s: &ChatSession) {
    crate::chat::trajectories::save_trajectory_snapshot(
        app.gcx.clone(),
        crate::chat::trajectories::trajectory_snapshot_from_session(s),
    )
    .await
    .unwrap();
}

pub(super) async fn other(
    app: &AppState,
    id: &str,
    f: impl FnOnce(&mut ChatSession),
) -> Arc<tokio::sync::Mutex<ChatSession>> {
    let s = build(id, f);
    save(app, &s).await;
    let arc = Arc::new(tokio::sync::Mutex::new(s));
    app.chat
        .sessions
        .write()
        .await
        .insert(id.into(), arc.clone());
    arc
}

/// Holds the queue processor of a chat so a queued command stays visible in `command_queue`.
pub(super) fn hold_queue(s: &mut ChatSession) {
    s.queue_processor_running
        .store(true, std::sync::atomic::Ordering::SeqCst);
}

pub(super) async fn approve(
    app: &AppState,
    code: &str,
) -> (refact_chat_api::PythonApprovalRequest, Value) {
    let task = spawn(app, code);
    let request = next_request(app, &task).await;
    decide(app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    (request, result(&finish(task).await))
}

pub(super) async fn deny(app: &AppState, code: &str) -> refact_chat_api::PythonApprovalRequest {
    let task = spawn(app, code);
    let request = next_request(app, &task).await;
    decide(app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    finish(task).await;
    request
}

/// Raises the card, lets `moved` change the target, then allows; returns `[ok, error]`.
pub(super) async fn moved_then_allowed<F>(app: &AppState, code: &str, moved: F) -> Value
where
    F: std::future::Future<Output = ()>,
{
    let task = spawn(
        app,
        &format!("try:\n    r = {code}\n    out = [r['ok'], r.get('error')]\nexcept Exception as e:\n    out = [False, type(e).__name__ + ': ' + str(e)]\nout"),
    );
    let request = next_request(app, &task).await;
    moved.await;
    decide(app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    result(&finish(task).await)
}

/// Runs a call that must be refused before any card; returns the Python error type.
pub(super) async fn refused(app: &AppState, code: &str) -> String {
    let m = run(app, code).await;
    assert!(
        session(app)
            .await
            .lock()
            .await
            .runtime
            .python_approvals
            .is_empty(),
        "a card was raised"
    );
    m.extra["python"]["error"]["type"]
        .as_str()
        .unwrap_or_else(|| panic!("not refused: {}", m.content.content_text_only()))
        .to_string()
}

fn busy_this_chat() -> impl FnOnce(&mut ChatSession) {
    |s| {
        s.runtime.state = refact_chat_api::SessionState::ExecutingTools;
        s.turn_depth = 1;
    }
}

pub(super) async fn until<F: Fn(&ChatSession) -> bool>(
    session: &Arc<tokio::sync::Mutex<ChatSession>>,
    done: F,
) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !done(&*session.lock().await) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the chat did not reach the expected state");
}

fn queued_types(s: &ChatSession) -> Vec<Value> {
    s.command_queue
        .iter()
        .map(|r| serde_json::to_value(&r.command).unwrap())
        .collect()
}

// ---------------------------------------------------------------- chat.set_auto_approve

#[tokio::test]
async fn chat_set_auto_approve_on_is_risky_queued_and_lands_after_the_turn() {
    let (_dir, app, _) = fixture().await;
    busy_this_chat()(&mut *session(&app).await.lock().await);
    let (request, out) = approve(
        &app,
        "await engine('chat.set_auto_approve', editing_tools=True, all_confirmations=True)",
    )
    .await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.class, Class::Risky);
    assert_eq!(engine.title, "change automatic approvals in this chat");
    assert_eq!(
        engine.scope.as_deref(),
        Some("this chat, from its next turn")
    );
    let warning = engine.warning.unwrap();
    assert!(warning.contains("Edits to files will be applied without asking you."));
    assert!(warning.contains("Every tool except agent control will run without asking you."));
    assert!(!warning.contains("Shell commands"));
    assert_eq!(engine.items.len(), 2);
    assert!(out["text"]
        .as_str()
        .unwrap()
        .contains("never affect engine functions"));
    let s = session(&app).await;
    {
        let s = s.lock().await;
        assert!(!s.thread.auto_approve_editing_tools && !s.thread.autonomous_no_confirm);
        let queued = queued_types(&s);
        assert_eq!(queued.len(), 1);
        assert_eq!(
            queued[0]["patch"],
            json!({"auto_approve_editing_tools":true,"autonomous_no_confirm":true})
        );
    }
    {
        let mut s = s.lock().await;
        s.runtime.state = refact_chat_api::SessionState::Idle;
        s.turn_depth = 0;
        s.queue_notify.notify_one();
    }
    until(&s, |s| {
        s.thread.auto_approve_editing_tools && s.thread.autonomous_no_confirm
    })
    .await;
    assert!(!s.lock().await.thread.auto_approve_dangerous_commands);
}

#[tokio::test]
async fn chat_set_auto_approve_off_is_a_change_and_deny_queues_nothing() {
    let (_dir, app, _) = fixture().await;
    {
        let s = session(&app).await;
        let mut s = s.lock().await;
        busy_this_chat()(&mut s);
        s.thread.auto_approve_editing_tools = true;
    }
    let request = deny(
        &app,
        "await engine('chat.set_auto_approve', editing_tools=False)",
    )
    .await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.class, Class::Change);
    assert!(engine.warning.is_none());
    assert!(session(&app).await.lock().await.command_queue.is_empty());
    let same = result(
        &run(
            &app,
            "(await engine('chat.set_auto_approve', dangerous_commands=False))['text']",
        )
        .await,
    );
    assert_eq!(same, "nothing to change");
    assert_eq!(
        refused(&app, "await engine('chat.set_auto_approve')").await,
        "TypeError"
    );
}

// ---------------------------------------------------------------- chats.set_auto_approve

#[tokio::test]
async fn chats_set_auto_approve_names_the_target_and_refuses_busy_chats() {
    let (_dir, app, _) = fixture().await;
    let target = other(&app, "zone-aa", |_| {}).await;
    let request = deny(
        &app,
        "await engine('chats.set_auto_approve', chat_id='zone-aa', dangerous_commands=True)",
    )
    .await;
    assert!(target.lock().await.command_queue.is_empty(), "deny queued");
    let engine = request.engine.unwrap();
    assert_eq!(engine.class, Class::Risky);
    assert!(engine
        .warning
        .unwrap()
        .starts_with("zone-aa title: Shell commands"));
    assert!(engine.scope.unwrap().contains("zone-aa"));
    let (_, out) = approve(
        &app,
        "await engine('chats.set_auto_approve', chat_id='zone-aa', dangerous_commands=True)",
    )
    .await;
    assert_eq!(out["ok"], true, "{out}");
    until(&target, |s| s.thread.auto_approve_dangerous_commands).await;
    let request = deny(
        &app,
        "await engine('chats.set_auto_approve', chat_id='zone-aa', dangerous_commands=False)",
    )
    .await;
    assert_eq!(request.engine.unwrap().class, Class::Change);

    other(&app, "zone-aa-busy", |s| {
        s.runtime.state = refact_chat_api::SessionState::Generating
    })
    .await;
    for (id, kind) in [
        ("zone-aa-busy", "PermissionError"),
        ("python-test", "TypeError"),
        ("zone-aa-unknown", "TypeError"),
    ] {
        assert_eq!(
            refused(
                &app,
                &format!(
                    "await engine('chats.set_auto_approve', chat_id='{id}', editing_tools=True)"
                )
            )
            .await,
            kind,
            "{id}"
        );
    }
    assert!(!app
        .chat
        .sessions
        .read()
        .await
        .contains_key("zone-aa-unknown"));
}

#[tokio::test]
async fn chats_set_auto_approve_moved_target_is_refused() {
    let (_dir, app, _) = fixture().await;
    let target = other(&app, "zone-aa-moved", |_| {}).await;
    let t = target.clone();
    let out = moved_then_allowed(
        &app,
        "await engine('chats.set_auto_approve', chat_id='zone-aa-moved', editing_tools=True)",
        async move { t.lock().await.thread.title = "zone renamed".into() },
    )
    .await;
    assert_eq!(out[0], false, "{out}");
    assert!(out[1]
        .as_str()
        .unwrap()
        .contains("changed since the preview"));
    assert!(target.lock().await.command_queue.is_empty());
    assert!(!target.lock().await.thread.auto_approve_editing_tools);
}

// ---------------------------------------------------------------- chats.clear_privacy_records

pub(super) fn record(path: &str, zone: &str) -> refact_privacy::FileRecord {
    refact_privacy::FileRecord {
        path: path.into(),
        zone: zone.into(),
        attribution: refact_privacy::Attribution::Declared,
    }
}

fn zones_of(s: &ChatSession) -> Vec<String> {
    crate::privacy::records::records_to_carry(&s.messages)
        .unwrap()
        .into_iter()
        .map(|r| format!("{}={}", r.path, r.zone))
        .collect()
}

#[tokio::test]
async fn clear_privacy_records_forgets_secret_keeps_blocked_and_refuses_busy() {
    let (_dir, app, _) = fixture().await;
    let target = other(&app, "zone-priv", |s| {
        crate::privacy::records::merge_records(
            &mut s.messages[1],
            [
                record("/zone/p.txt", "secret"),
                record("/zone/b.txt", "blocked"),
            ],
        );
    })
    .await;
    let request = deny(
        &app,
        "await engine('chats.clear_privacy_records', chat_id='zone-priv', paths=['/zone/p.txt', '/zone/b.txt'])",
    )
    .await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.class, Class::Risky);
    assert_eq!(engine.title, "forget privacy records in another chat");
    assert!(engine
        .warning
        .unwrap()
        .contains("destinations that were blocked"));
    let p = engine
        .items
        .iter()
        .find(|i| i.label == "/zone/p.txt")
        .unwrap();
    assert_eq!(p.old, Some(json!("secret")));
    assert_eq!(p.new, Some(json!("forgotten")));
    let b = engine
        .items
        .iter()
        .find(|i| i.label == "/zone/b.txt")
        .unwrap();
    assert_eq!(b.new, Some(json!("kept (blocked zone)")));
    assert_eq!(
        zones_of(&*target.lock().await).len(),
        2,
        "deny changed records"
    );

    let (_, out) = approve(
        &app,
        "(await engine('chats.clear_privacy_records', chat_id='zone-priv', paths=['/zone/p.txt', '/zone/b.txt']))['data']",
    )
    .await;
    assert_eq!(out["cleared"], 1, "{out}");
    assert_eq!(out["remaining"], 1, "{out}");
    assert_eq!(zones_of(&*target.lock().await), ["/zone/b.txt=blocked"]);

    let only_blocked = result(
        &run(
            &app,
            "(await engine('chats.clear_privacy_records', chat_id='zone-priv', paths=['/zone/b.txt']))['text']",
        )
        .await,
    );
    assert_eq!(only_blocked, "nothing to change");

    other(&app, "zone-priv-busy", |s| {
        crate::privacy::records::merge_records(
            &mut s.messages[1],
            [record("/zone/p.txt", "secret")],
        );
        s.runtime.state = refact_chat_api::SessionState::Generating;
    })
    .await;
    let unloaded = build("zone-priv-saved", |_| {});
    save(&app, &unloaded).await;
    for (id, kind) in [
        ("zone-priv-busy", "PermissionError"),
        ("zone-priv-saved", "PermissionError"),
        ("python-test", "TypeError"),
    ] {
        assert_eq!(
            refused(
                &app,
                &format!("await engine('chats.clear_privacy_records', chat_id='{id}', paths=['/zone/p.txt'])")
            )
            .await,
            kind,
            "{id}"
        );
    }
    assert!(!app
        .chat
        .sessions
        .read()
        .await
        .contains_key("zone-priv-saved"));
}

#[tokio::test]
async fn clear_privacy_records_moved_target_is_refused() {
    let (_dir, app, _) = fixture().await;
    let target = other(&app, "zone-priv-moved", |s| {
        crate::privacy::records::merge_records(
            &mut s.messages[1],
            [record("/zone/p.txt", "secret")],
        );
    })
    .await;
    let t = target.clone();
    let out = moved_then_allowed(
        &app,
        "await engine('chats.clear_privacy_records', chat_id='zone-priv-moved', paths=['/zone/p.txt'])",
        async move {
            let mut s = t.lock().await;
            crate::privacy::records::merge_records(&mut s.messages[3], [record("/zone/p.txt", "other")]);
        },
    )
    .await;
    assert_eq!(out[0], false, "{out}");
    assert!(out[1]
        .as_str()
        .unwrap()
        .contains("changed since the preview"));
    assert_eq!(zones_of(&*target.lock().await).len(), 2);
}

// ---------------------------------------------------------------- chats.retry_from / edit / remove

#[tokio::test]
async fn retry_from_cuts_at_a_user_message_and_starts_a_turn() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-retry", |s| s.thread.mode = "ask".into()).await;
    let long = format!("zone new {}", "x".repeat(400));
    let request = deny(
        &app,
        &format!("await engine('chats.retry_from', chat_id='zone-retry', index=2, text='{long}')"),
    )
    .await;
    assert_eq!(
        target.lock().await.messages.len(),
        5,
        "deny changed the chat"
    );
    assert!(target.lock().await.command_queue.is_empty());
    let engine = request.engine.unwrap();
    assert_eq!(engine.class, Class::Destructive);
    assert_eq!(engine.title, "rewrite another chat from a message");
    assert_eq!(
        engine.warning.as_deref(),
        Some("Messages after this point in that chat will be deleted and cannot be restored.")
    );
    let removed = engine
        .items
        .iter()
        .find(|i| i.label == "messages removed")
        .unwrap();
    assert_eq!(removed.old, Some(json!(3)));
    let new = engine
        .items
        .iter()
        .find(|i| i.label == "new message")
        .unwrap();
    assert_eq!(
        new.new,
        Some(json!(long)),
        "the card must show the full new text"
    );

    let (_, out) = approve(
        &app,
        "(await engine('chats.retry_from', chat_id='zone-retry', index=2, text='zone retried'))['data']",
    )
    .await;
    assert_eq!(out["messages_removed"], 3, "{out}");
    until(&target, |s| {
        s.messages
            .iter()
            .any(|m| m.content.content_text_only() == "zone retried")
    })
    .await;
    let kept: Vec<String> = target
        .lock()
        .await
        .messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| m.message_id.clone())
        .take(2)
        .collect();
    assert_eq!(kept, ["zone-retry-m0", "zone-retry-m1"]);
    assert!(target.lock().await.messages.iter().all(|m| ![
        "zone-retry-m2",
        "zone-retry-m3",
        "zone-retry-m4"
    ]
    .contains(&m.message_id.as_str())));
}

#[tokio::test]
async fn retry_from_refusals_and_moved_target() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-rr", hold_queue).await;
    other(&app, "zone-rr-busy", |s| {
        s.runtime.state = refact_chat_api::SessionState::Generating
    })
    .await;
    for (code, kind) in [
        ("chat_id='zone-rr', index=1", "PermissionError"),
        ("chat_id='zone-rr', index=9", "TypeError"),
        ("chat_id='zone-rr-busy', index=0", "PermissionError"),
        ("chat_id='python-test', index=0", "TypeError"),
        ("chat_id='zone-rr-unknown', index=0", "TypeError"),
    ] {
        assert_eq!(
            refused(
                &app,
                &format!("await engine('chats.retry_from', {code}, text='zone x')")
            )
            .await,
            kind,
            "{code}"
        );
    }
    let t = target.clone();
    let out = moved_then_allowed(
        &app,
        "await engine('chats.retry_from', chat_id='zone-rr', index=2, text='zone x')",
        async move {
            let mut s = t.lock().await;
            s.messages[2].content =
                refact_core::chat_types::ChatContent::SimpleText("zone edited meanwhile".into());
        },
    )
    .await;
    assert_eq!(out[0], false, "{out}");
    assert!(out[1]
        .as_str()
        .unwrap()
        .contains("changed since the preview"));
    assert!(target.lock().await.command_queue.is_empty());
}

pub(super) fn guarded_policy(app: &AppState) -> refact_privacy::PrivacyPolicy {
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.zones = vec![refact_privacy::Zone {
        name: "zone_a".into(),
        patterns: vec!["zone_a.txt".into()],
        send_to: vec!["model".into()],
        on_shell_read: refact_privacy::ShellBehavior::Withhold,
    }];
    policy
}

#[tokio::test]
async fn retry_from_and_edit_message_refuse_text_that_may_not_reach_the_target_model() {
    let (dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-egr", hold_queue).await;
    crate::privacy::save_privacy_policy(app.gcx.clone(), guarded_policy(&app))
        .await
        .unwrap();
    std::fs::write(dir.path().join("project/zone_a.txt"), "guarded").unwrap();
    for call in [
        "await engine('chats.retry_from', chat_id='zone-egr', index=2, text='zone ' + x)",
        "await engine('chats.edit_message', chat_id='zone-egr', message_id='zone-egr-m2', text='zone ' + x)",
    ] {
        assert_eq!(
            refused(&app, &format!("x = open('zone_a.txt').read()\n{call}")).await,
            "PermissionError",
            "{call}"
        );
    }
    let s = target.lock().await;
    assert!(s.command_queue.is_empty());
    assert_eq!(
        s.messages[2].content.content_text_only(),
        "zone second question"
    );
}

#[tokio::test]
async fn edit_message_carries_run_records_and_refuses_non_user_and_unknown_messages() {
    let (dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-edit", |s| s.thread.mode = "ask".into()).await;
    let mut policy = guarded_policy(&app);
    policy.zones[0].send_to = vec!["*".into()];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
    std::fs::write(dir.path().join("project/zone_a.txt"), "guarded").unwrap();
    let long = format!("zone edited {}", "y".repeat(400));
    let request = deny(
        &app,
        &format!("await engine('chats.edit_message', chat_id='zone-edit', message_id='zone-edit-m2', text='{long}')"),
    )
    .await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.class, Class::Destructive);
    assert_eq!(
        engine.warning.as_deref(),
        Some("The original text will be overwritten and cannot be restored.")
    );
    assert_eq!(engine.items[0].old, Some(json!("zone second question")));
    assert_eq!(engine.items[0].new, Some(json!(long)), "full new text");
    assert_eq!(
        target.lock().await.messages[2].content.content_text_only(),
        "zone second question"
    );
    let request = deny(
        &app,
        "await engine('chats.edit_message', chat_id='zone-edit', message_id='zone-edit-m2', text='zone z', regenerate=True)",
    )
    .await;
    let engine = request.engine.unwrap();
    assert!(engine
        .warning
        .unwrap()
        .ends_with("Later messages will be deleted."));
    assert!(engine
        .items
        .iter()
        .any(|i| i.label == "messages removed" && i.old == Some(json!(2))));

    let (_, out) = approve(
        &app,
        "x = open('zone_a.txt').read()\n(await engine('chats.edit_message', chat_id='zone-edit', message_id='zone-edit-m2', text='zone edited'))['data']",
    )
    .await;
    assert_eq!(out["messages_removed"], 0, "{out}");
    until(&target, |s| {
        s.messages[2].content.content_text_only() == "zone edited"
    })
    .await;
    let s = target.lock().await;
    assert_eq!(s.messages.len(), 5);
    assert!(
        s.messages[2]
            .extra
            .get("privacy")
            .is_some_and(|p| p.to_string().contains("zone_a")),
        "the run's records did not travel with the text: {:?}",
        s.messages[2].extra
    );
    drop(s);
    assert_eq!(
        refused(
            &app,
            "x = open('zone_a.txt').read()\nawait engine('chats.retry_from', chat_id='zone-edit', index=0, text='zone q')"
        )
        .await,
        "PermissionError",
        "a retry from the first message has no message to carry the run's records"
    );
    let m = run(
        &app,
        "x = open('zone_a.txt').read()\ntry:\n    await engine('chats.retry_from', chat_id='zone-edit', index=2, text='zone q')\n    r = 'asked'\nexcept PermissionError as e:\n    r = str(e)\nr",
    )
    .await;
    let text = result(&m).as_str().unwrap().to_string();
    assert!(text.contains("privacy records"), "{text}");
    let s = target.lock().await;
    assert_eq!(s.messages.len(), 5, "a refused retry changed the chat");
    assert!(s.messages[1].extra.get("privacy").is_none());
    assert!(s.command_queue.is_empty());
    drop(s);
    for (code, kind) in [
        ("message_id='zone-edit-m1'", "PermissionError"),
        ("message_id='zone-edit-nope'", "TypeError"),
    ] {
        assert_eq!(
            refused(
                &app,
                &format!("await engine('chats.edit_message', chat_id='zone-edit', {code}, text='zone q')")
            )
            .await,
            kind,
            "{code}"
        );
    }
}

#[tokio::test]
async fn edit_message_regenerate_on_the_last_message_deletes_nothing() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-last", hold_queue).await;
    let same = result(
        &run(
            &app,
            "(await engine('chats.edit_message', chat_id='zone-last', message_id='zone-last-m4', text='zone third question', regenerate=True))['text']",
        )
        .await,
    );
    assert_eq!(same, "nothing to change");
    let request = deny(
        &app,
        "await engine('chats.edit_message', chat_id='zone-last', message_id='zone-last-m4', text='zone other', regenerate=True)",
    )
    .await;
    let engine = request.engine.unwrap();
    assert!(!engine.warning.unwrap().contains("Later messages"));
    assert!(engine.items.iter().all(|i| i.label != "messages removed"));
    assert!(target.lock().await.command_queue.is_empty());
}

#[tokio::test]
async fn edit_message_moved_target_is_refused() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-edm", hold_queue).await;
    let t = target.clone();
    let out = moved_then_allowed(
        &app,
        "await engine('chats.edit_message', chat_id='zone-edm', message_id='zone-edm-m2', text='zone q')",
        async move {
            let mut s = t.lock().await;
            let m = message("zone-edm-m5", "assistant", "zone late answer");
            s.messages.push(m);
        },
    )
    .await;
    assert_eq!(out[0], false, "{out}");
    assert!(out[1]
        .as_str()
        .unwrap()
        .contains("changed since the preview"));
    assert!(target.lock().await.command_queue.is_empty());
}

#[tokio::test]
async fn remove_message_deletes_one_user_message_only() {
    let (_dir, app, _) = fixture().await;
    let target = other(&app, "zone-rm", |_| {}).await;
    let request = deny(
        &app,
        "await engine('chats.remove_message', chat_id='zone-rm', message_id='zone-rm-m4')",
    )
    .await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.class, Class::Destructive);
    assert_eq!(engine.title, "delete a message from another chat");
    assert_eq!(
        engine.warning.as_deref(),
        Some("The message will be deleted and cannot be restored.")
    );
    assert_eq!(engine.items[0].old, Some(json!("zone third question")));
    assert_eq!(target.lock().await.messages.len(), 5);
    approve(
        &app,
        "await engine('chats.remove_message', chat_id='zone-rm', message_id='zone-rm-m4')",
    )
    .await;
    until(&target, |s| s.messages.len() == 4).await;
    assert!(target
        .lock()
        .await
        .messages
        .iter()
        .all(|m| m.message_id != "zone-rm-m4"));
    for (code, kind) in [
        ("message_id='zone-rm-m3'", "PermissionError"),
        ("message_id='zone-rm-nope'", "TypeError"),
    ] {
        assert_eq!(
            refused(
                &app,
                &format!("await engine('chats.remove_message', chat_id='zone-rm', {code})")
            )
            .await,
            kind,
            "{code}"
        );
    }
    let t = target.clone();
    hold_queue(&mut *target.lock().await);
    let out = moved_then_allowed(
        &app,
        "await engine('chats.remove_message', chat_id='zone-rm', message_id='zone-rm-m2')",
        async move { t.lock().await.runtime.state = refact_chat_api::SessionState::Generating },
    )
    .await;
    assert_eq!(out[0], false, "{out}");
    assert!(target.lock().await.command_queue.is_empty());
}

// ---------------------------------------------------------------- chats.delete

pub(super) async fn exists(app: &AppState, id: &str) -> bool {
    crate::chat::trajectories::find_trajectory_path(app.gcx.clone(), id)
        .await
        .is_some()
}

#[tokio::test]
async fn delete_removes_a_saved_unloaded_chat() {
    let (_dir, app, _) = fixture().await;
    save(&app, &build("zone-del", |_| {})).await;
    deny(&app, "await engine('chats.delete', chat_id='zone-del')").await;
    assert!(exists(&app, "zone-del").await, "deny deleted the chat");
    let (request, out) = approve(
        &app,
        "(await engine('chats.delete', chat_id='zone-del'))['data']",
    )
    .await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.class, Class::Destructive);
    assert_eq!(engine.title, "delete another chat");
    assert_eq!(
        engine.warning.as_deref(),
        Some("This chat and its Python workspace will be deleted and cannot be restored.")
    );
    let item = engine.items[0]
        .old
        .as_ref()
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        item.contains("zone-del title") && item.contains("5 messages"),
        "{item}"
    );
    assert_eq!(out, json!({"chat_id":"zone-del","deleted":true}));
    assert!(!exists(&app, "zone-del").await);
    let listed = crate::chat::trajectories::list_all_trajectories_meta(app.clone(), false)
        .await
        .unwrap();
    assert!(
        listed.iter().all(|m| m.id != "zone-del"),
        "index entry kept"
    );
}

/// A loaded idle chat is now deleted after it is closed in the engine (K4); one that a viewer
/// still holds stays refused (D9), which is the loaded case this test keeps.
#[tokio::test]
async fn delete_refuses_this_loaded_parent_and_task_chats() {
    let (_dir, app, _) = fixture().await;
    let _viewer = other(&app, "zone-del-loaded", |_| {}).await;
    save(&app, &build("zone-del-parent", |_| {})).await;
    save(
        &app,
        &build("zone-del-child", |s| {
            s.thread.parent_id = Some("zone-del-parent".into())
        }),
    )
    .await;
    for (id, kind) in [
        ("python-test", "TypeError"),
        ("zone-del-loaded", "PermissionError"),
        ("zone-del-parent", "PermissionError"),
        ("zone-del-unknown", "TypeError"),
    ] {
        assert_eq!(
            refused(
                &app,
                &format!("await engine('chats.delete', chat_id='{id}')")
            )
            .await,
            kind,
            "{id}"
        );
    }
    assert!(exists(&app, "zone-del-loaded").await);
    assert!(exists(&app, "zone-del-parent").await);
}

#[tokio::test]
async fn delete_refuses_a_task_bound_chat() {
    let (_dir, app, _) = fixture().await;
    save(&app, &build("zone-del-task", |_| {})).await;
    let path = crate::chat::trajectories::find_trajectory_path(app.gcx.clone(), "zone-del-task")
        .await
        .unwrap();
    let mut json: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    json["task_meta"] = json!({"task_id":"zone-task","role":"planner"});
    std::fs::write(&path, serde_json::to_string(&json).unwrap()).unwrap();
    assert_eq!(
        refused(
            &app,
            "await engine('chats.delete', chat_id='zone-del-task')"
        )
        .await,
        "PermissionError"
    );
    assert!(exists(&app, "zone-del-task").await);
}

#[tokio::test]
async fn delete_moved_target_is_refused() {
    let (_dir, app, _) = fixture().await;
    save(&app, &build("zone-del-moved", |_| {})).await;
    let app2 = app.clone();
    let out = moved_then_allowed(
        &app,
        "await engine('chats.delete', chat_id='zone-del-moved')",
        async move {
            let mut s = build("zone-del-moved", |_| {});
            s.messages
                .push(message("zone-del-moved-m5", "assistant", "zone more"));
            s.trajectory_version = 2;
            save(&app2, &s).await;
        },
    )
    .await;
    assert_eq!(out[0], false, "{out}");
    assert!(out[1]
        .as_str()
        .unwrap()
        .contains("changed since the preview"));
    assert!(exists(&app, "zone-del-moved").await);
}

// ---------------------------------------------------------------- review fixes of the ordinary changes

/// Caps for this chat's model with reasoning options, and this chat in the middle of its turn.
async fn busy_with_caps(app: &AppState) {
    install(app).await;
    {
        let mut state = app.gcx.caps_state.write().await;
        let mut caps = (**state.caps.as_ref().unwrap()).clone();
        let mut record = (*caps.chat_models["zone/model-a"]).clone();
        record.reasoning_effort_options = Some(vec!["low".into(), "high".into()]);
        record.max_output_tokens = Some(8000);
        record.supports_temperature = true;
        caps.chat_models
            .insert("zone/model-a".into(), Arc::new(record));
        state.caps = Some(Arc::new(caps));
    }
    let s = session(app).await;
    let mut s = s.lock().await;
    s.thread.model = "zone/model-a".into();
    s.thread.mode = "explore".into();
    busy_this_chat()(&mut s);
}

fn this_chat_fingerprint(s: &ChatSession) -> Value {
    json!({
        "thread": serde_json::to_value(&s.thread).unwrap(),
        "queue": s.command_queue.len(),
        "goal": serde_json::to_value(&s.goal).unwrap(),
        "status": serde_json::to_value(s.goal_status).unwrap(),
        "recovery": serde_json::to_value(&s.runtime.recovery).unwrap(),
    })
}

#[tokio::test]
async fn deny_leaves_this_chat_unchanged_for_every_queued_setter() {
    let (_dir, app, _) = fixture().await;
    busy_with_caps(&app).await;
    session(&app)
        .await
        .lock()
        .await
        .install_goal("agent", "zone goal", true, Default::default());
    for call in [
        "await engine('chat.set_mode', mode='agent')",
        "await engine('chat.set_reasoning', reasoning_effort='high')",
        "await engine('chat.set_sampling', temperature=0.3)",
        "await engine('chat.set_context', context_tokens_cap=8000)",
        "await engine('chat.set_flags', parallel_tool_calls=False)",
        "await engine('chat.set_recovery', max_retries=7)",
        "await engine('chat.set_goal_budget', cooldown_ms=4321)",
        "await engine('chat.stop_goal')",
        "await engine('chat.set_auto_approve', dangerous_commands=True)",
    ] {
        let before = this_chat_fingerprint(&*session(&app).await.lock().await);
        let request = deny(&app, call).await;
        assert!(request.engine.is_some(), "{call}: no card");
        let after = this_chat_fingerprint(&*session(&app).await.lock().await);
        assert_eq!(before, after, "{call}: a denied card changed the chat");
    }
}

#[tokio::test]
async fn set_goal_budget_refuses_supervised_goals_and_goals_with_hard_limits() {
    let (_dir, app, _) = fixture().await;
    busy_with_caps(&app).await;
    for budget in [
        refact_chat_api::GoalBudget {
            max_turns: Some(3),
            ..Default::default()
        },
        refact_chat_api::GoalBudget {
            supervised: true,
            ..Default::default()
        },
    ] {
        {
            let s = session(&app).await;
            let mut s = s.lock().await;
            s.install_goal("agent", "zone goal", true, budget.clone());
            s.goal.as_mut().unwrap().budget = budget.clone();
        }
        assert_eq!(
            refused(
                &app,
                "await engine('chat.set_goal_budget', cooldown_ms=1000)"
            )
            .await,
            "PermissionError",
            "{budget:?}"
        );
        let s = session(&app).await;
        let s = s.lock().await;
        assert!(s.command_queue.is_empty());
        assert_eq!(s.goal.as_ref().unwrap().budget, budget);
    }
}

#[tokio::test]
async fn set_mode_card_is_truthful_about_the_prompt_and_the_next_request_uses_the_new_tools() {
    let (_dir, app, _) = fixture().await;
    busy_with_caps(&app).await;
    let (request, out) = approve(&app, "await engine('chat.set_mode', mode='ask')").await;
    let engine = request.engine.unwrap();
    let prompt = engine
        .items
        .iter()
        .find(|i| i.label == "system prompt")
        .unwrap();
    assert!(
        prompt
            .new
            .as_ref()
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with("kept"),
        "{prompt:?}"
    );
    assert_eq!(
        engine.class,
        Class::Risky,
        "ask has tool_confirm rules with auto"
    );
    let text = out["text"].as_str().unwrap();
    assert!(text.contains("system prompt stays"), "{text}");
    assert!(
        text.contains("validated against this chat as it is now"),
        "{text}"
    );
    let s = session(&app).await;
    {
        let mut s = s.lock().await;
        s.runtime.state = refact_chat_api::SessionState::Idle;
        s.turn_depth = 0;
        s.queue_notify.notify_one();
    }
    until(&s, |s| s.thread.mode == "ask" && s.command_queue.is_empty()).await;
    let (mode, catalog) = {
        let s = s.lock().await;
        (s.thread.mode.clone(), s.tool_catalog.is_some())
    };
    assert!(!catalog, "the old mode's catalog survived the switch");
    let names = |tools: Vec<Box<dyn Tool + Send>>| {
        let mut n: Vec<String> = tools.iter().map(|t| t.tool_description().name).collect();
        n.sort();
        n
    };
    let next = names(
        crate::tools::tools_list::get_tools_for_mode(app.gcx.clone(), &mode, Some("zone/model-a"))
            .await,
    );
    let ask = names(
        crate::tools::tools_list::get_tools_for_mode(app.gcx.clone(), "ask", Some("zone/model-a"))
            .await,
    );
    let explore = names(
        crate::tools::tools_list::get_tools_for_mode(
            app.gcx.clone(),
            "explore",
            Some("zone/model-a"),
        )
        .await,
    );
    assert_ne!(
        ask, explore,
        "the two modes must differ for this test to mean anything"
    );
    assert_eq!(next, ask);
}

#[tokio::test]
async fn set_recovery_turning_retries_on_from_off_is_risky() {
    let (_dir, app, _) = fixture().await;
    busy_with_caps(&app).await;
    session(&app).await.lock().await.thread.recovery_policy =
        Some(refact_core::recovery::RecoveryPolicyOverrides {
            mode: Some(refact_core::recovery::RecoveryMode::Off),
            ..Default::default()
        });
    let request = deny(&app, "await engine('chat.set_recovery', mode='transient')").await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.class, Class::Risky);
    assert!(engine.warning.unwrap().contains("retried more"));
}

#[tokio::test]
async fn set_mode_accepts_only_known_legacy_names() {
    let (_dir, app, _) = fixture().await;
    busy_with_caps(&app).await;
    assert_eq!(
        refused(&app, "await engine('chat.set_mode', mode='Explore2')").await,
        "TypeError"
    );
    let same = result(
        &run(
            &app,
            "(await engine('chat.set_mode', mode='EXPLORE'))['text']",
        )
        .await,
    );
    assert_eq!(same, "nothing to change");
}

#[tokio::test]
async fn stop_goal_leaves_the_recovery_alone_when_nothing_was_queued() {
    let (_dir, app, _) = fixture().await;
    busy_with_caps(&app).await;
    {
        let s = session(&app).await;
        let mut s = s.lock().await;
        s.install_goal("agent", "zone goal", true, Default::default());
        s.runtime.recovery = Some(refact_core::recovery::RecoveryRecord {
            operation_id: "zone-op".into(),
            run_id: "zone-run".into(),
            revision: 1,
            state: refact_core::recovery::RecoveryState::Waiting,
            policy: Default::default(),
            policy_source: refact_core::recovery::RecoveryPolicySource::Global,
            manual: false,
            retries_used: 0,
            latest_error: refact_core::recovery::RecoveryError {
                category: "zone".into(),
                title: "zone".into(),
                explanation: "zone".into(),
                suggested_action: "zone".into(),
                is_retryable: true,
                raw_error: "zone".into(),
            },
            next_retry_at_ms: None,
            deadline_at_ms: None,
            retry_not_before_ms: None,
            terminal_reason: None,
            attempts: Vec::new(),
        });
        s.test_queue_capacity = Some(0);
    }
    let (_, out) = approve(
        &app,
        "r = await engine('chat.stop_goal')\n[r['ok'], r.get('error')]",
    )
    .await;
    assert_eq!(out[0], false, "{out}");
    let s = session(&app).await;
    let s = s.lock().await;
    assert_eq!(
        s.runtime.recovery.as_ref().unwrap().state,
        refact_core::recovery::RecoveryState::Waiting,
        "the recovery was cancelled although nothing was queued"
    );
}

pub(super) fn other_fingerprint(s: &ChatSession) -> Value {
    json!({
        "thread": serde_json::to_value(&s.thread).unwrap(),
        "queue": s.command_queue.len(),
        "deliveries": s.pending_deliveries.len(),
        "messages": s.messages.len(),
        "version": s.trajectory_version,
        "abort": s.abort_flag.load(std::sync::atomic::Ordering::SeqCst),
    })
}

#[tokio::test]
async fn deny_leaves_other_chats_unchanged_for_every_ordinary_change() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let idle = other(&app, "zone-deny", hold_queue).await;
    let busy = other(&app, "zone-deny-busy", |s| {
        s.runtime.state = refact_chat_api::SessionState::Generating
    })
    .await;
    for (target, call) in [
        (
            &idle,
            "await engine('chats.set_params', chat_id='zone-deny', temperature=0.4)",
        ),
        (
            &idle,
            "await engine('chats.send', chat_id='zone-deny', text='zone hi')",
        ),
        (
            &busy,
            "await engine('chats.abort', chat_id='zone-deny-busy')",
        ),
        (
            &idle,
            "await engine('chats.fork', chat_id='zone-deny', up_to_message_id='zone-deny-m2')",
        ),
        (&idle, "await engine('chats.compact', chat_id='zone-deny')"),
    ] {
        let before = other_fingerprint(&*target.lock().await);
        let sessions = app.chat.sessions.read().await.len();
        let request = deny(&app, call).await;
        assert!(request.engine.is_some(), "{call}: no card");
        assert_eq!(before, other_fingerprint(&*target.lock().await), "{call}");
        assert_eq!(
            sessions,
            app.chat.sessions.read().await.len(),
            "{call}: a chat was created"
        );
    }
}

#[tokio::test]
async fn ordinary_changes_of_a_moved_chat_are_refused() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    type Move = fn(&mut ChatSession);
    let cases: [(&str, &str, Move, Move); 5] = [
        ("zone-mv-params", "await engine('chats.set_params', chat_id='zone-mv-params', temperature=0.4)", |_| {}, |s| s.thread.title = "zone moved".into()),
        ("zone-mv-send", "await engine('chats.send', chat_id='zone-mv-send', text='zone hi')", |_| {}, |s| s.thread.mode = "agent".into()),
        ("zone-mv-abort", "await engine('chats.abort', chat_id='zone-mv-abort')", |s| s.runtime.state = refact_chat_api::SessionState::Generating, |s| s.runtime.state = refact_chat_api::SessionState::ExecutingTools),
        ("zone-mv-fork", "await engine('chats.fork', chat_id='zone-mv-fork', up_to_message_id='zone-mv-fork-m2')", |_| {}, |s| s.trajectory_version += 1),
        ("zone-mv-compact", "await engine('chats.compact', chat_id='zone-mv-compact')", |_| {}, |s| s.trajectory_version += 1),
    ];
    for (id, call, setup, moved) in cases {
        let target = other(&app, id, |s| {
            hold_queue(s);
            setup(s)
        })
        .await;
        let t = target.clone();
        let sessions = app.chat.sessions.read().await.len();
        let out = moved_then_allowed(&app, call, async move { moved(&mut *t.lock().await) }).await;
        assert_eq!(out[0], false, "{id}: {out}");
        assert!(
            out[1]
                .as_str()
                .unwrap()
                .contains("changed since the preview"),
            "{id}: {out}"
        );
        let s = target.lock().await;
        assert!(
            s.command_queue.is_empty() && s.pending_deliveries.is_empty(),
            "{id}"
        );
        assert!(
            !s.abort_flag.load(std::sync::atomic::Ordering::SeqCst),
            "{id}"
        );
        assert_eq!(
            sessions,
            app.chat.sessions.read().await.len(),
            "{id}: a chat was created"
        );
    }
}

#[tokio::test]
async fn fork_reports_a_refused_branch_quickly_and_removes_the_empty_chat() {
    let (_dir, app, _) = fixture().await;
    let source = other(&app, "zone-fk", |_| {}).await;
    let _held = crate::chat::worktree_lifecycle::transition_lock(&source).await;
    let sessions: Vec<String> = app.chat.sessions.read().await.keys().cloned().collect();
    let started = std::time::Instant::now();
    let (_, out) = approve(
        &app,
        "r = await engine('chats.fork', chat_id='zone-fk', up_to_message_id='zone-fk-m2')\n[r['ok'], r.get('error')]",
    )
    .await;
    assert_eq!(out[0], false, "{out}");
    let error = out[1].as_str().unwrap();
    assert!(
        error.contains("refused") && error.contains("transition"),
        "{error}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "waited {:?}",
        started.elapsed()
    );
    let mut after: Vec<String> = app.chat.sessions.read().await.keys().cloned().collect();
    let mut before = sessions;
    before.sort();
    after.sort();
    assert_eq!(before, after, "the empty fork stayed registered");
}

#[tokio::test]
async fn fork_card_counts_only_the_messages_the_branch_copies() {
    let (_dir, app, _) = fixture().await;
    other(&app, "zone-fc", |s| {
        s.messages
            .insert(1, message("zone-fc-ev", "event", "zone event"));
    })
    .await;
    let request = deny(
        &app,
        "await engine('chats.fork', chat_id='zone-fc', up_to_message_id='zone-fc-m2')",
    )
    .await;
    let engine = request.engine.unwrap();
    let copied = engine
        .items
        .iter()
        .find(|i| i.label == "copied messages")
        .unwrap();
    assert_eq!(
        copied.new,
        Some(json!(3)),
        "the event message is not copied"
    );
}

#[tokio::test]
async fn compact_preview_does_not_open_a_saved_chat() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    save(&app, &build("zone-cp-saved", |_| {})).await;
    let out = result(
        &run(
            &app,
            "(await engine('chats.compact_preview', chat_id='zone-cp-saved'))['data']",
        )
        .await,
    );
    assert_eq!(out["source_messages"], 5, "{out}");
    assert!(
        !app.chat.sessions.read().await.contains_key("zone-cp-saved"),
        "a read opened the chat"
    );
}

#[tokio::test]
async fn send_card_shows_the_full_text() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    other(&app, "zone-sendfull", hold_queue).await;
    let long = format!("zone {}", "z".repeat(500));
    let request = deny(
        &app,
        &format!("await engine('chats.send', chat_id='zone-sendfull', text='{long}')"),
    )
    .await;
    let engine = request.engine.unwrap();
    let message = engine.items.iter().find(|i| i.label == "message").unwrap();
    assert_eq!(message.new, Some(json!(long)));
}
