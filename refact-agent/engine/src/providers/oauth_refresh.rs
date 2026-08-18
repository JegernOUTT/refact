use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use crate::global_context::GlobalContext;
use crate::providers::config_store;
use crate::providers::traits::ProviderTrait;
pub use refact_providers::oauth_refresh::{
    clear_invalid_refresh_token, is_invalid_refresh_token, is_permanent_refresh_error,
    mark_invalid_refresh_token,
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
    refresh_expiring_oauth_tokens(&gcx).await;
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
        refresh_expiring_oauth_tokens(&gcx).await;
    }
}

pub(crate) async fn refresh_expiring_oauth_tokens(gcx: &Arc<GlobalContext>) {
    refresh_expiring_claude_code_tokens(gcx).await;
    let (http_client, config_dir) = { (gcx.http_client.clone(), gcx.config_dir.clone()) };
    try_refresh_openai_codex_instances(gcx, &http_client, &config_dir).await;
    try_refresh_xai_oauth_instances(gcx, &http_client, &config_dir).await;
    try_refresh_google_antigravity_instances(gcx, &http_client, &config_dir).await;
}

pub(crate) async fn refresh_expiring_claude_code_tokens(gcx: &Arc<GlobalContext>) {
    let http_client = gcx.http_client.clone();
    try_refresh_claude_code_instances(gcx, &http_client).await;
}

fn proactive_refresh_can_reuse_newer_expiry(
    rejected_expires_at: Option<i64>,
    rejected_status: Option<reqwest::StatusCode>,
    current_expires_at: i64,
) -> bool {
    rejected_status.is_none()
        && rejected_expires_at.is_some_and(|expires_at| current_expires_at > expires_at)
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
    let refresh_guard =
        crate::providers::claude_code::ClaudeCodeProvider::lock_refresh_guard().await?;
    let config_dir = gcx.config_dir.clone();
    let oauth_file_guard =
        config_store::lock_provider_oauth_file(&config_dir, provider_name).await?;
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
    sync_claude_code_tokens_from_disk(gcx, &config_dir, provider_name, &mut provider).await?;

    if provider
        .access_token_changed_since_rejection(rejected_access_token)
        .is_some()
    {
        return Ok(Some(provider));
    }
    if proactive_refresh_can_reuse_newer_expiry(
        rejected_expires_at,
        rejected_status,
        provider.oauth_tokens.expires_at,
    ) {
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
    let refreshed_tokens = match crate::providers::claude_code_oauth::refresh_access_token(
        http_client,
        &previous_tokens.refresh_token,
    )
    .await
    {
        Ok(tokens) => tokens,
        Err(error) if is_permanent_refresh_error(&error) => {
            let cleared = save_claude_code_refreshed_tokens(
                gcx,
                &config_dir,
                provider_name,
                &previous_tokens,
                &crate::providers::claude_code_oauth::OAuthTokens::default(),
            )
            .await?;
            if !cleared {
                return Ok(current_claude_code_provider(gcx, provider_name).await);
            }
            mark_invalid_refresh_token(provider_name, &previous_tokens.refresh_token);
            drop(oauth_file_guard);
            drop(refresh_guard);
            invalidate_caps(gcx).await;
            return Err(format!(
                "Claude Code OAuth refresh token is invalid. Please log in again in Claude Code provider settings: {}",
                error
            ));
        }
        Err(error) => return Err(format!("Claude Code OAuth refresh failed: {}", error)),
    };

    if !save_claude_code_refreshed_tokens(
        gcx,
        &config_dir,
        provider_name,
        &previous_tokens,
        &refreshed_tokens,
    )
    .await?
    {
        return Ok(current_claude_code_provider(gcx, provider_name).await);
    }

    provider.oauth_tokens = refreshed_tokens;
    drop(oauth_file_guard);
    drop(refresh_guard);
    invalidate_caps(gcx).await;
    Ok(Some(provider))
}

async fn sync_claude_code_tokens_from_disk(
    gcx: &Arc<GlobalContext>,
    config_dir: &Path,
    provider_name: &str,
    provider: &mut crate::providers::claude_code::ClaudeCodeProvider,
) -> Result<(), String> {
    let path = config_store::provider_config_path(config_dir, provider_name);
    let content = match tokio::fs::read_to_string(&path).await {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "Failed to read Claude Code provider config: {error}"
            ))
        }
    };
    let yaml: serde_yaml::Value = serde_yaml::from_str(&content)
        .map_err(|error| format!("Claude Code provider config is invalid YAML: {error}"))?;
    let Some(tokens_value) = yaml.get("oauth_tokens").cloned() else {
        return Ok(());
    };
    let disk_tokens: crate::providers::claude_code_oauth::OAuthTokens =
        serde_yaml::from_value(tokens_value)
            .map_err(|error| format!("Failed to parse Claude Code OAuth tokens: {error}"))?;
    if disk_tokens == provider.oauth_tokens {
        return Ok(());
    }

    provider.oauth_tokens = disk_tokens.clone();
    let mut registry = gcx.providers.write().await;
    if let Some(current) = registry.get_mut(provider_name).and_then(|current| {
        current
            .as_any_mut()
            .downcast_mut::<crate::providers::claude_code::ClaudeCodeProvider>()
    }) {
        current.apply_oauth_refresh_tokens(
            &disk_tokens.access_token,
            &disk_tokens.refresh_token,
            disk_tokens.expires_at,
        );
    }
    Ok(())
}

pub(crate) async fn refresh_claude_code_instance_if_needed(
    gcx: &Arc<GlobalContext>,
    provider_name: &str,
) -> Result<(), String> {
    let tokens = {
        let registry = gcx.providers.read().await;
        registry
            .get(provider_name)
            .and_then(|provider| {
                provider
                    .as_any()
                    .downcast_ref::<crate::providers::claude_code::ClaudeCodeProvider>()
            })
            .map(|provider| provider.oauth_tokens.clone())
    };
    let Some(tokens) = tokens else {
        return Ok(());
    };
    if !needs_refresh(tokens.expires_at) {
        return Ok(());
    }
    force_refresh_claude_code_for_retry(
        gcx,
        &gcx.http_client,
        provider_name,
        &tokens.access_token,
        Some(tokens.expires_at),
        None,
    )
    .await
    .map(|_| ())
}

async fn invalidate_caps(gcx: &Arc<GlobalContext>) {
    let mut caps = gcx.caps_state.write().await;
    caps.caps = None;
    caps.last_attempted_ts = 0;
}

async fn current_claude_code_provider(
    gcx: &Arc<GlobalContext>,
    provider_name: &str,
) -> Option<crate::providers::claude_code::ClaudeCodeProvider> {
    gcx.providers
        .read()
        .await
        .get(provider_name)
        .and_then(|provider| {
            provider
                .as_any()
                .downcast_ref::<crate::providers::claude_code::ClaudeCodeProvider>()
        })
        .cloned()
}

async fn save_claude_code_refreshed_tokens(
    gcx: &Arc<GlobalContext>,
    config_dir: &std::path::Path,
    provider_name: &str,
    expected_tokens: &crate::providers::claude_code_oauth::OAuthTokens,
    refreshed_tokens: &crate::providers::claude_code_oauth::OAuthTokens,
) -> Result<bool, String> {
    let registry_matches = {
        let registry = gcx.providers.read().await;
        registry
            .get(provider_name)
            .and_then(|provider| {
                provider
                    .as_any()
                    .downcast_ref::<crate::providers::claude_code::ClaudeCodeProvider>()
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
        let current_tokens: crate::providers::claude_code_oauth::OAuthTokens = yaml_map
            .get(&serde_yaml::Value::String("oauth_tokens".to_string()))
            .cloned()
            .map(serde_yaml::from_value)
            .transpose()
            .map_err(|error| format!("Failed to parse existing OAuth tokens: {}", error))?
            .unwrap_or_default();
        if &current_tokens != expected_tokens {
            return Ok(None);
        }

        yaml_map.insert(
            serde_yaml::Value::String("oauth_tokens".to_string()),
            serde_yaml::to_value(refreshed_tokens)
                .map_err(|error| format!("Failed to serialize OAuth tokens: {}", error))?,
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
                    .downcast_mut::<crate::providers::claude_code::ClaudeCodeProvider>()
            })
            .filter(|provider| &provider.oauth_tokens == expected_tokens)
            .map(|provider| {
                provider.apply_oauth_refresh_tokens(
                    &refreshed_tokens.access_token,
                    &refreshed_tokens.refresh_token,
                    refreshed_tokens.expires_at,
                );
            })
            .is_some()
    };

    Ok(changed)
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

async fn try_refresh_xai_oauth_instances(
    gcx: &Arc<GlobalContext>,
    http_client: &reqwest::Client,
    config_dir: &std::path::Path,
) {
    let candidates = {
        let registry = gcx.providers.read().await;
        registry
            .iter()
            .filter(|(_, provider)| provider.base_provider_name() == "xai_oauth")
            .filter_map(|(_, provider)| {
                let oauth_tokens = provider
                    .as_any()
                    .downcast_ref::<crate::providers::xai_oauth::XAIOAuthProvider>()?
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
        try_refresh_xai_oauth(gcx, http_client, config_dir, candidate).await;
    }
}

async fn try_refresh_xai_oauth(
    gcx: &Arc<GlobalContext>,
    http_client: &reqwest::Client,
    config_dir: &std::path::Path,
    candidate: OAuthRefreshCandidate<crate::providers::xai_oauth_flow::OAuthTokens>,
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

    let _guard = match crate::providers::xai_oauth::XAIOAuthProvider::lock_refresh_guard().await {
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
                    .downcast_ref::<crate::providers::xai_oauth::XAIOAuthProvider>()
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

    match crate::providers::xai_oauth_flow::refresh_access_token(
        http_client,
        &oauth_tokens.refresh_token,
    )
    .await
    {
        Ok(new_tokens) => {
            tracing::info!("{}: OAuth token refreshed successfully", display_name);
            let saved = match save_xai_refreshed_tokens(
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
                        "{}: OAuth refresh token is invalid; clearing saved refresh token. Please log in again if Grok stops working: {}",
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
                let cleared = match save_xai_refreshed_tokens(
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

async fn try_refresh_google_antigravity_instances(
    gcx: &Arc<GlobalContext>,
    http_client: &reqwest::Client,
    config_dir: &std::path::Path,
) {
    let candidates = {
        let registry = gcx.providers.read().await;
        registry
            .iter()
            .filter(|(_, provider)| provider.base_provider_name() == "google_antigravity")
            .filter_map(|(_, provider)| {
                let oauth_tokens = provider
                    .as_any()
                    .downcast_ref::<
                        crate::providers::google_antigravity::GoogleAntigravityProvider,
                    >()?
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
        try_refresh_google_antigravity(gcx, http_client, config_dir, candidate).await;
    }
}

async fn try_refresh_google_antigravity(
    gcx: &Arc<GlobalContext>,
    http_client: &reqwest::Client,
    config_dir: &std::path::Path,
    candidate: OAuthRefreshCandidate<crate::providers::google_antigravity_oauth::OAuthTokens>,
) {
    use crate::providers::google_antigravity::GoogleAntigravityProvider;

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

    let _guard = match GoogleAntigravityProvider::lock_refresh_guard().await {
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
                provider.as_any().downcast_ref::<
                    crate::providers::google_antigravity::GoogleAntigravityProvider,
                >()
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

    match crate::providers::google_antigravity_oauth::refresh_access_token(
        http_client,
        &oauth_tokens.refresh_token,
    )
    .await
    {
        Ok(new_tokens) => {
            tracing::info!("{}: OAuth token refreshed successfully", display_name);
            let project_id = if new_tokens.project_id.is_empty() {
                &oauth_tokens.project_id
            } else {
                &new_tokens.project_id
            };
            let saved = match save_google_antigravity_refreshed_tokens(
                gcx,
                config_dir,
                &instance_id,
                &oauth_tokens,
                &new_tokens.access_token,
                &new_tokens.refresh_token,
                new_tokens.expires_at,
                project_id,
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
                        concat!(
                            "{}: OAuth refresh token is invalid; clearing saved refresh token. ",
                            "Please log in again if Google Antigravity stops working: {}"
                        ),
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
                let cleared = match save_google_antigravity_refreshed_tokens(
                    gcx,
                    config_dir,
                    &instance_id,
                    &oauth_tokens,
                    "",
                    "",
                    0,
                    &oauth_tokens.project_id,
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

async fn save_google_antigravity_refreshed_tokens(
    gcx: &Arc<GlobalContext>,
    config_dir: &std::path::Path,
    provider_name: &str,
    expected_tokens: &crate::providers::google_antigravity_oauth::OAuthTokens,
    access_token: &str,
    refresh_token: &str,
    expires_at: i64,
    project_id: &str,
) -> Result<bool, String> {
    let registry_matches = {
        let registry = gcx.providers.read().await;
        registry
            .get(provider_name)
            .and_then(|provider| {
                provider.as_any().downcast_ref::<
                    crate::providers::google_antigravity::GoogleAntigravityProvider,
                >()
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
        let current_tokens: crate::providers::google_antigravity_oauth::OAuthTokens =
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
        tokens_map.insert(
            serde_yaml::Value::String("project_id".to_string()),
            serde_yaml::Value::String(project_id.to_string()),
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
                provider.as_any_mut().downcast_mut::<
                    crate::providers::google_antigravity::GoogleAntigravityProvider,
                >()
            })
            .filter(|provider| &provider.oauth_tokens == expected_tokens)
            .map(|provider| {
                provider.apply_oauth_refresh_tokens(access_token, refresh_token, expires_at);
                provider.oauth_tokens.project_id = project_id.to_string();
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

async fn save_xai_refreshed_tokens(
    gcx: &Arc<GlobalContext>,
    config_dir: &std::path::Path,
    provider_name: &str,
    expected_tokens: &crate::providers::xai_oauth_flow::OAuthTokens,
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
                    .downcast_ref::<crate::providers::xai_oauth::XAIOAuthProvider>()
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
        let current_tokens: crate::providers::xai_oauth_flow::OAuthTokens =
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
                    .downcast_mut::<crate::providers::xai_oauth::XAIOAuthProvider>()
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
        super::clear_invalid_refresh_token("openai_codex", "same-refresh-token-test");
        assert!(!super::is_invalid_refresh_token(
            "openai_codex",
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
    async fn claude_code_refresh_adopts_tokens_written_by_another_process() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        let providers_dir = config_dir.join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(
            providers_dir.join("claude_code.yaml"),
            "oauth_tokens:\n  access_token: fresh-access\n  refresh_token: fresh-refresh\n  expires_at: 9223372036854775807\n",
        )
        .await
        .unwrap();
        let current = crate::providers::claude_code::ClaudeCodeProvider {
            oauth_tokens: crate::providers::claude_code_oauth::OAuthTokens {
                access_token: "stale-access".to_string(),
                refresh_token: "stale-refresh".to_string(),
                expires_at: 1,
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
        let registry = gcx.providers.read().await;
        let provider = registry.get("claude_code").unwrap();
        let provider = provider
            .as_any()
            .downcast_ref::<crate::providers::claude_code::ClaudeCodeProvider>()
            .unwrap();
        assert_eq!(provider.oauth_tokens.refresh_token, "fresh-refresh");
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

    #[test]
    fn claude_code_auth_retry_does_not_reuse_rejected_token_with_newer_expiry() {
        assert!(!super::proactive_refresh_can_reuse_newer_expiry(
            Some(1),
            Some(reqwest::StatusCode::UNAUTHORIZED),
            i64::MAX,
        ));
        assert!(super::proactive_refresh_can_reuse_newer_expiry(
            Some(1),
            None,
            i64::MAX,
        ));
    }

    #[tokio::test]
    async fn claude_code_refreshed_tokens_preserve_provider_identity() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        let providers_dir = config_dir.join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(
            providers_dir.join("claude-code-work.yaml"),
            "base_provider: claude_code\ndisplay_name: Work Claude\nenabled_models:\n  - claude-sonnet-4-6\noauth_tokens:\n  access_token: old\n  refresh_token: old-refresh\n  expires_at: 1\n",
        )
        .await
        .unwrap();
        let expected_tokens = crate::providers::claude_code_oauth::OAuthTokens {
            access_token: "old".to_string(),
            refresh_token: "old-refresh".to_string(),
            expires_at: 1,
        };
        let provider = crate::providers::claude_code::ClaudeCodeProvider {
            enabled: true,
            enabled_models: vec!["claude-sonnet-4-6".to_string()],
            oauth_tokens: expected_tokens.clone(),
            ..Default::default()
        };
        {
            let mut registry = gcx.providers.write().await;
            registry.add(Box::new(crate::providers::instance::ProviderInstance::new(
                "claude-code-work",
                "claude_code",
                "Work Claude",
                Box::new(provider),
            )));
        }
        let refreshed_tokens = crate::providers::claude_code_oauth::OAuthTokens {
            access_token: "new".to_string(),
            refresh_token: "new-refresh".to_string(),
            expires_at: i64::MAX,
        };

        let saved = super::save_claude_code_refreshed_tokens(
            &gcx,
            &config_dir,
            "claude-code-work",
            &expected_tokens,
            &refreshed_tokens,
        )
        .await
        .unwrap();

        assert!(saved);
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            &tokio::fs::read_to_string(providers_dir.join("claude-code-work.yaml"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(yaml["base_provider"].as_str(), Some("claude_code"));
        assert_eq!(yaml["display_name"].as_str(), Some("Work Claude"));
        assert_eq!(
            yaml["enabled_models"][0].as_str(),
            Some("claude-sonnet-4-6")
        );
        assert_eq!(yaml["oauth_tokens"]["access_token"].as_str(), Some("new"));
    }

    #[tokio::test]
    async fn claude_code_stale_refresh_does_not_overwrite_newer_login() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_dir = gcx.config_dir.clone();
        let providers_dir = config_dir.join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(
            providers_dir.join("claude-code-work.yaml"),
            "base_provider: claude_code\ndisplay_name: Work Claude\noauth_tokens:\n  access_token: login-access\n  refresh_token: login-refresh\n  expires_at: 99\n",
        )
        .await
        .unwrap();
        let stale_tokens = crate::providers::claude_code_oauth::OAuthTokens {
            access_token: "old-access".to_string(),
            refresh_token: "old-refresh".to_string(),
            expires_at: 1,
        };
        let provider = crate::providers::claude_code::ClaudeCodeProvider {
            oauth_tokens: crate::providers::claude_code_oauth::OAuthTokens {
                access_token: "login-access".to_string(),
                refresh_token: "login-refresh".to_string(),
                expires_at: 99,
            },
            ..Default::default()
        };
        {
            let mut registry = gcx.providers.write().await;
            registry.add(Box::new(crate::providers::instance::ProviderInstance::new(
                "claude-code-work",
                "claude_code",
                "Work Claude",
                Box::new(provider),
            )));
        }
        let stale_refresh = crate::providers::claude_code_oauth::OAuthTokens {
            access_token: "stale-access".to_string(),
            refresh_token: "stale-refresh".to_string(),
            expires_at: i64::MAX,
        };

        let saved = super::save_claude_code_refreshed_tokens(
            &gcx,
            &config_dir,
            "claude-code-work",
            &stale_tokens,
            &stale_refresh,
        )
        .await
        .unwrap();

        assert!(!saved);
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            &tokio::fs::read_to_string(providers_dir.join("claude-code-work.yaml"))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            yaml["oauth_tokens"]["access_token"].as_str(),
            Some("login-access")
        );
        let registry = gcx.providers.read().await;
        let provider = registry.get("claude-code-work").unwrap();
        let provider = provider
            .as_any()
            .downcast_ref::<crate::providers::claude_code::ClaudeCodeProvider>()
            .unwrap();
        assert_eq!(provider.oauth_tokens.access_token, "login-access");
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
