use refact_llm::adapters::openai_responses::OpenAiResponsesAdapter;

use super::wire_soak_helpers::{
    assert_no_literal_role_strings_in_body, assert_no_plan_history_in_body,
    assert_plan_count_in_body, default_settings, generate_mixed_corpus, lower_body,
    three_plan_versions,
};

fn lower_openai_responses(
    messages: Vec<refact_core::chat_types::ChatMessage>,
) -> serde_json::Value {
    lower_body(
        &OpenAiResponsesAdapter,
        messages,
        default_settings("https://api.openai.com/v1/responses", "gpt-4.1"),
    )
}

#[test]
fn assert_no_literal_role_strings() {
    let body = lower_openai_responses(generate_mixed_corpus(13, 100));

    assert_no_literal_role_strings_in_body(&body);
}

#[test]
fn assert_all_plans_are_rendered_chronologically() {
    let body = lower_openai_responses(three_plan_versions());

    assert_plan_count_in_body(&body, 3);
    assert_no_plan_history_in_body(&body);
}

#[test]
fn assert_mixed_corpus_renders_each_plan_message() {
    let body = lower_openai_responses(generate_mixed_corpus(29, 100));

    assert_plan_count_in_body(&body, 5);
    assert_no_plan_history_in_body(&body);
}
