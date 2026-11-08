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
fn worktree_sync_is_available_to_lifecycle_modes_only() {
    let modes = embedded_defaults("modes");
    let parse = |name: &str| {
        let content = modes
            .iter()
            .find(|(filename, _)| filename == &format!("{name}.yaml"))
            .unwrap_or_else(|| panic!("missing mode {name}"));
        serde_yaml::from_str::<serde_yaml::Value>(&content.1).unwrap()
    };
    for (name, previous_version) in [
        ("agent", 30),
        ("solo", 2),
        ("task_agent", 29),
        ("task_planner", 33),
        ("design", 28),
    ] {
        let mode = parse(name);
        let tools = mode["tools"].as_sequence().expect("mode tools list");
        assert!(
            tools
                .iter()
                .any(|tool| tool.as_str() == Some("worktree_sync")),
            "{name} must expose worktree_sync"
        );
        assert!(
            mode["schema_version"].as_u64().unwrap() > previous_version,
            "{name} must upgrade existing installations"
        );
    }
    for name in ["ask", "brainstorming", "debug", "explore", "learn", "plan"] {
        let mode = parse(name);
        let tools = mode["tools"].as_sequence().expect("mode tools list");
        assert!(
            !tools
                .iter()
                .any(|tool| tool.as_str() == Some("worktree_sync")),
            "read-only mode {name} must not expose worktree_sync"
        );
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
