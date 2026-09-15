//! Engine-side facade over the single tracked question service
//! (`refact_chat_api::QuestionRegistry`). It resolves the configured bounds,
//! applies peer authority to cross-tree questions, and owns the one safe
//! blocking wait: a waiter releases its execution permit before parking and
//! reacquires it before continuing.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::Utc;
use refact_chat_api::{AskQuestion, QuestionKind, QuestionLimits, QuestionSettlement};

use crate::app_state::AppState;
use crate::coordination::peer_authority::{PeerAuthority, PeerGrantRegistry};

pub fn limits_from_settings() -> QuestionLimits {
    let settings = refact_work_core::coordination_settings();
    QuestionLimits {
        max_pending: settings.max_pending_questions,
        default_expiry_secs: settings.question_default_expiry_secs,
        max_expiry_secs: settings.question_max_expiry_secs,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityOutcome {
    Allowed,
    EscalateToUser,
}

/// `PlanApproval` and `PermissionEscalation` are decisions, not messages, so
/// they need the strongest peer authority. Everything else is question traffic.
pub fn required_authority(kind: QuestionKind) -> PeerAuthority {
    match kind {
        QuestionKind::PlanApproval | QuestionKind::PermissionEscalation => PeerAuthority::Control,
        _ => PeerAuthority::Mail,
    }
}

/// Fail closed. Inside one authorized tree a parent may ask a child and a child
/// may ask its parent. Across trees the asker needs an explicit peer grant of at
/// least the authority the question kind demands; a parent that does not itself
/// hold that authority cannot grant it, so a decision escalates to the user
/// instead of being answered by an unauthorized peer.
pub fn resolve_authority(
    grants: &PeerGrantRegistry,
    project: &str,
    asker_root: &str,
    answerer_root: &str,
    kind: QuestionKind,
    now_ms: u64,
) -> Result<AuthorityOutcome, String> {
    if kind == QuestionKind::UserInput || answerer_root == "user" {
        return Ok(AuthorityOutcome::Allowed);
    }
    if asker_root.trim().is_empty() || answerer_root.trim().is_empty() || project.trim().is_empty()
    {
        return Err("question authority requires a project and both principals".to_string());
    }
    if asker_root == answerer_root {
        return Ok(AuthorityOutcome::Allowed);
    }
    let required = required_authority(kind);
    if grants.allows(project, asker_root, answerer_root, required, now_ms) {
        return Ok(AuthorityOutcome::Allowed);
    }
    match kind {
        QuestionKind::PlanApproval | QuestionKind::PermissionEscalation => {
            Ok(AuthorityOutcome::EscalateToUser)
        }
        _ => Err(format!(
            "{answerer_root} is outside the authorized tree of {asker_root} and holds no {required:?} peer grant"
        )),
    }
}

/// Park the waiter's execution so its admission permit is released while it
/// idles, run `waiting`, then reacquire before the caller continues. Never
/// holds a permit across an idle wait.
pub async fn park_and_wait<T>(
    app: &AppState,
    waiter_chat_id: &str,
    abort: &Arc<AtomicBool>,
    waiting: impl std::future::Future<Output = T>,
) -> Result<T, String> {
    let execution = app.agent_executions.get(waiter_chat_id);
    if let Some(execution) = &execution {
        execution.park().await.map_err(|error| error.to_string())?;
    }
    let poll_interval = refact_work_core::coordination_settings().lock_poll_interval();
    tokio::pin!(waiting);
    let outcome = loop {
        if abort.load(Ordering::Relaxed)
            || app.runtime.shutdown_flag.load(Ordering::Relaxed)
            || execution
                .as_ref()
                .is_some_and(|execution| execution.is_cancelled())
        {
            break Err("question wait cancelled".to_string());
        }
        tokio::select! {
            value = &mut waiting => break Ok(value),
            _ = tokio::time::sleep(poll_interval) => {}
        }
    };
    if let Some(execution) = &execution {
        let resumed = execution.resume().await.map_err(|error| error.to_string());
        if outcome.is_ok() {
            resumed?;
        }
    }
    outcome
}

/// Ask a question and block on it safely: the waiter parks, the service settles
/// the question exactly once, and the waiter reacquires its permit before it is
/// told the outcome.
pub async fn ask_and_wait(
    app: &AppState,
    waiter_chat_id: &str,
    request: AskQuestion,
    abort: &Arc<AtomicBool>,
) -> Result<QuestionSettlement, String> {
    let now = Utc::now();
    let question = app
        .questions()
        .ask(request, now)
        .map_err(|error| error.to_string())?;
    crate::coordination::causal::record_question(app, waiter_chat_id, &question.id).await;
    let questions = app.questions().clone();
    let id = question.id.clone();
    let settled = park_and_wait(app, waiter_chat_id, abort, async move {
        questions.wait(&id, now).await
    })
    .await;
    match settled {
        Ok(settled) => settled.map_err(|error| error.to_string()),
        Err(error) => {
            let _ = app
                .questions()
                .cancel(&question.id, &question.asker, Utc::now());
            Err(error)
        }
    }
}

/// True while any of these principals has a question the service still
/// considers open, so the stall monitor can tell an idle agent apart from one
/// that is waiting for an answer.
pub fn has_open_question(app: &AppState, askers: &[&str]) -> bool {
    let now = Utc::now();
    let questions = app.questions();
    askers
        .iter()
        .any(|asker| !questions.pending_for_asker(asker, now).is_empty())
}

/// The asker principal a board card uses for its planner questions.
pub fn board_card_asker(task_id: &str, card_id: &str) -> String {
    format!("task:{task_id}:card:{card_id}")
}

/// The answering principal for board planner questions. Deliberately derived
/// from the task rather than the live planner chat id, so the correlation
/// survives a restart.
pub fn board_planner_answerer(task_id: &str) -> String {
    format!("task:{task_id}:planner")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordination::peer_authority::PeerGrant;

    const KINDS: [QuestionKind; 5] = [
        QuestionKind::UserInput,
        QuestionKind::AgentParent,
        QuestionKind::BoardPlanner,
        QuestionKind::PlanApproval,
        QuestionKind::PermissionEscalation,
    ];

    fn grant(authority: PeerAuthority) -> PeerGrant {
        PeerGrant {
            project: "project".into(),
            sender_root: "parent-root".into(),
            recipient_root: "peer-root".into(),
            authority,
            granted_by: "user".into(),
            expires_at_ms: 1_000,
        }
    }

    #[test]
    fn one_tree_is_authorized_in_both_directions_for_every_kind() {
        let grants = PeerGrantRegistry::default();
        for kind in KINDS {
            assert_eq!(
                resolve_authority(&grants, "project", "root", "root", kind, 1),
                Ok(AuthorityOutcome::Allowed)
            );
        }
    }

    #[test]
    fn a_parent_cannot_grant_authority_it_lacks_and_escalates_instead() {
        let grants = PeerGrantRegistry::default();
        grants.grant(grant(PeerAuthority::Mail), 1).unwrap();
        for kind in [
            QuestionKind::PlanApproval,
            QuestionKind::PermissionEscalation,
        ] {
            assert_eq!(required_authority(kind), PeerAuthority::Control);
            assert_eq!(
                resolve_authority(&grants, "project", "parent-root", "peer-root", kind, 2),
                Ok(AuthorityOutcome::EscalateToUser)
            );
        }
        grants.grant(grant(PeerAuthority::Control), 2).unwrap();
        for kind in [
            QuestionKind::PlanApproval,
            QuestionKind::PermissionEscalation,
        ] {
            assert_eq!(
                resolve_authority(&grants, "project", "parent-root", "peer-root", kind, 3),
                Ok(AuthorityOutcome::Allowed)
            );
        }
    }

    #[test]
    fn unresolved_cross_tree_authority_refuses_rather_than_defaulting_to_allow() {
        let grants = PeerGrantRegistry::default();
        for kind in [QuestionKind::AgentParent, QuestionKind::BoardPlanner] {
            assert!(
                resolve_authority(&grants, "project", "parent-root", "peer-root", kind, 1).is_err()
            );
        }
        grants.grant(grant(PeerAuthority::Mail), 1).unwrap();
        assert_eq!(
            resolve_authority(
                &grants,
                "project",
                "parent-root",
                "peer-root",
                QuestionKind::AgentParent,
                2
            ),
            Ok(AuthorityOutcome::Allowed)
        );
        assert!(resolve_authority(
            &grants,
            "other-project",
            "parent-root",
            "peer-root",
            QuestionKind::AgentParent,
            2
        )
        .is_err());
        assert!(resolve_authority(
            &grants,
            "project",
            "",
            "peer-root",
            QuestionKind::AgentParent,
            2
        )
        .is_err());
    }

    #[test]
    fn user_input_never_needs_a_peer_grant() {
        let grants = PeerGrantRegistry::default();
        assert_eq!(
            resolve_authority(
                &grants,
                "project",
                "parent-root",
                "user",
                QuestionKind::UserInput,
                1
            ),
            Ok(AuthorityOutcome::Allowed)
        );
    }

    async fn app_with_cap(cap: usize) -> AppState {
        use refact_work_core::{AdmissionScheduler, CoordinationSettings};
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut app = AppState::from_gcx(gcx).await;
        app.work_scheduler = Arc::new(AdmissionScheduler::new(CoordinationSettings {
            max_active_engine: cap,
            max_active_project: cap,
            max_active_root: cap,
            ..CoordinationSettings::default()
        }));
        app
    }

    async fn parked_execution(
        app: &AppState,
        chat_id: &str,
    ) -> Arc<crate::agents::execution::RunExecution> {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Some(execution) = app.agent_executions.get(chat_id) {
                    if execution.state() == crate::agents::execution::ExecutionState::Parked {
                        return execution;
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("waiter never parked")
    }

    #[tokio::test]
    async fn a_waiter_releases_its_permit_and_reacquires_before_continuing() {
        let app = app_with_cap(1).await;
        let parent = format!("parent-{}", uuid::Uuid::new_v4());
        let other = format!("other-{}", uuid::Uuid::new_v4());
        crate::tasks::admission::reserve(&app, &parent, "project", "root", 0)
            .await
            .unwrap();
        crate::tasks::admission::wait_for_permit(&parent)
            .await
            .unwrap();
        crate::tasks::admission::reserve(&app, &other, "project", "root", 0)
            .await
            .unwrap();
        let queued = tokio::spawn({
            let other = other.clone();
            async move { crate::tasks::admission::wait_for_permit(&other).await }
        });

        let abort = Arc::new(AtomicBool::new(false));
        let asking = tokio::spawn({
            let app = app.clone();
            let parent = parent.clone();
            let abort = abort.clone();
            async move {
                ask_and_wait(
                    &app,
                    &parent,
                    AskQuestion::new(
                        QuestionKind::AgentParent,
                        &parent,
                        "answerer",
                        "correlation",
                        "May I continue?",
                    )
                    .with_id("parked-question"),
                    &abort,
                )
                .await
            }
        });

        let execution = parked_execution(&app, &parent).await;
        assert!(
            !execution.holds_permit(),
            "a waiting agent must not hold an execution permit"
        );
        queued.await.unwrap().unwrap();
        assert_eq!(app.work_scheduler.snapshot().active, 1);

        app.questions()
            .answer("parked-question", "answerer", "yes".into(), Utc::now())
            .unwrap();
        crate::tasks::admission::end(&other).await;

        assert_eq!(
            asking.await.unwrap().unwrap(),
            QuestionSettlement::Answered("yes".into())
        );
        assert!(
            execution.holds_permit(),
            "a resumed agent must hold its permit again"
        );
        drop(execution);
        crate::tasks::admission::end(&parent).await;
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
    }

    #[tokio::test]
    async fn an_abandoned_wait_cancels_its_question_and_gives_the_permit_back() {
        let app = app_with_cap(1).await;
        let parent = format!("parent-{}", uuid::Uuid::new_v4());
        crate::tasks::admission::reserve(&app, &parent, "project", "root", 0)
            .await
            .unwrap();
        crate::tasks::admission::wait_for_permit(&parent)
            .await
            .unwrap();

        let abort = Arc::new(AtomicBool::new(false));
        let asking = tokio::spawn({
            let app = app.clone();
            let parent = parent.clone();
            let abort = abort.clone();
            async move {
                ask_and_wait(
                    &app,
                    &parent,
                    AskQuestion::new(
                        QuestionKind::BoardPlanner,
                        &parent,
                        "answerer",
                        "correlation",
                        "May I continue?",
                    )
                    .with_id("abandoned-question"),
                    &abort,
                )
                .await
            }
        });
        parked_execution(&app, &parent).await;
        abort.store(true, Ordering::Relaxed);
        assert!(asking.await.unwrap().is_err());
        assert_eq!(
            app.questions().get("abandoned-question").unwrap().state,
            refact_chat_api::QuestionState::Cancelled
        );
        assert!(app.agent_executions.get(&parent).unwrap().holds_permit());
        crate::tasks::admission::end(&parent).await;
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
    }

    #[test]
    fn configured_limits_come_from_coordination_settings() {
        let limits = limits_from_settings();
        limits.validate().unwrap();
        let settings = refact_work_core::coordination_settings();
        assert_eq!(limits.max_pending, settings.max_pending_questions);
        assert_eq!(
            limits.default_expiry_secs,
            settings.question_default_expiry_secs
        );
        assert_eq!(limits.max_expiry_secs, settings.question_max_expiry_secs);
    }
}
