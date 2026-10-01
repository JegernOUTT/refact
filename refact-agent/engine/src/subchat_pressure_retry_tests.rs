use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use indexmap::IndexMap;
use refact_privacy::PolicyLoad;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::app_state::AppState;
use crate::call_validation::{ChatMessage, ReasoningEffort};
use crate::caps::{BaseModelRecord, ChatModelRecord, CodeAssistantCaps};
use crate::chat::types::ThreadParams;
use crate::llm::{WireFormat, params::CacheControl, pressure::install_provider_pressure};
use refact_providers::pressure::uninstall_pressure_registry;
use refact_work_core::CoordinationSettings;

async fn serve_status(
    listener: tokio::net::TcpListener,
    status: u16,
    max_requests: usize,
) -> (Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let server_hits = hits.clone();
    let task = tokio::spawn(async move {
        for _ in 0..max_requests {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let count = socket.read(&mut chunk).await.unwrap();
                assert_ne!(count, 0, "connection closed before request headers arrived");
                request.extend_from_slice(&chunk[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            server_hits.fetch_add(1, Ordering::SeqCst);
            let (label, body) = match status {
                401 => ("Unauthorized", r#"{"error":{"message":"invalid api key"}}"#),
                _ => (
                    "Service Unavailable",
                    r#"{"error":{"message":"temporarily unavailable"}}"#,
                ),
            };
            let response = format!(
                "HTTP/1.1 {status} {label}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (hits, task)
}

async fn run_subchat(status: u16, pressure_attempts: Option<usize>) -> (usize, String) {
    uninstall_pressure_registry();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (hits, server) = serve_status(listener, status, 8).await;

    if let Some(max_attempts) = pressure_attempts {
        let mut settings = CoordinationSettings::default();
        settings.provider_pressure_enabled = true;
        settings.provider_retry_max_attempts = max_attempts;
        settings.provider_retry_base_backoff_ms = 1;
        settings.provider_retry_max_backoff_ms = 1;
        settings.provider_retry_jitter_per_mille = 0;
        settings.provider_max_retry_after_ms = 10;
        settings.provider_breaker_failure_threshold = 20;
        install_provider_pressure(&settings).unwrap();
    }

    let gcx = crate::global_context::tests::make_test_gcx().await;
    let app = AppState::from_gcx(gcx.clone()).await;
    *gcx.privacy_policy_load.write().unwrap() = PolicyLoad::default();
    let model_id = "test/subchat-pressure-retry";
    let model = Arc::new(ChatModelRecord {
        base: BaseModelRecord {
            id: model_id.to_string(),
            name: "subchat-pressure-retry".to_string(),
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

    let messages = vec![ChatMessage::new("user".to_string(), "hello".to_string())];
    let ccx = Arc::new(tokio::sync::Mutex::new(
        crate::at_commands::at_commands::AtCommandsContext::new_from_app(
            app,
            8192,
            1,
            false,
            messages.clone(),
            "subchat-pressure-test".to_string(),
            None,
            model_id.to_string(),
            None,
            None,
        )
        .await,
    ));
    let result = crate::subchat::subchat_stream(
        ccx,
        model_id,
        "agent",
        messages,
        vec![],
        false,
        None,
        512,
        Option::<ReasoningEffort>::None,
        CacheControl::Ephemeral,
        false,
        None,
        true,
    )
    .await;
    server.abort();
    let _ = server.await;
    let error = result.unwrap_err();
    let count = hits.load(Ordering::SeqCst);
    uninstall_pressure_registry();
    (count, error)
}

#[tokio::test]
#[serial_test::serial(provider_pressure, coordination_metrics)]
async fn subchat_401_is_sent_once() {
    let (count, error) = run_subchat(401, Some(2)).await;

    assert_eq!(count, 1);
    assert!(error.contains("401"), "{error}");
}

#[tokio::test]
#[serial_test::serial(provider_pressure, coordination_metrics)]
async fn subchat_persistent_503_stops_at_pressure_attempt_limit() {
    let (count, error) = run_subchat(503, Some(2)).await;

    assert_eq!(count, 3);
    assert!(error.contains("not retried: HTTP 503"), "{error}");
}

#[tokio::test(start_paused = true)]
#[serial_test::serial(provider_pressure, coordination_metrics)]
async fn subchat_503_without_registry_keeps_legacy_retry_count() {
    let (count, error) = run_subchat(503, None).await;

    assert_eq!(count, 6);
    assert!(error.contains("503"), "{error}");
}
