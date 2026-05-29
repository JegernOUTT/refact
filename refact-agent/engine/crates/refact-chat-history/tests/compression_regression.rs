use refact_chat_history::compression_exemption::{
    event_source, event_subkind, exemption_for, CompressionExemption,
};
use refact_core::chat_types::ChatMessage;

const LARGE_CORPUS: &str = include_str!("fixtures/large_corpus.json");

fn large_corpus() -> Vec<ChatMessage> {
    serde_json::from_str(LARGE_CORPUS).expect("large corpus fixture should deserialize")
}

fn plan_jsons(messages: &[ChatMessage]) -> Vec<String> {
    messages
        .iter()
        .filter(|message| message.role == "plan")
        .map(|message| serde_json::to_string(message).expect("plan should serialize"))
        .collect()
}

fn event_ids_for_source(messages: &[ChatMessage], subkind: &str, source: &str) -> Vec<String> {
    messages
        .iter()
        .filter(|message| {
            message.role == "event"
                && event_subkind(message) == Some(subkind)
                && event_source(message) == source
        })
        .map(|message| message.message_id.clone())
        .collect()
}

fn assistant_texts(messages: &[ChatMessage]) -> Vec<String> {
    messages
        .iter()
        .filter(|message| message.role == "assistant")
        .map(|message| message.content.content_text_only())
        .collect()
}

#[test]
fn large_corpus_preserves_plans_without_history_limit_rewrites() {
    let messages = large_corpus();
    let plans = plan_jsons(&messages);

    assert_eq!(plans.len(), 5);
    for message in messages.iter().filter(|message| message.role == "plan") {
        assert_eq!(exemption_for(message), CompressionExemption::Never);
    }
}

#[test]
fn large_corpus_event_exemption_contract_is_stable() {
    let messages = large_corpus();

    for source in ["exec:build", "exec:test"] {
        assert!(!event_ids_for_source(&messages, "process_completed", source).is_empty());
    }

    for subkind in ["summarization_marker", "system_notice", "cancellation_note"] {
        let anchors: Vec<&ChatMessage> = messages
            .iter()
            .filter(|message| message.role == "event" && event_subkind(message) == Some(subkind))
            .collect();
        assert!(!anchors.is_empty(), "fixture should contain {subkind}");
        assert!(anchors
            .iter()
            .all(|message| exemption_for(message) == CompressionExemption::PreserveAnchor));
    }
}

#[test]
fn large_corpus_assistant_content_available_for_segment_summarizer() {
    let original_assistant_texts = assistant_texts(&large_corpus());

    assert!(!original_assistant_texts.is_empty());
    assert!(original_assistant_texts
        .iter()
        .any(|text| !text.trim().is_empty()));
}
