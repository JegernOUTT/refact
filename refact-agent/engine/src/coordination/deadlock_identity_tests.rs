use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::agents::spawn::{NotifyParent, SpawnRequest, SpawnWorktreeMode};
use crate::agents::types::BgAgentKind;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::chat::types::ChatSession;
use crate::subchat::SubchatResult;
use crate::tools::tool_wait::ToolWait;
use crate::tools::tools_description::Tool;
use refact_chat_api::{AskQuestion, QuestionKind};
use refact_work_core::{
    AdmissionRequest, CheckoutKey, CoordinationSettings, ProjectKey, Ticket, WorkKey, WorkScope,
};
use tokio::sync::{Mutex, Notify};

fn spawn_request(parent_chat_id: &str, title: &str, subchat_depth: usize) -> SpawnRequest {
    SpawnRequest {
        native_supervision_guard: None,
        kind: BgAgentKind::Subagent,
        parent_chat_id: parent_chat_id.to_string(),
        parent_root_chat_id: Some("wait-root".to_string()),
        parent_tool_call_id: None,
        config_name: "test_spawn".to_string(),
        title: title.to_string(),
        prompt: format!("Run {title}"),
        initial_context_messages: Vec::new(),
        history_source_model: None,
        custom_instructions: None,
        tools: None,
        target_files: Vec::new(),
        max_steps: 10,
        model: "openai/gpt-4o".to_string(),
        model_type: None,
        goal: None,
        plan: None,
        deliverable: None,
        profile: None,
        worktree_mode: SpawnWorktreeMode::Inherit,
        parent_subchat_tx: None,
        parent_worktree: None,
        parent_task_meta: None,
        subchat_depth,
        notify_parent: NotifyParent::Silent,
        completion_push: Default::default(),
    }
}

pub(super) fn parked_run(id: &str) -> refact_work_core::RunKey {
    WorkKey::background_agent(id).run(1)
}

pub(super) fn work_scope() -> WorkScope {
    WorkScope::new(
        ProjectKey::new("identity-test-project"),
        CheckoutKey::new("wait-root"),
    )
}

pub(super) async fn create_running_record(
    app: &crate::app_state::AppState,
    parent_chat_id: &str,
    child_chat_id: &str,
    agent_id: Option<&str>,
) -> String {
    let request = crate::agents::types::CreateAgentRequest {
        parent_chat_id: parent_chat_id.to_string(),
        parent_root_chat_id: Some("wait-root".to_string()),
        parent_tool_call_id: None,
        kind: BgAgentKind::Subagent,
        config_name: "test_spawn".to_string(),
        title: "Identity test".to_string(),
        prompt: "Identity test".to_string(),
        target_files: Vec::new(),
        model: "openai/gpt-4o".to_string(),
        model_type: None,
        goal_summary: None,
        plan_present: false,
        worktree_id: None,
        worktree_branch: None,
    };
    let created = match agent_id {
        Some(agent_id) => app.agents.create_with_id(request, agent_id).await,
        None => app.agents.create(request).await,
    };
    let (record, _, _) = created.unwrap();
    app.agents
        .mark_running(&record.agent_id, child_chat_id.to_string())
        .await
        .unwrap();
    record.agent_id
}

pub(super) async fn parked_ticket(
    app: &crate::app_state::AppState,
    run: refact_work_core::RunKey,
) -> Ticket {
    app.work_scheduler
        .reserve(AdmissionRequest::new(run, work_scope()))
        .unwrap()
        .acquire()
        .await
        .unwrap()
        .park()
        .unwrap()
}

pub(super) async fn app_with_workspace() -> (crate::app_state::AppState, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    let config = temp.path().join("config");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(cache, config).await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.clone()];
    let (project_root, _) = crate::caps::project_model_defaults_state(gcx.clone()).await;
    let mut caps = crate::caps::CodeAssistantCaps::default();
    caps.chat_models.insert(
        "openai/gpt-4o".into(),
        Arc::new(crate::caps::ChatModelRecord {
            base: crate::caps::BaseModelRecord {
                id: "openai/gpt-4o".into(),
                ..Default::default()
            },
            supports_tools: true,
            supports_agent: true,
            ..Default::default()
        }),
    );
    {
        let mut state = gcx.caps_state.write().await;
        state.caps = Some(Arc::new(caps));
        state.loaded_project_root = Some(project_root);
        state.last_attempted_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    app.chat.sessions.write().await.insert(
        "wait-root".to_string(),
        Arc::new(Mutex::new(ChatSession::new("wait-root".to_string()))),
    );
    (app, temp)
}

#[tokio::test]
#[serial_test::serial(test_runner, runtime_settings)]
async fn background_agent_waiting_on_its_own_child_is_tracked() {
    let (identity_tx, mut identity_rx) = tokio::sync::mpsc::unbounded_channel();
    let parent_started = Arc::new(Notify::new());
    let child_started = Arc::new(Notify::new());
    let release_child = Arc::new(Notify::new());
    let wait_invoked = Arc::new(Notify::new());
    let _runner = {
        let identity_tx = identity_tx.clone();
        let parent_started = parent_started.clone();
        let child_started = child_started.clone();
        let release_child = release_child.clone();
        let wait_invoked = wait_invoked.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |gcx, messages, config| {
            let identity_tx = identity_tx.clone();
            let parent_started = parent_started.clone();
            let child_started = child_started.clone();
            let release_child = release_child.clone();
            let wait_invoked = wait_invoked.clone();
            Box::pin(async move {
                let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
                if config.title.as_deref() == Some("Y") {
                    child_started.notify_one();
                    release_child.notified().await;
                    return Ok(SubchatResult {
                        messages,
                        metering: serde_json::Map::new(),
                        chat_id: config.chat_id,
                        aborted: false,
                    });
                }
                parent_started.notify_one();
                let parent_chat_id = config.chat_id.clone().expect("X child chat id");
                let child = crate::agents::spawn::spawn_background_agent(
                    app.clone(),
                    spawn_request(&parent_chat_id, "Y", config.subchat_depth),
                )
                .await
                .expect("spawn child Y");
                tokio::time::timeout(Duration::from_secs(30), child_started.notified())
                    .await
                    .expect("child Y starts running");
                let _ = identity_tx.send((
                    app.clone(),
                    config.background_agent_id.clone().unwrap(),
                    child.agent_id.clone(),
                ));
                let ccx = Arc::new(Mutex::new(
                    AtCommandsContext::new_from_app(
                        app,
                        4096,
                        20,
                        false,
                        Vec::new(),
                        parent_chat_id,
                        config.root_chat_id.clone(),
                        config.model.clone(),
                        None,
                        config.worktree.clone(),
                    )
                    .await,
                ));
                wait_invoked.notify_one();
                ToolWait {
                    config_path: String::new(),
                }
                .tool_execute(
                    ccx,
                    &"wait-y".to_string(),
                    &HashMap::from([
                        (
                            "target".to_string(),
                            serde_json::json!({"kind": "agents", "agent_ids": [child.agent_id]}),
                        ),
                        ("timeout_ms".to_string(), serde_json::json!(30_000)),
                    ]),
                )
                .await?;
                Ok(SubchatResult {
                    messages,
                    metering: serde_json::Map::new(),
                    chat_id: config.chat_id,
                    aborted: false,
                })
            })
        }))
    };
    let (app, _temp) = app_with_workspace().await;
    let handle = crate::agents::spawn::spawn_background_agent(
        app.clone(),
        spawn_request("wait-root", "X", 0),
    )
    .await
    .expect("spawn parent X");
    tokio::time::timeout(Duration::from_secs(30), parent_started.notified())
        .await
        .expect("X runner starts");
    let (graph_app, waiter, target) =
        tokio::time::timeout(Duration::from_secs(30), identity_rx.recv())
            .await
            .expect("X spawns child Y")
            .expect("wait identity notification");
    tokio::time::timeout(Duration::from_secs(30), wait_invoked.notified())
        .await
        .expect("X invokes agent_wait");
    let edge_tracked = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let present = graph_app.wait_graph.edges().iter().any(|edge| {
                edge.from.work.id == waiter
                    && edge.to.run().is_some_and(|run| run.work.id == target)
            });
            if present {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok();
    release_child.notify_one();
    let _ = tokio::time::timeout(Duration::from_secs(30), handle.completion_rx)
        .await
        .expect("parent X completes")
        .expect("parent X result");

    assert!(
        edge_tracked,
        "agent_wait must declare the background-agent X → Y edge"
    );
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn background_agent_cycle_is_detected_and_refused() {
    for (x_id, y_id) in [
        (
            "bgagent-00000000-0000-0000-0000-000000000001",
            "bgagent-00000000-0000-0000-0000-000000000002",
        ),
        (
            "bgagent-00000000-0000-0000-0000-000000000002",
            "bgagent-00000000-0000-0000-0000-000000000001",
        ),
    ] {
        let (mut app, _temp) = app_with_workspace().await;
        app.work_scheduler = Arc::new(refact_work_core::AdmissionScheduler::new(
            CoordinationSettings {
                max_active_engine: 4,
                max_active_project: 4,
                max_active_root: 4,
                ..CoordinationSettings::default()
            },
        ));
        let x_id = create_running_record(&app, "wait-root", "subchat-cycle-x", Some(x_id)).await;
        let y_id =
            create_running_record(&app, "subchat-cycle-x", "subchat-cycle-y", Some(y_id)).await;
        let x_chat = "subchat-cycle-x";
        let _x_ticket = parked_ticket(&app, parked_run(&x_id)).await;
        let _y_ticket = parked_ticket(&app, parked_run(&y_id)).await;
        app.questions()
            .ask(
                AskQuestion::new(
                    QuestionKind::AgentParent,
                    &y_id,
                    x_chat,
                    "cycle-question",
                    "May I continue?",
                ),
                chrono::Utc::now(),
            )
            .unwrap();
        let ccx = Arc::new(Mutex::new(
            AtCommandsContext::new_from_app(
                app.clone(),
                4096,
                20,
                false,
                Vec::new(),
                x_chat.to_string(),
                Some("wait-root".to_string()),
                "openai/gpt-4o".to_string(),
                None,
                None,
            )
            .await,
        ));
        let waited_agent_id = y_id.clone();
        let tool = tokio::spawn(async move {
            ToolWait {
                config_path: String::new(),
            }
            .tool_execute(
                ccx,
                &"cycle-wait".to_string(),
                &HashMap::from([
                    (
                        "target".to_string(),
                        serde_json::json!({"kind": "agents", "agent_ids": [waited_agent_id]}),
                    ),
                    ("timeout_ms".to_string(), serde_json::json!(30_000)),
                ]),
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if app.wait_graph.edges().iter().any(|edge| {
                    edge.from.work.id == x_id
                        && edge.to.run().is_some_and(|run| run.work.id == y_id)
                }) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("X declares its wait on Y");

        let report = match crate::coordination::deadlock::scan(&app) {
            refact_work_core::Detection::Deadlock(report) => report,
            other => panic!("expected a deadlock, got {other:?}"),
        };
        assert!(report.involves(&parked_run(&x_id)));
        assert!(report.involves(&parked_run(&y_id)));
        let error = tool.await.unwrap().unwrap_err();
        assert!(
            error.starts_with(refact_work_core::DEADLOCK_CODE),
            "{error}"
        );
        assert!(error.contains(&x_id), "{error}");
        assert!(error.contains(&y_id), "{error}");
    }
}

#[tokio::test]
#[serial_test::serial(runtime_settings)]
async fn await_any_with_unauthorized_id_declares_no_edge() {
    let (mut app, _temp) = app_with_workspace().await;
    app.work_scheduler = Arc::new(refact_work_core::AdmissionScheduler::new(
        CoordinationSettings {
            max_active_engine: 4,
            max_active_project: 4,
            max_active_root: 4,
            ..CoordinationSettings::default()
        },
    ));
    let caller = WorkKey::trajectory("wait-root").run(1);
    let _caller_ticket = app
        .work_scheduler
        .reserve(AdmissionRequest::new(caller, work_scope()))
        .unwrap()
        .acquire()
        .await
        .unwrap();
    let own_id = create_running_record(&app, "wait-root", "subchat-own", None).await;
    let foreign_id = create_running_record(&app, "foreign-root", "subchat-foreign", None).await;
    let _own_ticket = parked_ticket(&app, parked_run(&own_id)).await;
    let _foreign_ticket = parked_ticket(&app, parked_run(&foreign_id)).await;
    let ccx = Arc::new(Mutex::new(
        AtCommandsContext::new_from_app(
            app.clone(),
            4096,
            20,
            false,
            Vec::new(),
            "wait-root".to_string(),
            None,
            "openai/gpt-4o".to_string(),
            None,
            None,
        )
        .await,
    ));

    let error = ToolWait {
        config_path: String::new(),
    }
    .tool_execute(
        ccx,
        &"unauthorized-wait".to_string(),
        &HashMap::from([
            (
                "target".to_string(),
                serde_json::json!({"kind": "agents", "agent_ids": [own_id, foreign_id]}),
            ),
            ("timeout_ms".to_string(), serde_json::json!(1000)),
        ]),
    )
    .await
    .unwrap_err();

    assert!(error.contains("bgagent-"), "{error}");
    assert!(app.wait_graph.edges().is_empty());
}
