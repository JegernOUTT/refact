use std::collections::HashSet;

use refact_chat_history::compression_exemption::{event_source, event_subkind};
use refact_chat_history::history_limit::{tier0_deterministic_compact_with, CompactAggression};
use refact_core::chat_types::ChatMessage;

const PRESERVE_LAST_N: usize = 10;
const LARGE_CORPUS: &str = include_str!("fixtures/large_corpus.json");

fn large_corpus() -> Vec<ChatMessage> {
    serde_json::from_str(LARGE_CORPUS).expect("large corpus fixture should deserialize")
}

fn compressed_corpus(aggression: CompactAggression) -> Vec<ChatMessage> {
    let mut messages = large_corpus();
    tier0_deterministic_compact_with(&mut messages, PRESERVE_LAST_N, aggression);
    messages
}

fn compressed_corpora() -> [(CompactAggression, Vec<ChatMessage>); 2] {
    [
        (
            CompactAggression::Standard,
            compressed_corpus(CompactAggression::Standard),
        ),
        (
            CompactAggression::Aggressive,
            compressed_corpus(CompactAggression::Aggressive),
        ),
    ]
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

fn event_jsons_for_source(messages: &[ChatMessage], subkind: &str, source: &str) -> Vec<String> {
    messages
        .iter()
        .filter(|message| {
            message.role == "event"
                && event_subkind(message) == Some(subkind)
                && event_source(message) == source
        })
        .map(|message| serde_json::to_string(message).expect("event should serialize"))
        .collect()
}

fn summary_texts_for_source(
    messages: &[ChatMessage],
    summarized_subkind: &str,
    source: &str,
) -> Vec<String> {
    messages
        .iter()
        .filter(|message| {
            if message.role != "event" || event_subkind(message) != Some("summarization_marker") {
                return false;
            }
            let Some(event) = message.extra.get("event") else {
                return false;
            };
            event.get("source").and_then(|value| value.as_str()) == Some(source)
                && event
                    .get("payload")
                    .and_then(|payload| payload.get("summarized_subkind"))
                    .and_then(|value| value.as_str())
                    == Some(summarized_subkind)
        })
        .map(|message| message.content.content_text_only())
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
fn large_corpus_preserves_plans() {
    let original_plan_jsons = plan_jsons(&large_corpus());
    assert_eq!(original_plan_jsons.len(), 5);

    for (aggression, messages) in compressed_corpora() {
        assert_eq!(
            plan_jsons(&messages),
            original_plan_jsons,
            "plans changed under {aggression:?}"
        );
    }
}

#[test]
fn large_corpus_drops_old_ticks() {
    let original = large_corpus();
    let recent_message_ids: HashSet<&str> = original
        .iter()
        .rev()
        .take(PRESERVE_LAST_N)
        .map(|message| message.message_id.as_str())
        .collect();
    let expected_recent_tick_ids: Vec<&str> = original
        .iter()
        .filter(|message| {
            message.role == "event"
                && event_subkind(message) == Some("tick")
                && recent_message_ids.contains(message.message_id.as_str())
        })
        .map(|message| message.message_id.as_str())
        .collect();

    let messages = compressed_corpus(CompactAggression::Standard);
    let actual_tick_ids: Vec<&str> = messages
        .iter()
        .filter(|message| message.role == "event" && event_subkind(message) == Some("tick"))
        .map(|message| message.message_id.as_str())
        .collect();

    assert_eq!(actual_tick_ids, expected_recent_tick_ids);
}

#[test]
fn large_corpus_keeps_recent_process_completed_per_source() {
    let messages = compressed_corpus(CompactAggression::Standard);

    for source in ["exec:build", "exec:test"] {
        let process_event_ids = event_ids_for_source(&messages, "process_completed", source);
        assert!(
            process_event_ids.len() <= 3,
            "too many process_completed events for {source}"
        );
        assert_eq!(
            process_event_ids,
            event_ids_for_source(&large_corpus(), "process_completed", source)
                .into_iter()
                .rev()
                .take(3)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>(),
            "did not keep the latest process_completed events for {source}"
        );
        assert_eq!(
            summary_texts_for_source(&messages, "process_completed", source),
            vec![format!(
                "<event-history source=\"{source}\">22 earlier process_completed events</event-history>"
            )],
            "missing process_completed summary for {source}"
        );
    }
}

#[test]
fn large_corpus_preserves_anchors_under_aggressive() {
    let original = large_corpus();
    let messages = compressed_corpus(CompactAggression::Aggressive);

    let anchor_sources = [
        ("summarization_marker", "anchor:summarization"),
        ("system_notice", "anchor:system"),
        ("cancellation_note", "anchor:cancellation"),
    ];

    for (subkind, source) in anchor_sources {
        assert_eq!(
            event_jsons_for_source(&messages, subkind, source),
            event_jsons_for_source(&original, subkind, source),
            "anchor events changed for {subkind}"
        );
    }
}

#[test]
fn large_corpus_no_assistant_content_loss() {
    let original_assistant_texts = assistant_texts(&large_corpus());
    assert!(!original_assistant_texts.is_empty());

    for (aggression, messages) in compressed_corpora() {
        assert_eq!(
            assistant_texts(&messages),
            original_assistant_texts,
            "assistant content changed under {aggression:?}"
        );
    }
}
