use crate::buddy::autonomous_workflows::{AUTONOMOUS_BUDDY_WORKFLOWS, ERROR_DETECTIVE_WORKFLOW_ID};
use crate::yaml_configs::customization_types::SubagentConfig;

fn workflow_ids() -> Vec<&'static str> {
    AUTONOMOUS_BUDDY_WORKFLOWS
        .iter()
        .map(|workflow| workflow.id)
        .collect()
}

fn load_workflow_yaml(id: &str) -> SubagentConfig {
    let path = format!("src/yaml_configs/defaults/subagents/{id}.yaml");
    let yaml =
        std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("failed to read {path}: {err}"));
    serde_yaml::from_str(&yaml).unwrap_or_else(|err| panic!("failed to parse {path}: {err}"))
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
fn every_workflow_yaml_includes_buddy_log_activity_in_tools() {
    for id in workflow_ids() {
        let config = load_workflow_yaml(id);
        assert!(
            config.tools.iter().any(|tool| tool == "buddy_log_activity"),
            "{id} does not include buddy_log_activity"
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
fn legacy_artifacts_module_no_longer_exists() {
    assert!(!std::path::Path::new("src/buddy/artifacts.rs").exists());
}

#[test]
fn error_detective_renamed_to_refact_error_detective() {
    assert_eq!(ERROR_DETECTIVE_WORKFLOW_ID, "refact_error_detective");
    assert!(!workflow_ids().contains(&"buddy_error_detective"));
    assert!(workflow_ids().contains(&"refact_error_detective"));
}
