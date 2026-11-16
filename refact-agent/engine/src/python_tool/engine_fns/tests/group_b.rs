use super::*;

fn project(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("project")
}

fn engine_of(
    request: &refact_chat_api::PythonApprovalRequest,
) -> refact_chat_api::PythonEngineRequest {
    request.engine.clone().expect("an engine card")
}

/// Runs `code` that makes one change, checks the card with `check`, then answers `decision`.
async fn card(
    app: &AppState,
    code: &str,
    decision: Decision,
    check: impl FnOnce(&refact_chat_api::PythonEngineRequest),
) -> ChatMessage {
    let task = spawn(app, code);
    let request = next_request(app, &task).await;
    check(&engine_of(&request));
    decide(app, &request.request_id, decision).await.unwrap();
    finish(task).await
}

async fn blocked_zone(app: &AppState, pattern: &str) {
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.zones = vec![refact_privacy::Zone {
        name: "zone_a".into(),
        patterns: vec![pattern.into()],
        send_to: vec!["nobody".into()],
        on_shell_read: refact_privacy::ShellBehavior::Withhold,
    }];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
}

/// A tool message in the chat history that read a file of a guarded zone.
fn guarded_history(
    app: &AppState,
    file: &std::path::Path,
) -> Vec<crate::call_validation::ChatMessage> {
    let mut read = crate::call_validation::ChatMessage::new("tool".into(), "guarded".into());
    read.tool_call_id = "cat-1".into();
    let record = crate::privacy::records::declared_file_record(&app.gcx, file).unwrap();
    crate::privacy::records::merge_records(&mut read, [record]);
    vec![
        crate::call_validation::ChatMessage::new("user".into(), "read it".into()),
        serde_json::from_value(json!({"role":"assistant","content":"","tool_calls":[{"id":"cat-1","type":"function","function":{"name":"cat","arguments":"{}"}}]})).unwrap(),
        read,
    ]
}

fn memory(dir: &tempfile::TempDir, id: &str, body: &str) -> std::path::PathBuf {
    let knowledge = project(dir).join(".refact/knowledge");
    std::fs::create_dir_all(&knowledge).unwrap();
    let path = knowledge.join(format!("{id}.md"));
    std::fs::write(
        &path,
        format!("---\nid: {id}\ntitle: {id}\ntags: [component:zone]\nfilenames: [secrets/zone_b.env, src/ok.rs]\nstatus: active\nkind: domain\n---\n\n{body}\n"),
    )
    .unwrap();
    path
}

#[tokio::test]
async fn knowledge_update_card_effect_stale_and_denied() {
    let (dir, app, _) = fixture().await;
    let path = memory(&dir, "zone-one", "old body");
    let file = path.to_string_lossy().into_owned();
    let m = card(
        &app,
        &format!("(await engine('knowledge.update', file_path={file:?}, title='renamed', tags=['component:zone']))['data']"),
        Decision::AllowOnce,
        |engine| {
            assert_eq!(engine.title, "edit a memory");
            assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Change);
            assert_eq!(engine.items[0].label, "title");
            assert_eq!(engine.items[0].new, Some(json!("renamed")));
            assert!(engine.scope.as_deref().unwrap().starts_with("project knowledge ("));
        },
    )
    .await;
    assert_eq!(result(&m)["updated_fields"], json!(["title", "tags"]));
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("title: \"renamed\""));

    let before = std::fs::read_to_string(&path).unwrap();
    card(
        &app,
        &format!("await engine('knowledge.update', file_path={file:?}, content='new body')"),
        Decision::Deny,
        |engine| {
            assert_eq!(
                engine.class,
                refact_chat_api::PythonEngineClass::Destructive
            );
            assert_eq!(
                engine.warning.as_deref(),
                Some("The memory's previous text is overwritten and the engine keeps no copy.")
            );
        },
    )
    .await;
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);

    let task = spawn(
        &app,
        &format!("r = await engine('knowledge.update', file_path={file:?}, content='new body')\n[r['ok'], r['error']]"),
    );
    let request = next_request(&app, &task).await;
    std::fs::write(&path, format!("{before}\nedited meanwhile\n")).unwrap();
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let out = result(&finish(task).await);
    assert_eq!(out[0], false);
    assert!(
        out[1]
            .as_str()
            .unwrap()
            .contains("changed since the preview"),
        "{out}"
    );
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("edited meanwhile"));

    let m = run(
        &app,
        "bad = []\nfor kw in [{'file_path': '../outside.md', 'title': 'x'}, {'file_path': 'x.md'}, {'file_path': 'x.md', 'status': 'gone'}]:\n    try:\n        await engine('knowledge.update', **kw)\n    except TypeError as e:\n        bad.append(str(e))\nbad",
    )
    .await;
    assert_eq!(result(&m).as_array().unwrap().len(), 3, "{}", result(&m));
}

#[tokio::test]
async fn knowledge_update_card_lists_normalised_tags_and_automatic_links_and_writes_them() {
    let (dir, app, _) = fixture().await;
    let path = memory(&dir, "zone-one", "first body");
    std::fs::write(
        project(&dir).join(".refact/knowledge/zone-two.md"),
        "---\nid: zone-two\ntitle: zone-two\ntags: [component:zone, workflow:zone]\nstatus: active\nkind: domain\n---\n\nsecond body\n",
    )
    .unwrap();
    let file = path.to_string_lossy().into_owned();
    let tags: Vec<String> = ["component:zone".to_string(), "workflow:zone".to_string()]
        .into_iter()
        .chain((0..20).map(|i| format!("Zone Tag {i}")))
        .collect();
    let mut shown = Value::Null;
    let m = card(
        &app,
        &format!("(await engine('knowledge.update', file_path={file:?}, tags={tags:?}))['ok']"),
        Decision::AllowOnce,
        |engine| shown = json!(engine.items),
    )
    .await;
    assert_eq!(result(&m), true, "{}", m.content.content_text_only());
    let item = |label: &str| {
        shown
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["label"] == label)
            .cloned()
            .unwrap_or_else(|| panic!("no {label} item in {shown}"))
    };
    let planned_tags = item("tags")["new"].as_array().unwrap().clone();
    assert_eq!(planned_tags.len(), 16, "{shown}");
    assert!(
        planned_tags
            .iter()
            .all(|t| t.as_str().unwrap() != "Zone Tag 0"),
        "{shown}"
    );
    assert!(planned_tags.contains(&json!("zone-tag-0")), "{shown}");
    assert_eq!(item("links")["new"], json!(["zone-two"]), "{shown}");
    let written = std::fs::read_to_string(&path).unwrap();
    let (frontmatter, _) = crate::knowledge_graph::KnowledgeFrontmatter::parse(&written);
    assert_eq!(json!(frontmatter.tags), json!(planned_tags), "{written}");
    assert_eq!(frontmatter.links, vec!["zone-two".to_string()], "{written}");
}

#[tokio::test]
async fn knowledge_update_shows_full_new_content_and_quotes_no_memory_text_when_stale() {
    let (dir, app, _) = fixture().await;
    let path = memory(&dir, "zone-long", "zone-old-body");
    let file = path.to_string_lossy().into_owned();
    let long = format!("{}zone-tail-instruction", "a".repeat(400));
    card(
        &app,
        &format!("await engine('knowledge.update', file_path={file:?}, content={long:?})"),
        Decision::Deny,
        |engine| {
            let content = engine.items.iter().find(|i| i.label == "content").unwrap();
            assert_eq!(content.new, Some(json!(long)));
        },
    )
    .await;
    let task = spawn(
        &app,
        &format!("r = await engine('knowledge.update', file_path={file:?}, content='zone-new')\n[r['ok'], r['error']]"),
    );
    let request = next_request(&app, &task).await;
    std::fs::write(
        &path,
        "---\ntitle: zone-moved-secret\n---\n\nzone-moved-body\n",
    )
    .unwrap();
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let out = result(&finish(task).await);
    let error = out[1].as_str().unwrap();
    assert!(
        error.contains("changed since the preview") && error.contains("memory"),
        "{error}"
    );
    for quoted in ["zone-old-body", "zone-moved-secret", "zone-moved-body"] {
        assert!(!error.contains(quoted), "{error}");
    }
}

#[tokio::test]
async fn knowledge_update_refuses_zoned_content_for_shared_memory() {
    let (dir, app, ccx) = fixture().await;
    let path = memory(&dir, "zone-one", "old body");
    let guarded = project(&dir).join("zone_a.txt");
    std::fs::write(&guarded, "secret").unwrap();
    blocked_zone(&app, "zone_a.txt").await;
    ccx.lock().await.messages = guarded_history(&app, &guarded);
    let m = run_ccx(
        ccx,
        &format!("try:\n    await engine('knowledge.update', file_path={:?}, content='copied')\n    r = 'asked'\nexcept PermissionError as e:\n    r = str(e)\nr", path.to_string_lossy()),
    )
    .await;
    assert!(
        result(&m).as_str().unwrap().contains("not sent"),
        "{}",
        result(&m)
    );
    assert!(session(&app)
        .await
        .lock()
        .await
        .runtime
        .python_approvals
        .is_empty());
}

#[tokio::test]
async fn knowledge_delete_archives_or_deletes_and_lists_incoming_links() {
    let (dir, app, _) = fixture().await;
    let target = memory(&dir, "zone-one", "first");
    let linking = project(&dir).join(".refact/knowledge/zone-two.md");
    std::fs::write(
        &linking,
        "---\nid: zone-two\ntitle: zone-two\nlinks: [zone-one]\nstatus: active\n---\n\nsecond\n",
    )
    .unwrap();
    let file = target.to_string_lossy().into_owned();
    let m = card(
        &app,
        &format!("(await engine('knowledge.delete', file_path={file:?}))['data']"),
        Decision::AllowOnce,
        |engine| {
            assert_eq!(engine.title, "archive a memory");
            assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Change);
            assert_eq!(engine.items[1].old, Some(json!("1 docs link here")));
        },
    )
    .await;
    assert_eq!(result(&m)["archived"], true);
    let m = run(
        &app,
        &format!("(await engine('knowledge.delete', file_path={file:?}))['text']"),
    )
    .await;
    assert_eq!(result(&m), "nothing to change");
    let m = card(
        &app,
        &format!("(await engine('knowledge.delete', file_path={file:?}, archive=False))['data']"),
        Decision::AllowOnce,
        |engine| {
            assert_eq!(
                engine.class,
                refact_chat_api::PythonEngineClass::Destructive
            );
            assert_eq!(
                engine.warning.as_deref(),
                Some("The memory file is removed from disk and cannot be restored by the engine.")
            );
        },
    )
    .await;
    assert_eq!(result(&m)["deleted"], true);
    assert!(!target.exists());
}

#[tokio::test]
async fn knowledge_graph_hides_referenced_files_of_blocked_zones() {
    let (dir, app, _) = fixture().await;
    memory(&dir, "zone-one", "first");
    blocked_zone(&app, "secrets/**").await;
    let m = run(
        &app,
        "(await engine('knowledge.graph', kinds=['file', 'doc']))['data']",
    )
    .await;
    let out = result(&m);
    let ids: Vec<&str> = out["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["id"].as_str())
        .collect();
    assert!(ids.iter().any(|id| id.ends_with("src/ok.rs")), "{ids:?}");
    assert!(!out.to_string().contains("zone_b.env"), "{out}");
}

#[tokio::test]
async fn enrichment_preview_needs_the_llm_budget_and_passes_the_egress_check() {
    let (dir, app, ccx) = fixture().await;
    coordination(&dir, "python_max_llm_calls: 0\n");
    let m = run(&app, "try:\n    await engine('knowledge.enrichment_preview', text='hello')\n    r = 'ran'\nexcept PermissionError as e:\n    r = str(e)\nr").await;
    assert!(
        result(&m)
            .as_str()
            .unwrap()
            .contains("python_max_llm_calls"),
        "{}",
        result(&m)
    );
    coordination(&dir, "");
    super::agents_fns::scoped_session(&app, &project(&dir)).await;
    {
        let mut state = app.gcx.caps_state.write().await;
        let mut caps = (**state.caps.as_ref().unwrap()).clone();
        caps.chat_models.insert(
            "cheap/light".into(),
            std::sync::Arc::new(crate::caps::ChatModelRecord {
                base: crate::caps::BaseModelRecord {
                    id: "cheap/light".into(),
                    tokenizer: "fake".into(),
                    n_ctx: 32_000,
                    endpoint: "https://example.invalid/v1".into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
        );
        caps.defaults.chat_light_model = "cheap/light".into();
        state.caps = Some(std::sync::Arc::new(caps));
    }
    let guarded = project(&dir).join("zone_a.txt");
    std::fs::write(&guarded, "secret").unwrap();
    blocked_zone(&app, "zone_a.txt").await;
    ccx.lock().await.messages = guarded_history(&app, &guarded);
    let m = run_ccx(ccx, "try:\n    r = await engine('knowledge.enrichment_preview', text='hello')\n    r = r['error']\nexcept PermissionError as e:\n    r = str(e)\n[r, (await engine('session.budget'))['data']['llm']]").await;
    let out = result(&m);
    assert!(out[0].as_str().unwrap().contains("not sent"), "{out}");
    assert_eq!(out[1]["calls_used"], 0, "{out}");
}

#[tokio::test]
async fn session_budget_reports_calls_llm_and_deadline() {
    let (dir, app, _) = fixture().await;
    coordination(
        &dir,
        "python_max_nested_calls: 30\npython_max_llm_calls: 4\n",
    );
    let m = run(
        &app,
        "await engine('session.limits')\n(await engine('session.budget'))['data']",
    )
    .await;
    let out = result(&m);
    assert_eq!(out["nested_calls"], json!({"used": 2, "max": 30}));
    assert_eq!(out["llm"]["enabled"], true);
    assert_eq!(out["llm"]["calls_max"], 4);
    assert!(out["deadline_in_ms"].as_u64().unwrap() > 0, "{out}");
    coordination(&dir, "python_max_llm_calls: 0\n");
    let m = run(&app, "(await engine('session.budget'))['data']['llm']").await;
    assert_eq!(result(&m), json!({"enabled": false}));
}

fn running_meta(chat: Option<&str>, command: &str) -> refact_exec::ExecProcessMeta {
    let mut meta =
        refact_exec::ExecProcessMeta::new(refact_exec::ExecMode::Background, command.into());
    meta.owner.chat_id = chat.map(String::from);
    meta
}

#[tokio::test]
async fn processes_kill_card_effect_and_refusals() {
    let (dir, app, _) = fixture().await;
    let registry = app.runtime.exec_registry.clone();
    let in_project = |chat: &str, command: &str| {
        let mut meta = running_meta(Some(chat), command);
        meta.owner.workspace = Some(project(&dir));
        meta
    };
    let mut foreign = registry
        .register_test_process(in_project("other-chat", "sleep 100"))
        .await;
    let own = registry
        .register_test_process(in_project("python-test", "sleep 100"))
        .await;
    let mut service_meta = in_project("other-chat", "server");
    service_meta.owner.service_name = Some("svc".into());
    let service = registry.register_test_process(service_meta).await;
    let nowhere = registry
        .register_test_process(running_meta(Some("other-chat"), "sleep 100"))
        .await;
    let (agent, _, _) = app
        .agents
        .create(crate::agents::types::CreateAgentRequest {
            parent_chat_id: "python-test".into(),
            parent_root_chat_id: Some("python-test".into()),
            parent_tool_call_id: None,
            kind: crate::agents::types::BgAgentKind::Subagent,
            config_name: "subagent".into(),
            title: "own".into(),
            prompt: "own".into(),
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
    let child = format!("zone-child-{}", agent.agent_id);
    app.agents
        .mark_running(&agent.agent_id, child.clone())
        .await
        .unwrap();
    let child_process = registry
        .register_test_process(in_project(&child, "sleep 100"))
        .await;
    let id = foreign.process_id().as_str().to_string();
    let m = run(
        &app,
        &format!("bad = []\nfor pid in [{:?}, {:?}, {:?}, {:?}]:\n    try:\n        await engine('processes.kill', process_id=pid)\n    except (TypeError, PermissionError) as e:\n        bad.append(str(e))\nbad", own.process_id().as_str(), service.process_id().as_str(), nowhere.process_id().as_str(), child_process.process_id().as_str()),
    )
    .await;
    let out = result(&m);
    assert!(out[0].as_str().unwrap().contains("process_kill"), "{out}");
    assert!(
        out[1].as_str().unwrap().contains("managed service"),
        "{out}"
    );
    assert!(
        out[2].as_str().unwrap().contains("workspace folders"),
        "{out}"
    );
    assert!(out[3].as_str().unwrap().contains("process_kill"), "{out}");

    card(
        &app,
        &format!("await engine('processes.kill', process_id={id:?})"),
        Decision::Deny,
        |engine| {
            assert_eq!(engine.title, "kill another chat's process");
            assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Destructive);
            assert_eq!(engine.items[1].new, Some(json!("chat other-chat")));
            assert_eq!(engine.items[1].old, None);
            assert_eq!(engine.warning.as_deref(), Some("The process is stopped immediately; any work it was doing is lost and its owning chat sees it fail."));
        },
    )
    .await;
    assert!(!registry
        .get(foreign.process_id())
        .await
        .unwrap()
        .status
        .is_terminal());

    let acknowledge = tokio::spawn(async move {
        foreign.acknowledge_kill().await;
        foreign
    });
    let task = spawn(
        &app,
        &format!("(await engine('processes.kill', process_id={id:?}))['data']"),
    );
    let request = next_request(&app, &task).await;
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let out = result(&finish(task).await);
    assert_eq!(out["status"], "killed", "{out}");
    tokio::time::timeout(Duration::from_secs(15), acknowledge)
        .await
        .unwrap()
        .unwrap();
}

#[test]
fn cancelling_any_job_except_read_only_tools_is_destructive() {
    for (tool, destructive) in [
        ("mcp_github_create_issue", true),
        ("cmdline_build", true),
        ("undo_textdoc", true),
        ("cat", false),
        ("search", true),
        ("codegraph_overview", false),
        ("search_pattern", false),
    ] {
        assert_eq!(
            super::super::processes::effectful(tool),
            destructive,
            "{tool}"
        );
    }
}

#[tokio::test]
async fn index_set_settings_classes_effect_and_refusals() {
    let (dir, app, _) = fixture().await;
    std::fs::write(
        project(&dir).join(".refact/indexing.yaml"),
        "blocklist:\n  - zone_a\nadditional_indexing_dirs: []\n",
    )
    .unwrap();
    card(
        &app,
        "await engine('index.set_settings', scope='project', blocklist=['zone_a', 'zone_b'])",
        Decision::Deny,
        |engine| {
            assert_eq!(engine.title, "change indexing settings");
            assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Change);
            assert!(engine.warning.is_none());
        },
    )
    .await;
    let m = card(
        &app,
        "(await engine('index.set_settings', scope='project', blocklist=[]))['data']",
        Decision::AllowOnce,
        |engine| {
            assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Risky);
            assert_eq!(engine.warning.as_deref(), Some("More files will be indexed, and with VecDB enabled their contents are sent to the embedding provider."));
            assert_eq!(engine.items[0].old, Some(json!("zone_a")));
        },
    )
    .await;
    assert_eq!(result(&m)["blocklist"], json!([]));
    let m = run(
        &app,
        "try:\n    await engine('index.set_settings', scope='project', additional_indexing_dirs=['../escape'])\n    r = 'asked'\nexcept TypeError as e:\n    r = str(e)\nr",
    )
    .await;
    assert!(
        result(&m).as_str().unwrap().contains("escape"),
        "{}",
        result(&m)
    );
    let m = run(
        &app,
        "(await engine('index.set_settings', scope='project', blocklist=[]))['text']",
    )
    .await;
    assert_eq!(result(&m), "nothing to change");
}

#[tokio::test]
async fn index_settings_stay_editable_while_a_listed_directory_is_blocked() {
    let (dir, app, _) = fixture().await;
    let blocked = project(&dir).join("zone_dir");
    let open_dir = project(&dir).join("open_dir");
    std::fs::create_dir_all(&blocked).unwrap();
    std::fs::create_dir_all(&open_dir).unwrap();
    std::fs::write(
        project(&dir).join(".refact/indexing.yaml"),
        format!(
            "blocklist: []\nadditional_indexing_dirs:\n  - {}\n  - {}\n",
            blocked.display(),
            open_dir.display()
        ),
    )
    .unwrap();
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.blocked = vec![blocked.to_string_lossy().into_owned()];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
    let keep = open_dir.to_string_lossy().into_owned();
    let m = card(
        &app,
        "(await engine('index.set_settings', scope='project', blocklist=['zone_build']))['ok']",
        Decision::AllowOnce,
        |engine| assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Change),
    )
    .await;
    assert_eq!(result(&m), true, "{}", m.content.content_text_only());
    let m = card(
        &app,
        &format!("(await engine('index.set_settings', scope='project', additional_indexing_dirs=[{keep:?}]))['ok']"),
        Decision::AllowOnce,
        |engine| assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Change),
    )
    .await;
    assert_eq!(result(&m), true, "{}", m.content.content_text_only());
    let m = run(
        &app,
        &format!("try:\n    await engine('index.set_settings', scope='project', additional_indexing_dirs=[{keep:?}, {:?}])\n    r = 'asked'\nexcept PermissionError as e:\n    r = str(e)\nr", blocked.to_string_lossy()),
    )
    .await;
    assert!(
        result(&m).as_str().unwrap().contains("privacy-blocked"),
        "{}",
        result(&m)
    );
}

#[tokio::test]
#[serial_test::serial(browser_settings)]
async fn browser_settings_patch_classes_and_refusals() {
    let (dir, app, _) = fixture().await;
    let path = dir.path().join("config/browser.yaml");
    let m = run(
        &app,
        "bad = []\nfor patch in [{'launch': {'extra_args': ['--remote-debugging-port=9']}}, {'viewport': {'desktop': {'scale_factor': 9.0}}}]:\n    try:\n        await engine('system.set_browser_settings', patch=patch)\n    except (TypeError, PermissionError) as e:\n        bad.append(str(e))\nbad",
    )
    .await;
    assert_eq!(result(&m).as_array().unwrap().len(), 2, "{}", result(&m));
    card(
        &app,
        "await engine('system.set_browser_settings', patch={'launch': {'chromium_sandbox': False}})",
        Decision::Deny,
        |engine| {
            assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Risky);
            assert_eq!(engine.items[0].label, "launch.chromium_sandbox");
            assert!(engine.warning.as_deref().unwrap().starts_with("This weakens browser isolation"));
        },
    )
    .await;
    assert!(!path.exists());
    let m = card(
        &app,
        "(await engine('system.set_browser_settings', patch={'lifecycle': {'idle_timeout_secs': 77}}))['ok']",
        Decision::AllowOnce,
        |engine| assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Change),
    )
    .await;
    assert_eq!(result(&m), true);
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("idle_timeout_secs: 77"));
    for (patch, label, phrase) in [
        (
            "{'launch': {'proxy_server': 'http://zone-proxy.local:3128'}}",
            "launch.proxy_server",
            "proxy",
        ),
        (
            "{'launch': {'proxy_bypass': 'zone.local'}}",
            "launch.proxy_bypass",
            "proxy",
        ),
        (
            "{'launch': {'downloads_dir': '/tmp/zone-downloads'}}",
            "launch.downloads_dir",
            "/tmp/zone-downloads",
        ),
    ] {
        card(
            &app,
            &format!("await engine('system.set_browser_settings', patch={patch})"),
            Decision::Deny,
            |engine| {
                assert_eq!(
                    engine.class,
                    refact_chat_api::PythonEngineClass::Risky,
                    "{label}"
                );
                assert_eq!(engine.items[0].label, label);
                assert!(
                    engine.warning.as_deref().unwrap().contains(phrase),
                    "{:?}",
                    engine.warning
                );
            },
        )
        .await;
    }
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        !saved.contains("zone-proxy")
            && !saved.contains("zone.local")
            && !saved.contains("zone-downloads"),
        "{saved}"
    );
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(browser_settings)]
async fn a_new_browser_program_is_risky_and_named_in_the_warning() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, app, _) = fixture().await;
    let program = dir.path().join("zone-chrome.sh");
    std::fs::write(&program, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let program = program.to_string_lossy().into_owned();
    let path = dir.path().join("config/browser.yaml");
    card(
        &app,
        &format!("await engine('system.set_browser_settings', patch={{'launch': {{'chrome_path': {program:?}}}}})"),
        Decision::Deny,
        |engine| {
            assert_eq!(engine.class, refact_chat_api::PythonEngineClass::Risky);
            assert_eq!(engine.items[0].label, "launch.chrome_path");
            assert_eq!(engine.items[0].new, Some(json!(program)));
            let warning = engine.warning.as_deref().unwrap();
            assert!(warning.contains(&program) && warning.contains("runs that file"), "{warning}");
        },
    )
    .await;
    assert!(
        !path.exists()
            || !std::fs::read_to_string(&path)
                .unwrap()
                .contains("zone-chrome")
    );
    let m = card(
        &app,
        &format!("(await engine('system.set_browser_settings', patch={{'launch': {{'chrome_path': {program:?}}}}}))['ok']"),
        Decision::AllowOnce,
        |_| {},
    )
    .await;
    assert_eq!(result(&m), true);
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("zone-chrome.sh"));
}

async fn store(app: &AppState, m: &ChatMessage) {
    session(app).await.lock().await.add_message(m.clone());
}

/// Polls the shutdown channel without blocking the runtime, so tasks that would send can run.
async fn received(
    receiver: &std::sync::mpsc::Receiver<String>,
    within: Duration,
) -> Option<String> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        if let Ok(message) = receiver.try_recv() {
            return Some(message);
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn system_shutdown_is_sent_only_after_the_run_message_is_stored() {
    let (_dir, app, _) = fixture().await;
    let app = crate::tools::tool_python::tests::with_tools(app);
    let (sender, receiver) = std::sync::mpsc::channel::<String>();
    *app.gcx.ask_shutdown_sender.lock().unwrap() = sender;
    let task = spawn(
        &app,
        "r = await engine('system.shutdown')\nawait fixture(ms=1500)\nr['data']",
    );
    let request = next_request(&app, &task).await;
    let engine = engine_of(&request);
    assert_eq!(
        engine.class,
        refact_chat_api::PythonEngineClass::Destructive
    );
    assert!(engine
        .warning
        .as_deref()
        .unwrap()
        .starts_with("Every chat and script stops"));
    decide(&app, &request.request_id, Decision::AllowOnce)
        .await
        .unwrap();
    let m = finish(task).await;
    assert_eq!(result(&m), json!({"status":"requested","completed":false}));
    assert_eq!(m.extra["python"]["status"], "completed");
    assert_eq!(m.extra["python"]["engine_changes_total"], 1);
    assert_eq!(
        received(&receiver, Duration::from_millis(500)).await,
        None,
        "the shutdown was sent before the run's message was stored"
    );
    store(&app, &m).await;
    assert_eq!(
        received(&receiver, Duration::from_secs(15))
            .await
            .as_deref(),
        Some("python engine function system.shutdown")
    );

    let (sender, receiver) = std::sync::mpsc::channel::<String>();
    *app.gcx.ask_shutdown_sender.lock().unwrap() = sender;
    let m = card(
        &app,
        "await engine('system.shutdown')",
        Decision::Deny,
        |_| {},
    )
    .await;
    store(&app, &m).await;
    assert_eq!(received(&receiver, Duration::from_millis(500)).await, None);
}

#[tokio::test]
async fn an_approved_shutdown_is_dropped_when_the_run_is_stopped_or_fails() {
    let (_dir, app, _) = fixture().await;
    let app = crate::tools::tool_python::tests::with_tools(app);
    let (sender, receiver) = std::sync::mpsc::channel::<String>();
    *app.gcx.ask_shutdown_sender.lock().unwrap() = sender;
    for (code, status) in [
        (
            "await engine('system.shutdown')\nawait fixture(ms=20000)",
            "cancelled",
        ),
        (
            "await engine('system.shutdown')\nraise ValueError('zone failure')",
            "failed",
        ),
    ] {
        let mut events = session(&app).await.lock().await.event_tx.subscribe();
        let task = spawn(&app, code);
        let request = next_request(&app, &task).await;
        decide(&app, &request.request_id, Decision::AllowOnce)
            .await
            .unwrap();
        if status == "cancelled" {
            tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    let applied = session(&app)
                        .await
                        .lock()
                        .await
                        .runtime
                        .python_runs
                        .iter()
                        .any(|r| {
                            r.calls.iter().any(|c| {
                                c.tool == "engine.system.shutdown" && c.duration_ms.is_some()
                            })
                        });
                    if applied {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("the shutdown change did not finish");
            app.python_tool
                .stop_run("python-test", &request.tool_call_id)
                .unwrap();
        }
        let m = finish(task).await;
        let python = &m.extra["python"];
        assert_eq!(python["status"], status, "{python}");
        assert!(python.get("engine_changes").is_none(), "{python}");
        assert!(python.get("engine_changes_total").is_none(), "{python}");
        let shutdown = python["calls"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["tool"] == "engine.system.shutdown")
            .unwrap();
        assert_eq!(shutdown["status"], "cancelled", "{shutdown}");
        assert_eq!(
            shutdown["summary"],
            format!("shutdown dropped: the run ended {status}"),
            "{shutdown}"
        );
        let run_dir = std::path::PathBuf::from(python["run_dir"].as_str().unwrap());
        let log: Value = serde_json::from_slice(
            &std::fs::read(run_dir.join(format!("calls/{}_engine.json", shutdown["seq"]))).unwrap(),
        )
        .unwrap();
        assert_eq!(log["record"]["status"], "cancelled", "{log}");
        assert_eq!(log["record"]["summary"], shutdown["summary"], "{log}");
        let mut live = None;
        while let Ok(event) = events.try_recv() {
            let event: Value = serde_json::from_str(&event).unwrap();
            if event["update"]["kind"] == "call_finished"
                && event["update"]["record"]["tool"] == "engine.system.shutdown"
            {
                live = Some(event["update"]["record"].clone());
            }
        }
        let live = live.expect("no live record of the shutdown call");
        assert_eq!(live["status"], "cancelled", "{live}");
        assert_eq!(live["summary"], shutdown["summary"], "{live}");
        let text = m.content.content_text_only();
        assert!(
            text.contains(&format!(
                "shutdown was not performed because the run ended {status}"
            )),
            "{text}"
        );
        store(&app, &m).await;
    }
    assert_eq!(received(&receiver, Duration::from_millis(500)).await, None);
}

#[tokio::test]
async fn a_failed_summary_write_keeps_the_run_completed() {
    let (_dir, app, _) = fixture().await;
    let app = crate::tools::tool_python::tests::with_tools(app);
    let task = spawn(&app, "await fixture(ms=1500)\n'zone done'");
    let run_id = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Some(run) = session(&app).await.lock().await.runtime.python_runs.first() {
                break run.run_id.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the run did not start");
    let workspace = crate::python_tool::workspace::PythonWorkspace::for_chat(&app, "python-test")
        .await
        .unwrap();
    let summary = workspace
        .run_dir(&run_id)
        .join(crate::python_tool::workspace::RUN_SUMMARY_FILE);
    std::fs::create_dir_all(summary.join("blocker")).unwrap();
    let m = finish(task).await;
    let python = &m.extra["python"];
    assert_eq!(python["status"], "completed", "{python}");
    assert_eq!(result(&m), json!("zone done"));
    let meta: Value = serde_json::from_slice(
        &std::fs::read(workspace.run_dir(&run_id).join("run.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(meta["status"], "completed");
    assert!(summary.is_dir());
}

#[tokio::test]
async fn scripts_cannot_read_the_bug_bundle_directory() {
    let (_dir, app, _) = fixture().await;
    let reports = app.gcx.cache_dir.join("bug-reports");
    std::fs::create_dir_all(&reports).unwrap();
    std::fs::write(reports.join("refact-bug-zone.zip"), "zone log tail").unwrap();
    let path = reports
        .join("refact-bug-zone.zip")
        .to_string_lossy()
        .into_owned();
    let m = run(
        &app,
        &format!("try:\n    r = open({path:?}).read()\nexcept PermissionError as e:\n    r = 'denied'\nr"),
    )
    .await;
    assert_eq!(result(&m), "denied", "{}", m.content.content_text_only());
}
