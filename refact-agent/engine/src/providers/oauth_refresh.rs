use std::collections::HashSet;
use std::sync::Arc;

use crate::global_context::GlobalContext;
use crate::providers::config_store;
use crate::providers::traits::ProviderTrait;
pub use refact_providers::oauth_refresh::{
    is_invalid_refresh_token, is_permanent_refresh_error, mark_invalid_refresh_token,
};

const REFRESH_CHECK_INTERVAL_SECS: u64 = 60;
const REFRESH_BEFORE_EXPIRY_MS: i64 = 5 * 60 * 1000;

lazy_static::lazy_static! {
    static ref OAUTH_FAILED_INSTANCES: std::sync::Mutex<HashSet<String>> =
        std::sync::Mutex::new(HashSet::new());
}

fn mark_oauth_failure(instance_id: &str) -> bool {
    OAUTH_FAILED_INSTANCES
        .lock()
        .map(|mut failures| failures.insert(instance_id.to_string()))
        .unwrap_or(true)
}

fn clear_oauth_failure(instance_id: &str) -> bool {
    OAUTH_FAILED_INSTANCES
        .lock()
        .map(|mut failures| failures.remove(instance_id))
        .unwrap_or(false)
}

#[cfg(test)]
fn oauth_failed_instance_count_for_test() -> usize {
    OAUTH_FAILED_INSTANCES
        .lock()
        .map(|failures| failures.len())
        .unwrap_or(0)
}

#[cfg(test)]
fn clear_refresh_tracking_for_test() {
    if let Ok(mut failures) = OAUTH_FAILED_INSTANCES.lock() {
        failures.clear();
    }
}

#[cfg(test)]
fn collect_oauth_refresh_instances_for_base(
    providers: Vec<(String, String)>,
    base_provider: &str,
) -> Vec<String> {
    providers
        .into_iter()
        .filter_map(|(instance_id, base)| (base == base_provider).then_some(instance_id))
        .collect()
}

#[derive(Clone)]
struct OAuthRefreshCandidate<T> {
    instance_id: String,
    display_name: String,
    oauth_tokens: T,
}

pub async fn oauth_token_refresh_background_task(gcx: Arc<GlobalContext>) {
    let _ = try_refresh_all_providers(&gcx).await;
    loop {
        let shutdown_flag = gcx.shutdown_flag.clone();
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(REFRESH_CHECK_INTERVAL_SECS)) => {}
            _ = async {
                while !shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            } => {
                tracing::info!("OAuth token refresh: shutdown detected, stopping");
                return;
            }
        }
        let _ = try_refresh_all_providers(&gcx).await;
    }
}

async fn try_refresh_all_providers(gcx: &Arc<GlobalContext>) -> () {
    let (http_client, config_dir) = { (gcx.http_client.clone(), gcx.config_dir.clone()) };

    try_refresh_claude_code_instances(gcx, &http_client).await;
    try_refresh_openai_codex_instances(gcx, &http_client, &config_dir).await;
}

async fn try_refresh_claude_code_instances(
    gcx: &Arc<GlobalContext>,
    http_client: &reqwest::Client,
) {
    let candidates = {
        let registry = gcx.providers.read().await;
        registry
            .iter()
            .filter(|(_, provider)| provider.base_provider_name() == "claude_code")
            .filter_map(|(_, provider)| {
                let oauth_tokens = provider
                    .as_any()
                    .downcast_ref::<crate::providers::claude_code::ClaudeCodeProvider>()?
                    .oauth_tokens
                    .clone();
                Some(OAuthRefreshCandidate {
                    instance_id: provider.name().to_string(),
                    display_name: provider.display_name().to_string(),
                    oauth_tokens,
                })
            })
            .collect::<Vec<_>>()
    };

    for candidate in candidates {
        try_refresh_claude_code(gcx, http_client, candidate).await;
    }
}

async fn try_refresh_claude_code(
    gcx: &Arc<GlobalContext>,
    http_client: &reqwest::Client,
    candidate: OAuthRefreshCandidate<crate::providers::claude_code_oauth::OAuthTokens>,
) {
    let oauth_tokens = candidate.oauth_tokens;
    let instance_id = candidate.instance_id;
    let display_name = candidate.display_name;

    if oauth_tokens.is_empty() || oauth_tokens.refresh_token.is_empty() {
        return;
    }

    if !needs_refresh(oauth_tokens.expires_at) {
        return;
    }

    if is_invalid_refresh_token(&instance_id, &oauth_tokens.refresh_token) {
        return;
    }

    tracing::info!(
        "{}: refreshing OAuth token (expires_at={})",
        display_name,
        oauth_tokens.expires_at
    );

    match force_refresh_claude_code_for_retry(
        gcx,
        http_client,
        &instance_id,
        &oauth_tokens.access_token,
        Some(oauth_tokens.expires_at),
        None,
    )
    .await
    {
        Ok(Some(_)) => {
            tracing::info!("{}: OAuth token refreshed successfully", display_name);
            if clear_oauth_failure(&instance_id) {
                let ev = crate::buddy::actor::make_runtime_event(
                    "connection_restored",
                    &format!("{}: OAuth token refreshed", display_name),
                    "provider",
                    &format!("oauth_{}", instance_id),
                    "completed",
                    None,
                );
                crate::buddy::actor::buddy_enqueue_event(
                    crate::app_state::AppState::from_gcx((*gcx).clone()).await,
                    ev,
                )
                .await;
            }
        }
        Ok(None) => {}
        Err(e) => {
            let first_failure = mark_oauth_failure(&instance_id);
            if is_permanent_refresh_error(&e) {
                mark_invalid_refresh_token(&instance_id, &oauth_tokens.refresh_token);
                if first_failure {
                    tracing::warn!(
                        "{}: OAuth refresh token is invalid; clearing saved OAuth tokens. Please log in again: {}",
                        display_name,
                        e
                    );
                } else {
                    tracing::debug!(
                        "{}: OAuth refresh token is still invalid: {}",
                        display_name,
                        e
                    );
                }
                if first_failure {
                    let ev = crate::buddy::actor::make_runtime_event(
                        "connection_lost",
                        &format!("{} OAuth expired — please log in again", display_name),
                        "provider",
                        &format!("oauth_{}", instance_id),
                        "failed",
                        Some("high"),
                    );
                    crate::buddy::actor::buddy_enqueue_event(
                        crate::app_state::AppState::from_gcx((*gcx).clone()).await,
                        ev,
                    )
                    .await;
                }
                return;
            }
            if first_failure {
                tracing::warn!("{}: OAuth token refresh failed: {}", display_name, e);
                let ev = crate::buddy::actor::make_runtime_event(
                    "connection_lost",
                    &format!("{}: OAuth refresh failed", display_name),
                    "provider",
                    &format!("oauth_{}", instance_id),
                    "failed",
                    Some("high"),
                );
                crate::buddy::actor::buddy_enqueue_event(
                    crate::app_state::AppState::from_gcx((*gcx).clone()).await,
                    ev,
                )
                .await;
            } else {
                tracing::debug!("{}: OAuth token refresh still failing: {}", display_name, e);
            }
        }
    }
}

pub async fn force_refresh_claude_code_for_retry(
    gcx: &Arc<GlobalContext>,
    http_client: &reqwest::Client,
    provider_name: &str,
    rejected_access_token: &str,
    rejected_expires_at: Option<i64>,
    rejected_status: Option<reqwest::StatusCode>,
) -> Result<Option<crate::providers::claude_code::ClaudeCodeProvider>, String> {
    let _guard = crate::providers::claude_code::ClaudeCodeProvider::lock_refresh_guard().await?;
    let (mut provider, config_dir) = {
        let registry = gcx.providers.read().await;
        let provider = registry
            .get(provider_name)
            .and_then(|provider| {
                provider
                    .as_any()
                    .downcast_ref::<crate::providers::claude_code::ClaudeCodeProvider>()
            })
            .cloned();
        (provider, gcx.config_dir.clone())
    };
    let Some(mut provider) = provider.take() else {
        return Ok(None);
    };

    if provider
        .access_token_changed_since_rejection(rejected_access_token)
        .is_some()
    {
        return Ok(Some(provider));
    }
    if rejected_expires_at.is_some_and(|expires_at| provider.oauth_tokens.expires_at > expires_at) {
        return Ok(Some(provider));
    }

    if let Some(status) = rejected_status {
        if !crate::providers::claude_code::ClaudeCodeProvider::should_force_refresh_for_status(
            status,
            &provider.oauth_tokens.refresh_token,
            false,
        ) {
            return Ok(None);
        }
    } else {
        if !needs_refresh(provider.oauth_tokens.expires_at) {
            return Ok(Some(provider));
        }
        if provider.oauth_tokens.refresh_token.is_empty() {
            return Ok(None);
        }
    }

    let previous_tokens = provider.oauth_tokens.clone();
    let refresh_result = provider
        .refresh_access_token_and_persist(http_client, &config_dir, provider_name)
        .await;

    if !provider.auth_state_matches(&previous_tokens) {
        let changed = {
            let mut registry = gcx.providers.write().await;
            registry
                .get_mut(provider_name)
                .and_then(|current| {
                    current
                        .as_any_mut()
                        .downcast_mut::<crate::providers::claude_code::ClaudeCodeProvider>()
                })
                .map(|current| {
                    current.update_auth_state_from_if_current(&provider, &previous_tokens)
                })
                .unwrap_or(false)
        };

        if changed {
            let mut caps = gcx.caps_state.write().await;
            caps.caps = None;
            caps.last_attempted_ts = 0;
        }
    }

    refresh_result.map(|access_token| access_token.map(|_| provider))
}

async fn try_refresh_openai_codex_instances(
    gcx: &Arc<GlobalContext>,
    http_client: &reqwest::Client,
    config_dir: &std::path::Path,
) {
    let candidates = {
        let registry = gcx.providers.read().await;
        registry
            .iter()
            .filter(|(_, provider)| provider.base_provider_name() == "openai_codex")
            .filter_map(|(_, provider)| {
                let oauth_tokens = provider
                    .as_any()
                    .downcast_ref::<crate::providers::openai_codex::OpenAICodexProvider>()?
                    .oauth_tokens
                    .clone();
                Some(OAuthRefreshCandidate {
                    instance_id: provider.name().to_string(),
                    display_name: provider.display_name().to_string(),
                    oauth_tokens,
                })
            })
            .collect::<Vec<_>>()
    };

    for candidate in candidates {
        try_refresh_openai_codex(gcx, http_client, config_dir, candidate).await;
    }
}

async fn try_refresh_openai_codex(
    gcx: &Arc<GlobalContext>,
    http_client: &reqwest::Client,
    config_dir: &std::path::Path,
    candidate: OAuthRefreshCandidate<crate::providers::openai_codex_oauth::OAuthTokens>,
) {
    let instance_id = candidate.instance_id;
    let display_name = candidate.display_name;
    let candidate_tokens = candidate.oauth_tokens;

    if candidate_tokens.is_empty() || candidate_tokens.refresh_token.is_empty() {
        return;
    }

    if !needs_refresh(candidate_tokens.expires_at) {
        return;
    }

    if is_invalid_refresh_token(&instance_id, &candidate_tokens.refresh_token) {
        return;
    }

    let _guard =
        match crate::providers::openai_codex::OpenAICodexProvider::lock_refresh_guard().await {
            Ok(guard) => guard,
            Err(error) => {
                tracing::warn!(
                    "{}: failed to acquire OAuth refresh guard: {}",
                    display_name,
                    error
                );
                return;
            }
        };
    let Some(oauth_tokens) = ({
        let registry = gcx.providers.read().await;
        registry
            .get(&instance_id)
            .and_then(|provider| {
                provider
                    .as_any()
                    .downcast_ref::<crate::providers::openai_codex::OpenAICodexProvider>()
            })
            .map(|provider| provider.oauth_tokens.clone())
    }) else {
        return;
    };
    if oauth_tokens.is_empty()
        || oauth_tokens.refresh_token.is_empty()
        || !needs_refresh(oauth_tokens.expires_at)
        || is_invalid_refresh_token(&instance_id, &oauth_tokens.refresh_token)
    {
        return;
    }

    tracing::info!(
        "{}: refreshing OAuth token (expires_at={})",
        display_name,
        oauth_tokens.expires_at
    );

    match crate::providers::openai_codex_oauth::refresh_access_token(
        http_client,
        &oauth_tokens.refresh_token,
    )
    .await
    {
        Ok(new_tokens) => {
            tracing::info!("{}: OAuth token refreshed successfully", display_name);
            let saved = match save_refreshed_tokens(
                gcx,
                config_dir,
                &instance_id,
                &oauth_tokens,
                &new_tokens.access_token,
                &new_tokens.refresh_token,
                new_tokens.expires_at,
            )
            .await
            {
                Ok(saved) => saved,
                Err(e) => {
                    tracing::warn!("{}: failed to save refreshed tokens: {}", display_name, e);
                    false
                }
            };
            if saved && clear_oauth_failure(&instance_id) {
                let ev = crate::buddy::actor::make_runtime_event(
                    "connection_restored",
                    &format!("{}: OAuth token refreshed", display_name),
                    "provider",
                    &format!("oauth_{}", instance_id),
                    "completed",
                    None,
                );
                crate::buddy::actor::buddy_enqueue_event(
                    crate::app_state::AppState::from_gcx((*gcx).clone()).await,
                    ev,
                )
                .await;
            }
        }
        Err(e) => {
            let first_failure = mark_oauth_failure(&instance_id);
            if is_permanent_refresh_error(&e) {
                mark_invalid_refresh_token(&instance_id, &oauth_tokens.refresh_token);
                if first_failure {
                    tracing::warn!(
                        "{}: OAuth refresh token is invalid; clearing saved refresh token. Please log in again if Codex stops working: {}",
                        display_name,
                        e
                    );
                } else {
                    tracing::debug!(
                        "{}: OAuth refresh token is still invalid: {}",
                        display_name,
                        e
                    );
                }
                let cleared = match save_refreshed_tokens(
                    gcx,
                    config_dir,
                    &instance_id,
                    &oauth_tokens,
                    "",
                    "",
                    0,
                )
                .await
                {
                    Ok(cleared) => cleared,
                    Err(save_err) => {
                        tracing::warn!(
                            "{}: failed to clear invalid OAuth refresh token: {}",
                            display_name,
                            save_err
                        );
                        false
                    }
                };
                if first_failure && cleared {
                    let ev = crate::buddy::actor::make_runtime_event(
                        "connection_lost",
                        &format!(
                            "{} OAuth expired — please log in again if needed",
                            display_name
                        ),
                        "provider",
                        &format!("oauth_{}", instance_id),
                        "failed",
                        Some("high"),
                    );
                    crate::buddy::actor::buddy_enqueue_event(
                        crate::app_state::AppState::from_gcx((*gcx).clone()).await,
                        ev,
                    )
                    .await;
                }
                return;
            }
            if first_failure {
                tracing::warn!("{}: OAuth token refresh failed: {}", display_name, e);
                let ev = crate::buddy::actor::make_runtime_event(
                    "connection_lost",
                    &format!("{}: OAuth refresh failed", display_name),
                    "provider",
                    &format!("oauth_{}", instance_id),
                    "failed",
                    Some("high"),
                );
                crate::buddy::actor::buddy_enqueue_event(
                    crate::app_state::AppState::from_gcx((*gcx).clone()).await,
                    ev,
                )
                .await;
            } else {
                tracing::debug!("{}: OAuth token refresh still failing: {}", display_name, e);
            }
        }
    }
}

fn needs_refresh(expires_at: i64) -> bool {
    if expires_at == 0 {
        return true;
    }
    let now_ms = chrono::Utc::now().timestamp_millis();
    now_ms >= expires_at - REFRESH_BEFORE_EXPIRY_MS
}

pub(crate) async fn save_refreshed_tokens(
    gcx: &Arc<GlobalContext>,
    config_dir: &std::path::Path,
    provider_name: &str,
    expected_tokens: &crate::providers::openai_codex_oauth::OAuthTokens,
    access_token: &str,
    refresh_token: &str,
    expires_at: i64,
) -> Result<bool, String> {
    let registry_matches = {
        let registry = gcx.providers.read().await;
        registry
            .get(provider_name)
            .and_then(|provider| {
                provider
                    .as_any()
                    .downcast_ref::<crate::providers::openai_codex::OpenAICodexProvider>()
            })
            .is_some_and(|provider| &provider.oauth_tokens == expected_tokens)
    };
    if !registry_matches {
        return Ok(false);
    }

    let updated = config_store::update_provider_config_if(config_dir, provider_name, |existing| {
        let Some(value) = existing else {
            return Ok(None);
        };
        let mut yaml_map = value.as_mapping().cloned().ok_or_else(|| {
            "Config file root is not a YAML mapping. Cannot safely patch.".to_string()
        })?;

        let mut tokens_map = yaml_map
            .get(&serde_yaml::Value::String("oauth_tokens".to_string()))
            .and_then(|v| v.as_mapping())
            .cloned()
            .unwrap_or_default();
        let current_tokens: crate::providers::openai_codex_oauth::OAuthTokens =
            serde_yaml::from_value(serde_yaml::Value::Mapping(tokens_map.clone()))
                .map_err(|error| format!("Failed to parse existing OAuth tokens: {}", error))?;
        if &current_tokens != expected_tokens {
            return Ok(None);
        }

        tokens_map.insert(
            serde_yaml::Value::String("access_token".to_string()),
            serde_yaml::Value::String(access_token.to_string()),
        );
        tokens_map.insert(
            serde_yaml::Value::String("refresh_token".to_string()),
            serde_yaml::Value::String(refresh_token.to_string()),
        );
        tokens_map.insert(
            serde_yaml::Value::String("expires_at".to_string()),
            serde_yaml::Value::Number(serde_yaml::Number::from(expires_at)),
        );

        yaml_map.insert(
            serde_yaml::Value::String("oauth_tokens".to_string()),
            serde_yaml::Value::Mapping(tokens_map),
        );

        Ok(Some(serde_yaml::Value::Mapping(yaml_map)))
    })
    .await?;
    if updated.is_none() {
        return Ok(false);
    }

    let changed = {
        let mut registry = gcx.providers.write().await;
        registry
            .get_mut(provider_name)
            .and_then(|provider| {
                provider
                    .as_any_mut()
                    .downcast_mut::<crate::providers::openai_codex::OpenAICodexProvider>()
            })
            .filter(|provider| &provider.oauth_tokens == expected_tokens)
            .map(|provider| {
                provider.apply_oauth_refresh_tokens(access_token, refresh_token, expires_at);
            })
            .is_some()
    };

    if changed {
        let caps_state = gcx.caps_state.clone();
        let mut caps_state = caps_state.write().await;
        caps_state.caps = None;
        caps_state.last_attempted_ts = 0;
    }

    Ok(changed)
}

#[cfg(test)]
mod tests {
    lazy_static::lazy_static! {
        static ref REFRESH_TRACKING_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    }

    fn refresh_tracking_test_guard() -> std::sync::MutexGuard<'static, ()> {
        REFRESH_TRACKING_TEST_LOCK
            .lock()
            .expect("refresh tracking test lock poisoned")
    }

    #[test]
    fn permanent_refresh_error_detects_invalid_grant() {
        assert!(super::is_permanent_refresh_error(
            r#"Token refresh failed (400 Bad Request): {"error":"invalid_grant"}"#
        ));
        assert!(super::is_permanent_refresh_error("INVALID_GRANT"));
        assert!(super::is_permanent_refresh_error("Invalid_Grant"));
        assert!(super::is_permanent_refresh_error(
            r#"Token refresh failed (400 Bad Request): {"error":{"code":"Invalid_Grant"}}"#
        ));
    }

    #[test]
    fn permanent_refresh_error_ignores_transient_failure() {
        for error in [
            "Token refresh request failed: operation timed out",
            "Token refresh failed (500 Internal Server Error)",
            "network connection reset by peer",
        ] {
            assert!(!super::is_permanent_refresh_error(error), "{error}");
        }
    }

    #[test]
    fn invalid_refresh_token_tracking_is_per_instance() {
        let _guard = refresh_tracking_test_guard();
        super::clear_refresh_tracking_for_test();
        super::mark_invalid_refresh_token("openai_codex", "same-refresh-token-test");

        assert!(super::is_invalid_refresh_token(
            "openai_codex",
            "same-refresh-token-test"
        ));
        assert!(!super::is_invalid_refresh_token(
            "openai_codex_2",
            "same-refresh-token-test"
        ));

        super::clear_refresh_tracking_for_test();
    }

    #[test]
    fn oauth_failure_tracking_is_per_instance() {
        let _guard = refresh_tracking_test_guard();
        super::clear_refresh_tracking_for_test();

        assert!(super::mark_oauth_failure("claude_code"));
        assert!(super::mark_oauth_failure("claude_code_2"));
        assert!(!super::mark_oauth_failure("claude_code"));
        assert_eq!(super::oauth_failed_instance_count_for_test(), 2);
        assert!(super::clear_oauth_failure("claude_code"));
        assert_eq!(super::oauth_failed_instance_count_for_test(), 1);

        super::clear_refresh_tracking_for_test();
    }

    #[test]
    fn oauth_refresh_helper_collects_all_instances_for_base() {
        let providers = vec![
            ("claude_code".to_string(), "claude_code".to_string()),
            ("claude_code_work".to_string(), "claude_code".to_string()),
            ("openai_codex".to_string(), "openai_codex".to_string()),
        ];

        assert_eq!(
            super::collect_oauth_refresh_instances_for_base(providers, "claude_code"),
            vec!["claude_code".to_string(), "claude_code_work".to_string()]
        );
    }

    #[tokio::test]
    async fn claude_code_refresh_rereads_registry_and_skips_stale_token() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let current = crate::providers::claude_code::ClaudeCodeProvider {
            oauth_tokens: crate::providers::claude_code_oauth::OAuthTokens {
                access_token: "fresh-access".to_string(),
                refresh_token: "refresh".to_string(),
                expires_at: i64::MAX,
            },
            ..Default::default()
        };
        {
            let mut registry = gcx.providers.write().await;
            registry.add(Box::new(current));
        }
        let http_client = gcx.http_client.clone();

        let refreshed = super::force_refresh_claude_code_for_retry(
            &gcx,
            &http_client,
            "claude_code",
            "stale-access",
            None,
            Some(reqwest::StatusCode::UNAUTHORIZED),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(refreshed.oauth_tokens.access_token, "fresh-access");
    }

    #[tokio::test]
    async fn claude_code_proactive_refresh_skips_already_renewed_expiry() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let current = crate::providers::claude_code::ClaudeCodeProvider {
            oauth_tokens: crate::providers::claude_code_oauth::OAuthTokens {
                access_token: "same-access".to_string(),
                refresh_token: "refresh".to_string(),
                expires_at: i64::MAX,
            },
            ..Default::default()
        };
        {
            let mut registry = gcx.providers.write().await;
            registry.add(Box::new(current));
        }
        let http_client = gcx.http_client.clone();

        let refreshed = super::force_refresh_claude_code_for_retry(
            &gcx,
            &http_client,
            "claude_code",
            "same-access",
            Some(1),
            None,
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(refreshed.oauth_tokens.access_token, "same-access");
        assert_eq!(refreshed.oauth_tokens.expires_at, i64::MAX);
    }

    #[tokio::test]
    async fn claude_code_auth_retry_skips_same_token_with_newer_expiry() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let current = crate::providers::claude_code::ClaudeCodeProvider {
            oauth_tokens: crate::providers::claude_code_oauth::OAuthTokens {
                access_token: "same-access".to_string(),
                refresh_token: "refresh".to_string(),
                expires_at: i64::MAX,
            },
            ..Default::default()
        };
        {
            let mut registry = gcx.providers.write().await;
            registry.add(Box::new(current));
        }
        let http_client = gcx.http_client.clone();

        let refreshed = super::force_refresh_claude_code_for_retry(
            &gcx,
            &http_client,
            "claude_code",
            "same-access",
            Some(1),
            Some(reqwest::StatusCode::UNAUTHORIZED),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(refreshed.oauth_tokens.access_token, "same-access");
        assert_eq!(refreshed.oauth_tokens.expires_at, i64::MAX);
    }

    #[tokio::test]
    async fn refreshed_tokens_preserve_existing_provider_identity() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        let providers_dir = config_dir.join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(
            providers_dir.join("codex-prod.yaml"),
            "base_provider: openai_codex\ndisplay_name: Renamed Codex\noauth_tokens:\n  access_token: old\n  refresh_token: old-refresh\n  expires_at: 1\n",
        )
        .await
        .unwrap();
        let mut provider = crate::providers::openai_codex::OpenAICodexProvider::default();
        provider.oauth_tokens.access_token = "old".to_string();
        provider.oauth_tokens.refresh_token = "old-refresh".to_string();
        provider.oauth_tokens.expires_at = 1;
        let expected_tokens = provider.oauth_tokens.clone();
        {
            let mut registry = gcx.providers.write().await;
            registry.add(Box::new(crate::providers::instance::ProviderInstance::new(
                "codex-prod",
                "openai_codex",
                "Renamed Codex",
                Box::new(provider),
            )));
        }

        let saved = super::save_refreshed_tokens(
            &gcx,
            &config_dir,
            "codex-prod",
            &expected_tokens,
            "new",
            "new-refresh",
            i64::MAX,
        )
        .await
        .unwrap();
        assert!(saved);

        let yaml: serde_yaml::Value = serde_yaml::from_str(
            &tokio::fs::read_to_string(providers_dir.join("codex-prod.yaml"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(yaml["base_provider"].as_str(), Some("openai_codex"));
        assert_eq!(yaml["display_name"].as_str(), Some("Renamed Codex"));
        assert_eq!(yaml["oauth_tokens"]["access_token"].as_str(), Some("new"));
    }

    #[tokio::test]
    async fn stale_refreshed_tokens_do_not_overwrite_newer_login() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        let providers_dir = config_dir.join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(
            providers_dir.join("codex-prod.yaml"),
            "base_provider: openai_codex\ndisplay_name: Codex\noauth_tokens:\n  access_token: login-access\n  refresh_token: login-refresh\n  expires_at: 99\n",
        )
        .await
        .unwrap();
        let stale_tokens = crate::providers::openai_codex_oauth::OAuthTokens {
            access_token: "old-access".to_string(),
            refresh_token: "old-refresh".to_string(),
            expires_at: 1,
            ..Default::default()
        };
        let mut provider = crate::providers::openai_codex::OpenAICodexProvider::default();
        provider.oauth_tokens = crate::providers::openai_codex_oauth::OAuthTokens {
            access_token: "login-access".to_string(),
            refresh_token: "login-refresh".to_string(),
            expires_at: 99,
            ..Default::default()
        };
        {
            let mut registry = gcx.providers.write().await;
            registry.add(Box::new(crate::providers::instance::ProviderInstance::new(
                "codex-prod",
                "openai_codex",
                "Codex",
                Box::new(provider),
            )));
        }

        let saved = super::save_refreshed_tokens(
            &gcx,
            &config_dir,
            "codex-prod",
            &stale_tokens,
            "stale-access",
            "stale-refresh",
            i64::MAX,
        )
        .await
        .unwrap();

        assert!(!saved);
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            &tokio::fs::read_to_string(providers_dir.join("codex-prod.yaml"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            yaml["oauth_tokens"]["access_token"].as_str(),
            Some("login-access")
        );
    }

    #[tokio::test]
    async fn stale_refreshed_tokens_do_not_recreate_deleted_provider() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        let stale_tokens = crate::providers::openai_codex_oauth::OAuthTokens {
            access_token: "old-access".to_string(),
            refresh_token: "old-refresh".to_string(),
            expires_at: 1,
            ..Default::default()
        };

        let saved = super::save_refreshed_tokens(
            &gcx,
            &config_dir,
            "codex-prod",
            &stale_tokens,
            "stale-access",
            "stale-refresh",
            i64::MAX,
        )
        .await
        .unwrap();

        assert!(!saved);
        assert!(!config_dir.join("providers.d/codex-prod.yaml").exists());
        assert!(!gcx.providers.read().await.has_instance("codex-prod"));
    }
}
