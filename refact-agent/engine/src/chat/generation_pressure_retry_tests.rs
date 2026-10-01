use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use indexmap::IndexMap;
use refact_privacy::PolicyLoad;
use serial_test::serial;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex as AMutex;

use super::generation::start_generation;
use super::stream_core::LlmStreamError;
use super::types::ChatSession;
use crate::app_state::AppState;
use crate::call_validation::{ChatContent, ChatMessage};
use crate::caps::{BaseModelRecord, ChatModelRecord, CodeAssistantCaps};
use crate::llm::{
    WireFormat,
    pressure::{PressureRetry, pressure_retry},
};
use crate::llm::pressure::install_provider_pressure;
use refact_providers::pressure::{FailureReport, uninstall_pressure_registry};
use refact_work_core::CoordinationSettings;

fn pressure_error(status: u16, retry_after: Option<&str>, body: &str) -> LlmStreamError {
    LlmStreamError {
        message: format!("HTTP {status}: {body}"),
        partial_output_emitted: false,
        pressure_failure: Some((
            "provider/credential".to_string(),
            FailureReport::from_http(status, retry_after, body).unwrap(),
        )),
    }
}

#[test]
#[serial(provider_pressure, coordination_metrics, runtime_settings)]
fn pressure_retry_honours_retry_after() {
    uninstall_pressure_registry();
    install_provider_pressure(&CoordinationSettings::default()).unwrap();

    assert!(matches!(
        pressure_retry(&pressure_error(429, Some("3"), "rate limit"), 0),
        Some(PressureRetry::Retry {
            delay_ms: 3_000..,
            honoured_retry_after: true,
            attempt: 1,
            max_attempts: 4,
            ..
        })
    ));
    uninstall_pressure_registry();
}

#[test]
#[serial(provider_pressure, coordination_metrics, runtime_settings)]
fn pressure_retry_gives_up_for_auth_failures() {
    uninstall_pressure_registry();
    install_provider_pressure(&CoordinationSettings::default()).unwrap();

    assert!(matches!(
        pressure_retry(&pressure_error(401, None, "invalid api key"), 0),
        Some(PressureRetry::GiveUp { cause }) if cause.contains("HTTP 401")
    ));
    uninstall_pressure_registry();
}

#[test]
#[serial(provider_pressure, coordination_metrics, runtime_settings)]
fn pressure_retry_without_pressure_failure_returns_none() {
    uninstall_pressure_registry();
    install_provider_pressure(&CoordinationSettings::default()).unwrap();
    let error = LlmStreamError {
        message: "network error".to_string(),
        partial_output_emitted: false,
        pressure_failure: None,
    };

    assert_eq!(pressure_retry(&error, 0), None);
    uninstall_pressure_registry();
}

#[test]
#[serial(provider_pressure, coordination_metrics, runtime_settings)]
fn pressure_retry_without_registry_returns_none() {
    uninstall_pressure_registry();

    assert_eq!(
        pressure_retry(&pressure_error(429, Some("3"), "rate limit"), 0),
        None
    );
    uninstall_pressure_registry();
}

#[test]
#[serial(provider_pressure, coordination_metrics, runtime_settings)]
fn generation_retry_gives_up_when_pressure_attempt_budget_is_exhausted() {
    uninstall_pressure_registry();
    let settings = CoordinationSettings {
        provider_retry_max_attempts: 2,
        ..CoordinationSettings::default()
    };
    install_provider_pressure(&settings).unwrap();
    let error = pressure_error(503, None, "temporarily unavailable");

    assert!(matches!(
        pressure_retry(&error, 2),
        Some(PressureRetry::GiveUp { cause }) if cause.contains("HTTP 503")
    ));
    uninstall_pressure_registry();
}

async fn serve_statuses(
    listener: tokio::net::TcpListener,
    statuses: Vec<(u16, Option<&'static str>, &'static str)>,
) -> (Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let server_hits = hits.clone();
    let task = tokio::spawn(async move {
        for (status, retry_after, body) in statuses {
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
) -> (usize, String, Vec<ChatMessage>) {
    uninstall_pressure_registry();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (hits, server) = serve_statuses(listener, statuses).await;

    let mut settings = CoordinationSettings::default();
    settings.provider_pressure_enabled = pressure_enabled;
    settings.provider_retry_max_attempts = max_pressure_retries;
    settings.provider_retry_base_backoff_ms = 1;
    settings.provider_retry_max_backoff_ms = 1;
    settings.provider_retry_jitter_per_mille = 0;
    settings.provider_max_retry_after_ms = 10_000;
    settings.provider_breaker_failure_threshold = 20;
    install_provider_pressure(&settings).unwrap();

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

    start_generation(app.clone(), session_arc.clone()).await;
    server.abort();
    let _ = server.await;
    let session = session_arc.lock().await;
    let error = session
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "error" && message.extra.get("retry_status").is_none())
        .map(|message| message.content.content_text_only())
        .unwrap_or_else(|| session.runtime.error.clone().unwrap_or_default());
    let messages = session.messages.clone();
    let count = hits.load(Ordering::SeqCst);
    drop(session);
    uninstall_pressure_registry();
    (count, error, messages)
}

#[tokio::test]
#[serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn generation_401_is_sent_once() {
    let (count, error, _) = run_generation(
        vec![(401, None, r#"{"error":{"message":"invalid api key"}}"#)],
        2,
        true,
    )
    .await;

    assert_eq!(count, 1);
    assert!(error.contains("401"), "{error}");
}

#[tokio::test]
#[serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn generation_persistent_503_stops_at_pressure_attempt_limit() {
    let (count, error, messages) = run_generation(
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
    assert!(error.contains("not retried: HTTP 503"), "{error}");
    let retry_statuses = retry_statuses(&messages);
    assert_eq!(retry_statuses.len(), count - 1);
    assert_eq!(
        retry_statuses
            .iter()
            .map(|status| status["attempt"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert!(retry_statuses
        .iter()
        .all(|status| status["max_attempts"] == 2));
    assert!(messages
        .iter()
        .filter(|message| message.extra.get("retry_status").is_some())
        .all(|message| crate::chat::diagnostics::is_ui_only_message(message)));
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
    let (count, _, messages) = run_generation(
        vec![
            (429, Some("2"), r#"{"error":{"message":"rate limit"}}"#),
            (
                503,
                None,
                r#"{"error":{"message":"temporarily unavailable"}}"#,
            ),
        ],
        1,
        true,
    )
    .await;

    assert_eq!(count, 2);
    let retry_statuses = retry_statuses(&messages);
    assert_eq!(retry_statuses.len(), 1);
    assert_eq!(retry_statuses[0]["attempt"], 1);
    assert_eq!(retry_statuses[0]["max_attempts"], 1);
    assert_eq!(retry_statuses[0]["delay_secs"], 2);
}

#[tokio::test(start_paused = true)]
#[serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn generation_without_pressure_registry_uses_legacy_retry_status_budget() {
    let (count, _, messages) = run_generation(
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

    assert_eq!(count, 6);
    let retry_statuses = retry_statuses(&messages);
    assert_eq!(retry_statuses.len(), count - 1);
    assert_eq!(
        retry_statuses
            .iter()
            .map(|status| status["attempt"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
    assert!(retry_statuses.iter().all(|status| {
        status["max_attempts"] == crate::chat::retry_policy::MAX_LLM_RETRY_ATTEMPTS
    }));
}
