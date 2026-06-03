use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::buddy::autonomous_workflows::{AUTONOMOUS_BUDDY_WORKFLOWS, ERROR_DETECTIVE_WORKFLOW_ID};
use crate::yaml_configs::customization_types::SubagentConfig;

const CONDUCTOR_WORKFLOW_ID: &str = "buddy_conductor";
const BUDDY_CONDUCTOR_TOOL_SURFACE: &[&str] = &[
    "shell",
    "web",
    "web_search",
    "knowledge",
    "search_trajectories",
    "get_trajectory_context",
    "strategic_planning",
    "code_review",
    "swarm_investigate",
    "get_plan",
    "set_plan",
    "update_plan",
    "compress_chat_probe",
    "compress_chat_apply",
    "task_overview",
    "board_get",
    "ready_cards",
    "check_agents",
    "wait_agents",
    "spawn_agent",
    "spawn_agents_batch",
    "agent_pulse",
    "agent_chat_summary",
    "agent_diff",
    "agent_steer",
    "task_questions_list",
    "planner_reply",
    "card_comment_add",
    "card_comment_list",
    "task_broadcast",
    "pause_agent",
    "resume_agent",
    "cancel_agent",
    "merge_agent",
    "merge_ready_in_order",
    "restart_agent",
    "task_verify_card",
    "doc_list",
    "doc_get",
    "task_mem_search",
    "task_mem_save",
    "task_mem_inbox",
    "task_mem_triage_done",
    "conductor_steer_chat",
    "conductor_spawn_planner",
    "conductor_memo",
    "conductor_set_autonomy",
    "conductor_escalate",
    "conductor_goal_status",
    "conductor_say",
    "conductor_ask",
    "conductor_trajectory_list",
    "conductor_trajectory_digest",
    "conductor_trajectory_edit",
    "conductor_trajectory_remove",
    "conductor_trajectory_insert",
    "buddy_runtime_event",
];

const BUDDY_CONDUCTOR_DENIED_TOOLS: &[&str] = &[
    "cat",
    "regex_search",
    "search_pattern",
    "search_symbol_definition",
    "symbol_def",
    "search_semantic",
    "semantic_search",
    "tree",
    "Grep",
    "Glob",
    "create_textdoc",
    "update_textdoc",
    "update_textdoc_by_lines",
    "update_textdoc_regex",
    "update_textdoc_anchored",
    "replace_textdoc",
    "apply_patch",
    "patch",
    "text_edit",
    "undo_textdoc",
    "rm",
    "mv",
];

fn workflow_ids() -> Vec<&'static str> {
    AUTONOMOUS_BUDDY_WORKFLOWS
        .iter()
        .map(|workflow| workflow.id)
        .collect()
}

fn subagent_yaml_path(id: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates")
        .join("refact-yaml-configs")
        .join("src")
        .join("defaults")
        .join("subagents")
        .join(format!("{id}.yaml"))
}

fn read_workflow_yaml(id: &str) -> String {
    let path = subagent_yaml_path(id);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
}

fn load_workflow_yaml(id: &str) -> SubagentConfig {
    let path = subagent_yaml_path(id);
    let yaml = read_workflow_yaml(id);
    serde_yaml::from_str(&yaml)
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()))
}

fn assert_has_tools(config: &SubagentConfig, tools: &[&str]) {
    let configured = config.tools.iter().map(String::as_str).collect::<HashSet<_>>();
    for tool in tools {
        assert!(configured.contains(tool), "missing required tool {tool}");
    }
}

fn assert_lacks_tools(config: &SubagentConfig, tools: &[&str]) {
    let configured = config.tools.iter().map(String::as_str).collect::<HashSet<_>>();
    for tool in tools {
        assert!(!configured.contains(tool), "denied tool {tool} is configured");
    }
}

fn configured_tool_names(config: &SubagentConfig) -> Vec<&str> {
    config.tools.iter().map(String::as_str).collect()
}

#[tokio::test]
async fn every_workflow_yaml_loadable_via_get_delegate_config() {
    let gcx = crate::global_context::tests::make_test_gcx().await;

    for id in workflow_ids() {
        let config =
            crate::yaml_configs::customization_registry::get_subagent_config(gcx.clone(), id, None)
                .await;
        assert!(config.is_some(), "missing subagent config for {id}");
    }
}

#[tokio::test]
async fn buddy_conductor_yaml_loadable_via_get_delegate_config() {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let config = crate::yaml_configs::customization_registry::get_subagent_config(
        gcx,
        CONDUCTOR_WORKFLOW_ID,
        None,
    )
    .await;

    assert!(config.is_some(), "missing subagent config for {CONDUCTOR_WORKFLOW_ID}");
}

#[test]
fn every_workflow_yaml_includes_buddy_runtime_event_in_tools() {
    for id in workflow_ids() {
        let config = load_workflow_yaml(id);
        assert!(
            config.tools.iter().any(|tool| tool == "buddy_runtime_event"),
            "{id} does not include buddy_runtime_event"
        );
    }
}

#[test]
fn every_workflow_yaml_sets_autonomous_no_confirm_true() {
    for id in workflow_ids() {
        let config = load_workflow_yaml(id);
        assert_eq!(
            config.subchat.autonomous_no_confirm,
            Some(true),
            "{id} does not set autonomous_no_confirm"
        );
    }
}

#[test]
fn buddy_conductor_yaml_loads_and_uses_buddy_schema() {
    let config = load_workflow_yaml(CONDUCTOR_WORKFLOW_ID);

    assert_eq!(config.id, CONDUCTOR_WORKFLOW_ID);
    assert_eq!(config.subchat.model_type.as_deref(), Some("buddy"));
    assert_eq!(config.subchat.autonomous_no_confirm, Some(true));
    assert_eq!(config.extra.get("mode").and_then(|value| value.as_str()), Some("buddy"));
    let prompt = config.messages.system_prompt.as_deref().unwrap_or_default();
    assert!(prompt.contains("pinned-plan goal"));
    assert!(prompt.contains("pure conductor"));
    assert!(prompt.contains("FullAuto"));
    assert!(prompt.contains("free shell"));
    assert!(prompt.contains("Human wins"));
    assert!(prompt.contains("Ghost messaging"));
    assert!(prompt.contains("Prefer steering over trajectory surgery"));
    assert!(prompt.contains("paused-only"));
    assert!(prompt.contains("Read summaries"));
    assert!(prompt.contains("Escalate"));
}

#[test]
fn buddy_conductor_tool_surface_includes_required_coordination_tools() {
    let config = load_workflow_yaml(CONDUCTOR_WORKFLOW_ID);

    assert_has_tools(&config, BUDDY_CONDUCTOR_TOOL_SURFACE);
}

#[test]
fn buddy_conductor_tool_surface_matches_locked_v2_decision() {
    let config = load_workflow_yaml(CONDUCTOR_WORKFLOW_ID);

    assert_eq!(
        configured_tool_names(&config),
        BUDDY_CONDUCTOR_TOOL_SURFACE,
        "buddy_conductor tool surface drifted from the locked v2 decision"
    );
}

#[tokio::test]
async fn buddy_conductor_tool_surface_uses_registered_tool_names() {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(
        &gcx.config_dir,
    )
    .await
    .unwrap();

    let registered = crate::tools::tools_list::get_available_tools(gcx)
        .await
        .into_iter()
        .map(|tool| tool.tool_description().name)
        .collect::<HashSet<_>>();
    let config = load_workflow_yaml(CONDUCTOR_WORKFLOW_ID);

    for tool in configured_tool_names(&config) {
        assert!(registered.contains(tool), "unregistered conductor tool {tool}");
    }
}

#[test]
fn buddy_conductor_tool_surface_denies_direct_read_and_edit_tools() {
    let config = load_workflow_yaml(CONDUCTOR_WORKFLOW_ID);

    assert_lacks_tools(&config, BUDDY_CONDUCTOR_DENIED_TOOLS);
}

#[test]
fn legacy_artifacts_module_no_longer_exists() {
    assert!(!std::path::Path::new("src/buddy/artifacts.rs").exists());
}

#[test]
fn error_detective_renamed_to_refact_error_detective() {
    assert_eq!(ERROR_DETECTIVE_WORKFLOW_ID, "refact_error_detective");
    assert!(!workflow_ids().contains(&"buddy_error_detective"));
    assert!(workflow_ids().contains(&"refact_error_detective"));
}
