use std::collections::{BTreeMap, HashMap};
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
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
    #[serde(default)]
    pub opportunities: Vec<BuddyOpportunity>,
    #[serde(default)]
    pub dismissed_history: Vec<DismissEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DismissEntry {
    pub cooldown_key: String,
    pub dismissed_at: chrono::DateTime<chrono::Utc>,
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

#[derive(Debug, Clone, Serialize)]
pub struct BuddyFact {
    pub kind: BuddyFactKind,
    pub key: String,
    pub source: &'static str,
    pub payload: serde_json::Value,
    pub seen_at: chrono::DateTime<chrono::Utc>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BuddyFactKind {
    TaskStuck,
    TaskAbandoned,
    TaskClusterDuplicate,
    TrajectoryClutter,
    ChatRetryStreak,
    MemoryOrphan,
    MemoryStaleConflict,
    MemoryRecurringLesson,
    ModePromptOverlap,
    SkillTriggerWeak,
    AgentsMdGapDetected,
    DefaultModelMissing,
    BrokenModelReference,
    McpAuthExpired,
    IntegrationFailing,
    DiagnosticCluster,
    FrontendErrorBurst,
    GitDiffWidening,
    UncommittedPressure,
    WorktreeHygiene,
}

fn default_cooldown_secs() -> u64 {
    1800
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyOpportunity {
    pub id: String,
    pub kind: BuddyOpportunityKind,
    pub summary: String,
    pub priority: BuddyPriority,
    pub confidence: f32,
    pub fact_keys: Vec<String>,
    pub cooldown_key: String,
    #[serde(default = "default_cooldown_secs")]
    pub cooldown_secs: u64,
    pub status: OpportunityStatus,
    pub proposed_actions: Vec<BuddyAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub humor: Option<String>,
    #[serde(default)]
    pub humor_allowed: bool,
    #[serde(default)]
    pub related: BuddyOpportunityLinks,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BuddyOpportunityKind {
    TaskHealth,
    TrajectoryCleanup,
    ChatRecap,
    MemoryGarden,
    ConfigDrift,
    AgentsMdGap,
    ProviderTuning,
    IntegrationFix,
    DiagnosticInvestigation,
    GitHygiene,
    WorktreeCleanup,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BuddyPriority {
    Low,
    Normal,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum OpportunityStatus {
    New,
    Shown,
    Dismissed,
    Accepted,
    Completed,
    Expired,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuddyOpportunityLinks {
    #[serde(default)]
    pub chat_ids: Vec<String>,
    #[serde(default)]
    pub task_ids: Vec<String>,
    #[serde(default)]
    pub memory_ids: Vec<String>,
    #[serde(default)]
    pub config_paths: Vec<String>,
    #[serde(default)]
    pub page: Option<BuddyPage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BuddyAction {
    OpenPage {
        page: BuddyPage,
    },
    LaunchInvestigationChat {
        preload: InvestigationContext,
    },
    DraftSkill {
        draft_id: String,
        label: String,
    },
    DraftCommand {
        draft_id: String,
        label: String,
    },
    DraftDelegate {
        draft_id: String,
        label: String,
    },
    DraftMode {
        draft_id: String,
        label: String,
    },
    DraftAgentsMdPatch {
        content: String,
    },
    DraftDefaultsChange {
        defaults_kind: DefaultsKind,
        patch: serde_json::Value,
    },
    DraftCustomizationChange {
        customization_kind: CustomizationKind,
        id: String,
        patch: serde_json::Value,
    },
    OfferMarketplaceInstall {
        market_kind: MarketKind,
        item_id: String,
    },
    CreatePulseReport {
        scope: PulseScope,
    },
    Dismiss,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DefaultsKind {
    ChatModel,
    ChatLightModel,
    ChatBuddyModel,
    ChatThinkingModel,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CustomizationKind {
    Mode,
    Skill,
    Command,
    Delegate,
    Hook,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MarketKind {
    Mcp,
    Skill,
    Command,
    Delegate,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PulseScope {
    All,
    Tasks,
    Trajectories,
    Memory,
    Providers,
    Mcp,
    Customization,
    Diagnostics,
    Git,
    Worktrees,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BuddyPage {
    Buddy,
    Stats,
    Customization,
    Providers,
    DefaultModels,
    Integrations,
    Extensions,
    MarketplaceHub,
    Marketplace,
    SkillsMarketplace,
    CommandsMarketplace,
    DelegatesMarketplace,
    TasksList,
    TaskWorkspace { task_id: String },
    KnowledgeGraph,
    Worktrees,
    SetupMode { mode: String },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuddyPulse {
    pub generated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub tasks: TaskPulse,
    pub trajectories: TrajectoryPulse,
    pub memory: MemoryPulse,
    pub providers: ProviderPulse,
    pub mcp: McpPulse,
    pub customization: CustomizationPulse,
    pub diagnostics: DiagnosticPulse,
    pub git: GitPulse,
    pub worktrees: WorktreePulse,
    pub humor: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskPulse {
    pub total: u32,
    pub stuck: u32,
    pub abandoned: u32,
    pub by_status: std::collections::HashMap<String, u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrajectoryPulse {
    pub total: u32,
    pub untitled: u32,
    pub oldest_age_days: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryPulse {
    pub total: u32,
    pub orphan: u32,
    pub stale_conflicts: u32,
    #[serde(default)]
    pub duplicate_candidates: u32,
    #[serde(default)]
    pub merge_candidates: u32,
    #[serde(default)]
    pub archive_candidates: u32,
    #[serde(default)]
    pub review_candidates: u32,
    #[serde(default)]
    pub conflict_candidates: u32,
    #[serde(default)]
    pub pending_ops: u32,
    #[serde(default)]
    pub applied_ops: u32,
    #[serde(default)]
    pub failed_ops: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderPulse {
    pub defaults_ok: bool,
    pub broken_refs: u32,
    pub quota_warnings: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpPulse {
    pub total: u32,
    pub failing: u32,
    pub auth_expiring: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CompetitorImportPulse {
    pub last_run_at: Option<String>,
    pub discovered_candidates: u32,
    pub created: u32,
    pub updated: u32,
    pub stale: u32,
    pub conflicts: u32,
    pub user_modified: u32,
    pub unsupported: u32,
    pub errors: u32,
    pub sources_seen: BTreeMap<String, u32>,
    pub attention_items: u32,
    pub has_attention_items: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomizationPulse {
    pub modes: u32,
    pub skills: u32,
    pub commands: u32,
    pub subagents: u32,
    pub hooks: u32,
    #[serde(default)]
    pub competitor_import: CompetitorImportPulse,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiagnosticPulse {
    pub last_hour: u32,
    pub top_error_types: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitPulse {
    pub uncommitted_files: u32,
    pub diff_lines_4h: u32,
    pub branches: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WorktreePulse {
    pub total_registered: u32,
    pub total_discovered: u32,
    pub total: u32,
    pub clean: u32,
    pub dirty: u32,
    pub stale: u32,
    pub conflicted: u32,
    pub shared: u32,
    pub abandoned_clean: u32,
    pub changed_files: u32,
    pub additions: u32,
    pub deletions: u32,
    pub missing_registry_paths: u32,
    pub unregistered_cache_dirs: u32,
    pub merged_branches: u32,
    pub newest_age_hours: Option<u64>,
    pub oldest_age_hours: Option<u64>,
    pub disk_usage_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyDraft {
    pub id: String,
    pub kind: DraftKind,
    pub title: String,
    pub yaml_or_json: String,
    pub explanation: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DraftKind {
    Skill,
    Command,
    Delegate,
    Mode,
    AgentsMd,
    DefaultsModel,
    Hook,
    PulseReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvestigationContext {
    #[serde(default)]
    pub fact_keys: Vec<String>,
    #[serde(default)]
    pub diagnostic_ids: Vec<String>,
    #[serde(default)]
    pub log_excerpt: String,
    #[serde(default)]
    pub config_summary: String,
    pub initial_user_message: String,
}
