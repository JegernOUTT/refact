use super::*;

#[test]
fn discovery_filters_underlying_model_not_profile_name_and_preserves_pagination() {
    let page = json!({"nextToken":"page/2+opaque", "inferenceProfileSummaries":[
        {"inferenceProfileId":"custom", "inferenceProfileArn":"arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/custom", "inferenceProfileName":"Unrelated name", "status":"ACTIVE", "models":[{"modelArn":"arn:aws:bedrock:us-east-1::foundation-model/anthropic.claude-sonnet-5"}]},
        {"inferenceProfileId":"not-anthropic", "inferenceProfileName":"anthropic.claude-sonnet-5", "models":[{"modelArn":"arn:aws:bedrock:us-east-1::foundation-model/amazon.nova"}]}
    ]});
    let mut profiles = Vec::new();
    assert_eq!(
        profile_page(&page, &mut profiles).unwrap(),
        Some("page/2+opaque".into())
    );
    assert_eq!(
        profiles,
        vec![
            json!({"id":"custom", "arn":"arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/custom", "name":"Unrelated name", "status":"ACTIVE", "model_ids":["anthropic.claude-sonnet-5"]})
        ]
    );
}

#[test]
fn verified_identity_refuses_changed_principal() {
    let identity = parse_identity(b"<GetCallerIdentityResponse><GetCallerIdentityResult><Account>123456789012</Account><Arn>arn:aws:sts::123456789012:assumed-role/test/session</Arn><UserId>ROLE:session</UserId></GetCallerIdentityResult></GetCallerIdentityResponse>").unwrap();
    assert_eq!(
        identity,
        json!({"account":"123456789012", "arn":"arn:aws:sts::123456789012:assumed-role/test/session", "user_id":"ROLE:session"})
    );
    require_identity(&identity, &identity).unwrap();
    let mut changed = identity.clone();
    changed["arn"] = json!("arn:aws:sts::123456789012:assumed-role/other/session");
    assert_ne!(changed, identity);
    assert_eq!(
        require_identity(&changed, &identity).unwrap_err(),
        "AWS identity changed; verify setup again"
    );
}

#[tokio::test]
async fn probe_discards_sensitive_aws_response_without_retry() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let secret = "fixture-only-aws-secret-session-token";
    let worker = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 8192];
        let read = stream.read(&mut request).await.unwrap();
        assert!(std::str::from_utf8(&request[..read])
            .unwrap()
            .starts_with("POST /model/test/invoke HTTP/1.1"));
        let body = format!("{{\"message\":\"{secret}\"}}");
        stream.write_all(format!("HTTP/1.1 403 Forbidden\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        secret
    });
    let prepared = PreparedRequest {
        url: format!("http://{address}/model/test/invoke"),
        headers: HeaderMap::new(),
        body_bytes: b"{}".to_vec(),
        framing: ResponseFraming::Sse,
    };
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        execute_probe(&client().unwrap(), prepared),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(worker.await.unwrap(), secret);
    assert_eq!(
        result,
        (
            403,
            "access_denied: check IAM permissions and Anthropic model access".into()
        )
    );
    assert!(!result.1.contains(secret));
}

#[tokio::test]
async fn context_probe_rejects_unproven_model_without_credentials_or_network() {
    let config = ClaudeTransportConfig {
        backend: ClaudeBackend::Bedrock,
        ..Default::default()
    };
    assert_eq!(
        probe(&config, "us.anthropic.claude-haiku-4-5-20251001-v1:0", true)
            .await
            .unwrap_err(),
        "unsupported_context: 1M context is not established for this model ID"
    );
    assert!(supports_1m("global.anthropic.claude-sonnet-5"));
    assert!(!supports_1m("arn:aws:bedrock:us-east-1:123456789012:application-inference-profile/anthropic.claude-sonnet-5"));
}

#[tokio::test]
async fn context_beta_is_scoped_to_approved_exact_model() {
    let model = "us.anthropic.claude-sonnet-4-6";
    let mut config = ClaudeTransportConfig {
        backend: ClaudeBackend::Bedrock,
        aws_region: Some("us-east-1".into()),
        aws_access_key_id: Some("FIXTURE".into()),
        aws_secret_access_key: Some("fixture-only-secret".into()),
        bedrock_context_1m_models: vec![model.into()],
        ..Default::default()
    };
    let prepare = |config| async move {
        prepare_attempt(
            String::new(),
            HeaderMap::new(),
            json!({"messages":[]}),
            model,
            false,
            &config,
        )
        .await
        .unwrap()
    };
    let enabled = prepare(config.clone()).await;
    assert_eq!(
        serde_json::from_slice::<Value>(&enabled.body_bytes).unwrap()["anthropic_beta"],
        json!(["context-1m-2025-08-07"])
    );
    assert!(enabled
        .url
        .ends_with("/model/us.anthropic.claude-sonnet-4-6/invoke"));
    config.bedrock_context_1m_models = vec!["us.anthropic.claude-opus-4-6".into()];
    let disabled = prepare(config).await;
    assert!(serde_json::from_slice::<Value>(&disabled.body_bytes)
        .unwrap()
        .get("anthropic_beta")
        .is_none());
}

#[test]
fn identity_decodes_escaped_iam_path() {
    let xml = br#"<GetCallerIdentityResponse xmlns="https://sts.amazonaws.com/doc/2011-06-15/"><GetCallerIdentityResult><Account>123456789012</Account><Arn>arn:aws:iam::123456789012:user/team&amp;amp;ops/&quot;engineering&quot;/alice</Arn><UserId>user123</UserId></GetCallerIdentityResult></GetCallerIdentityResponse>"#;
    assert_eq!(
        parse_identity(xml).unwrap()["arn"],
        "arn:aws:iam::123456789012:user/team&amp;ops/\"engineering\"/alice"
    );
}

#[tokio::test]
async fn probe_validates_success_envelope() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for (status, body, success) in [
        (
            200,
            r#"{"type":"message","content":[{"type":"text","text":"Hi"}],"usage":{}}"#,
            true,
        ),
        (200, r#"{"type":"message","content":[],"usage":{}}"#, true),
        (200, "fixture-secret-not-json", false),
        (204, "", false),
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let worker = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 8192];
            stream.read(&mut request).await.unwrap();
            stream.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        });
        let prepared = PreparedRequest {
            url: format!("http://{address}/model/test/invoke"),
            headers: HeaderMap::new(),
            body_bytes: b"{}".to_vec(),
            framing: ResponseFraming::Sse,
        };
        let result = execute_probe(&client().unwrap(), prepared).await;
        worker.await.unwrap();
        if success {
            assert_eq!(
                result.unwrap(),
                (200, "available: model invocation succeeded".into())
            );
        } else {
            let error = result.unwrap_err();
            assert_eq!(error, "Invalid Bedrock probe response");
        }
    }
}

#[test]
fn identity_rejects_dtd_without_exposing_response() {
    let xml = b"<!DOCTYPE GetCallerIdentityResponse [<!ENTITY x 'fixture-secret'>]><GetCallerIdentityResponse><GetCallerIdentityResult><Account>123456789012</Account><Arn>arn:aws:iam::123456789012:user/&x;</Arn><UserId>user</UserId></GetCallerIdentityResult></GetCallerIdentityResponse>";
    assert_eq!(
        parse_identity(xml).unwrap_err(),
        "Invalid STS identity response"
    );
}
