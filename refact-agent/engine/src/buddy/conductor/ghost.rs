use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use hyper::StatusCode;
use refact_buddy_core::conductor::{ConductorGoal, ConductorWakeReason, GoalLedger, PendingQuestion};
use refact_buddy_core::conductor_store::{load_goal_ledger, save_goal_ledger};
use refact_buddy_core::types::{BuddyGhostMessage, BuddyGhostMessageRole};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::buddy::events::BuddyEvent;
use crate::custom_error::ScratchError;
use crate::global_context::GlobalContext;

const MAX_GHOST_CONTENT_CHARS: usize = 4000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhostAskResult {
    pub ghost: BuddyGhostMessage,
    pub question: PendingQuestion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhostAnswerResult {
    pub goal_id: String,
    pub question_id: String,
    pub answered: bool,
}

pub fn cap_ghost_content(value: &str) -> String {
    value.chars().take(MAX_GHOST_CONTENT_CHARS).collect()
}

pub async fn conductor_ghost_say(
    gcx: Arc<GlobalContext>,
    project_root: &Path,
    goal_id: &str,
    content: &str,
    source_chat_id: Option<String>,
) -> Result<BuddyGhostMessage, ScratchError> {
    let mut ledger = load_required(project_root, goal_id).await?;
    let ghost = make_ghost_message(
        goal_id,
        BuddyGhostMessageRole::Say,
        content,
        source_chat_id,
        None,
    );
    ledger.ghost_messages.push(ghost.clone());
    persist_and_emit(gcx, project_root, goal_id, &ledger, ghost.clone()).await?;
    Ok(ghost)
}

pub async fn conductor_ghost_memo(
    gcx: Arc<GlobalContext>,
    project_root: &Path,
    goal_id: &str,
    content: &str,
    source_chat_id: Option<String>,
) -> Result<BuddyGhostMessage, ScratchError> {
    let mut ledger = load_required(project_root, goal_id).await?;
    let ghost = make_ghost_message(
        goal_id,
        BuddyGhostMessageRole::Memo,
        content,
        source_chat_id,
        None,
    );
    ledger.ghost_messages.push(ghost.clone());
    persist_and_emit(gcx, project_root, goal_id, &ledger, ghost.clone()).await?;
    Ok(ghost)
}

pub async fn conductor_ghost_ask(
    gcx: Arc<GlobalContext>,
    project_root: &Path,
    goal_id: &str,
    question: &str,
    blocking: bool,
    source_chat_id: Option<String>,
) -> Result<GhostAskResult, ScratchError> {
    let mut ledger = load_required(project_root, goal_id).await?;
    let question_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let capped_question = cap_ghost_content(question);
    let pending = PendingQuestion {
        id: question_id.clone(),
        question: capped_question.clone(),
        asked_at: now,
        source_chat_id: source_chat_id.clone(),
        blocking,
        answer: None,
        answered_at: None,
    };
    let ghost = make_ghost_message(
        goal_id,
        BuddyGhostMessageRole::Ask,
        &capped_question,
        source_chat_id,
        Some(question_id),
    );
    ledger.pending_questions.push(pending.clone());
    ledger.ghost_messages.push(ghost.clone());
    persist_and_emit(gcx, project_root, goal_id, &ledger, ghost.clone()).await?;
    Ok(GhostAskResult {
        ghost,
        question: pending,
    })
}

pub async fn conductor_ghost_answer(
    app: AppState,
    project_root: &Path,
    goal_id: &str,
    question_id: &str,
    answer: &str,
) -> Result<GhostAnswerResult, ScratchError> {
    let mut ledger = load_goal_ledger(project_root, goal_id)
        .await
        .map_err(store_error)?
        .ok_or_else(|| {
            ScratchError::new(StatusCode::NOT_FOUND, format!("goal not found: {goal_id}"))
        })?;
    let question = ledger
        .pending_questions
        .iter_mut()
        .find(|question| question.id == question_id)
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::NOT_FOUND,
                format!("question not found: {question_id}"),
            )
        })?;
    question.answer = Some(cap_ghost_content(answer));
    question.answered_at = Some(Utc::now().to_rfc3339());
    save_goal_ledger(project_root, goal_id, &ledger)
        .await
        .map_err(store_error)?;
    crate::buddy::conductor::wake::refresh_conductor_wake_targets(app.gcx.clone()).await;
    emit_goal_updated(app.gcx.clone(), goal_id, ledger).await;
    let enqueued = crate::buddy::conductor::wake::enqueue_goal_wake_after_target_refresh(
        app.gcx.clone(),
        goal_id,
        ConductorWakeReason::GhostAnswer,
    )
    .await;
    Ok(GhostAnswerResult {
        goal_id: goal_id.to_string(),
        question_id: question_id.to_string(),
        answered: enqueued,
    })
}

fn make_ghost_message(
    goal_id: &str,
    role: BuddyGhostMessageRole,
    content: &str,
    source_chat_id: Option<String>,
    question_id: Option<String>,
) -> BuddyGhostMessage {
    BuddyGhostMessage {
        id: Uuid::new_v4().to_string(),
        goal_id: Some(goal_id.to_string()),
        role,
        content: cap_ghost_content(content),
        created_at: Utc::now().to_rfc3339(),
        source_chat_id,
        question_id,
    }
}

async fn load_required(project_root: &Path, goal_id: &str) -> Result<GoalLedger, ScratchError> {
    load_goal_ledger(project_root, goal_id)
        .await
        .map_err(store_error)
        .and_then(|ledger| {
            ledger.ok_or_else(|| {
                ScratchError::new(StatusCode::NOT_FOUND, format!("goal not found: {goal_id}"))
            })
        })
}

async fn persist_and_emit(
    gcx: Arc<GlobalContext>,
    project_root: &Path,
    goal_id: &str,
    ledger: &GoalLedger,
    ghost: BuddyGhostMessage,
) -> Result<(), ScratchError> {
    save_goal_ledger(project_root, goal_id, ledger)
        .await
        .map_err(store_error)?;
    crate::buddy::conductor::wake::refresh_conductor_wake_targets(gcx.clone()).await;
    if let Some(tx) = gcx.buddy_events_tx.as_ref() {
        let _ = tx.send(BuddyEvent::ConductorGhostMessage { ghost });
    }
    emit_goal_updated(gcx, goal_id, ledger.clone()).await;
    Ok(())
}

async fn emit_goal_updated(gcx: Arc<GlobalContext>, goal_id: &str, ledger: GoalLedger) {
    if let Some(tx) = gcx.buddy_events_tx.as_ref() {
        let _ = tx.send(BuddyEvent::ConductorGoalUpdated {
            goal: ConductorGoal::from_ledger(goal_id.to_string(), ledger),
        });
    }
}

fn store_error(error: refact_buddy_core::conductor_store::ConductorStoreError) -> ScratchError {
    let status = match error {
        refact_buddy_core::conductor_store::ConductorStoreError::InvalidGoalId(_) => {
            StatusCode::BAD_REQUEST
        }
        refact_buddy_core::conductor_store::ConductorStoreError::Io { .. }
        | refact_buddy_core::conductor_store::ConductorStoreError::Json { .. } => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    };
    ScratchError::new(status, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_buddy_core::conductor::{GoalBudget, GoalLedger, GoalStatus};
    use refact_buddy_core::conductor_store::save_goal_ledger;
    use tokio::time::timeout;

    async fn gcx(root: &Path) -> Arc<GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
        gcx
    }

    #[tokio::test]
    async fn conductor_ghost_say_persists_and_emits_event() {
        let dir = tempfile::tempdir().unwrap();
        save_goal_ledger(dir.path(), "goal-say", &GoalLedger::default())
            .await
            .unwrap();
        let gcx = gcx(dir.path()).await;
        let mut rx = gcx.buddy_events_tx.as_ref().unwrap().subscribe();

        let ghost = conductor_ghost_say(
            gcx.clone(),
            dir.path(),
            "goal-say",
            "Tiny status sparkle",
            Some("conductor-chat".to_string()),
        )
        .await
        .unwrap();

        assert_eq!(ghost.role, BuddyGhostMessageRole::Say);
        assert_eq!(ghost.goal_id.as_deref(), Some("goal-say"));
        assert_eq!(ghost.source_chat_id.as_deref(), Some("conductor-chat"));
        let ledger = load_goal_ledger(dir.path(), "goal-say")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.ghost_messages, vec![ghost.clone()]);
        let event = timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match event {
            BuddyEvent::ConductorGhostMessage { ghost: emitted } => assert_eq!(emitted, ghost),
            other => panic!("expected ConductorGhostMessage, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn conductor_ghost_ask_persists_question_with_controls() {
        let dir = tempfile::tempdir().unwrap();
        save_goal_ledger(dir.path(), "goal-ask", &GoalLedger::default())
            .await
            .unwrap();
        let gcx = gcx(dir.path()).await;
        let mut rx = gcx.buddy_events_tx.as_ref().unwrap().subscribe();

        let result = conductor_ghost_ask(
            gcx,
            dir.path(),
            "goal-ask",
            "Should chaos continue?",
            true,
            Some("conductor-chat".to_string()),
        )
        .await
        .unwrap();

        assert_eq!(result.ghost.role, BuddyGhostMessageRole::Ask);
        assert_eq!(
            result.ghost.question_id.as_deref(),
            Some(result.question.id.as_str())
        );
        assert!(result.question.blocking);
        let ledger = load_goal_ledger(dir.path(), "goal-ask")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.pending_questions, vec![result.question.clone()]);
        assert_eq!(ledger.ghost_messages, vec![result.ghost.clone()]);
        let event = timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match event {
            BuddyEvent::ConductorGhostMessage { ghost } => assert_eq!(ghost, result.ghost),
            other => panic!("expected ConductorGhostMessage, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn conductor_ghost_say_rejects_missing_goal_without_creating_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = gcx(dir.path()).await;

        let error = conductor_ghost_say(gcx, dir.path(), "ghost-missing", "boo", None)
            .await
            .unwrap_err();

        assert_eq!(error.status_code, StatusCode::NOT_FOUND);
        assert!(error.message.contains("goal not found"));
        assert_eq!(
            load_goal_ledger(dir.path(), "ghost-missing").await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn conductor_ghost_answer_persists_and_emits_complete_goal_update() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = gcx(dir.path()).await;
        let question = PendingQuestion {
            id: "question-1".to_string(),
            question: "Continue?".to_string(),
            asked_at: "2026-06-03T00:00:00Z".to_string(),
            blocking: true,
            ..Default::default()
        };
        save_goal_ledger(
            dir.path(),
            "goal-answer-event",
            &GoalLedger {
                title: Some("Answerable goal".to_string()),
                budget: Some(GoalBudget {
                    wall_clock_secs: Some(120),
                    no_progress_wakes: Some(2),
                    ..Default::default()
                }),
                pending_questions: vec![question],
                no_progress_wakes: 1,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mut rx = gcx.buddy_events_tx.as_ref().unwrap().subscribe();
        let app = AppState::from_gcx(gcx).await;

        let result =
            conductor_ghost_answer(app, dir.path(), "goal-answer-event", "question-1", "Yes")
                .await
                .unwrap();

        assert!(result.answered);
        let event = timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match event {
            BuddyEvent::ConductorGoalUpdated { goal } => {
                assert_eq!(goal.id, "goal-answer-event");
                assert_eq!(goal.title, "Answerable goal");
                assert_eq!(goal.budget.wall_clock_secs, Some(120));
                assert_eq!(goal.spent.no_progress_wakes, 1);
                assert_eq!(
                    goal.ledger.pending_questions[0].answer.as_deref(),
                    Some("Yes")
                );
            }
            other => panic!("expected ConductorGoalUpdated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn conductor_ghost_answer_active_goal_enqueues_ghost_answer() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = gcx(dir.path()).await;
        save_answerable_goal(dir.path(), "goal-answer-active", GoalStatus::Running).await;
        let app = AppState::from_gcx(gcx.clone()).await;

        let result =
            conductor_ghost_answer(app, dir.path(), "goal-answer-active", "question-1", "Yes")
                .await
                .unwrap();

        assert!(result.answered);
        let ledger = load_goal_ledger(dir.path(), "goal-answer-active")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.pending_questions[0].answer.as_deref(), Some("Yes"));
        let bus = gcx.conductor_wake_bus.lock().await;
        let mailbox = bus.mailbox("goal-answer-active").unwrap();
        assert_eq!(mailbox.reasons, vec![ConductorWakeReason::GhostAnswer]);
    }

    #[tokio::test]
    async fn conductor_ghost_answer_terminal_goal_persists_without_wake() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = gcx(dir.path()).await;
        save_answerable_goal(dir.path(), "goal-answer-done", GoalStatus::Done).await;
        let app = AppState::from_gcx(gcx.clone()).await;

        let result = conductor_ghost_answer(
            app,
            dir.path(),
            "goal-answer-done",
            "question-1",
            "Done answer",
        )
        .await
        .unwrap();

        assert!(!result.answered);
        let ledger = load_goal_ledger(dir.path(), "goal-answer-done")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            ledger.pending_questions[0].answer.as_deref(),
            Some("Done answer")
        );
        assert!(gcx
            .conductor_wake_bus
            .lock()
            .await
            .mailbox("goal-answer-done")
            .is_none());
    }

    #[tokio::test]
    async fn conductor_ghost_answer_paused_goal_persists_without_wake() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = gcx(dir.path()).await;
        save_answerable_goal(dir.path(), "goal-answer-paused", GoalStatus::Paused).await;
        let app = AppState::from_gcx(gcx.clone()).await;

        let result = conductor_ghost_answer(
            app,
            dir.path(),
            "goal-answer-paused",
            "question-1",
            "Pause answer",
        )
        .await
        .unwrap();

        assert!(!result.answered);
        let ledger = load_goal_ledger(dir.path(), "goal-answer-paused")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            ledger.pending_questions[0].answer.as_deref(),
            Some("Pause answer")
        );
        assert!(gcx
            .conductor_wake_bus
            .lock()
            .await
            .mailbox("goal-answer-paused")
            .is_none());
    }

    async fn save_answerable_goal(project_root: &Path, goal_id: &str, status: GoalStatus) {
        save_goal_ledger(
            project_root,
            goal_id,
            &GoalLedger {
                status: Some(status),
                pending_questions: vec![PendingQuestion {
                    id: "question-1".to_string(),
                    question: "Continue?".to_string(),
                    asked_at: "2026-06-03T00:00:00Z".to_string(),
                    blocking: true,
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
}
