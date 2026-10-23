use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::app_state::AppState;
use crate::chat::post_merge_check::{PostMergeCheckRequest, PostMergeCheckResult};
use crate::chat::post_merge_check::{
    post_merge_check_with_runner, PostMergeCommand, PostMergeCommandOutput, PostMergeCommandRunner,
};
use crate::coordination::leases::leases_for;
use crate::coordination::merge_gate::{checkout_key, merge_lease_key};
use crate::global_context::GlobalContext;
use crate::tasks::types::VerificationOutcome;
use crate::worktrees::git::run_git;
use crate::worktrees::service::{merge_lock_for, WorktreeService};
use crate::worktrees::types::{CreateWorktreeRequest, MergeWorktreeRequest, WorktreeMergeStrategy};

use super::tool_task_merge_agent::merge_under_available_lease;

pub(super) const TARGET: &str = "refact/chat/planner";

pub(super) fn git(root: &Path, args: &[&str]) -> String {
    run_git(root, args).unwrap_or_else(|error| panic!("git {args:?}: {error}"))
}

pub(super) struct Fixture {
    _temp: tempfile::TempDir,
    pub(super) gcx: Arc<GlobalContext>,
    pub(super) service: WorktreeService,
    pub(super) source: PathBuf,
    pub(super) planner: PathBuf,
    pub(super) agent: PathBuf,
    id: String,
}

impl Fixture {
    pub(super) async fn new(planner_checkout: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir(&source).unwrap();
        git(&source, &["init"]);
        git(&source, &["checkout", "-b", "main"]);
        git(&source, &["config", "user.name", "Test User"]);
        git(&source, &["config", "user.email", "test@example.com"]);
        std::fs::write(source.join("file.txt"), "base\n").unwrap();
        std::fs::write(source.join(".gitignore"), ".refact/\n").unwrap();
        git(&source, &["add", "file.txt", ".gitignore"]);
        git(&source, &["commit", "-m", "initial"]);
        git(&source, &["branch", TARGET]);
        let planner = temp.path().join("planner checkout");
        if planner_checkout {
            git(
                &source,
                &["worktree", "add", planner.to_str().unwrap(), TARGET],
            );
        }
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            temp.path().join("cache"),
            temp.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        let service = WorktreeService::new(gcx.cache_dir.clone(), source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/task/task-1/card/T-1/agent".to_string()),
                base_branch: Some(TARGET.to_string()),
                kind: Some("task_agent".to_string()),
                task_id: Some("task-1".to_string()),
                card_id: Some("T-1".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let agent = created.worktree.meta.root;
        std::fs::write(agent.join("file.txt"), "merged\n").unwrap();
        git(&agent, &["add", "file.txt"]);
        git(&agent, &["commit", "-m", "agent change"]);
        Self {
            _temp: temp,
            gcx,
            service,
            source,
            planner,
            agent,
            id: created.worktree.meta.id,
        }
    }

    fn request(&self) -> MergeWorktreeRequest {
        MergeWorktreeRequest {
            strategy: WorktreeMergeStrategy::Squash,
            target_branch: Some(TARGET.to_string()),
            commit_message: Some("squash agent change".to_string()),
            include_uncommitted: true,
            ..Default::default()
        }
    }

    fn check_request(&self) -> PostMergeCheckRequest {
        PostMergeCheckRequest {
            task_id: "task-1".to_string(),
            card_id: "T-1".to_string(),
            workspace_root: self.source.clone(),
            enabled: true,
            timeout: Duration::from_secs(5),
            expected_merge_commit: String::new(),
            expected_card_state: None,
        }
    }

    pub(super) fn write_board(&self, legacy: bool) {
        let branch = git(&self.agent, &["branch", "--show-current"]);
        let card: crate::tasks::types::BoardCard = serde_json::from_value(serde_json::json!({
            "id": "T-1",
            "title": "Target checkout regression",
            "column": "done",
            "instructions": "## Acceptance Criteria\n- Verify: `cargo test --lib`",
            "created_at": chrono::Utc::now().to_rfc3339(),
            "final_report": "done",
            "agent_branch": branch.trim(),
            "agent_worktree": self.agent,
            "agent_worktree_name": if legacy { None } else { Some(&self.id) },
            "base_branch": TARGET
        }))
        .unwrap();
        let meta: crate::tasks::types::TaskMeta = serde_json::from_value(serde_json::json!({
            "id": "task-1", "name": "Target checkout", "status": "active",
            "created_at": chrono::Utc::now().to_rfc3339(),
            "updated_at": chrono::Utc::now().to_rfc3339(),
            "base_branch": TARGET
        }))
        .unwrap();
        let mut board = crate::tasks::types::TaskBoard::default();
        board.cards.push(card);
        let task = self.source.join(".refact/tasks/task-1");
        std::fs::create_dir_all(&task).unwrap();
        std::fs::write(
            task.join("meta.yaml"),
            serde_yaml::to_string(&meta).unwrap(),
        )
        .unwrap();
        std::fs::write(
            task.join("board.yaml"),
            serde_yaml::to_string(&board).unwrap(),
        )
        .unwrap();
    }
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn merge_into_base_checked_out_in_planner_worktree_succeeds() {
    let fixture = Fixture::new(true).await;
    let source_before = git(&fixture.source, &["rev-parse", "HEAD"]);
    std::fs::write(fixture.source.join("notes.txt"), "source stays dirty\n").unwrap();
    let source_status = git(&fixture.source, &["status", "--porcelain"]);
    let target_before = git(&fixture.planner, &["rev-parse", "HEAD"]);
    let app = AppState::from_gcx(fixture.gcx.clone()).await;
    let leases = leases_for(&app).await.unwrap();
    let planner = fixture.planner.clone();
    let source = fixture.source.clone();
    let (response, verification, lease_used) = merge_under_available_lease(
        &app,
        &fixture.service,
        &fixture.id,
        fixture.request(),
        Some(fixture.check_request()),
        move |_, request| async move {
            assert_eq!(request.workspace_root, planner.canonicalize().unwrap());
            assert_eq!(
                git(&planner, &["rev-parse", "HEAD"]).trim(),
                request.expected_merge_commit
            );
            assert!(leases
                .inspect(
                    &merge_lease_key(&checkout_key(&planner)),
                    refact_work_core::store::records::now_ms()
                )
                .await
                .unwrap()
                .is_some());
            assert!(leases
                .inspect(
                    &merge_lease_key(&checkout_key(&source)),
                    refact_work_core::store::records::now_ms()
                )
                .await
                .unwrap()
                .is_none());
            Ok(PostMergeCheckResult {
                checked: true,
                outcome: VerificationOutcome::Passed,
                auto_reverted: false,
                command: Some("test".to_string()),
                exit_code: Some(0),
                output_tail: String::new(),
                merge_commit: None,
                revert_commit: None,
                fix_card_id: None,
                skipped_reason: None,
                revert_skipped_reason: None,
            })
        },
    )
    .await
    .unwrap();
    assert!(response.merged);
    assert!(lease_used);
    assert!(verification.unwrap().passed);
    assert_ne!(git(&fixture.planner, &["rev-parse", "HEAD"]), target_before);
    assert_eq!(
        git(&fixture.planner, &["rev-parse", "HEAD^"]),
        target_before
    );
    assert_eq!(
        git(&fixture.planner, &["rev-list", "--count", "HEAD"]).trim(),
        "2"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.planner.join("file.txt")).unwrap(),
        "merged\n"
    );
    assert_eq!(
        git(&fixture.source, &["branch", "--show-current"]).trim(),
        "main"
    );
    assert_eq!(git(&fixture.source, &["rev-parse", "HEAD"]), source_before);
    assert_eq!(
        git(&fixture.source, &["status", "--porcelain"]),
        source_status
    );
    assert_eq!(
        std::fs::read_to_string(fixture.source.join("file.txt")).unwrap(),
        "base\n"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.source.join("notes.txt")).unwrap(),
        "source stays dirty\n"
    );
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn merge_refuses_when_target_worktree_is_dirty_and_touches_nothing() {
    let fixture = Fixture::new(true).await;
    std::fs::write(fixture.planner.join("file.txt"), "staged user edit\n").unwrap();
    git(&fixture.planner, &["add", "file.txt"]);
    std::fs::write(fixture.planner.join("file.txt"), "unstaged user edit\n").unwrap();
    std::fs::write(fixture.planner.join("notes.txt"), "user notes\n").unwrap();
    std::fs::write(fixture.agent.join("pending.txt"), "do not auto-commit\n").unwrap();
    let snapshot = |root: &Path| {
        (
            git(root, &["rev-parse", "HEAD"]),
            git(root, &["status", "--porcelain"]),
            git(root, &["diff", "--cached"]),
            git(root, &["diff"]),
        )
    };
    let before = [&fixture.source, &fixture.planner, &fixture.agent].map(|root| snapshot(root));
    let registry = std::fs::read(fixture.service.registry_path()).unwrap();
    let app = AppState::from_gcx(fixture.gcx.clone()).await;
    let (response, _, _) = merge_under_available_lease(
        &app,
        &fixture.service,
        &fixture.id,
        fixture.request(),
        Some(fixture.check_request()),
        |_, _| async { panic!("dirty target must not be verified") },
    )
    .await
    .unwrap();
    assert!(!response.merged);
    assert_eq!(response.status, "dirty_baseline");
    assert_eq!(
        response.dirty_baseline.unwrap().branch.as_deref(),
        Some(TARGET)
    );
    let after = [&fixture.source, &fixture.planner, &fixture.agent].map(|root| snapshot(root));
    assert_eq!(before, after);
    assert_eq!(
        std::fs::read(fixture.service.registry_path()).unwrap(),
        registry
    );
    assert_eq!(
        std::fs::read_to_string(fixture.planner.join("notes.txt")).unwrap(),
        "user notes\n"
    );
    assert!(git(&fixture.source, &["stash", "list"]).is_empty());
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn merge_into_base_not_checked_out_anywhere_keeps_todays_behaviour() {
    let fixture = Fixture::new(false).await;
    let app = AppState::from_gcx(fixture.gcx.clone()).await;
    let (response, _, _) = merge_under_available_lease(
        &app,
        &fixture.service,
        &fixture.id,
        fixture.request(),
        None,
        |_, _| async { panic!("check not configured") },
    )
    .await
    .unwrap();
    assert!(response.merged);
    assert_eq!(
        git(&fixture.source, &["branch", "--show-current"]).trim(),
        TARGET
    );
    assert_eq!(
        std::fs::read_to_string(fixture.source.join("file.txt")).unwrap(),
        "merged\n"
    );
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn unoccupied_base_with_dirty_source_uses_and_removes_temporary_checkout() {
    let fixture = Fixture::new(false).await;
    std::fs::write(fixture.source.join("notes.txt"), "user notes\n").unwrap();
    let app = AppState::from_gcx(fixture.gcx.clone()).await;
    let (response, _, _) = merge_under_available_lease(
        &app,
        &fixture.service,
        &fixture.id,
        fixture.request(),
        None,
        |_, _| async { panic!("check not configured") },
    )
    .await
    .unwrap();
    assert!(response.merged);
    assert_eq!(
        git(&fixture.source, &["branch", "--show-current"]).trim(),
        "main"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.source.join("file.txt")).unwrap(),
        "base\n"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.source.join("notes.txt")).unwrap(),
        "user notes\n"
    );
    assert_eq!(
        git(&fixture.source, &["show", &format!("{TARGET}:file.txt")]),
        "merged\n"
    );
    assert!(!git(&fixture.source, &["worktree", "list", "--porcelain"])
        .contains("refact-merge-target-"));
}

#[tokio::test]
async fn merge_service_serialization_uses_selected_checkout_without_changing_registry_identity() {
    let fixture = Fixture::new(true).await;
    let selected = fixture.service.for_merge_target(TARGET).await.unwrap();
    assert_eq!(selected.registry_path(), fixture.service.registry_path());
    assert_eq!(
        selected.source_workspace_root(),
        fixture.service.source_workspace_root()
    );
    assert!(Arc::ptr_eq(
        &merge_lock_for(selected.merge_checkout_root()),
        &merge_lock_for(&fixture.planner)
    ));
    assert!(!Arc::ptr_eq(
        &merge_lock_for(selected.merge_checkout_root()),
        &merge_lock_for(&fixture.source)
    ));
    let lock = merge_lock_for(&fixture.planner);
    let guard = lock.lock().await;
    let merge = selected.merge_worktree(&fixture.id, fixture.request());
    tokio::pin!(merge);
    assert!(tokio::time::timeout(Duration::from_millis(100), &mut merge)
        .await
        .is_err());
    assert_eq!(
        std::fs::read_to_string(fixture.planner.join("file.txt")).unwrap(),
        "base\n"
    );
    drop(guard);
    assert!(merge.await.unwrap().merged);
}

struct RegressionRunner {
    checkout: PathBuf,
    verified: bool,
    reverted: bool,
}

#[async_trait::async_trait]
impl PostMergeCommandRunner for RegressionRunner {
    async fn run(
        &mut self,
        root: &Path,
        command: PostMergeCommand,
        _: Duration,
    ) -> PostMergeCommandOutput {
        assert_eq!(root, self.checkout.canonicalize().unwrap());
        let (success, output) = match command {
            PostMergeCommand::Verify { .. } => {
                self.verified = true;
                assert_eq!(
                    std::fs::read_to_string(root.join("file.txt")).unwrap(),
                    "merged\n"
                );
                (false, "deterministic regression".to_string())
            }
            PostMergeCommand::Git(args) => {
                self.reverted |= args.first().map(String::as_str) == Some("revert");
                (
                    true,
                    git(root, &args.iter().map(String::as_str).collect::<Vec<_>>()),
                )
            }
        };
        PostMergeCommandOutput {
            exit_code: Some(i32::from(!success)),
            success,
            output,
            outcome: if success {
                VerificationOutcome::Passed
            } else {
                VerificationOutcome::CommandFailed
            },
        }
    }
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn planner_checkout_is_revalidated_and_auto_reverted_under_its_lease() {
    let fixture = Fixture::new(true).await;
    fixture.write_board(false);
    let app = AppState::from_gcx(fixture.gcx.clone()).await;
    let leases = leases_for(&app).await.unwrap();
    let planner = fixture.planner.clone();
    let (response, verification, _) = merge_under_available_lease(
        &app,
        &fixture.service,
        &fixture.id,
        MergeWorktreeRequest {
            delete_after_merge: true,
            ..fixture.request()
        },
        Some(fixture.check_request()),
        move |gcx, request| async move {
            assert!(leases
                .inspect(
                    &merge_lease_key(&checkout_key(&planner)),
                    refact_work_core::store::records::now_ms()
                )
                .await
                .unwrap()
                .is_some());
            let mut runner = RegressionRunner {
                checkout: planner,
                verified: false,
                reverted: false,
            };
            let result = post_merge_check_with_runner(gcx, request, &mut runner).await;
            assert!(runner.verified);
            assert!(runner.reverted);
            result
        },
    )
    .await
    .unwrap();
    assert!(response.merged);
    assert!(verification.unwrap().auto_reverted);
    assert_eq!(
        std::fs::read_to_string(fixture.planner.join("file.txt")).unwrap(),
        "base\n"
    );
    assert_eq!(
        git(&fixture.source, &["rev-list", "--count", "HEAD"]).trim(),
        "1"
    );
    assert!(!fixture.agent.exists());
}

#[tokio::test]
#[serial_test::serial(runtime_settings, coordination_metrics)]
async fn merge_agent_tool_routes_registered_and_legacy_worktrees_to_planner_checkout() {
    use crate::at_commands::at_commands::AtCommandsContext;
    use crate::chat::types::TaskMeta;
    use crate::tools::tools_description::Tool;
    use super::tool_task_merge_agent::ToolTaskMergeAgent;

    for legacy in [false, true] {
        let fixture = Fixture::new(true).await;
        fixture.write_board(legacy);
        let ccx = AtCommandsContext::new_from_app(
            AppState::from_gcx(fixture.gcx.clone()).await,
            4096,
            20,
            false,
            vec![],
            "planner-chat".to_string(),
            None,
            "model".to_string(),
            Some(TaskMeta {
                task_id: "task-1".to_string(),
                role: "planner".to_string(),
                agent_id: None,
                card_id: None,
                planner_chat_id: None,
            }),
            None,
        )
        .await;
        ToolTaskMergeAgent::new()
            .tool_execute(
                Arc::new(tokio::sync::Mutex::new(ccx)),
                &"merge-call".to_string(),
                &std::collections::HashMap::from([(
                    "card_id".to_string(),
                    serde_json::json!("T-1"),
                )]),
            )
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(fixture.planner.join("file.txt")).unwrap(),
            "merged\n"
        );
        assert_eq!(
            std::fs::read_to_string(fixture.source.join("file.txt")).unwrap(),
            "base\n"
        );
        assert_eq!(
            git(&fixture.source, &["branch", "--show-current"]).trim(),
            "main"
        );
        assert!(!fixture.agent.exists());
    }
}
