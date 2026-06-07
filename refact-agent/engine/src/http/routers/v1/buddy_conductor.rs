use axum::extract::{Path, State};
use axum::response::Result;
use chrono::Utc;
use hyper::StatusCode;
use refact_buddy_core::conductor::{
    ConductorGoal, ConductorWakeReason, GoalAutonomy, GoalStatus, validate_goal_for_create,
    validate_goal_status_transition,
};
use refact_buddy_core::conductor_store::{
    ConductorStoreError, list_goal_ledgers, load_goal_ledger, save_goal_ledger,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::buddy::conductor::budget::hydrate_goal_spent;
use crate::buddy::events::BuddyEvent;
use crate::custom_error::ScratchError;

#[derive(Debug, Clone, Serialize)]
pub struct ConductorGoalsResponse {
    pub goals: Vec<ConductorGoal>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateConductorGoalRequest {
    #[serde(default)]
    pub id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub plan_doc_slug: Option<String>,
    #[serde(default)]
    pub plan_markdown: String,
    #[serde(default)]
    pub done_when: refact_buddy_core::conductor::DoneWhen,
    #[serde(default)]
    pub autonomy: GoalAutonomy,
    pub budget: refact_buddy_core::conductor::GoalBudget,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PatchConductorGoalRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub plan_doc_slug: Option<Option<String>>,
    #[serde(default)]
    pub plan_markdown: Option<String>,
    #[serde(default)]
    pub done_when: Option<refact_buddy_core::conductor::DoneWhen>,
    #[serde(default)]
    pub autonomy: Option<GoalAutonomy>,
    #[serde(default)]
    pub status: Option<GoalStatus>,
    #[serde(default)]
    pub budget: Option<refact_buddy_core::conductor::GoalBudget>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AutonomyRequest {
    pub autonomy: GoalAutonomy,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConductorAnswerRequest {
    pub goal_id: String,
    pub question_id: String,
    pub answer: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConductorAnswerResponse {
    pub goal_id: String,
    pub question_id: String,
    pub answered: bool,
}

pub async fn handle_v1_buddy_conductor_goals_list(
    State(app): State<AppState>,
) -> Result<axum::Json<ConductorGoalsResponse>, ScratchError> {
    let project_root = project_root(&app).await?;
    let stored_goals = list_goal_ledgers(&project_root)
        .await
        .map_err(store_error)?;
    let mut goals = Vec::with_capacity(stored_goals.len());
    for stored in stored_goals {
        goals.push(
            hydrate_goal_spent(
                app.gcx.clone(),
                ConductorGoal::from_ledger(stored.goal_id, stored.ledger),
            )
            .await,
        );
    }
    Ok(axum::Json(ConductorGoalsResponse { goals }))
}

pub async fn handle_v1_buddy_conductor_goal_create(
    State(app): State<AppState>,
    axum::Json(req): axum::Json<CreateConductorGoalRequest>,
) -> Result<axum::Json<ConductorGoal>, ScratchError> {
    let project_root = project_root(&app).await?;
    let now = Utc::now().to_rfc3339();
    let id = normalized_goal_id(req.id).unwrap_or_else(|| Uuid::new_v4().to_string());
    let title = req.title.trim().to_string();
    if title.is_empty() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "title is required".to_string(),
        ));
    }
    if load_goal_ledger(&project_root, &id)
        .await
        .map_err(store_error)?
        .is_some()
    {
        return Err(ScratchError::new(
            StatusCode::CONFLICT,
            format!("conductor goal already exists: {id}"),
        ));
    }
    let mut goal = ConductorGoal {
        id,
        title,
        plan_doc_slug: req.plan_doc_slug.and_then(non_empty),
        plan_markdown: req.plan_markdown,
        done_when: req.done_when,
        status: GoalStatus::Active,
        autonomy: req.autonomy,
        budget: req.budget,
        created_at: Some(now.clone()),
        updated_at: Some(now),
        ..ConductorGoal::default()
    };
    validate_active_operation(&goal)?;
    let metadata = goal.clone();
    goal.ledger.apply_goal_metadata(&metadata);
    save_goal_ledger(&project_root, &goal.id, &goal.ledger)
        .await
        .map_err(store_error)?;
    let goal = refresh_targets_and_emit(app, goal, Some(ConductorWakeReason::GoalCreated)).await;
    Ok(axum::Json(goal))
}

pub async fn handle_v1_buddy_conductor_goal_get(
    State(app): State<AppState>,
    Path(goal_id): Path<String>,
) -> Result<axum::Json<ConductorGoal>, ScratchError> {
    Ok(axum::Json(load_goal(&app, &goal_id).await?))
}

pub async fn handle_v1_buddy_conductor_goal_patch(
    State(app): State<AppState>,
    Path(goal_id): Path<String>,
    axum::Json(req): axum::Json<PatchConductorGoalRequest>,
) -> Result<axum::Json<ConductorGoal>, ScratchError> {
    let project_root = project_root(&app).await?;
    let mut goal = load_goal_from_root(&project_root, &goal_id).await?;
    if let Some(title) = req.title {
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(ScratchError::new(
                StatusCode::BAD_REQUEST,
                "title cannot be empty".to_string(),
            ));
        }
        goal.title = title;
    }
    if let Some(plan_doc_slug) = req.plan_doc_slug {
        goal.plan_doc_slug = plan_doc_slug.and_then(non_empty);
    }
    if let Some(plan_markdown) = req.plan_markdown {
        goal.plan_markdown = plan_markdown;
    }
    if let Some(done_when) = req.done_when {
        goal.done_when = done_when;
    }
    if let Some(autonomy) = req.autonomy {
        goal.autonomy = autonomy;
    }
    if let Some(status) = req.status {
        validate_goal_transition(goal.status, status)?;
        apply_goal_status(&mut goal, status);
    }
    if let Some(budget) = req.budget {
        goal.budget = budget;
    }
    validate_active_operation(&goal)?;
    persist_goal(&project_root, &mut goal).await?;
    let goal = refresh_targets_and_emit(app, goal, None).await;
    Ok(axum::Json(goal))
}

pub async fn handle_v1_buddy_conductor_goal_pause(
    State(app): State<AppState>,
    Path(goal_id): Path<String>,
) -> Result<axum::Json<ConductorGoal>, ScratchError> {
    set_goal_status(app, goal_id, GoalStatus::Paused).await
}

pub async fn handle_v1_buddy_conductor_goal_resume(
    State(app): State<AppState>,
    Path(goal_id): Path<String>,
) -> Result<axum::Json<ConductorGoal>, ScratchError> {
    set_goal_status(app, goal_id, GoalStatus::Active).await
}

pub async fn handle_v1_buddy_conductor_goal_stop(
    State(app): State<AppState>,
    Path(goal_id): Path<String>,
) -> Result<axum::Json<ConductorGoal>, ScratchError> {
    set_goal_status(app, goal_id, GoalStatus::Abandoned).await
}

pub async fn handle_v1_buddy_conductor_goal_autonomy(
    State(app): State<AppState>,
    Path(goal_id): Path<String>,
    axum::Json(req): axum::Json<AutonomyRequest>,
) -> Result<axum::Json<ConductorGoal>, ScratchError> {
    let project_root = project_root(&app).await?;
    let mut goal = load_goal_from_root(&project_root, &goal_id).await?;
    goal.autonomy = req.autonomy;
    if matches!(
        req.autonomy,
        GoalAutonomy::Governed | GoalAutonomy::FullAuto
    ) {
        validate_active_operation(&goal)?;
    }
    persist_goal(&project_root, &mut goal).await?;
    let goal = refresh_targets_and_emit(app, goal, None).await;
    Ok(axum::Json(goal))
}

pub async fn handle_v1_buddy_conductor_goal_manual_wake(
    State(app): State<AppState>,
    Path(goal_id): Path<String>,
) -> Result<axum::Json<serde_json::Value>, ScratchError> {
    let goal = load_goal(&app, &goal_id).await?;
    validate_active_operation(&goal)?;
    let enqueued = crate::buddy::conductor::wake::enqueue_goal_wake(
        app.gcx.clone(),
        &goal.id,
        ConductorWakeReason::Manual,
    )
    .await;
    Ok(axum::Json(
        json!({ "enqueued": enqueued, "goal_id": goal.id }),
    ))
}
pub async fn handle_v1_buddy_conductor_answer(
    State(app): State<AppState>,
    axum::Json(req): axum::Json<ConductorAnswerRequest>,
) -> Result<axum::Json<ConductorAnswerResponse>, ScratchError> {
    let goal_id = req.goal_id.trim().to_string();
    let question_id = req.question_id.trim().to_string();
    let answer = req.answer.trim().to_string();
    if goal_id.is_empty() || question_id.is_empty() || answer.is_empty() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "goal_id, question_id, and answer are required".to_string(),
        ));
    }
    let project_root = project_root(&app).await?;
    let result = crate::buddy::conductor::ghost::conductor_ghost_answer(
        app,
        &project_root,
        &goal_id,
        &question_id,
        &answer,
    )
    .await?;
    Ok(axum::Json(ConductorAnswerResponse {
        goal_id: result.goal_id,
        question_id: result.question_id,
        answered: result.answered,
    }))
}

async fn set_goal_status(
    app: AppState,
    goal_id: String,
    status: GoalStatus,
) -> Result<axum::Json<ConductorGoal>, ScratchError> {
    let project_root = project_root(&app).await?;
    let mut goal = load_goal_from_root(&project_root, &goal_id).await?;
    validate_goal_transition(goal.status, status)?;
    apply_goal_status(&mut goal, status);
    if status == GoalStatus::Active {
        validate_active_operation(&goal)?;
    }
    persist_goal(&project_root, &mut goal).await?;
    let goal = refresh_targets_and_emit(app, goal, None).await;
    Ok(axum::Json(goal))
}
async fn load_goal(app: &AppState, goal_id: &str) -> Result<ConductorGoal, ScratchError> {
    let project_root = project_root(app).await?;
    let goal = load_goal_from_root(&project_root, goal_id).await?;
    Ok(hydrate_goal_spent(app.gcx.clone(), goal).await)
}

async fn load_goal_from_root(
    project_root: &std::path::Path,
    goal_id: &str,
) -> Result<ConductorGoal, ScratchError> {
    let ledger = load_goal_ledger(project_root, goal_id)
        .await
        .map_err(store_error)?
        .ok_or_else(|| {
            ScratchError::new(StatusCode::NOT_FOUND, format!("goal not found: {goal_id}"))
        })?;
    Ok(ConductorGoal::from_ledger(goal_id.to_string(), ledger))
}

async fn persist_goal(
    project_root: &std::path::Path,
    goal: &mut ConductorGoal,
) -> Result<(), ScratchError> {
    goal.updated_at = Some(Utc::now().to_rfc3339());
    let metadata = goal.clone();
    goal.ledger.apply_goal_metadata(&metadata);
    save_goal_ledger(project_root, &goal.id, &goal.ledger)
        .await
        .map_err(store_error)
}

fn validate_goal_transition(from: GoalStatus, to: GoalStatus) -> Result<(), ScratchError> {
    validate_goal_status_transition(from, to)
        .map_err(|error| ScratchError::new(StatusCode::CONFLICT, error.to_string()))
}

fn validate_active_operation(goal: &ConductorGoal) -> Result<(), ScratchError> {
    validate_goal_for_create(goal).map_err(|error| {
        ScratchError::new(
            StatusCode::BAD_REQUEST,
            format!("invalid conductor goal: {error}"),
        )
    })
}

fn apply_goal_status(goal: &mut ConductorGoal, status: GoalStatus) {
    goal.status = status;
    goal.completed_at = if status.is_terminal() {
        goal.completed_at
            .clone()
            .or_else(|| Some(Utc::now().to_rfc3339()))
    } else {
        None
    };
}

async fn refresh_targets_and_emit(
    app: AppState,
    goal: ConductorGoal,
    wake_reason: Option<ConductorWakeReason>,
) -> ConductorGoal {
    crate::buddy::conductor::wake::refresh_conductor_wake_targets(app.gcx.clone()).await;
    if let Some(reason) = wake_reason {
        let _ = crate::buddy::conductor::wake::enqueue_goal_wake_after_target_refresh(
            app.gcx.clone(),
            &goal.id,
            reason,
        )
        .await;
    }
    let goal = hydrate_goal_spent(app.gcx.clone(), goal).await;
    let _ = app
        .buddy
        .buddy_events_tx
        .send(BuddyEvent::ConductorGoalUpdated { goal: goal.clone() });
    goal
}

async fn project_root(app: &AppState) -> Result<std::path::PathBuf, ScratchError> {
    crate::files_correction::get_project_dirs(app.gcx.clone())
        .await
        .into_iter()
        .next()
        .ok_or_else(|| {
            ScratchError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "no project root".to_string(),
            )
        })
}

fn store_error(error: ConductorStoreError) -> ScratchError {
    let status = match error {
        ConductorStoreError::InvalidGoalId(_) => StatusCode::BAD_REQUEST,
        ConductorStoreError::MissingGoal(_) => StatusCode::NOT_FOUND,
        ConductorStoreError::Update(_) => StatusCode::BAD_REQUEST,
        ConductorStoreError::Io { .. } | ConductorStoreError::Json { .. } => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    };
    ScratchError::new(status, error.to_string())
}

fn normalized_goal_id(value: Option<String>) -> Option<String> {
    value.and_then(non_empty)
}

fn non_empty(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use async_trait::async_trait;
    use refact_buddy_core::conductor::{DoneWhen, GoalBudget, GoalLedger};
    use refact_buddy_core::conductor_store::{load_goal_ledger, save_goal_ledger};
    use refact_chat_api::ChatCommand;
    use refact_runtime_api::{
        ChatSessionFacade, ChatSessionSnapshot, ChatSessionUpdate, CreateSessionRequest,
        RuntimeTrajectorySnapshot, SessionState,
    };
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::time::timeout;

    async fn test_app() -> (AppState, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        (AppState::from_gcx(gcx).await, dir)
    }

    struct MockChatFacade {
        pushed: StdMutex<Vec<(String, ChatCommand)>>,
        updates: StdMutex<usize>,
    }

    impl MockChatFacade {
        fn new() -> Self {
            Self {
                pushed: StdMutex::new(Vec::new()),
                updates: StdMutex::new(0),
            }
        }

        fn pushed_commands(&self) -> Vec<(String, ChatCommand)> {
            self.pushed.lock().unwrap().clone()
        }

        fn update_count(&self) -> usize {
            *self.updates.lock().unwrap()
        }
    }

    #[async_trait]
    impl ChatSessionFacade for MockChatFacade {
        async fn session_snapshot(&self, _chat_id: &str) -> Result<ChatSessionSnapshot, String> {
            Ok(ChatSessionSnapshot {
                messages: vec![],
                thread: refact_chat_api::ThreadParams::default(),
                session_state: SessionState::Idle,
                pause_reasons: vec![],
            })
        }

        async fn update_session(
            &self,
            _chat_id: &str,
            _update: ChatSessionUpdate,
        ) -> Result<(), String> {
            *self.updates.lock().unwrap() += 1;
            Ok(())
        }

        async fn create_session(&self, _request: CreateSessionRequest) -> Result<(), String> {
            Ok(())
        }

        async fn push_command(&self, chat_id: &str, command: ChatCommand) -> Result<(), String> {
            self.pushed
                .lock()
                .unwrap()
                .push((chat_id.to_string(), command));
            Ok(())
        }

        async fn session_state(&self, _chat_id: &str) -> Result<Option<SessionState>, String> {
            Ok(Some(SessionState::Idle))
        }

        async fn maybe_save_session(&self, _chat_id: &str) -> Result<(), String> {
            Ok(())
        }

        async fn save_trajectory_snapshot(
            &self,
            _snapshot: RuntimeTrajectorySnapshot,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    fn create_req(id: &str) -> CreateConductorGoalRequest {
        CreateConductorGoalRequest {
            id: Some(id.to_string()),
            title: "Ship conductor routes".to_string(),
            plan_doc_slug: Some("master-plan".to_string()),
            plan_markdown: "# Ship conductor routes".to_string(),
            done_when: DoneWhen {
                summary: "Routes work".to_string(),
                checklist: vec!["tests pass".to_string()],
            },
            autonomy: GoalAutonomy::FullAuto,
            budget: GoalBudget {
                wall_clock_secs: Some(3600),
                no_progress_wakes: Some(3),
                total_tokens: Some(100_000),
                usd: None,
            },
        }
    }

    async fn create_goal(app: AppState, id: &str) -> ConductorGoal {
        handle_v1_buddy_conductor_goal_create(State(app), axum::Json(create_req(id)))
            .await
            .unwrap()
            .0
    }

    async fn save_legacy_invalid_goal(
        root: &std::path::Path,
        id: &str,
        status: GoalStatus,
        autonomy: GoalAutonomy,
    ) {
        save_goal_ledger(
            root,
            id,
            &GoalLedger {
                title: Some(id.to_string()),
                status: Some(status),
                autonomy: Some(autonomy),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    async fn write_task_meta(root: &std::path::Path, task_id: &str, goal_id: &str) {
        let task_dir = root.join(".refact").join("tasks").join(task_id);
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        let now = Utc::now().to_rfc3339();
        let meta = crate::tasks::types::TaskMeta {
            schema_version: 1,
            id: task_id.to_string(),
            name: task_id.to_string(),
            status: crate::tasks::types::TaskStatus::Active,
            created_at: now.clone(),
            updated_at: now,
            cards_total: 0,
            cards_done: 0,
            cards_failed: 0,
            agents_active: 0,
            base_branch: None,
            base_commit: None,
            default_agent_model: None,
            is_name_generated: false,
            last_agents_summary_at: None,
            planner_session_state: None,
            conductor: Some(crate::tasks::types::TaskConductorLink {
                goal_id: goal_id.to_string(),
                role: crate::tasks::types::TaskConductorRole::Planner,
            }),
        };
        tokio::fs::write(
            task_dir.join("meta.yaml"),
            serde_yaml::to_string(&meta).unwrap(),
        )
        .await
        .unwrap();
    }

    async fn write_trajectory(
        root: &std::path::Path,
        task_id: &str,
        chat_id: &str,
        model: &str,
        messages: Vec<Value>,
    ) {
        let dir = root
            .join(".refact")
            .join("tasks")
            .join(task_id)
            .join("trajectories")
            .join("planner");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let task_meta = crate::chat::types::TaskMeta {
            task_id: task_id.to_string(),
            role: "planner".to_string(),
            agent_id: None,
            card_id: None,
            planner_chat_id: Some(chat_id.to_string()),
        };
        let trajectory = json!({
            "id": chat_id,
            "model": model,
            "messages": messages,
            "task_meta": task_meta
        });
        tokio::fs::write(
            dir.join(format!("{chat_id}.json")),
            serde_json::to_string(&trajectory).unwrap(),
        )
        .await
        .unwrap();
    }

    fn usage_message(message_id: &str, prompt: usize, completion: usize, total: usize) -> Value {
        json!({
            "role": "assistant",
            "message_id": message_id,
            "content": "ok",
            "usage": {
                "prompt_tokens": prompt,
                "completion_tokens": completion,
                "total_tokens": total,
                "cache_read_input_tokens": 2
            }
        })
    }

    #[tokio::test]
    async fn buddy_conductor_routes_crud_goal() {
        let (app, _dir) = test_app().await;

        let created = create_goal(app.clone(), "goal-crud").await;
        assert_eq!(created.id, "goal-crud");
        assert_eq!(created.title, "Ship conductor routes");
        assert_eq!(created.status, GoalStatus::Active);
        assert_eq!(
            serde_json::to_value(&created).unwrap()["status"],
            json!("active")
        );

        let listed = handle_v1_buddy_conductor_goals_list(State(app.clone()))
            .await
            .unwrap()
            .0;
        assert_eq!(listed.goals.len(), 1);
        assert_eq!(listed.goals[0].id, "goal-crud");

        let fetched =
            handle_v1_buddy_conductor_goal_get(State(app.clone()), Path("goal-crud".to_string()))
                .await
                .unwrap()
                .0;
        assert_eq!(fetched.done_when.summary, "Routes work");

        let patched = handle_v1_buddy_conductor_goal_patch(
            State(app),
            Path("goal-crud".to_string()),
            axum::Json(PatchConductorGoalRequest {
                title: Some("Updated conductor routes".to_string()),
                autonomy: Some(GoalAutonomy::Governed),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(patched.title, "Updated conductor routes");
        assert_eq!(patched.autonomy, GoalAutonomy::Governed);
    }

    #[tokio::test]
    async fn buddy_conductor_routes_reject_missing_required_budgets() {
        let (app, _dir) = test_app().await;
        let mut req = create_req("goal-bad-budget");
        req.budget.wall_clock_secs = None;

        let error = handle_v1_buddy_conductor_goal_create(State(app), axum::Json(req))
            .await
            .unwrap_err();

        assert_eq!(error.status_code, StatusCode::BAD_REQUEST);
        assert!(error.message.contains("wall_clock_secs"));
    }

    #[tokio::test]
    async fn buddy_conductor_routes_create_rejects_empty_done_when() {
        let (app, _dir) = test_app().await;
        let mut req = create_req("goal-empty-done-when");
        req.done_when = DoneWhen {
            summary: "   ".to_string(),
            checklist: vec!["".to_string(), "  ".to_string()],
        };

        let error = handle_v1_buddy_conductor_goal_create(State(app), axum::Json(req))
            .await
            .unwrap_err();

        assert_eq!(error.status_code, StatusCode::BAD_REQUEST);
        assert!(error.message.contains("done_when"));
    }

    #[tokio::test]
    async fn buddy_conductor_routes_create_rejects_missing_done_when() {
        let (app, _dir) = test_app().await;
        let req: CreateConductorGoalRequest = serde_json::from_value(json!({
            "id": "goal-missing-done-when",
            "title": "Missing done_when",
            "budget": {
                "wall_clock_secs": 60,
                "no_progress_wakes": 2
            }
        }))
        .unwrap();

        let error = handle_v1_buddy_conductor_goal_create(State(app), axum::Json(req))
            .await
            .unwrap_err();

        assert_eq!(error.status_code, StatusCode::BAD_REQUEST);
        assert!(error.message.contains("done_when"));
    }

    #[tokio::test]
    async fn buddy_conductor_routes_patch_rejects_clearing_done_when() {
        let (app, _dir) = test_app().await;
        create_goal(app.clone(), "goal-clear-done-when").await;

        let error = handle_v1_buddy_conductor_goal_patch(
            State(app),
            Path("goal-clear-done-when".to_string()),
            axum::Json(PatchConductorGoalRequest {
                done_when: Some(DoneWhen {
                    summary: "".to_string(),
                    checklist: Vec::new(),
                }),
                ..Default::default()
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(error.status_code, StatusCode::BAD_REQUEST);
        assert!(error.message.contains("done_when"));
    }

    #[tokio::test]
    async fn buddy_conductor_routes_patch_accepts_canonical_and_legacy_active() {
        let (app, dir) = test_app().await;
        save_legacy_invalid_goal(
            dir.path(),
            "goal-patch-active",
            GoalStatus::Proposed,
            GoalAutonomy::ReadOnly,
        )
        .await;

        let active = handle_v1_buddy_conductor_goal_patch(
            State(app.clone()),
            Path("goal-patch-active".to_string()),
            axum::Json(PatchConductorGoalRequest {
                status: Some(GoalStatus::Active),
                done_when: Some(DoneWhen {
                    summary: "Ready".to_string(),
                    checklist: Vec::new(),
                }),
                budget: Some(GoalBudget {
                    wall_clock_secs: Some(60),
                    no_progress_wakes: Some(1),
                    ..GoalBudget::default()
                }),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(active.status, GoalStatus::Active);
        assert_eq!(
            serde_json::to_value(&active).unwrap()["status"],
            json!("active")
        );

        save_legacy_invalid_goal(
            dir.path(),
            "goal-patch-proposed",
            GoalStatus::Proposed,
            GoalAutonomy::ReadOnly,
        )
        .await;
        let proposed = handle_v1_buddy_conductor_goal_patch(
            State(app.clone()),
            Path("goal-patch-proposed".to_string()),
            axum::Json(PatchConductorGoalRequest {
                status: Some(GoalStatus::Proposed),
                done_when: Some(DoneWhen {
                    summary: "Ready later".to_string(),
                    checklist: Vec::new(),
                }),
                budget: Some(GoalBudget {
                    wall_clock_secs: Some(60),
                    no_progress_wakes: Some(1),
                    ..GoalBudget::default()
                }),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(proposed.status, GoalStatus::Proposed);
        assert_eq!(
            serde_json::to_value(&proposed).unwrap()["status"],
            json!("proposed")
        );

        let status: GoalStatus = serde_json::from_value(json!("running")).unwrap();
        let active_again = handle_v1_buddy_conductor_goal_patch(
            State(app),
            Path("goal-patch-active".to_string()),
            axum::Json(PatchConductorGoalRequest {
                status: Some(status),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(active_again.status, GoalStatus::Active);
        assert_eq!(
            serde_json::to_value(&active_again).unwrap()["status"],
            json!("active")
        );
    }

    #[tokio::test]
    async fn buddy_conductor_routes_terminal_resume_and_patch_active_conflict() {
        let (app, _dir) = test_app().await;
        create_goal(app.clone(), "goal-terminal-active").await;
        let _ = handle_v1_buddy_conductor_goal_stop(
            State(app.clone()),
            Path("goal-terminal-active".to_string()),
        )
        .await
        .unwrap();

        let resume_error = handle_v1_buddy_conductor_goal_resume(
            State(app.clone()),
            Path("goal-terminal-active".to_string()),
        )
        .await
        .unwrap_err();
        assert_eq!(resume_error.status_code, StatusCode::CONFLICT);

        let patch_error = handle_v1_buddy_conductor_goal_patch(
            State(app),
            Path("goal-terminal-active".to_string()),
            axum::Json(PatchConductorGoalRequest {
                status: Some(GoalStatus::Active),
                ..Default::default()
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(patch_error.status_code, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn buddy_conductor_routes_manual_wake_rejects_invalid_goal_without_enqueueing() {
        let (app, dir) = test_app().await;
        save_legacy_invalid_goal(
            dir.path(),
            "goal-invalid-wake",
            GoalStatus::Active,
            GoalAutonomy::FullAuto,
        )
        .await;

        let error = handle_v1_buddy_conductor_goal_manual_wake(
            State(app.clone()),
            Path("goal-invalid-wake".to_string()),
        )
        .await
        .unwrap_err();

        assert_eq!(error.status_code, StatusCode::BAD_REQUEST);
        assert!(error.message.contains("done_when"));
        assert!(app
            .buddy
            .conductor_wake_bus
            .lock()
            .await
            .mailbox("goal-invalid-wake")
            .is_none());
    }

    #[tokio::test]
    async fn buddy_conductor_routes_full_auto_rejects_invalid_goal_without_enqueueing() {
        let (app, dir) = test_app().await;
        save_legacy_invalid_goal(
            dir.path(),
            "goal-invalid-full-auto",
            GoalStatus::Active,
            GoalAutonomy::ReadOnly,
        )
        .await;

        let error = handle_v1_buddy_conductor_goal_autonomy(
            State(app.clone()),
            Path("goal-invalid-full-auto".to_string()),
            axum::Json(AutonomyRequest {
                autonomy: GoalAutonomy::FullAuto,
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(error.status_code, StatusCode::BAD_REQUEST);
        assert!(error.message.contains("done_when"));
        assert!(app
            .buddy
            .conductor_wake_bus
            .lock()
            .await
            .mailbox("goal-invalid-full-auto")
            .is_none());
    }

    #[tokio::test]
    async fn buddy_conductor_routes_pause_resume_persist_status() {
        let (app, dir) = test_app().await;
        create_goal(app.clone(), "goal-pause").await;

        let paused = handle_v1_buddy_conductor_goal_pause(
            State(app.clone()),
            Path("goal-pause".to_string()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(paused.status, GoalStatus::Paused);
        let ledger = load_goal_ledger(dir.path(), "goal-pause")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.status, Some(GoalStatus::Paused));

        let resumed =
            handle_v1_buddy_conductor_goal_resume(State(app), Path("goal-pause".to_string()))
                .await
                .unwrap()
                .0;
        assert_eq!(resumed.status, GoalStatus::Active);
        let ledger = load_goal_ledger(dir.path(), "goal-pause")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.status, Some(GoalStatus::Active));
    }

    #[tokio::test]
    async fn buddy_conductor_routes_manual_wake_enqueues_via_wake_bus() {
        let (app, _dir) = test_app().await;
        create_goal(app.clone(), "goal-wake").await;

        let response = handle_v1_buddy_conductor_goal_manual_wake(
            State(app.clone()),
            Path("goal-wake".to_string()),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(response["enqueued"], true);
        let bus = app.buddy.conductor_wake_bus.lock().await;
        let mailbox = bus.mailbox("goal-wake").unwrap();
        assert!(mailbox.reasons.contains(&ConductorWakeReason::Manual));
    }

    #[tokio::test]
    async fn buddy_conductor_routes_goal_created_wake_targets_only_created_goal() {
        let (app, _dir) = test_app().await;
        create_goal(app.clone(), "goal-existing").await;
        {
            let mut bus = app.buddy.conductor_wake_bus.lock().await;
            bus.reconcile_targets(&crate::buddy::conductor::wake::ConductorWakeTargets::default());
        }

        create_goal(app.clone(), "goal-new").await;

        let bus = app.buddy.conductor_wake_bus.lock().await;
        assert!(bus.mailbox("goal-existing").is_none());
        let mailbox = bus.mailbox("goal-new").unwrap();
        assert_eq!(mailbox.reasons, vec![ConductorWakeReason::GoalCreated]);
    }

    #[tokio::test]
    async fn buddy_conductor_routes_startup_reattach_excludes_paused_targets() {
        let (app, dir) = test_app().await;
        let mut goal = create_req("goal-reattach");
        goal.title = "Reattach me".to_string();
        let mut created = ConductorGoal {
            id: "goal-reattach".to_string(),
            title: goal.title,
            status: GoalStatus::Paused,
            autonomy: GoalAutonomy::FullAuto,
            budget: goal.budget,
            ..ConductorGoal::default()
        };
        created.ledger.task_ids = vec!["task-1".to_string()];
        let metadata = created.clone();
        created.ledger.apply_goal_metadata(&metadata);
        save_goal_ledger(dir.path(), "goal-reattach", &created.ledger)
            .await
            .unwrap();

        let targets =
            crate::buddy::conductor::wake::refresh_conductor_wake_targets(app.gcx.clone()).await;

        assert!(targets.goal_ids().is_empty());
        assert!(targets.goals_for_task("task-1").is_empty());
    }

    #[tokio::test]
    async fn buddy_conductor_routes_emit_goal_updated_event() {
        let (app, _dir) = test_app().await;
        let mut rx = app.buddy.buddy_events_tx.subscribe();

        create_goal(app, "goal-event").await;

        let event = timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match event {
            BuddyEvent::ConductorGoalUpdated { goal } => {
                assert_eq!(goal.id, "goal-event");
            }
            other => panic!("expected ConductorGoalUpdated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn conductor_goal_routes_return_hydrated_spent() {
        let (app, dir) = test_app().await;
        write_task_meta(dir.path(), "task-spent", "goal-spent").await;
        write_trajectory(
            dir.path(),
            "task-spent",
            "planner-spent",
            "test/unpriced",
            vec![usage_message("usage-1", 17, 5, 22)],
        )
        .await;
        save_goal_ledger(
            dir.path(),
            "goal-spent",
            &GoalLedger {
                task_ids: vec!["task-spent".to_string()],
                no_progress_wakes: 6,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let listed = handle_v1_buddy_conductor_goals_list(State(app.clone()))
            .await
            .unwrap()
            .0;
        let fetched =
            handle_v1_buddy_conductor_goal_get(State(app), Path("goal-spent".to_string()))
                .await
                .unwrap()
                .0;

        assert_eq!(listed.goals[0].spent.prompt_tokens, 17);
        assert_eq!(listed.goals[0].spent.completion_tokens, 5);
        assert_eq!(listed.goals[0].spent.total_tokens, 22);
        assert_eq!(listed.goals[0].spent.cache_read_tokens, 2);
        assert_eq!(listed.goals[0].spent.usd, None);
        assert_eq!(listed.goals[0].spent.no_progress_wakes, 6);
        assert_eq!(fetched.spent.total_tokens, 22);
        assert_eq!(fetched.spent.no_progress_wakes, 6);
    }

    #[tokio::test]
    async fn buddy_conductor_answer_persists_and_enqueues_ghost_answer_only() {
        let (app, dir) = test_app().await;
        create_goal(app.clone(), "goal-answer").await;
        let mock = Arc::new(MockChatFacade::new());
        let mut app = app;
        app.chat.facade = mock.clone();
        let ask = crate::buddy::conductor::ghost::conductor_ghost_ask(
            app.gcx.clone(),
            dir.path(),
            "goal-answer",
            "Can I proceed?",
            true,
            Some("target-chat".to_string()),
        )
        .await
        .unwrap();

        let response = handle_v1_buddy_conductor_answer(
            State(app.clone()),
            axum::Json(ConductorAnswerRequest {
                goal_id: "goal-answer".to_string(),
                question_id: ask.question.id.clone(),
                answer: "Yes, tiny gremlin".to_string(),
            }),
        )
        .await
        .unwrap()
        .0;

        assert!(response.answered);
        assert_eq!(response.question_id, ask.question.id);
        let ledger = load_goal_ledger(dir.path(), "goal-answer")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            ledger.pending_questions[0].answer.as_deref(),
            Some("Yes, tiny gremlin")
        );
        assert!(ledger.pending_questions[0].answered_at.is_some());
        assert!(mock.pushed_commands().is_empty());
        assert_eq!(mock.update_count(), 0);
        let bus = app.buddy.conductor_wake_bus.lock().await;
        let mailbox = bus.mailbox("goal-answer").unwrap();
        assert!(mailbox.reasons.contains(&ConductorWakeReason::GhostAnswer));
    }

    #[tokio::test]
    async fn buddy_conductor_answer_duplicate_returns_conflict_and_preserves_original() {
        let (app, dir) = test_app().await;
        create_goal(app.clone(), "goal-answer-duplicate").await;
        let ask = crate::buddy::conductor::ghost::conductor_ghost_ask(
            app.gcx.clone(),
            dir.path(),
            "goal-answer-duplicate",
            "Can I proceed?",
            true,
            Some("target-chat".to_string()),
        )
        .await
        .unwrap();

        let _ = handle_v1_buddy_conductor_answer(
            State(app.clone()),
            axum::Json(ConductorAnswerRequest {
                goal_id: "goal-answer-duplicate".to_string(),
                question_id: ask.question.id.clone(),
                answer: "Original".to_string(),
            }),
        )
        .await
        .unwrap();
        let error = handle_v1_buddy_conductor_answer(
            State(app.clone()),
            axum::Json(ConductorAnswerRequest {
                goal_id: "goal-answer-duplicate".to_string(),
                question_id: ask.question.id.clone(),
                answer: "Replacement".to_string(),
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(error.status_code, StatusCode::CONFLICT);
        let ledger = load_goal_ledger(dir.path(), "goal-answer-duplicate")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            ledger.pending_questions[0].answer.as_deref(),
            Some("Original")
        );
    }

    #[tokio::test]
    async fn buddy_conductor_answer_paused_goal_returns_conflict_without_mutation_or_wake() {
        let (app, dir) = test_app().await;
        create_goal(app.clone(), "goal-answer-paused").await;
        let _ = handle_v1_buddy_conductor_goal_pause(
            State(app.clone()),
            Path("goal-answer-paused".to_string()),
        )
        .await
        .unwrap();
        let ask = crate::buddy::conductor::ghost::conductor_ghost_ask(
            app.gcx.clone(),
            dir.path(),
            "goal-answer-paused",
            "Can I proceed?",
            true,
            Some("target-chat".to_string()),
        )
        .await
        .unwrap();
        {
            let mut bus = app.buddy.conductor_wake_bus.lock().await;
            bus.reconcile_targets(&crate::buddy::conductor::wake::ConductorWakeTargets::default());
        }

        let error = handle_v1_buddy_conductor_answer(
            State(app.clone()),
            axum::Json(ConductorAnswerRequest {
                goal_id: "goal-answer-paused".to_string(),
                question_id: ask.question.id.clone(),
                answer: "No mutate".to_string(),
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(error.status_code, StatusCode::CONFLICT);
        let ledger = load_goal_ledger(dir.path(), "goal-answer-paused")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.pending_questions[0].answer, None);
        assert!(app
            .buddy
            .conductor_wake_bus
            .lock()
            .await
            .mailbox("goal-answer-paused")
            .is_none());
    }

    #[tokio::test]
    async fn buddy_conductor_answer_terminal_goal_returns_conflict_without_mutation_or_wake() {
        let (app, dir) = test_app().await;
        create_goal(app.clone(), "goal-answer-terminal").await;
        let _ = handle_v1_buddy_conductor_goal_stop(
            State(app.clone()),
            Path("goal-answer-terminal".to_string()),
        )
        .await
        .unwrap();
        let ask = crate::buddy::conductor::ghost::conductor_ghost_ask(
            app.gcx.clone(),
            dir.path(),
            "goal-answer-terminal",
            "Can I proceed?",
            true,
            Some("target-chat".to_string()),
        )
        .await
        .unwrap();
        {
            let mut bus = app.buddy.conductor_wake_bus.lock().await;
            bus.reconcile_targets(&crate::buddy::conductor::wake::ConductorWakeTargets::default());
        }

        let error = handle_v1_buddy_conductor_answer(
            State(app.clone()),
            axum::Json(ConductorAnswerRequest {
                goal_id: "goal-answer-terminal".to_string(),
                question_id: ask.question.id.clone(),
                answer: "No mutate".to_string(),
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(error.status_code, StatusCode::CONFLICT);
        let ledger = load_goal_ledger(dir.path(), "goal-answer-terminal")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.pending_questions[0].answer, None);
        assert!(app
            .buddy
            .conductor_wake_bus
            .lock()
            .await
            .mailbox("goal-answer-terminal")
            .is_none());
    }

    #[tokio::test]
    async fn buddy_conductor_routes_control_stop_and_autonomy() {
        let (app, dir) = test_app().await;
        create_goal(app.clone(), "goal-control").await;

        let changed = handle_v1_buddy_conductor_goal_autonomy(
            State(app.clone()),
            Path("goal-control".to_string()),
            axum::Json(AutonomyRequest {
                autonomy: GoalAutonomy::ReadOnly,
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(changed.autonomy, GoalAutonomy::ReadOnly);

        let stopped =
            handle_v1_buddy_conductor_goal_stop(State(app), Path("goal-control".to_string()))
                .await
                .unwrap()
                .0;
        assert_eq!(stopped.status, GoalStatus::Abandoned);
        assert!(stopped.completed_at.is_some());
        let ledger = load_goal_ledger(dir.path(), "goal-control")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ledger.autonomy, Some(GoalAutonomy::ReadOnly));
        assert_eq!(ledger.status, Some(GoalStatus::Abandoned));
        assert!(ledger.completed_at.is_some());
    }
}
