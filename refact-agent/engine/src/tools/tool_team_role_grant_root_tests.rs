use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::agents::types::{BgAgentKind, CreateAgentRequest};
use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ContextEnum;
use crate::chat::types::ChatSession;
use crate::coordination::peer_authority::{PeerAuthority, PeerGrant};
use crate::coordination::teams::load_team;
use crate::tools::tool_team::{root_for_address, ToolTeamRoleGrant};
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

async fn fixture() -> (
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
    let project = crate::coordination::roots::project_identity(&project_path);
    let assigner = add_chat(&app, &project_path, "lead-chat", None, Some("lead-chat")).await;
    let ccx = Arc::new(AMutex::new(
        AtCommandsContext::new_from_app(
            app.clone(),
            4096,
            20,
            false,
            vec![],
            "lead-chat".to_string(),
            Some("lead-chat".to_string()),
            "test/default".to_string(),
            None,
            None,
        )
        .await,
    ));
    drop(assigner);
    (app, workspace, project, ccx)
}

async fn add_chat(
    app: &AppState,
    project: &std::path::Path,
    chat_id: &str,
    parent_id: Option<&str>,
    root_chat_id: Option<&str>,
) -> Arc<AMutex<AtCommandsContext>> {
    let mut session = ChatSession::new(chat_id.to_string());
    session.thread.model = "test/default".to_string();
    session.thread.mode = "agent".to_string();
    session.thread.parent_id = parent_id.map(str::to_string);
    session.thread.root_chat_id = root_chat_id.map(str::to_string);
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
            root_chat_id.map(str::to_string),
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

fn grant_args(member_chat_id: &str, recipient_chat_id: &str) -> HashMap<String, Value> {
    HashMap::from([
        ("team".into(), json!("ship-it")),
        ("role".into(), json!("implementer")),
        ("member_chat_id".into(), json!(member_chat_id)),
        ("recipient_chat_id".into(), json!(recipient_chat_id)),
        ("authority".into(), json!("mail")),
    ])
}

fn peer_grant(project: &str, recipient_root: &str) -> PeerGrant {
    PeerGrant {
        project: project.into(),
        sender_root: "lead-chat".into(),
        recipient_root: recipient_root.into(),
        authority: PeerAuthority::Control,
        granted_by: "user".into(),
        expires_at_ms: now_ms().saturating_add(60_000),
    }
}

async fn register_team_and_seat(
    app: &AppState,
    project: &str,
    member_chat: &str,
    claimed_at_ms: u64,
) {
    let spec = load_team(app.gcx.clone(), "ship-it").await.unwrap();
    app.teams.register(spec).unwrap();
    app.teams
        .with_roster(project, "ship-it", |_, roster| {
            roster
                .claim(
                    "implementer",
                    member_chat,
                    "member-seat",
                    None,
                    claimed_at_ms,
                )
                .unwrap();
        })
        .unwrap();
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
async fn trajectory_row_without_root_uses_its_own_chat_not_its_parent() {
    let (app, workspace, project, ccx) = fixture().await;
    add_chat(
        &app,
        workspace.path(),
        "member-chat",
        Some("member-parent"),
        None,
    )
    .await;
    add_chat(&app, workspace.path(), "recipient-chat", None, None).await;
    let catalog =
        crate::coordination::catalog::build_catalog(&app, Some(workspace.path().to_str().unwrap()))
            .await
            .unwrap();
    let (member_root, _) =
        root_for_address(&catalog.all_rows(), None, Some("member-chat"), "member").unwrap();
    assert_eq!(member_root, "member-chat");
    register_team_and_seat(&app, &project, "member-chat", now_ms()).await;
    app.peer_grants
        .grant(peer_grant(&project, "recipient-chat"), now_ms())
        .unwrap();

    let result = execute(ccx, &grant_args("member-chat", "recipient-chat"))
        .await
        .unwrap();

    assert!(app.peer_grants.allows(
        &project,
        "member-chat",
        "recipient-chat",
        PeerAuthority::Mail,
        now_ms()
    ));
    assert!(matches!(
        &result.1[0],
        ContextEnum::ChatMessage(message)
            if message.extra["team_role_grant"]["member_chat_id"] == "member-chat"
                && message.extra["team_role_grant"]["recipient_chat_id"] == "recipient-chat"
    ));
}

#[tokio::test]
async fn background_agent_row_uses_its_parent_root() {
    let (app, workspace, _project, _ccx) = fixture().await;
    add_chat(
        &app,
        workspace.path(),
        "agent-child-chat",
        Some("agent-parent-chat"),
        None,
    )
    .await;
    let (agent, _, _) = app
        .agents
        .create(CreateAgentRequest {
            parent_chat_id: "agent-parent-chat".into(),
            parent_root_chat_id: Some("agent-parent-root".into()),
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
    app.agents
        .mark_running(&agent.agent_id, "agent-child-chat".into())
        .await
        .unwrap();

    let catalog =
        crate::coordination::catalog::build_catalog(&app, Some(workspace.path().to_str().unwrap()))
            .await
            .unwrap();
    let (root, chat) =
        root_for_address(&catalog.all_rows(), Some(&agent.agent_id), None, "member").unwrap();

    assert_eq!(root, "agent-parent-root");
    assert_eq!(chat.as_deref(), Some("agent-child-chat"));
}

#[tokio::test]
async fn stale_seat_is_refused() {
    let (app, workspace, project, ccx) = fixture().await;
    add_chat(
        &app,
        workspace.path(),
        "member-chat",
        None,
        Some("member-chat"),
    )
    .await;
    add_chat(
        &app,
        workspace.path(),
        "recipient-chat",
        None,
        Some("recipient-chat"),
    )
    .await;
    let ttl_ms = refact_work_core::coordination_settings().team_heartbeat_ttl_ms;
    let now = now_ms();
    register_team_and_seat(
        &app,
        &project,
        "member-chat",
        now.saturating_sub(ttl_ms.saturating_add(1)),
    )
    .await;
    app.peer_grants
        .grant(peer_grant(&project, "recipient-chat"), now)
        .unwrap();

    let error = execute(ccx, &grant_args("member-chat", "recipient-chat"))
        .await
        .unwrap_err();

    assert_eq!(error, "member does not hold implementer in ship-it");
    assert!(!app.peer_grants.allows(
        &project,
        "member-chat",
        "recipient-chat",
        PeerAuthority::Mail,
        now_ms()
    ));
    let live_ids = BTreeSet::from(["member-chat".to_string()]);
    app.teams
        .with_roster(&project, "ship-it", |_, roster| {
            assert!(!roster.has_live_seat("implementer", "member-chat", &live_ids, now));
        })
        .unwrap();
}
