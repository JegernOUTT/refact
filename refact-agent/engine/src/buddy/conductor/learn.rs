use std::collections::BTreeSet;
use std::sync::Arc;

use chrono::Utc;
use refact_buddy_core::conductor::{
    ConductorGoal, ConductorLearningRecord, GoalLedger, LearningBudgetSnapshot, LearningOutcome,
    MemoKind,
};
use refact_buddy_core::conductor_store::{list_goal_ledgers, mutate_goal_ledger, MissingGoalBehavior};
use uuid::Uuid;

use crate::buddy::jobs::autonomous_chats::redact_and_cap_text;
use crate::global_context::GlobalContext;

const MAX_LEARNING_RECORDS: usize = 32;
const MAX_LEARNING_SUMMARY_CHARS: usize = 700;
const MAX_LEARNING_ITEM_CHARS: usize = 180;
const MAX_LEARNING_ITEMS: usize = 5;
const MAX_PRIOR_LESSONS: usize = 5;
const MAX_PRIOR_LESSON_CHARS: usize = 320;
const KNOWLEDGE_MIN_SUMMARY_CHARS: usize = 80;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorConductorLesson {
    pub goal_id: String,
    pub outcome: LearningOutcome,
    pub summary: String,
    pub useful_tools_or_strategies: Vec<String>,
    pub future_tunables: Vec<String>,
    pub created_at: String,
}

pub async fn record_goal_learning(
    gcx: Arc<GlobalContext>,
    project_root: &std::path::Path,
    goal: &mut ConductorGoal,
    outcome: LearningOutcome,
    reason: Option<&str>,
    source_chat_id: Option<String>,
) -> Result<bool, String> {
    let inserted = append_learning_record(goal, outcome, reason, source_chat_id);
    if inserted {
        let replacement = goal.ledger.clone();
        mutate_goal_ledger(
            project_root,
            &goal.id,
            MissingGoalBehavior::CreateDefault,
            |ledger| {
                *ledger = replacement;
                Ok(())
            },
        )
        .await
        .map_err(|error| error.to_string())?;
        persist_learning_knowledge_if_appropriate(gcx, goal.ledger.learning_records.last()).await;
    }
    Ok(inserted)
}

pub fn append_learning_record(
    goal: &mut ConductorGoal,
    outcome: LearningOutcome,
    reason: Option<&str>,
    source_chat_id: Option<String>,
) -> bool {
    if goal
        .ledger
        .learning_records
        .iter()
        .any(|record| record.outcome == outcome)
    {
        return false;
    }
    let record = build_learning_record(goal, outcome, reason, source_chat_id);
    goal.ledger.learning_records.push(record);
    goal.ledger.learning_records.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    if goal.ledger.learning_records.len() > MAX_LEARNING_RECORDS {
        goal.ledger.learning_records.truncate(MAX_LEARNING_RECORDS);
    }
    true
}

pub fn build_learning_record(
    goal: &ConductorGoal,
    outcome: LearningOutcome,
    reason: Option<&str>,
    source_chat_id: Option<String>,
) -> ConductorLearningRecord {
    let summary = learning_summary(goal, outcome, reason);
    ConductorLearningRecord {
        id: Uuid::new_v4().to_string(),
        outcome,
        goal_id: clean(&goal.id, 80),
        goal_title: clean(&goal.title, 160),
        summary,
        what_worked: learning_what_worked(goal, outcome),
        failures: learning_failures(goal, outcome, reason),
        budget_used: LearningBudgetSnapshot::from(&goal.spent),
        no_progress_wakes: goal
            .spent
            .no_progress_wakes
            .max(goal.ledger.no_progress_wakes),
        useful_tools_or_strategies: learning_strategies(goal),
        future_tunables: learning_tunables(goal, outcome),
        created_at: Utc::now().to_rfc3339(),
        source_chat_id: source_chat_id.map(|value| clean(&value, 120)),
        related_task_id: goal
            .ledger
            .planner_task_id
            .as_deref()
            .map(|value| clean(value, 120)),
    }
}

pub async fn load_prior_lessons(
    project_root: &std::path::Path,
    goal: &ConductorGoal,
) -> Result<Vec<PriorConductorLesson>, String> {
    let ledgers = list_goal_ledgers(project_root)
        .await
        .map_err(|error| error.to_string())?;
    let mut lessons = ledgers
        .into_iter()
        .filter(|entry| entry.goal_id != goal.id)
        .flat_map(|entry| lessons_from_ledger(&entry.goal_id, &entry.ledger, goal))
        .collect::<Vec<_>>();
    lessons.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.goal_id.cmp(&right.goal_id))
    });
    lessons.truncate(MAX_PRIOR_LESSONS);
    Ok(lessons)
}

pub fn lessons_from_ledger(
    goal_id: &str,
    ledger: &GoalLedger,
    current_goal: &ConductorGoal,
) -> Vec<PriorConductorLesson> {
    ledger
        .learning_records
        .iter()
        .filter(|record| lesson_is_relevant(record, current_goal))
        .map(|record| PriorConductorLesson {
            goal_id: clean(goal_id, 80),
            outcome: record.outcome,
            summary: clean(&record.summary, MAX_PRIOR_LESSON_CHARS),
            useful_tools_or_strategies: clean_items(
                &record.useful_tools_or_strategies,
                MAX_LEARNING_ITEMS,
                MAX_LEARNING_ITEM_CHARS,
            ),
            future_tunables: clean_items(
                &record.future_tunables,
                MAX_LEARNING_ITEMS,
                MAX_LEARNING_ITEM_CHARS,
            ),
            created_at: clean(&record.created_at, 80),
        })
        .collect()
}

pub fn lesson_text(lesson: &PriorConductorLesson) -> String {
    let mut parts = vec![format!(
        "{} {:?}: {}",
        lesson.goal_id, lesson.outcome, lesson.summary
    )];
    if !lesson.useful_tools_or_strategies.is_empty() {
        parts.push(format!(
            "strategies={}",
            lesson.useful_tools_or_strategies.join("; ")
        ));
    }
    if !lesson.future_tunables.is_empty() {
        parts.push(format!("tunables={}", lesson.future_tunables.join("; ")));
    }
    clean(&parts.join(" | "), MAX_PRIOR_LESSON_CHARS)
}

fn learning_summary(
    goal: &ConductorGoal,
    outcome: LearningOutcome,
    reason: Option<&str>,
) -> String {
    let outcome_text = match outcome {
        LearningOutcome::Done => "completed",
        LearningOutcome::Escalated => "escalated",
    };
    let mut lines = vec![format!(
        "Conductor goal '{}' {} after {} no-progress wakes, {} total tokens, and {} elapsed seconds.",
        goal.title,
        outcome_text,
        goal.spent.no_progress_wakes.max(goal.ledger.no_progress_wakes),
        goal.spent.total_tokens,
        goal.spent.elapsed_secs
    )];
    if let Some(reason) = reason.map(str::trim).filter(|reason| !reason.is_empty()) {
        lines.push(format!("Reason: {reason}"));
    }
    if !goal.done_when.summary.trim().is_empty() {
        lines.push(format!("Done criteria: {}", goal.done_when.summary));
    }
    clean(&lines.join(" "), MAX_LEARNING_SUMMARY_CHARS)
}

fn learning_what_worked(goal: &ConductorGoal, outcome: LearningOutcome) -> Vec<String> {
    let mut items = Vec::new();
    if matches!(outcome, LearningOutcome::Done) {
        push_item(
            &mut items,
            format!("Goal reached done criteria: {}", goal.done_when.summary),
        );
    }
    if !goal.ledger.task_ids.is_empty() || goal.ledger.planner_task_id.is_some() {
        push_item(&mut items, "Conductor kept task ownership in the ledger.");
    }
    if !goal.ledger.chat_ids.is_empty() {
        push_item(
            &mut items,
            "Conductor retained chat ids for budget and history reuse.",
        );
    }
    for memo in goal
        .ledger
        .memos
        .iter()
        .filter(|memo| matches!(memo.kind, MemoKind::Decision | MemoKind::Progress))
    {
        push_item(&mut items, memo.content.clone());
    }
    if items.is_empty() {
        push_item(
            &mut items,
            "Ledger state was captured before outcome handling.",
        );
    }
    capped_items(items)
}

fn learning_failures(
    goal: &ConductorGoal,
    outcome: LearningOutcome,
    reason: Option<&str>,
) -> Vec<String> {
    let mut items = Vec::new();
    if matches!(outcome, LearningOutcome::Escalated) {
        push_item(
            &mut items,
            reason.unwrap_or("Conductor escalated for human attention."),
        );
    }
    for memo in goal
        .ledger
        .memos
        .iter()
        .filter(|memo| matches!(memo.kind, MemoKind::Risk | MemoKind::Escalation))
    {
        push_item(&mut items, memo.content.clone());
    }
    capped_items(items)
}

fn learning_strategies(goal: &ConductorGoal) -> Vec<String> {
    let mut items = Vec::new();
    if !goal.ledger.task_ids.is_empty() || goal.ledger.planner_task_id.is_some() {
        push_item(&mut items, "task ledger scan");
    }
    if !goal.ledger.chat_ids.is_empty() {
        push_item(&mut items, "chat usage aggregation");
    }
    if goal.ledger.no_progress_wakes > 0 {
        push_item(&mut items, "no-progress wake counter");
    }
    if goal.budget.total_tokens.is_some() || goal.budget.usd.is_some() {
        push_item(&mut items, "budget guardrail");
    }
    capped_items(items)
}

fn learning_tunables(goal: &ConductorGoal, outcome: LearningOutcome) -> Vec<String> {
    let mut items = Vec::new();
    if let Some(limit) = goal.budget.no_progress_wakes {
        push_item(
            &mut items,
            format!(
                "no_progress_wakes limit={} spent={}",
                limit,
                goal.spent
                    .no_progress_wakes
                    .max(goal.ledger.no_progress_wakes)
            ),
        );
    }
    if let Some(limit) = goal.budget.wall_clock_secs {
        push_item(
            &mut items,
            format!(
                "wall_clock_secs limit={} spent={}",
                limit, goal.spent.elapsed_secs
            ),
        );
    }
    if let Some(limit) = goal.budget.total_tokens {
        push_item(
            &mut items,
            format!(
                "total_tokens limit={} spent={}",
                limit, goal.spent.total_tokens
            ),
        );
    }
    if matches!(outcome, LearningOutcome::Escalated) {
        push_item(&mut items, "escalation threshold may need adjustment");
    }
    capped_items(items)
}

fn lesson_is_relevant(record: &ConductorLearningRecord, current_goal: &ConductorGoal) -> bool {
    if current_goal.title.trim().is_empty() {
        return true;
    }
    let title_tokens = tokens(&current_goal.title);
    if title_tokens.is_empty() {
        return true;
    }
    let haystack = tokens(&format!(
        "{} {} {} {}",
        record.goal_title,
        record.summary,
        record.useful_tools_or_strategies.join(" "),
        record.future_tunables.join(" ")
    ));
    title_tokens.iter().any(|token| haystack.contains(token))
}

fn tokens(text: &str) -> BTreeSet<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|token| token.len() >= 4)
        .map(str::to_ascii_lowercase)
        .collect()
}

fn push_item(items: &mut Vec<String>, value: impl Into<String>) {
    let value = value.into();
    let value = clean(&value, MAX_LEARNING_ITEM_CHARS);
    if !value.is_empty() && !items.iter().any(|existing| existing == &value) {
        items.push(value);
    }
}

fn capped_items(items: Vec<String>) -> Vec<String> {
    clean_items(&items, MAX_LEARNING_ITEMS, MAX_LEARNING_ITEM_CHARS)
}

fn clean_items(items: &[String], max_items: usize, max_chars: usize) -> Vec<String> {
    let mut cleaned = items
        .iter()
        .map(|item| clean(item, max_chars))
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    cleaned.dedup();
    cleaned.truncate(max_items);
    cleaned
}

fn clean(text: &str, max_chars: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    redact_and_cap_text(&normalized, max_chars)
}

async fn persist_learning_knowledge_if_appropriate(
    gcx: Arc<GlobalContext>,
    record: Option<&ConductorLearningRecord>,
) {
    let Some(record) = record else {
        return;
    };
    if record.summary.len() < KNOWLEDGE_MIN_SUMMARY_CHARS {
        return;
    }
    let tags = vec![
        "buddy".to_string(),
        "conductor".to_string(),
        "lesson".to_string(),
    ];
    let empty = Vec::<String>::new();
    let title = format!("Conductor lesson: {}", record.goal_title);
    let mut frontmatter =
        crate::memories::create_frontmatter(Some(&title), &tags, &empty, &empty, "lesson");
    frontmatter.summary = Some(record.summary.clone());
    frontmatter.source_tool = Some("buddy_conductor_learning".to_string());
    frontmatter.source_chat_id = record.source_chat_id.clone();
    frontmatter.source_confidence = Some(0.75);
    let content = learning_knowledge_body(record);
    if let Err(error) = crate::memories::memories_add(gcx, &frontmatter, &content).await {
        tracing::debug!("failed to persist conductor learning memory: {}", error);
    }
}

fn learning_knowledge_body(record: &ConductorLearningRecord) -> String {
    let mut lines = vec![record.summary.clone()];
    if !record.what_worked.is_empty() {
        lines.push(format!("Worked: {}", record.what_worked.join("; ")));
    }
    if !record.failures.is_empty() {
        lines.push(format!("Failures: {}", record.failures.join("; ")));
    }
    if !record.useful_tools_or_strategies.is_empty() {
        lines.push(format!(
            "Strategies: {}",
            record.useful_tools_or_strategies.join("; ")
        ));
    }
    if !record.future_tunables.is_empty() {
        lines.push(format!("Tunables: {}", record.future_tunables.join("; ")));
    }
    clean(&lines.join("\n"), 1800)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::conductor::packet::{build_conductor_packet, ConductorPacketInput};
    use refact_buddy_core::conductor::{
        ConductorMemo, ConductorWakeReason, DoneWhen, GoalBudget, GoalBudgetSpent, GoalStatus,
    };
    use refact_buddy_core::conductor_store::{load_goal_ledger, save_goal_ledger};

    async fn test_gcx(root: &std::path::Path) -> Arc<GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
        gcx
    }

    fn goal(goal_id: &str) -> ConductorGoal {
        ConductorGoal {
            id: goal_id.to_string(),
            title: "Ship conductor safely".to_string(),
            done_when: DoneWhen {
                summary: "All conductor cards are complete".to_string(),
                checklist: vec!["tests pass".to_string()],
            },
            status: GoalStatus::Running,
            budget: GoalBudget {
                wall_clock_secs: Some(7200),
                no_progress_wakes: Some(4),
                total_tokens: Some(10_000),
                usd: Some(2.0),
            },
            spent: GoalBudgetSpent {
                elapsed_secs: 300,
                prompt_tokens: 1000,
                completion_tokens: 250,
                total_tokens: 1250,
                cache_read_tokens: 125,
                usd: Some(0.42),
                no_progress_wakes: 2,
            },
            ledger: GoalLedger {
                planner_task_id: Some("task-1".to_string()),
                task_ids: vec!["task-1".to_string()],
                chat_ids: vec!["planner-chat".to_string()],
                memos: vec![
                    ConductorMemo {
                        id: "memo-progress".to_string(),
                        kind: MemoKind::Progress,
                        content: "Bounded turns and task snapshots worked".to_string(),
                        created_at: "2026-06-03T00:00:00Z".to_string(),
                        source_chat_id: Some("planner-chat".to_string()),
                        related_task_id: Some("task-1".to_string()),
                    },
                    ConductorMemo {
                        id: "memo-risk".to_string(),
                        kind: MemoKind::Risk,
                        content: "Temporary turn failures happened".to_string(),
                        created_at: "2026-06-03T00:00:01Z".to_string(),
                        source_chat_id: None,
                        related_task_id: Some("task-1".to_string()),
                    },
                ],
                no_progress_wakes: 2,
                ..GoalLedger::default()
            },
            ..ConductorGoal::default()
        }
    }

    #[tokio::test]
    async fn completion_record_is_persisted_in_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx(dir.path()).await;
        let mut goal = goal("goal-done");
        goal.status = GoalStatus::Done;

        let inserted = record_goal_learning(
            gcx,
            dir.path(),
            &mut goal,
            LearningOutcome::Done,
            None,
            Some("planner-chat".to_string()),
        )
        .await
        .unwrap();

        assert!(inserted);
        let ledger = load_goal_ledger(dir.path(), "goal-done")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.learning_records.len(), 1);
        let record = &ledger.learning_records[0];
        assert_eq!(record.outcome, LearningOutcome::Done);
        assert!(record.summary.contains("completed"));
        assert_eq!(record.budget_used.total_tokens, 1250);
        assert_eq!(record.no_progress_wakes, 2);
        assert!(record
            .what_worked
            .iter()
            .any(|item| item.contains("done criteria") || item.contains("Bounded turns")));
    }

    #[test]
    fn escalation_record_contains_failures_and_tunables() {
        let mut goal = goal("goal-escalated");

        let inserted = append_learning_record(
            &mut goal,
            LearningOutcome::Escalated,
            Some("Conductor no-progress wake budget exhausted before turn."),
            None,
        );

        assert!(inserted);
        let record = &goal.ledger.learning_records[0];
        assert_eq!(record.outcome, LearningOutcome::Escalated);
        assert!(record.summary.contains("escalated"));
        assert!(record
            .failures
            .iter()
            .any(|item| item.contains("no-progress")));
        assert!(record
            .future_tunables
            .iter()
            .any(|item| item.contains("no_progress_wakes")));
    }

    #[tokio::test]
    async fn packet_includes_relevant_prior_lessons() {
        let dir = tempfile::tempdir().unwrap();
        let mut prior = goal("prior-goal");
        append_learning_record(
            &mut prior,
            LearningOutcome::Done,
            Some("Conductor packet stayed bounded"),
            Some("prior-chat".to_string()),
        );
        save_goal_ledger(dir.path(), "prior-goal", &prior.ledger)
            .await
            .unwrap();
        let current = goal("current-goal");

        let lessons = load_prior_lessons(dir.path(), &current).await.unwrap();
        let built = build_conductor_packet(ConductorPacketInput {
            goal: current,
            wake_reasons: vec![ConductorWakeReason::Manual],
            task_boards: Vec::new(),
            agent_statuses: Vec::new(),
            last_wake_at: None,
            prior_lessons: lessons,
        });

        assert_eq!(built.packet.prior_lessons.shown, 1);
        assert!(built.json.contains("prior-goal"));
        assert!(built.text.contains("Prior lessons"));
    }

    #[test]
    fn learning_records_are_redacted() {
        let mut goal = goal("goal-secret");
        goal.ledger.memos.push(ConductorMemo {
            id: "secret".to_string(),
            kind: MemoKind::Decision,
            content: "Use token=rawsecret and api_key: should_not_leak".to_string(),
            created_at: "2026-06-03T00:00:02Z".to_string(),
            source_chat_id: None,
            related_task_id: None,
        });

        append_learning_record(&mut goal, LearningOutcome::Done, None, None);
        let json = serde_json::to_string(&goal.ledger.learning_records[0]).unwrap();

        assert!(!json.contains("rawsecret"));
        assert!(!json.contains("should_not_leak"));
    }

    #[test]
    fn learning_records_and_prior_lessons_are_size_capped() {
        let mut capped_goal = goal("goal-cap");
        capped_goal.title = "Ship conductor safely ".repeat(60);
        capped_goal.ledger.memos.clear();
        for idx in 0..20 {
            capped_goal.ledger.memos.push(ConductorMemo {
                id: format!("memo-{idx}"),
                kind: MemoKind::Progress,
                content: "very long lesson body ".repeat(80),
                created_at: format!("2026-06-03T00:00:{idx:02}Z"),
                source_chat_id: None,
                related_task_id: None,
            });
        }

        append_learning_record(&mut capped_goal, LearningOutcome::Done, None, None);
        let record = &capped_goal.ledger.learning_records[0];

        assert!(record.summary.len() <= MAX_LEARNING_SUMMARY_CHARS);
        assert!(record.what_worked.len() <= MAX_LEARNING_ITEMS);
        assert!(record
            .what_worked
            .iter()
            .all(|item| item.len() <= MAX_LEARNING_ITEM_CHARS));
        let current_goal = goal("ship-conductor-next");
        let lessons = lessons_from_ledger("goal-cap", &capped_goal.ledger, &current_goal);
        assert!(lessons.len() <= MAX_PRIOR_LESSONS);
        assert!(lesson_text(&lessons[0]).len() <= MAX_PRIOR_LESSON_CHARS);
    }
}
