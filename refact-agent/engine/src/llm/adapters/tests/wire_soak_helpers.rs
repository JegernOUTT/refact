use refact_core::chat_types::{ChatContent, ChatMessage, ChatToolCall, ChatToolFunction};
use refact_llm::adapter::{AdapterSettings, LlmWireAdapter};
use refact_llm::canonical::LlmRequest;
use serde_json::{Value, json};

pub fn default_settings(endpoint: &str, model_name: &str) -> AdapterSettings {
    AdapterSettings {
        api_key: "test-key".to_string(),
        auth_token: String::new(),
        endpoint: endpoint.to_string(),
        extra_headers: Default::default(),
        model_name: model_name.to_string(),
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

pub fn lower_body(
    adapter: &dyn LlmWireAdapter,
    messages: Vec<ChatMessage>,
    settings: AdapterSettings,
) -> Value {
    let req = LlmRequest::new(settings.model_name.clone(), messages);
    adapter
        .build_http(&refact_privacy::testing::cleared(req), &settings)
        .unwrap()
        .body
}

pub fn chronological_plan_messages() -> Vec<ChatMessage> {
    vec![
        ChatMessage::new("system".to_string(), "Mode system prompt".to_string()),
        ChatMessage::new("user".to_string(), "user one".to_string()),
        plan_message("agent", 1, "first plan"),
        ChatMessage::new("user".to_string(), "user two".to_string()),
        plan_message("agent", 2, "second plan"),
        ChatMessage::new("user".to_string(), "user three".to_string()),
    ]
}

pub fn event_message(subkind: &str, source: &str, payload: Value, content: &str) -> ChatMessage {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "event".to_string(),
        json!({"subkind": subkind, "source": source, "payload": payload}),
    );
    ChatMessage {
        role: "event".to_string(),
        content: ChatContent::SimpleText(content.to_string()),
        extra,
        ..Default::default()
    }
}

pub fn plan_message(mode: &str, version: u64, content: &str) -> ChatMessage {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "plan".to_string(),
        json!({"mode": mode, "version": version}),
    );
    ChatMessage {
        role: "plan".to_string(),
        content: ChatContent::SimpleText(content.to_string()),
        extra,
        ..Default::default()
    }
}

fn tool_message(call_id: &str, index: usize, seed: u64) -> ChatMessage {
    ChatMessage {
        role: "tool".to_string(),
        content: ChatContent::SimpleText(format!("tool result {index} seed {seed}")),
        tool_call_id: call_id.to_string(),
        ..Default::default()
    }
}

pub fn tool_ordering_messages(hidden_role: &str) -> Vec<ChatMessage> {
    let hidden = match hidden_role {
        "plan" => plan_message("agent", 1, "plan after result"),
        "event" => event_message(
            "tick",
            "tool.sleep",
            json!({"elapsed_ms": 50, "remaining_ms": 50}),
            "tick",
        ),
        other => panic!("unsupported hidden role {other}"),
    };
    vec![
        ChatMessage::new("user".to_string(), "run tool".to_string()),
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(String::new()),
            tool_calls: Some(vec![ChatToolCall {
                id: "call_order".to_string(),
                index: Some(0),
                function: ChatToolFunction {
                    name: "set_plan".to_string(),
                    arguments: r#"{"content":"plan"}"#.to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
                started_at_ms: None,
                completed_at_ms: None,
            }]),
            ..Default::default()
        },
        tool_message("call_order", 0, 0),
        hidden,
    ]
}

pub fn assert_tool_result_immediately_follows_tool_call(body: &Value) {
    let wire_messages = body["messages"].as_array().unwrap();
    let tool_index = wire_messages
        .iter()
        .position(|message| message["role"] == "tool")
        .expect("tool result missing from wire body");
    let prior = &wire_messages[tool_index - 1];
    assert_eq!(prior["role"], "assistant");
    assert!(
        prior.get("tool_calls").is_some(),
        "prior message: {prior:?}"
    );
}
