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
