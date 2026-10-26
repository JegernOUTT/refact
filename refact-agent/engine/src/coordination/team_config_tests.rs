use std::collections::HashMap;

use crate::coordination::teams::load_team;
use crate::global_context::GlobalContext;
use crate::yaml_configs::customization_registry::RegistryCache;
use crate::yaml_configs::customization_types::{
    ProjectRegistry, SubagentConfig, SubagentMessages, SubagentPrompts, SubchatConfig,
};

fn subagent(extra: HashMap<String, serde_yaml::Value>) -> SubagentConfig {
    SubagentConfig {
        schema_version: 1,
        id: "ship-it".to_string(),
        title: "Ship it".to_string(),
        description: String::new(),
        specific: false,
        expose_as_tool: false,
        has_code: false,
        tool: None,
        subchat: SubchatConfig::default(),
        messages: SubagentMessages::default(),
        prompts: SubagentPrompts::default(),
        gather_files: Default::default(),
        tools: Vec::new(),
        base: None,
        match_models: None,
        extra,
    }
}

async fn gcx_with_team(declaration: Option<&str>) -> std::sync::Arc<GlobalContext> {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let mut extra = HashMap::new();
    if let Some(declaration) = declaration {
        extra.insert(
            "team".to_string(),
            serde_yaml::from_str(declaration).unwrap(),
        );
    }
    let registry = ProjectRegistry {
        subagents: HashMap::from([("ship-it".to_string(), subagent(extra))]),
        ..Default::default()
    };
    gcx.project_registry_cache.write().unwrap().cache.insert(
        gcx.config_dir.clone(),
        RegistryCache {
            project_root: gcx.config_dir.clone(),
            registry,
            last_scan: std::time::SystemTime::now(),
        },
    );
    gcx
}

#[tokio::test]
async fn bad_ceiling_value_is_refused() {
    let gcx = gcx_with_team(Some(
        "roles:\n  - name: implementer\n    authority_ceiling: admin\n",
    ))
    .await;

    let error = load_team(gcx, "ship-it").await.unwrap_err();

    assert!(error.contains("unknown authority ceiling `admin`"));
}
