use std::path::Path;

use chrono::{DateTime, Utc};
use refact_buddy_core::conductor::{
    ConductorGoal, ConductorMemo, ConductorRecurring, ConductorWakeReason, GoalLedger, GoalStatus,
    MemoKind,
};
use refact_buddy_core::conductor_store::{list_goal_ledgers, save_goal_ledger};
use uuid::Uuid;

use crate::buddy::events::BuddyEvent;
use crate::global_context::SharedGlobalContext;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecurringRunReport {
    pub cron_enqueued: Vec<String>,
    pub stale_escalated: Vec<String>,
    pub invalid_cron_escalated: Vec<String>,
}

pub async fn service_recurring_goals(
    gcx: SharedGlobalContext,
    project_root: &Path,
    now: DateTime<Utc>,
) -> RecurringRunReport {
    let mut report = RecurringRunReport::default();
    let Ok(stored_ledgers) = list_goal_ledgers(project_root).await else {
        return report;
    };
    for stored in stored_ledgers {
        let mut ledger = stored.ledger;
        if terminal_or_paused(&ledger) {
            continue;
        }
        if maybe_escalate_invalid_cron(gcx.clone(), project_root, &stored.goal_id, &mut ledger)
            .await
        {
            report.invalid_cron_escalated.push(stored.goal_id.clone());
            continue;
        }
        if maybe_escalate_stale(gcx.clone(), project_root, &stored.goal_id, &mut ledger, now).await
        {
            report.stale_escalated.push(stored.goal_id.clone());
            continue;
        }
        if maybe_enqueue_due_cron(gcx.clone(), project_root, &stored.goal_id, &mut ledger, now)
            .await
        {
            report.cron_enqueued.push(stored.goal_id.clone());
        }
    }
    report
}

pub fn recurring_due_at(
    recurring: &ConductorRecurring,
    created_at: Option<&str>,
    now: DateTime<Utc>,
) -> Result<bool, String> {
    if !recurring.enabled || recurring.cron.trim().is_empty() {
        return Ok(false);
    }
    let from_ms = recurring
        .last_enqueued_at
        .as_deref()
        .or(created_at)
        .and_then(parse_ms)
        .unwrap_or(0);
    let Some(next_ms) = crate::scheduler::next_run_ms(
        &recurring.cron,
        from_ms,
        crate::scheduler::scheduler_timezone(),
    ) else {
        return Err("recurring conductor cron has no future run".to_string());
    };
    Ok(next_ms <= now.timestamp_millis() as u64)
}

async fn maybe_enqueue_due_cron(
    gcx: SharedGlobalContext,
    project_root: &Path,
    goal_id: &str,
    ledger: &mut GoalLedger,
    now: DateTime<Utc>,
) -> bool {
    let due = match ledger.recurring.as_ref() {
        Some(recurring) => recurring_due_at(recurring, ledger.created_at.as_deref(), now),
        None => return false,
    };
    if due != Ok(true) {
        return false;
    }
    if !super::wake::enqueue_goal_wake(gcx.clone(), goal_id, ConductorWakeReason::Cron).await {
        return false;
    }
    if let Some(recurring) = ledger.recurring.as_mut() {
        recurring.last_enqueued_at = Some(now.to_rfc3339());
    }
    ledger.last_wake_reason = Some(ConductorWakeReason::Cron);
    save_and_emit(gcx, project_root, goal_id, ledger).await
}

async fn maybe_escalate_stale(
    gcx: SharedGlobalContext,
    project_root: &Path,
    goal_id: &str,
    ledger: &mut GoalLedger,
    now: DateTime<Utc>,
) -> bool {
    let Some(stale_after_secs) = ledger
        .recurring
        .as_ref()
        .and_then(|recurring| recurring.stale_after_secs)
    else {
        return false;
    };
    let Some(anchor) = latest_activity(ledger) else {
        return false;
    };
    if now.signed_duration_since(anchor).num_seconds() < stale_after_secs as i64 {
        return false;
    }
    ledger.status = Some(GoalStatus::Escalated);
    push_escalation(
        ledger,
        format!("Conductor goal went stale for at least {stale_after_secs} seconds."),
    );
    save_and_emit(gcx, project_root, goal_id, ledger).await
}

async fn maybe_escalate_invalid_cron(
    gcx: SharedGlobalContext,
    project_root: &Path,
    goal_id: &str,
    ledger: &mut GoalLedger,
) -> bool {
    let Some(recurring) = ledger.recurring.as_ref() else {
        return false;
    };
    if !recurring.enabled || recurring.cron.trim().is_empty() {
        return false;
    }
    if crate::scheduler::parse_cron(&recurring.cron).is_ok() {
        return false;
    }
    ledger.status = Some(GoalStatus::Escalated);
    push_escalation(ledger, "Recurring conductor cron is invalid.".to_string());
    save_and_emit(gcx, project_root, goal_id, ledger).await
}

async fn save_and_emit(
    gcx: SharedGlobalContext,
    project_root: &Path,
    goal_id: &str,
    ledger: &GoalLedger,
) -> bool {
    if save_goal_ledger(project_root, goal_id, ledger)
        .await
        .is_err()
    {
        return false;
    }
    if let Some(tx) = gcx.buddy_events_tx.as_ref() {
        let _ = tx.send(BuddyEvent::ConductorGoalUpdated {
            goal: ConductorGoal::from_ledger(goal_id.to_string(), ledger.clone()),
        });
    }
    true
}

fn terminal_or_paused(ledger: &GoalLedger) -> bool {
    matches!(
        ledger.status.unwrap_or_default(),
        GoalStatus::Done
            | GoalStatus::Escalated
            | GoalStatus::Abandoned
            | GoalStatus::Failed
            | GoalStatus::Cancelled
            | GoalStatus::Paused
    )
}

fn latest_activity(ledger: &GoalLedger) -> Option<DateTime<Utc>> {
    [
        ledger.last_progress_at.as_deref(),
        ledger.last_wake_at.as_deref(),
        ledger.created_at.as_deref(),
    ]
    .into_iter()
    .flatten()
    .filter_map(parse_dt)
    .max()
}

fn parse_ms(value: &str) -> Option<u64> {
    parse_dt(value).and_then(|dt| u64::try_from(dt.timestamp_millis()).ok())
}

fn parse_dt(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn push_escalation(ledger: &mut GoalLedger, content: String) {
    if ledger
        .memos
        .iter()
        .rev()
        .take(3)
        .any(|memo| memo.kind == MemoKind::Escalation && memo.content == content)
    {
        return;
    }
    ledger.memos.push(ConductorMemo {
        id: Uuid::new_v4().to_string(),
        kind: MemoKind::Escalation,
        content,
        created_at: Utc::now().to_rfc3339(),
        source_chat_id: None,
        related_task_id: ledger.planner_task_id.clone(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use refact_buddy_core::conductor::GoalBudget;
    use refact_buddy_core::conductor_store::load_goal_ledger;
    use tokio::time::timeout;

    fn ts(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).unwrap()
    }

    async fn test_gcx(root: &std::path::Path) -> SharedGlobalContext {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
        gcx
    }

    #[test]
    fn recurring_due_uses_existing_scheduler_cron_policy() {
        let recurring = ConductorRecurring {
            enabled: true,
            cron: "*/5 * * * *".to_string(),
            last_enqueued_at: Some("2026-01-01T00:00:00Z".to_string()),
            stale_after_secs: None,
        };

        assert!(recurring_due_at(&recurring, None, ts(1767225900)).unwrap());
    }

    #[tokio::test]
    async fn cron_wake_enqueue_updates_ledger_without_duplicate_reasons() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx(dir.path()).await;
        save_goal_ledger(
            dir.path(),
            "goal-cron",
            &GoalLedger {
                created_at: Some("2026-01-01T00:00:00Z".to_string()),
                recurring: Some(ConductorRecurring {
                    enabled: true,
                    cron: "*/5 * * * *".to_string(),
                    last_enqueued_at: Some("2026-01-01T00:00:00Z".to_string()),
                    stale_after_secs: None,
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let report = service_recurring_goals(gcx.clone(), dir.path(), ts(1767225900)).await;
        service_recurring_goals(gcx.clone(), dir.path(), ts(1767225900)).await;

        assert_eq!(report.cron_enqueued, vec!["goal-cron".to_string()]);
        let bus = gcx.conductor_wake_bus.lock().await;
        let mailbox = bus.mailbox("goal-cron").unwrap();
        assert_eq!(mailbox.reasons, vec![ConductorWakeReason::Cron]);
        drop(bus);
        let ledger = load_goal_ledger(dir.path(), "goal-cron")
            .await
            .unwrap()
            .unwrap();
        assert!(ledger.recurring.unwrap().last_enqueued_at.is_some());
    }

    #[tokio::test]
    async fn stale_recurring_goal_escalates() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx(dir.path()).await;
        save_goal_ledger(
            dir.path(),
            "goal-stale",
            &GoalLedger {
                last_progress_at: Some("2026-01-01T00:00:00Z".to_string()),
                recurring: Some(ConductorRecurring {
                    enabled: true,
                    cron: "0 * * * *".to_string(),
                    last_enqueued_at: Some("2026-01-01T00:00:00Z".to_string()),
                    stale_after_secs: Some(60),
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let report = service_recurring_goals(gcx, dir.path(), ts(1767225661)).await;

        assert_eq!(report.stale_escalated, vec!["goal-stale".to_string()]);
        let ledger = load_goal_ledger(dir.path(), "goal-stale")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.status, Some(GoalStatus::Escalated));
        assert!(ledger
            .memos
            .iter()
            .any(|memo| memo.kind == MemoKind::Escalation));
    }

    #[tokio::test]
    async fn recurring_stale_emits_complete_goal_update() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx(dir.path()).await;
        save_goal_ledger(
            dir.path(),
            "goal-stale-event",
            &GoalLedger {
                title: Some("Stale rich goal".to_string()),
                budget: Some(GoalBudget {
                    wall_clock_secs: Some(600),
                    no_progress_wakes: Some(2),
                    ..Default::default()
                }),
                last_progress_at: Some("2026-01-01T00:00:00Z".to_string()),
                recurring: Some(ConductorRecurring {
                    enabled: true,
                    cron: "0 * * * *".to_string(),
                    last_enqueued_at: Some("2026-01-01T00:00:00Z".to_string()),
                    stale_after_secs: Some(60),
                }),
                no_progress_wakes: 1,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mut rx = gcx.buddy_events_tx.as_ref().unwrap().subscribe();

        let report = service_recurring_goals(gcx, dir.path(), ts(1767225661)).await;

        assert_eq!(report.stale_escalated, vec!["goal-stale-event".to_string()]);
        let event = timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match event {
            BuddyEvent::ConductorGoalUpdated { goal } => {
                assert_eq!(goal.id, "goal-stale-event");
                assert_eq!(goal.title, "Stale rich goal");
                assert_eq!(goal.status, GoalStatus::Escalated);
                assert_eq!(goal.budget.wall_clock_secs, Some(600));
                assert_eq!(goal.spent.no_progress_wakes, 1);
            }
            other => panic!("expected ConductorGoalUpdated, got {other:?}"),
        }
    }
}
