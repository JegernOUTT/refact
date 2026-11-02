use super::*;
use super::tests::{app_with_privacy_policy, model_record, privacy_policy};
use std::sync::atomic::Ordering;
use std::time::Instant as StdInstant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{watch, Notify};

#[derive(Clone, Copy)]
enum ResponseKind {
    Success,
    RateLimited,
}

async fn read_request_headers(socket: &mut tokio::net::TcpStream) {
    let mut request = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let count = socket.read(&mut chunk).await.unwrap();
        assert_ne!(count, 0, "connection closed before request headers arrived");
        request.extend_from_slice(&chunk[..count]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return;
        }
    }
}

async fn write_response(
    socket: &mut tokio::net::TcpStream,
    kind: ResponseKind,
    release: Option<&mut watch::Receiver<bool>>,
    headers_seen: Option<&Notify>,
) {
    match kind {
        ResponseKind::Success => {
            let body = b"data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(headers.as_bytes()).await.unwrap();
            if let Some(headers_seen) = headers_seen {
                headers_seen.notify_one();
            }
            if let Some(release) = release {
                while !*release.borrow() {
                    release.changed().await.unwrap();
                }
            }
            socket.write_all(body).await.unwrap();
        }
        ResponseKind::RateLimited => {
            let body = r#"{"error":{"message":"rate limit"}}"#;
            let headers = format!(
                "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 2\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(headers.as_bytes()).await.unwrap();
        }
    }
}

fn stream_params(
    model: &BaseModelRecord,
    abort: Option<(Arc<AtomicBool>, Arc<Notify>)>,
) -> StreamRunParams {
    let (abort_flag, abort_notify) = abort
        .map(|(flag, notify)| (Some(flag), Some(notify)))
        .unwrap_or((None, None));
    StreamRunParams {
        recovery_budget: None,
        llm_request: LlmRequest::new(
            model.id.clone(),
            vec![crate::call_validation::ChatMessage::new(
                "user".to_string(),
                "hello".to_string(),
            )],
        ),
        model_rec: model.clone(),
        chat_id: None,
        allow_websocket: false,
        abort_flag,
        abort_notify,
        supports_tools: false,
        supports_reasoning: false,
        reasoning_type: None,
        supports_temperature: true,
        priority: RequestPriority::Interactive,
    }
}

async fn wait_for_deferred_admission() {
    let deadline = StdInstant::now() + Duration::from_secs(20);
    loop {
        if crate::coordination::metrics::recorder().snapshot().counter(
            crate::coordination::metrics::PERMITS,
            "kind",
            "deferred",
        ) > 0
        {
            return;
        }
        assert!(
            StdInstant::now() < deadline,
            "stream never deferred in admission"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn install_single_slot_pressure() {
    let mut settings = refact_work_core::CoordinationSettings::default();
    settings.provider_initial_concurrency = 1;
    settings.provider_min_concurrency = 1;
    settings.provider_max_concurrency = 1;
    settings.provider_interactive_reserved_slots = 0;
    crate::llm::pressure::install_provider_pressure(&settings).unwrap();
}

#[tokio::test]
#[serial_test::serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn immediate_admission_does_not_start_recovery_deadline() {
    refact_providers::pressure::uninstall_pressure_registry();
    install_single_slot_pressure();
    let budget = Arc::new(refact_core::recovery::RecoveryBudget::new(
        Default::default(),
    ));
    let permit = crate::llm::pressure::acquire_for_recovery(
        "deadline-start",
        RequestPriority::Interactive,
        None,
        None,
        &budget,
    )
    .await
    .unwrap();
    assert_eq!(budget.deadline_remaining(), Duration::MAX);
    assert_eq!(budget.sends_used(), 0);
    drop(permit);
    refact_providers::pressure::uninstall_pressure_registry();
}

#[tokio::test]
#[serial_test::serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn second_stream_reaches_the_server_only_after_the_first_permit_is_released() {
    refact_providers::pressure::uninstall_pressure_registry();
    crate::coordination::metrics::recorder().reset();
    install_single_slot_pressure();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let first_seen = Arc::new(Notify::new());
    let server_hits = hits.clone();
    let first_headers_seen = first_seen.clone();
    let (release_tx, release_rx) = watch::channel(false);
    let server = tokio::spawn(async move {
        for index in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let server_hits = server_hits.clone();
            let first_headers_seen = first_headers_seen.clone();
            let mut release = release_rx.clone();
            tokio::spawn(async move {
                read_request_headers(&mut socket).await;
                server_hits.fetch_add(1, Ordering::SeqCst);
                write_response(
                    &mut socket,
                    ResponseKind::Success,
                    (index == 0).then_some(&mut release),
                    (index == 0).then_some(first_headers_seen.as_ref()),
                )
                .await;
            });
        }
    });

    let mut model = model_record("pressure-concurrency");
    model.api_key = "shared-test-credential".to_string();
    model.endpoint = format!("http://{address}/v1/chat/completions");
    let bucket = super::provider_pressure_bucket(&model);
    let app = app_with_privacy_policy(privacy_policy("pressure-concurrency")).await;
    let first_app = app.clone();
    let first_model = model.clone();
    let first = tokio::spawn(async move {
        super::run_llm_stream(
            first_app,
            stream_params(&first_model, None),
            &mut ReplayCollector::default(),
        )
        .await
    });
    first_seen.notified().await;

    let second_app = app.clone();
    let second_model = model.clone();
    let second = tokio::spawn(async move {
        super::run_llm_stream(
            second_app,
            stream_params(&second_model, None),
            &mut ReplayCollector::default(),
        )
        .await
    });
    wait_for_deferred_admission().await;
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        refact_providers::pressure::pressure_registry()
            .unwrap()
            .snapshot(&bucket)
            .unwrap()
            .in_flight,
        1
    );

    release_tx.send_replace(true);
    assert!(first.await.unwrap().is_ok());
    assert!(second.await.unwrap().is_ok());
    server.await.unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(
        refact_providers::pressure::pressure_registry()
            .unwrap()
            .snapshot(&bucket)
            .unwrap()
            .in_flight,
        0
    );
    refact_providers::pressure::uninstall_pressure_registry();
}

#[tokio::test]
#[serial_test::serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn abort_while_waiting_for_admission_never_reaches_the_server() {
    refact_providers::pressure::uninstall_pressure_registry();
    crate::coordination::metrics::recorder().reset();
    install_single_slot_pressure();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server_hits = hits.clone();
    let server = tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            read_request_headers(&mut socket).await;
            server_hits.fetch_add(1, Ordering::SeqCst);
        }
    });
    let mut model = model_record("pressure-abort");
    model.endpoint = format!("http://{address}/v1/chat/completions");
    let bucket = super::provider_pressure_bucket(&model);
    let held = crate::llm::pressure::admit(&bucket, RequestPriority::Interactive)
        .unwrap()
        .unwrap();
    let flag = Arc::new(AtomicBool::new(false));
    let notify = Arc::new(Notify::new());
    let app = app_with_privacy_policy(privacy_policy("pressure-abort")).await;
    let stream_flag = flag.clone();
    let stream_notify = notify.clone();
    let task = tokio::spawn(async move {
        super::run_llm_stream(
            app,
            stream_params(&model, Some((stream_flag, stream_notify))),
            &mut ReplayCollector::default(),
        )
        .await
    });
    wait_for_deferred_admission().await;
    flag.store(true, Ordering::SeqCst);
    notify.notify_one();
    let error = match task.await.unwrap() {
        Ok(_) => panic!("aborted stream unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.message, super::ABORT_ERROR_MESSAGE);
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    assert_eq!(
        refact_providers::pressure::pressure_registry()
            .unwrap()
            .snapshot(&bucket)
            .unwrap()
            .in_flight,
        1
    );
    drop(held);
    assert_eq!(
        refact_providers::pressure::pressure_registry()
            .unwrap()
            .snapshot(&bucket)
            .unwrap()
            .in_flight,
        0
    );
    server.abort();
    refact_providers::pressure::uninstall_pressure_registry();
}

#[tokio::test]
#[serial_test::serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn retry_after_header_sets_the_bucket_cooldown_window() {
    refact_providers::pressure::uninstall_pressure_registry();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request_headers(&mut socket).await;
        write_response(&mut socket, ResponseKind::RateLimited, None, None).await;
    });
    let mut settings = refact_work_core::CoordinationSettings::default();
    settings.provider_initial_concurrency = 4;
    settings.provider_max_concurrency = 4;
    settings.provider_retry_max_backoff_ms = 1_000;
    settings.provider_max_retry_after_ms = 5_000;
    settings.provider_breaker_failure_threshold = 8;
    crate::llm::pressure::install_provider_pressure(&settings).unwrap();
    let mut model = model_record("pressure-retry-after");
    model.api_key = "retry-after-credential".to_string();
    model.endpoint = format!("http://{address}/v1/chat/completions");
    let bucket = super::provider_pressure_bucket(&model);
    let before = refact_providers::pressure::pressure_registry()
        .unwrap()
        .now_ms();
    let app = app_with_privacy_policy(privacy_policy("pressure-retry-after")).await;
    let result = super::run_llm_stream(
        app,
        stream_params(&model, None),
        &mut ReplayCollector::default(),
    )
    .await;
    let error = match result {
        Ok(_) => panic!("rate-limited stream unexpectedly succeeded"),
        Err(error) => error,
    };
    let after = refact_providers::pressure::pressure_registry()
        .unwrap()
        .now_ms();
    server.await.unwrap();

    let (reported_bucket, report) = error.pressure_failure.unwrap();
    assert_eq!(reported_bucket, bucket);
    assert_eq!(
        report.class,
        refact_providers::pressure::PressureClass::RateLimit
    );
    let until = refact_providers::pressure::pressure_registry()
        .unwrap()
        .snapshot(&bucket)
        .unwrap()
        .retry_after_until_ms
        .unwrap();
    assert!(before + 2_000 <= until);
    assert!(until <= after + 2_000);
    refact_providers::pressure::uninstall_pressure_registry();
}

#[tokio::test]
#[serial_test::serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn retry_after_longer_than_the_admission_wait_bound_fails_fast_without_sending() {
    refact_providers::pressure::uninstall_pressure_registry();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server_hits = hits.clone();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request_headers(&mut socket).await;
        server_hits.fetch_add(1, Ordering::SeqCst);
        write_response(&mut socket, ResponseKind::RateLimited, None, None).await;
    });
    let mut settings = refact_work_core::CoordinationSettings::default();
    settings.provider_initial_concurrency = 4;
    settings.provider_max_concurrency = 4;
    settings.provider_retry_max_backoff_ms = 1;
    settings.provider_retry_base_backoff_ms = 1;
    settings.provider_max_retry_after_ms = 5_000;
    settings.provider_breaker_failure_threshold = 8;
    settings.validate().unwrap();
    crate::llm::pressure::install_provider_pressure(&settings).unwrap();
    let mut model = model_record("pressure-retry-after-bound");
    model.api_key = "retry-after-bound-credential".to_string();
    model.endpoint = format!("http://{address}/v1/chat/completions");
    let bucket = super::provider_pressure_bucket(&model);
    let app = app_with_privacy_policy(privacy_policy("pressure-retry-after-bound")).await;
    let first_result = super::run_llm_stream(
        app.clone(),
        stream_params(&model, None),
        &mut ReplayCollector::default(),
    )
    .await;
    let first_error = match first_result {
        Ok(_) => panic!("rate-limited stream unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(
        first_error.pressure_failure.unwrap().1.class,
        refact_providers::pressure::PressureClass::RateLimit
    );
    server.await.unwrap();
    let until = refact_providers::pressure::pressure_registry()
        .unwrap()
        .snapshot(&bucket)
        .unwrap()
        .retry_after_until_ms
        .unwrap();
    let before = refact_providers::pressure::pressure_registry()
        .unwrap()
        .now_ms();
    let mut second_params = stream_params(&model, None);
    second_params.recovery_budget = Some(Arc::new(refact_core::recovery::RecoveryBudget::new(
        Default::default(),
    )));
    let second_result =
        super::run_llm_stream(app, second_params, &mut ReplayCollector::default()).await;
    let second_error = match second_result {
        Ok(_) => panic!("cooling-down stream unexpectedly succeeded"),
        Err(error) => error,
    };
    assert!(second_error.is_cooldown());
    assert_eq!(second_error.retry_not_before_ms(), Some(until));
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert!(before < until);
    assert!(
        refact_providers::pressure::pressure_registry()
            .unwrap()
            .now_ms()
            < until
    );
    refact_providers::pressure::uninstall_pressure_registry();
}
