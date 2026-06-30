use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::goal_role;
use crate::chat::internal_roles;
use crate::chat::types::ChatSession;
use crate::tools::tools_description::{
    json_schema_from_params, Tool, ToolDesc, ToolSource, ToolSourceType,
};

pub struct ToolUpdateGoal {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolUpdateGoal {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "update_goal".to_string(),
            display_name: "Update Goal".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Append an incremental update to the current goal. Use when the goal evolves; it does not rewrite the original goal.".to_string(),
            input_schema: json_schema_from_params(
                &[("note", "string", "Goal update note. Required.")],
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

        let (gcx, chat_id) = {
            let cgcx = ccx.lock().await;
            (cgcx.app.gcx.clone(), cgcx.chat_id.clone())
        };
        let session_arc = {
            let sessions = gcx.chat_sessions.read().await;
            sessions.get(&chat_id).cloned()
        }
        .ok_or_else(|| format!("chat session `{chat_id}` not found"))?;

        let (seq, result_truncation) = {
            let mut session = session_arc.lock().await;
            if !has_base_goal_including_queued(&session) {
                return Err("no goal to update; call set_goal first".to_string());
            }
            let seq = goal_delta_count_including_queued(&session) + 1;
            let (delta, result_truncation) = internal_roles::goal_delta_with_truncation(
                "tool.update_goal",
                json!({"seq": seq}),
                note,
            );
            session.queue_post_tool_side_effect(delta);
            (seq, result_truncation)
        };

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(
                    update_goal_tool_result(seq, result_truncation).to_string(),
                ),
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }
}

fn update_goal_tool_result(
    seq: usize,
    truncation: Option<internal_roles::PlanDeltaTruncation>,
) -> Value {
    let Some(truncation) = truncation else {
        return json!({"seq": seq, "truncated": false});
    };
    json!({
        "seq": seq,
        "truncated": true,
        "original_chars": truncation.original_chars,
        "kept_chars": truncation.kept_chars,
    })
}

fn has_base_goal_including_queued(session: &ChatSession) -> bool {
    goal_role::current_base_goal(session).is_some()
        || session
            .post_tool_side_effects
            .iter()
            .any(|message| message.role == internal_roles::GOAL_ROLE)
}

fn goal_delta_count_including_queued(session: &ChatSession) -> usize {
    goal_role::goal_delta_events(session).len()
        + session
            .post_tool_side_effects
            .iter()
            .filter(|message| is_goal_delta(message))
            .count()
}

fn is_goal_delta(message: &ChatMessage) -> bool {
    message.role == internal_roles::EVENT_ROLE
        && message
            .extra
            .get("event")
            .and_then(|event| event.get("subkind"))
            .and_then(|subkind| subkind.as_str())
            == Some("goal_delta")
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
    use refact_chat_api::GoalBudget;
    use crate::app_state::AppState;
    use crate::call_validation::{ChatToolCall, ChatToolFunction};
    use crate::chat::internal_roles::{EVENT_ROLE, GOAL_ROLE};
    use crate::llm::adapter::{AdapterSettings, LlmWireAdapter};
    use crate::llm::adapters::openai_chat::OpenAiChatAdapter;
    use crate::tools::tools_list::get_tools_for_mode;

    const CHAT_ID: &str = "update-goal-chat";

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

    fn goal_delta_payload(message: &ChatMessage) -> &Value {
        &message.extra["event"]["payload"]
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
    async fn appends_goal_delta_event() {
        let mut session = ChatSession::new(CHAT_ID.to_string());
        crate::chat::goal_role::install_goal(
            &mut session,
            "agent",
            "base goal",
            true,
            GoalBudget::default(),
        );
        session.add_message(internal_roles::goal_delta(
            "tool.update_goal",
            json!({"seq": 1}),
            "first note",
        ));
        let (gcx, ccx) = ccx_for_session(session).await;
        let mut tool = ToolUpdateGoal {
            config_path: String::new(),
        };
        let args = HashMap::from([("note".to_string(), json!("second note"))]);

        let (_, messages) = tool
            .tool_execute(ccx.clone(), &"call".to_string(), &args)
            .await
            .unwrap();

        assert_eq!(
            tool_result_json(&messages),
            json!({"seq": 2, "truncated": false})
        );
        let session_arc = gcx
            .chat_sessions
            .read()
            .await
            .get(CHAT_ID)
            .cloned()
            .unwrap();
        let session = session_arc.lock().await;
        assert_eq!(session.post_tool_side_effects.len(), 1);
        assert_eq!(session.post_tool_side_effects[0].role, EVENT_ROLE);
        assert_eq!(
            content_text(&session.post_tool_side_effects[0]),
            "second note"
        );
        assert_eq!(
            session.post_tool_side_effects[0].extra["event"],
            json!({
                "subkind": "goal_delta",
                "source": "tool.update_goal",
                "payload": {"seq": 2}
            })
        );
    }

    #[tokio::test]
    async fn oversized_note_is_truncated_with_metadata() {
        let mut session = ChatSession::new(CHAT_ID.to_string());
        crate::chat::goal_role::install_goal(
            &mut session,
            "agent",
            "base goal",
            true,
            GoalBudget::default(),
        );
        let (gcx, ccx) = ccx_for_session(session).await;
        let mut tool = ToolUpdateGoal {
            config_path: String::new(),
        };
        let note = "a".repeat(internal_roles::MAX_GOAL_DELTA_CHARS + 100);
        let original_chars = note.chars().count();
        let args = HashMap::from([("note".to_string(), json!(note))]);

        let (_, messages) = tool
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap();

        let result = tool_result_json(&messages);
        assert_eq!(result["seq"], json!(1));
        assert_eq!(result["truncated"], json!(true));
        assert_eq!(result["original_chars"], json!(original_chars));
        let session_arc = gcx
            .chat_sessions
            .read()
            .await
            .get(CHAT_ID)
            .cloned()
            .unwrap();
        let session = session_arc.lock().await;
        let delta = &session.post_tool_side_effects[0];
        let content = content_text(delta);
        assert!(content.chars().count() <= internal_roles::MAX_GOAL_DELTA_CHARS);
        assert!(content.contains("[truncated:"));
        assert_eq!(goal_delta_payload(delta)["truncated"], json!(true));
        let kept_chars = goal_delta_payload(delta)["kept_chars"].as_u64().unwrap() as usize;
        assert_eq!(result["kept_chars"], json!(kept_chars));
    }

    #[tokio::test]
    async fn rejects_when_no_goal() {
        let session = ChatSession::new(CHAT_ID.to_string());
        let (_gcx, ccx) = ccx_for_session(session).await;
        let mut tool = ToolUpdateGoal {
            config_path: String::new(),
        };
        let args = HashMap::from([("note".to_string(), json!("new direction"))]);

        let err = tool
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap_err();

        assert_eq!(err, "no goal to update; call set_goal first");
    }

    #[tokio::test]
    async fn missing_note_errors() {
        let session = ChatSession::new(CHAT_ID.to_string());
        let (_gcx, ccx) = ccx_for_session(session).await;
        let mut tool = ToolUpdateGoal {
            config_path: String::new(),
        };

        let err = tool
            .tool_execute(ccx, &"call".to_string(), &HashMap::new())
            .await
            .unwrap_err();

        assert_eq!(err, "argument `note` is missing");
    }

    #[tokio::test]
    async fn whitespace_only_note_errors() {
        let session = ChatSession::new(CHAT_ID.to_string());
        let (_gcx, ccx) = ccx_for_session(session).await;
        let mut tool = ToolUpdateGoal {
            config_path: String::new(),
        };
        let args = HashMap::from([("note".to_string(), json!("  \n\t"))]);

        let err = tool
            .tool_execute(ccx, &"call".to_string(), &args)
            .await
            .unwrap_err();

        assert_eq!(err, "argument `note` must be non-empty");
    }

    #[tokio::test]
    async fn update_goal_side_effects_are_after_tool_result() {
        let mut session = ChatSession::new(CHAT_ID.to_string());
        crate::chat::goal_role::install_goal(
            &mut session,
            "agent",
            "base goal",
            true,
            GoalBudget::default(),
        );
        session.add_message(assistant_tool_call(
            "call-goal",
            "update_goal",
            r#"{"note":"new"}"#,
        ));
        let (gcx, ccx) = ccx_for_session(session).await;
        let session_arc = gcx
            .chat_sessions
            .read()
            .await
            .get(CHAT_ID)
            .cloned()
            .unwrap();

        let mut tool = ToolUpdateGoal {
            config_path: String::new(),
        };
        let args = HashMap::from([("note".to_string(), json!("new"))]);
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
        assert_eq!(roles, vec!["goal", "assistant", "tool", "event"]);
        assert_eq!(
            session.messages[3].extra["event"]["subkind"],
            json!("goal_delta")
        );
        assert_openai_tool_result_not_preceded_by_hidden_role(session.messages.clone());
    }

    #[test]
    fn queued_base_goal_allows_update() {
        let mut session = ChatSession::new(CHAT_ID.to_string());
        session.queue_post_tool_side_effect(internal_roles::goal(
            "agent",
            1,
            "base",
            None,
            true,
            GoalBudget::default(),
        ));

        assert!(has_base_goal_including_queued(&session));
        assert_eq!(session.post_tool_side_effects[0].role, GOAL_ROLE);
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
                .any(|tool| tool.tool_description().name == "update_goal");
            assert!(has_tool, "{mode} should expose update_goal");
        }
        for mode in ["NO_TOOLS", "shell", "explore"] {
            let has_tool = get_tools_for_mode(gcx.clone(), mode, None)
                .await
                .into_iter()
                .any(|tool| tool.tool_description().name == "update_goal");
            assert!(!has_tool, "{mode} should not expose update_goal");
        }
    }
}
