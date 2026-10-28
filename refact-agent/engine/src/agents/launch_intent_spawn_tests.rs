use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use refact_work_core::identity::{ProjectKey, RunKey, WorkKey, WorkScope};
use refact_work_core::store::filesystem::FilesystemKind;
use refact_work_core::store::records::{IntentState, RunIntentRecord};
use refact_work_core::store::CoordinationStore;
use refact_work_core::{AdmissionRequest, CheckoutKey};
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
    cache: PathBuf,
    app: AppState,
    store: Option<Arc<CoordinationStore>>,
}

async fn fixture(parent_id: &str, coordination: bool) -> Fixture {
    let temp = tempfile::tempdir().expect("fixture tempdir");
    let root = temp.path().join("workspace");
    let cache = temp.path().join("cache");
    let config = temp.path().join("config");
    std::fs::create_dir_all(&root).expect("workspace");
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(cache.clone(), config).await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
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
    app.chat.sessions.write().await.insert(
        parent_id.to_string(),
        Arc::new(Mutex::new(crate::chat::types::ChatSession::new(
            parent_id.to_string(),
        ))),
    );
    app.chat.sessions.read().await[parent_id]
        .lock()
        .await
        .work_project_root = Some(root.clone());
    let store = if coordination {
        let mut settings = crate::file_activity::settings();
        settings.enabled = true;
        Some(Arc::new(
            CoordinationStore::open_on_filesystem(
                &refact_work_core::default_store_path(&cache),
                &settings,
                FilesystemKind::Local,
            )
            .await
            .expect("coordination store"),
        ))
    } else {
        None
    };
    Fixture {
        _temp: temp,
        root,
        cache,
        app,
        store,
    }
}

struct SettingsReset;

impl Drop for SettingsReset {
    fn drop(&mut self) {
        crate::runtime_settings::reset_for_test();
        let _ = refact_work_core::install_coordination_settings(
            crate::runtime_settings::TrajectoryRuntimeSettings::default().coordination,
        );
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
        title: "Launch intent lifecycle".into(),
        prompt: "Exercise durable spawn boundaries".into(),
        initial_context_messages: Vec::new(),
        history_source_model: None,
        custom_instructions: None,
        tools: None,
        target_files: Vec::new(),
        max_steps: 2,
        model: "model".into(),
        model_type: None,
        context_tokens_cap: None,
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

fn completed_result(mut messages: Vec<ChatMessage>, config: SubchatConfig) -> SubchatResult {
    messages.push(ChatMessage::new("assistant".into(), "completed".into()));
    SubchatResult {
        messages,
        metering: serde_json::Map::new(),
        chat_id: config.chat_id,
        aborted: false,
    }
}

fn project(root: &Path) -> ProjectKey {
    ProjectKey::new(crate::coordination::roots::project_identity(root))
}

async fn intent_record(fixture: &Fixture, run: &RunKey) -> RunIntentRecord {
    let store = fixture.store.as_ref().expect("enabled coordination store");
    store
        .get_run_intent(&store.authorize(project(&fixture.root)), run)
        .await
        .unwrap()
        .unwrap()
}

async fn wait_for_intent_state(
    fixture: &Fixture,
    run: &RunKey,
    expected: IntentState,
) -> RunIntentRecord {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let record = intent_record(fixture, run).await;
            if record.state == expected {
                return record;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("launch intent did not reach expected state")
}

#[serial(test_runner, runtime_settings)]
#[tokio::test]
async fn completed_fresh_spawn_persists_each_launch_boundary_and_releases_chat_slot() {
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let _runner = {
        let started = started.clone();
        let release = release.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |gcx, messages, config| {
            let started = started.clone();
            let release = release.clone();
            Box::pin(async move {
                let app = AppState::from_gcx(gcx).await;
                let chat_id = config.chat_id.as_deref().expect("child chat id");
                let slot = app
                    .agent_executions
                    .launch_intent(chat_id)
                    .expect("registered launch intent");
                assert!(!slot.is_empty());
                started.notify_one();
                release.notified().await;
                Ok(completed_result(messages, config))
            })
        }))
    };
    let fixture = fixture("intent-complete", true).await;
    let mut limits = crate::file_activity::settings();
    limits.max_active_engine = 1;
    limits.max_active_project = 1;
    limits.max_active_root = 1;
    limits.max_outstanding = 2;
    limits.max_pending = 2;
    limits.max_parked_residency = 2;
    fixture.app.work_scheduler.configure(limits).unwrap();
    let blocker = fixture
        .app
        .work_scheduler
        .reserve(AdmissionRequest::new(
            WorkKey::background_agent("launch-blocker").run(1),
            WorkScope::new(project(&fixture.root), CheckoutKey::new("launch-blocker")),
        ))
        .unwrap()
        .acquire()
        .await
        .unwrap();
    let handle = crate::agents::spawn::spawn_background_agent(
        fixture.app.clone(),
        request("intent-complete"),
    )
    .await
    .expect("spawn accepted");
    let run = RunKey::new(WorkKey::background_agent(&handle.agent_id), 1);
    let persisted = intent_record(&fixture, &run).await;
    assert_eq!(persisted.state, IntentState::Persisted);
    assert_eq!(persisted.generation, 0);

    drop(blocker);
    let dispatched = wait_for_intent_state(&fixture, &run, IntentState::Dispatched).await;
    assert_eq!(dispatched.generation, 1);
    tokio::time::timeout(Duration::from_secs(10), started.notified())
        .await
        .expect("runner started after acknowledgement");
    let acknowledged = intent_record(&fixture, &run).await;
    assert_eq!(acknowledged.state, IntentState::Acknowledged);
    assert_eq!(acknowledged.generation, 2);
    release.notify_one();
    let completed = tokio::time::timeout(Duration::from_secs(30), handle.completion_rx)
        .await
        .expect("completion timeout")
        .expect("completion channel");
    assert_eq!(completed.status, BgAgentStatus::Completed);
    let terminal = wait_for_intent_state(&fixture, &run, IntentState::Terminal).await;
    assert_eq!(terminal.state, IntentState::Terminal);
    assert_eq!(terminal.generation, 3);
    assert!(fixture
        .app
        .agent_executions
        .launch_intent(&handle.child_chat_id)
        .is_none());
}

#[serial(test_runner, runtime_settings)]
#[tokio::test]
async fn admission_refusal_settles_persisted_intent_without_dispatch() {
    let fixture = fixture("intent-refusal", true).await;
    let mut limits = crate::file_activity::settings();
    limits.max_active_engine = 1;
    limits.max_active_project = 1;
    limits.max_active_root = 1;
    limits.max_outstanding = 1;
    limits.max_pending = 1;
    limits.max_parked_residency = 1;
    fixture.app.work_scheduler.configure(limits).unwrap();
    let _blocker = fixture
        .app
        .work_scheduler
        .reserve(AdmissionRequest::new(
            WorkKey::background_agent("capacity-blocker").run(1),
            WorkScope::new(project(&fixture.root), CheckoutKey::new("capacity-blocker")),
        ))
        .unwrap()
        .acquire()
        .await
        .unwrap();
    assert!(crate::agents::spawn::spawn_background_agent(
        fixture.app.clone(),
        request("intent-refusal"),
    )
    .await
    .is_err());
    let record = fixture
        .app
        .agents
        .list_for_parent("intent-refusal", Default::default())
        .await
        .pop()
        .expect("accepted agent record");
    assert!(record.child_chat_id.is_none());
    let run = RunKey::new(WorkKey::background_agent(&record.agent_id), 1);
    let terminal = intent_record(&fixture, &run).await;
    assert_eq!(terminal.state, IntentState::Terminal);
    assert_eq!(terminal.generation, 1);
}

#[serial(test_runner, runtime_settings)]
#[tokio::test]
async fn queued_child_cancellation_settles_without_acknowledging() {
    let _runner = crate::agents::spawn::install_test_runner(Arc::new(|_gcx, messages, config| {
        Box::pin(async move { Ok(completed_result(messages, config)) })
    }));
    let fixture = fixture("intent-cancel-queued", true).await;
    let mut limits = crate::file_activity::settings();
    limits.max_active_engine = 1;
    limits.max_active_project = 1;
    limits.max_active_root = 1;
    fixture.app.work_scheduler.configure(limits).unwrap();
    let blocker = fixture
        .app
        .work_scheduler
        .reserve(AdmissionRequest::new(
            WorkKey::background_agent("queue-blocker").run(1),
            WorkScope::new(project(&fixture.root), CheckoutKey::new("queue-blocker")),
        ))
        .unwrap()
        .acquire()
        .await
        .unwrap();
    let handle = crate::agents::spawn::spawn_background_agent(
        fixture.app.clone(),
        request("intent-cancel-queued"),
    )
    .await
    .unwrap();
    let run = RunKey::new(WorkKey::background_agent(&handle.agent_id), 1);
    assert_eq!(
        intent_record(&fixture, &run).await.state,
        IntentState::Persisted
    );
    fixture
        .app
        .agents
        .cancel(
            "intent-cancel-queued",
            &handle.agent_id,
            Some("cancel queued test".into()),
        )
        .await
        .unwrap();
    assert!(fixture.app.work_scheduler.cancel(&run));
    let cancelled = tokio::time::timeout(Duration::from_secs(10), handle.completion_rx)
        .await
        .expect("cancellation completion timeout")
        .unwrap();
    drop(blocker);
    assert_eq!(cancelled.status, BgAgentStatus::Cancelled);
    let terminal = wait_for_intent_state(&fixture, &run, IntentState::Terminal).await;
    assert_eq!(terminal.state, IntentState::Terminal);
    assert_eq!(terminal.generation, 1);
    assert!(fixture
        .app
        .agent_executions
        .launch_intent(&handle.child_chat_id)
        .is_none());
}

#[serial(test_runner, runtime_settings)]
#[tokio::test]
async fn coordination_disabled_spawn_keeps_lifecycle_and_creates_no_intent() {
    let _reset = SettingsReset;
    let runner_started = Arc::new(Notify::new());
    let started = runner_started.clone();
    let _runner =
        crate::agents::spawn::install_test_runner(Arc::new(move |_gcx, messages, config| {
            let started = started.clone();
            Box::pin(async move {
                started.notify_one();
                Ok(completed_result(messages, config))
            })
        }));
    crate::runtime_settings::reset_for_test();
    let mut settings = crate::runtime_settings::TrajectoryRuntimeSettings::default();
    settings.coordination.enabled = false;
    crate::runtime_settings::install_live(&settings);
    refact_work_core::install_coordination_settings(settings.coordination.clone()).unwrap();
    let fixture = fixture("intent-disabled", false).await;
    let handle = crate::agents::spawn::spawn_background_agent(
        fixture.app.clone(),
        request("intent-disabled"),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(10), runner_started.notified())
        .await
        .expect("disabled-coordination spawn still runs");
    let completed = tokio::time::timeout(Duration::from_secs(30), handle.completion_rx)
        .await
        .expect("completion timeout")
        .unwrap();
    assert_eq!(completed.status, BgAgentStatus::Completed);
    assert!(!refact_work_core::default_store_path(&fixture.cache).exists());
    assert!(fixture
        .app
        .agent_executions
        .launch_intent(&handle.child_chat_id)
        .is_none());
}
