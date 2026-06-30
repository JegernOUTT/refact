use std::any::Any;
use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use refact_core::llm_types::WireFormat;
use refact_core::model_caps::ModelCapabilities;
use refact_core::models_dev::{load_models_dev_snapshot_catalog, ModelsDevCatalog};
use crate::config::resolve_env_var;
use crate::models_dev_provider::{
    build_models_dev_available_models, models_dev_provider_wire_format,
    models_dev_runtime_endpoint, validate_models_dev_endpoint, ModelsDevEndpointSource,
    ModelsDevProviderConfig, ModelsDevProviderFamily,
};
use crate::traits::{
    parse_custom_models, parse_enabled_models, set_model_enabled_impl, AvailableModel,
    CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
};

const DEFAULT_MODELS_DEV_PROVIDER_ID: &str = "opencode-go";
const DEFAULT_USAGE_ENDPOINT: &str = "";
const OPENCODE_USAGE_TIMEOUT: Duration = Duration::from_secs(8);

fn default_models_dev_provider_id() -> String {
    DEFAULT_MODELS_DEV_PROVIDER_ID.to_string()
}

fn default_usage_endpoint() -> String {
    DEFAULT_USAGE_ENDPOINT.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeProvider {
    pub api_key: String,
    #[serde(default = "default_models_dev_provider_id")]
    pub models_dev_provider_id: String,
    #[serde(default = "default_usage_endpoint")]
    pub usage_endpoint: String,
    pub enabled: bool,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
}

impl Default for OpenCodeProvider {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            models_dev_provider_id: default_models_dev_provider_id(),
            usage_endpoint: default_usage_endpoint(),
            enabled: false,
            enabled_models: Vec::new(),
            custom_models: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct OpenCodeUsageWindow {
    pub used_percent: f64,
    pub reset_at: Option<String>,
    pub reset_after_seconds: Option<u64>,
    pub limit_window_seconds: Option<u64>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct OpenCodeUsage {
    pub plan_type: Option<String>,
    pub workspace_id: Option<String>,
    pub balance: Option<f64>,
    pub rolling: Option<OpenCodeUsageWindow>,
    pub weekly: Option<OpenCodeUsageWindow>,
    pub monthly: Option<OpenCodeUsageWindow>,
    pub raw_extra: Map<String, Value>,
}

impl OpenCodeProvider {
    fn models_dev_config(&self) -> ModelsDevProviderConfig {
        ModelsDevProviderConfig::new(
            &self.models_dev_provider_id,
            ModelsDevProviderFamily::OpenCode,
        )
        .with_wire_format_override(WireFormat::OpenaiChatCompletions)
    }

    fn available_models_from_catalog(
        &self,
        catalog: &ModelsDevCatalog,
    ) -> Result<Vec<AvailableModel>, String> {
        build_models_dev_available_models(
            catalog,
            &self.models_dev_config(),
            &self.enabled_models,
            &self.custom_models,
        )
    }

    fn build_runtime_from_catalog(
        &self,
        catalog: &ModelsDevCatalog,
    ) -> Result<ProviderRuntime, String> {
        let api_key = resolve_env_var(&self.api_key, "", "opencode api_key");
        let config = self.models_dev_config();
        let chat_endpoint = models_dev_runtime_endpoint(catalog, &config)?;
        let wire_format = models_dev_provider_wire_format(catalog, &config);

        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: self.enabled && !api_key.is_empty() && !self.enabled_models.is_empty(),
            readonly: false,
            wire_format,
            chat_endpoint,
            completion_endpoint: String::new(),
            embedding_endpoint: String::new(),
            completion_endpoint_style: String::new(),
            embedding_endpoint_style: String::new(),
            api_key,
            auth_token: String::new(),
            tokenizer_api_key: String::new(),
            extra_headers: HashMap::new(),
            supports_cache_control: true,
            chat_models: Vec::new(),
            completion_models: Vec::new(),
            embedding_model: None,
        })
    }

    pub async fn fetch_usage(
        &self,
        http_client: &reqwest::Client,
    ) -> Result<OpenCodeUsage, String> {
        let api_key = resolve_env_var(&self.api_key, "", "opencode api_key");
        if api_key.is_empty() {
            return Err("OpenCode API key is not configured".to_string());
        }
        let usage_endpoint = self.usage_endpoint.trim();
        if usage_endpoint.is_empty() {
            return Err(
                "OpenCode quota API is not currently available; configure a usage endpoint after OpenCode exposes one"
                    .to_string(),
            );
        }
        validate_models_dev_endpoint(
            usage_endpoint,
            ModelsDevProviderFamily::OpenCode,
            ModelsDevEndpointSource::UserConfigured,
            &[],
        )
        .map_err(|e| format!("OpenCode usage endpoint is not safe: {e}"))?;

        let response = http_client
            .get(usage_endpoint)
            .bearer_auth(api_key)
            .header("Accept", "application/json")
            .timeout(OPENCODE_USAGE_TIMEOUT)
            .send()
            .await
            .map_err(|e| format!("OpenCode usage request failed: {e}"))?;
        let status = response.status();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let text = response
            .text()
            .await
            .map_err(|e| format!("OpenCode usage response read failed: {e}"))?;
        if !status.is_success() {
            return Err(format_usage_http_error(status, &content_type, &text));
        }
        let payload: Value = serde_json::from_str(&text)
            .map_err(|e| format!("OpenCode usage response was not valid JSON: {e}"))?;
        Ok(Self::parse_usage_payload(&payload))
    }

    pub fn parse_usage_payload(root: &Value) -> OpenCodeUsage {
        let data = root.get("data").unwrap_or(root);
        let mut usage = OpenCodeUsage {
            plan_type: first_string(data, &["plan_type", "planType", "plan", "subscription"]),
            workspace_id: first_string(data, &["workspace_id", "workspaceID", "workspace"]),
            balance: first_f64(
                data,
                &["balance", "credits", "credit_balance", "creditBalance"],
            ),
            rolling: first_window(
                data,
                &["rolling", "rolling_usage", "rollingUsage", "five_hour"],
            ),
            weekly: first_window(data, &["weekly", "weekly_usage", "weeklyUsage", "week"]),
            monthly: first_window(data, &["monthly", "monthly_usage", "monthlyUsage", "month"]),
            raw_extra: collect_raw_extra(
                data,
                &[
                    "plan_type",
                    "planType",
                    "plan",
                    "subscription",
                    "workspace_id",
                    "workspaceID",
                    "workspace",
                    "balance",
                    "credits",
                    "credit_balance",
                    "creditBalance",
                    "rolling",
                    "rolling_usage",
                    "rollingUsage",
                    "five_hour",
                    "weekly",
                    "weekly_usage",
                    "weeklyUsage",
                    "week",
                    "monthly",
                    "monthly_usage",
                    "monthlyUsage",
                    "month",
                ],
            ),
        };

        if usage.rolling.is_none() && usage.weekly.is_none() && usage.monthly.is_none() {
            usage.rolling = parse_usage_window(data);
        }
        usage
    }
}

#[async_trait]
impl ProviderTrait for OpenCodeProvider {
    fn name(&self) -> &str {
        "opencode"
    }

    fn display_name(&self) -> &str {
        "OpenCode Go"
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
        WireFormat::OpenaiChatCompletions
    }

    fn model_filter_regex(&self) -> Option<&'static str> {
        None
    }

    fn provider_schema(&self) -> &'static str {
        r#"
fields:
  api_key:
    f_type: string_long
    f_desc: "OpenCode API key. You can set $OPENCODE_API_KEY or paste a key from OpenCode settings."
    f_placeholder: "$OPENCODE_API_KEY or sk-..."
    f_label: "API Key"
    smartlinks:
      - sl_label: "Open OpenCode settings"
        sl_goto: "https://opencode.ai/settings"
  models_dev_provider_id:
    f_type: string
    f_desc: "models.dev provider id: opencode-go for OpenCode Go subscription, or opencode for the base Zen endpoint."
    f_default: "opencode-go"
    f_label: "Catalog Provider"
    f_extra: true
  usage_endpoint:
    f_type: string_long
    f_desc: "Optional OpenCode usage endpoint used for quota visualization when OpenCode exposes one. Only safe HTTPS opencode.ai endpoints are accepted."
    f_default: ""
    f_label: "Usage Endpoint"
    f_extra: true
description: |
  OpenCode Go / Zen subscription models via the OpenCode OpenAI-compatible API.
available:
  on_your_laptop_possible: true
  when_isolated_possible: true
"#
    }

    fn provider_settings_apply(&mut self, yaml: serde_yaml::Value) -> Result<(), String> {
        if let Some(api_key) = yaml.get("api_key").and_then(|v| v.as_str()) {
            if api_key != "***" {
                self.api_key = api_key.to_string();
            }
        }
        if let Some(provider_id) = yaml.get("models_dev_provider_id").and_then(|v| v.as_str()) {
            let provider_id = provider_id.trim();
            if !provider_id.is_empty() {
                self.models_dev_provider_id = provider_id.to_string();
            }
        }
        if let Some(usage_endpoint) = yaml.get("usage_endpoint").and_then(|v| v.as_str()) {
            self.usage_endpoint = usage_endpoint.trim().to_string();
        }
        if let Some(enabled) = yaml.get("enabled").and_then(|v| v.as_bool()) {
            self.enabled = enabled;
        }
        parse_enabled_models(&yaml, &mut self.enabled_models);
        parse_custom_models(&yaml, &mut self.custom_models);
        Ok(())
    }

    fn provider_settings_as_json(&self) -> serde_json::Value {
        json!({
            "api_key": if self.api_key.is_empty() { "" } else { "***" },
            "models_dev_provider_id": self.models_dev_provider_id,
            "usage_endpoint": self.usage_endpoint,
            "enabled": self.enabled,
            "enabled_models": self.enabled_models,
            "custom_models": self.custom_models
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let catalog = load_models_dev_snapshot_catalog()?;
        self.build_runtime_from_catalog(&catalog)
    }

    fn has_credentials(&self) -> bool {
        let key = resolve_env_var(&self.api_key, "", "opencode api_key");
        !key.is_empty()
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

    fn set_model_enabled(&mut self, model_id: &str, enabled: bool) {
        set_model_enabled_impl(&mut self.enabled_models, model_id, enabled);
    }

    fn add_custom_model(&mut self, model_id: String, config: CustomModelConfig) {
        self.custom_models.insert(model_id, config);
    }

    fn remove_custom_model(&mut self, model_id: &str) -> bool {
        self.custom_models.remove(model_id).is_some()
    }

    fn custom_model_pricing(&self, model_id: &str) -> Option<ModelPricing> {
        self.custom_models
            .get(model_id)
            .and_then(|config| config.pricing.clone())
    }

    async fn fetch_available_models(
        &self,
        _http_client: &reqwest::Client,
        _model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        match load_models_dev_snapshot_catalog() {
            Ok(catalog) => match self.available_models_from_catalog(&catalog) {
                Ok(models) => models,
                Err(e) => {
                    tracing::warn!("OpenCode: failed to build models.dev model list: {e}");
                    self.get_custom_models_only()
                }
            },
            Err(e) => {
                tracing::warn!("OpenCode: failed to load models.dev catalog: {e}");
                self.get_custom_models_only()
            }
        }
    }
}

fn first_window(data: &Value, keys: &[&str]) -> Option<OpenCodeUsageWindow> {
    keys.iter()
        .filter_map(|key| data.get(*key))
        .find_map(parse_usage_window)
}

fn format_usage_http_error(status: reqwest::StatusCode, content_type: &str, text: &str) -> String {
    let trimmed = text.trim();
    let is_html = content_type.contains("text/html")
        || trimmed.starts_with("<!DOCTYPE")
        || trimmed.starts_with("<html");
    if status == reqwest::StatusCode::NOT_FOUND && is_html {
        return "OpenCode quota API is not currently available; configure a usage endpoint after OpenCode exposes one"
            .to_string();
    }
    if is_html {
        return format!("OpenCode usage request failed with {status}: non-JSON response");
    }
    let preview = trimmed.chars().take(500).collect::<String>();
    if preview.is_empty() {
        format!("OpenCode usage request failed with {status}")
    } else {
        format!("OpenCode usage request failed with {status}: {preview}")
    }
}

fn parse_usage_window(value: &Value) -> Option<OpenCodeUsageWindow> {
    let object = value.as_object()?;
    let used_percent = first_f64_from_object(
        object,
        &[
            "used_percent",
            "usedPercent",
            "usage_percent",
            "usagePercent",
            "percent_used",
            "percentUsed",
            "utilization",
        ],
    )?;
    Some(OpenCodeUsageWindow {
        used_percent,
        reset_at: first_timestamp_or_string_from_object(
            object,
            &["reset_at", "resetAt", "resets_at", "resetsAt"],
        ),
        reset_after_seconds: first_u64_from_object(
            object,
            &[
                "reset_after_seconds",
                "resetAfterSeconds",
                "resetInSec",
                "reset_in_sec",
            ],
        ),
        limit_window_seconds: first_u64_from_object(
            object,
            &[
                "limit_window_seconds",
                "limitWindowSeconds",
                "window_seconds",
                "windowSeconds",
            ],
        ),
        status: first_string_from_object(object, &["status"]),
    })
}

fn first_timestamp_or_string_from_object(
    object: &Map<String, Value>,
    keys: &[&str],
) -> Option<String> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(timestamp_or_string))
}

fn timestamp_or_string(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    let timestamp = value.as_i64()?;
    if timestamp < 0 {
        return None;
    }
    let seconds = if timestamp > 1_000_000_000_000 {
        timestamp / 1000
    } else {
        timestamp
    };
    use std::time::UNIX_EPOCH;
    let datetime: chrono::DateTime<chrono::Utc> =
        (UNIX_EPOCH + Duration::from_secs(seconds as u64)).into();
    Some(datetime.to_rfc3339())
}

fn first_string(data: &Value, keys: &[&str]) -> Option<String> {
    let object = data.as_object()?;
    first_string_from_object(object, keys)
}

fn first_string_from_object(object: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        object.get(*key).and_then(|value| match value {
            Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            Value::Bool(value) => Some(value.to_string()),
            _ => None,
        })
    })
}

fn first_f64(data: &Value, keys: &[&str]) -> Option<f64> {
    let object = data.as_object()?;
    first_f64_from_object(object, keys)
}

fn first_f64_from_object(object: &Map<String, Value>, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(as_f64_loose))
}

fn first_u64_from_object(object: &Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(as_u64_loose))
}

fn as_f64_loose(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
    .filter(|value| value.is_finite())
}

fn as_u64_loose(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number
            .as_u64()
            .or_else(|| number.as_f64().map(|v| v as u64)),
        Value::String(text) => text.parse::<u64>().ok(),
        _ => None,
    }
}

fn collect_raw_extra(data: &Value, known_keys: &[&str]) -> Map<String, Value> {
    let mut raw_extra = Map::new();
    let Some(object) = data.as_object() else {
        return raw_extra;
    };
    for (key, value) in object {
        if !known_keys.iter().any(|known| known == key) {
            raw_extra.insert(key.clone(), value.clone());
        }
    }
    raw_extra
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_usage_parser_preserves_go_quota_windows() {
        let usage = OpenCodeProvider::parse_usage_payload(&json!({
            "data": {
                "plan": "lite",
                "workspaceID": "wrk_123",
                "balance": "12.5",
                "rollingUsage": {
                    "usagePercent": 42,
                    "reset_at": 1781096163,
                    "resetInSec": 3600,
                    "limitWindowSeconds": 18000,
                    "status": "ok"
                },
                "weeklyUsage": {
                    "usagePercent": "77",
                    "reset_after_seconds": "7200"
                },
                "monthlyUsage": {
                    "used_percent": 100,
                    "status": "rate-limited"
                },
                "unknown": {"kept": true}
            }
        }));

        assert_eq!(usage.plan_type.as_deref(), Some("lite"));
        assert_eq!(usage.workspace_id.as_deref(), Some("wrk_123"));
        assert_eq!(usage.balance, Some(12.5));
        let rolling = usage.rolling.unwrap();
        assert_eq!(rolling.used_percent, 42.0);
        assert_eq!(
            rolling.reset_at.as_deref(),
            Some("2026-06-10T12:56:03+00:00")
        );
        assert_eq!(usage.weekly.unwrap().reset_after_seconds, Some(7200));
        assert_eq!(
            usage.monthly.unwrap().status.as_deref(),
            Some("rate-limited")
        );
        assert!(usage.raw_extra.contains_key("unknown"));
    }

    #[tokio::test]
    async fn opencode_usage_endpoint_rejects_untrusted_hosts() {
        let provider = OpenCodeProvider {
            api_key: "sk-test".to_string(),
            usage_endpoint: "https://example.com/usage".to_string(),
            ..Default::default()
        };

        let err = provider
            .fetch_usage(&reqwest::Client::new())
            .await
            .unwrap_err();

        assert!(err.contains("OpenCode usage endpoint is not safe"));
    }
}
