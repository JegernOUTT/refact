use super::*;
use refact_core::claude_backend::{ClaudeBackend, ClaudeTransportConfig, CLAUDE_TRANSPORT_HEADER};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
#[serial_test::serial(provider_pressure, coordination_metrics, runtime_settings)]
async fn custom_bearer_full_request_omits_internal_config_and_subscription_mutations() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut buffer = [0u8; 4096];
            let read = socket.read(&mut buffer).await.unwrap();
            assert_ne!(read, 0);
            bytes.extend_from_slice(&buffer[..read]);
            if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                let length: usize = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:").map(str::trim))
                    .unwrap()
                    .parse()
                    .unwrap();
                if bytes.len() < end + 4 + length {
                    continue;
                }
                assert!(headers.contains("authorization: bearer custom-secret"));
                assert!(!headers.contains("x-refact-internal"));
                let body: Value = serde_json::from_slice(&bytes[end + 4..]).unwrap();
                assert!(!body.to_string().contains("custom-secret"));
                assert!(!body.to_string().contains("mcp_"));
                break;
            }
        }
        let body = "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\ndata: {\"type\":\"message_stop\"}\n\n";
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
    });
    use crate::providers::traits::ProviderTrait;
    let mut provider = crate::providers::claude_code::ClaudeCodeProvider::default();
    provider.provider_settings_apply(serde_yaml::from_str(&format!(
        "backend: custom\nauth_type: bearer\nauth_token: custom-secret\nbase_url: http://{address}\nenabled: true\nenabled_models: [claude]"
    )).unwrap()).unwrap();
    let runtime = provider.build_runtime().unwrap();
    let mut model = tests::model_record("claude_code");
    model.wire_format = WireFormat::AnthropicMessages;
    model.endpoint = runtime.chat_endpoint;
    model.extra_headers = runtime.extra_headers;
    let params = StreamRunParams {
        llm_request: LlmRequest::new(
            model.id.clone(),
            vec![crate::call_validation::ChatMessage::new(
                "user".into(),
                "hello".into(),
            )],
        ),
        model_rec: model,
        chat_id: None,
        allow_websocket: false,
        abort_flag: None,
        abort_notify: None,
        supports_tools: true,
        supports_reasoning: false,
        reasoning_type: None,
        supports_temperature: true,
        priority: RequestPriority::Interactive,
    };
    let app = tests::app_with_privacy_policy(tests::privacy_policy("claude_code")).await;
    let mut collector = ReplayCollector::default();
    match run_llm_stream(app, params, &mut collector).await.unwrap() {
        LlmStreamOutcome::Choices(choices) => assert_eq!(choices[0].content, "ok"),
        _ => panic!("unexpected cache guard pause"),
    }
    server.await.unwrap();
}

#[tokio::test]
async fn cloud_retry_rechecks_sealed_privacy_before_preparation() {
    let app = tests::app_with_privacy_policy(tests::privacy_policy("trusted")).await;
    let mut model = tests::model_record("untrusted");
    model.wire_format = WireFormat::AnthropicMessages;
    let params = StreamRunParams {
        llm_request: tests::request_with_secret(),
        model_rec: model.clone(),
        chat_id: None,
        allow_websocket: false,
        abort_flag: None,
        abort_notify: None,
        supports_tools: false,
        supports_reasoning: false,
        reasoning_type: None,
        supports_temperature: true,
        priority: RequestPriority::Interactive,
    };
    let prepared = PreparedLlmStreamRequest {
        client: reqwest::Client::new(),
        wire_format: model.wire_format,
        adapter: get_adapter(model.wire_format),
        adapter_settings: tests::adapter_settings(&model),
        http_parts: HttpParts {
            url: "http://127.0.0.1:1".into(),
            headers: Default::default(),
            body: json!({}),
        },
        command_credential: None,
        snapshot_for_commit: None,
        header_retry_config: Some((Duration::from_secs(1), 2)),
    };
    let config = ClaudeTransportConfig {
        backend: ClaudeBackend::Bedrock,
        ..Default::default()
    };
    let error = send_claude_cloud_request(&app, &params, &prepared, &config)
        .await
        .err()
        .unwrap();
    assert!(error.starts_with("Output withheld by user privacy policy"));
}

fn crc(bytes: &[u8]) -> u32 {
    let mut value = !0u32;
    for byte in bytes {
        value ^= *byte as u32;
        for _ in 0..8 {
            value = (value >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(value & 1));
        }
    }
    !value
}

fn frame(event: &str) -> Vec<u8> {
    use base64::Engine;
    let payload = serde_json::to_vec(
        &json!({"bytes": base64::engine::general_purpose::STANDARD.encode(event)}),
    )
    .unwrap();
    let mut headers = Vec::new();
    for (key, value) in [(":message-type", "event"), (":event-type", "chunk")] {
        headers.push(key.len() as u8);
        headers.extend(key.as_bytes());
        headers.push(7);
        headers.extend((value.len() as u16).to_be_bytes());
        headers.extend(value.as_bytes());
    }
    let mut bytes = Vec::new();
    bytes.extend(((16 + headers.len() + payload.len()) as u32).to_be_bytes());
    bytes.extend((headers.len() as u32).to_be_bytes());
    bytes.extend(crc(&bytes).to_be_bytes());
    bytes.extend(headers);
    bytes.extend(payload);
    bytes.extend(crc(&bytes).to_be_bytes());
    bytes
}

async fn response(bytes: Vec<u8>) -> reqwest::Response {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 4096];
        socket.read(&mut request).await.unwrap();
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        socket.write_all(&bytes).await.unwrap();
    });
    let response = reqwest::Client::new()
        .get(format!("http://{address}"))
        .send()
        .await
        .unwrap();
    server.await.unwrap();
    response
}

#[tokio::test]
async fn bedrock_binary_collector_text_and_truncation() {
    let text = frame(
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}"#,
    );
    let stop = frame(r#"{"type":"message_stop"}"#);
    let mut collector = ReplayCollector::default();
    let result = run_llm_ndjson_request(
        response([text.clone(), stop].concat()).await,
        get_adapter(WireFormat::AnthropicMessages),
        "",
        None,
        None,
        &mut collector,
        None,
        None,
        true,
    )
    .await
    .unwrap();
    assert_eq!(result[0].content, "hello");
    let mut truncated = text;
    truncated.pop();
    let error = run_llm_ndjson_request(
        response(truncated).await,
        get_adapter(WireFormat::AnthropicMessages),
        "",
        None,
        None,
        &mut collector,
        None,
        None,
        true,
    )
    .await
    .err()
    .unwrap();
    assert!(error.contains("Truncated Bedrock"));
}

#[tokio::test]
async fn bedrock_errors_retain_partial_output_and_cancel_is_terminal() {
    let text = frame(
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"partial"}}"#,
    );
    let error =
        frame(r#"{"type":"error","error":{"type":"overloaded_error","message":"overloaded"}}"#);
    let mut collector = ReplayCollector::default();
    let mut partial = false;
    let mut tracking = PartialOutputCollector {
        collector: &mut collector,
        partial_output_emitted: &mut partial,
        progress_events: Arc::new(AtomicU64::new(0)),
    };
    let result = run_llm_ndjson_request(
        response([text, error].concat()).await,
        get_adapter(WireFormat::AnthropicMessages),
        "",
        None,
        None,
        &mut tracking,
        None,
        None,
        true,
    )
    .await;
    assert!(result.err().unwrap().contains("overloaded"));
    assert!(partial);
    let result = run_llm_ndjson_request(
        response(frame(r#"{"type":"message_stop"}"#)).await,
        get_adapter(WireFormat::AnthropicMessages),
        "",
        Some(Arc::new(AtomicBool::new(true))),
        None,
        &mut collector,
        None,
        None,
        true,
    )
    .await;
    assert_eq!(result.err().unwrap(), ABORT_ERROR_MESSAGE);
}

#[tokio::test]
async fn bedrock_binary_collector_preserves_tools_and_usage() {
    let events = [
        json!({"type":"message_start","message":{"usage":{"input_tokens":12,"output_tokens":0}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"tool_1","name":"lookup","input":{}}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"query\":\"test\"}"}}),
        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":7}}),
        json!({"type":"message_stop"}),
    ];
    let bytes = events
        .iter()
        .flat_map(|event| frame(&event.to_string()))
        .collect();
    let mut collector = ReplayCollector::default();
    let choices = run_llm_ndjson_request(
        response(bytes).await,
        get_adapter(WireFormat::AnthropicMessages),
        "",
        None,
        None,
        &mut collector,
        None,
        None,
        true,
    )
    .await
    .unwrap();
    assert_eq!(choices[0].tool_calls_raw[0]["function"]["name"], "lookup");
    assert_eq!(
        choices[0].tool_calls_raw[0]["function"]["arguments"],
        "{\"query\":\"test\"}"
    );
    assert_eq!(choices[0].finish_reason.as_deref(), Some("tool_use"));
    assert_eq!(choices[0].usage.as_ref().unwrap().completion_tokens, 7);
}

#[test]
fn native_marker_disables_subscription_identity_even_with_stale_token() {
    let mut model = BaseModelRecord {
        id: "claude_code/claude".into(),
        wire_format: WireFormat::AnthropicMessages,
        auth_token: "stale-oauth".into(),
        ..Default::default()
    };
    assert_eq!(claude_code_instance_id(&model), Some("claude_code"));
    for backend in [
        ClaudeBackend::Custom,
        ClaudeBackend::Anthropic,
        ClaudeBackend::Bedrock,
        ClaudeBackend::Vertex,
        ClaudeBackend::Foundry,
    ] {
        model.extra_headers.insert(
            CLAUDE_TRANSPORT_HEADER.into(),
            serde_json::to_string(&ClaudeTransportConfig {
                backend,
                ..Default::default()
            })
            .unwrap(),
        );
        assert!(claude_code_instance_id(&model).is_none());
    }
}
