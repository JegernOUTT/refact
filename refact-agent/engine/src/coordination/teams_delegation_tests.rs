use std::collections::HashMap;

use crate::coordination::peer_authority::{PeerAuthority, PeerGrant, PeerGrantRegistry};
use crate::coordination::teams::{load_team, propose_role_grant, RoleAssignment, RoleRefusal, RoleSpec};
use crate::global_context::GlobalContext;
use crate::yaml_configs::customization_registry::RegistryCache;
use crate::yaml_configs::customization_types::{
    ProjectRegistry, SubagentConfig, SubagentMessages, SubagentPrompts, SubchatConfig,
};

const PROJECT: &str = "delegation-project";
const NOW_MS: u64 = 1_000;

fn assignment<'a>(assigner_root: &'a str, member_root: &'a str) -> RoleAssignment<'a> {
    RoleAssignment {
        team: "team",
        project: PROJECT,
        assigner_root,
        member_root,
        recipient_root: "recipient",
    }
}

fn assigner_grant(expires_at_ms: u64) -> PeerGrant {
    PeerGrant {
        project: PROJECT.to_string(),
        sender_root: "assigner".to_string(),
        recipient_root: "recipient".to_string(),
        authority: PeerAuthority::Mail,
        granted_by: "user-confirmed".to_string(),
        expires_at_ms,
    }
}

#[test]
fn derived_grant_never_outlives_the_assigners_grant() {
    let registry = PeerGrantRegistry::default();
    registry
        .grant(assigner_grant(NOW_MS + 60_000), NOW_MS)
        .unwrap();
    let role = RoleSpec::new("worker")
        .with_ceiling(PeerAuthority::Mail)
        .with_peer_mail_ttl_secs(3_600);

    let proposal = propose_role_grant(
        &registry,
        &role,
        &assignment("assigner", "member"),
        PeerAuthority::Mail,
        NOW_MS,
    )
    .unwrap();

    assert!(proposal.expires_at_ms() <= NOW_MS + 60_000);
}

#[test]
fn derived_grant_is_capped_by_the_max_peer_grant_ttl() {
    let registry = PeerGrantRegistry::default();
    registry.grant(assigner_grant(u64::MAX), NOW_MS).unwrap();
    let max_ttl_secs = refact_work_core::coordination_settings().peer_grant_max_ttl_secs;
    let role = RoleSpec::new("worker")
        .with_ceiling(PeerAuthority::Mail)
        .with_peer_mail_ttl_secs(max_ttl_secs.saturating_add(1));

    let proposal = propose_role_grant(
        &registry,
        &role,
        &assignment("assigner", "member"),
        PeerAuthority::Mail,
        NOW_MS,
    )
    .unwrap();

    assert_eq!(
        proposal.expires_at_ms(),
        NOW_MS.saturating_add(max_ttl_secs.saturating_mul(1_000))
    );
}

#[test]
fn member_equal_to_assigner_is_refused_and_the_assigners_grant_is_untouched() {
    let registry = PeerGrantRegistry::default();
    let original = assigner_grant(NOW_MS + 60_000);
    registry.grant(original.clone(), NOW_MS).unwrap();
    let role = RoleSpec::new("worker")
        .with_ceiling(PeerAuthority::Mail)
        .with_peer_mail_ttl_secs(3_600);

    let refusal = propose_role_grant(
        &registry,
        &role,
        &assignment("assigner", "assigner"),
        PeerAuthority::Mail,
        NOW_MS,
    )
    .unwrap_err();

    assert_eq!(refusal, RoleRefusal::MemberIsAssigner);
    assert_eq!(
        refusal.to_string(),
        "a role grant cannot target the assigner's own root"
    );
    assert_eq!(
        registry.get(PROJECT, "assigner", "recipient", NOW_MS),
        Some(original)
    );
}

fn subagent(extra: HashMap<String, serde_yaml::Value>) -> SubagentConfig {
    SubagentConfig {
        schema_version: 1,
        id: "team".to_string(),
        title: "Team".to_string(),
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

async fn gcx_with_team(declaration: &str) -> std::sync::Arc<GlobalContext> {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let extra = HashMap::from([(
        "team".to_string(),
        serde_yaml::from_str(declaration).unwrap(),
    )]);
    let registry = ProjectRegistry {
        subagents: HashMap::from([("team".to_string(), subagent(extra))]),
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
async fn out_of_range_role_ttl_fails_team_load() {
    let max_ttl_secs = refact_work_core::coordination_settings().peer_grant_max_ttl_secs;
    let gcx = gcx_with_team(&format!(
        "roles:\n  - name: worker\n    peer_mail_ttl_secs: {}\n",
        max_ttl_secs.saturating_add(1)
    ))
    .await;

    let error = load_team(gcx, "team").await.unwrap_err();

    assert!(error.contains("team `team` role `worker`"));
    assert!(error.contains("peer_mail_ttl_secs"));
}
