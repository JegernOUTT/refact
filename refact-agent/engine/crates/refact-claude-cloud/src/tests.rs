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
    let input = json!({
        "model": model, "stream": true, "max_tokens": 4096,
        "metadata": {"user_id": "cache-regression"},
        "tools": [{"name": "inspect", "description": "Inspect payload", "input_schema": {
            "type": "object", "cache_control": {"type": "ephemeral", "ttl": "1h"}
        }, "cache_control": {"type": "ephemeral", "ttl": "1h"}}],
        "system": [{"type": "text", "text": "Use inspect", "cache_control": {"type": "ephemeral", "ttl": "1h"}}],
        "messages": [
            {"role": "assistant", "content": [
                {"type": "thinking", "thinking": "Opaque reasoning\n café", "signature": "sig+/=\n",
                 "cache_control": {"type": "ephemeral", "ttl": "1h"}},
                {"type": "redacted_thinking", "data": "opaque+/=",
                 "cache_control": {"type": "ephemeral", "ttl": "1h"}},
                {"type": "tool_use", "id": "call_1", "name": "inspect", "input": {
                    "cache_control": {"type": "ephemeral", "ttl": "1h"}, "ttl": "user-value"
                }}
            ]},
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "call_1", "content": [
                    {"type": "text", "text": "User data", "cache_control": {"type": "ephemeral", "ttl": "1h"}}
                ], "cache_control": {"type": "ephemeral", "ttl": "1h"}},
                {"type": "text", "text": "Continue", "cache_control": {"type": "ephemeral"}}
            ]}
        ]
    });
    let signed_blocks = serde_json::to_vec(&input["messages"][0]["content"]).unwrap();
    let p = prepare_attempt(
        "https://ignored.invalid".into(),
        headers,
        input.clone(),
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
    assert_eq!(
        serde_json::to_vec(&body["messages"][0]["content"]).unwrap(),
        signed_blocks,
        "signed thinking and tool input must retain exact serialized bytes"
    );
    let mut expected = input.clone();
    expected.as_object_mut().unwrap().remove("model");
    expected.as_object_mut().unwrap().remove("stream");
    expected["anthropic_version"] = json!("bedrock-2023-05-31");
    expected["anthropic_beta"] = json!(["interleaved-thinking-2025-05-14"]);
    expected["tools"][0]["cache_control"] = json!({"type": "ephemeral"});
    expected["system"][0]["cache_control"] = json!({"type": "ephemeral"});
    expected["messages"][1]["content"][0]["cache_control"] = json!({"type": "ephemeral"});
    assert_eq!(body, expected, "only protocol cache TTLs should change");
    let mut anthropic = config(ClaudeBackend::Anthropic);
    anthropic.api_key = Some("test-key".into());
    let direct = prepare_attempt(
        "https://ignored.invalid".into(),
        HeaderMap::new(),
        input.clone(),
        model,
        true,
        &anthropic,
    )
    .await
    .unwrap();
    assert_eq!(direct.body_bytes, serde_json::to_vec(&input).unwrap());
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
fn azure_plan_preserves_identity_priority_and_falls_back_to_cli() {
    let plan = |vars: &[&str]| {
        let vars: Vec<String> = vars.iter().map(|v| v.to_string()).collect();
        azure_plan(move |name| vars.iter().any(|v| v == name))
    };
    let secret = ["AZURE_TENANT_ID", "AZURE_CLIENT_ID", "AZURE_CLIENT_SECRET"];
    assert_eq!(
        plan(
            &[
                &secret[..],
                &["AZURE_FEDERATED_TOKEN_FILE", "IDENTITY_ENDPOINT"]
            ]
            .concat()
        ),
        AzurePlan::ClientSecret
    );
    assert_eq!(
        plan(&[
            "AZURE_TENANT_ID",
            "AZURE_CLIENT_ID",
            "AZURE_FEDERATED_TOKEN_FILE",
            "MSI_ENDPOINT"
        ]),
        AzurePlan::WorkloadIdentity
    );
    assert_eq!(plan(&["IDENTITY_ENDPOINT"]), AzurePlan::ManagedIdentity);
    assert_eq!(plan(&["MSI_ENDPOINT"]), AzurePlan::ManagedIdentity);
    assert_eq!(
        plan(&["AZURE_TENANT_ID", "AZURE_CLIENT_ID"]),
        AzurePlan::ImdsThenDeveloperTools
    );
    assert_eq!(plan(&[]), AzurePlan::ImdsThenDeveloperTools);
    assert_eq!(
        azure_chain(AzurePlan::ImdsThenDeveloperTools)
            .unwrap()
            .sources
            .len(),
        2
    );
}

fn stub_source(
    calls: Arc<std::sync::atomic::AtomicUsize>,
    token: Option<&'static str>,
    expires_in_minutes: i64,
    delay: Option<Duration>,
    timeout: Option<Duration>,
) -> AzureSource {
    AzureSource {
        fetch: Arc::new(move || {
            let calls = calls.clone();
            Box::pin(async move {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if let Some(delay) = delay {
                    tokio::time::sleep(delay).await;
                }
                token
                    .map(|t| {
                        azure_core::credentials::AccessToken::new(
                            t,
                            azure_core::time::OffsetDateTime::UNIX_EPOCH
                                + azure_core::time::Duration::minutes(expires_in_minutes),
                        )
                    })
                    .ok_or_else(|| "stub failure secret-detail".to_string())
            })
        }),
        timeout,
    }
}

#[tokio::test(start_paused = true)]
async fn azure_chain_falls_back_to_cli_caches_and_sticks_to_selected_source() {
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    let epoch = azure_core::time::OffsetDateTime::UNIX_EPOCH;
    let imds = Arc::new(AtomicUsize::new(0));
    let cli = Arc::new(AtomicUsize::new(0));
    let chain = AzureChain::new(vec![
        stub_source(
            imds.clone(),
            Some("imds-token"),
            60,
            Some(Duration::from_secs(60)),
            Some(Duration::from_secs(3)),
        ),
        stub_source(cli.clone(), Some("cli-token"), 60, None, None),
    ]);
    assert_eq!(chain.token(epoch).await.unwrap(), "cli-token");
    assert_eq!(
        chain
            .token(epoch + azure_core::time::Duration::minutes(54))
            .await
            .unwrap(),
        "cli-token"
    );
    assert_eq!((imds.load(SeqCst), cli.load(SeqCst)), (1, 1));
    assert_eq!(
        chain
            .token(epoch + azure_core::time::Duration::minutes(56))
            .await
            .unwrap(),
        "cli-token"
    );
    assert_eq!((imds.load(SeqCst), cli.load(SeqCst)), (1, 2));

    let first = Arc::new(AtomicUsize::new(0));
    let second = Arc::new(AtomicUsize::new(0));
    let chain = AzureChain::new(vec![
        stub_source(first.clone(), Some("managed-token"), 60, None, None),
        stub_source(second.clone(), Some("cli-token"), 60, None, None),
    ]);
    assert_eq!(chain.token(epoch).await.unwrap(), "managed-token");
    assert_eq!(second.load(SeqCst), 0);

    let calls = Arc::new(AtomicUsize::new(0));
    let chain = AzureChain::new(vec![
        stub_source(calls.clone(), None, 60, None, None),
        stub_source(calls.clone(), None, 60, None, None),
    ]);
    let error = chain.token(epoch).await.unwrap_err();
    assert_eq!(error, "Azure token acquisition failed");
    assert!(!error.contains("secret-detail"));
    assert_eq!(calls.load(SeqCst), 2);
}

#[tokio::test]
async fn foundry_explicit_credentials_bypass_azure_chain() {
    let mut c = config(ClaudeBackend::Foundry);
    c.azure_resource = Some("resource".into());
    c.api_key = Some("azure-key".into());
    c.auth_token = Some("azure-token".into());
    let p = prepare_attempt(
        "https://ignored.invalid".into(),
        HeaderMap::new(),
        json!({}),
        "m",
        false,
        &c,
    )
    .await
    .unwrap();
    assert_eq!(p.headers["authorization"], "Bearer azure-token");
    assert!(p.headers["authorization"].is_sensitive());
    assert!(!p.headers.contains_key("api-key"));
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

fn console_config() -> ClaudeTransportConfig {
    let mut c = config(ClaudeBackend::Anthropic);
    c.auth_type = ClaudeAuthType::Oauth;
    c.auth_token = Some("console-token".into());
    c.api_key = Some("unused-key".into());
    c
}

async fn prepare_with(
    headers: HeaderMap,
    c: &ClaudeTransportConfig,
) -> Result<PreparedRequest, String> {
    prepare_attempt(
        "https://wrong-provider.invalid/v1/messages".into(),
        headers,
        json!({"model":"claude","stream":true,"messages":[]}),
        "claude",
        true,
        c,
    )
    .await
}

#[tokio::test]
async fn console_oauth_uses_bearer_default_endpoint_betas_and_workspace() {
    let mut headers = stale_headers();
    header(
        &mut headers,
        "anthropic-beta",
        "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14",
    )
    .unwrap();
    header(&mut headers, "anthropic-workspace-id", "stale-workspace").unwrap();
    let mut c = console_config();
    c.oauth_workspace_id = Some(" wrkspc_1 ".into());
    let p = prepare_with(headers, &c).await.unwrap();
    assert_eq!(p.url, "https://api.anthropic.com/v1/messages");
    assert_eq!(p.framing, ResponseFraming::Sse);
    assert_eq!(p.headers["authorization"], "Bearer console-token");
    assert!(p.headers["authorization"].is_sensitive());
    assert!(!p.headers.contains_key("x-api-key"));
    assert!(!p.headers.contains_key("x-refact-internal-claude-transport"));
    assert_eq!(
        p.headers["anthropic-beta"],
        "interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14,oauth-2025-04-20"
    );
    assert_eq!(p.headers["anthropic-workspace-id"], "wrkspc_1");
    assert_eq!(p.headers["anthropic-version"], "2023-06-01");
    assert_eq!(p.headers.get_all("authorization").iter().count(), 1);

    for workspace in [None, Some(""), Some("   ")] {
        let mut headers = stale_headers();
        header(&mut headers, "anthropic-workspace-id", "stale-workspace").unwrap();
        let mut c = console_config();
        c.oauth_workspace_id = workspace.map(str::to_string);
        let p = prepare_with(headers, &c).await.unwrap();
        assert!(!p.headers.contains_key("anthropic-workspace-id"));
        assert_eq!(p.headers["anthropic-beta"], "oauth-2025-04-20");
        assert_eq!(p.headers["authorization"], "Bearer console-token");
    }
}

#[tokio::test]
async fn console_oauth_rejects_endpoints_missing_token_and_custom_oauth() {
    for base in [
        "https://api.anthropic.com",
        "https://proxy.invalid",
        "http://127.0.0.1:8123",
        "",
    ] {
        let mut c = console_config();
        c.base_url = Some(base.into());
        assert!(prepare_with(stale_headers(), &c)
            .await
            .err()
            .unwrap()
            .contains("default Anthropic endpoint"));
    }
    for token in [None, Some(""), Some("  ")] {
        let mut c = console_config();
        c.auth_token = token.map(str::to_string);
        assert!(prepare_with(stale_headers(), &c)
            .await
            .err()
            .unwrap()
            .contains("auth_token"));
    }
    let mut c = config(ClaudeBackend::Custom);
    c.auth_type = ClaudeAuthType::Oauth;
    c.base_url = Some("https://custom.invalid".into());
    c.auth_token = Some("token".into());
    c.api_key = Some("key".into());
    assert!(prepare_with(stale_headers(), &c)
        .await
        .err()
        .unwrap()
        .contains("custom endpoints"));
}

#[tokio::test]
async fn stale_workspace_header_never_reaches_other_backends() {
    let mut anthropic = config(ClaudeBackend::Anthropic);
    anthropic.api_key = Some("api".into());
    anthropic.oauth_workspace_id = Some("wrkspc_1".into());
    let mut custom = config(ClaudeBackend::Custom);
    custom.base_url = Some("https://custom.invalid".into());
    custom.api_key = Some("key".into());
    custom.oauth_workspace_id = Some("wrkspc_1".into());
    let mut bedrock = config(ClaudeBackend::Bedrock);
    bedrock.aws_region = Some("us-east-1".into());
    bedrock.auth_token = Some("aws".into());
    let mut vertex = config(ClaudeBackend::Vertex);
    vertex.gcp_project = Some("p".into());
    vertex.gcp_region = Some("global".into());
    vertex.auth_token = Some("gcp".into());
    let mut foundry = config(ClaudeBackend::Foundry);
    foundry.azure_resource = Some("resource".into());
    foundry.api_key = Some("azure".into());
    for c in [anthropic, custom, bedrock, vertex, foundry] {
        let mut headers = stale_headers();
        header(&mut headers, "anthropic-workspace-id", "stale-workspace").unwrap();
        header(&mut headers, "anthropic-beta", "oauth-2025-04-20").unwrap();
        let p = prepare_with(headers, &c).await.unwrap();
        assert!(!p.headers.contains_key("anthropic-workspace-id"));
        assert!(!p.headers.contains_key("anthropic-beta"));
        assert!(!p.headers.contains_key("x-refact-internal-claude-transport"));
        assert!(p
            .headers
            .get("authorization")
            .is_none_or(|v| v != "Bearer oauth-secret"));
        assert!(p.headers.get("x-api-key").is_none_or(|v| v != "wrong"));
    }
    let mut headers = stale_headers();
    header(&mut headers, "anthropic-workspace-id", "subscription-ws").unwrap();
    let p = prepare_with(headers, &config(ClaudeBackend::Oauth))
        .await
        .unwrap();
    assert_eq!(p.url, "https://wrong-provider.invalid/v1/messages");
    assert_eq!(p.headers["authorization"], "Bearer oauth-secret");
    assert_eq!(p.headers["anthropic-workspace-id"], "subscription-ws");
}

#[tokio::test]
async fn resettable_gives_fresh_arc_keeps_in_flight_holder_and_singleflight() {
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    let slot: Resettable<OnceCell<usize>> = Resettable::new();
    let held = slot.get();
    assert!(Arc::ptr_eq(&held, &slot.get()));
    let calls = AtomicUsize::new(0);
    let init = || async {
        calls.fetch_add(1, SeqCst);
        tokio::task::yield_now().await;
        1usize
    };
    let in_flight = held.get_or_init(init);
    slot.reset();
    assert_eq!(in_flight.await, &1);
    assert_eq!(held.get(), Some(&1));
    let fresh = slot.get();
    assert!(!Arc::ptr_eq(&held, &fresh));
    assert!(fresh.get().is_none());
    let (a, b) = tokio::join!(
        fresh.get_or_init(|| async {
            calls.fetch_add(1, SeqCst);
            tokio::task::yield_now().await;
            2usize
        }),
        fresh.get_or_init(|| async {
            calls.fetch_add(1, SeqCst);
            3usize
        }),
    );
    assert_eq!((*a, *b), (2, 2));
    assert_eq!(calls.load(SeqCst), 2);
    assert!(Arc::ptr_eq(&fresh, &slot.get()));
}

#[tokio::test]
async fn reset_cached_credentials_isolates_backends() {
    let snapshot = || {
        (
            AWS_CREDENTIALS.get(),
            AWS_SDK.get(),
            GOOGLE_ADC.get(),
            AZURE_CHAIN.get(),
        )
    };
    let (aws_creds, aws_sdk, google, azure) = snapshot();
    for backend in [
        ClaudeBackend::Oauth,
        ClaudeBackend::Anthropic,
        ClaudeBackend::Custom,
    ] {
        reset_cached_credentials(backend).await;
    }
    let same = snapshot();
    assert!(Arc::ptr_eq(&aws_creds, &same.0));
    assert!(Arc::ptr_eq(&aws_sdk, &same.1));
    assert!(Arc::ptr_eq(&google, &same.2));
    assert!(Arc::ptr_eq(&azure, &same.3));

    reset_cached_credentials(ClaudeBackend::Bedrock).await;
    let after = snapshot();
    assert!(!Arc::ptr_eq(&aws_creds, &after.0));
    assert!(!Arc::ptr_eq(&aws_sdk, &after.1));
    assert!(Arc::ptr_eq(&google, &after.2));
    assert!(Arc::ptr_eq(&azure, &after.3));

    reset_cached_credentials(ClaudeBackend::Vertex).await;
    let after_vertex = snapshot();
    assert!(Arc::ptr_eq(&after.0, &after_vertex.0));
    assert!(Arc::ptr_eq(&after.1, &after_vertex.1));
    assert!(!Arc::ptr_eq(&google, &after_vertex.2));
    assert!(Arc::ptr_eq(&azure, &after_vertex.3));

    reset_cached_credentials(ClaudeBackend::Foundry).await;
    let after_foundry = snapshot();
    assert!(Arc::ptr_eq(&after_vertex.0, &after_foundry.0));
    assert!(Arc::ptr_eq(&after_vertex.1, &after_foundry.1));
    assert!(Arc::ptr_eq(&after_vertex.2, &after_foundry.2));
    assert!(!Arc::ptr_eq(&azure, &after_foundry.3));

    let key = (Some("reset-test".into()), Some("us-east-1".into()));
    let entry = AWS_CREDENTIALS.get().entry(key.clone()).await.unwrap();
    *entry.lock().await = Some(CachedAwsCredentials {
        credentials: Credentials::new("old", "secret", None, None, "test"),
        refresh_at: SystemTime::now() + Duration::from_secs(3600),
    });
    reset_cached_credentials(ClaudeBackend::Bedrock).await;
    assert!(entry.lock().await.is_some());
    let fresh = AWS_CREDENTIALS.get().entry(key).await.unwrap();
    assert!(!Arc::ptr_eq(&entry, &fresh));
    let refreshed = resolve_aws_credentials(&fresh, SystemTime::now, || async {
        Ok(Credentials::new("new", "secret", None, None, "test"))
    })
    .await
    .unwrap();
    assert_eq!(refreshed.access_key_id(), "new");
}
