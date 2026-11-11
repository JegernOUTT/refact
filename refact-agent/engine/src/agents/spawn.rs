use std::path::{Path, PathBuf};
#[cfg(test)]
use std::pin::Pin;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;
use refact_core::chat_types::{PendingDelivery, PushMode};
use tokio::sync::{Mutex as AMutex, mpsc::UnboundedSender, oneshot};
use uuid::Uuid;

use crate::agents::types::{
    AgentCompletion, BackgroundAgent, BgAgentKind, CreateAgentRequest, NO_TEXT_RESULT_SUMMARY,
};
use crate::app_state::AppState;
use crate::call_validation::{ChatContent, ChatMessage};
use crate::chat::internal_roles::{event, EventSubkind};
use crate::chat::types::{ChatEvent, GoalBudget, GoalCriterion, TaskMeta};
use crate::ext::hooks::HookEvent;
use crate::ext::hooks_runner::{HookPayload, get_project_dir_string, run_observation_hooks};
use crate::global_context::GlobalContext;
use crate::subchat::{
    RunnerLeaseGuard, SubchatConfig, SubchatProgress, SubchatResult,
    resolve_subchat_config_with_parent,
};
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::{
    CreateWorktreeRequest, MergeWorktreeRequest, WorktreeMergeStrategy, WorktreeMeta,
};

const MAX_DIFF_SUMMARY_PATHSPECS: usize = 100;
/// Steps one deliverable correction may spend. A correction reposts a final
/// message, it does not resume the task.
const CORRECTION_MAX_STEPS: usize = 4;

#[derive(Default)]
struct SpawnOwners {
    closing: bool,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    cancellations: Vec<std::sync::Weak<AtomicBool>>,
}
static SPAWN_OWNERS: std::sync::OnceLock<std::sync::Mutex<SpawnOwners>> =
    std::sync::OnceLock::new();

pub(crate) fn track_spawn_owner(
    task: impl std::future::Future<Output = ()> + Send + 'static,
    cleanup: bool,
) -> Result<(), String> {
    let mut owners = SPAWN_OWNERS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "spawn owner unavailable")?;
    owners.tasks.retain(|task| !task.is_finished());
    if !cleanup && (owners.closing || owners.tasks.len() >= 64) {
        return Err("spawn owner shutdown or capacity reached".into());
    }
    owners.tasks.push(tokio::spawn(task));
    Ok(())
}

pub(crate) fn track_cancellable_spawn_owner(
    task: impl std::future::Future<Output = ()> + Send + 'static,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    let mut owners = SPAWN_OWNERS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "spawn owner unavailable")?;
    owners.tasks.retain(|task| !task.is_finished());
    owners.cancellations.retain(|flag| flag.strong_count() > 0);
    if owners.closing || owners.tasks.len() >= 64 {
        return Err("spawn owner shutdown or capacity reached".into());
    }
    owners.cancellations.push(Arc::downgrade(cancel));
    owners.tasks.push(tokio::spawn(task));
    Ok(())
}

/// Drain and join every owned spawn task.
///
/// `close_admission` permanently refuses all later spawn-owner registration and
/// must therefore be set only when the process is actually shutting down.
/// `BackgroundTasksHolder::abort` is also used to *recycle* a holder at runtime
/// (vecdb reload), and latching this process-global gate there would make every
/// later supervised spawn fail with "spawn owner shutdown or capacity reached"
/// for the remaining life of the process.
pub(crate) async fn drain_spawn_owners(close_admission: bool) {
    loop {
        let tasks = {
            let mut owners = SPAWN_OWNERS.get_or_init(Default::default).lock().unwrap();
            owners.closing = owners.closing || close_admission;
            for cancel in owners
                .cancellations
                .drain(..)
                .filter_map(|flag| flag.upgrade())
            {
                cancel.store(true, std::sync::atomic::Ordering::Release);
            }
            std::mem::take(&mut owners.tasks)
        };
        if tasks.is_empty() {
            break;
        }
        for task in tasks {
            let _ = task.await;
        }
    }
}

async fn handoff_startup_result<T: Clone, F: std::future::Future<Output = ()>>(
    result: Result<T, String>,
    tx: oneshot::Sender<Result<T, String>>,
    accepted: oneshot::Receiver<()>,
    cleanup: impl FnOnce(T) -> F,
) {
    match result {
        Ok(created) => {
            if tx.send(Ok(created.clone())).is_err() || accepted.await.is_err() {
                cleanup(created).await;
            }
        }
        Err(error) => {
            let _ = tx.send(Err(error));
        }
    }
}

#[cfg(test)]
type TestRunner = Arc<
    dyn Fn(
            Arc<GlobalContext>,
            Vec<ChatMessage>,
            SubchatConfig,
        )
            -> Pin<Box<dyn std::future::Future<Output = Result<SubchatResult, String>> + Send>>
        + Send
        + Sync,
>;

#[cfg(test)]
static TEST_RUNNER: std::sync::OnceLock<std::sync::Mutex<Option<TestRunner>>> =
    std::sync::OnceLock::new();

#[cfg(test)]
pub struct TestRunnerGuard;

#[cfg(test)]
impl Drop for TestRunnerGuard {
    fn drop(&mut self) {
        if let Some(runner) = TEST_RUNNER.get() {
            *runner.lock().unwrap() = None;
        }
    }
}

#[cfg(test)]
pub fn install_test_runner(runner: TestRunner) -> TestRunnerGuard {
    *TEST_RUNNER
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap() = Some(runner);
    TestRunnerGuard
}

#[derive(Clone)]
pub struct SpawnRequest {
    pub native_supervision_guard: Option<crate::buddy::supervision_authority::NativeSpawnGuard>,
    pub kind: BgAgentKind,
    pub parent_chat_id: String,
    pub parent_root_chat_id: Option<String>,
    pub parent_tool_call_id: Option<String>,
    pub config_name: String,
    pub title: String,
    pub prompt: String,
    pub initial_context_messages: Vec<ChatMessage>,
    pub history_source_model: Option<String>,
    pub custom_instructions: Option<String>,
    pub tools: Option<Vec<String>>,
    pub target_files: Vec<String>,
    pub max_steps: usize,
    pub model: String,
    pub model_type: Option<String>,
    pub context_tokens_cap: Option<usize>,
    pub goal: Option<SpawnGoal>,
    pub plan: Option<String>,
    pub deliverable: Option<refact_chat_api::DeliverableContract>,
    pub profile: Option<super::profile::ResolvedProfile>,
    pub worktree_mode: SpawnWorktreeMode,
    pub parent_subchat_tx: Option<Arc<AMutex<UnboundedSender<Value>>>>,
    pub parent_worktree: Option<WorktreeMeta>,
    pub parent_task_meta: Option<TaskMeta>,
    pub subchat_depth: usize,
    pub notify_parent: NotifyParent,
    pub completion_push: PushMode,
}

#[derive(Clone, PartialEq, serde::Serialize)]
pub struct SpawnGoal {
    pub content: String,
    pub criteria: Vec<GoalCriterion>,
    pub budget: Option<GoalBudget>,
}

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub enum SpawnWorktreeMode {
    #[default]
    Inherit,
    Isolated {
        auto_merge: bool,
    },
}

#[derive(Clone)]
struct SpawnedWorktree {
    meta: WorktreeMeta,
    base_branch: Option<String>,
    source_workspace_root: PathBuf,
    auto_merge: bool,
    parent_worktree: Option<WorktreeMeta>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NotifyParent {
    Auto,
    Silent,
}

pub struct SpawnHandle {
    pub agent_id: String,
    pub child_chat_id: String,
    pub launch_revision: u64,
    pub launch_digest: String,
    pub worktree_branch: Option<String>,
    pub auto_merge: Option<bool>,
    pub completion_rx: oneshot::Receiver<BackgroundAgent>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PreparedLaunch {
    schema_version: u32,
    revision: u64,
    config: SubchatConfig,
    messages: Vec<ChatMessage>,
    project_root: PathBuf,
    worktree_mode: SpawnWorktreeMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile: Option<super::profile::ResolvedProfile>,
    #[serde(skip)]
    parent_run: Option<refact_agents_core::registry::ParentRun>,
    #[serde(skip)]
    parent_abort: Option<Arc<AtomicBool>>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct DurableRunConfig {
    schema_version: u32,
    run_generation: u64,
    launch_digest: String,
    project_root: PathBuf,
    config: SubchatConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    replacement: Option<ReplacementIntent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_attachment: Option<RecoveryAttachment>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct RecoveryAttachment {
    worktree: Option<WorktreeMeta>,
    trajectory_revision: String,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct ReplacementIntent {
    operation: Uuid,
    previous_generation: u64,
    previous_digest: String,
    attachment: RecoveryAttachment,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct AppliedCheckout {
    schema_version: u32,
    operation: Uuid,
    seed_digest: String,
    worktree: WorktreeMeta,
}

async fn persist_run_config(
    app: &AppState,
    agent_id: &str,
    config: &SubchatConfig,
) -> Result<(), String> {
    let record = app.agents.get_any(agent_id).await?;
    persist_resume_config(app, &record, config, None, None).await
}

async fn persist_resume_config(
    app: &AppState,
    record: &BackgroundAgent,
    config: &SubchatConfig,
    replacement: Option<ReplacementIntent>,
    previous_attachment: Option<RecoveryAttachment>,
) -> Result<(), String> {
    let agent_id = record.agent_id.as_str();
    let root = app
        .agent_executions
        .saved_scope(agent_id)
        .ok_or("accepted project scope unavailable")?;
    let limits = crate::runtime_settings::coordination_for_project(&root).await?;
    let accepted = app
        .agents
        .accepted_launch(
            &record.parent_chat_id,
            agent_id,
            limits.launch_context_bytes,
        )
        .await?;
    let payload = DurableRunConfig {
        schema_version: 1,
        run_generation: record.run_generation,
        launch_digest: launch_digest(&accepted),
        project_root: root,
        config: config.clone(),
        replacement,
        previous_attachment,
    };
    let payload = refact_agents_core::storage::AcceptedLaunchPayload::serialize(
        &payload,
        limits.launch_context_bytes,
    )?;
    app.agents
        .save_run_config(agent_id, record.run_generation, &payload)
        .await
}

pub(crate) struct RecoveredRun {
    pub config: SubchatConfig,
    pub root: PathBuf,
    pub authority_migrated: bool,
    isolated_worktree: Option<SpawnedWorktree>,
    replace_worktree: bool,
    previous_attachment: Option<RecoveryAttachment>,
    pending_replacement: Option<ReplacementIntent>,
}

pub(crate) async fn recover_run_config(
    app: &AppState,
    parent_chat_id: &str,
    agent_id: &str,
) -> Result<RecoveredRun, String> {
    let record = app.agents.get_any(agent_id).await?;
    if record.parent_chat_id != parent_chat_id {
        return Err("child owner does not match".into());
    }
    let parent = app
        .chat
        .sessions
        .read()
        .await
        .get(parent_chat_id)
        .cloned()
        .ok_or("parent session unavailable")?;
    let (source, parent_thread) = {
        let parent = parent.lock().await;
        (
            crate::coordination::scope::session_source_root(&parent),
            parent.thread.clone(),
        )
    };
    let root = trusted_project_root(app, source).await?;
    let limits = crate::runtime_settings::coordination_for_project(&root).await?;
    let (saved_payload, applied) = app
        .agents
        .recovery_run_config(
            parent_chat_id,
            agent_id,
            record.run_generation,
            record.run_generation,
            limits.launch_context_bytes,
        )
        .await?;
    let saved: DurableRunConfig = saved_payload.decode()?;
    let accepted = app
        .agents
        .accepted_launch(parent_chat_id, agent_id, limits.launch_context_bytes)
        .await?;
    let seed: PreparedLaunch = accepted.decode()?;
    let mut config = saved.config;
    let mut previous_attachment = saved.previous_attachment;
    let replacement_pending = saved.replacement.is_some() && applied.is_none();
    if let Some(intent) = &saved.replacement {
        if intent.operation.is_nil()
            || intent.previous_generation == 0
            || intent.previous_generation >= record.run_generation
            || intent.attachment.trajectory_revision.len() != 64
            || !record.previous_runs.iter().any(|run| {
                run.run_generation == intent.previous_generation
                    && run.status == crate::agents::types::BgAgentStatus::Completed
            })
        {
            return Err("invalid accepted checkout replacement intent".into());
        }
        let (previous, previous_applied) = app
            .agents
            .recovery_run_config(
                parent_chat_id,
                agent_id,
                record.run_generation,
                intent.previous_generation,
                limits.launch_context_bytes,
            )
            .await?;
        let previous_config: DurableRunConfig = previous.decode()?;
        let previous_worktree = match previous_applied {
            Some(applied) => {
                let applied: AppliedCheckout = applied.decode()?;
                if applied.schema_version != 1
                    || applied.seed_digest != launch_digest(&previous)
                    || previous_config
                        .replacement
                        .as_ref()
                        .map(|intent| intent.operation)
                        != Some(applied.operation)
                {
                    return Err("previous checkout setup does not match its seed".into());
                }
                Some(applied.worktree)
            }
            None => previous_config.config.worktree.clone(),
        };
        if launch_digest(&previous) != intent.previous_digest
            || previous_config.schema_version != 1
            || previous_config.run_generation != intent.previous_generation
            || previous_config.launch_digest != launch_digest(&accepted)
            || previous_config.project_root != root
            || previous_config.config.chat_id != config.chat_id
            || previous_config.config.parent_id != config.parent_id
            || previous_config.config.background_agent_id != config.background_agent_id
            || previous_config.config.model != config.model
            || previous_worktree != config.worktree
            || !record.previous_runs.iter().any(|run| {
                run.run_generation == intent.previous_generation
                    && run.worktree_id.as_ref()
                        == previous_worktree.as_ref().map(|worktree| &worktree.id)
                    && run.worktree_branch
                        == previous_worktree
                            .as_ref()
                            .and_then(|worktree| worktree.branch.clone())
            })
        {
            return Err("checkout replacement intent does not match prior committed seed".into());
        }
        if let Some(applied) = applied {
            let applied: AppliedCheckout = applied.decode()?;
            if applied.schema_version != 1
                || applied.operation != intent.operation
                || applied.seed_digest != launch_digest(&saved_payload)
                || applied.worktree.source_workspace_root != root
                || applied.worktree.branch.as_deref()
                    != Some(format!("refact/subagent/{}", intent.operation.simple()).as_str())
                || config.worktree.as_ref().is_none_or(|previous| {
                    applied.worktree.id == previous.id
                        || applied.worktree.root == previous.root
                        || applied.worktree.base_branch != previous.base_branch
                        || applied.worktree.kind != previous.kind
                })
                || Some(&applied.worktree.id) != record.worktree_id.as_ref()
                || applied.worktree.branch != record.worktree_branch
            {
                return Err(
                    "checkout publication outcome unknown; inspect retained setup before recovery"
                        .into(),
                );
            }
            if applied.worktree.root.is_dir() {
                let service =
                    WorktreeService::new_async(app.gcx.cache_dir.clone(), root.clone()).await?;
                if service.get_worktree(&applied.worktree.id).await?.meta != applied.worktree {
                    return Err("committed replacement checkout registration changed".into());
                }
            }
            config.worktree = Some(applied.worktree);
            previous_attachment = Some(intent.attachment.clone());
        }
    } else if applied.is_some() {
        return Err("checkout setup has no accepted replacement intent".into());
    }
    if previous_attachment.as_ref().is_some_and(|previous| {
        previous.trajectory_revision.len() != 64
            || previous
                .worktree
                .as_ref()
                .is_some_and(|worktree| worktree.source_workspace_root != root)
    }) {
        return Err("invalid previous checkout attachment receipt".into());
    }
    if saved.schema_version != 1
        || seed.schema_version != 1
        || seed.revision != 1
        || saved.run_generation != record.run_generation
        || saved.launch_digest != launch_digest(&accepted)
        || saved.project_root != root
        || seed.project_root != root
        || record.child_chat_id.is_none()
        || config.chat_id != record.child_chat_id
        || seed.config.chat_id != record.child_chat_id
        || config.parent_id.as_deref() != Some(parent_chat_id)
        || seed.config.parent_id != config.parent_id
        || config.background_agent_id.as_deref() != Some(agent_id)
        || config.model != seed.config.model
        || config.model != record.model
        || config.root_chat_id != seed.config.root_chat_id
        || config.root_chat_id != record.parent_root_chat_id
        || config.mode != seed.config.mode
        || config.context_tokens_cap() != seed.config.context_tokens_cap()
        || config.subchat_depth != seed.config.subchat_depth
        || !config.stateful
        || !seed.config.stateful
        || config.max_steps == 0
        || (!config.ordinary_background_has_unlimited_steps()
            && config.max_steps > limits.subagent_max_steps)
        || config.subchat_depth == 0
        || config.subchat_depth >= limits.max_subchat_depth
        || !tools_within_ceiling(&config.tools, &seed.config.tools)
        || config.worktree.as_ref().map(|w| &w.id) != record.worktree_id.as_ref()
    {
        return Err(
            "persisted run does not match accepted child identity or current limits".into(),
        );
    }
    validate_stopped_cap(&record, &config)?;
    if config.worktree.is_none() {
        validate_unscoped_project(app, &root).await?;
    }
    let isolated_worktree = match (&seed.worktree_mode, &config.worktree) {
        (
            SpawnWorktreeMode::Isolated {
                auto_merge,
            },
            Some(meta),
        ) => Some(SpawnedWorktree {
            meta: meta.clone(),
            base_branch: meta.base_branch.clone(),
            source_workspace_root: root.clone(),
            auto_merge: *auto_merge,
            parent_worktree: seed.config.worktree.clone(),
        }),
        _ => None,
    };
    let replace_worktree = replacement_pending
        || config.worktree.as_ref().is_some_and(|worktree| {
            record.merge_status.as_deref() == Some("merged") || !worktree.root.is_dir()
        });
    if let Some(worktree) = &config.worktree {
        if dunce::canonicalize(&worktree.source_workspace_root)
            .ok()
            .as_ref()
            != Some(&root)
        {
            return Err("original child checkout is unavailable".into());
        }
        if replace_worktree {
            let isolated = isolated_worktree
                .as_ref()
                .filter(|_| {
                    replacement_pending
                        || record.status == crate::agents::types::BgAgentStatus::Completed
                })
                .ok_or("original child checkout is unavailable; spawn a new agent instead")?;
            continuation_base_commit(isolated).await?;
        }
    }
    config.autonomous_no_confirm &=
        seed.config.autonomous_no_confirm && parent_thread.autonomous_no_confirm;
    config.auto_approve_editing_tools &= seed.config.auto_approve_editing_tools
        && (parent_thread.auto_approve_editing_tools || parent_thread.autonomous_no_confirm);
    config.auto_approve_dangerous_commands &= seed.config.auto_approve_dangerous_commands
        && super::approval::dangerous_confirmation_enabled(app, parent_chat_id).await;
    let ceiling =
        super::tool_authority::current_ceiling(app, parent_chat_id, &config.model, &config.mode)
            .await?;
    let authority_migrated = matches!(config.tools, crate::subchat::ToolsPolicy::All);
    config.tools = super::tool_authority::intersect(&config.tools, &ceiling);
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(app.gcx.clone(), 0)
        .await
        .map_err(|_| "saved child model unavailable")?;
    let model = crate::caps::resolve_chat_model(caps, &config.model)
        .map_err(|_| "saved child model unavailable")?;
    super::launch_spec::clear_current_context(&app.gcx, &seed.messages, &model.base)?;
    Ok(RecoveredRun {
        config,
        root,
        authority_migrated,
        isolated_worktree,
        replace_worktree,
        previous_attachment,
        pending_replacement: if replacement_pending {
            saved.replacement
        } else {
            None
        },
    })
}

async fn ensure_replacement_not_started(
    root: &Path,
    replacement: Option<&ReplacementIntent>,
) -> Result<(), String> {
    let Some(replacement) = replacement else {
        return Ok(());
    };
    let root = root.to_path_buf();
    let reference = format!(
        "refs/heads/refact/subagent/{}",
        replacement.operation.simple()
    );
    tokio::task::spawn_blocking(move || {
        let repo = crate::worktrees::git::discover_repo(&root)?;
        let absent = matches!(repo.find_reference(&reference), Err(error) if error.code() == git2::ErrorCode::NotFound);
        if absent { Ok(()) } else { Err("replacement checkout setup was already attempted; inspect retained checkout and publication evidence before recovery".into()) }
    }).await.map_err(|_| "replacement checkout inspection unavailable".to_string())?
}

fn tools_within_ceiling(
    tools: &crate::subchat::ToolsPolicy,
    ceiling: &crate::subchat::ToolsPolicy,
) -> bool {
    use crate::subchat::ToolsPolicy;
    match (tools, ceiling) {
        (_, ToolsPolicy::All) | (ToolsPolicy::None, _) => true,
        (ToolsPolicy::Only(tools), ToolsPolicy::Only(allowed)) => {
            tools.iter().all(|tool| allowed.contains(tool))
        }
        (ToolsPolicy::Only(tools), ToolsPolicy::None) => tools.is_empty(),
        _ => false,
    }
}

fn validate_stopped_cap(record: &BackgroundAgent, config: &SubchatConfig) -> Result<(), String> {
    let cap = record
        .error
        .as_deref()
        .and_then(crate::agents::types::CapStop::from_error);
    if cap.as_ref().is_some_and(|cap| {
        config.max_steps != cap.limit
            || cap.used < cap.limit
            || (cap.run_generation != 0 && cap.run_generation != record.run_generation)
    }) {
        return Err("saved child configuration does not match the stopped run".into());
    }
    Ok(())
}

fn validate_recovery_trajectory(
    record: &BackgroundAgent,
    config: &SubchatConfig,
    replace_worktree: bool,
    previous_attachment: Option<&RecoveryAttachment>,
    trajectory: &crate::chat::trajectories::LoadedTrajectory,
) -> Result<(), String> {
    if Some(&trajectory.thread.id) != record.child_chat_id.as_ref()
        || trajectory.thread.parent_id.as_deref() != Some(record.parent_chat_id.as_str())
        || trajectory.thread.root_chat_id != config.root_chat_id
        || (trajectory
            .thread
            .worktree
            .as_ref()
            .map(|w| (&w.id, &w.root))
            != config.worktree.as_ref().map(|w| (&w.id, &w.root))
            && !(replace_worktree && trajectory.thread.worktree.is_none())
            && !previous_attachment.is_some_and(|previous| {
                trajectory.thread.worktree == previous.worktree
                    && resume_trajectory_revision(trajectory).ok().as_deref()
                        == Some(previous.trajectory_revision.as_str())
            }))
    {
        return Err("saved trajectory identity does not match this child".into());
    }
    if let Some(cap) = record
        .error
        .as_deref()
        .and_then(crate::agents::types::CapStop::from_error)
    {
        let saved =
            crate::subchat::previous_agent_cap(&trajectory.messages, Some(&record.agent_id));
        if saved.as_ref() != Some(&cap) && !(saved.is_none() && cap.run_generation == 0) {
            return Err("saved trajectory does not contain the matching stopped attempt".into());
        }
    }
    Ok(())
}

pub(crate) async fn recovery_unavailable_reason(
    app: &AppState,
    record: &BackgroundAgent,
    run: &RecoveredRun,
) -> Option<String> {
    let Some(child) = record.child_chat_id.as_deref() else {
        return Some("child trajectory unavailable".into());
    };
    if app.agent_executions.get(child).is_some() || app.agent_executions.task_active(child) {
        return Some("previous child execution is still active".into());
    }
    let Some(trajectory) =
        crate::chat::trajectories::load_trajectory_for_recovery(app.gcx.clone(), child).await
    else {
        return Some("saved child trajectory is unavailable".into());
    };
    if let Err(error) = validate_recovery_trajectory(
        record,
        &run.config,
        run.replace_worktree,
        run.previous_attachment.as_ref(),
        &trajectory,
    ) {
        return Some(error);
    }
    if let Err(error) =
        ensure_replacement_not_started(&run.root, run.pending_replacement.as_ref()).await
    {
        return Some(error);
    }
    let cap = record
        .error
        .as_deref()
        .and_then(crate::agents::types::CapStop::from_error);
    let used = cap.as_ref().map_or(record.step_count as usize, |cap| {
        cap.used.max(record.step_count as usize)
    });
    remaining_goal_steps(
        &trajectory,
        used,
        usize::MAX,
        run.config.ordinary_background_has_unlimited_steps(),
    )
    .err()
}

fn remaining_goal_steps(
    trajectory: &crate::chat::trajectories::LoadedTrajectory,
    used_steps: usize,
    mut remaining: usize,
    unlimited: bool,
) -> Result<usize, String> {
    let projected_goal = crate::chat::goal_snapshot_from_messages(&trajectory.messages, None);
    for goal in trajectory.goal.iter().chain(projected_goal.iter()) {
        if matches!(
            goal.status,
            crate::chat::types::GoalStatus::Paused | crate::chat::types::GoalStatus::Stopped
        ) {
            return Err("child goal is explicitly paused or stopped; resume its goal through the authorized user control first".into());
        }
        if unlimited {
            continue;
        }
        if refact_chat_api::goal_budget_exhaustion_status(
            &goal.budget,
            &goal.progress,
            chrono::Utc::now().timestamp_millis().max(0) as u64,
        )
        .is_some()
        {
            return Err(
                "explicit goal budget is exhausted; a step override cannot widen it".into(),
            );
        }
        if let Some(limit) = goal.budget.max_turns.filter(|limit| *limit > 0) {
            let used = used_steps.max(goal.progress.turns_used as usize);
            if used >= limit as usize {
                return Err(
                    "explicit goal turn budget is exhausted; a step override cannot widen it"
                        .into(),
                );
            }
            remaining = remaining.min(limit as usize - used);
        }
    }
    Ok(remaining)
}

fn launch_digest(payload: &refact_agents_core::storage::AcceptedLaunchPayload) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(payload.as_bytes()))
}

/// The settings a child would actually launch with, computed by the same
/// preparation path as an accepted launch but without creating a record,
/// reserving capacity or touching a worktree.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LaunchEffective {
    pub schema_version: u32,
    pub revision: u64,
    pub model: String,
    pub context_tokens_cap: Option<usize>,
    pub n_ctx: usize,
    pub mode: String,
    pub tools: crate::subchat::ToolsPolicy,
    pub max_steps: usize,
    pub subchat_depth: usize,
    pub stateful: bool,
    pub autonomous_no_confirm: bool,
    pub auto_approve_editing_tools: bool,
    pub auto_approve_dangerous_commands: bool,
    pub worktree_mode: String,
    pub parent_worktree_id: Option<String>,
    pub project_root: PathBuf,
    pub messages: usize,
    pub accepted_bytes: usize,
    pub accepted_byte_limit: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<super::profile::ResolvedProfile>,
}

async fn validate_launch_request(app: &AppState, req: &SpawnRequest) -> Result<PathBuf, String> {
    if let Some(goal) = &req.goal {
        let policy = crate::chat::verify_cmd::VerifyCommandPolicy::load(app.gcx.clone()).await;
        crate::chat::evidence::validate_goal_verify_commands(&goal.criteria, &policy)?;
    }
    let max_depth = crate::runtime_settings::current()
        .coordination
        .max_subchat_depth;
    if req.subchat_depth >= max_depth.saturating_sub(1) {
        return Err(format!("subchat depth limit ({}) exceeded", max_depth));
    }
    // Reject an invalid launch before creating a record, reserving capacity or
    // preparing a worktree. Queued setup must not turn a deterministic argument
    // error into a successful acceptance followed by a background failure.
    #[cfg(test)]
    let synthetic_config = req.config_name == "test_spawn";
    #[cfg(not(test))]
    let synthetic_config = false;
    if !synthetic_config {
        let config = crate::yaml_configs::customization_registry::get_subagent_config(
            app.gcx.clone(),
            &req.config_name,
            None,
        )
        .await
        .ok_or_else(|| format!("subagent config '{}' not found", req.config_name))?;
        if config.messages.system_prompt.is_none() {
            return Err(format!(
                "subagent config '{}' has no system prompt",
                req.config_name
            ));
        }
    }
    let parent = app
        .chat
        .sessions
        .read()
        .await
        .get(&req.parent_chat_id)
        .cloned()
        .ok_or("parent session unavailable")?;
    let source = crate::coordination::scope::session_source_root(&*parent.lock().await);
    let root = trusted_project_root(app, source).await?;
    if req.parent_worktree.as_ref().is_some_and(|worktree| {
        dunce::canonicalize(&worktree.source_workspace_root)
            .ok()
            .as_ref()
            != Some(&root)
    }) {
        return Err("requested parent checkout does not match trusted project scope".into());
    }
    if req.parent_worktree.is_none() {
        validate_unscoped_project(app, &root).await?;
    }
    Ok(root)
}

async fn validate_unscoped_project(app: &AppState, root: &Path) -> Result<(), String> {
    if crate::files_correction::get_active_project_path(app.gcx.clone())
        .await
        .and_then(|path| dunce::canonicalize(path).ok())
        .as_deref()
        != Some(root)
    {
        return Err("original unscoped child workspace is no longer active".into());
    }
    Ok(())
}

async fn trusted_project_root(app: &AppState, source: Option<PathBuf>) -> Result<PathBuf, String> {
    let source = source.ok_or("parent project scope unavailable")?;
    crate::coordination::roots::resolve_source_workspace_root(
        app.gcx.clone(),
        Some(source.to_str().ok_or("parent project scope is invalid")?),
    )
    .await
    .map_err(|_| "parent project scope is not available or authorized".to_string())
}

pub async fn preview_launch(app: &AppState, req: &SpawnRequest) -> Result<LaunchEffective, String> {
    let project_root = validate_launch_request(app, req).await?;
    let limits = crate::runtime_settings::coordination_for_project(&project_root).await?;
    let child_chat_id = format!("subchat-preview-{}", Uuid::new_v4());
    let (prepared, payload) =
        prepare_launch(app, req, &child_chat_id, project_root.clone()).await?;
    let config = prepared.config;
    Ok(LaunchEffective {
        schema_version: prepared.schema_version,
        revision: prepared.revision,
        context_tokens_cap: config.context_tokens_cap(),
        n_ctx: config.n_ctx,
        model: config.model,
        mode: config.mode,
        tools: config.tools,
        max_steps: config.max_steps,
        subchat_depth: config.subchat_depth,
        stateful: config.stateful,
        autonomous_no_confirm: config.autonomous_no_confirm,
        auto_approve_editing_tools: config.auto_approve_editing_tools,
        auto_approve_dangerous_commands: config.auto_approve_dangerous_commands,
        worktree_mode: match &prepared.worktree_mode {
            SpawnWorktreeMode::Inherit => "inherit".to_string(),
            SpawnWorktreeMode::Isolated {
                auto_merge,
            } => {
                format!("isolated(auto_merge={auto_merge})")
            }
        },
        parent_worktree_id: config.worktree.as_ref().map(|w| w.id.clone()),
        project_root,
        messages: prepared.messages.len(),
        accepted_bytes: payload.as_bytes().len(),
        accepted_byte_limit: limits.launch_context_bytes,
        profile: prepared.profile,
    })
}

/// Roll back native startup effects on every error and on future cancellation.
/// The consumed mandate binding is deliberately retained: uncertain dispatches
/// must never be replayed automatically.
struct SupervisedStartupRollback {
    app: AppState,
    child: String,
    worktree: Option<SpawnedWorktree>,
    agent_id: Option<String>,
    armed: bool,
}

impl Drop for SupervisedStartupRollback {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let app = self.app.clone();
        let child = self.child.clone();
        let worktree = self.worktree.take();
        let agent_id = self.agent_id.take();
        let _ = track_spawn_owner(
            async move {
                if let Some(agent_id) = agent_id {
                    let _ = app
                        .agents
                        .mark_cancelled(&agent_id, Some("supervised startup cancelled".into()))
                        .await;
                }
                if let Some(session) = app.chat.sessions.write().await.remove(&child) {
                    session.lock().await.close_event_channel();
                }
                cleanup_spawn_worktree(app, worktree.as_ref()).await;
            },
            true,
        );
    }
}

pub async fn spawn_background_agent(
    app: AppState,
    req: SpawnRequest,
) -> Result<SpawnHandle, String> {
    let (parent_run, parent_abort) = launch_parent_state(&app, &req.parent_chat_id).await?;
    if let Some(guard) = &req.native_supervision_guard {
        guard.validate_request(&req)?;
    }
    let project_root = validate_launch_request(&app, &req).await?;
    let child_uuid = Uuid::new_v4();
    let child_chat_id = if req.native_supervision_guard.is_some() {
        format!("subchat-supervised-{child_uuid}")
    } else {
        format!("subchat-{child_uuid}")
    };
    if let Some(guard) = &req.native_supervision_guard {
        guard.bind_child(&child_chat_id)?;
    }
    crate::coordination::causal::record_spawn(&app, &req.parent_chat_id, &child_chat_id).await;
    let (prepared, launch_payload) = prepare_launch_for_parent_run(
        &app,
        &req,
        &child_chat_id,
        project_root.clone(),
        (parent_run, parent_abort),
    )
    .await?;
    validate_launch_parent(&app, &req.parent_chat_id, &prepared).await?;
    let launch_digest = launch_digest(&launch_payload);
    let parent_session = app
        .chat
        .sessions
        .read()
        .await
        .get(&req.parent_chat_id)
        .cloned();
    let mut launch_provenance = if let Some(parent) = parent_session {
        let parent = parent.lock().await;
        parent
            .work_provenance
            .snapshot(parent.messages.len(), &crate::file_activity::settings())
    } else {
        (Vec::new(), false)
    };
    let context = crate::coordination::provenance::WorkProvenance::from_messages(
        &req.initial_context_messages,
        &crate::file_activity::settings(),
    )
    .snapshot(
        req.initial_context_messages.len(),
        &crate::file_activity::settings(),
    );
    launch_provenance.0.extend(context.0);
    launch_provenance.1 &= context.1;
    let (send, receive) = oneshot::channel();
    let (start_send, start_receive) = oneshot::channel();
    let executions = app.agent_executions.clone();
    let setup_id = format!("accept:{child_chat_id}");
    executions.spawn_owned(
        setup_id,
        Arc::new(AtomicBool::new(false)),
        Arc::new(tokio::sync::Notify::new()),
        async move {
            let result = accept_prepared_launch(
                app,
                req,
                prepared,
                launch_payload,
                launch_digest,
                project_root,
                launch_provenance,
                child_uuid,
                child_chat_id,
                start_receive,
            )
            .await;
            let _ = send.send(result);
        },
    )?;
    let accepted = receive
        .await
        .map_err(|_| "launch acceptance interrupted; inspect work before retrying".to_string())??;
    start_send
        .send(())
        .map_err(|_| "launch stopped before acknowledgement; inspect the child".to_string())?;
    Ok(accepted)
}

fn launch_stopping(app: &AppState) -> bool {
    app.gcx.shutdown_flag.load(Ordering::Acquire)
        || app
            .agent_executions
            .board_admission_closed
            .load(Ordering::Acquire)
}

async fn accept_prepared_launch(
    app: AppState,
    req: SpawnRequest,
    prepared: PreparedLaunch,
    launch_payload: refact_agents_core::storage::AcceptedLaunchPayload,
    launch_digest: String,
    project_root: PathBuf,
    launch_provenance: (Vec<refact_privacy::FileRecord>, bool),
    child_uuid: Uuid,
    child_chat_id: String,
    start_receive: oneshot::Receiver<()>,
) -> Result<SpawnHandle, String> {
    if launch_stopping(&app) {
        return Err("launch cancelled before acceptance".into());
    }
    validate_launch_parent(&app, &req.parent_chat_id, &prepared).await?;
    // Native startup effects are rolled back on every error and on future
    // cancellation; the consumed mandate binding is deliberately retained.
    let mut rollback = SupervisedStartupRollback {
        app: app.clone(),
        child: child_chat_id.clone(),
        worktree: None,
        agent_id: None,
        armed: req.native_supervision_guard.is_some(),
    };
    if let Some(guard) = &req.native_supervision_guard {
        guard.validate_request(&req)?;
    }
    let (record, abort_flag, cancel_notify) = app
        .agents
        .create_for_parent_run(
            CreateAgentRequest {
                parent_chat_id: req.parent_chat_id.clone(),
                parent_root_chat_id: req.parent_root_chat_id.clone(),
                parent_tool_call_id: req.parent_tool_call_id.clone(),
                kind: req.kind,
                config_name: req.config_name.clone(),
                title: req.title.clone(),
                prompt: req.prompt.clone(),
                target_files: req.target_files.clone(),
                model: req.model.clone(),
                model_type: req.model_type.clone(),
                goal_summary: req.goal.as_ref().map(|goal| goal.content.clone()),
                plan_present: req.plan.is_some(),
                worktree_id: None,
                worktree_branch: None,
            },
            prepared.parent_run.as_ref(),
        )
        .await?;
    let agent_id = record.agent_id.clone();
    if let Err(error) = crate::review::runs::record_accepted_launch(&app, &req, &record).await {
        let _ = app.agents.mark_failed(&agent_id, error.clone()).await;
        return Err(error);
    }

    let launch_intent = crate::coordination::launch_intents::LaunchIntentSlot::persist_for_agent(
        &app, &agent_id, 1,
    )
    .await;
    rollback.agent_id = Some(agent_id.clone());
    if let Some(contract) = req
        .deliverable
        .clone()
        .filter(|contract| !contract.is_empty())
    {
        if let Err(error) = app.agents.install_deliverable(&agent_id, contract).await {
            launch_intent.settle().await;
            app.agents.mark_failed(&agent_id, error.clone()).await?;
            return Err(error);
        }
    }
    if let Some(profile) = req.profile.as_ref() {
        if let Err(error) = app
            .agents
            .install_profile_name(&agent_id, profile.name.clone())
            .await
        {
            launch_intent.settle().await;
            app.agents.mark_failed(&agent_id, error.clone()).await?;
            return Err(error);
        }
    }
    if launch_stopping(&app) || abort_flag.load(Ordering::Acquire) {
        launch_intent.settle().await;
        app.agents
            .mark_cancelled(&agent_id, Some("launch cancelled during acceptance".into()))
            .await?;
        return Err("launch cancelled during acceptance".into());
    }
    app.agent_executions
        .remember_launch_provenance(&agent_id, launch_provenance);
    let project_identity = crate::coordination::roots::project_identity(&project_root);
    app.agent_executions.remember_scope(&agent_id, project_root);
    let root = req
        .parent_root_chat_id
        .clone()
        .unwrap_or_else(|| req.parent_chat_id.clone());
    let run = refact_work_core::RunKey::new(
        refact_work_core::WorkKey::background_agent(agent_id.clone()),
        1,
    );
    let scope = refact_work_core::WorkScope::new(
        refact_work_core::ProjectKey::new(project_identity),
        refact_work_core::CheckoutKey::new(root),
    );
    // Launch context, not content: a root chat's own child keeps that chat's
    // ordinary standing, a child of another agent is background exploration.
    let mut request =
        refact_work_core::AdmissionRequest::new(run, scope).with_depth(req.subchat_depth);
    if req.subchat_depth > 0 {
        request = request.with_priority(refact_work_core::WorkPriority::Speculative);
    }
    let ticket = match app.work_scheduler.reserve(request) {
        Ok(ticket) => ticket,
        Err(error) => {
            launch_intent.settle().await;
            app.agent_executions.forget_launch_provenance(&agent_id);
            let _ = app.agents.mark_failed(&agent_id, error.to_string()).await;
            return Err(error.to_string());
        }
    };
    if let Err(error) = app
        .agents
        .save_accepted_launch(&agent_id, &launch_payload)
        .await
    {
        launch_intent.settle().await;
        app.agent_executions.forget_launch_provenance(&agent_id);
        let _ = app.agents.mark_failed(&agent_id, error.clone()).await;
        return Err(error);
    }
    let setup = async {
        app.agents
            .set_completion_push(&agent_id, req.completion_push)
            .await?;
        app.agents.get_any(&agent_id).await
    }
    .await;
    let record = match setup {
        Ok(record) => record,
        Err(error) => {
            launch_intent.settle().await;
            app.agent_executions.forget_launch_provenance(&agent_id);
            let _ = app.agents.mark_failed(&agent_id, error.clone()).await;
            return Err(error);
        }
    };
    emit_background_agent_update(app.clone(), &record).await;
    let (completion_tx, completion_rx) = oneshot::channel();
    push_sibling_notice(
        &app,
        &agent_id,
        &req,
        format!(
            "▸ sibling queued: {} targeting {}",
            record.title,
            format_notice_files(&record.target_files)
        ),
    )
    .await;
    let handle = SpawnHandle {
        agent_id: agent_id.clone(),
        child_chat_id: child_chat_id.clone(),
        launch_revision: prepared.revision,
        launch_digest,
        worktree_branch: None,
        auto_merge: match req.worktree_mode {
            SpawnWorktreeMode::Isolated {
                auto_merge,
            } => Some(auto_merge),
            _ => None,
        },
        completion_rx,
    };
    let executions = app.agent_executions.clone();
    let tracked_chat_id = child_chat_id.clone();
    let rejected_app = app.clone();
    let rejected_agent = agent_id.clone();
    let task_intent = launch_intent.clone();
    let registration = executions.spawn_owned(
        tracked_chat_id,
        abort_flag.clone(),
        cancel_notify.clone(),
        async move {
            if !wait_for_acceptance(start_receive, &abort_flag, &cancel_notify).await {
                drop(ticket);
                app.agent_executions.forget_launch_provenance(&agent_id);
                if let Ok(record) = app
                    .agents
                    .mark_cancelled(
                        &agent_id,
                        Some("Launch caller left before acceptance".into()),
                    )
                    .await
                {
                    let _ = completion_tx.send(record);
                }
                task_intent.settle().await;
                app.agent_executions.forget_launch_intent(&child_chat_id);
                return;
            }
            let acquire = ticket.acquire();
            tokio::pin!(acquire);
            let admission = loop {
                let cancelled = cancel_notify.notified();
                tokio::pin!(cancelled);
                cancelled.as_mut().enable();
                if abort_flag.load(Ordering::Relaxed) || launch_stopping(&app) {
                    break Err(refact_work_core::AdmissionError::Cancelled);
                }
                tokio::select! {
                    result = &mut acquire => break result,
                    _ = &mut cancelled => {}
                }
            };
            app.agent_executions.forget_launch_provenance(&agent_id);
            let permit = match admission {
                Ok(permit) => permit,
                Err(error) => {
                    let failed = if abort_flag.load(Ordering::Relaxed) || launch_stopping(&app) {
                        app.agents
                            .mark_cancelled(&agent_id, Some(error.to_string()))
                            .await
                    } else {
                        app.agents.mark_failed(&agent_id, error.to_string()).await
                    }
                    .unwrap_or_else(|_| {
                        fallback_failed_record(agent_id.clone(), req.clone(), error.to_string())
                    });
                    let _ = completion_tx.send(failed);
                    task_intent.settle().await;
                    app.agent_executions.forget_launch_intent(&child_chat_id);
                    return;
                }
            };
            if abort_flag.load(Ordering::Relaxed) || launch_stopping(&app) {
                drop(permit);
                let cancelled = app
                    .agents
                    .mark_cancelled(&agent_id, Some("Cancelled while queued".into()))
                    .await;
                if let Ok(record) = cancelled {
                    let _ = completion_tx.send(record);
                }
                task_intent.settle().await;
                app.agent_executions.forget_launch_intent(&child_chat_id);
                return;
            }
            let execution = match app.agent_executions.install(&child_chat_id, permit) {
                Ok(execution) => execution,
                Err(error) => {
                    if let Ok(record) = app.agents.mark_cancelled(&agent_id, Some(error)).await {
                        let _ = completion_tx.send(record);
                    }
                    task_intent.settle().await;
                    app.agent_executions.forget_launch_intent(&child_chat_id);
                    return;
                }
            };
            let _registration = app.agent_executions.registration(&child_chat_id);
            app.agent_executions
                .remember_launch_intent(&child_chat_id, task_intent.clone());
            let child_chat_id_for_cleanup = child_chat_id.clone();
            task_intent.dispatched().await;
            let result = prepare_and_run(
                app.clone(),
                req,
                prepared,
                child_uuid,
                child_chat_id,
                agent_id,
                abort_flag,
            )
            .await;
            drop(execution);
            drop(_registration);
            task_intent.settle().await;
            app.agent_executions
                .forget_launch_intent(&child_chat_id_for_cleanup);
            let _ = completion_tx.send(result);
        },
    );
    if let Err(error) = registration {
        rejected_app
            .agent_executions
            .forget_launch_provenance(&rejected_agent);
        let _ = rejected_app
            .agents
            .mark_cancelled(&rejected_agent, Some(error.clone()))
            .await;
        launch_intent.settle().await;
        return Err(error);
    }
    rollback.armed = false;
    Ok(handle)
}

async fn wait_for_acceptance(
    receive: oneshot::Receiver<()>,
    cancel: &AtomicBool,
    notify: &tokio::sync::Notify,
) -> bool {
    tokio::pin!(receive);
    loop {
        let notified = notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if cancel.load(Ordering::Acquire) {
            return false;
        }
        tokio::select! {
            result = &mut receive => return result.is_ok() && !cancel.load(Ordering::Acquire),
            _ = notified => {}
        }
    }
}

async fn prepare_and_run(
    app: AppState,
    req: SpawnRequest,
    mut prepared: PreparedLaunch,
    child_uuid: Uuid,
    child_chat_id: String,
    agent_id: String,
    abort_flag: Arc<AtomicBool>,
) -> BackgroundAgent {
    if let Err(error) = revalidate_prepared_launch(&app, &req, &mut prepared).await {
        return app
            .agents
            .mark_failed(&agent_id, error)
            .await
            .unwrap_or_else(|error| fallback_failed_record(agent_id, req, error));
    }
    if abort_flag.load(Ordering::Relaxed) || launch_stopping(&app) {
        return app
            .agents
            .mark_cancelled(
                &agent_id,
                Some("Accepted launch cancelled before setup".into()),
            )
            .await
            .unwrap_or_else(|error| fallback_failed_record(agent_id, req, error));
    }
    let spawned_worktree = match create_spawn_worktree(
        app.clone(),
        &prepared.worktree_mode,
        prepared.config.worktree.as_ref(),
        &prepared.project_root,
        &child_chat_id,
        &child_uuid,
        req.native_supervision_guard.is_some(),
    )
    .await
    {
        Ok(worktree) => worktree,
        Err(error) => {
            return app
                .agents
                .mark_failed(&agent_id, error)
                .await
                .unwrap_or_else(|error| fallback_failed_record(agent_id, req, error));
        }
    };
    if abort_flag.load(Ordering::Acquire)
        || launch_stopping(&app)
        || validate_launch_parent(&app, &req.parent_chat_id, &prepared)
            .await
            .is_err()
    {
        cleanup_spawn_worktree(app.clone(), spawned_worktree.as_ref()).await;
        return app
            .agents
            .mark_cancelled(&agent_id, Some("Cancelled during setup".into()))
            .await
            .unwrap_or_else(|error| fallback_failed_record(agent_id, req, error));
    }
    if let Some(guard) = &req.native_supervision_guard {
        if let Err(error) = guard.bind_edit_scope(
            &child_chat_id,
            spawned_worktree
                .as_ref()
                .map(|worktree| worktree.meta.root.as_path()),
        ) {
            cleanup_spawn_worktree(app.clone(), spawned_worktree.as_ref()).await;
            return app
                .agents
                .mark_failed(&agent_id, error)
                .await
                .unwrap_or_else(|error| fallback_failed_record(agent_id, req, error));
        }
    }
    let effective_worktree = spawned_worktree
        .as_ref()
        .map(|w| w.meta.clone())
        .or_else(|| prepared.config.worktree.clone());
    if let Err(error) = app
        .agents
        .set_worktree_metadata(
            &agent_id,
            effective_worktree
                .as_ref()
                .map(|worktree| worktree.id.clone()),
            effective_worktree
                .as_ref()
                .and_then(|worktree| worktree.branch.clone()),
        )
        .await
    {
        cleanup_spawn_worktree(app.clone(), spawned_worktree.as_ref()).await;
        return fallback_failed_record(agent_id, req, error);
    }
    prepared.config.worktree = effective_worktree;
    run_spawned_agent(
        app,
        req,
        prepared.config,
        prepared.messages,
        agent_id,
        child_chat_id,
        abort_flag,
        spawned_worktree,
        prepared.parent_run,
    )
    .await
}

async fn prepare_launch(
    app: &AppState,
    req: &SpawnRequest,
    child_chat_id: &str,
    project_root: PathBuf,
) -> Result<
    (
        PreparedLaunch,
        refact_agents_core::storage::AcceptedLaunchPayload,
    ),
    String,
> {
    let project_root = dunce::canonicalize(project_root)
        .map_err(|_| "parent project scope is not available or authorized".to_string())?;
    let parent_state = launch_parent_state(app, &req.parent_chat_id).await?;
    prepare_launch_for_parent_run(app, req, child_chat_id, project_root, parent_state).await
}

async fn prepare_launch_for_parent_run(
    app: &AppState,
    req: &SpawnRequest,
    child_chat_id: &str,
    project_root: PathBuf,
    parent_state: (
        Option<refact_agents_core::registry::ParentRun>,
        Arc<AtomicBool>,
    ),
) -> Result<
    (
        PreparedLaunch,
        refact_agents_core::storage::AcceptedLaunchPayload,
    ),
    String,
> {
    let (parent_run, parent_abort) = parent_state;
    let limits = crate::runtime_settings::coordination_for_project(&project_root).await?;
    if req.initial_context_messages.len() > limits.launch_context_messages
        || crate::coordination::bounds::serialized_size(
            &(
                &req.initial_context_messages,
                &req.prompt,
                &req.custom_instructions,
                &req.plan,
                &req.goal,
                &req.target_files,
                &req.tools,
                &req.profile,
            ),
            limits.launch_context_bytes,
        )
        .is_none()
    {
        return Err("launch arguments exceed configured context bounds".into());
    }
    let max_steps = req
        .goal
        .as_ref()
        .and_then(|g| g.budget.as_ref())
        .and_then(|b| b.max_turns)
        .filter(|n| *n > 0)
        .map(|n| req.max_steps.min(n as usize))
        .unwrap_or(req.max_steps);
    let max_steps = req
        .profile
        .as_ref()
        .and_then(|profile| profile.max_steps)
        .map_or(max_steps, |cap| max_steps.min(cap));
    let resolve_config = async {
        resolve_subchat_config_with_parent(
            app.gcx.clone(),
            &req.config_name,
            true,
            Some(child_chat_id.to_string()),
            Some(req.title.clone()),
            Some(req.parent_chat_id.clone()),
            Some(req.kind.as_str().to_string()),
            req.parent_root_chat_id.clone(),
            req.tools.clone(),
            max_steps,
            false,
            None,
            "agent".to_string(),
            req.parent_task_meta.clone(),
            req.parent_worktree.clone(),
            req.parent_tool_call_id.clone(),
            req.parent_subchat_tx.clone(),
            None,
            req.subchat_depth + 1,
        )
        .await
    };
    #[cfg(test)]
    let config_result = if req.config_name == "test_spawn" {
        drop(resolve_config);
        Ok(SubchatConfig {
            runner_lease_token: None,
            native_job_fence: None,
            history_source_model: None,
            tool_name: req.config_name.clone(),
            stateful: true,
            autonomous_no_confirm: true,
            auto_approve_editing_tools: true,
            auto_approve_dangerous_commands: true,
            chat_id: Some(child_chat_id.to_string()),
            title: Some(req.title.clone()),
            parent_id: Some(req.parent_chat_id.clone()),
            link_type: Some(req.kind.as_str().into()),
            root_chat_id: req.parent_root_chat_id.clone(),
            tools: crate::subchat::ToolsPolicy::from_option(req.tools.clone()),
            max_steps,
            prepend_system_prompt: false,
            wrap_up: None,
            task_meta: req.parent_task_meta.clone(),
            worktree: req.parent_worktree.clone(),
            model: req.model.clone(),
            mode: "agent".into(),
            n_ctx: 4096,
            implicit_context_window: false,
            max_new_tokens: 512,
            temperature: None,
            reasoning_effort: None,
            cache_control: crate::llm::params::CacheControl::Ephemeral,
            parent_tool_call_id: req.parent_tool_call_id.clone(),
            parent_subchat_tx: req.parent_subchat_tx.clone(),
            abort_flag: None,
            soft_abort: false,
            activity_stamp: None,
            background_agent_id: None,
            subchat_depth: req.subchat_depth + 1,
            final_step_force_answer: false,
            buddy_meta: None,
            step_progress: None,
            trace_parent: crate::subchat::TraceParent::from_parts(
                Some(&req.parent_chat_id),
                req.parent_root_chat_id.as_deref(),
            ),
        })
    } else {
        resolve_config.await
    };
    #[cfg(not(test))]
    let config_result = resolve_config.await;
    let mut config = config_result?;
    config.model = req.model.clone();
    config.history_source_model = req.history_source_model.clone();
    let implicit_window = req.config_name == "subagent" && req.native_supervision_guard.is_none();
    if implicit_window || req.context_tokens_cap.is_some() {
        let caps = crate::global_context::try_load_caps_quickly_if_not_present(app.gcx.clone(), 0)
            .await
            .map_err(|_| "launch model unavailable")?;
        let model_n_ctx = crate::caps::resolve_chat_model(caps, &config.model)?
            .base
            .n_ctx;
        crate::subchat::apply_launch_context_window(
            &mut config,
            model_n_ctx,
            req.context_tokens_cap,
            implicit_window,
        )?;
    }
    let configured_consent = crate::yaml_configs::customization_registry::get_subagent_config(
        app.gcx.clone(),
        &req.config_name,
        None,
    )
    .await
    .map(|config| config.subchat.auto_approve_dangerous_commands != Some(false))
    .unwrap_or(false);
    #[cfg(test)]
    let configured_consent = configured_consent || req.config_name == "test_spawn";
    config.auto_approve_dangerous_commands = configured_consent
        && super::approval::dangerous_confirmation_enabled(app, &req.parent_chat_id).await;
    if let Some(profile) = &req.profile {
        if let Some(effort) = &profile.reasoning_effort {
            config.reasoning_effort = Some(effort.clone());
        }
        config.autonomous_no_confirm &= profile.approval.autonomous_no_confirm.unwrap_or(true);
        config.auto_approve_editing_tools &=
            profile.approval.auto_approve_editing_tools.unwrap_or(true);
        config.auto_approve_dangerous_commands &= profile
            .approval
            .auto_approve_dangerous_commands
            .unwrap_or(true);
    }
    narrow_parent_approval(app, req, &mut config).await;
    config.tools = super::tool_authority::freeze(
        app,
        &req.parent_chat_id,
        &config.model,
        &config.mode,
        &config.tools,
    )
    .await?;
    if let Some(guard) = &req.native_supervision_guard {
        guard.validate_request(req)?;
        config.autonomous_no_confirm = false;
        config.auto_approve_editing_tools = false;
        config.auto_approve_dangerous_commands = false;
        config.wrap_up = None;
        config.tools = crate::subchat::ToolsPolicy::Supervised {
            tools: req.tools.clone().unwrap_or_default(),
            budget: req
                .goal
                .as_ref()
                .and_then(|g| g.budget.clone())
                .ok_or("finite goal budget required")?,
        };
    }
    let mut messages = build_messages(
        app.clone(),
        &req.config_name,
        &req.prompt,
        req.plan.as_deref(),
        req.goal.as_ref(),
    )
    .await?;
    let parent = app
        .chat
        .sessions
        .read()
        .await
        .get(&req.parent_chat_id)
        .cloned();
    if let Some(parent) = parent {
        let parent = parent.lock().await;
        if let Some(source) = parent
            .goal
            .as_ref()
            .filter(|goal| matches!(goal.owner, Some(refact_chat_api::GoalOwner::Task { .. })))
        {
            for message in messages.iter_mut().filter(|message| message.role == "goal") {
                if let Some(meta) = message
                    .extra
                    .get_mut("goal")
                    .and_then(|value| value.as_object_mut())
                {
                    meta.insert(
                        "assigned_from".into(),
                        serde_json::json!({"owner":source.owner,"revision":source.revision}),
                    );
                }
            }
        }
    }
    let mut context = req.initial_context_messages.clone();
    if let Some(instructions) = req.profile.as_ref().and_then(|p| p.instructions.as_ref()) {
        context.push(ChatMessage::new("user".to_string(), instructions.clone()));
    }
    if let Some(instructions) = req.custom_instructions.as_ref() {
        context.push(ChatMessage::new(
            "user".to_string(),
            format!("Parent-provided child instructions:\n{instructions}"),
        ));
    }
    if !context.is_empty() {
        let index = usize::from(
            messages
                .first()
                .is_some_and(|message| message.role == "system"),
        );
        messages.splice(index..index, context);
    }
    let ordinary_launch = req.parent_tool_call_id.is_some()
        && req.config_name == "subagent"
        && req.native_supervision_guard.is_none();
    if max_steps == 0 || (!ordinary_launch && max_steps > limits.subagent_max_steps) {
        return Err("launch step bound exceeds Settings ceiling".into());
    }
    if messages.len() > limits.launch_context_messages
        || crate::coordination::bounds::serialized_size(&messages, limits.launch_context_bytes)
            .is_none()
    {
        return Err("prepared launch exceeds configured context bounds".into());
    }
    let prepared = PreparedLaunch {
        schema_version: 1,
        revision: 1,
        config,
        messages,
        project_root,
        worktree_mode: req.worktree_mode.clone(),
        profile: req.profile.clone(),
        parent_run,
        parent_abort: Some(parent_abort),
    };
    let payload = refact_agents_core::storage::AcceptedLaunchPayload::serialize(
        &prepared,
        limits.launch_context_bytes,
    )?;
    Ok((prepared, payload))
}

#[cfg(test)]
mod accepted_launch_tests {
    use super::*;

    #[tokio::test]
    #[serial_test::serial(test_runner)]
    async fn queued_launch_freezes_then_narrows_parent_authority_without_mutating_seed() {
        let (_gcx, app, parent, workspace) =
            super::super::tests::app_with_parent_session("narrow-parent").await;
        let mut limits = refact_work_core::CoordinationSettings::default();
        limits.max_active_engine = 1;
        limits.max_active_project = 1;
        limits.max_active_root = 1;
        app.work_scheduler.configure(limits).unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let blocker = app
            .work_scheduler
            .reserve(refact_work_core::AdmissionRequest::new(
                refact_work_core::WorkKey::background_agent("blocker").run(1),
                refact_work_core::WorkScope::new(
                    refact_work_core::ProjectKey::new(
                        crate::coordination::roots::project_identity(&root),
                    ),
                    refact_work_core::CheckoutKey::new("narrow-parent"),
                ),
            ))
            .unwrap()
            .acquire()
            .await
            .unwrap();
        parent.lock().await.active_command.allowed_tools = vec!["cat".into(), "shell".into()];
        parent.lock().await.thread.auto_approve_dangerous_commands = true;
        let captured = Arc::new(std::sync::Mutex::new(None));
        let capture = captured.clone();
        let _runner = install_test_runner(Arc::new(move |_gcx, messages, config| {
            *capture.lock().unwrap() =
                Some((config.tools.clone(), config.auto_approve_dangerous_commands));
            Box::pin(async move {
                Ok(SubchatResult {
                    messages,
                    metering: Default::default(),
                    chat_id: config.chat_id,
                    aborted: false,
                })
            })
        }));
        let mut req = super::super::tests::subagent_spawn_request("narrow-parent", "src/a.rs");
        req.tools = None;
        parent.lock().await.work_project_root = None;
        assert!(validate_launch_request(&app, &req).await.is_err());
        let foreign = tempfile::tempdir().unwrap();
        parent.lock().await.work_project_root = Some(foreign.path().into());
        assert!(validate_launch_request(&app, &req).await.is_err());
        let private_root = foreign.path().join("private-parent-root-not-present");
        assert!(private_root.is_absolute());
        assert!(!private_root.exists());
        parent.lock().await.work_project_root = Some(private_root.clone());
        let error = preview_launch(&app, &req)
            .await
            .err()
            .expect("invalid scope must deny preview");
        assert_eq!(error, "parent project scope is not available or authorized");
        assert!(!error.contains(private_root.to_str().unwrap()));
        parent.lock().await.work_project_root = Some(root.clone());
        let handle = spawn_background_agent(app.clone(), req).await.unwrap();
        assert!(captured.lock().unwrap().is_none());
        assert!(!app
            .chat
            .sessions
            .read()
            .await
            .contains_key(&handle.child_chat_id));
        let bytes = app
            .agents
            .accepted_launch("narrow-parent", &handle.agent_id, 1024 * 1024)
            .await
            .unwrap();
        let seed: PreparedLaunch = bytes.decode().unwrap();
        assert!(seed.config.auto_approve_dangerous_commands);
        assert!(seed.config.tools.allows_tool("shell"));
        assert!(!seed.config.tools.allows_tool("tasks_set"));
        let req = super::super::tests::subagent_spawn_request("narrow-parent", "src/a.rs");
        for source in [None, Some(foreign.path().to_path_buf())] {
            parent.lock().await.work_project_root = source;
            let mut queued: PreparedLaunch = bytes.decode().unwrap();
            assert!(revalidate_prepared_launch(&app, &req, &mut queued)
                .await
                .is_err());
        }
        parent.lock().await.work_project_root = Some(root);
        parent.lock().await.active_command.allowed_tools = vec!["cat".into(), "tasks_set".into()];
        parent.lock().await.thread.auto_approve_dangerous_commands = false;
        drop(blocker);
        let record = tokio::time::timeout(Duration::from_secs(5), handle.completion_rx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            record.status,
            crate::agents::types::BgAgentStatus::Completed
        );
        let (policy, dangerous) = captured.lock().unwrap().clone().unwrap();
        assert!(!dangerous);
        assert!(policy.allows_tool("cat"));
        assert!(!policy.allows_tool("shell"));
        assert!(!policy.allows_tool("tasks_set"));
        let unchanged = app
            .agents
            .accepted_launch("narrow-parent", &handle.agent_id, 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(bytes.as_bytes(), unchanged.as_bytes());
        let recovered = recover_run_config(&app, "narrow-parent", &handle.agent_id)
            .await
            .unwrap();
        assert!(!recovered.authority_migrated);
        assert!(!recovered.config.auto_approve_dangerous_commands);
        assert!(!recovered.config.tools.allows_tool("shell"));
        assert!(!recovered.config.tools.allows_tool("tasks_set"));
        parent.lock().await.work_project_root = None;
        assert!(recover_run_config(&app, "narrow-parent", &handle.agent_id)
            .await
            .is_err());
        parent.lock().await.work_project_root = Some(foreign.path().into());
        assert!(recover_run_config(&app, "narrow-parent", &handle.agent_id)
            .await
            .is_err());
        parent.lock().await.work_project_root = Some(private_root.clone());
        let error = recover_run_config(&app, "narrow-parent", &handle.agent_id)
            .await
            .err()
            .expect("invalid scope must deny recovery");
        assert_eq!(error, "parent project scope is not available or authorized");
        assert!(!error.contains(private_root.to_str().unwrap()));
        app.agent_executions.shutdown().await;
    }

    #[tokio::test]
    #[serial_test::serial(test_runner)]
    async fn shutdown_joins_unacknowledged_launch_without_dispatch() {
        let (_gcx, app, _parent, workspace) =
            super::super::tests::app_with_parent_session("unacked-parent").await;
        let req = super::super::tests::subagent_spawn_request("unacked-parent", "src/a.rs");
        let child_uuid = Uuid::new_v4();
        let child = format!("subchat-{child_uuid}");
        let root = workspace.path().canonicalize().unwrap();
        let (prepared, payload) = prepare_launch(&app, &req, &child, root.clone())
            .await
            .unwrap();
        let digest = launch_digest(&payload);
        let (send, receive) = oneshot::channel();
        let handle = accept_prepared_launch(
            app.clone(),
            req,
            prepared,
            payload,
            digest,
            root,
            (Vec::new(), true),
            child_uuid,
            child.clone(),
            receive,
        )
        .await
        .unwrap();
        assert!(app.agent_executions.task_active(&child));
        tokio::time::timeout(Duration::from_secs(5), app.agent_executions.shutdown())
            .await
            .unwrap();
        assert!(send.send(()).is_err());
        let record = handle.completion_rx.await.unwrap();
        assert_eq!(
            record.status,
            crate::agents::types::BgAgentStatus::Cancelled
        );
        assert!(!app.chat.sessions.read().await.contains_key(&child));
        assert!(record.worktree_id.is_none());
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
        assert!(!app.agent_executions.task_active(&child));
        let req = super::super::tests::subagent_spawn_request("unacked-parent", "src/a.rs");
        assert!(spawn_background_agent(app.clone(), req).await.is_err());
        assert_eq!(
            app.agents
                .list_for_parent("unacked-parent", Default::default())
                .await
                .len(),
            1
        );
    }

    #[tokio::test]
    #[serial_test::serial(test_runner)]
    async fn subtree_cancel_settles_unacknowledged_launch_and_rejects_prepared_descendant() {
        let (_gcx, app, _parent, workspace) =
            super::super::tests::app_with_parent_session("cancel-launcher").await;
        let owner = app
            .agents
            .create(CreateAgentRequest {
                parent_chat_id: "root-owner".into(),
                parent_root_chat_id: Some("root-owner".into()),
                parent_tool_call_id: Some("launch-parent".into()),
                kind: BgAgentKind::Subagent,
                config_name: "test_spawn".into(),
                title: "Parent".into(),
                prompt: "Parent".into(),
                target_files: vec!["src/a.rs".into()],
                model: "model".into(),
                model_type: None,
                goal_summary: None,
                plan_present: false,
                worktree_id: None,
                worktree_branch: None,
            })
            .await
            .unwrap()
            .0;
        let mut authority_request =
            super::super::tests::subagent_spawn_request("cancel-launcher", "src/a.rs");
        authority_request.tools = Some(vec!["cat".into()]);
        let authority = prepare_launch(
            &app,
            &authority_request,
            "authority-seed",
            workspace.path().canonicalize().unwrap(),
        )
        .await
        .unwrap()
        .0
        .config;
        app.agents
            .mark_running(&owner.agent_id, "cancel-launcher".into())
            .await
            .unwrap();
        app.agent_executions
            .remember_config(&owner.agent_id, authority);
        let mut req = super::super::tests::subagent_spawn_request("cancel-launcher", "src/a.rs");
        req.parent_root_chat_id = Some("root-owner".into());
        req.subchat_depth = 1;
        req.worktree_mode = SpawnWorktreeMode::Isolated {
            auto_merge: false,
        };
        let root = workspace.path().canonicalize().unwrap();
        let child_uuid = Uuid::new_v4();
        let child = format!("subchat-{child_uuid}");
        let (prepared, payload) = prepare_launch(&app, &req, &child, root.clone())
            .await
            .unwrap();
        assert!(prepared.parent_run.is_some());
        let late_uuid = Uuid::new_v4();
        let late_child = format!("subchat-{late_uuid}");
        let (late, late_payload) = prepare_launch(&app, &req, &late_child, root.clone())
            .await
            .unwrap();
        let dispatched = Arc::new(AtomicBool::new(false));
        let observed = dispatched.clone();
        let _runner = install_test_runner(Arc::new(move |_, _, _| {
            observed.store(true, Ordering::SeqCst);
            Box::pin(async { Err("cancelled child dispatched".into()) })
        }));
        let (send, receive) = oneshot::channel();
        let digest = launch_digest(&payload);
        let handle = accept_prepared_launch(
            app.clone(),
            req.clone(),
            prepared,
            payload,
            digest,
            root.clone(),
            (Vec::new(), true),
            child_uuid,
            child.clone(),
            receive,
        )
        .await
        .unwrap();
        assert!(!handle.agent_id.is_empty());
        assert_eq!(
            app.agents
                .get_any(&handle.agent_id)
                .await
                .unwrap()
                .run_generation,
            1
        );
        assert!(app.agent_executions.task_active(&child));
        assert_eq!(app.work_scheduler.snapshot().outstanding, 1);
        let cancelled = app
            .agents
            .cancel_subtree("root-owner", &owner.agent_id, true, None)
            .await
            .unwrap();
        assert_eq!(cancelled.len(), 2);
        let _ = send.send(());
        let record = tokio::time::timeout(Duration::from_secs(5), handle.completion_rx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            record.status,
            crate::agents::types::BgAgentStatus::Cancelled
        );
        let (_, late_receive) = oneshot::channel();
        let late_digest = launch_digest(&late_payload);
        let rejected = accept_prepared_launch(
            app.clone(),
            req,
            late,
            late_payload,
            late_digest,
            root,
            (Vec::new(), true),
            late_uuid,
            late_child.clone(),
            late_receive,
        )
        .await;
        assert!(rejected.is_err());
        app.agent_executions.shutdown().await;
        assert!(!dispatched.load(Ordering::SeqCst));
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
        assert!(!app.agent_executions.task_active(&child));
        assert!(!app.chat.sessions.read().await.contains_key(&child));
        assert!(!app.chat.sessions.read().await.contains_key(&late_child));
        assert!(record.worktree_id.is_none());
        assert!(app
            .agent_executions
            .launch_provenance(&record.agent_id)
            .is_none());
        assert_eq!(
            app.agents
                .list_for_parent("cancel-launcher", Default::default())
                .await
                .len(),
            1
        );
    }

    #[tokio::test]
    #[serial_test::serial(test_runner)]
    async fn public_launch_shutdown_cancels_then_joins_running_cleanup() {
        let (_gcx, app, _parent, _workspace) =
            super::super::tests::app_with_parent_session("shutdown-parent").await;
        let (started, ready) = oneshot::channel();
        let (release, wait) = oneshot::channel();
        let channels = Arc::new(std::sync::Mutex::new(Some((started, wait))));
        let _runner = install_test_runner(Arc::new(move |_gcx, messages, config| {
            let (started, wait) = channels.lock().unwrap().take().unwrap();
            Box::pin(async move {
                let cancel = config.abort_flag.as_ref().unwrap().clone();
                started.send(cancel.clone()).unwrap();
                wait.await.unwrap();
                assert!(cancel.load(Ordering::Acquire));
                Ok(SubchatResult {
                    messages,
                    metering: Default::default(),
                    chat_id: config.chat_id,
                    aborted: true,
                })
            })
        }));
        let req = super::super::tests::subagent_spawn_request("shutdown-parent", "src/a.rs");
        let handle = spawn_background_agent(app.clone(), req).await.unwrap();
        let cancel = tokio::time::timeout(Duration::from_secs(5), ready)
            .await
            .unwrap()
            .unwrap();
        let mut shutdown = Box::pin(app.agent_executions.shutdown());
        use std::future::Future;
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(shutdown.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        assert!(cancel.load(Ordering::Acquire));
        assert_eq!(app.work_scheduler.snapshot().active, 1);
        drop(shutdown);
        assert!(app.agent_executions.task_active(&handle.child_chat_id));
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), app.agent_executions.shutdown())
            .await
            .unwrap();
        let record = handle.completion_rx.await.unwrap();
        assert_eq!(
            record.status,
            crate::agents::types::BgAgentStatus::Cancelled
        );
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
    }

    #[tokio::test]
    #[serial_test::serial(test_runner)]
    async fn abandoned_acceptance_releases_ticket_without_creating_child() {
        let (_gcx, app, _parent, workspace) =
            super::super::tests::app_with_parent_session("abandoned-parent").await;
        let req = super::super::tests::subagent_spawn_request("abandoned-parent", "src/a.rs");
        let child_uuid = Uuid::new_v4();
        let child = format!("subchat-{child_uuid}");
        let root = workspace.path().canonicalize().unwrap();
        let (prepared, payload) = prepare_launch(&app, &req, &child, root.clone())
            .await
            .unwrap();
        let digest = launch_digest(&payload);
        let (send, receive) = oneshot::channel();
        drop(send);
        let handle = accept_prepared_launch(
            app.clone(),
            req,
            prepared,
            payload,
            digest,
            root,
            (Vec::new(), true),
            child_uuid,
            child.clone(),
            receive,
        )
        .await
        .unwrap();
        let record = tokio::time::timeout(Duration::from_secs(5), handle.completion_rx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            record.status,
            crate::agents::types::BgAgentStatus::Cancelled
        );
        assert!(!app.chat.sessions.read().await.contains_key(&child));
        assert!(record.worktree_id.is_none());
        assert!(app
            .agent_executions
            .launch_provenance(&record.agent_id)
            .is_none());
        assert_eq!(app.work_scheduler.snapshot().active, 0);
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
    }

    #[tokio::test]
    #[serial_test::serial(test_runner)]
    async fn prepared_payload_obeys_project_limits_before_record_or_ticket() {
        let (_gcx, app, _parent, workspace) =
            super::super::tests::app_with_parent_session("bounded-parent").await;
        let limits_path = crate::runtime_settings::project_coordination_path(workspace.path());
        std::fs::create_dir_all(limits_path.parent().unwrap()).unwrap();
        std::fs::write(&limits_path, "launch_context_bytes: 1024\n").unwrap();
        let mut req = super::super::tests::subagent_spawn_request("bounded-parent", "src/a.rs");
        req.prompt = "large".repeat(1024);
        assert!(spawn_background_agent(app.clone(), req).await.is_err());
        assert!(app
            .agents
            .list_for_parent("bounded-parent", Default::default())
            .await
            .is_empty());
        assert_eq!(app.work_scheduler.snapshot().outstanding, 0);
        std::fs::write(&limits_path, "launch_context_bytes: 32768\n").unwrap();
        let req = super::super::tests::subagent_spawn_request("bounded-parent", "src/a.rs");
        let (_, payload) = prepare_launch(
            &app,
            &req,
            "subchat-bounded",
            workspace.path().canonicalize().unwrap(),
        )
        .await
        .unwrap();
        assert!(payload.as_bytes().len() <= 32768);
        assert!(payload.decode::<PreparedLaunch>().is_ok());
    }

    fn resolved_profile(revision: u64, max_steps: usize) -> super::super::profile::ResolvedProfile {
        super::super::profile::ResolvedProfile {
            schema_version: super::super::profile::PROFILE_SCHEMA_VERSION,
            name: "reviewer".into(),
            revision,
            model: None,
            model_type: None,
            tools: None,
            target_files: Vec::new(),
            max_steps: Some(max_steps),
            reasoning_effort: Some(refact_core::chat_types::ReasoningEffort::High),
            approval: super::super::profile::ProfileApproval {
                autonomous_no_confirm: Some(false),
                auto_approve_editing_tools: Some(false),
                auto_approve_dangerous_commands: Some(false),
            },
            budget: None,
            instructions: Some("# Agent Profile: reviewer\n## Role\nCareful reviewer".into()),
            provenance: Default::default(),
        }
    }

    #[tokio::test]
    async fn accepted_bytes_pin_the_profile_revision_and_omit_it_entirely_without_one() {
        let (_gcx, app, parent, workspace) =
            super::super::tests::app_with_parent_session("profile-parent").await;
        parent.lock().await.thread.autonomous_no_confirm = true;
        parent.lock().await.thread.auto_approve_editing_tools = true;
        parent.lock().await.thread.auto_approve_dangerous_commands = true;
        let root = workspace.path().canonicalize().unwrap();
        let mut req = super::super::tests::subagent_spawn_request("profile-parent", "src/a.rs");
        req.max_steps = 9;

        let (plain, plain_payload) = prepare_launch(&app, &req, "subchat-plain", root.clone())
            .await
            .unwrap();
        let plain_value: Value = serde_json::from_slice(plain_payload.as_bytes()).unwrap();
        assert!(plain_value.get("profile").is_none());
        assert_eq!(plain.config.max_steps, 9);
        assert!(plain.config.autonomous_no_confirm);
        assert!(plain.config.auto_approve_dangerous_commands);
        assert!(plain.config.reasoning_effort.is_none());
        assert!(plain.profile.is_none());
        assert!(!plain.messages.iter().any(|message| message
            .content
            .content_text_only()
            .contains("Agent Profile")));

        let mut accepted_req = req.clone();
        accepted_req.profile = Some(resolved_profile(7, 3));
        let (prepared, accepted) =
            prepare_launch(&app, &accepted_req, "subchat-profile", root.clone())
                .await
                .unwrap();
        assert_eq!(prepared.config.max_steps, 3);
        assert!(!prepared.config.autonomous_no_confirm);
        assert!(!prepared.config.auto_approve_editing_tools);
        assert!(!prepared.config.auto_approve_dangerous_commands);
        assert_eq!(
            prepared.config.reasoning_effort,
            Some(refact_core::chat_types::ReasoningEffort::High)
        );
        assert!(prepared.messages.iter().any(|message| message
            .content
            .content_text_only()
            .contains("Careful reviewer")));
        let accepted_value: Value = serde_json::from_slice(accepted.as_bytes()).unwrap();
        assert_eq!(accepted_value["profile"]["revision"], 7);
        assert_eq!(accepted_value["profile"]["name"], "reviewer");
        let accepted_digest = launch_digest(&accepted);

        let mut edited_req = req.clone();
        edited_req.profile = Some(resolved_profile(8, 3));
        let (_, edited) = prepare_launch(&app, &edited_req, "subchat-profile", root)
            .await
            .unwrap();
        assert_ne!(edited.as_bytes(), accepted.as_bytes());
        assert_ne!(launch_digest(&edited), accepted_digest);
        assert_eq!(
            serde_json::from_slice::<Value>(accepted.as_bytes()).unwrap()["profile"]["revision"],
            7
        );
        assert_eq!(launch_digest(&accepted), accepted_digest);
    }
}

async fn narrow_parent_approval(app: &AppState, req: &SpawnRequest, config: &mut SubchatConfig) {
    let parent = app
        .chat
        .sessions
        .read()
        .await
        .get(&req.parent_chat_id)
        .cloned();
    if let Some(parent) = parent {
        let parent = parent.lock().await;
        config.autonomous_no_confirm &= parent.thread.autonomous_no_confirm;
        config.auto_approve_editing_tools &=
            parent.thread.auto_approve_editing_tools || parent.thread.autonomous_no_confirm;
        drop(parent);
        config.auto_approve_dangerous_commands &=
            super::approval::dangerous_confirmation_enabled(app, &req.parent_chat_id).await;
    } else {
        config.autonomous_no_confirm = false;
        config.auto_approve_editing_tools = false;
        config.auto_approve_dangerous_commands = false;
    }
}

async fn launch_parent_state(
    app: &AppState,
    chat_id: &str,
) -> Result<
    (
        Option<refact_agents_core::registry::ParentRun>,
        Arc<AtomicBool>,
    ),
    String,
> {
    let parent_run = app.agents.parent_run(chat_id).await?;
    let parent = app
        .chat
        .sessions
        .read()
        .await
        .get(chat_id)
        .cloned()
        .ok_or("parent session unavailable")?;
    let parent = parent.lock().await;
    if parent.closed || parent.abort_flag.load(Ordering::Acquire) {
        return Err("parent launch was cancelled or closed".into());
    }
    Ok((parent_run, parent.abort_flag.clone()))
}

async fn validate_launch_parent(
    app: &AppState,
    chat_id: &str,
    prepared: &PreparedLaunch,
) -> Result<(), String> {
    if prepared
        .parent_abort
        .as_ref()
        .is_none_or(|flag| flag.load(Ordering::Acquire))
    {
        return Err("parent launch was cancelled".into());
    }
    app.agents
        .validate_parent_run(chat_id, prepared.parent_run.as_ref())
        .await
}

async fn revalidate_prepared_launch(
    app: &AppState,
    req: &SpawnRequest,
    prepared: &mut PreparedLaunch,
) -> Result<(), String> {
    validate_launch_parent(app, &req.parent_chat_id, prepared).await?;
    if prepared
        .config
        .worktree
        .as_ref()
        .is_some_and(|worktree| !worktree.root.is_dir())
    {
        return Err("accepted parent checkout is unavailable".into());
    }
    let parent = app
        .chat
        .sessions
        .read()
        .await
        .get(&req.parent_chat_id)
        .cloned()
        .ok_or("parent session unavailable")?;
    let pinned_review = crate::review::runs::is_pinned_launch(
        app,
        &prepared.project_root,
        &req.parent_chat_id,
        prepared.config.worktree.as_ref(),
    )
    .await?;
    {
        let parent = parent.lock().await;
        if parent.closed || parent.abort_flag.load(Ordering::Acquire) {
            return Err("parent launch was cancelled or closed".into());
        }
        if crate::coordination::scope::session_source_root(&parent)
            .and_then(|root| dunce::canonicalize(root).ok())
            .as_ref()
            != Some(&prepared.project_root)
            || (!pinned_review
                && parent.thread.worktree.as_ref().map(|w| (&w.id, &w.root))
                    != prepared.config.worktree.as_ref().map(|w| (&w.id, &w.root)))
        {
            return Err("accepted parent scope changed while queued".into());
        }
    }
    trusted_project_root(app, Some(prepared.project_root.clone())).await?;
    if prepared.config.worktree.is_none() {
        validate_unscoped_project(app, &prepared.project_root).await?;
    }
    narrow_parent_approval(app, req, &mut prepared.config).await;
    let ceiling = super::tool_authority::current_ceiling(
        app,
        &req.parent_chat_id,
        &prepared.config.model,
        &prepared.config.mode,
    )
    .await?;
    prepared.config.tools = super::tool_authority::intersect(&prepared.config.tools, &ceiling);
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(app.gcx.clone(), 0)
        .await
        .map_err(|_| "accepted launch model unavailable")?;
    let model = crate::caps::resolve_chat_model(caps, &prepared.config.model)
        .map_err(|_| "accepted launch model unavailable")?;
    super::launch_spec::clear_current_context(&app.gcx, &prepared.messages, &model.base)?;
    Ok(())
}

pub async fn spawn_and_wait(
    app: AppState,
    req: SpawnRequest,
    timeout: Option<Duration>,
) -> Result<BackgroundAgent, String> {
    let handle = spawn_background_agent(app, req).await?;
    match timeout {
        Some(timeout) => tokio::time::timeout(timeout, handle.completion_rx)
            .await
            .map_err(|_| "background agent timed out".to_string())?
            .map_err(|_| "background agent task ended without a record".to_string()),
        None => handle
            .completion_rx
            .await
            .map_err(|_| "background agent task ended without a record".to_string()),
    }
}

pub(crate) fn stable_child_chat_id(config: &SubchatConfig, child_chat_id: &str) -> String {
    config
        .chat_id
        .as_deref()
        .map(str::trim)
        .filter(|chat_id| !chat_id.is_empty())
        .unwrap_or(child_chat_id)
        .to_string()
}

pub async fn resume_capped_agent(
    app: AppState,
    parent_chat_id: &str,
    agent_id: &str,
    max_steps: usize,
    expected_change_seq: u64,
) -> Result<BackgroundAgent, String> {
    resume_agent(
        app,
        parent_chat_id,
        agent_id,
        Some(max_steps),
        expected_change_seq,
        None,
        false,
    )
    .await
}

pub async fn resume_agent(
    app: AppState,
    parent_chat_id: &str,
    agent_id: &str,
    raised_max_steps: Option<usize>,
    expected_change_seq: u64,
    instruction: Option<String>,
    reconcile_incomplete: bool,
) -> Result<BackgroundAgent, String> {
    let parent_state = launch_parent_state(&app, parent_chat_id).await?;
    let intent = app.agent_executions.reserve_resume(agent_id)?;
    let parent_chat_id = parent_chat_id.to_string();
    let agent_id = agent_id.to_string();
    let executions = app.agent_executions.clone();
    let task_id = format!("resume:{agent_id}");
    let (send, receive) = oneshot::channel();
    let (start_send, start_receive) = oneshot::channel();
    executions.spawn_owned(
        task_id,
        Arc::new(AtomicBool::new(false)),
        Arc::new(tokio::sync::Notify::new()),
        async move {
            let _intent = intent;
            let result = resume_agent_for_parent_run(
                app,
                &parent_chat_id,
                &agent_id,
                raised_max_steps,
                expected_change_seq,
                instruction,
                reconcile_incomplete,
                start_receive,
                parent_state,
            )
            .await;
            let _ = send.send(result);
        },
    )?;
    let record = receive.await.map_err(|_| {
        "resume preparation interrupted; inspect the same child before retrying".to_string()
    })??;
    start_send
        .send(())
        .map_err(|_| "resume stopped before acknowledgement; inspect the child".to_string())?;
    Ok(record)
}

#[cfg(test)]
pub(super) async fn resume_agent_inner(
    app: AppState,
    parent_chat_id: &str,
    agent_id: &str,
    raised_max_steps: Option<usize>,
    expected_change_seq: u64,
    instruction: Option<String>,
    reconcile_incomplete: bool,
    start_receive: oneshot::Receiver<()>,
) -> Result<BackgroundAgent, String> {
    let parent_state = launch_parent_state(&app, parent_chat_id).await?;
    resume_agent_for_parent_run(
        app,
        parent_chat_id,
        agent_id,
        raised_max_steps,
        expected_change_seq,
        instruction,
        reconcile_incomplete,
        start_receive,
        parent_state,
    )
    .await
}

async fn resume_agent_for_parent_run(
    app: AppState,
    parent_chat_id: &str,
    agent_id: &str,
    raised_max_steps: Option<usize>,
    expected_change_seq: u64,
    instruction: Option<String>,
    reconcile_incomplete: bool,
    start_receive: oneshot::Receiver<()>,
    parent_state: (
        Option<refact_agents_core::registry::ParentRun>,
        Arc<AtomicBool>,
    ),
) -> Result<BackgroundAgent, String> {
    let (parent_run, parent_abort) = parent_state;
    use refact_agents_core::types::CapStop;
    if launch_stopping(&app) || parent_abort.load(Ordering::Acquire) {
        return Err("resume cancelled before preparation".into());
    }
    let old = app.agents.get_any(agent_id).await?;
    if old.parent_chat_id != parent_chat_id || old.change_seq != expected_change_seq {
        return Err("agent owner or stopped revision does not match".into());
    }
    let cap = old.error.as_deref().and_then(CapStop::from_error);
    if !old.status.is_terminal() {
        return Err("child has not stopped".into());
    }
    if raised_max_steps.is_some() && !cap.as_ref().is_some_and(|cap| cap.parent_can_raise_steps()) {
        return Err("only a resumable child max_steps cap can be raised".into());
    }
    if cap
        .as_ref()
        .is_some_and(|cap| !cap.parent_can_raise_steps())
    {
        return Err("the stopped cap requires separate user authorization".into());
    }
    let RecoveredRun {
        mut config,
        root: accepted_root,
        authority_migrated,
        mut isolated_worktree,
        replace_worktree,
        previous_attachment,
        pending_replacement,
    } = recover_run_config(&app, parent_chat_id, agent_id).await?;
    ensure_replacement_not_started(&accepted_root, pending_replacement.as_ref()).await?;
    let limits = crate::runtime_settings::coordination_for_project(&accepted_root).await?;
    if instruction
        .as_ref()
        .is_some_and(|text| text.len() > limits.launch_context_bytes)
    {
        return Err("resume instruction exceeds configured context size".into());
    }
    let child_chat_id = old
        .child_chat_id
        .clone()
        .ok_or("child trajectory unavailable")?;
    if !config.stateful
        || config.chat_id.as_deref() != Some(child_chat_id.as_str())
        || config.background_agent_id.as_deref() != Some(agent_id)
        || config.parent_id.as_deref() != Some(parent_chat_id)
        || config.root_chat_id != old.parent_root_chat_id
        || config.subchat_depth == 0
        || config.subchat_depth >= limits.max_subchat_depth
    {
        return Err("saved child configuration does not match the stopped run".into());
    }
    if app.agent_executions.get(&child_chat_id).is_some()
        || app.agent_executions.task_active(&child_chat_id)
    {
        return Err(
            "previous child execution is still active; retry after it releases admission".into(),
        );
    }
    let used_steps = cap.as_ref().map_or(old.step_count as usize, |cap| {
        cap.used.max(old.step_count as usize)
    });
    let max_steps = raised_max_steps.unwrap_or(config.max_steps);
    let unlimited = config.ordinary_background_has_unlimited_steps();
    if !unlimited
        && (max_steps <= used_steps
            || raised_max_steps.is_some_and(|steps| steps <= config.max_steps)
            || max_steps > limits.subagent_max_steps)
    {
        return Err("max_steps must exceed lifetime usage and stay within the Settings subagent step ceiling; inspect the cap before raising it".into());
    }
    let mut live_session = app.chat.sessions.read().await.get(&child_chat_id).cloned();
    let mut live_revision = if let Some(session) = &live_session {
        let revision = resume_session_revision(&*session.lock().await)?;
        crate::chat::trajectories::flush_trajectory_for_session(app.clone(), session.clone())
            .await?;
        Some(revision)
    } else {
        None
    };
    let mut trajectory =
        crate::chat::trajectories::load_trajectory_for_recovery(app.gcx.clone(), &child_chat_id)
            .await
            .ok_or_else(|| "saved child trajectory is unavailable".to_string())?;
    validate_recovery_trajectory(
        &old,
        &config,
        replace_worktree,
        previous_attachment.as_ref(),
        &trajectory,
    )?;
    config.autonomous_no_confirm &= trajectory.thread.autonomous_no_confirm;
    config.auto_approve_editing_tools &= trajectory.thread.auto_approve_editing_tools;
    config.auto_approve_dangerous_commands &= trajectory.thread.auto_approve_dangerous_commands;
    let trajectory_revision = resume_trajectory_revision(&trajectory)?;
    let mut messages = trajectory.messages.clone();
    reconcile_resume_tool_pairs(&mut messages, reconcile_incomplete)?;
    let remaining = remaining_goal_steps(
        &trajectory,
        used_steps,
        if unlimited {
            config.max_steps
        } else {
            max_steps - used_steps
        },
        unlimited,
    )?;
    let project_root = config
        .worktree
        .as_ref()
        .map(|w| w.source_workspace_root.clone())
        .or_else(|| Some(accepted_root.clone()))
        .ok_or("original child project scope unavailable; refusing to resume in a possibly different project")?;
    let project_root = dunce::canonicalize(project_root)
        .map_err(|error| format!("original child project unavailable: {error}"))?;
    let parent = app
        .chat
        .sessions
        .read()
        .await
        .get(parent_chat_id)
        .cloned()
        .ok_or("parent session is unavailable")?;
    let parent = parent.lock().await.thread.clone();
    config.autonomous_no_confirm &= parent.autonomous_no_confirm;
    config.auto_approve_editing_tools &=
        parent.auto_approve_editing_tools || parent.autonomous_no_confirm;
    config.auto_approve_dangerous_commands &=
        super::approval::dangerous_confirmation_enabled(&app, parent_chat_id).await;
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(app.gcx.clone(), 0)
        .await
        .map_err(|_| {
            "saved child model is unavailable; resume cannot silently change it".to_string()
        })?;
    let model = crate::caps::resolve_chat_model(caps, &config.model).map_err(|_| {
        "saved child model is unavailable; resume cannot silently change it".to_string()
    })?;
    super::launch_spec::clear_current_context(&app.gcx, &messages, &model.base)?;
    let generation = old
        .run_generation
        .checked_add(1)
        .ok_or("attempt generation overflow")?;
    let run = refact_work_core::WorkKey::background_agent(agent_id).run(generation);
    let scope = refact_work_core::WorkScope::new(
        refact_work_core::ProjectKey::new(crate::coordination::roots::project_identity(
            &project_root,
        )),
        refact_work_core::CheckoutKey::new(
            old.parent_root_chat_id
                .clone()
                .unwrap_or_else(|| parent_chat_id.to_string()),
        ),
    );
    if app.agent_executions.get(&child_chat_id).is_some()
        || app.agent_executions.task_active(&child_chat_id)
    {
        return Err("previous child execution is still active".into());
    }
    if launch_stopping(&app) {
        return Err("resume cancelled before reopening".into());
    }
    // A resume keeps the class its launch context assigned.
    let launcher_depth = config.subchat_depth - 1;
    let mut request =
        refact_work_core::AdmissionRequest::new(run, scope).with_depth(launcher_depth);
    if launcher_depth > 0 {
        request = request.with_priority(refact_work_core::WorkPriority::Speculative);
    }
    let replacement = if replace_worktree {
        let (previous_generation, previous_digest) = if let Some(pending) = pending_replacement {
            (pending.previous_generation, pending.previous_digest)
        } else {
            let previous = app
                .agents
                .run_config(
                    parent_chat_id,
                    agent_id,
                    old.run_generation,
                    limits.launch_context_bytes,
                )
                .await?;
            (old.run_generation, launch_digest(&previous))
        };
        Some(ReplacementIntent {
            operation: Uuid::new_v4(),
            previous_generation,
            previous_digest,
            attachment: RecoveryAttachment {
                worktree: trajectory.thread.worktree.clone(),
                trajectory_revision: trajectory_revision.clone(),
            },
        })
    } else {
        None
    };
    let ticket = app
        .work_scheduler
        .reserve(request)
        .map_err(|error| error.to_string())?;
    let (record, abort, notify) = app
        .agents
        .reopen_for_parent_run(
            agent_id,
            parent_chat_id,
            expected_change_seq,
            raised_max_steps.is_some(),
            parent_run.as_ref(),
        )
        .await?;
    emit_background_agent_update(app.clone(), &record).await;
    let launch_intent = crate::coordination::launch_intents::LaunchIntentSlot::persist_for_agent(
        &app, agent_id, generation,
    )
    .await;
    app.agent_executions.remember_scope(agent_id, accepted_root);
    let continuation_start = messages.len();
    let (notice_source, notice_text) = continuation_notice(old.status);
    messages.push(event(
        EventSubkind::SystemNotice,
        notice_source,
        serde_json::json!({
            "agent_id": agent_id,
            "run_generation": generation,
            "steps_before": used_steps,
            "previous_status": old.status.as_str(),
        }),
        notice_text,
    ));
    if authority_migrated {
        messages.push(event(
            EventSubkind::SystemNotice,
            crate::agents::push::NOTICE_AUTHORITY_MIGRATED,
            serde_json::json!({"agent_id": agent_id, "run_generation": generation, "tools": config.tools}),
            "This child was accepted before concrete tool authority existed. Its open tool policy was replaced by the parent's current concrete ceiling for this resume; the accepted launch bytes are unchanged.",
        ));
    }
    if let Some(instruction) = instruction {
        messages.push(ChatMessage::new("user".into(), instruction));
    }
    config.max_steps = max_steps;
    config.abort_flag = Some(abort.clone());
    config.step_progress = None;
    if let Err(error) = persist_resume_config(
        &app,
        &record,
        &config,
        replacement.clone(),
        previous_attachment,
    )
    .await
    {
        launch_intent.settle().await;
        let rollback = app
            .agents
            .rollback_unaccepted_resume(&old, &record, &abort)
            .await;
        if !matches!(rollback, Ok(true)) {
            finish_resume_error(&app, agent_id, generation, &abort, error.clone(), false).await;
        }
        return Err(format!("{error}; resume not accepted; inspect before retrying (metadata rollback: {rollback:?})"));
    }
    let attempt = match app
        .agents
        .begin_checkout_setup(
            parent_chat_id,
            agent_id,
            generation,
            record.change_seq,
            &abort,
            parent_run.as_ref(),
            replacement
                .as_ref()
                .map_or_else(Uuid::new_v4, |intent| intent.operation),
        )
        .await
    {
        Ok(attempt) => attempt,
        Err(error) => {
            launch_intent.settle().await;
            finish_resume_error(&app, agent_id, generation, &abort, error.clone(), true).await;
            return Err(error);
        }
    };
    if let Err(error) = app
        .agents
        .with_checkout_setup(&attempt, |_| {
            app.agent_executions
                .remember_config(agent_id, config.clone())
        })
        .await
    {
        launch_intent.settle().await;
        finish_resume_error(&app, agent_id, generation, &abort, error.clone(), true).await;
        return Err(error);
    }
    config.max_steps = remaining;
    config.prepend_system_prompt = false;
    let agent_id = agent_id.to_string();
    let executions = app.agent_executions.clone();
    let task_chat = child_chat_id.clone();
    let parent_chat_id = parent_chat_id.to_string();
    let rejected_app = app.clone();
    let rejected_agent = agent_id.clone();
    let rejected_abort = abort.clone();
    let task_intent = launch_intent.clone();
    let mut response = record.clone();
    if replace_worktree {
        response.worktree_id = None;
        response.worktree_branch = None;
    }
    let registration =
        executions.spawn_owned(task_chat, abort.clone(), notify.clone(), async move {
            if !wait_for_acceptance(start_receive, &abort, &notify).await {
                task_intent.settle().await;
                app.agent_executions.forget_launch_intent(&child_chat_id);
                finish_resume_error(
                    &app,
                    &agent_id,
                    generation,
                    &abort,
                    "Resume caller left before acceptance".into(),
                    true,
                )
                .await;
                return;
            }
            let acquire = ticket.acquire();
            tokio::pin!(acquire);
            let admission = loop {
                let notification = notify.notified();
                tokio::pin!(notification);
                notification.as_mut().enable();
                if abort.load(Ordering::Relaxed) || launch_stopping(&app) {
                    break Err(refact_work_core::AdmissionError::Cancelled);
                }
                tokio::select! {
                    result = &mut acquire => break result,
                    _ = &mut notification => {}
                }
            };
            let permit = match admission {
                Ok(permit) => permit,
                Err(error) => {
                    task_intent.settle().await;
                    app.agent_executions.forget_launch_intent(&child_chat_id);
                    finish_resume_error(
                        &app,
                        &agent_id,
                        generation,
                        &abort,
                        error.to_string(),
                        abort.load(Ordering::Relaxed)
                            || matches!(error, refact_work_core::AdmissionError::Cancelled),
                    )
                    .await;
                    return;
                }
            };
            if abort.load(Ordering::Relaxed)
                || launch_stopping(&app)
                || parent_abort.load(Ordering::Acquire)
                || app
                    .agents
                    .validate_parent_run(&parent_chat_id, parent_run.as_ref())
                    .await
                    .is_err()
            {
                task_intent.settle().await;
                app.agent_executions.forget_launch_intent(&child_chat_id);
                finish_resume_error(
                    &app,
                    &agent_id,
                    generation,
                    &abort,
                    "Cancelled while queued".into(),
                    true,
                )
                .await;
                return;
            }
            let execution = match app.agent_executions.install(&child_chat_id, permit) {
                Ok(execution) => execution,
                Err(error) => {
                    task_intent.settle().await;
                    app.agent_executions.forget_launch_intent(&child_chat_id);
                    finish_resume_error(&app, &agent_id, generation, &abort, error, true).await;
                    return;
                }
            };
            let _registration = app.agent_executions.registration(&child_chat_id);
            app.agent_executions
                .remember_launch_intent(&child_chat_id, task_intent.clone());
            task_intent.dispatched().await;
            let mut runner_guard = None;
            let validation = async {
                let current_config = recover_run_config(&app, &parent_chat_id, &agent_id)
                    .await?
                    .config;
                config.tools =
                    super::tool_authority::intersect(&config.tools, &current_config.tools);
                config.autonomous_no_confirm &= current_config.autonomous_no_confirm;
                config.auto_approve_editing_tools &= current_config.auto_approve_editing_tools;
                config.auto_approve_dangerous_commands &=
                    current_config.auto_approve_dangerous_commands;
                let caps =
                    crate::global_context::try_load_caps_quickly_if_not_present(app.gcx.clone(), 0)
                        .await
                        .map_err(|_| "saved child model unavailable")?;
                let model = crate::caps::resolve_chat_model(caps, &config.model)
                    .map_err(|_| "saved child model unavailable")?;
                let deliveries = super::delivery::drain_authorized(&app, &agent_id, true).await?;
                let mut followup_messages = Vec::new();
                for delivery in &deliveries {
                    if !messages.iter().any(|message| {
                        refact_core::chat_types::delivery_id_of_message(message)
                            == Some(delivery.id.as_str())
                    }) {
                        followup_messages.extend(delivery.stamp_messages());
                    }
                }
                messages.splice(continuation_start..continuation_start, followup_messages);
                super::launch_spec::clear_current_context(&app.gcx, &messages, &model.base)?;
                let current = crate::chat::trajectories::load_trajectory_for_recovery(
                    app.gcx.clone(),
                    &child_chat_id,
                )
                .await
                .ok_or("saved child trajectory unavailable")?;
                if resume_trajectory_revision(&current)? != trajectory_revision {
                    let mut changed = Vec::new();
                    if recovery_thread_revision(&current.thread)?
                        != recovery_thread_revision(&trajectory.thread)?
                    {
                        changed.push("thread");
                    }
                    if recovery_messages_revision(&current.messages)?
                        != recovery_messages_revision(&trajectory.messages)?
                    {
                        changed.push("messages");
                    }
                    if recovery_digest(&current.goal)? != recovery_digest(&trajectory.goal)? {
                        changed.push("goal");
                    }
                    if recovery_digest(&current.goal_ledger)?
                        != recovery_digest(&trajectory.goal_ledger)?
                    {
                        changed.push("ledger");
                    }
                    if recovery_digest(&current.pending_deliveries)?
                        != recovery_digest(&trajectory.pending_deliveries)?
                    {
                        changed.push("deliveries");
                    }
                    if current.updated_at != trajectory.updated_at {
                        changed.push("timestamp");
                    }
                    return Err(format!(
                        "child trajectory changed while resume was queued ({})",
                        changed.join(", ")
                    ));
                }
                config.max_steps = config.max_steps.min(remaining_goal_steps(
                    &current,
                    used_steps,
                    if unlimited { config.max_steps } else { max_steps - used_steps },
                    unlimited,
                )?);
                if replacement.is_some() {
                    let setup = &attempt;
                    app.agents.with_checkout_setup(setup, |_| ()).await?;
                    let mut fresh_worktree = None;
                    let mut retain_checkout = false;
                    let prepared = async {
                        let transition = live_session
                            .as_ref()
                            .map(crate::chat::worktree_lifecycle::try_transition_lock)
                            .transpose()?;
                        if let Some(session) = live_session.as_ref() {
                            let sessions = app
                                .chat
                                .sessions
                                .try_read()
                                .map_err(|_| "Child registry busy; retry continuation")?;
                            if !sessions
                                .get(&child_chat_id)
                                .is_some_and(|current| Arc::ptr_eq(current, session))
                            {
                                return Err(
                                    "Child session changed before checkout preparation".to_string()
                                );
                            }
                            let guarded = session
                                .try_lock()
                                .map_err(|_| "Child session busy; retry continuation")?;
                            if Some(resume_session_revision(&guarded)?.as_str())
                                != live_revision.as_deref()
                            {
                                return Err(
                                    "Child session changed before checkout preparation".to_string()
                                );
                            }
                        }
                        let previous = isolated_worktree.as_ref().ok_or(
                            "original merge target unavailable; spawn a new agent instead",
                        )?;
                        let (current_parent, current_abort) =
                            launch_parent_state(&app, &parent_chat_id).await?;
                        app.agents
                            .validate_parent_run(&parent_chat_id, parent_run.as_ref())
                            .await?;
                        if !Arc::ptr_eq(&current_abort, &parent_abort)
                            || current_abort.load(Ordering::Acquire)
                            || abort.load(Ordering::Acquire)
                            || launch_stopping(&app)
                        {
                            return Err("Cancelled before checkout preparation".into());
                        }
                        app.agents
                            .validate_parent_run(&parent_chat_id, current_parent.as_ref())
                            .await?;
                        let fresh =
                            create_continued_worktree(&app, previous, &child_chat_id, setup.operation()).await?;
                        fresh_worktree = Some(fresh.clone());
                        if abort.load(Ordering::Acquire)
                            || parent_abort.load(Ordering::Acquire)
                            || launch_stopping(&app)
                        {
                            return Err("Cancelled during checkout preparation".into());
                        }
                        let current = recover_run_config(&app, &parent_chat_id, &agent_id).await?;
                        config.tools =
                            super::tool_authority::intersect(&config.tools, &current.config.tools);
                        config.autonomous_no_confirm &= current.config.autonomous_no_confirm;
                        config.auto_approve_editing_tools &=
                            current.config.auto_approve_editing_tools;
                        config.auto_approve_dangerous_commands &=
                            current.config.auto_approve_dangerous_commands;
                        super::launch_spec::clear_current_context(
                            &app.gcx,
                            &messages,
                            &model.base,
                        )?;
                        let current = crate::chat::trajectories::load_trajectory_for_recovery(
                            app.gcx.clone(),
                            &child_chat_id,
                        )
                        .await
                        .ok_or("saved child trajectory unavailable")?;
                        if resume_trajectory_revision(&current)? != trajectory_revision {
                            return Err(
                                "child trajectory changed while preparing continuation".into()
                            );
                        }
                        let seed = app
                            .agents
                            .run_config(
                                &parent_chat_id,
                                &agent_id,
                                generation,
                                limits.launch_context_bytes,
                            )
                            .await?;
                        let applied =
                            refact_agents_core::storage::AcceptedLaunchPayload::serialize(
                                &AppliedCheckout {
                                    schema_version: 1,
                                    operation: setup.operation(),
                                    seed_digest: launch_digest(&seed),
                                    worktree: fresh.meta.clone(),
                                },
                                limits.launch_context_bytes,
                            )?;
                        let mut saved_config = config.clone();
                        saved_config.worktree = Some(fresh.meta.clone());
                        saved_config.max_steps = max_steps;
                        match app
                            .agents
                            .commit_checkout_setup(
                                setup,
                                fresh.meta.id.clone(),
                                fresh.meta.branch.clone(),
                                &applied,
                                |_| {
                                    app.agent_executions
                                        .remember_config(&agent_id, saved_config)
                                },
                            )
                            .await
                        {
                            Ok(_) => retain_checkout = true,
                            Err(error) => {
                                retain_checkout = error.retain_checkout;
                                return Err(error.message);
                            }
                        }
                        config.worktree = Some(fresh.meta.clone());
                        app.agents
                            .validate_parent_run(&parent_chat_id, parent_run.as_ref())
                            .await?;
                        if abort.load(Ordering::Acquire)
                            || parent_abort.load(Ordering::Acquire)
                            || launch_stopping(&app)
                        {
                            return Err(
                                "Cancelled after checkout publication; retain for recovery".into(),
                            );
                        }
                        let current = recover_run_config(&app, &parent_chat_id, &agent_id).await?;
                        config.tools =
                            super::tool_authority::intersect(&config.tools, &current.config.tools);
                        config.autonomous_no_confirm &= current.config.autonomous_no_confirm;
                        config.auto_approve_editing_tools &=
                            current.config.auto_approve_editing_tools;
                        config.auto_approve_dangerous_commands &=
                            current.config.auto_approve_dangerous_commands;
                        super::launch_spec::clear_current_context(
                            &app.gcx,
                            &messages,
                            &model.base,
                        )?;
                        install_resumed_session(
                            &app,
                            &trajectory,
                            &trajectory.messages,
                            &config,
                            &project_root,
                            live_session.as_ref(),
                            live_revision.as_deref(),
                            false,
                            transition.as_ref(),
                        )
                        .await?;
                        live_session = app.chat.sessions.read().await.get(&child_chat_id).cloned();
                        let session = live_session.as_ref().ok_or("child session unavailable")?;
                        crate::chat::trajectories::flush_trajectory_for_session(
                            app.clone(),
                            session.clone(),
                        )
                        .await?;
                        live_revision = Some(resume_session_revision(&*session.lock().await)?);
                        trajectory = crate::chat::trajectories::load_trajectory_for_recovery(
                            app.gcx.clone(),
                            &child_chat_id,
                        )
                        .await
                        .ok_or("saved child trajectory unavailable")?;
                        isolated_worktree = Some(fresh);
                        Ok::<_, String>(())
                    }
                    .await;
                    if let Err(error) = prepared {
                        if !retain_checkout {
                            if let Some(fresh) = fresh_worktree.as_ref() {
                                let cleanup = async {
                                    app.agents.with_checkout_setup(&attempt, |_| ()).await?;
                                    if parent_abort.load(Ordering::Acquire) || launch_stopping(&app) {
                                        return Err("checkout cleanup authority changed; retained for inspection".to_string());
                                    }
                                    let service = WorktreeService::new_async(
                                        app.gcx.cache_dir.clone(),
                                        fresh.source_workspace_root.clone(),
                                    )
                                    .await?;
                                    let service = crate::worktrees::service::fenced_service(
                                        service, app.runtime.tool_jobs.scopes.clone(),
                                    );
                                    let observed = service.get_worktree(&fresh.meta.id).await?;
                                    if observed.meta != fresh.meta
                                        || observed.status.dirty
                                        || observed.status.conflicted
                                        || observed.status.error.is_some()
                                        || !observed.status.is_git_worktree
                                        || observed.status.head_commit != fresh.meta.base_commit
                                        || observed.references.iter().any(|reference| {
                                            reference.chat_id.as_deref()
                                                != Some(child_chat_id.as_str())
                                                || reference.agent_id.is_some()
                                                || reference.task_id.is_some()
                                                || reference.card_id.is_some()
                                        })
                                    {
                                        return Err(
                                            "unpublished checkout changed; retained for inspection"
                                                .to_string(),
                                        );
                                    }
                                    for reference in observed.references {
                                        service
                                            .remove_reference(&fresh.meta.id, &reference)
                                            .await?;
                                    }
                                    let archive =
                                        service.preview_worktree_archive(&fresh.meta.id).await?;
                                    if archive.record.meta != fresh.meta
                                        || Some(&archive.head) != fresh.meta.base_commit.as_ref()
                                    {
                                        return Err(
                                            "unpublished checkout changed; retained for inspection"
                                                .to_string(),
                                        );
                                    }
                                    service.archive_worktree_checked(archive).await
                                }
                                .await;
                                if let Err(cleanup) = cleanup {
                                    return Err(format!("{error}; {cleanup}"));
                                }
                            }
                        }
                        return Err(error);
                    }
                }
                if replacement.is_none() {
                    app.agents.with_checkout_setup(&attempt, |_| ()).await?;
                }
                if abort.load(Ordering::Acquire)
                    || parent_abort.load(Ordering::Acquire)
                    || launch_stopping(&app)
                {
                    return Err("Cancelled before resumed session start".into());
                }
                config.runner_lease_token = Some(Uuid::new_v4().simple().to_string());
                config.abort_flag = Some(abort.clone());
                runner_guard = install_resumed_session(
                    &app,
                    &trajectory,
                    &messages,
                    &config,
                    &project_root,
                    live_session.as_ref(),
                    live_revision.as_deref(),
                    true,
                    None,
                )
                .await?;
                let session = app
                    .chat
                    .sessions
                    .read()
                    .await
                    .get(&child_chat_id)
                    .cloned()
                    .ok_or("child session unavailable")?;
                crate::chat::trajectories::flush_trajectory_for_session(app.clone(), session)
                    .await?;
                for delivery in deliveries {
                    app.agents.consume_delivery(&agent_id, &delivery.id).await?;
                }
                super::delivery::publish_runner_queue(&app, &agent_id).await;
                Ok::<_, String>(())
            }
            .await;
            if let Err(error) = validation {
                task_intent.settle().await;
                app.agent_executions.forget_launch_intent(&child_chat_id);
                finish_resume_error(&app, &agent_id, generation, &abort, error, false).await;
                return;
            }
            let progress_task = install_agent_progress(&app, &agent_id, &mut config, used_steps);
            let worktree = config.worktree.clone();
            let run_config = config.clone();
            let baseline = collect_worktree_dirty_files(worktree.as_ref()).await;
            let result = match app
                .agents
                .mark_running(&agent_id, child_chat_id.clone())
                .await
            {
                Ok(running)
                    if !running.status.is_terminal()
                        && !abort.load(Ordering::Relaxed)
                        && !launch_stopping(&app) =>
                {
                    if let Some(intent) = app.agent_executions.launch_intent(&child_chat_id) {
                        intent.acknowledged().await;
                    }
                    emit_subagent_hook(&app, HookEvent::SubagentStart, &running, true).await;
                    emit_background_agent_update(app.clone(), &running).await;
                    if abort.load(Ordering::Acquire) || launch_stopping(&app) {
                        Err("Aborted before resumed execution".into())
                    } else {
                        run_background_subchat_with_lease(
                            app.gcx.clone(), messages, config, runner_guard.take(),
                        ).await
                    }
                }
                Ok(_) => Err("Aborted before resumed execution".into()),
                Err(error) => Err(error),
            };
            progress_task.finish().await;
            let (mut edited, diff, conflicts) =
                collect_workspace_changes(worktree.as_ref(), &baseline, &old.target_files).await;
            edited.extend(old.edited_files.clone());
            edited.sort();
            edited.dedup();
            let terminal = match result {
                Ok(result) if result.aborted || abort.load(Ordering::Relaxed) => {
                    app.agents
                        .mark_cancelled(&agent_id, Some("Aborted".into()))
                        .await
                }
                Ok(mut result) => {
                    let mut result_summary = final_assistant_text(&result.messages);
                    let deliverable_status = deliverable_status_with_corrections(
                        &app,
                        &agent_id,
                        &run_config,
                        &mut result,
                        &mut result_summary,
                    )
                    .await;
                    let handoff = handoff_report_for(&app, &agent_id, &result_summary).await;
                    app.agents
                        .mark_completed(
                            &agent_id,
                            AgentCompletion {
                                result_summary,
                                edited_files: edited,
                                diff_summary: diff.or(old.diff_summary),
                                conflict_summary: conflicts.or(old.conflict_summary),
                                child_chat_id: Some(child_chat_id.clone()),
                                deliverable_status,
                                handoff,
                            },
                        )
                        .await
                }
                Err(error) if error.starts_with("Aborted") || abort.load(Ordering::Relaxed) => {
                    app.agents.mark_cancelled(&agent_id, Some(error)).await
                }
                Err(error) => app.agents.mark_failed(&agent_id, error).await,
            };
            task_intent.settle().await;
            app.agent_executions.forget_launch_intent(&child_chat_id);
            if let Ok(mut record) = terminal {
                if record.status == crate::agents::types::BgAgentStatus::Completed {
                    record = finalize_spawn_worktree(
                        app.clone(),
                        &agent_id,
                        &old.title,
                        isolated_worktree,
                        parent_run.clone(),
                    )
                    .await
                    .unwrap_or(record);
                }
                emit_background_agent_update(app.clone(), &record).await;
                let _ = crate::agents::push::push_completion_to_parent(app.clone(), &record).await;
            }
            drop(execution);
        });
    if let Err(error) = registration {
        launch_intent.settle().await;
        finish_resume_error(
            &rejected_app,
            &rejected_agent,
            generation,
            &rejected_abort,
            error.clone(),
            true,
        )
        .await;
        return Err(error);
    }
    Ok(response)
}

/// The notice a resumed child sees, derived from the state it actually stopped
/// in so a caller cannot describe a continued turn as a plain recovery.
pub(crate) fn continuation_notice(
    previous: crate::agents::types::BgAgentStatus,
) -> (&'static str, &'static str) {
    use crate::agents::types::BgAgentStatus;
    match previous {
        BgAgentStatus::Completed => (
            crate::agents::push::NOTICE_CONTINUED_TURN,
            "The parent asked this already-completed child for another turn. The earlier completion stands and is preserved in the record; nothing about it is rewritten. Preserved tool results are historical and must not be replayed.",
        ),
        BgAgentStatus::Closed => (
            crate::agents::push::NOTICE_RESUME,
            "The parent reopened this child after deliberately closing it. Its history, goal and plan are unchanged; preserved tool results are historical, and incomplete tool outcomes must be inspected, not replayed automatically.",
        ),
        _ => (
            crate::agents::push::NOTICE_RESUME,
            "The parent explicitly resumed this same child. Preserved tool results are historical; incomplete tool outcomes must be inspected, not replayed automatically.",
        ),
    }
}

fn recovery_digest(value: &impl serde::Serialize) -> Result<String, String> {
    use sha2::Digest;
    struct HashWriter(sha2::Sha256);
    impl std::io::Write for HashWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = HashWriter(sha2::Sha256::new());
    let mut value =
        serde_json::to_value(value).map_err(|_| "child recovery revision unavailable")?;
    value.sort_all_objects();
    serde_json::to_writer(&mut writer, &value)
        .map_err(|_| "child recovery revision unavailable")?;
    Ok(hex::encode(writer.0.finalize()))
}

fn recovery_messages_revision(messages: &[ChatMessage]) -> Result<String, String> {
    use sha2::Digest;
    let mut digest = sha2::Sha256::new();
    for message in messages {
        digest.update(recovery_digest(message)?.as_bytes());
    }
    Ok(hex::encode(digest.finalize()))
}

fn recovery_thread_revision(thread: &crate::chat::types::ThreadParams) -> Result<String, String> {
    let mut thread = thread.clone();
    if let Some(prefix) = thread.frozen_request_prefix.as_mut() {
        if prefix.tools_canonical.is_none() {
            prefix.created_at.clear();
        }
    }
    recovery_digest(&thread)
}

fn resume_trajectory_revision(
    trajectory: &crate::chat::trajectories::LoadedTrajectory,
) -> Result<String, String> {
    recovery_digest(&(
        recovery_thread_revision(&trajectory.thread)?,
        recovery_messages_revision(&trajectory.messages)?,
        &trajectory.goal,
        &trajectory.goal_ledger,
        &trajectory.pending_deliveries,
        &trajectory.updated_at,
    ))
}

pub(super) fn resume_session_revision(
    session: &crate::chat::types::ChatSession,
) -> Result<String, String> {
    if session.closed
        || session.runner_lease.is_some()
        || session.native_commands_in_flight.load(Ordering::SeqCst) > 0
        || session.turn_depth > 0
        || session.is_compressing
        || session.draft_message.is_some()
        || !session.command_queue.is_empty()
    {
        return Err("child session is busy or closed".into());
    }
    recovery_digest(&(
        recovery_thread_revision(&session.thread)?,
        recovery_messages_revision(&session.messages)?,
        &session.goal,
        &session.goal_ledger,
        session.trajectory_version,
        &session.work_project_root,
    ))
}

pub(super) async fn install_resumed_session(
    app: &AppState,
    trajectory: &crate::chat::trajectories::LoadedTrajectory,
    messages: &[ChatMessage],
    config: &SubchatConfig,
    source: &Path,
    expected_session: Option<&Arc<AMutex<crate::chat::types::ChatSession>>>,
    expected_revision: Option<&str>,
    start: bool,
    held_transition: Option<&crate::chat::worktree_lifecycle::AttachmentTransitionGuard>,
) -> Result<Option<RunnerLeaseGuard>, String> {
    let chat_id = &trajectory.thread.id;
    let mut runner_guard = None;
    if let Some(expected) = expected_session {
        let acquired;
        let _transition = match held_transition {
            Some(transition) if transition.is_for(expected) => transition,
            Some(_) => return Err("Resume attachment guard does not match child session".into()),
            None => {
                acquired = crate::chat::worktree_lifecycle::try_transition_lock(expected)?;
                &acquired
            }
        };
        let (saved_source, requests, aborted, interrupted) = {
            let session = expected.lock().await;
            (
                crate::coordination::scope::session_source_root(&session),
                session.recent_request_ids.clone(),
                session.abort_flag.load(Ordering::Acquire),
                session.user_interrupt_flag.load(Ordering::Acquire),
            )
        };
        if saved_source
            .as_ref()
            .is_some_and(|saved| dunce::canonicalize(saved).ok().as_deref() != Some(source))
        {
            return Err("saved child session does not match accepted project scope".into());
        }
        let sessions = app
            .chat
            .sessions
            .try_read()
            .map_err(|_| "child session registry busy; retry resume")?;
        if !sessions
            .get(chat_id)
            .is_some_and(|session| Arc::ptr_eq(expected, session))
        {
            return Err("child session changed while resume was queued".into());
        }
        let mut session = expected
            .try_lock()
            .map_err(|_| "child session busy; retry resume")?;
        if Some(resume_session_revision(&session)?.as_str()) != expected_revision {
            return Err("child session was edited while resume was queued".into());
        }
        if crate::coordination::scope::session_source_root(&session) != saved_source
            || session.recent_request_ids != requests
            || session.abort_flag.load(Ordering::Acquire) != aborted
            || session.user_interrupt_flag.load(Ordering::Acquire) != interrupted
        {
            return Err("child session scope changed or resume was cancelled".into());
        }
        if start {
            runner_guard = Some(RunnerLeaseGuard::claim(expected, &mut session, config)?);
        }
        session.work_project_root = Some(source.to_path_buf());
        session.thread.autonomous_no_confirm &= config.autonomous_no_confirm;
        session.thread.auto_approve_editing_tools &= config.auto_approve_editing_tools;
        session.thread.auto_approve_dangerous_commands &= config.auto_approve_dangerous_commands;
        session.thread.worktree = config.worktree.clone();
        if start {
            session.replace_messages(messages.to_vec());
            session.refresh_runner_history()?;
            session.set_runtime_state(crate::chat::types::SessionState::Generating, None);
        } else {
            session.increment_version();
        }
    } else {
        let mut thread = trajectory.thread.clone();
        thread.autonomous_no_confirm &= config.autonomous_no_confirm;
        thread.auto_approve_editing_tools &= config.auto_approve_editing_tools;
        thread.auto_approve_dangerous_commands &= config.auto_approve_dangerous_commands;
        thread.worktree = config.worktree.clone();
        let mut session = crate::chat::types::ChatSession::new_with_trajectory(
            chat_id.clone(),
            messages.to_vec(),
            thread,
            trajectory.created_at.clone(),
            trajectory.wake_up_at,
            trajectory.waiting_for_card_ids.clone(),
            trajectory.goal.clone(),
        );
        session.work_project_root = Some(source.to_path_buf());
        session.set_goal_ledger(trajectory.goal_ledger.clone());
        session.goal_verification_blocked_until_ms = trajectory.goal_verification_blocked_until_ms;
        session.pending_deliveries = trajectory.pending_deliveries.clone().into();
        if !start {
            session.increment_version();
        }
        let session = Arc::new(AMutex::new(session));
        let _transition = crate::chat::worktree_lifecycle::try_transition_lock(&session)?;
        let mut sessions = app.chat.sessions.write().await;
        if sessions.contains_key(chat_id) {
            return Err("child session appeared while resume was queued".into());
        }
        if start {
            let mut guarded = session.lock().await;
            runner_guard = Some(RunnerLeaseGuard::claim(&session, &mut guarded, config)?);
            guarded.set_runtime_state(crate::chat::types::SessionState::Generating, None);
        }
        sessions.insert(chat_id.clone(), session);
    }
    Ok(runner_guard)
}

fn reconcile_resume_tool_pairs(
    messages: &mut Vec<ChatMessage>,
    reconcile: bool,
) -> Result<(), String> {
    let mut pending = std::collections::BTreeSet::new();
    let mut seen = std::collections::HashSet::new();
    for message in messages.iter() {
        if !pending.is_empty() && matches!(message.role.as_str(), "assistant" | "user") {
            return Err("saved child has an interrupted tool batch before later messages; inspect the trajectory before resuming".into());
        }
        if let Some(calls) = &message.tool_calls {
            for call in calls {
                if call.id.is_empty() || !seen.insert(call.id.clone()) {
                    return Err("saved child has duplicate tool calls".into());
                }
                pending.insert(call.id.clone());
            }
        }
        if matches!(
            message.role.as_str(),
            "tool" | "context_file" | "diff" | "plain_text"
        ) && !message.tool_call_id.is_empty()
            && !message.is_runtime_context_attachment()
        {
            if !pending.remove(&message.tool_call_id) && !seen.contains(&message.tool_call_id) {
                return Err("saved child has an unmatched tool result".into());
            }
        }
    }
    if !pending.is_empty() && !reconcile {
        return Err("saved child has incomplete tool results; inspect side effects and explicitly set reconcile_incomplete to mark their outcomes unknown without replay".into());
    }
    for id in pending {
        let mut result = ChatMessage::new(
            "tool".into(),
            "The previous attempt stopped without a recorded result for this tool call. Its outcome is UNKNOWN, not success or failure. The tool was NOT replayed. Inspect filesystem/process/provider state before deciding whether a new operation is safe.".into(),
        );
        result.tool_call_id = id;
        result.tool_failed = Some(true);
        result.extra.insert(
            "agent_recovery".into(),
            serde_json::json!({"outcome": "unknown", "replayed": false}),
        );
        messages.push(result);
    }
    Ok(())
}

#[cfg(test)]
fn validate_resume_tool_pairs(messages: &[ChatMessage]) -> Result<(), String> {
    let mut pending = std::collections::HashSet::new();
    let mut seen = std::collections::HashSet::new();
    for message in messages {
        if !pending.is_empty() && matches!(message.role.as_str(), "assistant" | "user") {
            return Err("saved child has an interrupted tool result batch".into());
        }
        if let Some(calls) = &message.tool_calls {
            if !pending.is_empty() {
                return Err("saved child has incomplete tool results".into());
            }
            for call in calls {
                if call.id.is_empty() || !seen.insert(call.id.clone()) {
                    return Err("saved child has duplicate tool calls".into());
                }
                pending.insert(call.id.clone());
            }
        }
        if message.role == "tool" && !pending.remove(&message.tool_call_id) {
            return Err("saved child has an unmatched tool result".into());
        }
    }
    if pending.is_empty() {
        Ok(())
    } else {
        Err("saved child has incomplete tool results".into())
    }
}

async fn finish_resume_error(
    app: &AppState,
    agent_id: &str,
    generation: u64,
    abort: &Arc<AtomicBool>,
    error: String,
    cancelled: bool,
) {
    let terminal = app
        .agents
        .finish_queued_resume(agent_id, generation, abort, error, cancelled)
        .await;
    if let Ok(record) = terminal {
        emit_background_agent_update(app.clone(), &record).await;
        let _ = crate::agents::push::push_completion_to_parent(app.clone(), &record).await;
    }
}

struct AgentProgressTask {
    stop: Option<oneshot::Sender<()>>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl AgentProgressTask {
    async fn finish(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
    }
}

impl Drop for AgentProgressTask {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

fn install_agent_progress(
    app: &AppState,
    agent_id: &str,
    config: &mut SubchatConfig,
    steps_before: usize,
) -> AgentProgressTask {
    let app = app.clone();
    let agent_id = agent_id.to_string();
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let (stop, mut stopped) = oneshot::channel();
    config.step_progress = Some(Arc::new(move |progress| {
        let _ = sender.send(progress);
    }));
    let handle = tokio::spawn(async move {
        let mut finishing = false;
        loop {
            let progress = tokio::select! {
                biased;
                progress = receiver.recv() => match progress { Some(progress) => progress, None => break },
                _ = &mut stopped, if !finishing => { finishing = true; receiver.close(); match receiver.recv().await { Some(progress) => progress, None => break } }
            };
            let updated = match progress {
                SubchatProgress::Step(step) => {
                    let step = steps_before.saturating_add(step);
                    app.agents
                        .update_activity(
                            &agent_id,
                            Some(format!("step {}", step)),
                            Some(step as u32),
                            None,
                        )
                        .await
                }
                SubchatProgress::ToolStarted {
                    name,
                    arg_preview,
                } => {
                    app.agents
                        .update_activity(
                            &agent_id,
                            None,
                            None,
                            Some(Some(match arg_preview {
                                Some(preview) => format!("{}: {}", name, preview),
                                None => name,
                            })),
                        )
                        .await
                }
                SubchatProgress::ToolsFinished => {
                    app.agents
                        .update_activity(&agent_id, None, None, Some(None))
                        .await
                }
                SubchatProgress::Usage {
                    tokens_delta,
                    cost_delta,
                } => {
                    app.agents
                        .add_usage(&agent_id, tokens_delta, cost_delta)
                        .await
                }
            };
            if let Ok(record) = updated {
                emit_background_agent_update(app.clone(), &record).await;
            }
        }
    });
    AgentProgressTask {
        stop: Some(stop),
        handle: Some(handle),
    }
}

async fn run_spawned_agent(
    app: AppState,
    req: SpawnRequest,
    mut config: crate::subchat::SubchatConfig,
    messages: Vec<ChatMessage>,
    agent_id: String,
    child_chat_id: String,
    abort_flag: Arc<AtomicBool>,
    spawned_worktree: Option<SpawnedWorktree>,
    parent_run: Option<refact_agents_core::registry::ParentRun>,
) -> BackgroundAgent {
    config.abort_flag = Some(abort_flag.clone());
    config.background_agent_id = Some(agent_id.clone());
    config.chat_id = Some(stable_child_chat_id(&config, &child_chat_id));
    config.final_step_force_answer = req.native_supervision_guard.is_none();
    // Install the opaque in-process capability before remember_config copies it.
    // Serde skips it: persisted/recovered configuration never restores authority.
    let native_owner = req.native_supervision_guard.as_ref().map(|guard| {
        let (fence, owner) = crate::buddy::supervision_authority::NativeJobFence::new_owner(
            guard.clone(),
            child_chat_id.clone(),
        );
        config.native_job_fence = Some(fence);
        owner
    });
    if let Err(error) = persist_run_config(&app, &agent_id, &config).await {
        cleanup_spawn_worktree(app.clone(), spawned_worktree.as_ref()).await;
        return app
            .agents
            .mark_failed(&agent_id, error)
            .await
            .unwrap_or_else(|error| fallback_failed_record(agent_id, req, error));
    }
    app.agent_executions
        .remember_config(&agent_id, config.clone());
    let progress_task = install_agent_progress(&app, &agent_id, &mut config, 0);
    let running = app
        .agents
        .mark_running(&agent_id, child_chat_id.clone())
        .await;
    match running {
        Ok(record)
            if !record.status.is_terminal()
                && !abort_flag.load(Ordering::Relaxed)
                && !launch_stopping(&app) =>
        {
            if let Some(intent) = app.agent_executions.launch_intent(&child_chat_id) {
                intent.acknowledged().await;
            }
            emit_subagent_hook(&app, HookEvent::SubagentStart, &record, false).await;
            crate::agents::push::mark_sub_goal_dispatched(&app, &record).await;
            emit_background_agent_update(app.clone(), &record).await;
        }
        Ok(record) if record.status.is_terminal() => {
            progress_task.finish().await;
            cleanup_spawn_worktree(app.clone(), spawned_worktree.as_ref()).await;
            return record;
        }
        result => {
            progress_task.finish().await;
            cleanup_spawn_worktree(app.clone(), spawned_worktree.as_ref()).await;
            return app
                .agents
                .mark_cancelled(&agent_id, Some("Launch stopped before execution".into()))
                .await
                .unwrap_or_else(|error| {
                    fallback_failed_record(agent_id, req, result.err().unwrap_or(error))
                });
        }
    }

    let scope_worktree = spawned_worktree
        .as_ref()
        .map(|worktree| &worktree.meta)
        .or(req.parent_worktree.as_ref());
    let dirty_baseline = collect_worktree_dirty_files(scope_worktree).await;

    let run_config = config.clone();
    let result = if abort_flag.load(Ordering::Relaxed) || launch_stopping(&app) {
        Err("Aborted before execution".into())
    } else if let Some(guard) = &req.native_supervision_guard {
        let run = guard.run(
            &child_chat_id,
            run_background_subchat(app.gcx.clone(), messages, config),
        );
        tokio::pin!(run);
        let fenced = std::future::poll_fn(|cx| {
            if abort_flag.load(Ordering::Acquire) {
                std::task::Poll::Ready(Err("Aborted: native owner cancelled".into()))
            } else {
                std::future::Future::poll(run.as_mut(), cx)
            }
        });
        tokio::select! {
            result = fenced => result,
            _ = async {
                while !abort_flag.load(Ordering::Acquire) {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            } => Err("Aborted: native owner cancelled".into()),
        }
    } else {
        run_background_subchat(app.gcx.clone(), messages, config).await
    };
    drop(native_owner);
    progress_task.finish().await;
    if req.native_supervision_guard.is_some() {
        crate::subchat::finish_stateful_subchat_session(&app, &child_chat_id, &run_config).await;
    }
    let final_record = match result {
        Ok(result) if result.aborted || abort_flag.load(Ordering::Relaxed) => {
            app.agents
                .mark_cancelled(&agent_id, Some("Aborted".into()))
                .await
        }
        Ok(mut result) => {
            let mut result_summary = final_assistant_text(&result.messages);
            let deliverable_status = deliverable_status_with_corrections(
                &app,
                &agent_id,
                &run_config,
                &mut result,
                &mut result_summary,
            )
            .await;
            let handoff = handoff_report_for(&app, &agent_id, &result_summary).await;
            let (edited_files, diff_summary, conflict_summary) =
                collect_workspace_changes(scope_worktree, &dirty_baseline, &req.target_files).await;
            app.agents
                .mark_completed(
                    &agent_id,
                    AgentCompletion {
                        result_summary,
                        edited_files,
                        diff_summary,
                        conflict_summary,
                        child_chat_id: Some(child_chat_id),
                        deliverable_status,
                        handoff,
                    },
                )
                .await
        }
        Err(error) if error == "Aborted" || error.starts_with("Aborted") => {
            app.agents.mark_cancelled(&agent_id, Some(error)).await
        }
        Err(error) => app.agents.mark_failed(&agent_id, error).await,
    };

    let final_record = match final_record {
        Ok(record) => {
            let record = if record.status == crate::agents::types::BgAgentStatus::Completed {
                finalize_spawn_worktree(
                    app.clone(),
                    &agent_id,
                    &req.title,
                    spawned_worktree,
                    parent_run,
                )
                .await
                .unwrap_or(record)
            } else {
                record
            };
            emit_background_agent_update(app.clone(), &record).await;
            if req.notify_parent == NotifyParent::Auto {
                let _ = crate::agents::push::push_completion_to_parent(app.clone(), &record).await;
            }
            record
        }
        Err(error) => fallback_failed_record(agent_id, req.clone(), error),
    };

    push_sibling_notice(
        &app,
        &final_record.agent_id,
        &req,
        format!(
            "✓ sibling finished: {} — {}, edited {}",
            final_record.title,
            final_record.status.as_str(),
            format_notice_files(&final_record.edited_files),
        ),
    )
    .await;
    final_record
}

async fn run_background_subchat(
    gcx: Arc<GlobalContext>,
    messages: Vec<ChatMessage>,
    config: SubchatConfig,
) -> Result<SubchatResult, String> {
    run_background_subchat_with_lease(gcx, messages, config, None).await
}

async fn run_background_subchat_with_lease(
    gcx: Arc<GlobalContext>,
    messages: Vec<ChatMessage>,
    config: SubchatConfig,
    runner_guard: Option<RunnerLeaseGuard>,
) -> Result<SubchatResult, String> {
    use futures::FutureExt;
    std::panic::AssertUnwindSafe(run_background_subchat_inner(gcx, messages, config, runner_guard))
        .catch_unwind().await.unwrap_or_else(|_| Err("child runner panicked; saved completed boundaries remain available for explicit resume".into()))
}

async fn run_background_subchat_inner(
    gcx: Arc<GlobalContext>,
    messages: Vec<ChatMessage>,
    config: SubchatConfig,
    runner_guard: Option<RunnerLeaseGuard>,
) -> Result<SubchatResult, String> {
    #[cfg(test)]
    let (mut config, mut runner_guard) = (config, runner_guard);
    #[cfg(test)]
    {
        let runner = TEST_RUNNER
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap()
            .clone();
        if let Some(runner) = runner {
            if runner_guard.is_none() {
                config.runner_lease_token = Some(uuid::Uuid::new_v4().simple().to_string());
                if config.abort_flag.is_none() {
                    config.abort_flag = Some(Arc::new(AtomicBool::new(false)));
                }
            }
            if config.stateful && runner_guard.is_none() {
                if let Some(chat_id) = config.chat_id.as_deref() {
                    let app = AppState::from_gcx(gcx.clone()).await;
                    let source = config
                        .background_agent_id
                        .as_deref()
                        .and_then(|agent_id| app.agent_executions.saved_scope(agent_id));
                    runner_guard = crate::subchat::install_stateful_subchat_session(
                        &app,
                        chat_id,
                        &config,
                        &messages,
                        source.as_deref(),
                    )
                    .await?;
                }
            }
            let result = runner(gcx, messages, config).await;
            if let Some(guard) = runner_guard.as_mut() {
                guard.release().await;
            }
            return result;
        }
    }
    crate::subchat::run_subchat_with_lease(gcx, messages, config, runner_guard).await
}

async fn build_messages(
    app: AppState,
    config_name: &str,
    prompt: &str,
    plan: Option<&str>,
    goal: Option<&SpawnGoal>,
) -> Result<Vec<ChatMessage>, String> {
    #[cfg(test)]
    if config_name == "test_spawn" {
        let mut messages = vec![
            ChatMessage::new("system".to_string(), "test system".to_string()),
            event(
                EventSubkind::SystemNotice,
                crate::agents::push::NOTICE_SPAWN,
                serde_json::json!({
                    "config_name": config_name,
                    "test": true,
                }),
                prompt.to_string(),
            ),
        ];
        append_goal_and_plan_messages(&mut messages, plan, goal);
        return Ok(messages);
    }
    let subagent_config = crate::yaml_configs::customization_registry::get_subagent_config(
        app.gcx.clone(),
        config_name,
        None,
    )
    .await
    .ok_or_else(|| format!("subagent config '{}' not found", config_name))?;
    let system_prompt = subagent_config.messages.system_prompt.ok_or_else(|| {
        format!(
            "messages.system_prompt not defined for subagent '{}'",
            config_name
        )
    })?;
    let mut messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: ChatContent::SimpleText(system_prompt),
            ..Default::default()
        },
        event(
            EventSubkind::SystemNotice,
            crate::agents::push::NOTICE_SPAWN,
            serde_json::json!({
                "config_name": config_name,
            }),
            prompt.to_string(),
        ),
    ];
    append_goal_and_plan_messages(&mut messages, plan, goal);
    Ok(messages)
}

fn append_goal_and_plan_messages(
    messages: &mut Vec<ChatMessage>,
    plan: Option<&str>,
    goal: Option<&SpawnGoal>,
) {
    if let Some(goal) = goal {
        let mut goal_message = crate::chat::internal_roles::goal(
            "agent",
            1,
            &goal.content,
            None,
            true,
            goal.budget.clone().unwrap_or_default(),
        );
        if let Some(plan) = plan {
            let plan_message = crate::chat::internal_roles::plan("agent", 1, plan, None);
            if let Some(meta) = goal_message
                .extra
                .get_mut("goal")
                .and_then(|value| value.as_object_mut())
            {
                let mut plan_meta = plan_message.extra["plan"].clone();
                plan_meta["content"] = serde_json::json!(plan_message.content.content_text_only());
                plan_meta["message_id"] = serde_json::json!(plan_message.message_id);
                meta.insert("plan".into(), plan_meta);
            }
        }
        if !goal.criteria.is_empty() {
            if let Some(meta) = goal_message
                .extra
                .get_mut("goal")
                .and_then(|value| value.as_object_mut())
            {
                meta.insert("criteria".to_string(), serde_json::json!(goal.criteria));
            }
        }
        messages.push(goal_message);
    }
}

async fn create_spawn_worktree(
    app: AppState,
    mode: &SpawnWorktreeMode,
    parent_worktree: Option<&WorktreeMeta>,
    accepted_root: &Path,
    child_chat_id: &str,
    child_uuid: &Uuid,
    supervised: bool,
) -> Result<Option<SpawnedWorktree>, String> {
    let SpawnWorktreeMode::Isolated {
        auto_merge,
    } = mode
    else {
        return Ok(None);
    };
    // Supervised children get a runtime-owned, persisted worktree kind. The
    // file-edit runtime uses it only to fail closed: an editing context in
    // such a worktree without a live native edit scope is denied.
    let worktree_kind = if supervised {
        crate::buddy::supervision_authority::SUPERVISED_WORKTREE_KIND
    } else {
        "subagent"
    };
    let source_workspace_root = match parent_worktree {
        Some(parent) => parent.source_workspace_root.clone(),
        None => accepted_root.to_path_buf(),
    };
    let (base_branch, base_commit) = match parent_worktree {
        Some(parent) => {
            let root = parent.root.clone();
            let base_branch = parent.branch.clone();
            let base_commit = tokio::task::spawn_blocking(move || {
                let repo = crate::worktrees::git::discover_repo(&root)?;
                crate::worktrees::git::head_commit(&repo).map(Some)
            })
            .await
            .map_err(|error| format!("Failed to inspect nested subagent base: {}", error))??;
            (base_branch, base_commit)
        }
        None => {
            let source = source_workspace_root.clone();
            tokio::task::spawn_blocking(move || {
                let repo = crate::worktrees::git::discover_repo(&source)?;
                Ok::<_, String>((
                    crate::worktrees::git::current_branch(&repo),
                    Some(crate::worktrees::git::head_commit(&repo)?),
                ))
            })
            .await
            .map_err(|error| format!("Failed to inspect isolated subagent base: {}", error))??
        }
    };
    let service =
        WorktreeService::new_async(app.gcx.cache_dir.clone(), source_workspace_root.clone())
            .await?;
    let child_short = child_uuid.simple().to_string()[..8].to_string();
    let created = service
        .create_worktree(CreateWorktreeRequest {
            source_workspace_root: Some(source_workspace_root.to_string_lossy().to_string()),
            branch: Some(format!("refact/subagent/{}", child_short)),
            base_branch: base_branch.clone(),
            base_commit,
            chat_id: Some(child_chat_id.to_string()),
            kind: Some(worktree_kind.to_string()),
            ..Default::default()
        })
        .await?;
    Ok(Some(SpawnedWorktree {
        meta: created.worktree.meta,
        base_branch,
        source_workspace_root,
        auto_merge: *auto_merge,
        parent_worktree: parent_worktree.cloned(),
    }))
}

async fn continuation_base_commit(spawned: &SpawnedWorktree) -> Result<String, String> {
    let target = spawned
        .base_branch
        .clone()
        .ok_or("original merge target cannot be determined; spawn a new agent instead")?;
    let source = spawned.source_workspace_root.clone();
    let parent = spawned.parent_worktree.clone();
    tokio::task::spawn_blocking(move || {
        let repo = crate::worktrees::git::discover_repo(&source)?;
        let head = crate::worktrees::git::commit_for_ref(&repo, &format!("refs/heads/{target}"))?;
        if let Some(parent) = parent {
            if !parent.root.is_dir() || parent.branch.as_ref() != Some(&target) {
                return Err("original parent checkout unavailable".to_string());
            }
            crate::worktrees::git::ensure_checkout_on_branch(
                &parent.root,
                "Original merge target",
                &target,
            )?;
        }
        Ok(head)
    })
    .await
    .map_err(|_| "original merge target unavailable; spawn a new agent instead".to_string())?
    .map_err(|_| "original merge target no longer exists; spawn a new agent instead".to_string())
}

async fn create_continued_worktree(
    app: &AppState,
    previous: &SpawnedWorktree,
    child_chat_id: &str,
    operation: Uuid,
) -> Result<SpawnedWorktree, String> {
    let base_commit = continuation_base_commit(previous).await?;
    let service = WorktreeService::new_async(
        app.gcx.cache_dir.clone(),
        previous.source_workspace_root.clone(),
    )
    .await?;
    let created = service
        .create_worktree(CreateWorktreeRequest {
            branch: Some(format!("refact/subagent/{}", operation.simple())),
            base_branch: previous.base_branch.clone(),
            base_commit: Some(base_commit),
            chat_id: Some(child_chat_id.to_string()),
            kind: Some(previous.meta.kind.clone()),
            ..Default::default()
        })
        .await?;
    Ok(SpawnedWorktree {
        meta: created.worktree.meta,
        ..previous.clone()
    })
}

async fn cleanup_spawn_worktree(app: AppState, spawned: Option<&SpawnedWorktree>) {
    let Some(spawned) = spawned else {
        return;
    };
    match WorktreeService::new_async(
        app.gcx.cache_dir.clone(),
        spawned.source_workspace_root.clone(),
    )
    .await
    {
        Ok(service) => {
            let service = crate::worktrees::service::fenced_service(
                service,
                app.runtime.tool_jobs.scopes.clone(),
            );
            let service = match service
                .fence_worktrees(&[spawned.meta.id.to_string()], false)
                .await
            {
                Ok(service) => service,
                Err(error) => {
                    tracing::warn!(%error, "Worktree cleanup deferred by job lifecycle barrier");
                    return;
                }
            };
            // Revalidate after the drain so an id rebound to a different
            // checkout is never deleted under the old checkout's barrier.
            match service.validate_worktree_meta_strict(&spawned.meta).await {
                Ok(meta) if meta == spawned.meta => {}
                Ok(_) => {
                    tracing::warn!("Worktree cleanup deferred: registered identity changed");
                    return;
                }
                Err(error) => {
                    tracing::warn!(%error, "Worktree cleanup deferred: identity unavailable");
                    return;
                }
            }
            if let Err(error) = service.delete_worktree(&spawned.meta.id, true, true).await {
                tracing::warn!(
                    "Failed to clean up isolated subagent worktree '{}': {}",
                    spawned.meta.id,
                    error
                );
            }
        }
        Err(error) => tracing::warn!(
            "Failed to construct worktree service for cleanup: {}",
            error
        ),
    }
}

async fn finalize_spawn_worktree(
    app: AppState,
    agent_id: &str,
    title: &str,
    spawned: Option<SpawnedWorktree>,
    parent_run: Option<refact_agents_core::registry::ParentRun>,
) -> Result<BackgroundAgent, String> {
    let Some(spawned) = spawned else {
        return app.agents.get_any(agent_id).await;
    };
    if !spawned.auto_merge {
        return app.agents.set_merge_status(agent_id, "skipped").await;
    }
    let pending = app.agents.set_merge_status(agent_id, "pending").await?;
    emit_background_agent_update(app.clone(), &pending).await;
    let result = merge_spawn_worktree(&app, agent_id, title, spawned, parent_run).await;
    match result {
        Ok(result) if result.merged || result.status == "nothing_to_merge" => {
            app.agents.set_merge_status(agent_id, "merged").await
        }
        Ok(result) if result.status == "conflict" => {
            let conflict_summary = result.conflict.map(|conflict| {
                format!(
                    "{}{}",
                    conflict.instructions,
                    if conflict.files.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", conflict.files.join(", "))
                    }
                )
            });
            app.agents
                .set_merge_outcome(agent_id, "conflict", conflict_summary, None)
                .await
        }
        Ok(result) => {
            app.agents
                .set_merge_outcome(
                    agent_id,
                    "failed",
                    None,
                    Some(format!("Auto-merge failed: {}", result.status)),
                )
                .await
        }
        Err(error) => {
            app.agents
                .set_merge_outcome(
                    agent_id,
                    "failed",
                    None,
                    Some(format!("Auto-merge failed: {}", error)),
                )
                .await
        }
    }
}

async fn merge_spawn_worktree(
    app: &AppState,
    agent_id: &str,
    title: &str,
    spawned: SpawnedWorktree,
    parent_run: Option<refact_agents_core::registry::ParentRun>,
) -> Result<crate::worktrees::types::MergeWorktreeResponse, String> {
    let service = prepare_spawn_merge_service(
        app,
        agent_id,
        spawned.source_workspace_root,
        spawned.meta.clone(),
        spawned.parent_worktree,
        parent_run,
    )
    .await?;
    crate::tools::tool_task_merge_agent::merge_under_available_lease(
        app,
        &service,
        &spawned.meta.id,
        MergeWorktreeRequest {
            strategy: WorktreeMergeStrategy::Squash,
            delete_after_merge: true,
            include_uncommitted: true,
            target_branch: spawned.base_branch,
            commit_message: Some(format!("subagent: {title}")),
            generate_commit_message: false,
            acknowledge_dirty_baseline: false,
        },
        None,
        |_gcx, _request| async { Err("No subagent post-merge check requested".to_string()) },
    )
    .await
    .map(|(response, _, _)| response)
}

pub(super) async fn prepare_spawn_merge_service(
    app: &AppState,
    agent_id: &str,
    source_workspace_root: PathBuf,
    source: WorktreeMeta,
    parent_worktree: Option<WorktreeMeta>,
    parent_run: Option<refact_agents_core::registry::ParentRun>,
) -> Result<WorktreeService, String> {
    let parent_authority = if parent_worktree.is_some() {
        let record = app.agents.get_any(agent_id).await?;
        let parent_session = app
            .chat
            .sessions
            .read()
            .await
            .get(&record.parent_chat_id)
            .cloned()
            .ok_or("Nested merge parent session unavailable")?;
        let (thread, project_root, version, requests) = {
            let parent = parent_session.lock().await;
            if parent.closed
                || parent.abort_flag.load(Ordering::Acquire)
                || parent.user_interrupt_flag.load(Ordering::Acquire)
                || parent.thread.worktree != parent_worktree
            {
                return Err("Nested merge parent session checkout changed or cancelled".into());
            }
            (
                parent.thread.clone(),
                parent.work_project_root.clone(),
                parent.trajectory_version,
                parent.recent_request_ids.clone(),
            )
        };
        let app = app.clone();
        let runtime = tokio::runtime::Handle::current();
        let transition = std::sync::Mutex::new(None);
        let authority: refact_worktrees::locking::PhaseValidator = Arc::new(move || {
            if launch_stopping(&app) {
                return Err("Nested merge stopped; retain source".into());
            }
            runtime.block_on(
                app.agents
                    .validate_parent_run(&record.parent_chat_id, parent_run.as_ref()),
            )?;
            let mut transition = transition
                .try_lock()
                .map_err(|_| "Nested merge parent validation already in progress")?;
            if transition.is_none() {
                *transition = Some(crate::chat::worktree_lifecycle::try_transition_lock(
                    &parent_session,
                )?);
            }
            let sessions = app
                .chat
                .sessions
                .try_read()
                .map_err(|_| "Nested merge parent session registry busy; retain source")?;
            if !sessions
                .get(&record.parent_chat_id)
                .is_some_and(|current| Arc::ptr_eq(current, &parent_session))
            {
                return Err("Nested merge parent session identity changed".into());
            }
            let parent = parent_session
                .try_lock()
                .map_err(|_| "Nested merge parent session busy; retain source")?;
            crate::chat::worktree_lifecycle::check_unchanged_attachment(&parent, &thread)?;
            if parent.chat_id != record.parent_chat_id
                || parent.thread.id != thread.id
                || parent.work_project_root != project_root
                || parent.abort_flag.load(Ordering::Acquire)
                || parent.user_interrupt_flag.load(Ordering::Acquire)
                || parent.trajectory_version != version
                || parent.recent_request_ids != requests
                || parent.command_queue.iter().any(|request| {
                    matches!(
                        request.command,
                        crate::chat::types::ChatCommand::UserMessage { .. }
                            | crate::chat::types::ChatCommand::Abort { .. }
                            | crate::chat::types::ChatCommand::SetParams { .. }
                    )
                })
            {
                return Err(
                    "Nested merge parent control changed or cancelled; retain source".into(),
                );
            }
            Ok(())
        });
        Some(authority)
    } else {
        None
    };
    let mut service =
        WorktreeService::new_async(app.gcx.cache_dir.clone(), source_workspace_root).await?;
    service =
        crate::worktrees::service::fenced_service(service, app.runtime.tool_jobs.scopes.clone());
    if let Some(parent) = parent_worktree {
        service = service.with_nested_merge_identity(source, parent);
    }
    if let Some(authority) = parent_authority {
        service = service.with_merge_authority(authority);
    }
    Ok(service)
}

pub(crate) async fn push_sibling_notice(
    app: &AppState,
    agent_id: &str,
    req: &SpawnRequest,
    text: String,
) {
    let root_chat_id = req
        .parent_root_chat_id
        .as_deref()
        .unwrap_or(req.parent_chat_id.as_str());
    for sibling in app.agents.list_all().await.into_iter().filter(|record| {
        record.agent_id != agent_id
            && !record.status.is_terminal()
            && (record.parent_root_chat_id.as_deref() == Some(root_chat_id)
                || record.parent_chat_id == root_chat_id)
    }) {
        if let Some(chat_id) = sibling.child_chat_id.as_deref() {
            if crate::agents::seal::is_sealed(app, chat_id).await {
                continue;
            }
        }
        if let Err(error) = crate::agents::delivery::deliver_to_agent(
            app.clone(),
            &sibling.agent_id,
            PendingDelivery::new(
                vec![event(
                    EventSubkind::SystemNotice,
                    crate::agents::push::NOTICE_SIBLING,
                    serde_json::json!({"from": agent_id}),
                    text.clone(),
                )],
                PushMode::Append,
                crate::agents::push::NOTICE_SIBLING.to_string(),
                true,
            ),
        )
        .await
        {
            tracing::debug!(
                "Failed to send sibling notice to '{}': {}",
                sibling.agent_id,
                error
            );
        }
    }
}

fn format_notice_files(files: &[String]) -> String {
    if files.is_empty() {
        "-".to_string()
    } else {
        files.join(", ")
    }
}

async fn emit_subagent_hook(
    app: &AppState,
    event: HookEvent,
    record: &BackgroundAgent,
    resumed: bool,
) {
    if !app.gcx.hooks_config.enabled {
        return;
    }
    let project_dir = get_project_dir_string(app.clone()).await;
    let child_chat_id = record
        .child_chat_id
        .clone()
        .unwrap_or_else(|| record.agent_id.clone());
    let mut payload = HookPayload::new(&event, child_chat_id, project_dir);
    payload
        .extra
        .insert("agent_id".into(), serde_json::json!(record.agent_id));
    payload.extra.insert(
        "run_generation".into(),
        serde_json::json!(record.run_generation),
    );
    payload.extra.insert(
        "parent_chat_id".into(),
        serde_json::json!(record.parent_chat_id),
    );
    payload.extra.insert(
        "child_chat_id".into(),
        serde_json::json!(record.child_chat_id),
    );
    payload
        .extra
        .insert("profile".into(), serde_json::json!(record.profile_name));
    if event == HookEvent::SubagentStop {
        payload
            .extra
            .insert("status".into(), serde_json::json!(record.status.as_str()));
        payload
            .extra
            .insert("steps_used".into(), serde_json::json!(record.step_count));
    } else {
        payload
            .extra
            .insert("resumed".into(), serde_json::json!(resumed));
    }
    run_observation_hooks(app.clone(), event, payload).await;
}

pub async fn emit_background_agent_update(app: AppState, record: &BackgroundAgent) {
    if record.status.is_terminal() {
        if let Some(chat_id) = record.child_chat_id.as_deref() {
            if let Some(cleanup) = crate::chat::notifications::prepare_sealed_chat_process_cleanup(
                &app,
                chat_id,
                crate::chat::notifications::ProcessCleanupOwner::AgentTerminal {
                    agent_id: &record.agent_id,
                    run_generation: record.run_generation,
                },
            )
            .await
            {
                crate::chat::notifications::stop_chat_processes(&app.gcx, cleanup).await;
            }
        }
    }
    if app.gcx.hooks_config.enabled
        && record.status.is_terminal()
        && app
            .agents
            .claim_stop_hook(&record.agent_id, record.run_generation)
            .await
    {
        emit_subagent_hook(
            &app,
            HookEvent::SubagentStop,
            record,
            record.run_generation > 1,
        )
        .await;
    }
    crate::coordination::decomposition::observe_agent_outcome(&app, record).await;
    if record.status.is_terminal() {
        crate::coordination::causal::record_completion(
            &app,
            &record.agent_id,
            record.child_chat_id.as_deref(),
        )
        .await;
    }
    let mut destinations = vec![record.parent_chat_id.clone()];
    if let Some(root_chat_id) = record.parent_root_chat_id.as_ref() {
        if root_chat_id != &record.parent_chat_id {
            destinations.push(root_chat_id.clone());
        }
    }
    for chat_id in destinations {
        emit_background_agent_update_to_session(&app, record, &chat_id).await;
    }
}

async fn emit_background_agent_update_to_session(
    app: &AppState,
    record: &BackgroundAgent,
    chat_id: &str,
) {
    let session_arc = {
        let sessions = app.chat.sessions.read().await;
        sessions.get(chat_id).cloned()
    };
    let Some(session_arc) = session_arc else {
        return;
    };
    let mut session = session_arc.lock().await;
    if session.closed {
        return;
    }
    let agent = crate::agents::types::BackgroundAgentSummary::from(record);
    session.upsert_background_agent(agent.clone());
    let seq = session.event_seq.saturating_add(1);
    session.emit(ChatEvent::BackgroundAgentUpdated {
        chat_id: chat_id.to_string(),
        seq,
        agent,
    });
}

fn status_paths(root: &Path) -> Vec<String> {
    git_lines(root, &["status", "--porcelain"])
        .into_iter()
        .filter_map(|line| line.get(3..).map(str::trim).map(str::to_string))
        .filter(|line| !line.is_empty())
        .collect()
}

async fn collect_worktree_dirty_files(
    worktree: Option<&WorktreeMeta>,
) -> std::collections::HashSet<String> {
    let Some(worktree) = worktree else {
        return std::collections::HashSet::new();
    };
    let root = worktree.root.clone();
    tokio::task::spawn_blocking(move || status_paths(&root).into_iter().collect())
        .await
        .unwrap_or_default()
}

async fn collect_workspace_changes(
    worktree: Option<&WorktreeMeta>,
    dirty_baseline: &std::collections::HashSet<String>,
    target_files: &[String],
) -> (Vec<String>, Option<String>, Option<String>) {
    let Some(worktree) = worktree else {
        return (Vec::new(), None, None);
    };
    let root = worktree.root.clone();
    let baseline = dirty_baseline.clone();
    let targets: Vec<String> = target_files.to_vec();
    tokio::task::spawn_blocking(move || {
        let dirty_now = status_paths(&root);
        let target_matches = |path: &str| {
            targets
                .iter()
                .any(|t| path == t || path.ends_with(t.trim_start_matches("./")))
        };
        let edited_files: Vec<String> = dirty_now
            .into_iter()
            .filter(|path| !baseline.contains(path) || target_matches(path))
            .collect();
        let diff_summary = if edited_files.is_empty() {
            None
        } else {
            let mut args: Vec<&str> = vec!["diff", "--stat", "HEAD", "--"];
            let capped: Vec<&str> = edited_files
                .iter()
                .take(MAX_DIFF_SUMMARY_PATHSPECS)
                .map(String::as_str)
                .collect();
            args.extend(capped);
            command_stdout(&root, &args).filter(|text| !text.trim().is_empty())
        };
        let conflict_summary = detect_conflicts(&root, &edited_files);
        (edited_files, diff_summary, conflict_summary)
    })
    .await
    .unwrap_or_else(|_| (Vec::new(), None, None))
}

fn command_stdout(root: &Path, args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
}

fn git_lines(root: &Path, args: &[&str]) -> Vec<String> {
    command_stdout(root, args)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn detect_conflicts(root: &Path, edited_files: &[String]) -> Option<String> {
    let files: Vec<String> = edited_files
        .iter()
        .filter(|rel| file_has_conflict_markers(&root.join(rel)))
        .cloned()
        .collect();
    if files.is_empty() {
        None
    } else {
        Some(format!("Conflict markers detected in {}", files.join(", ")))
    }
}

fn file_has_conflict_markers(path: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    let (mut start, mut sep, mut end) = (false, false, false);
    for line in content.lines() {
        if line.starts_with("<<<<<<< ") {
            start = true;
        } else if line == "=======" {
            sep = true;
        } else if line.starts_with(">>>>>>> ") {
            end = true;
        }
    }
    start && sep && end
}

fn fallback_failed_record(agent_id: String, req: SpawnRequest, error: String) -> BackgroundAgent {
    let deliverable = req
        .deliverable
        .clone()
        .filter(|contract| !contract.is_empty());
    let now = chrono::Utc::now();
    BackgroundAgent {
        schema_version: 1,
        run_generation: 1,
        stop_hook_generation: None,
        agent_id,
        parent_chat_id: req.parent_chat_id,
        parent_root_chat_id: req.parent_root_chat_id,
        parent_tool_call_id: req.parent_tool_call_id,
        child_chat_id: None,
        kind: req.kind,
        config_name: req.config_name,
        title: req.title,
        prompt: req.prompt,
        target_files: req.target_files,
        status: crate::agents::types::BgAgentStatus::Failed,
        paused_by: None,
        paused_at: None,
        progress: None,
        step_count: 0,
        last_activity: None,
        result_summary: None,
        result_payload_path: None,
        error: Some(error),
        edited_files: Vec::new(),
        diff_summary: None,
        conflict_summary: None,
        pending_deliveries: Vec::new(),
        followups: Vec::new(),
        delivery_ids: Vec::new(),
        dropped_deliveries: Vec::new(),
        completion_push: req.completion_push,
        completion_message_id: None,
        completion_pushed_at: None,
        deferred_at: None,
        close_reason: None,
        previous_runs: Vec::new(),
        model: req.model,
        model_type: None,
        current_tool: None,
        goal_summary: None,
        plan_present: false,
        worktree_id: None,
        worktree_branch: None,
        merge_status: None,
        questions: Vec::new(),
        tokens_used: 0,
        cost_usd: None,
        deliverable,
        deliverable_status: None,
        profile_name: None,
        handoff: None,
        created_at: now,
        started_at: None,
        finished_at: Some(now),
        last_update_at: now,
        change_seq: 1,
    }
}

fn final_assistant_text(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant")
        .map(|message| message.content.content_text_only())
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| NO_TEXT_RESULT_SUMMARY.to_string())
}

fn correction_policy() -> refact_chat_api::CorrectionPolicy {
    let coordination = crate::runtime_settings::current().coordination;
    refact_chat_api::CorrectionPolicy {
        max_attempts: coordination.deliverable_max_corrections as u32,
        result_budget_tokens: coordination.deliverable_result_budget_tokens,
    }
}

/// What is actually left to write a deliverable into: the unused context, capped
/// by the most the model may emit in one answer.
fn result_headroom(
    config: &crate::subchat::SubchatConfig,
    messages: &[ChatMessage],
    reference: &str,
) -> refact_chat_api::ResultHeadroom {
    let used: usize = messages
        .iter()
        .map(|message| {
            refact_codewiki::token_budget::estimate_tokens(&message.content.content_text_only())
        })
        .sum();
    let remaining = config.n_ctx.saturating_sub(used);
    refact_chat_api::ResultHeadroom {
        available_tokens: if config.max_new_tokens > 0 {
            remaining.min(config.max_new_tokens)
        } else {
            remaining
        },
        reference: reference.to_string(),
    }
}

/// Evaluates the deliverable contract and, while it is unmet, asks the child to
/// fix exactly the named elements — at most `deliverable_max_corrections` times.
/// Runs after the child's subchat returned, so no session lock is held, and under
/// the admission permit the launch already owns rather than a fresh reservation.
async fn deliverable_status_with_corrections(
    app: &AppState,
    agent_id: &str,
    config: &crate::subchat::SubchatConfig,
    result: &mut SubchatResult,
    result_summary: &mut String,
) -> Option<refact_chat_api::DeliverableStatus> {
    let contract = app.agents.get_any(agent_id).await.ok()?.deliverable?;
    let policy = correction_policy();
    let reference = config
        .chat_id
        .clone()
        .unwrap_or_else(|| agent_id.to_string());
    let mut status = contract.evaluate(result_summary);
    loop {
        let headroom = result_headroom(config, &result.messages, &reference);
        let (next, request) =
            refact_chat_api::plan_correction(&contract, status, &policy, &headroom);
        status = next;
        let Some(request) = request else { break };
        let aborted = config
            .abort_flag
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed));
        if aborted || launch_stopping(app) {
            status.corrections_exhausted = true;
            break;
        }
        let mut messages = result.messages.clone();
        messages.push(ChatMessage::new("user".to_string(), request.prompt));
        let mut correction_config = config.clone();
        correction_config.max_steps = config.max_steps.min(CORRECTION_MAX_STEPS);
        match run_background_subchat(app.gcx.clone(), messages, correction_config).await {
            Ok(corrected) if !corrected.aborted => {
                *result_summary = final_assistant_text(&corrected.messages);
                status = contract
                    .evaluate(result_summary)
                    .after_correction(request.attempt);
                result.messages = corrected.messages;
            }
            _ => {
                status.corrections_exhausted = true;
                break;
            }
        }
    }
    Some(status)
}

/// Reads the child's optional handoff-sufficiency declaration and filters every
/// path it names for the parent destination before the claim is stored.
async fn handoff_report_for(
    app: &AppState,
    agent_id: &str,
    final_text: &str,
) -> Option<refact_chat_api::HandoffReport> {
    let limits = crate::agents::handoff::current_limits();
    let mut report = refact_chat_api::handoff::parse_report(
        final_text,
        agent_id,
        chrono::Utc::now().timestamp_millis(),
        &limits,
    )?;
    let parent_chat_id = app
        .agents
        .get_any(agent_id)
        .await
        .map(|record| record.parent_chat_id)
        .unwrap_or_default();
    let destination = crate::agents::handoff::parent_destination(app, &parent_chat_id).await;
    crate::agents::handoff::redact_for_destination(&app.gcx, destination.as_ref(), &mut report);
    Some(report)
}

#[cfg(test)]
mod resume_validation_tests {
    use super::*;

    #[test]
    fn recovery_thread_revision_ignores_only_incomplete_prefix_clock() {
        let mut thread = crate::chat::types::ThreadParams::default();
        thread.frozen_request_prefix = Some(
            crate::chat::trajectories::new_legacy_incomplete_frozen_request_prefix(Some(
                "system".into(),
            )),
        );
        let first = recovery_thread_revision(&thread).unwrap();
        thread.frozen_request_prefix.as_mut().unwrap().created_at = "different load".into();
        assert_eq!(recovery_thread_revision(&thread).unwrap(), first);
        thread.frozen_request_prefix.as_mut().unwrap().system_prompt = Some("changed".into());
        assert_ne!(recovery_thread_revision(&thread).unwrap(), first);
        thread
            .frozen_request_prefix
            .as_mut()
            .unwrap()
            .tools_canonical = Some(serde_json::json!([]));
        let complete = recovery_thread_revision(&thread).unwrap();
        thread.frozen_request_prefix.as_mut().unwrap().created_at = "new revision".into();
        assert_ne!(recovery_thread_revision(&thread).unwrap(), complete);
    }

    #[test]
    fn recovery_revision_is_stable_across_json_reload_and_detects_changes() {
        let mut message = ChatMessage::new("assistant".into(), "saved".into());
        for index in 0..32 {
            message.extra.insert(
                format!("field-{index}"),
                serde_json::json!({"z":index,"a":true}),
            );
        }
        let messages = vec![message];
        let expected = recovery_messages_revision(&messages).unwrap();
        for _ in 0..16 {
            let mut restored: Vec<ChatMessage> =
                serde_json::from_slice(&serde_json::to_vec(&messages).unwrap()).unwrap();
            assert_eq!(recovery_messages_revision(&restored).unwrap(), expected);
            restored[0].content = ChatContent::SimpleText("changed".into());
            assert_ne!(recovery_messages_revision(&restored).unwrap(), expected);
        }
    }

    #[test]
    fn recovery_accepts_only_reserved_context_attachments_without_tool_effects() {
        for marker in [
            "project_context",
            "memories_context",
            "buddy_project_memory_pulse",
        ] {
            let attachment = ChatMessage {
                role: "context_file".into(),
                content: ChatContent::ContextFiles(vec![crate::call_validation::ContextFile {
                    file_name: "runtime.md".into(),
                    file_content: "context".into(),
                    ..Default::default()
                }]),
                tool_call_id: marker.into(),
                extra: serde_json::from_value(
                    serde_json::json!({"source": "untrusted", "privacy": {"zone": "normal"}}),
                )
                .unwrap(),
                ..Default::default()
            };
            let original = serde_json::to_value(&attachment).unwrap();
            let mut signed_call = call();
            signed_call.thinking_blocks = Some(vec![
                serde_json::json!({"type":"thinking", "thinking":"saved", "signature":"opaque-signature"}),
            ]);
            let signed = serde_json::to_value(&signed_call).unwrap();
            let mut messages = vec![attachment.clone(), signed_call, result()];
            reconcile_resume_tool_pairs(&mut messages, false).unwrap();
            assert_eq!(serde_json::to_value(&messages[0]).unwrap(), original);
            assert_eq!(serde_json::to_value(&messages[1]).unwrap(), signed);
            let mut malformed = attachment.clone();
            malformed.content = ChatContent::SimpleText("not an attachment".into());
            assert!(reconcile_resume_tool_pairs(&mut vec![malformed], true).is_err());
            let mut failed = attachment.clone();
            failed.tool_failed = Some(true);
            assert!(reconcile_resume_tool_pairs(&mut vec![failed], true).is_err());
            for role in ["tool", "diff", "plain_text"] {
                let mut orphan = attachment.clone();
                orphan.role = role.into();
                assert!(reconcile_resume_tool_pairs(&mut vec![orphan], true).is_err());
            }
            let mut orphan = attachment.clone();
            orphan.tool_call_id = "actual-orphan".into();
            assert!(reconcile_resume_tool_pairs(&mut vec![orphan], true).is_err());
            let mut colliding_call = call();
            colliding_call.tool_calls.as_mut().unwrap()[0].id = marker.into();
            assert!(
                reconcile_resume_tool_pairs(&mut vec![colliding_call, attachment], false).is_err()
            );
        }
        assert!(reconcile_resume_tool_pairs(&mut vec![call(), call()], true).is_err());
        assert!(reconcile_resume_tool_pairs(
            &mut vec![call(), ChatMessage::new("user".into(), "later".into())],
            true
        )
        .is_err());
        let mut incomplete = vec![call()];
        reconcile_resume_tool_pairs(&mut incomplete, true).unwrap();
        assert_eq!(incomplete[1].tool_call_id, "one");
        assert_eq!(
            incomplete[1].extra["agent_recovery"],
            serde_json::json!({"outcome":"unknown", "replayed":false})
        );
    }

    #[tokio::test]
    async fn recovery_accepts_context_from_instruction_producer_after_json_reload() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().join("AGENTS.md");
        tokio::fs::write(&path, "Project instructions")
            .await
            .unwrap();
        let context = crate::chat::system_context::create_instruction_files_message(&[
            crate::chat::system_context::InstructionFile {
                file_name: "AGENTS.md".into(),
                file_path: path.to_string_lossy().into(),
                source_tool: "project_context".into(),
                processed_content: None,
                importance: 1,
                max_chars: None,
            },
        ])
        .await
        .unwrap();
        let original = serde_json::to_value(&context).unwrap();
        let mut messages =
            serde_json::from_value::<Vec<ChatMessage>>(serde_json::json!([context])).unwrap();
        reconcile_resume_tool_pairs(&mut messages, false).unwrap();
        assert_eq!(serde_json::to_value(&messages[0]).unwrap(), original);
    }

    #[tokio::test]
    async fn recovery_revision_survives_reopen_but_detects_durable_edits() {
        use crate::agents::types::{BgAgentKind, CreateAgentRequest};
        use refact_core::chat_types::{PendingDelivery, PushMode};
        let workspace = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.path().into()];
        let app = AppState::from_gcx(gcx.clone()).await;
        let (record, _, _) = app
            .agents
            .create(CreateAgentRequest {
                parent_chat_id: "recovery-parent".into(),
                parent_root_chat_id: Some("recovery-parent".into()),
                parent_tool_call_id: None,
                kind: BgAgentKind::Subagent,
                config_name: "subagent".into(),
                title: "recovery".into(),
                prompt: "test".into(),
                target_files: vec![],
                model: "test".into(),
                model_type: None,
                goal_summary: None,
                plan_present: false,
                worktree_id: None,
                worktree_branch: None,
            })
            .await
            .unwrap();
        let child = "recovery-durable-child";
        app.agents
            .mark_running(&record.agent_id, child.into())
            .await
            .unwrap();
        let stopped = app
            .agents
            .mark_interrupted(&record.agent_id, "restart".into())
            .await
            .unwrap();
        assert!(!app.agents.has_runtime(&record.agent_id).await);
        let delivery = PendingDelivery::with_id(
            "saved-mail",
            vec![ChatMessage::new("user".into(), "pending".into())],
            PushMode::Append,
            "test",
            true,
        );
        let path = workspace
            .path()
            .join(".refact/trajectories")
            .join(format!("{child}.json"));
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        let saved = serde_json::json!({
            "id":child, "model":"test", "mode":"agent", "tool_use":"agent",
            "created_at":"2024-01-01T00:00:00Z", "updated_at":"2024-01-01T00:00:01Z",
            "messages":[{"message_id":"saved", "role":"user", "content":"saved"}],
            "pending_deliveries":[delivery],
            "goal": refact_chat_api::GoalSnapshot { content: "saved goal".into(), ..Default::default() },
            "goal_ledger": [{"seq":1, "at_ms":1, "op":"no_progress_noted"}]
        });
        tokio::fs::write(&path, serde_json::to_vec(&saved).unwrap())
            .await
            .unwrap();
        let baseline = crate::chat::trajectories::load_trajectory_for_recovery(gcx.clone(), child)
            .await
            .unwrap();
        assert_eq!(baseline.pending_deliveries.len(), 1);
        assert_eq!(baseline.goal.as_ref().unwrap().content, "saved goal");
        assert_eq!(baseline.goal_ledger.len(), 1);
        let revision = resume_trajectory_revision(&baseline).unwrap();
        app.agents
            .reopen_for_parent_run(
                &record.agent_id,
                "recovery-parent",
                stopped.change_seq,
                false,
                None,
            )
            .await
            .unwrap();
        assert!(app.agents.has_runtime(&record.agent_id).await);
        let ordinary = crate::chat::trajectories::load_trajectory_for_chat(gcx.clone(), child)
            .await
            .unwrap();
        assert!(ordinary.pending_deliveries.is_empty());
        let reloaded = crate::chat::trajectories::load_trajectory_for_recovery(gcx.clone(), child)
            .await
            .unwrap();
        assert_eq!(reloaded.pending_deliveries[0].id, "saved-mail");
        assert_eq!(resume_trajectory_revision(&reloaded).unwrap(), revision);
        for field in [
            "messages",
            "pending_deliveries",
            "model",
            "goal",
            "goal_ledger",
        ] {
            let mut edited = saved.clone();
            match field {
                "messages" => {
                    edited["messages"][0]["content"] = serde_json::json!("concurrent edit")
                }
                "pending_deliveries" => {
                    edited["pending_deliveries"][0]["id"] = serde_json::json!("concurrent-mail")
                }
                "goal" => edited["goal"]["content"] = serde_json::json!("concurrent goal"),
                "goal_ledger" => edited["goal_ledger"][0]["at_ms"] = serde_json::json!(2),
                _ => edited["model"] = serde_json::json!("different-model"),
            }
            tokio::fs::write(&path, serde_json::to_vec(&edited).unwrap())
                .await
                .unwrap();
            let changed =
                crate::chat::trajectories::load_trajectory_for_recovery(gcx.clone(), child)
                    .await
                    .unwrap();
            assert_ne!(
                resume_trajectory_revision(&changed).unwrap(),
                revision,
                "{field}"
            );
        }
        app.agents
            .mark_interrupted(&record.agent_id, "test cleanup".into())
            .await
            .unwrap();
    }

    fn call() -> ChatMessage {
        serde_json::from_value(serde_json::json!({
            "role": "assistant", "content": "", "tool_calls": [
                {"id": "one", "type": "function", "function": {"name": "cat", "arguments": "{}"}}
            ]
        }))
        .unwrap()
    }

    fn result() -> ChatMessage {
        ChatMessage {
            role: "tool".into(),
            tool_call_id: "one".into(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod supervised_rollback_tests {
    use super::*;

    #[tokio::test]
    async fn cancelling_startup_future_keeps_rollback_owner_alive() {
        let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let child = uuid::Uuid::new_v4().to_string();
        let session = Arc::new(AMutex::new(crate::chat::types::ChatSession::new(
            child.clone(),
        )));
        app.chat
            .sessions
            .write()
            .await
            .insert(child.clone(), session.clone());
        let rollback = SupervisedStartupRollback {
            app: app.clone(),
            child: child.clone(),
            worktree: None,
            agent_id: None,
            armed: true,
        };
        let (ready_tx, ready_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let _rollback = rollback;
            ready_tx.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        ready_rx.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(1), async {
            while !session.lock().await.closed {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!app.chat.sessions.read().await.contains_key(&child));
    }

    #[tokio::test]
    async fn failed_startup_rollback_closes_native_session() {
        let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let child = uuid::Uuid::new_v4().to_string();
        let session = Arc::new(AMutex::new(crate::chat::types::ChatSession::new(
            child.clone(),
        )));
        session
            .lock()
            .await
            .set_runtime_state(crate::chat::types::SessionState::Generating, None);
        app.chat
            .sessions
            .write()
            .await
            .insert(child.clone(), session.clone());
        let rollback = SupervisedStartupRollback {
            app,
            child,
            worktree: None,
            agent_id: None,
            armed: true,
        };
        drop(rollback);
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if session.lock().await.closed {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}
