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
        recovery_budget: None,
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
        recovery_budget: None,
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
    assert!(error.starts_with("Request not sent: user privacy policy"));
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
    response_with_status(bytes, reqwest::StatusCode::OK).await
}

async fn response_with_status(bytes: Vec<u8>, status: reqwest::StatusCode) -> reqwest::Response {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 4096];
        socket.read(&mut request).await.unwrap();
        socket
            .write_all(
                format!(
                    "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    status,
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

pub(super) struct RefreshHook {
    pub(super) provider: Option<crate::providers::claude_code::ClaudeCodeProvider>,
    pub(super) calls: Vec<(String, reqwest::StatusCode, String)>,
}

pub(super) struct RequestHook {
    pub(super) requests: Vec<(String, reqwest::header::HeaderMap, Vec<u8>)>,
    pub(super) responses: std::collections::VecDeque<reqwest::Response>,
}

tokio::task_local! {
    pub(super) static REFRESH_HOOK: std::cell::RefCell<RefreshHook>;
    pub(super) static REQUEST_HOOK: std::cell::RefCell<RequestHook>;
}

fn retry_params(config: Option<ClaudeTransportConfig>) -> StreamRunParams {
    let mut model = tests::model_record("claude_code");
    model.wire_format = WireFormat::AnthropicMessages;
    model.endpoint = "http://unexpected.invalid/override".into();
    if let Some(config) = config {
        model.extra_headers.insert(
            CLAUDE_TRANSPORT_HEADER.into(),
            serde_json::to_string(&config).unwrap(),
        );
    } else {
        model.auth_token = "subscription-rejected".into();
    }
    StreamRunParams {
        recovery_budget: None,
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
        supports_tools: false,
        supports_reasoning: false,
        reasoning_type: None,
        supports_temperature: true,
        priority: RequestPriority::Interactive,
    }
}

fn retry_prepared(params: &StreamRunParams) -> PreparedLlmStreamRequest {
    PreparedLlmStreamRequest {
        client: reqwest::Client::new(),
        wire_format: params.model_rec.wire_format,
        adapter: get_adapter(params.model_rec.wire_format),
        adapter_settings: tests::adapter_settings(&params.model_rec),
        http_parts: HttpParts {
            url: params.model_rec.endpoint.clone(),
            headers: Default::default(),
            body: json!({}),
        },
        command_credential: None,
        snapshot_for_commit: None,
        header_retry_config: Some((Duration::from_secs(1), 1)),
    }
}

fn console_config() -> ClaudeTransportConfig {
    ClaudeTransportConfig {
        backend: ClaudeBackend::Anthropic,
        auth_type: refact_core::claude_backend::ClaudeAuthType::Oauth,
        auth_token: Some("console-rejected".into()),
        oauth_workspace_id: Some("old-workspace".into()),
        ..Default::default()
    }
}

async fn status_response(status: reqwest::StatusCode) -> reqwest::Response {
    response_with_status(Vec::new(), status).await
}

#[tokio::test]
async fn console_oauth_rejection_refreshes_transport_and_retries_default_endpoint() {
    use crate::providers::traits::ProviderTrait;
    let mut provider = crate::providers::claude_code::ClaudeCodeProvider::default();
    provider
        .provider_settings_apply(
            serde_yaml::from_str("backend: anthropic\nauth_type: oauth").unwrap(),
        )
        .unwrap();
    provider.console_oauth_tokens.access_token = "console-fresh".into();
    provider.console_oauth_tokens.expires_at = i64::MAX;
    provider.console_oauth_tokens.workspace_id = Some("fresh-workspace".into());
    for status in [
        reqwest::StatusCode::UNAUTHORIZED,
        reqwest::StatusCode::FORBIDDEN,
    ] {
        let app = tests::app_with_privacy_policy(tests::privacy_policy("claude_code")).await;
        let mut params = retry_params(Some(console_config()));
        let budget = Arc::new(refact_core::recovery::RecoveryBudget::new(
            Default::default(),
        ));
        params.recovery_budget = Some(budget.clone());
        assert!(params.model_rec.auth_token.is_empty());
        let prepared = retry_prepared(&params);
        let responses = [
            status_response(status).await,
            status_response(reqwest::StatusCode::OK).await,
        ]
        .into();
        REFRESH_HOOK
            .scope(
                std::cell::RefCell::new(RefreshHook {
                    provider: Some(provider.clone()),
                    calls: vec![],
                }),
                async {
                    REQUEST_HOOK
                        .scope(
                            std::cell::RefCell::new(RequestHook {
                                requests: vec![],
                                responses,
                            }),
                            async {
                                let (response, _) = send_http_stream_request(
                                    &app,
                                    &params,
                                    &prepared,
                                    "claude_code",
                                )
                                .await
                                .unwrap();
                                assert_eq!(response.status(), reqwest::StatusCode::OK);
                                assert_eq!(budget.sends_used(), 2);
                                REQUEST_HOOK.with(|hook| {
                                    let hook = hook.borrow();
                                    assert_eq!(hook.requests.len(), 2);
                                    for ((url, headers, body), token) in hook
                                        .requests
                                        .iter()
                                        .zip(["console-rejected", "console-fresh"])
                                    {
                                        assert_eq!(url, "https://api.anthropic.com/v1/messages");
                                        assert_eq!(
                                            headers["authorization"],
                                            format!("Bearer {token}")
                                        );
                                        assert!(!headers.contains_key(CLAUDE_TRANSPORT_HEADER));
                                        assert!(!String::from_utf8_lossy(body).contains(token));
                                        assert!(!String::from_utf8_lossy(body).contains("mcp_"));
                                    }
                                    assert_eq!(
                                        hook.requests[1].1["anthropic-workspace-id"],
                                        "fresh-workspace"
                                    );
                                });
                                REFRESH_HOOK.with(|hook| {
                                    assert_eq!(
                                        hook.borrow().calls,
                                        vec![(
                                            "claude_code".into(),
                                            status,
                                            "console-rejected".into()
                                        )]
                                    )
                                });
                                assert!(params.model_rec.auth_token.is_empty());
                            },
                        )
                        .await;
                },
            )
            .await;
    }
}

#[tokio::test]
async fn non_console_cloud_rejection_does_not_refresh_and_console_base_override_is_rejected() {
    for config in [
        ClaudeTransportConfig {
            backend: ClaudeBackend::Custom,
            auth_type: refact_core::claude_backend::ClaudeAuthType::Bearer,
            auth_token: Some("custom".into()),
            base_url: Some("http://custom.invalid".into()),
            ..Default::default()
        },
        ClaudeTransportConfig {
            base_url: Some("http://unexpected.invalid".into()),
            ..console_config()
        },
    ] {
        let app = tests::app_with_privacy_policy(tests::privacy_policy("claude_code")).await;
        let params = retry_params(Some(config.clone()));
        let prepared = retry_prepared(&params);
        let responses = [status_response(reqwest::StatusCode::UNAUTHORIZED).await].into();
        REFRESH_HOOK
            .scope(
                std::cell::RefCell::new(RefreshHook {
                    provider: None,
                    calls: vec![],
                }),
                async {
                    REQUEST_HOOK
                        .scope(
                            std::cell::RefCell::new(RequestHook {
                                requests: vec![],
                                responses,
                            }),
                            async {
                                let error = send_http_stream_request(
                                    &app,
                                    &params,
                                    &prepared,
                                    "claude_code",
                                )
                                .await
                                .err()
                                .unwrap();
                                if is_claude_console_oauth(&config) {
                                    assert!(error.message.contains("default Anthropic endpoint"));
                                    REQUEST_HOOK
                                        .with(|hook| assert!(hook.borrow().requests.is_empty()));
                                }
                                REFRESH_HOOK.with(|hook| assert!(hook.borrow().calls.is_empty()));
                            },
                        )
                        .await;
                },
            )
            .await;
    }
}

#[tokio::test]
async fn subscription_oauth_rejection_still_retries_bearer_without_cloud_transport() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for (token, status) in [
            ("subscription-rejected", "401 Unauthorized"),
            ("subscription-fresh", "200 OK"),
        ] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buffer = [0u8; 4096];
                let read = socket.read(&mut buffer).await.unwrap();
                assert_ne!(read, 0);
                bytes.extend_from_slice(&buffer[..read]);
                if bytes.windows(4).any(|s| s == b"\r\n\r\n") {
                    break;
                }
            }
            assert!(String::from_utf8_lossy(&bytes)
                .to_ascii_lowercase()
                .contains(&format!("authorization: bearer {token}")));
            socket
                .write_all(
                    format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .as_bytes(),
                )
                .await
                .unwrap();
        }
    });
    let app = tests::app_with_privacy_policy(tests::privacy_policy("claude_code")).await;
    let mut params = retry_params(None);
    let budget = Arc::new(refact_core::recovery::RecoveryBudget::new(
        Default::default(),
    ));
    params.recovery_budget = Some(budget.clone());
    params.model_rec.endpoint = format!("http://{address}");
    let mut prepared = retry_prepared(&params);
    set_bearer_authorization_header(&mut prepared.http_parts.headers, "subscription-rejected")
        .unwrap();
    let mut provider = crate::providers::claude_code::ClaudeCodeProvider::default();
    provider.oauth_tokens.access_token = "subscription-fresh".into();
    provider.oauth_tokens.expires_at = i64::MAX;
    REFRESH_HOOK
        .scope(
            std::cell::RefCell::new(RefreshHook {
                provider: Some(provider),
                calls: vec![],
            }),
            async {
                let (response, _) =
                    send_http_stream_request(&app, &params, &prepared, "claude_code")
                        .await
                        .unwrap();
                assert_eq!(response.status(), reqwest::StatusCode::OK);
                assert_eq!(budget.sends_used(), 2);
                REFRESH_HOOK
                    .with(|hook| assert_eq!(hook.borrow().calls[0].2, "subscription-rejected"));
            },
        )
        .await;
    server.await.unwrap();
}

#[tokio::test]
async fn console_oauth_retry_rejects_provider_mode_switch() {
    let app = tests::app_with_privacy_policy(tests::privacy_policy("claude_code")).await;
    let params = retry_params(Some(console_config()));
    let prepared = retry_prepared(&params);
    let responses = [status_response(reqwest::StatusCode::UNAUTHORIZED).await].into();
    REFRESH_HOOK
        .scope(
            std::cell::RefCell::new(RefreshHook {
                provider: Some(crate::providers::claude_code::ClaudeCodeProvider::default()),
                calls: vec![],
            }),
            async {
                REQUEST_HOOK
                    .scope(
                        std::cell::RefCell::new(RequestHook {
                            requests: vec![],
                            responses,
                        }),
                        async {
                            let error =
                                send_http_stream_request(&app, &params, &prepared, "claude_code")
                                    .await
                                    .err()
                                    .unwrap();
                            assert!(error.message.contains("mode changed"));
                            REQUEST_HOOK.with(|hook| assert_eq!(hook.borrow().requests.len(), 1));
                        },
                    )
                    .await;
            },
        )
        .await;
}
