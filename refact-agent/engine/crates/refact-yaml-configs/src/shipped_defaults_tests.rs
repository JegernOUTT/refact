use crate::project_configs_bootstrap::{
    compute_checksum, embedded_defaults, global_configs_try_create_all,
};
use serde::Deserialize;
use std::collections::HashMap;
use tokio::fs;

const BOOTSTRAP_KINDS: [&str; 4] = ["modes", "subagents", "toolbox_commands", "code_lens"];

type ShippedDefaults = HashMap<String, Vec<(u32, String)>>;

#[derive(Deserialize)]
struct SchemaVersion {
    schema_version: u32,
}

fn shipped_defaults() -> ShippedDefaults {
    serde_json::from_str(include_str!("shipped_defaults.json"))
        .expect("shipped defaults manifest should be valid JSON")
}

#[test]
fn every_embedded_default_is_recorded_in_the_manifest() {
    let manifest = shipped_defaults();

    for kind in BOOTSTRAP_KINDS {
        for (filename, content) in embedded_defaults(kind) {
            let key = format!("{kind}/{filename}");
            let version = serde_yaml::from_str::<SchemaVersion>(&content)
                .expect("embedded default should contain a valid schema_version")
                .schema_version;
            let checksum = compute_checksum(&content);
            let entries = manifest.get(&key).map(Vec::as_slice).unwrap_or(&[]);
            let max_recorded = entries
                .iter()
                .map(|(version, _)| *version)
                .max()
                .unwrap_or(0);
            let recorded = entries
                .iter()
                .any(|entry| entry == &(version, checksum.clone()));

            assert!(
                recorded,
                "{key} changed: bump schema_version above {max_recorded} and append [{version}, {checksum}] to shipped_defaults.json"
            );
        }
    }
}

#[test]
fn manifest_versions_are_unique_and_increasing() {
    for (key, entries) in shipped_defaults() {
        let mut previous = None;
        for (version, _) in entries {
            assert!(
                previous.map_or(true, |previous| version > previous),
                "{key} manifest schema versions must be unique and strictly increasing"
            );
            previous = Some(version);
        }
    }
}

#[test]
fn shipped_modes_exclude_native_sync_and_recommend_shell_git() {
    for (filename, content) in embedded_defaults("modes") {
        assert!(!content.contains("worktree_sync"), "{filename}");
        if [
            "agent.yaml",
            "solo.yaml",
            "design.yaml",
            "task_agent.yaml",
            "task_planner.yaml",
        ]
        .contains(&filename.as_str())
        {
            assert!(
                content.contains("shell Git status, merge, add and commit"),
                "{filename}"
            );
        }
    }
}

#[tokio::test]
async fn bumped_default_upgrades_an_unmodified_install() {
    let manifest = shipped_defaults();
    let main_ask = r###"schema_version: 14
id: ask
title: Ask
description: Quick Q&A with web research capability
specific: false

thread_defaults:
  include_project_info: false
  checkpoints_enabled: false
  auto_approve_editing_tools: false
  auto_approve_dangerous_commands: false

ui:
  order: 5
  tags: [chat, research]

prompt: |
  You are Buddy, a persistent coding companion for this project. Today your job is to answer questions directly and concisely.

  %BUDDY_PERSONALITY%

  For factual questions, use your knowledge. For current information, APIs, or technical accuracy, use web tools.

  Guidelines:
  - Be direct and helpful
  - Use `web_search` for quick lookups
  - Use `web` to fetch specific URLs
  - If the question requires codebase access, suggest switching to Explore or Agent mode

  ## Background Subagents
  Use `subagent()` for complex research that needs multiple web searches:
  - "Compare options" → `subagent(task="Research and compare X vs Y", expected_result="Comparison table with pros/cons", tools="web_search,web")`
  - "Gather documentation" → `subagent(task="Find official docs for API X", expected_result="Summary with links", tools="web_search,web")`

  Use `agents_overview()` before spawning and restrict `tools` to read-only capabilities for research. Use `agent_message()` to steer or answer background agents, and `agent_list()`, `agent_status()`, `agent_wait()`, `agent_result()`, and `agent_cancel()` to inspect or control them.

  ## Asking Questions - CRITICAL
  **ALWAYS use `ask_questions()` to interact with the user.** Never ask questions in plain text - use the tool.

  Use `ask_questions()` whenever you:
  - Need clarification on the question
  - Want to narrow down the scope
  - Have follow-up questions
  - Need user preferences

  Example: `ask_questions(questions='[{"id":"q1","type":"free_text","text":"Can you be more specific about what you need?"}]')`

  %TRAJECTORY_LABEL_INSTRUCTIONS%

  %RICH_CONTENT_INSTRUCTIONS%

  %SYSTEM_INFO%

  %MODELS_INFO%

allow_integrations: false
allow_mcp: true
allow_subagents: false

tools:
  - set_trajectory_label
  - codegraph_overview
  - code_health
  - git_risk
  - code_why
  - code_duplication
  - dead_code
  - security_scan
  - pr_blast
  - code_map
  - add_workspace_folder
  - web
  - web_search
  - subagent
  - agents_overview
  - agent_message
  - agent_mailbox
  - agent_peer_grant
  - agent_list
  - agent_status
  - agent_wait
  - agent_result
  - agent_cancel
  - ask_questions

tool_confirm:
  rules:
    - match: "codegraph_overview"
      action: auto
    - match: "code_health"
      action: auto
    - match: "git_risk"
      action: auto
    - match: "code_why"
      action: auto
    - match: "code_duplication"
      action: auto
    - match: "dead_code"
      action: auto
    - match: "security_scan"
      action: auto
    - match: "pr_blast"
      action: auto
    - match: "code_map"
      action: auto
    - match: "subagent"
      action: auto
    - match: "agents_overview"
      action: auto
    - match: "agent_message"
      action: auto
    - match: "agent_peer_grant"
      action: ask
    - match: "*"
      action: auto
"###;
    let expected_baseline = manifest
        .get("modes/ask.yaml")
        .and_then(|entries| entries.iter().find(|(version, _)| *version == 14))
        .map(|(_, checksum)| checksum)
        .expect("manifest should contain main's ask.yaml baseline");
    assert_eq!(compute_checksum(&main_ask), *expected_baseline);

    let temp = tempfile::tempdir().expect("temporary config directory should be created");
    let config_dir = temp.path();
    let ask_path = config_dir.join("modes/ask.yaml");
    fs::create_dir_all(ask_path.parent().expect("ask.yaml has a parent"))
        .await
        .expect("modes directory should be created");
    fs::write(&ask_path, &main_ask)
        .await
        .expect("main's ask.yaml should be installed");
    let checksums = HashMap::from([("modes/ask.yaml".to_string(), expected_baseline.clone())]);
    fs::write(
        config_dir.join("default-checksums.yaml"),
        serde_yaml::to_string(&checksums).expect("baseline should serialize"),
    )
    .await
    .expect("main's checksum baseline should be installed");

    global_configs_try_create_all(config_dir)
        .await
        .expect("bootstrap should upgrade the installed default");

    let installed = fs::read_to_string(ask_path)
        .await
        .expect("upgraded ask.yaml should be readable");
    let embedded_ask = embedded_defaults("modes")
        .into_iter()
        .find(|(filename, _)| filename == "ask.yaml")
        .map(|(_, content)| content)
        .expect("embedded mode ask.yaml should exist");
    assert_eq!(installed, embedded_ask);
    assert!(installed.contains("  - agent_mailbox"));
}

#[tokio::test]
async fn organization_bootstrap_installs_exact_semantic_no_tools_builtin() {
    let dir = tempfile::tempdir().unwrap();
    global_configs_try_create_all(dir.path()).await.unwrap();
    let text = fs::read_to_string(dir.path().join("subagents/buddy_organization.yaml"))
        .await
        .unwrap();
    let config: crate::customization_types::SubagentConfig = serde_yaml::from_str(&text).unwrap();
    assert_eq!(config.id, "buddy_organization");
    assert!(!config.expose_as_tool);
    assert!(config.tools.is_empty());
    let value: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
    assert_eq!(value["subchat"]["model_type"].as_str(), Some("buddy"));
    assert_eq!(value["subchat"]["max_steps"].as_u64(), Some(1));
    let system = config.messages.system_prompt.unwrap();
    assert!(system.contains("topic_name"));
    assert!(system.contains("tag_names"));
    assert!(system.contains("fingerprint"));
    assert!(!system.contains("expected_revision"));
}

const AUDIT_BASELINE_MODES: &[(&str, u32)] = &[
    ("agent", 31),
    ("ask", 15),
    ("brainstorming", 10),
    ("buddy", 21),
    ("configurator", 15),
    ("debug", 26),
    ("design", 29),
    ("explore", 22),
    ("learn", 24),
    ("past_work", 15),
    ("plan", 23),
    ("setup", 24),
    ("setup_agents_md", 25),
    ("setup_commands", 25),
    ("setup_hooks", 23),
    ("setup_knowledge", 23),
    ("setup_mcp", 24),
    ("setup_modes", 24),
    ("setup_skills", 25),
    ("setup_subagents", 22),
    ("shell", 15),
    ("solo", 3),
    ("task_agent", 30),
    ("task_planner", 34),
];

fn audit_baseline(name: &str, version: u32) -> String {
    format!("schema_version: {version}\nid: {name}\ntools: [cat, worktree_merge]\nprompt: |\n  Previous installed mode fixture.\n")
}

#[test]
fn audit_modes_preserve_baselines_and_record_current_revision() {
    let manifest = shipped_defaults();
    let embedded: HashMap<_, _> = embedded_defaults("modes").into_iter().collect();
    assert_eq!(AUDIT_BASELINE_MODES.len(), 24);
    assert_eq!(embedded.len(), AUDIT_BASELINE_MODES.len());
    for &(name, old_version) in AUDIT_BASELINE_MODES {
        let key = format!("modes/{name}.yaml");
        let entries = &manifest[&key];
        assert!(entries.iter().any(|(version, _)| *version == old_version));
        let previous_max = entries[..entries.len() - 1]
            .iter()
            .map(|(v, _)| *v)
            .max()
            .unwrap();
        let text = &embedded[&format!("{name}.yaml")];
        let mode: crate::customization_types::ModeConfig = serde_yaml::from_str(text).unwrap();
        assert_eq!(
            mode.schema_version,
            previous_max.max(old_version) + 1,
            "single revision bump for {name}"
        );
        assert_eq!(
            entries.last().unwrap(),
            &(mode.schema_version, compute_checksum(text))
        );
    }
}

async fn install_audit_baselines(
    dir: &std::path::Path,
    modified: bool,
    baselined: bool,
) -> HashMap<String, String> {
    fs::create_dir_all(dir.join("modes")).await.unwrap();
    let mut checksums = HashMap::new();
    for &(name, version) in AUDIT_BASELINE_MODES {
        let baseline = audit_baseline(name, version);
        let key = format!("modes/{name}.yaml");
        let content = if modified {
            // Deliberately remove grants and deny a tool: upgrades must not undo intentional policy.
            let mut value: serde_yaml::Value = serde_yaml::from_str(&baseline).unwrap();
            value["tools"] = serde_yaml::from_str("[cat]").unwrap();
            value["tool_confirm"] =
                serde_yaml::from_str("rules: [{match: shell, action: deny}]").unwrap();
            format!(
                "# intentional local restriction\n{}",
                serde_yaml::to_string(&value).unwrap()
            )
        } else {
            baseline.to_owned()
        };
        fs::write(dir.join(&key), content).await.unwrap();
        checksums.insert(key, compute_checksum(&baseline));
    }
    if baselined {
        fs::write(
            dir.join("default-checksums.yaml"),
            serde_yaml::to_string(&checksums).unwrap(),
        )
        .await
        .unwrap();
    }
    checksums
}

async fn saved_baselines(dir: &std::path::Path) -> HashMap<String, String> {
    serde_yaml::from_str(
        &fs::read_to_string(dir.join("default-checksums.yaml"))
            .await
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn all_audit_modes_upgrade_unmodified_baselined_installs_and_remain_stable() {
    let temp = tempfile::tempdir().unwrap();
    install_audit_baselines(temp.path(), false, true).await;
    for _ in 0..2 {
        global_configs_try_create_all(temp.path()).await.unwrap();
        let saved = saved_baselines(temp.path()).await;
        for (file, embedded) in embedded_defaults("modes") {
            let key = format!("modes/{file}");
            assert_eq!(
                fs::read_to_string(temp.path().join(&key)).await.unwrap(),
                embedded,
                "{key}"
            );
            assert_eq!(saved[&key], compute_checksum(&embedded));
        }
    }
}

#[tokio::test]
async fn all_audit_modes_preserve_baselined_custom_restrictions_byte_for_byte() {
    let temp = tempfile::tempdir().unwrap();
    let old_checksums = install_audit_baselines(temp.path(), true, true).await;
    let mut before = HashMap::new();
    for key in old_checksums.keys() {
        before.insert(key.clone(), fs::read(temp.path().join(key)).await.unwrap());
    }
    for _ in 0..2 {
        global_configs_try_create_all(temp.path()).await.unwrap();
        let saved = saved_baselines(temp.path()).await;
        for (key, content) in &before {
            assert_eq!(
                &fs::read(temp.path().join(key)).await.unwrap(),
                content,
                "{key}"
            );
            assert_eq!(saved[key], old_checksums[key]);
        }
    }
}

#[tokio::test]
async fn unbaselined_older_unknown_modes_upgrade_with_exact_backups() {
    let temp = tempfile::tempdir().unwrap();
    install_audit_baselines(temp.path(), true, false).await;
    let mut originals = HashMap::new();
    for &(name, _) in AUDIT_BASELINE_MODES {
        originals.insert(
            format!("{name}.yaml"),
            fs::read(temp.path().join(format!("modes/{name}.yaml")))
                .await
                .unwrap(),
        );
    }
    global_configs_try_create_all(temp.path()).await.unwrap();
    let saved = saved_baselines(temp.path()).await;
    for (file, embedded) in embedded_defaults("modes") {
        let key = format!("modes/{file}");
        assert_eq!(
            fs::read_to_string(temp.path().join(&key)).await.unwrap(),
            embedded
        );
        assert_eq!(saved[&key], compute_checksum(&embedded));
        let backups: Vec<_> = std::fs::read_dir(temp.path().join("modes"))
            .unwrap()
            .map(Result::unwrap)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&format!("{file}.bak-"))
            })
            .collect();
        assert_eq!(
            backups.len(),
            1,
            "unknown older custom {file} needs a backup"
        );
        assert_eq!(fs::read(backups[0].path()).await.unwrap(), originals[&file]);
    }
}

#[tokio::test]
async fn prior_same_version_missing_sync_now_upgrades_with_exact_backup() {
    let temp = tempfile::tempdir().unwrap();
    let &(name, old_version) = AUDIT_BASELINE_MODES
        .iter()
        .find(|(name, _)| *name == "agent")
        .unwrap();
    let baseline = audit_baseline(name, old_version);
    let stale = baseline.clone();
    assert_eq!(
        serde_yaml::from_str::<SchemaVersion>(&stale)
            .unwrap()
            .schema_version,
        old_version
    );
    assert!(!stale.contains("worktree_sync"));
    fs::create_dir_all(temp.path().join("modes")).await.unwrap();
    let path = temp.path().join(format!("modes/{name}.yaml"));
    fs::write(&path, &stale).await.unwrap();
    global_configs_try_create_all(temp.path()).await.unwrap();
    let current = embedded_defaults("modes")
        .into_iter()
        .find(|(file, _)| file == "agent.yaml")
        .unwrap()
        .1;
    assert_eq!(fs::read_to_string(&path).await.unwrap(), current);
    let backup = std::fs::read_dir(temp.path().join("modes"))
        .unwrap()
        .map(Result::unwrap)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("agent.yaml.bak-")
        })
        .unwrap();
    assert_eq!(fs::read_to_string(backup.path()).await.unwrap(), stale);
}

#[test]
fn audited_mode_semantic_workflows_and_confirmation_boundaries() {
    use crate::customization_types::ModeConfig;
    let modes: HashMap<String, ModeConfig> = embedded_defaults("modes")
        .into_iter()
        .map(|(_, text)| {
            let mode: ModeConfig = serde_yaml::from_str(&text).unwrap();
            (mode.id.clone(), mode)
        })
        .collect();
    let has = |mode: &ModeConfig, tool: &str| mode.tools.iter().any(|name| name == tool);
    let exact_rule = |mode: &ModeConfig, tool: &str, action: &str| {
        mode.tool_confirm
            .rules
            .iter()
            .any(|rule| rule.match_pattern == tool && rule.action == action)
    };
    let mut spawning = 0;
    for mode in modes.values() {
        assert!(
            !mode.prompt.contains("ask_questions(questions='"),
            "{} JSON arrays must not be strings",
            mode.id
        );
        if has(mode, "subagent") {
            spawning += 1;
            for tool in [
                "agents_overview",
                "agent_message",
                "agent_list",
                "agent_status",
                "wait",
                "agent_result",
                "agent_cancel",
                "agent_lifecycle",
            ] {
                assert!(has(mode, tool), "{} missing {tool}", mode.id);
            }
            assert!(exact_rule(mode, "agent_lifecycle", "ask"));
        }
        if mode.id.starts_with("setup") && mode.id != "setup_knowledge" {
            for tool in ["apply_patch", "undo_textdoc"] {
                assert!(exact_rule(mode, tool, "ask"), "{} {tool}", mode.id);
            }
        }
    }
    assert_eq!(spawning, 15);
    let design = &modes["design"];
    for tool in [
        "worktree_create",
        "worktree_merge",
        "shell",
        "process_start",
        "process_list",
        "process_read",
        "process_kill",
        "process_write_stdin",
    ] {
        assert!(has(design, tool));
        assert!(exact_rule(design, tool, "ask"));
    }
    assert!(has(design, "ask_questions"));
    for tool in ["set_goal", "board_create", "schedule"] {
        assert!(!has(design, tool));
    }
    assert!(has(&modes["task_planner"], "restart_agent"));
    assert!(exact_rule(&modes["task_planner"], "restart_agent", "ask"));
    assert!(!has(&modes["task_planner"], "worktree_create"));
    assert!(has(&modes["configurator"], "ask_questions"));
    assert!(!has(&modes["configurator"], "shell"));
    assert!(exact_rule(&modes["configurator"], "ask_questions", "auto"));
    assert!(modes["learn"].ui.tags.iter().any(|tag| tag == "guided"));
    assert!(!has(&modes["learn"], "create_knowledge"));
    for id in [
        "ask",
        "brainstorming",
        "explore",
        "past_work",
        "plan",
        "setup_knowledge",
    ] {
        for tool in [
            "apply_patch",
            "create_textdoc",
            "shell",
            "process_start",
            "worktree_create",
            "worktree_merge",
        ] {
            assert!(!has(&modes[id], tool), "{id} must exclude {tool}");
        }
    }
    for id in ["setup", "setup_knowledge"] {
        assert!(!has(&modes[id], "subagent"));
    }
    for id in ["task_agent", "task_planner"] {
        let prompt = &modes[id].prompt;
        for unsafe_advice in [
            "git add -A",
            "rm -rf node_modules",
            "ship the production fix without the test",
            "cherry-pick just the production files",
        ] {
            assert!(!prompt.contains(unsafe_advice), "{id}: {unsafe_advice}");
        }
    }
    assert!(modes["task_agent"].prompt.contains("`goal.plan.content`"));
    assert!(modes["setup_subagents"].prompt.contains("agent_profile"));
    assert!(modes["setup_modes"]
        .prompt
        .contains("independently monotonic"));
}
