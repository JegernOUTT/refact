use crate::claude_code::ClaudeCodeProvider;
use crate::traits::ProviderTrait;
use refact_core::claude_backend::{ClaudeTransportConfig, CLAUDE_TRANSPORT_HEADER};

fn apply(provider: &mut ClaudeCodeProvider, yaml: &str) -> Result<(), String> {
    provider.provider_settings_apply(serde_yaml::from_str(yaml).unwrap())
}

#[test]
fn settings_are_transactional_and_preserve_partial_secrets() {
    let mut p = ClaudeCodeProvider::default();
    apply(&mut p, "backend: anthropic\napi_key: saved\nenabled: true").unwrap();
    for invalid in [
        "api_key: replacement\nbackend: invalid",
        "enabled: false\nauth_type: invalid",
        "enabled: false\nbase_url: ftp://example.com",
        "api_key: [bad]",
        "aws_region: '$BAD NAME'",
        "enabled_models: [123]",
    ] {
        let before = serde_json::to_value(&p).unwrap();
        assert!(apply(&mut p, invalid).is_err(), "{invalid}");
        assert_eq!(serde_json::to_value(&p).unwrap(), before);
    }
    apply(&mut p, "api_key: '***'\naws_secret_access_key: hidden").unwrap();
    assert_eq!(p.transport.api_key.as_deref(), Some("saved"));
    assert_eq!(p.provider_settings_as_json()["api_key"], "***");
    assert_eq!(
        p.provider_settings_as_json()["aws_secret_access_key"],
        "***"
    );
    assert!(!format!("{p:?}").contains("saved"));
    apply(&mut p, "api_key: ''").unwrap();
    assert_eq!(p.provider_settings_as_json()["api_key"], "");
}

#[test]
fn environment_references_are_preserved_without_mutation() {
    let env = std::collections::HashMap::from([("KEY", "value"), ("NATIVE", "fallback")]);
    let lookup = |name: &str| env.get(name).map(|v| v.to_string());
    assert_eq!(
        crate::claude_backend::resolve_with(&Some("$KEY".into()), &["NATIVE"], lookup),
        Some("value".into())
    );
    assert_eq!(
        crate::claude_backend::resolve_with(&Some("$MISSING".into()), &["NATIVE"], lookup),
        None
    );
    assert_eq!(
        crate::claude_backend::resolve_with(&None, &["NATIVE"], lookup),
        Some("fallback".into())
    );
    let mut p = ClaudeCodeProvider::default();
    apply(&mut p, "backend: custom\nauth_type: bearer\nauth_token: '$REFACT_TEST_MISSING_CLAUDE_TOKEN_7281'\nbase_url: https://localhost:8123").unwrap();
    assert_eq!(
        p.transport.auth_token.as_deref(),
        Some("$REFACT_TEST_MISSING_CLAUDE_TOKEN_7281")
    );
    assert!(p.resolved_transport().auth_token.is_none());
    assert_eq!(p.provider_settings_as_json()["auth_token"], "***");
    assert!(std::env::var("REFACT_TEST_MISSING_CLAUDE_TOKEN_7281").is_err());
    let original = p.clone();
    apply(&mut p, "auth_token: replacement").unwrap();
    assert_ne!(p.transport.auth_token, original.transport.auth_token);
}

#[test]
fn non_subscription_runtime_is_typed_and_never_uses_oauth_heuristic() {
    for backend in ["anthropic", "bedrock", "vertex", "foundry", "custom"] {
        let mut p = ClaudeCodeProvider::default();
        apply(&mut p, &format!("backend: {backend}\napi_key: direct-key\nauth_type: none\nbase_url: https://example.com\nenabled: true\nenabled_models: [explicit-id]")).unwrap();
        p.oauth_tokens.access_token = "stale-oauth".into();
        assert!(!p.is_subscription());
        assert!(p.resolve_auth().is_err());
        assert_eq!(p.provider_settings_as_json()["oauth_connected"], false);
        let runtime = p.build_runtime().unwrap();
        assert!(runtime.enabled);
        assert!(runtime.auth_token.is_empty());
        assert!(runtime.api_key.is_empty());
        let t: ClaudeTransportConfig =
            serde_json::from_str(&runtime.extra_headers[CLAUDE_TRANSPORT_HEADER]).unwrap();
        assert_eq!(serde_json::to_value(t).unwrap()["backend"], backend);
        assert!(!runtime
            .extra_headers
            .values()
            .any(|v| v.contains("stale-oauth")));
    }
}

#[test]
fn backend_switch_retains_settings_but_excludes_irrelevant_credentials() {
    let mut p = ClaudeCodeProvider::default();
    apply(
        &mut p,
        "backend: anthropic\napi_key: secret\naws_secret_access_key: aws-secret",
    )
    .unwrap();
    apply(&mut p, "backend: vertex\ngcp_project: my-project").unwrap();
    assert!(p.resolved_transport().api_key.is_none());
    assert!(p.resolved_transport().aws_secret_access_key.is_none());
    assert_eq!(p.transport.api_key.as_deref(), Some("secret"));
    assert!(p.has_credentials());
    apply(&mut p, "backend: oauth").unwrap();
    assert!(!p.has_credentials());
    assert!(!p
        .build_runtime()
        .unwrap()
        .extra_headers
        .contains_key(CLAUDE_TRANSPORT_HEADER));
}

#[tokio::test]
async fn cloud_discovery_and_startup_skip_native_credentials_and_requests() {
    for backend in ["bedrock", "vertex", "foundry", "custom"] {
        let mut p = ClaudeCodeProvider::default();
        apply(
            &mut p,
            &format!("backend: {backend}\nauth_type: none\nbase_url: http://127.0.0.1:1"),
        )
        .unwrap();
        p.oauth_tokens.access_token = "expired".into();
        p.oauth_tokens.refresh_token = "must-not-refresh".into();
        let client = reqwest::Client::new();
        assert!(p
            .fetch_available_models(&client, &Default::default())
            .await
            .is_empty());
        p.startup_refresh_and_sync(&client, std::path::Path::new("/nonexistent"), "unused")
            .await
            .unwrap();
    }
}

#[test]
fn bedrock_never_inherits_a_custom_or_foundry_bearer() {
    for backend in ["custom", "foundry"] {
        let mut provider = ClaudeCodeProvider::default();
        apply(
            &mut provider,
            &format!("backend: {backend}\nauth_type: bearer\nauth_token: previous-provider-token"),
        )
        .unwrap();
        apply(&mut provider, "backend: bedrock\naws_region: us-east-1").unwrap();
        assert_ne!(
            provider.resolved_transport().auth_token.as_deref(),
            Some("previous-provider-token")
        );
        assert_eq!(
            provider.transport.auth_token.as_deref(),
            Some("previous-provider-token")
        );
    }
}

#[test]
fn legacy_anthropic_loads_and_preserves_identity() {
    let mut p: crate::anthropic::AnthropicProvider =
        serde_yaml::from_str("api_key: old-key\nenabled: true\nenabled_models: [claude-sonnet-4]")
            .unwrap();
    assert_eq!(p.provider_settings_as_json()["backend"], "anthropic");
    let runtime = p.build_runtime().unwrap();
    assert_eq!(runtime.name, "anthropic");
    let transport: ClaudeTransportConfig =
        serde_json::from_str(&runtime.extra_headers[CLAUDE_TRANSPORT_HEADER]).unwrap();
    assert_eq!(
        transport.base_url.as_deref(),
        Some("https://api.anthropic.com")
    );
    assert_eq!(p.base_provider_name(), "anthropic");
    assert!(p
        .provider_settings_apply(serde_yaml::from_str("backend: vertex\napi_key: replace").unwrap())
        .is_err());
    assert_eq!(p.api_key, "old-key");
}

#[test]
fn native_cloud_endpoints_and_foundry_credentials_are_backend_scoped() {
    let mut provider = ClaudeCodeProvider::default();
    apply(&mut provider, "backend: custom\nauth_type: bearer\nauth_token: custom-token\napi_key: custom-key\nbase_url: http://custom.invalid").unwrap();
    for backend in ["bedrock", "vertex"] {
        apply(&mut provider, &format!("backend: {backend}")).unwrap();
        assert!(provider.resolved_transport().base_url.is_none());
    }
    apply(&mut provider, "backend: foundry\nazure_resource: resource").unwrap();
    let transport = provider.resolved_transport();
    assert_ne!(transport.api_key.as_deref(), Some("custom-key"));
    assert_ne!(transport.auth_token.as_deref(), Some("custom-token"));
    assert_ne!(transport.base_url.as_deref(), Some("http://custom.invalid"));
    apply(&mut provider, "foundry_api_key: azure-key\nfoundry_auth_token: azure-token\nfoundry_base_url: https://azure.invalid/anthropic").unwrap();
    let transport = provider.resolved_transport();
    assert_eq!(transport.api_key.as_deref(), Some("azure-key"));
    assert_eq!(transport.auth_token.as_deref(), Some("azure-token"));
    assert_eq!(
        transport.base_url.as_deref(),
        Some("https://azure.invalid/anthropic")
    );
    assert_eq!(
        provider.provider_settings_as_json()["foundry_api_key"],
        "***"
    );
    assert_eq!(
        provider.provider_settings_as_json()["foundry_auth_token"],
        "***"
    );
    apply(&mut provider, "backend: custom").unwrap();
    assert_eq!(
        provider.resolved_transport().auth_token.as_deref(),
        Some("custom-token")
    );
    assert_eq!(
        provider.resolved_transport().base_url.as_deref(),
        Some("http://custom.invalid")
    );
}

#[test]
fn schema_includes_six_backends_and_secret_metadata() {
    let schema: serde_yaml::Value = serde_yaml::from_str(crate::claude_backend::SCHEMA).unwrap();
    assert_eq!(
        schema["fields"]["backend"]["f_options"]
            .as_sequence()
            .unwrap()
            .len(),
        6
    );
    for field in crate::claude_backend::SECRET_FIELDS {
        assert_eq!(schema["fields"][*field]["f_secret"].as_bool(), Some(true));
    }
}
