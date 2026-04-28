use std::any::Any;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::caps::model_caps::ModelCapabilities;
use crate::llm::adapter::WireFormat;

static REGEX_CACHE: OnceLock<RwLock<HashMap<&'static str, Regex>>> = OnceLock::new();

fn get_cached_regex(pattern: &'static str) -> Option<Regex> {
    let cache = REGEX_CACHE.get_or_init(|| RwLock::new(HashMap::new()));

    if let Ok(guard) = cache.read() {
        if let Some(regex) = guard.get(pattern) {
            return Some(regex.clone());
        }
    }

    match Regex::new(pattern) {
        Ok(regex) => {
            if let Ok(mut guard) = cache.write() {
                guard.insert(pattern, regex.clone());
            }
            Some(regex)
        }
        Err(e) => {
            tracing::warn!("Failed to compile regex '{}': {}", pattern, e);
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    ModelCaps,
    Api,
    Local,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelPricing {
    pub prompt: f64,
    pub generated: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation: Option<f64>,
}

impl ModelPricing {
    pub fn is_valid(&self) -> bool {
        let valid_price = |p: f64| p.is_finite() && p >= 0.0;
        let valid_opt = |p: Option<f64>| p.map_or(true, valid_price);
        valid_price(self.prompt)
            && valid_price(self.generated)
            && valid_opt(self.cache_read)
            && valid_opt(self.cache_creation)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CustomModelConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n_ctx: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_tools: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_parallel_tools: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_strict_tools: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_multimodality: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort_options: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_thinking_budget: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_adaptive_thinking_budget: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokenizer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderVariant {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_last_30m: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub throughput_last_30m: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_last_30m: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported_parameters: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailableModel {
    pub id: String,
    pub display_name: Option<String>,
    pub n_ctx: usize,
    pub supports_tools: bool,
    #[serde(default)]
    pub supports_parallel_tools: bool,
    #[serde(default)]
    pub supports_strict_tools: bool,
    pub supports_multimodality: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort_options: Option<Vec<String>>,
    #[serde(default)]
    pub supports_thinking_budget: bool,
    #[serde(default)]
    pub supports_adaptive_thinking_budget: bool,
    pub tokenizer: Option<String>,
    pub enabled: bool,
    pub is_custom: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_providers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_variants: Vec<ProviderVariant>,
    /// Optional base/root model identifier (e.g. "Qwen/Qwen3.6-27B-FP8" from vLLM).
    /// Used as a fallback for model capabilities registry resolution when the
    /// provider-reported id is a custom alias.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_model: Option<String>,
}

impl AvailableModel {
    pub fn from_caps(
        id: &str,
        caps: &ModelCapabilities,
        enabled: bool,
        pricing: Option<ModelPricing>,
    ) -> Self {
        Self {
            id: id.to_string(),
            display_name: None,
            n_ctx: caps.n_ctx,
            supports_tools: caps.supports_tools,
            supports_parallel_tools: caps.supports_parallel_tools,
            supports_strict_tools: caps.supports_strict_tools,
            supports_multimodality: caps.supports_vision,
            reasoning_effort_options: caps.reasoning_effort_options.clone(),
            supports_thinking_budget: caps.supports_thinking_budget,
            supports_adaptive_thinking_budget: caps.supports_adaptive_thinking_budget,
            tokenizer: if caps.tokenizer.is_empty() {
                None
            } else {
                Some(caps.tokenizer.clone())
            },
            enabled,
            is_custom: false,
            pricing,
            available_providers: Vec::new(),
            selected_provider: None,
            max_output_tokens: None,
            provider_variants: Vec::new(),
            base_model: None,
        }
    }

    pub fn from_custom(id: &str, config: &CustomModelConfig, enabled: bool) -> Self {
        Self {
            id: id.to_string(),
            display_name: None,
            n_ctx: config.n_ctx.unwrap_or(4096),
            supports_tools: config.supports_tools.unwrap_or(false),
            supports_parallel_tools: config.supports_parallel_tools.unwrap_or(false),
            supports_strict_tools: config.supports_strict_tools.unwrap_or(false),
            supports_multimodality: config.supports_multimodality.unwrap_or(false),
            reasoning_effort_options: config.reasoning_effort_options.clone(),
            supports_thinking_budget: config.supports_thinking_budget.unwrap_or(false),
            supports_adaptive_thinking_budget: config
                .supports_adaptive_thinking_budget
                .unwrap_or(false),
            tokenizer: config.tokenizer.clone(),
            enabled,
            is_custom: true,
            pricing: config.pricing.clone(),
            available_providers: Vec::new(),
            selected_provider: None,
            max_output_tokens: config.max_output_tokens,
            provider_variants: Vec::new(),
            base_model: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModel {
    pub id: String,
    pub base_name: String,
    pub enabled: bool,
    pub n_ctx: usize,
    pub supports_tools: bool,
    pub supports_multimodality: bool,
    pub supports_reasoning: Option<String>,
    pub supports_agent: bool,
    pub wire_format_override: Option<WireFormat>,
    pub endpoint_override: Option<String>,
    pub user_configured: bool,
    pub removable: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRuntime {
    pub name: String,
    pub display_name: String,
    pub enabled: bool,
    pub readonly: bool,
    pub wire_format: WireFormat,
    pub chat_endpoint: String,
    pub completion_endpoint: String,
    pub embedding_endpoint: String,
    #[serde(skip_serializing)]
    pub api_key: String,
    /// OAuth/Bearer token for providers that use Authorization: Bearer auth
    /// (e.g., Claude Code CLI OAuth tokens). When set, adapters should use
    /// `Authorization: Bearer <token>` instead of provider-specific key headers.
    #[serde(skip_serializing)]
    #[serde(default)]
    pub auth_token: String,
    #[serde(skip_serializing)]
    pub tokenizer_api_key: String,
    /// Extra headers for HTTP requests. Currently populated by CustomProvider
    /// but not yet connected to the LLM request flow (which uses the caps system).
    /// Kept for future integration when provider system replaces caps for requests.
    #[serde(skip_serializing)]
    #[allow(dead_code)]
    pub extra_headers: HashMap<String, String>,
    pub support_metadata: bool,
    /// Whether this provider supports Anthropic-style prompt cache_control headers.
    /// Set to false for providers like vLLM that reject unknown fields.
    #[serde(default = "default_true")]
    pub supports_cache_control: bool,
    pub chat_models: Vec<ProviderModel>,
    pub completion_models: Vec<ProviderModel>,
    pub embedding_model: Option<ProviderModel>,
}

impl ProviderRuntime {
    pub fn redacted(&self) -> Self {
        Self {
            api_key: if self.api_key.is_empty() {
                String::new()
            } else {
                "***".to_string()
            },
            auth_token: if self.auth_token.is_empty() {
                String::new()
            } else {
                "***".to_string()
            },
            tokenizer_api_key: if self.tokenizer_api_key.is_empty() {
                String::new()
            } else {
                "***".to_string()
            },
            extra_headers: HashMap::new(),
            ..self.clone()
        }
    }
}

#[async_trait]
pub trait ProviderTrait: Send + Sync {
    fn name(&self) -> &'static str;

    fn display_name(&self) -> &'static str;

    /// Downcast to concrete type. Used for provider-specific operations
    /// that aren't part of the trait interface (e.g., accessing provider-specific fields).
    #[allow(dead_code)]
    fn as_any(&self) -> &dyn Any;

    /// Mutable downcast to concrete type. Used for provider-specific mutations.
    #[allow(dead_code)]
    fn as_any_mut(&mut self) -> &mut dyn Any;

    fn clone_box(&self) -> Box<dyn ProviderTrait>;

    fn default_wire_format(&self) -> WireFormat;

    /// Returns all wire formats this provider can use. Used for UI wire format selection
    /// and request routing. Default returns only `default_wire_format()`.
    #[allow(dead_code)]
    fn supported_wire_formats(&self) -> Vec<WireFormat> {
        vec![self.default_wire_format()]
    }

    fn model_filter_regex(&self) -> Option<&'static str>;

    fn provider_schema(&self) -> &'static str;

    fn provider_settings_apply(&mut self, yaml: serde_yaml::Value) -> Result<(), String>;

    fn provider_settings_as_json(&self) -> serde_json::Value;

    fn build_runtime(&self) -> Result<ProviderRuntime, String>;

    fn is_readonly(&self) -> bool {
        false
    }

    /// Whether this provider should be hidden from the providers list UI.
    /// Used for response-API variants that are merged into their parent provider.
    fn is_hidden_from_list(&self) -> bool {
        false
    }

    /// Whether this provider has valid credentials configured.
    /// Used to derive provider status without the manual `enabled` toggle.
    fn has_credentials(&self) -> bool {
        false
    }

    /// Number of models the user has selected/enabled for this provider.
    /// For allowlist providers: enabled_models().len()
    /// For denylist providers: override to compute actual selected count.
    fn selected_model_count(&self) -> usize {
        self.enabled_models().len()
    }

    // Model discovery methods
    fn model_source(&self) -> ModelSource {
        ModelSource::ModelCaps
    }

    fn enabled_models(&self) -> &[String] {
        &[]
    }

    fn disabled_models(&self) -> &[String] {
        &[]
    }

    fn custom_models(&self) -> &HashMap<String, CustomModelConfig> {
        static EMPTY: OnceLock<HashMap<String, CustomModelConfig>> = OnceLock::new();
        EMPTY.get_or_init(HashMap::new)
    }

    fn set_model_enabled(&mut self, _model_id: &str, _enabled: bool) {
        // Default: no-op, providers override this
    }

    fn set_selected_provider(&mut self, _model_id: &str, _provider: Option<String>) {
        // Default: no-op, providers override this
    }

    fn selected_providers(&self) -> &HashMap<String, String> {
        static EMPTY: OnceLock<HashMap<String, String>> = OnceLock::new();
        EMPTY.get_or_init(HashMap::new)
    }

    fn add_custom_model(&mut self, _model_id: String, _config: CustomModelConfig) {
        // Default: no-op, providers override this
    }

    fn remove_custom_model(&mut self, _model_id: &str) -> bool {
        false
    }

    fn model_pricing(&self, _model_id: &str) -> Option<ModelPricing> {
        None
    }

    /// Discover and return available models for this provider.
    /// Providers that need network access (API fetching) override this async method.
    /// Default implementation matches against model_caps using the provider's filter regex
    /// and enabled/disabled model lists.
    async fn fetch_available_models(
        &self,
        http_client: &reqwest::Client,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let _ = http_client; // unused in default impl
        self.get_available_models_from_caps(model_caps)
    }

    /// Optional startup hook for providers that need to refresh dynamic state
    /// (for example, model catalogs) and persist provider-local config.
    async fn startup_refresh_and_sync(
        &mut self,
        _http_client: &reqwest::Client,
        _config_dir: &std::path::Path,
    ) -> Result<(), String> {
        Ok(())
    }

    fn get_available_models_from_caps(
        &self,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let enabled_set: std::collections::HashSet<_> =
            self.enabled_models().iter().map(|s| s.as_str()).collect();
        let custom_models = self.custom_models();

        let mut models_map: HashMap<String, AvailableModel> = HashMap::new();

        let regex_opt: Option<Regex> = self.model_filter_regex().and_then(get_cached_regex);

        for (name, caps) in model_caps {
            let matches = match &regex_opt {
                Some(regex) => regex.is_match(name),
                None => true,
            };
            if matches {
                let disabled = self.disabled_models().contains(&name.to_string());
                let enabled = if disabled {
                    false
                } else {
                    enabled_set.contains(name.as_str())
                };
                let pricing = self.model_pricing(name);
                models_map.insert(
                    name.clone(),
                    AvailableModel::from_caps(name, caps, enabled, pricing),
                );
            }
        }

        let mut models: Vec<AvailableModel> = models_map.into_values().collect();
        merge_custom_models(&mut models, custom_models, &enabled_set);
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models
    }

    fn get_custom_models_only(&self) -> Vec<AvailableModel> {
        let enabled_set: std::collections::HashSet<_> =
            self.enabled_models().iter().map(|s| s.as_str()).collect();

        let mut models: Vec<AvailableModel> = self
            .custom_models()
            .iter()
            .map(|(id, config)| {
                let enabled = enabled_set.contains(id.as_str());
                AvailableModel::from_custom(id, config, enabled)
            })
            .collect();

        models.sort_by(|a, b| a.id.cmp(&b.id));
        models
    }
}
// ============================================================================
// Helper functions for reducing boilerplate in provider implementations
// ============================================================================

pub fn merge_custom_models(
    models: &mut Vec<AvailableModel>,
    custom_models: &HashMap<String, CustomModelConfig>,
    enabled_set: &std::collections::HashSet<&str>,
) {
    for (id, config) in custom_models {
        let enabled = enabled_set.contains(id.as_str());
        if let Some(existing) = models.iter_mut().find(|m| m.id == *id) {
            let has_capability_overrides = config.n_ctx.is_some()
                || config.supports_tools.is_some()
                || config.supports_parallel_tools.is_some()
                || config.supports_strict_tools.is_some()
                || config.supports_multimodality.is_some()
                || config.reasoning_effort_options.is_some()
                || config.supports_thinking_budget.is_some()
                || config.supports_adaptive_thinking_budget.is_some()
                || config.tokenizer.is_some()
                || config.max_output_tokens.is_some();
            if let Some(n_ctx) = config.n_ctx {
                existing.n_ctx = n_ctx;
            }
            if let Some(v) = config.supports_tools {
                existing.supports_tools = v;
            }
            if let Some(v) = config.supports_parallel_tools {
                existing.supports_parallel_tools = v;
            }
            if let Some(v) = config.supports_strict_tools {
                existing.supports_strict_tools = v;
            }
            if let Some(v) = config.supports_multimodality {
                existing.supports_multimodality = v;
            }
            if config.reasoning_effort_options.is_some() {
                existing.reasoning_effort_options = config.reasoning_effort_options.clone();
            }
            if let Some(v) = config.supports_thinking_budget {
                existing.supports_thinking_budget = v;
            }
            if let Some(v) = config.supports_adaptive_thinking_budget {
                existing.supports_adaptive_thinking_budget = v;
            }
            if config.tokenizer.is_some() {
                existing.tokenizer = config.tokenizer.clone();
            }
            if config.pricing.is_some() {
                existing.pricing = config.pricing.clone();
            }
            if config.max_output_tokens.is_some() {
                existing.max_output_tokens = config.max_output_tokens;
            }
            if has_capability_overrides {
                existing.is_custom = true;
            }
        } else {
            models.push(AvailableModel::from_custom(id, config, enabled));
        }
    }
}

pub fn normalize_endpoint(endpoint: &str) -> String {
    let s = endpoint.trim().trim_end_matches('/');
    let s = s.strip_suffix("/v1").unwrap_or(s);
    s.to_string()
}

pub fn derive_endpoint_from_chat_url(chat_endpoint: &str) -> Option<String> {
    let s = chat_endpoint.trim().trim_end_matches('/');
    for suffix in &[
        "/v1/chat/completions",
        "/chat/completions",
        "/v1/completions",
        "/completions",
    ] {
        if let Some(base) = s.strip_suffix(suffix) {
            if !base.is_empty() {
                return Some(base.to_string());
            }
        }
    }
    None
}

/// Parse enabled_models from YAML, replacing the existing list
pub fn parse_enabled_models(yaml: &serde_yaml::Value, enabled_models: &mut Vec<String>) {
    if let Some(models) = yaml.get("enabled_models").and_then(|v| v.as_sequence()) {
        enabled_models.clear();
        enabled_models.extend(models.iter().filter_map(|v| v.as_str().map(String::from)));
    }
}

/// Parse custom_models from YAML, replacing the existing map
pub fn parse_custom_models(
    yaml: &serde_yaml::Value,
    custom_models: &mut HashMap<String, CustomModelConfig>,
) {
    if let Some(custom) = yaml.get("custom_models").and_then(|v| v.as_mapping()) {
        custom_models.clear();
        for (key, value) in custom {
            if let Some(model_id) = key.as_str() {
                if let Ok(config) = serde_yaml::from_value(value.clone()) {
                    custom_models.insert(model_id.to_string(), config);
                }
            }
        }
    }
}

/// Standard implementation for set_model_enabled (allowlist - adds when enabled)
pub fn set_model_enabled_impl(enabled_models: &mut Vec<String>, model_id: &str, enabled: bool) {
    if enabled {
        if !enabled_models.iter().any(|m| m == model_id) {
            enabled_models.push(model_id.to_string());
        }
    } else {
        enabled_models.retain(|m| m != model_id);
    }
}

/// Standard implementation for set_model_enabled with denylist semantics
/// (adds to disabled_models when disabled, removes when enabled)
pub fn set_model_disabled_impl(disabled_models: &mut Vec<String>, model_id: &str, enabled: bool) {
    if enabled {
        disabled_models.retain(|m| m != model_id);
    } else {
        if !disabled_models.iter().any(|m| m == model_id) {
            disabled_models.push(model_id.to_string());
        }
    }
}

/// Parse disabled_models from YAML, replacing the existing list
pub fn parse_disabled_models(yaml: &serde_yaml::Value, disabled_models: &mut Vec<String>) {
    if let Some(models) = yaml.get("disabled_models").and_then(|v| v.as_sequence()) {
        disabled_models.clear();
        disabled_models.extend(models.iter().filter_map(|v| v.as_str().map(String::from)));
    }
}
