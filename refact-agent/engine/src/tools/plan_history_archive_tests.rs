use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ContextEnum;
use crate::chat::types::ChatSession;
use crate::tools::tool_get_goal::ToolGetGoal;
use crate::tools::tool_get_plan::ToolGetPlan;
use crate::tools::tools_description::Tool;
use refact_chat_api::role_projection::{fold_role_messages, ProjectedRole};

async fn ccx(app: AppState, chat_id: &str) -> Arc<AMutex<AtCommandsContext>> {
    Arc::new(AMutex::new(
        AtCommandsContext::new_from_app(
            app,
            4096,
            20,
            false,
            vec![],
            chat_id.to_string(),
            None,
            "model".to_string(),
            None,
            None,
        )
        .await,
    ))
}

async fn insert_session(app: &AppState, session: ChatSession) {
    app.chat
        .sessions
        .write()
        .await
        .insert(session.chat_id.clone(), Arc::new(AMutex::new(session)));
}

fn result_json(result: (bool, Vec<ContextEnum>)) -> Value {
    assert!(!result.0);
    match result.1.into_iter().next().expect("tool output") {
        ContextEnum::ChatMessage(message) => {
            serde_json::from_str(&message.content.content_text_only()).unwrap()
        }
        ContextEnum::ContextFile(_) => panic!("expected chat message"),
    }
}

fn role_session(role: ProjectedRole, count: usize, folded: bool) -> ChatSession {
    let mut session = ChatSession::new(format!("archive-{}-{folded}", role.noun()));
    let base = match role {
        ProjectedRole::Plan => crate::chat::internal_roles::plan("agent", 1, "Approved base", None),
        ProjectedRole::Goal => crate::chat::internal_roles::goal(
            "agent",
            1,
            "Approved base",
            None,
            true,
            refact_chat_api::GoalBudget::default(),
        ),
    };
    let mut base = base;
    base.message_id = format!("approved-{}-base", role.noun());
    base.extra[role.noun()]["created_at_ms"] = json!(1);
    session.messages.push(base);
    for seq in 1..=count {
        let mut delta = match role {
            ProjectedRole::Plan => crate::chat::internal_roles::plan_delta(
                "tool.update_plan",
                json!({"seq": seq}),
                format!("update {seq}"),
            ),
            ProjectedRole::Goal => crate::chat::internal_roles::goal_delta(
                "tool.update_goal",
                json!({"seq": seq}),
                format!("update {seq}"),
            ),
        };
        delta.message_id = format!("{}-delta-{seq}", role.noun());
        session.messages.push(delta);
    }
    if folded {
        let folded_messages = fold_role_messages(
            role,
            &session.messages,
            crate::runtime_settings::plan_projection_limits(),
        );
        let report = refact_core::active_context::make_reconstruction_report(
            folded_messages,
            Default::default(),
        )
        .unwrap();
        session.messages.push(report);
    }
    session
}

async fn get_plan(session: ChatSession, include_history: bool) -> Value {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let app = AppState::from_gcx(gcx).await;
    let chat_id = session.chat_id.clone();
    insert_session(&app, session).await;
    result_json(
        ToolGetPlan::new(String::new())
            .tool_execute(
                ccx(app, &chat_id).await,
                &"read-plan".to_string(),
                &HashMap::from([("include_history".to_string(), json!(include_history))]),
            )
            .await
            .unwrap(),
    )["plan"]
        .clone()
}

async fn get_goal(session: ChatSession, include_history: bool) -> Value {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let app = AppState::from_gcx(gcx).await;
    let chat_id = session.chat_id.clone();
    insert_session(&app, session).await;
    result_json(
        ToolGetGoal::new(String::new())
            .tool_execute(
                ccx(app, &chat_id).await,
                &"read-goal".to_string(),
                &HashMap::from([("include_history".to_string(), json!(include_history))]),
            )
            .await
            .unwrap(),
    )["goal"]
        .clone()
}

fn assert_complete_history(value: &Value) {
    let history = value["history"].as_array().expect("history");
    assert_eq!(history.len(), 100);
    for (index, entry) in history.iter().enumerate() {
        let seq = index + 1;
        assert_eq!(
            entry,
            &json!({"seq": seq, "content": format!("update {seq}")})
        );
    }
    assert!(history
        .iter()
        .all(|entry| !entry["content"].as_str().unwrap().contains("checkpoint")));
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn history_after_fold_lists_all_deltas_once() {
    crate::runtime_settings::reset_for_test();
    let plan = get_plan(role_session(ProjectedRole::Plan, 100, true), true).await;
    assert_complete_history(&plan);
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn goal_history_after_fold_lists_all_deltas_once() {
    crate::runtime_settings::reset_for_test();
    let goal = get_goal(role_session(ProjectedRole::Goal, 100, true), true).await;
    assert_complete_history(&goal);
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn projection_fields_unchanged_by_fold() {
    crate::runtime_settings::reset_for_test();
    let before_plan = get_plan(role_session(ProjectedRole::Plan, 100, false), false).await;
    let after_plan = get_plan(role_session(ProjectedRole::Plan, 100, true), false).await;
    for field in ["content", "delta_count", "checkpoint"] {
        assert_eq!(before_plan[field], after_plan[field], "plan {field}");
    }

    let before_goal = get_goal(role_session(ProjectedRole::Goal, 100, false), false).await;
    let after_goal = get_goal(role_session(ProjectedRole::Goal, 100, true), false).await;
    for field in ["content", "delta_count", "checkpoint"] {
        assert_eq!(before_goal[field], after_goal[field], "goal {field}");
    }
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn history_without_fold_is_unchanged() {
    crate::runtime_settings::reset_for_test();
    let plan = get_plan(role_session(ProjectedRole::Plan, 3, false), true).await;
    assert_eq!(
        plan["history"],
        json!([
            {"seq": 1, "content": "update 1"},
            {"seq": 2, "content": "update 2"},
            {"seq": 3, "content": "update 3"}
        ])
    );
    let goal = get_goal(role_session(ProjectedRole::Goal, 3, false), true).await;
    assert_eq!(goal["history"], plan["history"]);
}
