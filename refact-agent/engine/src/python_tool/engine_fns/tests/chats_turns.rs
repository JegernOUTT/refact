use super::chats_risky::{
    approve, build, deny, hold_queue, install, message, moved_then_allowed, other,
    other_fingerprint, refused, until,
};
use super::*;
use crate::chat::types::ChatSession;
use refact_chat_api::PythonEngineClass as Class;

fn queued_commands(s: &ChatSession) -> Vec<Value> {
    s.command_queue
        .iter()
        .map(|r| serde_json::to_value(&r.command).unwrap())
        .collect()
}

#[tokio::test]
async fn regenerate_queues_a_turn_and_its_card_says_what_that_chat_may_do() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-rg", |s| {
        hold_queue(s);
        s.thread.mode = "ask".into();
    })
    .await;
    let request = deny(&app, "await engine('chats.regenerate', chat_id='zone-rg')").await;
    assert!(target.lock().await.command_queue.is_empty(), "deny queued");
    let card = request.engine.unwrap();
    assert_eq!(card.title, "start a new answer in another chat");
    let last = card
        .items
        .iter()
        .find(|i| i.label == "last message")
        .unwrap();
    assert_eq!(last.new, Some(json!("#4 user, 19 characters")));
    assert!(card.items.iter().any(|i| i.label == "approval flags"));
    assert!(card.items.iter().any(|i| i.label == "goal"));
    assert_eq!(
        card.class,
        Class::Change,
        "auto rules for tools that change nothing do not make it risky"
    );
    let (_, out) = approve(
        &app,
        "(await engine('chats.regenerate', chat_id='zone-rg'))['data']",
    )
    .await;
    assert_eq!(out["opened"], false, "{out}");
    let s = target.lock().await;
    let queued = queued_commands(&s);
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0]["type"], "regenerate");
    assert!(s.command_queue[0].client_request_id.starts_with("engine-"));
}

#[tokio::test]
async fn regenerate_refuses_a_finished_answer_busy_chats_and_held_goals() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    other(&app, "zone-rg-done", |s| {
        hold_queue(s);
        s.messages
            .push(message("zone-rg-done-m5", "assistant", "zone done"));
    })
    .await;
    other(&app, "zone-rg-busy", |s| {
        s.runtime.state = refact_chat_api::SessionState::Generating
    })
    .await;
    let held = other(&app, "zone-rg-held", hold_queue).await;
    held.lock().await.goal_stopped_by_abort = true;
    other(&app, "zone-rg-rec", |s| {
        hold_queue(s);
        s.runtime.recovery = Some(serde_json::from_value(json!({
            "operation_id":"o","run_id":"r","revision":1,"state":"succeeded",
            "policy":{"mode":"transient","max_retries":1,"delay_ms":0,"total_timeout_ms":1000},
            "policy_source":"global","manual":false,"retries_used":0,
            "latest_error":{"category":"c","title":"t","explanation":"e","suggested_action":"s","is_retryable":true,"raw_error":"r"},
            "next_retry_at_ms":null,"deadline_at_ms":null,"terminal_reason":null,"attempts":[]
        })).unwrap())
    })
    .await;
    for (id, kind) in [
        ("zone-rg-done", "TypeError"),
        ("zone-rg-busy", "PermissionError"),
        ("zone-rg-held", "PermissionError"),
        ("zone-rg-rec", "PermissionError"),
        ("python-test", "TypeError"),
    ] {
        assert_eq!(
            refused(
                &app,
                &format!("await engine('chats.regenerate', chat_id='{id}')")
            )
            .await,
            kind,
            "{id}"
        );
    }
    assert!(held.lock().await.command_queue.is_empty());
}

#[tokio::test]
async fn regenerate_is_risky_for_a_chat_that_runs_tools_without_asking_and_refuses_a_moved_one() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    other(&app, "zone-rg-auto", |s| {
        hold_queue(s);
        s.thread.auto_approve_dangerous_commands = true;
    })
    .await;
    let card = deny(
        &app,
        "await engine('chats.regenerate', chat_id='zone-rg-auto')",
    )
    .await
    .engine
    .unwrap();
    assert_eq!(card.class, Class::Risky);
    assert!(card.warning.unwrap().contains("without asking"));
    let target = other(&app, "zone-rg-mv", hold_queue).await;
    let t = target.clone();
    let out = moved_then_allowed(
        &app,
        "await engine('chats.regenerate', chat_id='zone-rg-mv')",
        async move {
            t.lock()
                .await
                .messages
                .push(message("zone-rg-mv-m5", "user", "zone more"));
        },
    )
    .await;
    assert_eq!(out[0], false, "{out}");
    assert!(target.lock().await.command_queue.is_empty());
}

#[tokio::test]
async fn send_reports_delivered_or_queued_truthfully_and_describes_a_preempt() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let idle = other(&app, "zone-sd", |_| {}).await;
    let (_, out) = approve(
        &app,
        "(await engine('chats.send', chat_id='zone-sd', text='zone hello'))['data']",
    )
    .await;
    assert_eq!(out["outcome"], "delivered", "{out}");
    assert_eq!(out["queued"], false, "{out}");
    until(&idle, |s| {
        s.messages
            .iter()
            .any(|m| m.content.content_text_only() == "zone hello")
    })
    .await;
    let sent = idle
        .lock()
        .await
        .messages
        .iter()
        .find(|m| m.content.content_text_only() == "zone hello")
        .unwrap()
        .clone();
    assert_eq!(sent.extra["delivery"]["source"], "engine.chats.send");

    let busy = other(&app, "zone-sd-busy", |s| {
        s.runtime.state = refact_chat_api::SessionState::Generating;
        s.turn_depth = 1;
    })
    .await;
    let card = deny(
        &app,
        "await engine('chats.send', chat_id='zone-sd-busy', text='zone stop', push='preempt')",
    )
    .await
    .engine
    .unwrap();
    assert!(
        card.items.iter().any(|i| i.label == "running turn"),
        "a preempt card says what it interrupts: {:?}",
        card.items
    );
    let (_, out) = approve(
        &app,
        "(await engine('chats.send', chat_id='zone-sd-busy', text='zone later'))['data']",
    )
    .await;
    assert_eq!(out["outcome"], "queued", "{out}");
    assert!(busy.lock().await.pending_deliveries.len() == 1);
    assert_eq!(
        refused(
            &app,
            "await engine('chats.send', chat_id='zone-sd', text='zone x', push='soon')"
        )
        .await,
        "TypeError"
    );
}

#[tokio::test]
async fn a_refused_admission_delivers_nothing_and_leaves_nothing_to_retry() {
    let (_dir, app, _) = fixture().await;
    let target = other(&app, "zone-adm", |_| {}).await;
    let before = other_fingerprint(&*target.lock().await);
    let delivery = refact_chat_api::PendingDelivery::new(
        vec![ChatMessage::new("user".into(), "zone refused".into())],
        refact_chat_api::PushMode::WhenIdle,
        "engine.chats.send",
        true,
    );
    let id = delivery.id.clone();
    let refusal = |_: &ChatSession| -> Result<(), String> { Err("zone pins moved".into()) };
    let outcome = crate::chat::delivery::deliver_to_chat_admitted(
        app.clone(),
        "zone-adm",
        delivery,
        &refusal,
    )
    .await;
    assert_eq!(outcome.err().as_deref(), Some("zone pins moved"));
    assert_eq!(before, other_fingerprint(&*target.lock().await));
    let source_root = crate::coordination::scope::session_source_root(&*target.lock().await);
    let project = crate::coordination::outbox::project_key_for_root(&app, source_root).await;
    if let Some(outbox) = crate::coordination::outbox::outbox_for(&app, project).await {
        let state = outbox.status_of(&id).await.unwrap();
        assert!(
            state.is_none() || state.is_some_and(|state| state.is_terminal()),
            "a refused delivery stays retryable: {state:?}"
        );
    }
}

#[tokio::test]
async fn chat_send_lands_after_this_turn_as_literal_script_text() {
    let (_dir, app, _) = fixture().await;
    {
        let s = session(&app).await;
        let mut s = s.lock().await;
        s.runtime.state = refact_chat_api::SessionState::ExecutingTools;
        s.turn_depth = 1;
    }
    let (_, out) = approve(
        &app,
        "(await engine('chat.send', text='zone follow up'))['data']",
    )
    .await;
    assert_eq!(out["outcome"], "queued", "{out}");
    let s = session(&app).await;
    let s = s.lock().await;
    let delivery = s.pending_deliveries.back().unwrap();
    assert_eq!(delivery.source, "engine.chat.send");
    assert_eq!(delivery.push, refact_chat_api::PushMode::WhenIdle);
}

#[tokio::test]
async fn queue_lists_commands_as_metadata_and_deliveries_with_their_records() {
    let (_dir, app, _) = fixture().await;
    let delivery = refact_chat_api::PendingDelivery::new(
        vec![{
            let mut m = ChatMessage::new("user".into(), "zone delivered text".into());
            crate::privacy::records::attach_record(
                &mut m,
                super::chats_risky::record("/x/zone_secret.txt", "zone_secret"),
            );
            m
        }],
        refact_chat_api::PushMode::WhenIdle,
        "scheduler.cron",
        true,
    );
    other(&app, "zone-q", |s| {
        hold_queue(s);
        s.command_queue.push_back(
            serde_json::from_value(json!({"client_request_id":"zone-q-cmd","type":"user_message","content":"zone queued secret words"}))
                .unwrap(),
        );
        s.pending_deliveries.push_back(delivery);
    })
    .await;
    let m = run(
        &app,
        "(await engine('chats.queue', chat_id='zone-q'))['data']",
    )
    .await;
    let out = result(&m);
    let rows = out["items"].as_array().unwrap();
    let command = rows.iter().find(|r| r["id"] == "zone-q-cmd").unwrap();
    assert!(command.get("preview").is_none(), "{command}");
    assert_eq!(command["chars"], 24);
    let delivered = rows.iter().find(|r| r["kind"] == "delivery").unwrap();
    assert!(delivered["preview"]
        .as_str()
        .unwrap()
        .contains("zone delivered text"));
    assert!(
        m.extra
            .get("privacy")
            .is_some_and(|p| p.to_string().contains("zone_secret")),
        "{:?}",
        m.extra
    );
    assert!(!serde_json::to_string(&m)
        .unwrap()
        .contains("zone queued secret words"));
    other(&app, "zone-q-runner", |s| {
        s.runner_lease = Some((
            "lease".into(),
            Arc::new(std::sync::atomic::AtomicBool::new(true)),
        ))
    })
    .await;
    let task = other(&app, "zone-q-task", |_| {}).await;
    task.lock().await.thread.task_meta =
        Some(serde_json::from_value(json!({"task_id":"t","role":"agent"})).unwrap());
    for id in ["zone-q-runner", "zone-q-task"] {
        let out = result(
            &run(
                &app,
                &format!("(await engine('chats.queue', chat_id='{id}'))['data']"),
            )
            .await,
        );
        assert_eq!(out["chat_id"], id, "{out}");
        assert_eq!(out["size"], 0, "{out}");
    }
    other(&app, "zone-q-buddy", |s| s.thread.mode = "buddy".into()).await;
    assert_eq!(
        refused(&app, "await engine('chats.queue', chat_id='zone-q-buddy')").await,
        "PermissionError"
    );
}

#[tokio::test]
async fn cancel_queued_removes_one_item_of_another_chat() {
    let (_dir, app, _) = fixture().await;
    let target = other(&app, "zone-cq", |s| {
        hold_queue(s);
        s.runtime.state = refact_chat_api::SessionState::Generating;
        s.command_queue.push_back(
            serde_json::from_value(json!({"client_request_id":"zone-cq-cmd","type":"user_message","content":"zone queued"}))
                .unwrap(),
        );
    })
    .await;
    let card = deny(
        &app,
        "await engine('chats.cancel_queued', chat_id='zone-cq', id='zone-cq-cmd')",
    )
    .await
    .engine
    .unwrap();
    assert_eq!(card.class, Class::Destructive);
    assert_eq!(
        target.lock().await.command_queue.len(),
        1,
        "deny removed it"
    );
    let (_, out) = approve(
        &app,
        "(await engine('chats.cancel_queued', chat_id='zone-cq', id='zone-cq-cmd'))['data']",
    )
    .await;
    assert_eq!(out["removed"], true, "{out}");
    assert!(target.lock().await.command_queue.is_empty());
    assert_eq!(
        refused(
            &app,
            "await engine('chats.cancel_queued', chat_id='zone-cq', id='zone-cq-cmd')"
        )
        .await,
        "TypeError"
    );
    let delivery = refact_chat_api::PendingDelivery::new(
        vec![ChatMessage::new("user".into(), "zone later".into())],
        refact_chat_api::PushMode::WhenIdle,
        "scheduler.cron",
        true,
    );
    let delivery_id = delivery.id.clone();
    target.lock().await.pending_deliveries.push_back(delivery);
    let card = deny(
        &app,
        &format!("await engine('chats.cancel_queued', chat_id='zone-cq', id='{delivery_id}')"),
    )
    .await
    .engine
    .unwrap();
    assert_eq!(card.class, Class::Destructive, "a queued user delivery");
}

#[tokio::test]
async fn goal_control_and_budget_of_another_chat_are_checked_and_queued() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    assert_eq!(
        refused(
            &app,
            "await engine('chats.goal_control', chat_id='zone-gc-none', action='pause')"
        )
        .await,
        "TypeError"
    );
    other(&app, "zone-gc-none", hold_queue).await;
    assert_eq!(
        refused(
            &app,
            "await engine('chats.goal_control', chat_id='zone-gc-none', action='pause')"
        )
        .await,
        "TypeError",
        "no goal"
    );
    let target = other(&app, "zone-gc", hold_queue).await;
    target.lock().await.install_goal(
        "agent",
        "zone ship it",
        true,
        refact_chat_api::GoalBudget::default(),
    );
    let (_, out) = approve(
        &app,
        "(await engine('chats.goal_control', chat_id='zone-gc', action='pause'))['data']",
    )
    .await;
    assert_eq!(out["action"], "pause", "{out}");
    let queued = queued_commands(&*target.lock().await);
    assert_eq!(queued.last().unwrap()["type"], "goal_control");
    assert_eq!(queued.last().unwrap()["action"], "pause");
    target
        .lock()
        .await
        .goal_set_status(refact_chat_api::GoalStatus::Paused);
    target.lock().await.command_queue.clear();
    let card = deny(
        &app,
        "await engine('chats.goal_control', chat_id='zone-gc', action='resume')",
    )
    .await
    .engine
    .unwrap();
    assert_eq!(card.class, Class::Risky);
    assert!(card.warning.unwrap().contains("without you"));
    assert!(target.lock().await.command_queue.is_empty(), "deny queued");
    target.lock().await.goal.as_mut().unwrap().budget.supervised = true;
    assert_eq!(
        refused(
            &app,
            "await engine('chats.set_goal_budget', chat_id='zone-gc', cooldown_ms=5)"
        )
        .await,
        "PermissionError",
        "supervised goals stay refused"
    );
}

#[tokio::test]
async fn trajectory_reads_and_changes_refuse_chats_of_another_project() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let mut foreign = build("zone-fp", |_| {});
    foreign.work_project_root = Some(std::env::temp_dir());
    app.chat
        .sessions
        .write()
        .await
        .insert("zone-fp".into(), Arc::new(tokio::sync::Mutex::new(foreign)));
    for call in [
        "await engine('chats.read', chat_id='zone-fp')",
        "await engine('chats.queue', chat_id='zone-fp')",
        "await engine('chats.send', chat_id='zone-fp', text='zone x')",
        "await engine('chats.regenerate', chat_id='zone-fp')",
        "await engine('chats.goal_control', chat_id='zone-fp', action='pause')",
    ] {
        assert_eq!(refused(&app, call).await, "PermissionError", "{call}");
    }
}

/// A1: a change that opens a saved chat with an active goal says that opening it lets the goal
/// continue, and its preview leaves the chat closed.
#[tokio::test]
async fn changes_that_open_a_saved_chat_with_an_active_goal_say_so() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let mut s = build("zone-og", |_| {});
    let _ = s.install_goal(
        "agent",
        "zone keep going",
        true,
        refact_chat_api::GoalBudget::default(),
    );
    super::chats_risky::save(&app, &s).await;
    for code in [
        "await engine('chats.set_params', chat_id='zone-og', temperature=0.4)",
        "await engine('chats.send', chat_id='zone-og', text='zone hi')",
        "await engine('chats.goal_control', chat_id='zone-og', action='pause')",
    ] {
        let card = deny(&app, code).await.engine.unwrap();
        let warning = card.warning.unwrap_or_default();
        assert!(
            warning.contains("opening it to apply this lets its active goal continue"),
            "{code}: {warning}"
        );
        assert!(
            !app.chat.sessions.read().await.contains_key("zone-og"),
            "{code}"
        );
    }
}

/// A2 through the real queue: the processor latches who starts a turn when it takes the command;
/// a typed message is a person, the same command queued by an engine function is not.
#[tokio::test]
async fn the_queue_latches_the_origin_of_the_turn_it_starts() {
    let (_dir, app, _) = fixture().await;
    let target = other(&app, "zone-lt", |s| {
        s.thread.model = "zone/unknown-model".into();
        s.thread.recovery_policy = Some(serde_json::from_value(json!({"mode":"off"})).unwrap());
    })
    .await;
    let push = |id: &str, text: &str| -> refact_chat_api::CommandRequest {
        serde_json::from_value(json!({"client_request_id":id,"type":"user_message","content":text}))
            .unwrap()
    };
    {
        let mut s = target.lock().await;
        s.command_queue.push_back(push("typed-lt", "zone typed"));
        let running = s.queue_processor_running.clone();
        running.store(true, std::sync::atomic::Ordering::SeqCst);
        tokio::spawn(crate::chat::process_command_queue(
            app.clone(),
            target.clone(),
            running,
        ));
    }
    until(&target, |s| s.turn_origin.is_some()).await;
    assert_eq!(
        target.lock().await.turn_origin.as_ref().unwrap().automation,
        None,
        "a typed message starts a person's turn"
    );
    {
        let mut s = target.lock().await;
        s.command_queue
            .push_back(push("engine-lt", "zone scripted"));
        s.queue_notify.notify_one();
    }
    until(&target, |s| {
        s.turn_origin
            .as_ref()
            .is_some_and(|origin| origin.automation.is_some())
    })
    .await;
}

fn accept_gate(id: &str) -> (Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>) {
    let (entered, release) = (
        Arc::new(tokio::sync::Notify::new()),
        Arc::new(tokio::sync::Notify::new()),
    );
    crate::chat::delivery::ACCEPT_GATE.lock().unwrap().push((
        id.into(),
        entered.clone(),
        release.clone(),
    ));
    (entered, release)
}

fn has_text(s: &ChatSession, text: &str) -> bool {
    s.messages
        .iter()
        .chain(s.pending_deliveries.iter().flat_map(|d| d.messages.iter()))
        .any(|m| m.content.content_text_only() == text)
}

/// T7, A8: chats.send compares the model as resolved, its provider and the approval flags under
/// the lock that accepts it: a change between the answer and that lock sends nothing.
#[tokio::test]
async fn send_compares_its_pins_under_the_accepting_lock() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    for (id, default_model) in [("zone-sk-flags", false), ("zone-sk-model", true)] {
        let target = other(&app, id, |s| {
            s.thread.mode = "ask".into();
            s.thread.model = String::new();
        })
        .await;
        let (entered, release) = accept_gate(id);
        let task = spawn(
            &app,
            &format!("r = await engine('chats.send', chat_id='{id}', text='zone pinned')\n[r['ok'], r.get('error')]"),
        );
        let request = next_request(&app, &task).await;
        decide(&app, &request.request_id, Decision::AllowOnce)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(20), entered.notified())
            .await
            .expect("the send did not reach its accepting lock");
        if default_model {
            let mut state = app.gcx.caps_state.write().await;
            let mut caps = (**state.caps.as_ref().unwrap()).clone();
            caps.defaults.chat_default_model = "zone/model-a".into();
            state.caps = Some(Arc::new(caps));
        } else {
            target.lock().await.thread.auto_approve_dangerous_commands = true;
        }
        release.notify_one();
        let out = result(&finish(task).await);
        if default_model {
            install(&app).await;
        }
        assert_eq!(out[0], false, "{id}: {out}");
        assert!(
            out[1]
                .as_str()
                .unwrap()
                .contains("changed since the preview"),
            "{id}: {out}"
        );
        assert!(!has_text(&*target.lock().await, "zone pinned"), "{id}");
    }
}

/// T5: cancelling a queued delivery pins its whole envelope (text, push, source, records) and
/// checks it again with that chat's owner under its lock.
#[tokio::test]
async fn cancelling_a_delivery_pins_its_whole_envelope() {
    let (_dir, app, _) = fixture().await;
    let delivery = refact_chat_api::PendingDelivery::new(
        vec![ChatMessage::new("user".into(), "zone later".into())],
        refact_chat_api::PushMode::WhenIdle,
        "scheduler.cron",
        true,
    );
    let id = delivery.id.clone();
    let target = other(&app, "zone-ce", |s| {
        hold_queue(s);
        s.runtime.state = refact_chat_api::SessionState::Generating;
        s.pending_deliveries.push_back(delivery);
    })
    .await;
    let t = target.clone();
    let out = moved_then_allowed(
        &app,
        &format!("await engine('chats.cancel_queued', chat_id='zone-ce', id='{id}')"),
        async move {
            t.lock().await.pending_deliveries[0].push = refact_chat_api::PushMode::Append;
        },
    )
    .await;
    assert_eq!(out[0], false, "{out}");
    assert!(
        out[1]
            .as_str()
            .unwrap()
            .contains("changed since the preview"),
        "{out}"
    );
    assert_eq!(target.lock().await.pending_deliveries.len(), 1);
    let (_, out) = approve(
        &app,
        &format!("(await engine('chats.cancel_queued', chat_id='zone-ce', id='{id}'))['data']"),
    )
    .await;
    assert_eq!(out["removed"], true, "{out}");
    assert!(target.lock().await.pending_deliveries.is_empty());
}

/// T8: a paused goal is not an active one, for saved and loaded chats alike: the first send to a
/// saved chat with a paused goal goes through, and the card of a loaded one says that the goal
/// is paused rather than pursued.
#[tokio::test]
async fn a_paused_goal_is_not_pursued_in_saved_and_loaded_chats() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let mut saved = build("zone-pz-saved", |s| s.thread.mode = "ask".into());
    let _ = saved.install_goal(
        "agent",
        "zone keep going",
        true,
        refact_chat_api::GoalBudget::default(),
    );
    saved.goal_set_status(refact_chat_api::GoalStatus::Paused);
    super::chats_risky::save(&app, &saved).await;
    let (_, out) = approve(
        &app,
        "r = await engine('chats.send', chat_id='zone-pz-saved', text='zone hi')\n[r['ok'], r.get('error'), r['data']]",
    )
    .await;
    assert_eq!(out[0], true, "{out}");
    assert_eq!(out[2]["opened"], true, "{out}");
    let loaded = other(&app, "zone-pz-loaded", |s| {
        hold_queue(s);
        s.thread.mode = "ask".into();
    })
    .await;
    {
        let mut s = loaded.lock().await;
        let _ = s.install_goal(
            "agent",
            "zone keep going",
            true,
            refact_chat_api::GoalBudget::default(),
        );
        s.goal_set_status(refact_chat_api::GoalStatus::Paused);
    }
    let card = deny(
        &app,
        "await engine('chats.send', chat_id='zone-pz-loaded', text='zone x')",
    )
    .await
    .engine
    .unwrap();
    let goal = card.items.iter().find(|i| i.label == "goal").unwrap();
    let goal = goal.new.as_ref().unwrap().as_str().unwrap();
    assert!(goal.contains("has a paused goal"), "{goal}");
    assert!(!goal.contains("pursued without you"), "{goal}");
    assert_eq!(card.class, Class::Change, "{:?}", card.warning);
}

/// T9: goal_control pins only the goal's identity and status: a chat working on its goal while
/// the card waits (progress recorded, file saved) is still paused, and so is a saved chat.
#[tokio::test]
async fn goal_control_of_a_working_chat_and_of_a_saved_chat_is_applied() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-gw", |s| {
        hold_queue(s);
        s.runtime.state = refact_chat_api::SessionState::Generating;
    })
    .await;
    let _ = target.lock().await.install_goal(
        "agent",
        "zone ship it",
        true,
        refact_chat_api::GoalBudget::default(),
    );
    let (t, app2) = (target.clone(), app.clone());
    let out = moved_then_allowed(
        &app,
        "await engine('chats.goal_control', chat_id='zone-gw', action='pause')",
        async move {
            let mut s = t.lock().await;
            if let Some(goal) = s.goal.as_mut() {
                goal.revision += 1;
            }
            s.messages
                .push(message("zone-gw-m5", "assistant", "zone progress"));
            super::chats_risky::save(&app2, &s).await;
        },
    )
    .await;
    assert_eq!(out[0], true, "{out}");
    assert_eq!(
        queued_commands(&*target.lock().await).last().unwrap()["type"],
        "goal_control"
    );
    let mut saved = build("zone-gsv", |_| {});
    let _ = saved.install_goal(
        "agent",
        "zone ship it",
        true,
        refact_chat_api::GoalBudget::default(),
    );
    super::chats_risky::save(&app, &saved).await;
    let (_, out) = approve(
        &app,
        "r = await engine('chats.goal_control', chat_id='zone-gsv', action='pause')\n[r['ok'], r.get('error'), r['data']]",
    )
    .await;
    assert_eq!(out[0], true, "{out}");
    assert_eq!(out[2]["opened"], true, "{out}");
}

/// T11: the A5 risk test reads the tools that chat has: an automatic handoff_to_mode makes a
/// turn start risky, an approval flag without a tool it approves does not.
#[tokio::test]
async fn turn_cards_judge_risk_from_the_tools_that_chat_has() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    other(&app, "zone-rk-handoff", hold_queue).await;
    let card = deny(
        &app,
        "await engine('chats.regenerate', chat_id='zone-rk-handoff')",
    )
    .await
    .engine
    .unwrap();
    assert_eq!(card.class, Class::Risky, "explore hands off without asking");
    other(&app, "zone-rk-flag", |s| {
        hold_queue(s);
        s.thread.mode = "ask".into();
        s.thread.auto_approve_editing_tools = true;
    })
    .await;
    let card = deny(
        &app,
        "await engine('chats.regenerate', chat_id='zone-rk-flag')",
    )
    .await
    .engine
    .unwrap();
    assert_eq!(
        card.class,
        Class::Change,
        "an editing flag without an editing tool: {:?}",
        card.warning
    );
}

/// T12: the card of a cancelled delivery merges that chat's derived privacy labels into this run,
/// whatever the answer.
#[tokio::test]
async fn the_cancel_card_carries_the_derived_labels_of_the_text_it_shows() {
    let (dir, app, _) = fixture().await;
    let delivery = refact_chat_api::PendingDelivery::new(
        vec![ChatMessage::new("user".into(), "zone derived text".into())],
        refact_chat_api::PushMode::WhenIdle,
        "scheduler.cron",
        true,
    );
    let id = delivery.id.clone();
    let target = other(&app, "zone-cd", |s| {
        hold_queue(s);
        s.runtime.state = refact_chat_api::SessionState::Generating;
        s.pending_deliveries.push_back(delivery);
    })
    .await;
    target
        .lock()
        .await
        .derived_privacy_zones
        .write()
        .unwrap()
        .insert(
            dir.path().join("project/derived.txt"),
            crate::privacy::records::DerivedZone {
                zone: "zone_derived".into(),
                origin: None,
            },
        );
    let task = spawn(
        &app,
        &format!("try:\n    await engine('chats.cancel_queued', chat_id='zone-cd', id='{id}')\nexcept PermissionError:\n    pass"),
    );
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    let m = finish(task).await;
    assert!(
        m.extra
            .get("privacy")
            .is_some_and(|p| p.to_string().contains("zone_derived")),
        "{:?}",
        m.extra
    );
}

/// T13: a target title that looks like a secret is masked on the card instead of refusing it.
#[tokio::test]
async fn a_secret_looking_title_is_masked_on_the_card() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    other(&app, "zone-st", |s| {
        hold_queue(s);
        s.thread.mode = "ask".into();
        s.thread.title = "fix DATABASE_URL=postgres://u:pw@h/db".into();
    })
    .await;
    let card = deny(
        &app,
        "await engine('chats.send', chat_id='zone-st', text='zone hi')",
    )
    .await
    .engine
    .unwrap();
    let chat = card.items.iter().find(|i| i.label == "chat").unwrap();
    let shown = chat.new.as_ref().unwrap().as_str().unwrap();
    assert!(shown.contains("fix DATABASE_URL"), "{shown}");
    assert!(!shown.contains("u:pw"), "{shown}");
    assert!(!card.scope.unwrap().contains("u:pw"));
}

/// T14: a preempt pins the turn it interrupts: when another turn started meanwhile, nothing is
/// interrupted.
#[tokio::test]
async fn a_preempt_pins_the_turn_it_interrupts() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let started = |at_ms| refact_chat_history::history_gates::TurnOrigin {
        automation: None,
        at_ms,
    };
    let target = other(&app, "zone-pre", |s| {
        s.thread.mode = "ask".into();
        s.runtime.state = refact_chat_api::SessionState::Generating;
        s.turn_depth = 1;
    })
    .await;
    target.lock().await.turn_origin = Some(started(1));
    let t = target.clone();
    let out = moved_then_allowed(
        &app,
        "await engine('chats.send', chat_id='zone-pre', text='zone stop', push='preempt')",
        async move { t.lock().await.turn_origin = Some(started(2)) },
    )
    .await;
    assert_eq!(out[0], false, "{out}");
    assert!(
        out[1]
            .as_str()
            .unwrap()
            .contains("changed since the preview"),
        "{out}"
    );
    assert!(!has_text(&*target.lock().await, "zone stop"));
}

/// T14: chat.send words its result from the delivery receipt.
#[tokio::test]
async fn chat_send_words_its_result_from_the_receipt() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let (_, out) = approve(
        &app,
        "r = await engine('chat.send', text='zone soon')\n[r['data']['outcome'], r['text']]",
    )
    .await;
    assert_eq!(out[0], "delivered", "{out}");
    assert!(
        out[1].as_str().unwrap().contains("delivered")
            && !out[1].as_str().unwrap().contains("queued"),
        "{out}"
    );
}

/// T14: chat.send checks egress on the model its requests go to, not the name its thread keeps.
#[tokio::test]
async fn chat_send_checks_egress_on_the_resolved_model() {
    let (dir, app, _) = fixture().await;
    install(&app).await;
    {
        let s = session(&app).await;
        let mut s = s.lock().await;
        s.thread.model = String::new();
        s.runtime.state = refact_chat_api::SessionState::ExecutingTools;
        s.turn_depth = 1;
    }
    crate::privacy::save_privacy_policy(app.gcx.clone(), super::chats_risky::guarded_policy(&app))
        .await
        .unwrap();
    std::fs::write(dir.path().join("project/zone_a.txt"), "guarded").unwrap();
    let (_, out) = approve(
        &app,
        "x = open('zone_a.txt').read()\n(await engine('chat.send', text='zone ' + x))['data']",
    )
    .await;
    assert_eq!(out["outcome"], "queued", "{out}");
}

/// T15: a fork keeps the delivery stamp of copied user messages, so text a script wrote stays
/// literal in the copy.
#[tokio::test]
async fn a_fork_keeps_the_script_stamp_of_copied_user_messages() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let delivery = refact_chat_api::PendingDelivery::new(
        vec![message("zone-fs-d", "user", "zone see @web x")],
        refact_chat_api::PushMode::WhenIdle,
        "engine.chats.send",
        true,
    );
    other(&app, "zone-fs", |s| {
        s.messages.extend(delivery.stamp_messages())
    })
    .await;
    let (_, out) = approve(
        &app,
        "(await engine('chats.fork', chat_id='zone-fs', up_to_message_id='zone-fs-d'))['data']",
    )
    .await;
    let fork = app.chat.sessions.read().await[out["new_chat_id"].as_str().unwrap()].clone();
    let s = fork.lock().await;
    let copy = s
        .messages
        .iter()
        .find(|m| m.content.content_text_only() == "zone see @web x")
        .unwrap();
    assert_eq!(
        copy.extra[refact_chat_api::DELIVERY_EXTRA_KEY]["source"],
        "engine.chats.send"
    );
}

/// T21: a chat whose thread says it is a child of an agent is an agent chat, even when the
/// in-memory registry does not know it.
#[tokio::test]
async fn a_chat_linked_as_an_agent_child_is_refused_without_the_registry() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-lk", |s| {
        s.thread.parent_id = Some("python-test".into());
        s.thread.link_type = Some("subagent".into());
    })
    .await;
    let before = other_fingerprint(&*target.lock().await);
    for call in [
        "await engine('chats.send', chat_id='zone-lk', text='zone x')",
        "await engine('chats.patch', chat_id='zone-lk', ops=[{'op':'edit','message_id':'zone-lk-m2','text':'zone x'}])",
    ] {
        assert_eq!(refused(&app, call).await, "PermissionError", "{call}");
    }
    assert_eq!(before, other_fingerprint(&*target.lock().await));
}

/// A6 end to end: a chat under a live supervision grant, a companion conversation and a chat a
/// background agent runs are refused before any card.
#[tokio::test]
async fn supervised_companion_and_agent_chats_are_refused_end_to_end() {
    let (dir, app, _) = fixture().await;
    install(&app).await;
    other(&app, "zone-sv", |_| {}).await;
    let mandate = crate::buddy::supervision_authority::bind_test_engineering_scope(
        "zone-sv",
        dir.path(),
        vec![],
    )
    .unwrap();
    other(&app, "zone-cpn", |s| {
        s.thread.buddy_meta = Some(
            serde_json::from_value(json!({"is_buddy_chat":false,"buddy_chat_kind":"companion"}))
                .unwrap(),
        )
    })
    .await;
    let (agent, _, _) = app
        .agents
        .create(crate::agents::types::CreateAgentRequest {
            parent_chat_id: "python-test".into(),
            parent_root_chat_id: Some("python-test".into()),
            parent_tool_call_id: None,
            kind: crate::agents::types::BgAgentKind::Subagent,
            config_name: "subagent".into(),
            title: "zone agent".into(),
            prompt: "zone agent".into(),
            target_files: vec![],
            model: "model".into(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
        })
        .await
        .unwrap();
    app.agents
        .mark_running(&agent.agent_id, "zone-agc".into())
        .await
        .unwrap();
    other(&app, "zone-agc", |_| {}).await;
    for id in ["zone-sv", "zone-cpn", "zone-agc"] {
        for call in [
            format!("await engine('chats.send', chat_id='{id}', text='zone x')"),
            format!("await engine('chats.patch', chat_id='{id}', ops=[{{'op':'edit','message_id':'{id}-m2','text':'zone x'}}])"),
        ] {
            assert_eq!(refused(&app, &call).await, "PermissionError", "{call}");
        }
    }
    crate::buddy::supervision_authority::invalidate(&mandate).unwrap();
}

/// T6: a regenerate checks the held goal again under the lock that queues it, with the predicate
/// of its preview.
#[tokio::test]
async fn a_goal_paused_before_the_regenerate_is_queued_refuses_it() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-rp", |s| {
        hold_queue(s);
        s.thread.mode = "ask".into();
    })
    .await;
    let _ = target.lock().await.install_goal(
        "agent",
        "zone keep going",
        true,
        refact_chat_api::GoalBudget::default(),
    );
    let hook: fn(&mut ChatSession) = |s| {
        s.goal_set_status(refact_chat_api::GoalStatus::Paused);
    };
    crate::python_tool::engine_fns::chats::TEST_HOOKS
        .lock()
        .unwrap()
        .push(("regenerate:zone-rp".into(), hook));
    let (_, out) = approve(
        &app,
        "r = await engine('chats.regenerate', chat_id='zone-rp')\n[r['ok'], r.get('error')]",
    )
    .await;
    crate::python_tool::engine_fns::chats::TEST_HOOKS
        .lock()
        .unwrap()
        .retain(|(key, _)| key != "regenerate:zone-rp");
    assert_eq!(out[0], false, "{out}");
    assert!(target.lock().await.command_queue.is_empty());
}

/// T23: the regenerate refusal for a recovery record names what archives it, not goal_control;
/// the short forms' output schemas name `opened`.
#[tokio::test]
async fn regenerate_texts_and_short_form_schemas_say_what_the_code_does() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    other(&app, "zone-rr", |s| {
        hold_queue(s);
        s.runtime.recovery = Some(serde_json::from_value(json!({
            "operation_id":"o","run_id":"r","revision":1,"state":"succeeded",
            "policy":{"mode":"transient","max_retries":1,"delay_ms":0,"total_timeout_ms":1000},
            "policy_source":"global","manual":false,"retries_used":0,
            "latest_error":{"category":"c","title":"t","explanation":"e","suggested_action":"s","is_retryable":true,"raw_error":"r"},
            "next_retry_at_ms":null,"deadline_at_ms":null,"terminal_reason":null,"attempts":[]
        })).unwrap())
    })
    .await;
    let m = run(
        &app,
        "try:\n    await engine('chats.regenerate', chat_id='zone-rr')\n    r = 'ok'\nexcept PermissionError as e:\n    r = str(e)\nr",
    )
    .await;
    let text = result(&m);
    let text = text.as_str().unwrap();
    assert!(text.contains("recovery record"), "{text}");
    assert!(text.contains("chats.send"), "{text}");
    assert!(!text.contains("goal_control"), "{text}");
    for name in [
        "chats.edit_message",
        "chats.remove_message",
        "chats.retry_from",
    ] {
        let output = (crate::python_tool::engine_fns::lookup(name).unwrap().output)().unwrap();
        assert!(
            output["required"]
                .as_array()
                .unwrap()
                .contains(&json!("opened")),
            "{name}"
        );
        assert!(output["properties"].get("opened").is_some(), "{name}");
    }
}
