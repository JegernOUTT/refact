use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::tasks::types::TeamMember;
use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ContextEnum};
use crate::chat::types::ChatSession;
use crate::tools::tool_team::ToolTeamSeat;
use crate::tools::tools_description::Tool;
use crate::yaml_configs::customization_registry::RegistryCache;
use crate::coordination::teams::load_team;
use crate::yaml_configs::customization_types::{
    ProjectRegistry, SubagentConfig, SubagentMessages, SubagentPrompts, SubchatConfig,
};

fn team_config() -> SubagentConfig {
    let mut extra = HashMap::new();
    extra.insert(
        "team".to_string(),
        serde_yaml::from_str("roles:\n  - name: implementer\n  - name: reviewer\n").unwrap(),
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

async fn install_test_caps(gcx: Arc<crate::global_context::GlobalContext>) {
    let model_id = "test/default";
    let mut caps = crate::caps::CodeAssistantCaps::default();
    caps.chat_models.insert(
        model_id.to_string(),
        Arc::new(crate::caps::ChatModelRecord {
            base: crate::caps::BaseModelRecord {
                id: model_id.to_string(),
                name: model_id.to_string(),
                n_ctx: 200_000,
                endpoint: "https://example.com/v1/chat/completions".to_string(),
                ..Default::default()
            },
            supports_tools: true,
            supports_agent: true,
            ..Default::default()
        }),
    );
    caps.defaults.chat_default_model = model_id.to_string();
    let mut caps_state = gcx.caps_state.write().await;
    caps_state.caps = Some(Arc::new(caps));
    caps_state.last_attempted_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
}

async fn fixture() -> (AppState, tempfile::TempDir, Arc<AMutex<AtCommandsContext>>) {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let workspace = tempfile::tempdir().unwrap();
    let project = workspace.path().canonicalize().unwrap();
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![project.clone()];
    install_test_caps(gcx.clone()).await;
    let registry = ProjectRegistry {
        subagents: HashMap::from([("ship-it".to_string(), team_config())]),
        ..Default::default()
    };
    gcx.project_registry_cache.write().unwrap().cache.insert(
        project.clone(),
        RegistryCache {
            project_root: project.clone(),
            registry,
            last_scan: std::time::SystemTime::now(),
        },
    );
    let app = AppState::from_gcx(gcx).await;
    let ccx = caller(&app, "chat-a", &project).await;
    (app, workspace, ccx)
}

async fn caller(
    app: &AppState,
    chat_id: &str,
    project: &std::path::Path,
) -> Arc<AMutex<AtCommandsContext>> {
    let mut session = ChatSession::new(chat_id.to_string());
    session.thread.model = "test/default".to_string();
    session.thread.mode = "agent".to_string();
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
            Some("root-chat".to_string()),
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

async fn execute(
    ccx: Arc<AMutex<AtCommandsContext>>,
    action: &str,
    extra: &[(&str, Value)],
) -> Result<(bool, Vec<ContextEnum>), String> {
    let mut request_args = vec![("team", json!("ship-it")), ("action", json!(action))];
    request_args.extend_from_slice(extra);
    ToolTeamSeat {
        config_path: String::new(),
    }
    .tool_execute(ccx, &"team-call".to_string(), &args(&request_args))
    .await
}

fn result_text(result: &(bool, Vec<ContextEnum>)) -> String {
    let ContextEnum::ChatMessage(message) = &result.1[0] else {
        panic!("expected tool message")
    };
    let ChatContent::SimpleText(text) = &message.content else {
        panic!("expected text content")
    };
    text.clone()
}

fn extra(result: &(bool, Vec<ContextEnum>)) -> Value {
    let ContextEnum::ChatMessage(message) = &result.1[0] else {
        panic!("expected tool message")
    };
    message.extra["team_seat"].clone()
}

#[tokio::test]
async fn team_seat_is_registered_and_allow_listed() {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(&gcx.config_dir)
        .await
        .unwrap();
    let registered = crate::tools::tools_list::get_available_tools(gcx.clone()).await;
    let team_tools = registered
        .iter()
        .filter(|tool| tool.tool_description().name == "team_seat")
        .collect::<Vec<_>>();
    assert_eq!(team_tools.len(), 1);
    assert!(matches!(
        team_tools[0].tool_description().source.source_type,
        crate::tools::tools_description::ToolSourceType::Builtin
    ));
    assert!(!team_tools[0].tool_description().allow_parallel);
    for mode in ["agent", "task_agent", "task_planner"] {
        let config =
            crate::yaml_configs::customization_registry::get_mode_config(gcx.clone(), mode, None)
                .await
                .unwrap();
        assert!(
            config.tools.iter().any(|name| name == "team_seat"),
            "{mode}"
        );
    }
}

#[tokio::test]
async fn claim_then_second_claim_from_another_chat_is_refused() {
    let (app, workspace, first) = fixture().await;
    let second = caller(&app, "chat-b", workspace.path()).await;
    let claimed = execute(
        first,
        "claim",
        &[
            ("role", json!("implementer")),
            ("idempotency_key", json!("one")),
        ],
    )
    .await
    .unwrap();
    assert!(result_text(&claimed).contains("claimed"));
    let error = execute(
        second,
        "claim",
        &[
            ("role", json!("implementer")),
            ("idempotency_key", json!("two")),
        ],
    )
    .await
    .unwrap_err();
    assert!(
        error.contains("role `implementer` is held by `chat-a`"),
        "{error}"
    );
}

#[tokio::test]
async fn release_then_claim_again() {
    let (app, workspace, ccx) = fixture().await;
    execute(
        ccx.clone(),
        "claim",
        &[
            ("role", json!("implementer")),
            ("idempotency_key", json!("first")),
        ],
    )
    .await
    .unwrap();
    execute(ccx.clone(), "release", &[("role", json!("implementer"))])
        .await
        .unwrap();
    let other = caller(&app, "chat-b", workspace.path()).await;
    let reclaimed = execute(
        other,
        "claim",
        &[
            ("role", json!("implementer")),
            ("idempotency_key", json!("second")),
        ],
    )
    .await
    .unwrap();
    assert!(result_text(&reclaimed).contains("claimed"));
    app.teams
        .with_roster(
            &crate::coordination::roots::project_identity(workspace.path()),
            "ship-it",
            |_, roster| {
                assert_eq!(
                    roster.seat("implementer").unwrap().idempotency_key,
                    "second"
                )
            },
        )
        .unwrap();
}

#[tokio::test]
async fn reclaim_takes_over_only_an_expired_seat() {
    let (app, workspace, ccx) = fixture().await;
    let project = crate::coordination::roots::project_identity(workspace.path());
    let spec = load_team(app.gcx.clone(), "ship-it").await.unwrap();
    let ttl_ms = refact_work_core::coordination_settings().team_heartbeat_ttl_ms;
    app.teams.register(spec).unwrap();
    app.teams
        .with_roster(&project, "ship-it", |_, roster| {
            roster
                .claim(
                    "implementer",
                    "chat-a",
                    "old-key",
                    None,
                    now_ms().saturating_sub(ttl_ms + 1),
                )
                .unwrap();
        })
        .unwrap();
    let other = caller(&app, "chat-b", workspace.path()).await;
    let reclaimed = execute(
        other,
        "reclaim",
        &[
            ("role", json!("implementer")),
            ("idempotency_key", json!("new-key")),
        ],
    )
    .await
    .unwrap();
    assert!(result_text(&reclaimed).contains("claimed"));
    let fresh_refusal = execute(
        ccx.clone(),
        "reclaim",
        &[
            ("role", json!("implementer")),
            ("idempotency_key", json!("third-key")),
        ],
    )
    .await
    .unwrap_err();
    assert!(
        fresh_refusal.contains("still holds `implementer`"),
        "{fresh_refusal}"
    );
    let unclaimed_refusal = execute(
        ccx,
        "reclaim",
        &[
            ("role", json!("reviewer")),
            ("idempotency_key", json!("reviewer-key")),
        ],
    )
    .await
    .unwrap_err();
    assert!(
        unclaimed_refusal.contains("role `reviewer` is unclaimed"),
        "{unclaimed_refusal}"
    );
}

#[tokio::test]
async fn record_outcome_is_logged() {
    let (app, workspace, ccx) = fixture().await;
    execute(
        ccx.clone(),
        "claim",
        &[
            ("role", json!("implementer")),
            ("idempotency_key", json!("one")),
        ],
    )
    .await
    .unwrap();
    execute(
        ccx,
        "record_outcome",
        &[
            ("role", json!("implementer")),
            ("outcome", json!("complete")),
        ],
    )
    .await
    .unwrap();
    app.teams
        .with_roster(
            &crate::coordination::roots::project_identity(workspace.path()),
            "ship-it",
            |_, roster| {
                assert_eq!(
                    roster.seat("implementer").unwrap().status.as_deref(),
                    Some("complete")
                );
                let entries = roster.mission_log().entries();
                assert!(entries.iter().any(|entry| matches!(
                    &entry.event,
                    crate::coordination::teams::MissionEvent::OutcomeRecorded { outcome, .. }
                        if outcome == "complete"
                )));
            },
        )
        .unwrap();
}

#[tokio::test]
async fn unknown_role_or_team_is_refused() {
    let (_app, _workspace, ccx) = fixture().await;
    let role_error = execute(ccx.clone(), "claim", &[("role", json!("unknown"))])
        .await
        .unwrap_err();
    assert!(role_error.contains("this team defines no role `unknown`"));

    let invalid_team = args(&[
        ("team", json!("missing")),
        ("action", json!("claim")),
        ("role", json!("implementer")),
    ]);
    let error = ToolTeamSeat {
        config_path: String::new(),
    }
    .tool_execute(ccx, &"missing-team".to_string(), &invalid_team)
    .await
    .unwrap_err();
    assert!(error.contains("team `missing` is not defined"));
}

#[tokio::test]
async fn member_identity_comes_from_the_caller() {
    let (app, workspace, ccx) = fixture().await;
    let schema = ToolTeamSeat {
        config_path: String::new(),
    }
    .tool_description()
    .input_schema;
    assert!(schema["properties"].get("member").is_none());

    let claimed = execute(
        ccx,
        "claim",
        &[
            ("role", json!("reviewer")),
            ("member", json!("spoofed-chat")),
            ("idempotency_key", json!("own-seat")),
        ],
    )
    .await
    .unwrap();
    let payload = extra(&claimed);
    assert_eq!(payload["member_chat_id"], "chat-a");
    assert_eq!(payload["members"][0]["agent_chat_id"], "chat-a");
    let _expected: Vec<TeamMember> = serde_json::from_value(payload["members"].clone()).unwrap();
    app.teams
        .with_roster(
            &crate::coordination::roots::project_identity(workspace.path()),
            "ship-it",
            |_, roster| assert_eq!(roster.seat("reviewer").unwrap().member_chat_id, "chat-a"),
        )
        .unwrap();
}
