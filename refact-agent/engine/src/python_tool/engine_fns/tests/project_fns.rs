use super::*;

fn project(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("project")
}

#[tokio::test]
async fn index_status_and_settings_read_the_fixture() {
    let (dir, app, _) = fixture().await;
    *app.gcx.codegraph_error.lock().unwrap() = "boom Bearer abcdefghijklmnop".into();
    std::fs::write(
        project(&dir).join(".refact/indexing.yaml"),
        "blocklist:\n  - zone_a\nadditional_indexing_dirs: []\n",
    )
    .unwrap();
    let m = run(
        &app,
        "s = await engine('index.status')\np = await engine('index.status', parts=['file_index'])\nj = await engine('index.settings', scope='project')\ng = await engine('index.settings', scope='global')\n[s['data'], p['data'], j['data'], g['data']]",
    )
    .await;
    let out = result(&m);
    assert!(out.is_array(), "{}", m.content.content_text_only());
    assert_eq!(out[0]["codegraph"]["state"], "error");
    assert!(!out.to_string().contains("abcdefghijklmnop"), "{out}");
    assert_eq!(out[0]["vecdb"]["alive"], "turned_off");
    assert!(out[0]["file_index"].is_object());
    assert!(out[1].get("codegraph").is_none() && out[1]["file_index"].is_object());
    assert_eq!(out[2]["blocklist"], json!(["zone_a"]));
    assert_eq!(out[3]["scope"], "global");
    assert!(dir.path().join("config/indexing.yaml").exists());
}

#[tokio::test]
#[serial_test::serial(index_rescan)]
async fn index_rescan_enqueues_and_refuses_during_shutdown() {
    let (dir, app, _) = fixture().await;
    for name in ["zone_a.txt", "zone_b.txt", "zone_c.txt"] {
        std::fs::write(project(&dir).join(name), "zone text line\n").unwrap();
    }
    let task = spawn(&app, "(await engine('index.rescan'))['data']");
    let request = next_request(&app, &task).await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(engine.title, "rescan the workspace index");
    assert_eq!(
        engine.scope.as_deref(),
        Some("engine (all chats, all workspace folders)")
    );
    assert_eq!(engine.items[0].label, "files");
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let out = result(&finish(task).await);
    assert!(out["files_enqueued"].as_i64().unwrap() >= 3, "{out}");

    let task = spawn(&app, "await engine('index.rescan')");
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    assert_eq!(
        finish(task).await.extra["python"]["calls"][0]["approval"],
        "denied"
    );

    let task = spawn(
        &app,
        "r = await engine('index.rescan')\n[r['ok'], r['error']]",
    );
    let request = next_request(&app, &task).await;
    app.gcx
        .shutdown_flag
        .store(true, std::sync::atomic::Ordering::SeqCst);
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let out = result(&finish(task).await);
    app.gcx
        .shutdown_flag
        .store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(out[0], false, "{out}");
    assert!(out[1].as_str().unwrap().contains("shutting down"), "{out}");
}

async fn write_memories(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let knowledge = project(&dir).join(".refact/knowledge");
    std::fs::create_dir_all(&knowledge).unwrap();
    for (id, body) in [
        (
            "zone-one",
            "first note api_key=sk-abcdefghijklmnopqrstuvwxyz0123",
        ),
        ("zone-two", "second note"),
    ] {
        std::fs::write(
            knowledge.join(format!("{id}.md")),
            format!("---\nid: {id}\ntitle: {id}\ntags: [component:zone, workflow:zone]\nstatus: active\nkind: domain\n---\n\n{body}\n"),
        )
        .unwrap();
    }
    knowledge
}

#[tokio::test]
async fn knowledge_graph_bounds_nodes_masks_content_and_attaches_records() {
    let (dir, app, _) = fixture().await;
    write_memories(&dir).await;
    let m = run(
        &app,
        "a = await engine('knowledge.graph')\nb = await engine('knowledge.graph', include_content=True, kinds=['doc'])\nc = await engine('knowledge.graph', limit=1)\n[a['data'], b['data'], c['data']]",
    )
    .await;
    let out = result(&m);
    assert!(out.is_array(), "{}", m.content.content_text_only());
    assert_eq!(out[0]["stats"]["doc_count"], 2, "{out}");
    assert!(out[0]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|n| n.get("content").is_none()));
    let docs = out[1]["nodes"].as_array().unwrap();
    assert_eq!(docs.len(), 2);
    assert!(docs.iter().all(|n| n["content"].is_string()));
    assert!(!out
        .to_string()
        .contains("sk-abcdefghijklmnopqrstuvwxyz0123"));
    assert_eq!(out[2]["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(out[2]["truncated"], true);
    let files = m.extra["privacy"]["files"].as_array().unwrap();
    assert!(
        files
            .iter()
            .any(|f| f["path"].as_str().unwrap().ends_with("zone-one.md")),
        "{files:?}"
    );
}

#[tokio::test]
async fn knowledge_relink_previews_adds_links_and_refuses_moved_state() {
    let (dir, app, _) = fixture().await;
    let knowledge = write_memories(&dir).await;
    let task = spawn(&app, "(await engine('knowledge.relink'))['data']");
    let request = next_request(&app, &task).await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(engine.title, "relink memories");
    assert_eq!(engine.items[0].old, Some(json!("2")));
    assert!(engine
        .scope
        .as_deref()
        .unwrap()
        .starts_with("project knowledge ("));
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let out = result(&finish(task).await);
    assert_eq!(out["docs_scanned"], 2, "{out}");
    assert!(out["links_added"].as_u64().unwrap() >= 1, "{out}");

    let task = spawn(
        &app,
        "r = await engine('knowledge.relink')\n[r['ok'], r['error']]",
    );
    let request = next_request(&app, &task).await;
    std::fs::write(
        knowledge.join("zone-three.md"),
        "---\nid: zone-three\ntitle: zone-three\ntags: [component:zone, workflow:zone]\nstatus: active\nkind: domain\n---\n\nthird\n",
    )
    .unwrap();
    app.gcx
        .knowledge_generation
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
}

fn write_integration(dir: &tempfile::TempDir, name: &str, body: &str) -> String {
    let path = project(dir).join(".refact/integrations.d").join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, body).unwrap();
    path.to_string_lossy().into_owned()
}

#[tokio::test]
async fn integrations_list_masks_known_values_and_reports_errors() {
    let (dir, app, _) = fixture().await;
    std::fs::write(
        dir.path().join("config/variables.yaml"),
        "ZONE_VALUE: zone-literal-123\n",
    )
    .unwrap();
    write_integration(
        &dir,
        "cmdline_zone.yaml",
        "command: echo hi\nconfirmation:\n  ask_user: ['curl -H zone-literal-123 *']\n  deny: []\n",
    );
    write_integration(&dir, "mcp_http_zone.yaml", "url: http://127.0.0.1:1/mcp\n");
    write_integration(&dir, "cmdline_broken.yaml", "command: [unterminated\n");
    let m = run(
        &app,
        "a = await engine('integrations.list')\nb = await engine('integrations.list', kind='mcp')\nc = await engine('integrations.list', limit=1)\n[a['data'], b['data'], c['data']]",
    )
    .await;
    let out = result(&m);
    assert!(out.is_array(), "{}", m.content.content_text_only());
    let names: Vec<_> = out[0]["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"cmdline_zone") && names.contains(&"mcp_http_zone"),
        "{out}"
    );
    assert!(!out.to_string().contains("zone-literal-123"), "{out}");
    assert!(!out[0]["errors"].as_array().unwrap().is_empty(), "{out}");
    assert!(out[1]["integrations"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["kind"] == "mcp"));
    assert_eq!(out[2]["integrations"].as_array().unwrap().len(), 1);
    assert_eq!(out[2]["truncated"], true);
}

#[tokio::test]
async fn integrations_mcp_get_oauth_status_interactions_and_export() {
    let (dir, app, _) = fixture().await;
    let path = write_integration(
        &dir,
        "mcp_http_zone.yaml",
        "url: http://127.0.0.1:1/mcp\nheaders:\n  X-Zone: hdr-value-456\nauth_type: oauth2_pkce\n",
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
            access_token: "zone-access-789".into(),
            refresh_token: "zone-refresh-789".into(),
            expires_at: 1,
            client_id: "cid".into(),
            client_secret: None,
            scopes: vec!["read".into()],
            bound_url: None,
        },
    )
    .await
    .unwrap();
    let global = dir.path().join("config/integrations.d");
    std::fs::create_dir_all(&global).unwrap();
    std::fs::write(
        global.join("mcp_stdio_zone.yaml"),
        "command: zone-server\nenv:\n  ZONE: env-value-321\n",
    )
    .unwrap();
    let (id, _rx) = crate::integrations::mcp::mcp_interactions::create_mcp_interaction(
        &path,
        crate::integrations::mcp::mcp_interactions::MCPInteractionKind::Elicitation {
            message: "open https://h/p?token=qtok-1".into(),
            requested_schema: None,
            url: None,
            elicitation_id: None,
        },
    )
    .await;
    let m = run(
        &app,
        "g = await engine('integrations.mcp_get', server='zone')\no = await engine('integrations.mcp_oauth_status', server='zone')\ni = await engine('integrations.mcp_interactions')\ne = await engine('integrations.mcp_export')\nbad = []\ntry:\n    await engine('integrations.mcp_get', server='nothing')\nexcept TypeError as x:\n    bad.append(str(x))\n[g['data'], o['data'], i['data'], e['data'], bad]",
    )
    .await;
    crate::integrations::mcp::mcp_interactions::cancel_mcp_interactions_for_config(&path).await;
    let out = result(&m);
    assert!(out.is_array(), "{}", m.content.content_text_only());
    let text = out.to_string();
    for secret in [
        "hdr-value-456",
        "zone-access-789",
        "zone-refresh-789",
        "qtok-1",
        "env-value-321",
    ] {
        assert!(!text.contains(secret), "{secret} leaked: {text}");
    }
    assert_eq!(out[0]["name"], "zone");
    assert!(out[0].get("logs_tail").is_none());
    assert_eq!(out[1]["auth_type"], "oauth2_pkce");
    assert_eq!(out[1]["authenticated"], false);
    assert_eq!(out[1]["needs_login"], true);
    let interactions = out[2]["interactions"].as_array().unwrap();
    assert!(interactions.iter().any(|i| i["id"] == id.as_str()), "{out}");
    assert!(interactions.iter().all(|i| i.get("config_path").is_none()));
    let servers = out[3]["servers"].as_array().unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0]["config_name"], "mcp_stdio_zone");
    assert!(
        out[4][0].as_str().unwrap().contains("Unknown MCP server"),
        "{out}"
    );
}

fn exec_meta(chat: &str, command: &str) -> refact_exec::ExecProcessMeta {
    let mut meta =
        refact_exec::ExecProcessMeta::new(refact_exec::ExecMode::Background, command.into());
    meta.owner.chat_id = Some(chat.into());
    meta
}

#[tokio::test]
async fn processes_list_marks_ownership_masks_and_hides_foreign_paths() {
    let (dir, app, _) = fixture().await;
    let blocked = project(&dir).join("zone_a.txt");
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.blocked = vec![blocked.to_string_lossy().into_owned()];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
    let registry = &app.runtime.exec_registry;
    let mut _running = Vec::new();
    for meta in [
        exec_meta(
            "python-test",
            "API_KEY=sk-abcdefghijklmnopqrstuvwxyz0123 ./run",
        ),
        exec_meta("other-chat", &format!("cat {}", blocked.display())),
        exec_meta("other-chat", "sleep 100"),
    ] {
        _running.push(registry.register_test_process(meta).await);
    }
    let m = run(
        &app,
        "a = await engine('processes.list')\nb = await engine('processes.list', chat_id='other-chat', limit=1)\n[a['data'], b['data']]",
    )
    .await;
    let out = result(&m);
    assert!(out.is_array(), "{}", m.content.content_text_only());
    let rows = out[0]["processes"].as_array().unwrap();
    assert_eq!(rows.len(), 3, "{out}");
    assert_eq!(rows.iter().filter(|r| r["own"] == true).count(), 1);
    assert!(
        !out.to_string()
            .contains("sk-abcdefghijklmnopqrstuvwxyz0123"),
        "{out}"
    );
    assert!(!out.to_string().contains("zone_a.txt"), "{out}");
    assert!(rows
        .iter()
        .any(|r| r["command_preview"] == "<hidden: other chat>"));
    assert_eq!(out[1]["processes"].as_array().unwrap().len(), 1);
    assert_eq!(out[1]["total"], 2);
    assert_eq!(out[1]["truncated"], true);
}

#[tokio::test]
async fn processes_classify_raw_previews_and_preserve_own_identity() {
    let (dir, app, _) = fixture().await;
    let blocked = project(&dir).join("zone_a.txt");
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.blocked = vec![blocked.to_string_lossy().into_owned()];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
    let registry = &app.runtime.exec_registry;
    let mut running = Vec::new();
    for mut meta in [
        exec_meta("other-chat", &format!("cat token={}", blocked.display())),
        exec_meta(
            "other-chat",
            "sleep 100 api_key=sk-abcdefghijklmnopqrstuvwxyz0123",
        ),
        exec_meta("python-test", &format!("own token={}", blocked.display())),
    ] {
        meta.owner.workspace = Some(project(&dir));
        running.push(registry.register_test_process(meta).await);
    }
    let m = run(&app, "(await engine('processes.list'))['data']").await;
    let out = result(&m);
    let rows = out["processes"].as_array().unwrap();
    assert_eq!(rows.len(), 3, "{out}");
    let hidden = rows
        .iter()
        .find(|row| row["command_preview"] == "<hidden: other chat>")
        .unwrap();
    assert_eq!(hidden["own"], false);
    assert_eq!(hidden["process_id"], running[0].process_id().as_str());
    assert!(
        rows.iter().any(|row| row["command_preview"]
            .as_str()
            .unwrap()
            .starts_with("sleep 100")),
        "{out}"
    );
    let own = rows.iter().find(|row| row["own"] == true).unwrap();
    assert_eq!(own["process_id"], running[2].process_id().as_str());
    assert!(
        own["command_preview"].as_str().unwrap().starts_with("own "),
        "{out}"
    );
    assert!(!out.to_string().contains("zone_a.txt"), "{out}");
    assert!(
        !out.to_string()
            .contains("sk-abcdefghijklmnopqrstuvwxyz0123"),
        "{out}"
    );
    let task = spawn(
        &app,
        &format!(
            "await engine('processes.kill', process_id={:?})",
            running[0].process_id().as_str()
        ),
    );
    let request = next_request(&app, &task).await;
    let engine = request.engine.clone().unwrap();
    assert!(engine.items[0]
        .old
        .as_ref()
        .unwrap()
        .as_str()
        .unwrap()
        .starts_with("<hidden: other chat>"));
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    finish(task).await;
}

fn job_binding(chat: &str, tool: &str) -> crate::tool_jobs::JobBinding {
    crate::tool_jobs::JobBinding {
        scope_keys: vec![],
        chat_id: chat.into(),
        tool_call_id: format!("call-{chat}-{tool}"),
        tool_name: tool.into(),
        model: "zone/model".into(),
        turn_id: "turn".into(),
    }
}

async fn background_job(app: &AppState, chat: &str, tool: &str) -> String {
    app.runtime
        .tool_jobs
        .submit_background(job_binding(chat, tool), |_cancel| async {
            tokio::time::sleep(Duration::from_secs(120)).await;
            (
                crate::tool_jobs::JobStatus::Succeeded,
                crate::tool_jobs::JobResult {
                    had_corrections: false,
                    messages: vec![],
                    context_files: vec![],
                },
            )
        })
        .await
        .unwrap()
        .job_id
}

#[tokio::test]
async fn processes_jobs_and_cancel_job() {
    let (_dir, app, _) = fixture().await;
    let own = background_job(&app, "python-test", "search_pattern").await;
    let foreign = background_job(&app, "other-chat", "search_pattern").await;
    let editing = background_job(&app, "other-chat", "shell").await;
    let m = run(
        &app,
        "a = await engine('processes.jobs')\nb = await engine('processes.jobs', scope='all', limit=2)\nbad = []\ntry:\n    await engine('processes.cancel_job', chat_id='python-test', job_id='x')\nexcept TypeError as e:\n    bad.append(str(e))\n[a['data'], b['data'], bad]",
    )
    .await;
    let out = result(&m);
    assert!(out.is_array(), "{}", m.content.content_text_only());
    assert_eq!(out[0]["jobs"].as_array().unwrap().len(), 1);
    assert_eq!(out[0]["jobs"][0]["job_id"], own.as_str());
    assert_eq!(out[1]["total"], 3);
    assert_eq!(out[1]["truncated"], true);
    assert!(out[1]["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .all(|j| j.get("result").is_none()));
    assert!(out[2][0].as_str().unwrap().contains("tool_job"), "{out}");

    let task = spawn(
        &app,
        &format!("(await engine('processes.cancel_job', chat_id='other-chat', job_id='{foreign}'))['data']"),
    );
    let request = next_request(&app, &task).await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(engine.title, "cancel another chat's tool job");
    assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Change);
    assert_eq!(engine.scope.as_deref(), Some("chat other-chat"));
    assert_eq!(engine.items[0].new, Some(json!("cancel requested")));
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let out = result(&finish(task).await);
    assert_eq!(out["status"], "cancel_requested", "{out}");

    let task = spawn(
        &app,
        &format!("await engine('processes.cancel_job', chat_id='other-chat', job_id='{editing}')"),
    );
    let request = next_request(&app, &task).await;
    let engine = request.engine.clone().unwrap();
    assert_eq!(
        engine.class,
        refact_chat_api::PythonEngineClass::Destructive
    );
    assert!(engine.warning.unwrap().contains("partial effects"));
    decide(&app, &request.request_id, Decision::Deny)
        .await
        .unwrap();
    finish(task).await;
    let status = app
        .runtime
        .tool_jobs
        .get("other-chat", &editing)
        .unwrap()
        .status;
    assert_eq!(status, crate::tool_jobs::JobStatus::Running);

    let task = spawn(
        &app,
        &format!("r = await engine('processes.cancel_job', chat_id='other-chat', job_id='{editing}')\n[r['ok'], r['error']]"),
    );
    let request = next_request(&app, &task).await;
    app.runtime.tool_jobs.cancel("other-chat", &editing);
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
}

#[tokio::test]
async fn apps_details_bad_and_unknown_ids() {
    let (_dir, app, _) = fixture().await;
    let m = run(
        &app,
        "out = []\ntry:\n    await engine('apps.details', app_id='nope')\nexcept TypeError as e:\n    out.append(str(e))\nr = await engine('apps.details', app_id='00000000-0000-4000-8000-000000000000')\nout.append(r['ok'])\nout",
    )
    .await;
    let out = result(&m);
    assert!(out[0].as_str().unwrap().contains("app_id"), "{out}");
    assert_eq!(out[1], false, "{out}");
}

#[cfg(unix)]
#[tokio::test]
async fn apps_details_returns_metadata_without_chat_content() {
    let (dir, app, _) = fixture().await;
    super::agents_fns::scoped_session(&app, &dir.path().join("project")).await;
    let created = crate::apps::execute(
        &app,
        crate::apps::Caller::Host,
        crate::apps::Command::Create {
            slug: "zone-app".into(),
            name: "Zone App".into(),
        },
    )
    .await
    .unwrap();
    let id = created["manifest"]["app_id"].as_str().unwrap().to_string();
    {
        let session = session(&app).await;
        let mut session = session.lock().await;
        session.thread.title = format!("{} SECRET TITLE TAIL", "t".repeat(90));
        session.messages = vec![crate::call_validation::ChatMessage::new(
            "user".into(),
            "SECRET MESSAGE".into(),
        )];
    }
    let m = run(
        &app,
        &format!(
            "r = await engine('apps.details', app_id='{id}')\nr['data'] if r['ok'] else r['error']"
        ),
    )
    .await;
    let out = result(&m);
    assert_eq!(out["app"]["manifest"]["app_id"], id.as_str(), "{out}");
    assert!(out["coverage"].is_object());
    assert!(!out.to_string().contains("SECRET"), "{out}");
}
