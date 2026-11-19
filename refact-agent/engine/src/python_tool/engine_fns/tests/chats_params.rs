use super::chats_risky::{
    approve, build, deny, install, moved_then_allowed, other, other_fingerprint, refused, save,
    until,
};
use super::*;
use crate::chat::types::ChatSession;
use refact_chat_api::PythonEngineClass as Class;

/// The old and new value of the card item `label`.
fn shown(
    engine: &refact_chat_api::PythonEngineRequest,
    label: &str,
) -> (Option<Value>, Option<Value>) {
    let item = engine
        .items
        .iter()
        .find(|item| item.label == label)
        .unwrap_or_else(|| panic!("no {label} item: {:?}", engine.items));
    (item.old.clone(), item.new.clone())
}

#[tokio::test]
async fn set_params_turns_approval_flags_on_with_a_risky_card() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-pf", |_| {}).await;
    let (request, out) = approve(
        &app,
        "await engine('chats.set_params', chat_id='zone-pf', editing_tools=True, all_confirmations=True, temperature=0.5)",
    )
    .await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.class, Class::Risky);
    let warning = engine.warning.clone().unwrap();
    assert!(warning.starts_with("zone-pf title: "), "{warning}");
    assert!(
        warning.contains("Edits to files will be applied without asking you."),
        "{warning}"
    );
    assert!(
        warning.contains("Every tool except agent control will run without asking you."),
        "{warning}"
    );
    assert!(!warning.contains("outside this chat's tree"), "{warning}");
    let labels: Vec<&str> = engine.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(
        labels,
        ["temperature", "editing_tools", "all_confirmations"]
    );
    assert_eq!(out["ok"], true, "{out}");
    assert_eq!(out["data"]["opened"], false, "{out}");
    assert!(
        out["text"]
            .as_str()
            .unwrap()
            .contains("never affect engine functions"),
        "{out}"
    );
    for key in [
        "auto_approve_editing_tools",
        "autonomous_no_confirm",
        "temperature",
    ] {
        assert!(out["data"]["changed"][key].is_object(), "{key}: {out}");
    }
    until(&target, |s| {
        s.thread.auto_approve_editing_tools && s.thread.autonomous_no_confirm
    })
    .await;
    assert!(!target.lock().await.thread.auto_approve_dangerous_commands);
}

#[tokio::test]
async fn a_flag_given_with_a_mode_wins_over_the_default_of_that_mode() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-pm", |_| {}).await;
    let (request, out) = approve(
        &app,
        "await engine('chats.set_params', chat_id='zone-pm', mode='agent', editing_tools=False)",
    )
    .await;
    let engine = request.engine.unwrap();
    assert!(
        engine
            .items
            .iter()
            .all(|i| i.label != "editing_tools" && i.label != "auto_approve_editing_tools"),
        "{:?}",
        engine.items
    );
    assert_eq!(out["ok"], true, "{out}");
    until(&target, |s| s.thread.mode == "agent").await;
    assert!(!target.lock().await.thread.auto_approve_editing_tools);
}

/// D1 and A9: turning approvals on or widening the mode of a chat outside this chat's tree is a
/// risky card that names that chat, its tree, project and worktree, not a refusal. Replaces the
/// former `chats_set_params_cannot_turn_on_approvals_in_another_tree` and
/// `chats_set_params_cannot_widen_the_mode_of_another_tree`, whose contract D1 changed.
#[tokio::test]
async fn another_tree_gets_a_risky_card_that_names_its_tree_project_and_worktree() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-d1", |s| {
        s.thread.root_chat_id = Some("zone-elsewhere".into())
    })
    .await;
    let engine = deny(
        &app,
        "await engine('chats.set_params', chat_id='zone-d1', mode='agent')",
    )
    .await
    .engine
    .unwrap();
    assert_eq!(engine.class, Class::Risky);
    let warning = engine.warning.clone().unwrap();
    assert!(
        warning.starts_with(
            "zone-d1 title (zone-d1) is outside this chat's tree (tree zone-elsewhere, project "
        ),
        "{warning}"
    );
    assert!(
        warning.contains("The new mode lets tools run without asking you."),
        "{warning}"
    );
    assert_eq!(shown(&engine, "chat").1, Some(json!("zone-d1")));
    assert_eq!(shown(&engine, "tree").1, Some(json!("zone-elsewhere")));
    let project = shown(&engine, "project").1.unwrap();
    assert_ne!(project, json!("unknown"));
    assert!(warning.contains(project.as_str().unwrap()), "{warning}");
    assert_eq!(
        shown(&engine, "worktree").1,
        Some(json!("none (the project folder)"))
    );
    assert_eq!(
        shown(&engine, "mode"),
        (Some(json!("explore")), Some(json!("agent")))
    );
    assert_eq!(
        shown(&engine, "editing_tools"),
        (Some(json!(false)), Some(json!(true)))
    );
    for flag in ["dangerous_commands", "all_confirmations"] {
        assert_eq!(
            shown(&engine, flag),
            (Some(json!(false)), Some(json!(false))),
            "{flag}"
        );
    }
    {
        let s = target.lock().await;
        assert_eq!(s.thread.mode, "explore");
        assert!(s.command_queue.is_empty());
    }

    let engine = deny(
        &app,
        "await engine('chats.set_params', chat_id='zone-d1', mode='debug')",
    )
    .await
    .engine
    .unwrap();
    assert_eq!(engine.class, Class::Risky);
    assert!(engine
        .warning
        .unwrap()
        .contains("outside this chat's tree (tree zone-elsewhere"));

    let engine = deny(
        &app,
        "await engine('chats.set_auto_approve', chat_id='zone-d1', dangerous_commands=True)",
    )
    .await
    .engine
    .unwrap();
    assert_eq!(engine.class, Class::Risky);
    assert_eq!(engine.title, "change automatic approvals in another chat");
    assert!(engine
        .warning
        .clone()
        .unwrap()
        .contains("Shell commands and other confirmations will run without asking you."));
    assert_eq!(
        shown(&engine, "mode"),
        (Some(json!("explore")), Some(json!("explore")))
    );
    assert_eq!(
        shown(&engine, "dangerous_commands"),
        (Some(json!(false)), Some(json!(true)))
    );
    assert_eq!(
        shown(&engine, "editing_tools"),
        (Some(json!(false)), Some(json!(false)))
    );

    let engine = deny(
        &app,
        "await engine('chats.set_params', chat_id='zone-d1', temperature=0.4)",
    )
    .await
    .engine
    .unwrap();
    assert_eq!(engine.class, Class::Change);
    assert!(engine.warning.is_none());
    assert!(engine.items.iter().all(|i| i.label != "tree"));

    other(&app, "zone-d1-own", |_| {}).await;
    let engine = deny(
        &app,
        "await engine('chats.set_params', chat_id='zone-d1-own', mode='debug')",
    )
    .await
    .engine
    .unwrap();
    assert_eq!(engine.class, Class::Risky);
    let warning = engine.warning.unwrap();
    assert!(
        warning.contains("The new mode lets tools run without asking you."),
        "{warning}"
    );
    assert!(!warning.contains("outside this chat's tree"), "{warning}");
    assert!(engine.items.iter().all(|i| i.label != "tree"));

    let (request, out) = approve(
        &app,
        "await engine('chats.set_auto_approve', chat_id='zone-d1', editing_tools=True)",
    )
    .await;
    assert_eq!(request.engine.unwrap().class, Class::Risky);
    assert_eq!(out["ok"], true, "{out}");
    until(&target, |s| s.thread.auto_approve_editing_tools).await;
    let engine = deny(
        &app,
        "await engine('chats.set_auto_approve', chat_id='zone-d1', editing_tools=False)",
    )
    .await
    .engine
    .unwrap();
    assert_eq!(
        engine.class,
        Class::Change,
        "turning a flag off is an ordinary change"
    );
}

#[tokio::test]
async fn structural_and_internal_settings_are_type_errors() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-sk", |_| {}).await;
    for key in [
        "parent_id",
        "task_meta",
        "worktree",
        "link_type",
        "root_chat_id",
        "buddy_meta",
        "tool_use",
        "frozen_request_prefix",
        "title",
        "auto_approve_editing_tools",
    ] {
        assert_eq!(
            refused(
                &app,
                &format!("await engine('chats.set_params', chat_id='zone-sk', {key}='x')")
            )
            .await,
            "TypeError",
            "{key}"
        );
    }
    assert!(target.lock().await.command_queue.is_empty());
}

#[tokio::test]
async fn set_params_refuses_busy_targets_and_targets_it_does_not_own() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    type Setup = fn(&mut ChatSession);
    let cases: Vec<(&str, Setup)> = vec![
        ("zone-pp-gen", |s| {
            s.runtime.state = refact_chat_api::SessionState::Generating
        }),
        ("zone-pp-queue", |s| {
            super::chats_risky::hold_queue(s);
            s.command_queue.push_back(
                serde_json::from_value(json!({"client_request_id":"q","type":"regenerate"}))
                    .unwrap(),
            );
        }),
        ("zone-pp-taken", |s| s.commands_taken = 1),
        ("zone-pp-runner", |s| {
            s.runner_lease = Some((
                "lease".into(),
                Arc::new(std::sync::atomic::AtomicBool::new(true)),
            ))
        }),
        ("zone-pp-buddy", |s| s.thread.mode = "buddy".into()),
    ];
    for (id, setup) in cases {
        let target = other(&app, id, setup).await;
        let before = other_fingerprint(&*target.lock().await);
        for call in [
            format!("await engine('chats.set_params', chat_id='{id}', temperature=0.4)"),
            format!("await engine('chats.set_auto_approve', chat_id='{id}', editing_tools=True)"),
        ] {
            assert_eq!(refused(&app, &call).await, "PermissionError", "{call}");
        }
        assert_eq!(before, other_fingerprint(&*target.lock().await), "{id}");
    }
    for role in ["planner", "agent"] {
        let id = format!("zone-pp-task-{role}");
        let target = other(&app, &id, |_| {}).await;
        target.lock().await.thread.task_meta =
            Some(serde_json::from_value(json!({"task_id":"t","role":role})).unwrap());
        assert_eq!(
            refused(
                &app,
                &format!("await engine('chats.set_params', chat_id='{id}', temperature=0.4)")
            )
            .await,
            "PermissionError",
            "{id}"
        );
    }
}

/// A8: the settings are compared again under the lock that accepts the command, so a change that
/// lands after the answer and before that lock applies nothing.
#[tokio::test]
async fn settings_changed_right_before_the_change_is_accepted_are_refused() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let target = other(&app, "zone-a8", |_| {}).await;
    let hook: fn(&mut ChatSession) = |s| s.thread.auto_approve_dangerous_commands = true;
    crate::python_tool::engine_fns::chats::TEST_HOOKS
        .lock()
        .unwrap()
        .push(("zone-a8".into(), hook));
    let (_, out) = approve(
        &app,
        "r = await engine('chats.set_params', chat_id='zone-a8', temperature=0.4)\n[r['ok'], r.get('error')]",
    )
    .await;
    crate::python_tool::engine_fns::chats::TEST_HOOKS
        .lock()
        .unwrap()
        .retain(|(id, _)| id != "zone-a8");
    assert_eq!(out[0], false, "{out}");
    assert!(
        out[1]
            .as_str()
            .unwrap()
            .contains("changed since the preview"),
        "{out}"
    );
    let s = target.lock().await;
    assert!(s.command_queue.is_empty());
    assert!(s.thread.temperature.is_none());
}

/// A1: the preview of a saved chat reads its file without opening it, as opening would make it
/// (the mode's default approval flags of a file that does not store them); the chat is opened
/// only after approval and the result says so.
#[tokio::test]
async fn a_saved_chat_is_seen_as_opening_makes_it_and_opened_only_after_approval() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    save(
        &app,
        &build("zone-pss", |s| {
            s.thread.mode = "agent".into();
            s.thread.root_chat_id = Some("zone-elsewhere".into());
        }),
    )
    .await;
    let path = crate::chat::trajectories::find_trajectory_path(app.gcx.clone(), "zone-pss")
        .await
        .unwrap();
    let mut file: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for key in [
        "auto_approve_editing_tools",
        "auto_approve_dangerous_commands",
    ] {
        file.as_object_mut().unwrap().remove(key);
    }
    std::fs::write(&path, serde_json::to_vec_pretty(&file).unwrap()).unwrap();
    let call = "await engine('chats.set_params', chat_id='zone-pss', dangerous_commands=True)";
    let engine = deny(&app, call).await.engine.unwrap();
    assert_eq!(
        shown(&engine, "editing_tools"),
        (Some(json!(true)), Some(json!(true)))
    );
    assert!(
        !app.chat.sessions.read().await.contains_key("zone-pss"),
        "the preview opened the chat"
    );
    let (_, out) = approve(&app, call).await;
    assert_eq!(out["ok"], true, "{out}");
    assert_eq!(out["data"]["opened"], true, "{out}");
    assert!(
        out["text"]
            .as_str()
            .unwrap()
            .contains("the chat was opened"),
        "{out}"
    );
    let target = app.chat.sessions.read().await["zone-pss"].clone();
    until(&target, |s| {
        s.thread.auto_approve_dangerous_commands && s.thread.auto_approve_editing_tools
    })
    .await;
}

/// T10: the card of a mode switch in another tree lists each confirmation and permission that
/// changes, old and new, and every tool name added or removed.
#[tokio::test]
async fn a_mode_switch_card_lists_the_rules_permissions_and_tools_that_change() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    other(&app, "zone-mr", |s| {
        s.thread.root_chat_id = Some("zone-elsewhere".into())
    })
    .await;
    let engine = deny(
        &app,
        "await engine('chats.set_params', chat_id='zone-mr', mode='agent')",
    )
    .await
    .engine
    .unwrap();
    assert_eq!(
        shown(&engine, "permission allow_integrations"),
        (Some(json!(false)), Some(json!(true)))
    );
    let labels: Vec<&str> = engine.items.iter().map(|i| i.label.as_str()).collect();
    assert!(
        labels
            .iter()
            .any(|label| label.starts_with("confirmation ")),
        "{labels:?}"
    );
    assert!(!labels.contains(&"tool confirmation rules"), "{labels:?}");
    let added = shown(&engine, "tools added").1.unwrap();
    let (count, names) = added.as_str().unwrap().split_once(" (").unwrap();
    assert_eq!(
        count.parse::<usize>().unwrap(),
        names.trim_end_matches(')').split(", ").count(),
        "every added tool is named: {added}"
    );
    assert_eq!(shown(&engine, "model").1, Some(json!("zone/model-a")));
}

/// T20: a parameter change binds to the file of a saved chat: a file changed between the card
/// and the answer, even with the same settings, applies nothing and leaves the chat closed.
#[tokio::test]
async fn set_params_of_a_saved_chat_whose_file_changed_applies_nothing() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    save(&app, &build("zone-pf3", |_| {})).await;
    let app2 = app.clone();
    let out = moved_then_allowed(
        &app,
        "await engine('chats.set_params', chat_id='zone-pf3', temperature=0.4)",
        async move {
            let mut s = build("zone-pf3", |_| {});
            s.messages.push(super::chats_risky::message(
                "zone-pf3-m9",
                "user",
                "zone written elsewhere",
            ));
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
    assert!(!app.chat.sessions.read().await.contains_key("zone-pf3"));
}
