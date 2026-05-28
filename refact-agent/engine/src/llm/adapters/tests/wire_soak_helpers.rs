use refact_core::chat_types::{ChatContent, ChatMessage};
use refact_llm::adapter::{AdapterSettings, LlmWireAdapter};
use refact_llm::canonical::LlmRequest;
use serde_json::{Value, json};

pub fn generate_mixed_corpus(seed: u64, len: usize) -> Vec<ChatMessage> {
    let mut base = Vec::with_capacity(100);

    for idx in 0..30 {
        base.push(ChatMessage::new(
            "user".to_string(),
            format!("user message {idx} seed {seed}"),
        ));
    }
    for idx in 0..30 {
        base.push(ChatMessage::new(
            "assistant".to_string(),
            format!("assistant message {idx} seed {seed}"),
        ));
    }
    for idx in 0..10 {
        base.push(tool_message(&format!("call_{seed}_{idx}"), idx, seed));
    }
    for idx in 0..5 {
        base.push(plan_message(
            "agent",
            (idx + 1) as u64,
            &format!("plan version {} seed {seed}", idx + 1),
        ));
    }
    for idx in 0..25 {
        let subkind = match idx % 5 {
            0 => "mode_switch",
            1 => "tool_decision",
            2 => "process_completed",
            3 => "system_note",
            _ => "subchat_update",
        };
        base.push(event_message(
            subkind,
            match idx % 3 {
                0 => "mode",
                1 => "tools",
                _ => "runtime",
            },
            json!({"seed": seed, "index": idx, "subkind": subkind}),
            &format!("event {idx} seed {seed}"),
        ));
    }

    if !base.is_empty() {
        let offset = (seed as usize) % base.len();
        base.rotate_left(offset);
    }

    let mut messages = Vec::with_capacity(len);
    while messages.len() < len {
        messages.extend(base.iter().cloned());
    }
    messages.truncate(len);
    messages
}

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
    adapter.build_http(&req, &settings).unwrap().body
}

pub fn assert_no_literal_role_strings_in_body(body: &Value) {
    let serialized = body.to_string();
    assert!(!serialized.contains("\"role\":\"event\""));
    assert!(!serialized.contains("\"role\":\"plan\""));
    assert!(!serialized.contains("\"role\": \"event\""));
    assert!(!serialized.contains("\"role\": \"plan\""));
}

pub fn assert_plan_count_in_body(body: &Value, expected: usize) {
    assert_eq!(body.to_string().matches("<plan mode=").count(), expected);
}

pub fn assert_no_plan_history_in_body(body: &Value) {
    assert!(!body.to_string().contains("<plan-history>"));
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

pub fn three_plan_versions() -> Vec<ChatMessage> {
    vec![
        ChatMessage::new("user".to_string(), "first user".to_string()),
        plan_message("agent", 1, "first plan"),
        ChatMessage::new("user".to_string(), "second user".to_string()),
        plan_message("agent", 2, "second plan"),
        event_message(
            "mode_switch",
            "mode",
            json!({"next": "agent"}),
            "mode changed",
        ),
        plan_message("agent", 3, "third plan"),
        ChatMessage::new("user".to_string(), "third user".to_string()),
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
