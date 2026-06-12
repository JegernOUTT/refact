use serde::{Deserialize, Serialize};

use crate::conductor::PublicConductorGoal;
use crate::settings::BuddySettings;
use crate::types::*;

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
    ConductorGoalUpdated {
        goal: PublicConductorGoal,
    },
    ConductorGhostMessage {
        ghost: BuddyGhostMessage,
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
    fn conductor_goal_updated_round_trips() {
        let goal = PublicConductorGoal {
            id: "goal-1".to_string(),
            title: "Buddy Conductor".to_string(),
            ..PublicConductorGoal::default()
        };
        let event = BuddyEvent::<serde_json::Value>::ConductorGoalUpdated { goal };
        let json = serde_json::to_string(&event).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let decoded: BuddyEvent = serde_json::from_str(&json).unwrap();

        assert_eq!(value["event_type"], "ConductorGoalUpdated");
        match decoded {
            BuddyEvent::ConductorGoalUpdated { goal } => {
                assert_eq!(goal.id, "goal-1");
                assert_eq!(goal.title, "Buddy Conductor");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn conductor_ghost_message_round_trips() {
        let ghost = BuddyGhostMessage {
            id: "ghost-1".to_string(),
            goal_id: Some("goal-1".to_string()),
            role: BuddyGhostMessageRole::Ask,
            content: "Need a tiny human answer".to_string(),
            created_at: "2026-06-03T00:00:00Z".to_string(),
            source_chat_id: Some("chat-1".to_string()),
            question_id: Some("question-1".to_string()),
        };
        let event = BuddyEvent::<serde_json::Value>::ConductorGhostMessage {
            ghost: ghost.clone(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let decoded: BuddyEvent = serde_json::from_str(&json).unwrap();

        assert_eq!(value["event_type"], "ConductorGhostMessage");
        assert_eq!(value["ghost"]["role"], "ask");
        match decoded {
            BuddyEvent::ConductorGhostMessage { ghost: decoded } => {
                assert_eq!(decoded, ghost);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
