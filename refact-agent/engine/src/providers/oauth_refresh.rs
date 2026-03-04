use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::global_context::GlobalContext;
use crate::providers::create_provider;

const REFRESH_CHECK_INTERVAL_SECS: u64 = 60;
const REFRESH_BEFORE_EXPIRY_MS: i64 = 5 * 60 * 1000; // 5 minutes before expiry

pub async fn oauth_token_refresh_background_task(gcx: Arc<ARwLock<GlobalContext>>) {
    loop {
        let shutdown_flag = gcx.read().await.shutdown_flag.clone();
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

async fn try_refresh_all_providers(gcx: &Arc<ARwLock<GlobalContext>>) -> () {
    let (http_client, config_dir) = {
        let gcx_locked = gcx.read().await;
        (gcx_locked.http_client.clone(), gcx_locked.config_dir.clone())
    };

    try_refresh_claude_code(gcx, &http_client, &config_dir).await;
    try_refresh_openai_codex(gcx, &http_client, &config_dir).await;
}

async fn try_refresh_claude_code(
    gcx: &Arc<ARwLock<GlobalContext>>,
    http_client: &reqwest::Client,
    config_dir: &std::path::Path,
) {
    let oauth_tokens = {
        let gcx_locked = gcx.read().await;
        let registry = gcx_locked.providers.read().await;
        let provider = match registry.get("claude_code") {
            Some(p) => p,
            None => return,
        };
        let any = provider.as_any();
        let cc = match any.downcast_ref::<crate::providers::claude_code::ClaudeCodeProvider>() {
            Some(p) => p,
            None => return,
        };
        cc.oauth_tokens.clone()
    };

    if oauth_tokens.is_empty() || oauth_tokens.refresh_token.is_empty() {
        return;
    }

    if !needs_refresh(oauth_tokens.expires_at) {
        return;
    }

    tracing::info!("Claude Code: refreshing OAuth token (expires_at={})", oauth_tokens.expires_at);

    match crate::providers::claude_code_oauth::refresh_access_token(
        http_client,
        &oauth_tokens.refresh_token,
    ).await {
        Ok(new_tokens) => {
            tracing::info!("Claude Code: OAuth token refreshed successfully");
            if let Err(e) = save_refreshed_tokens(gcx, config_dir, "claude_code", &new_tokens.access_token, &new_tokens.refresh_token, new_tokens.expires_at).await {
                tracing::warn!("Claude Code: failed to save refreshed tokens: {}", e);
            }
        }
        Err(e) => {
            tracing::warn!("Claude Code: OAuth token refresh failed: {}", e);
        }
    }
}

async fn try_refresh_openai_codex(
    gcx: &Arc<ARwLock<GlobalContext>>,
    http_client: &reqwest::Client,
    config_dir: &std::path::Path,
) {
    let oauth_tokens = {
        let gcx_locked = gcx.read().await;
        let registry = gcx_locked.providers.read().await;
        let provider = match registry.get("openai_codex") {
            Some(p) => p,
            None => return,
        };
        let any = provider.as_any();
        let oc = match any.downcast_ref::<crate::providers::openai_codex::OpenAICodexProvider>() {
            Some(p) => p,
            None => return,
        };
        oc.oauth_tokens.clone()
    };

    if oauth_tokens.is_empty() || oauth_tokens.refresh_token.is_empty() {
        return;
    }

    if !needs_refresh(oauth_tokens.expires_at) {
        return;
    }

    tracing::info!("OpenAI Codex: refreshing OAuth token (expires_at={})", oauth_tokens.expires_at);

    match crate::providers::openai_codex_oauth::refresh_access_token(
        http_client,
        &oauth_tokens.refresh_token,
    ).await {
        Ok(new_tokens) => {
            tracing::info!("OpenAI Codex: OAuth token refreshed successfully");
            if let Err(e) = save_refreshed_tokens(gcx, config_dir, "openai_codex", &new_tokens.access_token, &new_tokens.refresh_token, new_tokens.expires_at).await {
                tracing::warn!("OpenAI Codex: failed to save refreshed tokens: {}", e);
            }
        }
        Err(e) => {
            tracing::warn!("OpenAI Codex: OAuth token refresh failed: {}", e);
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

async fn save_refreshed_tokens(
    gcx: &Arc<ARwLock<GlobalContext>>,
    config_dir: &std::path::Path,
    provider_name: &str,
    access_token: &str,
    refresh_token: &str,
    expires_at: i64,
) -> Result<(), String> {
    let providers_dir = config_dir.join("providers.d");
    let config_path = providers_dir.join(format!("{}.yaml", provider_name));

    tokio::fs::create_dir_all(&providers_dir).await
        .map_err(|e| format!("Failed to create providers.d: {}", e))?;

    let mut yaml_map: serde_yaml::Mapping = if config_path.exists() {
        let content = tokio::fs::read_to_string(&config_path)
            .await
            .map_err(|e| format!("Failed to read config: {}", e))?;
        let value: serde_yaml::Value = serde_yaml::from_str(&content)
            .map_err(|e| format!("Failed to parse YAML: {}", e))?;
        value
            .as_mapping()
            .cloned()
            .ok_or_else(|| "Config file root is not a YAML mapping. Cannot safely patch.".to_string())?
    } else {
        serde_yaml::Mapping::new()
    };

    // Preserve any additional OAuth fields already stored (e.g. openai_api_key)
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

    let content = serde_yaml::to_string(&yaml_map)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;

    use std::sync::atomic::{AtomicU64, Ordering};
    static REFRESH_COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique_id = REFRESH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path = config_path.with_extension(format!("yaml.tmp.refresh.{}.{}", std::process::id(), unique_id));

    tokio::fs::write(&temp_path, &content).await
        .map_err(|e| format!("Failed to write temp config: {}", e))?;
    tokio::fs::rename(&temp_path, &config_path).await
        .map_err(|e| format!("Failed to rename config: {}", e))?;

    {
        let gcx_locked = gcx.read().await;
        let mut registry = gcx_locked.providers.write().await;

        let full_content = tokio::fs::read_to_string(&config_path).await
            .map_err(|e| format!("Failed to reload config: {}", e))?;
        let yaml: serde_yaml::Value = serde_yaml::from_str(&full_content)
            .map_err(|e| format!("Invalid YAML after save: {}", e))?;

        let mut provider = create_provider(provider_name)
            .ok_or_else(|| format!("Failed to create provider '{}'", provider_name))?;
        provider.provider_settings_apply(yaml)
            .map_err(|e| format!("Failed to apply settings: {}", e))?;
        registry.add(provider);
    }

    {
        let mut gcx_locked = gcx.write().await;
        gcx_locked.caps = None;
        gcx_locked.caps_last_attempted_ts = 0;
    }

    Ok(())
}
