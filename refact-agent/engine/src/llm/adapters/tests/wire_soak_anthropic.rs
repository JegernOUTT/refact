use refact_core::chat_types::{ChatContent, ChatMessage};
use refact_llm::adapters::anthropic::AnthropicAdapter;

use super::wire_soak_helpers::{
    assert_no_literal_role_strings_in_body, assert_no_plan_history_in_body,
    assert_plan_count_in_body, default_settings, event_message, generate_mixed_corpus, lower_body,
    three_plan_versions,
};

fn lower_anthropic(messages: Vec<ChatMessage>) -> serde_json::Value {
    lower_body(
        &AnthropicAdapter,
        messages,
        default_settings("https://api.anthropic.com/v1/messages", "claude-sonnet-4-5"),
    )
}

#[test]
fn assert_no_literal_role_strings() {
    let body = lower_anthropic(generate_mixed_corpus(13, 100));

    assert_no_literal_role_strings_in_body(&body);
}

#[test]
fn assert_all_plans_are_rendered_chronologically() {
    let body = lower_anthropic(three_plan_versions());

    assert_plan_count_in_body(&body, 3);
    assert_no_plan_history_in_body(&body);
}

#[test]
fn assert_mixed_corpus_renders_each_plan_message() {
    let body = lower_anthropic(generate_mixed_corpus(29, 100));

    assert_plan_count_in_body(&body, 5);
    assert_no_plan_history_in_body(&body);
}

#[test]
fn thinking_block_ordering_preserved_across_event_injections() {
    let body = lower_anthropic(vec![
        ChatMessage::new("user".to_string(), "Solve this".to_string()),
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("First answer".to_string()),
            thinking_blocks: Some(vec![
                serde_json::json!({
                    "type": "thinking",
                    "index": 0,
                    "thinking": "first thought",
                    "signature": "sig_first"
                }),
                serde_json::json!({
                    "type": "thinking",
                    "index": 1,
                    "thinking": "second thought",
                    "signature": "sig_second"
                }),
            ]),
            ..Default::default()
        },
        event_message(
            "process_completed",
            "runtime",
            serde_json::json!({"process_id": "proc_1", "exit_code": 0}),
            "process finished",
        ),
        ChatMessage::new("assistant".to_string(), "Second answer".to_string()),
    ]);

    let messages = body["messages"].as_array().unwrap();
    let first_assistant = messages
        .iter()
        .find(|message| message["role"] == "assistant")
        .unwrap();
    let content = first_assistant["content"].as_array().unwrap();

    assert_eq!(content[0]["type"], "thinking");
    assert_eq!(content[0]["thinking"], "first thought");
    assert_eq!(content[1]["type"], "thinking");
    assert_eq!(content[1]["thinking"], "second thought");
    assert_eq!(content[2]["type"], "text");
    assert_eq!(content[2]["text"], "First answer");
}
