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
async fn dirty_target_is_refused_before_any_merge_attempt() {
    for strategy in ["squash", "merge"] {
        for linked_target in [false, true] {
            let fixture = Fixture::new(linked_target).await;
            std::fs::write(fixture.target.join("file.txt"), "staged user edit\n").unwrap();
            git(&fixture.target, &["add", "file.txt"]);
            std::fs::write(fixture.target.join("file.txt"), "unstaged user edit\n").unwrap();
            std::fs::write(fixture.target.join("untracked.txt"), "user notes\n").unwrap();
            std::fs::write(fixture.agent.join("pending.txt"), "not auto-committed\n").unwrap();
            let roots = [&fixture.source, &fixture.target, &fixture.agent];
            let before = roots.map(|root| snapshot(root));
            let board_path = fixture.source.join(".refact/tasks/task-1/board.yaml");
            let board = std::fs::read(&board_path).unwrap();
            let orig_head = std::fs::read(git_state_path(&fixture.target, "ORIG_HEAD")).ok();

            let error = fixture.merge(strategy).await.unwrap_err();

            assert!(error.contains("uncommitted changes"), "{error}");
            assert_eq!(before, roots.map(|root| snapshot(root)));
            assert_eq!(std::fs::read(board_path).unwrap(), board);
            assert_eq!(
                std::fs::read(git_state_path(&fixture.target, "ORIG_HEAD")).ok(),
                orig_head
            );
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
                std::fs::read_to_string(fixture.agent.join("pending.txt")).unwrap(),
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
