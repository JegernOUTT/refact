use serde::{Deserialize, Serialize};
use crate::diagnostics::DiagnosticContext;
use crate::settings::BuddySettings;
use crate::types::{
    BuddyDraft, BuddyOpportunity, BuddyPulse, BuddyRuntimeEvent, BuddySpeechItem, BuddyState,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddySnapshot {
    pub state: BuddyState,
    pub settings: BuddySettings,
    pub enabled: bool,
    #[serde(default)]
    pub recent_diagnostics: Vec<DiagnosticContext>,
    pub runtime_queue: Vec<BuddyRuntimeEvent>,
    pub now_playing: Option<BuddyRuntimeEvent>,
    pub active_speech: Option<BuddySpeechItem>,
    #[serde(default)]
    pub pulse: BuddyPulse,
    #[serde(default)]
    pub opportunities: Vec<BuddyOpportunity>,
    #[serde(default)]
    pub active_drafts: Vec<BuddyDraft>,
}
