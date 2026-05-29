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
