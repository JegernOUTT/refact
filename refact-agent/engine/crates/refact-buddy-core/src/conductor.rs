use std::fmt;

use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Planned,
    Running,
    WaitingForHuman,
    Paused,
    Done,
    Failed,
    Cancelled,
}

impl Default for GoalStatus {
    fn default() -> Self {
        Self::Running
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
    pub total_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
pub struct DoneWhen {
    pub summary: String,
    pub checklist: Vec<String>,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct GoalLedger {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planner_task_id: Option<String>,
    pub task_ids: Vec<String>,
    pub chat_ids: Vec<String>,
    pub memos: Vec<ConductorMemo>,
    pub pending_questions: Vec<PendingQuestion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_progress_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_wake_reason: Option<ConductorWakeReason>,
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
    MissingWallClockSecs,
    ZeroWallClockSecs,
    MissingNoProgressWakes,
    ZeroNoProgressWakes,
}

impl fmt::Display for GoalValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
    validate_goal_budget(&goal.budget)
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
            status: GoalStatus::Running,
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
                pending_questions: vec![PendingQuestion {
                    id: "question-1".to_string(),
                    question: "Continue?".to_string(),
                    asked_at: "2026-06-03T00:00:01Z".to_string(),
                    source_chat_id: Some("chat-1".to_string()),
                    blocking: true,
                    answer: Some("Yes".to_string()),
                    answered_at: Some("2026-06-03T00:00:02Z".to_string()),
                }],
                last_progress_at: Some("2026-06-03T00:00:03Z".to_string()),
                last_wake_reason: Some(ConductorWakeReason::Heartbeat),
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

        assert_eq!(goal.status, GoalStatus::Running);
        assert_eq!(goal.autonomy, GoalAutonomy::FullAuto);
        assert_eq!(goal.done_when, DoneWhen::default());
        assert_eq!(goal.spent.elapsed_secs, 0);
        assert!(goal.ledger.memos.is_empty());
        validate_goal_for_create(&goal).unwrap();
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
    fn parse_goal_doc_rejects_missing_wall_clock_budget() {
        let doc = r#"---
title: Missing wall clock
budget:
  no_progress_wakes: 2
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
