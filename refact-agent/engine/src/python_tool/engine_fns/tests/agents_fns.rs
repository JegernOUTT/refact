use super::*;
use crate::agents::types::{BgAgentKind, CreateAgentRequest};

/// Installs caps with the fixture's `model` and gives the fixture session a project scope.
pub(super) async fn scoped_session(app: &AppState, project: &std::path::Path) {
    let mut caps = crate::caps::CodeAssistantCaps::default();
    caps.chat_models.insert(
        "model".into(),
        std::sync::Arc::new(crate::caps::ChatModelRecord {
            base: crate::caps::BaseModelRecord {
                id: "model".into(),
                n_ctx: 200_000,
                ..Default::default()
            },
            supports_tools: true,
            supports_agent: true,
            ..Default::default()
        }),
    );
    let (root, _) = crate::caps::project_model_defaults_state(app.gcx.clone()).await;
    {
        let mut state = app.gcx.caps_state.write().await;
        state.caps = Some(std::sync::Arc::new(caps));
        state.loaded_project_root = Some(root);
        state.last_attempted_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
    let session = session(app).await;
    let mut session = session.lock().await;
    session.work_project_root = Some(project.to_path_buf());
    session.thread.model = "model".into();
}

fn agent_request(parent: &str, root: &str, title: &str) -> CreateAgentRequest {
    CreateAgentRequest {
        parent_chat_id: parent.into(),
        parent_root_chat_id: Some(root.into()),
        parent_tool_call_id: None,
        kind: BgAgentKind::Subagent,
        config_name: "subagent".into(),
        title: title.into(),
        prompt: title.into(),
        target_files: vec![],
        model: "test/model".into(),
        model_type: None,
        goal_summary: None,
        plan_present: false,
        worktree_id: None,
        worktree_branch: None,
    }
}

#[tokio::test]
async fn agents_list_reduces_foreign_rows_and_bounds_the_list() {
    let (_dir, app, _) = fixture().await;
    let (own, _, _) = app
        .agents
        .create(agent_request("python-test", "python-test", "own agent"))
        .await
        .unwrap();
    app.agents
        .update_progress(
            &own.agent_id,
            "own progress Bearer abcdefghijklmnop".into(),
            1,
        )
        .await
        .unwrap();
    for i in 0..2 {
        let (foreign, _, _) = app
            .agents
            .create(agent_request(
                "other-root",
                "other-root",
                &format!("foreign {i}"),
            ))
            .await
            .unwrap();
        app.agents
            .update_progress(&foreign.agent_id, "foreign zone progress".into(), 1)
            .await
            .unwrap();
    }
    let m = run(
        &app,
        "a = await engine('agents.list')\nb = await engine('agents.list', limit=1)\nc = await engine('agents.list', root_chat_id='other-root')\n[a['data'], b['data'], c['data']]",
    )
    .await;
    let out = result(&m);
    assert!(out.is_array(), "{}", m.content.content_text_only());
    let rows = out[0]["agents"].as_array().unwrap();
    assert_eq!(rows.len(), 3, "{out}");
    let own_row = rows
        .iter()
        .find(|r| r["agent_id"] == own.agent_id.as_str())
        .unwrap();
    assert!(own_row["progress"]
        .as_str()
        .unwrap()
        .contains("own progress"));
    assert!(!out.to_string().contains("abcdefghijklmnop"), "{out}");
    assert!(!out.to_string().contains("foreign zone progress"), "{out}");
    for row in rows
        .iter()
        .filter(|r| r["agent_id"] != own.agent_id.as_str())
    {
        assert!(
            row.get("progress").is_none() && row.get("target_files").is_none(),
            "{row}"
        );
        assert!(row["title"].is_string() && row["status"].is_string());
    }
    assert_eq!(out[0]["own_tree"], json!([own.agent_id]));
    assert_eq!(out[1]["agents"].as_array().unwrap().len(), 1);
    assert_eq!(out[1]["truncated"], true);
    assert_eq!(out[2]["total"], 2);
}

#[tokio::test]
async fn agents_reads_are_refused_in_agent_chats() {
    let (_dir, _app, ccx) = fixture().await;
    ccx.lock().await.background_agent_id = Some("bgagent-x".into());
    let m = run_ccx(
        ccx,
        "out = []\nfor name in ['agents.list', 'agents.attention', 'agents.tasks', 'processes.list']:\n    try:\n        await engine(name)\n        out.append('ran')\n    except PermissionError as e:\n        out.append(str(e))\nout",
    )
    .await;
    let out = result(&m);
    for row in out.as_array().unwrap() {
        assert!(row.as_str().unwrap().contains("root chat"), "{out}");
    }
}

#[tokio::test]
async fn agents_attention_omits_foreign_text() {
    let (_dir, app, _) = fixture().await;
    let ask = |text: &str, asker: &str| {
        crate::attention::AttentionRequest::new(
            crate::attention::AttentionScope::new("/project", "AllowToSendAnywhere"),
            refact_chat_api::QuestionKind::UserInput,
            asker,
            "user",
            text,
        )
    };
    let now = chrono::Utc::now();
    app.attention()
        .join(
            ask("own question", "chat:python-test"),
            "chat:python-test",
            now,
        )
        .unwrap();
    app.attention()
        .join(
            ask("foreign zone question", "chat:other-chat"),
            "chat:other-chat",
            now,
        )
        .unwrap();
    let m = run(&app, "(await engine('agents.attention'))['data']").await;
    let out = result(&m);
    let waiting = out["waiting"].as_array().unwrap();
    assert_eq!(waiting.len(), 2, "{out}");
    assert!(waiting.iter().any(|e| e["text"] == "own question"));
    assert!(!out.to_string().contains("foreign zone question"), "{out}");
    assert!(waiting
        .iter()
        .any(|e| e["own"] == false && e.get("text").is_none()));
}

fn git(root: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_repo(root: &std::path::Path) {
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "zone@example.com"]);
    git(root, &["config", "user.name", "zone"]);
    git(root, &["checkout", "-q", "-b", "main"]);
    std::fs::write(root.join("zone_a.txt"), "a\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "init"]);
}

async fn task(app: &AppState, name: &str) -> String {
    crate::tasks::storage::create_task(app.gcx.clone(), name)
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn agents_tasks_and_set_task() {
    let (dir, app, _) = fixture().await;
    init_repo(&dir.path().join("project"));
    let task_id = task(&app, "zone task").await;
    let m = run(
        &app,
        &format!("a = await engine('agents.tasks')\nb = await engine('agents.tasks', task_id='{task_id}')\nbad = []\nfor kw in [{{'task_id': '{task_id}'}}, {{'task_id': '{task_id}', 'base_branch': 'no-such-branch'}}, {{'task_id': 'missing', 'task_name': 'x'}}]:\n    try:\n        await engine('agents.set_task', **kw)\n    except TypeError as e:\n        bad.append(str(e))\n[a['data'], b['data'], bad]"),
    )
    .await;
    let out = result(&m);
    assert!(out.is_array(), "{}", m.content.content_text_only());
    assert_eq!(out[0]["total"], 1, "{out}");
    assert_eq!(out[1]["meta"]["name"], "zone task");
    assert!(out[1]["board_summary"]["ready"].is_array());
    let bad = out[2].as_array().unwrap();
    assert_eq!(bad.len(), 3, "{out}");
    assert!(bad[0].as_str().unwrap().contains("at least one"), "{out}");
    assert!(bad[1].as_str().unwrap().contains("no-such-branch"), "{out}");
    assert!(bad[2].as_str().unwrap().contains("no task"), "{out}");
    assert!(session(&app)
        .await
        .lock()
        .await
        .runtime
        .python_approvals
        .is_empty());

    let script = format!("(await engine('agents.set_task', task_id='{task_id}', task_name='zone renamed', base_branch='main'))['data']");
    let t = spawn(&app, &script);
    let request = next_request(&app, &t).await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(engine.title, "change a task");
    assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Change);
    assert_eq!(engine.scope.as_deref(), Some("task zone task"));
    let name = engine.items.iter().find(|i| i.label == "name").unwrap();
    assert_eq!(name.old, Some(json!("zone task")));
    assert_eq!(name.new, Some(json!("zone renamed")));
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let out = result(&finish(t).await);
    assert_eq!(out["name"], "zone renamed", "{out}");
    let meta = crate::tasks::storage::load_task_meta(app.gcx.clone(), &task_id)
        .await
        .unwrap();
    assert_eq!(meta.base_branch.as_deref(), Some("main"));

    let t = spawn(
        &app,
        &format!("await engine('agents.set_task', task_id='{task_id}', status='abandoned')"),
    );
    let request = next_request(&app, &t).await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(
        engine.class,
        refact_chat_api::PythonEngineClass::Destructive
    );
    assert!(engine.warning.unwrap().contains("Abandoning"));
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    finish(t).await;
    let meta = crate::tasks::storage::load_task_meta(app.gcx.clone(), &task_id)
        .await
        .unwrap();
    assert_eq!(meta.status, crate::tasks::types::TaskStatus::Planning);

    let t = spawn(
        &app,
        &format!("r = await engine('agents.set_task', task_id='{task_id}', task_name='zone again')\n[r['ok'], r['error']]"),
    );
    let request = next_request(&app, &t).await;
    let mut meta = crate::tasks::storage::load_task_meta(app.gcx.clone(), &task_id)
        .await
        .unwrap();
    meta.name = "moved meanwhile".into();
    crate::tasks::storage::save_task_meta(app.gcx.clone(), &task_id, &meta)
        .await
        .unwrap();
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let out = result(&finish(t).await);
    assert_eq!(out[0], false, "{out}");
    assert!(
        out[1]
            .as_str()
            .unwrap()
            .contains("changed since the preview"),
        "{out}"
    );
    let meta = crate::tasks::storage::load_task_meta(app.gcx.clone(), &task_id)
        .await
        .unwrap();
    assert_eq!(meta.name, "moved meanwhile");
}

#[tokio::test]
async fn agents_worktrees_wait_graph_and_checkpoints() {
    let (dir, app, ccx) = fixture().await;
    init_repo(&dir.path().join("project"));
    scoped_session(&app, &dir.path().join("project")).await;
    {
        let mut message = crate::call_validation::ChatMessage::new("user".into(), "hi".into());
        message.checkpoints = vec![refact_core::chat_types::Checkpoint {
            workspace_folder: dir.path().join("project"),
            commit_hash: "abc123".into(),
        }];
        ccx.lock().await.messages = vec![message];
    }
    let m = run_ccx(
        ccx,
        "w = await engine('agents.worktrees')\nc = await engine('agents.checkpoints')\ng = await engine('agents.wait_graph')\ntry:\n    await engine('agents.worktree_diff', worktree_id='nothing-here')\n    d = 'ran'\nexcept TypeError as e:\n    d = str(e)\n[w['data'], c['data'], d, g['ok'], g['data'], g['error']]",
    )
    .await;
    let out = result(&m);
    assert!(out.is_array(), "{}", m.content.content_text_only());
    assert_eq!(out[0]["worktrees"], json!([]), "{out}");
    assert_eq!(out[0]["truncated"], false);
    assert_eq!(out[1]["checkpoints"][0]["commit_hash"], "abc123", "{out}");
    assert_eq!(out[1]["checkpoints"][0]["message_index"], 0);
    assert_ne!(out[2], "ran", "{out}");
    assert_eq!(out[3], true, "{out}");
    assert_eq!(out[4]["wait_graph"]["edges"], json!([]), "{out}");
}
