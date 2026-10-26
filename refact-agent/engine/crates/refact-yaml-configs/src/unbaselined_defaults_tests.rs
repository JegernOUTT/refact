use super::*;

const KEY: &str = "modes/task_agent.yaml";

fn embedded_task_agent() -> String {
    get_defaults_for_kind("modes")
        .into_iter()
        .find(|(filename, _)| filename == "task_agent.yaml")
        .unwrap()
        .1
}

async fn install(config_dir: &Path, key: &str, content: &str) -> PathBuf {
    let path = config_dir.join(key);
    fs::create_dir_all(path.parent().unwrap()).await.unwrap();
    fs::write(&path, content).await.unwrap();
    path
}

fn backups(path: &Path) -> Vec<PathBuf> {
    let prefix = format!("{}.bak-", path.file_name().unwrap().to_str().unwrap());
    std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|entry| {
            entry
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(&prefix)
        })
        .collect()
}

#[tokio::test]
async fn unbaselined_older_default_is_upgraded_with_backup() {
    let temp = tempfile::tempdir().unwrap();
    let embedded = embedded_task_agent();
    let version = extract_schema_version(&embedded) - 1;
    let old = format!("schema_version: {version}\r\nid: task_agent\r\nprompt: Local café 🐾\r\n");
    assert!(!is_known_shipped_checksum(KEY, &compute_checksum(&old)));
    let path = install(temp.path(), KEY, &old).await;

    for _ in 0..2 {
        global_configs_try_create_all(temp.path()).await.unwrap();
        assert_eq!(
            fs::read(&path).await.unwrap(),
            embedded.as_bytes(),
            "unbaselined older default must be replaced with the embedded default"
        );
        let backups = backups(&path);
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(&backups[0]).await.unwrap(), old.as_bytes());
        let name = backups[0].file_name().unwrap().to_str().unwrap();
        let timestamp = name.strip_prefix("task_agent.yaml.bak-").unwrap();
        chrono::NaiveDateTime::parse_from_str(timestamp, "%Y%m%dT%H%M%SZ").unwrap();
        assert_eq!(
            load_checksums(&temp.path().join(CHECKSUM_FILE))
                .await
                .get(KEY),
            Some(&compute_checksum(&embedded))
        );
    }
}

#[tokio::test]
async fn unbaselined_known_shipped_checksum_upgrades_without_backup() {
    let temp = tempfile::tempdir().unwrap();
    let embedded = get_defaults_for_kind("modes")
        .into_iter()
        .find(|(filename, _)| filename == "ask.yaml")
        .unwrap()
        .1;
    let old = r###"schema_version: 14
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
"###.to_string();
    assert!(extract_schema_version(&old) < extract_schema_version(&embedded));
    assert!(is_known_shipped_checksum(
        "modes/ask.yaml",
        &compute_checksum(&old)
    ));
    let path = install(temp.path(), "modes/ask.yaml", &old).await;

    global_configs_try_create_all(temp.path()).await.unwrap();

    assert_eq!(fs::read(&path).await.unwrap(), embedded.as_bytes());
    assert!(backups(&path).is_empty());
    assert_eq!(
        load_checksums(&temp.path().join(CHECKSUM_FILE))
            .await
            .get("modes/ask.yaml"),
        Some(&compute_checksum(&embedded))
    );
}

#[tokio::test]
async fn baselined_user_modified_default_is_kept() {
    let temp = tempfile::tempdir().unwrap();
    let baseline = "schema_version: 1\nid: task_agent\nprompt: Original\n";
    let old = "schema_version: 1\nid: task_agent\nprompt: User edit\n";
    let path = install(temp.path(), KEY, old).await;
    let checksums = HashMap::from([(KEY.to_string(), compute_checksum(baseline))]);
    save_checksums(&temp.path().join(CHECKSUM_FILE), &checksums).await;

    for _ in 0..2 {
        global_configs_try_create_all(temp.path()).await.unwrap();
        assert_eq!(fs::read(&path).await.unwrap(), old.as_bytes());
        assert!(backups(&path).is_empty());
        assert_eq!(
            load_checksums(&temp.path().join(CHECKSUM_FILE))
                .await
                .get(KEY),
            checksums.get(KEY)
        );
    }
}

#[tokio::test]
async fn unbaselined_newer_or_equal_version_is_kept() {
    let embedded = embedded_task_agent();
    let version = extract_schema_version(&embedded);
    for old in [
        format!("schema_version: {version}\nid: task_agent\nprompt: User edit\n"),
        format!("schema_version: {}\nid: task_agent\n", version + 1),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = install(temp.path(), KEY, &old).await;
        global_configs_try_create_all(temp.path()).await.unwrap();
        assert_eq!(fs::read(&path).await.unwrap(), old.as_bytes());
        assert!(backups(&path).is_empty());
        assert!(!load_checksums(&temp.path().join(CHECKSUM_FILE))
            .await
            .contains_key(KEY));
    }
}

#[tokio::test]
async fn unparseable_default_is_never_touched() {
    for old in [
        "schema_version: 1\ntools: [",
        "id: task_agent\nprompt: No schema version\n",
        "schema_version: invalid\nid: task_agent\n",
        "",
        "  \n\t\n",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = install(temp.path(), KEY, old).await;
        global_configs_try_create_all(temp.path()).await.unwrap();
        assert_eq!(fs::read(&path).await.unwrap(), old.as_bytes());
        assert!(backups(&path).is_empty());
        assert!(!load_checksums(&temp.path().join(CHECKSUM_FILE))
            .await
            .contains_key(KEY));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn backup_failure_keeps_unbaselined_default_and_records_nothing() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let old = "schema_version: 1\nid: task_agent\nprompt: User edit\n";
    let path = install(temp.path(), KEY, old).await;
    let parent = path.parent().unwrap();
    let permissions = std::fs::metadata(parent).unwrap().permissions();
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o555)).unwrap();

    global_configs_try_create_all(temp.path()).await.unwrap();

    std::fs::set_permissions(parent, permissions).unwrap();
    assert_eq!(fs::read(&path).await.unwrap(), old.as_bytes());
    assert!(backups(&path).is_empty());
    assert!(!load_checksums(&temp.path().join(CHECKSUM_FILE))
        .await
        .contains_key(KEY));
}
