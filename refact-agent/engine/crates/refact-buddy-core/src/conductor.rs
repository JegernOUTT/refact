use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ConductorGoal {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_doc_slug: Option<String>,
    pub plan_markdown: String,
    pub done_when: DoneWhen,
    pub status: GoalStatus,
    pub autonomy: GoalAutonomy,
    pub budget: GoalBudget,
    pub spent: GoalBudgetSpent,
    pub ledger: GoalLedger,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

impl Default for ConductorGoal {
    fn default() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            plan_doc_slug: None,
            plan_markdown: String::new(),
            done_when: DoneWhen::default(),
            status: GoalStatus::default(),
            autonomy: GoalAutonomy::default(),
            budget: GoalBudget::default(),
            spent: GoalBudgetSpent::default(),
            ledger: GoalLedger::default(),
            created_at: None,
            updated_at: None,
            completed_at: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalStatus {
    Proposed,
    Active,
    Paused,
    Escalated,
    Done,
    Abandoned,
}

impl GoalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Escalated => "escalated",
            Self::Done => "done",
            Self::Abandoned => "abandoned",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "proposed" | "planned" => Some(Self::Proposed),
            "active" | "running" => Some(Self::Active),
            "paused" | "waiting_for_human" => Some(Self::Paused),
            "escalated" | "failed" => Some(Self::Escalated),
            "done" => Some(Self::Done),
            "abandoned" | "cancelled" => Some(Self::Abandoned),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Escalated | Self::Abandoned)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalTransitionError {
    pub from: GoalStatus,
    pub to: GoalStatus,
}

impl fmt::Display for GoalTransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid conductor goal status transition: {} -> {}",
            self.from.as_str(),
            self.to.as_str()
        )
    }
}

impl std::error::Error for GoalTransitionError {}

pub fn validate_goal_status_transition(
    from: GoalStatus,
    to: GoalStatus,
) -> Result<(), GoalTransitionError> {
    let allowed = match from {
        GoalStatus::Proposed => matches!(
            to,
            GoalStatus::Active | GoalStatus::Paused | GoalStatus::Abandoned
        ),
        GoalStatus::Active => matches!(
            to,
            GoalStatus::Paused | GoalStatus::Done | GoalStatus::Escalated | GoalStatus::Abandoned
        ),
        GoalStatus::Paused => matches!(to, GoalStatus::Active | GoalStatus::Abandoned),
        GoalStatus::Done | GoalStatus::Escalated | GoalStatus::Abandoned => to == from,
    };
    if allowed || from == to {
        Ok(())
    } else {
        Err(GoalTransitionError { from, to })
    }
}

impl Default for GoalStatus {
    fn default() -> Self {
        Self::Active
    }
}

impl Serialize for GoalStatus {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for GoalStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).ok_or_else(|| {
            de::Error::unknown_variant(
                &value,
                &[
                    "proposed",
                    "active",
                    "paused",
                    "escalated",
                    "done",
                    "abandoned",
                ],
            )
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoalAutonomy {
    ReadOnly,
    Governed,
    FullAuto,
}

impl Default for GoalAutonomy {
    fn default() -> Self {
        Self::FullAuto
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GoalBudget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_clock_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_progress_wakes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(alias = "token_ceiling")]
    pub total_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(alias = "usd_ceiling")]
    pub usd: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GoalBudgetSpent {
    pub elapsed_secs: u64,
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub cache_read_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usd: Option<f64>,
    pub no_progress_wakes: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct GoalBudgetWakeBuckets {
    pub wall_clock_secs: u8,
    pub no_progress_wakes: u8,
    pub total_tokens: u8,
    pub usd: u8,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct LearningBudgetSnapshot {
    pub elapsed_secs: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub cache_read_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usd: Option<String>,
}

impl From<&GoalBudgetSpent> for LearningBudgetSnapshot {
    fn from(spent: &GoalBudgetSpent) -> Self {
        Self {
            elapsed_secs: spent.elapsed_secs,
            prompt_tokens: spent.prompt_tokens,
            completion_tokens: spent.completion_tokens,
            total_tokens: spent.total_tokens,
            cache_read_tokens: spent.cache_read_tokens,
            usd: spent.usd.map(|usd| format!("{usd:.6}")),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DoneWhen {
    pub summary: String,
    pub checklist: Vec<String>,
}

impl DoneWhen {
    pub fn has_completion_criteria(&self) -> bool {
        !self.summary.trim().is_empty() || self.checklist.iter().any(|item| !item.trim().is_empty())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ConductorMemo {
    pub id: String,
    pub kind: MemoKind,
    pub content: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_task_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ConductorLearningRecord {
    pub id: String,
    pub outcome: LearningOutcome,
    pub goal_id: String,
    pub goal_title: String,
    pub summary: String,
    pub what_worked: Vec<String>,
    pub failures: Vec<String>,
    pub budget_used: LearningBudgetSnapshot,
    pub no_progress_wakes: u32,
    pub useful_tools_or_strategies: Vec<String>,
    pub future_tunables: Vec<String>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_task_id: Option<String>,
}

impl Default for ConductorLearningRecord {
    fn default() -> Self {
        Self {
            id: String::new(),
            outcome: LearningOutcome::default(),
            goal_id: String::new(),
            goal_title: String::new(),
            summary: String::new(),
            what_worked: Vec::new(),
            failures: Vec::new(),
            budget_used: LearningBudgetSnapshot::default(),
            no_progress_wakes: 0,
            useful_tools_or_strategies: Vec::new(),
            future_tunables: Vec::new(),
            created_at: String::new(),
            source_chat_id: None,
            related_task_id: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LearningOutcome {
    Done,
    Escalated,
}

impl Default for LearningOutcome {
    fn default() -> Self {
        Self::Done
    }
}

impl Default for ConductorMemo {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: MemoKind::default(),
            content: String::new(),
            created_at: String::new(),
            source_chat_id: None,
            related_task_id: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoKind {
    Progress,
    Decision,
    Risk,
    Handoff,
    HumanSteering,
    Surgery,
    Escalation,
}

impl Default for MemoKind {
    fn default() -> Self {
        Self::Progress
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GoalLedger {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_doc_slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_markdown: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_when: Option<DoneWhen>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<GoalBudget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<GoalStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autonomy: Option<GoalAutonomy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planner_task_id: Option<String>,
    pub task_ids: Vec<String>,
    pub chat_ids: Vec<String>,
    pub memos: Vec<ConductorMemo>,
    pub learning_records: Vec<ConductorLearningRecord>,
    pub ghost_messages: Vec<crate::types::BuddyGhostMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurring: Option<ConductorRecurring>,
    pub pending_questions: Vec<PendingQuestion>,
    #[serde(default)]
    pub no_progress_wakes: u32,
    #[serde(default)]
    pub turn_failures: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_wake_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_progress_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_wake_reason: Option<ConductorWakeReason>,
    #[serde(default, skip_serializing_if = "GoalBudgetWakeBuckets::is_empty")]
    pub budget_wake_buckets: GoalBudgetWakeBuckets,
}

impl GoalBudgetWakeBuckets {
    pub fn is_empty(&self) -> bool {
        self.wall_clock_secs == 0
            && self.no_progress_wakes == 0
            && self.total_tokens == 0
            && self.usd == 0
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ConductorRecurring {
    pub enabled: bool,
    pub cron: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_enqueued_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_after_secs: Option<u64>,
}

impl GoalLedger {
    pub fn apply_goal_metadata(&mut self, goal: &ConductorGoal) {
        self.title = Some(goal.title.clone()).filter(|value| !value.trim().is_empty());
        self.plan_doc_slug = goal
            .plan_doc_slug
            .clone()
            .filter(|value| !value.trim().is_empty());
        self.plan_markdown =
            Some(goal.plan_markdown.clone()).filter(|value| !value.trim().is_empty());
        self.done_when = Some(goal.done_when.clone());
        self.budget = Some(goal.budget.clone());
        self.status = Some(goal.status);
        self.autonomy = Some(goal.autonomy);
        self.created_at = goal.created_at.clone();
        self.updated_at = goal.updated_at.clone();
        self.completed_at = goal.completed_at.clone();
    }
}

impl ConductorGoal {
    pub fn from_ledger(goal_id: String, ledger: GoalLedger) -> Self {
        Self {
            id: goal_id.clone(),
            title: ledger.title.clone().unwrap_or(goal_id),
            plan_doc_slug: ledger.plan_doc_slug.clone(),
            plan_markdown: ledger.plan_markdown.clone().unwrap_or_default(),
            done_when: ledger.done_when.clone().unwrap_or_default(),
            status: ledger.status.unwrap_or_default(),
            autonomy: ledger.autonomy.unwrap_or_default(),
            budget: ledger.budget.clone().unwrap_or_default(),
            spent: GoalBudgetSpent {
                no_progress_wakes: ledger.no_progress_wakes,
                ..GoalBudgetSpent::default()
            },
            created_at: ledger.created_at.clone(),
            updated_at: ledger.updated_at.clone(),
            completed_at: ledger.completed_at.clone(),
            ledger,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PendingQuestion {
    pub id: String,
    pub question: String,
    pub asked_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_chat_id: Option<String>,
    pub blocking: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_at: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConductorWakeReason {
    Manual,
    GoalCreated,
    TaskBoard,
    ChatLifecycle,
    AgentStall,
    HumanSteering,
    GhostAnswer,
    Budget,
    Heartbeat,
    Opportunity,
    Cron,
}

impl Default for ConductorWakeReason {
    fn default() -> Self {
        Self::Manual
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalValidationError {
    MissingDoneWhen,
    MissingWallClockSecs,
    ZeroWallClockSecs,
    MissingNoProgressWakes,
    ZeroNoProgressWakes,
}

impl fmt::Display for GoalValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingDoneWhen => {
                write!(f, "goal done_when requires a summary or checklist item")
            }
            Self::MissingWallClockSecs => write!(f, "goal budget requires wall_clock_secs"),
            Self::ZeroWallClockSecs => {
                write!(f, "goal budget wall_clock_secs must be greater than zero")
            }
            Self::MissingNoProgressWakes => write!(f, "goal budget requires no_progress_wakes"),
            Self::ZeroNoProgressWakes => {
                write!(f, "goal budget no_progress_wakes must be greater than zero")
            }
        }
    }
}

impl std::error::Error for GoalValidationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalDocParseError {
    Frontmatter(String),
    Validation(GoalValidationError),
}

impl fmt::Display for GoalDocParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Frontmatter(err) => write!(f, "failed to parse goal document frontmatter: {err}"),
            Self::Validation(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for GoalDocParseError {}

impl From<GoalValidationError> for GoalDocParseError {
    fn from(err: GoalValidationError) -> Self {
        Self::Validation(err)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct GoalDocFrontmatter {
    #[serde(alias = "goal_id")]
    id: Option<String>,
    #[serde(alias = "name")]
    title: Option<String>,
    #[serde(alias = "slug")]
    plan_doc_slug: Option<String>,
    done_when: Option<DoneWhen>,
    autonomy: Option<GoalAutonomy>,
    budget: Option<GoalBudget>,
}

pub fn parse_goal_doc(content: &str) -> Result<ConductorGoal, GoalDocParseError> {
    let (frontmatter, body) = split_frontmatter(content)?;
    let metadata = parse_frontmatter(frontmatter)?;
    let plan_markdown = body.trim().to_string();
    let title = non_empty(metadata.title)
        .or_else(|| first_markdown_heading(&plan_markdown))
        .unwrap_or_else(|| "Untitled conductor goal".to_string());
    let goal = ConductorGoal {
        id: non_empty(metadata.id).unwrap_or_default(),
        title,
        plan_doc_slug: non_empty(metadata.plan_doc_slug),
        plan_markdown,
        done_when: metadata.done_when.unwrap_or_default(),
        autonomy: metadata.autonomy.unwrap_or_default(),
        budget: metadata.budget.unwrap_or_default(),
        ..ConductorGoal::default()
    };
    validate_goal_for_create(&goal)?;
    Ok(goal)
}

pub fn validate_goal_for_create(goal: &ConductorGoal) -> Result<(), GoalValidationError> {
    validate_done_when(&goal.done_when)?;
    validate_goal_budget(&goal.budget)
}

pub fn validate_done_when(done_when: &DoneWhen) -> Result<(), GoalValidationError> {
    if done_when.has_completion_criteria() {
        Ok(())
    } else {
        Err(GoalValidationError::MissingDoneWhen)
    }
}

pub fn validate_goal_budget(budget: &GoalBudget) -> Result<(), GoalValidationError> {
    match budget.wall_clock_secs {
        None => return Err(GoalValidationError::MissingWallClockSecs),
        Some(0) => return Err(GoalValidationError::ZeroWallClockSecs),
        Some(_) => {}
    }
    match budget.no_progress_wakes {
        None => Err(GoalValidationError::MissingNoProgressWakes),
        Some(0) => Err(GoalValidationError::ZeroNoProgressWakes),
        Some(_) => Ok(()),
    }
}

fn parse_frontmatter(frontmatter: Option<&str>) -> Result<GoalDocFrontmatter, GoalDocParseError> {
    match frontmatter.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => serde_yaml::from_str::<GoalDocFrontmatter>(value)
            .map_err(|err| GoalDocParseError::Frontmatter(err.to_string())),
        None => Ok(GoalDocFrontmatter::default()),
    }
}

fn split_frontmatter(content: &str) -> Result<(Option<&str>, &str), GoalDocParseError> {
    let Some(after_open) = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))
    else {
        return Ok((None, content));
    };
    let Some(end) = after_open.find("\n---") else {
        return Err(GoalDocParseError::Frontmatter(
            "missing closing frontmatter marker".to_string(),
        ));
    };
    let frontmatter = &after_open[..end];
    let after_marker = &after_open[end + "\n---".len()..];
    let body = after_marker
        .strip_prefix("\r\n")
        .or_else(|| after_marker.strip_prefix('\n'))
        .unwrap_or(after_marker);
    Ok((Some(frontmatter), body))
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn first_markdown_heading(body: &str) -> Option<String> {
    body.lines().find_map(|line| {
        line.trim()
            .strip_prefix("# ")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_goal() -> ConductorGoal {
        ConductorGoal {
            id: "goal-1".to_string(),
            title: "Ship conductor".to_string(),
            plan_doc_slug: Some("master-plan".to_string()),
            plan_markdown: "# Ship conductor\nImplement the plan.".to_string(),
            done_when: DoneWhen {
                summary: "All conductor cards are done".to_string(),
                checklist: vec!["tests pass".to_string()],
            },
            status: GoalStatus::Active,
            autonomy: GoalAutonomy::FullAuto,
            budget: GoalBudget {
                wall_clock_secs: Some(7200),
                no_progress_wakes: Some(4),
                total_tokens: Some(100_000),
                usd: Some(5.5),
            },
            spent: GoalBudgetSpent {
                elapsed_secs: 30,
                prompt_tokens: 800,
                completion_tokens: 400,
                total_tokens: 1200,
                cache_read_tokens: 400,
                usd: Some(0.12),
                no_progress_wakes: 1,
            },
            ledger: GoalLedger {
                title: Some("Ship conductor".to_string()),
                plan_doc_slug: Some("master-plan".to_string()),
                plan_markdown: Some("# Ship conductor\nImplement the plan.".to_string()),
                done_when: Some(DoneWhen {
                    summary: "All conductor cards are done".to_string(),
                    checklist: vec!["tests pass".to_string()],
                }),
                budget: Some(GoalBudget {
                    wall_clock_secs: Some(7200),
                    no_progress_wakes: Some(4),
                    total_tokens: Some(100_000),
                    usd: Some(5.5),
                }),
                created_at: Some("2026-06-03T00:00:00Z".to_string()),
                updated_at: Some("2026-06-03T00:00:03Z".to_string()),
                completed_at: None,
                status: Some(GoalStatus::Active),
                autonomy: Some(GoalAutonomy::FullAuto),
                planner_task_id: Some("task-1".to_string()),
                task_ids: vec!["task-1".to_string()],
                chat_ids: vec!["chat-1".to_string()],
                memos: vec![ConductorMemo {
                    id: "memo-1".to_string(),
                    kind: MemoKind::Decision,
                    content: "Use Buddy Home".to_string(),
                    created_at: "2026-06-03T00:00:00Z".to_string(),
                    source_chat_id: Some("chat-1".to_string()),
                    related_task_id: Some("task-1".to_string()),
                }],
                learning_records: Vec::new(),
                ghost_messages: Vec::new(),
                recurring: None,
                pending_questions: vec![PendingQuestion {
                    id: "question-1".to_string(),
                    question: "Continue?".to_string(),
                    asked_at: "2026-06-03T00:00:01Z".to_string(),
                    source_chat_id: Some("chat-1".to_string()),
                    blocking: true,
                    answer: Some("Yes".to_string()),
                    answered_at: Some("2026-06-03T00:00:02Z".to_string()),
                }],
                no_progress_wakes: 1,
                turn_failures: 0,
                last_wake_at: Some("2026-06-03T00:00:04Z".to_string()),
                last_progress_at: Some("2026-06-03T00:00:03Z".to_string()),
                last_wake_reason: Some(ConductorWakeReason::Heartbeat),
                ..Default::default()
            },
            created_at: Some("2026-06-03T00:00:00Z".to_string()),
            updated_at: Some("2026-06-03T00:00:03Z".to_string()),
            completed_at: None,
        }
    }

    #[test]
    fn conductor_goal_round_trips_through_json() {
        let goal = full_goal();
        let json = serde_json::to_string(&goal).unwrap();
        let decoded: ConductorGoal = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded, goal);
    }

    #[test]
    fn legacy_goal_missing_optional_fields_deserializes_with_defaults() {
        let json = serde_json::json!({
            "id": "goal-legacy",
            "title": "Legacy goal",
            "plan_markdown": "# Legacy goal",
            "budget": {
                "wall_clock_secs": 60,
                "no_progress_wakes": 2
            }
        });

        let goal: ConductorGoal = serde_json::from_value(json).unwrap();

        assert_eq!(goal.status, GoalStatus::Active);
        assert_eq!(goal.autonomy, GoalAutonomy::FullAuto);
        assert_eq!(goal.done_when, DoneWhen::default());
        assert_eq!(goal.spent.elapsed_secs, 0);
        assert!(goal.ledger.memos.is_empty());
        assert_eq!(
            validate_goal_for_create(&goal),
            Err(GoalValidationError::MissingDoneWhen)
        );
    }

    #[test]
    fn parse_goal_doc_accepts_valid_plan_document() {
        let doc = r#"---
name: Buddy Conductor
slug: master-plan
autonomy: full_auto
budget:
  wall_clock_secs: 7200
  no_progress_wakes: 4
done_when:
  summary: All cards are complete
  checklist:
    - Tests pass
---
# Buddy Conductor
Implement the approved plan.
"#;

        let goal = parse_goal_doc(doc).unwrap();

        assert_eq!(goal.title, "Buddy Conductor");
        assert_eq!(goal.plan_doc_slug.as_deref(), Some("master-plan"));
        assert_eq!(goal.autonomy, GoalAutonomy::FullAuto);
        assert_eq!(goal.budget.wall_clock_secs, Some(7200));
        assert_eq!(goal.budget.no_progress_wakes, Some(4));
        assert_eq!(goal.done_when.summary, "All cards are complete");
        assert_eq!(goal.done_when.checklist, vec!["Tests pass".to_string()]);
        assert_eq!(
            goal.plan_markdown,
            "# Buddy Conductor\nImplement the approved plan."
        );
    }

    #[test]
    fn parse_goal_doc_accepts_approved_budget_aliases() {
        let doc = r#"---
title: Budget aliases
budget:
  wall_clock_secs: 7200
  no_progress_wakes: 4
  token_ceiling: 123456
  usd_ceiling: 7.25
done_when:
  summary: Budget is bounded
---
# Budget aliases
"#;

        let goal = parse_goal_doc(doc).unwrap();

        assert_eq!(goal.budget.wall_clock_secs, Some(7200));
        assert_eq!(goal.budget.no_progress_wakes, Some(4));
        assert_eq!(goal.budget.total_tokens, Some(123456));
        assert_eq!(goal.budget.usd, Some(7.25));
    }

    #[test]
    fn legacy_budget_keys_still_deserialize() {
        let budget: GoalBudget = serde_json::from_value(serde_json::json!({
            "wall_clock_secs": 60,
            "no_progress_wakes": 2,
            "total_tokens": 5000,
            "usd": 1.5
        }))
        .unwrap();

        assert_eq!(budget.total_tokens, Some(5000));
        assert_eq!(budget.usd, Some(1.5));
    }

    #[test]
    fn goal_status_deserializes_approved_and_legacy_states() {
        for (raw, expected) in [
            ("planned", GoalStatus::Proposed),
            ("running", GoalStatus::Active),
            ("waiting_for_human", GoalStatus::Paused),
            ("paused", GoalStatus::Paused),
            ("done", GoalStatus::Done),
            ("escalated", GoalStatus::Escalated),
            ("abandoned", GoalStatus::Abandoned),
            ("failed", GoalStatus::Escalated),
            ("cancelled", GoalStatus::Abandoned),
        ] {
            let status: GoalStatus = serde_json::from_value(serde_json::json!(raw)).unwrap();
            assert_eq!(status, expected);
        }
    }

    #[test]
    fn goal_status_serializes_only_canonical_states() {
        for (status, expected) in [
            (GoalStatus::Proposed, "proposed"),
            (GoalStatus::Active, "active"),
            (GoalStatus::Paused, "paused"),
            (GoalStatus::Escalated, "escalated"),
            (GoalStatus::Done, "done"),
            (GoalStatus::Abandoned, "abandoned"),
        ] {
            let encoded = serde_json::to_value(status).unwrap();
            assert_eq!(encoded, serde_json::json!(expected));
        }
    }

    #[test]
    fn legacy_running_round_trips_as_active() {
        let status: GoalStatus = serde_json::from_value(serde_json::json!("running")).unwrap();
        assert_eq!(status, GoalStatus::Active);
        assert_eq!(
            serde_json::to_value(status).unwrap(),
            serde_json::json!("active")
        );
    }

    #[test]
    fn goal_status_terminal_semantics_include_escalated_and_abandoned() {
        assert!(!GoalStatus::Proposed.is_terminal());
        assert!(!GoalStatus::Active.is_terminal());
        assert!(!GoalStatus::Paused.is_terminal());
        assert!(GoalStatus::Done.is_terminal());
        assert!(GoalStatus::Escalated.is_terminal());
        assert!(GoalStatus::Abandoned.is_terminal());
    }

    #[test]
    fn goal_status_transition_validator_accepts_allowed_transitions() {
        for (from, to) in [
            (GoalStatus::Proposed, GoalStatus::Active),
            (GoalStatus::Proposed, GoalStatus::Paused),
            (GoalStatus::Proposed, GoalStatus::Abandoned),
            (GoalStatus::Active, GoalStatus::Paused),
            (GoalStatus::Active, GoalStatus::Done),
            (GoalStatus::Active, GoalStatus::Escalated),
            (GoalStatus::Active, GoalStatus::Abandoned),
            (GoalStatus::Paused, GoalStatus::Active),
            (GoalStatus::Paused, GoalStatus::Abandoned),
            (GoalStatus::Done, GoalStatus::Done),
        ] {
            validate_goal_status_transition(from, to).unwrap();
        }
    }

    #[test]
    fn goal_status_transition_validator_rejects_invalid_transitions() {
        for (from, to) in [
            (GoalStatus::Proposed, GoalStatus::Done),
            (GoalStatus::Paused, GoalStatus::Done),
            (GoalStatus::Done, GoalStatus::Active),
            (GoalStatus::Escalated, GoalStatus::Paused),
            (GoalStatus::Abandoned, GoalStatus::Active),
        ] {
            let error = validate_goal_status_transition(from, to).unwrap_err();
            assert_eq!(error.from, from);
            assert_eq!(error.to, to);
        }
    }

    #[test]
    fn parse_goal_doc_rejects_missing_done_when() {
        let doc = r#"---
title: Missing done when
budget:
  wall_clock_secs: 60
  no_progress_wakes: 2
---
# Missing done when
"#;

        let err = parse_goal_doc(doc).unwrap_err();

        assert_eq!(
            err,
            GoalDocParseError::Validation(GoalValidationError::MissingDoneWhen)
        );
        assert!(err.to_string().contains("done_when"));
    }

    #[test]
    fn validate_goal_for_create_rejects_empty_done_when() {
        let mut goal = full_goal();
        goal.done_when = DoneWhen {
            summary: "   ".to_string(),
            checklist: vec!["".to_string(), "  ".to_string()],
        };

        assert_eq!(
            validate_goal_for_create(&goal),
            Err(GoalValidationError::MissingDoneWhen)
        );
    }

    #[test]
    fn validate_goal_for_create_accepts_summary_only_done_when() {
        let mut goal = full_goal();
        goal.done_when = DoneWhen {
            summary: "Ready to ship".to_string(),
            checklist: Vec::new(),
        };

        validate_goal_for_create(&goal).unwrap();
    }

    #[test]
    fn validate_goal_for_create_accepts_checklist_only_done_when() {
        let mut goal = full_goal();
        goal.done_when = DoneWhen {
            summary: "".to_string(),
            checklist: vec!["   ".to_string(), "Smoke tests pass".to_string()],
        };

        validate_goal_for_create(&goal).unwrap();
    }

    #[test]
    fn parse_goal_doc_rejects_missing_wall_clock_budget() {
        let doc = r#"---
title: Missing wall clock
budget:
  no_progress_wakes: 2
done_when:
  summary: Budget validation still runs
---
# Missing wall clock
"#;

        let err = parse_goal_doc(doc).unwrap_err();

        assert_eq!(
            err,
            GoalDocParseError::Validation(GoalValidationError::MissingWallClockSecs)
        );
    }

    #[test]
    fn parse_goal_doc_rejects_missing_no_progress_budget() {
        let doc = r#"---
title: Missing no-progress
budget:
  wall_clock_secs: 60
done_when:
  summary: Budget validation still runs
---
# Missing no-progress
"#;

        let err = parse_goal_doc(doc).unwrap_err();

        assert_eq!(
            err,
            GoalDocParseError::Validation(GoalValidationError::MissingNoProgressWakes)
        );
    }

    #[test]
    fn validate_goal_budget_rejects_zero_required_budgets() {
        let zero_wall_clock = GoalBudget {
            wall_clock_secs: Some(0),
            no_progress_wakes: Some(1),
            ..GoalBudget::default()
        };
        let zero_no_progress = GoalBudget {
            wall_clock_secs: Some(60),
            no_progress_wakes: Some(0),
            ..GoalBudget::default()
        };

        assert_eq!(
            validate_goal_budget(&zero_wall_clock),
            Err(GoalValidationError::ZeroWallClockSecs)
        );
        assert_eq!(
            validate_goal_budget(&zero_no_progress),
            Err(GoalValidationError::ZeroNoProgressWakes)
        );
    }

    #[test]
    fn goal_autonomy_defaults_to_full_auto() {
        assert_eq!(GoalAutonomy::default(), GoalAutonomy::FullAuto);

        let goal: ConductorGoal = serde_json::from_value(serde_json::json!({
            "budget": {
                "wall_clock_secs": 60,
                "no_progress_wakes": 1
            }
        }))
        .unwrap();

        assert_eq!(goal.autonomy, GoalAutonomy::FullAuto);
    }
}
