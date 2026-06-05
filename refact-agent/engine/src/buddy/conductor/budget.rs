use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use refact_buddy_core::conductor::{ConductorGoal, GoalBudgetSpent};
use serde::Deserialize;
use tokio::fs;

use crate::call_validation::{ChatMessage, ChatUsage};
use crate::chat::types::TaskMeta as ChatTaskMeta;
use crate::global_context::GlobalContext;
use crate::providers::pricing::{compute_cost, lookup_model_pricing};
use crate::providers::traits::ModelPricing;
use crate::tasks::storage;

#[derive(Debug, Deserialize)]
struct BudgetTrajectory {
    id: String,
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(default)]
    task_meta: Option<ChatTaskMeta>,
}

#[derive(Default)]
struct UsageAccumulator {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    cache_read_tokens: u64,
    usd: f64,
    saw_usage: bool,
    missing_usd: bool,
}

pub async fn aggregate_goal_spent(
    gcx: Arc<GlobalContext>,
    goal: &ConductorGoal,
) -> Result<GoalBudgetSpent, String> {
    let owned_task_ids = owned_goal_task_ids(gcx.clone(), goal).await?;
    let owned_task_roots = existing_owned_task_roots(gcx.clone(), &owned_task_ids).await;
    let mut trajectory_paths = Vec::new();

    for task_dir in owned_task_roots.values() {
        trajectory_paths
            .extend(collect_json_files(task_dir.join("trajectories").join("planner")).await?);
        trajectory_paths
            .extend(collect_json_files(task_dir.join("trajectories").join("agents")).await?);
    }

    for chat_id in &goal.ledger.chat_ids {
        if let Some(path) =
            crate::chat::trajectories::find_trajectory_path(gcx.clone(), chat_id).await
        {
            if path_is_under_owned_task_roots(&path, &owned_task_roots) {
                trajectory_paths.push(path);
            }
        }
    }

    let mut accumulator = UsageAccumulator::default();
    let mut pricing_cache: HashMap<String, Option<ModelPricing>> = HashMap::new();
    let mut seen_paths = HashSet::new();
    let mut seen_trajectory_ids = HashSet::new();
    let mut seen_message_keys = HashSet::new();

    for path in trajectory_paths {
        if !seen_paths.insert(path.clone()) {
            continue;
        }
        let trajectory = read_budget_trajectory(&path).await?;
        if !trajectory_is_owned(&trajectory, &path, &owned_task_ids, &owned_task_roots) {
            continue;
        }
        let trajectory_key = if trajectory.id.is_empty() {
            path.to_string_lossy().to_string()
        } else {
            trajectory.id.clone()
        };
        if !seen_trajectory_ids.insert(trajectory_key.clone()) {
            continue;
        }
        add_trajectory_usage(
            gcx.clone(),
            &mut pricing_cache,
            &mut accumulator,
            &mut seen_message_keys,
            &trajectory_key,
            &trajectory,
        )
        .await;
    }

    Ok(GoalBudgetSpent {
        elapsed_secs: elapsed_secs(goal.created_at.as_deref()),
        prompt_tokens: accumulator.prompt_tokens,
        completion_tokens: accumulator.completion_tokens,
        total_tokens: accumulator.total_tokens,
        cache_read_tokens: accumulator.cache_read_tokens,
        usd: if accumulator.saw_usage && !accumulator.missing_usd {
            Some(accumulator.usd)
        } else {
            None
        },
        no_progress_wakes: goal.spent.no_progress_wakes,
    })
}

pub async fn hydrate_goal_spent(gcx: Arc<GlobalContext>, mut goal: ConductorGoal) -> ConductorGoal {
    if goal.spent.elapsed_secs > 0
        || goal.spent.prompt_tokens > 0
        || goal.spent.completion_tokens > 0
        || goal.spent.total_tokens > 0
        || goal.spent.cache_read_tokens > 0
        || goal.spent.usd.is_some()
    {
        goal.spent.no_progress_wakes = goal.ledger.no_progress_wakes;
        return goal;
    }
    if let Ok(mut spent) = aggregate_goal_spent(gcx, &goal).await {
        spent.no_progress_wakes = goal.ledger.no_progress_wakes;
        goal.spent = spent;
    } else {
        goal.spent.no_progress_wakes = goal.ledger.no_progress_wakes;
    }
    goal
}

async fn owned_goal_task_ids(
    gcx: Arc<GlobalContext>,
    goal: &ConductorGoal,
) -> Result<HashSet<String>, String> {
    let mut task_ids = HashSet::new();
    if let Some(task_id) = goal
        .ledger
        .planner_task_id
        .as_deref()
        .map(str::trim)
        .filter(|task_id| !task_id.is_empty())
    {
        task_ids.insert(task_id.to_string());
    }
    for task_id in &goal.ledger.task_ids {
        let task_id = task_id.trim();
        if !task_id.is_empty() {
            task_ids.insert(task_id.to_string());
        }
    }
    if !goal.id.is_empty() {
        for task in storage::list_tasks(gcx).await? {
            if task
                .conductor
                .as_ref()
                .is_some_and(|link| link.goal_id == goal.id)
            {
                task_ids.insert(task.id);
            }
        }
    }
    Ok(task_ids)
}

async fn existing_owned_task_roots(
    gcx: Arc<GlobalContext>,
    task_ids: &HashSet<String>,
) -> HashMap<String, PathBuf> {
    let mut roots = HashMap::new();
    for task_id in task_ids {
        if let Ok(task_dir) = storage::find_task_dir(gcx.clone(), task_id).await {
            roots.insert(task_id.clone(), task_dir);
        }
    }
    roots
}

async fn collect_json_files(root: PathBuf) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut pending = vec![root];
    while let Some(dir) = pending.pop() {
        let mut entries = match fs::read_dir(&dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Failed to read trajectory dir: {error}")),
        };
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| error.to_string())?
        {
            let file_type = entry.file_type().await.map_err(|error| error.to_string())?;
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                pending.push(path);
            } else if file_type.is_file()
                && path.extension().and_then(|ext| ext.to_str()) == Some("json")
            {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

async fn read_budget_trajectory(path: &Path) -> Result<BudgetTrajectory, String> {
    let content = fs::read_to_string(path)
        .await
        .map_err(|error| format!("Failed to read trajectory {}: {error}", path.display()))?;
    serde_json::from_str(&content)
        .map_err(|error| format!("Failed to parse trajectory {}: {error}", path.display()))
}

fn trajectory_is_owned(
    trajectory: &BudgetTrajectory,
    path: &Path,
    owned_task_ids: &HashSet<String>,
    owned_task_roots: &HashMap<String, PathBuf>,
) -> bool {
    let path_owned = path_is_under_owned_task_roots(path, owned_task_roots);
    match trajectory.task_meta.as_ref() {
        Some(task_meta) => {
            let role_allowed = task_meta.role == "planner" || task_meta.role == "agents";
            role_allowed && owned_task_ids.contains(&task_meta.task_id)
        }
        None => path_owned,
    }
}

fn path_is_under_owned_task_roots(
    path: &Path,
    owned_task_roots: &HashMap<String, PathBuf>,
) -> bool {
    owned_task_roots
        .values()
        .any(|task_dir| path.starts_with(task_dir.join("trajectories")))
}

async fn add_trajectory_usage(
    gcx: Arc<GlobalContext>,
    pricing_cache: &mut HashMap<String, Option<ModelPricing>>,
    accumulator: &mut UsageAccumulator,
    seen_message_keys: &mut HashSet<(String, String)>,
    trajectory_key: &str,
    trajectory: &BudgetTrajectory,
) {
    for message in &trajectory.messages {
        let Some(usage) = message.usage.as_ref() else {
            continue;
        };
        if !message.message_id.is_empty()
            && !seen_message_keys.insert((trajectory_key.to_string(), message.message_id.clone()))
        {
            continue;
        }
        add_usage_tokens(accumulator, usage);
        match usage_usd(gcx.clone(), pricing_cache, &trajectory.model, usage).await {
            Some(usd) => accumulator.usd += usd,
            None => accumulator.missing_usd = true,
        }
    }
}

fn add_usage_tokens(accumulator: &mut UsageAccumulator, usage: &ChatUsage) {
    accumulator.saw_usage = true;
    accumulator.prompt_tokens += usage.prompt_tokens as u64;
    accumulator.completion_tokens += usage.completion_tokens as u64;
    accumulator.total_tokens += usage.total_tokens as u64;
    accumulator.cache_read_tokens += usage.cache_read_tokens.unwrap_or(0) as u64;
}

async fn usage_usd(
    gcx: Arc<GlobalContext>,
    pricing_cache: &mut HashMap<String, Option<ModelPricing>>,
    model_id: &str,
    usage: &ChatUsage,
) -> Option<f64> {
    if model_id.trim().is_empty() {
        return None;
    }
    if !pricing_cache.contains_key(model_id) {
        let pricing = lookup_model_pricing(&gcx, model_id).await;
        pricing_cache.insert(model_id.to_string(), pricing);
    }
    pricing_cache
        .get(model_id)
        .and_then(|pricing| pricing.as_ref())
        .and_then(|pricing| compute_cost(usage, pricing))
        .map(|metering| metering.total_usd)
}

fn elapsed_secs(created_at: Option<&str>) -> u64 {
    let Some(created_at) = created_at else {
        return 0;
    };
    let Ok(created_at) = chrono::DateTime::parse_from_rfc3339(created_at) else {
        return 0;
    };
    chrono::Utc::now()
        .signed_duration_since(created_at.with_timezone(&chrono::Utc))
        .num_seconds()
        .max(0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::model_caps::ModelCapabilities;
    use crate::caps::CodeAssistantCaps;
    use crate::providers::traits::ModelPricing;
    use crate::tasks::types::{TaskConductorLink, TaskConductorRole, TaskMeta, TaskStatus};
    use chrono::{Duration, Utc};
    use serde_json::{json, Value};

    async fn test_gcx(root: &Path) -> Arc<GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
        gcx
    }

    async fn seed_pricing(gcx: &Arc<GlobalContext>, model_id: &str, pricing: ModelPricing) {
        let mut model_caps = HashMap::new();
        model_caps.insert(
            model_id.to_string(),
            ModelCapabilities {
                pricing: Some(pricing),
                ..Default::default()
            },
        );
        let caps = CodeAssistantCaps {
            model_caps: Arc::new(model_caps),
            ..Default::default()
        };
        let mut caps_state = gcx.caps_state.write().await;
        caps_state.caps = Some(Arc::new(caps));
    }

    fn test_pricing() -> ModelPricing {
        ModelPricing {
            prompt: 1.0,
            generated: 2.0,
            cache_read: Some(0.5),
            cache_creation: None,
            context_over_200k: None,
        }
    }

    fn goal(goal_id: &str, task_ids: Vec<&str>, chat_ids: Vec<&str>) -> ConductorGoal {
        ConductorGoal {
            id: goal_id.to_string(),
            created_at: Some((Utc::now() - Duration::seconds(42)).to_rfc3339()),
            ledger: refact_buddy_core::conductor::GoalLedger {
                planner_task_id: task_ids.first().map(|task_id| task_id.to_string()),
                task_ids: task_ids.into_iter().map(ToString::to_string).collect(),
                chat_ids: chat_ids.into_iter().map(ToString::to_string).collect(),
                ..Default::default()
            },
            spent: GoalBudgetSpent {
                no_progress_wakes: 3,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    async fn write_task_meta(root: &Path, task_id: &str, goal_id: Option<&str>) {
        let task_dir = root.join(".refact").join("tasks").join(task_id);
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        let now = Utc::now().to_rfc3339();
        let meta = TaskMeta {
            schema_version: 1,
            id: task_id.to_string(),
            name: task_id.to_string(),
            status: TaskStatus::Active,
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
            conductor: goal_id.map(|goal_id| TaskConductorLink {
                goal_id: goal_id.to_string(),
                role: TaskConductorRole::Planner,
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
        root: &Path,
        task_id: &str,
        role: &str,
        agent_id: Option<&str>,
        chat_id: &str,
        model: &str,
        messages: Vec<Value>,
    ) {
        let mut dir = root
            .join(".refact")
            .join("tasks")
            .join(task_id)
            .join("trajectories")
            .join(role);
        if let Some(agent_id) = agent_id {
            dir = dir.join(agent_id);
        }
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let task_meta = ChatTaskMeta {
            task_id: task_id.to_string(),
            role: role.to_string(),
            agent_id: agent_id.map(ToString::to_string),
            card_id: (role == "agents").then(|| "card-1".to_string()),
            planner_chat_id: (role == "planner").then(|| chat_id.to_string()),
        };
        let trajectory = json!({
            "id": chat_id,
            "title": chat_id,
            "model": model,
            "mode": if role == "planner" { "task_planner" } else { "task_agent" },
            "tool_use": "agent",
            "messages": messages,
            "created_at": "2026-06-03T00:00:00Z",
            "updated_at": "2026-06-03T00:00:01Z",
            "include_project_info": true,
            "checkpoints_enabled": true,
            "task_meta": task_meta
        });
        tokio::fs::write(
            dir.join(format!("{chat_id}.json")),
            serde_json::to_string(&trajectory).unwrap(),
        )
        .await
        .unwrap();
    }

    fn usage_message(
        message_id: &str,
        prompt_tokens: usize,
        completion_tokens: usize,
        total_tokens: usize,
        cache_read_tokens: Option<usize>,
    ) -> Value {
        let mut usage = json!({
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": total_tokens
        });
        if let Some(cache_read_tokens) = cache_read_tokens {
            usage["cache_read_input_tokens"] = json!(cache_read_tokens);
        }
        json!({
            "role": "assistant",
            "message_id": message_id,
            "content": "ok",
            "usage": usage
        })
    }

    #[tokio::test]
    async fn token_cache_sum_uses_recorded_usage_and_caps_price() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx(dir.path()).await;
        seed_pricing(&gcx, "test/priced", test_pricing()).await;
        write_task_meta(dir.path(), "task-1", Some("goal-1")).await;
        write_trajectory(
            dir.path(),
            "task-1",
            "planner",
            None,
            "planner-priced",
            "test/priced",
            vec![
                usage_message("m1", 100, 20, 120, Some(30)),
                usage_message("m2", 5, 6, 11, None),
            ],
        )
        .await;

        let spent = aggregate_goal_spent(gcx, &goal("goal-1", vec!["task-1"], vec![]))
            .await
            .unwrap();

        assert_eq!(spent.prompt_tokens, 105);
        assert_eq!(spent.completion_tokens, 26);
        assert_eq!(spent.total_tokens, 131);
        assert_eq!(spent.cache_read_tokens, 30);
        assert_eq!(spent.no_progress_wakes, 3);
        assert!((spent.usd.unwrap() - 0.000172).abs() < 1e-12);
        assert!((42..=60).contains(&spent.elapsed_secs));
    }

    #[tokio::test]
    async fn missing_usage_records_do_not_add_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx(dir.path()).await;
        seed_pricing(&gcx, "test/priced", test_pricing()).await;
        write_task_meta(dir.path(), "task-1", Some("goal-1")).await;
        write_trajectory(
            dir.path(),
            "task-1",
            "planner",
            None,
            "planner-no-usage",
            "test/priced",
            vec![json!({"role": "assistant", "message_id": "m1", "content": "ok"})],
        )
        .await;

        let spent = aggregate_goal_spent(gcx, &goal("goal-1", vec!["task-1"], vec![]))
            .await
            .unwrap();

        assert_eq!(spent.prompt_tokens, 0);
        assert_eq!(spent.completion_tokens, 0);
        assert_eq!(spent.total_tokens, 0);
        assert_eq!(spent.cache_read_tokens, 0);
        assert_eq!(spent.usd, None);
    }

    #[tokio::test]
    async fn missing_price_keeps_tokens_and_omits_usd() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx(dir.path()).await;
        write_task_meta(dir.path(), "task-1", Some("goal-1")).await;
        write_trajectory(
            dir.path(),
            "task-1",
            "planner",
            None,
            "planner-unpriced",
            "test/unpriced",
            vec![usage_message("m1", 7, 8, 15, Some(2))],
        )
        .await;

        let spent = aggregate_goal_spent(gcx, &goal("goal-1", vec!["task-1"], vec![]))
            .await
            .unwrap();

        assert_eq!(spent.prompt_tokens, 7);
        assert_eq!(spent.completion_tokens, 8);
        assert_eq!(spent.total_tokens, 15);
        assert_eq!(spent.cache_read_tokens, 2);
        assert_eq!(spent.usd, None);
    }

    #[tokio::test]
    async fn hydrate_goal_spent_preserves_no_progress_when_aggregation_fails() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut goal = goal("goal-fallback", vec![], vec![]);
        goal.ledger.no_progress_wakes = 4;
        goal.spent.no_progress_wakes = 9;

        let hydrated = hydrate_goal_spent(gcx, goal).await;

        assert_eq!(hydrated.spent.no_progress_wakes, 4);
        assert_eq!(hydrated.spent.total_tokens, 0);
    }

    #[tokio::test]
    async fn duplicate_messages_within_same_trajectory_are_deduplicated() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx(dir.path()).await;
        seed_pricing(&gcx, "test/priced", test_pricing()).await;
        write_task_meta(dir.path(), "task-1", Some("goal-1")).await;
        write_trajectory(
            dir.path(),
            "task-1",
            "planner",
            None,
            "planner-dedupe",
            "test/priced",
            vec![
                usage_message("same-message", 10, 5, 15, None),
                usage_message("same-message", 10, 5, 15, None),
            ],
        )
        .await;

        let spent = aggregate_goal_spent(
            gcx,
            &goal("goal-1", vec!["task-1", "task-1"], vec!["planner-dedupe"]),
        )
        .await
        .unwrap();

        assert_eq!(spent.prompt_tokens, 10);
        assert_eq!(spent.completion_tokens, 5);
        assert_eq!(spent.total_tokens, 15);
    }

    #[tokio::test]
    async fn same_message_id_in_different_trajectories_is_counted_per_trajectory() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx(dir.path()).await;
        seed_pricing(&gcx, "test/priced", test_pricing()).await;
        write_task_meta(dir.path(), "task-1", Some("goal-1")).await;
        write_trajectory(
            dir.path(),
            "task-1",
            "planner",
            None,
            "planner-same-message",
            "test/priced",
            vec![usage_message("same-message", 10, 5, 15, None)],
        )
        .await;
        write_trajectory(
            dir.path(),
            "task-1",
            "agents",
            Some("agent-1"),
            "agent-same-message",
            "test/priced",
            vec![usage_message("same-message", 20, 7, 27, None)],
        )
        .await;

        let spent = aggregate_goal_spent(gcx, &goal("goal-1", vec!["task-1"], vec![]))
            .await
            .unwrap();

        assert_eq!(spent.prompt_tokens, 30);
        assert_eq!(spent.completion_tokens, 12);
        assert_eq!(spent.total_tokens, 42);
    }

    #[tokio::test]
    async fn unowned_tasks_and_chat_ids_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx(dir.path()).await;
        seed_pricing(&gcx, "test/priced", test_pricing()).await;
        write_task_meta(dir.path(), "owned-task", Some("goal-1")).await;
        write_task_meta(dir.path(), "unowned-task", None).await;
        write_trajectory(
            dir.path(),
            "owned-task",
            "planner",
            None,
            "owned-chat",
            "test/priced",
            vec![usage_message("owned-message", 3, 4, 7, None)],
        )
        .await;
        write_trajectory(
            dir.path(),
            "unowned-task",
            "planner",
            None,
            "unowned-chat",
            "test/priced",
            vec![usage_message("unowned-message", 100, 100, 200, None)],
        )
        .await;

        let spent = aggregate_goal_spent(
            gcx,
            &goal("goal-1", vec!["owned-task"], vec!["unowned-chat"]),
        )
        .await
        .unwrap();

        assert_eq!(spent.prompt_tokens, 3);
        assert_eq!(spent.completion_tokens, 4);
        assert_eq!(spent.total_tokens, 7);
    }

    #[tokio::test]
    async fn planner_and_agent_trajectories_are_included() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = test_gcx(dir.path()).await;
        seed_pricing(&gcx, "test/priced", test_pricing()).await;
        write_task_meta(dir.path(), "task-1", Some("goal-1")).await;
        write_trajectory(
            dir.path(),
            "task-1",
            "planner",
            None,
            "planner-chat",
            "test/priced",
            vec![usage_message("planner-message", 11, 12, 23, None)],
        )
        .await;
        write_trajectory(
            dir.path(),
            "task-1",
            "agents",
            Some("agent-1"),
            "agent-chat",
            "test/priced",
            vec![usage_message("agent-message", 13, 14, 27, Some(5))],
        )
        .await;

        let spent = aggregate_goal_spent(gcx, &goal("goal-1", vec!["task-1"], vec![]))
            .await
            .unwrap();

        assert_eq!(spent.prompt_tokens, 24);
        assert_eq!(spent.completion_tokens, 26);
        assert_eq!(spent.total_tokens, 50);
        assert_eq!(spent.cache_read_tokens, 5);
    }
}
