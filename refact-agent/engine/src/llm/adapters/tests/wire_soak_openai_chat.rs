use refact_llm::adapters::openai_chat::OpenAiChatAdapter;

use super::wire_soak_helpers::{
    assert_multiple_plan_history_in_body, assert_no_literal_role_strings_in_body,
    assert_plan_count_in_body, default_settings, generate_mixed_corpus, lower_body,
    three_plan_versions,
};

fn lower_openai_chat(messages: Vec<refact_core::chat_types::ChatMessage>) -> serde_json::Value {
    lower_body(
        &OpenAiChatAdapter,
        messages,
        default_settings("https://api.openai.com/v1/chat/completions", "gpt-4.1"),
    )
}

#[test]
fn assert_no_literal_role_strings() {
    let body = lower_openai_chat(generate_mixed_corpus(13, 100));

    assert_no_literal_role_strings_in_body(&body);
}

#[test]
fn assert_plan_appears_at_most_once() {
    let body = lower_openai_chat(generate_mixed_corpus(29, 100));

    assert_plan_count_in_body(&body, 1);
}

#[test]
fn assert_plan_history_appears_when_multiple() {
    let body = lower_openai_chat(three_plan_versions());

    assert_multiple_plan_history_in_body(&body);
}
