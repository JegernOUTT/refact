use std::path::{Path, PathBuf};
#[cfg(test)]
use std::pin::Pin;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{Mutex as AMutex, mpsc::UnboundedSender, oneshot};
use uuid::Uuid;

use crate::agents::registry::InboxMessage;
use crate::agents::types::{
    AgentCompletion, BackgroundAgent, BgAgentKind, CreateAgentRequest, NO_TEXT_RESULT_SUMMARY,
};
use crate::app_state::AppState;
use crate::at_commands::at_commands::MAX_SUBCHAT_DEPTH;
use crate::call_validation::{ChatContent, ChatMessage};
use crate::chat::internal_roles::{event, EventSubkind};
use crate::chat::types::{ChatEvent, GoalBudget, GoalCriterion, TaskMeta};
use crate::global_context::GlobalContext;
use crate::subchat::{
    SubchatConfig, SubchatProgress, SubchatResult, resolve_subchat_config_with_parent,
};
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::{
    CreateWorktreeRequest, MergeWorktreeRequest, WorktreeMergeStrategy, WorktreeMeta,
};

const MAX_DIFF_SUMMARY_PATHSPECS: usize = 100;

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
    pub kind: BgAgentKind,
    pub parent_chat_id: String,
    pub parent_root_chat_id: Option<String>,
    pub parent_tool_call_id: Option<String>,
    pub config_name: String,
    pub title: String,
    pub prompt: String,
    pub tools: Option<Vec<String>>,
    pub target_files: Vec<String>,
    pub max_steps: usize,
    pub model: String,
    pub model_type: Option<String>,
    pub goal: Option<SpawnGoal>,
    pub plan: Option<String>,
    pub worktree_mode: SpawnWorktreeMode,
    pub parent_subchat_tx: Option<Arc<AMutex<UnboundedSender<Value>>>>,
    pub parent_worktree: Option<WorktreeMeta>,
    pub parent_task_meta: Option<TaskMeta>,
    pub subchat_depth: usize,
    pub notify_parent: NotifyParent,
}

#[derive(Clone)]
pub struct SpawnGoal {
    pub content: String,
    pub criteria: Vec<GoalCriterion>,
    pub budget: Option<GoalBudget>,
}

#[derive(Clone, Default)]
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
    pub worktree_branch: Option<String>,
    pub auto_merge: Option<bool>,
    pub completion_rx: oneshot::Receiver<BackgroundAgent>,
}

pub async fn spawn_background_agent(
    app: AppState,
    req: SpawnRequest,
) -> Result<SpawnHandle, String> {
    if req.subchat_depth >= MAX_SUBCHAT_DEPTH {
        return Err(format!(
            "subchat depth limit ({}) exceeded",
            MAX_SUBCHAT_DEPTH
        ));
    }
    let child_uuid = Uuid::new_v4();
    let child_chat_id = format!("subchat-{}", child_uuid);
    let config_name = req.config_name.clone();
    let config_title = req.title.clone();
    let parent_chat_id = req.parent_chat_id.clone();
    let link_type = req.kind.as_str().to_string();
    let parent_root_chat_id = req.parent_root_chat_id.clone();
    let parent_task_meta = req.parent_task_meta.clone();
    let spawned_worktree = create_spawn_worktree(
        app.clone(),
        &req.worktree_mode,
        req.parent_worktree.as_ref(),
        &child_chat_id,
        &child_uuid,
    )
    .await?;
    let effective_worktree = spawned_worktree
        .as_ref()
        .map(|worktree| worktree.meta.clone())
        .or_else(|| req.parent_worktree.clone());
    let parent_tool_call_id = req.parent_tool_call_id.clone();
    let parent_subchat_tx = req.parent_subchat_tx.clone();
    let max_steps = req
        .goal
        .as_ref()
        .and_then(|goal| goal.budget.as_ref())
        .and_then(|budget| budget.max_turns)
        .map(|max_turns| req.max_steps.min(max_turns as usize))
        .unwrap_or(req.max_steps);
    let tools = req.tools.clone();
    let subchat_depth = req.subchat_depth;
    #[cfg(test)]
    let config_result = if config_name == "test_spawn" {
        Ok(SubchatConfig {
            tool_name: config_name.clone(),
            stateful: true,
            autonomous_no_confirm: false,
            chat_id: Some(child_chat_id.clone()),
            title: Some(config_title.clone()),
            parent_id: Some(parent_chat_id.clone()),
            link_type: Some(link_type.clone()),
            root_chat_id: parent_root_chat_id.clone(),
            tools: crate::subchat::ToolsPolicy::from_option(tools.clone()),
            max_steps,
            prepend_system_prompt: false,
            wrap_up: None,
            task_meta: parent_task_meta.clone(),
            worktree: effective_worktree.clone(),
            model: req.model.clone(),
            mode: "agent".to_string(),
            n_ctx: 4096,
            max_new_tokens: 512,
            temperature: None,
            reasoning_effort: None,
            cache_control: crate::llm::params::CacheControl::Ephemeral,
            parent_tool_call_id: parent_tool_call_id.clone(),
            parent_subchat_tx: parent_subchat_tx.clone(),
            abort_flag: None,
            background_agent_id: None,
            subchat_depth: subchat_depth + 1,
            final_step_force_answer: false,
            buddy_meta: None,
            step_progress: None,
            trace_parent: crate::subchat::TraceParent::from_parts(
                Some(&parent_chat_id),
                parent_root_chat_id.as_deref(),
            ),
        })
    } else {
        resolve_subchat_config_with_parent(
            app.gcx.clone(),
            &config_name,
            true,
            Some(child_chat_id.clone()),
            Some(config_title),
            Some(parent_chat_id),
            Some(link_type),
            parent_root_chat_id,
            tools.clone(),
            max_steps,
            false,
            None,
            "agent".to_string(),
            parent_task_meta,
            effective_worktree.clone(),
            parent_tool_call_id,
            parent_subchat_tx,
            None,
            subchat_depth + 1,
        )
        .await
    };
    #[cfg(not(test))]
    let config_result = resolve_subchat_config_with_parent(
        app.gcx.clone(),
        &config_name,
        true,
        Some(child_chat_id.clone()),
        Some(config_title),
        Some(parent_chat_id),
        Some(link_type),
        parent_root_chat_id,
        tools,
        max_steps,
        false,
        None,
        "agent".to_string(),
        parent_task_meta,
        effective_worktree.clone(),
        parent_tool_call_id,
        parent_subchat_tx,
        None,
        subchat_depth + 1,
    )
    .await;

    let mut config = match config_result {
        Ok(config) => config,
        Err(error) => {
            cleanup_spawn_worktree(app.clone(), spawned_worktree.as_ref()).await;
            return Err(error);
        }
    };
    config.model = req.model.clone();

    let messages = match build_messages(
        app.clone(),
        &req.config_name,
        &req.prompt,
        req.plan.as_deref(),
        req.goal.as_ref(),
    )
    .await
    {
        Ok(messages) => messages,
        Err(error) => {
            cleanup_spawn_worktree(app.clone(), spawned_worktree.as_ref()).await;
            return Err(error);
        }
    };

    let (record, abort_flag, _) = match app
        .agents
        .create(CreateAgentRequest {
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
            worktree_id: effective_worktree
                .as_ref()
                .map(|worktree| worktree.id.clone()),
            worktree_branch: effective_worktree
                .as_ref()
                .and_then(|worktree| worktree.branch.clone()),
        })
        .await
    {
        Ok(created) => created,
        Err(error) => {
            cleanup_spawn_worktree(app.clone(), spawned_worktree.as_ref()).await;
            return Err(error);
        }
    };
    emit_background_agent_update(app.clone(), &record).await;

    let agent_id = record.agent_id.clone();
    let handle_agent_id = agent_id.clone();
    let handle_child_chat_id = child_chat_id.clone();
    let handle_worktree_branch = spawned_worktree
        .as_ref()
        .and_then(|worktree| worktree.meta.branch.clone());
    let handle_auto_merge = spawned_worktree
        .as_ref()
        .map(|worktree| worktree.auto_merge);
    let (completion_tx, completion_rx) = oneshot::channel();

    push_sibling_notice(
        &app,
        &record.agent_id,
        &req,
        format!(
            "▸ sibling started: {} targeting {}",
            record.title,
            format_notice_files(&record.target_files),
        ),
    )
    .await;

    tokio::spawn(async move {
        let agent_id_for_run = agent_id.clone();
        let final_record = match tokio::spawn(run_spawned_agent(
            app.clone(),
            req.clone(),
            config,
            messages,
            agent_id,
            child_chat_id,
            abort_flag,
            spawned_worktree,
        ))
        .await
        {
            Ok(record) => record,
            Err(error) => {
                let message = if error.is_panic() {
                    "agent task panicked".to_string()
                } else {
                    "agent task cancelled".to_string()
                };
                match app
                    .agents
                    .mark_failed(&agent_id_for_run, message.clone())
                    .await
                {
                    Ok(record) => {
                        emit_background_agent_update(app.clone(), &record).await;
                        record
                    }
                    Err(_) => fallback_failed_record(agent_id_for_run, req, message),
                }
            }
        };
        let _ = completion_tx.send(final_record);
    });

    Ok(SpawnHandle {
        agent_id: handle_agent_id,
        child_chat_id: handle_child_chat_id,
        worktree_branch: handle_worktree_branch,
        auto_merge: handle_auto_merge,
        completion_rx,
    })
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

async fn run_spawned_agent(
    app: AppState,
    req: SpawnRequest,
    mut config: crate::subchat::SubchatConfig,
    messages: Vec<ChatMessage>,
    agent_id: String,
    child_chat_id: String,
    abort_flag: Arc<AtomicBool>,
    spawned_worktree: Option<SpawnedWorktree>,
) -> BackgroundAgent {
    config.abort_flag = Some(abort_flag);
    config.background_agent_id = Some(agent_id.clone());
    config.final_step_force_answer = true;
    {
        let progress_app = app.clone();
        let progress_agent_id = agent_id.clone();
        config.step_progress = Some(Arc::new(move |progress| {
            let app = progress_app.clone();
            let agent_id = progress_agent_id.clone();
            tokio::spawn(async move {
                let updated = match progress {
                    SubchatProgress::Step(step) => {
                        app.agents
                            .update_activity(
                                &agent_id,
                                Some(format!("step {}", step)),
                                Some(step as u32),
                                None,
                            )
                            .await
                    }
                    SubchatProgress::ToolStarted { name, arg_preview } => {
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
            });
        }));
    }
    let running = app
        .agents
        .mark_running(&agent_id, child_chat_id.clone())
        .await;
    if let Ok(record) = running.as_ref() {
        emit_background_agent_update(app.clone(), record).await;
    }

    let scope_worktree = spawned_worktree
        .as_ref()
        .map(|worktree| &worktree.meta)
        .or(req.parent_worktree.as_ref());
    let dirty_baseline = collect_worktree_dirty_files(scope_worktree).await;

    let result = run_background_subchat(app.gcx.clone(), messages, config).await;
    let final_record = match result {
        Ok(result) => {
            let result_summary = result
                .messages
                .iter()
                .rev()
                .find(|message| message.role == "assistant")
                .map(|message| message.content.content_text_only())
                .filter(|text| !text.trim().is_empty())
                .unwrap_or_else(|| NO_TEXT_RESULT_SUMMARY.to_string());
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
                finalize_spawn_worktree(app.clone(), &agent_id, &req.title, spawned_worktree)
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
    #[cfg(test)]
    {
        let runner = TEST_RUNNER
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap()
            .clone();
        if let Some(runner) = runner {
            if config.stateful {
                if let Some(chat_id) = config.chat_id.as_deref() {
                    let app = AppState::from_gcx(gcx.clone()).await;
                    crate::subchat::install_stateful_subchat_session(
                        &app, chat_id, &config, &messages,
                    )
                    .await;
                }
            }
            return runner(gcx, messages, config).await;
        }
    }
    crate::subchat::run_subchat(gcx, messages, config).await
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
                "agents.spawn",
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
            "agents.spawn",
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
    if let Some(plan) = plan {
        messages.push(crate::chat::internal_roles::plan("agent", 1, plan, None));
    }
    if let Some(goal) = goal {
        let mut goal_message = crate::chat::internal_roles::goal(
            "agent",
            1,
            &goal.content,
            None,
            true,
            goal.budget.clone().unwrap_or_default(),
        );
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
    child_chat_id: &str,
    child_uuid: &Uuid,
) -> Result<Option<SpawnedWorktree>, String> {
    let SpawnWorktreeMode::Isolated { auto_merge } = mode else {
        return Ok(None);
    };
    let source_workspace_root = match parent_worktree {
        Some(parent) => parent.source_workspace_root.clone(),
        None => crate::files_correction::get_active_project_path(app.gcx.clone())
            .await
            .ok_or_else(|| "No active project folder found for isolated subagent".to_string())?,
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
            kind: Some("subagent".to_string()),
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
) -> Result<BackgroundAgent, String> {
    let Some(spawned) = spawned else {
        return app.agents.get_any(agent_id).await;
    };
    if !spawned.auto_merge {
        return app.agents.set_merge_status(agent_id, "skipped").await;
    }
    let pending = app.agents.set_merge_status(agent_id, "pending").await?;
    emit_background_agent_update(app.clone(), &pending).await;
    if let Some(parent_worktree) = spawned.parent_worktree.clone() {
        return finalize_nested_spawn_worktree(app, agent_id, title, spawned, &parent_worktree)
            .await;
    }
    let service =
        WorktreeService::new_async(app.gcx.cache_dir.clone(), spawned.source_workspace_root)
            .await?;
    let result = service
        .merge_worktree(
            &spawned.meta.id,
            MergeWorktreeRequest {
                strategy: WorktreeMergeStrategy::Squash,
                delete_after_merge: true,
                include_uncommitted: true,
                target_branch: spawned.base_branch,
                commit_message: Some(format!("subagent: {}", title)),
                generate_commit_message: false,
            },
        )
        .await;
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

async fn finalize_nested_spawn_worktree(
    app: AppState,
    agent_id: &str,
    title: &str,
    spawned: SpawnedWorktree,
    parent_worktree: &WorktreeMeta,
) -> Result<BackgroundAgent, String> {
    let child_branch = spawned
        .meta
        .branch
        .as_deref()
        .ok_or_else(|| "isolated subagent worktree has no branch".to_string())?
        .to_string();
    let parent_root = parent_worktree.root.clone();
    let child_root = spawned.meta.root.clone();
    let commit_message = format!("subagent: {title}");
    let merge_result = tokio::task::spawn_blocking(move || {
        let result = crate::worktrees::git::commit_all(&child_root, &commit_message)
            .and_then(|_| {
                crate::worktrees::git::run_git(&parent_root, &["merge", "--squash", &child_branch])
            })
            .and_then(|_| {
                crate::worktrees::git::run_git_with_refact_author(
                    &parent_root,
                    &["commit", "-m", &commit_message, "--no-gpg-sign"],
                )
            });
        if result.is_err() {
            crate::worktrees::git::cleanup_failed_merge(&parent_root);
        }
        result
    })
    .await
    .map_err(|error| format!("nested subagent merge task failed: {error}"))?;
    match merge_result {
        Ok(_) => {
            let service = WorktreeService::new_async(
                app.gcx.cache_dir.clone(),
                spawned.source_workspace_root,
            )
            .await?;
            service
                .delete_worktree(&spawned.meta.id, true, true)
                .await?;
            app.agents.set_merge_status(agent_id, "merged").await
        }
        Err(error) => {
            if error.to_lowercase().contains("conflict") {
                app.agents
                    .set_merge_outcome(agent_id, "conflict", Some(error), None)
                    .await
            } else {
                app.agents
                    .set_merge_outcome(
                        agent_id,
                        "failed",
                        None,
                        Some(format!("Auto-merge failed: {error}")),
                    )
                    .await
            }
        }
    }
}

async fn push_sibling_notice(app: &AppState, agent_id: &str, req: &SpawnRequest, text: String) {
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
        if let Err(error) = app
            .agents
            .push_inbox(
                &sibling.agent_id,
                InboxMessage {
                    from: agent_id.to_string(),
                    text: text.clone(),
                    queued_at: chrono::Utc::now(),
                },
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

pub async fn emit_background_agent_update(app: AppState, record: &BackgroundAgent) {
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
    let now = chrono::Utc::now();
    BackgroundAgent {
        schema_version: 1,
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
        progress: None,
        step_count: 0,
        last_activity: None,
        result_summary: None,
        result_payload_path: None,
        error: Some(error),
        edited_files: Vec::new(),
        diff_summary: None,
        conflict_summary: None,
        completion_message_id: None,
        completion_pushed_at: None,
        deferred_at: None,
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
        created_at: now,
        started_at: None,
        finished_at: Some(now),
        last_update_at: now,
        change_seq: 1,
    }
}
