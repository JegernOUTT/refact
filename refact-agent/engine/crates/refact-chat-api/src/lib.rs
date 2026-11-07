use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
pub use refact_core::recovery::{RecoveryPolicy, RecoveryPolicyOverrides, RecoveryRecord};

pub mod chat_local_types;
pub mod code_lens;
pub mod deliverable;
pub mod diagnostics;
pub mod goal_ledger;
pub mod goal_role;
pub mod handoff;
pub mod internal_roles;
pub mod notification_event;
pub mod plan_role;
pub mod question;
#[cfg(test)]
mod role_checkpoint_tests;
pub mod role_projection;
pub mod seal;
pub mod sub_goal;
pub mod tool_enrichment;

pub use goal_ledger::{
    goal_budget_exhaustion_status, reduce_goal_ledger, seed_transferred_goal_ledger,
    status_changed_since, GoalLedgerEntry, GoalLedgerOp, GoalLedgerState,
};
pub use sub_goal::{
    aggregate_sub_goals, sub_goal_effective_outcome, SubGoalDeclaration, SubGoalDeclarationError,
    SubGoalEdge, SubGoalEdgeKind, SubGoalGraph, SubGoalLane, SubGoalLimits, SubGoalNode,
    SubGoalNodeState, SubGoalOutcome, SubGoalOwner, SubGoalReport, SubGoalRollup,
    DEFAULT_SUB_GOAL_MAX_DEPTH, DEFAULT_SUB_GOAL_MAX_FAN_OUT, ROOT_SUB_GOAL_ID,
};

pub use chat_local_types::{
    max_queue_size, session_cleanup_interval, session_idle_timeout, stream_heartbeat,
    stream_idle_timeout, stream_total_timeout, install_runtime_timeouts, EnqueueCommandOutcome,
    PendingBrowserMessage, RuntimeChatTimeouts, TrajectorySourceIdentity,
};
pub use notification_event::{NotificationEvent, NotificationQuestion};
pub use seal::SealReason;
pub use question::{
    AskQuestion, QuestionContract, QuestionError, QuestionKind, QuestionLimits, QuestionRegistry,
    QuestionSettlement, QuestionState, QUESTION_MAX_EXPIRY_SECS,
};
pub use deliverable::{
    plan_correction, CorrectionPolicy, CorrectionRequest, DeliverableContract, DeliverableOmission,
    DeliverableStatus, ResultHeadroom, DELIVERABLE_CHECK_STRUCTURAL,
};
pub use handoff::{
    HandoffCategorySignal, HandoffFinding, HandoffGap, HandoffGapCategory, HandoffLimits,
    HandoffMetric, HandoffReport, HandoffSufficiency, HANDOFF_BLOCK_TAG,
    HANDOFF_MIN_RECURRING_CHILDREN, HANDOFF_SOURCE_CHILD_DECLARATION,
};
pub use tool_enrichment::{
    attach_tool_enrichment, attach_tool_enrichment_to_extra, redact_tool_enrichment,
    sanitize_http_url, tool_enrichment_from_extra, ToolEnrichment, ToolEnrichmentKind,
    ToolEnrichmentPrivacy, ToolEnrichmentProvenance, ToolEnrichmentReference,
    ToolEnrichmentReferenceDetails, TOOL_ENRICHMENT_EXTRA_KEY, TOOL_ENRICHMENT_SCHEMA_VERSION,
};
pub use refact_core::buddy_meta::BuddyThreadMeta;
pub use refact_core::chat_types::{
    delivery_id_of_message, CausalStamp, ChatMessage, ContextFile, DeliveryOutcome, DeliveryStatus,
    DeliveryStatusRecord, PeerMailStamp, PendingDelivery, PushMode, DELIVERY_EXTRA_KEY,
};
pub use refact_core::worktree_meta::WorktreeMeta;

pub const CLIENT_MESSAGE_ID_EXTRA_KEY: &str = "client_message_id";
pub const MAX_CLIENT_MESSAGE_ID_CHARS: usize = 256;

pub fn validate_client_message_id(client_message_id: Option<&str>) -> Result<(), String> {
    let Some(client_message_id) = client_message_id else {
        return Ok(());
    };
    if client_message_id.is_empty() {
        return Err("client_message_id must be non-empty when provided".to_string());
    }
    if client_message_id.chars().count() > MAX_CLIENT_MESSAGE_ID_CHARS {
        return Err(format!(
            "client_message_id exceeds {MAX_CLIENT_MESSAGE_ID_CHARS} characters"
        ));
    }
    Ok(())
}

fn default_true() -> bool {
    true
}

fn is_false(value: &bool) -> bool {
    !*value
}

pub(crate) fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiffBox {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserTabInfo {
    pub tab_id: String,
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserSnapshot {
    pub runtime_id: String,
    pub connected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_tab: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tabs: Vec<BrowserTabInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub timestamp: String,
    pub source: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BrowserMeta {
    pub browser_runtime_id: Option<String>,
    pub profile_dir: Option<String>,
    #[serde(default)]
    pub tab_urls: Vec<String>,
    pub active_tab_id: Option<String>,
    pub window_bounds: Option<WindowBounds>,
    #[serde(default)]
    pub attach_screenshot_on_send: bool,
    #[serde(default = "default_true")]
    pub mask_passwords: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Idle,
    Generating,
    ExecutingTools,
    Paused,
    WaitingIde,
    WaitingUserInput,
    Interrupted,
    Completed,
    Error,
}

impl Default for SessionState {
    fn default() -> Self {
        SessionState::Idle
    }
}

impl std::fmt::Display for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionState::Idle => write!(f, "idle"),
            SessionState::Generating => write!(f, "generating"),
            SessionState::ExecutingTools => write!(f, "executing_tools"),
            SessionState::Paused => write!(f, "paused"),
            SessionState::WaitingIde => write!(f, "waiting_ide"),
            SessionState::WaitingUserInput => write!(f, "waiting_user_input"),
            SessionState::Interrupted => write!(f, "interrupted"),
            SessionState::Completed => write!(f, "completed"),
            SessionState::Error => write!(f, "error"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompressionPhase {
    Checking,
    Running,
    Applied,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompressionReason {
    AutoCompactDisabled,
    SessionCompactionDisabled,
    MaxAttemptsReached,
    PendingToolCalls,
    NoEligibleSegment,
    EffectiveContextUnknown,
    ProviderLengthStop,
    ContextLengthStop,
    PressureLow,
    NoSummaryModel,
    InputTooLarge,
    TransientFailure,
    SourceChanged,
    InsufficientSavings,
    PreCompactDenied,
}

impl CompressionReason {
    pub const ALL: [Self; 15] = [
        Self::AutoCompactDisabled,
        Self::SessionCompactionDisabled,
        Self::MaxAttemptsReached,
        Self::PendingToolCalls,
        Self::NoEligibleSegment,
        Self::EffectiveContextUnknown,
        Self::ProviderLengthStop,
        Self::ContextLengthStop,
        Self::PressureLow,
        Self::NoSummaryModel,
        Self::InputTooLarge,
        Self::TransientFailure,
        Self::SourceChanged,
        Self::InsufficientSavings,
        Self::PreCompactDenied,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    #[default]
    Active,
    Verifying,
    Paused,
    Completed,
    Stopped,
    BudgetExhausted,
    NoProgress,
    Transferred,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalBudget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_minutes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_cents: Option<u64>,
    #[serde(default = "default_goal_budget_cooldown_ms")]
    pub cooldown_ms: u64,
    #[serde(default = "default_goal_no_progress_token_threshold")]
    pub no_progress_token_threshold: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_progress_turns: Option<u32>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub explicit: bool,
}

fn default_goal_budget_cooldown_ms() -> u64 {
    30_000
}

const LEGACY_GOAL_BUDGET_COOLDOWN_MS: u64 = 1_500;

fn default_goal_no_progress_token_threshold() -> u64 {
    50
}

impl Default for GoalBudget {
    fn default() -> Self {
        Self {
            max_turns: None,
            max_minutes: None,
            max_tokens: None,
            max_cost_cents: None,
            cooldown_ms: default_goal_budget_cooldown_ms(),
            no_progress_token_threshold: default_goal_no_progress_token_threshold(),
            no_progress_turns: None,
            explicit: false,
        }
    }
}

impl GoalBudget {
    pub fn legacy_default_hard_limits() -> Self {
        Self {
            max_turns: Some(10),
            max_minutes: Some(15),
            max_tokens: Some(200_000),
            max_cost_cents: None,
            cooldown_ms: default_goal_budget_cooldown_ms(),
            no_progress_token_threshold: default_goal_no_progress_token_threshold(),
            no_progress_turns: Some(2),
            explicit: false,
        }
    }

    pub fn migrate_legacy_default_hard_limits(mut self) -> Self {
        if self.explicit {
            return self;
        }
        if self.max_turns == Some(10)
            && self.max_minutes == Some(15)
            && self.max_tokens == Some(200_000)
            && self.no_progress_turns == Some(2)
        {
            self = Self {
                cooldown_ms: self.cooldown_ms,
                no_progress_token_threshold: self.no_progress_token_threshold,
                ..Default::default()
            };
        }
        if self.cooldown_ms == LEGACY_GOAL_BUDGET_COOLDOWN_MS
            && self.max_turns.is_none()
            && self.max_minutes.is_none()
            && self.max_tokens.is_none()
            && self.max_cost_cents.is_none()
            && self.no_progress_turns.is_none()
        {
            self.cooldown_ms = default_goal_budget_cooldown_ms();
        }
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct GoalProgress {
    pub turns_used: u32,
    pub tokens_used: u64,
    pub started_at_ms: u64,
    pub no_progress_turns: u32,
    pub last_nudge_at_ms: u64,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub cost_used_cents: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct GoalAttempt {
    pub at_ms: u64,
    pub trigger: String,
    pub verdict: String,
    pub gaps: Vec<String>,
    pub verifier_reply: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub criteria_verdicts: Vec<CriterionVerdict>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct GoalCriterion {
    pub id: String,
    pub text: String,
    /// Prose for the semantic judge. Never executed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_hint: Option<String>,
    /// An executable contract proposal. Runs only after acceptance under policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify_command: Option<String>,
}

/// Whether a criterion was decided by executed evidence or by semantic judgement.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerdictSource {
    #[default]
    Judged,
    Measured,
}

impl VerdictSource {
    pub fn is_measured(self) -> bool {
        matches!(self, VerdictSource::Measured)
    }

    pub fn label(self) -> &'static str {
        match self {
            VerdictSource::Judged => "judged",
            VerdictSource::Measured => "measured",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CriterionEvidence {
    pub spec_id: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub verdict: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CriterionVerdict {
    pub id: String,
    pub met: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(default)]
    pub source: VerdictSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<CriterionEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalEvent {
    pub at_ms: u64,
    pub kind: String,
    pub text: String,
}

/// What a waiting goal is waiting on. A wait without one of these is
/// indistinguishable from being stuck, so there is no untyped variant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum GoalWaitSubject {
    Process { process_id: String },
    Child { agent_id: String },
    Event { name: String },
    Deadline { until_ms: u64 },
}

impl GoalWaitSubject {
    pub fn label(&self) -> String {
        match self {
            GoalWaitSubject::Process {
                process_id,
            } => format!("process {process_id}"),
            GoalWaitSubject::Child {
                agent_id,
            } => format!("child run {agent_id}"),
            GoalWaitSubject::Event {
                name,
            } => format!("event {name}"),
            GoalWaitSubject::Deadline {
                until_ms,
            } => format!("deadline {until_ms}"),
        }
    }

    pub fn own_deadline_ms(&self) -> Option<u64> {
        match self {
            GoalWaitSubject::Deadline {
                until_ms,
            } => Some(*until_ms),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalWait {
    pub subject: GoalWaitSubject,
    #[serde(default)]
    pub since_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u64>,
}

impl GoalWait {
    pub fn new(subject: GoalWaitSubject, deadline_ms: Option<u64>) -> Self {
        let deadline_ms = deadline_ms.or_else(|| subject.own_deadline_ms());
        Self {
            subject,
            since_ms: 0,
            deadline_ms,
        }
    }

    /// A deadline is a wait, never a pass: expiry is reported as its own
    /// outcome naming the subject.
    pub fn expired_at(&self, now_ms: u64) -> bool {
        self.deadline_ms.is_some_and(|until_ms| now_ms >= until_ms)
    }

    pub fn matches(&self, subject: &GoalWaitSubject) -> bool {
        &self.subject == subject
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoalWaitResume {
    Settled,
    Expired,
    Cancelled,
}

impl GoalWaitResume {
    pub fn label(self) -> &'static str {
        match self {
            GoalWaitResume::Settled => "settled",
            GoalWaitResume::Expired => "expired",
            GoalWaitResume::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GoalOwner {
    Chat { id: String },
    Task { id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GoalPlanInstall {
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum GoalPlanMutation {
    Install(GoalPlanInstall),
    Append(GoalPlanNote),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GoalPlanNote {
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct GoalPlanSnapshot {
    pub content: String,
    pub mode: String,
    pub version: u32,
    pub created_at_ms: u64,
    pub delta_count: usize,
    pub checkpoint: Option<role_projection::CheckpointProvenance>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<GoalPlanHistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalPlanHistoryEntry {
    pub seq: u64,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct GoalSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<GoalOwner>,
    #[serde(default)]
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<GoalPlanSnapshot>,
    pub content: String,
    pub version: u32,
    pub active: bool,
    pub status: GoalStatus,
    pub budget: GoalBudget,
    pub progress: GoalProgress,
    pub attempts: Vec<GoalAttempt>,
    pub events: Vec<GoalEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub criteria: Vec<GoalCriterion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snoozed_until_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait: Option<GoalWait>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    pub transferred_from: Option<String>,
    pub transferred_to: Option<String>,
    /// Child lanes composed under this one goal. Empty for every goal that
    /// never declared any, so an older snapshot reads unchanged.
    #[serde(default, skip_serializing_if = "sub_goal::SubGoalGraph::is_empty")]
    pub sub_goals: sub_goal::SubGoalGraph,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TaskMeta {
    pub task_id: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planner_chat_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct FrozenRequestPrefix {
    pub schema_version: u32,
    pub created_at: String,
    pub system_prompt: Option<String>,
    pub tools_canonical: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ClaudeCodeIdentity {
    pub device_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadParams {
    #[serde(default)]
    pub recovery_policy: Option<RecoveryPolicyOverrides>,
    pub id: String,
    pub title: String,
    pub model: String,
    pub mode: String,
    pub tool_use: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boost_reasoning: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_budget: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    pub context_tokens_cap: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_compression_cap: Option<usize>,
    /// Only new chats opt in; missing persisted fields must remain legacy/unset.
    #[serde(default)]
    pub auto_compression_cap_pending: bool,
    pub include_project_info: bool,
    pub checkpoints_enabled: bool,
    #[serde(default)]
    pub is_title_generated: bool,
    #[serde(default)]
    pub auto_approve_editing_tools: bool,
    #[serde(default)]
    pub auto_approve_dangerous_commands: bool,
    #[serde(default)]
    pub autonomous_no_confirm: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_meta: Option<TaskMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<WorktreeMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_meta: Option<BrowserMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_skill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_enrichment_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buddy_meta: Option<BuddyThreadMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_compact_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen_request_prefix: Option<FrozenRequestPrefix>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_code_identity: Option<ClaudeCodeIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none", skip_deserializing)]
    pub reactive_compact_attempts: Option<usize>,
}

impl refact_core::worktree_meta::WorktreeThread for ThreadParams {
    fn worktree(&self) -> Option<&WorktreeMeta> {
        self.worktree.as_ref()
    }
}

/// Ninety percent of the smallest known positive context window. Unknown windows
/// remain unset; integer arithmetic avoids rounding and overflow surprises.
pub fn default_auto_compression_cap(
    model_window: Option<usize>,
    request_window: Option<usize>,
) -> Option<usize> {
    [model_window, request_window]
        .into_iter()
        .flatten()
        .filter(|window| *window > 0)
        .min()
        .map(|window| window / 10 * 9 + window % 10 * 9 / 10)
}

impl ThreadParams {
    /// Call only for NEW chats once their model/request windows are known. Never
    /// call while loading persisted chats. Explicit values (including zero) win;
    /// model switches leave the cap unchanged, and an explicit reset can call this
    /// again after clearing the cap.
    pub fn initialize_new_chat_compression_cap(
        &mut self,
        model_window: Option<usize>,
        request_window: Option<usize>,
    ) {
        if self.auto_compression_cap.is_none() {
            self.auto_compression_cap = default_auto_compression_cap(
                model_window,
                request_window.or(self.context_tokens_cap),
            );
        }
    }
    /// Resolve the new-chat default once, after the actual model is selected.
    /// Legacy loaded chats never opt in, even if their history is empty.
    pub fn resolve_pending_compression_cap(&mut self, model_window: Option<usize>) -> bool {
        if !self.auto_compression_cap_pending {
            return false;
        }
        if self.auto_compression_cap.is_none() && !model_window.is_some_and(|n| n > 0) {
            return false;
        }
        self.initialize_new_chat_compression_cap(model_window, None);
        self.auto_compression_cap_pending = false;
        true
    }

    pub fn auto_compact_enabled_effective(&self) -> bool {
        self.auto_compact_enabled.unwrap_or(true)
    }
}

impl Default for ThreadParams {
    fn default() -> Self {
        Self {
            recovery_policy: None,
            id: Uuid::new_v4().to_string(),
            title: "New Chat".to_string(),
            model: String::new(),
            mode: "agent".to_string(),
            tool_use: "agent".to_string(),
            boost_reasoning: None,
            reasoning_effort: None,
            thinking_budget: None,
            temperature: None,
            frequency_penalty: None,
            max_tokens: None,
            parallel_tool_calls: None,
            context_tokens_cap: None,
            auto_compression_cap: None,
            auto_compression_cap_pending: true,
            include_project_info: true,
            checkpoints_enabled: true,
            is_title_generated: false,
            auto_approve_editing_tools: false,
            auto_approve_dangerous_commands: false,
            autonomous_no_confirm: false,
            task_meta: None,
            worktree: None,
            parent_id: None,
            link_type: None,
            root_chat_id: None,
            previous_response_id: None,
            browser_meta: None,
            active_skill: None,
            auto_enrichment_enabled: None,
            buddy_meta: None,
            auto_compact_enabled: None,
            frozen_request_prefix: None,
            claude_code_identity: None,
            reactive_compact_attempts: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedItem {
    pub client_request_id: String,
    pub priority: bool,
    pub command_type: String,
    pub preview: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content: String,
    /// Delivery rows only: requested landing boundary, editable while pending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push: Option<PushMode>,
    /// Delivery rows only: producer that enqueued this batch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Delivery rows only: the event descriptor `{ subkind, source, payload? }`
    /// of the delivered event message, absent when the batch carries no event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enqueued_at_ms: Option<u64>,
}

pub const DELIVERY_COMMAND_TYPE: &str = "delivery";

impl QueuedItem {
    pub fn from_pending_delivery(delivery: &PendingDelivery) -> Self {
        let preview = serde_json::Value::String(delivery.preview_text());
        Self {
            client_request_id: delivery.id.clone(),
            priority: delivery.push == PushMode::Preempt,
            command_type: DELIVERY_COMMAND_TYPE.to_string(),
            preview: extract_preview(&preview),
            content: extract_full_text_capped(&preview),
            push: Some(delivery.push),
            source: Some(delivery.source.clone()),
            event: delivery_event_descriptor(delivery),
            enqueued_at_ms: Some(delivery.enqueued_at_ms),
        }
    }
}

fn delivery_event_descriptor(delivery: &PendingDelivery) -> Option<serde_json::Value> {
    delivery
        .messages
        .iter()
        .filter(|message| message.role == internal_roles::EVENT_ROLE)
        .find_map(|message| {
            let event = message.extra.get("event")?;
            let subkind = event
                .get("subkind")
                .and_then(serde_json::Value::as_str)
                .filter(|subkind| !subkind.is_empty())?;
            let source = event
                .get("source")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(delivery.source.as_str());
            let mut descriptor = serde_json::json!({
                "subkind": subkind,
                "source": source,
            });
            if let Some(payload) = event.get("payload") {
                descriptor["payload"] = payload.clone();
            }
            Some(descriptor)
        })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardInterruption {
    pub condition: String,
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub owner_chat_id: Option<String>,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interruption: Option<BoardInterruption>,
    #[serde(default)]
    pub recovery: Option<RecoveryRecord>,
    #[serde(default)]
    pub waiting_interruptible: bool,
    pub state: SessionState,
    pub paused: bool,
    pub error: Option<String>,
    pub queue_size: usize,
    #[serde(default)]
    pub goal_active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_status: Option<GoalStatus>,
    #[serde(default)]
    pub goal_turns_used: u32,
    #[serde(default)]
    pub goal_tokens_used: u64,
    #[serde(default)]
    pub goal_no_progress_turns: u32,
    #[serde(default)]
    pub is_compressing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression_phase: Option<CompressionPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression_reason: Option<CompressionReason>,
    #[serde(default)]
    pub pause_reasons: Vec<PauseReason>,
    #[serde(default)]
    pub queued_items: Vec<QueuedItem>,
    #[serde(default, skip_serializing)]
    pub auto_approved_tool_ids: Vec<String>,
    #[serde(default, skip_serializing)]
    pub accepted_tool_ids: Vec<String>,
    #[serde(default, skip_serializing)]
    pub paused_message_index: Option<usize>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            interruption: None,
            recovery: None,
            state: SessionState::Idle,
            paused: false,
            error: None,
            queue_size: 0,
            waiting_interruptible: false,
            goal_active: false,
            goal_status: None,
            goal_turns_used: 0,
            goal_tokens_used: 0,
            goal_no_progress_turns: 0,
            is_compressing: false,
            compression_phase: None,
            compression_reason: None,
            pause_reasons: Vec::new(),
            queued_items: Vec::new(),
            auto_approved_tool_ids: Vec::new(),
            accepted_tool_ids: Vec::new(),
            paused_message_index: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PauseReason {
    #[serde(rename = "type")]
    pub reason_type: String,
    pub tool_name: String,
    pub command: String,
    pub rule: String,
    pub tool_call_id: String,
    pub integr_config_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentQuestionSummary {
    pub id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<QuestionKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<QuestionState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// Only what a human actually chose. A defaulted question leaves this
    /// empty so no surface can render a timeout as consent.
    pub answer: Option<String>,
    /// The documented default that was applied instead of an answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defaulted_answer: Option<String>,
    pub asked_at: DateTime<Utc>,
    pub answered_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundAgentSummary {
    pub agent_id: String,
    pub parent_chat_id: String,
    pub child_chat_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub title: String,
    pub progress: Option<String>,
    pub step_count: u32,
    pub last_activity: Option<String>,
    pub target_files: Vec<String>,
    pub edited_files: Vec<String>,
    pub diff_summary: Option<String>,
    pub conflict_summary: Option<String>,
    pub result_summary: Option<String>,
    pub error: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub change_seq: u64,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub model_type: Option<String>,
    #[serde(default)]
    pub current_tool: Option<String>,
    #[serde(default)]
    pub goal_summary: Option<String>,
    #[serde(default)]
    pub plan_present: bool,
    #[serde(default)]
    pub worktree_branch: Option<String>,
    #[serde(default)]
    pub merge_status: Option<String>,
    #[serde(default)]
    pub pending_questions: u32,
    #[serde(default)]
    pub tokens_used: u64,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub questions: Vec<AgentQuestionSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deliverable: Option<DeliverableContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deliverable_status: Option<DeliverableStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<HandoffReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatEvent {
    RecoveryUpdated {
        recovery: Option<RecoveryRecord>,
    },
    Snapshot {
        thread: ThreadParams,
        runtime: RuntimeState,
        messages: Vec<ChatMessage>,
        background_agents: Vec<BackgroundAgentSummary>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        browser: Option<BrowserSnapshot>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        goal: Option<GoalSnapshot>,
        #[serde(default)]
        pending_questions: Vec<PendingQuestionSet>,
    },
    BackgroundAgentUpdated {
        chat_id: String,
        seq: u64,
        agent: BackgroundAgentSummary,
    },
    ExecProcessSpawned {
        chat_id: String,
        seq: u64,
        process: ExecProcessSpawn,
    },
    ThreadUpdated {
        #[serde(flatten)]
        params: serde_json::Value,
    },
    QueueUpdated {
        queue_size: usize,
        queued_items: Vec<QueuedItem>,
    },
    MessageAdded {
        message: ChatMessage,
        index: usize,
    },
    ProcessCompleted {
        process_id: String,
        status: String,
        exit_code: Option<i32>,
        short_description: String,
        mode: String,
    },
    MessageUpdated {
        message_id: String,
        message: ChatMessage,
    },
    MessageRemoved {
        message_id: String,
    },
    MessagesTruncated {
        from_index: usize,
    },
    StreamStarted {
        message_id: String,
    },
    StreamDelta {
        message_id: String,
        ops: Vec<DeltaOp>,
    },
    StreamFinished {
        message_id: String,
        finish_reason: Option<String>,
    },
    PauseRequired {
        reasons: Vec<PauseReason>,
    },
    PauseCleared {},
    IdeToolRequired {
        tool_call_id: String,
        tool_name: String,
        args: serde_json::Value,
    },
    SubchatUpdate {
        tool_call_id: String,
        subchat_id: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attached_files: Vec<String>,
    },
    RuntimeUpdated {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        interruption: Option<BoardInterruption>,
        #[serde(default)]
        waiting_interruptible: bool,
        state: SessionState,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(default)]
        goal_active: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        goal_status: Option<GoalStatus>,
        #[serde(default)]
        goal_turns_used: u32,
        #[serde(default)]
        goal_tokens_used: u64,
        #[serde(default)]
        goal_no_progress_turns: u32,
        #[serde(default)]
        is_compressing: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        compression_phase: Option<CompressionPhase>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        compression_reason: Option<CompressionReason>,
    },
    Ack {
        client_request_id: String,
        accepted: bool,
        result: Option<serde_json::Value>,
    },
    BrowserFrame {
        tab_id: String,
        mime: String,
        data: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        diff_boxes: Vec<DiffBox>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        changed_text: Option<String>,
    },
    BrowserStatus {
        runtime_id: String,
        connected: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        active_tab: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tabs: Vec<BrowserTabInfo>,
    },
    BrowserClosed {
        runtime_id: String,
        reason: String,
    },
    BrowserTimeline {
        events: Vec<TimelineEntry>,
    },
    BrowserContextOversize {
        total_bytes: usize,
        action_count: usize,
        action_bytes: usize,
        console_count: usize,
        console_bytes: usize,
        network_count: usize,
        network_bytes: usize,
        mutation_bytes: usize,
        pending_message_id: String,
    },
    BrowserToolbarAction {
        action: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecProcessSpawn {
    pub process_id: String,
    pub command_preview: String,
    pub mode: String,
    pub tty: bool,
    pub status: String,
    pub started_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DeltaOp {
    AppendContent {
        text: String,
    },
    AppendReasoning {
        text: String,
    },
    SetReasoning {
        text: String,
    },
    SetToolCalls {
        tool_calls: Vec<serde_json::Value>,
    },
    SetThinkingBlocks {
        blocks: Vec<serde_json::Value>,
    },
    AddCitation {
        citation: serde_json::Value,
    },
    AddServerContentBlock {
        block: serde_json::Value,
    },
    SetUsage {
        usage: serde_json::Value,
    },
    MergeExtra {
        extra: serde_json::Map<String, serde_json::Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub chat_id: String,
    #[serde(
        serialize_with = "serialize_seq_as_string",
        deserialize_with = "deserialize_seq_from_string"
    )]
    pub seq: u64,
    #[serde(flatten)]
    pub event: ChatEvent,
}

fn serialize_seq_as_string<S>(seq: &u64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&seq.to_string())
}

fn deserialize_seq_from_string<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let s: String = serde::Deserialize::deserialize(deserializer)?;
    s.parse().map_err(D::Error::custom)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatCommand {
    RecoveryRetry {
        run_id: String,
        expected_revision: u64,
        options: RecoveryPolicy,
        #[serde(default)]
        model: Option<String>,
    },
    RecoveryRetryNow {
        run_id: String,
        expected_revision: u64,
    },
    UserMessage {
        content: serde_json::Value,
        #[serde(default)]
        attachments: Vec<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        context_files: Vec<serde_json::Value>,
        #[serde(default)]
        suppress_auto_enrichment: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_message_id: Option<String>,
    },
    RetryFromIndex {
        index: usize,
        content: serde_json::Value,
        #[serde(default)]
        attachments: Vec<serde_json::Value>,
    },
    SetParams {
        patch: serde_json::Value,
    },
    SetGoal {
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plan: Option<GoalPlanInstall>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        criteria: Option<Vec<GoalCriterion>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        budget: Option<GoalBudget>,
    },
    SetGoalBudget {
        budget: GoalBudget,
    },
    UpdateGoal {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plan: Option<GoalPlanMutation>,
    },
    GoalControl {
        action: String,
    },
    Abort {},
    AnswerQuestions {
        tool_call_id: String,
        answers: Vec<QuestionAnswer>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        asked_at_ms: Option<u64>,
    },
    SubmitQuestionResponse {
        tool_call_id: String,
        asked_at_ms: u64,
        response: QuestionResponse,
    },
    CleanBackgroundProcesses {
        #[serde(default)]
        include_services: bool,
    },
    ToolDecision {
        tool_call_id: String,
        accepted: bool,
    },
    ToolDecisions {
        decisions: Vec<ToolDecisionItem>,
    },
    IdeToolResult {
        tool_call_id: String,
        content: String,
        #[serde(default)]
        tool_failed: bool,
    },
    UpdateMessage {
        message_id: String,
        content: serde_json::Value,
        #[serde(default)]
        attachments: Vec<serde_json::Value>,
        #[serde(default)]
        regenerate: bool,
    },
    RemoveMessage {
        message_id: String,
        #[serde(default)]
        regenerate: bool,
    },
    Regenerate {},
    RestoreMessages {
        messages: Vec<serde_json::Value>,
    },
    BranchFromChat {
        source_chat_id: String,
        up_to_message_id: String,
    },
    BrowserContextDecision {
        pending_message_id: String,
        #[serde(default = "default_true")]
        include_actions: bool,
        #[serde(default = "default_true")]
        include_console: bool,
        #[serde(default = "default_true")]
        include_network: bool,
        #[serde(default = "default_true")]
        include_mutations: bool,
        #[serde(default = "default_true")]
        include_screenshot: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_n_actions: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_n_console: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_n_network: Option<usize>,
    },
    /// Unified delivery of externally produced messages into this chat.
    /// Routed into the pending-delivery queue immediately rather than waiting
    /// for the whole command loop, so pending edits work during generation.
    DeliverMessages {
        delivery: PendingDelivery,
    },
    /// Reprioritize or cancel a still-pending delivery.
    UpdatePendingDelivery {
        delivery_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        push: Option<PushMode>,
        #[serde(default)]
        cancel: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingQuestion {
    pub id: String,
    #[serde(rename = "type")]
    pub question_type: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionAnswer {
    pub id: String,
    pub answer: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "disposition", rename_all = "snake_case", deny_unknown_fields)]
pub enum QuestionResponseValue {
    Answered { answer: serde_json::Value },
    Custom { answer: String },
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuestionResponseItem {
    pub id: String,
    #[serde(flatten)]
    pub value: QuestionResponseValue,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub explanation: String,
}

impl<'de> Deserialize<'de> for QuestionResponseItem {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let mut fields = serde_json::Map::<String, serde_json::Value>::deserialize(deserializer)?;
        let id = fields
            .remove("id")
            .and_then(|v| v.as_str().map(str::to_owned))
            .ok_or_else(|| D::Error::custom("response id must be a string"))?;
        let explanation = match fields.remove("explanation") {
            Some(serde_json::Value::String(value)) => value,
            None => String::new(),
            _ => return Err(D::Error::custom("explanation must be a string")),
        };
        let disposition = fields.remove("disposition");
        let value = match disposition.as_ref().and_then(|v| v.as_str()) {
            Some("answered") => QuestionResponseValue::Answered {
                answer: fields
                    .remove("answer")
                    .ok_or_else(|| D::Error::custom("answer required"))?,
            },
            Some("custom") => QuestionResponseValue::Custom {
                answer: fields
                    .remove("answer")
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .ok_or_else(|| D::Error::custom("custom answer must be a string"))?,
            },
            Some("skipped") => QuestionResponseValue::Skipped,
            _ => return Err(D::Error::custom("invalid response disposition")),
        };
        if !fields.is_empty() {
            return Err(D::Error::custom("unexpected response fields"));
        }
        Ok(Self {
            id,
            value,
            explanation,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionResponse {
    pub answers: Vec<QuestionResponseItem>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub overall_comments: String,
}

impl QuestionResponse {
    pub fn legacy_answers(&self) -> Vec<QuestionAnswer> {
        self.answers
            .iter()
            .map(|item| QuestionAnswer {
                id: item.id.clone(),
                answer: match &item.value {
                    QuestionResponseValue::Answered {
                        answer,
                    } => answer.clone(),
                    QuestionResponseValue::Custom {
                        answer,
                    } => format!("[Custom answer] {answer}").into(),
                    QuestionResponseValue::Skipped => "[Skipped]".into(),
                },
            })
            .collect()
    }
}

pub const QUESTION_RESPONSE_PROTOCOL_VERSION: u8 = 2;

fn question_response_protocol_version() -> u8 {
    QUESTION_RESPONSE_PROTOCOL_VERSION
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingQuestionStatus {
    Pending,
    Answered,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingQuestionSet {
    pub tool_call_id: String,
    pub questions: Vec<PendingQuestion>,
    pub asked_at_ms: u64,
    pub status: PendingQuestionStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub answers: Vec<QuestionAnswer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution_id: Option<String>,
    #[serde(default = "question_response_protocol_version")]
    pub response_protocol_version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<QuestionResponse>,
}

#[cfg(test)]
mod question_response_tests {
    use super::*;

    #[test]
    fn structured_response_roundtrip_preserves_skips_and_explanations() {
        let value = serde_json::json!({
            "answers": [
                {"id":"a","disposition":"custom","answer":"neither","explanation":"why"},
                {"id":"b","disposition":"skipped"},
                {"id":"c","disposition":"answered","answer":false}
            ],
            "overall_comments":"context"
        });
        let response: QuestionResponse = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(response).unwrap(), value);
        assert!(
            serde_json::from_value::<QuestionResponse>(serde_json::json!({
                "answers":[{"id":"a","disposition":"skipped","answer":true}]
            }))
            .is_err()
        );
    }

    #[test]
    fn response_command_requires_instance_and_legacy_answers_are_unchanged() {
        assert!(serde_json::from_value::<ChatCommand>(serde_json::json!({
            "type":"submit_question_response","tool_call_id":"t","response":{"answers":[]}
        }))
        .is_err());
        let legacy: ChatCommand = serde_json::from_value(serde_json::json!({
            "type":"answer_questions","tool_call_id":"t","answers":[{"id":"a","answer":false}]
        }))
        .unwrap();
        assert!(matches!(legacy, ChatCommand::AnswerQuestions { .. }));
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDecisionItem {
    pub tool_call_id: String,
    pub accepted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeApprovalGuard {
    pub project_root: String,
    pub engine_identity: String,
    pub source_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisionCommandGuard {
    pub mandate_id: String,
    pub project_id: String,
    pub worker_id: String,
    pub engine_instance_id: String,
    pub worker_epoch: u64,
    pub owner_epoch: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_approval_guard: Option<NativeApprovalGuard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supervision_guard: Option<SupervisionCommandGuard>,
    pub client_request_id: String,
    #[serde(default)]
    pub priority: bool,
    #[serde(flatten)]
    pub command: ChatCommand,
}

impl CommandRequest {
    pub fn to_queued_item(&self) -> QueuedItem {
        let (command_type, preview, content) = match &self.command {
            ChatCommand::RecoveryRetry {
                ..
            } => ("recovery_retry".into(), String::new(), String::new()),
            ChatCommand::RecoveryRetryNow {
                ..
            } => ("recovery_retry_now".into(), String::new(), String::new()),
            ChatCommand::UserMessage {
                content,
                context_files,
                ..
            } => {
                let full = extract_full_text_capped(content);
                let mut preview = extract_preview(content);
                if !context_files.is_empty() {
                    preview = format!("[+{} ctx] {}", context_files.len(), preview);
                }
                ("user_message".to_string(), preview, full)
            }
            ChatCommand::RetryFromIndex {
                content,
                index,
                ..
            } => (
                "retry_from_index".to_string(),
                format!("@{}: {}", index, extract_preview(content)),
                String::new(),
            ),
            ChatCommand::SetParams {
                patch,
            } => {
                let model = patch.get("model").and_then(|v| v.as_str()).unwrap_or("");
                (
                    "set_params".to_string(),
                    format!("model={}", model),
                    String::new(),
                )
            }
            ChatCommand::SetGoal {
                content, ..
            } => ("set_goal".to_string(), content.clone(), content.clone()),
            ChatCommand::SetGoalBudget {
                ..
            } => ("set_goal_budget".to_string(), String::new(), String::new()),
            ChatCommand::UpdateGoal {
                note,
                plan,
            } => {
                let text = note
                    .clone()
                    .unwrap_or_else(|| serde_json::to_string(plan).unwrap_or_default());
                ("update_goal".to_string(), text.clone(), text)
            }
            ChatCommand::GoalControl {
                action,
            } => ("goal_control".to_string(), action.clone(), String::new()),
            ChatCommand::Abort {} => ("abort".to_string(), String::new(), String::new()),
            ChatCommand::SubmitQuestionResponse {
                tool_call_id, ..
            } => (
                "submit_question_response".into(),
                format!("Response to {tool_call_id}"),
                String::new(),
            ),
            ChatCommand::AnswerQuestions {
                tool_call_id,
                answers,
                ..
            } => (
                "answer_questions".to_string(),
                format!("{} answers", answers.len()),
                tool_call_id.clone(),
            ),
            ChatCommand::CleanBackgroundProcesses {
                include_services,
            } => (
                "clean_background_processes".to_string(),
                format!("include_services={include_services}"),
                String::new(),
            ),
            ChatCommand::ToolDecision {
                tool_call_id,
                accepted,
            } => (
                "tool_decision".to_string(),
                format!("{}: {}", tool_call_id, accepted),
                String::new(),
            ),
            ChatCommand::ToolDecisions {
                decisions,
            } => (
                "tool_decisions".to_string(),
                format!("{} decisions", decisions.len()),
                String::new(),
            ),
            ChatCommand::IdeToolResult {
                tool_call_id, ..
            } => (
                "ide_tool_result".to_string(),
                tool_call_id.clone(),
                String::new(),
            ),
            ChatCommand::UpdateMessage {
                message_id, ..
            } => (
                "update_message".to_string(),
                message_id.clone(),
                String::new(),
            ),
            ChatCommand::RemoveMessage {
                message_id, ..
            } => (
                "remove_message".to_string(),
                message_id.clone(),
                String::new(),
            ),
            ChatCommand::Regenerate {} => ("regenerate".to_string(), String::new(), String::new()),
            ChatCommand::RestoreMessages {
                messages,
            } => (
                "restore_messages".to_string(),
                format!("{} messages", messages.len()),
                String::new(),
            ),
            ChatCommand::BranchFromChat {
                source_chat_id, ..
            } => (
                "branch_from_chat".to_string(),
                source_chat_id.clone(),
                String::new(),
            ),
            ChatCommand::BrowserContextDecision {
                pending_message_id,
                ..
            } => (
                "browser_context_decision".to_string(),
                pending_message_id.clone(),
                String::new(),
            ),
            ChatCommand::DeliverMessages {
                delivery,
            } => {
                return QueuedItem::from_pending_delivery(delivery);
            }
            ChatCommand::UpdatePendingDelivery {
                delivery_id,
                push,
                cancel,
            } => (
                "update_pending_delivery".to_string(),
                if *cancel {
                    format!("cancel {delivery_id}")
                } else {
                    format!("{delivery_id} -> {}", push.unwrap_or_default().as_str())
                },
                String::new(),
            ),
        };
        QueuedItem {
            client_request_id: self.client_request_id.clone(),
            priority: self.priority,
            command_type,
            preview,
            content,
            push: None,
            source: None,
            event: None,
            enqueued_at_ms: None,
        }
    }
}

const MAX_CONTENT_CHARS: usize = 8192;

fn extract_full_text(content: &serde_json::Value) -> String {
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    if let Some(arr) = content.as_array() {
        return arr
            .iter()
            .find_map(|item| {
                if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                    item.get("text").and_then(|t| t.as_str()).map(String::from)
                } else {
                    None
                }
            })
            .unwrap_or_default();
    }
    String::new()
}

fn extract_full_text_capped(content: &serde_json::Value) -> String {
    let text = extract_full_text(content);
    if text.chars().count() > MAX_CONTENT_CHARS {
        format!(
            "{}…",
            text.chars().take(MAX_CONTENT_CHARS).collect::<String>()
        )
    } else {
        text
    }
}

fn extract_preview(content: &serde_json::Value) -> String {
    const MAX_PREVIEW: usize = 120;
    let text = if let Some(s) = content.as_str() {
        s.to_string()
    } else if let Some(arr) = content.as_array() {
        arr.iter()
            .find_map(|item| {
                if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                    item.get("text").and_then(|t| t.as_str()).map(String::from)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "[Image attachment]".to_string())
    } else {
        String::new()
    };
    if text.chars().count() > MAX_PREVIEW {
        format!("{}…", text.chars().take(MAX_PREVIEW).collect::<String>())
    } else {
        text
    }
}

#[derive(Debug, Clone, Default)]
pub struct ActiveCommandContext {
    pub name: String,
    pub allowed_tools: Vec<String>,
    pub model_override: Option<String>,
    pub context_fork: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_criterion_verdict_without_a_source_reads_as_judged() {
        let verdict: CriterionVerdict =
            serde_json::from_value(json!({"id": "C1", "met": true})).unwrap();

        assert_eq!(verdict.source, VerdictSource::Judged);
        assert!(!verdict.source.is_measured());
        assert!(verdict.evidence.is_none());
        assert_eq!(CriterionVerdict::default().source, VerdictSource::Judged);
    }

    #[test]
    fn a_measured_criterion_verdict_carries_its_evidence_round_trip() {
        let verdict = CriterionVerdict {
            id: "C1".to_string(),
            met: false,
            note: "cargo test --lib exited 1 [failed]".to_string(),
            source: VerdictSource::Measured,
            evidence: Some(CriterionEvidence {
                spec_id: "criterion:C1".to_string(),
                command: "cargo test --lib".to_string(),
                exit_code: Some(1),
                verdict: "failed".to_string(),
            }),
        };

        let encoded = serde_json::to_value(&verdict).unwrap();

        assert_eq!(encoded["source"], json!("measured"));
        assert_eq!(encoded["evidence"]["exit_code"], json!(1));
        assert_eq!(
            serde_json::from_value::<CriterionVerdict>(encoded).unwrap(),
            verdict
        );
        assert_eq!(VerdictSource::Measured.label(), "measured");
        assert_eq!(VerdictSource::Judged.label(), "judged");
    }

    #[test]
    fn a_criterion_keeps_prose_hints_and_executable_commands_apart() {
        let criterion: GoalCriterion = serde_json::from_value(json!({
            "id": "C1",
            "text": "tests pass",
            "verify_hint": "make sure the tests pass",
            "verify_command": "cargo test --lib",
        }))
        .unwrap();

        assert_eq!(
            criterion.verify_hint.as_deref(),
            Some("make sure the tests pass")
        );
        assert_eq!(
            criterion.verify_command.as_deref(),
            Some("cargo test --lib")
        );

        let hint_only: GoalCriterion =
            serde_json::from_value(json!({"id": "C2", "text": "docs", "verify_hint": "read them"}))
                .unwrap();
        assert!(hint_only.verify_command.is_none());
    }

    fn background_agent_summary() -> BackgroundAgentSummary {
        BackgroundAgentSummary {
            agent_id: "bgagent-1".to_string(),
            parent_chat_id: "parent-chat".to_string(),
            child_chat_id: Some("child-chat".to_string()),
            kind: "delegate".to_string(),
            status: "waiting_for_approval".to_string(),
            title: "Patch frog pond".to_string(),
            progress: Some("Inspecting reeds".to_string()),
            step_count: 3,
            last_activity: Some("reading files".to_string()),
            target_files: vec!["src/frog.rs".to_string()],
            edited_files: vec!["src/frog.rs".to_string()],
            diff_summary: Some("one frog changed".to_string()),
            conflict_summary: None,
            result_summary: Some("frog patched".to_string()),
            error: None,
            started_at: Some("2026-05-27T00:00:00Z".to_string()),
            finished_at: None,
            change_seq: 7,
            model: "test-model".to_string(),
            model_type: Some("thinking".to_string()),
            current_tool: Some("cat: src/frog.rs".to_string()),
            goal_summary: Some("Patch the frog pond".to_string()),
            plan_present: true,
            worktree_branch: Some("refact/subagent/frogs".to_string()),
            merge_status: Some("pending".to_string()),
            pending_questions: 0,
            tokens_used: 123,
            cost_usd: Some(0.42),
            questions: Vec::new(),
            deliverable: None,
            deliverable_status: None,
            handoff: None,
        }
    }

    fn finite_goal_budget() -> GoalBudget {
        GoalBudget {
            max_turns: Some(10),
            max_minutes: Some(15),
            max_tokens: Some(200_000),
            max_cost_cents: None,
            cooldown_ms: 1_500,
            no_progress_token_threshold: 50,
            no_progress_turns: Some(2),
            explicit: false,
        }
    }

    fn goal_snapshot() -> GoalSnapshot {
        GoalSnapshot {
            owner: None,
            revision: 0,
            plan: None,
            content: "Ship the frog pond".to_string(),
            version: 2,
            active: true,
            status: GoalStatus::Verifying,
            budget: GoalBudget::default(),
            progress: GoalProgress {
                turns_used: 3,
                tokens_used: 1234,
                started_at_ms: 10,
                no_progress_turns: 1,
                last_nudge_at_ms: 20,
                cost_used_cents: 0,
            },
            attempts: vec![GoalAttempt {
                at_ms: 30,
                trigger: "done".to_string(),
                verdict: "needs_work".to_string(),
                gaps: vec!["tests".to_string()],
                verifier_reply: "Run tests".to_string(),
                criteria_verdicts: Vec::new(),
            }],
            events: vec![GoalEvent {
                at_ms: 40,
                kind: "delta".to_string(),
                text: "Added verification".to_string(),
            }],
            criteria: Vec::new(),
            snoozed_until_ms: None,
            wait: None,
            stop_reason: None,
            transferred_from: Some("source-chat".to_string()),
            transferred_to: Some("target-chat".to_string()),
            sub_goals: sub_goal::SubGoalGraph::default(),
        }
    }

    /// A wait is persisted state: every subject must survive the round trip with
    /// its identity intact, or a reloaded chat cannot tell what it waits on.
    #[test]
    fn a_goal_wait_round_trips_with_its_typed_subject() {
        let subjects = [
            GoalWaitSubject::Process {
                process_id: "proc-7".to_string(),
            },
            GoalWaitSubject::Child {
                agent_id: "bgagent-3".to_string(),
            },
            GoalWaitSubject::Event {
                name: "ci-run-42".to_string(),
            },
            GoalWaitSubject::Deadline {
                until_ms: 1_730_000_000_000,
            },
        ];
        for subject in subjects {
            let mut goal = goal_snapshot();
            goal.wait = Some(GoalWait::new(subject.clone(), None));
            let restored: GoalSnapshot =
                serde_json::from_str(&serde_json::to_string(&goal).unwrap()).unwrap();
            assert_eq!(restored.wait.as_ref().unwrap().subject, subject);
            assert_eq!(restored, goal);
        }

        let deadline_only = GoalWait::new(
            GoalWaitSubject::Deadline {
                until_ms: 5_000,
            },
            None,
        );
        assert_eq!(deadline_only.deadline_ms, Some(5_000));
        assert!(!deadline_only.expired_at(4_999));
        assert!(deadline_only.expired_at(5_000));

        let unbounded = GoalWait::new(
            GoalWaitSubject::Event {
                name: "ci".to_string(),
            },
            None,
        );
        assert!(!unbounded.expired_at(u64::MAX));
    }

    #[test]
    fn test_goal_snapshot_roundtrip_full() {
        let snapshot = goal_snapshot();
        let json = serde_json::to_value(&snapshot).unwrap();

        assert_eq!(json["content"], "Ship the frog pond");
        assert_eq!(json["status"], "verifying");
        assert!(json["budget"].get("max_turns").is_none());
        assert!(json["budget"].get("max_minutes").is_none());
        assert!(json["budget"].get("max_tokens").is_none());
        assert_eq!(json["budget"]["cooldown_ms"], 30_000);
        assert_eq!(json["budget"]["no_progress_token_threshold"], 50);
        assert!(json["budget"].get("no_progress_turns").is_none());
        assert_eq!(json["progress"]["turns_used"], 3);
        assert_eq!(json["attempts"][0]["gaps"][0], "tests");
        assert_eq!(json["events"][0]["text"], "Added verification");

        let roundtrip: GoalSnapshot = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip, snapshot);
    }

    #[test]
    fn test_goal_budget_default_omits_hard_limits_and_roundtrips_unlimited() {
        let budget = GoalBudget::default();
        let json = serde_json::to_value(&budget).unwrap();

        assert!(json.get("max_turns").is_none());
        assert!(json.get("max_minutes").is_none());
        assert!(json.get("max_tokens").is_none());
        assert_eq!(json["cooldown_ms"], 30_000);
        assert_eq!(json["no_progress_token_threshold"], 50);
        assert!(json.get("no_progress_turns").is_none());
        assert!(json.get("explicit").is_none());

        let roundtrip: GoalBudget = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip, budget);
    }

    #[test]
    fn test_goal_budget_finite_roundtrips_unchanged() {
        let budget = finite_goal_budget();
        let json = serde_json::to_value(&budget).unwrap();

        assert_eq!(json["max_turns"], 10);
        assert_eq!(json["max_minutes"], 15);
        assert_eq!(json["max_tokens"], 200_000);
        assert_eq!(json["cooldown_ms"], 1_500);
        assert_eq!(json["no_progress_token_threshold"], 50);
        assert_eq!(json["no_progress_turns"], 2);
        assert!(json.get("explicit").is_none());

        let roundtrip: GoalBudget = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip, budget);
    }

    #[test]
    fn test_goal_budget_explicit_legacy_default_does_not_migrate() {
        let mut budget = GoalBudget::legacy_default_hard_limits();
        budget.explicit = true;

        assert_eq!(budget.clone().migrate_legacy_default_hard_limits(), budget);
    }

    #[test]
    fn test_goal_budget_legacy_cooldown_migrates_for_implicit_budgets_only() {
        let implicit = GoalBudget {
            cooldown_ms: 1_500,
            ..Default::default()
        };
        assert_eq!(
            implicit.migrate_legacy_default_hard_limits().cooldown_ms,
            30_000
        );

        let finite = GoalBudget {
            cooldown_ms: 1_500,
            max_turns: Some(5),
            ..Default::default()
        };
        assert_eq!(
            finite.migrate_legacy_default_hard_limits().cooldown_ms,
            1_500
        );

        let legacy_tuple = GoalBudget::legacy_default_hard_limits();
        let legacy_tuple = GoalBudget {
            cooldown_ms: 1_500,
            ..legacy_tuple
        };
        assert_eq!(
            legacy_tuple
                .migrate_legacy_default_hard_limits()
                .cooldown_ms,
            30_000
        );

        let custom = GoalBudget {
            cooldown_ms: 1_000,
            ..Default::default()
        };
        assert_eq!(
            custom.migrate_legacy_default_hard_limits().cooldown_ms,
            1_000
        );

        let explicit = GoalBudget {
            cooldown_ms: 1_500,
            explicit: true,
            ..Default::default()
        };
        assert_eq!(
            explicit.migrate_legacy_default_hard_limits().cooldown_ms,
            1_500
        );
    }

    #[test]
    fn test_goal_budget_explicit_serializes_and_roundtrips() {
        let mut budget = GoalBudget::default();
        budget.explicit = true;
        let json = serde_json::to_value(&budget).unwrap();

        assert_eq!(json["explicit"], true);

        let roundtrip: GoalBudget = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip, budget);
    }

    #[test]
    fn new_chat_cap_is_90_percent_of_smallest_positive_window() {
        assert_eq!(
            default_auto_compression_cap(Some(1000), Some(800)),
            Some(720)
        );
        assert_eq!(default_auto_compression_cap(Some(0), Some(1000)), Some(900));
        assert_eq!(default_auto_compression_cap(None, Some(0)), None);
        assert_eq!(default_auto_compression_cap(Some(1), None), Some(0));
        let mut params = ThreadParams::default();
        params.initialize_new_chat_compression_cap(Some(1000), None);
        assert_eq!(params.auto_compression_cap, Some(900));
        params.initialize_new_chat_compression_cap(Some(2000), None);
        assert_eq!(
            params.auto_compression_cap,
            Some(900),
            "model switch preserves stored cap"
        );
        params.auto_compression_cap = None;
        params.initialize_new_chat_compression_cap(Some(2000), None);
        assert_eq!(
            params.auto_compression_cap,
            Some(1800),
            "explicit reset recomputes"
        );
        params.auto_compression_cap = Some(0);
        params.initialize_new_chat_compression_cap(Some(2000), None);
        assert_eq!(
            params.auto_compression_cap,
            Some(0),
            "explicit zero is preserved"
        );
        let old: ThreadParams =
            serde_json::from_value(serde_json::to_value(ThreadParams::default()).unwrap()).unwrap();
        assert_eq!(
            old.auto_compression_cap, None,
            "loading does not initialize defaults"
        );
    }

    #[test]
    fn legacy_unset_cap_never_initializes_even_when_model_arrives() {
        let mut value = serde_json::to_value(ThreadParams::default()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("auto_compression_cap_pending");
        let mut legacy: ThreadParams = serde_json::from_value(value).unwrap();
        assert!(!legacy.auto_compression_cap_pending);
        assert!(!legacy.resolve_pending_compression_cap(Some(1000)));
        assert_eq!(legacy.auto_compression_cap, None);
    }

    #[test]
    fn new_chat_compression_cap_waits_for_model_and_resolves_once() {
        let mut params = ThreadParams {
            context_tokens_cap: Some(800),
            ..Default::default()
        };
        assert!(params.auto_compression_cap_pending);
        assert!(!params.resolve_pending_compression_cap(None));
        assert_eq!(params.auto_compression_cap, None);
        // A restart before the model is known retains new-chat provenance.
        let mut params: ThreadParams =
            serde_json::from_value(serde_json::to_value(params).unwrap()).unwrap();
        assert!(params.resolve_pending_compression_cap(Some(1000)));
        assert_eq!(params.auto_compression_cap, Some(720));
        assert!(!params.auto_compression_cap_pending);
        assert!(!params.resolve_pending_compression_cap(Some(2000)));
        assert_eq!(params.auto_compression_cap, Some(720));
        assert_eq!(default_auto_compression_cap(Some(19), None), Some(17));
        assert_eq!(
            default_auto_compression_cap(Some(usize::MAX), None),
            Some(usize::MAX / 10 * 9 + usize::MAX % 10 * 9 / 10)
        );
    }

    #[test]
    fn new_chat_compression_cap_preserves_explicit_including_zero() {
        for cap in [0, 123, 2000] {
            let mut params = ThreadParams {
                auto_compression_cap: Some(cap),
                ..Default::default()
            };
            assert!(params.resolve_pending_compression_cap(None));
            assert_eq!(params.auto_compression_cap, Some(cap));
            assert!(!params.auto_compression_cap_pending);
            assert!(!params.resolve_pending_compression_cap(Some(1000)));
            assert_eq!(params.auto_compression_cap, Some(cap));
        }
    }

    #[test]
    fn test_auto_compact_effective_defaults_to_enabled() {
        assert!(ThreadParams::default().auto_compact_enabled_effective());

        let unset = ThreadParams {
            auto_compact_enabled: None,
            ..Default::default()
        };
        assert!(unset.auto_compact_enabled_effective());

        let disabled = ThreadParams {
            auto_compact_enabled: Some(false),
            ..Default::default()
        };
        assert!(!disabled.auto_compact_enabled_effective());
    }

    #[test]
    fn test_auto_compact_missing_in_json_is_effectively_enabled() {
        let json = r#"{
            "id":"test",
            "title":"Test",
            "model":"gpt-4",
            "mode":"agent",
            "tool_use":"agent",
            "include_project_info":true,
            "checkpoints_enabled":true
        }"#;

        let params: ThreadParams = serde_json::from_str(json).unwrap();
        assert!(params.auto_compact_enabled.is_none());
        assert!(params.auto_compact_enabled_effective());
        assert!(params.auto_compression_cap.is_none());
    }

    fn delivery_with(messages: Vec<ChatMessage>) -> PendingDelivery {
        PendingDelivery::with_id(
            "delivery-1",
            messages,
            PushMode::Append,
            "agents.push",
            false,
        )
    }

    #[test]
    fn test_event_envelope_seq_deserializes_from_string() {
        let json = r#"{"chat_id":"abc","seq":"999","type":"pause_cleared"}"#;
        let envelope: EventEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(envelope.seq, 999);
        assert_eq!(envelope.chat_id, "abc");
    }

    #[test]
    fn test_event_envelope_invalid_seq_fails() {
        let json = r#"{"chat_id":"abc","seq":"not_a_number","type":"pause_cleared"}"#;
        let result: Result<EventEnvelope, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_chat_command_user_message_defaults() {
        let json = r#"{"type":"user_message","content":"hello"}"#;
        let cmd: ChatCommand = serde_json::from_str(json).unwrap();
        match cmd {
            ChatCommand::UserMessage {
                content,
                attachments,
                context_files,
                client_message_id,
                suppress_auto_enrichment: _,
            } => {
                assert_eq!(content, json!("hello"));
                assert!(attachments.is_empty());
                assert!(context_files.is_empty());
                assert_eq!(client_message_id, None);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_client_message_id_validation_bounds() {
        assert!(validate_client_message_id(None).is_ok());
        assert!(validate_client_message_id(Some("optimistic-1")).is_ok());
        assert!(validate_client_message_id(Some("")).is_err());
        assert!(
            validate_client_message_id(Some(&"a".repeat(MAX_CLIENT_MESSAGE_ID_CHARS + 1))).is_err()
        );
    }

    #[test]
    fn test_chat_command_all_variants_roundtrip() {
        let commands = vec![
            json!({"type":"user_message","content":"hi","attachments":[]}),
            json!({"type":"retry_from_index","index":2,"content":"retry","attachments":[]}),
            json!({"type":"set_params","patch":{"title":"New"}}),
            json!({"type":"set_goal","content":"finish the pond"}),
            json!({"type":"set_goal_budget","budget":{"max_turns":3,"cooldown_ms":1500,"no_progress_token_threshold":50}}),
            json!({"type":"update_goal","note":"verify the reeds"}),
            json!({"type":"goal_control","action":"pause"}),
            json!({"type":"abort"}),
            json!({"type":"tool_decision","tool_call_id":"tc1","accepted":true}),
            json!({"type":"tool_decisions","decisions":[{"tool_call_id":"tc1","accepted":false}]}),
            json!({"type":"ide_tool_result","tool_call_id":"tc1","content":"ok","tool_failed":false}),
            json!({"type":"update_message","message_id":"m1","content":"x","attachments":[],"regenerate":true}),
            json!({"type":"remove_message","message_id":"m1","regenerate":false}),
        ];
        for cmd_json in commands {
            let cmd: ChatCommand = serde_json::from_value(cmd_json.clone()).unwrap();
            let roundtrip = serde_json::to_value(&cmd).unwrap();
            assert_eq!(roundtrip["type"], cmd_json["type"]);
        }
    }

    #[test]
    fn test_chat_command_goal_variants_roundtrip() {
        let commands = vec![
            json!({"type":"set_goal","content":"finish the pond"}),
            json!({"type":"set_goal_budget","budget":{"max_turns":3,"cooldown_ms":1500,"no_progress_token_threshold":50}}),
            json!({"type":"update_goal","note":"verify the reeds"}),
            json!({"type":"goal_control","action":"resume"}),
        ];

        for cmd_json in commands {
            let cmd: ChatCommand = serde_json::from_value(cmd_json.clone()).unwrap();
            let roundtrip = serde_json::to_value(&cmd).unwrap();
            assert_eq!(roundtrip, cmd_json);
        }
    }

    #[test]
    fn test_snapshot_missing_goal_defaults_none() {
        let json = r#"{
            "type":"snapshot",
            "thread":{
                "id":"test",
                "title":"Test",
                "model":"gpt-4",
                "mode":"agent",
                "tool_use":"agent",
                "context_tokens_cap":null,
                "include_project_info":true,
                "checkpoints_enabled":true
            },
            "runtime":{
                "state":"idle",
                "paused":false,
                "error":null,
                "queue_size":0
            },
            "messages":[],
            "background_agents":[]
        }"#;

        let event: ChatEvent = serde_json::from_str(json).unwrap();
        match event {
            ChatEvent::Snapshot {
                runtime,
                goal,
                pending_questions,
                ..
            } => {
                assert!(!runtime.goal_active);
                assert_eq!(runtime.goal_status, None);
                assert_eq!(runtime.goal_turns_used, 0);
                assert_eq!(runtime.goal_tokens_used, 0);
                assert_eq!(runtime.goal_no_progress_turns, 0);
                assert_eq!(goal, None);
                assert!(pending_questions.is_empty());
            }
            _ => panic!("Expected Snapshot"),
        }
    }

    #[test]
    fn test_command_request_flattens_command() {
        let req = CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "req-1".into(),
            priority: false,
            command: ChatCommand::Abort {},
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["client_request_id"], "req-1");
        assert_eq!(json["type"], "abort");
    }

    #[test]
    fn test_set_goal_command_budget_is_optional() {
        let cmd: ChatCommand =
            serde_json::from_str(r#"{"type":"set_goal","content":"ship"}"#).unwrap();
        match cmd {
            ChatCommand::SetGoal {
                content,
                budget,
                ..
            } => {
                assert_eq!(content, "ship");
                assert_eq!(budget, None);
            }
            other => panic!("expected SetGoal, got {other:?}"),
        }

        let budget = finite_goal_budget();
        let cmd = ChatCommand::SetGoal {
            plan: None,
            criteria: None,
            content: "ship".to_string(),
            budget: Some(budget.clone()),
        };
        let json = serde_json::to_value(&cmd).unwrap();
        assert_eq!(json["type"], "set_goal");
        assert_eq!(json["budget"], json!(budget));
    }

    #[test]
    fn test_set_goal_budget_command_serde_and_queue_preview() {
        let budget = GoalBudget {
            max_turns: Some(4),
            max_minutes: None,
            max_tokens: None,
            max_cost_cents: None,
            cooldown_ms: 1_500,
            no_progress_token_threshold: 50,
            no_progress_turns: None,
            explicit: false,
        };
        let req = CommandRequest {
            native_approval_guard: None,
            supervision_guard: None,
            client_request_id: "req-budget".into(),
            priority: false,
            command: ChatCommand::SetGoalBudget {
                budget: budget.clone(),
            },
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["type"], "set_goal_budget");
        assert_eq!(json["budget"], json!(budget));

        let roundtrip: CommandRequest = serde_json::from_value(json).unwrap();
        match roundtrip.command {
            ChatCommand::SetGoalBudget {
                budget: parsed,
            } => assert_eq!(parsed, budget),
            other => panic!("expected SetGoalBudget, got {other:?}"),
        }

        let queued = req.to_queued_item();
        assert_eq!(queued.command_type, "set_goal_budget");
        assert!(queued.preview.is_empty());
        assert!(queued.content.is_empty());
    }

    #[test]
    fn test_runtime_updated_serde() {
        let event = ChatEvent::RuntimeUpdated {
            interruption: None,
            waiting_interruptible: false,
            state: SessionState::Completed,
            error: None,
            goal_active: false,
            goal_status: None,
            goal_turns_used: 0,
            goal_tokens_used: 0,
            goal_no_progress_turns: 0,
            is_compressing: false,
            compression_phase: None,
            compression_reason: None,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "runtime_updated");
        assert_eq!(json["state"], "completed");
        assert!(json.get("error").is_none());
        assert_eq!(json["is_compressing"], false);

        let event_with_error = ChatEvent::RuntimeUpdated {
            interruption: None,
            waiting_interruptible: false,
            state: SessionState::Error,
            error: Some("test error".into()),
            goal_active: true,
            goal_status: Some(GoalStatus::NoProgress),
            goal_turns_used: 4,
            goal_tokens_used: 5678,
            goal_no_progress_turns: 2,
            is_compressing: true,
            compression_phase: Some(CompressionPhase::Failed),
            compression_reason: Some(CompressionReason::TransientFailure),
        };
        let json2 = serde_json::to_value(&event_with_error).unwrap();
        assert_eq!(json2["type"], "runtime_updated");
        assert_eq!(json2["state"], "error");
        assert_eq!(json2["error"], "test error");
        assert_eq!(json2["goal_active"], true);
        assert_eq!(json2["goal_status"], "no_progress");
        assert_eq!(json2["goal_turns_used"], 4);
        assert_eq!(json2["goal_tokens_used"], 5678);
        assert_eq!(json2["goal_no_progress_turns"], 2);
        assert_eq!(json2["is_compressing"], true);
        assert_eq!(json2["compression_phase"], "failed");
        assert_eq!(json2["compression_reason"], "transient_failure");
    }

    #[test]
    fn test_runtime_updated_missing_goal_and_compression_fields_defaults() {
        let json = r#"{
            "type":"runtime_updated",
            "state":"completed"
        }"#;

        let event: ChatEvent = serde_json::from_str(json).unwrap();

        match event {
            ChatEvent::RuntimeUpdated {
                state,
                error,
                goal_active,
                goal_status,
                goal_turns_used,
                goal_tokens_used,
                goal_no_progress_turns,
                is_compressing,
                compression_phase,
                compression_reason,
                waiting_interruptible,
                interruption,
            } => {
                assert!(interruption.is_none());
                assert!(!waiting_interruptible);
                assert_eq!(state, SessionState::Completed);
                assert_eq!(error, None);
                assert!(!goal_active);
                assert_eq!(goal_status, None);
                assert_eq!(goal_turns_used, 0);
                assert_eq!(goal_tokens_used, 0);
                assert_eq!(goal_no_progress_turns, 0);
                assert!(!is_compressing);
                assert_eq!(compression_phase, None);
                assert_eq!(compression_reason, None);
            }
            other => panic!("expected RuntimeUpdated, got {other:?}"),
        }
    }

    #[test]
    fn test_browser_meta_serde_roundtrip_minimal() {
        let json_str = r#"{}"#;
        let meta: BrowserMeta = serde_json::from_str(json_str).unwrap();
        assert!(meta.browser_runtime_id.is_none());
        assert!(meta.profile_dir.is_none());
        assert!(meta.tab_urls.is_empty());
        assert!(meta.active_tab_id.is_none());
        assert!(meta.window_bounds.is_none());
        assert!(!meta.attach_screenshot_on_send);
        assert!(meta.mask_passwords);
    }

    #[test]
    fn test_browser_events_in_event_envelope() {
        let envelope = EventEnvelope {
            chat_id: "chat-1".to_string(),
            seq: 5,
            event: ChatEvent::BrowserFrame {
                tab_id: "t1".to_string(),
                mime: "image/jpeg".to_string(),
                data: "base64".to_string(),
                diff_boxes: vec![],
                changed_text: None,
            },
        };
        let json = serde_json::to_value(&envelope).unwrap();
        assert_eq!(json["chat_id"], "chat-1");
        assert_eq!(json["seq"], "5");
        assert_eq!(json["type"], "browser_frame");
    }
}
