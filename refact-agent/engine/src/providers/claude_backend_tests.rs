use super::*;

async fn json_body(response: Response<Body>) -> Value {
    serde_json::from_slice(&hyper::body::to_bytes(response.into_body()).await.unwrap()).unwrap()
}

async fn save_fixture(gcx: &Arc<GlobalContext>, name: &str, settings: Value) {
    tokio::fs::create_dir_all(gcx.config_dir.join("providers.d"))
        .await
        .unwrap();
    tokio::fs::write(
        provider_file_path(&gcx.config_dir, name),
        serde_yaml::to_string(&settings).unwrap(),
    )
    .await
    .unwrap();
    reload_provider_from_disk(gcx.clone(), name, &gcx.config_dir)
        .await
        .unwrap();
}

#[tokio::test]
async fn native_backends_reject_oauth_and_do_not_fetch_subscription_usage() {
    for backend in ["anthropic", "bedrock", "vertex", "foundry", "custom"] {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        save_fixture(&gcx, "work-claude", json!({
            "base_provider": "claude_code", "backend": backend,
            "auth_type": "none", "base_url": "https://example.invalid",
            "api_key": "fixture-native-key",
            "oauth_tokens": {"access_token": "fixture-stale-oauth", "refresh_token": "fixture-refresh"}
        })).await;
        let app = AppState::from_gcx(gcx.clone()).await;
        let path = || {
            Path(ProviderPathParams {
                name: "work-claude".into(),
            })
        };
        assert!(handle_v1_provider_oauth_start(
            State(app.clone()),
            path(),
            hyper::body::Bytes::new()
        )
        .await
        .is_err());
        assert!(handle_v1_provider_oauth_exchange(
            State(app.clone()),
            path(),
            hyper::body::Bytes::from(r#"{"session_id":"unused","code":"unused"}"#)
        )
        .await
        .is_err());
        assert!(handle_v1_provider_oauth_logout(State(app.clone()), path())
            .await
            .is_err());
        let status = json_body(
            handle_v1_provider_get(State(app.clone()), path())
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status["settings"]["oauth_connected"], false);
        let usage = json_body(
            handle_v1_provider_usage(State(app.clone()), path())
                .await
                .unwrap(),
        )
        .await;
        assert!(usage["error"].as_str().unwrap().contains("unsupported"));
        let resets = json_body(
            handle_v1_provider_usage_resets(State(app.clone()), path())
                .await
                .unwrap(),
        )
        .await;
        assert!(resets["error"].as_str().unwrap().contains("unsupported"));
        assert!(handle_v1_provider_usage_redeem(
            State(app.clone()),
            path(),
            hyper::body::Bytes::new()
        )
        .await
        .is_err());
        let mut cached_subscription =
            ProviderQuotaSnapshot::unavailable("work-claude", "claude_code");
        cached_subscription.available = true;
        cached_subscription.source = ProviderQuotaSource::ClaudeCode;
        gcx.quota_cache
            .lock()
            .await
            .insert("work-claude".into(), cached_subscription);
        let quota = provider_quota_cached(gcx.clone(), "work-claude", "claude_code", false).await;
        assert!(!quota.available);
        assert_eq!(quota.source, ProviderQuotaSource::Unavailable);
        let before = tokio::fs::read(provider_file_path(&gcx.config_dir, "work-claude"))
            .await
            .unwrap();
        let error = save_provider_oauth_tokens(
            &gcx,
            &gcx.config_dir,
            "work-claude",
            "claude_code",
            &serde_yaml::to_value(json!({"access_token":"late-callback"})).unwrap(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.status_code, StatusCode::CONFLICT);
        assert_eq!(
            before,
            tokio::fs::read(provider_file_path(&gcx.config_dir, "work-claude"))
                .await
                .unwrap()
        );
    }
}

#[tokio::test]
async fn partial_native_save_preserves_secrets_identity_and_rejects_invalid_transaction() {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    let secrets = [
        "api_key",
        "auth_token",
        "aws_access_key_id",
        "aws_secret_access_key",
        "aws_session_token",
        "foundry_api_key",
        "foundry_auth_token",
    ];
    let mut fixture = json!({"base_provider":"claude_code", "display_name":"My private chain", "backend":"bedrock", "aws_region":"us-east-1", "enabled_models":["custom-model"], "unknown_hidden_field":"keep"});
    for key in secrets {
        fixture[key] = json!(format!("fixture-{key}"));
    }
    save_fixture(&gcx, "work-claude", fixture).await;
    let app = AppState::from_gcx(gcx.clone()).await;
    let path = || {
        Path(ProviderPathParams {
            name: "work-claude".into(),
        })
    };
    let incoming = json!({"backend":"custom", "auth_type":"none", "base_url":"https://example.invalid", "api_key":"***", "auth_token":"", "aws_secret_access_key":"***"});
    handle_v1_provider_update(
        State(app.clone()),
        path(),
        hyper::body::Bytes::from(serde_json::to_vec(&incoming).unwrap()),
    )
    .await
    .unwrap();
    let file = provider_file_path(&gcx.config_dir, "work-claude");
    let before = tokio::fs::read(&file).await.unwrap();
    let saved: serde_yaml::Value = serde_yaml::from_slice(&before).unwrap();
    assert_eq!(saved["backend"].as_str(), Some("custom"));
    assert_eq!(saved["auth_token"].as_str(), Some(""));
    for key in [
        "api_key",
        "aws_access_key_id",
        "aws_secret_access_key",
        "aws_session_token",
        "foundry_api_key",
        "foundry_auth_token",
    ] {
        assert_eq!(saved[key].as_str(), Some(format!("fixture-{key}").as_str()));
    }
    assert_eq!(saved["unknown_hidden_field"].as_str(), Some("keep"));
    assert_eq!(saved["display_name"].as_str(), Some("My private chain"));
    let public = json_body(
        handle_v1_provider_get(State(app.clone()), path())
            .await
            .unwrap(),
    )
    .await
    .to_string();
    for key in secrets {
        assert!(!public.contains(&format!("fixture-{key}")));
    }
    for invalid in [
        json!({"backend":"fixture-secret-invalid"}),
        json!({"auth_type":"fixture-secret-invalid"}),
        json!({"extra_headers":{"X-Refact-Internal-Claude-Transport":"injection"}}),
    ] {
        let error = handle_v1_provider_update(
            State(app.clone()),
            path(),
            hyper::body::Bytes::from(serde_json::to_vec(&invalid).unwrap()),
        )
        .await
        .unwrap_err();
        assert!(!error.message.contains("fixture-secret-invalid"));
        assert_eq!(before, tokio::fs::read(&file).await.unwrap());
    }
}

#[tokio::test]
async fn legacy_anthropic_alias_keeps_identity_and_models_without_oauth() {
    let gcx = crate::global_context::tests::make_test_gcx().await;
    save_fixture(&gcx, "anthropic-work", json!({"base_provider":"anthropic", "display_name":"Legacy work", "api_key":"fixture-key", "enabled_models":["claude-sonnet-4-6"]})).await;
    let app = AppState::from_gcx(gcx.clone()).await;
    let path = || {
        Path(ProviderPathParams {
            name: "anthropic-work".into(),
        })
    };
    let detail = json_body(
        handle_v1_provider_get(State(app.clone()), path())
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(detail["name"], "anthropic-work");
    assert_eq!(detail["base_provider"], "anthropic");
    assert_eq!(detail["display_name"], "Legacy work");
    assert!(handle_v1_provider_oauth_logout(State(app.clone()), path())
        .await
        .is_err());
    assert!(handle_v1_provider_models(State(app), path()).await.is_ok());
}
