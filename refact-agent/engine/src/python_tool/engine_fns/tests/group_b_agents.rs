use super::*;
use crate::agents::types::{BgAgentKind, CreateAgentRequest};

fn project(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("project")
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
        model: "model".into(),
        model_type: None,
        goal_summary: None,
        plan_present: false,
        worktree_id: None,
        worktree_branch: None,
    }
}

async fn ask(
    app: &AppState,
    code: &str,
) -> (
    tokio::task::JoinHandle<ChatMessage>,
    refact_chat_api::PythonApprovalRequest,
) {
    let task = spawn(app, code);
    let request = next_request(app, &task).await;
    (task, request)
}

/// Caps whose task planner slot names `model`.
async fn planner_on(app: &AppState, dir: &tempfile::TempDir, model: &str) {
    super::agents_fns::scoped_session(app, &project(dir)).await;
    let mut state = app.gcx.caps_state.write().await;
    let mut caps = (**state.caps.as_ref().unwrap()).clone();
    caps.defaults.task_planner_agent_model = model.into();
    state.caps = Some(std::sync::Arc::new(caps));
}

#[tokio::test]
async fn agents_message_reaches_a_foreign_agent_and_refuses_own_and_ended() {
    let (_dir, app, _) = fixture().await;
    let (foreign, _, _) = app
        .agents
        .create(agent_request("other-root", "other-root", "foreign"))
        .await
        .unwrap();
    let (own, _, _) = app
        .agents
        .create(agent_request("python-test", "python-test", "own"))
        .await
        .unwrap();
    let m = run(
        &app,
        &format!("try:\n    await engine('agents.message', agent_id={:?}, text='hi')\n    r = 'asked'\nexcept TypeError as e:\n    r = str(e)\nr", own.agent_id),
    )
    .await;
    assert!(
        result(&m).as_str().unwrap().contains("agent_message"),
        "{}",
        result(&m)
    );

    let (task, request) = ask(
        &app,
        &format!(
            "await engine('agents.message', agent_id={:?}, text='stop now')",
            foreign.agent_id
        ),
    )
    .await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(engine.title, "message another chat's agent");
    assert_eq!(engine.items[1].new, Some(json!("other-root")));
    assert_eq!(engine.items[2].new, Some(json!("stop now")));
    assert!(
        engine.items.iter().all(|i| i.old != Some(json!(""))),
        "{:?}",
        engine.items
    );
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    finish(task).await;
    assert!(app
        .agents
        .get_any(&foreign.agent_id)
        .await
        .unwrap()
        .pending_deliveries
        .is_empty());

    let (task, request) = ask(&app, &format!("(await engine('agents.message', agent_id={:?}, text='stop now', push='when_idle'))['data']['queued']", foreign.agent_id)).await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    assert_eq!(result(&finish(task).await), true);
    let record = app.agents.get_any(&foreign.agent_id).await.unwrap();
    assert_eq!(record.pending_deliveries.len(), 1);
    assert!(record.pending_deliveries[0].messages[0]
        .content
        .content_text_only()
        .contains("stop now"));
}

#[tokio::test]
async fn agents_cancel_stops_a_foreign_subtree_and_refuses_a_changed_agent() {
    let (_dir, app, _) = fixture().await;
    let (foreign, _, _) = app
        .agents
        .create(agent_request("other-root", "other-root", "foreign"))
        .await
        .unwrap();
    let (task, request) = ask(
        &app,
        &format!(
            "await engine('agents.cancel', agent_id={:?})",
            foreign.agent_id
        ),
    )
    .await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(
        engine.class,
        refact_chat_api::PythonEngineClass::Destructive
    );
    assert_eq!(engine.scope.as_deref(), Some("chat other-root"));
    assert_eq!(
        engine.warning.as_deref(),
        Some(
            "The agent's run stops now and its unfinished work cannot be resumed from this point."
        )
    );
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    finish(task).await;
    assert!(!app
        .agents
        .get_any(&foreign.agent_id)
        .await
        .unwrap()
        .status
        .is_terminal());

    let (task, request) = ask(&app, &format!("r = await engine('agents.cancel', agent_id={:?}, reason='done')\nr['data']['cancelled_agent_ids']", foreign.agent_id)).await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    assert_eq!(result(&finish(task).await), json!([foreign.agent_id]));
    assert!(app
        .agents
        .get_any(&foreign.agent_id)
        .await
        .unwrap()
        .status
        .is_terminal());
}

#[tokio::test]
async fn planner_instructions_and_task_deletion() {
    let (dir, app, _) = fixture().await;
    planner_on(&app, &dir, "model").await;
    let task_id = crate::tasks::storage::create_task(app.gcx.clone(), "zone task")
        .await
        .unwrap()
        .id;
    let (task, request) = ask(&app, &format!("await engine('agents.set_planner_instructions', task_id={task_id:?}, content='plan carefully')")).await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(engine.title, "replace planner instructions");
    assert_eq!(engine.items[0].new, Some(json!("plan carefully")));
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    finish(task).await;
    assert_eq!(
        crate::tasks::storage::load_planner_instructions(app.gcx.clone(), &task_id)
            .await
            .unwrap(),
        "plan carefully"
    );
    let m = run(&app, &format!("(await engine('agents.set_planner_instructions', task_id={task_id:?}, content='plan carefully'))['text']")).await;
    assert_eq!(result(&m), "nothing to change");

    let (task, request) = ask(&app, &format!("r = await engine('agents.set_planner_instructions', task_id={task_id:?}, content='other')\n[r['ok'], r['error']]")).await;
    crate::tasks::storage::save_planner_instructions(app.gcx.clone(), &task_id, "edited meanwhile")
        .await
        .unwrap();
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let out = result(&finish(task).await);
    assert!(
        out[1]
            .as_str()
            .unwrap()
            .contains("changed since the preview"),
        "{out}"
    );

    let (task, request) = ask(
        &app,
        &format!("await engine('agents.delete_task', task_id={task_id:?})"),
    )
    .await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(
        engine.class,
        refact_chat_api::PythonEngineClass::Destructive
    );
    assert!(engine.items[0]
        .old
        .as_ref()
        .unwrap()
        .as_str()
        .unwrap()
        .starts_with("zone task ("));
    assert_eq!(engine.warning.as_deref(), Some("The task folder with its board, documents and planner trajectories is removed and cannot be restored by the engine."));
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    finish(task).await;
    assert!(
        crate::tasks::storage::load_task_meta(app.gcx.clone(), &task_id)
            .await
            .is_ok()
    );
    let (task, request) = ask(
        &app,
        &format!("(await engine('agents.delete_task', task_id={task_id:?}))['data']"),
    )
    .await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    assert_eq!(result(&finish(task).await)["deleted"], true);
    assert!(
        crate::tasks::storage::load_task_meta(app.gcx.clone(), &task_id)
            .await
            .is_err()
    );
    drop(dir);
}

#[tokio::test]
#[serial_test::serial(file_provenance)]
async fn forget_provenance_counts_and_erases_records() {
    let (dir, app, _) = fixture().await;
    crate::coordination::file_provenance::reset_for_tests();
    let m = run(&app, "try:\n    await engine('agents.forget_provenance')\n    r = 'asked'\nexcept Exception as e:\n    r = str(e)\nr").await;
    assert!(
        result(&m).to_string().contains("no provenance store"),
        "{}",
        result(&m)
    );
    let store = std::sync::Arc::new(
        refact_work_core::CoordinationStore::open_in_memory(
            &refact_work_core::CoordinationSettings::default(),
        )
        .await
        .unwrap(),
    );
    crate::coordination::file_provenance::install_sink(
        store,
        refact_work_core::ProjectKey::new("zone"),
        "engine-a",
        None,
    );
    let file = project(&dir).join("zone.txt");
    std::fs::write(&file, "x").unwrap();
    let key = crate::file_activity::resolve_key(&file).0;
    crate::coordination::file_provenance::observe(
        crate::coordination::file_provenance::ObservedActivity {
            file_key: key.clone(),
            file_alias: None,
            path: key,
            actor: Some("chat-a".into()),
            tool: "patch".into(),
            access: refact_work_core::store::records::AccessKind::Write,
            digest_before: None,
            digest_after: None,
            at_ms: 1,
        },
    );
    let (task, request) = ask(
        &app,
        "(await engine('agents.forget_provenance', path='zone.txt'))['data']",
    )
    .await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(engine.items[0].old, Some(json!("1 records")));
    assert!(engine
        .warning
        .as_deref()
        .unwrap()
        .starts_with("Recorded authorship"));
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    assert_eq!(result(&finish(task).await)["removed"], 1);
    let m = run(
        &app,
        "(await engine('agents.forget_provenance', path='zone.txt'))['text']",
    )
    .await;
    assert_eq!(result(&m), "nothing to change");
    crate::coordination::file_provenance::reset_for_tests();
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

#[tokio::test]
async fn worktree_diff_withholds_quoted_and_ambiguous_blocked_paths() {
    let (dir, app, _) = fixture().await;
    let root = project(&dir);
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "zone@example.com"]);
    git(&root, &["config", "user.name", "zone"]);
    std::fs::create_dir_all(root.join("secrets")).unwrap();
    std::fs::create_dir_all(root.join("docs b")).unwrap();
    for name in ["secrets/clé.env", "x.txt", "docs b/x.txt"] {
        std::fs::write(root.join(name), "base\n").unwrap();
    }
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "init"]);
    for name in ["secrets/clé.env", "x.txt", "docs b/x.txt"] {
        std::fs::write(root.join(name), "SECRET-CONTENT\n").unwrap();
    }
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.blocked = vec![
        root.join("secrets").to_string_lossy().into_owned(),
        root.join("x.txt").to_string_lossy().into_owned(),
    ];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
    let diff = crate::worktrees::git::diff_for_path(&root, None, None, 200_000).unwrap();
    assert!(
        diff.patch.contains("\"a/secrets/cl"),
        "git quotes the name: {}",
        diff.patch
    );
    let filter = super::super::common::PathFilter::new_for_test(&app, "model").await;
    let (patch, files, withheld, unreadable) =
        super::super::agents::filter_diff(&diff.patch, &diff.files, |path| {
            filter.allows(&root.join(path).to_string_lossy())
        });
    assert_eq!(patch.matches("SECRET-CONTENT").count(), 1, "{patch}");
    assert!(patch.contains("docs b/x.txt"), "{patch}");
    assert!(
        withheld.iter().any(|p| p == "secrets/clé.env"),
        "{withheld:?}"
    );
    assert!(withheld.iter().any(|p| p == "x.txt"), "{withheld:?}");
    assert_eq!(unreadable, 0);
    assert!(files.iter().all(|f| f.path == "docs b/x.txt"), "{files:?}");
    let (patch, _, _, unreadable) = super::super::agents::filter_diff(
        "diff --git \"a/odd\" \"b/odd\\x\"\n+SECRET-CONTENT\n",
        &[],
        |_| true,
    );
    assert_eq!(unreadable, 1);
    assert!(!patch.contains("SECRET-CONTENT"), "{patch}");
}

#[test]
fn a_rename_from_a_blocked_path_does_not_list_the_blocked_name() {
    let renamed = crate::worktrees::types::WorktreeDiffFile {
        path: "public.txt".into(),
        old_path: Some("secrets/zone.env".into()),
        status: "renamed".into(),
        source: "committed".into(),
        additions: None,
        deletions: None,
    };
    let (_, files, withheld, _) =
        super::super::agents::filter_diff("", &[renamed], |path| !path.starts_with("secrets/"));
    assert!(files.is_empty(), "{files:?}");
    assert_eq!(withheld, vec!["secrets/zone.env".to_string()]);
}

fn write_integration(dir: &tempfile::TempDir, name: &str, body: &str) -> String {
    let path = project(dir).join(".refact/integrations.d").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, body).unwrap();
    path.to_string_lossy().into_owned()
}

/// Only the `allowed` MCP server may be used by the fixture's model provider.
async fn mcp_only(app: &AppState, allowed: &str) {
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.tool_access.providers.insert(
        "model".into(),
        refact_privacy::ProviderToolAccess {
            mcp: vec![allowed.into()],
        },
    );
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
}

#[tokio::test]
async fn mcp_reads_and_answers_follow_the_model_filter_and_hide_previews() {
    use crate::integrations::mcp::mcp_interactions::{create_mcp_interaction, MCPInteractionKind};
    let (dir, app, _) = fixture().await;
    let open = write_integration(&dir, "mcp_stdio_open.yaml", "command: open-server\n");
    let hidden = write_integration(&dir, "mcp_stdio_hidden.yaml", "command: hidden-server\n");
    let global = dir.path().join("config/integrations.d");
    std::fs::create_dir_all(&global).unwrap();
    std::fs::write(
        global.join("mcp_stdio_hidden.yaml"),
        "command: hidden-global\n",
    )
    .unwrap();
    std::fs::write(
        global.join("mcp_stdio_open.yaml"),
        "command: open-global\nargs: [\"--auth-header\", \"zone-hdr-1\"]\n",
    )
    .unwrap();
    mcp_only(&app, "open").await;
    let (sampling, _rx1) = create_mcp_interaction(
        &open,
        MCPInteractionKind::SamplingApproval {
            message_count: 2,
            max_tokens: 50,
            preview: "OTHER-CHAT-CONTENT".into(),
        },
    )
    .await;
    let (foreign, _rx2) = create_mcp_interaction(
        &hidden,
        MCPInteractionKind::Elicitation {
            message: "HIDDEN-SERVER-ASK".into(),
            requested_schema: None,
            url: None,
            elicitation_id: None,
        },
    )
    .await;
    let m = run(&app, "[(await engine('integrations.mcp_interactions'))['data'], (await engine('integrations.mcp_export'))['data']]").await;
    let out = result(&m);
    let text = out.to_string();
    assert!(!text.contains("OTHER-CHAT-CONTENT"), "{text}");
    assert!(
        !text.contains("HIDDEN-SERVER-ASK") && !text.contains(&foreign),
        "{text}"
    );
    assert!(text.contains(&sampling), "{text}");
    let names: Vec<_> = out[1]["servers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["config_name"].clone())
        .collect();
    assert_eq!(names, vec![json!("mcp_stdio_open")]);
    assert!(!text.contains("zone-hdr-1"), "{text}");

    let (task, request) = ask(&app, &format!("(await engine('integrations.mcp_interaction_respond', id={sampling:?}, action='accept'))['data']")).await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Risky);
    assert_eq!(engine.warning.as_deref(), Some("Accepting lets this MCP server run a model call with the shown messages, which can send chat data to the server."));
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    assert_eq!(result(&finish(task).await)["delivered"], true);
    let response = tokio::time::timeout(Duration::from_secs(15), _rx1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.action, "accept");
    let m = run(&app, &format!("try:\n    await engine('integrations.mcp_interaction_respond', id={foreign:?}, action='accept')\n    r = 'asked'\nexcept TypeError as e:\n    r = str(e)\nr")).await;
    assert!(
        result(&m).as_str().unwrap().contains("no pending"),
        "{}",
        result(&m)
    );
    crate::integrations::mcp::mcp_interactions::cancel_mcp_interactions_for_config(&hidden).await;
}

#[tokio::test]
async fn integrations_save_delete_and_refusals() {
    let (dir, app, _) = fixture().await;
    let m = run(
        &app,
        "bad = []\nfor kw in [{'name': '../x', 'settings': {}}, {'name': 'unknown_x', 'settings': {}}, {'name': 'mcp_stdio_x', 'settings': {'oauth_tokens': {}}}]:\n    try:\n        await engine('integrations.save', **kw)\n    except (TypeError, PermissionError) as e:\n        bad.append(str(e))\nbad",
    )
    .await;
    assert_eq!(result(&m).as_array().unwrap().len(), 3, "{}", result(&m));
    let path = project(&dir).join(".refact/integrations.d/cmdline_zone.yaml");
    let (task, request) = ask(&app, "(await engine('integrations.save', name='cmdline_zone', settings={'command': 'echo zone', 'command_workdir': '', 'description': 'zone'}))['data']").await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Risky);
    assert_eq!(engine.title, "save an integration");
    assert!(
        engine
            .items
            .iter()
            .any(|i| i.label == "command" && i.new == Some(json!("echo zone"))),
        "{:?}",
        engine.items
    );
    assert_eq!(engine.warning.as_deref(), Some("This integration can run commands or send data to an external server without further confirmation rules you have not reviewed. It runs the command: echo zone."));
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    finish(task).await;
    assert!(!path.exists());
    let (task, request) = ask(&app, "(await engine('integrations.save', name='cmdline_zone', settings={'command': 'echo zone', 'command_workdir': '', 'description': 'zone', 'parameters': []}))['data']").await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let out = result(&finish(task).await);
    assert_eq!(out["created"], true, "{out}");
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("echo zone"));

    let (task, request) = ask(
        &app,
        "(await engine('integrations.delete', name='cmdline_zone'))['data']",
    )
    .await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(
        engine.class,
        refact_chat_api::PythonEngineClass::Destructive
    );
    assert_eq!(engine.items[0].old, Some(json!("cmdline_zone (project)")));
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    assert_eq!(result(&finish(task).await)["removed"], true);
    assert!(!path.exists());
}

#[tokio::test]
async fn mcp_oauth_logout_removes_tokens() {
    let (dir, app, _) = fixture().await;
    let path = write_integration(
        &dir,
        "mcp_http_zone.yaml",
        "url: http://127.0.0.1:1/mcp\nauth_type: oauth2_pkce\n",
    );
    app.gcx.integration_sessions.lock().await.insert(
        path.clone(),
        std::sync::Arc::new(tokio::sync::Mutex::new(Box::new(
            crate::integrations::mcp::session_mcp::SessionMCP::detached(&path),
        ))),
    );
    crate::integrations::mcp::mcp_auth::save_tokens_to_config(
        &path,
        &crate::integrations::mcp::mcp_auth::MCPOAuthTokens {
            access_token: "zone-access-1".into(),
            refresh_token: "zone-refresh-1".into(),
            expires_at: 0,
            client_id: "cid".into(),
            client_secret: None,
            scopes: vec![],
            bound_url: None,
        },
    )
    .await
    .unwrap();
    let (task, request) = ask(
        &app,
        "(await engine('integrations.mcp_oauth_logout', server='zone'))['data']",
    )
    .await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(
        engine.class,
        refact_chat_api::PythonEngineClass::Destructive
    );
    assert_eq!(engine.items[0].old, Some(json!("signed in")));
    assert!(!serde_json::to_string(&request)
        .unwrap()
        .contains("zone-access-1"));
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    assert_eq!(result(&finish(task).await)["signed_out"], true);
    assert!(
        crate::integrations::mcp::mcp_auth::load_tokens_from_config(&path)
            .await
            .is_none()
    );
}

#[tokio::test]
async fn integrations_save_shows_new_env_and_names_the_command_and_refuses_new_secrets() {
    let (dir, app, _) = fixture().await;
    let path = write_integration(
        &dir,
        "mcp_stdio_zone.yaml",
        "command: node server.js\nargs: [\"--port\", \"8\"]\nenv:\n  API_TOKEN: zone-kept-token\n  MODE: dev\n",
    );
    let before = std::fs::read_to_string(&path).unwrap();
    let (task, request) = ask(
        &app,
        "await engine('integrations.save', name='mcp_stdio_zone', settings={'env': {'NODE_OPTIONS': '--require /tmp/zone.js', 'API_TOKEN': '***'}})",
    )
    .await;
    let engine = request.engine.clone().unwrap();
    let env = engine
        .items
        .iter()
        .find(|i| i.label == "env.NODE_OPTIONS")
        .unwrap();
    assert_eq!(env.old, None);
    assert_eq!(env.new, Some(json!("--require /tmp/zone.js")));
    assert!(
        engine.items.iter().all(|i| i.label != "env.API_TOKEN"),
        "{:?}",
        engine.items
    );
    let warning = engine.warning.as_deref().unwrap();
    assert!(warning.contains("node server.js --port 8"), "{warning}");
    assert!(warning.contains("NODE_OPTIONS"), "{warning}");
    assert!(!serde_json::to_string(&request)
        .unwrap()
        .contains("zone-kept-token"));
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    finish(task).await;
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        saved.contains("zone-kept-token") && saved.contains("/tmp/zone.js"),
        "{saved}"
    );

    let m = run(
        &app,
        "try:\n    await engine('integrations.save', name='mcp_stdio_zone', settings={'env': {'API_TOKEN': 'zone-new-token'}})\n    r = 'asked'\nexcept PermissionError as e:\n    r = str(e)\nr",
    )
    .await;
    let text = result(&m).as_str().unwrap().to_string();
    assert!(
        text.contains("env.API_TOKEN") && text.contains("looks like a secret"),
        "{text}"
    );
    assert!(!text.contains("zone-new-token"), "{text}");
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("zone-kept-token"));
    assert_ne!(std::fs::read_to_string(&path).unwrap(), before);
}

#[tokio::test]
async fn integrations_stale_errors_do_not_quote_the_config() {
    let (dir, app, _) = fixture().await;
    let path = write_integration(
        &dir,
        "cmdline_zone.yaml",
        "command: echo zone\ncommand_workdir: ''\ndescription: zone\nparameters: []\n",
    );
    for code in [
        "r = await engine('integrations.save', name='cmdline_zone', settings={'description': 'other'})\n[r['ok'], r['error']]",
        "r = await engine('integrations.delete', name='cmdline_zone')\n[r['ok'], r['error']]",
    ] {
        let (task, request) = ask(&app, code).await;
        std::fs::write(&path, "command: echo zone\ncommand_workdir: ''\ndescription: zone\nparameters: []\ntoken: zone-moved-secret\n").unwrap();
        decide(&app, &request.request_id, Decision::AllowOnce).await.unwrap();
        let out = result(&finish(task).await);
        let error = out[1].as_str().unwrap();
        assert!(error.contains("changed since the preview"), "{error}");
        assert!(!error.contains("zone-moved") && !error.contains("echo zone"), "{error}");
        std::fs::write(&path, "command: echo zone\ncommand_workdir: ''\ndescription: zone\nparameters: []\n").unwrap();
    }
    assert!(std::path::Path::new(&path).exists());
}

#[tokio::test]
async fn agents_cancel_survives_steps_but_not_a_new_run() {
    let (_dir, app, _) = fixture().await;
    let (foreign, _, _) = app
        .agents
        .create(agent_request("other-root", "other-root", "foreign"))
        .await
        .unwrap();
    let (task, request) = ask(
        &app,
        &format!(
            "(await engine('agents.cancel', agent_id={:?}))['data']['cancelled_agent_ids']",
            foreign.agent_id
        ),
    )
    .await;
    app.agents
        .update_progress(&foreign.agent_id, "working".into(), 7)
        .await
        .unwrap();
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    assert_eq!(result(&finish(task).await), json!([foreign.agent_id]));
}

#[tokio::test]
async fn agents_message_shows_the_full_text_and_delivery_and_carries_privacy_records() {
    let (dir, app, _) = fixture().await;
    let (foreign, _, _) = app
        .agents
        .create(agent_request("other-root", "other-root", "foreign"))
        .await
        .unwrap();
    std::fs::write(project(&dir).join("zone_note.txt"), "zone note").unwrap();
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.zones = vec![refact_privacy::Zone {
        name: "zone_note".into(),
        patterns: vec!["zone_note.txt".into()],
        send_to: vec!["*".into()],
        on_shell_read: refact_privacy::ShellBehavior::Withhold,
    }];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
    let long = format!("{}zone-hidden-tail", "a".repeat(400));
    let (task, request) = ask(
        &app,
        &format!("open('zone_note.txt').read()\n(await engine('agents.message', agent_id={:?}, text={long:?}, push='preempt'))['ok']", foreign.agent_id),
    )
    .await;
    let engine = request.engine.clone().unwrap();
    let message = engine.items.iter().find(|i| i.label == "message").unwrap();
    assert_eq!(message.new, Some(json!(long)));
    let delivery = engine.items.iter().find(|i| i.label == "delivery").unwrap();
    assert!(
        delivery
            .new
            .as_ref()
            .unwrap()
            .as_str()
            .unwrap()
            .starts_with("preempt"),
        "{delivery:?}"
    );
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    assert_eq!(result(&finish(task).await), true);
    let record = app.agents.get_any(&foreign.agent_id).await.unwrap();
    let sent = &record.pending_deliveries[0].messages[0];
    assert!(
        sent.extra
            .get("privacy")
            .is_some_and(|p| p.to_string().contains("zone_note")),
        "{:?}",
        sent.extra
    );
}

#[tokio::test]
async fn agents_message_is_checked_against_the_agents_own_provider() {
    let (dir, app, _) = fixture().await;
    let mut request = agent_request("other-root", "other-root", "foreign");
    request.model = "zone-provider/m".into();
    let (foreign, _, _) = app.agents.create(request).await.unwrap();
    std::fs::write(project(&dir).join("zone_a.txt"), "guarded").unwrap();
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
    let m = run(
        &app,
        &format!("x = open('zone_a.txt').read()\ntry:\n    await engine('agents.message', agent_id={:?}, text='zone ' + x)\n    r = 'asked'\nexcept PermissionError as e:\n    r = str(e)\nr", foreign.agent_id),
    )
    .await;
    assert!(
        result(&m).as_str().unwrap().contains("was not sent"),
        "{}",
        result(&m)
    );
}

#[tokio::test]
async fn planner_instructions_are_checked_against_the_planner_model_and_shown_in_full() {
    let (dir, app, _) = fixture().await;
    planner_on(&app, &dir, "zone-planner/m").await;
    let task_id = crate::tasks::storage::create_task(app.gcx.clone(), "zone task")
        .await
        .unwrap()
        .id;
    let long = format!("{}zone-hidden-tail", "p".repeat(400));
    let (task, request) = ask(&app, &format!("await engine('agents.set_planner_instructions', task_id={task_id:?}, content={long:?})")).await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(engine.items[0].new, Some(json!(long)));
    assert_eq!(
        engine.items[1].new,
        Some(json!("the task planner on zone-planner/m"))
    );
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    finish(task).await;

    std::fs::write(project(&dir).join("zone_a.txt"), "guarded").unwrap();
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
    let m = run(
        &app,
        &format!("x = open('zone_a.txt').read()\ntry:\n    await engine('agents.set_planner_instructions', task_id={task_id:?}, content='zone ' + x)\n    r = 'asked'\nexcept PermissionError as e:\n    r = str(e)\nr"),
    )
    .await;
    assert!(
        result(&m).as_str().unwrap().contains("was not sent"),
        "{}",
        result(&m)
    );

    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.zones[0].send_to = vec!["model".into(), "zone-planner".into()];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
    let (task, request) = ask(
        &app,
        &format!("x = open('zone_a.txt').read()\n(await engine('agents.set_planner_instructions', task_id={task_id:?}, content='zone ' + x))['ok']"),
    )
    .await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    assert_eq!(result(&finish(task).await), true);
}

fn checkpoint_repo(root: &std::path::Path) {
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "zone@example.com"]);
    git(root, &["config", "user.name", "zone"]);
    git(root, &["checkout", "-q", "-b", "main"]);
    std::fs::write(root.join("zone_a.txt"), "a1\n").unwrap();
    std::fs::write(root.join("zone_b.txt"), "b1\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "init"]);
}

#[tokio::test]
async fn restore_checkpoint_lists_files_refuses_a_moved_tree_and_restores() {
    let (dir, app, ccx) = fixture().await;
    let root = project(&dir);
    checkpoint_repo(&root);
    let (checkpoint, _) = crate::git::checkpoints::create_workspace_checkpoint_for_root(
        app.gcx.clone(),
        &root,
        None,
        "python-test",
    )
    .await
    .unwrap();
    std::fs::write(root.join("zone_a.txt"), "a2\n").unwrap();
    {
        let mut message = crate::call_validation::ChatMessage::new("user".into(), "hi".into());
        message.checkpoints = vec![checkpoint];
        ccx.lock().await.messages = vec![message];
    }
    let restore =
        "r = await engine('agents.restore_checkpoint', message_index=0)\n[r['ok'], r['error']]";
    let task = {
        let ccx = ccx.clone();
        tokio::spawn(async move { run_ccx(ccx, restore).await })
    };
    let request = next_request(&app, &task).await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(
        engine.class,
        refact_chat_api::PythonEngineClass::Destructive
    );
    let labels: Vec<&str> = engine.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(
        labels,
        vec!["file", "files overwritten"],
        "{:?}",
        engine.items
    );
    assert_eq!(engine.items[0].old, Some(json!("zone_a.txt")));
    assert_eq!(engine.items[0].new, Some(json!("modified back")));
    assert_eq!(engine.items[1].new, Some(json!("1 in total")));
    assert!(engine
        .warning
        .as_deref()
        .unwrap()
        .contains("1 files are overwritten"));
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    finish(task).await;
    assert_eq!(
        std::fs::read_to_string(root.join("zone_a.txt")).unwrap(),
        "a2\n"
    );

    let task = {
        let ccx = ccx.clone();
        tokio::spawn(async move { run_ccx(ccx, restore).await })
    };
    let request = next_request(&app, &task).await;
    std::fs::write(root.join("zone_a.txt"), "a3\n").unwrap();
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
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
        std::fs::read_to_string(root.join("zone_a.txt")).unwrap(),
        "a3\n"
    );

    let task = {
        let ccx = ccx.clone();
        tokio::spawn(async move { run_ccx(ccx, restore).await })
    };
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let out = result(&finish(task).await);
    assert_eq!(out[0], true, "{out}");
    assert_eq!(
        std::fs::read_to_string(root.join("zone_a.txt")).unwrap(),
        "a1\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("zone_b.txt")).unwrap(),
        "b1\n"
    );
}

#[tokio::test]
async fn cleanup_worktrees_shows_uncommitted_work_and_deletes_only_on_approval() {
    let (dir, app, _) = fixture().await;
    let root = project(&dir);
    checkpoint_repo(&root);
    let service = crate::worktrees::service::WorktreeService::new(
        app.paths.cache_dir.clone(),
        dunce::canonicalize(&root).unwrap(),
    )
    .unwrap();
    let created = service
        .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
            branch: Some("refact/zone-cleanup".into()),
            kind: Some("chat".into()),
            ..Default::default()
        })
        .await
        .unwrap()
        .worktree
        .meta;
    std::fs::write(created.root.join("zone_a.txt"), "a1\nzone-edit\n").unwrap();
    std::fs::write(created.root.join("zone_new.txt"), "zone-new\n").unwrap();
    let code = format!(
        "(await engine('agents.cleanup_worktrees', ids=[{:?}], clean_only=False, min_age_hours=0))['data']",
        created.id
    );
    let (task, request) = ask(&app, &code).await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(
        engine.class,
        refact_chat_api::PythonEngineClass::Destructive
    );
    let old = engine.items[0]
        .old
        .as_ref()
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    assert!(old.contains("uncommitted: 2 files"), "{old}");
    let warning = engine.warning.as_deref().unwrap();
    assert!(
        warning.contains("1 of these worktrees hold uncommitted work (2 files"),
        "{warning}"
    );
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    finish(task).await;
    assert!(created.root.exists());

    let (task, request) = ask(&app, &code).await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let out = result(&finish(task).await);
    assert_eq!(out["deleted"].as_array().map(Vec::len), Some(1), "{out}");
    assert!(!created.root.exists());
}
