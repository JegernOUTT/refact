use std::time::Duration;

use crate::chat::task_agent_monitor::capture_failed_agent_diff;
use crate::tasks::{admission, storage};
use crate::tools::restart_fresh_diff_tests::{git, Fixture};
use crate::worktrees::service::WorktreeService;

#[tokio::test]
async fn capture_without_base_commit_diffs_from_merge_base() {
    let fixture = Fixture::new().await;
    let root = fixture.old.worktree_path();
    let meta = fixture.old.worktree_meta();
    std::fs::write(fixture.source.join("base-only.txt"), "base branch edit\n").unwrap();
    git(&fixture.source, &["add", "base-only.txt"]);
    git(&fixture.source, &["commit", "-m", "advance base branch"]);
    std::fs::write(root.join("agent-only.txt"), "agent branch edit\n").unwrap();
    git(&root, &["add", "agent-only.txt"]);
    git(&root, &["commit", "-m", "agent edit"]);

    let report =
        capture_failed_agent_diff(root.to_str().unwrap(), None, meta.base_branch.as_deref())
            .await
            .unwrap();
    let base_tip = git(&fixture.source, &["rev-parse", "HEAD"]);
    let recorded_base_report = capture_failed_agent_diff(
        root.to_str().unwrap(),
        Some(base_tip.trim()),
        meta.base_branch.as_deref(),
    )
    .await
    .unwrap();
    fixture.old.cleanup_unlinked(fixture.gcx).await;

    assert!(recorded_base_report.contains("-base branch edit"));
    assert!(report.contains("+agent branch edit"), "{report}");
    assert!(
        !report.contains("base-only.txt") && !report.contains("-base branch edit"),
        "base branch advance must not appear as reversed agent changes: {report}"
    );
}

#[tokio::test]
async fn late_recapture_refusal_is_recorded_on_the_card() {
    let fixture = Fixture::new().await;
    let original = storage::load_board(fixture.gcx.clone(), "task-1")
        .await
        .unwrap()
        .get_card("T-1")
        .unwrap()
        .clone();
    let holder = fixture
        .app
        .work_scheduler
        .reserve(refact_work_core::AdmissionRequest::new(
            refact_work_core::WorkKey::background_agent("holder").run(1),
            admission::board_scope(
                &crate::coordination::roots::project_identity(&fixture.source),
                "planner",
            ),
        ))
        .unwrap()
        .acquire()
        .await
        .unwrap();
    let chat = fixture.restart("fresh", false).await.unwrap();
    let queued = storage::load_board(fixture.gcx.clone(), "task-1")
        .await
        .unwrap();
    assert_eq!(queued.get_card("T-1").unwrap().column, "doing");
    assert_eq!(
        queued.get_card("T-1").unwrap().agent_chat_id.as_deref(),
        Some(chat.as_str())
    );
    WorktreeService::new_async(fixture.gcx.cache_dir.clone(), fixture.source.clone())
        .await
        .unwrap()
        .delete_worktree(&fixture.old.worktree_name(), true, true)
        .await
        .unwrap();
    let worktrees = git(&fixture.source, &["worktree", "list", "--porcelain"]);
    let branches = git(&fixture.source, &["branch", "--list"]);
    holder.release();
    tokio::time::timeout(Duration::from_secs(30), async {
        while admission::is_tracked(&chat).await {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("refused restart must release admission");
    admission::cancel_setup_and_wait(&chat, Duration::from_secs(30))
        .await
        .unwrap();

    let board = storage::load_board(fixture.gcx.clone(), "task-1")
        .await
        .unwrap();
    let card = board.get_card("T-1").unwrap();
    assert_eq!(card.column, original.column);
    assert_eq!(card.agent_chat_id, original.agent_chat_id);
    assert_eq!(card.assignee, original.assignee);
    assert_eq!(card.agent_worktree, original.agent_worktree);
    assert_eq!(card.agent_worktree_name, original.agent_worktree_name);
    assert_eq!(card.agent_branch, original.agent_branch);
    assert_eq!(card.final_report, original.final_report);
    assert_eq!(
        git(&fixture.source, &["worktree", "list", "--porcelain"]),
        worktrees
    );
    assert_eq!(git(&fixture.source, &["branch", "--list"]), branches);
    assert!(fixture.facade.created.lock().unwrap().is_empty());
    assert!(fixture.facade.started.lock().unwrap().is_empty());
    assert!(fixture.app.agent_executions.get(&chat).is_none());
    assert_eq!(fixture.app.work_scheduler.snapshot().outstanding, 0);
    assert!(
        card.status_updates.iter().any(|update| {
            update
                .message
                .contains("Fresh restart refused after it was queued")
                && update.message.contains("Cannot open worktree")
                && update
                    .message
                    .contains("retry with force=true to restart without the capture")
        }),
        "late recapture refusal disappeared from the restored card: {:?}",
        card.status_updates
    );
}
