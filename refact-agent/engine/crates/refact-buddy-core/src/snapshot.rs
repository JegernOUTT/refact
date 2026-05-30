use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use crate::diagnostics::DiagnosticContext;
use crate::settings::BuddySettings;
use crate::types::{
    BuddyDraft, BuddyOpportunity, BuddyPulse, BuddyRuntimeEvent, BuddySpeechItem, BuddyState,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyStorageMetadata {
    pub project_root: String,
    pub buddy_dir: String,
    pub settings_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatReactionAttempt {
    pub attempted_at: String,
    pub chat_id: String,
    pub result: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatReactionDebug {
    #[serde(default)]
    pub recent_attempts: Vec<ChatReactionAttempt>,
    #[serde(default)]
    pub counts_by_result: HashMap<String, u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_skip_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_emitted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddySnapshot {
    pub state: BuddyState,
    pub settings: BuddySettings,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<BuddyStorageMetadata>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_reaction_debug: Option<ChatReactionDebug>,
}
