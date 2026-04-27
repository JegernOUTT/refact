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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    /// Whether the user has explicitly dismissed this runtime event.
    /// Persisted across sessions; dismissed events are kept in the queue but hidden by the UI.
    #[serde(default)]
    pub dismissed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyIdentity {
    pub name: String,
    pub created_at: String,
    pub palette_index: usize,
}

fn default_first_growth_goal() -> u64 {
    20
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuddyCareAction {
    Feed,
    Play,
    Pet,
    Sleep,
    Clean,
}

impl BuddyCareAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Feed => "feed",
            Self::Play => "play",
            Self::Pet => "pet",
            Self::Sleep => "sleep",
            Self::Clean => "clean",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BuddyProgression {
    pub stage: u32,
    pub stage_name: String,
    pub level: u32,
    pub xp: u64,
    pub xp_next: u64,
}

impl Default for BuddyProgression {
    fn default() -> Self {
        Self {
            stage: 0,
            stage_name: "Egg".to_string(),
            level: 1,
            xp: 0,
            xp_next: default_first_growth_goal(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BuddyNeeds {
    pub hunger: u8,
    pub energy: u8,
    pub hygiene: u8,
    pub boredom: u8,
    pub affection: u8,
}

impl Default for BuddyNeeds {
    fn default() -> Self {
        Self {
            hunger: 80,
            energy: 85,
            hygiene: 80,
            boredom: 15,
            affection: 75,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BuddyCondition {
    pub sleeping: bool,
    pub hungry: bool,
    pub sleepy: bool,
    pub dirty: bool,
    pub bored: bool,
    pub lonely: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BuddyEvolutionState {
    pub care_score: u64,
    pub neglect_score: u64,
    pub open_seconds: u64,
    pub last_evolved_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BuddyPetState {
    pub needs: BuddyNeeds,
    pub condition: BuddyCondition,
    pub evolution: BuddyEvolutionState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BuddyPersonalityTraits {
    pub playfulness: u8,
    pub chaos: u8,
    pub sociability: u8,
    pub curiosity: u8,
    pub resilience: u8,
}

impl Default for BuddyPersonalityTraits {
    fn default() -> Self {
        Self {
            playfulness: 50,
            chaos: 50,
            sociability: 50,
            curiosity: 50,
            resilience: 50,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BuddyPersonalityProfile {
    pub archetype_id: String,
    pub archetype_label: String,
    pub vibe: String,
    pub summary: String,
    pub prompt: String,
    pub traits: BuddyPersonalityTraits,
}

impl Default for BuddyPersonalityProfile {
    fn default() -> Self {
        Self {
            archetype_id: "helper_sprite".to_string(),
            archetype_label: "Helper Sprite".to_string(),
            vibe: "Playful, quirky, helpful".to_string(),
            summary: "An energetic helper with gentle mischief and warm humor.".to_string(),
            prompt: "Playful, quirky, helpful. Think energetic pet meets curious assistant—gentle mischief, warm humor, celebration of small wins".to_string(),
            traits: BuddyPersonalityTraits::default(),
        }
    }
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

fn default_quest_status() -> String {
    "active".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyQuest {
    pub id: String,
    pub quest_type: String,
    pub title: String,
    pub description: String,
    pub icon: String,
    pub created_at: String,
    pub accepted_at: String,
    #[serde(default = "default_quest_status")]
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub progress: u32,
    pub goal: u32,
    #[serde(default)]
    pub baseline: u32,
    pub reward_xp: u64,
    #[serde(default)]
    pub controls: Vec<BuddyControl>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddySuggestion {
    pub id: String,
    pub suggestion_type: String,
    pub title: String,
    pub description: String,
    pub created_at: String,
    pub dismissed: bool,
    #[serde(default)]
    pub controls: Vec<BuddyControl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quest: Option<BuddyQuest>,
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
    pub pet: BuddyPetState,
    #[serde(default)]
    pub personality: BuddyPersonalityProfile,
    #[serde(default)]
    pub onboarding: BuddyOnboarding,
    #[serde(default)]
    pub job_cooldowns: HashMap<String, BuddyJobState>,
    #[serde(default)]
    pub active_quest: Option<BuddyQuest>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
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
