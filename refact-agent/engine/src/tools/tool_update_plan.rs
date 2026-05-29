use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::internal_roles::{self, EventSubkind};
use crate::chat::plan_role;
use crate::chat::types::ChatSession;
use crate::tools::tools_description::{
    json_schema_from_params, Tool, ToolDesc, ToolSource, ToolSourceType,
};

pub struct ToolUpdatePlan {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolUpdatePlan {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "update_plan".to_string(),
            display_name: "Update Plan".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Append an incremental update to the current plan (cache-safe delta merged into the current plan). Use when the plan evolves; it does not rewrite the original plan.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("note", "string", "Plan update note. Required."),
                    (
                        "summary",
                        "string",
                        "Short description of what changed, ≤120 chars. Optional.",
                    ),
                ],
                &["note"],
            ),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let note = string_arg(args, "note")?;
        if note.trim().is_empty() {
            return Err("argument `note` must be non-empty".to_string());
        }
        let summary = optional_string_arg(args, "summary")?;
        if summary
            .as_ref()
            .is_some_and(|summary| summary.chars().count() > 120)
        {
            return Err("argument `summary` must be at most 120 chars".to_string());
        }

        let (gcx, chat_id) = {
            let cgcx = ccx.lock().await;
            (cgcx.app.gcx.clone(), cgcx.chat_id.clone())
        };
        let session_arc = {
            let sessions = gcx.chat_sessions.read().await;
            sessions.get(&chat_id).cloned()
        }
        .ok_or_else(|| format!("chat session `{chat_id}` not found"))?;

        let seq = {
            let mut session = session_arc.lock().await;
            if !has_base_plan_including_queued(&session) {
                return Err("no plan to update; call set_plan first".to_string());
            }
            let seq = plan_delta_count_including_queued(&session) + 1;
            session.queue_post_tool_side_effect(internal_roles::plan_delta(
                "tool.update_plan",
                json!({"seq": seq, "summary": summary}),
                note,
            ));
            session.queue_post_tool_side_effect(internal_roles::event(
                EventSubkind::SystemNotice,
                "tool.update_plan",
                json!({"seq": seq, "summary": summary}),
                format!("Plan updated (delta {seq})"),
            ));
            seq
        };

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(json!({ "seq": seq }).to_string()),
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }
}

fn has_base_plan_including_queued(session: &ChatSession) -> bool {
    plan_role::current_base_plan(session).is_some()
        || session
            .post_tool_side_effects
            .iter()
            .any(|message| message.role == internal_roles::PLAN_ROLE)
}

fn plan_delta_count_including_queued(session: &ChatSession) -> usize {
    plan_role::plan_delta_events(session).len()
        + session
            .post_tool_side_effects
            .iter()
            .filter(|message| is_plan_delta(message))
            .count()
}

fn is_plan_delta(message: &ChatMessage) -> bool {
    message.role == internal_roles::EVENT_ROLE
        && message
            .extra
            .get("event")
            .and_then(|event| event.get("subkind"))
            .and_then(|subkind| subkind.as_str())
            == Some("plan_delta")
}

fn string_arg(args: &HashMap<String, Value>, name: &str) -> Result<String, String> {
    match args.get(name) {
        Some(Value::String(value)) => Ok(value.clone()),
        Some(value) => Err(format!("argument `{name}` is not a string: {value:?}")),
        None => Err(format!("argument `{name}` is missing")),
    }
}

fn optional_string_arg(
    args: &HashMap<String, Value>,
    name: &str,
) -> Result<Option<String>, String> {
    match args.get(name) {
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(Value::Null) | None => Ok(None),
        Some(value) => Err(format!("argument `{name}` is not a string: {value:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::chat::internal_roles::{EVENT_ROLE, PLAN_ROLE};
    use crate::tools::tools_list::get_tools_for_mode;

    const CHAT_ID: &str = "update-plan-chat";

    async fn ccx_for_session(
        session: ChatSession,
    ) -> (
        Arc<crate::global_context::GlobalContext>,
        Arc<AMutex<AtCommandsContext>>,
    ) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(
            &gcx.config_dir,
        )
        .await
        .unwrap();
        gcx.chat_sessions
            .write()
            .await
            .insert(CHAT_ID.to_string(), Arc::new(AMutex::new(session)));
        (gcx.clone(), make_ccx(gcx).await)
    }

    async fn make_ccx(
        gcx: Arc<crate::global_context::GlobalContext>,
    ) -> Arc<AMutex<AtCommandsContext>> {
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                AppState::from_gcx(gcx).await,
                4096,
                20,
                false,
                vec![],
                CHAT_ID.to_string(),
                None,
                "model".to_string(),
                None,
                None,
            )
            .await,
        ))
    }

    fn content_text(message: &ChatMessage) -> &str {
        match &message.content {
            ChatContent::SimpleText(text) => text,
            ChatContent::Multimodal(_) | ChatContent::ContextFiles(_) => {
                panic!("expected text message")
            }
        }
    }

    fn tool_result_json(messages: &[ContextEnum]) -> Value {
        match &messages[0] {
            ContextEnum::ChatMessage(message) => {
                serde_json::from_str(content_text(message)).unwrap()
            }
            ContextEnum::ContextFile(_) => panic!("expected tool chat message"),
        }
    }

    fn plan_delta_payload(message: &ChatMessage) -> &Value {
        &message.extra["event"]["payload"]
    }

    #[tokio::test]
    async fn appends_plan_delta_event() {
        let mut session = ChatSession::new(CHAT_ID.to_string());
        session.install_plan("agent", "## Plan\n- base");
        session.add_message(internal_roles::plan_delta(
            "tool.update_plan",
            json!({"seq": 1, "summary": "first"}),
            "first note",
        ));
        let (gcx, ccx) = ccx_for_session(session).await;
        let mut tool = ToolUpdatePlan {
            config_path: String::new(),
        };
        let args = HashMap::from([
            ("note".to_string(), json!("second note")),
            ("summary".to_string(), json!("second")),
        ]);

        let (_, messages) = tool
            .tool_execute(ccx.clone(), &"call".to_string(), &args)
            .await
            .unwrap();

        assert_eq!(tool_result_json(&messages), json!({"seq": 2}));
        let session_arc = gcx
            .chat_sessions
            .read()
            .await
            .get(CHAT_ID)
            .cloned()
            .unwrap();
        let session = session_arc.lock().await;
        assert_eq!(session.post_tool_side_effects.len(), 2);
        assert_eq!(session.post_tool_side_effects[0].role, EVENT_ROLE);
        assert_eq!(
            content_text(&session.post_tool_side_effects[0]),
            "second note"
        );
        assert_eq!(
            session.post_tool_side_effects[0].extra["event"],
            json!({
                "subkind": "plan_delta",
                "source": "tool.update_plan",
                "payload": {"seq": 2, "summary": "second"}
            })
        );
        assert_eq!(session.post_tool_side_effects[1].role, EVENT_ROLE);
        assert_eq!(
            content_text(&session.post_tool_side_effects[1]),
            "Plan updated (delta 2)"
        );
        assert_eq!(
            session.post_tool_side_effects[1].extra["event"],
            json!({
                "subkind": "system_notice",
                "source": "tool.update_plan",
                "payload": {"seq": 2, "summary": "second"}
            })
        );
        drop(session);

        let (_, messages) = tool
            .tool_execute(
                ccx,
                &"call2".to_string(),
                &HashMap::from([("note".to_string(), json!("third note"))]),
            )
            .await
            .unwrap();

        assert_eq!(tool_result_json(&messages), json!({"seq": 3}));
        let session_arc = gcx
            .chat_sessions
            .read()
            .await
            .get(CHAT_ID)
            .cloned()
            .unwrap();
        let session = session_arc.lock().await;
        assert_eq!(session.post_tool_side_effects.len(), 4);
        assert_eq!(
            plan_delta_payload(&session.post_tool_side_effects[2])["seq"],
            json!(3)
        );
        assert_eq!(
            plan_delta_payload(&session.post_tool_side_effects[2])["summary"],
            Value::Null
        );
    }

    #[tokio::test]
    async fn errors_when_no_plan() {
        let session = ChatSession::new(CHAT_ID.to_string());
        let (_gcx, ccx) = ccx_for_session(session).await;
        let mut tool = ToolUpdatePlan {
            config_path: String::new(),
        };
        let args = HashMap::from([("note".to_string(), json!("new direction"))]);

        let err = tool
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap_err();

        assert_eq!(err, "no plan to update; call set_plan first");
    }

    #[test]
    fn queued_base_plan_allows_update() {
        let mut session = ChatSession::new(CHAT_ID.to_string());
        session.queue_post_tool_side_effect(internal_roles::plan("agent", 1, "base", None));

        assert!(has_base_plan_including_queued(&session));
        assert_eq!(session.post_tool_side_effects[0].role, PLAN_ROLE);
    }

    #[tokio::test]
    async fn available_in_agent_task_planner_task_agent_modes() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        crate::yaml_configs::project_configs_bootstrap::global_configs_try_create_all(
            &gcx.config_dir,
        )
        .await
        .unwrap();

        let supported = get_tools_for_mode(gcx.clone(), "agent", None)
            .await
            .into_iter()
            .any(|tool| tool.tool_description().name == "update_plan");
        let task_planner = get_tools_for_mode(gcx.clone(), "task_planner", None)
            .await
            .into_iter()
            .any(|tool| tool.tool_description().name == "update_plan");
        let task_agent = get_tools_for_mode(gcx.clone(), "task_agent", None)
            .await
            .into_iter()
            .any(|tool| tool.tool_description().name == "update_plan");
        let no_tools = get_tools_for_mode(gcx.clone(), "NO_TOOLS", None)
            .await
            .into_iter()
            .any(|tool| tool.tool_description().name == "update_plan");
        let shell = get_tools_for_mode(gcx.clone(), "shell", None)
            .await
            .into_iter()
            .any(|tool| tool.tool_description().name == "update_plan");
        let explore = get_tools_for_mode(gcx, "explore", None)
            .await
            .into_iter()
            .any(|tool| tool.tool_description().name == "update_plan");

        assert!(supported);
        assert!(task_planner);
        assert!(task_agent);
        assert!(!no_tools);
        assert!(!shell);
        assert!(!explore);
    }
}
