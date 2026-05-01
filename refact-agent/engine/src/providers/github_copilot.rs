use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::caps::model_caps::ModelCapabilities;
use crate::llm::adapter::WireFormat;
use crate::providers::github_copilot_oauth::{resolve_api_base, OAuthTokens};
use crate::providers::traits::{
    merge_custom_models, parse_custom_models, parse_enabled_models, set_model_enabled_impl,
    AvailableModel, CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
};

const REQUEST_TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitHubCopilotProvider {
    #[serde(default)]
    pub oauth_tokens: OAuthTokens,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
}

impl GitHubCopilotProvider {
    fn resolve_token(&self) -> String {
        if self.oauth_tokens.has_valid_access_token() {
            self.oauth_tokens.access_token.clone()
        } else {
            String::new()
        }
    }

    fn api_base(&self) -> Result<String, String> {
        resolve_api_base(
            self.oauth_tokens.enterprise_url.as_deref(),
            self.oauth_tokens.api_base.as_deref(),
        )
    }

    fn chat_endpoint_for_api_base(api_base: &str) -> String {
        format!("{}/chat/completions", api_base.trim_end_matches('/'))
    }

    fn models_endpoint_for_api_base(api_base: &str) -> String {
        format!("{}/models", api_base.trim_end_matches('/'))
    }

    fn copilot_headers() -> HashMap<String, String> {
        HashMap::from([
            (
                "Openai-Intent".to_string(),
                "conversation-edits".to_string(),
            ),
            ("x-initiator".to_string(), "user".to_string()),
        ])
    }

    fn diagnose_auth_status(&self) -> String {
        if self.oauth_tokens.has_valid_access_token() {
            return "OK (GitHub Copilot OAuth login)".to_string();
        }
        if !self.oauth_tokens.access_token.is_empty() && self.oauth_tokens.is_expired() {
            return "GitHub Copilot OAuth token expired. Log in again.".to_string();
        }
        if let Err(error) = self.api_base() {
            return format!("GitHub Copilot API base is invalid: {error}");
        }
        "No credentials found".to_string()
    }

    fn redacted_oauth_tokens(&self) -> Value {
        json!({
            "access_token": if self.oauth_tokens.access_token.is_empty() { "" } else { "***" },
            "expires_at": self.oauth_tokens.expires_at,
            "enterprise_url": self.oauth_tokens.enterprise_url,
            "api_base": self.oauth_tokens.api_base,
        })
    }

    fn catalog_model_id(capability_key: &str) -> Option<&str> {
        ["github-copilot/", "github_copilot/"]
            .iter()
            .find_map(|prefix| capability_key.strip_prefix(prefix))
    }

    fn resolve_catalog_caps<'a>(
        model_caps: &'a HashMap<String, ModelCapabilities>,
        model_id: &str,
    ) -> Option<&'a ModelCapabilities> {
        ["github-copilot", "github_copilot"]
            .iter()
            .find_map(|provider| model_caps.get(&format!("{provider}/{model_id}")))
    }

    fn fetch_models_from_catalog(
        &self,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let enabled_set: HashSet<&str> = self.enabled_models.iter().map(|s| s.as_str()).collect();
        let mut models_map = HashMap::new();
        for (capability_key, caps) in model_caps {
            let Some(model_id) = Self::catalog_model_id(capability_key) else {
                continue;
            };
            let enabled =
                enabled_set.contains(model_id) || enabled_set.contains(capability_key.as_str());
            let pricing = self
                .custom_model_pricing(model_id)
                .or_else(|| self.custom_model_pricing(capability_key));
            models_map.insert(
                model_id.to_string(),
                AvailableModel::from_caps(model_id, caps, enabled, pricing),
            );
        }
        self.finish_models(models_map, &enabled_set)
    }

    async fn fetch_models_from_api(
        &self,
        http_client: &reqwest::Client,
        model_caps: &HashMap<String, ModelCapabilities>,
        access_token: &str,
        api_base: &str,
    ) -> Vec<AvailableModel> {
        let response = match tokio::time::timeout(
            Duration::from_secs(REQUEST_TIMEOUT_SECS),
            http_client
                .get(Self::models_endpoint_for_api_base(api_base))
                .header(
                    reqwest::header::AUTHORIZATION,
                    format!("Bearer {access_token}"),
                )
                .header(
                    reqwest::header::USER_AGENT,
                    format!("refact-lsp {}", env!("CARGO_PKG_VERSION")),
                )
                .send(),
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                tracing::warn!(
                    "GitHub Copilot: failed to reach /models (network error): {}, using models.dev catalog fallback",
                    error
                );
                return self.fetch_models_from_catalog(model_caps);
            }
            Err(_) => {
                tracing::warn!(
                    "GitHub Copilot: /models request timed out, using models.dev catalog fallback"
                );
                return self.fetch_models_from_catalog(model_caps);
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            tracing::warn!(
                "GitHub Copilot: /models returned {}. Check login/setup; using models.dev catalog fallback",
                status
            );
            return self.fetch_models_from_catalog(model_caps);
        }

        let body: Value = match response.json().await {
            Ok(body) => body,
            Err(error) => {
                tracing::warn!(
                    "GitHub Copilot: failed to parse /models response: {}, using models.dev catalog fallback",
                    error
                );
                return self.fetch_models_from_catalog(model_caps);
            }
        };

        match self.available_models_from_live_response(&body, model_caps, api_base) {
            Ok(models) => models,
            Err(error) => {
                tracing::warn!(
                    "GitHub Copilot: invalid /models response: {}, using models.dev catalog fallback",
                    error
                );
                self.fetch_models_from_catalog(model_caps)
            }
        }
    }

    fn available_models_from_live_response(
        &self,
        root: &Value,
        model_caps: &HashMap<String, ModelCapabilities>,
        api_base: &str,
    ) -> Result<Vec<AvailableModel>, String> {
        let models_array = root
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| "GitHub Copilot /models response missing data array".to_string())?;
        let enabled_set: HashSet<&str> = self.enabled_models.iter().map(|s| s.as_str()).collect();
        let mut models_map = HashMap::new();

        for model in models_array {
            let Some(id) = Self::live_model_id(model) else {
                continue;
            };
            if !Self::live_model_is_available(model) {
                continue;
            }
            let enabled = enabled_set.contains(id);
            let pricing = self.custom_model_pricing(id);
            let mut available = if let Some(caps) = Self::resolve_catalog_caps(model_caps, id) {
                AvailableModel::from_caps(id, caps, enabled, pricing)
            } else {
                self.unknown_live_model(id.to_string(), enabled, pricing.clone(), model)
            };

            available.display_name =
                Self::live_model_display_name(model).or(available.display_name);
            self.apply_live_capabilities(&mut available, model);
            self.apply_endpoint_override(&mut available, model, api_base);
            models_map.insert(id.to_string(), available);
        }

        Ok(self.finish_models(models_map, &enabled_set))
    }

    fn finish_models(
        &self,
        mut models_map: HashMap<String, AvailableModel>,
        enabled_set: &HashSet<&str>,
    ) -> Vec<AvailableModel> {
        let mut models: Vec<AvailableModel> = models_map.drain().map(|(_, model)| model).collect();
        merge_custom_models(&mut models, &self.custom_models, enabled_set);
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models
    }

    fn live_model_id(model: &Value) -> Option<&str> {
        model
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
    }

    fn live_model_display_name(model: &Value) -> Option<String> {
        model
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(ToString::to_string)
    }

    fn live_model_is_available(model: &Value) -> bool {
        if model
            .get("model_picker_enabled")
            .and_then(Value::as_bool)
            .is_some_and(|enabled| !enabled)
        {
            return false;
        }
        let policy_state = model
            .get("policy")
            .and_then(|policy| policy.get("state"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .replace('-', "_")
            .replace(' ', "_");
        !matches!(
            policy_state.as_str(),
            "disabled" | "policy_disabled" | "denied" | "blocked" | "not_entitled"
        )
    }

    fn live_limits(model: &Value) -> Option<&Value> {
        model.get("capabilities")?.get("limits")
    }

    fn live_supports(model: &Value) -> Option<&Value> {
        model.get("capabilities")?.get("supports")
    }

    fn live_usize_field(obj: &Value, key: &str) -> Option<usize> {
        obj.get(key)
            .and_then(Value::as_u64)
            .map(|value| value as usize)
    }

    fn live_bool_field(obj: &Value, key: &str) -> Option<bool> {
        obj.get(key).and_then(Value::as_bool)
    }

    fn live_reasoning_effort(model: &Value) -> Option<Vec<String>> {
        let efforts = Self::live_supports(model)?
            .get("reasoning_effort")
            .and_then(Value::as_array)?
            .iter()
            .filter_map(|value| value.as_str().map(ToString::to_string))
            .collect::<Vec<_>>();
        (!efforts.is_empty()).then_some(efforts)
    }

    fn live_supports_vision(model: &Value) -> bool {
        if Self::live_supports(model)
            .and_then(|supports| Self::live_bool_field(supports, "vision"))
            .unwrap_or(false)
        {
            return true;
        }
        Self::live_limits(model)
            .and_then(|limits| limits.get("vision"))
            .and_then(|vision| vision.get("supported_media_types"))
            .and_then(Value::as_array)
            .map(|types| {
                types
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|media_type| media_type.starts_with("image/"))
            })
            .unwrap_or(false)
    }

    fn apply_live_capabilities(&self, available: &mut AvailableModel, model: &Value) {
        if let Some(limits) = Self::live_limits(model) {
            if let Some(n_ctx) = Self::live_usize_field(limits, "max_context_window_tokens") {
                available.n_ctx = n_ctx;
            } else if let Some(n_ctx) = Self::live_usize_field(limits, "max_prompt_tokens") {
                available.n_ctx = n_ctx;
            }
            if let Some(max_output) = Self::live_usize_field(limits, "max_output_tokens") {
                available.max_output_tokens = Some(max_output);
            }
        }
        if let Some(supports) = Self::live_supports(model) {
            if let Some(tool_calls) = Self::live_bool_field(supports, "tool_calls") {
                available.supports_tools = tool_calls;
            }
            if let Some(structured) = Self::live_bool_field(supports, "structured_outputs") {
                available.supports_strict_tools = structured;
            }
            if let Some(adaptive) = Self::live_bool_field(supports, "adaptive_thinking") {
                available.supports_adaptive_thinking_budget = adaptive;
            }
            available.supports_thinking_budget = available.supports_thinking_budget
                || supports.get("max_thinking_budget").is_some()
                || supports.get("min_thinking_budget").is_some();
        }
        available.supports_multimodality = Self::live_supports_vision(model);
        if let Some(reasoning) = Self::live_reasoning_effort(model) {
            available.reasoning_effort_options = Some(reasoning);
        }
    }

    fn apply_endpoint_override(
        &self,
        available: &mut AvailableModel,
        model: &Value,
        api_base: &str,
    ) {
        let Some(endpoints) = model.get("supported_endpoints").and_then(Value::as_array) else {
            return;
        };
        let endpoint_values: Vec<&str> = endpoints.iter().filter_map(Value::as_str).collect();
        let api_base = api_base.trim_end_matches('/');
        if endpoint_values
            .iter()
            .any(|endpoint| *endpoint == "/v1/messages")
        {
            available.wire_format_override = Some(WireFormat::AnthropicMessages);
            available.endpoint_override = Some(format!("{}/v1/messages", api_base));
        } else if endpoint_values
            .iter()
            .any(|endpoint| *endpoint == "/v1/chat/completions")
        {
            available.wire_format_override = Some(WireFormat::OpenaiChatCompletions);
            available.endpoint_override = Some(format!("{}/v1/chat/completions", api_base));
        } else if endpoint_values
            .iter()
            .any(|endpoint| *endpoint == "/v1/responses")
        {
            available.wire_format_override = Some(WireFormat::OpenaiResponses);
            available.endpoint_override = Some(format!("{}/v1/responses", api_base));
        } else if endpoint_values
            .iter()
            .any(|endpoint| *endpoint == "/responses")
        {
            available.wire_format_override = Some(WireFormat::OpenaiResponses);
            available.endpoint_override = Some(format!("{}/responses", api_base));
        }
    }

    fn unknown_live_model(
        &self,
        id: String,
        enabled: bool,
        pricing: Option<ModelPricing>,
        model: &Value,
    ) -> AvailableModel {
        let n_ctx = Self::live_limits(model)
            .and_then(|limits| Self::live_usize_field(limits, "max_context_window_tokens"))
            .or_else(|| {
                Self::live_limits(model)
                    .and_then(|limits| Self::live_usize_field(limits, "max_prompt_tokens"))
            })
            .unwrap_or(8192);
        let max_output_tokens = Self::live_limits(model)
            .and_then(|limits| Self::live_usize_field(limits, "max_output_tokens"));
        let supports_tools = Self::live_supports(model)
            .and_then(|supports| Self::live_bool_field(supports, "tool_calls"))
            .unwrap_or(false);
        let supports_strict_tools = Self::live_supports(model)
            .and_then(|supports| Self::live_bool_field(supports, "structured_outputs"))
            .unwrap_or(false);
        let supports_adaptive_thinking_budget = Self::live_supports(model)
            .and_then(|supports| Self::live_bool_field(supports, "adaptive_thinking"))
            .unwrap_or(false);
        let supports_thinking_budget = Self::live_supports(model)
            .map(|supports| {
                supports.get("max_thinking_budget").is_some()
                    || supports.get("min_thinking_budget").is_some()
            })
            .unwrap_or(false);

        AvailableModel {
            id,
            display_name: None,
            n_ctx,
            supports_tools,
            supports_parallel_tools: false,
            supports_strict_tools,
            supports_multimodality: Self::live_supports_vision(model),
            reasoning_effort_options: Self::live_reasoning_effort(model),
            supports_thinking_budget,
            supports_adaptive_thinking_budget,
            supports_cache_control: true,
            tokenizer: None,
            enabled,
            is_custom: false,
            pricing,
            available_providers: Vec::new(),
            selected_provider: None,
            max_output_tokens,
            provider_variants: Vec::new(),
            wire_format_override: None,
            endpoint_override: None,
            base_model: None,
        }
    }
}

#[async_trait]
impl ProviderTrait for GitHubCopilotProvider {
    fn name(&self) -> &str {
        "github_copilot"
    }

    fn display_name(&self) -> &str {
        "GitHub Copilot"
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
fields: {}
oauth:
  supported: true
  methods:
    - id: github
      label: "GitHub Copilot"
      description: "Login with your GitHub account that has an active Copilot subscription"
description: |
  Use your GitHub Copilot subscription through GitHub's Copilot API.

  **Setup:** Click **Login with GitHub Copilot**, enter the device code on GitHub, then select models.
available:
  on_your_laptop_possible: true
  when_isolated_possible: true
"#
    }

    fn provider_settings_apply(&mut self, yaml: serde_yaml::Value) -> Result<(), String> {
        if let Some(oauth_tokens) = yaml.get("oauth_tokens") {
            self.oauth_tokens = serde_yaml::from_value(oauth_tokens.clone()).unwrap_or_default();
        }
        parse_enabled_models(&yaml, &mut self.enabled_models);
        parse_custom_models(&yaml, &mut self.custom_models);
        Ok(())
    }

    fn provider_settings_as_json(&self) -> Value {
        json!({
            "auth_status": self.diagnose_auth_status(),
            "oauth_connected": self.oauth_tokens.has_valid_access_token(),
            "oauth_tokens": self.redacted_oauth_tokens(),
            "enabled_models": self.enabled_models,
            "custom_models": self.custom_models,
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let api_base = self.api_base()?;
        let token = self.resolve_token();
        let has_auth = !token.is_empty();
        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: has_auth && !self.enabled_models.is_empty(),
            readonly: false,
            wire_format: self.default_wire_format(),
            chat_endpoint: Self::chat_endpoint_for_api_base(&api_base),
            completion_endpoint: String::new(),
            embedding_endpoint: String::new(),
            api_key: token,
            auth_token: String::new(),
            tokenizer_api_key: String::new(),
            extra_headers: Self::copilot_headers(),
            supports_cache_control: true,
            chat_models: Vec::new(),
            completion_models: Vec::new(),
            embedding_model: None,
        })
    }

    fn has_credentials(&self) -> bool {
        self.oauth_tokens.has_valid_access_token()
    }

    fn model_source(&self) -> ModelSource {
        if self.oauth_tokens.has_valid_access_token() {
            ModelSource::Api
        } else {
            ModelSource::ModelCaps
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
        let token = self.resolve_token();
        if token.is_empty() {
            return self.fetch_models_from_catalog(model_caps);
        }
        match self.api_base() {
            Ok(api_base) => {
                self.fetch_models_from_api(http_client, model_caps, &token, &api_base)
                    .await
            }
            Err(error) => {
                tracing::warn!(
                    "GitHub Copilot: invalid API base: {}, using models.dev catalog fallback",
                    error
                );
                self.fetch_models_from_catalog(model_caps)
            }
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::github_copilot_oauth::DEFAULT_COPILOT_API_BASE;
    use crate::providers::traits::ProviderTrait;
    use serde_json::json;

    fn copilot_caps(n_ctx: usize) -> ModelCapabilities {
        ModelCapabilities {
            n_ctx,
            max_output_tokens: 4096,
            supports_tools: true,
            supports_parallel_tools: true,
            supports_vision: false,
            pricing: Some(ModelPricing {
                prompt: 0.0,
                generated: 0.0,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn caps_map() -> HashMap<String, ModelCapabilities> {
        HashMap::from([
            ("github-copilot/gpt-4.1".to_string(), copilot_caps(128_000)),
            (
                "github_copilot/claude-sonnet-4".to_string(),
                copilot_caps(200_000),
            ),
            ("openai/gpt-4.1".to_string(), copilot_caps(64_000)),
        ])
    }

    fn provider_with_token() -> GitHubCopilotProvider {
        GitHubCopilotProvider {
            oauth_tokens: OAuthTokens {
                access_token: "gho-token".to_string(),
                expires_at: 0,
                api_base: Some(DEFAULT_COPILOT_API_BASE.to_string()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn github_copilot_settings_redact_token_and_report_auth_status() {
        let provider = provider_with_token();
        let settings = provider.provider_settings_as_json();

        assert_eq!(settings["auth_status"], "OK (GitHub Copilot OAuth login)");
        assert_eq!(settings["oauth_connected"], true);
        assert_eq!(settings["oauth_tokens"]["access_token"], "***");
        assert_eq!(
            settings["oauth_tokens"]["api_base"],
            DEFAULT_COPILOT_API_BASE
        );
        assert!(!settings.to_string().contains("gho-token"));
    }

    #[test]
    fn github_copilot_runtime_requires_credentials_and_selected_models() {
        let mut provider = GitHubCopilotProvider::default();
        provider.enabled_models = vec!["gpt-4.1".to_string()];
        let no_token = provider.build_runtime().unwrap();
        assert!(!no_token.enabled);

        provider.oauth_tokens.access_token = "gho-token".to_string();
        let enabled = provider.build_runtime().unwrap();
        assert!(enabled.enabled);
        assert_eq!(enabled.api_key, "gho-token");
        assert_eq!(
            enabled.chat_endpoint,
            "https://api.githubcopilot.com/chat/completions"
        );
    }

    #[test]
    fn github_copilot_runtime_headers_include_required_copilot_headers() {
        let mut provider = provider_with_token();
        provider.enabled_models = vec!["gpt-4.1".to_string()];

        let runtime = provider.build_runtime().unwrap();

        assert_eq!(
            runtime
                .extra_headers
                .get("Openai-Intent")
                .map(String::as_str),
            Some("conversation-edits")
        );
        assert_eq!(
            runtime.extra_headers.get("x-initiator").map(String::as_str),
            Some("user")
        );
        assert!(runtime.extra_headers.get("Authorization").is_none());
        assert!(runtime.extra_headers.get("authorization").is_none());
    }

    #[test]
    fn github_copilot_live_models_filter_and_map_capabilities() {
        let mut provider = provider_with_token();
        provider.enabled_models = vec!["gpt-4.1".to_string()];
        let live = json!({
            "data": [
                {
                    "model_picker_enabled": true,
                    "id": "gpt-4.1",
                    "name": "GPT 4.1",
                    "version": "gpt-4.1-2025-04-14",
                    "supported_endpoints": ["/v1/responses"],
                    "policy": {"state": "enabled"},
                    "capabilities": {
                        "limits": {
                            "max_context_window_tokens": 256000,
                            "max_output_tokens": 8192,
                            "max_prompt_tokens": 240000,
                            "vision": {
                                "supported_media_types": ["image/png"]
                            }
                        },
                        "supports": {
                            "adaptive_thinking": true,
                            "max_thinking_budget": 16384,
                            "reasoning_effort": ["low", "high"],
                            "streaming": true,
                            "structured_outputs": true,
                            "tool_calls": true,
                            "vision": true
                        }
                    }
                },
                {
                    "model_picker_enabled": false,
                    "id": "picker-off",
                    "name": "Picker Off",
                    "capabilities": {"limits": {}, "supports": {"tool_calls": true, "streaming": true}}
                },
                {
                    "model_picker_enabled": true,
                    "id": "policy-disabled",
                    "name": "Policy Disabled",
                    "policy": {"state": "disabled"},
                    "capabilities": {"limits": {}, "supports": {"tool_calls": true, "streaming": true}}
                }
            ]
        });

        let models = provider
            .available_models_from_live_response(&live, &HashMap::new(), DEFAULT_COPILOT_API_BASE)
            .unwrap();
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();

        assert_eq!(ids, vec!["gpt-4.1"]);
        let model = &models[0];
        assert!(model.enabled);
        assert_eq!(model.display_name.as_deref(), Some("GPT 4.1"));
        assert_eq!(model.n_ctx, 256000);
        assert_eq!(model.max_output_tokens, Some(8192));
        assert!(model.supports_tools);
        assert!(model.supports_strict_tools);
        assert!(model.supports_multimodality);
        assert!(model.supports_thinking_budget);
        assert!(model.supports_adaptive_thinking_budget);
        assert_eq!(
            model.reasoning_effort_options.as_ref().unwrap(),
            &vec!["low".to_string(), "high".to_string()]
        );
        assert_eq!(
            model.wire_format_override,
            Some(WireFormat::OpenaiResponses)
        );
        assert_eq!(
            model.endpoint_override.as_deref(),
            Some("https://api.githubcopilot.com/v1/responses")
        );
    }

    #[test]
    fn github_copilot_live_messages_endpoint_maps_anthropic_wire_format() {
        let mut provider = provider_with_token();
        provider.enabled_models = vec!["claude-sonnet-4".to_string()];
        let live = json!({
            "data": [{
                "model_picker_enabled": true,
                "id": "claude-sonnet-4",
                "name": "Claude Sonnet 4",
                "supported_endpoints": ["/v1/messages"],
                "policy": {"state": "enabled"},
                "capabilities": {"limits": {}, "supports": {"tool_calls": true}}
            }]
        });

        let models = provider
            .available_models_from_live_response(&live, &caps_map(), DEFAULT_COPILOT_API_BASE)
            .unwrap();
        let model = models
            .iter()
            .find(|model| model.id == "claude-sonnet-4")
            .unwrap();

        assert!(model.enabled);
        assert_eq!(
            model.wire_format_override,
            Some(WireFormat::AnthropicMessages)
        );
        assert_eq!(
            model.endpoint_override.as_deref(),
            Some("https://api.githubcopilot.com/v1/messages")
        );
    }

    #[test]
    fn github_copilot_live_chat_endpoint_still_maps_openai_chat() {
        let provider = provider_with_token();
        let live = json!({
            "data": [{
                "model_picker_enabled": true,
                "id": "gpt-4.1",
                "name": "GPT 4.1",
                "supported_endpoints": ["/v1/chat/completions"],
                "policy": {"state": "enabled"},
                "capabilities": {"limits": {}, "supports": {"tool_calls": true}}
            }]
        });

        let models = provider
            .available_models_from_live_response(&live, &caps_map(), DEFAULT_COPILOT_API_BASE)
            .unwrap();
        let model = models.iter().find(|model| model.id == "gpt-4.1").unwrap();

        assert_eq!(
            model.wire_format_override,
            Some(WireFormat::OpenaiChatCompletions)
        );
        assert_eq!(
            model.endpoint_override.as_deref(),
            Some("https://api.githubcopilot.com/v1/chat/completions")
        );
    }

    #[test]
    fn github_copilot_models_dev_fallback_includes_catalog_and_custom_models() {
        let mut provider = GitHubCopilotProvider::default();
        provider.enabled_models = vec!["gpt-4.1".to_string(), "custom-copilot".to_string()];
        provider.custom_models.insert(
            "custom-copilot".to_string(),
            CustomModelConfig {
                n_ctx: Some(4096),
                supports_tools: Some(true),
                ..Default::default()
            },
        );

        let models = provider.fetch_models_from_catalog(&caps_map());
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();

        assert_eq!(ids, vec!["claude-sonnet-4", "custom-copilot", "gpt-4.1"]);
        assert!(
            models
                .iter()
                .find(|model| model.id == "gpt-4.1")
                .unwrap()
                .enabled
        );
        assert!(
            models
                .iter()
                .find(|model| model.id == "custom-copilot")
                .unwrap()
                .is_custom
        );
        assert!(!ids.contains(&"openai/gpt-4.1"));
    }
}
