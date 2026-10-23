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
        request.worktree_mode = SpawnWorktreeMode::Isolated { auto_merge: true };
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
        let done = continue_child(&fixture.app, &first).await;
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
    request.worktree_mode = SpawnWorktreeMode::Isolated { auto_merge: true };
    let first = await_completion(
        spawn_background_agent(fixture.app.clone(), request)
            .await
            .unwrap(),
    )
    .await;
    let first = settled(&fixture.app, &first).await;
    assert_eq!(first.merge_status.as_deref(), Some("merged"));
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
    request.worktree_mode = SpawnWorktreeMode::Isolated { auto_merge: true };
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
async fn unscoped_child_continue_unchanged() {
    let fixture = fixture(false).await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let _runner = runner(seen.clone(), Arc::new(Mutex::new(None)), false);
    let request = spawn_request(PARENT, BgAgentKind::Subagent);
    let first = await_completion(
        spawn_background_agent(fixture.app.clone(), request)
            .await
            .unwrap(),
    )
    .await;
    let first = settled(&fixture.app, &first).await;
    let done = continue_child(&fixture.app, &first).await;
    assert_eq!(done.status, BgAgentStatus::Completed, "{:?}", done.error);
    assert_eq!(done.run_generation, 2);
    assert!(done.worktree_id.is_none());
    assert!(done.merge_status.is_none());
    assert_eq!(*seen.lock().await, vec![None, None]);
    fixture.app.agent_executions.shutdown().await;
}
