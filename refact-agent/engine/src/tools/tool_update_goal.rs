use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::goal_role;
use crate::chat::internal_roles;
use crate::chat::types::{ChatSession, PendingDelivery, PushMode};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

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
            description: "Update the goal outcome, its execution plan, or both atomically. Provide note, plan.content to install an absent plan, or plan.note to append a plan update. Existing approved bases are not rewritten.".to_string(),
            input_schema: json!({"type":"object", "properties": {
                "note":{"type":"string"}, "push":PushMode::schema(),
                "plan":{"oneOf":[
                    {"type":"object","properties":{"content":{"type":"string"}},"required":["content"],"additionalProperties":false},
                    {"type":"object","properties":{"note":{"type":"string"}},"required":["note"],"additionalProperties":false}
                ]}},"additionalProperties":false}),
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
        let push = PushMode::from_args(args)?;
        let note = args
            .get("note")
            .map(|_| string_arg(args, "note"))
            .transpose()?;
        let plan: Option<refact_chat_api::GoalPlanMutation> = args
            .get("plan")
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("argument `plan` is malformed: {e}"))
            })
            .transpose()?;
        if note.is_none() && plan.is_none() {
            return Err("provide at least one of note or plan".into());
        }
        if note.as_ref().is_some_and(|n| n.trim().is_empty()) {
            return Err("argument `note` must be non-empty".into());
        }

        let (gcx, chat_id) = {
            let cgcx = ccx.lock().await;
            (cgcx.app.gcx.clone(), cgcx.chat_id.clone())
        };
        let policy = crate::chat::verify_cmd::VerifyCommandPolicy::load(gcx.clone()).await;
        let session_arc = {
            let sessions = gcx.chat_sessions.read().await;
            sessions.get(&chat_id).cloned()
        }
        .ok_or_else(|| format!("chat session `{chat_id}` not found"))?;

        crate::tasks::goal::hydrate_session_task_goal_for_mutation(gcx.clone(), &session_arc)
            .await?;
        let (seq, result_truncation, advisory) = {
            let mut session = session_arc.lock().await;
            let messages = goal_role::prepare_goal_update(
                &session,
                note.as_deref(),
                plan.as_ref(),
                "tool.update_goal",
            )?;
            let seq = goal_delta_count_including_queued(&session)? + usize::from(note.is_some());
            let result_truncation = messages.iter().find_map(|m| {
                let payload = m.extra.get("event")?.get("payload")?;
                payload["truncated"].as_bool().filter(|v| *v).map(|_| {
                    internal_roles::PlanDeltaTruncation {
                        original_chars: payload["original_chars"].as_u64().unwrap_or_default()
                            as usize,
                        kept_chars: payload["kept_chars"].as_u64().unwrap_or_default() as usize,
                    }
                })
            });
            let mut delivery = PendingDelivery::new(messages, push, "tool.update_goal", false);
            delivery.after_tool_call_id = Some(tool_call_id.clone());
            session.enqueue_delivery(delivery)?;
            let advisory = crate::chat::evidence::unverifiable_goal_criteria(&session, &policy);
            (seq, result_truncation, advisory)
        };
        if let Err(error) =
            crate::tasks::goal::commit_session_task_goal(gcx.clone(), &session_arc).await
        {
            let mut session = session_arc.lock().await;
            session
                .pending_deliveries
                .retain(|d| d.after_tool_call_id.as_deref() != Some(tool_call_id.as_str()));
            drop(session);
            let _ = crate::tasks::goal::hydrate_session_task_goal(gcx, &session_arc).await;
            return Err(error);
        }
        let mut result = update_goal_tool_result(seq, result_truncation);
        if plan.is_some() {
            let session = session_arc.lock().await;
            let projection = session.try_accepted_control_projection()?;
            if let Some(plan) = goal_role::current_goal_plan(
                &projection.messages,
                crate::runtime_settings::plan_projection_limits(),
                false,
            ) {
                result["plan"] =
                    json!({"seq":plan.delta_count,"truncated":result_truncation.is_some()});
            }
        }
        let mut result = result.to_string();
        if !advisory.is_empty() {
            result.push_str(&format!("\n\nAdvisory:\n- {}", advisory.join("\n- ")));
        }

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result),
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

#[cfg(test)]
fn has_base_goal_including_queued(session: &ChatSession) -> Result<bool, String> {
    Ok(goal_role::current_base_goal(&session.try_accepted_control_projection()?).is_some())
}

fn goal_delta_count_including_queued(session: &ChatSession) -> Result<usize, String> {
    Ok(goal_role::effective_goal_delta_count(
        &session.try_accepted_control_projection()?,
    ))
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

    fn delivery_messages(session: &ChatSession) -> Vec<ChatMessage> {
        session
            .messages
            .iter()
            .filter(|message| message.extra.contains_key("delivery"))
            .chain(
                session
                    .pending_deliveries
                    .iter()
                    .flat_map(|delivery| delivery.messages.iter()),
            )
            .cloned()
            .collect()
    }

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
                started_at_ms: None,
                completed_at_ms: None,
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
            .build_http(&refact_privacy::testing::cleared(req), &default_settings())
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
        assert_eq!(delivery_messages(&session).len(), 1);
        assert_eq!(delivery_messages(&session)[0].role, EVENT_ROLE);
        assert_eq!(content_text(&delivery_messages(&session)[0]), "second note");
        let event = delivery_messages(&session)[0].extra["event"].clone();
        assert_eq!(event["subkind"], "goal_delta");
        assert_eq!(event["source"], "tool.update_goal");
        assert_eq!(event["payload"]["seq"], 2);
        assert_eq!(
            event["payload"]["goal_id"],
            goal_role::current_base_goal(&session).unwrap().message_id
        );
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

        assert_eq!(err, "provide at least one of note or plan");
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
        session.drain_pending_deliveries();

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
}
