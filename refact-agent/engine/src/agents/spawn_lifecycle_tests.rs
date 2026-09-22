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
    let (project_root, _) = crate::caps::project_model_defaults_state(gcx.clone()).await;
    {
        let mut caps = crate::caps::CodeAssistantCaps::default();
        caps.chat_models.insert(
            "openai/gpt-4o".into(),
            Arc::new(crate::caps::ChatModelRecord {
                base: crate::caps::BaseModelRecord {
                    id: "openai/gpt-4o".into(),
                    ..Default::default()
                },
                supports_tools: true,
                supports_agent: true,
                ..Default::default()
            }),
        );
        let mut state = gcx.caps_state.write().await;
        state.caps = Some(Arc::new(caps));
        state.loaded_project_root = Some(project_root);
        state.last_attempted_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
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
        native_supervision_guard: None,
        kind,
        parent_chat_id: parent_chat_id.to_string(),
        parent_root_chat_id: Some(parent_chat_id.to_string()),
        parent_tool_call_id: None,
        config_name: "test_spawn".to_string(),
        title: "Lifecycle test".to_string(),
        prompt: "Run the lifecycle test".to_string(),
        initial_context_messages: Vec::new(),
        history_source_model: None,
        custom_instructions: None,
        tools: None,
        target_files: Vec::new(),
        max_steps: 10,
        model: "openai/gpt-4o".to_string(),
        model_type: None,
        goal: None,
        plan: None,
        deliverable: None,

        profile: None,
        worktree_mode: SpawnWorktreeMode::Inherit,
        parent_subchat_tx: None,
        parent_worktree: None,
        parent_task_meta: None,
        subchat_depth: 0,
        notify_parent: NotifyParent::Silent,
        completion_push: Default::default(),
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
        aborted: false,
    }
}

#[serial(test_runner)]
#[tokio::test]
async fn queued_spawn_is_discoverable_only_with_permitted_launch_provenance() {
    use refact_work_core::{AdmissionRequest, WorkKey, WorkScope, CoordinationSettings};
    use crate::coordination::{scope::WorkCaller, service, types::WorkOverviewRequest};
    let fixture = repo_fixture().await;
    fixture
        .app
        .work_scheduler
        .configure(CoordinationSettings {
            max_active_engine: 1,
            max_active_project: 1,
            max_active_root: 1,
            ..Default::default()
        })
        .unwrap();
    let project = crate::coordination::roots::project_identity(&fixture.source);
    let permit = fixture
        .app
        .work_scheduler
        .reserve(AdmissionRequest::new(
            WorkKey::trajectory("blocker").run(1),
            WorkScope::new(
                refact_work_core::ProjectKey::new(project),
                refact_work_core::CheckoutKey::new("blocker"),
            ),
        ))
        .unwrap()
        .acquire()
        .await
        .unwrap();
    let mut parent = crate::chat::types::ChatSession::new("launch-parent".into());
    parent.work_project_root = Some(fixture.source.clone());
    *fixture.app.gcx.privacy_settings.write().unwrap() =
        Arc::new(refact_core::privacy_types::PrivacySettings {
            privacy_rules: refact_core::privacy_types::FilePrivacySettings {
                only_send_to_servers_I_control: Vec::new(),
                blocked: Vec::new(),
            },
            loaded_ts: u64::MAX / 2,
        });
    let mut source = ChatMessage::new("tool".into(), "guarded context".into());
    source.extra.insert(
        "privacy".into(),
        serde_json::json!({"files":[{
            "path":fixture.source.join("guarded.rs"), "zone":"guarded", "attribution":"declared"
        }]}),
    );
    parent.add_message(source);
    fixture
        .app
        .chat
        .sessions
        .write()
        .await
        .insert("launch-parent".into(), Arc::new(Mutex::new(parent)));
    fixture.app.gcx.privacy_policy_load.write().unwrap().policy =
        Arc::new(refact_privacy::PrivacyPolicy {
            zones: vec![refact_privacy::Zone {
                name: "guarded".into(),
                patterns: vec!["**/guarded.rs".into()],
                send_to: vec!["trusted".into()],
                on_shell_read: refact_privacy::ShellBehavior::Withhold,
            }],
            ..Default::default()
        });
    let handle = crate::agents::spawn::spawn_background_agent(
        fixture.app.clone(),
        spawn_request("launch-parent", BgAgentKind::Subagent),
    )
    .await
    .unwrap();
    let mut caller = WorkCaller {
        source_root: fixture.source.clone(),
        checkout_root: fixture.source.clone(),
        root_chat_id: Some("observer".into()),
        principal: "observer".into(),
        model: Some(crate::caps::BaseModelRecord {
            id: "trusted/model".into(),
            ..Default::default()
        }),
    };
    let page = service::overview(&fixture.app, &caller, WorkOverviewRequest::default())
        .await
        .unwrap();
    let row = page
        .rows
        .iter()
        .find(|row| row.key.id == handle.agent_id)
        .unwrap();
    assert_eq!(row.state, crate::coordination::types::WorkStateTag::Queued);
    assert_eq!(row.title.as_deref(), Some("Lifecycle test"));
    assert!(!fixture
        .app
        .chat
        .sessions
        .read()
        .await
        .contains_key(&handle.child_chat_id));
    caller.model.as_mut().unwrap().id = "other/model".into();
    let denied = service::overview(&fixture.app, &caller, WorkOverviewRequest::default())
        .await
        .unwrap();
    assert!(!serde_json::to_string(&denied)
        .unwrap()
        .contains("Lifecycle test"));
    fixture
        .app
        .work_scheduler
        .cancel(&WorkKey::background_agent(&handle.agent_id).run(1));
    let id = handle.agent_id.clone();
    await_completion(handle).await;
    assert!(fixture
        .app
        .agent_executions
        .launch_provenance(&id)
        .is_none());
    drop(permit);
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
async fn spawned_subchat_approval_defaults_never_exceed_parent_authority() {
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
    for (parent_id, authority, expected) in [
        ("missing-parent", None, (false, false, false)),
        (
            "unprivileged-parent",
            Some((false, false, false)),
            (false, false, false),
        ),
        (
            "editing-parent",
            Some((false, true, false)),
            (false, true, false),
        ),
        (
            "autonomous-parent",
            Some((true, false, false)),
            (true, true, true),
        ),
    ] {
        if let Some((autonomous, editing, dangerous)) = authority {
            let mut parent = crate::chat::types::ChatSession::new(parent_id.into());
            parent.thread.autonomous_no_confirm = autonomous;
            parent.thread.auto_approve_editing_tools = editing;
            parent.thread.auto_approve_dangerous_commands = dangerous;
            fixture
                .app
                .chat
                .sessions
                .write()
                .await
                .insert(parent_id.into(), Arc::new(Mutex::new(parent)));
        }
        let completed = await_completion(
            crate::agents::spawn::spawn_background_agent(
                fixture.app.clone(),
                spawn_request(parent_id, BgAgentKind::Subagent),
            )
            .await
            .expect("spawn test subagent"),
        )
        .await;
        assert_eq!(completed.status, BgAgentStatus::Completed);
        assert_eq!(*observed.lock().await, Some(expected), "{parent_id}");
    }
}

#[serial(test_runner, runtime_settings)]
#[tokio::test]
async fn deliverable_contract_unmet_spends_one_bounded_correction_then_reports_unmet() {
    crate::runtime_settings::reset_for_test();
    let mut settings = crate::runtime_settings::TrajectoryRuntimeSettings::default();
    settings.coordination.deliverable_result_budget_tokens = 64;
    crate::runtime_settings::install_live(&settings);

    let runs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let observed = runs.clone();
    let _runner =
        crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, messages, config| {
            let runs = runs.clone();
            Box::pin(async move {
                runs.lock().await.push(
                    messages
                        .last()
                        .map(|message| message.content.content_text_only())
                        .unwrap_or_default(),
                );
                let mut messages = messages;
                messages.push(ChatMessage::new(
                    "assistant".to_string(),
                    "# Findings\nnothing yet\n".to_string(),
                ));
                Ok(SubchatResult {
                    messages,
                    metering: serde_json::Map::new(),
                    chat_id: config.chat_id,
                    aborted: false,
                })
            })
        }));

    let fixture = repo_fixture().await;
    let mut request = spawn_request("parent-deliverable", BgAgentKind::Subagent);
    request.deliverable = Some(refact_chat_api::DeliverableContract {
        required_sections: vec!["Findings".into(), "Changes".into()],
        output_schema: None,
        evidence_required: true,
    });

    let completed = await_completion(
        crate::agents::spawn::spawn_background_agent(fixture.app.clone(), request)
            .await
            .expect("spawn deliverable agent"),
    )
    .await;

    let status = completed
        .deliverable_status
        .expect("deliverable status recorded");
    assert!(!status.met);
    assert_eq!(status.corrections_used, 1);
    assert!(status.corrections_exhausted);
    assert!(status.omission.is_none());
    assert_eq!(status.missing_sections, vec!["Changes"]);
    assert!(status.evidence_missing);

    let runs = observed.lock().await;
    assert_eq!(runs.len(), 2, "exactly one bounded correction attempt");
    assert!(
        runs[1].contains("Deliverable correction 1 of 1"),
        "{}",
        runs[1]
    );
    assert!(runs[1].contains("`# Changes`"), "{}", runs[1]);
    assert!(runs[1].contains("`# Evidence`"), "{}", runs[1]);
    assert!(!runs[1].contains("`# Findings`"), "{}", runs[1]);
    drop(runs);
    crate::runtime_settings::reset_for_test();
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
async fn sibling_lifecycle_notices_arrive_in_running_sibling_deliveries() {
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
    let deliveries = fixture.app.agents.pending_deliveries(&first.agent_id).await;
    assert_eq!(deliveries.len(), 2);
    assert!(deliveries
        .iter()
        .all(|delivery| delivery.push == refact_core::chat_types::PushMode::Append));
    let texts = deliveries
        .iter()
        .flat_map(|delivery| delivery.messages.iter())
        .map(|message| message.content.content_text_only())
        .collect::<Vec<_>>();

    assert!(texts
        .iter()
        .any(|text| text.contains("▸ sibling queued: Second sibling")));
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
            verify_command: None,
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
async fn capped_child_resumes_once_with_full_history_and_remaining_lifetime_budget() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use refact_agents_core::types::CapStop;

    const PARENT: &str = "parent-cap-resume";
    const PLAN: &str = "Preserve the first edit, then finish the second edit.";
    let attempts = Arc::new(AtomicUsize::new(0));
    let stopped_context = Arc::new(Mutex::new(None));
    let resumed_started = Arc::new(Notify::new());
    let release_resumed = Arc::new(Notify::new());
    let fixture = repo_fixture().await;
    assert!(
        crate::runtime_settings::current()
            .coordination
            .subagent_max_steps
            >= 20
    );
    let mut parent = crate::chat::types::ChatSession::new(PARENT.to_string());
    parent.thread.worktree = Some(fixture.parent_worktree.clone());
    // Queue completion pushes without starting a real parent provider turn.
    parent.set_runtime_state(SessionState::Generating, None);
    fixture
        .app
        .chat
        .sessions
        .write()
        .await
        .insert(PARENT.to_string(), Arc::new(Mutex::new(parent)));
    let _runner = {
        let attempts = attempts.clone();
        let stopped_context = stopped_context.clone();
        let resumed_started = resumed_started.clone();
        let release_resumed = release_resumed.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |gcx, mut messages, config| {
            let attempts = attempts.clone();
            let stopped_context = stopped_context.clone();
            let resumed_started = resumed_started.clone();
            let release_resumed = release_resumed.clone();
            Box::pin(async move {
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                let app = AppState::from_gcx(gcx.clone()).await;
                let chat_id = config.chat_id.clone().expect("same stateful child id");
                let agent_id = config.background_agent_id.as_deref().expect("agent id");
                let worktree = config.worktree.as_ref().expect("original project scope");
                let progress = config.step_progress.as_ref().expect("progress callback");
                assert_eq!(
                    config.max_steps, 10,
                    "only the remaining steps reach the runner"
                );
                assert_eq!(config.parent_id.as_deref(), Some(PARENT));
                assert!(messages.iter().any(|message| {
                    message.role == "plan" && message.content.content_text_only() == PLAN
                }));
                let session = app
                    .chat
                    .sessions
                    .read()
                    .await
                    .get(&chat_id)
                    .cloned()
                    .expect("registered child session");
                let thread = session.lock().await.thread.clone();
                if attempt == 0 {
                    // Real runner output has stable IDs/indexes before saving.
                    // Seed them here too so the assertion checks exact history
                    // preservation, not the loader's legacy-record migration.
                    for (index, message) in messages.iter_mut().enumerate() {
                        if message.message_id.is_empty() {
                            message.message_id = format!("cap-seed-{index}");
                        }
                    }
                    std::fs::write(worktree.root.join("before-cap.txt"), "first edit\n")
                        .expect("write pre-cap edit");
                    for step in 1..=10 {
                        let call_id = format!("cap-tool-{step}");
                        let mut call = ChatMessage::new("assistant".into(), format!("step {step}"));
                        call.message_id = format!("cap-call-message-{step}");
                        call.tool_calls = Some(serde_json::from_value(serde_json::json!([{
                            "id": call_id,
                            "index": 0,
                            "type": "function",
                            "function": {"name": "cat", "arguments": "{\"paths\":\"before-cap.txt\"}"}
                        }])).expect("fake tool call"));
                        let mut result = ChatMessage::new("tool".into(), "first edit\n".into());
                        result.message_id = format!("cap-result-message-{step}");
                        result.tool_call_id = call_id;
                        messages.extend([call, result]);
                    }
                    progress(SubchatProgress::Step(10));
                    wait_for_record(&app, PARENT, agent_id, |record| record.step_count == 10).await;
                    let stop = CapStop {
                        key: refact_work_core::CapKind::Steps,
                        scope: refact_work_core::CapScope::Child,
                        limit: 10,
                        used: 10,
                        resumable: true,
                        run_generation: 1,
                    };
                    messages.push(crate::chat::internal_roles::event(
                        crate::chat::internal_roles::EventSubkind::SystemNotice,
                        "agents.cap_stop",
                        serde_json::json!({"agent_id": agent_id, "cap_stop": stop}),
                        "Child reached its step cap.",
                    ));
                    *stopped_context.lock().await = Some((
                        chat_id.clone(),
                        worktree.clone(),
                        serde_json::to_value(&messages).unwrap(),
                    ));
                    save_trajectory_as(gcx, &thread, &messages).await;
                    mirror_subchat_messages_into_session(&app, &chat_id, &messages).await;
                    finish_stateful_subchat_session(&app, &chat_id, &config).await;
                    return Err(stop.as_error().expect("typed cap error"));
                }
                assert_eq!(
                    attempt, 1,
                    "a capped logical run has exactly one continuation"
                );
                let (original_chat, original_worktree, original_messages) = stopped_context
                    .lock()
                    .await
                    .clone()
                    .expect("stopped context");
                assert_eq!(chat_id, original_chat);
                assert_eq!(*worktree, original_worktree);
                let original_len = original_messages.as_array().unwrap().len();
                assert_eq!(
                    serde_json::to_value(&messages[..original_len]).unwrap(),
                    original_messages
                );
                assert_eq!(
                    messages[original_len].extra["event"]["source"],
                    "agents.resume"
                );
                assert!(!config.prepend_system_prompt);
                assert_eq!(
                    std::fs::read_to_string(worktree.root.join("before-cap.txt")).unwrap(),
                    "first edit\n"
                );
                assert_eq!(
                    app.agent_executions
                        .saved_config(agent_id)
                        .expect("lifetime config")
                        .max_steps,
                    20
                );
                progress(SubchatProgress::Step(1));
                wait_for_record(&app, PARENT, agent_id, |record| record.step_count == 11).await;
                resumed_started.notify_one();
                release_resumed.notified().await;
                std::fs::write(worktree.root.join("after-cap.txt"), "second edit\n")
                    .expect("write continuation edit");
                messages.push(ChatMessage::new(
                    "assistant".into(),
                    "Both edits are complete.".into(),
                ));
                save_trajectory_as(gcx, &thread, &messages).await;
                mirror_subchat_messages_into_session(&app, &chat_id, &messages).await;
                finish_stateful_subchat_session(&app, &chat_id, &config).await;
                Ok(SubchatResult {
                    messages,
                    metering: serde_json::Map::new(),
                    chat_id: Some(chat_id),
                    aborted: false,
                })
            })
        }))
    };
    let mut request = spawn_request(PARENT, BgAgentKind::Subagent);
    request.parent_worktree = Some(fixture.parent_worktree.clone());
    request.plan = Some(PLAN.into());
    request.target_files = vec!["before-cap.txt".into(), "after-cap.txt".into()];
    request.worktree_mode = SpawnWorktreeMode::Isolated { auto_merge: false };
    request.notify_parent = NotifyParent::Auto;
    let stopped = await_completion(
        crate::agents::spawn::spawn_background_agent(fixture.app.clone(), request)
            .await
            .expect("spawn capped child"),
    )
    .await;
    assert_eq!(stopped.status, BgAgentStatus::Failed);
    assert_eq!(stopped.step_count, 10);
    assert_eq!(
        CapStop::from_error(stopped.error.as_deref().expect("cap stop"))
            .expect("typed stop")
            .used,
        10
    );
    let child_chat_id = stopped.child_chat_id.as_deref().expect("original child id");
    assert!(fixture.app.agent_executions.get(child_chat_id).is_none());
    let latest = fixture
        .app
        .agents
        .get(PARENT, &stopped.agent_id)
        .await
        .expect("post-push revision");
    assert!(latest.change_seq > stopped.change_seq);
    assert!(latest.completion_message_id.is_some());
    let saved = load_trajectory_for_chat(fixture.app.gcx.clone(), child_chat_id)
        .await
        .expect("durable stopped trajectory");
    assert_eq!(
        saved
            .messages
            .iter()
            .filter(|message| message.role == "tool")
            .count(),
        10
    );
    let resumed = crate::agents::spawn::resume_capped_agent(
        fixture.app.clone(),
        PARENT,
        &stopped.agent_id,
        20,
        latest.change_seq,
    )
    .await
    .expect("resume same capped child");
    assert_eq!(resumed.agent_id, stopped.agent_id);
    assert_eq!(resumed.child_chat_id, stopped.child_chat_id);
    tokio::time::timeout(Duration::from_secs(30), resumed_started.notified())
        .await
        .expect("continuation started");
    let duplicate = crate::agents::spawn::resume_capped_agent(
        fixture.app.clone(),
        PARENT,
        &stopped.agent_id,
        20,
        latest.change_seq,
    )
    .await
    .expect_err("stale duplicate must not launch another continuation");
    assert!(duplicate.contains("revision does not match"));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    release_resumed.notify_one();
    let completed = wait_for_record(&fixture.app, PARENT, &stopped.agent_id, |record| {
        record.status.is_terminal() && record.completion_message_id.is_some()
    })
    .await;
    tokio::time::timeout(Duration::from_secs(30), async {
        while fixture.app.agent_executions.get(child_chat_id).is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("continuation released admission");
    assert_eq!(completed.status, BgAgentStatus::Completed);
    assert_eq!(completed.step_count, 11);
    assert_eq!(completed.created_at, stopped.created_at);
    assert_eq!(
        completed.result_summary.as_deref(),
        Some("Both edits are complete.")
    );
    assert!(completed.error.is_none());
    assert!(completed
        .edited_files
        .contains(&"before-cap.txt".to_string()));
    assert!(completed
        .edited_files
        .contains(&"after-cap.txt".to_string()));
    let (_, child_worktree, _) = stopped_context.lock().await.clone().unwrap();
    assert_eq!(
        std::fs::read_to_string(child_worktree.root.join("before-cap.txt")).unwrap(),
        "first edit\n"
    );
    assert_eq!(
        std::fs::read_to_string(child_worktree.root.join("after-cap.txt")).unwrap(),
        "second edit\n"
    );
    assert!(!fixture.source.join("before-cap.txt").exists());
    assert!(!fixture.source.join("after-cap.txt").exists());
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    let saved_config = fixture
        .app
        .agent_executions
        .saved_config(&stopped.agent_id)
        .expect("completed stateful children retain their continuation configuration");
    assert_eq!(saved_config.max_steps, 20);
    assert_eq!(saved_config.chat_id.as_deref(), Some(child_chat_id));
    assert_eq!(completed.run_generation, 2);
    assert_eq!(
        fixture
            .app
            .agents
            .list_for_parent(PARENT, Default::default())
            .await
            .len(),
        1
    );
}

#[serial(test_runner)]
#[tokio::test]
async fn interrupted_child_resumes_same_history_without_replaying_unknown_tools() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let fixture = repo_fixture().await;
    let parent_id = "resume-interruption-parent";
    let mut parent = crate::chat::types::ChatSession::new(parent_id.into());
    parent.work_project_root = Some(fixture.source.clone());
    parent.set_runtime_state(SessionState::Generating, None);
    fixture
        .app
        .chat
        .sessions
        .write()
        .await
        .insert(parent_id.into(), Arc::new(Mutex::new(parent)));
    let attempts = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(Notify::new());
    let _runner =
        {
            let attempts = attempts.clone();
            let finished = finished.clone();
            crate::agents::spawn::install_test_runner(Arc::new(move |gcx, mut messages, config| {
                let attempts = attempts.clone();
                let finished = finished.clone();
                Box::pin(async move {
                    let app = AppState::from_gcx(gcx.clone()).await;
                    let id = config.chat_id.as_ref().unwrap();
                    let session = app.chat.sessions.read().await.get(id).cloned().unwrap();
                    let thread = session.lock().await.thread.clone();
                    if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                        let mut call =
                            ChatMessage::new("assistant".into(), "in-flight command".into());
                        call.tool_calls = Some(serde_json::from_value(serde_json::json!([{
                        "id":"unknown-command","type":"function","index":0,
                        "function":{"name":"shell","arguments":"{\"command\":\"write once\"}"}
                    }])).unwrap());
                        messages.push(call);
                        save_trajectory_as(gcx, &thread, &messages).await;
                        mirror_subchat_messages_into_session(&app, id, &messages).await;
                        finish_stateful_subchat_session(&app, id, &config).await;
                        return Err("transport interrupted after command dispatch".into());
                    }
                    assert_eq!(
                        messages
                            .iter()
                            .filter(|message| message.extra.contains_key("agent_recovery"))
                            .count(),
                        1
                    );
                    assert!(messages
                        .iter()
                        .any(|message| message.content.content_text_only()
                            == "Inspect the unknown command, do not replay it."));
                    let result = completed_result(messages, config);
                    finished.notify_one();
                    Ok(result)
                })
            }))
        };
    let stopped = await_completion(
        crate::agents::spawn::spawn_background_agent(
            fixture.app.clone(),
            spawn_request(parent_id, BgAgentKind::Subagent),
        )
        .await
        .unwrap(),
    )
    .await;
    let record = fixture.app.agents.get_any(&stopped.agent_id).await.unwrap();
    assert!(crate::agents::spawn::resume_agent(
        fixture.app.clone(),
        parent_id,
        &record.agent_id,
        None,
        record.change_seq,
        None,
        false
    )
    .await
    .unwrap_err()
    .contains("incomplete tool results"));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    let resumed = crate::agents::spawn::resume_agent(
        fixture.app.clone(),
        parent_id,
        &record.agent_id,
        None,
        record.change_seq,
        Some("Inspect the unknown command, do not replay it.".into()),
        true,
    )
    .await
    .unwrap();
    assert_eq!(resumed.child_chat_id, stopped.child_chat_id);
    assert_eq!(resumed.run_generation, stopped.run_generation + 1);
    tokio::time::timeout(Duration::from_secs(30), finished.notified())
        .await
        .unwrap();
    wait_for_record(&fixture.app, parent_id, &record.agent_id, |record| {
        record.status.is_terminal()
    })
    .await;
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[serial(test_runner)]
#[tokio::test]
async fn twelve_runs_across_both_runtimes_admit_four_queue_eight_and_rotate_roots() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use crate::tools::tool_spawn_ab::tests::{await_started, task_meta, test_card, RecordingFacade};
    use crate::tools::tool_spawn_ab::{dispatch_ab_variants, AbVariantSpec, QueuedAbVariant};
    let mut fixture = repo_fixture().await;
    fixture
        .app
        .work_scheduler
        .configure(refact_work_core::CoordinationSettings {
            max_active_engine: 4,
            max_active_project: 4,
            max_active_root: 4,
            ..Default::default()
        })
        .unwrap();
    let facade = Arc::new(RecordingFacade::default());
    fixture.app.chat.facade = facade.clone();
    let task_dir = fixture.source.join(".refact/tasks/task-1");
    std::fs::create_dir_all(&task_dir).unwrap();
    std::fs::write(
        task_dir.join("meta.yaml"),
        serde_yaml::to_string(&task_meta("task-1")).unwrap(),
    )
    .unwrap();
    let mut board = crate::tasks::types::TaskBoard::default();
    for id in ["T-1", "T-2", "T-3"] {
        let mut card = test_card();
        card.id = id.into();
        card.title = format!("Card {id}");
        board.cards.push(card);
    }
    std::fs::write(
        task_dir.join("board.yaml"),
        serde_yaml::to_string(&board).unwrap(),
    )
    .unwrap();
    let parent_id = "acceptance-parent";
    let mut parent = crate::chat::types::ChatSession::new(parent_id.into());
    parent.work_project_root = Some(fixture.source.clone());
    parent.set_runtime_state(SessionState::Generating, None);
    fixture
        .app
        .chat
        .sessions
        .write()
        .await
        .insert(parent_id.into(), Arc::new(Mutex::new(parent)));
    let chat_started = Arc::new(AtomicUsize::new(0));
    let holds: Arc<std::sync::Mutex<Vec<tokio::sync::oneshot::Sender<()>>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let _runner = {
        let chat_started = chat_started.clone();
        let holds = holds.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, messages, config| {
            let chat_started = chat_started.clone();
            let holds = holds.clone();
            Box::pin(async move {
                let (release, held) = tokio::sync::oneshot::channel::<()>();
                holds.lock().unwrap().push(release);
                chat_started.fetch_add(1, Ordering::SeqCst);
                let _ = held.await;
                Ok(completed_result(messages, config))
            })
        }))
    };
    let mut chat_handles = Vec::new();
    for _ in 0..6 {
        chat_handles.push(
            crate::agents::spawn::spawn_background_agent(
                fixture.app.clone(),
                spawn_request(parent_id, BgAgentKind::Subagent),
            )
            .await
            .unwrap(),
        );
    }
    let wait_chat_started = |count: usize| {
        let chat_started = chat_started.clone();
        async move {
            tokio::time::timeout(Duration::from_secs(30), async {
                while chat_started.load(Ordering::SeqCst) < count {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("chat runs admitted");
        }
    };
    wait_chat_started(4).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(chat_started.load(Ordering::SeqCst), 4);
    let project = crate::coordination::roots::project_identity(&fixture.source);
    let mut variants = Vec::new();
    for id in ["T-1", "T-2", "T-3"] {
        let mut card = test_card();
        card.id = id.into();
        let spec = AbVariantSpec {
            model: None,
            extra_instructions: None,
            suggested_steps: 30,
        };
        let launched = tokio::time::timeout(
            Duration::from_secs(5),
            dispatch_ab_variants(
                fixture.app.clone(),
                task_meta("task-1"),
                "planner".into(),
                "planner",
                &project,
                card,
                String::new(),
                QueuedAbVariant::new(id, "a", "model-a".into(), spec.clone()),
                QueuedAbVariant::new(id, "b", "model-b".into(), spec),
                0,
            ),
        )
        .await
        .expect("board spawn never waits for execution capacity")
        .unwrap();
        assert!(launched.a.worktree.is_empty() && launched.b.worktree.is_empty());
        variants.push(launched);
    }
    let snapshot = fixture.app.work_scheduler.snapshot();
    assert_eq!(snapshot.active, 4, "{snapshot:?}");
    assert_eq!(snapshot.pending, 8, "{snapshot:?}");
    assert!(facade.created.lock().unwrap().is_empty());
    assert!(facade.started.lock().unwrap().is_empty());
    assert!(!fixture.cache.join("worktrees").exists());
    let mut queued_chat = 0;
    for handle in &chat_handles {
        let record = fixture
            .app
            .agents
            .get(parent_id, &handle.agent_id)
            .await
            .unwrap();
        match record.status {
            BgAgentStatus::Queued => {
                queued_chat += 1;
                assert!(record.started_at.is_none());
            }
            BgAgentStatus::Running => {}
            other => panic!("unexpected status {other:?}"),
        }
    }
    assert_eq!(queued_chat, 2);
    assert_eq!(fixture.app.chat.sessions.read().await.len(), 5);
    let board = crate::tasks::storage::load_board(fixture.app.gcx.clone(), "task-1")
        .await
        .unwrap();
    for id in ["T-1", "T-2", "T-3"] {
        let card = board.get_card(id).unwrap();
        assert!(card.ab_variants.is_some());
        assert!(card.started_at.is_none());
    }
    let release_one_chat_run = || {
        let release = holds.lock().unwrap().remove(0);
        release.send(()).unwrap();
    };
    release_one_chat_run();
    wait_chat_started(5).await;
    assert!(facade.started.lock().unwrap().is_empty());
    release_one_chat_run();
    await_started(&facade, 1).await;
    assert_eq!(chat_started.load(Ordering::SeqCst), 5);
    release_one_chat_run();
    wait_chat_started(6).await;
    assert_eq!(facade.started.lock().unwrap().len(), 1);
    release_one_chat_run();
    await_started(&facade, 2).await;
    release_one_chat_run();
    await_started(&facade, 3).await;
    release_one_chat_run();
    await_started(&facade, 4).await;
    for handle in chat_handles {
        let done = await_completion(handle).await;
        assert_eq!(done.status, BgAgentStatus::Completed, "{:?}", done.error);
    }
    assert_eq!(fixture.app.work_scheduler.snapshot().active, 4);
    let started: Vec<String> = facade.started.lock().unwrap().clone();
    for chat_id in &started {
        crate::tasks::admission::end(chat_id).await;
    }
    await_started(&facade, 6).await;
    let started: Vec<String> = facade.started.lock().unwrap().clone();
    for chat_id in &started[4..] {
        crate::tasks::admission::end(chat_id).await;
    }
    let board = crate::tasks::storage::load_board(fixture.app.gcx.clone(), "task-1")
        .await
        .unwrap();
    for id in ["T-1", "T-2", "T-3"] {
        let stored = board.get_card(id).unwrap().ab_variants.clone().unwrap();
        assert!(!stored.a.worktree.is_empty() && !stored.b.worktree.is_empty());
        crate::tools::tool_spawn_ab::cleanup_ab_variant(fixture.app.gcx.clone(), &stored.a)
            .await
            .unwrap();
        crate::tools::tool_spawn_ab::cleanup_ab_variant(fixture.app.gcx.clone(), &stored.b)
            .await
            .unwrap();
    }
    let snapshot = fixture.app.work_scheduler.snapshot();
    assert_eq!(snapshot.active, 0, "{snapshot:?}");
    assert_eq!(snapshot.outstanding, 0, "{snapshot:?}");
}

#[serial(test_runner)]
#[tokio::test]
async fn parent_parks_its_only_permit_during_agent_wait_and_reacquires_before_continuing() {
    use crate::agents::execution::ExecutionState;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let fixture = repo_fixture().await;
    fixture
        .app
        .work_scheduler
        .configure(refact_work_core::CoordinationSettings {
            max_active_engine: 1,
            max_active_project: 1,
            max_active_root: 1,
            ..Default::default()
        })
        .unwrap();
    let parent_id = "parking-parent";
    let mut parent = crate::chat::types::ChatSession::new(parent_id.into());
    parent.work_project_root = Some(fixture.source.clone());
    parent.thread.mode = "agent".into();
    parent.thread.model = "openai/gpt-4o".into();
    parent.set_runtime_state(SessionState::Generating, None);
    let parent = Arc::new(Mutex::new(parent));
    fixture
        .app
        .chat
        .sessions
        .write()
        .await
        .insert(parent_id.into(), parent.clone());
    let project = crate::coordination::roots::project_identity(&fixture.source);
    let permit = fixture
        .app
        .work_scheduler
        .reserve(refact_work_core::AdmissionRequest::new(
            refact_work_core::WorkKey::trajectory(parent_id).run(1),
            refact_work_core::WorkScope::new(
                refact_work_core::ProjectKey::new(project),
                refact_work_core::CheckoutKey::new(parent_id),
            ),
        ))
        .unwrap()
        .acquire()
        .await
        .unwrap();
    let execution = fixture
        .app
        .agent_executions
        .install(parent_id, permit)
        .unwrap();
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = Arc::new(std::sync::Mutex::new(
        None::<(ExecutionState, refact_work_core::SchedulerSnapshot)>,
    ));
    let _runner = {
        let attempts = attempts.clone();
        let observed = observed.clone();
        let execution = execution.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |gcx, messages, config| {
            let attempts = attempts.clone();
            let observed = observed.clone();
            let execution = execution.clone();
            Box::pin(async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                *observed.lock().unwrap() =
                    Some((execution.state(), gcx.work_scheduler.snapshot()));
                Ok(completed_result(messages, config))
            })
        }))
    };
    let handle = crate::agents::spawn::spawn_background_agent(
        fixture.app.clone(),
        spawn_request(parent_id, BgAgentKind::Subagent),
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(attempts.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture
            .app
            .agents
            .get(parent_id, &handle.agent_id)
            .await
            .unwrap()
            .status,
        BgAgentStatus::Queued
    );
    let before = fixture.app.work_scheduler.snapshot();
    assert_eq!((before.active, before.pending, before.parked), (1, 1, 0));
    let ccx = Arc::new(Mutex::new(
        crate::at_commands::at_commands::AtCommandsContext::new_from_app(
            fixture.app.clone(),
            4096,
            20,
            false,
            Vec::new(),
            parent_id.into(),
            None,
            "openai/gpt-4o".into(),
            None,
            None,
        )
        .await,
    ));
    let thread = parent.lock().await.thread.clone();
    let call =
        |id: &str, name: &str, arguments: serde_json::Value| crate::call_validation::ChatToolCall {
            id: id.into(),
            function: crate::call_validation::ChatToolFunction {
                name: name.into(),
                arguments: arguments.to_string(),
            },
            index: Some(0),
            tool_type: "function".into(),
            extra_content: None,
            started_at_ms: None,
            completed_at_ms: None,
        };
    let (messages, _) = tokio::time::timeout(
        Duration::from_secs(60),
        crate::chat::tools::execute_tools(
            fixture.app.clone(),
            &[call(
                "wait-child",
                "agent_wait",
                serde_json::json!({"agent_id": handle.agent_id, "timeout_ms": 30000}),
            )],
            &[],
            &thread,
            "agent",
            Some("openai/gpt-4o"),
            crate::chat::tools::ExecuteToolsOptions {
                execution_context: Some(ccx.clone()),
                ..Default::default()
            },
        ),
    )
    .await
    .expect("parent wait must not deadlock on its own permit");
    assert_eq!(messages[0].tool_call_id, "wait-child");
    assert_ne!(
        messages[0].tool_failed,
        Some(true),
        "{:?}",
        messages[0].content
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    let (parent_state, snapshot) = observed.lock().unwrap().clone().expect("child ran");
    assert_eq!(parent_state, ExecutionState::Parked);
    assert_eq!((snapshot.active, snapshot.parked), (1, 1), "{snapshot:?}");
    let done = await_completion(handle).await;
    assert_eq!(done.status, BgAgentStatus::Completed, "{:?}", done.error);
    assert_eq!(execution.state(), ExecutionState::Running);
    assert!(execution.holds_permit());
    let after = fixture.app.work_scheduler.snapshot();
    assert_eq!((after.active, after.pending, after.parked), (1, 0, 0));
    fixture.app.agent_executions.shutdown().await;
}

async fn crash_fixture_app(root: &Path) -> (AppState, PathBuf) {
    let source = std::fs::canonicalize(root.join("repo")).unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        root.join("cache"),
        root.join("config"),
    )
    .await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
    let (project_root, _) = crate::caps::project_model_defaults_state(gcx.clone()).await;
    {
        let mut caps = crate::caps::CodeAssistantCaps::default();
        caps.chat_models.insert(
            "openai/gpt-4o".into(),
            Arc::new(crate::caps::ChatModelRecord {
                base: crate::caps::BaseModelRecord {
                    id: "openai/gpt-4o".into(),
                    ..Default::default()
                },
                supports_tools: true,
                supports_agent: true,
                ..Default::default()
            }),
        );
        let mut state = gcx.caps_state.write().await;
        state.caps = Some(Arc::new(caps));
        state.loaded_project_root = Some(project_root);
        state.last_attempted_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
    let app = AppState::from_gcx(gcx).await;
    let mut parent = crate::chat::types::ChatSession::new("crash-parent".into());
    parent.work_project_root = Some(source.clone());
    parent.set_runtime_state(SessionState::Generating, None);
    app.chat
        .sessions
        .write()
        .await
        .insert("crash-parent".into(), Arc::new(Mutex::new(parent)));
    (app, source)
}

#[test]
fn subprocess_crash_launch() {
    let Ok(root) = std::env::var("REFACT_TEST_CRASH_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (app, _source) = crash_fixture_app(&root).await;
        let marker = root.join("running");
        let _runner = crate::agents::spawn::install_test_runner(Arc::new(
            move |gcx, mut messages, config| {
                let marker = marker.clone();
                Box::pin(async move {
                    let app = AppState::from_gcx(gcx.clone()).await;
                    let id = config.chat_id.as_ref().unwrap();
                    let session = app.chat.sessions.read().await.get(id).cloned().unwrap();
                    let thread = session.lock().await.thread.clone();
                    messages.push(ChatMessage::new(
                        "assistant".into(),
                        "first boundary".into(),
                    ));
                    save_trajectory_as(gcx, &thread, &messages).await;
                    mirror_subchat_messages_into_session(&app, id, &messages).await;
                    app.agents
                        .update_progress(
                            config.background_agent_id.as_ref().unwrap(),
                            "first boundary saved".into(),
                            1,
                        )
                        .await
                        .unwrap();
                    app.agents
                        .add_usage(config.background_agent_id.as_ref().unwrap(), 23, Some(0.07))
                        .await
                        .unwrap();
                    std::fs::write(&marker, "running").unwrap();
                    std::future::pending().await
                })
            },
        ));
        crate::agents::spawn::spawn_background_agent(
            app.clone(),
            spawn_request("crash-parent", BgAgentKind::Subagent),
        )
        .await
        .unwrap();
        std::future::pending::<()>().await;
    });
}

#[serial(test_runner)]
#[tokio::test]
async fn killed_engine_process_leaves_a_recoverable_child_without_replay() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("repo");
    std::fs::create_dir_all(&source).unwrap();
    init_repo(&source);
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "agents::spawn_lifecycle_tests::subprocess_crash_launch",
            "--nocapture",
        ])
        .env("REFACT_TEST_CRASH_ROOT", temp.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let marker = temp.path().join("running");
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while !marker.exists() {
        if std::time::Instant::now() >= deadline || child.try_wait().unwrap().is_some() {
            let _ = child.kill();
            let _ = child.wait();
            panic!("crash subprocess never reached its first saved boundary");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    let (app, _source) = crash_fixture_app(temp.path()).await;
    let records = app.agents.list_all().await;
    assert_eq!(records.len(), 1);
    let record = records.into_iter().next().unwrap();
    assert_eq!(record.status, BgAgentStatus::Interrupted);
    assert_eq!(record.tokens_used, 23);
    assert_eq!(record.cost_usd, Some(0.07));
    assert!(record.step_count <= 1);
    let child_chat_id = record.child_chat_id.clone().expect("child chat id");
    assert!(app.agent_executions.get(&child_chat_id).is_none());
    let recovered =
        crate::agents::spawn::recover_run_config(&app, "crash-parent", &record.agent_id)
            .await
            .unwrap();
    assert_eq!(
        recovered.config.chat_id.as_deref(),
        Some(child_chat_id.as_str())
    );
    assert!(!recovered.authority_migrated);
    let saved = load_trajectory_for_chat(app.gcx.clone(), &child_chat_id)
        .await
        .expect("child trajectory persisted before the crash");
    assert!(saved
        .messages
        .iter()
        .any(|message| message.content.content_text_only() == "first boundary"));
    let attempts = Arc::new(AtomicUsize::new(0));
    let _runner = {
        let attempts = attempts.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, messages, config| {
            let attempts = attempts.clone();
            Box::pin(async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                assert!(messages
                    .iter()
                    .any(|message| message.content.content_text_only() == "first boundary"));
                assert_eq!(
                    messages
                        .iter()
                        .filter(|message| message.extra.contains_key("agent_recovery"))
                        .count(),
                    0,
                    "no incomplete tool outcome existed, so nothing needed reconciling"
                );
                assert!(messages.iter().any(|message| {
                    message
                        .extra
                        .get("event")
                        .is_some_and(|event| event["source"] == "agents.resume")
                }));
                Ok(completed_result(messages, config))
            })
        }))
    };
    let resumed = crate::agents::spawn::resume_agent(
        app.clone(),
        "crash-parent",
        &record.agent_id,
        None,
        record.change_seq,
        None,
        true,
    )
    .await
    .unwrap();
    assert_eq!(resumed.run_generation, record.run_generation + 1);
    let done = wait_for_record(&app, "crash-parent", &record.agent_id, |record| {
        record.status.is_terminal()
    })
    .await;
    assert_eq!(done.status, BgAgentStatus::Completed, "{:?}", done.error);
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert_eq!(done.run_generation, record.run_generation + 1);
    assert_eq!(done.tokens_used, 23);
    assert_eq!(done.cost_usd, Some(0.07));
    app.agent_executions.shutdown().await;
}

#[serial(test_runner)]
#[tokio::test]
async fn panicking_child_runner_releases_capacity_and_terminalizes_honestly() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let fixture = repo_fixture().await;
    fixture
        .app
        .work_scheduler
        .configure(refact_work_core::CoordinationSettings {
            max_active_engine: 1,
            max_active_project: 1,
            max_active_root: 1,
            ..Default::default()
        })
        .unwrap();
    let parent_id = "panic-parent";
    let mut parent = crate::chat::types::ChatSession::new(parent_id.into());
    parent.work_project_root = Some(fixture.source.clone());
    parent.set_runtime_state(SessionState::Generating, None);
    fixture
        .app
        .chat
        .sessions
        .write()
        .await
        .insert(parent_id.into(), Arc::new(Mutex::new(parent)));
    let attempts = Arc::new(AtomicUsize::new(0));
    let _runner = {
        let attempts = attempts.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, messages, config| {
            let attempts = attempts.clone();
            Box::pin(async move {
                if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    panic!("runner exploded while holding the only permit");
                }
                Ok(completed_result(messages, config))
            })
        }))
    };
    let crashed = await_completion(
        crate::agents::spawn::spawn_background_agent(
            fixture.app.clone(),
            spawn_request(parent_id, BgAgentKind::Subagent),
        )
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(crashed.status, BgAgentStatus::Failed);
    assert!(crashed
        .error
        .as_deref()
        .is_some_and(|error| error.contains("panicked")));
    let child = crashed.child_chat_id.clone().expect("child chat");
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let snapshot = fixture.app.work_scheduler.snapshot();
        let released = fixture.app.agent_executions.get(&child).is_none()
            && !fixture.app.agent_executions.task_active(&child)
            && snapshot.active == 0
            && snapshot.outstanding == 0;
        if released {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "panicking child left execution state behind: {snapshot:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let next = tokio::time::timeout(
        Duration::from_secs(30),
        await_completion(
            crate::agents::spawn::spawn_background_agent(
                fixture.app.clone(),
                spawn_request(parent_id, BgAgentKind::Subagent),
            )
            .await
            .unwrap(),
        ),
    )
    .await
    .expect("capacity returns after the panicking child");
    assert_eq!(next.status, BgAgentStatus::Completed, "{:?}", next.error);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.app.work_scheduler.snapshot().active, 0);
}

#[serial(test_runner)]
#[tokio::test]
async fn legacy_open_tool_authority_migrates_to_the_concrete_parent_ceiling_on_resume() {
    use sha2::Digest;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let fixture = repo_fixture().await;
    let parent_id = "legacy-authority-parent";
    let mut parent = crate::chat::types::ChatSession::new(parent_id.into());
    parent.work_project_root = Some(fixture.source.clone());
    parent.thread.mode = "agent".into();
    parent.set_runtime_state(SessionState::Generating, None);
    let parent = Arc::new(Mutex::new(parent));
    fixture
        .app
        .chat
        .sessions
        .write()
        .await
        .insert(parent_id.into(), parent.clone());
    let attempts = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(Notify::new());
    let resumed_run = Arc::new(std::sync::Mutex::new(
        None::<(crate::subchat::ToolsPolicy, Vec<ChatMessage>)>,
    ));
    let _runner = {
        let attempts = attempts.clone();
        let finished = finished.clone();
        let resumed_run = resumed_run.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |gcx, mut messages, config| {
            let attempts = attempts.clone();
            let finished = finished.clone();
            let resumed_run = resumed_run.clone();
            Box::pin(async move {
                let app = AppState::from_gcx(gcx.clone()).await;
                let id = config.chat_id.as_ref().unwrap();
                let session = app.chat.sessions.read().await.get(id).cloned().unwrap();
                let thread = session.lock().await.thread.clone();
                if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    messages.push(ChatMessage::new("assistant".into(), "stopped early".into()));
                    save_trajectory_as(gcx, &thread, &messages).await;
                    mirror_subchat_messages_into_session(&app, id, &messages).await;
                    finish_stateful_subchat_session(&app, id, &config).await;
                    return Err("transport interrupted".into());
                }
                *resumed_run.lock().unwrap() = Some((config.tools.clone(), messages.clone()));
                let result = completed_result(messages, config);
                finished.notify_one();
                Ok(result)
            })
        }))
    };
    let stopped = await_completion(
        crate::agents::spawn::spawn_background_agent(
            fixture.app.clone(),
            spawn_request(parent_id, BgAgentKind::Subagent),
        )
        .await
        .unwrap(),
    )
    .await;
    let record = fixture.app.agents.get_any(&stopped.agent_id).await.unwrap();
    assert_eq!(record.status, BgAgentStatus::Failed);
    let storage = fixture.cache.join(".refact/background_agents");
    let launch_path = storage
        .join("launches")
        .join(format!("{}.json", record.agent_id));
    let run_path = storage.join("runs").join(format!(
        "{}.{}.json",
        record.agent_id, record.run_generation
    ));
    let mut launch: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&launch_path).unwrap()).unwrap();
    assert!(launch["config"]["tools"]["Only"].is_array());
    launch["config"]["tools"] = serde_json::json!("All");
    let launch_bytes = serde_json::to_vec(&launch).unwrap();
    std::fs::write(&launch_path, &launch_bytes).unwrap();
    let mut run: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&run_path).unwrap()).unwrap();
    run["config"]["tools"] = serde_json::json!("All");
    run["launch_digest"] = serde_json::json!(hex::encode(sha2::Sha256::digest(&launch_bytes)));
    std::fs::write(&run_path, serde_json::to_vec(&run).unwrap()).unwrap();
    parent.lock().await.active_command.allowed_tools = vec!["cat".into()];
    let concrete = serde_json::json!({"Only": ["cat"]});
    let recovered =
        crate::agents::spawn::recover_run_config(&fixture.app, parent_id, &record.agent_id)
            .await
            .unwrap();
    assert!(recovered.authority_migrated);
    assert_eq!(
        serde_json::to_value(&recovered.config.tools).unwrap(),
        concrete
    );
    let ccx = Arc::new(Mutex::new(
        crate::at_commands::at_commands::AtCommandsContext::new_from_app(
            fixture.app.clone(),
            4096,
            20,
            false,
            Vec::new(),
            parent_id.into(),
            None,
            "openai/gpt-4o".into(),
            None,
            None,
        )
        .await,
    ));
    let mut inspect = crate::tools::tool_background_agents::ToolAgentResume {
        config_path: String::new(),
    };
    let (_, output) = crate::tools::tools_description::Tool::tool_execute(
        &mut inspect,
        ccx,
        &"inspect-legacy".into(),
        &std::collections::HashMap::from([
            ("agent_id".into(), serde_json::json!(record.agent_id)),
            ("action".into(), serde_json::json!("inspect")),
        ]),
    )
    .await
    .unwrap();
    let crate::call_validation::ContextEnum::ChatMessage(receipt) = &output[0] else {
        panic!("missing receipt")
    };
    let receipt: serde_json::Value =
        serde_json::from_str(&receipt.content.content_text_only()).unwrap();
    assert_eq!(receipt["tool_authority"]["migrated_from_legacy_all"], true);
    assert_eq!(receipt["tool_authority"]["tools"], concrete);
    assert_eq!(std::fs::read(&launch_path).unwrap(), launch_bytes);
    let resumed = crate::agents::spawn::resume_agent(
        fixture.app.clone(),
        parent_id,
        &record.agent_id,
        None,
        record.change_seq,
        None,
        true,
    )
    .await
    .unwrap();
    assert_eq!(resumed.run_generation, record.run_generation + 1);
    tokio::time::timeout(Duration::from_secs(30), finished.notified())
        .await
        .unwrap();
    let done = wait_for_record(&fixture.app, parent_id, &record.agent_id, |record| {
        record.status.is_terminal()
    })
    .await;
    assert_eq!(done.status, BgAgentStatus::Completed, "{:?}", done.error);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    let (tools, messages) = resumed_run.lock().unwrap().clone().unwrap();
    assert_eq!(serde_json::to_value(&tools).unwrap(), concrete);
    let notice = messages
        .iter()
        .find(|message| {
            message
                .extra
                .get("event")
                .is_some_and(|event| event["source"] == "agents.authority_migrated")
        })
        .expect("migration notice");
    assert_eq!(notice.extra["event"]["payload"]["tools"], concrete);
    assert_eq!(
        notice.extra["event"]["payload"]["run_generation"],
        resumed.run_generation
    );
    assert_eq!(std::fs::read(&launch_path).unwrap(), launch_bytes);
    let migrated_run: serde_json::Value = serde_json::from_slice(
        &std::fs::read(storage.join("runs").join(format!(
            "{}.{}.json",
            record.agent_id, resumed.run_generation
        )))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(migrated_run["config"]["tools"], concrete);
    let settled =
        crate::agents::spawn::recover_run_config(&fixture.app, parent_id, &record.agent_id)
            .await
            .unwrap();
    assert!(!settled.authority_migrated);
    assert_eq!(
        serde_json::to_value(&settled.config.tools).unwrap(),
        concrete
    );
}

#[serial(test_runner)]
#[tokio::test]
async fn cold_restart_resumes_authoritative_child_without_replaying_unknown_effects() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let fixture = repo_fixture().await;
    let parent_id = "cold-parent";
    let mut parent = crate::chat::types::ChatSession::new(parent_id.into());
    parent.work_project_root = Some(fixture.source.clone());
    parent.set_runtime_state(SessionState::Generating, None);
    fixture
        .app
        .chat
        .sessions
        .write()
        .await
        .insert(parent_id.into(), Arc::new(Mutex::new(parent)));
    let entered = Arc::new(Notify::new());
    let attempts = Arc::new(AtomicUsize::new(0));
    let effect = fixture.source.join("once.txt");
    let _runner = crate::agents::spawn::install_test_runner({
        let entered = entered.clone();
        let attempts = attempts.clone();
        let effect = effect.clone();
        Arc::new(move |gcx, mut messages, config| {
            let entered = entered.clone();
            let attempts = attempts.clone();
            let effect = effect.clone();
            Box::pin(async move {
                let app = AppState::from_gcx(gcx.clone()).await;
                let id = config.chat_id.as_ref().unwrap();
                let session = app.chat.sessions.read().await.get(id).cloned().unwrap();
                if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    std::fs::write(&effect, "executed once").unwrap();
                    let mut call = ChatMessage::new("assistant".into(), "pending result".into());
                    call.message_id = "cold-call".into();
                    call.tool_calls = Some(
                        serde_json::from_value(serde_json::json!([{
                            "id":"cold-effect","type":"function","index":0,
                            "function":{"name":"shell","arguments":"{}"}
                        }]))
                        .unwrap(),
                    );
                    messages.push(call);
                    for (index, message) in messages.iter_mut().enumerate() {
                        if message.message_id.is_empty() {
                            message.message_id = format!("cold-{index}");
                        }
                    }
                    {
                        let mut session = session.lock().await;
                        session.replace_messages(messages);
                        session.goal_record_progress_with_cost(23, true, 7);
                    }
                    crate::chat::trajectories::flush_trajectory_for_session(app.clone(), session)
                        .await
                        .unwrap();
                    app.agents
                        .update_progress(
                            config.background_agent_id.as_ref().unwrap(),
                            "saved".into(),
                            1,
                        )
                        .await
                        .unwrap();
                    app.agents
                        .add_usage(config.background_agent_id.as_ref().unwrap(), 23, Some(0.07))
                        .await
                        .unwrap();
                    entered.notify_one();
                    return std::future::pending().await;
                }
                assert_eq!(std::fs::read_to_string(&effect).unwrap(), "executed once");
                assert_eq!(
                    messages
                        .iter()
                        .filter(|m| m.extra.contains_key("agent_recovery"))
                        .count(),
                    1
                );
                assert_eq!(
                    messages
                        .iter()
                        .filter(|m| m.message_id == "cold-call")
                        .count(),
                    1
                );
                let session = session.lock().await;
                let goal = session.goal.as_ref().unwrap();
                assert_eq!(goal.progress.tokens_used, 23);
                assert!(!session.goal_ledger.is_empty());
                assert!(messages
                    .iter()
                    .any(|m| m.role == "plan"
                        && m.content.content_text_only() == "Preserve this plan"));
                drop(session);
                Ok(completed_result(messages, config))
            })
        })
    });
    let mut request = spawn_request(parent_id, BgAgentKind::Subagent);
    request.plan = Some("Preserve this plan".into());
    request.goal = Some(SpawnGoal {
        content: "Continue safely".into(),
        criteria: Vec::new(),
        budget: None,
    });
    let handle = crate::agents::spawn::spawn_background_agent(fixture.app.clone(), request)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), entered.notified())
        .await
        .unwrap();
    fixture
        .app
        .agents
        .flush_pending_writes_for_test()
        .await
        .unwrap();
    fixture
        .app
        .agent_executions
        .interrupt_tasks_for_test()
        .await;
    let new_gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        fixture.cache.clone(),
        fixture._temp.path().join("config"),
    )
    .await;
    *new_gcx.documents_state.workspace_folders.lock().unwrap() = vec![fixture.source.clone()];
    let caps = fixture.app.gcx.caps_state.read().await.caps.clone();
    let (root, _) = crate::caps::project_model_defaults_state(new_gcx.clone()).await;
    {
        let mut state = new_gcx.caps_state.write().await;
        state.caps = caps;
        state.loaded_project_root = Some(root);
        state.last_attempted_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
    let restarted = AppState::from_gcx(new_gcx).await;
    let mut parent = crate::chat::types::ChatSession::new(parent_id.into());
    parent.work_project_root = Some(fixture.source.clone());
    parent.set_runtime_state(SessionState::Generating, None);
    restarted
        .chat
        .sessions
        .write()
        .await
        .insert(parent_id.into(), Arc::new(Mutex::new(parent)));
    let record = restarted.agents.get_any(&handle.agent_id).await.unwrap();
    assert_eq!(record.status, BgAgentStatus::Interrupted);
    assert!(restarted
        .agent_executions
        .saved_config(&record.agent_id)
        .is_none());
    assert!(!restarted
        .chat
        .sessions
        .read()
        .await
        .contains_key(&handle.child_chat_id));
    let run_path = fixture
        .cache
        .join(".refact/background_agents/runs")
        .join(format!("{}.1.json", record.agent_id));
    let run_bytes = std::fs::read(&run_path).unwrap();
    for invalid in [b"{".as_slice(), b"{}".as_slice()] {
        std::fs::write(&run_path, invalid).unwrap();
        assert!(
            crate::agents::spawn::recover_run_config(&restarted, parent_id, &record.agent_id)
                .await
                .is_err()
        );
        assert_eq!(
            restarted
                .agents
                .get_any(&record.agent_id)
                .await
                .unwrap()
                .change_seq,
            record.change_seq
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }
    std::fs::remove_file(&run_path).unwrap();
    assert!(
        crate::agents::spawn::recover_run_config(&restarted, parent_id, &record.agent_id)
            .await
            .is_err()
    );
    std::fs::write(&run_path, &run_bytes).unwrap();
    let mut altered: serde_json::Value = serde_json::from_slice(&run_bytes).unwrap();
    altered["run_generation"] = serde_json::json!(2);
    std::fs::write(&run_path, serde_json::to_vec(&altered).unwrap()).unwrap();
    assert!(
        crate::agents::spawn::recover_run_config(&restarted, parent_id, &record.agent_id)
            .await
            .is_err()
    );
    std::fs::write(&run_path, &run_bytes).unwrap();
    let config = crate::agents::spawn::recover_run_config(&restarted, parent_id, &record.agent_id)
        .await
        .unwrap()
        .config;
    assert_eq!(
        config.chat_id.as_deref(),
        Some(handle.child_chat_id.as_str())
    );
    assert!(!restarted
        .chat
        .sessions
        .read()
        .await
        .contains_key(&handle.child_chat_id));
    assert!(crate::agents::spawn::resume_agent(
        restarted.clone(),
        parent_id,
        &record.agent_id,
        None,
        record.change_seq,
        None,
        false
    )
    .await
    .unwrap_err()
    .contains("incomplete tool results"));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    let ccx = Arc::new(Mutex::new(
        crate::at_commands::at_commands::AtCommandsContext::new_from_app(
            restarted.clone(),
            4096,
            20,
            false,
            Vec::new(),
            parent_id.into(),
            None,
            "openai/gpt-4o".into(),
            None,
            None,
        )
        .await,
    ));
    let mut inspect = crate::tools::tool_background_agents::ToolAgentResume {
        config_path: String::new(),
    };
    let (_, output) = crate::tools::tools_description::Tool::tool_execute(
        &mut inspect,
        ccx,
        &"inspect-cold".into(),
        &std::collections::HashMap::from([
            ("agent_id".into(), serde_json::json!(record.agent_id)),
            ("action".into(), serde_json::json!("inspect")),
        ]),
    )
    .await
    .unwrap();
    let crate::call_validation::ContextEnum::ChatMessage(receipt) = &output[0] else {
        panic!("missing receipt")
    };
    let receipt: serde_json::Value =
        serde_json::from_str(&receipt.content.content_text_only()).unwrap();
    assert_eq!(receipt["resume_configuration_available"], true);
    assert_eq!(receipt["original_scope_available"], true);
    assert_eq!(receipt["run_generation"], 1);
    assert!(restarted
        .agent_executions
        .saved_config(&record.agent_id)
        .is_none());
    assert_eq!(
        restarted
            .agents
            .get_any(&record.agent_id)
            .await
            .unwrap()
            .change_seq,
        record.change_seq
    );
    let queued = crate::agents::spawn::resume_agent(
        restarted.clone(),
        parent_id,
        &record.agent_id,
        None,
        record.change_seq,
        None,
        true,
    )
    .await
    .unwrap();
    assert_eq!(queued.run_generation, 2);
    let done = wait_for_record(&restarted, parent_id, &record.agent_id, |r| {
        r.status.is_terminal()
    })
    .await;
    assert_eq!(done.status, BgAgentStatus::Completed, "{:?}", done.error);
    assert_eq!(done.tokens_used, 23);
    assert_eq!(done.step_count, 1);
    assert_eq!(done.cost_usd, Some(0.07));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    restarted.agent_executions.shutdown().await;
}

#[serial(test_runner)]
#[tokio::test]
async fn unacknowledged_resume_cancellation_never_dispatches_or_changes_history() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    for shutdown in [false, true] {
        let fixture = repo_fixture().await;
        let parent_id = "resume-ack-parent";
        let mut parent = crate::chat::types::ChatSession::new(parent_id.into());
        parent.work_project_root = Some(fixture.source.clone());
        parent.set_runtime_state(SessionState::Generating, None);
        fixture
            .app
            .chat
            .sessions
            .write()
            .await
            .insert(parent_id.into(), Arc::new(Mutex::new(parent)));
        let attempts = Arc::new(AtomicUsize::new(0));
        let seen = attempts.clone();
        let _runner =
            crate::agents::spawn::install_test_runner(Arc::new(move |gcx, messages, config| {
                let seen = seen.clone();
                Box::pin(async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    let app = AppState::from_gcx(gcx.clone()).await;
                    let id = config.chat_id.as_ref().unwrap();
                    let session = app.chat.sessions.read().await.get(id).cloned().unwrap();
                    let thread = session.lock().await.thread.clone();
                    save_trajectory_as(gcx, &thread, &messages).await;
                    mirror_subchat_messages_into_session(&app, id, &messages).await;
                    finish_stateful_subchat_session(&app, id, &config).await;
                    Err("recoverable test interruption".into())
                })
            }));
        let stopped = await_completion(
            crate::agents::spawn::spawn_background_agent(
                fixture.app.clone(),
                spawn_request(parent_id, BgAgentKind::Subagent),
            )
            .await
            .unwrap(),
        )
        .await;
        let child = stopped.child_chat_id.as_deref().unwrap();
        let saved = load_trajectory_for_chat(fixture.app.gcx.clone(), child)
            .await
            .expect("fake runner persisted the authoritative trajectory before mirroring it");
        let saved_messages = serde_json::to_value(&saved.messages).unwrap();
        let session = fixture
            .app
            .chat
            .sessions
            .read()
            .await
            .get(child)
            .cloned()
            .unwrap();
        let original = serde_json::to_value(&session.lock().await.messages).unwrap();
        let record = fixture.app.agents.get_any(&stopped.agent_id).await.unwrap();
        let (send, receive) = tokio::sync::oneshot::channel();
        let queued = crate::agents::spawn::resume_agent_inner(
            fixture.app.clone(),
            parent_id,
            &record.agent_id,
            None,
            record.change_seq,
            Some("not authorized until acknowledgement".into()),
            false,
            receive,
        )
        .await
        .unwrap();
        assert_eq!(queued.run_generation, record.run_generation + 1);
        if shutdown {
            tokio::time::timeout(
                Duration::from_secs(10),
                fixture.app.agent_executions.shutdown(),
            )
            .await
            .unwrap();
            assert!(send.send(()).is_err());
        } else {
            drop(send);
        }
        let cancelled = wait_for_record(&fixture.app, parent_id, &record.agent_id, |record| {
            record.status.is_terminal()
        })
        .await;
        assert_eq!(cancelled.status, BgAgentStatus::Cancelled);
        tokio::time::timeout(
            Duration::from_secs(10),
            fixture.app.agent_executions.shutdown(),
        )
        .await
        .unwrap();
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert_eq!(
            serde_json::to_value(&session.lock().await.messages).unwrap(),
            original
        );
        let after = load_trajectory_for_chat(fixture.app.gcx.clone(), child)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(after.messages).unwrap(),
            saved_messages
        );
        assert_eq!(fixture.app.work_scheduler.snapshot().outstanding, 0);
    }
}

#[serial(test_runner)]
#[tokio::test]
async fn queued_resume_revalidates_scope_policy_history_and_pause_without_dispatch() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    for mutation in ["scope", "privacy", "disk", "session", "pause"] {
        let fixture = repo_fixture().await;
        let parent_id = "resume-race-parent";
        let source = fixture.source.join("private.txt");
        std::fs::write(&source, "source").unwrap();
        let mut parent = crate::chat::types::ChatSession::new(parent_id.into());
        parent.work_project_root = Some(fixture.source.clone());
        parent.set_runtime_state(SessionState::Generating, None);
        let parent = Arc::new(Mutex::new(parent));
        fixture
            .app
            .chat
            .sessions
            .write()
            .await
            .insert(parent_id.into(), parent.clone());
        let attempts = Arc::new(AtomicUsize::new(0));
        let _runner = crate::agents::spawn::install_test_runner({
            let attempts = attempts.clone();
            Arc::new(move |gcx, messages, config| {
                let attempts = attempts.clone();
                Box::pin(async move {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    let app = AppState::from_gcx(gcx.clone()).await;
                    let id = config.chat_id.as_ref().unwrap();
                    let session = app.chat.sessions.read().await.get(id).cloned().unwrap();
                    let thread = session.lock().await.thread.clone();
                    save_trajectory_as(gcx, &thread, &messages).await;
                    finish_stateful_subchat_session(&app, id, &config).await;
                    Err("recoverable interruption".into())
                })
            })
        });
        let mut request = spawn_request(parent_id, BgAgentKind::Subagent);
        request.goal = Some(SpawnGoal {
            content: "Preserve pause".into(),
            criteria: Vec::new(),
            budget: None,
        });
        let mut message = ChatMessage::new("user".into(), "private source".into());
        message.extra.insert(
            "privacy".into(),
            serde_json::json!({"files":[{
                "path":source.to_string_lossy(),"zone":"normal","attribution":"declared"
            }]}),
        );
        request.initial_context_messages.push(message);
        let stopped = await_completion(
            crate::agents::spawn::spawn_background_agent(fixture.app.clone(), request)
                .await
                .unwrap(),
        )
        .await;
        let child = stopped.child_chat_id.clone().unwrap();
        let session = fixture
            .app
            .chat
            .sessions
            .read()
            .await
            .get(&child)
            .cloned()
            .unwrap();
        let mut limits = crate::file_activity::settings();
        limits.max_active_engine = 1;
        limits.max_active_project = 1;
        limits.max_active_root = 1;
        fixture.app.work_scheduler.configure(limits).unwrap();
        let blocker = fixture
            .app
            .work_scheduler
            .reserve(refact_work_core::AdmissionRequest::new(
                refact_work_core::WorkKey::background_agent("blocker").run(1),
                refact_work_core::WorkScope::new(
                    refact_work_core::ProjectKey::new("blocker"),
                    refact_work_core::CheckoutKey::new("blocker"),
                ),
            ))
            .unwrap()
            .acquire()
            .await
            .unwrap();
        let queued = crate::agents::spawn::resume_agent(
            fixture.app.clone(),
            parent_id,
            &stopped.agent_id,
            None,
            stopped.change_seq,
            None,
            false,
        )
        .await
        .unwrap();
        assert_eq!(queued.status, BgAgentStatus::Queued);
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        let foreign = tempfile::tempdir().unwrap();
        match mutation {
            "scope" => parent.lock().await.work_project_root = Some(foreign.path().to_path_buf()),
            "privacy" => {
                let mut policy = refact_privacy::PrivacyPolicy::default();
                policy.blocked.push(source.to_string_lossy().into_owned());
                fixture.app.gcx.privacy_policy_load.write().unwrap().policy = Arc::new(policy);
            }
            "disk" => {
                let mut trajectory = load_trajectory_for_chat(fixture.app.gcx.clone(), &child)
                    .await
                    .unwrap();
                trajectory
                    .messages
                    .push(ChatMessage::new("user".into(), "new user edit".into()));
                save_trajectory_as(
                    fixture.app.gcx.clone(),
                    &trajectory.thread,
                    &trajectory.messages,
                )
                .await;
            }
            "session" => {
                session
                    .lock()
                    .await
                    .add_message(ChatMessage::new("user".into(), "new user edit".into()));
            }
            "pause" => {
                session
                    .lock()
                    .await
                    .goal_set_status(crate::chat::types::GoalStatus::Paused);
            }
            _ => unreachable!(),
        }
        drop(blocker);
        let terminal = wait_for_record(&fixture.app, parent_id, &stopped.agent_id, |r| {
            r.status.is_terminal()
        })
        .await;
        assert_eq!(
            terminal.status,
            BgAgentStatus::Failed,
            "{mutation}: {:?}",
            terminal.error
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 1, "{mutation}");
        if mutation == "session" {
            assert!(session
                .lock()
                .await
                .messages
                .iter()
                .any(|m| m.content.content_text_only() == "new user edit"));
        }
        if mutation == "pause" {
            assert_eq!(
                session.lock().await.goal.as_ref().unwrap().status,
                crate::chat::types::GoalStatus::Paused
            );
        }
        fixture.app.agent_executions.shutdown().await;
        assert_eq!(fixture.app.work_scheduler.snapshot().active, 0);
    }
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
                    aborted: false,
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

async fn wait_until_slot_released(app: &AppState, child_chat_id: &str) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while app.agent_executions.get(child_chat_id).is_some()
            || app.agent_executions.task_active(child_chat_id)
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("child released its execution slot");
}

fn notice_source(message: &ChatMessage) -> Option<&str> {
    message.extra.get("event")?.get("source")?.as_str()
}

#[serial(test_runner)]
#[tokio::test]
async fn closed_child_releases_its_slot_and_resumes_the_same_trajectory_next_generation() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let fixture = repo_fixture().await;
    let parent_id = "close-resume-parent";
    let mut parent = crate::chat::types::ChatSession::new(parent_id.into());
    parent.work_project_root = Some(fixture.source.clone());
    parent.set_runtime_state(SessionState::Generating, None);
    fixture
        .app
        .chat
        .sessions
        .write()
        .await
        .insert(parent_id.into(), Arc::new(Mutex::new(parent)));
    let attempts = Arc::new(AtomicUsize::new(0));
    let running = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let finished = Arc::new(Notify::new());
    let _runner = {
        let attempts = attempts.clone();
        let running = running.clone();
        let release = release.clone();
        let finished = finished.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |gcx, mut messages, config| {
            let attempts = attempts.clone();
            let running = running.clone();
            let release = release.clone();
            let finished = finished.clone();
            Box::pin(async move {
                let app = AppState::from_gcx(gcx.clone()).await;
                let id = config.chat_id.as_ref().unwrap().clone();
                let session = app.chat.sessions.read().await.get(&id).cloned().unwrap();
                let thread = session.lock().await.thread.clone();
                if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    messages.push(ChatMessage::new(
                        "assistant".into(),
                        "first pass before the close".into(),
                    ));
                    save_trajectory_as(gcx, &thread, &messages).await;
                    mirror_subchat_messages_into_session(&app, &id, &messages).await;
                    finish_stateful_subchat_session(&app, &id, &config).await;
                    running.notify_one();
                    release.notified().await;
                    return Err("Aborted after the parent closed this child".into());
                }
                assert!(messages
                    .iter()
                    .any(|message| message.content.content_text_only()
                        == "first pass before the close"));
                let notice = messages
                    .iter()
                    .rev()
                    .find(|message| notice_source(message) == Some("agents.resume"))
                    .expect("closed children carry an explicit resume notice");
                assert_eq!(
                    notice.extra["event"]["payload"]["previous_status"],
                    "closed"
                );
                assert!(messages.iter().any(
                    |message| message.content.content_text_only() == "Finish what you started."
                ));
                let mut messages = messages;
                messages.push(ChatMessage::new(
                    "assistant".into(),
                    "closed work done".into(),
                ));
                save_trajectory_as(gcx, &thread, &messages).await;
                mirror_subchat_messages_into_session(&app, &id, &messages).await;
                finish_stateful_subchat_session(&app, &id, &config).await;
                finished.notify_one();
                Ok(SubchatResult {
                    messages,
                    metering: serde_json::Map::new(),
                    chat_id: Some(id),
                    aborted: false,
                })
            })
        }))
    };
    let handle = crate::agents::spawn::spawn_background_agent(
        fixture.app.clone(),
        spawn_request(parent_id, BgAgentKind::Subagent),
    )
    .await
    .unwrap();
    let agent_id = handle.agent_id.clone();
    let child_chat_id = handle.child_chat_id.clone();
    tokio::time::timeout(Duration::from_secs(30), running.notified())
        .await
        .expect("first attempt started");
    let live = fixture.app.agents.get_any(&agent_id).await.unwrap();
    assert!(!live.status.is_terminal());

    let (closed, already) = crate::agents::lifecycle::close_agent(
        fixture.app.clone(),
        parent_id,
        &agent_id,
        Some(live.run_generation),
        Some("paused by the parent".into()),
    )
    .await
    .unwrap();
    assert!(!already);
    assert_eq!(closed.status, BgAgentStatus::Closed);
    assert!(closed.status.is_terminal());
    assert!(fixture.app.agent_executions.get(&child_chat_id).is_none());

    release.notify_one();
    let late = await_completion(handle).await;
    assert_eq!(
        late.status,
        BgAgentStatus::Closed,
        "a late runner exit must not rewrite the close"
    );
    wait_until_slot_released(&fixture.app, &child_chat_id).await;
    assert_eq!(attempts.load(Ordering::SeqCst), 1);

    let stopped = fixture.app.agents.get_any(&agent_id).await.unwrap();
    assert_eq!(stopped.status, BgAgentStatus::Closed);
    assert_eq!(
        stopped.close_reason.as_deref(),
        Some("paused by the parent")
    );
    let resumed = crate::agents::spawn::resume_agent(
        fixture.app.clone(),
        parent_id,
        &agent_id,
        None,
        stopped.change_seq,
        Some("Finish what you started.".into()),
        false,
    )
    .await
    .unwrap();
    assert_eq!(resumed.status, BgAgentStatus::Queued);
    assert_eq!(
        resumed.child_chat_id.as_deref(),
        Some(child_chat_id.as_str())
    );
    assert_eq!(resumed.run_generation, stopped.run_generation + 1);
    assert_eq!(resumed.created_at, stopped.created_at);
    assert!(resumed.close_reason.is_none());
    assert_eq!(resumed.previous_runs.len(), 1);
    assert_eq!(resumed.previous_runs[0].status, BgAgentStatus::Closed);
    assert_eq!(resumed.previous_runs[0].run_generation, 1);

    tokio::time::timeout(Duration::from_secs(30), finished.notified())
        .await
        .expect("resumed attempt finished");
    let done = wait_for_record(&fixture.app, parent_id, &agent_id, |record| {
        record.status.is_terminal()
    })
    .await;
    assert_eq!(done.status, BgAgentStatus::Completed);
    assert_eq!(done.run_generation, 2);
    assert_eq!(done.result_summary.as_deref(), Some("closed work done"));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[serial(test_runner)]
#[tokio::test]
async fn completed_child_takes_a_continued_turn_without_rewriting_its_completion() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let fixture = repo_fixture().await;
    let parent_id = "continued-turn-parent";
    let mut parent = crate::chat::types::ChatSession::new(parent_id.into());
    parent.work_project_root = Some(fixture.source.clone());
    parent.set_runtime_state(SessionState::Generating, None);
    fixture
        .app
        .chat
        .sessions
        .write()
        .await
        .insert(parent_id.into(), Arc::new(Mutex::new(parent)));
    let attempts = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(Notify::new());
    let _runner = {
        let attempts = attempts.clone();
        let finished = finished.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |gcx, mut messages, config| {
            let attempts = attempts.clone();
            let finished = finished.clone();
            Box::pin(async move {
                let app = AppState::from_gcx(gcx.clone()).await;
                let id = config.chat_id.as_ref().unwrap().clone();
                let session = app.chat.sessions.read().await.get(&id).cloned().unwrap();
                let thread = session.lock().await.thread.clone();
                let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                if attempt == 1 {
                    assert!(messages
                        .iter()
                        .any(|message| message.content.content_text_only() == "first answer"));
                    let notice = messages
                        .iter()
                        .rev()
                        .find(|message| notice_source(message) == Some("agents.continued_turn"))
                        .expect("a continued turn is announced as such, not as a recovery");
                    assert_eq!(
                        notice.extra["event"]["payload"]["previous_status"],
                        "completed"
                    );
                    assert!(notice.content.content_text_only().contains("preserved"));
                    assert!(messages
                        .iter()
                        .all(|message| { notice_source(message) != Some("agents.resume") }));
                }
                messages.push(ChatMessage::new(
                    "assistant".into(),
                    if attempt == 0 {
                        "first answer"
                    } else {
                        "second answer"
                    }
                    .into(),
                ));
                save_trajectory_as(gcx, &thread, &messages).await;
                mirror_subchat_messages_into_session(&app, &id, &messages).await;
                finish_stateful_subchat_session(&app, &id, &config).await;
                if attempt == 1 {
                    finished.notify_one();
                }
                Ok(SubchatResult {
                    messages,
                    metering: serde_json::Map::new(),
                    chat_id: Some(id),
                    aborted: false,
                })
            })
        }))
    };
    let handle = crate::agents::spawn::spawn_background_agent(
        fixture.app.clone(),
        spawn_request(parent_id, BgAgentKind::Subagent),
    )
    .await
    .unwrap();
    let agent_id = handle.agent_id.clone();
    let child_chat_id = handle.child_chat_id.clone();
    let completed = await_completion(handle).await;
    assert_eq!(completed.status, BgAgentStatus::Completed);
    assert_eq!(completed.result_summary.as_deref(), Some("first answer"));
    wait_until_slot_released(&fixture.app, &child_chat_id).await;

    let before = fixture.app.agents.get_any(&agent_id).await.unwrap();
    let queued = crate::agents::spawn::resume_agent(
        fixture.app.clone(),
        parent_id,
        &agent_id,
        None,
        before.change_seq,
        Some("Now do the second half.".into()),
        false,
    )
    .await
    .unwrap();
    assert_eq!(
        queued.status,
        BgAgentStatus::Queued,
        "a continuation request is queued, never reported as the earlier completion"
    );
    assert_eq!(queued.run_generation, before.run_generation + 1);
    assert_eq!(
        queued.child_chat_id.as_deref(),
        Some(child_chat_id.as_str())
    );
    assert_eq!(queued.previous_runs.len(), 1);
    assert_eq!(queued.previous_runs[0].status, BgAgentStatus::Completed);
    assert_eq!(
        queued.previous_runs[0].result_summary.as_deref(),
        Some("first answer"),
        "the prior completion is preserved, not rewritten"
    );
    assert_eq!(queued.previous_runs[0].run_generation, 1);

    tokio::time::timeout(Duration::from_secs(30), finished.notified())
        .await
        .expect("continued turn finished");
    let done = wait_for_record(&fixture.app, parent_id, &agent_id, |record| {
        record.status.is_terminal()
    })
    .await;
    assert_eq!(done.status, BgAgentStatus::Completed);
    assert_eq!(done.run_generation, 2);
    assert_eq!(done.result_summary.as_deref(), Some("second answer"));
    assert_eq!(
        done.previous_runs[0].result_summary.as_deref(),
        Some("first answer")
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}
