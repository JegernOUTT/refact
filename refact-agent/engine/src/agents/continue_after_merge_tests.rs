use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::agents::spawn::{
    install_test_runner, resume_agent, spawn_background_agent, NotifyParent, SpawnWorktreeMode,
    TestRunnerGuard,
};
use crate::agents::spawn_lifecycle_tests::{await_completion, repo_fixture, spawn_request, RepoFixture};
use crate::agents::types::{BackgroundAgent, BgAgentKind, BgAgentStatus};
use crate::app_state::AppState;
use crate::call_validation::ChatMessage;
use crate::chat::types::{ChatSession, SessionState};
use crate::subchat::SubchatResult;
use crate::worktrees::types::WorktreeMeta;

const PARENT: &str = "continue-after-merge-parent";

fn git(root: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

async fn fixture(nested: bool) -> RepoFixture {
    let mut fixture = repo_fixture().await;
    std::fs::write(fixture.source.join(".gitignore"), ".refact/\n").unwrap();
    std::fs::write(fixture.source.join(".git/info/exclude"), ".refact/\n").unwrap();
    git(&fixture.source, &["add", ".gitignore"]);
    git(&fixture.source, &["commit", "-m", "ignore runtime state"]);
    let mut parent = ChatSession::new(PARENT.into());
    parent.work_project_root = Some(fixture.source.clone());
    parent.thread.mode = "agent".into();
    parent.thread.model = "openai/gpt-4o".into();
    if nested {
        let service = crate::worktrees::service::WorktreeService::new(
            fixture.cache.clone(),
            fixture.source.clone(),
        )
        .unwrap();
        fixture.parent_worktree = service
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                branch: Some("refact/subagent/parent".into()),
                base_branch: Some("work".into()),
                kind: Some("subagent".into()),
                chat_id: Some(PARENT.into()),
                ..Default::default()
            })
            .await
            .unwrap()
            .worktree
            .meta;
        parent.thread.worktree = Some(fixture.parent_worktree.clone());
    }
    parent.set_runtime_state(SessionState::Generating, None);
    fixture
        .app
        .chat
        .sessions
        .write()
        .await
        .insert(PARENT.into(), Arc::new(Mutex::new(parent)));
    fixture
}

fn runner(
    seen: Arc<Mutex<Vec<Option<WorktreeMeta>>>>,
    expected_head: Arc<Mutex<Option<String>>>,
    fail_first: bool,
) -> TestRunnerGuard {
    install_test_runner(Arc::new(move |gcx, mut messages, config| {
        let seen = seen.clone();
        let expected_head = expected_head.clone();
        Box::pin(async move {
            let app = AppState::from_gcx(gcx).await;
            let child = config.chat_id.as_ref().unwrap();
            let record = app
                .agents
                .get_any(config.background_agent_id.as_ref().unwrap())
                .await
                .unwrap();
            assert_eq!(record.status, BgAgentStatus::Running);
            assert!(!crate::agents::seal::is_sealed(&app, child).await);
            let session = app.chat.sessions.read().await.get(child).cloned().unwrap();
            assert_eq!(session.lock().await.thread.worktree, config.worktree);
            if let Some(worktree) = &config.worktree {
                assert!(worktree.root.is_dir());
                assert_eq!(record.worktree_id.as_ref(), Some(&worktree.id));
                if record.run_generation == 1 {
                    std::fs::write(worktree.root.join("A.txt"), "first generation\n").unwrap();
                } else {
                    assert_eq!(
                        std::fs::read_to_string(worktree.root.join("A.txt")).unwrap(),
                        "first generation\n"
                    );
                    if let Some(head) = expected_head.lock().await.as_ref() {
                        assert_eq!(&git(&worktree.root, &["rev-parse", "HEAD"]), head);
                        assert_eq!(
                            git(&worktree.root, &["show", "HEAD:A.txt"]),
                            "first generation"
                        );
                        assert!(worktree.root.join("target-latest.txt").exists());
                    }
                    std::fs::write(worktree.root.join("B.txt"), "second generation\n").unwrap();
                }
            }
            seen.lock().await.push(config.worktree.clone());
            messages.push(ChatMessage::new(
                "assistant".into(),
                format!("generation {} done", record.run_generation),
            ));
            let thread = session.lock().await.thread.clone();
            crate::chat::trajectories::save_trajectory_as(app.gcx.clone(), &thread, &messages)
                .await;
            crate::subchat::mirror_subchat_messages_into_session(&app, child, &messages).await;
            crate::subchat::finish_stateful_subchat_session(&app, child, &config).await;
            crate::chat::trajectories::flush_trajectory_for_session(app.clone(), session)
                .await
                .unwrap();
            if fail_first && record.run_generation == 1 {
                return Err("recoverable interruption".into());
            }
            Ok(SubchatResult {
                messages,
                chat_id: config.chat_id,
                metering: Default::default(),
                aborted: false,
            })
        })
    }))
}

async fn settled(app: &AppState, agent: &BackgroundAgent) -> BackgroundAgent {
    let child = agent.child_chat_id.as_deref().unwrap();
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let record = app.agents.get_any(&agent.agent_id).await.unwrap();
            if record.status.is_terminal()
                && !app.agent_executions.task_active(child)
                && app.agent_executions.get(child).is_none()
            {
                return record;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("generation settled")
}

async fn block_admission(app: &AppState) -> refact_work_core::Permit {
    let mut limits = crate::file_activity::settings();
    limits.max_active_engine = 1;
    limits.max_active_project = 1;
    limits.max_active_root = 1;
    app.work_scheduler.configure(limits).unwrap();
    app.work_scheduler
        .reserve(refact_work_core::AdmissionRequest::new(
            refact_work_core::WorkKey::background_agent("continuation-blocker").run(1),
            refact_work_core::WorkScope::single("blocker"),
        ))
        .unwrap()
        .acquire()
        .await
        .unwrap()
}

async fn continue_child(app: &AppState, before: &BackgroundAgent) -> BackgroundAgent {
    let queued = resume_agent(
        app.clone(),
        PARENT,
        &before.agent_id,
        None,
        before.change_seq,
        Some("Do the next generation.".into()),
        false,
    )
    .await
    .expect("continue completed child");
    assert_eq!(queued.run_generation, before.run_generation + 1);
    assert_eq!(queued.child_chat_id, before.child_chat_id);
    settled(app, &queued).await
}

#[tokio::test]
#[serial_test::serial(test_runner)]
async fn continue_after_merge_runs_in_a_fresh_worktree() {
    for nested in [false, true] {
        let fixture = fixture(nested).await;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let expected_head = Arc::new(Mutex::new(None));
        let _runner = runner(seen.clone(), expected_head.clone(), false);
        let mut request = spawn_request(PARENT, BgAgentKind::Subagent);
        request.worktree_mode = SpawnWorktreeMode::Isolated {
            auto_merge: true,
        };
        request.notify_parent = NotifyParent::Auto;
        if nested {
            request.parent_worktree = Some(fixture.parent_worktree.clone());
        }
        let first = await_completion(
            spawn_background_agent(fixture.app.clone(), request)
                .await
                .unwrap(),
        )
        .await;
        let first = settled(&fixture.app, &first).await;
        assert_eq!(
            first.merge_status.as_deref(),
            Some("merged"),
            "{:?}",
            first.error
        );
        let original = seen.lock().await[0].clone().unwrap();
        assert!(!original.root.exists());
        let target = if nested {
            &fixture.parent_worktree.root
        } else {
            &fixture.source
        };
        assert!(target.join("A.txt").exists());
        std::fs::write(target.join("target-latest.txt"), "new target HEAD\n").unwrap();
        git(target, &["add", "target-latest.txt"]);
        git(target, &["commit", "-m", "advance merge target"]);
        *expected_head.lock().await = Some(git(target, &["rev-parse", "HEAD"]));
        if !nested {
            fixture
                .app
                .chat
                .sessions
                .write()
                .await
                .remove(first.child_chat_id.as_ref().unwrap());
            git(&fixture.source, &["checkout", "main"]);
        }
        let blocker = block_admission(&fixture.app).await;
        let checkouts = git(&fixture.source, &["worktree", "list", "--porcelain"]);
        let branches = git(
            &fixture.source,
            &[
                "for-each-ref",
                "--format=%(refname):%(objectname)",
                "refs/heads",
            ],
        );
        let queued = resume_agent(
            fixture.app.clone(),
            PARENT,
            &first.agent_id,
            None,
            first.change_seq,
            None,
            false,
        )
        .await
        .unwrap();
        assert_eq!(queued.status, BgAgentStatus::Queued);
        assert_eq!(queued.child_chat_id, first.child_chat_id);
        assert!(queued.worktree_id.is_none());
        assert!(queued.worktree_branch.is_none());
        tokio::time::timeout(Duration::from_secs(10), async {
            while fixture.app.work_scheduler.snapshot().pending == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            git(&fixture.source, &["worktree", "list", "--porcelain"]),
            checkouts
        );
        assert_eq!(
            git(
                &fixture.source,
                &[
                    "for-each-ref",
                    "--format=%(refname):%(objectname)",
                    "refs/heads"
                ]
            ),
            branches
        );
        assert_eq!(seen.lock().await.len(), 1);
        let recovered =
            crate::agents::spawn::recover_run_config(&fixture.app, PARENT, &first.agent_id)
                .await
                .unwrap();
        assert_eq!(recovered.config.worktree, Some(original.clone()));
        drop(blocker);
        let done = settled(&fixture.app, &queued).await;
        assert_eq!(done.status, BgAgentStatus::Completed, "{:?}", done.error);
        assert_eq!(
            done.merge_status.as_deref(),
            Some("merged"),
            "{:?}",
            done.error
        );
        assert_eq!(done.run_generation, 2);
        let fresh = seen.lock().await[1].clone().unwrap();
        assert_ne!(original.id, fresh.id);
        assert_ne!(original.branch, fresh.branch);
        assert_eq!(original.kind, fresh.kind);
        assert_eq!(done.worktree_id.as_ref(), Some(&fresh.id));
        assert!(!fresh.root.exists());
        assert_eq!(
            std::fs::read_to_string(target.join("B.txt")).unwrap(),
            "second generation\n"
        );
        assert_eq!(done.previous_runs.len(), 1);
        let previous = &done.previous_runs[0];
        assert_eq!(previous.run_generation, 1);
        assert_eq!(previous.status, BgAgentStatus::Completed);
        assert_eq!(previous.worktree_id.as_ref(), Some(&original.id));
        assert_eq!(previous.worktree_branch, original.branch);
        assert_eq!(previous.merge_status.as_deref(), Some("merged"));
        let saved = crate::agents::spawn::recover_run_config(&fixture.app, PARENT, &done.agent_id)
            .await
            .unwrap();
        assert_eq!(saved.config.worktree, Some(fresh));
        let parent = fixture
            .app
            .chat
            .sessions
            .read()
            .await
            .get(PARENT)
            .cloned()
            .unwrap();
        let parent = parent.lock().await;
        let notices: Vec<_> = parent
            .messages
            .iter()
            .chain(
                parent
                    .pending_deliveries
                    .iter()
                    .flat_map(|delivery| &delivery.messages),
            )
            .filter_map(|message| message.extra.get("event")?.get("payload"))
            .filter(|payload| payload["agent_id"] == done.agent_id)
            .collect();
        assert_eq!(notices.len(), 2);
        for (index, notice) in notices.iter().enumerate() {
            assert_eq!(notice["run_generation"], index + 1);
            assert_eq!(notice["merge_status"], "merged");
        }
        drop(parent);
        fixture.app.agent_executions.shutdown().await;
    }
}

#[tokio::test]
#[serial_test::serial(test_runner)]
async fn continue_after_merge_refuses_when_target_is_gone() {
    let fixture = fixture(false).await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let _runner = runner(seen.clone(), Arc::new(Mutex::new(None)), false);
    let mut request = spawn_request(PARENT, BgAgentKind::Subagent);
    request.worktree_mode = SpawnWorktreeMode::Isolated {
        auto_merge: true,
    };
    let first = await_completion(
        spawn_background_agent(fixture.app.clone(), request)
            .await
            .unwrap(),
    )
    .await;
    let first = settled(&fixture.app, &first).await;
    assert_eq!(first.merge_status.as_deref(), Some("merged"));
    let merged = git(&fixture.source, &["rev-parse", "refs/heads/work"]);
    git(&fixture.source, &["checkout", "main"]);
    git(&fixture.source, &["branch", "-D", "work"]);
    let error = resume_agent(
        fixture.app.clone(),
        PARENT,
        &first.agent_id,
        None,
        first.change_seq,
        None,
        false,
    )
    .await
    .unwrap_err();
    assert!(error.contains("spawn a new agent instead"), "{error}");
    let unchanged = fixture.app.agents.get_any(&first.agent_id).await.unwrap();
    assert_eq!(unchanged, first);
    assert_eq!(seen.lock().await.len(), 1);
    let original = seen.lock().await[0].clone().unwrap();
    git(&fixture.source, &["branch", "work", &merged]);
    let child = first.child_chat_id.as_deref().unwrap();
    let before =
        crate::chat::trajectories::load_trajectory_for_chat(fixture.app.gcx.clone(), child)
            .await
            .unwrap();
    let original_messages = serde_json::to_value(&before.messages).unwrap();
    let runs = fixture.cache.join(".refact/background_agents/runs");
    let first_seed = std::fs::read(runs.join(format!("{}.1.json", first.agent_id))).unwrap();
    let launch_path = fixture
        .cache
        .join(".refact/background_agents/launches")
        .join(format!("{}.json", first.agent_id));
    let launch = std::fs::read(&launch_path).unwrap();
    let result_path = first.result_payload_path.as_ref().unwrap();
    let result = std::fs::read(result_path).unwrap();

    let checkouts = git(&fixture.source, &["worktree", "list", "--porcelain"]);
    let branches = git(
        &fixture.source,
        &[
            "for-each-ref",
            "--format=%(refname):%(objectname)",
            "refs/heads",
        ],
    );
    let (ack, receive) = tokio::sync::oneshot::channel();
    let queued = crate::agents::spawn::resume_agent_inner(
        fixture.app.clone(),
        PARENT,
        &first.agent_id,
        None,
        first.change_seq,
        None,
        false,
        receive,
    )
    .await
    .unwrap();
    assert!(queued.worktree_id.is_none());
    assert!(queued.worktree_branch.is_none());
    assert_eq!(
        git(&fixture.source, &["worktree", "list", "--porcelain"]),
        checkouts
    );
    assert_eq!(
        git(
            &fixture.source,
            &[
                "for-each-ref",
                "--format=%(refname):%(objectname)",
                "refs/heads"
            ]
        ),
        branches
    );
    drop(ack);
    let cancelled = settled(&fixture.app, &queued).await;
    assert_eq!(cancelled.status, BgAgentStatus::Cancelled);
    assert_eq!(seen.lock().await.len(), 1);
    assert_eq!(
        git(&fixture.source, &["worktree", "list", "--porcelain"]),
        checkouts
    );
    assert_eq!(
        serde_json::to_value(
            crate::chat::trajectories::load_trajectory_for_chat(fixture.app.gcx.clone(), child)
                .await
                .unwrap()
                .messages
        )
        .unwrap(),
        original_messages
    );
    assert!(
        crate::agents::spawn::recover_run_config(&fixture.app, PARENT, &first.agent_id)
            .await
            .is_ok()
    );
    let blocker = block_admission(&fixture.app).await;
    let queued = resume_agent(
        fixture.app.clone(),
        PARENT,
        &first.agent_id,
        None,
        cancelled.change_seq,
        None,
        false,
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while fixture.app.work_scheduler.snapshot().pending == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    git(&fixture.source, &["branch", "-D", "work"]);
    drop(blocker);
    let failed = settled(&fixture.app, &queued).await;
    assert_eq!(failed.status, BgAgentStatus::Failed);
    assert!(failed
        .error
        .as_ref()
        .unwrap()
        .contains("spawn a new agent instead"));
    assert_eq!(seen.lock().await.len(), 1);
    assert_eq!(failed.worktree_id.as_ref(), Some(&original.id));
    assert!(!original.root.exists());
    assert_eq!(
        git(&fixture.source, &["worktree", "list", "--porcelain"]),
        checkouts
    );
    git(&fixture.source, &["branch", "work", &merged]);
    let recovered = crate::agents::spawn::recover_run_config(&fixture.app, PARENT, &first.agent_id)
        .await
        .unwrap();
    assert_eq!(recovered.config.worktree, Some(original.clone()));
    assert_eq!(std::fs::read(result_path).unwrap(), result);
    let done = continue_child(&fixture.app, &failed).await;
    assert_eq!(done.agent_id, first.agent_id);
    assert_eq!(done.child_chat_id, first.child_chat_id);
    assert_eq!(done.run_generation, 4);
    assert_eq!(done.status, BgAgentStatus::Completed, "{:?}", done.error);
    assert_eq!(done.previous_runs[0].result_summary, first.result_summary);
    assert_eq!(done.previous_runs[0].worktree_id, first.worktree_id);
    assert_eq!(
        std::fs::read(runs.join(format!("{}.1.json", first.agent_id))).unwrap(),
        first_seed
    );
    assert_eq!(std::fs::read(launch_path).unwrap(), launch);
    assert_eq!(seen.lock().await.len(), 2);
    fixture.app.agent_executions.shutdown().await;
}

#[tokio::test]
#[serial_test::serial(test_runner)]
async fn resumed_generation_with_existing_worktree_is_merged() {
    let fixture = fixture(true).await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let _runner = runner(seen.clone(), Arc::new(Mutex::new(None)), true);
    let mut request = spawn_request(PARENT, BgAgentKind::Subagent);
    request.parent_worktree = Some(fixture.parent_worktree.clone());
    request.worktree_mode = SpawnWorktreeMode::Isolated {
        auto_merge: true,
    };
    let first = await_completion(
        spawn_background_agent(fixture.app.clone(), request)
            .await
            .unwrap(),
    )
    .await;
    let first = settled(&fixture.app, &first).await;
    assert_eq!(first.status, BgAgentStatus::Failed);
    assert!(seen.lock().await[0].as_ref().unwrap().root.exists());
    let done = continue_child(&fixture.app, &first).await;
    assert_eq!(done.status, BgAgentStatus::Completed, "{:?}", done.error);
    assert_eq!(done.run_generation, 2);
    assert_eq!(
        done.merge_status.as_deref(),
        Some("merged"),
        "{:?}",
        done.error
    );
    {
        let seen = seen.lock().await;
        assert_eq!(seen[0], seen[1]);
    }
    assert!(fixture.parent_worktree.root.join("A.txt").exists());
    assert!(fixture.parent_worktree.root.join("B.txt").exists());
    assert!(!fixture.source.join("B.txt").exists());
    fixture.app.agent_executions.shutdown().await;
}

#[tokio::test]
#[serial_test::serial(test_runner)]
async fn completed_child_continuation_lifetime_and_recovery() {
    for finite_goal in [false, true] {
        let fixture = fixture(false).await;
        let _runner = install_test_runner(Arc::new(move |gcx, mut messages, config| {
            Box::pin(async move {
                let app = AppState::from_gcx(gcx.clone()).await;
                let child = config.chat_id.as_ref().unwrap();
                let agent = config.background_agent_id.as_ref().unwrap();
                let record = app.agents.get_any(agent).await.unwrap();
                let session = app.chat.sessions.read().await.get(child).cloned().unwrap();
                assert_eq!(
                    session.lock().await.work_project_root,
                    app.agent_executions.saved_scope(agent)
                );
                assert!(config.worktree.is_none());
                let error = if record.run_generation == 2 {
                    assert_eq!(config.max_steps, 1);
                    let stop =
                        crate::subchat::agent_step_cap(&messages, Some(agent), config.max_steps);
                    assert_eq!((stop.used, stop.limit, stop.run_generation), (6, 6, 2));
                    assert_eq!(stop.parent_can_raise_steps(), !finite_goal);
                    let error = stop.as_error().unwrap();
                    config.step_progress.as_ref().unwrap()(crate::subchat::SubchatProgress::Step(
                        1,
                    ));
                    messages.push(crate::chat::internal_roles::event(
                        crate::chat::internal_roles::EventSubkind::SystemNotice,
                        "agents.cap_stop",
                        serde_json::json!({"agent_id": agent, "cap_stop": stop}),
                        "cap reached",
                    ));
                    Some(error)
                } else {
                    config.step_progress.as_ref().unwrap()(crate::subchat::SubchatProgress::Step(
                        if record.run_generation == 1 { 5 } else { 1 },
                    ));
                    messages.push(ChatMessage::new(
                        "assistant".into(),
                        "generation done".into(),
                    ));
                    None
                };
                let thread = session.lock().await.thread.clone();
                crate::chat::trajectories::save_trajectory_as(gcx, &thread, &messages).await;
                crate::subchat::mirror_subchat_messages_into_session(&app, child, &messages).await;
                crate::subchat::finish_stateful_subchat_session(&app, child, &config).await;
                crate::chat::trajectories::flush_trajectory_for_session(app.clone(), session)
                    .await
                    .unwrap();
                if let Some(error) = error {
                    return Err(error);
                }
                Ok(SubchatResult {
                    messages,
                    chat_id: config.chat_id,
                    metering: Default::default(),
                    aborted: false,
                })
            })
        }));
        let mut request = spawn_request(PARENT, BgAgentKind::Subagent);
        request.max_steps = 6;
        if finite_goal {
            request.goal = Some(crate::agents::spawn::SpawnGoal {
                content: "finite task".into(),
                criteria: Vec::new(),
                budget: Some(crate::chat::types::GoalBudget {
                    max_turns: Some(6),
                    explicit: true,
                    ..Default::default()
                }),
            });
        }
        let first = await_completion(
            spawn_background_agent(fixture.app.clone(), request)
                .await
                .unwrap(),
        )
        .await;
        let first = settled(&fixture.app, &first).await;
        assert_eq!(first.step_count, 5);
        fixture
            .app
            .chat
            .sessions
            .write()
            .await
            .remove(first.child_chat_id.as_ref().unwrap());
        let stopped = continue_child(&fixture.app, &first).await;
        assert_eq!(stopped.status, BgAgentStatus::Failed);
        assert_eq!(stopped.step_count, 6);
        assert_eq!(stopped.run_generation, 2);
        let ccx = Arc::new(Mutex::new(
            crate::at_commands::at_commands::AtCommandsContext::new_from_app(
                fixture.app.clone(),
                4096,
                1,
                false,
                Vec::new(),
                PARENT.into(),
                None,
                "openai/gpt-4o".into(),
                None,
                None,
            )
            .await,
        ));
        let inspect = std::collections::HashMap::from([
            ("agent_id".into(), serde_json::json!(stopped.agent_id)),
            ("action".into(), serde_json::json!("inspect")),
        ]);
        let (_, contexts) = crate::tools::tool_background_agents::agent_lifecycle_execute(
            ccx.clone(),
            &"inspect".into(),
            &inspect,
        )
        .await
        .unwrap();
        let crate::call_validation::ContextEnum::ChatMessage(message) = &contexts[0] else {
            panic!("tool message")
        };
        let receipt: serde_json::Value =
            serde_json::from_str(&message.content.content_text_only()).unwrap();
        assert_eq!(receipt["can_raise_and_resume"], !finite_goal);
        assert_eq!(receipt["resume_configuration_available"], true);
        let path = fixture
            .cache
            .join(".refact/background_agents/runs")
            .join(format!("{}.2.json", stopped.agent_id));
        let original = std::fs::read(&path).unwrap();
        for (pointer, value) in [
            ("/config/max_steps", serde_json::json!(1)),
            ("/run_generation", serde_json::json!(1)),
        ] {
            let mut corrupt: serde_json::Value = serde_json::from_slice(&original).unwrap();
            *corrupt.pointer_mut(pointer).unwrap() = value;
            std::fs::write(&path, serde_json::to_vec(&corrupt).unwrap()).unwrap();
            let (_, contexts) = crate::tools::tool_background_agents::agent_lifecycle_execute(
                ccx.clone(),
                &"inspect-corrupt".into(),
                &inspect,
            )
            .await
            .unwrap();
            let crate::call_validation::ContextEnum::ChatMessage(message) = &contexts[0] else {
                panic!("tool message")
            };
            let receipt: serde_json::Value =
                serde_json::from_str(&message.content.content_text_only()).unwrap();
            assert_eq!(receipt["can_raise_and_resume"], false);
            assert_eq!(receipt["can_resume"], false);
            assert!(receipt["recovery_unavailable_reason"].as_str().is_some());
            assert_eq!(
                fixture.app.agents.get_any(&stopped.agent_id).await.unwrap(),
                stopped
            );
        }
        std::fs::write(&path, original).unwrap();
        let raised = resume_agent(
            fixture.app.clone(),
            PARENT,
            &stopped.agent_id,
            Some(7),
            stopped.change_seq,
            None,
            false,
        )
        .await;
        if finite_goal {
            assert!(raised
                .unwrap_err()
                .contains("only a resumable child max_steps cap can be raised"));
            assert_eq!(
                fixture.app.agents.get_any(&stopped.agent_id).await.unwrap(),
                stopped
            );
        } else {
            let completed = settled(&fixture.app, &raised.unwrap()).await;
            assert_eq!(completed.status, BgAgentStatus::Completed);
            assert_eq!(completed.step_count, 7);
            assert_eq!(completed.run_generation, 3);
        }
        fixture.app.agent_executions.shutdown().await;
    }
}
