use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use hyper::StatusCode;
use refact_buddy_core::conductor::{
    ConductorGoal, ConductorWakeReason, GoalLedger, GoalStatus, PendingQuestion,
    PublicConductorGoal,
};
use refact_buddy_core::conductor_store::{mutate_goal_ledger, MissingGoalBehavior};
use refact_buddy_core::types::{BuddyGhostMessage, BuddyGhostMessageRole};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::buddy::conductor::budget::hydrate_goal_spent;
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
    let ghost = make_ghost_message(
        goal_id,
        BuddyGhostMessageRole::Say,
        content,
        source_chat_id,
        None,
    );
    let (ledger, ()) = mutate_required(project_root, goal_id, |ledger| {
        ledger.ghost_messages.push(ghost.clone());
        Ok(())
    })
    .await?;
    emit_after_mutation(gcx, project_root, goal_id, &ledger, Some(ghost.clone())).await;
    Ok(ghost)
}

pub async fn conductor_ghost_memo(
    gcx: Arc<GlobalContext>,
    project_root: &Path,
    goal_id: &str,
    content: &str,
    source_chat_id: Option<String>,
) -> Result<BuddyGhostMessage, ScratchError> {
    let ghost = make_ghost_message(
        goal_id,
        BuddyGhostMessageRole::Memo,
        content,
        source_chat_id,
        None,
    );
    let (ledger, ()) = mutate_required(project_root, goal_id, |ledger| {
        ledger.ghost_messages.push(ghost.clone());
        Ok(())
    })
    .await?;
    emit_after_mutation(gcx, project_root, goal_id, &ledger, Some(ghost.clone())).await;
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
    let (ledger, ()) = mutate_required(project_root, goal_id, |ledger| {
        ledger.pending_questions.push(pending.clone());
        ledger.ghost_messages.push(ghost.clone());
        Ok(())
    })
    .await?;
    emit_after_mutation(gcx, project_root, goal_id, &ledger, Some(ghost.clone())).await;
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
    let capped_answer = cap_ghost_content(answer);
    let (ledger, ()) = mutate_required(project_root, goal_id, |ledger| {
        let status = ledger.status.unwrap_or_default();
        if status != GoalStatus::Active {
            return Err(GhostAnswerMutationError::InactiveGoal(status).to_string());
        }
        let question = ledger
            .pending_questions
            .iter_mut()
            .find(|question| question.id == question_id)
            .ok_or_else(|| {
                GhostAnswerMutationError::QuestionNotFound(question_id.to_string()).to_string()
            })?;
        if question.answer.is_some() {
            return Err(
                GhostAnswerMutationError::DuplicateAnswer(question_id.to_string()).to_string(),
            );
        }
        question.answer = Some(capped_answer);
        question.answered_at = Some(Utc::now().to_rfc3339());
        Ok(())
    })
    .await?;
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum GhostAnswerMutationError {
    InactiveGoal(GoalStatus),
    QuestionNotFound(String),
    DuplicateAnswer(String),
}

impl std::fmt::Display for GhostAnswerMutationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InactiveGoal(status) => write!(f, "inactive conductor goal: {status:?}"),
            Self::QuestionNotFound(question_id) => write!(f, "question not found: {question_id}"),
            Self::DuplicateAnswer(question_id) => {
                write!(f, "question already answered: {question_id}")
            }
        }
    }
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

async fn mutate_required<R>(
    project_root: &Path,
    goal_id: &str,
    updater: impl FnOnce(&mut GoalLedger) -> Result<R, String>,
) -> Result<(GoalLedger, R), ScratchError> {
    mutate_goal_ledger(
        project_root,
        goal_id,
        MissingGoalBehavior::RequireExisting,
        updater,
    )
    .await
    .map_err(store_error)
}

async fn emit_after_mutation(
    gcx: Arc<GlobalContext>,
    project_root: &Path,
    goal_id: &str,
    ledger: &GoalLedger,
    ghost: Option<BuddyGhostMessage>,
) {
    crate::buddy::conductor::wake::refresh_conductor_wake_targets_for_project(
        gcx.clone(),
        project_root,
    )
    .await;
    if let (Some(tx), Some(ghost)) = (gcx.buddy_events_tx.as_ref(), ghost) {
        let _ = tx.send(BuddyEvent::ConductorGhostMessage { ghost });
    }
    emit_goal_updated(gcx, goal_id, ledger.clone()).await;
}

async fn emit_goal_updated(gcx: Arc<GlobalContext>, goal_id: &str, ledger: GoalLedger) {
    if let Some(tx) = gcx.buddy_events_tx.as_ref() {
        let goal = hydrate_goal_spent(
            gcx.clone(),
            ConductorGoal::from_ledger(goal_id.to_string(), ledger),
        )
        .await;
        let _ = tx.send(BuddyEvent::ConductorGoalUpdated {
            goal: PublicConductorGoal::from(goal),
        });
    }
}

fn store_error(error: refact_buddy_core::conductor_store::ConductorStoreError) -> ScratchError {
    let status = match error {
        refact_buddy_core::conductor_store::ConductorStoreError::InvalidGoalId(_) => {
            StatusCode::BAD_REQUEST
        }
        refact_buddy_core::conductor_store::ConductorStoreError::MissingGoal(_) => {
            StatusCode::NOT_FOUND
        }
        refact_buddy_core::conductor_store::ConductorStoreError::Update(ref error)
            if error.contains("question not found") =>
        {
            StatusCode::NOT_FOUND
        }
        refact_buddy_core::conductor_store::ConductorStoreError::Update(ref error)
            if error.contains("already answered") || error.contains("inactive conductor goal") =>
        {
            StatusCode::CONFLICT
        }
        refact_buddy_core::conductor_store::ConductorStoreError::Update(_) => StatusCode::NOT_FOUND,
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
    use refact_buddy_core::conductor_store::{load_goal_ledger, save_goal_ledger};
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
                assert_eq!(goal.summary.pending_question_count, 1);
                assert_eq!(goal.summary.open_question_count, 0);
            }
            other => panic!("expected ConductorGoalUpdated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn conductor_ghost_answer_active_goal_enqueues_ghost_answer() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = gcx(dir.path()).await;
        save_answerable_goal(dir.path(), "goal-answer-active", GoalStatus::Active).await;
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
    async fn conductor_ghost_answer_duplicate_returns_conflict_without_mutation_or_wake() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = gcx(dir.path()).await;
        save_answerable_goal(dir.path(), "goal-answer-duplicate", GoalStatus::Active).await;
        let app = AppState::from_gcx(gcx.clone()).await;

        conductor_ghost_answer(
            app.clone(),
            dir.path(),
            "goal-answer-duplicate",
            "question-1",
            "Original",
        )
        .await
        .unwrap();
        let error = conductor_ghost_answer(
            app,
            dir.path(),
            "goal-answer-duplicate",
            "question-1",
            "Replacement",
        )
        .await
        .unwrap_err();

        assert_eq!(error.status_code, StatusCode::CONFLICT);
        assert!(error.message.contains("already answered"));
        let ledger = load_goal_ledger(dir.path(), "goal-answer-duplicate")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            ledger.pending_questions[0].answer.as_deref(),
            Some("Original")
        );
        let bus = gcx.conductor_wake_bus.lock().await;
        let mailbox = bus.mailbox("goal-answer-duplicate").unwrap();
        assert_eq!(mailbox.reasons, vec![ConductorWakeReason::GhostAnswer]);
    }

    #[tokio::test]
    async fn conductor_ghost_answer_missing_question_returns_not_found_without_wake() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = gcx(dir.path()).await;
        save_answerable_goal(
            dir.path(),
            "goal-answer-missing-question",
            GoalStatus::Active,
        )
        .await;
        let app = AppState::from_gcx(gcx.clone()).await;

        let error = conductor_ghost_answer(
            app,
            dir.path(),
            "goal-answer-missing-question",
            "missing-question",
            "Nope",
        )
        .await
        .unwrap_err();

        assert_eq!(error.status_code, StatusCode::NOT_FOUND);
        assert!(error.message.contains("question not found"));
        let ledger = load_goal_ledger(dir.path(), "goal-answer-missing-question")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.pending_questions[0].answer, None);
        assert!(gcx
            .conductor_wake_bus
            .lock()
            .await
            .mailbox("goal-answer-missing-question")
            .is_none());
    }

    #[tokio::test]
    async fn conductor_ghost_answer_terminal_goal_returns_conflict_without_mutation_or_wake() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = gcx(dir.path()).await;
        save_answerable_goal(dir.path(), "goal-answer-done", GoalStatus::Done).await;
        let app = AppState::from_gcx(gcx.clone()).await;

        let error = conductor_ghost_answer(
            app,
            dir.path(),
            "goal-answer-done",
            "question-1",
            "Done answer",
        )
        .await
        .unwrap_err();

        assert_eq!(error.status_code, StatusCode::CONFLICT);
        assert!(error.message.contains("inactive conductor goal"));
        let ledger = load_goal_ledger(dir.path(), "goal-answer-done")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.pending_questions[0].answer, None);
        assert!(gcx
            .conductor_wake_bus
            .lock()
            .await
            .mailbox("goal-answer-done")
            .is_none());
    }

    #[tokio::test]
    async fn conductor_ghost_answer_paused_goal_returns_conflict_without_mutation_or_wake() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = gcx(dir.path()).await;
        save_answerable_goal(dir.path(), "goal-answer-paused", GoalStatus::Paused).await;
        let app = AppState::from_gcx(gcx.clone()).await;

        let error = conductor_ghost_answer(
            app,
            dir.path(),
            "goal-answer-paused",
            "question-1",
            "Pause answer",
        )
        .await
        .unwrap_err();

        assert_eq!(error.status_code, StatusCode::CONFLICT);
        assert!(error.message.contains("inactive conductor goal"));
        let ledger = load_goal_ledger(dir.path(), "goal-answer-paused")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.pending_questions[0].answer, None);
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
