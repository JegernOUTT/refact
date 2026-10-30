use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use indexmap::IndexMap;
use refact_privacy::PolicyLoad;
use serial_test::serial;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex as AMutex;

use super::generation::start_generation;
use super::types::ChatSession;
use crate::app_state::AppState;
use crate::call_validation::{ChatContent, ChatMessage};
use crate::caps::{BaseModelRecord, ChatModelRecord, CodeAssistantCaps};
use crate::llm::WireFormat;
use crate::llm::pressure::install_provider_pressure;
use refact_providers::pressure::uninstall_pressure_registry;
use refact_work_core::CoordinationSettings;
use refact_core::recovery::{RecoveryMode, RecoveryPolicyOverrides, RecoveryRecord, RecoveryState};

const SAFETY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

// JoinHandle detaches on drop: keep abort handles so assertion failures cannot
// leave either generation or a loopback provider running in the test runtime.
#[derive(Default)]
struct TestTasks(Vec<tokio::task::AbortHandle>);

impl TestTasks {
    fn track<T>(&mut self, task: tokio::task::JoinHandle<T>) -> tokio::task::JoinHandle<T> {
        self.0.push(task.abort_handle());
        task
    }
}

impl Drop for TestTasks {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
        uninstall_pressure_registry();
    }
}

async fn read_provider_request(socket: &mut tokio::net::TcpStream) {
    let mut request = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let count = socket.read(&mut chunk).await.unwrap();
        assert_ne!(
            count, 0,
            "connection closed before complete request arrived"
        );
        request.extend_from_slice(&chunk[..count]);
        if let Some(header_end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&request[..header_end]).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .expect("fixture expects a fixed-length provider request");
            if request.len() >= header_end + 4 + length {
                break;
            }
        }
    }
}

pub(super) async fn serve_statuses(
    listener: tokio::net::TcpListener,
    statuses: Vec<(u16, Option<&'static str>, &'static str)>,
) -> (Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let server_hits = hits.clone();
    let task = tokio::spawn(async move {
        for (status, retry_after, body) in statuses {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_provider_request(&mut socket).await;
            server_hits.fetch_add(1, Ordering::SeqCst);
            let (label, content_type) = match status {
                200 => ("OK", "text/event-stream"),
                401 => ("Unauthorized", "application/json"),
                _ => ("Service Unavailable", "application/json"),
            };
            let retry = retry_after
                .map(|value| format!("Retry-After: {value}\r\n"))
                .unwrap_or_default();
            let response = format!(
                "HTTP/1.1 {status} {label}\r\nContent-Type: {content_type}\r\n{retry}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (hits, task)
}

async fn run_generation(
    statuses: Vec<(u16, Option<&'static str>, &'static str)>,
    max_pressure_retries: usize,
    pressure_enabled: bool,
) -> (usize, String, Vec<ChatMessage>, Option<RecoveryRecord>) {
    uninstall_pressure_registry();
    let mut tasks = TestTasks::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (hits, server) = serve_statuses(listener, statuses).await;
    let server = tasks.track(server);

    let mut settings = CoordinationSettings::default();
    settings.provider_pressure_enabled = pressure_enabled;
    settings.provider_retry_max_attempts = max_pressure_retries;
    settings.provider_retry_base_backoff_ms = 1;
    settings.provider_retry_max_backoff_ms = 1;
    settings.provider_retry_jitter_per_mille = 0;
    settings.provider_max_retry_after_ms = 10_000;
    settings.provider_breaker_failure_threshold = 20;
    install_provider_pressure(&settings).unwrap();

    let (app, session_arc) = generation_fixture(address).await;
    let task = tasks.track(tokio::spawn(start_generation(app, session_arc.clone())));
    tokio::time::timeout(SAFETY_TIMEOUT, task)
        .await
        .expect("generation must terminate")
        .unwrap();
    server.abort();
    let _ = server.await;
    let session = session_arc.lock().await;
    let error = session.runtime.error.clone().unwrap_or_default();
    let messages = session.messages.clone();
    let recovery = session.runtime.recovery.clone();
    let count = hits.load(Ordering::SeqCst);
    drop(session);
    uninstall_pressure_registry();
    (count, error, messages, recovery)
}

async fn generation_fixture(address: std::net::SocketAddr) -> (AppState, Arc<AMutex<ChatSession>>) {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let app = AppState::from_gcx(gcx.clone()).await;
    *gcx.privacy_policy_load.write().unwrap() = PolicyLoad::default();
    let model_id = "test/pressure-retry";
    let model = Arc::new(ChatModelRecord {
        base: BaseModelRecord {
            id: model_id.to_string(),
            name: "pressure-retry".to_string(),
            endpoint: format!("http://{address}/v1/chat/completions"),
            wire_format: WireFormat::OpenaiChatCompletions,
            tokenizer: "fake".to_string(),
            n_ctx: 8192,
            ..Default::default()
        },
        supports_tools: false,
        supports_temperature: false,
        ..Default::default()
    });
    let mut caps = CodeAssistantCaps::default();
    caps.chat_models = IndexMap::new();
    caps.chat_models.insert(model_id.to_string(), model);
    caps.defaults.chat_default_model = model_id.to_string();
    {
        let mut state = app.model.caps.write().await;
        state.caps = Some(Arc::new(caps));
        state.last_attempted_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
    let chat_id = "pressure-generation-test".to_string();
    let mut session = ChatSession::new(chat_id.clone());
    session.thread.model = model_id.to_string();
    session.thread.mode = "ask".to_string();
    session.thread.include_project_info = false;
    session.thread.recovery_policy = Some(RecoveryPolicyOverrides {
        mode: Some(RecoveryMode::Transient),
        max_retries: Some(2),
        delay_ms: Some(0),
        total_timeout_ms: Some(600_000),
    });
    session.add_message(ChatMessage {
        role: "system".to_string(),
        content: ChatContent::SimpleText("system".to_string()),
        ..Default::default()
    });
    session.add_message(ChatMessage {
        role: "user".to_string(),
        content: ChatContent::SimpleText("hello".to_string()),
        ..Default::default()
    });
    let session_arc = Arc::new(AMutex::new(session));
    app.chat
        .sessions
        .write()
        .await
        .insert(chat_id, session_arc.clone());

    (app, session_arc)
}

#[tokio::test]
#[serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn generation_401_is_sent_once() {
    let (count, _, _, recovery) = run_generation(
        vec![(401, None, r#"{"error":{"message":"invalid api key"}}"#)],
        2,
        true,
    )
    .await;

    assert_eq!(count, 1);
    let recovery = recovery.expect("401 must produce a recovery record");
    assert_eq!(recovery.state, RecoveryState::NeedsAction);
    assert!(!recovery.latest_error.is_retryable);
}

#[tokio::test]
#[serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn generation_persistent_503_stops_at_pressure_attempt_limit() {
    let (count, _, messages, recovery) = run_generation(
        vec![
            (
                503,
                None,
                r#"{"error":{"message":"temporarily unavailable"}}"#
            );
            8
        ],
        2,
        true,
    )
    .await;

    assert_eq!(count, 3);
    let recovery = recovery.expect("503 must produce a recovery record");
    assert_eq!(recovery.state, RecoveryState::Exhausted);
    assert_eq!(recovery.retries_used, 2);
    assert_eq!(recovery.attempts.len(), 3);
    assert!(retry_statuses(&messages).is_empty());
}

fn retry_statuses(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .filter_map(|message| message.extra.get("retry_status"))
        .cloned()
        .collect()
}

#[tokio::test]
#[serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn generation_retry_after_status_reports_honoured_delay() {
    uninstall_pressure_registry();
    let mut tasks = TestTasks::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (hits, server) = serve_statuses(
        listener,
        vec![(429, Some("60"), r#"{"error":{"message":"rate limit"}}"#)],
    )
    .await;
    let server = tasks.track(server);
    let (app, session) = generation_fixture(address).await;
    let task = tasks.track(tokio::spawn(start_generation(app, session.clone())));
    let record = tokio::time::timeout(SAFETY_TIMEOUT, async {
        loop {
            if let Some(record) = session.lock().await.runtime.recovery.clone() {
                if record.state == RecoveryState::Waiting {
                    break record;
                }
            }
            // Polling is bounded and cooperative; this delay is not the readiness oracle.
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("429 must enter waiting");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let not_before = record
        .retry_not_before_ms
        .expect("Retry-After must set a gate");
    assert!(not_before >= record.attempts[0].at_ms + 59_000);
    assert!(record.next_retry_at_ms.unwrap() >= not_before);
    session.lock().await.apply_priority_abort();
    tokio::time::timeout(SAFETY_TIMEOUT, task)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(SAFETY_TIMEOUT, server)
        .await
        .unwrap()
        .unwrap();
    uninstall_pressure_registry();
}

#[tokio::test]
#[serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn generation_without_pressure_registry_uses_unified_retry_budget() {
    let (count, _, messages, recovery) = run_generation(
        vec![
            (
                503,
                None,
                r#"{"error":{"message":"temporarily unavailable"}}"#
            );
            8
        ],
        2,
        false,
    )
    .await;

    assert_eq!(count, 3);
    let recovery = recovery.expect("503 must produce recovery without pressure registry");
    assert_eq!(recovery.state, RecoveryState::Exhausted);
    assert_eq!(recovery.retries_used, 2);
    assert!(retry_statuses(&messages).is_empty());
}

fn active_record() -> RecoveryRecord {
    use refact_core::recovery::{RecoveryError, RecoveryPolicy, RecoveryPolicySource};
    RecoveryRecord {
        operation_id: "operation-1".into(),
        run_id: "run-1".into(),
        revision: 4,
        state: RecoveryState::Attempting,
        policy: RecoveryPolicy::default(),
        policy_source: RecoveryPolicySource::Thread,
        manual: false,
        retries_used: 1,
        latest_error: RecoveryError {
            category: "network".into(),
            title: "Network".into(),
            explanation: "offline".into(),
            suggested_action: "retry".into(),
            is_retryable: true,
            raw_error: "Authorization: Bearer secret-token".into(),
        },
        next_retry_at_ms: Some(123),
        deadline_at_ms: Some(999),
        retry_not_before_ms: None,
        terminal_reason: None,
        attempts: vec![],
    }
}

#[tokio::test]
async fn populated_recovery_snapshot_load_interrupts_and_redacts() {
    let dir = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx().await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
    let mut session = ChatSession::new("recovery-reload".into());
    session.add_message(ChatMessage::new("user".into(), "continue".into()));
    session.runtime.recovery = Some(active_record());
    let snapshot = super::trajectories::trajectory_snapshot_from_session(&session);
    super::trajectories::save_trajectory_snapshot(gcx.clone(), snapshot)
        .await
        .unwrap();
    let path = super::trajectories::find_trajectory_path(gcx.clone(), "recovery-reload")
        .await
        .unwrap();
    assert!(
        path.starts_with(dir.path()),
        "persistence must use the isolated workspace"
    );
    let mut saved: serde_json::Value =
        serde_json::from_slice(&tokio::fs::read(&path).await.unwrap()).unwrap();
    assert_eq!(saved["recovery"]["operation_id"], "operation-1");
    assert_eq!(saved["recovery"]["state"], "attempting");
    // Old/external snapshots may contain secrets even if today's writer scrubs.
    // Inject on disk so the load-consumer oracle cannot pass via writer redaction.
    saved["recovery"]["latest_error"]["raw_error"] =
        serde_json::json!("Authorization: Bearer secret-token");
    saved["recovery"]["attempts"] = serde_json::json!([{
        "at_ms": 42, "retry_number": 1, "outcome": "failed",
        "error": saved["recovery"]["latest_error"].clone()
    }]);
    tokio::fs::write(&path, serde_json::to_vec(&saved).unwrap())
        .await
        .unwrap();
    let persisted = tokio::fs::read_to_string(&path).await.unwrap();
    assert!(
        persisted.contains("secret-token"),
        "load redaction needs an on-disk adversary"
    );
    let loaded = super::trajectories::load_trajectory_for_chat(gcx, "recovery-reload")
        .await
        .unwrap();
    let record = loaded.recovery.expect("saved recovery must load");
    assert_eq!(record.operation_id, "operation-1");
    assert_eq!(record.run_id, "run-1");
    assert_eq!(
        record.policy,
        refact_core::recovery::RecoveryPolicy::default()
    );
    assert_eq!(record.state, RecoveryState::NeedsAction);
    assert_eq!(record.revision, 5);
    assert_eq!(record.terminal_reason.as_deref(), Some("interrupted"));
    assert_eq!(record.next_retry_at_ms, None);
    assert!(!record.latest_error.raw_error.contains("secret-token"));
    assert_eq!(record.attempts.len(), 1);
    assert_eq!(record.attempts[0].at_ms, 42);
    assert!(!record.attempts[0]
        .error
        .as_ref()
        .unwrap()
        .raw_error
        .contains("secret-token"));
    assert!(record.holds_goal());

    // Exercise the shipping monitor consumer, not only RecoveryRecord::holds_goal.
    use super::goal_monitor::{
        try_apply_goal_nudge, GoalNudgeConfig, GoalNudgeOutcome, GoalNudgeReason, GoalNudgeSkip,
        GoalNudgeTrigger,
    };
    let mut restored = ChatSession::new("loaded-goal".into());
    restored.install_goal(
        "agent",
        "finish recovery",
        true,
        super::types::GoalBudget::default(),
    );
    restored.runtime.recovery = Some(record);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + 60_000;
    let nudge = |session: &mut ChatSession| {
        try_apply_goal_nudge(
            session,
            GoalNudgeTrigger::InlineTurnEnd,
            now_ms,
            std::time::Instant::now(),
            GoalNudgeConfig::default(),
            None,
        )
    };
    assert_eq!(
        nudge(&mut restored),
        GoalNudgeOutcome::Skipped(GoalNudgeSkip::WaitingForInput)
    );
    assert!(restored.command_queue.is_empty());
    restored.runtime.recovery = None;
    assert_eq!(
        nudge(&mut restored),
        GoalNudgeOutcome::Nudged(GoalNudgeReason::TurnEnd),
        "without recovery the same active goal must be eligible"
    );
}

#[tokio::test]
#[serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn deadline_cancels_pending_retry_stream() {
    use refact_core::recovery::{RecoveryBudget, RecoveryPolicy};
    uninstall_pressure_registry();
    let mut tasks = TestTasks::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
    let address = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let server_hits = hits.clone();
    let server = tasks.track(tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_provider_request(&mut socket).await;
        server_hits.fetch_add(1, Ordering::SeqCst);
        let mut bytes = [0; 8192];
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        // A valid SSE heartbeat, but no terminal chunk or [DONE]: the body stays pending.
        socket.write_all(b"3\r\n:\n\n\r\n").await.unwrap();
        ready_tx.send(()).unwrap();
        loop {
            match socket.read(&mut bytes).await {
                Ok(0) => break,
                Ok(_) => {},
                Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => break,
                Err(error) => panic!("unexpected provider socket error: {error}"),
            }
        }
        let _ = closed_tx.send(());
    }));
    let (app, session) = generation_fixture(address).await;
    let mut policy = RecoveryPolicy::default();
    policy.total_timeout_ms = 60_000;
    let budget = Arc::new(RecoveryBudget::new_manual(policy.clone()));
    {
        let mut s = session.lock().await;
        let mut record = active_record();
        record.policy = policy;
        s.runtime.recovery = Some(record);
        s.recovery_budget = Some(budget.clone());
    }
    let task = tasks.track(tokio::spawn(start_generation(app, session.clone())));
    tokio::time::timeout(SAFETY_TIMEOUT, ready_rx)
        .await
        .expect("provider readiness timed out")
        .expect("retry stream must reach provider before deadline");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(budget.sends_used(), 2);
    assert!(!task.is_finished());
    // Real time during the IO handshake prevents Tokio's idle auto-advance
    // from expiring the deadline before a provider request exists.
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(60)).await;
    tokio::time::resume();
    tokio::time::timeout(SAFETY_TIMEOUT, task)
        .await
        .expect("deadline must terminate generation")
        .unwrap();
    tokio::time::timeout(SAFETY_TIMEOUT, closed_rx)
        .await
        .expect("deadline must drop the pending HTTP body")
        .expect("provider must observe socket closure");
    let s = session.lock().await;
    let record = s.runtime.recovery.as_ref().expect("deadline recovery");
    assert_eq!(record.state, RecoveryState::Exhausted);
    assert_eq!(
        record.terminal_reason.as_deref(),
        Some("deadline_exhausted")
    );
    assert_eq!(budget.sends_used(), 2);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    drop(s);
    tokio::time::timeout(SAFETY_TIMEOUT, server)
        .await
        .unwrap()
        .unwrap();
}
