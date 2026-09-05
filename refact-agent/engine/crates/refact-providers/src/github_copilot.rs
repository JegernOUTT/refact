use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use refact_core::provider_types::{available_model_from_catalog_and_live, LiveModelFields};
use refact_core::model_caps::ModelCapabilities;
use refact_core::llm_types::WireFormat;
use crate::github_copilot_oauth::{resolve_api_base, OAuthTokens};
use crate::traits::{
    merge_custom_models, parse_custom_models, parse_enabled_models, set_model_enabled_impl,
    AvailableModel, CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
};

const REQUEST_TIMEOUT_SECS: u64 = 10;

// The exact `supported_endpoints` literals GitHub returns. There is no `/v1` prefix on the
// OpenAI-shaped routes: `/v1/chat/completions` and `/v1/responses` appear in no live record and
// return 404 when requested.
const CHAT_COMPLETIONS_ENDPOINT: &str = "/chat/completions";
const ANTHROPIC_MESSAGES_ENDPOINT: &str = "/v1/messages";
const RESPONSES_ENDPOINT: &str = "/responses";
const RESPONSES_WEBSOCKET_ENDPOINT: &str = "ws:/responses";

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

    fn resolve_catalog_caps<'a>(
        model_caps: &'a HashMap<String, ModelCapabilities>,
        model_id: &str,
    ) -> Option<&'a ModelCapabilities> {
        ["github-copilot", "github_copilot"]
            .iter()
            .find_map(|provider| model_caps.get(&format!("{provider}/{model_id}")))
    }

    /// Every failure path ends here: the models.dev snapshot carries github-copilot ids that the
    /// live API does not serve, so offering it as a fallback invents models the account cannot
    /// call. Only user-configured custom models survive a failed discovery.
    fn fallback_models(&self) -> Vec<AvailableModel> {
        self.get_custom_models_only()
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
                    "GitHub Copilot: transient failure reaching /models (network error): {}. Keeping custom models only, retry later",
                    error
                );
                return self.fallback_models();
            }
            Err(_) => {
                tracing::warn!(
                    "GitHub Copilot: transient failure, /models request timed out after {}s. Keeping custom models only, retry later",
                    REQUEST_TIMEOUT_SECS
                );
                return self.fallback_models();
            }
        };

        let status = response.status();
        if !status.is_success() {
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                tracing::warn!(
                    "GitHub Copilot: /models returned {}. The GitHub token is rejected — log in again. Keeping custom models only",
                    status
                );
            } else if status.is_server_error() {
                tracing::warn!(
                    "GitHub Copilot: transient failure, /models returned {}. Keeping custom models only, retry later",
                    status
                );
            } else {
                tracing::warn!(
                    "GitHub Copilot: /models returned {}. Keeping custom models only",
                    status
                );
            }
            return self.fallback_models();
        }

        let body: Value = match response.json().await {
            Ok(body) => body,
            Err(error) => {
                tracing::warn!(
                    "GitHub Copilot: /models returned a body that is not JSON: {}. Keeping custom models only",
                    error
                );
                return self.fallback_models();
            }
        };

        match self.available_models_from_live_response(&body, model_caps, api_base) {
            Ok(models) => models,
            Err(error) => {
                tracing::warn!(
                    "GitHub Copilot: unusable /models response: {}. Keeping custom models only",
                    error
                );
                self.fallback_models()
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
            let enabled = enabled_set.contains(id)
                || enabled_set.contains(format!("github-copilot/{id}").as_str())
                || enabled_set.contains(format!("github_copilot/{id}").as_str());
            let live = Self::live_fields(model, api_base);
            // 0, never a made-up number: this provider does not fabricate a context size.
            let available = available_model_from_catalog_and_live(
                id,
                Self::resolve_catalog_caps(model_caps, id),
                &live,
                enabled,
                0,
            );
            // Backstop, expected to be unreachable: every non-embeddings live record carries
            // `capabilities.limits.max_context_window_tokens`. A model with no context window
            // from either the live payload or models.dev is unusable, so drop it loudly rather
            // than publish a zero or invented budget.
            if available.n_ctx == 0 {
                tracing::warn!(
                    "GitHub Copilot: dropping model {} — neither /models nor models.dev reported a context window",
                    id
                );
                continue;
            }
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

    fn live_model_kind(model: &Value) -> Option<&str> {
        model
            .get("capabilities")?
            .get("type")
            .and_then(Value::as_str)
            .map(str::trim)
    }

    /// `model_picker_enabled` is deliberately NOT consulted. GitHub now returns it as `false` for
    /// every record while still shipping a `model_picker_category` for most of them, so treating
    /// it as an availability flag emptied the whole provider. The authoritative signals are the
    /// account policy state and the model kind.
    fn live_model_is_available(model: &Value) -> bool {
        if Self::live_model_kind(model).is_some_and(|kind| kind.eq_ignore_ascii_case("embeddings"))
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

    fn live_string_field(obj: &Value, key: &str) -> Option<String> {
        obj.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    }

    fn live_reasoning_effort(model: &Value) -> Option<Vec<String>> {
        Some(
            Self::live_supports(model)?
                .get("reasoning_effort")
                .and_then(Value::as_array)?
                .iter()
                .filter_map(|value| value.as_str().map(ToString::to_string))
                .collect::<Vec<_>>(),
        )
    }

    fn live_supports_vision(model: &Value) -> Option<bool> {
        if let Some(vision) = Self::live_supports(model)
            .and_then(|supports| Self::live_bool_field(supports, "vision"))
        {
            return Some(vision);
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
    }

    /// The literals below are the only values GitHub actually ships in `supported_endpoints`.
    /// `/v1/chat/completions` and `/v1/responses` never appear and 404 when called.
    fn live_endpoint_fields(model: &Value, api_base: &str) -> (Option<WireFormat>, Option<String>) {
        let Some(endpoints) = model.get("supported_endpoints").and_then(Value::as_array) else {
            return (None, None);
        };
        let endpoint_values: Vec<&str> = endpoints
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .collect();
        let api_base = api_base.trim_end_matches('/');
        let messages = endpoint_values.contains(&ANTHROPIC_MESSAGES_ENDPOINT);
        let chat = endpoint_values.contains(&CHAT_COMPLETIONS_ENDPOINT);
        let responses = endpoint_values.contains(&RESPONSES_ENDPOINT);
        // `ws:/responses` is the websocket transport of the very same Responses API. refact only
        // speaks HTTP, so it can never become an endpoint_override on its own; it is recognised
        // here purely so a websocket-only record is skipped loudly instead of silently.
        let websocket_responses = endpoint_values.contains(&RESPONSES_WEBSOCKET_ENDPOINT);
        if websocket_responses && !messages && !chat && !responses {
            tracing::debug!(
                "GitHub Copilot: model {} only advertises the websocket Responses endpoint, keeping the provider default wire format",
                Self::live_model_id(model).unwrap_or("<unknown>")
            );
            return (None, None);
        }
        // Records routinely advertise several families at once (Claude models ship both
        // `/v1/messages` and `/chat/completions`). Prefer the most expressive native protocol
        // instead of bailing out, and only give up when nothing recognisable is advertised.
        if messages {
            return (
                Some(WireFormat::AnthropicMessages),
                Some(format!("{api_base}{ANTHROPIC_MESSAGES_ENDPOINT}")),
            );
        }
        if responses {
            return (
                Some(WireFormat::OpenaiResponses),
                Some(format!("{api_base}{RESPONSES_ENDPOINT}")),
            );
        }
        if chat {
            return (
                Some(WireFormat::OpenaiChatCompletions),
                Some(format!("{api_base}{CHAT_COMPLETIONS_ENDPOINT}")),
            );
        }
        (None, None)
    }

    fn live_fields(model: &Value, api_base: &str) -> LiveModelFields {
        let supports = Self::live_supports(model);
        let (wire_format_override, endpoint_override) = Self::live_endpoint_fields(model, api_base);
        let max_thinking_tokens =
            supports.and_then(|supports| Self::live_usize_field(supports, "max_thinking_budget"));
        let min_thinking_budget =
            supports.and_then(|supports| Self::live_usize_field(supports, "min_thinking_budget"));
        LiveModelFields {
            display_name: Self::live_model_display_name(model),
            n_ctx: Self::live_limits(model)
                .and_then(|limits| Self::live_usize_field(limits, "max_context_window_tokens"))
                .or_else(|| {
                    Self::live_limits(model)
                        .and_then(|limits| Self::live_usize_field(limits, "max_prompt_tokens"))
                }),
            max_output_tokens: Self::live_limits(model)
                .and_then(|limits| Self::live_usize_field(limits, "max_output_tokens")),
            supports_tools: supports.and_then(|value| Self::live_bool_field(value, "tool_calls")),
            supports_parallel_tools: supports
                .and_then(|value| Self::live_bool_field(value, "parallel_tool_calls")),
            supports_strict_tools: supports
                .and_then(|value| Self::live_bool_field(value, "structured_outputs")),
            supports_multimodality: Self::live_supports_vision(model),
            reasoning_effort_options: Self::live_reasoning_effort(model),
            supports_thinking_budget: (max_thinking_tokens.is_some()
                || min_thinking_budget.is_some())
            .then_some(true),
            supports_adaptive_thinking_budget: supports
                .and_then(|value| Self::live_bool_field(value, "adaptive_thinking")),
            max_thinking_tokens,
            min_thinking_budget,
            tokenizer: model
                .get("capabilities")
                .and_then(|capabilities| Self::live_string_field(capabilities, "tokenizer")),
            upstream_provider: Self::live_string_field(model, "vendor"),
            wire_format_override,
            endpoint_override,
            ..Default::default()
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

    /// The default trait implementation enumerates the models.dev catalog. For GitHub Copilot the
    /// baked-in snapshot lists ids the live API does not serve, so callers that reach for the
    /// caps fallback — `caps.rs` and `providers/http.rs` both do it on their outer discovery
    /// timeout — must get custom models only, exactly like every in-provider failure path.
    fn get_available_models_from_caps(
        &self,
        _model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        self.fallback_models()
    }

    async fn fetch_available_models(
        &self,
        http_client: &reqwest::Client,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let token = self.resolve_token();
        if token.is_empty() {
            tracing::warn!(
                "GitHub Copilot: no usable GitHub token — log in again. Keeping custom models only"
            );
            return self.fallback_models();
        }
        match self.api_base() {
            Ok(api_base) => {
                self.fetch_models_from_api(http_client, model_caps, &token, &api_base)
                    .await
            }
            Err(error) => {
                tracing::warn!(
                    "GitHub Copilot: invalid API base: {}. Keeping custom models only",
                    error
                );
                self.fallback_models()
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
    use crate::github_copilot_oauth::DEFAULT_COPILOT_API_BASE;
    use crate::traits::ProviderTrait;
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

    /// A record shaped exactly like the live `GET https://api.githubcopilot.com/models` payload.
    fn live_record(id: &str, endpoints: Value, capabilities: Value) -> Value {
        json!({
            "id": id,
            "name": id,
            "object": "model",
            "vendor": "OpenAI",
            "version": format!("{id}-2025-04-14"),
            "preview": false,
            // Every live record ships this as false; it must not affect availability.
            "model_picker_enabled": false,
            "model_picker_category": "versatile",
            "policy": {"state": "enabled", "terms": "Enable access"},
            "supported_endpoints": endpoints,
            "capabilities": capabilities
        })
    }

    fn chat_capabilities() -> Value {
        json!({
            "object": "model_capabilities",
            "type": "chat",
            "family": "gpt-4.1",
            "tokenizer": "o200k_base",
            "limits": {
                "max_context_window_tokens": 128000,
                "max_prompt_tokens": 120000,
                "max_output_tokens": 16384
            },
            "supports": {"tool_calls": true, "streaming": true}
        })
    }

    #[test]
    fn github_copilot_live_models_keep_records_with_model_picker_enabled_false() {
        let mut provider = provider_with_token();
        provider.enabled_models = vec!["gpt-4.1".to_string()];
        // Exactly the production situation: every record says model_picker_enabled=false.
        let live = json!({
            "object": "list",
            "data": [live_record(
                "gpt-4.1",
                json!([CHAT_COMPLETIONS_ENDPOINT]),
                chat_capabilities(),
            )]
        });

        let models = provider
            .available_models_from_live_response(&live, &HashMap::new(), DEFAULT_COPILOT_API_BASE)
            .unwrap();
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();

        assert_eq!(
            ids,
            vec!["gpt-4.1"],
            "model_picker_enabled=false must not remove a policy-enabled chat model"
        );
        assert!(models[0].enabled);
    }

    #[test]
    fn github_copilot_live_models_drop_policy_disabled_and_embeddings_records() {
        let provider = provider_with_token();
        let mut embeddings_capabilities = chat_capabilities();
        embeddings_capabilities["type"] = json!("embeddings");
        embeddings_capabilities["supports"] = json!({"dimensions": true});
        let mut policy_disabled = live_record(
            "policy-disabled",
            json!([CHAT_COMPLETIONS_ENDPOINT]),
            chat_capabilities(),
        );
        policy_disabled["policy"] = json!({"state": "disabled", "terms": "Enable access"});

        let live = json!({
            "object": "list",
            "data": [
                live_record("gpt-4.1", json!([CHAT_COMPLETIONS_ENDPOINT]), chat_capabilities()),
                policy_disabled,
                live_record(
                    "text-embedding-3-small",
                    json!([CHAT_COMPLETIONS_ENDPOINT]),
                    embeddings_capabilities,
                )
            ]
        });

        let models = provider
            .available_models_from_live_response(&live, &HashMap::new(), DEFAULT_COPILOT_API_BASE)
            .unwrap();
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();

        assert_eq!(ids, vec!["gpt-4.1"]);
    }

    #[test]
    fn github_copilot_live_models_map_capabilities_from_the_real_payload_shape() {
        let mut provider = provider_with_token();
        provider.enabled_models = vec!["gpt-5".to_string()];
        let mut record = live_record("gpt-5", json!([RESPONSES_ENDPOINT]), chat_capabilities());
        record["capabilities"]["limits"]["vision"] = json!({
            "max_prompt_image_size": 3145728,
            "max_prompt_images": 1,
            "supported_media_types": ["image/jpeg", "image/png", "image/webp"]
        });
        record["capabilities"]["supports"] = json!({
            "tool_calls": true,
            "parallel_tool_calls": true,
            "streaming": true,
            "structured_outputs": true,
            "vision": true,
            "reasoning_effort": ["low", "medium", "high"],
            "max_thinking_budget": 16384,
            "min_thinking_budget": 1024,
            "adaptive_thinking": true
        });
        let live = json!({"object": "list", "data": [record]});

        let models = provider
            .available_models_from_live_response(&live, &HashMap::new(), DEFAULT_COPILOT_API_BASE)
            .unwrap();
        let model = &models[0];

        assert!(model.enabled);
        assert_eq!(model.display_name.as_deref(), Some("gpt-5"));
        assert_eq!(model.n_ctx, 128000);
        assert_eq!(model.max_output_tokens, Some(16384));
        assert!(model.supports_tools);
        assert!(model.supports_parallel_tools);
        assert!(model.supports_strict_tools);
        assert!(model.supports_multimodality);
        assert!(model.supports_thinking_budget);
        assert!(model.supports_adaptive_thinking_budget);
        assert_eq!(
            model.reasoning_effort_options.as_ref().unwrap(),
            &vec!["low".to_string(), "medium".to_string(), "high".to_string()]
        );
        assert_eq!(model.tokenizer.as_deref(), Some("o200k_base"));
        assert_eq!(model.upstream_provider.as_deref(), Some("OpenAI"));
        assert_eq!(
            model.wire_format_override,
            Some(WireFormat::OpenaiResponses)
        );
        assert_eq!(
            model.endpoint_override.as_deref(),
            Some("https://api.githubcopilot.com/responses")
        );
    }

    #[test]
    fn github_copilot_live_thinking_budget_numbers_survive_into_the_model() {
        let provider = provider_with_token();
        let mut record = live_record(
            "claude-sonnet-4.5",
            json!([ANTHROPIC_MESSAGES_ENDPOINT]),
            chat_capabilities(),
        );
        record["capabilities"]["supports"] = json!({
            "tool_calls": true,
            "max_thinking_budget": 65536,
            "min_thinking_budget": 1024
        });
        let live = json!({"object": "list", "data": [record]});

        let models = provider
            .available_models_from_live_response(&live, &HashMap::new(), DEFAULT_COPILOT_API_BASE)
            .unwrap();
        let model = &models[0];

        assert!(model.supports_thinking_budget);
        assert_eq!(model.max_thinking_tokens, Some(65536));
        assert_eq!(model.min_thinking_budget, Some(1024));
        assert_eq!(model.live_fields.max_thinking_tokens, Some(65536));
        assert_eq!(model.live_fields.min_thinking_budget, Some(1024));
    }

    #[test]
    fn github_copilot_live_parallel_tool_calls_false_is_authoritative() {
        let provider = provider_with_token();
        let mut record = live_record(
            "no-parallel",
            json!([CHAT_COMPLETIONS_ENDPOINT]),
            chat_capabilities(),
        );
        record["capabilities"]["supports"] =
            json!({"tool_calls": true, "parallel_tool_calls": false});
        let mut caps = caps_map();
        caps.insert(
            "github-copilot/no-parallel".to_string(),
            copilot_caps(64_000),
        );
        let live = json!({"object": "list", "data": [record]});

        let models = provider
            .available_models_from_live_response(&live, &caps, DEFAULT_COPILOT_API_BASE)
            .unwrap();
        let model = models
            .iter()
            .find(|model| model.id == "no-parallel")
            .unwrap();

        assert!(model.supports_tools);
        assert!(
            !model.supports_parallel_tools,
            "a live false must beat the catalog's true"
        );
    }

    #[test]
    fn github_copilot_multi_endpoint_records_resolve_to_anthropic_messages() {
        let provider = provider_with_token();
        // The real Claude records advertise both families at once.
        let live = json!({
            "object": "list",
            "data": [live_record(
                "claude-sonnet-4",
                json!([ANTHROPIC_MESSAGES_ENDPOINT, CHAT_COMPLETIONS_ENDPOINT]),
                chat_capabilities(),
            )]
        });

        let models = provider
            .available_models_from_live_response(&live, &caps_map(), DEFAULT_COPILOT_API_BASE)
            .unwrap();
        let model = models
            .iter()
            .find(|model| model.id == "claude-sonnet-4")
            .unwrap();

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
    fn github_copilot_chat_completions_endpoint_has_no_v1_prefix() {
        let provider = provider_with_token();
        let live = json!({
            "object": "list",
            "data": [live_record(
                "gpt-4.1",
                json!([CHAT_COMPLETIONS_ENDPOINT]),
                chat_capabilities(),
            )]
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
            Some("https://api.githubcopilot.com/chat/completions")
        );
    }

    #[test]
    fn github_copilot_websocket_only_responses_endpoint_is_not_used_as_an_override() {
        let provider = provider_with_token();
        let live = json!({
            "object": "list",
            "data": [live_record(
                "ws-only",
                json!([RESPONSES_WEBSOCKET_ENDPOINT]),
                chat_capabilities(),
            )]
        });

        let models = provider
            .available_models_from_live_response(&live, &HashMap::new(), DEFAULT_COPILOT_API_BASE)
            .unwrap();
        let model = &models[0];

        assert_eq!(model.id, "ws-only");
        assert_eq!(
            model.wire_format_override, None,
            "refact cannot speak the websocket Responses transport"
        );
        assert_eq!(model.endpoint_override, None);
    }

    #[test]
    fn github_copilot_websocket_endpoint_alongside_http_still_resolves_http() {
        let provider = provider_with_token();
        let live = json!({
            "object": "list",
            "data": [live_record(
                "gpt-5",
                json!([RESPONSES_ENDPOINT, RESPONSES_WEBSOCKET_ENDPOINT]),
                chat_capabilities(),
            )]
        });

        let models = provider
            .available_models_from_live_response(&live, &HashMap::new(), DEFAULT_COPILOT_API_BASE)
            .unwrap();
        let model = &models[0];

        assert_eq!(
            model.wire_format_override,
            Some(WireFormat::OpenaiResponses)
        );
        assert_eq!(
            model.endpoint_override.as_deref(),
            Some("https://api.githubcopilot.com/responses")
        );
    }

    #[test]
    fn github_copilot_absent_live_values_preserve_catalog_and_qualified_enablement() {
        let mut provider = provider_with_token();
        provider.enabled_models = vec!["github-copilot/gpt-4.1".to_string()];
        let mut caps = caps_map();
        caps.get_mut("github-copilot/gpt-4.1")
            .unwrap()
            .supports_vision = true;
        let mut record = live_record(
            "gpt-4.1",
            json!([CHAT_COMPLETIONS_ENDPOINT]),
            chat_capabilities(),
        );
        record["capabilities"]["limits"] = json!({});
        record["capabilities"]["supports"] = json!({});
        let live = json!({"object": "list", "data": [record]});

        let models = provider
            .available_models_from_live_response(&live, &caps, DEFAULT_COPILOT_API_BASE)
            .unwrap();
        let model = &models[0];

        assert!(model.enabled);
        assert!(model.supports_tools);
        assert!(model.supports_multimodality);
        assert_eq!(model.n_ctx, 128_000, "catalog n_ctx fills the live gap");
    }

    #[test]
    fn github_copilot_model_without_any_context_window_is_dropped() {
        let provider = provider_with_token();
        let mut record = live_record(
            "no-limits",
            json!([CHAT_COMPLETIONS_ENDPOINT]),
            chat_capabilities(),
        );
        record["capabilities"]["limits"] = json!({});
        let live = json!({"object": "list", "data": [record]});

        let models = provider
            .available_models_from_live_response(&live, &HashMap::new(), DEFAULT_COPILOT_API_BASE)
            .unwrap();

        assert!(
            models.is_empty(),
            "a model with no context window from either source must never be published"
        );
    }

    #[test]
    fn github_copilot_missing_data_array_is_an_error_not_an_empty_list() {
        let provider = provider_with_token();

        let result = provider.available_models_from_live_response(
            &json!({"object": "list"}),
            &HashMap::new(),
            DEFAULT_COPILOT_API_BASE,
        );

        assert!(result.is_err());
    }

    #[test]
    fn github_copilot_failure_fallback_is_custom_models_only_never_the_catalog() {
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

        // This is what every failure path (no token, bad api base, network, timeout, non-2xx,
        // bad JSON, missing data array) returns.
        let models = provider.fallback_models();
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();

        assert_eq!(ids, vec!["custom-copilot"]);
        assert!(models[0].is_custom);
        assert!(models[0].enabled);
        assert!(
            !ids.contains(&"gpt-4.1"),
            "models.dev must never contribute an id to this provider"
        );
    }

    #[test]
    fn github_copilot_catalog_only_enriches_ids_the_live_api_returned() {
        let provider = provider_with_token();
        let live = json!({
            "object": "list",
            // caps_map() also knows github_copilot/claude-sonnet-4, which must not appear.
            "data": [live_record(
                "gpt-4.1",
                json!([CHAT_COMPLETIONS_ENDPOINT]),
                chat_capabilities(),
            )]
        });

        let models = provider
            .available_models_from_live_response(&live, &caps_map(), DEFAULT_COPILOT_API_BASE)
            .unwrap();
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();

        assert_eq!(ids, vec!["gpt-4.1"]);
        assert!(
            models[0].pricing.is_some(),
            "models.dev may still enrich a live id with pricing"
        );
    }
}
