use serde::{Deserialize, Serialize};
use super::types::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum BuddyEvent {
    StateUpdated { state: BuddyState },
    ActivityAdded { activity: BuddyActivity },
    SuggestionAdded { suggestion: BuddySuggestion },
    SuggestionDismissed { suggestion_id: String },
    SettingsChanged { settings: super::settings::BuddySettings },
    DiagnosticAdded { diagnostic: super::diagnostics::DiagnosticContext },
    RuntimeEvent { event: BuddyRuntimeEvent },
}
