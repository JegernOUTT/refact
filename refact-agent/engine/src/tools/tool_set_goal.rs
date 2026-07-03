use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use refact_chat_api::GoalBudget;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::goal_role::{self, GoalInstallReport};
use crate::chat::internal_roles::{self, EventSubkind};
use crate::chat::types::ChatSession;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::yaml_configs::customization_registry::map_legacy_mode_to_id;

pub struct ToolSetGoal {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolSetGoal {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "set_goal".to_string(),
            display_name: "Set Goal".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Install the chat's single active goal. Fails if a goal already exists — use `update_goal` to evolve it.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "content": {"type": "string", "description": "Full goal body. Required."},
                    "criteria": {
                        "type": "array",
                        "description": "Optional structured success criteria the verifier checks one by one.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": {"type": "string", "description": "Short stable id, e.g. C1."},
                                "text": {"type": "string", "description": "The criterion itself."},
                                "verify_hint": {"type": "string", "description": "Optional hint on how to verify it."}
                            },
                            "required": ["id", "text"]
                        }
                    }
                },
                "required": ["content"],
            }),
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
        let content = string_arg(args, "content")?;
        if content.trim().is_empty() {
            return Err("argument `content` must be non-empty".to_string());
        }
        let criteria: Vec<refact_chat_api::GoalCriterion> = match args.get("criteria") {
            None | Some(Value::Null) => Vec::new(),
            Some(value) => serde_json::from_value(value.clone())
                .map_err(|error| format!("argument `criteria` is malformed: {error}"))?,
        };

        let (gcx, chat_id) = {
            let cgcx = ccx.lock().await;
            (cgcx.app.gcx.clone(), cgcx.chat_id.clone())
        };
        let session_arc = {
            let sessions = gcx.chat_sessions.read().await;
            sessions.get(&chat_id).cloned()
        }
        .ok_or_else(|| format!("chat session `{chat_id}` not found"))?;

        let report = {
            let mut session = session_arc.lock().await;
            if current_goal_including_queued(&session).is_some() {
                return Err("goal already exists; use update_goal".to_string());
            }
            let current_mode = map_legacy_mode_to_id(&session.thread.mode).to_string();
            let report = queue_goal_side_effect(&mut session, &current_mode, &content, &criteria);
            session.queue_post_tool_side_effect(internal_roles::event(
                EventSubkind::SystemNotice,
                "tool.set_goal",
                json!({"version": report.version}),
                format!("Goal updated to v{}", report.version),
            ));
            report
        };

        let result = json!({
            "version": report.version,
            "supersedes": report.supersedes,
        });

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result.to_string()),
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }
}

fn queue_goal_side_effect(
    session: &mut ChatSession,
    mode: &str,
    body: &str,
    criteria: &[refact_chat_api::GoalCriterion],
) -> GoalInstallReport {
    let mut message = internal_roles::goal(mode, 1, body, None, true, GoalBudget::default());
    if !criteria.is_empty() {
        if let Some(meta) = message
            .extra
            .get_mut("goal")
            .and_then(|value| value.as_object_mut())
        {
            meta.insert("criteria".to_string(), json!(criteria));
        }
    }
    session.queue_post_tool_side_effect(message);
    GoalInstallReport {
        version: 1,
        supersedes: None,
    }
}

fn current_goal_including_queued(session: &ChatSession) -> Option<&ChatMessage> {
    goal_role::current_base_goal(session).or_else(|| {
        session
            .post_tool_side_effects
            .iter()
            .enumerate()
            .filter_map(|(index, message)| {
                goal_role::goal_version(message).map(|version| (index, version, message))
            })
            .max_by_key(|(index, version, _)| (*version, *index))
            .map(|(_, _, message)| message)
    })
}

fn string_arg(args: &HashMap<String, Value>, name: &str) -> Result<String, String> {
    match args.get(name) {
        Some(Value::String(value)) => Ok(value.clone()),
        Some(value) => Err(format!("argument `{name}` is not a string: {value:?}")),
        None => Err(format!("argument `{name}` is missing")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::call_validation::{ChatToolCall, ChatToolFunction};
    use crate::chat::internal_roles::{EVENT_ROLE, GOAL_ROLE};
    use crate::llm::adapter::{AdapterSettings, LlmWireAdapter};
    use crate::llm::adapters::openai_chat::OpenAiChatAdapter;
    use crate::tools::tools_list::get_tools_for_mode;

    const CHAT_ID: &str = "set-goal-chat";

    async fn ccx_for_session(
        mode: &str,
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
        let mut session = ChatSession::new(CHAT_ID.to_string());
        session.thread.mode = mode.to_string();
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

    fn assistant_tool_call(tool_call_id: &str, name: &str, arguments: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(String::new()),
            tool_calls: Some(vec![ChatToolCall {
                id: tool_call_id.to_string(),
                index: Some(0),
                function: ChatToolFunction {
                    name: name.to_string(),
                    arguments: arguments.to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
            }]),
            ..Default::default()
        }
    }

    fn default_settings() -> AdapterSettings {
        AdapterSettings {
            api_key: "test-key".to_string(),
            auth_token: String::new(),
            endpoint: "https://api.openai.com/v1/chat/completions".to_string(),
            extra_headers: Default::default(),
            model_name: "gpt-4.1".to_string(),
            supports_tools: true,
            supports_reasoning: true,
            reasoning_type: None,
            supports_temperature: true,
            supports_max_completion_tokens: false,
            eof_is_done: false,
            supports_web_search: false,
            supports_cache_control: false,
        }
    }

    fn assert_openai_tool_result_not_preceded_by_hidden_role(messages: Vec<ChatMessage>) {
        let req = crate::llm::canonical::LlmRequest::new("gpt-4.1".to_string(), messages);
        let body = OpenAiChatAdapter
            .build_http(&req, &default_settings())
            .unwrap()
            .body;
        let wire_messages = body["messages"].as_array().unwrap();
        let tool_index = wire_messages
            .iter()
            .position(|message| message["role"] == "tool")
            .expect("tool result missing from wire messages");
        let prior = &wire_messages[tool_index - 1];
        assert_eq!(prior["role"], "assistant");
        assert!(
            prior.get("tool_calls").is_some(),
            "prior message: {prior:?}"
        );
    }

    #[tokio::test]
    async fn set_goal_installs_goal_side_effect() {
        let (gcx, ccx) = ccx_for_session("agent").await;
        let mut tool = ToolSetGoal {
            config_path: String::new(),
        };
        let args = HashMap::from([("content".to_string(), json!("Ship the pond"))]);

        let (_, messages) = tool
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap();

        assert_eq!(
            tool_result_json(&messages),
            json!({"version": 1, "supersedes": null})
        );
        let session_arc = gcx
            .chat_sessions
            .read()
            .await
            .get(CHAT_ID)
            .cloned()
            .unwrap();
        let session = session_arc.lock().await;
        assert!(session.messages.is_empty());
        assert_eq!(session.post_tool_side_effects.len(), 2);
        assert_eq!(session.post_tool_side_effects[0].role, GOAL_ROLE);
        assert_eq!(
            content_text(&session.post_tool_side_effects[0]),
            "Ship the pond"
        );
        assert_eq!(
            session.post_tool_side_effects[0].extra["goal"]["active"],
            json!(true)
        );
        assert_eq!(session.post_tool_side_effects[1].role, EVENT_ROLE);
        assert_eq!(
            session.post_tool_side_effects[1].extra["event"],
            json!({
                "subkind": "system_notice",
                "source": "tool.set_goal",
                "payload": {"version": 1}
            })
        );
    }

    #[tokio::test]
    async fn rejects_when_goal_exists() {
        let (gcx, ccx) = ccx_for_session("agent").await;
        let session_arc = gcx
            .chat_sessions
            .read()
            .await
            .get(CHAT_ID)
            .cloned()
            .unwrap();
        {
            let mut session = session_arc.lock().await;
            crate::chat::goal_role::install_goal(
                &mut session,
                "agent",
                "existing",
                true,
                GoalBudget::default(),
            );
        }
        let mut tool = ToolSetGoal {
            config_path: String::new(),
        };
        let args = HashMap::from([("content".to_string(), json!("second"))]);

        let err = tool
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap_err();

        assert_eq!(err, "goal already exists; use update_goal");
        let session = session_arc.lock().await;
        assert!(session.post_tool_side_effects.is_empty());
    }

    #[tokio::test]
    async fn empty_content_rejected() {
        let (_gcx, ccx) = ccx_for_session("agent").await;
        let mut tool = ToolSetGoal {
            config_path: String::new(),
        };
        let args = HashMap::from([("content".to_string(), json!("  \n\t"))]);

        let err = tool
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap_err();

        assert_eq!(err, "argument `content` must be non-empty");
    }

    #[tokio::test]
    async fn set_goal_side_effects_are_after_tool_result() {
        let (gcx, ccx) = ccx_for_session("agent").await;
        let session_arc = gcx
            .chat_sessions
            .read()
            .await
            .get(CHAT_ID)
            .cloned()
            .unwrap();
        session_arc.lock().await.add_message(assistant_tool_call(
            "call-goal",
            "set_goal",
            r#"{"content":"new"}"#,
        ));

        let mut tool = ToolSetGoal {
            config_path: String::new(),
        };
        let args = HashMap::from([("content".to_string(), json!("new"))]);
        let (_, results) = tool
            .tool_execute(ccx, &"call-goal".to_string(), &args)
            .await
            .unwrap();

        let mut session = session_arc.lock().await;
        for message in results {
            let ContextEnum::ChatMessage(message) = message else {
                panic!("expected chat message")
            };
            session.add_message(message);
        }
        session.drain_post_tool_side_effects();

        let roles: Vec<_> = session
            .messages
            .iter()
            .map(|message| message.role.as_str())
            .collect();
        assert_eq!(roles, vec!["assistant", "tool", "goal", "event"]);
        assert_openai_tool_result_not_preceded_by_hidden_role(session.messages.clone());
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
                .any(|tool| tool.tool_description().name == "set_goal");
            assert!(has_tool, "{mode} should expose set_goal");
        }
        for mode in ["NO_TOOLS", "shell", "explore"] {
            let has_tool = get_tools_for_mode(gcx.clone(), mode, None)
                .await
                .into_iter()
                .any(|tool| tool.tool_description().name == "set_goal");
            assert!(!has_tool, "{mode} should not expose set_goal");
        }
    }
}
