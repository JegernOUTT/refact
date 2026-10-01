use std::collections::HashMap;

use crate::coordination::peer_authority::PeerAuthority;
use crate::coordination::teams::{load_team, TeamRefusal};
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
async fn load_team_builds_the_declared_roles() {
    let gcx = gcx_with_team(Some(
        "roles:\n  - name: implementer\n    default_profile: coder\n    authority_ceiling: mail\n    peer_mail_ttl_secs: 120\n  - name: reviewer\n    authority_ceiling: metadata\n",
    ))
    .await;

    let team = load_team(gcx, "ship-it").await.unwrap();

    assert_eq!(team.name, "ship-it");
    assert_eq!(team.roles().len(), 2);
    let implementer = team.role("implementer").unwrap();
    assert_eq!(implementer.default_profile.as_deref(), Some("coder"));
    assert_eq!(implementer.authority_ceiling, Some(PeerAuthority::Mail));
    assert_eq!(implementer.peer_mail_ttl_secs, 120);
    let reviewer = team.role("reviewer").unwrap();
    assert_eq!(reviewer.authority_ceiling, Some(PeerAuthority::Metadata));
}

#[tokio::test]
async fn duplicate_role_is_refused() {
    let gcx = gcx_with_team(Some("roles:\n  - name: same\n  - name: same\n")).await;

    let error = load_team(gcx, "ship-it").await.unwrap_err();

    assert_eq!(
        error,
        TeamRefusal::DuplicateRole {
            role: "same".into()
        }
        .to_string()
    );
}

#[tokio::test]
async fn unknown_team_is_refused() {
    let gcx = gcx_with_team(None).await;

    let error = load_team(gcx, "missing").await.unwrap_err();

    assert!(error.contains("team `missing` is not defined"));
}

#[tokio::test]
async fn missing_or_malformed_team_declaration_is_refused() {
    let missing = gcx_with_team(None).await;
    let error = load_team(missing, "ship-it").await.unwrap_err();
    assert!(error.contains("has no `team` definition"));

    let malformed = gcx_with_team(Some("roles: not-a-list\n")).await;
    let error = load_team(malformed, "ship-it").await.unwrap_err();
    assert!(error.contains("team `ship-it` is malformed"));
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
