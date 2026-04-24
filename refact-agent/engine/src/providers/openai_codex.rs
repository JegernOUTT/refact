use std::any::Any;
use std::collections::{HashMap, HashSet};
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

/// Generic GPT-5 model IDs without a "codex" suffix that are included by this provider.
/// Explicit allowlist avoids accidentally pulling in unrelated variants like "-pro" or "-reasoning".
const GENERIC_GPT5_ALLOWLIST: &[&str] = &[
    "gpt-5.2",
    "gpt-5.4",
    "gpt-5.4-mini",
    "gpt-5.5",
];

fn is_codex_model(id: &str) -> bool {
    let lower = id.to_lowercase();
    if lower.contains("codex") {
        return true;
    }
    GENERIC_GPT5_ALLOWLIST.iter().any(|&allowed| lower == allowed)
}

fn is_codex_model_conservative(id: &str) -> bool {
    id.to_lowercase().contains("codex")
}

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

    fn resolve_auth(&self) -> (AuthSource, CodexAuth) {
        if !self.oauth_tokens.openai_api_key.is_empty() {
            return (
                AuthSource::InAppOAuth,
                CodexAuth::PlatformApiKey {
                    api_key: self.oauth_tokens.openai_api_key.clone(),
                },
            );
        }

        if self.oauth_tokens.has_valid_access_token() && !self.oauth_tokens.chatgpt_account_id.is_empty() {
            return (
                AuthSource::InAppOAuth,
                CodexAuth::ChatGptBackendOAuth {
                    access_token: self.oauth_tokens.access_token.clone(),
                    chatgpt_account_id: self.oauth_tokens.chatgpt_account_id.clone(),
                },
            );
        }

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

    async fn fetch_models_from_api(
        &self,
        http_client: &reqwest::Client,
        model_caps: &HashMap<String, ModelCapabilities>,
        api_key: &str,
    ) -> Vec<AvailableModel> {
        const OPENAI_MODELS_URL: &str = "https://api.openai.com/v1/models";

        let response = match http_client
            .get(OPENAI_MODELS_URL)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {api_key}"))
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!("OpenAI Codex: failed to reach /v1/models (network error): {}, using hardcoded list", e);
                return self.fetch_models_from_hardcoded_list(model_caps);
            }
        };

        let status = response.status();

        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            tracing::warn!("OpenAI Codex: /v1/models returned {} — API key invalid or revoked; returning custom models only", status);
            return self.get_custom_models_only();
        }

        if !status.is_success() {
            tracing::warn!("OpenAI Codex: /v1/models returned {} (transient), using hardcoded list", status);
            return self.fetch_models_from_hardcoded_list(model_caps);
        }

        let json: serde_json::Value = match response.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("OpenAI Codex: failed to parse /v1/models response: {}, using hardcoded list", e);
                return self.fetch_models_from_hardcoded_list(model_caps);
            }
        };

        let data_array = match json.get("data").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => {
                tracing::warn!("OpenAI Codex: /v1/models response missing or non-array 'data' field, using hardcoded list");
                return self.fetch_models_from_hardcoded_list(model_caps);
            }
        };

        let enabled_set: HashSet<&str> = self.enabled_models.iter().map(|s| s.as_str()).collect();
        let mut models_map: HashMap<String, AvailableModel> = HashMap::new();

        for model in data_array {
            let id = match model.get("id").and_then(|v| v.as_str()) {
                Some(id) => id.to_string(),
                None => continue,
            };
            if !is_codex_model(&id) {
                continue;
            }
            let enabled = enabled_set.contains(id.as_str());
            let pricing = self.model_pricing(id.as_str());
            if let Some(caps) = crate::caps::model_caps::resolve_model_caps(model_caps, &id) {
                models_map.insert(id.clone(), AvailableModel::from_caps(&id, &caps.caps, enabled, pricing));
            } else {
                models_map.insert(id.clone(), self.default_codex_model(id, enabled, pricing));
            }
        }

        for (name, caps) in model_caps {
            if is_codex_model_conservative(name) && !models_map.contains_key(name) {
                let enabled = enabled_set.contains(name.as_str());
                let pricing = self.model_pricing(name);
                models_map.insert(name.clone(), AvailableModel::from_caps(name, caps, enabled, pricing));
            }
        }

        tracing::info!("OpenAI Codex: {} models available (from /v1/models API)", models_map.len());

        let mut models: Vec<AvailableModel> = models_map.into_values().collect();
        merge_custom_models(&mut models, &self.custom_models, &enabled_set);
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models
    }

    fn fetch_models_from_hardcoded_list(&self, model_caps: &HashMap<String, ModelCapabilities>) -> Vec<AvailableModel> {
        let (_, auth) = self.resolve_auth();
        let (caps_filter, base_ids): (fn(&str) -> bool, &[&str]) = match auth {
            CodexAuth::ChatGptBackendOAuth { .. } => (
                is_codex_model_conservative,
                &[
                    "gpt-5.3-codex",
                    "gpt-5.3-codex-spark",
                    "gpt-5.2-codex",
                    "gpt-5.1-codex-max",
                    "gpt-5.1-codex-mini",
                ],
            ),
            _ => (
                is_codex_model,
                &[
                    "gpt-5.5",
                    "gpt-5.4",
                    "gpt-5.4-mini",
                    "gpt-5.3-codex",
                    "gpt-5.3-codex-spark",
                    "gpt-5.2-codex",
                    "gpt-5.1-codex-max",
                    "gpt-5.1-codex-mini",
                ],
            ),
        };

        let mut seen: HashSet<String> = base_ids.iter().map(|s| s.to_string()).collect();
        let mut codex_model_ids: Vec<String> = base_ids.iter().map(|s| s.to_string()).collect();
        for model_id in model_caps.keys() {
            if caps_filter(model_id) && seen.insert(model_id.clone()) {
                codex_model_ids.push(model_id.clone());
            }
        }

        tracing::info!("OpenAI Codex: {} models available (hardcoded + caps-discovered)", codex_model_ids.len());

        let enabled_set: HashSet<&str> = self.enabled_models.iter().map(|s| s.as_str()).collect();
        let mut models: Vec<AvailableModel> = Vec::new();

        for model_id in &codex_model_ids {
            let enabled = enabled_set.contains(model_id.as_str());
            let pricing = self.model_pricing(model_id);

            if let Some(caps) = crate::caps::model_caps::resolve_model_caps(model_caps, model_id) {
                models.push(AvailableModel::from_caps(model_id, &caps.caps, enabled, pricing));
            } else {
                tracing::debug!("OpenAI Codex: no model_caps match for '{}', using defaults", model_id);
                models.push(self.default_codex_model(model_id.clone(), enabled, pricing));
            }
        }

        merge_custom_models(&mut models, &self.custom_models, &enabled_set);
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models
    }

    fn default_codex_model(&self, id: String, enabled: bool, pricing: Option<ModelPricing>) -> AvailableModel {
        AvailableModel {
            id,
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
        }
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
        // Conservative regex covering only codex-named models.
        // Generic GPT-5 allowlist entries (GENERIC_GPT5_ALLOWLIST) are applied at runtime
        // via is_codex_model() in fetch_available_models(); they are intentionally omitted here
        // to avoid false positives in shared caps-based code paths.
        Some(r"(?i)^gpt.*codex")
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
            supports_cache_control: true,
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
        let (_, auth) = self.resolve_auth();
        match auth {
            CodexAuth::PlatformApiKey { ref api_key } if !api_key.is_empty() => ModelSource::Api,
            _ => ModelSource::ModelCaps,
        }
    }

    fn enabled_models(&self) -> &[String] {
        &self.enabled_models
    }

    fn custom_models(&self) -> &HashMap<String, CustomModelConfig> {
        &self.custom_models
    }

    async fn fetch_available_models(
        &self,
        http_client: &reqwest::Client,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let (_, auth) = self.resolve_auth();
        match auth {
            CodexAuth::None => {
                tracing::warn!("OpenAI Codex: no auth");
                return self.get_custom_models_only();
            }
            CodexAuth::PlatformApiKey { ref api_key } if !api_key.is_empty() => {
                return self.fetch_models_from_api(http_client, model_caps, api_key).await;
            }
            _ => {}
        }

        self.fetch_models_from_hardcoded_list(model_caps)
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::caps::model_caps::ModelCapabilities;
    use crate::providers::traits::{AvailableModel, CustomModelConfig, ModelSource, ProviderTrait};
    use crate::providers::openai_codex_oauth::OAuthTokens;
    use super::OpenAICodexProvider;

    fn provider_with_api_key(api_key: &str) -> OpenAICodexProvider {
        OpenAICodexProvider {
            oauth_tokens: OAuthTokens {
                openai_api_key: api_key.to_string(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn provider_with_oauth(access_token: &str, chatgpt_account_id: &str) -> OpenAICodexProvider {
        OpenAICodexProvider {
            oauth_tokens: OAuthTokens {
                access_token: access_token.to_string(),
                chatgpt_account_id: chatgpt_account_id.to_string(),
                expires_at: i64::MAX,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn empty_caps() -> HashMap<String, ModelCapabilities> {
        HashMap::new()
    }

    #[test]
    fn model_source_api_when_platform_key_present() {
        let p = provider_with_api_key("sk-test");
        assert_eq!(p.model_source(), ModelSource::Api);
    }

    #[test]
    fn model_source_model_caps_when_oauth_only() {
        let p = provider_with_oauth("tok", "acct-123");
        assert_eq!(p.model_source(), ModelSource::ModelCaps);
    }

    #[test]
    fn model_source_model_caps_when_no_auth() {
        let p = OpenAICodexProvider::default();
        assert_eq!(p.model_source(), ModelSource::ModelCaps);
    }

    #[test]
    fn is_codex_model_matches_codex_named_models() {
        assert!(super::is_codex_model("gpt-5.3-codex"));
        assert!(super::is_codex_model("gpt-5.2-codex"));
        assert!(super::is_codex_model("gpt-5.1-codex-max"));
        assert!(super::is_codex_model("gpt-5.1-codex-mini"));
        assert!(super::is_codex_model("GPT-5.3-CODEX"), "must be case-insensitive");
        assert!(super::is_codex_model("gpt-5-codex"));
        assert!(super::is_codex_model("gpt-5.3-codex-spark"));
    }

    #[test]
    fn is_codex_model_matches_explicit_allowlist_variants() {
        assert!(super::is_codex_model("gpt-5.2"), "gpt-5.2 is in allowlist");
        assert!(super::is_codex_model("gpt-5.4"), "gpt-5.4 is in allowlist");
        assert!(super::is_codex_model("gpt-5.4-mini"), "gpt-5.4-mini is in allowlist");
        assert!(super::is_codex_model("gpt-5.5"), "gpt-5.5 is in allowlist");
    }

    #[test]
    fn is_codex_model_excludes_non_allowlist_generic_gpt5() {
        assert!(!super::is_codex_model("gpt-5.3"), "gpt-5.3 not in allowlist");
        assert!(!super::is_codex_model("gpt-5.6"), "gpt-5.6 not in allowlist");
        assert!(!super::is_codex_model("gpt-5.1"), "gpt-5.1 not in allowlist");
    }

    #[test]
    fn is_codex_model_excludes_non_codex() {
        assert!(!super::is_codex_model("gpt-4o"));
        assert!(!super::is_codex_model("gpt-4"));
        assert!(!super::is_codex_model("gpt-3.5-turbo"));
        assert!(!super::is_codex_model("gpt-5.2-pro"), "not in allowlist");
        assert!(!super::is_codex_model("gpt-5.4-preview"), "not in allowlist");
        assert!(!super::is_codex_model("gpt-5.4-reasoning"), "not in allowlist");
    }

    #[test]
    fn model_filter_regex_matches_codex_named_only() {
        let p = provider_with_api_key("sk-test");
        let pattern = p.model_filter_regex().expect("filter regex must be set");
        let re = regex::Regex::new(pattern).unwrap();

        assert!(re.is_match("gpt-5.3-codex"));
        assert!(re.is_match("GPT-5.3-CODEX"), "must be case-insensitive");
        assert!(re.is_match("gpt-5.1-codex-max"));

        assert!(!re.is_match("gpt-5.4"), "allowlist entry: not in conservative regex");
        assert!(!re.is_match("gpt-5.5"), "allowlist entry: not in conservative regex");
        assert!(!re.is_match("gpt-5.2"), "allowlist entry: not in conservative regex");
        assert!(!re.is_match("gpt-4o"));
        assert!(!re.is_match("gpt-4"));
        assert!(!re.is_match("gpt-3.5-turbo"));
        assert!(!re.is_match("gpt-5.1"));
        assert!(!re.is_match("gpt-5.2-pro"));
    }

    #[tokio::test]
    async fn no_auth_returns_empty_when_no_custom_models() {
        let p = OpenAICodexProvider::default();
        let client = reqwest::Client::new();
        let models: Vec<AvailableModel> = p.fetch_available_models(&client, &empty_caps()).await;
        assert!(models.is_empty());
    }

    #[tokio::test]
    async fn oauth_path_only_shows_codex_named_models() {
        let p = provider_with_oauth("tok", "acct-123");
        let client = reqwest::Client::new();
        let models: Vec<AvailableModel> = p.fetch_available_models(&client, &empty_caps()).await;
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();

        let conservative = regex::Regex::new(r"(?i)^gpt.*codex").unwrap();
        for id in &ids {
            assert!(conservative.is_match(id), "OAuth path returned non-codex model: '{}'", id);
        }

        assert!(!ids.contains(&"gpt-5.5"));
        assert!(!ids.contains(&"gpt-5.4"));
        assert!(!ids.contains(&"gpt-5.4-mini"));
    }

    #[test]
    fn auth_failure_returns_custom_models_only_not_hardcoded() {
        let p = provider_with_api_key("sk-revoked");
        let models: Vec<AvailableModel> = p.get_custom_models_only();
        assert!(
            models.is_empty(),
            "auth failure with no custom models must return empty, got: {:?}",
            models.iter().map(|m| &m.id).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn oauth_fallback_no_duplicates_when_caps_overlap() {
        let mut caps: HashMap<String, ModelCapabilities> = HashMap::new();
        caps.insert("gpt-5.3-codex".to_string(), ModelCapabilities::default());
        caps.insert("gpt-5.2-codex".to_string(), ModelCapabilities::default());
        caps.insert("gpt-5.6-codex".to_string(), ModelCapabilities::default());

        let p = provider_with_oauth("tok", "acct");
        let client = reqwest::Client::new();
        let models: Vec<AvailableModel> = p.fetch_available_models(&client, &caps).await;
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();

        for id in &ids {
            let count = ids.iter().filter(|x| **x == *id).count();
            assert_eq!(count, 1, "model '{}' appears {} times (must be 1)", id, count);
        }
        assert!(ids.contains(&"gpt-5.6-codex"));
    }

    #[tokio::test]
    async fn oauth_fallback_models_are_sorted() {
        let p = provider_with_oauth("tok", "acct");
        let client = reqwest::Client::new();
        let models: Vec<AvailableModel> = p.fetch_available_models(&client, &empty_caps()).await;
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted);
    }

    #[tokio::test]
    async fn custom_models_appear_with_no_auth() {
        let mut p = OpenAICodexProvider::default();
        p.custom_models.insert("my-custom".to_string(), CustomModelConfig::default());
        let client = reqwest::Client::new();
        let models: Vec<AvailableModel> = p.fetch_available_models(&client, &empty_caps()).await;
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"my-custom"));
    }

    #[tokio::test]
    async fn custom_models_appear_with_oauth() {
        let mut p = provider_with_oauth("tok", "acct");
        p.custom_models.insert("my-custom".to_string(), CustomModelConfig::default());
        let client = reqwest::Client::new();
        let models: Vec<AvailableModel> = p.fetch_available_models(&client, &empty_caps()).await;
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"my-custom"));
    }
}
