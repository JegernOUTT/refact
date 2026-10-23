use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ContextEnum;
use crate::chat::task_agent_monitor::capture_failed_agent_diff;
use crate::global_context::GlobalContext;
use crate::tasks::{admission, storage};
use crate::tools::tool_spawn_ab::tests::{await_started, setup_repo_task, RecordingFacade};
use crate::tools::tool_task_restart_agent::ToolTaskRestartAgent;
use crate::tools::tool_task_spawn_agent::{prepare_agent_worktree, PreparedWorktree};
use crate::tools::tools_description::Tool;

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

struct Fixture {
    _temp: tempfile::TempDir,
    gcx: Arc<GlobalContext>,
    source: PathBuf,
    app: AppState,
    ccx: Arc<Mutex<AtCommandsContext>>,
    facade: Arc<RecordingFacade>,
    old: PreparedWorktree,
}

impl Fixture {
    async fn new() -> Self {
        let (gcx, temp, source) = setup_repo_task().await;
        let meta = storage::load_task_meta(gcx.clone(), "task-1")
            .await
            .unwrap();
        let old =
            prepare_agent_worktree(gcx.clone(), &meta, "task-1", "old-agent", "T-1", "old-chat")
                .await
                .unwrap();
        let worktree = old.worktree_meta();
        storage::update_board_atomic(gcx.clone(), "task-1", move |board| {
            let card = board.get_card_mut("T-1").unwrap();
            card.column = "failed".into();
            card.assignee = Some("old-agent".into());
            card.agent_chat_id = Some("old-chat".into());
            card.agent_worktree = Some(worktree.root.to_string_lossy().into_owned());
            card.agent_worktree_name = Some(worktree.id);
            card.agent_branch = worktree.branch;
            card.base_commit = worktree.base_commit;
            card.base_branch = worktree.base_branch;
            card.final_report = Some("Automatic failure before the final edits".into());
            Ok(())
        })
        .await
        .unwrap();
        let mut caps = crate::caps::CodeAssistantCaps::default();
        caps.chat_models.insert(
            "test/model".into(),
            Arc::new(crate::caps::ChatModelRecord::default()),
        );
        {
            let mut state = gcx.caps_state.write().await;
            state.caps = Some(Arc::new(caps));
            state.last_attempted_ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
        }
        let mut app = AppState::from_gcx(gcx.clone()).await;
        app.work_scheduler = Arc::new(refact_work_core::AdmissionScheduler::new(
            refact_work_core::CoordinationSettings {
                max_active_engine: 1,
                max_active_project: 1,
                max_active_root: 1,
                ..Default::default()
            },
        ));
        let facade = Arc::new(RecordingFacade::default());
        app.chat.facade = facade.clone();
        let ccx = Arc::new(Mutex::new(
            AtCommandsContext::new_from_app(
                app.clone(),
                4096,
                20,
                false,
                vec![],
                "planner".into(),
                Some("planner".into()),
                "test/model".into(),
                Some(refact_chat_api::TaskMeta {
                    task_id: "task-1".into(),
                    role: "planner".into(),
                    planner_chat_id: Some("planner".into()),
                    agent_id: None,
                    card_id: None,
                }),
                None,
            )
            .await,
        ));
        Self {
            _temp: temp,
            gcx,
            source,
            app,
            ccx,
            facade,
            old,
        }
    }

    async fn restart(&self, mode: &str, force: bool) -> Result<String, String> {
        let (_, output) = ToolTaskRestartAgent::new()
            .tool_execute(
                self.ccx.clone(),
                &"restart-call".into(),
                &HashMap::from([
                    ("card_id".into(), json!("T-1")),
                    ("mode".into(), json!(mode)),
                    ("force".into(), json!(force)),
                ]),
            )
            .await?;
        let ContextEnum::ChatMessage(message) = &output[0] else {
            panic!("tool result")
        };
        let response: Value = serde_json::from_str(&message.content.content_text_only()).unwrap();
        Ok(response["agent_chat_id"].as_str().unwrap().into())
    }

    async fn finish_setup(&self, chat: &str) {
        await_started(&self.facade, 1).await;
        admission::cancel_setup_and_wait(chat, Duration::from_secs(30))
            .await
            .unwrap();
        admission::end(chat).await;
    }

    async fn cleanup_current(&self) {
        let board = storage::load_board(self.gcx.clone(), "task-1")
            .await
            .unwrap();
        let name = board
            .get_card("T-1")
            .unwrap()
            .agent_worktree_name
            .as_deref()
            .unwrap();
        crate::worktrees::service::WorktreeService::new_async(
            self.gcx.cache_dir.clone(),
            self.source.clone(),
        )
        .await
        .unwrap()
        .delete_worktree(name, true, true)
        .await
        .unwrap();
        assert_eq!(self.app.work_scheduler.snapshot().outstanding, 0);
    }
}

#[tokio::test]
async fn fresh_restart_captures_committed_and_uncommitted_diff_before_delete() {
    let fixture = Fixture::new().await;
    let root = fixture.old.worktree_path();
    let old_branch = fixture.old.branch_name().unwrap();
    std::fs::write(root.join("first.txt"), "first committed edit\n").unwrap();
    git(&root, &["add", "first.txt"]);
    git(&root, &["commit", "-m", "first agent edit"]);
    std::fs::write(root.join("second.txt"), "second committed edit\n").unwrap();
    git(&root, &["add", "second.txt"]);
    git(&root, &["commit", "-m", "second agent edit"]);
    let before = fixture.old.worktree_meta();
    let monitor_capture = capture_failed_agent_diff(
        root.to_str().unwrap(),
        before.base_commit.as_deref(),
        before.base_branch.as_deref(),
    )
    .await
    .unwrap();
    assert!(!monitor_capture.contains("late untracked edit"));
    let holder = fixture
        .app
        .work_scheduler
        .reserve(refact_work_core::AdmissionRequest::new(
            refact_work_core::WorkKey::background_agent("holder").run(1),
            crate::tasks::admission::board_scope(
                &crate::coordination::roots::project_identity(&fixture.source),
                "planner",
            ),
        ))
        .unwrap()
        .acquire()
        .await
        .unwrap();
    let chat = fixture.restart("fresh", false).await.unwrap();
    assert!(root.exists());
    std::fs::write(root.join("file.txt"), "hello\nlate unstaged edit\n").unwrap();
    std::fs::write(root.join("staged.txt"), "late staged edit\n").unwrap();
    git(&root, &["add", "staged.txt"]);
    std::fs::create_dir(root.join("new-dir")).unwrap();
    std::fs::write(root.join("new-dir/untracked.txt"), "late untracked edit\n").unwrap();
    std::fs::write(fixture.source.join("main-only.txt"), "not agent work\n").unwrap();
    git(&fixture.source, &["add", "main-only.txt"]);
    git(&fixture.source, &["commit", "-m", "advance base branch"]);
    holder.release();
    fixture.finish_setup(&chat).await;
    assert!(
        !root.exists(),
        "fresh restart must actually delete the old checkout"
    );
    assert!(git(&fixture.source, &["branch", "--list", &old_branch])
        .trim()
        .is_empty());
    let board = storage::load_board(fixture.gcx.clone(), "task-1")
        .await
        .unwrap();
    let card = board.get_card("T-1").unwrap();
    let report = card
        .status_updates
        .iter()
        .find(|update| {
            update
                .message
                .starts_with("Fresh restart: previous worktree diff captured.")
        })
        .expect("fresh restart must durably capture the previous worktree diff before deletion");
    for expected in [
        "+first committed edit",
        "+second committed edit",
        "+late unstaged edit",
        "+late staged edit",
        "new-dir/untracked.txt",
        "+late untracked edit",
    ] {
        assert!(
            report.message.contains(expected),
            "missing {expected}: {}",
            report.message
        );
    }
    assert!(!report.message.contains("main-only.txt"));
    assert!(Path::new(card.agent_worktree.as_deref().unwrap()).exists());
    fixture.cleanup_current().await;
}

#[tokio::test]
async fn fresh_restart_refuses_when_capture_fails_without_force() {
    let fixture = Fixture::new().await;
    let root = fixture.old.worktree_path();
    std::fs::write(root.join("retained.txt"), "keep this edit\n").unwrap();
    storage::update_board_atomic(fixture.gcx.clone(), "task-1", |board| {
        board.get_card_mut("T-1").unwrap().base_commit = Some("missing-card-base".into());
        Ok(())
    })
    .await
    .unwrap();
    let board_path = fixture.source.join(".refact/tasks/task-1/board.yaml");
    let board_before = std::fs::read(&board_path).unwrap();
    let worktrees_before = git(&fixture.source, &["worktree", "list", "--porcelain"]);
    let error = fixture.restart("fresh", false).await.unwrap_err();
    assert!(error.contains("Fresh restart refused"), "{error}");
    assert!(
        error.contains("Cannot resolve card base 'missing-card-base'"),
        "{error}"
    );
    assert_eq!(std::fs::read(&board_path).unwrap(), board_before);
    assert_eq!(
        git(&fixture.source, &["worktree", "list", "--porcelain"]),
        worktrees_before
    );
    assert_eq!(
        std::fs::read_to_string(root.join("retained.txt")).unwrap(),
        "keep this edit\n"
    );
    assert!(fixture.facade.created.lock().unwrap().is_empty());
    assert_eq!(fixture.app.work_scheduler.snapshot().outstanding, 0);
    fixture.old.cleanup_unlinked(fixture.gcx).await;
}

#[tokio::test]
async fn forced_fresh_restart_records_that_no_diff_was_captured() {
    let fixture = Fixture::new().await;
    let root = fixture.old.worktree_path();
    storage::update_board_atomic(fixture.gcx.clone(), "task-1", |board| {
        let card = board.get_card_mut("T-1").unwrap();
        card.base_commit = None;
        card.base_branch = None;
        Ok(())
    })
    .await
    .unwrap();
    let chat = fixture.restart("fresh", true).await.unwrap();
    fixture.finish_setup(&chat).await;
    assert!(!root.exists());
    let board = storage::load_board(fixture.gcx.clone(), "task-1")
        .await
        .unwrap();
    assert!(board
        .get_card("T-1")
        .unwrap()
        .status_updates
        .iter()
        .any(|update| {
            update
                .message
                .contains("Fresh restart forced; no diff captured:")
                && update.message.contains("no recorded base")
        }));
    fixture.cleanup_current().await;
}

#[tokio::test]
async fn resume_restart_does_not_require_diff_capture() {
    let fixture = Fixture::new().await;
    let root = fixture.old.worktree_path();
    storage::update_board_atomic(fixture.gcx.clone(), "task-1", |board| {
        board.get_card_mut("T-1").unwrap().base_commit = Some("missing-card-base".into());
        Ok(())
    })
    .await
    .unwrap();
    let chat = fixture.restart("resume", false).await.unwrap();
    fixture.finish_setup(&chat).await;
    assert!(root.exists());
    let board = storage::load_board(fixture.gcx.clone(), "task-1")
        .await
        .unwrap();
    assert!(board
        .get_card("T-1")
        .unwrap()
        .status_updates
        .iter()
        .all(|update| !update.message.contains("diff captured")));
    fixture.cleanup_current().await;
}

#[tokio::test]
async fn shared_diff_capture_bounds_each_section() {
    let fixture = Fixture::new().await;
    let root = fixture.old.worktree_path();
    std::fs::write(root.join("committed.txt"), "c".repeat(5000)).unwrap();
    git(&root, &["add", "committed.txt"]);
    git(&root, &["commit", "-m", "large edit"]);
    std::fs::write(root.join("untracked.txt"), "u".repeat(5000)).unwrap();
    let meta = fixture.old.worktree_meta();
    let report = capture_failed_agent_diff(
        root.to_str().unwrap(),
        meta.base_commit.as_deref(),
        meta.base_branch.as_deref(),
    )
    .await
    .unwrap();
    assert_eq!(report.matches("... (truncated)").count(), 2);
    assert!(report.chars().count() < 4300);
    fixture.old.cleanup_unlinked(fixture.gcx).await;
}
