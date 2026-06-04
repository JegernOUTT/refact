use std::path::Path;

use chrono::Utc;
use refact_buddy_core::conductor::{
    ConductorMemo, ConductorWakeReason, GoalLedger, GoalStatus, MemoKind,
};
use refact_buddy_core::conductor_store::{list_goal_ledgers, save_goal_ledger};
use uuid::Uuid;

use crate::global_context::SharedGlobalContext;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReattachReport {
    pub active_goals: Vec<String>,
    pub heartbeat_enqueued: Vec<String>,
    pub missing_reference_escalated: Vec<String>,
}

pub async fn reattach_conductor_goals(
    gcx: SharedGlobalContext,
    project_root: &Path,
) -> ReattachReport {
    let heartbeat_goal_ids = runnable_goal_ids(project_root).await;
    let targets =
        super::wake::refresh_conductor_wake_targets_for_project(gcx.clone(), project_root).await;
    let mut report = ReattachReport {
        active_goals: targets.goal_ids(),
        ..Default::default()
    };
    report.missing_reference_escalated = mark_missing_references(gcx.clone(), project_root).await;
    if !report.missing_reference_escalated.is_empty() {
        let refreshed =
            super::wake::refresh_conductor_wake_targets_for_project(gcx.clone(), project_root)
                .await;
        report.active_goals = refreshed.goal_ids();
    }
    super::recurring::service_recurring_goals(gcx.clone(), project_root, Utc::now()).await;
    for goal_id in heartbeat_goal_ids {
        if super::wake::enqueue_goal_wake_if_idle(
            gcx.clone(),
            &goal_id,
            ConductorWakeReason::Heartbeat,
        )
        .await
        {
            report.heartbeat_enqueued.push(goal_id);
        }
    }
    report
}

async fn runnable_goal_ids(project_root: &Path) -> Vec<String> {
    let Ok(stored_ledgers) = list_goal_ledgers(project_root).await else {
        return Vec::new();
    };
    stored_ledgers
        .into_iter()
        .filter(|stored| !terminal_or_paused(&stored.ledger))
        .map(|stored| stored.goal_id)
        .collect()
}

async fn mark_missing_references(gcx: SharedGlobalContext, project_root: &Path) -> Vec<String> {
    let Ok(stored_ledgers) = list_goal_ledgers(project_root).await else {
        return Vec::new();
    };
    let mut escalated = Vec::new();
    for stored in stored_ledgers {
        let mut ledger = stored.ledger;
        if terminal_or_paused(&ledger)
            || !all_declared_task_refs_missing(gcx.clone(), &ledger).await
        {
            continue;
        }
        apply_terminal_status(&mut ledger, GoalStatus::Escalated);
        let content =
            "Conductor goal could not reattach because all declared task references are missing."
                .to_string();
        if !ledger.memos.iter().any(|memo| memo.content == content) {
            ledger.memos.push(ConductorMemo {
                id: Uuid::new_v4().to_string(),
                kind: MemoKind::Escalation,
                content,
                created_at: Utc::now().to_rfc3339(),
                source_chat_id: None,
                related_task_id: ledger.planner_task_id.clone(),
            });
        }
        if save_goal_ledger(project_root, &stored.goal_id, &ledger)
            .await
            .is_ok()
        {
            escalated.push(stored.goal_id);
        }
    }
    escalated
}

async fn all_declared_task_refs_missing(gcx: SharedGlobalContext, ledger: &GoalLedger) -> bool {
    let mut refs = Vec::new();
    if let Some(task_id) = ledger.planner_task_id.as_ref() {
        push_unique(&mut refs, task_id.clone());
    }
    for task_id in &ledger.task_ids {
        push_unique(&mut refs, task_id.clone());
    }
    if refs.is_empty() {
        return false;
    }
    for task_id in refs {
        if crate::tasks::storage::load_task_meta(gcx.clone(), &task_id)
            .await
            .is_ok()
        {
            return false;
        }
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

fn apply_terminal_status(ledger: &mut GoalLedger, status: GoalStatus) {
    ledger.status = Some(status);
    if status.is_terminal() && ledger.completed_at.is_none() {
        ledger.completed_at = Some(Utc::now().to_rfc3339());
    }
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !value.trim().is_empty() && !values.contains(&value) {
        values.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_buddy_core::conductor::ConductorRecurring;
    use refact_buddy_core::conductor_store::{load_goal_ledger, save_goal_ledger};

    async fn test_gcx(root: &std::path::Path) -> SharedGlobalContext {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
        gcx
    }

    #[tokio::test]
    async fn restart_reattach_resumes_active_and_recurring_goals() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx(dir.path()).await;
        save_goal_ledger(
            dir.path(),
            "goal-active",
            &GoalLedger {
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

        let report = reattach_conductor_goals(gcx.clone(), dir.path()).await;

        assert_eq!(report.active_goals, vec!["goal-active".to_string()]);
        assert_eq!(report.heartbeat_enqueued, vec!["goal-active".to_string()]);
        let bus = gcx.conductor_wake_bus.lock().await;
        assert!(bus.mailbox("goal-active").is_some());
    }

    #[tokio::test]
    async fn restart_reattach_does_not_duplicate_in_flight_turn() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx(dir.path()).await;
        save_goal_ledger(dir.path(), "goal-flight", &GoalLedger::default())
            .await
            .unwrap();
        {
            let mut bus = gcx.conductor_wake_bus.lock().await;
            bus.enqueue_goal("goal-flight", ConductorWakeReason::Manual, Utc::now());
            bus.drain_due(Utc::now() + chrono::Duration::seconds(5));
        }

        let report = reattach_conductor_goals(gcx.clone(), dir.path()).await;

        assert!(report.heartbeat_enqueued.is_empty());
        let bus = gcx.conductor_wake_bus.lock().await;
        let mailbox = bus.mailbox("goal-flight").unwrap();
        assert!(mailbox.in_flight);
        assert!(mailbox.reasons.is_empty());
    }

    #[tokio::test]
    async fn missing_task_reference_escalates_gracefully() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx(dir.path()).await;
        save_goal_ledger(
            dir.path(),
            "goal-missing",
            &GoalLedger {
                planner_task_id: Some("task-missing".to_string()),
                task_ids: vec!["task-missing".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let report = reattach_conductor_goals(gcx.clone(), dir.path()).await;

        assert_eq!(
            report.missing_reference_escalated,
            vec!["goal-missing".to_string()]
        );
        let ledger = load_goal_ledger(dir.path(), "goal-missing")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.status, Some(GoalStatus::Escalated));
        assert!(ledger.completed_at.is_some());
        assert!(!gcx
            .conductor_wake_targets
            .lock()
            .await
            .contains_goal("goal-missing"));
        assert!(ledger
            .memos
            .iter()
            .any(|memo| memo.kind == MemoKind::Escalation));
    }
}
