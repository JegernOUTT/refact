use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;

use crate::app_state::EngineChatSessionFacade;
use crate::global_context::GlobalContext;
use crate::tasks::types::{TaskBoard, TaskMeta};
use crate::tools::tools_description::Tool;

use super::merge_agent_target_checkout_tests::git;
use super::tool_agent_lifecycle::tests::{args, planner_ccx};
use super::tool_task_merge_agent::ToolTaskMergeAgent;

const AGENT: &str = "refact/task/task-1/card/T-1/legacy";

struct Fixture {
    _temp: tempfile::TempDir,
    gcx: Arc<GlobalContext>,
    source: PathBuf,
    target: PathBuf,
    agent: PathBuf,
    target_branch: String,
}

impl Fixture {
    async fn new(linked_target: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir(&source).unwrap();
        git(&source, &["init"]);
        git(&source, &["checkout", "-b", "main"]);
        git(&source, &["config", "core.autocrlf", "false"]);
        git(&source, &["config", "user.name", "Test User"]);
        git(&source, &["config", "user.email", "test@example.com"]);
        for file in ["file.txt", "second.txt"] {
            std::fs::write(source.join(file), "base\n").unwrap();
        }
        std::fs::write(source.join(".gitignore"), ".refact/\n").unwrap();
        git(&source, &["add", "-A"]);
        git(&source, &["commit", "-m", "initial"]);
        let base_commit = git(&source, &["rev-parse", "HEAD"]);
        let target_branch = if linked_target { "planner" } else { "main" }.to_string();
        let target = if linked_target {
            let target = temp.path().join("target checkout");
            git(
                &source,
                &[
                    "worktree",
                    "add",
                    "-b",
                    &target_branch,
                    target.to_str().unwrap(),
                ],
            );
            target
        } else {
            source.clone()
        };
        let agent = temp.path().join("unregistered agent");
        git(
            &source,
            &["worktree", "add", "-b", AGENT, agent.to_str().unwrap()],
        );
        for file in ["file.txt", "second.txt"] {
            std::fs::write(agent.join(file), "agent change\n").unwrap();
            std::fs::write(target.join(file), "target change\n").unwrap();
        }
        std::fs::write(agent.join("agent-only.txt"), "cleanly merged addition\n").unwrap();
        git(&agent, &["add", "-A"]);
        git(&agent, &["commit", "-m", "agent changes"]);
        git(&target, &["add", "-A"]);
        git(&target, &["commit", "-m", "conflicting target changes"]);
        if linked_target {
            std::fs::write(source.join("notes.txt"), "unrelated user notes\n").unwrap();
        }
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            temp.path().join("cache"),
            temp.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        let mut board = TaskBoard::default();
        board.cards.push(
            serde_json::from_value(json!({
                "id": "T-1",
                "title": "Legacy conflict regression",
                "column": "done",
                "instructions": "",
                "created_at": chrono::Utc::now().to_rfc3339(),
                "final_report": "done",
                "verifier_report": {
                    "passed": true, "classification": "passed", "recommendation": "merge",
                    "command_results": [{
                        "command": "fixture verification", "exit_code": 0,
                        "passed": true, "outcome": "passed"
                    }]
                },
                "agent_branch": AGENT,
                "agent_worktree": agent,
                "base_branch": target_branch,
                "base_commit": base_commit.trim()
            }))
            .unwrap(),
        );
        let meta: TaskMeta = serde_json::from_value(json!({
            "id": "task-1",
            "name": "Legacy conflict regression",
            "status": "active",
            "created_at": chrono::Utc::now().to_rfc3339(),
            "updated_at": chrono::Utc::now().to_rfc3339(),
            "base_branch": target_branch,
            "base_commit": base_commit.trim()
        }))
        .unwrap();
        let task = source.join(".refact/tasks/task-1");
        std::fs::create_dir_all(&task).unwrap();
        std::fs::write(
            task.join("board.yaml"),
            serde_yaml::to_string(&board).unwrap(),
        )
        .unwrap();
        std::fs::write(
            task.join("meta.yaml"),
            serde_yaml::to_string(&meta).unwrap(),
        )
        .unwrap();
        Self {
            _temp: temp,
            gcx,
            source,
            target,
            agent,
            target_branch,
        }
    }

    async fn merge(
        &self,
        strategy: &str,
    ) -> Result<(bool, Vec<crate::call_validation::ContextEnum>), String> {
        let facade = Arc::new(EngineChatSessionFacade::new(self.gcx.clone()));
        let ccx = planner_ccx(self.gcx.clone(), facade, "planner").await;
        ToolTaskMergeAgent::new()
            .tool_execute(
                ccx,
                &"legacy-conflict".to_string(),
                &args(&[("card_id", json!("T-1")), ("strategy", json!(strategy))]),
            )
            .await
    }
}

fn snapshot(root: &Path) -> Vec<String> {
    [
        vec!["show-ref"],
        vec!["status", "--porcelain"],
        vec!["diff", "--cached"],
        vec!["diff"],
        vec!["ls-files", "--stage"],
        vec!["worktree", "list", "--porcelain"],
    ]
    .map(|args| git(root, &args))
    .to_vec()
}

fn git_state_path(root: &Path, name: &str) -> PathBuf {
    let path = PathBuf::from(git(root, &["rev-parse", "--git-path", name]).trim());
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn legacy_merge_conflict_leaves_no_merge_state() {
    for strategy in ["squash", "merge"] {
        for linked_target in [false, true] {
            let fixture = Fixture::new(linked_target).await;
            let roots = [&fixture.source, &fixture.target, &fixture.agent];
            let before = roots.map(|root| snapshot(root));
            let head = git(&fixture.target, &["rev-parse", "HEAD"]);
            let board_path = fixture.source.join(".refact/tasks/task-1/board.yaml");
            let board = std::fs::read(&board_path).unwrap();

            fixture.merge(strategy).await.unwrap();

            assert!(
                git(&fixture.target, &["status", "--porcelain"]).is_empty(),
                "{strategy}: conflicted merge must restore a clean target"
            );
            assert!(!git_state_path(&fixture.target, "MERGE_HEAD").exists());
            assert!(!git_state_path(&fixture.target, "MERGE_MSG").exists());
            assert!(!git_state_path(&fixture.target, "SQUASH_MSG").exists());
            assert_eq!(git(&fixture.target, &["rev-parse", "HEAD"]), head);
            assert!(!fixture.target.join("agent-only.txt").exists());
            assert_eq!(before, roots.map(|root| snapshot(root)));
            assert_eq!(std::fs::read(board_path).unwrap(), board);
            for file in ["file.txt", "second.txt"] {
                assert_eq!(
                    std::fs::read_to_string(fixture.target.join(file)).unwrap(),
                    "target change\n"
                );
            }
        }
    }
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn dirty_target_conflict_is_rolled_back_and_preserves_user_edits() {
    for strategy in ["squash", "merge"] {
        for linked_target in [false, true] {
            let fixture = Fixture::new(linked_target).await;
            std::fs::write(fixture.target.join("file.txt"), "staged user edit\n").unwrap();
            git(&fixture.target, &["add", "file.txt"]);
            std::fs::write(fixture.target.join("file.txt"), "unstaged user edit\n").unwrap();
            std::fs::write(fixture.target.join("untracked.txt"), "user notes\n").unwrap();
            std::fs::write(fixture.agent.join("pending.txt"), "not auto-committed\n").unwrap();
            let roots = [&fixture.source, &fixture.target];
            let before = roots.map(|root| snapshot(root));
            let board_path = fixture.source.join(".refact/tasks/task-1/board.yaml");
            let board = std::fs::read(&board_path).unwrap();

            let head = git(&fixture.target, &["rev-parse", "HEAD"]);

            let result = fixture.merge(strategy).await.unwrap();

            let crate::call_validation::ContextEnum::ChatMessage(message) = &result.1[0] else {
                panic!("expected chat message")
            };
            let text = message.content.content_text_only();
            assert!(text.contains("Merge Conflicts Detected"), "{text}");
            assert!(!text.to_lowercase().contains("merged"), "{text}");
            assert_eq!(git(&fixture.target, &["rev-parse", "HEAD"]), head);
            assert!(git(&fixture.target, &["stash", "list"]).trim().is_empty());
            assert_eq!(
                git(&fixture.target, &["show", ":file.txt"]),
                "staged user edit\n"
            );
            let after = roots.map(|root| snapshot(root));
            for (before, after) in before.iter().zip(after.iter()) {
                assert_eq!(before[1..5], after[1..5]);
            }
            assert_eq!(
                git(&fixture.target, &["rev-parse", &fixture.target_branch]),
                head
            );
            assert_eq!(std::fs::read(board_path).unwrap(), board);
            for name in ["MERGE_HEAD", "MERGE_MSG", "SQUASH_MSG"] {
                assert!(!git_state_path(&fixture.target, name).exists(), "{name}");
            }
            assert_eq!(
                std::fs::read_to_string(fixture.target.join("file.txt")).unwrap(),
                "unstaged user edit\n"
            );
            assert_eq!(
                std::fs::read_to_string(fixture.target.join("untracked.txt")).unwrap(),
                "user notes\n"
            );
            assert_eq!(
                git(&fixture.agent, &["show", "HEAD:pending.txt"]),
                "not auto-committed\n"
            );
        }
    }
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn legacy_squash_commit_failure_retains_recoverable_state_and_user_edits() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new(true).await;
    for file in ["file.txt", "second.txt"] {
        std::fs::write(fixture.agent.join(file), "target change\n").unwrap();
    }
    git(&fixture.agent, &["add", "-A"]);
    git(
        &fixture.agent,
        &["commit", "-m", "resolve target conflicts"],
    );
    let hook = fixture._temp.path().join("hooks");
    std::fs::create_dir(&hook).unwrap();
    std::fs::write(hook.join("pre-commit"), "#!/bin/sh\nprintf 'staged user sentinel\\n' > file.txt\ngit add file.txt || exit 2\nprintf 'unstaged user sentinel\\n' > file.txt\nprintf 'untracked user sentinel\\n' > user-notes.txt\nexit 1\n").unwrap();
    std::fs::set_permissions(
        hook.join("pre-commit"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    git(
        &fixture.target,
        &["config", "core.hooksPath", hook.to_str().unwrap()],
    );
    let head = git(&fixture.target, &["rev-parse", "HEAD"]);
    let source = snapshot(&fixture.source);
    let agent = snapshot(&fixture.agent);
    let board_path = fixture.source.join(".refact/tasks/task-1/board.yaml");
    let board = std::fs::read(&board_path).unwrap();

    let error = fixture.merge("squash").await.unwrap_err();

    assert!(error.contains("Failed to commit squash merge"), "{error}");
    assert!(error.contains("integration state retained"), "{error}");
    assert_eq!(git(&fixture.target, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        git(&fixture.target, &["show", ":file.txt"]),
        "staged user sentinel\n"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.target.join("file.txt")).unwrap(),
        "unstaged user sentinel\n"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.target.join("user-notes.txt")).unwrap(),
        "untracked user sentinel\n"
    );
    assert_eq!(
        git(&fixture.target, &["show", ":agent-only.txt"]),
        "cleanly merged addition\n"
    );
    assert_eq!(snapshot(&fixture.source), source);
    assert_eq!(snapshot(&fixture.agent), agent);
    assert_eq!(std::fs::read(board_path).unwrap(), board);
    assert!(git_state_path(&fixture.target, "SQUASH_MSG").exists());
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn legacy_merge_unavailable_post_check_preserves_source() {
    let fixture = Fixture::new(true).await;
    for file in ["file.txt", "second.txt"] {
        std::fs::write(fixture.agent.join(file), "target change\n").unwrap();
    }
    git(&fixture.agent, &["add", "-A"]);
    git(
        &fixture.agent,
        &["commit", "-m", "resolve target conflicts"],
    );
    let head = git(&fixture.target, &["rev-parse", "HEAD"]);
    let source = snapshot(&fixture.source);
    let agent = snapshot(&fixture.agent);
    let facade = Arc::new(EngineChatSessionFacade::new(fixture.gcx.clone()));
    let ccx = planner_ccx(fixture.gcx.clone(), facade, "planner").await;

    let result = ToolTaskMergeAgent::new()
        .tool_execute(
            ccx,
            &"legacy-post-check".to_string(),
            &args(&[
                ("card_id", json!("T-1")),
                ("strategy", json!("squash")),
                ("auto_revert", json!(true)),
            ]),
        )
        .await
        .unwrap();

    assert_ne!(git(&fixture.target, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        std::fs::read_to_string(fixture.target.join("agent-only.txt")).unwrap(),
        "cleanly merged addition\n"
    );
    let crate::call_validation::ContextEnum::ChatMessage(message) = &result.1[0] else {
        panic!("expected tool message");
    };
    let crate::call_validation::ChatContent::SimpleText(text) = &message.content else {
        panic!("expected text");
    };
    assert!(text.contains("no verification commands"), "{text}");
    assert!(text.contains("Worktree and branch retained"), "{text}");
    assert_eq!(snapshot(&fixture.source)[1..5], source[1..5]);
    assert_eq!(snapshot(&fixture.agent)[1..5], agent[1..5]);
    assert!(fixture.agent.exists());
    assert_eq!(
        git(&fixture.source, &["rev-parse", AGENT]),
        git(&fixture.agent, &["rev-parse", "HEAD"])
    );
    let board = crate::tasks::storage::load_board(fixture.gcx.clone(), "task-1")
        .await
        .unwrap();
    let card = board.get_card("T-1").unwrap();
    assert_eq!(card.agent_branch.as_deref(), Some(AGENT));
    assert_eq!(
        card.agent_worktree.as_deref(),
        Some(fixture.agent.to_str().unwrap())
    );
}

fn resolve_agent_conflicts(fixture: &Fixture) {
    for file in ["file.txt", "second.txt"] {
        std::fs::write(fixture.agent.join(file), "target change\n").unwrap();
    }
    git(&fixture.agent, &["add", "-A"]);
    git(
        &fixture.agent,
        &["commit", "-m", "resolve target conflicts"],
    );
}

struct UserEdits {
    head: String,
    branch: String,
    index: String,
    cached: String,
    status: String,
}

fn make_user_edits(target: &Path) -> UserEdits {
    std::fs::write(target.join("file.txt"), "staged user edit\n").unwrap();
    git(target, &["add", "file.txt"]);
    std::fs::write(target.join("file.txt"), "unstaged user edit\n").unwrap();
    std::fs::write(target.join("untracked.txt"), "user notes\n").unwrap();
    UserEdits {
        head: git(target, &["rev-parse", "HEAD"]),
        branch: git(target, &["branch", "--show-current"]),
        index: git(target, &["ls-files", "--stage"]),
        cached: git(target, &["diff", "--cached"]),
        status: git(target, &["status", "--porcelain"]),
    }
}

fn assert_user_edits_restored(target: &Path, before: &UserEdits, context: &str) {
    assert_eq!(
        git(target, &["rev-parse", "HEAD"]),
        before.head,
        "{context}"
    );
    assert_eq!(
        git(target, &["branch", "--show-current"]),
        before.branch,
        "{context}"
    );
    assert_eq!(
        git(target, &["ls-files", "--stage"]),
        before.index,
        "{context}"
    );
    assert_eq!(
        git(target, &["diff", "--cached"]),
        before.cached,
        "{context}"
    );
    assert_eq!(
        git(target, &["status", "--porcelain"]),
        before.status,
        "{context}"
    );
    assert_eq!(
        std::fs::read(target.join("file.txt")).unwrap(),
        b"unstaged user edit\n",
        "{context}"
    );
    assert_eq!(
        git(target, &["show", ":file.txt"]),
        "staged user edit\n",
        "{context}"
    );
    assert_eq!(
        std::fs::read(target.join("untracked.txt")).unwrap(),
        b"user notes\n",
        "{context}"
    );
    assert!(!target.join("agent-only.txt").exists(), "{context}");
    for name in ["MERGE_HEAD", "MERGE_MSG", "SQUASH_MSG"] {
        assert!(!git_state_path(target, name).exists(), "{context}: {name}");
    }
    assert!(
        git(target, &["stash", "list"]).trim().is_empty(),
        "{context}: stash must not be left behind"
    );
}

fn merge_markers(fixture: &Fixture) -> Vec<String> {
    let board: TaskBoard = serde_yaml::from_slice(
        &std::fs::read(fixture.source.join(".refact/tasks/task-1/board.yaml")).unwrap(),
    )
    .unwrap();
    board
        .get_card("T-1")
        .unwrap()
        .status_updates
        .iter()
        .filter(|update| {
            update
                .message
                .starts_with(super::tool_task_merge_agent::MERGE_AGENT_STATUS_PREFIX)
        })
        .map(|update| update.message.clone())
        .collect()
}

fn tool_text(result: &(bool, Vec<crate::call_validation::ContextEnum>)) -> String {
    let crate::call_validation::ContextEnum::ChatMessage(message) = &result.1[0] else {
        panic!("expected chat message")
    };
    message.content.content_text_only()
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn authority_loss_after_stash_restores_user_edits_and_leaves_no_stash() {
    use super::tool_task_merge_agent::legacy_fault::{install, Action, Point};

    for (point, strategy) in [
        (Point::AfterStash, "squash"),
        (Point::AfterStash, "merge"),
        (Point::AfterMerge, "squash"),
    ] {
        {
            let context = format!("{point:?}/{strategy}");
            let fixture = Fixture::new(true).await;
            resolve_agent_conflicts(&fixture);
            let before = make_user_edits(&fixture.target);
            let agent_head = git(&fixture.agent, &["rev-parse", "HEAD"]);
            install(&fixture.target, point, Action::Fail);

            let error = fixture.merge(strategy).await.unwrap_err();

            assert!(
                error.contains("injected merge authority loss"),
                "{context}: {error}"
            );
            assert!(
                error.contains("uncommitted target edits were restored"),
                "{context}: {error}"
            );
            assert_user_edits_restored(&fixture.target, &before, &context);
            assert!(fixture.agent.exists(), "{context}");
            assert_eq!(git(&fixture.agent, &["rev-parse", "HEAD"]), agent_head);
            assert!(merge_markers(&fixture).is_empty(), "{context}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn cancelling_merge_after_stash_still_restores_user_edits() {
    use super::tool_task_merge_agent::legacy_fault::{install, Action, Point};

    let fixture = Arc::new(Fixture::new(true).await);
    resolve_agent_conflicts(&fixture);
    let before = make_user_edits(&fixture.target);
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    install(
        &fixture.target,
        Point::AfterStash,
        Action::Block {
            entered: entered_tx,
            release: release_rx,
        },
    );
    let running = fixture.clone();
    let merge = tokio::spawn(async move { running.merge("squash").await });
    tokio::task::spawn_blocking(move || {
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(120))
            .expect("merge section must reach the stash")
    })
    .await
    .unwrap();
    assert!(!git(&fixture.target, &["stash", "list"]).trim().is_empty());
    merge.abort();
    assert!(merge.await.unwrap_err().is_cancelled());
    release_tx.send(()).unwrap();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !crate::worktrees::git::run_git(&fixture.target, &["stash", "list"])
        .is_ok_and(|list| list.trim().is_empty())
    {
        assert!(
            std::time::Instant::now() < deadline,
            "cancelled merge abandoned the user's stash"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    for name in ["MERGE_HEAD", "SQUASH_MSG"] {
        assert!(!git_state_path(&fixture.target, name).exists(), "{name}");
    }
    assert_eq!(
        std::fs::read(fixture.target.join("file.txt")).unwrap(),
        b"unstaged user edit\n"
    );
    assert_eq!(
        git(&fixture.target, &["show", ":file.txt"]),
        "staged user edit\n"
    );
    assert_eq!(
        std::fs::read(fixture.target.join("untracked.txt")).unwrap(),
        b"user notes\n"
    );
    assert_eq!(
        git(&fixture.target, &["branch", "--show-current"]),
        before.branch
    );
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn ignored_target_file_colliding_with_incoming_tracked_file_is_not_clobbered() {
    for strategy in ["squash", "merge"] {
        for dirty in [false, true] {
            let context = format!("{strategy}/dirty={dirty}");
            let fixture = Fixture::new(true).await;
            resolve_agent_conflicts(&fixture);
            let common =
                PathBuf::from(git(&fixture.target, &["rev-parse", "--git-common-dir"]).trim());
            let common = if common.is_absolute() {
                common
            } else {
                fixture.target.join(common)
            };
            std::fs::create_dir_all(common.join("info")).unwrap();
            std::fs::write(common.join("info/exclude"), "*.log\n").unwrap();
            std::fs::write(fixture.agent.join("build.log"), "incoming tracked log\n").unwrap();
            git(&fixture.agent, &["add", "-f", "build.log"]);
            git(&fixture.agent, &["commit", "-m", "track a log"]);
            std::fs::write(fixture.target.join("build.log"), "precious ignored bytes\n").unwrap();
            let before = dirty.then(|| make_user_edits(&fixture.target));
            let head = git(&fixture.target, &["rev-parse", "HEAD"]);

            let result = fixture.merge(strategy).await.unwrap();

            let text = tool_text(&result);
            assert!(text.contains("Merge Not Performed"), "{context}: {text}");
            assert!(text.contains("build.log"), "{context}: {text}");
            assert!(!text.contains("Agent Work Merged"), "{context}: {text}");
            assert_eq!(
                std::fs::read(fixture.target.join("build.log")).unwrap(),
                b"precious ignored bytes\n",
                "{context}"
            );
            assert_eq!(
                git(&fixture.target, &["rev-parse", "HEAD"]),
                head,
                "{context}"
            );
            if let Some(before) = before.as_ref() {
                assert_user_edits_restored(&fixture.target, before, &context);
            } else {
                assert!(git(&fixture.target, &["status", "--porcelain"]).is_empty());
                assert!(git(&fixture.target, &["stash", "list"]).trim().is_empty());
            }
            assert!(fixture.agent.exists(), "{context}");
            assert!(merge_markers(&fixture).is_empty(), "{context}");
        }
    }
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn merging_done_card_records_exactly_one_marker_and_conflict_records_none() {
    for strategy in ["squash", "merge"] {
        let fixture = Fixture::new(true).await;
        resolve_agent_conflicts(&fixture);
        let result = fixture.merge(strategy).await.unwrap();
        assert!(
            tool_text(&result).contains("Agent Work Merged"),
            "{strategy}"
        );
        let markers = merge_markers(&fixture);
        assert_eq!(markers.len(), 1, "{strategy}: {markers:?}");
        assert!(markers[0].contains(strategy), "{markers:?}");
        assert!(markers[0].contains(&fixture.target_branch), "{markers:?}");

        let conflicted = Fixture::new(true).await;
        let result = conflicted.merge(strategy).await.unwrap();
        assert!(tool_text(&result).contains("Merge Conflicts Detected"));
        assert!(merge_markers(&conflicted).is_empty(), "{strategy}");
    }
}

fn board_card(fixture: &Fixture) -> crate::tasks::types::BoardCard {
    let board: TaskBoard = serde_yaml::from_slice(
        &std::fs::read(fixture.source.join(".refact/tasks/task-1/board.yaml")).unwrap(),
    )
    .unwrap();
    board.get_card("T-1").unwrap().clone()
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn late_authority_loss_after_applied_merge_is_merged_with_warning_and_one_marker() {
    use super::tool_task_merge_agent::legacy_fault::{install, Action, Point};

    for strategy in ["squash", "merge"] {
        let fixture = Fixture::new(true).await;
        resolve_agent_conflicts(&fixture);
        make_user_edits(&fixture.target);
        let head = git(&fixture.target, &["rev-parse", "HEAD"]);
        install(&fixture.target, Point::AfterCommit, Action::Fail);

        let result = fixture
            .merge(strategy)
            .await
            .expect("an applied merge is never reported as an error");

        let text = tool_text(&result);
        assert!(text.contains("Agent Work Merged"), "{strategy}: {text}");
        assert!(
            text.contains("injected merge authority loss"),
            "{strategy}: {text}"
        );
        assert!(text.contains("commit already landed"), "{strategy}: {text}");
        assert_ne!(git(&fixture.target, &["rev-parse", "HEAD"]), head);
        assert_eq!(
            std::fs::read_to_string(fixture.target.join("agent-only.txt")).unwrap(),
            "cleanly merged addition\n"
        );
        assert_eq!(
            std::fs::read(fixture.target.join("file.txt")).unwrap(),
            b"unstaged user edit\n"
        );
        assert_eq!(
            git(&fixture.target, &["show", ":file.txt"]),
            "staged user edit\n"
        );
        assert_eq!(
            std::fs::read(fixture.target.join("untracked.txt")).unwrap(),
            b"user notes\n"
        );
        assert!(git(&fixture.target, &["stash", "list"]).trim().is_empty());
        assert_eq!(merge_markers(&fixture).len(), 1, "{strategy}");
        assert!(fixture.agent.exists(), "{strategy}");
    }
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn board_failure_after_applied_merge_is_merged_with_warning_and_one_marker() {
    for strategy in ["squash", "merge"] {
        let fixture = Fixture::new(true).await;
        resolve_agent_conflicts(&fixture);
        make_user_edits(&fixture.target);
        let head = git(&fixture.target, &["rev-parse", "HEAD"]);

        let result = super::tool_task_merge_agent::FAIL_BOARD_AFTER_MERGE
            .scope(true, fixture.merge(strategy))
            .await
            .expect("an applied merge is never reported as an error");

        let text = tool_text(&result);
        assert!(text.contains("Agent Work Merged"), "{strategy}: {text}");
        assert!(
            text.contains("injected board update failure"),
            "{strategy}: {text}"
        );
        assert_ne!(git(&fixture.target, &["rev-parse", "HEAD"]), head);
        assert_eq!(
            std::fs::read(fixture.target.join("untracked.txt")).unwrap(),
            b"user notes\n"
        );
        assert!(git(&fixture.target, &["stash", "list"]).trim().is_empty());
        assert_eq!(merge_markers(&fixture).len(), 1, "{strategy}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn cancelled_tool_future_still_completes_merge_bookkeeping() {
    use super::tool_task_merge_agent::merge_task_gate::{install, Gate};

    let fixture = Arc::new(Fixture::new(true).await);
    resolve_agent_conflicts(&fixture);
    make_user_edits(&fixture.target);
    let head = git(&fixture.target, &["rev-parse", "HEAD"]);
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    install(
        "T-1",
        Gate {
            entered: entered_tx,
            release: release_rx,
        },
    );
    let running = fixture.clone();
    let merge = tokio::spawn(async move { running.merge("squash").await });
    entered_rx.await.expect("merge task must start");
    merge.abort();
    assert!(merge.await.unwrap_err().is_cancelled());
    release_tx.send(()).unwrap();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        let marker_written = merge_markers(&fixture).len() == 1;
        let stash_empty = crate::worktrees::git::run_git(&fixture.target, &["stash", "list"])
            .is_ok_and(|list| list.trim().is_empty());
        if marker_written && stash_empty {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "cancelled merge never completed its bookkeeping"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_ne!(git(&fixture.target, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        std::fs::read_to_string(fixture.target.join("agent-only.txt")).unwrap(),
        "cleanly merged addition\n"
    );
    assert_eq!(
        std::fs::read(fixture.target.join("file.txt")).unwrap(),
        b"unstaged user edit\n"
    );
    assert_eq!(
        std::fs::read(fixture.target.join("untracked.txt")).unwrap(),
        b"user notes\n"
    );
    assert_eq!(merge_markers(&fixture).len(), 1);
    let _ = board_card(&fixture);
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn authority_loss_inside_the_commit_window_keeps_the_commit_and_records_one_marker() {
    use super::tool_task_merge_agent::legacy_fault::{install, Action, Point};

    for strategy in ["squash", "merge"] {
        for dirty in [true, false] {
            let context = format!("{strategy}/dirty={dirty}");
            let fixture = Fixture::new(true).await;
            resolve_agent_conflicts(&fixture);
            if dirty {
                make_user_edits(&fixture.target);
            }
            let head = git(&fixture.target, &["rev-parse", "HEAD"]);
            install(&fixture.target, Point::CommitWindow, Action::Fail);

            let result = fixture
                .merge(strategy)
                .await
                .unwrap_or_else(|error| panic!("{context}: {error}"));

            let text = tool_text(&result);
            assert!(text.contains("Agent Work Merged"), "{context}: {text}");
            assert!(text.contains("commit already landed"), "{context}: {text}");
            assert_eq!(
                git(&fixture.target, &["rev-parse", "HEAD^1"]),
                head,
                "{context}"
            );
            assert_eq!(
                std::fs::read_to_string(fixture.target.join("agent-only.txt")).unwrap(),
                "cleanly merged addition\n",
                "{context}"
            );
            if dirty {
                assert_eq!(
                    std::fs::read(fixture.target.join("file.txt")).unwrap(),
                    b"unstaged user edit\n"
                );
                assert_eq!(
                    git(&fixture.target, &["show", ":file.txt"]),
                    "staged user edit\n"
                );
                assert_eq!(
                    std::fs::read(fixture.target.join("untracked.txt")).unwrap(),
                    b"user notes\n"
                );
            } else {
                assert!(git(&fixture.target, &["status", "--porcelain"]).is_empty());
            }
            assert!(git(&fixture.target, &["stash", "list"]).trim().is_empty());
            for name in ["MERGE_HEAD", "SQUASH_MSG"] {
                assert!(!git_state_path(&fixture.target, name).exists(), "{name}");
            }
            let markers = merge_markers(&fixture);
            assert_eq!(markers.len(), 1, "{context}: {markers:?}");
            let landed = git(&fixture.target, &["rev-parse", "HEAD"]);
            assert!(
                markers[0].ends_with(&format!(" at {}.", landed.trim())),
                "{markers:?}"
            );
        }
    }
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn lease_release_failure_after_applied_merge_is_a_warning() {
    use super::tool_task_merge_agent::legacy_fault::{install, Action, Point};

    let fixture = Fixture::new(true).await;
    resolve_agent_conflicts(&fixture);
    let head = git(&fixture.target, &["rev-parse", "HEAD"]);
    install(&fixture.target, Point::LeaseRelease, Action::Fail);

    let result = fixture
        .merge("squash")
        .await
        .expect("an applied merge is never reported as an error");

    let text = tool_text(&result);
    assert!(text.contains("Agent Work Merged"), "{text}");
    assert!(text.contains("releasing the merge lease failed"), "{text}");
    assert_ne!(git(&fixture.target, &["rev-parse", "HEAD"]), head);
    assert_eq!(merge_markers(&fixture).len(), 1);
}

#[cfg(unix)]
fn install_colliding_hook(fixture: &Fixture, file: &str) {
    use std::os::unix::fs::PermissionsExt;

    let hooks = fixture._temp.path().join("colliding-hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    for name in ["post-commit", "post-merge"] {
        let path = hooks.join(name);
        std::fs::write(&path, format!("#!/bin/sh\necho hook > {file}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    git(
        &fixture.target,
        &["config", "core.hooksPath", hooks.to_str().unwrap()],
    );
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn reapply_failure_with_failed_restore_says_the_edits_are_in_the_stash() {
    for strategy in ["squash", "merge"] {
        let fixture = Fixture::new(true).await;
        resolve_agent_conflicts(&fixture);
        std::fs::write(fixture.target.join("untracked.txt"), "user notes\n").unwrap();
        let head = git(&fixture.target, &["rev-parse", "HEAD"]);
        install_colliding_hook(&fixture, "untracked.txt");

        let result = fixture
            .merge(strategy)
            .await
            .unwrap_or_else(|error| panic!("{strategy}: {error}"));

        let text = tool_text(&result);
        assert!(!text.contains("Agent Work Merged"), "{strategy}: {text}");
        assert!(
            text.contains("Nothing was merged: the merge was rolled back, but restoring your uncommitted edits failed; they are saved in git stash"),
            "{strategy}: {text}"
        );
        assert!(!text.contains("restored in place"), "{strategy}: {text}");
        assert!(
            text.contains("Conflict cleanup failed"),
            "{strategy}: {text}"
        );
        assert_eq!(git(&fixture.target, &["rev-parse", "HEAD"]), head);
        let stash = git(&fixture.target, &["stash", "list", "--format=%H"]);
        assert!(!stash.trim().is_empty(), "{strategy}");
        assert!(
            text.contains(&format!("git stash apply --index {}", stash.trim())),
            "{strategy}: {text}"
        );
        assert!(merge_markers(&fixture).is_empty(), "{strategy}");
        assert!(fixture.agent.exists(), "{strategy}");
    }
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn failed_rollback_keeps_the_landed_commit_records_one_marker_and_names_the_stash() {
    for strategy in ["squash", "merge"] {
        let fixture = Fixture::new(true).await;
        resolve_agent_conflicts(&fixture);
        make_user_edits(&fixture.target);
        let head = git(&fixture.target, &["rev-parse", "HEAD"]);
        install_colliding_hook(&fixture, "untracked.txt");

        let result = fixture
            .merge(strategy)
            .await
            .unwrap_or_else(|error| panic!("{strategy}: {error}"));

        let text = tool_text(&result);
        assert!(text.contains("Agent Work Merged"), "{strategy}: {text}");
        assert!(text.contains("landed and was kept"), "{strategy}: {text}");
        let landed = git(&fixture.target, &["rev-parse", "HEAD"]);
        assert_eq!(
            git(&fixture.target, &["rev-parse", "HEAD^1"]),
            head,
            "{strategy}"
        );
        let stash = git(&fixture.target, &["stash", "list", "--format=%H"]);
        assert!(!stash.trim().is_empty(), "{strategy}");
        assert!(
            text.contains(&format!("git stash apply --index {}", stash.trim())),
            "{strategy}: {text}"
        );
        let markers = merge_markers(&fixture);
        assert_eq!(markers.len(), 1, "{strategy}: {markers:?}");
        assert!(
            markers[0].ends_with(&format!(" at {}.", landed.trim())),
            "{markers:?}"
        );
        assert!(fixture.agent.exists(), "{strategy}");
    }
}

#[cfg(unix)]
fn install_hook(fixture: &Fixture, name: &str, body: &str) {
    use std::os::unix::fs::PermissionsExt;

    let hooks = fixture._temp.path().join("named-hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let path = hooks.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    git(
        &fixture.target,
        &["config", "core.hooksPath", hooks.to_str().unwrap()],
    );
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn hook_commit_on_top_is_never_reset_away_and_the_marker_names_this_merge() {
    for (hook, strategy) in [("post-commit", "squash"), ("post-merge", "merge")] {
        let fixture = Fixture::new(true).await;
        resolve_agent_conflicts(&fixture);
        make_user_edits(&fixture.target);
        let head = git(&fixture.target, &["rev-parse", "HEAD"]);
        install_hook(
            &fixture,
            hook,
            "git -c core.hooksPath=/dev/null commit -q --no-verify --allow-empty -m hook-commit\necho hook > untracked.txt",
        );

        let result = fixture
            .merge(strategy)
            .await
            .unwrap_or_else(|error| panic!("{strategy}: {error}"));

        let text = tool_text(&result);
        let landed = git(&fixture.target, &["rev-parse", "HEAD"]);
        assert_eq!(
            git(&fixture.target, &["log", "-1", "--format=%s", "HEAD"]).trim(),
            "hook-commit",
            "{strategy}: {text}"
        );
        let own = git(&fixture.target, &["rev-parse", "HEAD^1"]);
        assert_eq!(
            git(&fixture.target, &["rev-parse", "HEAD^1^1"]),
            head,
            "{strategy}"
        );
        assert!(text.contains("Agent Work Merged"), "{strategy}: {text}");
        assert!(
            text.contains(&format!("moved to {}", landed.trim())),
            "{strategy}: {text}"
        );
        assert!(
            !text.contains("merge authority was lost"),
            "{strategy}: {text}"
        );
        let stash = git(&fixture.target, &["stash", "list", "--format=%H"]);
        assert!(
            text.contains(&format!("git stash apply --index {}", stash.trim())),
            "{strategy}: {text}"
        );
        let markers = merge_markers(&fixture);
        assert_eq!(markers.len(), 1, "{strategy}: {markers:?}");
        assert!(
            markers[0].ends_with(&format!(" at {}.", own.trim())),
            "{strategy}: {markers:?}"
        );
        assert!(fixture.agent.exists(), "{strategy}");
    }
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn safety_legacy_unknown_success_preserves_source() {
    let fixture = Fixture::new(true).await;
    resolve_agent_conflicts(&fixture);
    make_user_edits(&fixture.target);
    install_hook(&fixture, "post-commit", "oid=$(git commit-tree HEAD^1^{tree} -p HEAD^1 -m replacement) || exit 2\ngit update-ref HEAD \"$oid\"");
    let result = fixture.merge("squash").await;
    assert!(result.is_err(), "unexpected successful merge");
    assert_eq!(
        git(&fixture.target, &["log", "-1", "--format=%s"]).trim(),
        "replacement"
    );
    assert!(merge_markers(&fixture).is_empty());
    assert!(fixture.agent.exists());
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn safety_legacy_failed_hook_helper_is_not_a_squash_landing() {
    let fixture = Fixture::new(true).await;
    resolve_agent_conflicts(&fixture);
    make_user_edits(&fixture.target);
    install_hook(&fixture, "pre-commit", "oid=$(git commit-tree HEAD^{tree} -p HEAD -m helper) || exit 2\ngit update-ref HEAD \"$oid\" || exit 2\nexit 1");
    let result = fixture.merge("squash").await;
    assert!(result.is_err(), "unexpected successful merge");
    assert_eq!(
        git(&fixture.target, &["log", "-1", "--format=%s"]).trim(),
        "helper"
    );
    assert!(merge_markers(&fixture).is_empty());
    assert!(fixture.agent.exists());
    assert!(!git(&fixture.target, &["stash", "list"]).trim().is_empty());
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn squash_restaged_by_pre_commit_hook_is_merged_after_commit_window_error() {
    use super::tool_task_merge_agent::legacy_fault::{install, Action, Point};

    let fixture = Fixture::new(true).await;
    resolve_agent_conflicts(&fixture);
    make_user_edits(&fixture.target);
    let head = git(&fixture.target, &["rev-parse", "HEAD"]);
    install_hook(
        &fixture,
        "pre-commit",
        "printf 'formatted\\n' > agent-only.txt\ngit add agent-only.txt",
    );
    install(&fixture.target, Point::CommitWindow, Action::Fail);

    let result = fixture
        .merge("squash")
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let text = tool_text(&result);
    assert!(text.contains("Agent Work Merged"), "{text}");
    assert!(text.contains("commit already landed"), "{text}");
    let landed = git(&fixture.target, &["rev-parse", "HEAD"]);
    assert_eq!(git(&fixture.target, &["rev-parse", "HEAD^1"]), head);
    assert_eq!(
        git(&fixture.target, &["show", "HEAD:agent-only.txt"]),
        "formatted\n"
    );
    assert_eq!(
        git(&fixture.target, &["show", ":file.txt"]),
        "staged user edit\n"
    );
    assert_eq!(
        std::fs::read(fixture.target.join("untracked.txt")).unwrap(),
        b"user notes\n"
    );
    assert!(git(&fixture.target, &["stash", "list"]).trim().is_empty());
    let markers = merge_markers(&fixture);
    assert_eq!(markers.len(), 1, "{markers:?}");
    assert!(
        markers[0].ends_with(&format!(" at {}.", landed.trim())),
        "{markers:?}"
    );
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn squash_write_tree_failure_never_commits_and_restores_user_edits() {
    use super::tool_task_merge_agent::legacy_fault::{install, Action, Point};

    let fixture = Fixture::new(true).await;
    resolve_agent_conflicts(&fixture);
    let before = make_user_edits(&fixture.target);
    install(&fixture.target, Point::WriteTree, Action::Fail);

    let error = fixture.merge("squash").await.unwrap_err();

    assert!(
        error.contains("Failed to capture the squash tree"),
        "{error}"
    );
    assert!(error.contains("rolled back"), "{error}");
    assert_user_edits_restored(&fixture.target, &before, "write-tree");
    assert!(merge_markers(&fixture).is_empty());
    assert!(fixture.agent.exists());
}
