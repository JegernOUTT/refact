use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::agents::types::{BgAgentKind, CreateAgentRequest};
use crate::app_state::{AppState, AppToolRegistry};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatToolCall, ChatToolFunction, ContextEnum};
use crate::chat::types::{ChatSession, ThreadParams};
use crate::coordination::peer_authority::{PeerAuthority, PeerGrant};
use crate::coordination::teams::load_team;
use crate::tools::tool_team::ToolTeamRoleGrant;
use crate::tools::tools_description::Tool;
use crate::yaml_configs::customization_registry::RegistryCache;
use crate::yaml_configs::customization_types::{
    ProjectRegistry, SubagentConfig, SubagentMessages, SubagentPrompts, SubchatConfig,
};

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

async fn fixture() -> (AppState, tempfile::TempDir, Arc<AMutex<AtCommandsContext>>) {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let workspace = tempfile::tempdir().unwrap();
    let project = dunce::canonicalize(workspace.path()).unwrap();
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![project.clone()];
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
        project.clone(),
        RegistryCache {
            project_root: project.clone(),
            registry: ProjectRegistry {
                subagents: HashMap::from([("ship-it".into(), team_config())]),
                ..Default::default()
            },
            last_scan: std::time::SystemTime::now(),
        },
    );
    let app = AppState::from_gcx(gcx).await;
    let assigner = caller(&app, "lead-chat", &project).await;
    (app, workspace, assigner)
}

async fn caller(
    app: &AppState,
    chat_id: &str,
    project: &std::path::Path,
) -> Arc<AMutex<AtCommandsContext>> {
    let mut session = ChatSession::new(chat_id.to_string());
    session.thread.model = "test/default".to_string();
    session.thread.mode = "agent".to_string();
    session.thread.root_chat_id = Some(chat_id.to_string());
    session.work_project_root = Some(project.to_path_buf());
    app.chat
        .sessions
        .write()
        .await
        .insert(chat_id.to_string(), Arc::new(AMutex::new(session)));
    Arc::new(AMutex::new(
        AtCommandsContext::new_from_app(
            app.clone(),
            4096,
            20,
            false,
            vec![],
            chat_id.to_string(),
            Some(chat_id.to_string()),
            "test/default".to_string(),
            None,
            None,
        )
        .await,
    ))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn args(items: &[(&str, Value)]) -> HashMap<String, Value> {
    items
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect()
}

fn grant_args(member_chat_id: &str, recipient_chat_id: &str) -> HashMap<String, Value> {
    args(&[
        ("team", json!("ship-it")),
        ("role", json!("implementer")),
        ("member_chat_id", json!(member_chat_id)),
        ("recipient_chat_id", json!(recipient_chat_id)),
        ("authority", json!("mail")),
    ])
}

async fn register_and_seat(app: &AppState, project: &str, member_chat_id: &str) {
    let spec = load_team(app.gcx.clone(), "ship-it").await.unwrap();
    app.teams.register(spec).unwrap();
    app.teams
        .with_roster(project, "ship-it", |_, roster| {
            roster
                .claim("implementer", member_chat_id, "member-seat", None, now_ms())
                .unwrap();
        })
        .unwrap();
}

fn peer_grant(
    project: &str,
    sender_root: &str,
    recipient_root: &str,
    authority: PeerAuthority,
) -> PeerGrant {
    PeerGrant {
        project: project.into(),
        sender_root: sender_root.into(),
        recipient_root: recipient_root.into(),
        authority,
        granted_by: "user".into(),
        expires_at_ms: now_ms().saturating_add(60_000),
    }
}

async fn execute(
    ccx: Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<(bool, Vec<ContextEnum>), String> {
    ToolTeamRoleGrant {
        config_path: String::new(),
    }
    .tool_execute(ccx, &"role-grant-call".to_string(), args)
    .await
}

#[tokio::test]
async fn role_grant_confers_mail_but_never_more_than_the_assigner_holds() {
    let (app, workspace, ccx) = fixture().await;
    let project = crate::coordination::roots::project_identity(workspace.path());
    register_and_seat(&app, &project, "member-chat").await;
    app.peer_grants
        .grant(
            peer_grant(
                &project,
                "lead-chat",
                "recipient-chat",
                PeerAuthority::Control,
            ),
            now_ms(),
        )
        .unwrap();
    let (member_agent, _, _) = app
        .agents
        .create(CreateAgentRequest {
            parent_chat_id: "member-root-chat".into(),
            parent_root_chat_id: Some("member-root-chat".into()),
            parent_tool_call_id: None,
            kind: BgAgentKind::Subagent,
            config_name: "test-member".into(),
            title: "Team member".into(),
            prompt: "test member".into(),
            target_files: Vec::new(),
            model: "test/default".into(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
        })
        .await
        .unwrap();
    caller(&app, "member-root-chat", workspace.path()).await;
    app.agents
        .mark_running(&member_agent.agent_id, "member-agent-chat".into())
        .await
        .unwrap();
    caller(&app, "member-agent-chat", workspace.path()).await;
    caller(&app, "recipient-chat", workspace.path()).await;
    app.teams
        .with_roster(&project, "ship-it", |_, roster| {
            roster
                .release("implementer", "member-chat", now_ms())
                .unwrap();
            roster
                .claim(
                    "implementer",
                    "member-agent-chat",
                    "member-seat",
                    None,
                    now_ms(),
                )
                .unwrap();
        })
        .unwrap();

    execute(
        ccx,
        &args(&[
            ("team", json!("ship-it")),
            ("role", json!("implementer")),
            ("member_agent_id", json!(member_agent.agent_id)),
            ("recipient_chat_id", json!("recipient-chat")),
            ("authority", json!("mail")),
        ]),
    )
    .await
    .unwrap();

    let member_root =
        crate::coordination::catalog::build_catalog(&app, Some(workspace.path().to_str().unwrap()))
            .await
            .unwrap()
            .all_rows()
            .into_iter()
            .find(|row| row.key.id == member_agent.agent_id)
            .unwrap()
            .root_chat_id
            .unwrap();
    assert!(app.peer_grants.allows(
        &project,
        &member_root,
        "recipient-chat",
        PeerAuthority::Mail,
        now_ms()
    ));
    assert!(!app.peer_grants.allows(
        &project,
        &member_root,
        "recipient-chat",
        PeerAuthority::Control,
        now_ms()
    ));
}

#[tokio::test]
async fn assigner_without_authority_is_refused() {
    let (app, workspace, ccx) = fixture().await;
    let project = crate::coordination::roots::project_identity(workspace.path());
    register_and_seat(&app, &project, "member-chat").await;
    caller(&app, "member-chat", workspace.path()).await;
    caller(&app, "recipient-chat", workspace.path()).await;

    let error = execute(ccx, &grant_args("member-chat", "recipient-chat"))
        .await
        .unwrap_err();

    assert_eq!(error, "member or recipient is not reachable for this grant");
}

#[tokio::test]
async fn member_without_seat_is_refused() {
    let (app, workspace, ccx) = fixture().await;
    let spec = load_team(app.gcx.clone(), "ship-it").await.unwrap();
    app.teams.register(spec).unwrap();
    caller(&app, "member-chat", workspace.path()).await;
    caller(&app, "recipient-chat", workspace.path()).await;

    let error = execute(ccx, &grant_args("member-chat", "recipient-chat"))
        .await
        .unwrap_err();

    assert!(
        error.contains("member does not hold implementer in ship-it"),
        "{error}"
    );
}

#[tokio::test]
async fn member_equal_to_recipient_is_refused() {
    let (app, workspace, ccx) = fixture().await;
    let project = crate::coordination::roots::project_identity(workspace.path());
    register_and_seat(&app, &project, "same-chat").await;
    app.peer_grants
        .grant(
            peer_grant(&project, "lead-chat", "same-chat", PeerAuthority::Control),
            now_ms(),
        )
        .unwrap();
    caller(&app, "same-chat", workspace.path()).await;

    let error = execute(ccx, &grant_args("same-chat", "same-chat"))
        .await
        .unwrap_err();

    assert!(error.contains("identities must be distinct"), "{error}");
}

#[tokio::test]
async fn recipient_agent_id_resolves_to_its_root() {
    let (app, workspace, ccx) = fixture().await;
    let project = crate::coordination::roots::project_identity(workspace.path());
    register_and_seat(&app, &project, "member-chat").await;
    app.peer_grants
        .grant(
            peer_grant(
                &project,
                "lead-chat",
                "recipient-root-chat",
                PeerAuthority::Control,
            ),
            now_ms(),
        )
        .unwrap();
    caller(&app, "member-chat", workspace.path()).await;
    let recipient_root_ccx = caller(&app, "recipient-root-chat", workspace.path()).await;
    let (recipient_agent, _, _) = app
        .agents
        .create(CreateAgentRequest {
            parent_chat_id: "recipient-root-chat".into(),
            parent_root_chat_id: Some("recipient-root-chat".into()),
            parent_tool_call_id: None,
            kind: BgAgentKind::Subagent,
            config_name: "test-recipient".into(),
            title: "Recipient".into(),
            prompt: "recipient".into(),
            target_files: Vec::new(),
            model: "test/default".into(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
        })
        .await
        .unwrap();
    app.agents
        .mark_running(&recipient_agent.agent_id, "recipient-agent-chat".into())
        .await
        .unwrap();
    caller(&app, "recipient-agent-chat", workspace.path()).await;
    drop(recipient_root_ccx);

    let result = execute(
        ccx,
        &args(&[
            ("team", json!("ship-it")),
            ("role", json!("implementer")),
            ("member_chat_id", json!("member-chat")),
            ("recipient_agent_id", json!(recipient_agent.agent_id)),
            ("authority", json!("mail")),
        ]),
    )
    .await
    .unwrap();
    assert!(app.peer_grants.allows(
        &project,
        "member-chat",
        "recipient-root-chat",
        PeerAuthority::Mail,
        now_ms()
    ));
    assert!(
        matches!(&result.1[0], ContextEnum::ChatMessage(message) if message.extra["team_role_grant"]["recipient_agent_id"] == recipient_agent.agent_id)
    );
}

#[tokio::test]
async fn role_grant_requires_confirmation() {
    let (app, _workspace, ccx) = fixture().await;
    crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(
        &app.gcx.config_dir,
    )
    .await
    .unwrap();
    let chat_id = ccx.lock().await.chat_id.clone();
    let thread = ThreadParams {
        id: chat_id,
        model: "test/default".into(),
        auto_approve_editing_tools: false,
        auto_approve_dangerous_commands: false,
        autonomous_no_confirm: false,
        ..Default::default()
    };
    let call = ChatToolCall {
        id: "confirm-role-grant".into(),
        index: None,
        function: ChatToolFunction {
            name: "team_role_grant".into(),
            arguments: json!({}).to_string(),
        },
        tool_type: "function".into(),
        extra_content: None,
        started_at_ms: None,
        completed_at_ms: None,
    };

    let gcx = app.gcx.clone();
    let app = AppState {
        tool_registry: Arc::new(AppToolRegistry::with_fixture_tool_factory(
            gcx.clone(),
            Arc::new(|| {
                vec![Box::new(ToolTeamRoleGrant {
                    config_path: String::new(),
                })]
            }),
        )),
        ..app
    };
    let desc = ToolTeamRoleGrant {
        config_path: String::new(),
    }
    .tool_description();
    let catalog = Arc::new(refact_runtime_api::ToolCatalogSnapshot {
        index: refact_runtime_api::ToolRegistryIndex {
            tools: vec![desc],
            mcp_lazy_mode: false,
            mcp_total_count: 0,
            mcp_tool_index: vec![],
        },
        policy: vec![],
        aliases: refact_tool_api::ToolAliasRegistry::new(),
    });
    let (confirmations, denials) = crate::chat::tools::check_tools_confirmation(
        app,
        &[call],
        &[],
        &thread,
        Some(&catalog),
        "agent",
        None,
        &[],
        "",
    )
    .await;

    assert!(denials.is_empty());
    assert_eq!(confirmations.len(), 1);
    assert_eq!(confirmations[0].tool_name, "team_role_grant");
    assert_eq!(confirmations[0].rule, "explicit_user_consent");
}
