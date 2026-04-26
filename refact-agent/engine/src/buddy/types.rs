use std::collections::HashMap;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BuddyOnboarding {
    pub greeted: bool,
    pub tour_completed: bool,
    pub first_launch_at: String,
    pub last_greeting_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyRuntimeEvent {
    pub id: String,
    pub signal_type: String,
    pub title: String,
    pub description: Option<String>,
    pub source: String,
    pub status: String,
    pub progress: Option<u8>,
    pub dedupe_key: Option<String>,
    pub priority: String,
    pub created_at: String,
    pub ttl_ms: Option<u64>,
    #[serde(default)]
    pub speech_text: Option<String>,
    #[serde(default)]
    pub scene: Option<String>,
    #[serde(default)]
    pub duration_hint: Option<u32>,
    #[serde(default)]
    pub persistent: bool,
    #[serde(default)]
    pub controls: Vec<BuddyControl>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyIdentity {
    pub name: String,
    pub created_at: String,
    pub palette_index: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyProgression {
    pub stage: u32,
    pub stage_name: String,
    pub level: u32,
    pub xp: u64,
    pub xp_next: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddySkillLedger {
    pub unlocked: Vec<String>,
    pub locked: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyWorkflowSummary {
    pub workflow_id: String,
    pub last_run: Option<String>,
    pub run_count: u64,
    pub last_outcome: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddySemanticSnapshot {
    pub mood: String,
    pub focus: String,
    pub headline: String,
    pub last_active: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyActivity {
    pub icon: String,
    pub title: String,
    pub description: String,
    pub timestamp: String,
    pub activity_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddySuggestion {
    pub id: String,
    pub suggestion_type: String,
    pub title: String,
    pub description: String,
    pub created_at: String,
    pub dismissed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BuddyJobState {
    pub last_run: Option<String>,
    pub last_result: Option<String>,
    pub run_count: u32,
    pub snoozed_until: Option<String>,
    pub dismissed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyState {
    pub identity: BuddyIdentity,
    pub progression: BuddyProgression,
    pub skills: BuddySkillLedger,
    pub workflow_summaries: Vec<BuddyWorkflowSummary>,
    pub semantic: BuddySemanticSnapshot,
    pub recent_activities: Vec<BuddyActivity>,
    pub suggestion_state: Vec<BuddySuggestion>,
    #[serde(default)]
    pub onboarding: BuddyOnboarding,
    #[serde(default)]
    pub job_cooldowns: HashMap<String, BuddyJobState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyThreadMeta {
    pub is_buddy_chat: bool,
    pub buddy_chat_kind: String,
    pub workflow_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyControl {
    pub id: String,
    pub label: String,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_param: Option<String>,
    #[serde(default = "default_control_style")]
    pub style: String,
}

fn default_control_style() -> String {
    "secondary".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddySpeechItem {
    pub id: String,
    pub text: String,
    #[serde(default = "default_mood")]
    pub mood: String,
    #[serde(default = "default_scope")]
    pub scope: String,
    #[serde(default)]
    pub persistent: bool,
    #[serde(default = "default_ttl")]
    pub ttl_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dedupe_key: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub controls: Vec<BuddyControl>,
}

fn default_mood() -> String {
    "neutral".to_string()
}

fn default_scope() -> String {
    "global".to_string()
}

fn default_ttl() -> u64 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyConversationEntry {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub status: String,
    pub message_count: u32,
    pub icon: String,
    pub badge: Option<String>,
}
