use super::*;

use crate::app_state::AppState;
use crate::caps::{BaseModelRecord, ChatModelRecord, CodeAssistantCaps};
use crate::chat::types::{ChatCommand, CommandRequest};
use crate::coordination::types::WorkStateTag;
use crate::llm::WireFormat;
use refact_providers::pressure::{pressure_registry, uninstall_pressure_registry, RequestPriority};
use refact_work_core::CoordinationSettings;
use std::sync::atomic::AtomicUsize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Fixture {
    app: AppState,
    executor: Arc<SubchatExecutor>,
    workspace: tempfile::TempDir,
    bucket: String,
    calls: Arc<AtomicUsize>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
        uninstall_pressure_registry();
    }
}

async fn fixture() -> Fixture {
    uninstall_pressure_registry();
    crate::coordination::metrics::recorder().reset();
    crate::llm::pressure::install_provider_pressure(&CoordinationSettings {
        provider_initial_concurrency: 1,
        provider_min_concurrency: 1,
        provider_max_concurrency: 1,
        provider_interactive_reserved_slots: 0,
        ..Default::default()
    })
    .unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let workspace = tempfile::tempdir().unwrap();
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.path().into()];
    *gcx.privacy_policy_load.write().unwrap() = refact_privacy::PolicyLoad::default();
    let app = AppState::from_gcx(gcx.clone()).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let server_calls = calls.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut byte = [0; 1];
            if socket.read(&mut byte).await.unwrap() > 0 {
                server_calls.fetch_add(1, Ordering::SeqCst);
                let _ = socket.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            }
        }
    });
    let endpoint = format!("http://{address}/v1/chat/completions");
    let model = Arc::new(ChatModelRecord {
        base: BaseModelRecord {
            id: "test/review-capacity".into(),
            name: "review-capacity".into(),
            endpoint: endpoint.clone(),
            wire_format: WireFormat::OpenaiChatCompletions,
            tokenizer: "fake".into(),
            n_ctx: 100000,
            ..Default::default()
        },
        supports_tools: false,
        supports_temperature: false,
        ..Default::default()
    });
    let mut caps = CodeAssistantCaps::default();
    caps.chat_models.insert(model.base.id.clone(), model);
    {
        let mut state = app.model.caps.write().await;
        state.caps = Some(Arc::new(caps));
        state.last_attempted_ts = now_ms() / 1000;
    }
    let executor = Arc::new(SubchatExecutor::new(
        gcx,
        super::runner::tests::ctx_with_worktree(None),
    ));
    Fixture {
        app,
        executor,
        workspace,
        bucket: refact_providers::pressure::bucket_key("test", &endpoint),
        calls,
        server,
    }
}

async fn run_blocked_stage(fixture: &Fixture, queue_command: bool) -> StageProduct {
    run_blocked_stage_until(fixture, queue_command, false).await
}

async fn run_blocked_stage_until(
    fixture: &Fixture,
    queue_command: bool,
    cancelled: bool,
) -> StageProduct {
    let mut job = super::tests::job("mechanical", "mechanical");
    job.subchat.model = "test/review-capacity".into();
    job.subchat.params.subchat_model = job.subchat.model.clone();
    let trace = job.trace_chat_id.clone();
    let executor = fixture.executor.clone();
    let parent_abort = fixture.executor.ctx.abort_flag.clone();
    let review = tokio::spawn(async move {
        run_watched_stage(
            executor,
            job,
            Duration::from_secs(120),
            Duration::from_millis(10),
            Duration::from_millis(100),
            Some(parent_abort),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(30), async {
        while crate::coordination::metrics::recorder().snapshot().counter(
            crate::coordination::metrics::PERMITS,
            "kind",
            "deferred",
        ) == 0
        {
            assert!(
                !review.is_finished(),
                "stage exited before provider capacity wait"
            );
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("stage reached the provider-capacity wait");
    assert!(fixture.app.work_scheduler.snapshot().tickets.is_empty());
    if queue_command {
        let session = fixture.app.chat.sessions.read().await[&trace].clone();
        let mut session = session.lock().await;
        session.work_project_root = Some(fixture.workspace.path().into());
        session.enqueue_command(CommandRequest {
            client_request_id: "pending-review-generation".into(),
            command: ChatCommand::Regenerate {},
            priority: false,
            native_approval_guard: None,
            supervision_guard: None,
        });
    }
    if cancelled {
        fixture
            .executor
            .ctx
            .abort_flag
            .store(true, Ordering::SeqCst);
    }
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(121)).await;
    let product = review.await.unwrap();
    tokio::time::resume();
    assert!(fixture.executor.forwarders.lock().await.is_empty());
    product
}

#[tokio::test]
#[serial_test::serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn stage_waiting_for_capacity_reports_the_wait_not_stopped_responding() {
    let fixture = fixture().await;
    let _held = crate::llm::pressure::admit(&fixture.bucket, RequestPriority::Interactive)
        .unwrap()
        .unwrap();
    let product = run_blocked_stage(&fixture, false).await;
    let reason = product.run.reason.unwrap();
    assert!(
        reason.contains("waiting for provider capacity (test)"),
        "{reason}"
    );
    assert!(!reason.contains("stopped responding"), "{reason}");
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
#[serial_test::serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn abandoned_stage_leaves_no_queued_or_live_work() {
    let fixture = fixture().await;
    let _held = crate::llm::pressure::admit(&fixture.bucket, RequestPriority::Interactive)
        .unwrap()
        .unwrap();
    let product = run_blocked_stage(&fixture, true).await;
    let trace = product.run.trace_chat_id.unwrap();
    assert!(fixture
        .app
        .work_scheduler
        .snapshot()
        .tickets
        .iter()
        .all(|ticket| ticket.run.work.id != trace));
    let catalog = crate::coordination::catalog::build_catalog(
        &fixture.app,
        Some(fixture.workspace.path().to_str().unwrap()),
    )
    .await
    .unwrap();
    if let Some(row) = catalog.find(&trace) {
        assert_eq!(row.state, WorkStateTag::Cancelled);
    }
    let sessions = fixture.app.chat.sessions.read().await;
    if let Some(session) = sessions.get(&trace) {
        let session = session.lock().await;
        assert!(session.command_queue.is_empty());
    }
}

#[tokio::test]
#[serial_test::serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn abandoned_stage_never_starts_later() {
    let fixture = fixture().await;
    let held = crate::llm::pressure::admit(&fixture.bucket, RequestPriority::Interactive)
        .unwrap()
        .unwrap();
    let product = run_blocked_stage(&fixture, true).await;
    drop(held);
    let cycle =
        crate::llm::pressure::acquire(&fixture.bucket, RequestPriority::Background, None, None)
            .await
            .unwrap()
            .unwrap();
    drop(cycle);
    let trace = product.run.trace_chat_id.unwrap();
    let session = fixture.app.chat.sessions.read().await[&trace].clone();
    let processor = session.lock().await.queue_processor_running.clone();
    tokio::time::timeout(
        Duration::from_secs(10),
        crate::chat::process_command_queue(fixture.app.clone(), session, processor),
    )
    .await
    .expect("closed review stage cannot dispatch a queued generation");
    assert_eq!(
        pressure_registry()
            .unwrap()
            .snapshot(&fixture.bucket)
            .unwrap()
            .in_flight,
        0
    );
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
#[serial_test::serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn parent_cancel_settles_a_stage_waiting_for_provider_capacity() {
    let fixture = fixture().await;
    let held = crate::llm::pressure::admit(&fixture.bucket, RequestPriority::Interactive)
        .unwrap()
        .unwrap();
    let product = run_blocked_stage_until(&fixture, true, true).await;
    let trace = product.run.trace_chat_id.unwrap();
    let session = fixture.app.chat.sessions.read().await[&trace].clone();
    let session = session.lock().await;
    assert!(session.closed);
    assert!(session.command_queue.is_empty());
    drop(held);
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
}

struct PanickingStage {
    inner: Arc<SubchatExecutor>,
}

#[async_trait]
impl StageExecutor for PanickingStage {
    async fn execute(&self, job: StageJob, _: Arc<AtomicU64>) -> StageProduct {
        let mut session = crate::chat::types::ChatSession::new(job.trace_chat_id.clone());
        session.work_project_root = Some(
            self.inner
                .gcx
                .documents_state
                .workspace_folders
                .lock()
                .unwrap()[0]
                .clone(),
        );
        session.thread.link_type = Some("review_stage".into());
        session.runtime.state = crate::chat::types::SessionState::Generating;
        self.inner.gcx.chat_sessions.write().await.insert(
            job.trace_chat_id,
            Arc::new(tokio::sync::Mutex::new(session)),
        );
        panic!("stage panicked after installing its session");
    }

    async fn finish(&self, trace: &str, run: &StageRun) {
        self.inner.finish(trace, run).await;
    }
}

#[tokio::test]
#[serial_test::serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn panicking_stage_session_is_terminal_before_review_returns() {
    let fixture = fixture().await;
    let products = run_stage_jobs(
        Arc::new(PanickingStage {
            inner: fixture.executor.clone(),
        }),
        vec![super::tests::job("mechanical", "mechanical")],
        super::tests::params(1),
    )
    .await;
    assert_eq!(products[0].run.reason.as_deref(), Some("stage panicked"));
    let catalog = crate::coordination::catalog::build_catalog(&fixture.app, None)
        .await
        .unwrap();
    assert_eq!(
        catalog.find("subchat-rv-1-mechanical").unwrap().state,
        WorkStateTag::Cancelled
    );
}
