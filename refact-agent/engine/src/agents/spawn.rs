use std::path::Path;
#[cfg(test)]
use std::pin::Pin;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{Mutex as AMutex, mpsc::UnboundedSender, oneshot};
use uuid::Uuid;

use crate::agents::types::{AgentCompletion, BackgroundAgent, BgAgentKind, CreateAgentRequest};
use crate::app_state::AppState;
use crate::at_commands::at_commands::MAX_SUBCHAT_DEPTH;
use crate::call_validation::{ChatContent, ChatMessage};
use crate::chat::types::{ChatEvent, TaskMeta};
use crate::global_context::GlobalContext;
use crate::subchat::{SubchatConfig, SubchatResult, resolve_subchat_config_with_parent};
use crate::worktrees::types::WorktreeMeta;

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
    pub parent_subchat_tx: Option<Arc<AMutex<UnboundedSender<Value>>>>,
    pub parent_worktree: Option<WorktreeMeta>,
    pub parent_task_meta: Option<TaskMeta>,
    pub subchat_depth: usize,
    pub notify_parent: NotifyParent,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NotifyParent {
    Auto,
    Silent,
}

pub struct SpawnHandle {
    pub agent_id: String,
    pub child_chat_id: String,
    pub completion_rx: oneshot::Receiver<BackgroundAgent>,
}

fn tools_for_kind(kind: BgAgentKind) -> Option<Vec<String>> {
    match kind {
        BgAgentKind::Subagent => None,
        BgAgentKind::Delegate => Some(vec![
            "tree".to_string(),
            "cat".to_string(),
            "search_pattern".to_string(),
            "search_symbol_definition".to_string(),
            "search_semantic".to_string(),
            "knowledge".to_string(),
            "apply_patch".to_string(),
            "create_textdoc".to_string(),
            "update_textdoc".to_string(),
            "update_textdoc_anchored".to_string(),
            "update_textdoc_by_lines".to_string(),
            "update_textdoc_regex".to_string(),
            "undo_textdoc".to_string(),
            "mv".to_string(),
            "tasks_set".to_string(),
        ]),
    }
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
    let child_chat_id = format!("subchat-{}", Uuid::new_v4());
    let config_name = req.config_name.clone();
    let config_title = req.title.clone();
    let parent_chat_id = req.parent_chat_id.clone();
    let link_type = req.kind.as_str().to_string();
    let parent_root_chat_id = req.parent_root_chat_id.clone();
    let parent_task_meta = req.parent_task_meta.clone();
    let parent_worktree = req.parent_worktree.clone();
    let parent_tool_call_id = req.parent_tool_call_id.clone();
    let parent_subchat_tx = req.parent_subchat_tx.clone();
    let max_steps = req.max_steps;
    let tools = req.tools.clone().or_else(|| tools_for_kind(req.kind));
    let subchat_depth = req.subchat_depth;
    #[cfg(test)]
    let config = if config_name == "test_spawn" {
        SubchatConfig {
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
            worktree: parent_worktree.clone(),
            model: req.model.clone(),
            mode: "agent".to_string(),
            n_ctx: 4096,
            max_new_tokens: 512,
            temperature: None,
            reasoning_effort: None,
            parent_tool_call_id: parent_tool_call_id.clone(),
            parent_subchat_tx: parent_subchat_tx.clone(),
            abort_flag: None,
            subchat_depth: subchat_depth + 1,
            buddy_meta: None,
        }
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
            parent_worktree,
            parent_tool_call_id,
            parent_subchat_tx,
            None,
            subchat_depth + 1,
        )
        .await?
    };
    #[cfg(not(test))]
    let config = resolve_subchat_config_with_parent(
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
        parent_worktree,
        parent_tool_call_id,
        parent_subchat_tx,
        None,
        subchat_depth + 1,
    )
    .await?;

    let (record, abort_flag, _) = app
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
        })
        .await?;
    emit_background_agent_update(app.clone(), &record).await;

    let messages = build_messages(app.clone(), &req.config_name, &req.prompt).await?;
    let agent_id = record.agent_id.clone();
    let handle_agent_id = agent_id.clone();
    let handle_child_chat_id = child_chat_id.clone();
    let (completion_tx, completion_rx) = oneshot::channel();

    tokio::spawn(async move {
        let final_record = run_spawned_agent(
            app.clone(),
            req,
            config,
            messages,
            agent_id,
            child_chat_id,
            abort_flag,
        )
        .await;
        let _ = completion_tx.send(final_record);
    });

    Ok(SpawnHandle {
        agent_id: handle_agent_id,
        child_chat_id: handle_child_chat_id,
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
) -> BackgroundAgent {
    config.abort_flag = Some(abort_flag);
    let running = app
        .agents
        .mark_running(&agent_id, child_chat_id.clone())
        .await;
    if let Ok(record) = running.as_ref() {
        emit_background_agent_update(app.clone(), record).await;
    }

    let result = run_background_subchat(app.gcx.clone(), messages, config).await;
    let final_record = match result {
        Ok(result) => {
            let result_summary = result
                .messages
                .iter()
                .rev()
                .find(|message| message.role == "assistant")
                .map(|message| message.content.content_text_only())
                .unwrap_or_else(|| {
                    "Background agent completed but produced no response.".to_string()
                });
            let (edited_files, diff_summary, conflict_summary) =
                if req.kind == BgAgentKind::Delegate {
                    collect_delegate_changes(req.parent_worktree.as_ref()).await
                } else {
                    (Vec::new(), None, None)
                };
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

    match final_record {
        Ok(record) => {
            emit_background_agent_update(app.clone(), &record).await;
            if req.notify_parent == NotifyParent::Auto {
                let _ = crate::agents::push::push_completion_to_parent(app, &record).await;
            }
            record
        }
        Err(error) => fallback_failed_record(agent_id, req, error),
    }
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
            return runner(gcx, messages, config).await;
        }
    }
    crate::subchat::run_subchat(gcx, messages, config).await
}

async fn build_messages(
    app: AppState,
    config_name: &str,
    prompt: &str,
) -> Result<Vec<ChatMessage>, String> {
    #[cfg(test)]
    if config_name == "test_spawn" {
        return Ok(vec![
            ChatMessage::new("system".to_string(), "test system".to_string()),
            ChatMessage::new("user".to_string(), prompt.to_string()),
        ]);
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
    Ok(vec![
        ChatMessage {
            role: "system".to_string(),
            content: ChatContent::SimpleText(system_prompt),
            ..Default::default()
        },
        ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText(prompt.to_string()),
            ..Default::default()
        },
    ])
}

pub async fn emit_background_agent_update(app: AppState, record: &BackgroundAgent) {
    let session_arc = {
        let sessions = app.chat.sessions.read().await;
        sessions.get(&record.parent_chat_id).cloned()
    };
    let Some(session_arc) = session_arc else {
        return;
    };
    let mut session = session_arc.lock().await;
    if session.closed {
        return;
    }
    let seq = session.event_seq.saturating_add(1);
    session.emit(ChatEvent::BackgroundAgentUpdated {
        chat_id: record.parent_chat_id.clone(),
        seq,
        agent: record.into(),
    });
}

async fn collect_delegate_changes(
    worktree: Option<&WorktreeMeta>,
) -> (Vec<String>, Option<String>, Option<String>) {
    let Some(worktree) = worktree else {
        return (Vec::new(), None, None);
    };
    let root = worktree.root.clone();
    tokio::task::spawn_blocking(move || {
        let edited_files = git_lines(&root, &["status", "--porcelain"])
            .into_iter()
            .filter_map(|line| line.get(3..).map(str::trim).map(str::to_string))
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        let diff_summary =
            command_stdout(&root, &["diff", "--stat"]).filter(|text| !text.trim().is_empty());
        let conflict_summary = detect_conflicts(&root);
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

fn detect_conflicts(root: &Path) -> Option<String> {
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path
            .components()
            .any(|component| component.as_os_str() == ".git")
        {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        if content.contains("<<<<<<<") || content.contains("=======") || content.contains(">>>>>>>")
        {
            files.push(
                path.strip_prefix(root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string(),
            );
        }
    }
    if files.is_empty() {
        None
    } else {
        Some(format!("Conflict markers detected in {}", files.join(", ")))
    }
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
        created_at: now,
        started_at: None,
        finished_at: Some(now),
        last_update_at: now,
        change_seq: 1,
    }
}
