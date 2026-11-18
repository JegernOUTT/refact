use super::*;

fn project(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("project")
}

#[tokio::test]
async fn modes_list_reports_scope_and_broken_files() {
    let (dir, app, _) = fixture().await;
    let modes = project(&dir).join(".refact/modes");
    std::fs::create_dir_all(&modes).unwrap();
    let agent = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/crates/refact-yaml-configs/src/defaults/modes/agent.yaml"
    ))
    .unwrap()
    .replace("title: Agent", "title: Zone Agent");
    std::fs::write(modes.join("agent.yaml"), agent).unwrap();
    std::fs::write(modes.join("zone_broken.yaml"), "id: [unclosed").unwrap();
    let out = result(&run(&app, "(await engine('modes.list'))['data']").await);
    let agent = out["modes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == "agent")
        .cloned()
        .unwrap();
    assert_eq!(agent["scope"], "local", "{out}");
    assert_eq!(agent["title"], "Zone Agent");
    assert_eq!(agent["thread_defaults"]["auto_approve_editing_tools"], true);
    assert_eq!(out["current"], "agent");
    assert!(!out["errors"].as_array().unwrap().is_empty(), "{out}");
    assert!(out["modes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|m| m["specific"] == false));
}

#[tokio::test]
async fn modes_get_resolves_and_reads_raw_files() {
    let (dir, app, _) = fixture().await;
    let m = run(
        &app,
        "r = (await engine('modes.get', mode='agent', model='model'))['data']\nerrs = []\nfor kw in [dict(mode='agent', scope='local'), dict(mode='no_such_zone_mode'), dict(mode='../x')]:\n    try:\n        await engine('modes.get', **kw)\n        errs.append('ok')\n    except TypeError as e:\n        errs.append('TypeError')\n[r, errs]",
    )
    .await;
    let out = result(&m);
    assert!(
        !out[0]["config"]["tools"].as_array().unwrap().is_empty(),
        "{out}"
    );
    assert_eq!(out[0]["resolved_for_model"], "model");
    assert_eq!(out[1], json!(["TypeError", "TypeError", "TypeError"]));
    let modes = project(&dir).join(".refact/modes");
    std::fs::create_dir_all(&modes).unwrap();
    std::fs::write(modes.join("zone_mode.yaml"), "id: zone_mode\ntitle: Zone\n").unwrap();
    let raw = result(
        &run(
            &app,
            "(await engine('modes.get', mode='zone_mode', scope='local'))['data']",
        )
        .await,
    );
    assert_eq!(raw["config"]["title"], "Zone", "{raw}");
    assert_eq!(
        raw["version"],
        crate::yaml_configs::customization_service::version(b"id: zone_mode\ntitle: Zone\n")
    );
}

#[tokio::test]
async fn modes_skills_lists_skills_and_the_trigger_setting() {
    let (dir, app, _) = fixture().await;
    let skill = project(&dir).join(".refact/skills/zone-skill");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: zone-skill\ndescription: Zone skill\n---\nBody\n",
    )
    .unwrap();
    let out = result(&run(&app, "(await engine('modes.skills'))['data']").await);
    assert!(
        out["skills"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "zone-skill"),
        "{out}"
    );
    assert_eq!(out["auto_trigger"], "index_only");
    std::fs::write(
        project(&dir).join(".refact/skills.yaml"),
        "auto_trigger: off\n",
    )
    .unwrap();
    let out = result(
        &run(
            &app,
            "(await engine('modes.skills'))['data']['auto_trigger']",
        )
        .await,
    );
    assert_eq!(out, "off");
}

#[tokio::test]
async fn modes_project_info_lists_blocks_and_carries_records_with_content() {
    let (dir, app, _) = fixture().await;
    std::fs::write(
        project(&dir).join("AGENTS.md"),
        "zone instructions token=abcdefghijklmnopqrst\n",
    )
    .unwrap();
    let sizes = run(&app, "(await engine('modes.project_info'))['data']").await;
    let out = result(&sizes);
    let block = out["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["section"] == "instruction_files")
        .cloned()
        .unwrap();
    assert!(block.get("content").is_none(), "{out}");
    let full = run(
        &app,
        "(await engine('modes.project_info', content=True))['data']",
    )
    .await;
    let out = result(&full);
    let block = out["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["title"] == "AGENTS.md")
        .cloned()
        .unwrap();
    assert!(
        block["content"]
            .as_str()
            .unwrap()
            .contains("zone instructions"),
        "{out}"
    );
    assert!(!out.to_string().contains("abcdefghijklmnopqrst"));
    let files = full
        .extra
        .get("privacy")
        .map(|p| p["files"].clone())
        .unwrap_or_default()
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        files
            .iter()
            .any(|f| f["path"].as_str().is_some_and(|p| p.ends_with("AGENTS.md"))),
        "{:?}",
        full.extra
    );
}

#[tokio::test]
async fn tools_list_reports_groups_gating_and_this_chat() {
    let (_dir, app, _) = fixture().await;
    assert!(app.gcx.codegraph.lock().await.is_none());
    let out = result(&run(&app, "(await engine('tools.list'))['data']").await);
    let groups = out["groups"].as_array().unwrap();
    assert!(!groups.is_empty(), "{out}");
    let tools: Vec<&Value> = groups
        .iter()
        .flat_map(|g| g["tools"].as_array().unwrap())
        .collect();
    assert!(tools
        .iter()
        .all(|t| t.get("description").is_none() && t.get("input_schema").is_none()));
    assert!(
        out["gated"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["capability"] == "code graph index"
                && g["enablement"] == "enable code graph indexing"),
        "{out}"
    );
}

async fn set_catalog(app: &AppState) -> AppState {
    let app = crate::tools::tool_python::tests::with_tools(app.clone());
    let catalog = app
        .tool_registry
        .acquire_tool_catalog("agent", Some("model"), None)
        .await;
    let session = session(&app).await;
    let mut s = session.lock().await;
    s.thread.model = "model".into();
    s.tool_catalog = Some(catalog);
    drop(s);
    app
}

#[tokio::test]
async fn tools_check_uses_the_chat_policy_and_refuses_unknown_tools() {
    let (_dir, app, _) = fixture().await;
    let app = set_catalog(&app).await;
    let m = run(
        &app,
        "c = (await engine('tools.check', tool='fixture', args={'n': 1}))['data']\nbad = None\ntry:\n    await engine('tools.check', tool='zone_missing_tool', args={})\nexcept TypeError as e:\n    bad = str(e)\n[c, bad]",
    )
    .await;
    let out = result(&m);
    assert_eq!(out[0]["tool"], "fixture", "{out}");
    assert!(["auto", "ask", "deny"].contains(&out[0]["decision"].as_str().unwrap()));
    assert!(out[1]
        .as_str()
        .unwrap()
        .contains("not in this chat's tools"));
}

#[tokio::test]
async fn tools_check_on_shell_writes_no_audit_line() {
    let (dir, app, _) = fixture().await;
    let mut catalog = (*app
        .tool_registry
        .acquire_tool_catalog("agent", Some("model"), None)
        .await)
        .clone();
    if !catalog.index.tools.iter().any(|t| t.name == "shell") {
        let mut shell = catalog.index.tools[0].clone();
        shell.name = "shell".into();
        catalog.index.tools.push(shell);
    }
    session(&app).await.lock().await.tool_catalog = Some(Arc::new(catalog));
    let out = result(&run(&app, "[(await engine('tools.check', tool='shell', args={'command': 'sudo ls', 'description': 'x'}))['data'], (await engine('tools.check', tool='shell', args={'command': 'ls', 'description': 'x'}))['data']]").await);
    assert_eq!(out[0]["decision"], "deny", "{out}");
    assert_eq!(out[0]["layers"]["tool_rule"], "deny");
    assert_eq!(out[1]["layers"]["tool_rule"], "pass");
    assert!(!project(&dir).join(".refact/shell_audit.jsonl").exists());
}

#[tokio::test]
async fn tools_shell_policy_test_and_audit() {
    let (dir, app, _) = fixture().await;
    let out = result(&run(&app, "[(await engine('tools.shell_policy'))['data'], (await engine('tools.shell_test', command='sudo ls'))['data'], (await engine('tools.shell_test', command='ls'))['data']]").await);
    assert_eq!(
        out[0]["mode"],
        serde_json::to_value(crate::tools::shell_gate::ShellGatePolicy::default().mode).unwrap(),
        "{out}"
    );
    assert!(out[0].get("catalogue").is_none());
    assert_eq!(out[1]["decision"], "deny");
    assert_eq!(out[2]["decision"], "pass");
    std::fs::write(
        project(&dir).join(".refact/shell_policy.yaml"),
        "mode: strict\n",
    )
    .unwrap();
    let mode = result(
        &run(
            &app,
            "(await engine('tools.shell_policy', catalogue=True))['data']",
        )
        .await,
    );
    assert_eq!(mode["mode"], "strict", "{mode}");
    assert!(mode["catalogue"].is_array());
    let lines: Vec<String> = (0..3)
        .map(|i| json!({"ts_ms":i,"chat_id":"python-test","command":format!("curl -H \"Authorization: Bearer abcdefghijklmnop{i}\" https://h"),"decision":"confirmation","layer":"ask-rule","rule":"curl"}).to_string())
        .collect();
    std::fs::write(
        project(&dir).join(".refact/shell_audit.jsonl"),
        lines.join("\n") + "\n",
    )
    .unwrap();
    let audit = result(&run(&app, "(await engine('tools.shell_audit', limit=2))['data']").await);
    assert_eq!(audit["entries"].as_array().unwrap().len(), 2, "{audit}");
    assert_eq!(audit["total"], 3);
    assert_eq!(audit["truncated"], true);
    assert!(!audit.to_string().contains("abcdefghijklmnop"), "{audit}");
}

#[tokio::test]
async fn hooks_list_and_get_scrub_commands() {
    let (dir, app, _) = fixture().await;
    std::fs::write(
        project(&dir).join(".refact/hooks.yaml"),
        "hooks:\n  PreToolUse:\n    - matcher: \"shell\"\n      hooks:\n        - type: command\n          command: \"curl -H 'Authorization: Bearer abcdefghijklmnop' https://h\"\n          timeout: 5\n  PostToolUse:\n    - hooks:\n        - type: command\n          command: \"./zone_notify.sh\"\n",
    )
    .unwrap();
    let out = result(&run(&app, "[(await engine('hooks.list'))['data'], (await engine('hooks.get', scope='local'))['data']]").await);
    let registrations = out[0]["registrations"].as_array().unwrap();
    assert_eq!(registrations.len(), 2, "{out}");
    assert!(registrations
        .iter()
        .all(|r| r["trust"] == "untrusted_project" && r["runs"] == false));
    assert!(out[0]["recent"].is_array());
    let hooks = out[1]["hooks"].as_array().unwrap();
    assert_eq!(
        hooks.iter().map(|h| h["index"].clone()).collect::<Vec<_>>(),
        [json!(0), json!(1)]
    );
    let pre = hooks.iter().find(|h| h["event"] == "PreToolUse").unwrap();
    assert_eq!(
        (pre["timeout"].clone(), pre["matcher"].clone()),
        (json!(5), json!("shell"))
    );
    assert!(!out.to_string().contains("abcdefghijklmnop"), "{out}");
    let bad = run(&app, "await engine('hooks.get', scope='nowhere')").await;
    assert_eq!(bad.extra["python"]["error"]["type"], "TypeError");
}

async fn zone_policy(app: &AppState) {
    let mut policy = (*app.gcx.privacy_policy_load.read().unwrap().policy).clone();
    policy.blocked = vec!["zone_secrets/**".into()];
    policy.zones = vec![refact_privacy::Zone {
        name: "zone_a".into(),
        patterns: vec!["zone_a.txt".into()],
        send_to: vec!["zone".into()],
        on_shell_read: refact_privacy::ShellBehavior::Withhold,
    }];
    crate::privacy::save_privacy_policy(app.gcx.clone(), policy)
        .await
        .unwrap();
}

#[tokio::test]
async fn privacy_status_policy_and_check() {
    let (dir, app, _) = fixture().await;
    zone_policy(&app).await;
    std::fs::create_dir_all(project(&dir).join("zone_secrets")).unwrap();
    std::fs::write(project(&dir).join("zone_secrets/a.txt"), "x").unwrap();
    std::fs::write(project(&dir).join("zone_a.txt"), "x").unwrap();
    let out = result(&run(&app, "[(await engine('privacy.status'))['data'], (await engine('privacy.policy'))['data'], (await engine('privacy.check', paths=['zone_secrets/a.txt', 'zone_a.txt'], destination='zone/model-a'))['data'], (await engine('privacy.check', paths=['zone_a.txt'], destination='other/model'))['data']]").await);
    assert!(out[0]["config_error"].is_null(), "{out}");
    assert!(out[0]["observation"].is_object());
    assert_eq!(out[1]["policy"]["zones"][0]["name"], "zone_a");
    assert_eq!(out[2]["paths"][0]["zone"], "blocked");
    assert_eq!(out[2]["paths"][0]["sendable"], false);
    assert_eq!(out[2]["paths"][1]["zone"], "zone_a");
    assert_eq!(out[2]["paths"][1]["sendable"], true);
    assert_eq!(out[3]["paths"][0]["sendable"], false);
}

#[tokio::test]
async fn privacy_chat_lists_records_and_clears_destinations() {
    let (dir, app, _) = fixture().await;
    zone_policy(&app).await;
    {
        let session = session(&app).await;
        let mut s = session.lock().await;
        let mut tool = ChatMessage::new("tool".into(), "zone text".into());
        crate::privacy::records::attach_record(
            &mut tool,
            refact_privacy::FileRecord {
                path: project(&dir)
                    .join("zone_a.txt")
                    .to_string_lossy()
                    .into_owned(),
                zone: "zone_a".into(),
                attribution: refact_privacy::Attribution::Declared,
            },
        );
        s.messages.push(tool);
    }
    let out = result(&run(&app, "[(await engine('privacy.chat', destination='zone/model-a'))['data'], (await engine('privacy.chat', destination='other/model'))['data']]").await);
    assert_eq!(out[0]["records"][0]["zone"], "zone_a", "{out}");
    assert_eq!(out[0]["sendable"], true);
    assert_eq!(out[1]["sendable"], false);
    assert_eq!(out[1]["blocked"][0]["zone"], "zone_a");
    assert!(out[1].get("messages").is_none() && out[1].get("would_send").is_none());
    let other = run(
        &app,
        "await engine('privacy.chat', chat_id='zone-not-loaded')",
    )
    .await;
    assert_eq!(other.extra["python"]["error"]["type"], "PermissionError");
}
