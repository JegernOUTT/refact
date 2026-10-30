use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ContextEnum;
use crate::global_context::GlobalContext;
use crate::tasks::storage;
use crate::tasks::types::{BoardCard, TaskBoard};
use crate::tools::tool_spawn_ab::tests::{setup_repo_task, test_card, RecordingFacade};
use crate::tools::tool_spawn_ab::ToolSpawnAb;
use crate::tools::tool_task_spawn_agent::{scoped_worktree_gate, ToolTaskSpawnAgent};
use crate::tools::tools_description::Tool;

fn git(root: &Path, args: &[&str]) -> String {
    crate::worktrees::git::run_git(root, args).unwrap()
}

fn card(id: &str, column: &str, files: &[&str]) -> BoardCard {
    let mut card = test_card();
    card.id = id.into();
    card.title = id.into();
    card.column = column.into();
    card.target_files = files.iter().map(|file| (*file).into()).collect();
    card
}

struct Fixture {
    gcx: Arc<GlobalContext>,
    _temp: tempfile::TempDir,
    source: PathBuf,
    worktree: PathBuf,
    board: TaskBoard,
}

impl Fixture {
    async fn new() -> Self {
        let (gcx, temp, source) = setup_repo_task().await;
        for file in ["removed.txt", "renamed-from.txt"] {
            std::fs::write(source.join(file), "initial\n").unwrap();
            git(&source, &["add", file]);
        }
        git(&source, &["commit", "-m", "seed files"]);
        let worktree = temp.path().join("done-worktree");
        git(
            &source,
            &[
                "worktree",
                "add",
                "-b",
                "agent-done",
                worktree.to_str().unwrap(),
                "main",
            ],
        );
        std::fs::write(worktree.join("actual.txt"), "done card change\n").unwrap();
        std::fs::write(worktree.join("file.txt"), "modified\n").unwrap();
        git(&worktree, &["rm", "removed.txt"]);
        git(&worktree, &["mv", "renamed-from.txt", "renamed-to.txt"]);
        git(&worktree, &["add", "actual.txt", "file.txt"]);
        git(&worktree, &["commit", "-m", "done card changes"]);
        let mut done = card("DONE", "done", &["declared-only.txt"]);
        done.agent_branch = Some("agent-done".into());
        done.agent_worktree = Some(worktree.to_string_lossy().into_owned());
        done.base_branch = Some("main".into());
        let mut middle = card("MIDDLE", "done", &[]);
        middle.depends_on = vec!["DONE".into()];
        let board = TaskBoard {
            cards: vec![done, middle, card("NEXT", "planned", &["unrelated.txt"])],
            ..Default::default()
        };
        Self {
            gcx,
            _temp: temp,
            source,
            worktree,
            board,
        }
    }

    async fn gate(&self) -> Result<Option<String>, String> {
        self.gate_at(None).await
    }

    async fn gate_at(&self, base_commit: Option<&str>) -> Result<Option<String>, String> {
        scoped_worktree_gate(
            &self.board,
            "NEXT",
            &self.source,
            Some("main"),
            base_commit,
            "spawn a new task agent",
        )
        .await
    }

    async fn context(
        &self,
    ) -> (
        crate::app_state::AppState,
        Arc<tokio::sync::Mutex<AtCommandsContext>>,
        refact_work_core::Permit,
    ) {
        storage::update_board_atomic(self.gcx.clone(), "task-1", {
            let cards = self.board.cards.clone();
            move |board| {
                board.cards = cards;
                Ok(())
            }
        })
        .await
        .unwrap();
        let mut meta = storage::load_task_meta(self.gcx.clone(), "task-1")
            .await
            .unwrap();
        meta.base_branch = Some("main".into());
        meta.base_commit = Some(
            git(&self.source, &["rev-parse", "agent-done^"])
                .trim()
                .into(),
        );
        storage::save_task_meta(self.gcx.clone(), "task-1", &meta)
            .await
            .unwrap();
        let mut caps = crate::caps::CodeAssistantCaps::default();
        caps.defaults.task_planner_agent_model = "provider/test-model".into();
        caps.chat_models.insert(
            "provider/test-model".into(),
            Arc::new(crate::caps::ChatModelRecord::default()),
        );
        {
            let mut state = self.gcx.caps_state.write().await;
            state.caps = Some(Arc::new(caps));
            state.last_attempted_ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
        }
        let mut app = crate::app_state::AppState::from_gcx(self.gcx.clone()).await;
        app.chat.facade = Arc::new(RecordingFacade::default());
        app.work_scheduler = Arc::new(refact_work_core::AdmissionScheduler::new(
            refact_work_core::CoordinationSettings {
                max_active_engine: 1,
                max_active_project: 1,
                max_active_root: 1,
                ..Default::default()
            },
        ));
        let project = crate::coordination::roots::project_identity(&self.source);
        let permit = app
            .work_scheduler
            .reserve(refact_work_core::AdmissionRequest::new(
                refact_work_core::WorkKey::background_agent("holder").run(1),
                crate::tasks::admission::board_scope(&project, "planner-chat"),
            ))
            .unwrap()
            .acquire()
            .await
            .unwrap();
        let ccx = Arc::new(tokio::sync::Mutex::new(
            AtCommandsContext::new_from_app(
                app.clone(),
                4096,
                20,
                false,
                vec![],
                "planner-chat".into(),
                Some("planner-chat".into()),
                "provider/test-model".into(),
                Some(refact_chat_api::TaskMeta {
                    task_id: "task-1".into(),
                    role: "planner".into(),
                    agent_id: None,
                    card_id: None,
                    planner_chat_id: Some("planner-chat".into()),
                }),
                None,
            )
            .await,
        ));
        (app, ccx, permit)
    }
}

fn args() -> HashMap<String, Value> {
    HashMap::from([
        ("card_id".into(), json!("NEXT")),
        ("variant_a".into(), json!({"model":"provider/test-model"})),
        ("variant_b".into(), json!({"model":"provider/test-model"})),
    ])
}

fn response(output: &[ContextEnum]) -> Value {
    let ContextEnum::ChatMessage(message) = &output[0] else {
        panic!("expected tool response");
    };
    serde_json::from_str(&message.content.content_text_only()).unwrap()
}

#[tokio::test]
async fn unrelated_unmerged_done_card_does_not_block_spawn() {
    let fixture = Fixture::new().await;
    let (app, ccx, permit) = fixture.context().await;
    let result = ToolTaskSpawnAgent::new()
        .tool_execute(ccx, &"spawn".into(), &args())
        .await;
    crate::tasks::admission::shutdown(&app).await;
    permit.release();
    let (_, output) = result.expect("unrelated done work must not block spawning");
    let result = response(&output);
    assert_eq!(result["execution_state"], "queued");
    let warning = result["warning"].as_str().expect("unmerged work warning");
    assert!(warning.contains("DONE (`agent-done`)"), "{warning}");
    assert_eq!(warning.lines().count(), 1);
    assert!(fixture.worktree.is_dir());
}

#[tokio::test]
async fn dependent_card_is_blocked() {
    let mut fixture = Fixture::new().await;
    for dependency in ["DONE", "MIDDLE"] {
        fixture.board.get_card_mut("NEXT").unwrap().depends_on = vec![dependency.into()];
        let (app, ccx, permit) = fixture.context().await;
        let result = ToolTaskSpawnAgent::new()
            .tool_execute(ccx, &"spawn".into(), &args())
            .await;
        assert_eq!(app.work_scheduler.snapshot().outstanding, 1);
        crate::tasks::admission::shutdown(&app).await;
        permit.release();
        let error = result.unwrap_err();
        assert!(error.contains("DONE (`agent-done`)"), "{error}");
        assert!(error.contains("dependency"), "{error}");
        assert!(error.contains("merge_agent(card_id=...)"), "{error}");
        assert!(!error.contains("git branch -D"), "{error}");
        let board = storage::load_board(fixture.gcx.clone(), "task-1")
            .await
            .unwrap();
        assert_eq!(board.get_card("NEXT").unwrap().column, "planned");
    }
    let main = git(&fixture.source, &["rev-parse", "main"]);
    let included = git(&fixture.worktree, &["rev-parse", "HEAD"]);
    assert_ne!(main, included);
    assert!(git(&fixture.worktree, &["status", "--porcelain"]).is_empty());
    fixture.board.get_card_mut("NEXT").unwrap().target_files = vec!["actual.txt".into()];
    let warning = fixture
        .gate_at(Some(included.trim()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(warning, format!(
        "Advisory: retained task source DONE (`agent-done`) is included in requested launch base `{}`, not merged into target `main`. Checkouts and branches are retained; inclusion is not verification or main landing.",
        included.trim()
    ));
    let (mut app, ccx, permit) = fixture.context().await;
    let facade = Arc::new(RecordingFacade::default());
    app.chat.facade = facade.clone();
    ccx.lock().await.app = app.clone();
    git(&fixture.source, &["branch", "launch-base", included.trim()]);
    let mut spawn_args = args();
    spawn_args.insert("base_branch".into(), json!("main"));
    spawn_args.insert("base_commit".into(), json!("launch-base"));
    let result = ToolTaskSpawnAgent::new()
        .tool_execute(ccx, &"spawn-included".into(), &spawn_args)
        .await;
    let (_, output) = result.unwrap();
    let output = response(&output);
    assert_eq!(output["execution_state"], "queued");
    assert_eq!(output["warning"], warning);
    assert_eq!(output["base_branch"], "main");
    assert_eq!(output["base_commit"], included.trim());
    assert!(facade.created.lock().unwrap().is_empty());
    git(&fixture.source, &["branch", "-f", "launch-base", "main"]);
    assert_eq!(git(&fixture.source, &["rev-parse", "launch-base"]), main);
    permit.release();
    crate::tools::tool_spawn_ab::tests::await_started(&facade, 1).await;
    let thread = facade.threads.lock().unwrap()[0].clone();
    let worktree = thread.worktree.as_ref().unwrap();
    assert_eq!(worktree.base_commit.as_deref(), Some(included.trim()));
    assert_eq!(git(&worktree.root, &["rev-parse", "HEAD"]), included);
    assert_eq!(
        crate::chat::worktree_lifecycle::check_managed_attachment(&app, &thread.id, &thread).await,
        Ok(())
    );
    crate::tasks::admission::cancel_setup_and_wait(&thread.id, std::time::Duration::from_secs(5))
        .await
        .unwrap();
    crate::tasks::admission::end(&thread.id).await;
    crate::tasks::admission::shutdown(&app).await;
    assert!(fixture.worktree.is_dir());
    assert_eq!(git(&fixture.source, &["rev-parse", "main"]), main);
    assert_eq!(git(&fixture.worktree, &["rev-parse", "HEAD"]), included);

    std::fs::write(fixture.worktree.join("actual.txt"), "uncommitted work\n").unwrap();
    let error = fixture.gate_at(Some(included.trim())).await.unwrap_err();
    assert!(error.contains("dependency"), "{error}");
    assert!(error.contains("file overlap: actual.txt"), "{error}");
    git(&fixture.worktree, &["add", "actual.txt"]);
    assert!(fixture.gate_at(Some(included.trim())).await.is_err());
    git(&fixture.worktree, &["commit", "-m", "new predecessor work"]);
    assert_ne!(git(&fixture.worktree, &["rev-parse", "HEAD"]), included);
    assert!(git(&fixture.worktree, &["status", "--porcelain"]).is_empty());
    let error = fixture.gate_at(Some(included.trim())).await.unwrap_err();
    assert!(error.contains("dependency"), "{error}");
}

#[tokio::test]
async fn overlapping_target_files_are_blocked() {
    let mut fixture = Fixture::new().await;
    for path in [
        "actual.txt",
        "removed.txt",
        "renamed-from.txt",
        "renamed-to.txt",
    ] {
        fixture.board.get_card_mut("NEXT").unwrap().target_files = vec![path.into()];
        let error = fixture.gate().await.unwrap_err();
        assert!(error.contains("DONE (`agent-done`)"), "{error}");
        assert!(error.contains(&format!("file overlap: {path}")), "{error}");
    }
    fixture.board.get_card_mut("NEXT").unwrap().target_files = vec!["actual.txt".into()];
    let (app, ccx, permit) = fixture.context().await;
    let result = ToolTaskSpawnAgent::new()
        .tool_execute(ccx, &"spawn".into(), &args())
        .await;
    assert_eq!(app.work_scheduler.snapshot().outstanding, 1);
    crate::tasks::admission::shutdown(&app).await;
    permit.release();
    assert!(result.unwrap_err().contains("file overlap: actual.txt"));

    let included = git(&fixture.worktree, &["rev-parse", "HEAD"]);
    std::fs::write(fixture.worktree.join("new-wip.txt"), "untracked work\n").unwrap();
    fixture.board.get_card_mut("NEXT").unwrap().target_files = vec!["new-wip.txt".into()];
    let error = fixture.gate_at(Some(included.trim())).await.unwrap_err();
    assert!(error.contains("file overlap: new-wip.txt"), "{error}");
    assert!(fixture.worktree.join("new-wip.txt").is_file());
}

#[tokio::test]
async fn squash_merged_branch_is_recognised() {
    let mut fixture = Fixture::new().await;
    let next = fixture.board.get_card_mut("NEXT").unwrap();
    next.depends_on = vec!["DONE".into()];
    next.target_files = vec!["actual.txt".into()];
    let tree = git(&fixture.source, &["rev-parse", "agent-done^{tree}"]);
    let main = git(&fixture.source, &["rev-parse", "main"]);
    let candidate = git(
        &fixture.source,
        &[
            "commit-tree",
            tree.trim(),
            "-p",
            main.trim(),
            "-m",
            "candidate squash",
        ],
    );
    assert!(!crate::worktrees::git::branch_merged_into(
        &fixture.source,
        "agent-done",
        candidate.trim()
    ));
    let advisory = fixture
        .gate_at(Some(candidate.trim()))
        .await
        .unwrap()
        .unwrap();
    assert!(
        advisory.contains("included in requested launch base"),
        "{advisory}"
    );
    assert!(
        advisory.contains("not merged into target `main`"),
        "{advisory}"
    );
    assert_eq!(git(&fixture.source, &["rev-parse", "main"]), main);
    git(&fixture.source, &["merge", "--squash", "agent-done"]);
    git(&fixture.source, &["commit", "-m", "squash done"]);
    std::fs::write(fixture.source.join("base-only.txt"), "later base work\n").unwrap();
    git(&fixture.source, &["add", "base-only.txt"]);
    git(&fixture.source, &["commit", "-m", "unrelated base change"]);
    assert!(!crate::worktrees::git::branch_merged_into(
        &fixture.source,
        "agent-done",
        "main"
    ));
    let next = fixture.board.get_card_mut("NEXT").unwrap();
    next.depends_on = vec!["DONE".into()];
    next.target_files = vec!["actual.txt".into()];
    assert_eq!(fixture.gate().await.unwrap(), None);
    let (app, ccx, permit) = fixture.context().await;
    let result = ToolTaskSpawnAgent::new()
        .tool_execute(ccx, &"spawn".into(), &args())
        .await;
    crate::tasks::admission::shutdown(&app).await;
    permit.release();
    let (_, output) = result.unwrap();
    assert!(response(&output).get("warning").is_none());
    assert!(fixture.worktree.exists());
}

#[tokio::test]
async fn spawn_ab_uses_the_same_scoping() {
    for scope in ["unrelated", "dependency", "overlap", "squash"] {
        let mut fixture = Fixture::new().await;
        match scope {
            "dependency" => {
                fixture.board.get_card_mut("NEXT").unwrap().depends_on = vec!["MIDDLE".into()];
            }
            "overlap" => {
                fixture.board.get_card_mut("NEXT").unwrap().target_files =
                    vec!["actual.txt".into()];
            }
            "squash" => {
                git(&fixture.source, &["merge", "--squash", "agent-done"]);
                git(&fixture.source, &["commit", "-m", "squash"]);
                let next = fixture.board.get_card_mut("NEXT").unwrap();
                next.depends_on = vec!["DONE".into()];
                next.target_files = vec!["actual.txt".into()];
            }
            _ => {}
        }
        let (app, ccx, permit) = fixture.context().await;
        let result = ToolSpawnAb::new()
            .tool_execute(ccx, &"spawn-ab".into(), &args())
            .await;
        if matches!(scope, "dependency" | "overlap") {
            assert_eq!(app.work_scheduler.snapshot().outstanding, 1);
        }
        crate::tasks::admission::shutdown(&app).await;
        permit.release();
        match scope {
            "dependency" | "overlap" => {
                let error = result.unwrap_err();
                assert!(error.contains("Cannot spawn A/B agents"), "{error}");
                assert!(error.contains("DONE (`agent-done`)"), "{error}");
                let reason = if scope == "overlap" {
                    "file overlap"
                } else {
                    "dependency"
                };
                assert!(error.contains(reason), "{error}");
            }
            _ => {
                let (_, output) = result.unwrap();
                let result = response(&output);
                assert_eq!(result["execution_state"], "queued");
                if scope == "unrelated" {
                    assert!(result["warning"]
                        .as_str()
                        .unwrap()
                        .contains("DONE (`agent-done`)"));
                } else {
                    assert!(result.get("warning").is_none());
                }
            }
        }
        assert!(fixture.worktree.is_dir());
    }
}

#[tokio::test]
async fn incomplete_squash_is_not_recognised_as_merged() {
    let mut fixture = Fixture::new().await;
    std::fs::write(fixture.source.join("actual.txt"), "done card change\n").unwrap();
    git(&fixture.source, &["add", "actual.txt"]);
    git(&fixture.source, &["commit", "-m", "partial squash"]);
    fixture.board.get_card_mut("NEXT").unwrap().depends_on = vec!["DONE".into()];
    assert!(fixture.gate().await.unwrap_err().contains("dependency"));
    let partial = git(&fixture.source, &["rev-parse", "HEAD"]);
    assert!(fixture
        .gate_at(Some(partial.trim()))
        .await
        .unwrap_err()
        .contains("dependency"));
}

#[tokio::test]
async fn git_errors_use_declared_targets_without_blocking_unrelated_cards() {
    let mut fixture = Fixture::new().await;
    fixture.board.get_card_mut("DONE").unwrap().agent_branch = Some("missing-branch".into());
    assert!(fixture
        .gate()
        .await
        .unwrap()
        .unwrap()
        .contains("DONE (`missing-branch`)"));
    fixture.board.get_card_mut("NEXT").unwrap().target_files = vec!["declared-only.txt".into()];
    assert!(fixture
        .gate()
        .await
        .unwrap_err()
        .contains("file overlap: declared-only.txt"));

    fixture.board.get_card_mut("DONE").unwrap().agent_branch = Some("agent-done".into());
    fixture.board.get_card_mut("NEXT").unwrap().depends_on = vec!["DONE".into()];
    let included = git(&fixture.worktree, &["rev-parse", "HEAD"]);
    git(&fixture.worktree, &["checkout", "--detach"]);
    assert!(git(&fixture.worktree, &["branch", "--show-current"]).is_empty());
    assert!(fixture
        .gate_at(Some(included.trim()))
        .await
        .unwrap_err()
        .contains("dependency"));
    git(&fixture.worktree, &["checkout", "agent-done"]);
    let foreign = fixture._temp.path().join("foreign");
    git(
        &fixture.source,
        &[
            "clone",
            "--no-hardlinks",
            fixture.source.to_str().unwrap(),
            foreign.to_str().unwrap(),
        ],
    );
    git(
        &foreign,
        &["checkout", "-b", "agent-done", "origin/agent-done"],
    );
    assert_eq!(git(&foreign, &["rev-parse", "HEAD"]), included);
    assert_ne!(
        git(
            &foreign,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"]
        ),
        git(
            &fixture.worktree,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"]
        )
    );
    fixture.board.get_card_mut("DONE").unwrap().agent_worktree =
        Some(foreign.to_string_lossy().into_owned());
    assert!(fixture
        .gate_at(Some(included.trim()))
        .await
        .unwrap_err()
        .contains("dependency"));
    assert!(fixture.worktree.is_dir());
}

#[tokio::test]
async fn squash_merged_after_pinned_base_uses_current_branch_tip() {
    let mut fixture = Fixture::new().await;
    let pinned = git(&fixture.source, &["rev-parse", "main"]);
    git(&fixture.source, &["merge", "--squash", "agent-done"]);
    git(
        &fixture.source,
        &["commit", "-m", "squash after pinned base"],
    );
    fixture.board.get_card_mut("NEXT").unwrap().depends_on = vec!["DONE".into()];
    assert_eq!(
        scoped_worktree_gate(
            &fixture.board,
            "NEXT",
            &fixture.source,
            Some("main"),
            Some(pinned.trim()),
            "spawn a new task agent",
        )
        .await
        .unwrap(),
        None
    );
}

#[tokio::test]
async fn missing_base_branch_uses_pinned_commit_and_explains_it() {
    let mut fixture = Fixture::new().await;
    let pinned = git(&fixture.source, &["rev-parse", "main"]);
    git(&fixture.source, &["branch", "old-base"]);
    git(&fixture.source, &["branch", "-D", "old-base"]);
    let warning = scoped_worktree_gate(
        &fixture.board,
        "NEXT",
        &fixture.source,
        Some("old-base"),
        Some(pinned.trim()),
        "spawn a new task agent",
    )
    .await
    .unwrap()
    .unwrap();
    assert!(warning.contains("DONE (`agent-done`)"), "{warning}");
    assert!(
        warning.contains("Base branch `old-base` is unavailable"),
        "{warning}"
    );
    assert!(warning.contains("pinned base commit"), "{warning}");
    assert_eq!(warning.lines().count(), 1);
    fixture.board.get_card_mut("NEXT").unwrap().depends_on = vec!["DONE".into()];
    let error = scoped_worktree_gate(
        &fixture.board,
        "NEXT",
        &fixture.source,
        Some("old-base"),
        Some(pinned.trim()),
        "spawn a new task agent",
    )
    .await
    .unwrap_err();
    assert!(error.contains("dependency"), "{error}");
    assert!(error.contains("pinned base commit"), "{error}");
    let error = fixture.gate_at(Some("missing-commit")).await.unwrap_err();
    assert!(
        error.contains("requested launch base is unavailable"),
        "{error}"
    );
}
