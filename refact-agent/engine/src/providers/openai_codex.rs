use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::caps::model_caps::ModelCapabilities;
use crate::llm::adapter::WireFormat;
use crate::providers::openai_codex_oauth::OAuthTokens;
use crate::providers::traits::{
    AvailableModel, CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
    merge_custom_models, parse_enabled_models, parse_custom_models, set_model_enabled_impl,
};

#[derive(Debug, Clone, Copy, PartialEq)]
enum AuthSource {
    InAppOAuth,
    CodexCli,
    None,
}

#[derive(Debug, Clone)]
enum CodexAuth {
    PlatformApiKey { api_key: String },
    ChatGptBackendOAuth {
        access_token: String,
        chatgpt_account_id: String,
    },
    None,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenAICodexProvider {
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
    #[serde(default)]
    pub oauth_tokens: OAuthTokens,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAICodexUsageWindow {
    pub used_percent: f64,
    pub reset_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAICodexRateLimit {
    pub limit_reached: bool,
    pub primary_window: Option<OpenAICodexUsageWindow>,
    pub secondary_window: Option<OpenAICodexUsageWindow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAICodexCredits {
    pub balance: f64,
    pub unlimited: bool,
    pub has_credits: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenAICodexUsage {
    pub plan_type: Option<String>,
    pub rate_limit: Option<OpenAICodexRateLimit>,
    pub code_review_rate_limit: Option<OpenAICodexRateLimit>,
    pub credits: Option<OpenAICodexCredits>,
}

impl OpenAICodexProvider {
    fn needs_refresh_on_start(expires_at: i64) -> bool {
        const REFRESH_BEFORE_EXPIRY_MS: i64 = 5 * 60 * 1000;
        if expires_at == 0 {
            return true;
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        now_ms >= expires_at - REFRESH_BEFORE_EXPIRY_MS
    }

    async fn save_oauth_tokens_config(&self, config_dir: &std::path::Path) -> Result<(), String> {
        let providers_dir = config_dir.join("providers.d");
        let config_path = providers_dir.join("openai_codex.yaml");

        tokio::fs::create_dir_all(&providers_dir)
            .await
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

        let mut tokens_map = yaml_map
            .get(&serde_yaml::Value::String("oauth_tokens".to_string()))
            .and_then(|v| v.as_mapping())
            .cloned()
            .unwrap_or_default();

        tokens_map.insert(
            serde_yaml::Value::String("access_token".to_string()),
            serde_yaml::Value::String(self.oauth_tokens.access_token.clone()),
        );
        tokens_map.insert(
            serde_yaml::Value::String("refresh_token".to_string()),
            serde_yaml::Value::String(self.oauth_tokens.refresh_token.clone()),
        );
        tokens_map.insert(
            serde_yaml::Value::String("expires_at".to_string()),
            serde_yaml::Value::Number(serde_yaml::Number::from(self.oauth_tokens.expires_at)),
        );
        tokens_map.insert(
            serde_yaml::Value::String("openai_api_key".to_string()),
            serde_yaml::Value::String(self.oauth_tokens.openai_api_key.clone()),
        );
        tokens_map.insert(
            serde_yaml::Value::String("chatgpt_account_id".to_string()),
            serde_yaml::Value::String(self.oauth_tokens.chatgpt_account_id.clone()),
        );
        tokens_map.insert(
            serde_yaml::Value::String("api_key_exchange_error".to_string()),
            serde_yaml::Value::String(self.oauth_tokens.api_key_exchange_error.clone()),
        );

        yaml_map.insert(
            serde_yaml::Value::String("oauth_tokens".to_string()),
            serde_yaml::Value::Mapping(tokens_map),
        );

        let content = serde_yaml::to_string(&yaml_map)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;

        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique_id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp_path = config_path.with_extension(format!("yaml.tmp.oauth.{}.{}", std::process::id(), unique_id));

        tokio::fs::write(&temp_path, &content)
            .await
            .map_err(|e| format!("Failed to write temp config: {}", e))?;
        tokio::fs::rename(&temp_path, &config_path)
            .await
            .map_err(|e| format!("Failed to rename config: {}", e))?;

        Ok(())
    }

    /// Returns the credential to use for api.openai.com endpoints.
    ///
    /// IMPORTANT: Codex/ChatGPT OAuth produces an OAuth access token, but the OpenAI Platform
    /// API requires an API key with `api.responses.write` scope. Codex CLI obtains that API key
    /// via OAuth token-exchange and stores it as OPENAI_API_KEY.
    fn resolve_auth(&self) -> (AuthSource, CodexAuth) {
        // Prefer API key obtained via token-exchange in our OAuth flow.
        if !self.oauth_tokens.openai_api_key.is_empty() {
            return (
                AuthSource::InAppOAuth,
                CodexAuth::PlatformApiKey {
                    api_key: self.oauth_tokens.openai_api_key.clone(),
                },
            );
        }

        // If we have a ChatGPT OAuth access token + chatgpt_account_id, we can use
        // ChatGPT backend endpoint (Codex-style) without an OpenAI Platform org.
        if self.oauth_tokens.has_valid_access_token() && !self.oauth_tokens.chatgpt_account_id.is_empty() {
            return (
                AuthSource::InAppOAuth,
                CodexAuth::ChatGptBackendOAuth {
                    access_token: self.oauth_tokens.access_token.clone(),
                    chatgpt_account_id: self.oauth_tokens.chatgpt_account_id.clone(),
                },
            );
        }

        // Fall back to Codex CLI credentials: prefer OPENAI_API_KEY if present.
        if let Ok(cli_tokens) = crate::providers::openai_codex_oauth::read_codex_cli_credentials() {
            if !cli_tokens.openai_api_key.is_empty() {
                return (
                    AuthSource::CodexCli,
                    CodexAuth::PlatformApiKey {
                        api_key: cli_tokens.openai_api_key,
                    },
                );
            }
        }

        // Last resort: OAuth access token only (usually not enough).
        if self.oauth_tokens.has_valid_access_token() {
            return (
                AuthSource::InAppOAuth,
                CodexAuth::ChatGptBackendOAuth {
                    access_token: self.oauth_tokens.access_token.clone(),
                    chatgpt_account_id: String::new(),
                },
            );
        }

        (AuthSource::None, CodexAuth::None)
    }

    fn resolve_wham_token(&self) -> Result<String, String> {
        // The wham/usage endpoint uses the ChatGPT OAuth access token
        if self.oauth_tokens.has_valid_access_token() {
            return Ok(self.oauth_tokens.access_token.clone());
        }
        if let Ok(cli_tokens) = crate::providers::openai_codex_oauth::read_codex_cli_credentials() {
            if !cli_tokens.access_token.is_empty() {
                return Ok(cli_tokens.access_token);
            }
        }
        Err("No ChatGPT OAuth access token available for usage API".to_string())
    }

    pub async fn fetch_usage(&self, http_client: &reqwest::Client) -> Result<OpenAICodexUsage, String> {
        let token = self.resolve_wham_token()?;

        let resp = http_client
            .get("https://chatgpt.com/backend-api/wham/usage")
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let truncated: String = body.chars().take(512).collect();
            return Err(format!("Usage API returned {}: {}", status, truncated));
        }

        let root: serde_json::Value = resp.json().await
            .map_err(|e| format!("Failed to parse usage response: {}", e))?;

        let data = root.get("data").unwrap_or(&root);

        fn as_f64_loose(v: &serde_json::Value) -> Option<f64> {
            v.as_f64().or_else(|| v.as_i64().map(|i| i as f64))
        }

        let parse_window = |obj: &serde_json::Value| -> Option<OpenAICodexUsageWindow> {
            let used_percent = obj.get("used_percent").and_then(as_f64_loose)?;
            let reset_at = obj.get("reset_at").and_then(|v| {
                if let Some(ts) = v.as_i64() {
                    use std::time::{Duration, UNIX_EPOCH};
                    let dt: chrono::DateTime<chrono::Utc> = (UNIX_EPOCH + Duration::from_secs(ts as u64)).into();
                    Some(dt.to_rfc3339())
                } else {
                    v.as_str().map(|s| s.to_string())
                }
            });
            Some(OpenAICodexUsageWindow { used_percent, reset_at })
        };

        let parse_rate_limit = |rl: &serde_json::Value| -> OpenAICodexRateLimit {
            OpenAICodexRateLimit {
                limit_reached: rl.get("limit_reached").and_then(|v| v.as_bool()).unwrap_or(false),
                primary_window: rl.get("primary_window").and_then(|w| parse_window(w)),
                secondary_window: rl.get("secondary_window").and_then(|w| parse_window(w)),
            }
        };

        let plan_type = data.get("plan_type").and_then(|v| v.as_str()).map(|s| s.to_string());

        let rate_limit = data.get("rate_limit").map(|rl| parse_rate_limit(rl));

        let code_review_rate_limit = data.get("code_review_rate_limit").map(|rl| parse_rate_limit(rl));

        let credits = data.get("credits").map(|c| {
            let balance = c.get("balance")
                .and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()))
                .or_else(|| as_f64_loose(c.get("balance").unwrap_or(&serde_json::Value::Null)))
                .unwrap_or(0.0);
            OpenAICodexCredits {
                balance,
                unlimited: c.get("unlimited").and_then(|v| v.as_bool()).unwrap_or(false),
                has_credits: c.get("has_credits").and_then(|v| v.as_bool()).unwrap_or(false),
            }
        });

        Ok(OpenAICodexUsage { plan_type, rate_limit, code_review_rate_limit, credits })
    }

    fn diagnose_auth_status(&self) -> String {
        if !self.oauth_tokens.openai_api_key.is_empty() {
            return "OK (OAuth login — Platform API key)".to_string();
        }

        if self.oauth_tokens.has_valid_access_token() {
            if !self.oauth_tokens.chatgpt_account_id.is_empty() {
                if self.oauth_tokens.api_key_exchange_error.is_empty() {
                    return "Connected (ChatGPT backend)".to_string();
                }
                // Keep details in `api_key_exchange_error`; show a short user-friendly status.
                return "Connected (ChatGPT backend). Platform API key not available for this account.".to_string();
            }
            return "OAuth login incomplete: missing chatgpt_account_id".to_string();
        }

        if !self.oauth_tokens.is_empty() && self.oauth_tokens.has_refresh_token() {
            return "OAuth token expired — needs refresh".to_string();
        }
        if crate::providers::openai_codex_oauth::codex_cli_credentials_exist() {
            return "OK (Codex CLI session)".to_string();
        }
        "No credentials found".to_string()
    }
}

#[async_trait]
impl ProviderTrait for OpenAICodexProvider {
    fn name(&self) -> &'static str {
        "openai_codex"
    }

    fn display_name(&self) -> &'static str {
        "OpenAI Codex"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn ProviderTrait> {
        Box::new(self.clone())
    }

    fn default_wire_format(&self) -> WireFormat {
        WireFormat::OpenaiResponses
    }

    fn model_filter_regex(&self) -> Option<&'static str> {
        Some(r"^(gpt-.*codex|codex-)")
    }

    fn provider_schema(&self) -> &'static str {
        r#"
fields: {}
oauth:
  supported: true
  methods:
    - id: chatgpt
      label: "ChatGPT Plus/Pro"
      description: "Login with your ChatGPT Plus or Pro subscription"
description: |
  Use your ChatGPT Plus/Pro subscription to access OpenAI Codex models (GPT-5-Codex family).

  **Setup:** Click **Login with OpenAI** below, or install Codex CLI and run `codex login`.
available:
  on_your_laptop_possible: true
  when_isolated_possible: true
"#
    }

    fn provider_settings_apply(&mut self, yaml: serde_yaml::Value) -> Result<(), String> {
        if let Some(oauth_tokens) = yaml.get("oauth_tokens") {
            self.oauth_tokens = serde_yaml::from_value(oauth_tokens.clone())
                .unwrap_or_default();
        }
        parse_enabled_models(&yaml, &mut self.enabled_models);
        parse_custom_models(&yaml, &mut self.custom_models);
        Ok(())
    }

    fn provider_settings_as_json(&self) -> serde_json::Value {
        let auth_status = self.diagnose_auth_status();
        let oauth_connected = self.oauth_tokens.has_valid_access_token() || self.oauth_tokens.has_refresh_token();
        let api_key_ready = !self.oauth_tokens.openai_api_key.is_empty();

        json!({
            "auth_status": auth_status,
            "oauth_connected": oauth_connected,
            "api_key_ready": api_key_ready,
            "api_key_exchange_error": self.oauth_tokens.api_key_exchange_error,
            "enabled_models": self.enabled_models,
            "custom_models": self.custom_models
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let (_, auth) = self.resolve_auth();
        let mut extra_headers = HashMap::new();

        let (chat_endpoint, api_key) = match auth {
            CodexAuth::PlatformApiKey { api_key } => ("https://api.openai.com/v1/responses".to_string(), api_key),
            CodexAuth::ChatGptBackendOAuth { access_token, chatgpt_account_id, .. } => {
                // OpenCode/Codex-style endpoint: ChatGPT backend
                // Requires store:false (set in adapter by endpoint), and special headers.
                if !chatgpt_account_id.is_empty() {
                    extra_headers.insert("chatgpt-account-id".to_string(), chatgpt_account_id);
                }
                extra_headers.insert("OpenAI-Beta".to_string(), "responses=experimental".to_string());
                extra_headers.insert("originator".to_string(), "codex_cli_rs".to_string());
                extra_headers.insert("accept".to_string(), "text/event-stream".to_string());
                (
                    "https://chatgpt.com/backend-api/codex/responses".to_string(),
                    access_token,
                )
            }
            CodexAuth::None => (String::new(), String::new()),
        };

        let has_auth = !api_key.is_empty() && !chat_endpoint.is_empty();

        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: has_auth && !self.enabled_models.is_empty(),
            readonly: false,
            wire_format: self.default_wire_format(),
            chat_endpoint,
            completion_endpoint: String::new(),
            embedding_endpoint: String::new(),
            api_key,
            auth_token: String::new(),
            tokenizer_api_key: String::new(),
            extra_headers,
            support_metadata: false,
            chat_models: Vec::new(),
            completion_models: Vec::new(),
            embedding_model: None,
        })
    }

    fn has_credentials(&self) -> bool {
        if !self.oauth_tokens.openai_api_key.is_empty() {
            return true;
        }
        if self.oauth_tokens.has_valid_access_token() {
            return true;
        }
        if self.oauth_tokens.has_refresh_token() {
            return true;
        }
        crate::providers::openai_codex_oauth::codex_cli_credentials_exist()
    }

    fn model_source(&self) -> ModelSource {
        ModelSource::ModelCaps
    }

    fn enabled_models(&self) -> &[String] {
        &self.enabled_models
    }

    fn custom_models(&self) -> &HashMap<String, CustomModelConfig> {
        &self.custom_models
    }

    async fn fetch_available_models(
        &self,
        _http_client: &reqwest::Client,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let (_, auth) = self.resolve_auth();
        let has_auth = match auth {
            CodexAuth::PlatformApiKey { ref api_key } => !api_key.is_empty(),
            CodexAuth::ChatGptBackendOAuth { ref access_token, .. } => !access_token.is_empty(),
            CodexAuth::None => false,
        };
        if !has_auth {
            tracing::warn!("OpenAI Codex: no auth");
            return self.get_custom_models_only();
        }

        let mut codex_model_ids: Vec<String> = vec![
            "gpt-5.3-codex".to_string(),
            "gpt-5.2-codex".to_string(),
            "gpt-5.1-codex-max".to_string(),
            "gpt-5.2".to_string(),
            "gpt-5.1-codex-mini".to_string(),
        ];

        let codex_pattern = regex::Regex::new(r"(?i)^gpt.*codex").expect("valid static regex");
        for model_id in model_caps.keys() {
            if codex_pattern.is_match(model_id) && !codex_model_ids.contains(model_id) {
                codex_model_ids.push(model_id.clone());
            }
        }

        tracing::info!("OpenAI Codex: {} models available (hardcoded + discovered)", codex_model_ids.len());

        let enabled_set: std::collections::HashSet<_> =
            self.enabled_models.iter().map(|s| s.as_str()).collect();

        let mut models: Vec<AvailableModel> = Vec::new();

        for model_id in &codex_model_ids {
            let enabled = enabled_set.contains(model_id.as_str());
            let pricing = self.model_pricing(model_id);

            if let Some(caps) = crate::caps::model_caps::resolve_model_caps(model_caps, model_id) {
                let model = AvailableModel::from_caps(model_id, &caps.caps, enabled, pricing);
                models.push(model);
            } else {
                tracing::debug!("OpenAI Codex: no model_caps match for '{}', using defaults", model_id);
                models.push(AvailableModel {
                    id: model_id.to_string(),
                    display_name: None,
                    n_ctx: 200_000,
                    supports_tools: true,
                    supports_multimodality: true,
                    reasoning_effort_options: Some(vec!["low".to_string(), "medium".to_string(), "high".to_string()]),
                    supports_thinking_budget: false,
                    supports_adaptive_thinking_budget: false,
                    tokenizer: None,
                    enabled,
                    is_custom: false,
                    pricing,
                    available_providers: Vec::new(),
                    selected_provider: None,
                    max_output_tokens: None,
                    provider_variants: Vec::new(),
                });
            }
        }

        merge_custom_models(&mut models, &self.custom_models, &enabled_set);

        models.sort_by(|a, b| a.id.cmp(&b.id));
        models
    }

    fn set_model_enabled(&mut self, model_id: &str, enabled: bool) {
        set_model_enabled_impl(&mut self.enabled_models, model_id, enabled);
    }

    fn add_custom_model(&mut self, model_id: String, config: CustomModelConfig) {
        self.custom_models.insert(model_id, config);
    }

    fn remove_custom_model(&mut self, model_id: &str) -> bool {
        self.custom_models.remove(model_id).is_some()
    }

    fn model_pricing(&self, model_id: &str) -> Option<ModelPricing> {
        if let Some(config) = self.custom_models.get(model_id) {
            if config.pricing.is_some() {
                return config.pricing.clone();
            }
        }
        None
    }

    async fn startup_refresh_and_sync(
        &mut self,
        http_client: &reqwest::Client,
        config_dir: &std::path::Path,
    ) -> Result<(), String> {
        if self.oauth_tokens.is_empty() || self.oauth_tokens.refresh_token.is_empty() {
            return Ok(());
        }

        if !Self::needs_refresh_on_start(self.oauth_tokens.expires_at) {
            return Ok(());
        }

        tracing::info!("OpenAI Codex: refreshing OAuth token on startup");
        let mut refreshed = crate::providers::openai_codex_oauth::refresh_access_token(
            http_client,
            &self.oauth_tokens.refresh_token,
        )
        .await?;

        if refreshed.openai_api_key.is_empty() {
            refreshed.openai_api_key = self.oauth_tokens.openai_api_key.clone();
        }
        if refreshed.chatgpt_account_id.is_empty() {
            refreshed.chatgpt_account_id = self.oauth_tokens.chatgpt_account_id.clone();
        }
        if refreshed.api_key_exchange_error.is_empty() {
            refreshed.api_key_exchange_error = self.oauth_tokens.api_key_exchange_error.clone();
        }

        self.oauth_tokens = refreshed;
        self.save_oauth_tokens_config(config_dir).await
    }
}
