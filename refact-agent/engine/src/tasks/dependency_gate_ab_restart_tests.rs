use std::sync::Arc;

use serde_json::{json, Value};

use crate::tasks::storage;
use crate::tasks::types::BoardCard;
use crate::tools::tool_spawn_ab::tests::{setup_repo_task, RecordingFacade};
use crate::tools::tools_description::Tool;

fn cards(dependency_column: &str, dependent_column: &str) -> Vec<BoardCard> {
    let mut dependency = crate::tools::tool_spawn_ab::tests::test_card();
    dependency.id = "A".to_string();
    dependency.title = "Dependency A".to_string();
    dependency.column = dependency_column.to_string();

    let mut dependent = crate::tools::tool_spawn_ab::tests::test_card();
    dependent.id = "B".to_string();
    dependent.title = "Dependent B".to_string();
    dependent.column = dependent_column.to_string();
    dependent.depends_on = vec!["A".to_string()];
    if dependent_column == "failed" {
        dependent.final_report = Some("FAILED: prior attempt".to_string());
        dependent.completed_at = Some(chrono::Utc::now().to_rfc3339());
    }
    vec![dependency, dependent]
}

async fn set_cards(gcx: Arc<crate::global_context::GlobalContext>, cards: Vec<BoardCard>) {
    storage::update_board_atomic(gcx, "task-1", move |board| {
        board.cards = cards;
        Ok(())
    })
    .await
    .unwrap();
}

async fn set_caps(gcx: &Arc<crate::global_context::GlobalContext>) {
    let mut caps = crate::caps::CodeAssistantCaps::default();
    caps.defaults.task_planner_agent_model = "provider/test-model".to_string();
    caps.chat_models.insert(
        "provider/test-model".to_string(),
        Arc::new(crate::caps::ChatModelRecord::default()),
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut state = gcx.caps_state.write().await;
    state.caps = Some(Arc::new(caps));
    state.last_attempted_ts = now;
}

async fn planner_context(
    app: crate::app_state::AppState,
) -> Arc<tokio::sync::Mutex<crate::at_commands::at_commands::AtCommandsContext>> {
    Arc::new(tokio::sync::Mutex::new(
        crate::at_commands::at_commands::AtCommandsContext::new_from_app(
            app,
            4096,
            20,
            false,
            vec![],
            "planner-chat".to_string(),
            Some("planner-chat".to_string()),
            "provider/test-model".to_string(),
            Some(refact_chat_api::TaskMeta {
                task_id: "task-1".to_string(),
                role: "planner".to_string(),
                agent_id: None,
                card_id: None,
                planner_chat_id: Some("planner-chat".to_string()),
            }),
            None,
        )
        .await,
    ))
}

async fn hold_capacity(
    app: &mut crate::app_state::AppState,
    source: &std::path::Path,
) -> refact_work_core::Permit {
    app.work_scheduler = Arc::new(refact_work_core::AdmissionScheduler::new(
        refact_work_core::CoordinationSettings {
            max_active_engine: 1,
            max_active_project: 1,
            max_active_root: 1,
            ..Default::default()
        },
    ));
    let project = crate::coordination::roots::project_identity(source);
    app.work_scheduler
        .reserve(refact_work_core::AdmissionRequest::new(
            refact_work_core::WorkKey::background_agent("blocking-parent").run(1),
            crate::tasks::admission::board_scope(&project, "planner-chat"),
        ))
        .unwrap()
        .acquire()
        .await
        .unwrap()
}

fn spawn_args() -> std::collections::HashMap<String, Value> {
    std::collections::HashMap::from([
        ("card_id".to_string(), json!("B")),
        (
            "variant_a".to_string(),
            json!({"model":"provider/test-model","suggested_steps":1}),
        ),
        (
            "variant_b".to_string(),
            json!({"model":"provider/test-model","suggested_steps":1}),
        ),
    ])
}

#[tokio::test]
async fn spawn_ab_refuses_card_with_unmet_dependency_before_side_effects() {
    let (gcx, _temp, _source) = setup_repo_task().await;
    set_cards(gcx.clone(), cards("planned", "planned")).await;
    set_caps(&gcx).await;
    let mut app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    let facade = Arc::new(RecordingFacade::default());
    app.chat.facade = facade.clone();
    let ccx = planner_context(app.clone()).await;

    let error = crate::tools::tool_spawn_ab::ToolSpawnAb::new()
        .tool_execute(ccx, &"spawn-ab".to_string(), &spawn_args())
        .await
        .unwrap_err();

    assert!(error.contains("A"), "{error}");
    let board = storage::load_board(gcx.clone(), "task-1").await.unwrap();
    let blocked = board.get_card("B").unwrap();
    assert_eq!(blocked.column, "planned");
    assert!(blocked.ab_variants.is_none());
    assert!(facade.created.lock().unwrap().is_empty());
    assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
    assert!(!gcx.cache_dir.join("worktrees").exists());
    assert!(!gcx
        .chat_sessions
        .read()
        .await
        .keys()
        .any(|chat_id| chat_id.starts_with("agent-B-")));
}

#[tokio::test]
async fn spawn_ab_proceeds_after_override() {
    let (gcx, _temp, source) = setup_repo_task().await;
    set_cards(gcx.clone(), cards("planned", "planned")).await;
    storage::update_board_atomic(gcx.clone(), "task-1", |board| {
        board.owner_chat_id = Some("planner-chat".to_string());
        Ok(())
    })
    .await
    .unwrap();
    storage::override_card_dependencies(
        gcx.clone(),
        "task-1",
        "B",
        "planner-chat",
        "dependency is satisfied externally",
    )
    .await
    .unwrap();
    set_caps(&gcx).await;
    let mut app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    let facade = Arc::new(RecordingFacade::default());
    app.chat.facade = facade.clone();
    let holder = hold_capacity(&mut app, &source).await;
    let ccx = planner_context(app.clone()).await;

    crate::tools::tool_spawn_ab::ToolSpawnAb::new()
        .tool_execute(ccx, &"spawn-ab".to_string(), &spawn_args())
        .await
        .unwrap();

    let board = storage::load_board(gcx, "task-1").await.unwrap();
    let claimed = board.get_card("B").unwrap();
    assert_eq!(claimed.column, "doing");
    assert!(claimed.ab_variants.is_some());
    assert!(facade.created.lock().unwrap().is_empty());
    crate::tasks::admission::shutdown(&app).await;
    holder.release();
}

async fn restart_with_dependency(
    dependency_column: &str,
) -> (
    Arc<crate::global_context::GlobalContext>,
    tempfile::TempDir,
    std::path::PathBuf,
    crate::app_state::AppState,
    Arc<RecordingFacade>,
    Arc<tokio::sync::Mutex<crate::at_commands::at_commands::AtCommandsContext>>,
) {
    let (gcx, temp, source) = setup_repo_task().await;
    set_cards(gcx.clone(), cards(dependency_column, "failed")).await;
    set_caps(&gcx).await;
    let mut app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    let facade = Arc::new(RecordingFacade::default());
    app.chat.facade = facade.clone();
    let ccx = planner_context(app.clone()).await;
    (gcx, temp, source, app, facade, ccx)
}

#[tokio::test]
async fn restart_refuses_failed_card_whose_dependency_is_not_done() {
    let (gcx, _temp, source, mut app, facade, ccx) = restart_with_dependency("planned").await;
    let holder = hold_capacity(&mut app, &source).await;

    let error = crate::tools::tool_task_restart_agent::ToolTaskRestartAgent::new()
        .tool_execute(
            ccx,
            &"restart".to_string(),
            &std::collections::HashMap::from([("card_id".to_string(), json!("B"))]),
        )
        .await
        .unwrap_err();

    assert!(error.contains("A"), "{error}");
    let board = storage::load_board(gcx, "task-1").await.unwrap();
    assert_eq!(board.get_card("B").unwrap().column, "failed");
    assert!(facade.created.lock().unwrap().is_empty());
    assert_eq!(app.work_scheduler.snapshot().outstanding, 1);
    crate::tasks::admission::shutdown(&app).await;
    holder.release();
}

#[tokio::test]
async fn restart_proceeds_when_dependencies_are_done() {
    let (gcx, _temp, source, mut app, facade, ccx) = restart_with_dependency("done").await;
    let holder = hold_capacity(&mut app, &source).await;

    crate::tools::tool_task_restart_agent::ToolTaskRestartAgent::new()
        .tool_execute(
            ccx,
            &"restart".to_string(),
            &std::collections::HashMap::from([("card_id".to_string(), json!("B"))]),
        )
        .await
        .unwrap();

    let board = storage::load_board(gcx, "task-1").await.unwrap();
    assert_eq!(board.get_card("B").unwrap().column, "doing");
    assert!(facade.created.lock().unwrap().is_empty());
    crate::tasks::admission::shutdown(&app).await;
    holder.release();
}
