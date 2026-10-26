use std::sync::Arc;

use tokio::sync::Mutex;

use crate::agents::types::{BgAgentKind, CreateAgentRequest};
use crate::app_state::AppState;
use crate::chat::types::{ChatSession, TaskMeta};
use crate::tools::tools_list::{chat_has_parent_agent, filter_chat_tools, filter_tool_catalog_for_chat};

async fn fixture() -> AppState {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(&gcx.config_dir)
        .await
        .unwrap();
    AppState::from_gcx(gcx).await
}

fn board_task_meta() -> TaskMeta {
    TaskMeta {
        task_id: "task-board".into(),
        role: "agents".into(),
        agent_id: Some("board-agent".into()),
        card_id: Some("T-1".into()),
        planner_chat_id: Some("planner-chat".into()),
    }
}

async fn install_session(app: &AppState, chat_id: &str, task_meta: Option<TaskMeta>) {
    let mut session = ChatSession::new(chat_id.into());
    session.thread.task_meta = task_meta;
    app.chat
        .sessions
        .write()
        .await
        .insert(chat_id.into(), Arc::new(Mutex::new(session)));
}

async fn bind_child(app: &AppState, chat_id: &str) -> String {
    let (record, _, _) = app
        .agents
        .create(CreateAgentRequest {
            parent_chat_id: "parent-chat".into(),
            parent_root_chat_id: Some("parent-chat".into()),
            parent_tool_call_id: None,
            kind: BgAgentKind::Subagent,
            config_name: "subagent".into(),
            title: "Tool exposure child".into(),
            prompt: "Check tool exposure".into(),
            target_files: vec![],
            model: "test/model".into(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
        })
        .await
        .unwrap();
    app.agents
        .mark_running(&record.agent_id, chat_id.into())
        .await
        .unwrap();
    record.agent_id
}

fn names(catalog: &refact_runtime_api::ToolCatalogSnapshot) -> Vec<String> {
    catalog
        .index
        .tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect()
}

#[tokio::test]
async fn progress_report_hidden_for_top_level_chat_and_visible_for_child() {
    let app = fixture().await;
    install_session(&app, "top-level-chat", None).await;
    install_session(&app, "child-chat", None).await;
    let child_agent_id = bind_child(&app, "child-chat").await;
    let shared = app
        .tool_registry
        .acquire_tool_catalog("agent", None, None)
        .await;
    assert!(names(&shared).contains(&"progress_report".into()));

    let top = filter_tool_catalog_for_chat(&app, "top-level-chat", shared.clone()).await;
    assert!(!names(&top).contains(&"progress_report".into()));
    assert!(names(&top).contains(&"agent_message".into()));
    assert!(top.policy.iter().all(|tool| tool.name != "progress_report"));
    assert!(top.aliases.get_alias("progress_report").is_none());
    assert_eq!(
        names(&top),
        names(&shared)
            .into_iter()
            .filter(|name| name != "progress_report")
            .collect::<Vec<_>>()
    );

    let child = filter_tool_catalog_for_chat(&app, "child-chat", shared.clone()).await;
    assert!(names(&child).contains(&"progress_report".into()));
    assert!(Arc::ptr_eq(&shared, &child));
    let mut runner_tools = shared.index.tools.clone();
    filter_chat_tools(
        &mut runner_tools,
        chat_has_parent_agent(&app, "child-chat").await,
    );
    assert!(runner_tools
        .iter()
        .any(|tool| tool.name == "progress_report"));
    filter_chat_tools(
        &mut runner_tools,
        chat_has_parent_agent(&app, "top-level-chat").await,
    );
    assert!(!runner_tools
        .iter()
        .any(|tool| tool.name == "progress_report"));

    app.agents
        .mark_running(&child_agent_id, "replacement-child-chat".into())
        .await
        .unwrap();
    bind_child(&app, "top-level-chat").await;
    for chat_id in ["top-level-chat", "child-chat"] {
        let session = app
            .chat
            .sessions
            .read()
            .await
            .get(chat_id)
            .cloned()
            .unwrap();
        session.lock().await.release_turn_only_state();
    }
    let top_next = filter_tool_catalog_for_chat(&app, "top-level-chat", shared.clone()).await;
    let child_next = filter_tool_catalog_for_chat(&app, "child-chat", shared.clone()).await;
    assert_eq!(names(&top_next), names(&top));
    assert_eq!(names(&child_next), names(&child));
    assert!(names(&shared).contains(&"progress_report".into()));
}

#[tokio::test]
async fn progress_report_hidden_for_board_task_agent_without_parent_record() {
    let app = fixture().await;
    install_session(&app, "board-task-agent", Some(board_task_meta())).await;
    install_session(&app, "board-agent-with-parent", Some(board_task_meta())).await;
    bind_child(&app, "board-agent-with-parent").await;
    let shared = app
        .tool_registry
        .acquire_tool_catalog("task_agent", None, None)
        .await;
    assert!(names(&shared).contains(&"progress_report".into()));

    let board = filter_tool_catalog_for_chat(&app, "board-task-agent", shared.clone()).await;
    assert!(!names(&board).contains(&"progress_report".into()));
    assert!(names(&board).contains(&"agent_message".into()));
    let child = filter_tool_catalog_for_chat(&app, "board-agent-with-parent", shared).await;
    assert!(names(&child).contains(&"progress_report".into()));
}
