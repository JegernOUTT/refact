use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ContextEnum;
use crate::chat::types::ChatSession;
use crate::coordination::teams::load_team;
use crate::tools::tool_agent_interact::ToolAgentPeerGrant;
use crate::tools::tool_team::ToolTeamRoleGrant;
use crate::tools::tools_description::Tool;
use crate::yaml_configs::customization_registry::RegistryCache;
use crate::yaml_configs::customization_types::{
    ProjectRegistry, SubagentConfig, SubagentMessages, SubagentPrompts, SubchatConfig,
};

const UNREACHABLE: &str = "member or recipient is not reachable for this grant";
const ROOT_REFUSAL: &str = "Only a top-level root can change peer authority.";

fn team_config() -> SubagentConfig {
    let mut extra = HashMap::new();
    extra.insert(
        "team".to_string(),
        serde_yaml::from_str(
            "roles:\n  - name: implementer\n    authority_ceiling: mail\n    peer_mail_ttl_secs: 120\n",
        )
        .unwrap(),
    );
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

fn args(items: &[(&str, Value)]) -> HashMap<String, Value> {
    items
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect()
}

fn role_args(member_chat: &str, recipient_chat: &str) -> HashMap<String, Value> {
    args(&[
        ("team", json!("ship-it")),
        ("role", json!("implementer")),
        ("member_chat_id", json!(member_chat)),
        ("recipient_chat_id", json!(recipient_chat)),
        ("authority", json!("mail")),
    ])
}

async fn context(
    app: &AppState,
    chat_id: &str,
    root_chat_id: &str,
    background_agent_id: Option<&str>,
) -> Arc<AMutex<AtCommandsContext>> {
    let ccx = Arc::new(AMutex::new(
        AtCommandsContext::new_from_app(
            app.clone(),
            4096,
            20,
            false,
            vec![],
            chat_id.to_string(),
            Some(root_chat_id.to_string()),
            "test/default".to_string(),
            None,
            None,
        )
        .await,
    ));
    ccx.lock().await.background_agent_id = background_agent_id.map(str::to_string);
    ccx
}

async fn peer_grant_error(
    ccx: Arc<AMutex<AtCommandsContext>>,
) -> Result<(bool, Vec<ContextEnum>), String> {
    ToolAgentPeerGrant {
        config_path: String::new(),
    }
    .tool_execute(
        ccx,
        &"peer-grant".to_string(),
        &args(&[
            ("recipient_root", json!("recipient-root")),
            ("authority", json!("mail")),
        ]),
    )
    .await
}

async fn role_grant_error(
    ccx: Arc<AMutex<AtCommandsContext>>,
) -> Result<(bool, Vec<ContextEnum>), String> {
    ToolTeamRoleGrant {
        config_path: String::new(),
    }
    .tool_execute(
        ccx,
        &"role-grant".to_string(),
        &role_args("member-chat", "recipient-chat"),
    )
    .await
}

#[tokio::test]
async fn branch_chat_with_foreign_root_cannot_grant() {
    let (app, workspace, project, _) = privacy_fixture().await;
    let ccx = add_chat(
        &app,
        workspace.path(),
        "branch-chat",
        Some("foreign-root"),
        None,
    )
    .await;
    add_chat(
        &app,
        workspace.path(),
        "recipient-root",
        Some("recipient-root"),
        None,
    )
    .await;
    app.peer_grants
        .grant(
            crate::coordination::peer_authority::PeerGrant {
                project,
                sender_root: "branch-chat".into(),
                recipient_root: "recipient-root".into(),
                authority: crate::coordination::peer_authority::PeerAuthority::Control,
                granted_by: "user".into(),
                expires_at_ms: now_ms().saturating_add(60_000),
            },
            now_ms(),
        )
        .unwrap();

    assert_eq!(
        peer_grant_error(ccx.clone()).await.unwrap_err(),
        ROOT_REFUSAL
    );
    assert_eq!(role_grant_error(ccx).await.unwrap_err(), ROOT_REFUSAL);
}

#[tokio::test]
async fn task_agent_chat_cannot_grant() {
    let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
    let ccx = context(
        &app,
        "task-agent-chat",
        "planner-chat",
        Some("task-agent-id"),
    )
    .await;

    assert_eq!(
        peer_grant_error(ccx.clone()).await.unwrap_err(),
        ROOT_REFUSAL
    );
    assert_eq!(role_grant_error(ccx).await.unwrap_err(), ROOT_REFUSAL);
}

async fn privacy_fixture() -> (
    AppState,
    tempfile::TempDir,
    String,
    Arc<AMutex<AtCommandsContext>>,
) {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let workspace = tempfile::tempdir().unwrap();
    let project_path = workspace.path().canonicalize().unwrap();
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![project_path.clone()];
    let mut caps = crate::caps::CodeAssistantCaps::default();
    caps.chat_models.insert(
        "test/default".into(),
        Arc::new(crate::caps::ChatModelRecord {
            base: crate::caps::BaseModelRecord {
                id: "test/default".into(),
                name: "test/default".into(),
                n_ctx: 200_000,
                endpoint: "https://example.com/v1/chat/completions".into(),
                ..Default::default()
            },
            supports_tools: true,
            supports_agent: true,
            ..Default::default()
        }),
    );
    caps.defaults.chat_default_model = "test/default".into();
    {
        let mut state = gcx.caps_state.write().await;
        state.caps = Some(Arc::new(caps));
        state.last_attempted_ts = now_ms() / 1000;
    }
    gcx.project_registry_cache.write().unwrap().cache.insert(
        project_path.clone(),
        RegistryCache {
            project_root: project_path.clone(),
            registry: ProjectRegistry {
                subagents: HashMap::from([("ship-it".into(), team_config())]),
                ..Default::default()
            },
            last_scan: std::time::SystemTime::now(),
        },
    );
    let app = AppState::from_gcx(gcx).await;
    let lead = add_chat(&app, &project_path, "lead-chat", Some("lead-chat"), None).await;
    let project = crate::coordination::roots::project_identity(&project_path);
    (app, workspace, project, lead)
}

async fn add_chat(
    app: &AppState,
    project: &std::path::Path,
    chat_id: &str,
    root_chat_id: Option<&str>,
    privacy_path: Option<&std::path::Path>,
) -> Arc<AMutex<AtCommandsContext>> {
    let mut session = ChatSession::new(chat_id.to_string());
    session.thread.model = "test/default".into();
    session.thread.mode = "agent".into();
    session.thread.root_chat_id = root_chat_id.map(str::to_string);
    session.work_project_root = Some(project.to_path_buf());
    if let Some(path) = privacy_path {
        let mut message = crate::call_validation::ChatMessage::new("user".into(), "private".into());
        crate::privacy::records::merge_records(
            &mut message,
            [refact_privacy::FileRecord {
                path: path.to_string_lossy().into_owned(),
                zone: "private".into(),
                attribution: refact_privacy::Attribution::Declared,
            }],
        );
        session.add_message(message);
    }
    app.chat
        .sessions
        .write()
        .await
        .insert(chat_id.to_string(), Arc::new(AMutex::new(session)));
    context(app, chat_id, root_chat_id.unwrap_or(chat_id), None).await
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[tokio::test]
async fn privacy_cleared_row_is_unreachable_with_uniform_text() {
    use refact_privacy::{PrivacyPolicy, ShellBehavior, Zone};

    let (app, workspace, project, ccx) = privacy_fixture().await;
    let secret = workspace.path().join("private/secret.rs");
    std::fs::create_dir_all(secret.parent().unwrap()).unwrap();
    std::fs::write(&secret, "private source").unwrap();
    app.gcx.privacy_policy_load.write().unwrap().policy = Arc::new(PrivacyPolicy {
        zones: vec![Zone {
            name: "private".into(),
            patterns: vec![secret.to_string_lossy().into_owned()],
            send_to: vec!["another-provider".into()],
            on_shell_read: ShellBehavior::Withhold,
        }],
        ..Default::default()
    });

    let member = add_chat(
        &app,
        workspace.path(),
        "member-chat",
        Some("member-chat"),
        None,
    )
    .await;
    add_chat(
        &app,
        workspace.path(),
        "hidden-chat",
        Some("hidden-root"),
        Some(&secret),
    )
    .await;
    let spec = load_team(app.gcx.clone(), "ship-it").await.unwrap();
    app.teams.register(spec).unwrap();
    app.teams
        .with_roster(&project, "ship-it", |_, roster| {
            roster
                .claim("implementer", "member-chat", "seat", None, now_ms())
                .unwrap();
        })
        .unwrap();

    let hidden = ToolTeamRoleGrant {
        config_path: String::new(),
    }
    .tool_execute(
        ccx.clone(),
        &"hidden-role".to_string(),
        &role_args("member-chat", "hidden-chat"),
    )
    .await
    .unwrap_err();
    let missing = ToolTeamRoleGrant {
        config_path: String::new(),
    }
    .tool_execute(
        ccx,
        &"missing-role".to_string(),
        &role_args("member-chat", "missing-chat"),
    )
    .await
    .unwrap_err();
    assert_eq!(hidden, UNREACHABLE);
    assert_eq!(hidden, missing);

    let hidden_peer = ToolAgentPeerGrant {
        config_path: String::new(),
    }
    .tool_execute(
        member.clone(),
        &"hidden-peer".to_string(),
        &args(&[
            ("recipient_root", json!("hidden-root")),
            ("authority", json!("mail")),
        ]),
    )
    .await
    .unwrap_err();
    let missing_peer = ToolAgentPeerGrant {
        config_path: String::new(),
    }
    .tool_execute(
        member,
        &"missing-peer".to_string(),
        &args(&[
            ("recipient_root", json!("missing-root")),
            ("authority", json!("mail")),
        ]),
    )
    .await
    .unwrap_err();
    assert_eq!(hidden_peer, missing_peer);
}

#[tokio::test]
async fn refusal_never_echoes_resolved_roots() {
    use crate::coordination::peer_authority::{PeerAuthority, PeerGrant};

    let (app, workspace, project, ccx) = privacy_fixture().await;
    let branch = add_chat(
        &app,
        workspace.path(),
        "branch-secret-chat",
        Some("resolved-secret-root"),
        None,
    )
    .await;
    add_chat(
        &app,
        workspace.path(),
        "resolved-secret-root",
        Some("resolved-secret-root"),
        None,
    )
    .await;
    let branch_refusal = peer_grant_error(branch.clone()).await.unwrap_err();
    assert_eq!(branch_refusal, ROOT_REFUSAL);
    assert!(!branch_refusal.contains("resolved-secret-root"));
    assert!(!branch_refusal.contains("branch-secret-chat"));
    assert_eq!(role_grant_error(branch).await.unwrap_err(), ROOT_REFUSAL);

    add_chat(
        &app,
        workspace.path(),
        "member-chat",
        Some("resolved-member-root"),
        None,
    )
    .await;
    add_chat(
        &app,
        workspace.path(),
        "recipient-chat",
        Some("resolved-recipient-root"),
        None,
    )
    .await;
    let spec = load_team(app.gcx.clone(), "ship-it").await.unwrap();
    app.teams.register(spec).unwrap();
    app.teams
        .with_roster(&project, "ship-it", |_, roster| {
            roster
                .claim("implementer", "member-chat", "seat", None, now_ms())
                .unwrap();
        })
        .unwrap();

    let refused = role_grant_error(ccx.clone()).await.unwrap_err();
    assert_eq!(refused, UNREACHABLE);
    assert!(!refused.contains("lead-chat"));
    assert!(!refused.contains("resolved-member-root"));
    assert!(!refused.contains("resolved-recipient-root"));

    app.peer_grants
        .grant(
            PeerGrant {
                project,
                sender_root: "lead-chat".into(),
                recipient_root: "resolved-recipient-root".into(),
                authority: PeerAuthority::Control,
                granted_by: "user".into(),
                expires_at_ms: now_ms().saturating_add(60_000),
            },
            now_ms(),
        )
        .unwrap();
    let granted = ToolTeamRoleGrant {
        config_path: String::new(),
    }
    .tool_execute(
        ccx,
        &"role-grant".to_string(),
        &role_args("member-chat", "recipient-chat"),
    )
    .await
    .unwrap();
    let ContextEnum::ChatMessage(message) = &granted.1[0] else {
        panic!("expected tool message")
    };
    let output = message.content.content_text_only();
    assert!(!output.contains("resolved-member-root"));
    assert!(!output.contains("resolved-recipient-root"));
    assert_eq!(
        message.extra["team_role_grant"]["member_chat_id"],
        "member-chat"
    );
    assert_eq!(
        message.extra["team_role_grant"]["recipient_chat_id"],
        "recipient-chat"
    );
}
