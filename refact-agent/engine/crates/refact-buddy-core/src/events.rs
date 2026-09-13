use serde::{Deserialize, Serialize};
use crate::settings::BuddySettings;
use crate::types::*;
use crate::operator_events::OperatorSignal;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum BuddyEvent<Diagnostic = serde_json::Value> {
    StateUpdated {
        state: BuddyState,
    },
    ActivityAdded {
        activity: BuddyActivity,
    },
    SuggestionAdded {
        suggestion: BuddySuggestion,
    },
    SuggestionDismissed {
        suggestion_id: String,
    },
    SettingsChanged {
        settings: BuddySettings,
    },
    DiagnosticAdded {
        diagnostic: Diagnostic,
    },
    RuntimeEvent {
        event: BuddyRuntimeEvent,
    },
    OperatorSignal {
        signal: OperatorSignal,
    },
    SpeechUpdated {
        speech: BuddySpeechItem,
    },
    NavigationRequest {
        page: BuddyPage,
    },
    OpportunityProduced {
        opportunity: BuddyOpportunity,
    },
    OpportunityResolved {
        opportunity_id: String,
        status: OpportunityStatus,
    },
    PulseUpdated {
        pulse: BuddyPulse,
    },
    DraftCreated {
        draft: BuddyDraft,
    },
    DraftConsumed {
        draft_id: String,
    },
    DraftRemoved {
        draft_id: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_changed_keeps_event_type_tag() {
        let event = BuddyEvent::<serde_json::Value>::SettingsChanged {
            settings: BuddySettings::default(),
        };
        let value = serde_json::to_value(event).unwrap();

        assert_eq!(
            value.get("event_type").and_then(|v| v.as_str()),
            Some("SettingsChanged")
        );
        assert!(value.get("settings").is_some());
    }

    #[test]
    fn diagnostic_added_accepts_pure_payload() {
        let event = BuddyEvent::DiagnosticAdded {
            diagnostic: serde_json::json!({"error_type": "timeout"}),
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: BuddyEvent = serde_json::from_str(&json).unwrap();

        match back {
            BuddyEvent::DiagnosticAdded { diagnostic } => {
                assert_eq!(diagnostic["error_type"], "timeout");
            }
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[test]
    fn operator_signal_keeps_a_distinct_event_tag() {
        let signal: OperatorSignal = serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "kind": "observation",
            "authority": "hint",
            "project": {"project_id": "p", "canonical_root": "/p"},
            "worker": null,
            "source": {"source_kind": "chat", "source_id": "chat-1", "source_version": "1"},
            "source_sequence": null,
            "chat": null,
            "tool": null,
            "goal": null,
            "runtime_event": {
                "id": "event-1", "signal_type": "observed", "title": "Observed",
                "source": "test", "status": "completed", "priority": "normal",
                "created_at": "2026-01-01T00:00:00Z"
            }
        }))
        .unwrap();
        let value =
            serde_json::to_value(BuddyEvent::<serde_json::Value>::OperatorSignal { signal })
                .unwrap();
        assert_eq!(value["event_type"], "OperatorSignal");
    }
}
