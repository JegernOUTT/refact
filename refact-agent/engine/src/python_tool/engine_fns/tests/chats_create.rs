use super::chats_risky::{approve, deny, guarded_policy, hold_queue, install, refused};
use super::*;
use crate::chat::types::ChatSession;
use refact_chat_api::PythonEngineClass as Class;

/// Every file of another chat under the project's trajectories folder: not this chat's own files
/// (its run workspace) and not the index.
fn trajectory_files(dir: &tempfile::TempDir) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut folders = vec![dir.path().join("project/.refact/trajectories")];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                folders.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.retain(|path| {
        let path = path.to_string_lossy();
        !path.contains("/python-test/") && !path.contains("index.json")
    });
    files.sort();
    files
}

async fn new_session(app: &AppState, out: &Value) -> Arc<tokio::sync::Mutex<ChatSession>> {
    let id = out["data"]["chat_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no chat id: {out}"));
    app.chat.sessions.read().await[id].clone()
}

#[tokio::test]
async fn create_makes_a_saved_root_chat_with_its_settings_and_messages() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let (request, out) = approve(
        &app,
        "await engine('chats.create', title='zone created', model='zone/model-a', mode='debug', temperature=0.3, messages=[{'role':'user','text':'zone hello @file a.txt'},{'role':'assistant','text':'zone looking','tool_calls':[{'name':'cat','arguments':{'paths':'a.txt'},'result':'zone file text'}]},{'role':'assistant','text':'zone answer'},{'role':'user','text':'zone next'}])",
    )
    .await;
    let engine = request.engine.unwrap();
    assert_eq!(engine.title, "create a chat");
    assert_eq!(engine.scope.as_deref(), Some("new chat"));
    let labels: Vec<&str> = engine.items.iter().map(|i| i.label.as_str()).collect();
    for label in [
        "title",
        "model",
        "mode",
        "temperature",
        "editing_tools",
        "dangerous_commands",
        "all_confirmations",
        "system prompt",
        "#0 user",
        "#1 assistant",
        "#1 call cat",
        "#2 tool",
        "#3 assistant",
        "#4 user",
    ] {
        assert!(labels.contains(&label), "{label}: {labels:?}");
    }
    assert_eq!(out["ok"], true, "{out}");
    assert_eq!(out["data"]["messages"], 5, "{out}");
    assert_eq!(out["data"]["started"], false, "{out}");
    let id = out["data"]["chat_id"].as_str().unwrap().to_string();
    let session = new_session(&app, &out).await;
    {
        let s = session.lock().await;
        assert_eq!(s.thread.title, "zone created");
        assert_eq!(s.thread.model, "zone/model-a");
        assert_eq!(s.thread.mode, "debug");
        assert!(s
            .thread
            .temperature
            .is_some_and(|t| (t as f64 - 0.3).abs() < 1e-6));
        assert!(s.thread.parent_id.is_none() && s.thread.root_chat_id.is_none());
        let roles: Vec<&str> = s.messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, ["user", "assistant", "tool", "assistant", "user"]);
        assert_eq!(
            s.messages[0].extra["delivery"]["source"],
            "engine.chats.create"
        );
        assert_eq!(
            s.messages[0].content.content_text_only(),
            "zone hello @file a.txt"
        );
        assert_eq!(
            s.messages[2].tool_call_id,
            s.messages[1].tool_calls.as_ref().unwrap()[0].id
        );
    }
    let saved = crate::chat::trajectories::load_trajectory_for_chat(app.gcx.clone(), &id)
        .await
        .expect("the new chat was not saved");
    assert_eq!(saved.messages.len(), 5);
    assert_eq!(saved.thread.title, "zone created");
    let listed = result(&run(&app, "(await engine('chats.list', limit=200))['data']").await);
    assert!(listed.to_string().contains(&id), "{listed}");
}

#[tokio::test]
async fn create_preserves_reviewed_approval_flags_with_auto_edit_mode_defaults() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let mode = crate::yaml_configs::customization_registry::get_mode_config(
        app.gcx.clone(),
        "agent",
        None,
    )
    .await
    .unwrap();
    assert_eq!(mode.thread_defaults.auto_approve_editing_tools, Some(true));

    let hooks = &crate::python_tool::engine_fns::chats::TEST_HOOKS;
    let hook_key = "create:zone reviewed start";
    for (code, start) in [
        ("await engine('chats.create', messages=[{'role':'user','text':'zone hello'}])", false),
        ("await engine('chats.create', title='zone reviewed start', mode='agent', editing_tools=False, start=True, messages=[{'role':'user','text':'zone hello'}])", true),
    ] {
        if start {
            hooks.lock().unwrap().push((hook_key.into(), hold_queue));
        }
        let (request, out) = approve(&app, code).await;
        if start {
            hooks.lock().unwrap().retain(|(key, _)| key != hook_key);
        }
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["data"]["started"], start, "{out}");
        let id = out["data"]["chat_id"].as_str().unwrap();
        let saved = crate::chat::trajectories::load_trajectory_for_chat(app.gcx.clone(), id)
            .await
            .expect("the new chat was not saved");
        assert!(saved.auto_approve_editing_tools_present);
        assert!(saved.auto_approve_dangerous_commands_present);
        let session = new_session(&app, &out).await;
        let live = session.lock().await;
        assert_eq!(live.thread.mode, "agent");
        let engine = request.engine.unwrap();
        for (label, live, saved) in [
            (
                "editing_tools",
                live.thread.auto_approve_editing_tools,
                saved.thread.auto_approve_editing_tools,
            ),
            (
                "dangerous_commands",
                live.thread.auto_approve_dangerous_commands,
                saved.thread.auto_approve_dangerous_commands,
            ),
            (
                "all_confirmations",
                live.thread.autonomous_no_confirm,
                saved.thread.autonomous_no_confirm,
            ),
        ] {
            let reviewed = engine.items.iter().find(|item| item.label == label).unwrap();
            assert_eq!(reviewed.new, Some(json!(false)), "{label}: {code}");
            assert_eq!(reviewed.new, Some(json!(live)), "live {label}: {code}");
            assert_eq!(reviewed.new, Some(json!(saved)), "saved {label}: {code}");
        }
        if start {
            let queued = live.command_queue.front().expect("start was not queued");
            assert_eq!(queued.client_request_id, out["data"]["request_id"].as_str().unwrap());
            assert!(matches!(&queued.command, refact_chat_api::ChatCommand::Regenerate {}));
        } else {
            assert!(live.command_queue.is_empty());
        }
    }
}

#[tokio::test]
async fn create_fresh_session_does_not_reuse_an_existing_chat() {
    use crate::chat::session_runtime::{
        create_fresh_session_with_params, get_or_create_session_with_trajectory,
    };
    use crate::chat::trajectories::{load_trajectory_for_chat, try_save_trajectory};

    let (_dir, app, _) = fixture().await;
    let sessions = &app.chat.sessions;
    let id = "zone-existing";
    let params = json!({"auto_approve_editing_tools": false});
    let existing = get_or_create_session_with_trajectory(app.clone(), sessions, id)
        .await
        .unwrap();
    {
        let mut s = existing.lock().await;
        assert!(s.thread.auto_approve_editing_tools);
        s.set_title("zone original".into(), false);
    }
    assert_eq!(
        try_save_trajectory(app.clone(), existing.clone()).await,
        Ok(true)
    );

    assert!(
        create_fresh_session_with_params(app.clone(), sessions, id, &params)
            .await
            .is_err()
    );
    assert!(Arc::ptr_eq(&sessions.read().await[id], &existing));
    assert!(existing.lock().await.thread.auto_approve_editing_tools);

    sessions.write().await.remove(id).unwrap();
    assert!(
        create_fresh_session_with_params(app.clone(), sessions, id, &params)
            .await
            .is_err()
    );
    assert!(!sessions.read().await.contains_key(id));
    let saved = load_trajectory_for_chat(app.gcx.clone(), id).await.unwrap();
    assert_eq!(saved.thread.title, "zone original");
    assert!(saved.thread.auto_approve_editing_tools);
}

#[tokio::test]
async fn deny_creates_nothing() {
    let (dir, app, _) = fixture().await;
    install(&app).await;
    let sessions = app.chat.sessions.read().await.len();
    let files = trajectory_files(&dir);
    let request = deny(
        &app,
        "await engine('chats.create', title='zone denied', messages=[{'role':'user','text':'zone q'}])",
    )
    .await;
    assert!(request.engine.is_some());
    assert_eq!(sessions, app.chat.sessions.read().await.len());
    assert_eq!(files, trajectory_files(&dir));
}

#[tokio::test]
async fn start_needs_a_final_user_message_and_queues_an_answer() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    for code in [
        "await engine('chats.create', start=True, messages=[{'role':'user','text':'zone q'},{'role':'assistant','text':'zone a'}])",
        "await engine('chats.create', start=True)",
    ] {
        assert_eq!(refused(&app, code).await, "TypeError", "{code}");
    }
    let (request, out) = approve(
        &app,
        "await engine('chats.create', start=True, messages=[{'role':'user','text':'zone q'}])",
    )
    .await;
    let engine = request.engine.unwrap();
    assert!(engine.items.iter().any(|i| i.label == "start"));
    assert!(engine.items.iter().any(|i| i.label == "goal"));
    assert_eq!(out["ok"], true, "{out}");
    assert_eq!(out["data"]["started"], true, "{out}");
    assert!(
        out["data"]["request_id"]
            .as_str()
            .unwrap()
            .starts_with("engine-"),
        "{out}"
    );
}

#[tokio::test]
async fn unknown_and_bound_modes_unknown_models_and_foreign_keys_are_refused() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    let sessions = app.chat.sessions.read().await.len();
    for (code, kind) in [
        (
            "await engine('chats.create', mode='zone-no-such-mode')",
            "TypeError",
        ),
        (
            "await engine('chats.create', mode='task_agent')",
            "PermissionError",
        ),
        (
            "await engine('chats.create', model='zone/no-such-model')",
            "TypeError",
        ),
        (
            "await engine('chats.create', parent_id='python-test')",
            "TypeError",
        ),
        (
            "await engine('chats.create', messages=[{'role':'system','text':'zone sys'}])",
            "TypeError",
        ),
        (
            "await engine('chats.create', messages=[{'role':'user','text':'zone q','message_id':'x'}])",
            "TypeError",
        ),
        (
            "await engine('chats.create', messages=[{'role':'user','text':'set X_TOKEN=abc now'}])",
            "PermissionError",
        ),
    ] {
        assert_eq!(refused(&app, code).await, kind, "{code}");
    }
    assert_eq!(sessions, app.chat.sessions.read().await.len());
}

#[tokio::test]
async fn approval_flags_a_system_prompt_and_an_autonomous_start_make_it_risky() {
    let (_dir, app, _) = fixture().await;
    install(&app).await;
    for (code, class) in [
        ("await engine('chats.create', temperature=0.2)", Class::Change),
        ("await engine('chats.create', editing_tools=True)", Class::Risky),
        (
            "await engine('chats.create', system='zone be brief', messages=[{'role':'user','text':'zone q'}])",
            Class::Risky,
        ),
        (
            "await engine('chats.create', dangerous_commands=True, start=True, messages=[{'role':'user','text':'zone q'}])",
            Class::Risky,
        ),
    ] {
        let engine = deny(&app, code).await.engine.unwrap();
        assert_eq!(engine.class, class, "{code}");
    }
    let engine = deny(
        &app,
        "await engine('chats.create', system='zone be brief', messages=[{'role':'user','text':'zone q'}])",
    )
    .await
    .engine
    .unwrap();
    let system = engine
        .items
        .iter()
        .find(|i| i.label == "system prompt")
        .unwrap();
    assert_eq!(system.new, Some(json!("zone be brief")));
}

#[tokio::test]
async fn create_carries_run_records_and_checks_egress_for_the_new_chat_model() {
    let (dir, app, _) = fixture().await;
    install(&app).await;
    let mut policy = guarded_policy(&app);
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy.clone())
        .await
        .unwrap();
    std::fs::write(dir.path().join("project/zone_a.txt"), "guarded").unwrap();
    let code = "x = open('zone_a.txt').read()\nawait engine('chats.create', model='zone/model-a', messages=[{'role':'user','text':'zone ' + x}])";
    let sessions = app.chat.sessions.read().await.len();
    assert_eq!(refused(&app, code).await, "PermissionError");
    assert_eq!(sessions, app.chat.sessions.read().await.len());
    policy.zones[0].send_to = vec!["*".into()];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
    let (_, out) = approve(&app, code).await;
    assert_eq!(out["ok"], true, "{out}");
    let session = new_session(&app, &out).await;
    let s = session.lock().await;
    assert!(
        s.messages[0].extra["privacy"]
            .to_string()
            .contains("zone_a"),
        "{:?}",
        s.messages[0].extra
    );
}

#[tokio::test]
async fn a_save_that_fails_after_creation_discards_the_new_chat() {
    let (dir, app, _) = fixture().await;
    install(&app).await;
    let hook: fn(&mut ChatSession) = |s| {
        crate::chat::trajectories::set_test_message_serialization_failure(Some(
            s.messages[0].message_id.clone(),
        ))
    };
    crate::python_tool::engine_fns::chats::TEST_HOOKS
        .lock()
        .unwrap()
        .push(("create:zone doomed".into(), hook));
    let mut before: Vec<String> = app.chat.sessions.read().await.keys().cloned().collect();
    let files = trajectory_files(&dir);
    let (_, out) = approve(
        &app,
        "r = await engine('chats.create', title='zone doomed', messages=[{'role':'user','text':'zone q'}])\n[r['ok'], r.get('error')]",
    )
    .await;
    assert_eq!(out[0], false, "{out}");
    assert!(
        out[1].as_str().unwrap().contains("nothing was created"),
        "{out}"
    );
    let mut after: Vec<String> = app.chat.sessions.read().await.keys().cloned().collect();
    before.sort();
    after.sort();
    assert_eq!(before, after, "the new chat stayed open");
    assert_eq!(files, trajectory_files(&dir), "the new chat left a file");
    crate::python_tool::engine_fns::chats::TEST_HOOKS
        .lock()
        .unwrap()
        .retain(|(key, _)| key != "create:zone doomed");
    crate::chat::trajectories::set_test_message_serialization_failure(None);
}
