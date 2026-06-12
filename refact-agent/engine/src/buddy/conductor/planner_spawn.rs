use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use refact_buddy_core::conductor::{ConductorGoal, GoalAutonomy, GoalLedger};
use refact_buddy_core::conductor_store::{load_goal_ledger, save_goal_ledger};
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::call_validation::{ChatContent, ChatMessage};
use crate::chat::trajectories::try_save_trajectory;
use crate::chat::types::{ChatSession, TaskMeta as ChatTaskMeta, ThreadParams};
use crate::global_context::GlobalContext;
use crate::tasks::events::{emit_task_event, TaskEvent};
use crate::tasks::storage;
use crate::tasks::types::{TaskConductorLink, TaskConductorRole, TaskMeta};

#[derive(Debug, Clone, PartialEq)]
pub struct ConductorPlannerSpawn {
    pub goal_id: String,
    pub task_id: String,
    pub planner_chat_id: String,
    pub ledger: GoalLedger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlannerApprovalPolicy {
    auto_approve_editing_tools: bool,
    auto_approve_dangerous_commands: bool,
}

pub async fn conductor_spawn_planner(
    gcx: Arc<GlobalContext>,
    goal_id: &str,
    instructions: &str,
) -> Result<ConductorPlannerSpawn, String> {
    let goal = ConductorGoal {
        id: goal_id.to_string(),
        ..Default::default()
    };
    conductor_spawn_planner_for_goal(gcx, &goal, instructions).await
}

pub async fn conductor_spawn_planner_for_goal(
    gcx: Arc<GlobalContext>,
    goal: &ConductorGoal,
    instructions: &str,
) -> Result<ConductorPlannerSpawn, String> {
    let goal_id = goal.id.as_str();
    let policy = planner_approval_policy(goal.autonomy)?;
    let project_root = conductor_project_root(gcx.clone()).await?;
    let mut ledger = load_goal_ledger(&project_root, goal_id)
        .await
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| goal.ledger.clone());
    ensure_goal_can_spawn_planner(gcx.clone(), goal_id, &ledger).await?;

    let task = storage::create_task(gcx.clone(), &planner_task_name(goal_id)).await?;
    let task_id = task.id.clone();
    let planner_chat_id = storage::next_planner_chat_id(gcx.clone(), &task_id).await?;
    link_task_to_goal(gcx.clone(), goal_id, &task_id).await?;
    storage::save_planner_instructions(gcx.clone(), &task_id, instructions).await?;
    create_planner_session(
        gcx.clone(),
        goal_id,
        &task_id,
        &planner_chat_id,
        instructions,
        policy,
    )
    .await?;

    ledger.planner_task_id = Some(task_id.clone());
    push_unique(&mut ledger.task_ids, task_id.clone());
    push_unique(&mut ledger.chat_ids, planner_chat_id.clone());
    save_goal_ledger(&project_root, goal_id, &ledger)
        .await
        .map_err(|error| error.to_string())?;
    super::wake::refresh_conductor_wake_targets(gcx.clone()).await;

    Ok(ConductorPlannerSpawn {
        goal_id: goal_id.to_string(),
        task_id,
        planner_chat_id,
        ledger,
    })
}

fn planner_approval_policy(autonomy: GoalAutonomy) -> Result<PlannerApprovalPolicy, String> {
    match autonomy {
        GoalAutonomy::FullAuto => Ok(PlannerApprovalPolicy {
            auto_approve_editing_tools: true,
            auto_approve_dangerous_commands: true,
        }),
        GoalAutonomy::Governed => Ok(PlannerApprovalPolicy {
            auto_approve_editing_tools: false,
            auto_approve_dangerous_commands: false,
        }),
        GoalAutonomy::ReadOnly => {
            Err("read-only conductor goals cannot spawn planner tasks".to_string())
        }
    }
}

async fn conductor_project_root(gcx: Arc<GlobalContext>) -> Result<PathBuf, String> {
    crate::files_correction::get_project_dirs(gcx)
        .await
        .into_iter()
        .next()
        .ok_or_else(|| "No workspace folder found".to_string())
}

async fn ensure_goal_can_spawn_planner(
    gcx: Arc<GlobalContext>,
    goal_id: &str,
    ledger: &GoalLedger,
) -> Result<(), String> {
    if let Some(task_id) = ledger
        .planner_task_id
        .as_deref()
        .map(str::trim)
        .filter(|task_id| !task_id.is_empty())
    {
        return Err(format!(
            "conductor goal {goal_id} already has planner task {task_id}"
        ));
    }
    ensure_ledger_task_ownership(gcx.clone(), goal_id, ledger).await?;
    for task in storage::list_tasks(gcx).await? {
        if task
            .conductor
            .as_ref()
            .is_some_and(|link| link.goal_id == goal_id && link.role == TaskConductorRole::Planner)
        {
            return Err(format!(
                "conductor goal {goal_id} already owns planner task {}",
                task.id
            ));
        }
    }
    Ok(())
}

async fn ensure_ledger_task_ownership(
    gcx: Arc<GlobalContext>,
    goal_id: &str,
    ledger: &GoalLedger,
) -> Result<(), String> {
    let mut task_ids = BTreeSet::new();
    if let Some(task_id) = ledger
        .planner_task_id
        .as_deref()
        .map(str::trim)
        .filter(|task_id| !task_id.is_empty())
    {
        task_ids.insert(task_id.to_string());
    }
    for task_id in &ledger.task_ids {
        let task_id = task_id.trim();
        if !task_id.is_empty() {
            task_ids.insert(task_id.to_string());
        }
    }

    for task_id in task_ids {
        let Ok(task) = storage::load_task_meta(gcx.clone(), &task_id).await else {
            continue;
        };
        ensure_task_unowned_or_owned_by_goal(goal_id, &task)?;
    }
    Ok(())
}

fn ensure_task_unowned_or_owned_by_goal(goal_id: &str, task: &TaskMeta) -> Result<(), String> {
    if let Some(link) = task.conductor.as_ref() {
        if link.goal_id != goal_id {
            return Err(format!(
                "task {} is already owned by conductor goal {}",
                task.id, link.goal_id
            ));
        }
    }
    Ok(())
}

async fn link_task_to_goal(
    gcx: Arc<GlobalContext>,
    goal_id: &str,
    task_id: &str,
) -> Result<TaskMeta, String> {
    let mut meta = storage::load_task_meta(gcx.clone(), task_id).await?;
    ensure_task_unowned_or_owned_by_goal(goal_id, &meta)?;
    meta.conductor = Some(TaskConductorLink {
        goal_id: goal_id.to_string(),
        role: TaskConductorRole::Planner,
    });
    meta.updated_at = Utc::now().to_rfc3339();
    storage::save_task_meta(gcx.clone(), task_id, &meta).await?;
    emit_task_event(
        gcx,
        TaskEvent::TaskUpdated {
            task_id: task_id.to_string(),
            meta: meta.clone(),
        },
    )
    .await;
    Ok(meta)
}

async fn create_planner_session(
    gcx: Arc<GlobalContext>,
    goal_id: &str,
    task_id: &str,
    planner_chat_id: &str,
    instructions: &str,
    policy: PlannerApprovalPolicy,
) -> Result<(), String> {
    let mut session = ChatSession::new(planner_chat_id.to_string());
    session.thread = planner_thread(goal_id, task_id, planner_chat_id, policy);
    session.trajectory_events_tx = gcx.trajectory_events_tx.clone();
    session.add_message(ChatMessage {
        message_id: Uuid::new_v4().to_string(),
        role: "user".to_string(),
        content: ChatContent::SimpleText(instructions.to_string()),
        ..Default::default()
    });
    let session_arc = Arc::new(AMutex::new(session));
    {
        let mut sessions = gcx.chat_sessions.write().await;
        if sessions.contains_key(planner_chat_id) {
            return Err(format!("planner chat already exists: {planner_chat_id}"));
        }
        sessions.insert(planner_chat_id.to_string(), session_arc.clone());
    }
    try_save_trajectory(AppState::from_gcx(gcx).await, session_arc)
        .await
        .map_err(|error| format!("Failed to save planner trajectory: {error}"))?;
    Ok(())
}

fn planner_thread(
    goal_id: &str,
    task_id: &str,
    planner_chat_id: &str,
    policy: PlannerApprovalPolicy,
) -> ThreadParams {
    ThreadParams {
        id: planner_chat_id.to_string(),
        title: planner_title(goal_id),
        mode: "task_planner".to_string(),
        tool_use: "agent".to_string(),
        auto_approve_editing_tools: policy.auto_approve_editing_tools,
        auto_approve_dangerous_commands: policy.auto_approve_dangerous_commands,
        auto_enrichment_enabled: Some(false),
        task_meta: Some(ChatTaskMeta {
            task_id: task_id.to_string(),
            role: "planner".to_string(),
            agent_id: None,
            card_id: None,
            planner_chat_id: Some(planner_chat_id.to_string()),
        }),
        ..Default::default()
    }
}

fn planner_task_name(goal_id: &str) -> String {
    format!("Conductor planner: {goal_id}")
}

fn planner_title(goal_id: &str) -> String {
    format!("Conductor planner: {goal_id}")
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::trajectories::load_trajectory_for_chat;
    use refact_buddy_core::conductor_store::load_goal_ledger;

    async fn test_gcx() -> (Arc<GlobalContext>, tempfile::TempDir) {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        (gcx, temp)
    }

    #[tokio::test]
    async fn conductor_spawn_planner_links_task_chat_and_goal() {
        let (gcx, temp) = test_gcx().await;
        let spawn = conductor_spawn_planner(gcx.clone(), "goal-1", "Plan this conductor mission.")
            .await
            .unwrap();

        let meta = storage::load_task_meta(gcx.clone(), &spawn.task_id)
            .await
            .unwrap();
        assert_eq!(
            meta.conductor,
            Some(TaskConductorLink {
                goal_id: "goal-1".to_string(),
                role: TaskConductorRole::Planner,
            })
        );

        let trajectories =
            storage::list_task_trajectories(gcx.clone(), &spawn.task_id, "planner", None)
                .await
                .unwrap();
        assert_eq!(trajectories.len(), 1);
        assert_eq!(trajectories[0].id, spawn.planner_chat_id);

        let loaded = load_trajectory_for_chat(gcx.clone(), &spawn.planner_chat_id)
            .await
            .unwrap();
        assert_eq!(loaded.thread.mode, "task_planner");
        assert_eq!(
            loaded.thread.task_meta.as_ref().unwrap().task_id,
            spawn.task_id
        );
        assert_eq!(loaded.thread.task_meta.as_ref().unwrap().role, "planner");
        assert_eq!(
            loaded
                .thread
                .task_meta
                .as_ref()
                .unwrap()
                .planner_chat_id
                .as_deref(),
            Some(spawn.planner_chat_id.as_str())
        );
        assert!(loaded.thread.buddy_meta.is_none());
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.messages[0].role, "user");
        assert_eq!(
            loaded.messages[0].content.content_text_only(),
            "Plan this conductor mission."
        );
        assert_eq!(
            storage::load_planner_instructions(gcx, &spawn.task_id)
                .await
                .unwrap(),
            "Plan this conductor mission."
        );
        assert!(temp
            .path()
            .join(".refact/tasks")
            .join(&spawn.task_id)
            .exists());
    }

    #[tokio::test]
    async fn conductor_spawn_planner_sets_full_auto_flags() {
        let (gcx, _) = test_gcx().await;
        let spawn = conductor_spawn_planner(gcx.clone(), "goal-full-auto", "Plan with full auto.")
            .await
            .unwrap();

        let loaded = load_trajectory_for_chat(gcx.clone(), &spawn.planner_chat_id)
            .await
            .unwrap();
        assert!(loaded.thread.auto_approve_editing_tools);
        assert!(loaded.thread.auto_approve_dangerous_commands);
        assert!(!loaded.thread.autonomous_no_confirm);

        let live = gcx
            .chat_sessions
            .read()
            .await
            .get(&spawn.planner_chat_id)
            .cloned()
            .unwrap();
        let live = live.lock().await;
        assert!(live.thread.auto_approve_editing_tools);
        assert!(live.thread.auto_approve_dangerous_commands);
        assert!(!live.thread.autonomous_no_confirm);
    }

    #[tokio::test]
    async fn conductor_spawn_planner_maps_governed_to_no_auto_approval() {
        let (gcx, _) = test_gcx().await;
        let spawn = conductor_spawn_planner_for_goal(
            gcx.clone(),
            &ConductorGoal {
                id: "goal-governed".to_string(),
                autonomy: GoalAutonomy::Governed,
                ..Default::default()
            },
            "Plan with governed autonomy.",
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, &spawn.planner_chat_id)
            .await
            .unwrap();
        assert!(!loaded.thread.auto_approve_editing_tools);
        assert!(!loaded.thread.auto_approve_dangerous_commands);
        assert!(!loaded.thread.autonomous_no_confirm);
    }

    #[tokio::test]
    async fn conductor_spawn_planner_rejects_read_only() {
        let (gcx, temp) = test_gcx().await;
        let error = conductor_spawn_planner_for_goal(
            gcx.clone(),
            &ConductorGoal {
                id: "goal-read-only".to_string(),
                autonomy: GoalAutonomy::ReadOnly,
                ..Default::default()
            },
            "Should not mutate.",
        )
        .await
        .unwrap_err();

        assert!(error.contains("read-only"));
        assert!(storage::list_tasks(gcx).await.unwrap().is_empty());
        assert!(load_goal_ledger(temp.path(), "goal-read-only")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn conductor_spawn_planner_rejects_duplicate_task_ownership() {
        let (gcx, temp) = test_gcx().await;
        let existing = storage::create_task(gcx.clone(), "Existing planner")
            .await
            .unwrap();
        let mut meta = storage::load_task_meta(gcx.clone(), &existing.id)
            .await
            .unwrap();
        meta.conductor = Some(TaskConductorLink {
            goal_id: "other-goal".to_string(),
            role: TaskConductorRole::Planner,
        });
        storage::save_task_meta(gcx.clone(), &existing.id, &meta)
            .await
            .unwrap();
        save_goal_ledger(
            temp.path(),
            "goal-1",
            &GoalLedger {
                task_ids: vec![existing.id.clone()],
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let error = conductor_spawn_planner(gcx.clone(), "goal-1", "Plan should be rejected.")
            .await
            .unwrap_err();

        assert!(error.contains("already owned by conductor goal other-goal"));
        let tasks = storage::list_tasks(gcx).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, existing.id);
    }

    #[tokio::test]
    async fn conductor_spawn_planner_updates_goal_ledger_task_list() {
        let (gcx, temp) = test_gcx().await;
        let spawn = conductor_spawn_planner(gcx, "goal-ledger", "Plan and update the ledger.")
            .await
            .unwrap();

        let ledger = load_goal_ledger(temp.path(), "goal-ledger")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            ledger.planner_task_id.as_deref(),
            Some(spawn.task_id.as_str())
        );
        assert_eq!(ledger.task_ids, vec![spawn.task_id.clone()]);
        assert_eq!(ledger.chat_ids, vec![spawn.planner_chat_id.clone()]);
        assert_eq!(spawn.ledger, ledger);
    }
}
