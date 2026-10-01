use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serial_test::serial;
use tokio::sync::{Mutex, Notify};

use crate::agents::spawn::{SpawnRequest, SpawnWorktreeMode};
use crate::agents::types::{BgAgentKind, BgAgentStatus};
use crate::app_state::AppState;
use crate::call_validation::ChatMessage;
use crate::subchat::{SubchatConfig, SubchatResult};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    app: AppState,
}

async fn fixture(parent_id: &str, hooks_enabled: bool) -> Fixture {
    let temp = tempfile::tempdir().expect("fixture tempdir");
    let root = temp.path().join("workspace");
    let cache = temp.path().join("cache");
    let config = temp.path().join("config");
    std::fs::create_dir_all(&root).expect("workspace");
    let mut gcx = crate::global_context::tests::make_test_gcx_with_dirs(cache, config).await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
    if !hooks_enabled {
        Arc::get_mut(&mut gcx).unwrap().hooks_config.enabled = false;
    }
    let (project_root, _) = crate::caps::project_model_defaults_state(gcx.clone()).await;
    {
        let mut caps = crate::caps::CodeAssistantCaps::default();
        caps.chat_models.insert(
            "model".into(),
            Arc::new(crate::caps::ChatModelRecord {
                base: crate::caps::BaseModelRecord {
                    id: "model".into(),
                    ..Default::default()
                },
                supports_tools: true,
                supports_agent: true,
                ..Default::default()
            }),
        );
        let mut state = gcx.caps_state.write().await;
        state.caps = Some(Arc::new(caps));
        state.loaded_project_root = Some(project_root);
        state.last_attempted_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
    let app = AppState::from_gcx(gcx).await;
    let mut parent = crate::chat::types::ChatSession::new(parent_id.to_string());
    parent.work_project_root = Some(root.clone());
    parent.thread.model = "model".into();
    app.chat
        .sessions
        .write()
        .await
        .insert(parent_id.to_string(), Arc::new(Mutex::new(parent)));
    Fixture {
        _temp: temp,
        root,
        app,
    }
}

fn request(parent_id: &str) -> SpawnRequest {
    SpawnRequest {
        native_supervision_guard: None,
        kind: BgAgentKind::Subagent,
        parent_chat_id: parent_id.into(),
        parent_root_chat_id: Some(parent_id.into()),
        parent_tool_call_id: None,
        config_name: "test_spawn".into(),
        title: "Subagent hook lifecycle".into(),
        prompt: "Exercise subagent hooks".into(),
        initial_context_messages: Vec::new(),
        history_source_model: None,
        custom_instructions: None,
        tools: None,
        target_files: Vec::new(),
        max_steps: 4,
        model: "model".into(),
        model_type: None,
        goal: None,
        plan: None,
        deliverable: None,
        profile: None,
        worktree_mode: SpawnWorktreeMode::Inherit,
        parent_subchat_tx: None,
        parent_worktree: None,
        parent_task_meta: None,
        subchat_depth: 0,
        notify_parent: crate::agents::spawn::NotifyParent::Silent,
        completion_push: Default::default(),
    }
}

async fn install_hooks(fixture: &Fixture, marker: &std::path::Path) {
    let command = format!(
        "cat >> '{}'; echo >> '{}'",
        marker.display(),
        marker.display()
    );
    std::fs::write(
        fixture.app.paths.config_dir.join("hooks.yaml"),
        format!(
            "hooks:\n  SubagentStart:\n    - hooks:\n        - type: command\n          command: {command:?}\n  SubagentStop:\n    - hooks:\n        - type: command\n          command: {command:?}\n"
        ),
    )
    .unwrap();
    crate::ext::hooks_runner::clear_hook_cache_for_tests();
}

async fn complete_run(
    gcx: Arc<crate::global_context::GlobalContext>,
    mut messages: Vec<ChatMessage>,
    config: SubchatConfig,
    answer: &str,
) -> SubchatResult {
    let app = AppState::from_gcx(gcx.clone()).await;
    let chat_id = config.chat_id.clone().expect("child chat id");
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(&chat_id)
        .cloned()
        .expect("child session");
    let thread = session.lock().await.thread.clone();
    messages.push(ChatMessage::new("assistant".into(), answer.into()));
    crate::chat::trajectories::save_trajectory_as(gcx, &thread, &messages).await;
    crate::subchat::mirror_subchat_messages_into_session(&app, &chat_id, &messages).await;
    crate::subchat::finish_stateful_subchat_session(&app, &chat_id, &config).await;
    SubchatResult {
        messages,
        metering: serde_json::Map::new(),
        chat_id: Some(chat_id),
        aborted: false,
    }
}

async fn spawn_completed(
    fixture: &Fixture,
    parent_id: &str,
) -> crate::agents::types::BackgroundAgent {
    let handle =
        crate::agents::spawn::spawn_background_agent(fixture.app.clone(), request(parent_id))
            .await
            .expect("initial child accepted");
    tokio::time::timeout(Duration::from_secs(30), handle.completion_rx)
        .await
        .expect("initial completion timeout")
        .expect("initial completion record")
}

fn hook_payloads(marker: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(marker)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("hook payload json"))
        .collect()
}

#[cfg(unix)]
#[tokio::test]
#[serial(test_runner, permission_decision_hooks)]
async fn fresh_spawn_completes_with_one_start_then_one_stop() {
    let fixture = fixture("subagent-hook-fresh", true).await;
    let marker = fixture._temp.path().join("hooks.jsonl");
    install_hooks(&fixture, &marker).await;
    let _runner = crate::agents::spawn::install_test_runner(Arc::new(|gcx, messages, config| {
        Box::pin(async move { Ok(complete_run(gcx, messages, config, "finished").await) })
    }));

    let completed = spawn_completed(&fixture, "subagent-hook-fresh").await;
    let payloads = hook_payloads(&marker);

    assert_eq!(payloads.len(), 2);
    assert_eq!(payloads[0]["hook_event_name"], "SubagentStart");
    assert_eq!(payloads[0]["agent_id"], completed.agent_id);
    assert_eq!(payloads[0]["run_generation"], 1);
    assert_eq!(payloads[0]["parent_chat_id"], "subagent-hook-fresh");
    assert_eq!(
        payloads[0]["child_chat_id"],
        completed.child_chat_id.as_deref().unwrap()
    );
    assert!(payloads[0]["profile"].is_null());
    assert_eq!(payloads[0]["resumed"], false);
    assert_eq!(payloads[1]["hook_event_name"], "SubagentStop");
    assert_eq!(payloads[1]["agent_id"], completed.agent_id);
    assert_eq!(payloads[1]["run_generation"], 1);
    assert_eq!(payloads[1]["parent_chat_id"], "subagent-hook-fresh");
    assert_eq!(
        payloads[1]["child_chat_id"],
        completed.child_chat_id.as_deref().unwrap()
    );
    assert_eq!(payloads[1]["status"], "completed");
    assert_eq!(payloads[1]["steps_used"], completed.step_count);
}

#[cfg(unix)]
#[tokio::test]
#[serial(test_runner, permission_decision_hooks)]
async fn cancelled_child_emits_one_cancelled_stop() {
    let fixture = fixture("subagent-hook-cancelled", true).await;
    let marker = fixture._temp.path().join("hooks.jsonl");
    install_hooks(&fixture, &marker).await;
    let started = Arc::new(Notify::new());
    let _runner = {
        let started = started.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, _messages, config| {
            let started = started.clone();
            Box::pin(async move {
                started.notify_one();
                loop {
                    if config
                        .abort_flag
                        .as_ref()
                        .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire))
                    {
                        return Err("Aborted by test".into());
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
        }))
    };

    let handle = crate::agents::spawn::spawn_background_agent(
        fixture.app.clone(),
        request("subagent-hook-cancelled"),
    )
    .await
    .expect("child accepted");
    tokio::time::timeout(Duration::from_secs(10), started.notified())
        .await
        .expect("child runner started");
    fixture
        .app
        .agents
        .cancel(
            "subagent-hook-cancelled",
            &handle.agent_id,
            Some("cancel test".into()),
        )
        .await
        .unwrap();
    let cancelled = tokio::time::timeout(Duration::from_secs(30), handle.completion_rx)
        .await
        .expect("cancel completion timeout")
        .unwrap();
    let payloads = hook_payloads(&marker);

    assert_eq!(cancelled.status, BgAgentStatus::Cancelled);
    assert_eq!(payloads.len(), 2);
    assert_eq!(payloads[0]["hook_event_name"], "SubagentStart");
    assert_eq!(payloads[1]["hook_event_name"], "SubagentStop");
    assert_eq!(payloads[1]["status"], "cancelled");
}

#[cfg(unix)]
#[tokio::test]
#[serial(test_runner, permission_decision_hooks)]
async fn resumed_child_emits_start_with_resumed_true_and_generation_two() {
    let fixture = fixture("subagent-hook-resumed", true).await;
    let child_chat_id;
    let marker = fixture._temp.path().join("hooks.jsonl");
    install_hooks(&fixture, &marker).await;
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let _runner = {
        let attempts = attempts.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |gcx, messages, config| {
            let attempt = attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async move {
                Ok(complete_run(
                    gcx,
                    messages,
                    config,
                    if attempt == 0 {
                        "first answer"
                    } else {
                        "resumed answer"
                    },
                )
                .await)
            })
        }))
    };
    let first = spawn_completed(&fixture, "subagent-hook-resumed").await;
    assert_eq!(first.status, BgAgentStatus::Completed);
    let first = fixture.app.agents.get_any(&first.agent_id).await.unwrap();
    child_chat_id = first.child_chat_id.clone().unwrap();
    std::fs::write(&marker, "").unwrap();

    let (start, receive) = tokio::sync::oneshot::channel();
    let _queued = crate::agents::spawn::resume_agent_inner(
        fixture.app.clone(),
        "subagent-hook-resumed",
        &first.agent_id,
        None,
        first.change_seq,
        None,
        false,
        receive,
    )
    .await
    .expect("resume accepted");
    start.send(()).unwrap();
    let resumed = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let record = fixture.app.agents.get_any(&first.agent_id).await.unwrap();
            if record.status.is_terminal()
                && record.run_generation == 2
                && hook_payloads(&marker).len() == 2
            {
                break record;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("resumed generation terminal timeout");
    let payloads = hook_payloads(&marker);

    assert_eq!(resumed.status, BgAgentStatus::Completed);
    assert_eq!(payloads.len(), 2);
    assert_eq!(payloads[0]["hook_event_name"], "SubagentStart");
    assert_eq!(payloads[0]["resumed"], true);
    assert_eq!(payloads[0]["run_generation"], 2);
    assert_eq!(payloads[0]["agent_id"], first.agent_id);
    assert_eq!(payloads[1]["hook_event_name"], "SubagentStop");
    assert_eq!(payloads[1]["run_generation"], 2);
    assert_eq!(payloads[1]["status"], "completed");
}

#[cfg(unix)]
#[tokio::test]
#[serial(test_runner, permission_decision_hooks)]
async fn repeated_terminal_update_emits_one_stop_for_the_generation() {
    let fixture = fixture("subagent-hook-repeat", true).await;
    let marker = fixture._temp.path().join("hooks.jsonl");
    install_hooks(&fixture, &marker).await;
    let _runner = crate::agents::spawn::install_test_runner(Arc::new(|gcx, messages, config| {
        Box::pin(async move { Ok(complete_run(gcx, messages, config, "finished").await) })
    }));

    let completed = spawn_completed(&fixture, "subagent-hook-repeat").await;
    crate::agents::spawn::emit_background_agent_update(fixture.app.clone(), &completed).await;
    crate::agents::spawn::emit_background_agent_update(fixture.app.clone(), &completed).await;
    let payloads = hook_payloads(&marker);

    assert_eq!(payloads.len(), 2);
    assert_eq!(
        payloads
            .iter()
            .filter(|p| p["hook_event_name"] == "SubagentStop")
            .count(),
        1
    );
}

#[cfg(unix)]
#[tokio::test]
#[serial(test_runner, permission_decision_hooks)]
async fn disabled_hooks_create_no_marker_file() {
    let fixture = fixture("subagent-hook-disabled", false).await;
    let marker = fixture._temp.path().join("hooks.jsonl");
    install_hooks(&fixture, &marker).await;
    let _runner = crate::agents::spawn::install_test_runner(Arc::new(|gcx, messages, config| {
        Box::pin(async move { Ok(complete_run(gcx, messages, config, "finished").await) })
    }));

    let completed = spawn_completed(&fixture, "subagent-hook-disabled").await;

    assert_eq!(completed.status, BgAgentStatus::Completed);
    assert!(!marker.exists());
}

#[tokio::test]
#[serial(test_runner, permission_decision_hooks)]
async fn stop_hook_claim_is_idempotent_per_generation() {
    let fixture = fixture("subagent-hook-claim", true).await;
    let record = fixture
        .app
        .agents
        .create(crate::agents::types::CreateAgentRequest {
            parent_chat_id: "subagent-hook-claim".into(),
            parent_root_chat_id: Some("subagent-hook-claim".into()),
            parent_tool_call_id: None,
            kind: BgAgentKind::Subagent,
            config_name: "test".into(),
            title: "claim".into(),
            prompt: "claim".into(),
            target_files: Vec::new(),
            model: "model".into(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
        })
        .await
        .unwrap()
        .0;

    assert!(
        fixture
            .app
            .agents
            .claim_stop_hook(&record.agent_id, 1)
            .await
    );
    assert!(
        !fixture
            .app
            .agents
            .claim_stop_hook(&record.agent_id, 1)
            .await
    );
    assert!(
        fixture
            .app
            .agents
            .claim_stop_hook(&record.agent_id, 2)
            .await
    );
}
