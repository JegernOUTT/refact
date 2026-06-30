use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use refact_chat_api::{GoalProgress, GoalStatus};
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::goal_role;
use crate::chat::types::ChatSession;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolGetGoal {
    pub config_path: String,
}

impl ToolGetGoal {
    pub fn new(config_path: String) -> Self {
        Self { config_path }
    }
}

fn goal_value(session: &ChatSession, message: &ChatMessage) -> Result<Value, String> {
    let meta = message
        .extra
        .get("goal")
        .ok_or_else(|| "current goal is missing goal metadata".to_string())?;
    let version = meta
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| "current goal is missing version".to_string())?;
    let progress = GoalProgress::default();
    let content = goal_role::synthesize_current_goal(session)
        .ok_or_else(|| "current goal could not be synthesized".to_string())?;
    let delta_count = goal_role::goal_delta_events(session).len();

    Ok(json!({
        "content": content,
        "status": GoalStatus::Active,
        "version": version,
        "delta_count": delta_count,
        "turns_used": progress.turns_used,
        "tokens_used": progress.tokens_used,
        "latest_verdict": null,
        "gaps": [],
    }))
}

fn output_message(tool_call_id: &str, value: Value) -> Result<ContextEnum, String> {
    let content = serde_json::to_string(&value)
        .map_err(|error| format!("failed to serialize get_goal output: {error}"))?;
    Ok(ContextEnum::ChatMessage(ChatMessage {
        role: "tool".to_string(),
        content: ChatContent::SimpleText(content),
        tool_calls: None,
        tool_call_id: tool_call_id.to_string(),
        ..Default::default()
    }))
}

#[async_trait]
impl Tool for ToolGetGoal {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "get_goal".to_string(),
            display_name: "Get Goal".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Read the current goal installed on this chat. Returns merged goal content, status, version, delta count, budget counters, latest verifier verdict, and gaps.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "required": [],
            }),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        _args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (chat_facade, chat_id) = {
            let ccx = ccx.lock().await;
            (ccx.app.chat.facade.clone(), ccx.chat_id.clone())
        };

        let snapshot = chat_facade.session_snapshot(&chat_id).await?;
        let session = session_from_snapshot(chat_id, snapshot.thread, snapshot.messages);
        let goal = match goal_role::current_base_goal(&session) {
            Some(message) => goal_value(&session, message)?,
            None => Value::Null,
        };
        Ok((
            false,
            vec![output_message(tool_call_id, json!({ "goal": goal }))?],
        ))
    }
}

fn session_from_snapshot(
    chat_id: String,
    thread: crate::chat::types::ThreadParams,
    messages: Vec<ChatMessage>,
) -> ChatSession {
    let mut session = ChatSession::new(chat_id);
    session.thread = thread;
    session.messages = messages;
    session
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::chat::internal_roles;
    use crate::tools::tool_set_goal::ToolSetGoal;
    use crate::tools::tool_update_goal::ToolUpdateGoal;
    use crate::tools::tools_list::get_tools_for_mode;

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

    async fn drain_goal_side_effects(app: &AppState, chat_id: &str) {
        let session_arc = app
            .chat
            .sessions
            .read()
            .await
            .get(chat_id)
            .cloned()
            .unwrap();
        session_arc.lock().await.drain_post_tool_side_effects();
    }

    #[tokio::test]
    async fn goal_lifecycle_smoke() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(
            &gcx.config_dir,
        )
        .await
        .unwrap();
        let app = AppState::from_gcx(gcx.clone()).await;
        let chat_id = "goal-lifecycle-smoke";
        let mut session = ChatSession::new(chat_id.to_string());
        session.thread.mode = "agent".to_string();
        session.thread.tool_use = "agent".to_string();
        session.thread.model = "model".to_string();
        insert_session(&app, session).await;

        let mut set_tool = ToolSetGoal {
            config_path: String::new(),
        };
        let set_result = result_json(
            set_tool
                .tool_execute(
                    ccx(app.clone(), chat_id).await,
                    &"set-goal".to_string(),
                    &HashMap::from([("content".to_string(), json!("Ship the pond"))]),
                )
                .await
                .unwrap(),
        );
        assert_eq!(set_result, json!({"version": 1, "supersedes": null}));
        drain_goal_side_effects(&app, chat_id).await;

        let duplicate_err = set_tool
            .tool_execute(
                ccx(app.clone(), chat_id).await,
                &"set-goal-duplicate".to_string(),
                &HashMap::from([("content".to_string(), json!("second"))]),
            )
            .await
            .unwrap_err();
        assert_eq!(duplicate_err, "goal already exists; use update_goal");

        let mut update_tool = ToolUpdateGoal {
            config_path: String::new(),
        };
        let update_result = result_json(
            update_tool
                .tool_execute(
                    ccx(app.clone(), chat_id).await,
                    &"update-goal".to_string(),
                    &HashMap::from([("note".to_string(), json!("Add tests"))]),
                )
                .await
                .unwrap(),
        );
        assert_eq!(update_result, json!({"seq": 1, "truncated": false}));
        drain_goal_side_effects(&app, chat_id).await;

        let mut get_tool = ToolGetGoal::new(String::new());
        let get_result = result_json(
            get_tool
                .tool_execute(
                    ccx(app.clone(), chat_id).await,
                    &"get-goal".to_string(),
                    &HashMap::new(),
                )
                .await
                .unwrap(),
        );
        assert_eq!(get_result["goal"]["version"], json!(1));
        assert_eq!(get_result["goal"]["delta_count"], json!(1));
        assert_eq!(get_result["goal"]["status"], json!("active"));
        assert_eq!(get_result["goal"]["turns_used"], json!(0));
        assert_eq!(get_result["goal"]["tokens_used"], json!(0));
        assert_eq!(get_result["goal"]["latest_verdict"], Value::Null);
        assert_eq!(get_result["goal"]["gaps"], json!([]));
        assert_eq!(
            get_result["goal"]["content"],
            json!("Ship the pond\n\n---\n\n## Goal updates\n\nAdd tests")
        );
    }

    #[tokio::test]
    async fn no_goal_returns_null() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let chat_id = "get-goal-no-goal";
        insert_session(&app, ChatSession::new(chat_id.to_string())).await;
        let mut tool = ToolGetGoal::new(String::new());

        let output = result_json(
            tool.tool_execute(ccx(app, chat_id).await, &"tc".to_string(), &HashMap::new())
                .await
                .unwrap(),
        );

        assert_eq!(output, json!({ "goal": null }));
    }

    #[tokio::test]
    async fn with_goal_returns_synthesized_goal() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let chat_id = "get-goal-with-goal";
        let mut session = ChatSession::new(chat_id.to_string());
        crate::chat::goal_role::install_goal(
            &mut session,
            "agent",
            "base goal",
            true,
            refact_chat_api::GoalBudget::default(),
        );
        session.add_message(internal_roles::goal_delta(
            "tool.update_goal",
            json!({"seq": 1}),
            "first update",
        ));
        session.add_message(internal_roles::goal_delta(
            "tool.update_goal",
            json!({"seq": 2}),
            "second update",
        ));
        insert_session(&app, session).await;
        let mut tool = ToolGetGoal::new(String::new());

        let output = result_json(
            tool.tool_execute(ccx(app, chat_id).await, &"tc".to_string(), &HashMap::new())
                .await
                .unwrap(),
        );

        assert_eq!(
            output["goal"]["content"],
            json!("base goal\n\n---\n\n## Goal updates\n\nfirst update\n\nsecond update")
        );
        assert_eq!(output["goal"]["status"], json!("active"));
        assert_eq!(output["goal"]["version"], json!(1));
        assert_eq!(output["goal"]["delta_count"], json!(2));
    }

    #[tokio::test]
    async fn available_in_plan_supporting_modes() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(
            &gcx.config_dir,
        )
        .await
        .unwrap();

        for mode in ["agent", "task_agent", "task_planner"] {
            let has_tool = get_tools_for_mode(gcx.clone(), mode, None)
                .await
                .into_iter()
                .any(|tool| tool.tool_description().name == "get_goal");
            assert!(has_tool, "{mode} should expose get_goal");
        }
        for mode in ["NO_TOOLS", "shell", "explore"] {
            let has_tool = get_tools_for_mode(gcx.clone(), mode, None)
                .await
                .into_iter()
                .any(|tool| tool.tool_description().name == "get_goal");
            assert!(!has_tool, "{mode} should not expose get_goal");
        }
    }
}
