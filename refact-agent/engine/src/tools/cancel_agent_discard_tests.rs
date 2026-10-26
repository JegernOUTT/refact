use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use refact_runtime_api::{ChatSessionFacade, SessionState};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::app_state::EngineChatSessionFacade;
use crate::chat::types::{ChatSession, TaskMeta};
use crate::global_context::GlobalContext;
use crate::tasks::storage;
use crate::tools::tool_agent_lifecycle::tests::{
    args, output_text, planner_ccx, test_card, write_task, MockChatFacade,
};
use crate::tools::tool_agent_lifecycle::ToolCancelAgent;
use crate::tools::tools_description::Tool;
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::CreateWorktreeRequest;

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

struct Fixture {
    _temp: tempfile::TempDir,
    source: PathBuf,
    worktree: PathBuf,
    branch: String,
    name: String,
    gcx: Arc<GlobalContext>,
    service: WorktreeService,
}

impl Fixture {
    async fn new(column: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir(&source).unwrap();
        git(&source, &["init", "-b", "main"]);
        git(&source, &["config", "user.name", "Test User"]);
        git(&source, &["config", "user.email", "test@example.com"]);
        std::fs::write(source.join("file.txt"), "base\n").unwrap();
        git(&source, &["add", "file.txt"]);
        git(&source, &["commit", "-m", "initial"]);
        let mut card = test_card(column, Some("agent-chat-1".into()), None);
        card.final_report = Some("Original report".into());
        let gcx = write_task(&source, card).await;
        let service = WorktreeService::new(gcx.cache_dir.clone(), source.clone()).unwrap();
        let branch = "refact/task/task-1/card/T-39/agent".to_string();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some(branch.clone()),
                kind: Some("task_agent".into()),
                task_id: Some("task-1".into()),
                card_id: Some("T-39".into()),
                agent_id: Some("agent-1".into()),
                chat_id: Some("agent-chat-1".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let name = created.worktree.meta.id;
        let worktree = created.worktree.meta.root;
        storage::update_board_atomic(gcx.clone(), "task-1", |board| {
            let card = board.get_card_mut("T-39").unwrap();
            card.agent_worktree = Some(worktree.to_string_lossy().into_owned());
            card.agent_worktree_name = Some(name.clone());
            card.agent_branch = Some(branch.clone());
            Ok(())
        })
        .await
        .unwrap();
        Self {
            _temp: temp,
            source,
            worktree,
            branch,
            name,
            gcx,
            service,
        }
    }

    async fn discard(&self, extra: &[(&str, Value)]) -> Result<String, String> {
        let facade = Arc::new(EngineChatSessionFacade::new(self.gcx.clone()));
        self.discard_with_facade(extra, facade).await
    }

    async fn discard_with_facade(
        &self,
        extra: &[(&str, Value)],
        facade: Arc<dyn ChatSessionFacade>,
    ) -> Result<String, String> {
        let ccx = planner_ccx(self.gcx.clone(), facade, "planner").await;
        let mut arguments = args(&[
            ("card_id", json!("T-39")),
            ("discard_worktree", json!(true)),
        ]);
        arguments.extend(args(extra));
        ToolCancelAgent::new()
            .tool_execute(ccx, &"discard".into(), &arguments)
            .await
            .map(output_text)
    }

    async fn assert_removed(&self, column: &str, keep_branch: bool) {
        let board = storage::load_board(self.gcx.clone(), "task-1")
            .await
            .unwrap();
        let card = board.get_card("T-39").unwrap();
        assert!(
            card.agent_worktree.is_none(),
            "discard must clear agent_worktree"
        );
        assert!(
            card.agent_worktree_name.is_none(),
            "discard must clear agent_worktree_name"
        );
        assert!(
            card.agent_branch.is_none(),
            "discard must clear agent_branch even when the branch is kept"
        );
        assert_eq!(card.column, column);
        assert_eq!(card.final_report.as_deref(), Some("Original report"));
        assert_eq!(card.agent_chat_id.as_deref(), Some("agent-chat-1"));
        let message = &card.status_updates.last().unwrap().message;
        assert!(
            message.contains(self.worktree.to_str().unwrap()),
            "{message}"
        );
        assert!(
            message.contains(if keep_branch {
                "Branch kept."
            } else {
                "Branch deleted."
            }),
            "{message}"
        );
        assert!(!self.worktree.exists());
        assert!(self
            .service
            .load_registry()
            .await
            .unwrap()
            .records
            .iter()
            .all(|record| record.meta.id != self.name));
        assert!(self.service.get_worktree(&self.name).await.is_err());
        assert_eq!(
            !git(&self.source, &["branch", "--list", &self.branch])
                .trim()
                .is_empty(),
            keep_branch
        );
        assert!(!git(&self.source, &["worktree", "list", "--porcelain"])
            .contains(self.worktree.to_str().unwrap()));
    }

    async fn live_session(&self) -> Arc<Mutex<ChatSession>> {
        let mut session = ChatSession::new("agent-chat-1".into());
        session.thread.task_meta = Some(TaskMeta {
            task_id: "task-1".into(),
            card_id: Some("T-39".into()),
            role: "agents".into(),
            ..Default::default()
        });
        session.start_stream().unwrap();
        session.set_runtime_state(SessionState::ExecutingTools, None);
        let session = Arc::new(Mutex::new(session));
        self.gcx
            .chat_sessions
            .write()
            .await
            .insert("agent-chat-1".into(), session.clone());
        session
    }
}

#[tokio::test]
async fn discard_worktree_clears_fields_and_registry() {
    let fixture = Fixture::new("done").await;
    let output = fixture.discard(&[]).await.unwrap();
    assert!(output.contains("Branch kept."), "{output}");
    fixture.assert_removed("done", true).await;
}

#[tokio::test]
async fn discard_refuses_doing_cards_and_dirty_worktrees_without_force() {
    let fixture = Fixture::new("doing").await;
    let error = fixture
        .discard(&[("force", json!(true))])
        .await
        .unwrap_err();
    assert!(error.contains("cancel the agent first"), "{error}");
    assert!(fixture.worktree.exists());
    storage::update_board_atomic(fixture.gcx.clone(), "task-1", |board| {
        board.get_card_mut("T-39").unwrap().column = "done".into();
        Ok(())
    })
    .await
    .unwrap();
    std::fs::write(fixture.worktree.join("file.txt"), "uncommitted\n").unwrap();
    git(&fixture.worktree, &["add", "file.txt"]);
    for index in 0..25 {
        std::fs::write(
            fixture.worktree.join(format!("dirty-{index:02}.txt")),
            "keep\n",
        )
        .unwrap();
    }
    let before = storage::load_board(fixture.gcx.clone(), "task-1")
        .await
        .unwrap();
    let registry = std::fs::read(fixture.service.registry_path()).unwrap();
    let error = fixture.discard(&[]).await.unwrap_err();
    assert!(error.contains("force=true"), "{error}");
    assert!(error.contains("dirty-00.txt"), "{error}");
    assert!(error.contains("additional dirty files omitted"), "{error}");
    assert_eq!(
        error
            .lines()
            .filter(|line| line.starts_with("dirty-") || *line == "file.txt")
            .count(),
        20,
        "{error}"
    );
    assert!(!error.contains("dirty-20.txt"), "{error}");
    assert_eq!(
        registry,
        std::fs::read(fixture.service.registry_path()).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&before).unwrap(),
        serde_json::to_value(
            storage::load_board(fixture.gcx.clone(), "task-1")
                .await
                .unwrap()
        )
        .unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(fixture.worktree.join("file.txt")).unwrap(),
        "uncommitted\n"
    );
    fixture.discard(&[("force", json!(true))]).await.unwrap();
    fixture.assert_removed("done", true).await;
}

#[tokio::test]
async fn failed_card_worktree_can_be_discarded() {
    let fixture = Fixture::new("failed").await;
    let session = fixture.live_session().await;
    fixture.discard(&[]).await.unwrap();
    assert!(session.lock().await.abort_flag.load(Ordering::SeqCst));
    assert_eq!(session.lock().await.runtime.state, SessionState::Idle);
    fixture.assert_removed("failed", true).await;
}

#[tokio::test]
async fn discard_refuses_unconfirmed_stop_without_touching_worktree() {
    let fixture = Fixture::new("failed").await;
    fixture.live_session().await;
    let before = storage::load_board(fixture.gcx.clone(), "task-1")
        .await
        .unwrap();
    let registry = std::fs::read(fixture.service.registry_path()).unwrap();
    let mut mock = MockChatFacade::new(Some(SessionState::ExecutingTools));
    mock.ignore_abort = true;
    let error = fixture
        .discard_with_facade(&[("force", json!(true))], Arc::new(mock))
        .await
        .unwrap_err();
    assert!(error.contains("stop not confirmed"), "{error}");
    assert!(error.contains("still running after 10 s"), "{error}");
    assert!(fixture.worktree.exists());
    assert_eq!(
        registry,
        std::fs::read(fixture.service.registry_path()).unwrap()
    );
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(
            storage::load_board(fixture.gcx.clone(), "task-1")
                .await
                .unwrap()
        )
        .unwrap()
    );
}

#[tokio::test]
async fn discard_refuses_cards_without_worktrees_and_nonterminal_cards() {
    let root = tempfile::tempdir().unwrap();
    for column in ["done", "failed", "planned"] {
        let gcx = write_task(root.path(), test_card(column, None, None)).await;
        let ccx = planner_ccx(gcx, Arc::new(MockChatFacade::new(None)), "planner").await;
        let error = ToolCancelAgent::new()
            .tool_execute(
                ccx,
                &"discard".into(),
                &args(&[
                    ("card_id", json!("T-39")),
                    ("discard_worktree", json!(true)),
                ]),
            )
            .await
            .unwrap_err();
        assert!(
            error.contains(if column == "planned" {
                "expected done or failed"
            } else {
                "no worktree"
            }),
            "{error}"
        );
    }
}

#[tokio::test]
async fn discard_refuses_unregistered_worktree_without_raw_git_cleanup() {
    let fixture = Fixture::new("done").await;
    let mut registry = fixture.service.load_registry().await.unwrap();
    registry.records.clear();
    fixture.service.save_registry(&registry).await.unwrap();
    let error = fixture.discard(&[]).await.unwrap_err();
    assert!(
        error.contains("not registered with the worktree registry"),
        "{error}"
    );
    assert!(
        error.contains("cannot be discarded through cancel_agent"),
        "{error}"
    );
    assert!(
        error.contains(fixture.worktree.to_str().unwrap()),
        "{error}"
    );
    assert!(!error.contains("git "), "{error}");
    assert!(fixture.worktree.exists());
    let board = storage::load_board(fixture.gcx.clone(), "task-1")
        .await
        .unwrap();
    assert!(board.get_card("T-39").unwrap().agent_worktree.is_some());
}
