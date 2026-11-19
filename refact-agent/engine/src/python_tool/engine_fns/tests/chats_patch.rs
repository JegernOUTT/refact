use super::chats_risky::{
    approve, build, deny, guarded_policy, install, message, moved_then_allowed, other,
    other_fingerprint, record, refused, save,
};
use super::*;
use crate::chat::types::ChatSession;
use refact_chat_api::PythonEngineClass as Class;

fn texts(s: &ChatSession) -> Vec<String> {
    s.messages
        .iter()
        .map(|m| format!("{}:{}", m.role, m.content.content_text_only()))
        .collect()
}

async fn file_bytes(app: &AppState, id: &str) -> Vec<u8> {
    let path = crate::chat::trajectories::find_trajectory_path(app.gcx.clone(), id)
        .await
        .unwrap();
    std::fs::read(path).unwrap()
}

async fn loaded(app: &AppState, id: &str) -> bool {
    app.chat.sessions.read().await.contains_key(id)
}

/// Runs a script whose change is denied and returns the run's message.
async fn denied_run(
    app: &AppState,
    code: &str,
) -> (refact_chat_api::PythonApprovalRequest, ChatMessage) {
    let task = spawn(app, code);
    let request = next_request(app, &task).await;
    decide(app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    (request, finish(task).await)
}

/// Runs a script whose change is allowed and returns the run's message.
async fn allowed_run(
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

fn zones(m: &ChatMessage) -> String {
    m.extra
        .get("privacy")
        .map(|p| p.to_string())
        .unwrap_or_default()
}

const EDIT_REMOVE_INSERT: &str = "[{'op':'edit','message_id':'zone-pa-m2','text':'zone edited question'},{'op':'remove','message_id':'zone-pa-m3'},{'op':'insert','after':'zone-pa-m4','role':'assistant','text':'zone inserted answer'}]";

#[tokio::test]
async fn patch_applies_edit_remove_and_insert_as_one_saves_and_emits_one_snapshot() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-pa", |_| {}).await;
    let mut events = target.lock().await.subscribe();
    let version = target.lock().await.trajectory_version;
    let (request, out) = approve(
        &app,
        &format!(
            "(await engine('chats.patch', chat_id='zone-pa', ops={EDIT_REMOVE_INSERT}))['data']"
        ),
    )
    .await;
    let card = request.engine.unwrap();
    assert_eq!(card.class, Class::Destructive);
    assert_eq!(card.title, "rewrite another chat's history");
    let warning = card.warning.unwrap();
    assert!(warning.contains("cannot be restored"), "{warning}");
    assert!(warning.contains("real conversation"), "{warning}");
    let labels: Vec<&str> = card.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "#2 user: edit",
            "#3 assistant: remove",
            "after #4: insert assistant"
        ],
        "{:?}",
        card.items
    );
    assert_eq!(card.items[0].old, Some(json!("zone second question")));
    assert_eq!(card.items[0].new, Some(json!("zone edited question")));
    assert_eq!(card.items[1].old, Some(json!("zone second answer")));

    assert_eq!(out["edited"], 1, "{out}");
    assert_eq!(out["removed"], 1, "{out}");
    assert_eq!(out["inserted"], 1, "{out}");
    assert_eq!(out["opened"], false, "{out}");
    assert_eq!(out["persisted"], true, "{out}");
    let s = target.lock().await;
    assert_eq!(
        texts(&s),
        [
            "user:zone first question",
            "assistant:zone first answer",
            "user:zone edited question",
            "user:zone third question",
            "assistant:zone inserted answer",
        ]
    );
    assert_eq!(s.trajectory_version, version + 1);
    assert!(!s.trajectory_dirty, "the patch was not committed");
    assert_eq!(out["ids"], json!([s.messages[4].message_id]));
    assert_eq!(
        s.messages[2].extra["delivery"]["source"], "engine.chats.patch",
        "an edited user message is stamped as written by a script"
    );
    let stored = texts(&s);
    drop(s);
    let saved = crate::chat::trajectories::load_trajectory_for_chat(app.gcx.clone(), "zone-pa")
        .await
        .unwrap();
    let saved_texts: Vec<String> = saved
        .messages
        .iter()
        .map(|m| format!("{}:{}", m.role, m.content.content_text_only()))
        .collect();
    assert_eq!(saved_texts, stored, "the saved file differs from the chat");
    let mut snapshots = 0;
    while let Ok(event) = events.try_recv() {
        let event: Value = serde_json::from_str(&event).unwrap();
        if event["type"] == "snapshot" {
            snapshots += 1;
            assert_eq!(event["messages"].as_array().unwrap().len(), 5, "{event}");
        }
    }
    assert_eq!(snapshots, 1);
    let read = result(
        &run(
            &app,
            "(await engine('chats.read', chat_id='zone-pa'))['data']['digest']",
        )
        .await,
    );
    assert_eq!(read, out["digest"], "the returned digest is the next pin");
}

#[tokio::test]
async fn patch_inserts_closed_tool_calls_and_removes_calls_with_their_results() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-pt", |_| {}).await;
    let (_, out) = approve(
        &app,
        "(await engine('chats.patch', chat_id='zone-pt', ops=[{'op':'insert','after':'zone-pt-m2','role':'assistant','text':'zone looking','tool_calls':[{'name':'cat','arguments':{'paths':'a.txt'},'result':'zone file text'}]}]))['data']",
    )
    .await;
    let calls = out["tool_call_ids"][0].as_array().unwrap().clone();
    assert_eq!(calls.len(), 1, "{out}");
    let call_id = calls[0].as_str().unwrap().to_string();
    assert!(call_id.starts_with("call_"), "{call_id}");
    {
        let s = target.lock().await;
        let at = s
            .messages
            .iter()
            .position(|m| m.message_id == out["ids"][0].as_str().unwrap())
            .unwrap();
        assert_eq!(at, 3);
        assert_eq!(s.messages[at].tool_calls.as_ref().unwrap()[0].id, call_id);
        assert_eq!(s.messages[at + 1].role, "tool");
        assert_eq!(s.messages[at + 1].tool_call_id, call_id);
        assert_eq!(
            s.messages[at + 1].content.content_text_only(),
            "zone file text"
        );
    }
    let head = out["ids"][0].as_str().unwrap().to_string();
    assert_eq!(
        refused(
            &app,
            &format!("await engine('chats.patch', chat_id='zone-pt', ops=[{{'op':'remove','message_id':'{head}'}}])")
        )
        .await,
        "PermissionError",
        "removing a call without its results must stay refused"
    );
    let (request, out) = approve(
        &app,
        &format!("(await engine('chats.patch', chat_id='zone-pt', ops=[{{'op':'remove','message_id':'{head}','with_results':True}}]))['data']"),
    )
    .await;
    let card = request.engine.unwrap();
    assert!(
        card.items
            .iter()
            .any(|i| i.label.ends_with(": removed with #3")),
        "{:?}",
        card.items
    );
    assert_eq!(out["removed"], 2, "{out}");
    assert_eq!(target.lock().await.messages.len(), 5);
}

#[tokio::test]
async fn patch_deny_changes_nothing_leaves_a_saved_chat_unopened_and_the_shown_records_travel() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let mut chat = build("zone-pd", |_| {});
    crate::privacy::records::attach_record(
        &mut chat.messages[2],
        record("/x/zone_secret.txt", "zone_secret"),
    );
    save(&app, &chat).await;
    let before = file_bytes(&app, "zone-pd").await;
    let (request, m) = denied_run(
        &app,
        "try:\n    await engine('chats.patch', chat_id='zone-pd', ops=[{'op':'edit','message_id':'zone-pd-m2','text':'zone x'}])\nexcept PermissionError as e:\n    r = str(e)\nr",
    )
    .await;
    assert!(request.engine.is_some());
    assert!(
        !loaded(&app, "zone-pd").await,
        "a preview or a deny opened the chat"
    );
    assert_eq!(
        file_bytes(&app, "zone-pd").await,
        before,
        "deny wrote the file"
    );
    assert!(
        zones(&m).contains("zone_secret"),
        "the records of the text the card showed must reach this chat even on deny: {:?}",
        m.extra
    );
    let calls = std::fs::read_dir(
        std::path::Path::new(m.extra["python"]["run_dir"].as_str().unwrap()).join("calls"),
    )
    .unwrap()
    .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
    .collect::<Vec<_>>()
    .join("\n");
    assert!(
        !calls.contains("zone second question"),
        "the call file holds the other chat's text"
    );
}

#[tokio::test]
async fn patch_opens_a_saved_chat_only_after_approval_and_says_so() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    save(&app, &build("zone-po", |_| {})).await;
    let (_, out) = approve(
        &app,
        "r = await engine('chats.patch', chat_id='zone-po', ops=[{'op':'edit','message_id':'zone-po-m4','text':'zone new last'}])\n[r['data'], r['text']]",
    )
    .await;
    assert_eq!(out[0]["opened"], true, "{out}");
    assert!(
        out[1].as_str().unwrap().contains("the chat was opened"),
        "{out}"
    );
    assert!(loaded(&app, "zone-po").await);
    let saved = crate::chat::trajectories::load_trajectory_for_chat(app.gcx.clone(), "zone-po")
        .await
        .unwrap();
    assert_eq!(
        saved.messages[4].content.content_text_only(),
        "zone new last"
    );
}

#[tokio::test]
async fn patch_of_a_moved_target_is_refused_and_applies_nothing() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-pm", |_| {}).await;
    let t = target.clone();
    let out = moved_then_allowed(
        &app,
        "await engine('chats.patch', chat_id='zone-pm', ops=[{'op':'edit','message_id':'zone-pm-m2','text':'zone x'}])",
        async move {
            t.lock()
                .await
                .messages
                .push(message("zone-pm-m5", "assistant", "zone late answer"));
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
    assert_eq!(
        target.lock().await.messages[2].content.content_text_only(),
        "zone second question"
    );

    save(&app, &build("zone-pf", |_| {})).await;
    let app2 = app.clone();
    let out = moved_then_allowed(
        &app,
        "await engine('chats.patch', chat_id='zone-pf', ops=[{'op':'edit','message_id':'zone-pf-m2','text':'zone x'}])",
        async move {
            let mut s = build("zone-pf", |_| {});
            s.thread.title = "zone renamed by another engine".into();
            save(&app2, &s).await;
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
    assert!(
        !loaded(&app, "zone-pf").await,
        "a stale pin opened the saved chat"
    );
}

#[tokio::test]
async fn patch_never_saves_over_a_file_another_engine_changed() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-pk", |_| {}).await;
    let before = other_fingerprint(&*target.lock().await);
    let app2 = app.clone();
    let out = moved_then_allowed(
        &app,
        "await engine('chats.patch', chat_id='zone-pk', ops=[{'op':'edit','message_id':'zone-pk-m2','text':'zone x'}])",
        async move {
            let mut s = build("zone-pk", |_| {});
            s.messages.push(message("zone-pk-m9", "user", "zone written elsewhere"));
            save(&app2, &s).await;
        },
    )
    .await;
    assert_eq!(out[0], false, "{out}");
    assert!(out[1].as_str().unwrap().contains("disk"), "{out}");
    assert_eq!(before, other_fingerprint(&*target.lock().await));
    let saved = crate::chat::trajectories::load_trajectory_for_chat(app.gcx.clone(), "zone-pk")
        .await
        .unwrap();
    assert_eq!(
        saved.messages.len(),
        6,
        "the other engine's file was overwritten"
    );
}

#[tokio::test]
async fn patch_refuses_before_any_card_what_it_must_not_change() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    other(&app, "zone-pr", |_| {}).await;
    let big = "y".repeat(17 * 1024);
    let edit = |text: &str| {
        format!("await engine('chats.patch', chat_id='zone-pr', ops=[{{'op':'edit','message_id':'zone-pr-m2','text':'{text}'}}])")
    };
    let many = (0..51)
        .map(|i| format!("{{'op':'insert','after':'end','role':'user','text':'zone {i}'}}"))
        .collect::<Vec<_>>()
        .join(",");
    let total = (0..5)
        .map(|_| {
            format!(
                "{{'op':'insert','after':'end','role':'user','text':'{}'}}",
                "z".repeat(15 * 1024)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    for (code, kind) in [
        (edit("zone x").replace("zone-pr'", "python-test'"), "TypeError"),
        (edit("zone x").replace("zone-pr-m2", "zone-pr-nope"), "TypeError"),
        (edit("set X_TOKEN=abc now"), "PermissionError"),
        (edit(&big), "PermissionError"),
        (format!("await engine('chats.patch', chat_id='zone-pr', ops=[{many}])"), "TypeError"),
        (format!("await engine('chats.patch', chat_id='zone-pr', ops=[{total}])"), "PermissionError"),
        ("await engine('chats.patch', chat_id='zone-pr', ops=[{'op':'edit','message_id':'zone-pr-m2','text':'zone x','extra':{}}])".to_string(), "TypeError"),
        ("await engine('chats.patch', chat_id='zone-pr', ops=[{'op':'remove','message_id':'zone-pr-m4'},{'op':'edit','message_id':'zone-pr-m4','text':'zone x'}])".to_string(), "TypeError"),
        ("await engine('chats.patch', chat_id='zone-pr', expected_digest='h1-stale', ops=[{'op':'edit','message_id':'zone-pr-m2','text':'zone x'}])".to_string(), "TypeError"),
        ("await engine('chats.patch', chat_id='zone-pr', ops=[{'op':'insert','after':'zone-pr-m1','role':'system','text':'zone sys'}])".to_string(), "PermissionError"),
    ] {
        assert_eq!(refused(&app, &code).await, kind, "{}", &code[..code.len().min(160)]);
    }
    let same = result(
        &run(
            &app,
            "(await engine('chats.patch', chat_id='zone-pr', ops=[{'op':'edit','message_id':'zone-pr-m2','text':'zone second question'}]))['text']",
        )
        .await,
    );
    assert_eq!(same, "nothing to change");
}

#[tokio::test]
async fn patch_refuses_busy_targets_and_targets_it_does_not_own() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    type Setup = fn(&mut ChatSession);
    let cases: Vec<(&str, Setup)> = vec![
        ("zone-pb-gen", |s| {
            s.runtime.state = refact_chat_api::SessionState::Generating
        }),
        ("zone-pb-paused", |s| s.runtime.paused = true),
        ("zone-pb-queue", |s| {
            super::chats_risky::hold_queue(s);
            s.command_queue.push_back(
                serde_json::from_value(json!({"client_request_id":"q","type":"regenerate"}))
                    .unwrap(),
            );
        }),
        ("zone-pb-taken", |s| s.commands_taken = 1),
        ("zone-pb-delivery", |s| {
            s.pending_deliveries
                .push_back(refact_chat_api::PendingDelivery::new(
                    vec![ChatMessage::new("user".into(), "zone later".into())],
                    refact_chat_api::PushMode::WhenIdle,
                    "scheduler.cron",
                    true,
                ))
        }),
        ("zone-pb-wake", |s| {
            s.delivery_wake_sources.insert("external");
        }),
        ("zone-pb-recovery", |s| {
            s.runtime.recovery = Some(serde_json::from_value(json!({
                "operation_id":"o","run_id":"r","revision":1,"state":"waiting",
                "policy":{"mode":"transient","max_retries":1,"delay_ms":0,"total_timeout_ms":1000},
                "policy_source":"global","manual":false,"retries_used":0,
                "latest_error":{"category":"c","title":"t","explanation":"e","suggested_action":"s","is_retryable":true,"raw_error":"r"},
                "next_retry_at_ms":null,"deadline_at_ms":null,"terminal_reason":null,"attempts":[]
            })).unwrap())
        }),
        ("zone-pb-window", |s| {
            let mut call = message("zone-pb-window-m5", "assistant", "zone calling");
            call.tool_calls = Some(vec![serde_json::from_value(json!({"id":"call_open","type":"function","function":{"name":"cat","arguments":"{}"}})).unwrap()]);
            s.messages.push(call);
        }),
        ("zone-pb-runner", |s| {
            s.runner_lease = Some((
                "lease".into(),
                Arc::new(std::sync::atomic::AtomicBool::new(true)),
            ))
        }),
        ("zone-pb-buddy", |s| s.thread.mode = "buddy".into()),
    ];
    for (id, setup) in cases {
        let target = other(&app, id, setup).await;
        let before = other_fingerprint(&*target.lock().await);
        assert_eq!(
            refused(
                &app,
                &format!("await engine('chats.patch', chat_id='{id}', ops=[{{'op':'edit','message_id':'{id}-m2','text':'zone x'}}])")
            )
            .await,
            "PermissionError",
            "{id}"
        );
        assert_eq!(before, other_fingerprint(&*target.lock().await), "{id}");
    }
    for role in ["planner", "agent"] {
        let id = format!("zone-pb-task-{role}");
        let target = other(&app, &id, |_| {}).await;
        target.lock().await.thread.task_meta =
            Some(serde_json::from_value(json!({"task_id":"t","role":role})).unwrap());
        assert_eq!(
            refused(
                &app,
                &format!("await engine('chats.patch', chat_id='{id}', ops=[{{'op':'edit','message_id':'{id}-m2','text':'zone x'}}])")
            )
            .await,
            "PermissionError",
            "{id}"
        );
    }
    let mut foreign = build("zone-pb-foreign", |_| {});
    foreign.work_project_root = Some(std::env::temp_dir());
    app.chat.sessions.write().await.insert(
        "zone-pb-foreign".into(),
        Arc::new(tokio::sync::Mutex::new(foreign)),
    );
    assert_eq!(
        refused(
            &app,
            "await engine('chats.patch', chat_id='zone-pb-foreign', ops=[{'op':'edit','message_id':'zone-pb-foreign-m2','text':'zone x'}])"
        )
        .await,
        "PermissionError",
        "a chat of another project"
    );
}

#[tokio::test]
async fn patch_refuses_a_chat_whose_processor_took_a_command_that_has_not_started() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-pg", |_| {}).await;
    let (entered, resume) = (
        Arc::new(tokio::sync::Notify::new()),
        Arc::new(tokio::sync::Notify::new()),
    );
    *crate::chat::queue::COMMAND_TAKEN_GATE.lock().unwrap() =
        Some(("zone-pg".into(), entered.clone(), resume.clone()));
    {
        let mut s = target.lock().await;
        s.command_queue.push_back(
            serde_json::from_value(
                json!({"client_request_id":"gap","type":"set_params","patch":{"title":"zone gap"}}),
            )
            .unwrap(),
        );
        let running = s.queue_processor_running.clone();
        running.store(true, std::sync::atomic::Ordering::SeqCst);
        tokio::spawn(crate::chat::process_command_queue(
            app.clone(),
            target.clone(),
            running,
        ));
    }
    tokio::time::timeout(Duration::from_secs(20), entered.notified())
        .await
        .expect("the processor did not take the command");
    {
        let s = target.lock().await;
        assert!(
            s.command_queue.is_empty() && s.turn_finished(),
            "the gap was not reached"
        );
    }
    assert_eq!(
        refused(
            &app,
            "await engine('chats.patch', chat_id='zone-pg', ops=[{'op':'edit','message_id':'zone-pg-m2','text':'zone x'}])"
        )
        .await,
        "PermissionError"
    );
    *crate::chat::queue::COMMAND_TAKEN_GATE.lock().unwrap() = None;
    resume.notify_one();
    super::chats_risky::until(&target, |s| {
        s.thread.title == "zone gap" && s.commands_taken == 0
    })
    .await;
}

#[tokio::test]
async fn patch_carries_run_records_into_new_text_and_checks_egress_first() {
    let (dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-pe", |_| {}).await;
    let mut policy = guarded_policy(&app);
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy.clone())
        .await
        .unwrap();
    std::fs::write(dir.path().join("project/zone_a.txt"), "guarded").unwrap();
    let code = "x = open('zone_a.txt').read()\nawait engine('chats.patch', chat_id='zone-pe', ops=[{'op':'insert','after':'end','role':'assistant','text':'zone ' + x}])";
    assert_eq!(refused(&app, code).await, "PermissionError");
    assert_eq!(target.lock().await.messages.len(), 5);
    policy.zones[0].send_to = vec!["*".into()];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
    approve(&app, code).await;
    let s = target.lock().await;
    assert_eq!(s.messages.len(), 6);
    assert!(
        zones(&s.messages[5]).contains("zone_a"),
        "{:?}",
        s.messages[5].extra
    );
}

#[tokio::test]
async fn patch_of_the_system_prompt_syncs_the_frozen_prefix_and_resets_provider_state() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let tools = json!([{"type":"function","function":{"name":"cat"}}]);
    let target = other(&app, "zone-ps", |s| {
        s.messages
            .insert(0, message("zone-ps-sys", "system", "zone old prompt"));
        s.thread.frozen_request_prefix = Some(refact_chat_api::FrozenRequestPrefix {
            schema_version: 1,
            created_at: "2026-01-01T00:00:00Z".into(),
            system_prompt: Some("zone old prompt".into()),
            tools_canonical: Some(json!([{"type":"function","function":{"name":"cat"}}])),
        });
        s.thread.previous_response_id = Some("resp_1".into());
    })
    .await;
    let (request, _) = approve(
        &app,
        "await engine('chats.patch', chat_id='zone-ps', ops=[{'op':'edit','message_id':'zone-ps-sys','text':'zone new prompt'}])",
    )
    .await;
    let card = request.engine.unwrap();
    assert!(card.warning.unwrap().contains("system prompt"));
    assert!(card
        .items
        .iter()
        .any(|i| i.label == "system prompt (frozen copy)"));
    let s = target.lock().await;
    let prefix = s.thread.frozen_request_prefix.clone().unwrap();
    assert_eq!(prefix.system_prompt.as_deref(), Some("zone new prompt"));
    assert_eq!(prefix.tools_canonical, Some(tools.clone()));
    assert_eq!(s.messages[0].content.content_text_only(), "zone new prompt");
    assert!(s.thread.previous_response_id.is_none());
    assert!(s.thread.claude_code_identity.is_none());
    let prepared = crate::chat::cache_stability_tests::prepare_request(
        app.gcx.clone(),
        "zone/model-a",
        s.messages.clone(),
        Vec::new(),
        s.thread.frozen_request_prefix.clone(),
        false,
    )
    .await;
    let systems: Vec<String> = prepared
        .llm_request
        .messages
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| m.content.content_text_only())
        .collect();
    assert_eq!(
        systems,
        ["zone new prompt"],
        "the next request carries exactly the approved prompt"
    );
    assert_eq!(
        prepared.llm_request.tools.clone().map(Value::Array),
        Some(tools),
        "and the same tools"
    );
    drop(s);
    let other_edit = other(&app, "zone-ps2", |s| {
        s.thread.frozen_request_prefix = Some(refact_chat_api::FrozenRequestPrefix {
            schema_version: 1,
            created_at: "2026-01-01T00:00:00Z".into(),
            system_prompt: Some("zone kept".into()),
            tools_canonical: None,
        });
    })
    .await;
    approve(
        &app,
        "await engine('chats.patch', chat_id='zone-ps2', ops=[{'op':'edit','message_id':'zone-ps2-m0','text':'zone other'}])",
    )
    .await;
    assert_eq!(
        other_edit
            .lock()
            .await
            .thread
            .frozen_request_prefix
            .as_ref()
            .unwrap()
            .system_prompt
            .as_deref(),
        Some("zone kept"),
        "a patch that does not edit the system message keeps the frozen prefix"
    );
}

#[tokio::test]
async fn patch_keeps_removed_delivery_ids_in_the_duplicate_filter() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let delivery = refact_chat_api::PendingDelivery::new(
        vec![message("zone-pv-d", "user", "zone delivered")],
        refact_chat_api::PushMode::WhenIdle,
        "scheduler.cron",
        true,
    );
    let id = delivery.id.clone();
    let target = other(&app, "zone-pv", |s| {
        s.messages.extend(delivery.stamp_messages())
    })
    .await;
    let (request, _) = approve(
        &app,
        "await engine('chats.patch', chat_id='zone-pv', ops=[{'op':'remove','message_id':'zone-pv-d'}])",
    )
    .await;
    assert!(request.engine.is_some());
    let s = target.lock().await;
    assert!(s.messages.iter().all(|m| m.message_id != "zone-pv-d"));
    assert!(s.dropped_deliveries.iter().any(|d| d.id == id), "in memory");
    drop(s);
    let saved = crate::chat::trajectories::load_trajectory_for_chat(app.gcx.clone(), "zone-pv")
        .await
        .unwrap();
    assert!(
        saved.dropped_deliveries.iter().any(|d| d.id == id),
        "on disk"
    );
}

#[tokio::test]
async fn patch_warns_about_goals_and_keeps_text_off_the_call_summary() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-pw", |_| {}).await;
    let _ = target.lock().await.install_goal(
        "agent",
        "zone keep going",
        true,
        refact_chat_api::GoalBudget::default(),
    );
    let (request, _) = denied_run(
        &app,
        "try:\n    await engine('chats.patch', chat_id='zone-pw', ops=[{'op':'insert','after':'zone-pw-m2','role':'assistant','text':'zone done','tool_calls':[{'name':'cat','arguments':{},'result':'zone ok'}]}])\nexcept PermissionError:\n    pass",
    )
    .await;
    let warning = request.engine.unwrap().warning.unwrap();
    assert!(warning.contains("active goal"), "{warning}");
    assert!(warning.contains("evidence"), "{warning}");
    let (request, m) = allowed_run(
        &app,
        "r = await engine('chats.patch', chat_id='zone-pw', ops=[{'op':'edit','message_id':'zone-pw-m2','text':'zone changed'}])\nr['ok']",
    )
    .await;
    assert_eq!(result(&m), true);
    assert_eq!(
        request.engine.unwrap().items[0].old,
        Some(json!("zone second question")),
        "the card shows the old text"
    );
    let stored = serde_json::to_string(&m).unwrap();
    assert!(
        !stored.contains("zone second question"),
        "the call summary or the stored change items hold the other chat's text: {stored}"
    );
    assert!(m.extra["python"]["engine_changes"]
        .to_string()
        .contains("#2 user: edit"));
}

/// D12 and P1: the records of the text a short form's card shows travel into this run. The retry
/// card shows where it cuts but not that message's text, so nothing of that chat reaches this
/// one and a retry stays possible from a script that read nothing guarded.
#[tokio::test]
async fn short_history_forms_carry_the_records_of_the_text_their_card_shows() {
    for (function, code, shows_text) in [
        (
            "chats.edit_message",
            "await engine('chats.edit_message', chat_id='zone-sf', message_id='zone-sf-m2', text='zone x')",
            true,
        ),
        (
            "chats.remove_message",
            "await engine('chats.remove_message', chat_id='zone-sf', message_id='zone-sf-m2')",
            true,
        ),
        (
            "chats.retry_from",
            "await engine('chats.retry_from', chat_id='zone-sf', index=2, text='zone x')",
            false,
        ),
    ] {
        let (_dir, app, _) = fixture().await;
        install(&app).await;
        other(&app, "zone-sf", |s| {
            crate::privacy::records::attach_record(
                &mut s.messages[2],
                record("/x/zone_secret.txt", "zone_secret"),
            )
        })
        .await;
        let (request, m) = denied_run(
            &app,
            &format!("try:\n    {code}\nexcept PermissionError:\n    pass"),
        )
        .await;
        let card = serde_json::to_string(&request.engine.unwrap()).unwrap();
        assert_eq!(card.contains("zone second question"), shows_text, "{function}: {card}");
        assert_eq!(
            zones(&m).contains("zone_secret"),
            shows_text,
            "{function}: the records of the text the card showed travel, and only those: {:?}",
            m.extra
        );
        assert!(
            !serde_json::to_string(&m)
                .unwrap()
                .contains("zone second question"),
            "{function}"
        );
    }
}

/// A1 and A5 for the short forms: their previews plan from the saved file without opening the
/// chat, a form that starts a turn shows what that chat may do, and the handler opens the chat
/// only after approval and says so.
#[tokio::test]
async fn short_forms_plan_a_saved_chat_without_opening_it() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    save(
        &app,
        &build("zone-sa", |s| {
            s.thread.auto_approve_dangerous_commands = true
        }),
    )
    .await;
    let card = deny(
        &app,
        "await engine('chats.retry_from', chat_id='zone-sa', index=2, text='zone again')",
    )
    .await
    .engine
    .unwrap();
    assert!(card.items.iter().any(|i| i.label == "approval flags"));
    assert!(card.items.iter().any(|i| i.label == "goal"));
    let cut = card
        .items
        .iter()
        .find(|i| i.label == "cut at message")
        .unwrap();
    assert_eq!(cut.old, Some(json!("#2 user, 20 characters")));
    let warning = card.warning.unwrap();
    assert!(
        warning.starts_with("Messages after this point in that chat will be deleted"),
        "{warning}"
    );
    assert!(warning.contains("without asking"), "{warning}");
    assert!(
        !loaded(&app, "zone-sa").await,
        "the preview opened the chat"
    );
    for code in [
        "await engine('chats.edit_message', chat_id='zone-sa', message_id='zone-sa-m2', text='zone x')",
        "await engine('chats.remove_message', chat_id='zone-sa', message_id='zone-sa-m2')",
    ] {
        deny(&app, code).await;
        assert!(!loaded(&app, "zone-sa").await, "{code}");
    }
    let (_, out) = approve(
        &app,
        "(await engine('chats.edit_message', chat_id='zone-sa', message_id='zone-sa-m2', text='zone edited'))['data']",
    )
    .await;
    assert_eq!(out["opened"], true, "{out}");
    assert!(loaded(&app, "zone-sa").await);
}

/// P2: a find fragment is text of the other chat, so the call file and the call record keep
/// only its size.
#[tokio::test]
async fn a_find_fragment_stays_off_the_call_file_and_record() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    other(&app, "zone-pf2", |_| {}).await;
    let (_, m) = denied_run(
        &app,
        "try:\n    await engine('chats.patch', chat_id='zone-pf2', ops=[{'op':'edit','message_id':'zone-pf2-m2','find':'second question','replace':'other words'}])\nexcept PermissionError:\n    pass",
    )
    .await;
    let run_dir = std::path::PathBuf::from(m.extra["python"]["run_dir"].as_str().unwrap());
    let calls: Vec<String> = std::fs::read_dir(run_dir.join("calls"))
        .unwrap()
        .flatten()
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
        .collect();
    assert!(!calls.is_empty());
    for call in calls.iter().chain([&serde_json::to_string(&m).unwrap()]) {
        assert!(!call.contains("second question"), "{call}");
    }
    assert!(calls.iter().any(|call| call.contains("other words")));
}

/// T1: once an approved patch renamed its file into place, the patch also reaches that chat's
/// live history, even when the call that started it is gone (a timeout or a stopped run), and a
/// timeout of that call would say that the change is being completed.
#[tokio::test]
async fn a_patch_whose_call_ends_after_the_rename_still_completes_its_live_history() {
    let (dir, app, ccx) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-pt1", |_| {}).await;
    let (renamed, release) = (
        Arc::new(tokio::sync::Notify::new()),
        Arc::new(tokio::sync::Notify::new()),
    );
    crate::chat::trajectories::test_rename_hooks()
        .lock()
        .unwrap()
        .insert("zone-pt1".into(), (renamed.clone(), release.clone()));
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
        completing: Default::default(),
    };
    let function = super::super::lookup("chats.patch").unwrap();
    let args = json!({"chat_id":"zone-pt1","ops":[{"op":"edit","message_id":"zone-pt1-m2","text":"zone after the rename"}]});
    let preview = (function.preview.unwrap())(&ctx, &args).await.unwrap();
    *ctx.plan.lock().unwrap() = Some(preview.plan);
    {
        let call = (function.handler)(&ctx, &args);
        tokio::pin!(call);
        tokio::select! {
            _ = &mut call => panic!("the patch ended before its file was renamed"),
            _ = renamed.notified() => {}
            _ = tokio::time::sleep(Duration::from_secs(20)) => panic!("the patch did not reach its rename"),
        }
    }
    release.notify_one();
    super::chats_risky::until(&target, |s| {
        s.messages[2].content.content_text_only() == "zone after the rename"
    })
    .await;
    let (stored, dirty) = {
        let s = target.lock().await;
        (texts(&s), s.trajectory_dirty)
    };
    assert!(!dirty, "the live history was not committed");
    let saved = crate::chat::trajectories::load_trajectory_for_chat(app.gcx.clone(), "zone-pt1")
        .await
        .unwrap();
    let saved_texts: Vec<String> = saved
        .messages
        .iter()
        .map(|m| format!("{}:{}", m.role, m.content.content_text_only()))
        .collect();
    assert_eq!(
        saved_texts, stored,
        "the saved file and the live history differ"
    );
    assert!(
        ctx.completing
            .lock()
            .unwrap()
            .as_deref()
            .is_some_and(|note| note.contains("completes it")),
        "a timeout of that call would not say that the change is being completed"
    );
}

/// T17: a frozen prefix that keeps tools but no system prompt is partial: a system edit and a
/// system insert are refused before any card.
#[tokio::test]
async fn a_tools_only_frozen_prefix_refuses_system_changes() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let tools_only = || refact_chat_api::FrozenRequestPrefix {
        schema_version: 1,
        created_at: "2026-01-01T00:00:00Z".into(),
        system_prompt: None,
        tools_canonical: Some(json!([{"type":"function","function":{"name":"cat"}}])),
    };
    other(&app, "zone-ft", |s| {
        s.messages
            .insert(0, message("zone-ft-sys", "system", "zone prompt"));
        s.thread.frozen_request_prefix = Some(tools_only());
    })
    .await;
    other(&app, "zone-fi", |s| {
        s.thread.frozen_request_prefix = Some(tools_only());
    })
    .await;
    for code in [
        "await engine('chats.patch', chat_id='zone-ft', ops=[{'op':'edit','message_id':'zone-ft-sys','text':'zone other prompt'}])",
        "await engine('chats.patch', chat_id='zone-fi', ops=[{'op':'insert','after':'start','role':'system','text':'zone new prompt'}])",
    ] {
        assert_eq!(refused(&app, code).await, "PermissionError", "{code}");
    }
}

/// T18: the short forms show the usage, cost and checkpoints of the suffix they discard, and
/// keep the delivery ids of what they remove in that chat's duplicate filter.
#[tokio::test]
async fn short_forms_show_what_their_cut_discards_and_keep_its_delivery_ids() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let delivery = refact_chat_api::PendingDelivery::new(
        vec![message("zone-sc-d", "user", "zone delivered")],
        refact_chat_api::PushMode::WhenIdle,
        "scheduler.cron",
        true,
    );
    let id = delivery.id.clone();
    let target = other(&app, "zone-sc", |s| {
        super::chats_risky::hold_queue(s);
        s.messages[3].usage = Some(
            serde_json::from_value(
                json!({"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}),
            )
            .unwrap(),
        );
        s.messages[3].checkpoints = vec![serde_json::from_value(
            json!({"workspace_folder":"/x","commit_hash":"abcdef0123456789"}),
        )
        .unwrap()];
        s.messages.extend(delivery.stamp_messages());
    })
    .await;
    let (request, _) = approve(
        &app,
        "await engine('chats.retry_from', chat_id='zone-sc', index=2, text='zone again')",
    )
    .await;
    let card = request.engine.unwrap();
    let usage = card
        .items
        .iter()
        .find(|i| i.label == "recorded usage removed")
        .unwrap();
    assert_eq!(usage.old, Some(json!("15 tokens, $0.0000")));
    let checkpoints = card
        .items
        .iter()
        .find(|i| i.label == "file checkpoints removed")
        .unwrap();
    assert_eq!(checkpoints.old, Some(json!("#3: abcdef012345")));
    assert!(
        target
            .lock()
            .await
            .dropped_deliveries
            .iter()
            .any(|d| d.id == id),
        "the removed delivery id left the duplicate filter"
    );
}

/// T19: find and replace between two secrets it does not touch is allowed; a splice that makes a
/// new one is still refused.
#[tokio::test]
async fn find_and_replace_between_untouched_secrets_is_allowed() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-fx", |s| {
        s.messages[2] = message(
            "zone-fx-m2",
            "user",
            "X_TOKEN=first\nkeep this sentence\nZ_TOKEN=second",
        )
    })
    .await;
    approve(
        &app,
        "await engine('chats.patch', chat_id='zone-fx', ops=[{'op':'edit','message_id':'zone-fx-m2','find':'keep this sentence','replace':'keep that sentence'}])",
    )
    .await;
    assert_eq!(
        target.lock().await.messages[2].content.content_text_only(),
        "X_TOKEN=first\nkeep that sentence\nZ_TOKEN=second"
    );
    assert_eq!(
        refused(
            &app,
            "await engine('chats.patch', chat_id='zone-fx', ops=[{'op':'edit','message_id':'zone-fx-m2','find':'that sentence','replace':'Y_TOKEN=made'}])",
        )
        .await,
        "PermissionError"
    );
}

/// T20: a saved chat is opened after approval only from exactly the file the plan was made from:
/// a file replaced between the answer and the open leaves the chat closed and unwritten.
#[tokio::test]
async fn a_saved_chat_replaced_before_it_is_opened_is_not_opened() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    save(&app, &build("zone-po2", |_| {})).await;
    let path = crate::chat::trajectories::find_trajectory_path(app.gcx.clone(), "zone-po2")
        .await
        .unwrap();
    let (prepared, release) = (
        Arc::new(tokio::sync::Notify::new()),
        Arc::new(tokio::sync::Notify::new()),
    );
    super::super::chats::OPEN_GATE.lock().unwrap().push((
        "zone-po2".into(),
        prepared.clone(),
        release.clone(),
    ));
    let task = spawn(
        &app,
        "r = await engine('chats.patch', chat_id='zone-po2', ops=[{'op':'edit','message_id':'zone-po2-m2','text':'zone x'}])\n[r['ok'], r.get('error')]",
    );
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), prepared.notified())
        .await
        .expect("the patch did not reach the open");
    let mut newer = build("zone-po2", |_| {});
    newer.thread.title = "zone renamed elsewhere".into();
    save(&app, &newer).await;
    let replaced = std::fs::read(&path).unwrap();
    release.notify_one();
    let out = result(&finish(task).await);
    assert_eq!(out[0], false, "{out}");
    assert!(
        out[1].as_str().unwrap().contains("changed on disk"),
        "{out}"
    );
    assert!(
        !loaded(&app, "zone-po2").await,
        "a stale pin opened the chat"
    );
    assert_eq!(std::fs::read(&path).unwrap(), replaced);
}

/// T22: a saved file over the size limit is refused before it is parsed.
#[tokio::test]
async fn an_oversized_saved_file_is_refused_before_it_is_parsed() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    save(&app, &build("zone-big", |_| {})).await;
    let path = crate::chat::trajectories::find_trajectory_path(app.gcx.clone(), "zone-big")
        .await
        .unwrap();
    let file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    file.set_len(97 * 1024 * 1024).unwrap();
    drop(file);
    let m = run(
        &app,
        "try:\n    await engine('chats.patch', chat_id='zone-big', ops=[{'op':'edit','message_id':'zone-big-m2','text':'zone x'}])\n    r = 'ok'\nexcept PermissionError as e:\n    r = str(e)\nr",
    )
    .await;
    let out = result(&m);
    assert!(out.as_str().unwrap().contains("over 96 MiB"), "{out}");
}

/// T24, K2: the guarded save itself refuses when another engine changed the file after the
/// patch was planned under the lock: nothing is applied in memory or on disk.
#[tokio::test]
async fn the_guarded_save_refuses_a_file_another_engine_changed_after_the_plan() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-gs2", |_| {}).await;
    let before = other_fingerprint(&*target.lock().await);
    let path = crate::chat::trajectories::find_trajectory_path(app.gcx.clone(), "zone-gs2")
        .await
        .unwrap();
    let (prepared, release) = (
        Arc::new(tokio::sync::Notify::new()),
        Arc::new(tokio::sync::Notify::new()),
    );
    crate::chat::trajectories::test_snapshot_prepared_hooks()
        .lock()
        .unwrap()
        .insert(path.clone(), (prepared.clone(), release.clone()));
    let task = spawn(
        &app,
        "r = await engine('chats.patch', chat_id='zone-gs2', ops=[{'op':'edit','message_id':'zone-gs2-m2','text':'zone x'}])\n[r['ok'], r.get('error')]",
    );
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), prepared.notified())
        .await
        .expect("the patch did not reach its save");
    let mut newer = build("zone-gs2", |_| {});
    newer
        .messages
        .push(message("zone-gs2-m9", "user", "zone written elsewhere"));
    save(&app, &newer).await;
    release.notify_one();
    let out = result(&finish(task).await);
    assert_eq!(out[0], false, "{out}");
    assert!(
        out[1]
            .as_str()
            .unwrap()
            .contains("changed on disk since the change was planned"),
        "{out}"
    );
    assert_eq!(before, other_fingerprint(&*target.lock().await));
    let saved = crate::chat::trajectories::load_trajectory_for_chat(app.gcx.clone(), "zone-gs2")
        .await
        .unwrap();
    assert_eq!(
        saved.messages.len(),
        6,
        "the other engine's file was overwritten"
    );
}

/// T24: apply_patch checks the live chat again under its lock: a message that lands after the
/// answer stops the patch even though the planned items still match.
#[tokio::test]
async fn a_chat_that_moves_before_the_patch_takes_its_lock_is_refused() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-pl", |_| {}).await;
    let (entered, release) = (
        Arc::new(tokio::sync::Notify::new()),
        Arc::new(tokio::sync::Notify::new()),
    );
    super::super::chats::PATCH_LOCK_GATE.lock().unwrap().push((
        "zone-pl".into(),
        entered.clone(),
        release.clone(),
    ));
    let task = spawn(
        &app,
        "r = await engine('chats.patch', chat_id='zone-pl', ops=[{'op':'edit','message_id':'zone-pl-m2','text':'zone x'}])\n[r['ok'], r.get('error')]",
    );
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), entered.notified())
        .await
        .expect("the patch did not reach its lock");
    target
        .lock()
        .await
        .messages
        .push(message("zone-pl-m5", "assistant", "zone late answer"));
    release.notify_one();
    let out = result(&finish(task).await);
    assert_eq!(out[0], false, "{out}");
    assert!(
        out[1]
            .as_str()
            .unwrap()
            .contains("changed since the preview"),
        "{out}"
    );
    assert_eq!(
        target.lock().await.messages[2].content.content_text_only(),
        "zone second question"
    );
}

/// T24, D2/A3: the user messages edit_message and retry_from write are stamped as written by a
/// script, so their text stays literal.
#[tokio::test]
async fn edit_message_and_retry_from_stamp_what_they_write() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let edited = other(&app, "zone-se", |_| {}).await;
    approve(
        &app,
        "await engine('chats.edit_message', chat_id='zone-se', message_id='zone-se-m2', text='zone @web edited')",
    )
    .await;
    super::chats_risky::until(&edited, |s| {
        s.messages
            .iter()
            .any(|m| m.content.content_text_only() == "zone @web edited")
    })
    .await;
    let retried = other(&app, "zone-sr", |_| {}).await;
    approve(
        &app,
        "await engine('chats.retry_from', chat_id='zone-sr', index=2, text='zone @web again')",
    )
    .await;
    super::chats_risky::until(&retried, |s| {
        s.messages
            .iter()
            .any(|m| m.content.content_text_only() == "zone @web again")
    })
    .await;
    for (target, text) in [(&edited, "zone @web edited"), (&retried, "zone @web again")] {
        let s = target.lock().await;
        let written = s
            .messages
            .iter()
            .find(|m| m.content.content_text_only() == text)
            .unwrap();
        assert!(
            written.extra[refact_chat_api::DELIVERY_EXTRA_KEY]["source"]
                .as_str()
                .is_some_and(|source| source.starts_with("engine.")),
            "{text}: {:?}",
            written.extra
        );
    }
}

/// T6: a patch carries the settings it was approved under to the lock that applies it: a mode
/// or approval flag changed after the answer applies nothing.
#[tokio::test]
async fn a_patch_whose_settings_change_before_its_lock_applies_nothing() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-ps6", |_| {}).await;
    let (entered, release) = (
        Arc::new(tokio::sync::Notify::new()),
        Arc::new(tokio::sync::Notify::new()),
    );
    super::super::chats::PATCH_LOCK_GATE.lock().unwrap().push((
        "zone-ps6".into(),
        entered.clone(),
        release.clone(),
    ));
    let task = spawn(
        &app,
        "r = await engine('chats.patch', chat_id='zone-ps6', ops=[{'op':'edit','message_id':'zone-ps6-m2','text':'zone x'}])\n[r['ok'], r.get('error')]",
    );
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), entered.notified())
        .await
        .expect("the patch did not reach its lock");
    target.lock().await.thread.auto_approve_dangerous_commands = true;
    release.notify_one();
    let out = result(&finish(task).await);
    assert_eq!(out[0], false, "{out}");
    assert!(out[1].as_str().unwrap().contains("approval flags"), "{out}");
    assert_eq!(
        target.lock().await.messages[2].content.content_text_only(),
        "zone second question"
    );
}
