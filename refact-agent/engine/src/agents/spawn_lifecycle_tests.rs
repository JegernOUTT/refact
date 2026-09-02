use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use serial_test::serial;
use tempfile::TempDir;
use tokio::sync::{Mutex, Notify};

use crate::agents::spawn::{
    NotifyParent, SpawnGoal, SpawnHandle, SpawnRequest, SpawnWorktreeMode, TestRunnerGuard,
};
use crate::agents::types::{BackgroundAgent, BgAgentKind, BgAgentStatus};
use crate::app_state::AppState;
use crate::call_validation::ChatMessage;
use crate::chat::trajectories::{load_trajectory_for_chat, save_trajectory_as};
use crate::chat::types::{GoalBudget, GoalCriterion, SessionState, ThreadParams};
use crate::subchat::{
    finish_stateful_subchat_session, mirror_subchat_messages_into_session, SubchatConfig,
    SubchatProgress, SubchatResult,
};
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::WorktreeMeta;

struct RepoFixture {
    _temp: TempDir,
    source: PathBuf,
    cache: PathBuf,
    app: AppState,
    parent_worktree: WorktreeMeta,
}

fn run_git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|error| panic!("failed to run git {args:?}: {error}"));
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn git_succeeds(cwd: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .is_ok_and(|status| status.success())
}

fn init_repo(root: &Path) {
    run_git(root, &["init"]);
    run_git(root, &["checkout", "-b", "main"]);
    run_git(root, &["config", "core.autocrlf", "false"]);
    run_git(root, &["config", "user.email", "test@example.com"]);
    run_git(root, &["config", "user.name", "Test User"]);
    std::fs::write(root.join("base.txt"), "main\n").expect("write initial file");
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "initial"]);
    run_git(root, &["checkout", "-b", "work"]);
    std::fs::write(root.join("work-only.txt"), "work branch\n").expect("write work branch file");
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "work branch"]);
}

async fn repo_fixture() -> RepoFixture {
    let temp = tempfile::tempdir().expect("tempdir");
    let source = temp.path().join("repo");
    let cache = temp.path().join("cache");
    let config = temp.path().join("config");
    std::fs::create_dir_all(&source).expect("create source repo");
    init_repo(&source);
    let source = std::fs::canonicalize(source).expect("canonical source");
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(cache.clone(), config).await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
    let app = AppState::from_gcx(gcx).await;
    let parent_worktree = WorktreeMeta {
        id: "parent-work".to_string(),
        kind: "chat".to_string(),
        root: source.clone(),
        source_workspace_root: source.clone(),
        repo_root: source.clone(),
        branch: Some("work".to_string()),
        base_branch: Some("work".to_string()),
        base_commit: Some(run_git(&source, &["rev-parse", "HEAD"]).trim().to_string()),
        task_id: None,
        card_id: None,
        agent_id: None,
        enforce: false,
    };
    RepoFixture {
        _temp: temp,
        source,
        cache,
        app,
        parent_worktree,
    }
}

fn spawn_request(parent_chat_id: &str, kind: BgAgentKind) -> SpawnRequest {
    SpawnRequest {
        kind,
        parent_chat_id: parent_chat_id.to_string(),
        parent_root_chat_id: Some(parent_chat_id.to_string()),
        parent_tool_call_id: None,
        config_name: "test_spawn".to_string(),
        title: "Lifecycle test".to_string(),
        prompt: "Run the lifecycle test".to_string(),
        tools: None,
        target_files: Vec::new(),
        max_steps: 10,
        model: "openai/gpt-4o".to_string(),
        model_type: None,
        goal: None,
        plan: None,
        worktree_mode: SpawnWorktreeMode::Inherit,
        parent_subchat_tx: None,
        parent_worktree: None,
        parent_task_meta: None,
        subchat_depth: 0,
        notify_parent: NotifyParent::Silent,
    }
}

fn completed_result(mut messages: Vec<ChatMessage>, config: SubchatConfig) -> SubchatResult {
    messages.push(ChatMessage::new(
        "assistant".to_string(),
        "done".to_string(),
    ));
    SubchatResult {
        messages,
        metering: serde_json::Map::new(),
        chat_id: config.chat_id,
    }
}

async fn await_completion(handle: SpawnHandle) -> BackgroundAgent {
    tokio::time::timeout(Duration::from_secs(60), handle.completion_rx)
        .await
        .expect("agent completion timed out")
        .expect("agent completion channel")
}

async fn wait_for_record<F>(
    app: &AppState,
    parent_chat_id: &str,
    agent_id: &str,
    predicate: F,
) -> BackgroundAgent
where
    F: Fn(&BackgroundAgent) -> bool,
{
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let record = app
                .agents
                .get(parent_chat_id, agent_id)
                .await
                .expect("agent record");
            if predicate(&record) {
                return record;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("agent record did not reach expected state")
}

#[serial(test_runner)]
#[tokio::test]
async fn isolated_worktree_starts_from_parent_scope_branch() {
    let observed = Arc::new(Mutex::new(None));
    let _runner: TestRunnerGuard = {
        let observed = observed.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, messages, config| {
            let observed = observed.clone();
            Box::pin(async move {
                *observed.lock().await = config.worktree.clone();
                Ok(completed_result(messages, config))
            })
        }))
    };
    let fixture = repo_fixture().await;
    let mut request = spawn_request("parent-isolated-base", BgAgentKind::Subagent);
    request.parent_worktree = Some(fixture.parent_worktree.clone());
    request.worktree_mode = SpawnWorktreeMode::Isolated { auto_merge: false };

    let completed = await_completion(
        crate::agents::spawn::spawn_background_agent(fixture.app.clone(), request)
            .await
            .expect("spawn isolated agent"),
    )
    .await;
    let child = observed
        .lock()
        .await
        .clone()
        .expect("child worktree config");
    let branch = completed
        .worktree_branch
        .as_deref()
        .expect("child branch recorded");

    assert_eq!(completed.worktree_id.as_deref(), Some(child.id.as_str()));
    assert_eq!(child.branch.as_deref(), Some(branch));
    assert!(child.root.join("work-only.txt").exists());
    assert!(git_succeeds(
        &fixture.source,
        &["merge-base", "--is-ancestor", "work", branch]
    ));
}

#[serial(test_runner)]
#[tokio::test]
async fn spawned_test_subchat_session_carries_approval_defaults() {
    let observed = Arc::new(Mutex::new(None));
    let _runner = {
        let observed = observed.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |gcx, messages, config| {
            let observed = observed.clone();
            Box::pin(async move {
                let app = AppState::from_gcx(gcx).await;
                let chat_id = config.chat_id.as_deref().expect("stateful child chat id");
                let thread = app
                    .chat
                    .sessions
                    .read()
                    .await
                    .get(chat_id)
                    .cloned()
                    .expect("spawned child session")
                    .lock()
                    .await
                    .thread
                    .clone();
                *observed.lock().await = Some((
                    thread.autonomous_no_confirm,
                    thread.auto_approve_editing_tools,
                    thread.auto_approve_dangerous_commands,
                ));
                Ok(completed_result(messages, config))
            })
        }))
    };
    let fixture = repo_fixture().await;
    let completed = await_completion(
        crate::agents::spawn::spawn_background_agent(
            fixture.app,
            spawn_request("parent-approval-defaults", BgAgentKind::Subagent),
        )
        .await
        .expect("spawn test subagent"),
    )
    .await;

    assert_eq!(completed.status, BgAgentStatus::Completed);
    assert_eq!(*observed.lock().await, Some((true, true, true)));
}

#[serial(test_runner)]
#[tokio::test]
async fn isolated_auto_merge_merges_changes_and_cleans_worktree() {
    let _runner =
        crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, messages, config| {
            Box::pin(async move {
                let worktree = config.worktree.as_ref().expect("isolated worktree");
                std::fs::write(worktree.root.join("agent-change.txt"), "merged\n")
                    .expect("write isolated change");
                Ok(completed_result(messages, config))
            })
        }));
    let fixture = repo_fixture().await;
    let mut request = spawn_request("parent-auto-merge", BgAgentKind::Subagent);
    request.parent_worktree = Some(fixture.parent_worktree.clone());
    request.worktree_mode = SpawnWorktreeMode::Isolated { auto_merge: true };

    let completed = await_completion(
        crate::agents::spawn::spawn_background_agent(fixture.app.clone(), request)
            .await
            .expect("spawn auto-merge agent"),
    )
    .await;
    let worktree_id = completed.worktree_id.as_deref().expect("worktree id");
    let service =
        WorktreeService::new(fixture.cache.clone(), fixture.source.clone()).expect("service");

    assert_eq!(completed.merge_status.as_deref(), Some("merged"));
    assert_eq!(
        std::fs::read_to_string(fixture.source.join("agent-change.txt")).unwrap(),
        "merged\n"
    );
    assert!(!service.worktree_path_for_id(worktree_id).unwrap().exists());
}

#[serial(test_runner)]
#[tokio::test]
async fn nested_isolated_auto_merge_runs_inside_parent_worktree() {
    let fixture = repo_fixture().await;
    let service =
        WorktreeService::new(fixture.cache.clone(), fixture.source.clone()).expect("service");
    let parent = service
        .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
            branch: Some("refact/subagent/parent".to_string()),
            base_branch: Some("work".to_string()),
            chat_id: Some("parent-worktree".to_string()),
            kind: Some("subagent".to_string()),
            ..Default::default()
        })
        .await
        .expect("create parent worktree");
    let parent_worktree = parent.worktree.meta;
    let _runner =
        crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, messages, config| {
            Box::pin(async move {
                let worktree = config.worktree.as_ref().expect("isolated worktree");
                std::fs::write(worktree.root.join("nested-agent-change.txt"), "merged\n")
                    .expect("write nested isolated change");
                Ok(completed_result(messages, config))
            })
        }));
    let mut request = spawn_request("parent-nested-auto-merge", BgAgentKind::Subagent);
    request.parent_worktree = Some(parent_worktree.clone());
    request.worktree_mode = SpawnWorktreeMode::Isolated { auto_merge: true };

    let completed = await_completion(
        crate::agents::spawn::spawn_background_agent(fixture.app.clone(), request)
            .await
            .expect("spawn nested agent"),
    )
    .await;

    assert_eq!(completed.merge_status.as_deref(), Some("merged"));
    assert_eq!(
        std::fs::read_to_string(parent_worktree.root.join("nested-agent-change.txt")).unwrap(),
        "merged\n"
    );
    assert!(!fixture.source.join("nested-agent-change.txt").exists());
}

#[serial(test_runner)]
#[tokio::test]
async fn isolated_auto_merge_conflict_preserves_worktree_and_summary() {
    let fixture = repo_fixture().await;
    let source = fixture.source.clone();
    let _runner =
        crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, messages, config| {
            let source = source.clone();
            Box::pin(async move {
                let worktree = config.worktree.as_ref().expect("isolated worktree");
                std::fs::write(worktree.root.join("conflict.txt"), "agent side\n")
                    .expect("write agent conflict");
                std::fs::write(source.join("conflict.txt"), "base side\n")
                    .expect("write base conflict");
                run_git(&source, &["add", "conflict.txt"]);
                run_git(&source, &["commit", "-m", "base conflict"]);
                Ok(completed_result(messages, config))
            })
        }));
    let mut request = spawn_request("parent-auto-conflict", BgAgentKind::Subagent);
    request.parent_worktree = Some(fixture.parent_worktree.clone());
    request.worktree_mode = SpawnWorktreeMode::Isolated { auto_merge: true };

    let completed = await_completion(
        crate::agents::spawn::spawn_background_agent(fixture.app.clone(), request)
            .await
            .expect("spawn conflicting agent"),
    )
    .await;
    let worktree_id = completed.worktree_id.as_deref().expect("worktree id");
    let service =
        WorktreeService::new(fixture.cache.clone(), fixture.source.clone()).expect("service");

    assert_eq!(completed.merge_status.as_deref(), Some("conflict"));
    assert!(completed
        .conflict_summary
        .as_deref()
        .is_some_and(|summary| !summary.is_empty()));
    assert!(service.worktree_path_for_id(worktree_id).unwrap().exists());
}

#[serial(test_runner)]
#[tokio::test]
async fn isolated_worktree_without_auto_merge_is_skipped_and_retained() {
    let _runner =
        crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, messages, config| {
            Box::pin(async move { Ok(completed_result(messages, config)) })
        }));
    let fixture = repo_fixture().await;
    let mut request = spawn_request("parent-skip-merge", BgAgentKind::Subagent);
    request.parent_worktree = Some(fixture.parent_worktree.clone());
    request.worktree_mode = SpawnWorktreeMode::Isolated { auto_merge: false };

    let completed = await_completion(
        crate::agents::spawn::spawn_background_agent(fixture.app.clone(), request)
            .await
            .expect("spawn no-merge agent"),
    )
    .await;
    let worktree_id = completed.worktree_id.as_deref().expect("worktree id");
    let service =
        WorktreeService::new(fixture.cache.clone(), fixture.source.clone()).expect("service");

    assert_eq!(completed.merge_status.as_deref(), Some("skipped"));
    assert!(completed.worktree_branch.is_some());
    assert!(service.worktree_path_for_id(worktree_id).unwrap().exists());
}

#[serial(test_runner)]
#[tokio::test]
async fn inherit_mode_keeps_parent_worktree_and_has_no_merge_status() {
    let observed = Arc::new(Mutex::new(None));
    let _runner = {
        let observed = observed.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, messages, config| {
            let observed = observed.clone();
            Box::pin(async move {
                *observed.lock().await = config.worktree.clone();
                Ok(completed_result(messages, config))
            })
        }))
    };
    let fixture = repo_fixture().await;
    let mut request = spawn_request("parent-inherit", BgAgentKind::Subagent);
    request.parent_worktree = Some(fixture.parent_worktree.clone());

    let completed = await_completion(
        crate::agents::spawn::spawn_background_agent(fixture.app.clone(), request)
            .await
            .expect("spawn inherited agent"),
    )
    .await;

    assert_eq!(
        *observed.lock().await,
        Some(fixture.parent_worktree.clone())
    );
    assert_eq!(completed.worktree_id.as_deref(), Some("parent-work"));
    assert_eq!(completed.merge_status, None);
}

#[serial(test_runner)]
#[tokio::test]
async fn sibling_lifecycle_notices_arrive_in_running_sibling_inbox() {
    let first_started = Arc::new(Notify::new());
    let release_first = Arc::new(Notify::new());
    let _runner = {
        let first_started = first_started.clone();
        let release_first = release_first.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, messages, config| {
            let first_started = first_started.clone();
            let release_first = release_first.clone();
            Box::pin(async move {
                if config.title.as_deref() == Some("First sibling") {
                    first_started.notify_one();
                    release_first.notified().await;
                }
                Ok(completed_result(messages, config))
            })
        }))
    };
    let fixture = repo_fixture().await;
    let mut first_request = spawn_request("parent-siblings", BgAgentKind::Subagent);
    first_request.title = "First sibling".to_string();
    let first = crate::agents::spawn::spawn_background_agent(fixture.app.clone(), first_request)
        .await
        .expect("spawn first sibling");
    tokio::time::timeout(Duration::from_secs(30), first_started.notified())
        .await
        .expect("first sibling started");

    let mut second_request = spawn_request("parent-siblings", BgAgentKind::Subagent);
    second_request.title = "Second sibling".to_string();
    second_request.target_files = vec!["src/second.rs".to_string()];
    let second = crate::agents::spawn::spawn_background_agent(fixture.app.clone(), second_request)
        .await
        .expect("spawn second sibling");
    let second_completed = await_completion(second).await;
    let inbox = fixture
        .app
        .agents
        .inbox_for(&first.agent_id)
        .await
        .expect("first sibling inbox");
    let texts = inbox
        .lock()
        .await
        .iter()
        .map(|message| message.text.clone())
        .collect::<Vec<_>>();

    assert!(texts
        .iter()
        .any(|text| text.contains("▸ sibling started: Second sibling")));
    assert!(texts.iter().any(|text| {
        text.contains("✓ sibling finished: Second sibling")
            && text.contains(second_completed.status.as_str())
    }));

    release_first.notify_one();
    let first_completed = await_completion(first).await;
    assert_eq!(first_completed.status, BgAgentStatus::Completed);
}

#[serial(test_runner)]
#[tokio::test]
async fn progress_callback_updates_live_tool_usage_and_cost() {
    let tool_started = Arc::new(Notify::new());
    let release_runner = Arc::new(Notify::new());
    let _runner = {
        let tool_started = tool_started.clone();
        let release_runner = release_runner.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, messages, config| {
            let tool_started = tool_started.clone();
            let release_runner = release_runner.clone();
            Box::pin(async move {
                let progress = config.step_progress.as_ref().expect("progress callback");
                progress(SubchatProgress::ToolStarted {
                    name: "shell".to_string(),
                    arg_preview: Some("cargo test…".to_string()),
                });
                tool_started.notify_one();
                release_runner.notified().await;
                progress(SubchatProgress::Usage {
                    tokens_delta: 123,
                    cost_delta: Some(0.0123),
                });
                progress(SubchatProgress::ToolsFinished);
                Ok(completed_result(messages, config))
            })
        }))
    };
    let fixture = repo_fixture().await;
    let handle = crate::agents::spawn::spawn_background_agent(
        fixture.app.clone(),
        spawn_request("parent-progress", BgAgentKind::Subagent),
    )
    .await
    .expect("spawn reporting agent");
    tokio::time::timeout(Duration::from_secs(30), tool_started.notified())
        .await
        .expect("tool progress emitted");
    let live = wait_for_record(
        &fixture.app,
        "parent-progress",
        &handle.agent_id,
        |record| record.current_tool.as_deref() == Some("shell: cargo test…"),
    )
    .await;

    assert_eq!(live.current_tool.as_deref(), Some("shell: cargo test…"));
    release_runner.notify_one();
    let completed = await_completion(handle).await;
    let reported = wait_for_record(
        &fixture.app,
        "parent-progress",
        &completed.agent_id,
        |record| record.tokens_used == 123 && record.current_tool.is_none(),
    )
    .await;

    assert_eq!(reported.tokens_used, 123);
    assert_eq!(reported.cost_usd, Some(0.0123));
}

#[serial(test_runner)]
#[tokio::test]
async fn goal_seed_and_projection_survive_trajectory_reload() {
    let observed = Arc::new(Mutex::new(None));
    let _runner = {
        let observed = observed.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |gcx, messages, config| {
            let observed = observed.clone();
            Box::pin(async move {
                *observed.lock().await = Some((messages.clone(), config.max_steps));
                let chat_id = config.chat_id.clone().expect("child chat id");
                let thread = ThreadParams {
                    id: chat_id,
                    title: config.title.clone().unwrap_or_default(),
                    model: config.model.clone(),
                    mode: config.mode.clone(),
                    parent_id: config.parent_id.clone(),
                    link_type: config.link_type.clone(),
                    root_chat_id: config.root_chat_id.clone(),
                    ..Default::default()
                };
                save_trajectory_as(gcx, &thread, &messages).await;
                Ok(completed_result(messages, config))
            })
        }))
    };
    let fixture = repo_fixture().await;
    let mut request = spawn_request("parent-goal-reload", BgAgentKind::Subagent);
    request.goal = Some(SpawnGoal {
        content: "Ship lifecycle coverage".to_string(),
        criteria: vec![GoalCriterion {
            id: "C1".to_string(),
            text: "All paths are covered".to_string(),
            verify_hint: Some("Run the lifecycle suite".to_string()),
        }],
        budget: Some(GoalBudget {
            max_turns: Some(3),
            ..Default::default()
        }),
    });

    let completed = await_completion(
        crate::agents::spawn::spawn_background_agent(fixture.app.clone(), request)
            .await
            .expect("spawn goal agent"),
    )
    .await;
    let (messages, max_steps) = observed.lock().await.take().expect("captured seed");
    let goal_message = messages
        .iter()
        .find(|message| message.role == "goal")
        .expect("hidden goal seed");
    let child_chat_id = completed.child_chat_id.as_deref().expect("child chat id");
    let loaded = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Some(loaded) =
                load_trajectory_for_chat(fixture.app.gcx.clone(), child_chat_id).await
            {
                if loaded.goal.is_some() {
                    return loaded;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("persisted child trajectory with goal projection");
    let goal = loaded.goal.expect("goal projection after reload");

    assert_eq!(max_steps, 3);
    assert_eq!(goal_message.extra["goal"]["budget"]["max_turns"], 3);
    assert_eq!(goal_message.extra["goal"]["criteria"][0]["id"], "C1");
    assert_eq!(goal.content, "Ship lifecycle coverage");
    assert_eq!(goal.budget.max_turns, Some(3));
    assert_eq!(goal.criteria[0].id, "C1");
}

#[serial(test_runner)]
#[tokio::test]
async fn stateful_spawn_installs_goal_session_before_runner_executes() {
    let session_ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let session_ready_runner = session_ready.clone();
    let fixture = repo_fixture().await;
    let gcx = fixture.app.gcx.clone();
    let _runner =
        crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, messages, config| {
            let gcx = gcx.clone();
            let session_ready = session_ready_runner.clone();
            Box::pin(async move {
                let chat_id = config.chat_id.as_deref().expect("stateful child chat id");
                let session = gcx.chat_sessions.read().await.get(chat_id).cloned();
                let has_goal = match session {
                    Some(session) => session.lock().await.goal.is_some(),
                    None => false,
                };
                session_ready.store(has_goal, std::sync::atomic::Ordering::SeqCst);
                Ok(completed_result(messages, config))
            })
        }));
    let mut request = spawn_request("parent-goal-session", BgAgentKind::Subagent);
    request.goal = Some(SpawnGoal {
        content: "Install the goal projection".to_string(),
        criteria: Vec::new(),
        budget: None,
    });

    let completed = await_completion(
        crate::agents::spawn::spawn_background_agent(fixture.app.clone(), request)
            .await
            .expect("spawn goal agent"),
    )
    .await;

    assert_eq!(completed.status, BgAgentStatus::Completed);
    assert!(session_ready.load(std::sync::atomic::Ordering::SeqCst));
}

#[serial(test_runner)]
#[tokio::test]
async fn subagent_worktree_changes_are_collected_without_delegate_kind() {
    let _runner =
        crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, messages, config| {
            Box::pin(async move {
                let worktree = config.worktree.as_ref().expect("inherited worktree");
                std::fs::write(worktree.root.join("subagent-change.txt"), "edited\n")
                    .expect("write subagent change");
                Ok(completed_result(messages, config))
            })
        }));
    let fixture = repo_fixture().await;
    let mut request = spawn_request("parent-subagent-changes", BgAgentKind::Subagent);
    request.parent_worktree = Some(fixture.parent_worktree.clone());

    let completed = await_completion(
        crate::agents::spawn::spawn_background_agent(fixture.app.clone(), request)
            .await
            .expect("spawn subagent"),
    )
    .await;

    assert_eq!(completed.status, BgAgentStatus::Completed);
    assert!(completed
        .edited_files
        .iter()
        .any(|path| path == "subagent-change.txt"));
}

#[serial(test_runner)]
#[tokio::test]
async fn registered_subagent_session_mirrors_runner_messages_while_generating() {
    let observed_live = Arc::new(Mutex::new(None));
    let _runner = {
        let observed_live = observed_live.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |gcx, messages, config| {
            let observed_live = observed_live.clone();
            Box::pin(async move {
                let app = AppState::from_gcx(gcx).await;
                let chat_id = config.chat_id.clone().expect("stateful child chat id");
                let seed_len = messages.len();

                let mut committed = messages;
                committed.push(ChatMessage::new(
                    "assistant".to_string(),
                    "runner step".to_string(),
                ));
                committed.push(ChatMessage::new(
                    "user".to_string(),
                    "runner follow-up".to_string(),
                ));
                mirror_subchat_messages_into_session(&app, &chat_id, &committed).await;

                let live = app
                    .chat
                    .sessions
                    .read()
                    .await
                    .get(&chat_id)
                    .cloned()
                    .expect("registered subagent session");
                {
                    let live = live.lock().await;
                    *observed_live.lock().await = Some((
                        seed_len,
                        live.messages.len(),
                        live.runtime.state,
                        live.trajectory_dirty,
                    ));
                }

                let mut final_messages = committed;
                final_messages.push(ChatMessage::new(
                    "assistant".to_string(),
                    "done".to_string(),
                ));
                mirror_subchat_messages_into_session(&app, &chat_id, &final_messages).await;
                finish_stateful_subchat_session(&app, &chat_id, &config).await;
                Ok(SubchatResult {
                    messages: final_messages,
                    metering: serde_json::Map::new(),
                    chat_id: config.chat_id,
                })
            })
        }))
    };
    let fixture = repo_fixture().await;
    let completed = await_completion(
        crate::agents::spawn::spawn_background_agent(
            fixture.app.clone(),
            spawn_request("parent-session-mirror", BgAgentKind::Subagent),
        )
        .await
        .expect("spawn mirroring agent"),
    )
    .await;
    let (seed_len, live_len, live_state, live_dirty) =
        observed_live.lock().await.take().expect("live observation");
    let child_chat_id = completed.child_chat_id.as_deref().expect("child chat id");
    let session = fixture
        .app
        .chat
        .sessions
        .read()
        .await
        .get(child_chat_id)
        .cloned()
        .expect("registered subagent session after completion");
    let session = session.lock().await;

    assert_eq!(completed.status, BgAgentStatus::Completed);
    assert!(seed_len > 0);
    assert_eq!(live_len, seed_len + 2);
    assert_eq!(live_state, SessionState::Generating);
    assert!(!live_dirty);
    assert_eq!(session.messages.len(), seed_len + 3);
    assert_eq!(
        session.messages.last().map(|message| message.role.clone()),
        Some("assistant".to_string())
    );
    assert_eq!(session.runtime.state, SessionState::Idle);
    assert!(!session.trajectory_dirty);
}

#[serial(test_runner)]
#[tokio::test]
async fn dirty_non_runner_owned_session_is_not_overwritten_by_the_mirror() {
    let fixture = repo_fixture().await;
    let chat_id = "mirror-dirty-guard";
    let session = Arc::new(Mutex::new(crate::chat::types::ChatSession::new(
        chat_id.to_string(),
    )));
    {
        let mut guarded = session.lock().await;
        guarded.add_message(ChatMessage::new("user".to_string(), "mine".to_string()));
        assert!(guarded.trajectory_dirty);
    }
    fixture
        .app
        .chat
        .sessions
        .write()
        .await
        .insert(chat_id.to_string(), session.clone());

    mirror_subchat_messages_into_session(
        &fixture.app,
        chat_id,
        &[ChatMessage::new(
            "assistant".to_string(),
            "runner".to_string(),
        )],
    )
    .await;

    let session = session.lock().await;
    assert_eq!(session.messages.len(), 1);
    assert_eq!(
        session.messages[0].content.content_text_only(),
        "mine".to_string()
    );
}
