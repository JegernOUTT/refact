use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::global_context::GlobalContext;
use crate::providers::create_provider;
use crate::providers::config_store;

const REFRESH_CHECK_INTERVAL_SECS: u64 = 60;
const REFRESH_BEFORE_EXPIRY_MS: i64 = 5 * 60 * 1000;

lazy_static::lazy_static! {
    static ref INVALID_REFRESH_TOKENS: std::sync::Mutex<HashSet<String>> =
        std::sync::Mutex::new(HashSet::new());
    static ref OAUTH_FAILED_INSTANCES: std::sync::Mutex<HashSet<String>> =
        std::sync::Mutex::new(HashSet::new());
}

pub fn is_permanent_refresh_error(error: &str) -> bool {
    if let Some(value) = extract_json_object(error) {
        if json_contains_invalid_grant(&value) {
            return true;
        }
    }
    error.to_ascii_lowercase().contains("invalid_grant")
}

fn extract_json_object(text: &str) -> Option<serde_json::Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end < start {
        return None;
    }
    serde_json::from_str(&text[start..=end]).ok()
}

fn json_contains_invalid_grant(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => text.eq_ignore_ascii_case("invalid_grant"),
        serde_json::Value::Array(values) => values.iter().any(json_contains_invalid_grant),
        serde_json::Value::Object(map) => map.values().any(json_contains_invalid_grant),
        _ => false,
    }
}

pub fn mark_invalid_refresh_token(provider_name: &str, refresh_token: &str) {
    if refresh_token.is_empty() {
        return;
    }
    if let Ok(mut tokens) = INVALID_REFRESH_TOKENS.lock() {
        tokens.insert(refresh_token_key(provider_name, refresh_token));
    }
}

fn is_invalid_refresh_token(provider_name: &str, refresh_token: &str) -> bool {
    if refresh_token.is_empty() {
        return false;
    }
    INVALID_REFRESH_TOKENS
        .lock()
        .map(|tokens| tokens.contains(&refresh_token_key(provider_name, refresh_token)))
        .unwrap_or(false)
}

fn refresh_token_key(provider_name: &str, refresh_token: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    provider_name.hash(&mut hasher);
    refresh_token.hash(&mut hasher);
    format!("{}:{:x}", provider_name, hasher.finish())
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
    if let Ok(mut tokens) = INVALID_REFRESH_TOKENS.lock() {
        tokens.clear();
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

    try_refresh_claude_code_instances(gcx, &http_client, &config_dir).await;
    try_refresh_openai_codex_instances(gcx, &http_client, &config_dir).await;
}

async fn try_refresh_claude_code_instances(
    gcx: &Arc<GlobalContext>,
    http_client: &reqwest::Client,
    config_dir: &std::path::Path,
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
        try_refresh_claude_code(gcx, http_client, config_dir, candidate).await;
    }
}

async fn try_refresh_claude_code(
    gcx: &Arc<GlobalContext>,
    http_client: &reqwest::Client,
    config_dir: &std::path::Path,
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

    match crate::providers::claude_code_oauth::refresh_access_token(
        http_client,
        &oauth_tokens.refresh_token,
    )
    .await
    {
        Ok(new_tokens) => {
            tracing::info!("{}: OAuth token refreshed successfully", display_name);
            if let Err(e) = save_refreshed_tokens(
                gcx,
                config_dir,
                &instance_id,
                "claude_code",
                &display_name,
                &new_tokens.access_token,
                &new_tokens.refresh_token,
                new_tokens.expires_at,
            )
            .await
            {
                tracing::warn!("{}: failed to save refreshed tokens: {}", display_name, e);
            }
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
                if let Err(save_err) = save_refreshed_tokens(
                    gcx,
                    config_dir,
                    &instance_id,
                    "claude_code",
                    &display_name,
                    "",
                    "",
                    0,
                )
                .await
                {
                    tracing::warn!(
                        "{}: failed to clear invalid OAuth tokens: {}",
                        display_name,
                        save_err
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

    match crate::providers::openai_codex_oauth::refresh_access_token(
        http_client,
        &oauth_tokens.refresh_token,
    )
    .await
    {
        Ok(new_tokens) => {
            tracing::info!("{}: OAuth token refreshed successfully", display_name);
            if let Err(e) = save_refreshed_tokens(
                gcx,
                config_dir,
                &instance_id,
                "openai_codex",
                &display_name,
                &new_tokens.access_token,
                &new_tokens.refresh_token,
                new_tokens.expires_at,
            )
            .await
            {
                tracing::warn!("{}: failed to save refreshed tokens: {}", display_name, e);
            }
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
                if let Err(save_err) = save_refreshed_tokens(
                    gcx,
                    config_dir,
                    &instance_id,
                    "openai_codex",
                    &display_name,
                    "",
                    "",
                    0,
                )
                .await
                {
                    tracing::warn!(
                        "{}: failed to clear invalid OAuth refresh token: {}",
                        display_name,
                        save_err
                    );
                }
                if first_failure {
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
    base_provider: &str,
    display_name: &str,
    access_token: &str,
    refresh_token: &str,
    expires_at: i64,
) -> Result<(), String> {
    let updated = config_store::update_provider_config(config_dir, provider_name, |existing| {
        let mut yaml_map = match existing {
            Some(value) => value.as_mapping().cloned().ok_or_else(|| {
                "Config file root is not a YAML mapping. Cannot safely patch.".to_string()
            })?,
            None => serde_yaml::Mapping::new(),
        };

        yaml_map.insert(
            serde_yaml::Value::String("base_provider".to_string()),
            serde_yaml::Value::String(base_provider.to_string()),
        );
        yaml_map.insert(
            serde_yaml::Value::String("display_name".to_string()),
            serde_yaml::Value::String(display_name.to_string()),
        );

        let mut tokens_map = yaml_map
            .get(&serde_yaml::Value::String("oauth_tokens".to_string()))
            .and_then(|v| v.as_mapping())
            .cloned()
            .unwrap_or_default();

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

        Ok(serde_yaml::Value::Mapping(yaml_map))
    })
    .await?;

    {
        let mut registry = gcx.providers.write().await;

        let mut provider = create_provider(base_provider)
            .ok_or_else(|| format!("Failed to create provider '{}'", base_provider))?;
        provider
            .provider_settings_apply(updated)
            .map_err(|e| format!("Failed to apply settings: {}", e))?;
        if provider_name == base_provider {
            registry.add(provider);
        } else {
            registry.add(Box::new(crate::providers::instance::ProviderInstance::new(
                provider_name.to_string(),
                base_provider.to_string(),
                display_name.to_string(),
                provider,
            )));
        }
    }

    {
        let caps_state = gcx.caps_state.clone();
        let mut caps_state = caps_state.write().await;
        caps_state.caps = None;
        caps_state.last_attempted_ts = 0;
    }

    Ok(())
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
        super::mark_invalid_refresh_token("openai_codex", "same-refresh-token");

        assert!(super::is_invalid_refresh_token(
            "openai_codex",
            "same-refresh-token"
        ));
        assert!(!super::is_invalid_refresh_token(
            "openai_codex_2",
            "same-refresh-token"
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
}
