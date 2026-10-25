use super::*;
use serde_json::json;

#[tokio::test]
async fn aws_credentials_refresh_margin_fallback_and_failures() {
    let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    for (expiry, refresh_seconds) in [(Some(start + Duration::from_secs(900)), 600), (None, 3600)] {
        let entry = Mutex::new(None);
        let calls = std::sync::atomic::AtomicUsize::new(0);
        for seconds in [0, refresh_seconds - 1, refresh_seconds] {
            resolve_aws_credentials(
                &entry,
                || start + Duration::from_secs(seconds),
                || async {
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(Credentials::new("test", "secret", None, expiry, "test"))
                },
            )
            .await
            .unwrap();
        }
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
    let entry = Mutex::new(None);
    assert!(resolve_aws_credentials(
        &entry,
        || start,
        || async { Err("injected failure".into()) }
    )
    .await
    .is_err());
    assert!(entry.lock().await.is_none());
    let result = resolve_aws_credentials(
        &entry,
        || start,
        || async { Ok(Credentials::new("recovered", "secret", None, None, "test")) },
    )
    .await
    .unwrap();
    assert_eq!(result.access_key_id(), "recovered");
}

#[tokio::test]
async fn aws_cache_singleflight_independent_keys_and_safe_eviction() {
    let cache = AwsKeyCache::<Mutex<Option<CachedAwsCredentials>>>::default();
    let key = (Some("profile".into()), Some("region".into()));
    let entry = cache.entry(key.clone()).await.unwrap();
    assert!(Arc::ptr_eq(&entry, &cache.entry(key).await.unwrap()));
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let resolve = || async {
        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        tokio::task::yield_now().await;
        Ok(Credentials::new("test", "secret", None, None, "test"))
    };
    let (a, b) = tokio::join!(
        resolve_aws_credentials(&entry, SystemTime::now, resolve),
        resolve_aws_credentials(&entry, SystemTime::now, resolve),
    );
    assert!(a.is_ok() && b.is_ok());
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    let guard = entry.lock().await;
    let other = cache.entry((Some("other".into()), None)).await.unwrap();
    assert!(resolve_aws_credentials(&other, SystemTime::now, resolve)
        .await
        .is_ok());
    drop(guard);
    for n in 0..100 {
        cache
            .entry((Some(format!("profile-{n}")), None))
            .await
            .unwrap();
    }
    assert_eq!(cache.entries.lock().await.len(), 32);
    assert!(cache
        .entries
        .lock()
        .await
        .values()
        .any(|value| Arc::ptr_eq(value, &entry)));
    let busy = AwsKeyCache::<OnceCell<usize>>::default();
    let mut held = Vec::new();
    for n in 0..32 {
        held.push(busy.entry((Some(n.to_string()), None)).await.unwrap());
    }
    assert!(busy.entry((None, None)).await.is_err());
    drop(held);
    assert!(busy.entry((None, None)).await.is_ok());
}

#[tokio::test]
async fn explicit_aws_credentials_bypass_chain_cache() {
    let mut c = config(ClaudeBackend::Bedrock);
    c.aws_profile = Some("nonexistent-profile-no-network".into());
    c.aws_access_key_id = Some("explicit".into());
    c.aws_secret_access_key = Some("explicit-secret".into());
    assert_eq!(
        aws_credentials(&c).await.unwrap().access_key_id(),
        "explicit"
    );
    c.aws_access_key_id = Some("changed".into());
    assert_eq!(
        aws_credentials(&c).await.unwrap().access_key_id(),
        "changed"
    );
    c.aws_secret_access_key = None;
    assert!(aws_credentials(&c).await.is_err());
}

#[tokio::test]
async fn foundry_suffix_token_priority_and_feature_betas() {
    for suffix in ["", "/anthropic", "/anthropic/v1/messages"] {
        let mut c = config(ClaudeBackend::Foundry);
        c.base_url = Some(format!("https://resource.services.ai.azure.com{suffix}"));
        c.api_key = Some("unused-key".into());
        c.auth_token = Some("preferred-token".into());
        let mut headers = stale_headers();
        header(
            &mut headers,
            "anthropic-beta",
            "oauth-2025-04-20,interleaved-thinking-2025-05-14",
        )
        .unwrap();
        let p = prepare_attempt(
            "https://unused.invalid".into(),
            headers,
            json!({"messages":[]}),
            "m",
            true,
            &c,
        )
        .await
        .unwrap();
        assert_eq!(
            p.url,
            "https://resource.services.ai.azure.com/anthropic/v1/messages"
        );
        assert_eq!(p.headers["authorization"], "Bearer preferred-token");
        assert!(!p.headers.contains_key("api-key"));
        assert_eq!(
            p.headers["anthropic-beta"],
            "interleaved-thinking-2025-05-14"
        );
    }
}

fn config(backend: ClaudeBackend) -> ClaudeTransportConfig {
    ClaudeTransportConfig {
        backend,
        ..Default::default()
    }
}

#[tokio::test]
async fn configured_messages_endpoints_override_model_endpoint() {
    for backend in [ClaudeBackend::Custom, ClaudeBackend::Anthropic] {
        for suffix in ["", "/", "/v1", "/v1/", "/v1/messages"] {
            let mut c = config(backend);
            c.base_url = Some(format!("http://127.0.0.1:8123/gateway{suffix}"));
            c.api_key = Some("test-key".into());
            let prepared = prepare_attempt(
                "https://wrong-provider.invalid/v1/messages".into(),
                HeaderMap::new(),
                json!({"messages": []}),
                "claude",
                true,
                &c,
            )
            .await
            .unwrap();
            assert_eq!(prepared.url, "http://127.0.0.1:8123/gateway/v1/messages");
        }
    }
    let mut c = config(ClaudeBackend::Custom);
    c.auth_type = ClaudeAuthType::None;
    assert!(prepare_attempt(
        "https://api.anthropic.com/v1/messages".into(),
        HeaderMap::new(),
        json!({}),
        "claude",
        true,
        &c,
    )
    .await
    .err()
    .unwrap()
    .contains("base_url"));
}

fn stale_headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    for (k, v) in [
        ("authorization", "Bearer oauth-secret"),
        ("x-api-key", "wrong"),
        ("anthropic-beta", "claude-code-oauth"),
        ("x-refact-internal-claude-transport", "secret config"),
    ] {
        header(&mut h, k, v).unwrap();
    }
    h
}

#[tokio::test]
async fn standard_body_and_auth_isolation() {
    let body = json!({"model":"claude","stream":true,"system":[{"type":"text","text":"unchanged"}],"messages":[]});
    for auth in [
        ClaudeAuthType::ApiKey,
        ClaudeAuthType::Bearer,
        ClaudeAuthType::None,
    ] {
        let mut c = config(ClaudeBackend::Custom);
        c.auth_type = auth;
        c.api_key = Some("key".into());
        c.auth_token = Some("token".into());
        c.base_url = Some("https://custom.invalid".into());
        let p = prepare_attempt(
            "https://custom.invalid/v1/messages".into(),
            stale_headers(),
            body.clone(),
            "claude",
            true,
            &c,
        )
        .await
        .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&p.body_bytes).unwrap(),
            body
        );
        assert!(!p.headers.contains_key("anthropic-beta"));
        assert!(!p.headers.contains_key("x-refact-internal-claude-transport"));
        assert_eq!(
            p.headers.contains_key("authorization"),
            auth == ClaudeAuthType::Bearer
        );
        assert_eq!(
            p.headers.contains_key("x-api-key"),
            auth == ClaudeAuthType::ApiKey
        );
    }
    let mut c = config(ClaudeBackend::Anthropic);
    c.api_key = Some("api".into());
    let p = prepare_attempt(
        "https://api.anthropic.com/v1/messages".into(),
        stale_headers(),
        body.clone(),
        "claude",
        true,
        &c,
    )
    .await
    .unwrap();
    assert_eq!(p.headers["x-api-key"], "api");
    assert!(!p.headers.contains_key("authorization"));
}

#[tokio::test]
async fn bedrock_partition_arn_body_and_bearer() {
    let mut c = config(ClaudeBackend::Bedrock);
    c.aws_region = Some("cn-north-1".into());
    c.auth_token = Some("aws-token".into());
    let mut headers = stale_headers();
    header(
        &mut headers,
        "anthropic-beta",
        "oauth-2025-04-20,interleaved-thinking-2025-05-14",
    )
    .unwrap();
    let model = "arn:aws-cn:bedrock:cn-north-1:123:inference-profile/a";
    let p = prepare_attempt(
        "https://ignored.invalid".into(),
        headers,
        json!({"model":model,"stream":true,"messages":[]}),
        model,
        true,
        &c,
    )
    .await
    .unwrap();
    assert!(p
        .url
        .starts_with("https://bedrock-runtime.cn-north-1.amazonaws.com.cn/model/arn:"));
    assert!(p.url.contains("inference-profile%2Fa/"));
    assert_eq!(p.framing, ResponseFraming::AwsEventStream);
    let body: Value = serde_json::from_slice(&p.body_bytes).unwrap();
    assert!(body.get("model").is_none());
    assert!(body.get("stream").is_none());
    assert_eq!(body["anthropic_version"], "bedrock-2023-05-31");
    assert_eq!(
        body["anthropic_beta"],
        json!(["interleaved-thinking-2025-05-14"])
    );
    assert!(!p.headers.contains_key("anthropic-beta"));
    assert_eq!(p.headers["authorization"], "Bearer aws-token");
    assert!(p.headers["authorization"].is_sensitive());
    assert!(!p.headers.contains_key("x-api-key"));
}

#[tokio::test]
async fn vertex_and_foundry_native_routes_without_auth_network() {
    let mut c = config(ClaudeBackend::Vertex);
    c.gcp_project = Some("project".into());
    c.gcp_region = Some("global".into());
    c.auth_token = Some("gcp".into());
    let p = prepare_attempt(
        "https://ignored.invalid".into(),
        stale_headers(),
        json!({"model":"m","stream":true}),
        "claude@20260101",
        true,
        &c,
    )
    .await
    .unwrap();
    assert_eq!(p.url,"https://aiplatform.googleapis.com/v1/projects/project/locations/global/publishers/anthropic/models/claude@20260101:streamRawPredict");
    assert_eq!(
        serde_json::from_slice::<Value>(&p.body_bytes).unwrap(),
        json!({"anthropic_version":"vertex-2023-10-16","stream":true})
    );
    let mut c = config(ClaudeBackend::Foundry);
    c.azure_resource = Some("resource".into());
    c.api_key = Some("azure-key".into());
    let p = prepare_attempt(
        "https://ignored.invalid".into(),
        stale_headers(),
        json!({"model":"m"}),
        "m",
        true,
        &c,
    )
    .await
    .unwrap();
    assert_eq!(
        p.url,
        "https://resource.services.ai.azure.com/anthropic/v1/messages"
    );
    assert_eq!(p.headers["api-key"], "azure-key");
    assert!(!p.headers.contains_key("authorization"));
}

#[test]
fn signature_exact_bytes_time_region_and_token() {
    let credentials = || {
        Credentials::new(
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            Some("session".into()),
            None,
            "test",
        )
    };
    let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let sign_body = |body: &[u8]| {
        let mut h = HeaderMap::new();
        header(&mut h, "content-type", "application/json").unwrap();
        sign_request(
            "https://bedrock-runtime.us-east-1.amazonaws.com/model/m/invoke",
            &mut h,
            body,
            "us-east-1",
            credentials(),
            time,
        )
        .unwrap();
        h
    };
    let h = sign_body(br#"{"anthropic_version":"bedrock-2023-05-31"}"#);
    assert!(h["authorization"]
        .to_str()
        .unwrap()
        .contains("20231114/us-east-1/bedrock/aws4_request"));
    assert_eq!(h["x-amz-security-token"], "session");
    assert_eq!(h["x-amz-date"], "20231114T221320Z");
    assert_ne!(h["authorization"], sign_body(b"{}")["authorization"]);
    assert_eq!(
        h["authorization"],
        sign_body(br#"{"anthropic_version":"bedrock-2023-05-31"}"#)["authorization"]
    );
}

#[test]
fn endpoint_rejects_credentials_and_encodes_path() {
    assert!(endpoint("https://user:password@example.com", &["model"]).is_err());
    assert!(endpoint("https://example.com?secret=x", &["model"]).is_err());
    assert_eq!(
        endpoint("https://example.com/root/", &["model", "a/b %"]).unwrap(),
        "https://example.com/root/model/a%2Fb%20%25"
    );
}
