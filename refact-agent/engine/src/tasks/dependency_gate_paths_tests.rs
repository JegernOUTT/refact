use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Json;
use chrono::Utc;
use serde_json::json;

use crate::app_state::AppState;
use crate::http::routers::v1::tasks::{self, BoardPatch, UpdateBoardRequest};
use crate::tasks::storage;
use crate::tasks::types::{BoardCard, TaskBoard, TaskMeta, TaskStatus};

fn card(id: &str, depends_on: &[&str]) -> BoardCard {
    BoardCard {
        id: id.to_string(),
        title: format!("Card {id}"),
        column: "planned".to_string(),
        priority: "P1".to_string(),
        depends_on: depends_on.iter().map(|value| value.to_string()).collect(),
        instructions: String::new(),
        assignee: None,
        agent_chat_id: None,
        status_updates: vec![],
        comments: vec![],
        final_report: None,
        final_report_structured: None,
        verifier_report: None,
        created_at: Utc::now().to_rfc3339(),
        started_at: None,
        last_heartbeat_at: None,
        paused_at: None,
        paused_by: None,
        completed_at: None,
        agent_branch: None,
        agent_worktree: None,
        agent_worktree_name: None,
        base_branch: None,
        base_commit: None,
        ab_variants: None,
        target_files: vec![],
        scope_guard_mode: Default::default(),
        team_members: vec![],
        attention: None,
        deliverable: None,
        verification: Vec::new(),
    }
}

async fn setup_http_task(
    task_id: &str,
) -> (tempfile::TempDir, Arc<crate::global_context::GlobalContext>) {
    let temp = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
    let task_dir = temp.path().join(".refact/tasks").join(task_id);
    tokio::fs::create_dir_all(&task_dir).await.unwrap();
    let now = Utc::now().to_rfc3339();
    storage::save_task_meta(
        gcx.clone(),
        task_id,
        &TaskMeta {
            schema_version: 1,
            id: task_id.to_string(),
            name: task_id.to_string(),
            status: TaskStatus::Planning,
            created_at: now.clone(),
            updated_at: now,
            cards_total: 2,
            cards_done: 0,
            cards_failed: 0,
            agents_active: 0,
            base_branch: None,
            base_commit: None,
            default_agent_model: None,
            is_name_generated: false,
            last_agents_summary_at: None,
            planner_session_state: None,
        },
    )
    .await
    .unwrap();
    storage::save_board(
        gcx.clone(),
        task_id,
        &TaskBoard {
            cards: vec![card("A", &[]), card("B", &["A"])],
            ..TaskBoard::default()
        },
    )
    .await
    .unwrap();
    (temp, gcx)
}

fn app(gcx: Arc<crate::global_context::GlobalContext>) -> AppState {
    gcx.app_state(gcx.clone())
}

async fn move_card(
    gcx: Arc<crate::global_context::GlobalContext>,
    task_id: &str,
    rev: u64,
    column: &str,
) -> Result<Json<TaskBoard>, (StatusCode, String)> {
    tasks::handle_patch_board(
        State(app(gcx)),
        Path(task_id.to_string()),
        Json(UpdateBoardRequest {
            rev,
            patch: BoardPatch::MoveCard {
                id: "B".to_string(),
                column: column.to_string(),
            },
        }),
    )
    .await
}

#[tokio::test]
async fn http_move_into_doing_with_unmet_dependency_is_refused() {
    let (_temp, gcx) = setup_http_task("http-blocked").await;

    let (_, message) = move_card(gcx.clone(), "http-blocked", 0, "doing")
        .await
        .unwrap_err();

    assert!(message.contains("A"), "{message}");
    assert_eq!(
        storage::load_board(gcx, "http-blocked")
            .await
            .unwrap()
            .get_card("B")
            .unwrap()
            .column,
        "planned"
    );
}

#[tokio::test]
async fn http_move_into_doing_after_override_is_allowed() {
    let (_temp, gcx) = setup_http_task("http-override").await;
    storage::update_board_atomic(gcx.clone(), "http-override", |board| {
        board.owner_chat_id = Some("planner-chat".to_string());
        Ok(())
    })
    .await
    .unwrap();
    storage::override_card_dependencies(
        gcx.clone(),
        "http-override",
        "B",
        "planner-chat",
        "dependency is satisfied externally",
    )
    .await
    .unwrap();
    let rev = storage::load_board(gcx.clone(), "http-override")
        .await
        .unwrap()
        .rev;

    let board = move_card(gcx, "http-override", rev, "doing")
        .await
        .unwrap()
        .0;

    assert_eq!(board.get_card("B").unwrap().column, "doing");
}

#[tokio::test]
async fn http_move_to_terminal_columns_is_never_gated() {
    let (_temp, gcx) = setup_http_task("http-terminal").await;

    let board = move_card(gcx, "http-terminal", 0, "failed")
        .await
        .unwrap()
        .0;

    assert_eq!(board.get_card("B").unwrap().column, "failed");
}

#[tokio::test]
async fn spawn_agent_refuses_card_with_unmet_dependency_before_side_effects() {
    use crate::tools::tool_spawn_ab::tests::{setup_repo_task, RecordingFacade};
    use crate::tools::tools_description::Tool;

    let (gcx, _temp, _source) = setup_repo_task().await;
    storage::update_board_atomic(gcx.clone(), "task-1", |board| {
        let mut dependent = card("T-2", &["T-1"]);
        dependent.title = "Dependent card".to_string();
        board.cards.push(dependent);
        Ok(())
    })
    .await
    .unwrap();

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
    {
        let mut state = gcx.caps_state.write().await;
        state.caps = Some(Arc::new(caps));
        state.last_attempted_ts = now;
    }

    let mut app = AppState::from_gcx(gcx.clone()).await;
    let facade = Arc::new(RecordingFacade::default());
    app.chat.facade = facade.clone();
    let ccx = Arc::new(tokio::sync::Mutex::new(
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
    ));

    let error = crate::tools::tool_task_spawn_agent::ToolTaskSpawnAgent::new()
        .tool_execute(
            ccx,
            &"call".to_string(),
            &std::collections::HashMap::from([("card_id".to_string(), json!("T-2"))]),
        )
        .await
        .unwrap_err();

    assert!(error.contains("T-1"), "{error}");
    let board = storage::load_board(gcx.clone(), "task-1").await.unwrap();
    let blocked = board.get_card("T-2").unwrap();
    assert_eq!(blocked.column, "planned");
    assert!(blocked.agent_chat_id.is_none());
    assert!(facade.created.lock().unwrap().is_empty());
    assert!(!gcx.cache_dir.join("worktrees").exists());
    assert!(!gcx
        .chat_sessions
        .read()
        .await
        .keys()
        .any(|chat_id| chat_id.starts_with("agent-T-2-")));
}
