use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::plan_role;
use crate::chat::types::ChatSession;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolGetPlan {
    pub config_path: String,
}

impl ToolGetPlan {
    pub fn new(config_path: String) -> Self {
        Self { config_path }
    }
}

fn plan_value(session: &ChatSession, message: &ChatMessage) -> Result<Value, String> {
    let meta = message
        .extra
        .get("plan")
        .ok_or_else(|| "current plan is missing plan metadata".to_string())?;
    let mode = meta
        .get("mode")
        .and_then(Value::as_str)
        .ok_or_else(|| "current plan is missing mode".to_string())?;
    let version = meta
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| "current plan is missing version".to_string())?;
    let created_at_ms = meta
        .get("created_at_ms")
        .and_then(Value::as_u64)
        .ok_or_else(|| "current plan is missing created_at_ms".to_string())?;
    let content = plan_role::synthesize_current_plan(session)
        .ok_or_else(|| "current plan could not be synthesized".to_string())?;
    let delta_count = plan_role::plan_delta_events(session).len();

    Ok(json!({
        "content": content,
        "mode": mode,
        "version": version,
        "created_at_ms": created_at_ms,
        "delta_count": delta_count,
    }))
}

fn output_message(tool_call_id: &str, value: Value) -> Result<ContextEnum, String> {
    let content = serde_json::to_string(&value)
        .map_err(|error| format!("failed to serialize get_plan output: {error}"))?;
    Ok(ContextEnum::ChatMessage(ChatMessage {
        role: "tool".to_string(),
        content: ChatContent::SimpleText(content),
        tool_calls: None,
        tool_call_id: tool_call_id.to_string(),
        ..Default::default()
    }))
}

#[async_trait]
impl Tool for ToolGetPlan {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "get_plan".to_string(),
            display_name: "Get Plan".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Read the current plan installed on this chat. Returns the merged current content, mode, base version, creation timestamp, and delta count.".to_string(),
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
        let plan = match plan_role::current_base_plan(&session) {
            Some(message) => plan_value(&session, message)?,
            None => Value::Null,
        };
        Ok((
            false,
            vec![output_message(tool_call_id, json!({ "plan": plan }))?],
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
    use crate::chat::trajectories::{
        load_trajectory_for_chat, save_trajectory_snapshot, TrajectorySnapshot,
    };
    use crate::tools::tool_set_plan::ToolSetPlan;
    use crate::tools::tool_update_plan::ToolUpdatePlan;

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

    async fn drain_plan_side_effects(app: &AppState, chat_id: &str) {
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

    async fn session_messages(app: &AppState, chat_id: &str) -> Vec<ChatMessage> {
        let session_arc = app
            .chat
            .sessions
            .read()
            .await
            .get(chat_id)
            .cloned()
            .unwrap();
        let session = session_arc.lock().await;
        session.messages.clone()
    }

    #[tokio::test]
    async fn plan_lifecycle_smoke() {
        let workspace = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let app = AppState::from_gcx(gcx.clone()).await;
        let chat_id = "plan-lifecycle-smoke";
        let mut session = ChatSession::new(chat_id.to_string());
        session.thread.mode = "agent".to_string();
        session.thread.tool_use = "agent".to_string();
        session.thread.model = "model".to_string();
        session.thread.title = "Plan Lifecycle Smoke".to_string();
        session.created_at = "2024-01-01T00:00:00Z".to_string();
        insert_session(&app, session).await;

        let mut set_tool = ToolSetPlan {
            config_path: String::new(),
        };
        let set_result = result_json(
            set_tool
                .tool_execute(
                    ccx(app.clone(), chat_id).await,
                    &"set-plan".to_string(),
                    &HashMap::from([("content".to_string(), json!("## Plan\n- base"))]),
                )
                .await
                .unwrap(),
        );
        assert_eq!(set_result, json!({"version": 1, "supersedes": null}));
        drain_plan_side_effects(&app, chat_id).await;

        let messages = session_messages(&app, chat_id).await;
        let plans: Vec<_> = messages
            .iter()
            .filter(|message| message.role == internal_roles::PLAN_ROLE)
            .collect();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].extra["plan"]["version"], json!(1));
        assert_eq!(plans[0].content.content_text_only(), "## Plan\n- base");

        let duplicate_err = set_tool
            .tool_execute(
                ccx(app.clone(), chat_id).await,
                &"set-plan-duplicate".to_string(),
                &HashMap::from([("content".to_string(), json!("second"))]),
            )
            .await
            .unwrap_err();
        assert_eq!(
            duplicate_err,
            "a plan already exists; use update_plan to change it"
        );

        let mut update_tool = ToolUpdatePlan {
            config_path: String::new(),
        };
        let update_result = result_json(
            update_tool
                .tool_execute(
                    ccx(app.clone(), chat_id).await,
                    &"update-plan".to_string(),
                    &HashMap::from([("note".to_string(), json!("- changed"))]),
                )
                .await
                .unwrap(),
        );
        assert_eq!(update_result, json!({"seq": 1, "truncated": false}));
        drain_plan_side_effects(&app, chat_id).await;

        let mut get_tool = ToolGetPlan::new(String::new());
        let get_result = result_json(
            get_tool
                .tool_execute(
                    ccx(app.clone(), chat_id).await,
                    &"get-plan".to_string(),
                    &HashMap::new(),
                )
                .await
                .unwrap(),
        );
        assert_eq!(get_result["plan"]["version"], json!(1));
        assert_eq!(get_result["plan"]["delta_count"], json!(1));
        assert_eq!(get_result["plan"]["mode"], json!("agent"));
        assert_eq!(
            get_result["plan"]["content"],
            json!("## Plan\n- base\n\n---\n\n## Plan updates\n\n- changed")
        );

        let session_arc = app
            .chat
            .sessions
            .read()
            .await
            .get(chat_id)
            .cloned()
            .unwrap();
        let snapshot = {
            let session = session_arc.lock().await;
            TrajectorySnapshot::from_thread_parts(
                chat_id.to_string(),
                &session.thread,
                session.messages.clone(),
                session.created_at.clone(),
                session.trajectory_version,
            )
        };
        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();

        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        let loaded_session =
            session_from_snapshot(chat_id.to_string(), loaded.thread, loaded.messages);
        let loaded_plans: Vec<_> = loaded_session
            .messages
            .iter()
            .filter(|message| message.role == internal_roles::PLAN_ROLE)
            .collect();
        assert_eq!(loaded_plans.len(), 1);
        assert_eq!(loaded_plans[0].extra["plan"]["version"], json!(1));
        assert_eq!(
            loaded_plans[0].content.content_text_only(),
            "## Plan\n- base"
        );
        let loaded_deltas = plan_role::plan_delta_events(&loaded_session);
        assert_eq!(loaded_deltas.len(), 1);
        assert_eq!(
            loaded_deltas[0].extra["event"]["subkind"],
            json!("plan_delta")
        );
        assert_eq!(loaded_deltas[0].extra["event"]["payload"]["seq"], json!(1));
        assert_eq!(loaded_deltas[0].content.content_text_only(), "- changed");
        assert_eq!(
            plan_role::synthesize_current_plan(&loaded_session).unwrap(),
            "## Plan\n- base\n\n---\n\n## Plan updates\n\n- changed"
        );
    }

    #[tokio::test]
    async fn no_plan_returns_null() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let chat_id = "get-plan-no-plan";
        insert_session(&app, ChatSession::new(chat_id.to_string())).await;
        let mut tool = ToolGetPlan::new(String::new());

        let output = result_json(
            tool.tool_execute(ccx(app, chat_id).await, &"tc".to_string(), &HashMap::new())
                .await
                .unwrap(),
        );

        assert_eq!(output, json!({ "plan": null }));
    }

    #[tokio::test]
    async fn with_plan_returns_synthesized_plan() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let chat_id = "get-plan-with-plan";
        let mut session = ChatSession::new(chat_id.to_string());
        session.install_plan("agent", "base plan");
        session.add_message(internal_roles::plan_delta(
            "tool.update_plan",
            json!({"seq": 1}),
            "first update",
        ));
        session.add_message(internal_roles::plan_delta(
            "tool.update_plan",
            json!({"seq": 2}),
            "second update",
        ));
        let created_at_ms = session.messages[0].extra["plan"]["created_at_ms"]
            .as_u64()
            .unwrap();
        insert_session(&app, session).await;
        let mut tool = ToolGetPlan::new(String::new());

        let output = result_json(
            tool.tool_execute(ccx(app, chat_id).await, &"tc".to_string(), &HashMap::new())
                .await
                .unwrap(),
        );

        assert_eq!(
            output["plan"]["content"],
            json!("base plan\n\n---\n\n## Plan updates\n\nfirst update\n\nsecond update")
        );
        assert_eq!(output["plan"]["mode"], json!("agent"));
        assert_eq!(output["plan"]["version"], json!(1));
        assert_eq!(output["plan"]["created_at_ms"], json!(created_at_ms));
        assert_eq!(output["plan"]["delta_count"], json!(2));
    }
}
