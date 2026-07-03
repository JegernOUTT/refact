use std::path::{Path, PathBuf};

use crate::buddy::autonomous_workflows::{AUTONOMOUS_BUDDY_WORKFLOWS, ERROR_DETECTIVE_WORKFLOW_ID};
use crate::yaml_configs::customization_types::SubagentConfig;

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

fn load_workflow_yaml(id: &str) -> SubagentConfig {
    let path = subagent_yaml_path(id);
    let yaml = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    serde_yaml::from_str(&yaml)
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()))
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
fn workflow_model_types_follow_cost_policy() {
    let buddy_class = [
        "refact_error_detective",
        "refact_self_critic",
        "buddy_refactor_hunter",
        "buddy_dependency_radar",
    ];
    for id in workflow_ids() {
        let config = load_workflow_yaml(id);
        let model_type = config.subchat.model_type.as_deref().unwrap_or("default");
        if buddy_class.contains(&id) {
            assert_eq!(model_type, "buddy", "{id} must stay on the buddy model");
        } else {
            assert_eq!(
                model_type, "light",
                "{id} must run on the light model per the cost policy"
            );
        }
    }
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
