use std::path::{Path, PathBuf};

use refact_chat_api::CompressionReason;

fn gui_subscription_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../gui/src/services/refact/chatSubscription.ts")
}

fn gui_reducer_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../gui/src/features/Chat/Thread/reducer.ts")
}

fn string_literals_between(text: &str, start: &str, terminator: &str) -> Vec<String> {
    let rest = text
        .split_once(start)
        .unwrap_or_else(|| panic!("GUI compression contract no longer declares `{start}`"))
        .1;
    let block = rest
        .split_once(terminator)
        .unwrap_or_else(|| panic!("`{start}` is not terminated by `{terminator}`"))
        .0;
    block
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

#[test]
fn compression_reason_names_reach_gui_union_and_normalizer() {
    for reason in CompressionReason::ALL {
        match reason {
            CompressionReason::AutoCompactDisabled
            | CompressionReason::SessionCompactionDisabled
            | CompressionReason::MaxAttemptsReached
            | CompressionReason::PendingToolCalls
            | CompressionReason::NoEligibleSegment
            | CompressionReason::EffectiveContextUnknown
            | CompressionReason::ProviderLengthStop
            | CompressionReason::ContextLengthStop
            | CompressionReason::PressureLow
            | CompressionReason::NoSummaryModel
            | CompressionReason::InputTooLarge
            | CompressionReason::TransientFailure
            | CompressionReason::SourceChanged
            | CompressionReason::InsufficientSavings
            | CompressionReason::PreCompactDenied => {}
        }
        let name = serde_json::to_value(reason)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        let subscription = std::fs::read_to_string(gui_subscription_path()).unwrap();
        let union_names =
            string_literals_between(&subscription, "export type CompressionReason =", ";");
        assert!(
            union_names.contains(&name),
            "GUI CompressionReason union is missing `{name}`"
        );

        let reducer = std::fs::read_to_string(gui_reducer_path()).unwrap();
        let normalizer_names =
            string_literals_between(&reducer, "function normalizeCompressionReason(", "\n}\n");
        assert!(
            normalizer_names.contains(&name),
            "GUI compression reason normalizer is missing `{name}`"
        );
    }
}
