use refact_core::chat_types::ChatMessage;
use refact_llm::adapters::openai_chat::OpenAiChatAdapter;

use super::wire_soak_helpers::{
    assert_no_literal_role_strings_in_body, assert_no_plan_history_in_body,
    assert_plan_count_in_body, chronological_plan_messages, default_settings,
    generate_mixed_corpus, lower_body, three_plan_versions,
};

fn lower_openai_chat(messages: Vec<ChatMessage>) -> serde_json::Value {
    lower_body(
        &OpenAiChatAdapter,
        messages,
        default_settings("https://api.openai.com/v1/chat/completions", "gpt-4.1"),
    )
}

#[test]
fn snapshot_chronological_plan_wire_body() {
    let body = lower_openai_chat(chronological_plan_messages());

    insta::assert_snapshot!(serde_json::to_string_pretty(&body).unwrap(), @r###"
{
  "model": "gpt-4.1",
  "messages": [
    {
      "role": "system",
      "content": "Mode system prompt"
    },
    {
      "role": "user",
      "content": "user one"
    },
    {
      "role": "user",
      "content": "<plan mode=\"agent\" version=\"1\">\nfirst plan\n</plan>"
    },
    {
      "role": "user",
      "content": "user two"
    },
    {
      "role": "user",
      "content": "<plan mode=\"agent\" version=\"2\">\nsecond plan\n</plan>"
    },
    {
      "role": "user",
      "content": "user three"
    }
  ],
  "stream": true,
  "max_tokens": 4096,
  "stream_options": {
    "include_usage": true
  },
  "temperature": 1.0
}
"###);
}

#[test]
fn prefix_before_original_plan_is_stable_after_later_plan_update() {
    let first_body = lower_openai_chat(vec![
        ChatMessage::new("system".to_string(), "Mode system prompt".to_string()),
        ChatMessage::new("user".to_string(), "user one".to_string()),
        super::wire_soak_helpers::plan_message("agent", 1, "first plan"),
        ChatMessage::new("user".to_string(), "user two".to_string()),
    ]);
    let second_body = lower_openai_chat(chronological_plan_messages());

    let first_messages = first_body["messages"].as_array().unwrap();
    let second_messages = second_body["messages"].as_array().unwrap();

    assert_eq!(
        serde_json::to_vec(&first_messages[..3]).unwrap(),
        serde_json::to_vec(&second_messages[..3]).unwrap()
    );
}

#[test]
fn assert_no_literal_role_strings() {
    let body = lower_openai_chat(generate_mixed_corpus(13, 100));

    assert_no_literal_role_strings_in_body(&body);
}

#[test]
fn assert_all_plans_are_rendered_chronologically() {
    let body = lower_openai_chat(three_plan_versions());

    assert_plan_count_in_body(&body, 3);
    assert_no_plan_history_in_body(&body);
}

#[test]
fn assert_mixed_corpus_renders_each_plan_message() {
    let body = lower_openai_chat(generate_mixed_corpus(29, 100));

    assert_plan_count_in_body(&body, 5);
    assert_no_plan_history_in_body(&body);
}
