use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::chat::post_merge_check::{
    post_merge_check_with_runner, PostMergeCheckRequest, PostMergeCheckResult, PostMergeCommand,
    PostMergeCommandOutput, PostMergeCommandRunner,
};
use crate::coordination::leases::leases_for;
use crate::coordination::merge_gate::{checkout_key, merge_lease_key};
use crate::tasks::types::{BoardCard, TaskBoard, TaskMeta, TaskStatus, VerificationOutcome};
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::{CreateWorktreeRequest, MergeWorktreeRequest, WorktreeMergeStrategy};
use async_trait::async_trait;
use crate::global_context::GlobalContext;

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn init_repo(root: &Path) {
    std::fs::create_dir_all(root).unwrap();
    git(root, &["init"]);
    git(root, &["checkout", "-b", "main"]);
    git(root, &["config", "user.email", "test@example.com"]);
    git(root, &["config", "user.name", "Test User"]);
    std::fs::write(root.join(".gitignore"), ".refact/\n").unwrap();
    std::fs::write(root.join("file.txt"), "base\n").unwrap();
    git(root, &["add", ".gitignore", "file.txt"]);
    git(root, &["commit", "-m", "initial"]);
}

fn card(worktree_id: &str, branch: &str, root: &Path) -> BoardCard {
    BoardCard {
        id: "T-1".to_string(),
        title: "Card T-1".to_string(),
        column: "done".to_string(),
        priority: "P1".to_string(),
        depends_on: Vec::new(),
        instructions: "## Acceptance Criteria\n- Verify: `cargo test --lib`".to_string(),
        assignee: Some("agent-1".to_string()),
        agent_chat_id: Some("agent-chat-1".to_string()),
        status_updates: Vec::new(),
        comments: Vec::new(),
        final_report: Some("done".to_string()),
        final_report_structured: None,
        verifier_report: None,
        created_at: chrono::Utc::now().to_rfc3339(),
        started_at: None,
        last_heartbeat_at: None,
        paused_at: None,
        paused_by: None,
        completed_at: Some(chrono::Utc::now().to_rfc3339()),
        agent_branch: Some(branch.to_string()),
        agent_worktree: Some(root.to_string_lossy().to_string()),
        agent_worktree_name: Some(worktree_id.to_string()),
        base_branch: Some("main".to_string()),
        base_commit: None,
        ab_variants: None,
        team_members: Vec::new(),
        target_files: Vec::new(),
        scope_guard_mode: Default::default(),
        attention: None,
        deliverable: None,
        verification: Vec::new(),
    }
}

async fn task_board(
    gcx: Arc<GlobalContext>,
    root: &Path,
    worktree_id: &str,
    branch: &str,
    worktree_root: &Path,
) {
    let task_dir = root.join(".refact/tasks/task-1");
    tokio::fs::create_dir_all(&task_dir).await.unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    let meta = TaskMeta {
        schema_version: 1,
        id: "task-1".to_string(),
        name: "Task".to_string(),
        status: TaskStatus::Active,
        created_at: now.clone(),
        updated_at: now,
        cards_total: 1,
        cards_done: 1,
        cards_failed: 0,
        agents_active: 0,
        base_branch: Some("main".to_string()),
        base_commit: None,
        default_agent_model: None,
        is_name_generated: false,
        last_agents_summary_at: None,
        planner_session_state: None,
    };
    let mut board = TaskBoard::default();
    board.cards.push(card(worktree_id, branch, worktree_root));
    tokio::fs::write(
        task_dir.join("meta.yaml"),
        serde_yaml::to_string(&meta).unwrap(),
    )
    .await
    .unwrap();
    tokio::fs::write(
        task_dir.join("board.yaml"),
        serde_yaml::to_string(&board).unwrap(),
    )
    .await
    .unwrap();
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
}

async fn fixture() -> (
    tempfile::TempDir,
    Arc<GlobalContext>,
    WorktreeService,
    String,
    PathBuf,
) {
    crate::runtime_settings::reset_for_test();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    init_repo(&root);
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        temp.path().join("cache"),
        temp.path().join("config"),
    )
    .await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
    let service = WorktreeService::new_async(gcx.cache_dir.clone(), root.clone())
        .await
        .unwrap();
    let created = service
        .create_worktree(CreateWorktreeRequest {
            branch: Some("refact/task/task-1/card/T-1/w10".to_string()),
            kind: Some("task_agent".to_string()),
            task_id: Some("task-1".to_string()),
            card_id: Some("T-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            chat_id: Some("agent-chat-1".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();
    let branch = created.worktree.meta.branch.clone().unwrap();
    let worktree_root = created.worktree.meta.root.clone();
    std::fs::write(worktree_root.join("file.txt"), "merged\n").unwrap();
    git(&worktree_root, &["add", "file.txt"]);
    git(&worktree_root, &["commit", "-m", "agent change"]);
    task_board(
        gcx.clone(),
        &root,
        &created.worktree.meta.id,
        &branch,
        &worktree_root,
    )
    .await;
    (temp, gcx, service, created.worktree.meta.id, worktree_root)
}

fn merge_request() -> MergeWorktreeRequest {
    MergeWorktreeRequest {
        strategy: WorktreeMergeStrategy::Squash,
        delete_after_merge: true,
        include_uncommitted: true,
        target_branch: Some("main".to_string()),
        commit_message: Some("merge agent work".to_string()),
        ..Default::default()
    }
}

async fn post_merge_request(gcx: &Arc<GlobalContext>, root: &Path) -> PostMergeCheckRequest {
    PostMergeCheckRequest {
        task_id: "task-1".to_string(),
        card_id: "T-1".to_string(),
        workspace_root: root.to_path_buf(),
        enabled: true,
        timeout: Duration::from_secs(5),
        expected_merge_commit: String::new(),
        expected_card_state: Some(
            crate::chat::verifier::load_expected_card_state(gcx.clone(), "task-1", "T-1")
                .await
                .unwrap(),
        ),
    }
}

#[derive(Default)]
struct RevertingRunner {
    calls: Vec<PostMergeCommand>,
}

#[async_trait]
impl PostMergeCommandRunner for RevertingRunner {
    async fn run(
        &mut self,
        workspace_root: &Path,
        command: PostMergeCommand,
        _timeout: Duration,
    ) -> PostMergeCommandOutput {
        self.calls.push(command.clone());
        match command {
            PostMergeCommand::Verify { .. } => PostMergeCommandOutput {
                exit_code: Some(1),
                success: false,
                output: "error: regression".to_string(),
                outcome: VerificationOutcome::CommandFailed,
            },
            PostMergeCommand::Git(args) => {
                let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
                let output = Command::new("git")
                    .args(&refs)
                    .current_dir(workspace_root)
                    .output()
                    .unwrap();
                PostMergeCommandOutput {
                    exit_code: output.status.code(),
                    success: output.status.success(),
                    output: format!(
                        "{}{}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    ),
                    outcome: if output.status.success() {
                        VerificationOutcome::Passed
                    } else {
                        VerificationOutcome::CommandFailed
                    },
                }
            }
        }
    }
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn opted_in_failure_reverts_inside_the_live_lease_and_deletes_source() {
    let (_temp, gcx, service, worktree_id, worktree_root) = fixture().await;
    let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    let leases = leases_for(&app)
        .await
        .expect("coordination lease store is live");
    let source_root = service.source_workspace_root().to_path_buf();
    let request = post_merge_request(&gcx, &source_root).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let callback_calls = calls.clone();
    let callback_leases = leases.clone();
    let callback_root = source_root.clone();
    let outcome = super::merge_under_available_lease(
        &app,
        &service,
        &worktree_id,
        merge_request(),
        Some(request),
        move |gcx, request| {
            let callback_calls = callback_calls.clone();
            let leases = callback_leases.clone();
            let source_root = callback_root.clone();
            async move {
                callback_calls.fetch_add(1, Ordering::SeqCst);
                let held = leases
                    .inspect(
                        &merge_lease_key(&checkout_key(&source_root)),
                        refact_work_core::store::records::now_ms(),
                    )
                    .await
                    .unwrap();
                assert!(held.is_some(), "the verifier must run while the merge lease is held");
                let mut runner = RevertingRunner::default();
                let result = post_merge_check_with_runner(gcx, request, &mut runner).await;
                assert!(runner.calls.iter().any(|command| matches!(
                    command,
                    PostMergeCommand::Git(args) if args.first().map(String::as_str) == Some("revert")
                )));
                result
            }
        },
    )
    .await
    .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(outcome.0.merged);
    assert_eq!(
        std::fs::read_to_string(source_root.join("file.txt")).unwrap(),
        "base\n"
    );
    assert!(!worktree_root.exists());
    let verification = outcome.1.unwrap();
    assert_eq!(verification.outcome, VerificationOutcome::CommandFailed);
    assert!(verification.auto_reverted);
    assert!(verification.revert_commit.is_some());
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn opted_in_pass_runs_once_inside_lease_and_deletes_source() {
    let (_temp, gcx, service, worktree_id, worktree_root) = fixture().await;
    let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    let leases = leases_for(&app)
        .await
        .expect("coordination lease store is live");
    let source_root = service.source_workspace_root().to_path_buf();
    let request = post_merge_request(&gcx, &source_root).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let callback_calls = calls.clone();
    let callback_leases = leases.clone();
    let callback_root = source_root.clone();
    let outcome = super::merge_under_available_lease(
        &app,
        &service,
        &worktree_id,
        merge_request(),
        Some(request),
        move |_gcx, _request| {
            let callback_calls = callback_calls.clone();
            let leases = callback_leases.clone();
            let source_root = callback_root.clone();
            async move {
                callback_calls.fetch_add(1, Ordering::SeqCst);
                assert!(leases
                    .inspect(
                        &merge_lease_key(&checkout_key(&source_root)),
                        refact_work_core::store::records::now_ms(),
                    )
                    .await
                    .unwrap()
                    .is_some());
                Ok(PostMergeCheckResult {
                    checked: true,
                    auto_reverted: false,
                    command: Some("cargo test --lib".to_string()),
                    exit_code: Some(0),
                    output_tail: "ok".to_string(),
                    merge_commit: None,
                    revert_commit: None,
                    fix_card_id: None,
                    skipped_reason: None,
                    revert_skipped_reason: None,
                    outcome: VerificationOutcome::Passed,
                })
            }
        },
    )
    .await
    .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(outcome.0.merged);
    assert!(!worktree_root.exists());
    assert!(outcome.1.unwrap().passed);
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn infrastructure_failure_is_reported_and_preserves_source() {
    let (_temp, gcx, service, worktree_id, worktree_root) = fixture().await;
    let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    let leases = leases_for(&app)
        .await
        .expect("coordination lease store is live");
    let source_root = service.source_workspace_root().to_path_buf();
    let request = post_merge_request(&gcx, &source_root).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let callback_calls = calls.clone();
    let callback_leases = leases.clone();
    let callback_root = source_root.clone();
    let outcome = super::merge_under_available_lease(
        &app,
        &service,
        &worktree_id,
        merge_request(),
        Some(request),
        move |_gcx, _request| {
            let callback_calls = callback_calls.clone();
            let leases = callback_leases.clone();
            let source_root = callback_root.clone();
            async move {
                callback_calls.fetch_add(1, Ordering::SeqCst);
                assert!(leases
                    .inspect(
                        &merge_lease_key(&checkout_key(&source_root)),
                        refact_work_core::store::records::now_ms(),
                    )
                    .await
                    .unwrap()
                    .is_some());
                Err("fixture check failure".to_string())
            }
        },
    )
    .await
    .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        outcome.1.unwrap().outcome,
        VerificationOutcome::InfrastructureFailed
    );
    assert!(worktree_root.exists());
    assert_eq!(
        std::fs::read_to_string(source_root.join("file.txt")).unwrap(),
        "merged\n"
    );
}
#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn lease_callback_fails_closed_without_a_merge_commit() {
    let (_temp, gcx, _service, _worktree_id, _worktree_root) = fixture().await;
    let request = post_merge_request(&gcx, Path::new("/tmp")).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let callback_calls = calls.clone();
    let verification = super::verify_merged_checkout(
        gcx,
        move |_gcx, _request| {
            callback_calls.fetch_add(1, Ordering::SeqCst);
            async { Err("must not execute".to_string()) }
        },
        Some(request),
        PathBuf::from("/tmp"),
        None,
    )
    .await
    .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        verification.outcome,
        VerificationOutcome::InfrastructureFailed
    );
    assert_eq!(verification.output_tail, "merge produced no commit");
    assert!(!verification.auto_reverted);
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn lease_callback_uses_merge_commit_instead_of_current_head() {
    let (_temp, gcx, service, _worktree_id, _worktree_root) = fixture().await;
    let root = service.source_workspace_root().to_path_buf();
    git(&root, &["checkout", "-b", "feature"]);
    std::fs::write(root.join("feature.txt"), "feature\n").unwrap();
    git(&root, &["add", "feature.txt"]);
    git(&root, &["commit", "-m", "feature"]);
    git(&root, &["checkout", "main"]);
    git(
        &root,
        &["merge", "--no-ff", "feature", "-m", "merge feature"],
    );
    let merge_commit = git(&root, &["rev-parse", "HEAD"]).trim().to_string();
    std::fs::write(root.join("external.txt"), "external\n").unwrap();
    git(&root, &["add", "external.txt"]);
    git(&root, &["commit", "-m", "external commit"]);
    let current_head = git(&root, &["rev-parse", "HEAD"]).trim().to_string();
    assert_ne!(merge_commit, current_head);

    let request = post_merge_request(&gcx, &root).await;
    let captured = Arc::new(Mutex::new(None));
    let captured_request = captured.clone();
    let result = super::verify_merged_checkout(
        gcx,
        move |_gcx, request| {
            captured_request.lock().unwrap().replace(request);
            async {
                Ok(PostMergeCheckResult {
                    checked: true,
                    auto_reverted: false,
                    command: Some("cargo test".to_string()),
                    exit_code: Some(0),
                    output_tail: String::new(),
                    merge_commit: None,
                    revert_commit: None,
                    fix_card_id: None,
                    skipped_reason: None,
                    revert_skipped_reason: None,
                    outcome: VerificationOutcome::Passed,
                })
            }
        },
        Some(request),
        root,
        Some(merge_commit.clone()),
    )
    .await
    .unwrap();

    assert!(result.passed);
    let captured = captured.lock().unwrap();
    let captured = captured.as_ref().unwrap();
    assert_eq!(captured.expected_merge_commit, merge_commit);
    assert_ne!(captured.expected_merge_commit, current_head);
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn lease_callback_skips_revert_when_external_commit_moves_head() {
    let (_temp, gcx, service, worktree_id, worktree_root) = fixture().await;
    let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
    let source_root = service.source_workspace_root().to_path_buf();
    let request = post_merge_request(&gcx, &source_root).await;
    let external_commit = Arc::new(Mutex::new(None));
    let callback_external_commit = external_commit.clone();
    let callback_root = source_root.clone();
    let outcome = super::merge_under_available_lease(
        &app,
        &service,
        &worktree_id,
        merge_request(),
        Some(request),
        move |gcx, request| {
            let external_commit = callback_external_commit.clone();
            let source_root = callback_root.clone();
            async move {
                std::fs::write(source_root.join("external.txt"), "external\n").unwrap();
                git(&source_root, &["add", "external.txt"]);
                git(&source_root, &["commit", "-m", "external commit"]);
                let current_head = git(&source_root, &["rev-parse", "HEAD"])
                    .trim()
                    .to_string();
                *external_commit.lock().unwrap() = Some(current_head);
                let mut runner = RevertingRunner::default();
                let result = post_merge_check_with_runner(gcx, request, &mut runner).await;
                assert!(!runner.calls.iter().any(|command| matches!(
                    command,
                    PostMergeCommand::Git(args) if args.first().map(String::as_str) == Some("revert")
                )));
                result
            }
        },
    )
    .await
    .unwrap();

    assert!(outcome.0.merged);
    let verification = outcome.1.expect("opted-in post-merge verification");
    assert_eq!(verification.outcome, VerificationOutcome::CommandFailed);
    assert!(!verification.auto_reverted);
    assert_eq!(
        verification.revert_skipped_reason.as_deref(),
        Some("HEAD changed since merge")
    );
    let external_commit = external_commit.lock().unwrap().clone().unwrap();
    assert_eq!(
        git(&source_root, &["rev-parse", "HEAD"]).trim(),
        external_commit
    );
    let history = git(&source_root, &["log", "--format=%H"]);
    assert!(history.lines().any(|commit| commit == external_commit));
    assert!(
        worktree_root.exists(),
        "unreverted verification retains the source"
    );
}
