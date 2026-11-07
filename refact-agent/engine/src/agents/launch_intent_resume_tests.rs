use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use refact_work_core::identity::{ProjectKey, RunKey, WorkKey, WorkScope};
use refact_work_core::store::filesystem::FilesystemKind;
use refact_work_core::store::records::{
    now_ms, EngineInstanceRecord, IntentState, PresenceMark, RunIntentRecord,
};
use refact_work_core::store::CoordinationStore;
use refact_work_core::{AdmissionRequest, CheckoutKey, CoordinationSettings};
use serial_test::serial;
use tokio::sync::{Mutex, Notify};

use crate::agents::spawn::{SpawnRequest, SpawnWorktreeMode};
use crate::agents::types::{BgAgentKind, BgAgentStatus};
use crate::app_state::AppState;
use crate::call_validation::ChatMessage;
use crate::subchat::{SubchatConfig, SubchatResult};
use refact_agents_core::types::BackgroundAgent;

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    cache: PathBuf,
    app: AppState,
    store: Option<Arc<CoordinationStore>>,
    coordination: CoordinationSettings,
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

async fn fixture(parent_id: &str, coordination: CoordinationSettings) -> Fixture {
    let mut runtime = crate::runtime_settings::TrajectoryRuntimeSettings::default();
    runtime.coordination = coordination.clone();
    crate::runtime_settings::install_live(&runtime);
    refact_work_core::install_coordination_settings(coordination.clone()).unwrap();

    let temp = tempfile::tempdir().expect("fixture tempdir");
    let root = temp.path().join("workspace");
    let cache = temp.path().join("cache");
    let config = temp.path().join("config");
    std::fs::create_dir_all(&root).expect("workspace");
    let root = dunce::canonicalize(&root).expect("canonical workspace");
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
    let mut parent = crate::chat::types::ChatSession::new(parent_id.to_string());
    parent.work_project_root = Some(root.clone());
    parent.thread.model = "model".into();
    app.chat
        .sessions
        .write()
        .await
        .insert(parent_id.to_string(), Arc::new(Mutex::new(parent)));
    let store = if coordination.enabled {
        Some(Arc::new(
            CoordinationStore::open_on_filesystem(
                &refact_work_core::default_store_path(&cache),
                &coordination,
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
        coordination,
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
        title: "Resume launch intent".into(),
        prompt: "Exercise durable resume boundaries".into(),
        initial_context_messages: Vec::new(),
        history_source_model: None,
        custom_instructions: None,
        tools: None,
        target_files: Vec::new(),
        max_steps: 4,
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

async fn wait_for_registry_slot_to_clear(app: &AppState, child_chat_id: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while app.agent_executions.launch_intent(child_chat_id).is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("launch intent remained registered after settlement");
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
    .expect("resume intent did not reach expected state")
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

async fn spawn_completed(fixture: &Fixture, parent_id: &str) -> BackgroundAgent {
    let handle =
        crate::agents::spawn::spawn_background_agent(fixture.app.clone(), request(parent_id))
            .await
            .expect("initial child accepted");
    tokio::time::timeout(Duration::from_secs(30), handle.completion_rx)
        .await
        .expect("initial completion timeout")
        .expect("initial completion record")
}

async fn begin_resume(
    fixture: &Fixture,
    parent_id: &str,
    old: &BackgroundAgent,
) -> (BackgroundAgent, tokio::sync::oneshot::Sender<()>) {
    let (start, receive) = tokio::sync::oneshot::channel();
    let queued = crate::agents::spawn::resume_agent_inner(
        fixture.app.clone(),
        parent_id,
        &old.agent_id,
        None,
        old.change_seq,
        None,
        false,
        receive,
    )
    .await
    .expect("resume accepted");
    (queued, start)
}

#[serial(test_runner, runtime_settings)]
#[tokio::test]
async fn completed_resume_persists_new_generation_boundaries_without_mutating_generation_one() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let _reset = SettingsReset;
    let fixture = fixture("resume-boundaries", CoordinationSettings::default()).await;
    let attempts = Arc::new(AtomicUsize::new(0));
    let resumed_started = Arc::new(Notify::new());
    let resumed_release = Arc::new(Notify::new());
    let _runner = {
        let attempts = attempts.clone();
        let resumed_started = resumed_started.clone();
        let resumed_release = resumed_release.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |gcx, messages, config| {
            let attempt = attempts.fetch_add(1, Ordering::SeqCst);
            let resumed_started = resumed_started.clone();
            let resumed_release = resumed_release.clone();
            Box::pin(async move {
                if attempt == 1 {
                    resumed_started.notify_one();
                    resumed_release.notified().await;
                }
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
    let first = spawn_completed(&fixture, "resume-boundaries").await;
    assert_eq!(first.status, BgAgentStatus::Completed);
    let child_chat_id = first.child_chat_id.as_deref().unwrap();
    let run_one = RunKey::new(WorkKey::background_agent(&first.agent_id), 1);
    let before = intent_record(&fixture, &run_one).await;
    assert_eq!(before.state, IntentState::Terminal);
    assert_eq!(before.generation, 3);

    let (_, start) = begin_resume(&fixture, "resume-boundaries", &first).await;
    let session = fixture
        .app
        .chat
        .sessions
        .read()
        .await
        .get(child_chat_id)
        .cloned()
        .unwrap();
    let guard = session.lock().await;
    let run_two = RunKey::new(WorkKey::background_agent(&first.agent_id), 2);
    let persisted = wait_for_intent_state(&fixture, &run_two, IntentState::Persisted).await;
    assert_eq!(persisted.generation, 0);
    start.send(()).unwrap();
    let dispatched = wait_for_intent_state(&fixture, &run_two, IntentState::Dispatched).await;
    assert_eq!(dispatched.generation, 1);
    drop(guard);
    let acknowledged = wait_for_intent_state(&fixture, &run_two, IntentState::Acknowledged).await;
    assert_eq!(acknowledged.generation, 2);
    tokio::time::timeout(Duration::from_secs(30), resumed_started.notified())
        .await
        .expect("resumed runner started after acknowledgement");
    resumed_release.notify_one();
    let completed = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let record = fixture.app.agents.get_any(&first.agent_id).await.unwrap();
            if record.status.is_terminal() && record.run_generation == 2 {
                return record;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("resumed agent terminal timeout");
    assert_eq!(completed.status, BgAgentStatus::Completed);
    let terminal = wait_for_intent_state(&fixture, &run_two, IntentState::Terminal).await;
    assert_eq!(terminal.generation, 3);
    assert_eq!(intent_record(&fixture, &run_one).await, before);
    wait_for_registry_slot_to_clear(&fixture.app, child_chat_id).await;
    assert!(fixture
        .app
        .agent_executions
        .launch_intent(child_chat_id)
        .is_none());
}

#[serial(test_runner, runtime_settings)]
#[tokio::test]
async fn failed_run_config_persistence_settles_only_the_new_generation() {
    let _reset = SettingsReset;
    let fixture = fixture("resume-persist-failure", CoordinationSettings::default()).await;
    let _runner = crate::agents::spawn::install_test_runner(Arc::new(|gcx, messages, config| {
        Box::pin(async move { Ok(complete_run(gcx, messages, config, "first answer").await) })
    }));
    let first = spawn_completed(&fixture, "resume-persist-failure").await;
    let run_one = RunKey::new(WorkKey::background_agent(&first.agent_id), 1);
    let before = intent_record(&fixture, &run_one).await;
    let run_config = fixture
        .cache
        .join(".refact/background_agents/runs")
        .join(format!("{}.2.json", first.agent_id));
    std::fs::write(&run_config, "already present").unwrap();

    let (_, receive) = tokio::sync::oneshot::channel();
    let error = crate::agents::spawn::resume_agent_inner(
        fixture.app.clone(),
        "resume-persist-failure",
        &first.agent_id,
        None,
        first.change_seq,
        None,
        false,
        receive,
    )
    .await
    .unwrap_err();
    assert!(
        error.contains("accepted launch already exists or cannot be committed"),
        "{error}"
    );
    let run_two = RunKey::new(WorkKey::background_agent(&first.agent_id), 2);
    let terminal = wait_for_intent_state(&fixture, &run_two, IntentState::Terminal).await;
    assert_eq!(terminal.generation, 1);
    assert_eq!(intent_record(&fixture, &run_one).await, before);
    assert_eq!(
        fixture
            .app
            .agents
            .get_any(&first.agent_id)
            .await
            .unwrap()
            .status,
        BgAgentStatus::Failed
    );
}

#[serial(test_runner, runtime_settings)]
#[tokio::test]
async fn queued_resume_cancellation_settles_without_acknowledging() {
    let _reset = SettingsReset;
    let fixture = fixture("resume-cancel-queued", CoordinationSettings::default()).await;
    let _runner = crate::agents::spawn::install_test_runner(Arc::new(|gcx, messages, config| {
        Box::pin(async move { Ok(complete_run(gcx, messages, config, "first answer").await) })
    }));
    let first = spawn_completed(&fixture, "resume-cancel-queued").await;
    let run_one = RunKey::new(WorkKey::background_agent(&first.agent_id), 1);
    let before = intent_record(&fixture, &run_one).await;
    let mut limits = fixture.coordination.clone();
    limits.max_active_engine = 1;
    limits.max_active_project = 1;
    limits.max_active_root = 1;
    fixture.app.work_scheduler.configure(limits).unwrap();
    let blocker = fixture
        .app
        .work_scheduler
        .reserve(AdmissionRequest::new(
            WorkKey::background_agent("resume-blocker").run(1),
            WorkScope::new(project(&fixture.root), CheckoutKey::new("resume-blocker")),
        ))
        .unwrap()
        .acquire()
        .await
        .unwrap();
    let (queued, start) = begin_resume(&fixture, "resume-cancel-queued", &first).await;
    let run_two = RunKey::new(WorkKey::background_agent(&first.agent_id), 2);
    let persisted = intent_record(&fixture, &run_two).await;
    assert_eq!(persisted.state, IntentState::Persisted);
    assert_eq!(persisted.generation, 0);
    assert_eq!(queued.run_generation, 2);
    start.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while fixture.app.work_scheduler.snapshot().pending == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("resume ticket queued behind blocker");
    fixture
        .app
        .agents
        .cancel(
            "resume-cancel-queued",
            &first.agent_id,
            Some("cancel queued resume".into()),
        )
        .await
        .unwrap();
    drop(blocker);
    let cancelled = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let record = fixture.app.agents.get_any(&first.agent_id).await.unwrap();
            if record.status.is_terminal() && record.run_generation == 2 {
                return record;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled resumed record timeout");
    assert_eq!(cancelled.status, BgAgentStatus::Cancelled);
    let terminal = wait_for_intent_state(&fixture, &run_two, IntentState::Terminal).await;
    assert_eq!(terminal.generation, 1);
    assert_eq!(intent_record(&fixture, &run_one).await, before);
    assert!(fixture
        .app
        .agent_executions
        .launch_intent(first.child_chat_id.as_deref().unwrap())
        .is_none());
}

#[serial(test_runner, runtime_settings)]
#[tokio::test]
async fn dispatched_resume_crash_is_surfaced_by_real_reconciler_without_second_child() {
    use crate::coordination::presence::{EngineIdentity, EnginePresence};
    use crate::coordination::reconcile::{
        EvidenceProbe, InspectionReason, LaunchEvidence, Observation, ReconcileTrigger, Reconciler,
        ReconcilerSetup, StartAction,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    let _reset = SettingsReset;
    let mut coordination = CoordinationSettings::default();
    coordination.lease_ttl_ms = 500;
    coordination.lease_renew_interval_ms = 100;
    coordination.lease_takeover_grace_ms = 250;
    let fixture = fixture("resume-crash", coordination.clone()).await;
    let attempts = Arc::new(AtomicUsize::new(0));
    let _runner = {
        let attempts = attempts.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |gcx, messages, config| {
            let attempts = attempts.clone();
            Box::pin(async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                Ok(complete_run(gcx, messages, config, "first answer").await)
            })
        }))
    };
    let first = spawn_completed(&fixture, "resume-crash").await;
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    let (_, start) = begin_resume(&fixture, "resume-crash", &first).await;
    let session = fixture
        .app
        .chat
        .sessions
        .read()
        .await
        .get(first.child_chat_id.as_deref().unwrap())
        .cloned()
        .unwrap();
    let guard = session.lock().await;
    start.send(()).unwrap();
    let run_two = RunKey::new(WorkKey::background_agent(&first.agent_id), 2);
    let dispatched = wait_for_intent_state(&fixture, &run_two, IntentState::Dispatched).await;
    assert_eq!(dispatched.generation, 1);
    fixture
        .app
        .agent_executions
        .interrupt_tasks_for_test()
        .await;
    drop(guard);
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert_eq!(intent_record(&fixture, &run_two).await, dispatched);

    let store = fixture.store.as_ref().unwrap().clone();
    let project = project(&fixture.root);
    let engine_id = crate::coordination::engine_instance_id();
    store
        .put_engine_instance(
            &store.authorize(project.clone()),
            &EngineInstanceRecord {
                project: project.clone(),
                instance_id: engine_id,
                pid: 1,
                boot_id: String::new(),
                process_started_at_ms: -1,
                host_id: String::new(),
                heartbeat_at_ms: now_ms(),
                presence_mark: PresenceMark::Closed,
            },
        )
        .await
        .unwrap();
    let recovery_id = format!("recovery-{}", uuid::Uuid::new_v4());
    let recovering = EnginePresence::new(
        store.clone(),
        store.authorize(project.clone()),
        EngineIdentity::detect(recovery_id.clone()),
        Arc::new(|| false),
        &coordination,
    )
    .unwrap();
    recovering.refresh(now_ms()).await.unwrap();
    let starts = Arc::new(AtomicUsize::new(0));
    let counter = starts.clone();
    let start: StartAction = Arc::new(move |_, _| {
        counter.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    });
    let evidence: EvidenceProbe =
        Arc::new(|_| LaunchEvidence::new(Observation::No, Observation::No));
    let reconciler = Reconciler::new_surface_only(
        store,
        &ReconcilerSetup {
            project: project.clone(),
            instance_id: recovery_id,
            settings: coordination.clone(),
            evidence,
            start,
        },
    )
    .unwrap();
    let report = reconciler
        .reconcile(
            ReconcileTrigger::Startup,
            now_ms()
                + (coordination.lease_ttl_ms + coordination.lease_takeover_grace_ms + 1) as i64,
        )
        .await;
    assert!(report.started.is_empty());
    assert_eq!(starts.load(Ordering::SeqCst), 0);
    assert_eq!(report.needs_inspection.len(), 1);
    assert_eq!(report.needs_inspection[0].run, run_two);
    assert_eq!(
        report.needs_inspection[0].reason,
        InspectionReason::UnacknowledgedLaunch
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[serial(test_runner, runtime_settings)]
#[tokio::test]
async fn coordination_disabled_resume_keeps_runner_behavior_without_intent_store() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let _reset = SettingsReset;
    let coordination = CoordinationSettings {
        enabled: false,
        ..CoordinationSettings::default()
    };
    let fixture = fixture("resume-disabled", coordination).await;
    let attempts = Arc::new(AtomicUsize::new(0));
    let _runner = {
        let attempts = attempts.clone();
        crate::agents::spawn::install_test_runner(Arc::new(move |gcx, messages, config| {
            let attempt = attempts.fetch_add(1, Ordering::SeqCst);
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
    let first = spawn_completed(&fixture, "resume-disabled").await;
    let (_, start) = begin_resume(&fixture, "resume-disabled", &first).await;
    start.send(()).unwrap();
    let completed = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let record = fixture.app.agents.get_any(&first.agent_id).await.unwrap();
            if record.status.is_terminal() && record.run_generation == 2 {
                return record;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("coordination-disabled resume completion timeout");
    assert_eq!(completed.status, BgAgentStatus::Completed);
    assert_eq!(completed.result_summary.as_deref(), Some("resumed answer"));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert!(fixture.store.is_none());
    assert!(!refact_work_core::default_store_path(&fixture.cache).exists());
    wait_for_registry_slot_to_clear(&fixture.app, first.child_chat_id.as_deref().unwrap()).await;
    assert!(fixture
        .app
        .agent_executions
        .launch_intent(first.child_chat_id.as_deref().unwrap())
        .is_none());
}
