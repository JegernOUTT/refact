use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::llm_types::WireFormat;
use crate::model_caps::ModelCapabilities;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    ModelCaps,
    Api,
    Local,
    Manual,
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

fn default_true_runtime() -> bool {
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
    #[serde(skip_serializing)]
    #[serde(default)]
    pub auth_token: String,
    #[serde(skip_serializing)]
    pub tokenizer_api_key: String,
    #[serde(skip_serializing)]
    pub extra_headers: HashMap<String, String>,
    #[serde(default = "default_true_runtime")]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelTypeDefaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_new_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boost_reasoning: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_budget: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderDefaults {
    #[serde(default)]
    pub chat: ModelTypeDefaults,
    #[serde(default)]
    pub chat_model_2: ModelTypeDefaults,
    #[serde(default)]
    pub task_planner_agent_model: ModelTypeDefaults,
    #[serde(default)]
    pub chat_light: ModelTypeDefaults,
    #[serde(default)]
    pub chat_thinking: ModelTypeDefaults,
    #[serde(default)]
    pub chat_buddy: ModelTypeDefaults,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
}

impl ProviderDefaults {
    pub fn clear_legacy_refact_models(&mut self) -> bool {
        let mut changed = false;
        changed |= clear_legacy_refact_model_field(&mut self.chat.model);
        changed |= clear_legacy_refact_model_field(&mut self.chat_model_2.model);
        changed |= clear_legacy_refact_model_field(&mut self.task_planner_agent_model.model);
        changed |= clear_legacy_refact_model_field(&mut self.chat_light.model);
        changed |= clear_legacy_refact_model_field(&mut self.chat_thinking.model);
        changed |= clear_legacy_refact_model_field(&mut self.chat_buddy.model);
        changed |= clear_legacy_refact_model_field(&mut self.completion_model);
        changed |= clear_legacy_refact_model_field(&mut self.embedding_model);
        changed
    }

    pub fn defaults_for_model(
        &self,
        model_id: &str,
        chat_default_model: &str,
        chat_model_2: &str,
        chat_light_model: &str,
        chat_thinking_model: &str,
        chat_buddy_model: &str,
    ) -> &ModelTypeDefaults {
        if !chat_default_model.is_empty() && model_id == chat_default_model {
            &self.chat
        } else if !chat_model_2.is_empty() && model_id == chat_model_2 {
            &self.chat_model_2
        } else if !chat_thinking_model.is_empty() && model_id == chat_thinking_model {
            &self.chat_thinking
        } else if !chat_buddy_model.is_empty() && model_id == chat_buddy_model {
            &self.chat_buddy
        } else if !chat_light_model.is_empty() && model_id == chat_light_model {
            &self.chat_light
        } else {
            &self.chat
        }
    }

    pub fn defaults_for_task_planner_agent_model(
        &self,
        model_id: &str,
        task_planner_agent_model: &str,
        chat_default_model: &str,
        chat_model_2: &str,
        chat_light_model: &str,
        chat_thinking_model: &str,
        chat_buddy_model: &str,
    ) -> &ModelTypeDefaults {
        if !task_planner_agent_model.is_empty() && model_id == task_planner_agent_model {
            &self.task_planner_agent_model
        } else {
            self.defaults_for_model(
                model_id,
                chat_default_model,
                chat_model_2,
                chat_light_model,
                chat_thinking_model,
                chat_buddy_model,
            )
        }
    }

    pub async fn load(config_dir: &Path) -> Result<Self, String> {
        let defaults_path = config_dir.join("providers.d").join("defaults.yaml");
        match tokio::fs::read_to_string(&defaults_path).await {
            Ok(content) => {
                let mut defaults: Self = serde_yaml::from_str(&content)
                    .map_err(|e| format!("Failed to parse defaults.yaml: {}", e))?;
                if defaults.clear_legacy_refact_models() {
                    tracing::warn!(
                        "Legacy Cloud model defaults in providers.d/defaults.yaml were reset to none"
                    );
                    if let Err(e) = defaults.save(config_dir).await {
                        tracing::warn!(
                            "Failed to persist migrated providers.d/defaults.yaml: {}",
                            e
                        );
                    }
                }
                Ok(defaults)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("Failed to read defaults.yaml: {}", e)),
        }
    }

    pub async fn save(&self, config_dir: &Path) -> Result<(), String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let providers_dir = config_dir.join("providers.d");
        tokio::fs::create_dir_all(&providers_dir)
            .await
            .map_err(|e| format!("Failed to create providers.d directory: {}", e))?;

        let defaults_path = providers_dir.join("defaults.yaml");
        let mut normalized = self.clone();
        normalized.clear_legacy_refact_models();
        let content = serde_yaml::to_string(&normalized)
            .map_err(|e| format!("Failed to serialize defaults: {}", e))?;

        let temp_path = providers_dir.join(format!(
            "defaults.yaml.tmp.{}.{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));

        tokio::fs::write(&temp_path, &content)
            .await
            .map_err(|e| format!("Failed to write temp file: {}", e))?;

        tokio::fs::rename(&temp_path, &defaults_path)
            .await
            .map_err(|e| {
                let _ = std::fs::remove_file(&temp_path);
                format!("Failed to rename temp file to defaults.yaml: {}", e)
            })
    }
}

pub fn is_legacy_refact_model(model: &str) -> bool {
    let model = model.trim();
    model == "refact" || model.starts_with("refact/") || model.contains("/refact/")
}

fn clear_legacy_refact_model_field(model: &mut Option<String>) -> bool {
    let Some(value) = model.as_mut() else {
        return false;
    };

    let trimmed = value.trim();
    if is_legacy_refact_model(trimmed) {
        *model = Some(String::new());
        return true;
    }

    if trimmed != value.as_str() {
        *value = trimmed.to_string();
        return true;
    }

    false
}

pub fn resolve_env_var(value: &str, fallback: &str, context: &str) -> String {
    if value.is_empty() {
        return fallback.to_string();
    }
    if value.starts_with('$') {
        match std::env::var(&value[1..]) {
            Ok(env_val) => env_val,
            Err(e) => {
                tracing::error!("Failed to read env var {} for {}: {}", value, context, e);
                fallback.to_string()
            }
        }
    } else {
        value.to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelPricingTier {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation: Option<f64>,
}

impl ModelPricingTier {
    pub fn is_valid(&self) -> bool {
        let valid_price = |p: f64| p.is_finite() && p >= 0.0;
        self.prompt.map_or(true, valid_price)
            && self.generated.map_or(true, valid_price)
            && self.cache_read.map_or(true, valid_price)
            && self.cache_creation.map_or(true, valid_price)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelPricing {
    pub prompt: f64,
    pub generated: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_over_200k: Option<ModelPricingTier>,
}

impl ModelPricing {
    pub fn is_valid(&self) -> bool {
        let valid_price = |p: f64| p.is_finite() && p >= 0.0;
        let valid_opt = |p: Option<f64>| p.map_or(true, valid_price);
        valid_price(self.prompt)
            && valid_price(self.generated)
            && valid_opt(self.cache_read)
            && valid_opt(self.cache_creation)
            && self
                .context_over_200k
                .as_ref()
                .map_or(true, ModelPricingTier::is_valid)
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
    pub supports_cache_control: Option<bool>,
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

fn default_true() -> bool {
    true
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
    #[serde(default = "default_true")]
    pub supports_cache_control: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire_format_override: Option<WireFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_override: Option<String>,
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
            supports_multimodality: caps.supports_vision
                || caps.supports_video
                || caps.supports_audio
                || caps.supports_pdf,
            reasoning_effort_options: caps.reasoning_effort_options.clone(),
            supports_thinking_budget: caps.supports_thinking_budget,
            supports_adaptive_thinking_budget: caps.supports_adaptive_thinking_budget,
            supports_cache_control: caps.supports_cache_control,
            tokenizer: if caps.tokenizer.is_empty() {
                None
            } else {
                Some(caps.tokenizer.clone())
            },
            enabled,
            is_custom: false,
            pricing: pricing.or_else(|| caps.pricing.clone()),
            available_providers: Vec::new(),
            selected_provider: None,
            max_output_tokens: (caps.max_output_tokens > 0).then_some(caps.max_output_tokens),
            provider_variants: Vec::new(),
            wire_format_override: None,
            endpoint_override: None,
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
            supports_cache_control: config.supports_cache_control.unwrap_or(true),
            tokenizer: config.tokenizer.clone(),
            enabled,
            is_custom: true,
            pricing: config.pricing.clone(),
            available_providers: Vec::new(),
            selected_provider: None,
            max_output_tokens: config.max_output_tokens,
            provider_variants: Vec::new(),
            wire_format_override: None,
            endpoint_override: None,
            base_model: None,
        }
    }
}

pub fn parse_extra_headers_value(value: &serde_yaml::Value) -> Result<serde_yaml::Mapping, String> {
    match value {
        serde_yaml::Value::Mapping(map) => Ok(map.clone()),
        serde_yaml::Value::Null => Ok(serde_yaml::Mapping::new()),
        serde_yaml::Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Ok(serde_yaml::Mapping::new());
            }
            let parsed: serde_yaml::Value = serde_yaml::from_str(trimmed)
                .map_err(|e| format!("extra_headers must be a YAML/JSON object: {e}"))?;
            match parsed {
                serde_yaml::Value::Mapping(map) => Ok(map),
                serde_yaml::Value::Null => Ok(serde_yaml::Mapping::new()),
                _ => Err("extra_headers must be a YAML/JSON object".to_string()),
            }
        }
        _ => Err("extra_headers must be a YAML/JSON object".to_string()),
    }
}

pub fn extra_headers_mapping_to_hash_map(
    existing: Option<&HashMap<String, String>>,
    incoming: &serde_yaml::Mapping,
) -> HashMap<String, String> {
    let mut next_headers = HashMap::new();
    for (key, value) in incoming {
        let Some(key) = key.as_str() else {
            continue;
        };
        let Some(value) = value.as_str() else {
            continue;
        };
        if value == "***" {
            if let Some(existing_value) = existing.and_then(|headers| headers.get(key)) {
                next_headers.insert(key.to_string(), existing_value.clone());
            }
        } else {
            next_headers.insert(key.to_string(), value.to_string());
        }
    }
    next_headers
}

pub fn merge_custom_models(
    models: &mut Vec<AvailableModel>,
    custom_models: &HashMap<String, CustomModelConfig>,
    enabled_set: &std::collections::HashSet<&str>,
) {
    for (id, config) in custom_models {
        if is_legacy_refact_model(id) {
            continue;
        }
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
                || config.supports_cache_control.is_some()
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
            if let Some(v) = config.supports_cache_control {
                existing.supports_cache_control = v;
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

pub fn parse_enabled_models(yaml: &serde_yaml::Value, enabled_models: &mut Vec<String>) {
    if let Some(models) = yaml.get("enabled_models").and_then(|v| v.as_sequence()) {
        enabled_models.clear();
        enabled_models.extend(
            models
                .iter()
                .filter_map(|v| v.as_str())
                .filter(|model_id| !is_legacy_refact_model(model_id))
                .map(String::from),
        );
    }
}

pub fn parse_custom_models(
    yaml: &serde_yaml::Value,
    custom_models: &mut HashMap<String, CustomModelConfig>,
) {
    if let Some(custom) = yaml.get("custom_models").and_then(|v| v.as_mapping()) {
        custom_models.clear();
        for (key, value) in custom {
            if let Some(model_id) = key.as_str() {
                if is_legacy_refact_model(model_id) {
                    continue;
                }
                if let Ok(config) = serde_yaml::from_value(value.clone()) {
                    custom_models.insert(model_id.to_string(), config);
                }
            }
        }
    }
}

pub fn set_model_enabled_impl(enabled_models: &mut Vec<String>, model_id: &str, enabled: bool) {
    if is_legacy_refact_model(model_id) {
        enabled_models.retain(|m| !is_legacy_refact_model(m));
        return;
    }

    if enabled {
        if !enabled_models.iter().any(|m| m == model_id) {
            enabled_models.push(model_id.to_string());
        }
    } else {
        enabled_models.retain(|m| m != model_id);
    }
}

#[cfg(test)]
mod tests {
    use super::{ModelTypeDefaults, ProviderDefaults};

    #[test]
    fn clear_legacy_refact_models_resets_only_refact_models_to_none() {
        let mut defaults = ProviderDefaults {
            chat: ModelTypeDefaults {
                model: Some("openai/gpt-5".to_string()),
                ..Default::default()
            },
            chat_model_2: ModelTypeDefaults {
                model: Some("refact/secondary".to_string()),
                ..Default::default()
            },
            task_planner_agent_model: ModelTypeDefaults {
                model: Some("  refact/planner-agent  ".to_string()),
                ..Default::default()
            },
            chat_light: ModelTypeDefaults {
                model: Some("refact/grok-4-fast-non-reasoning".to_string()),
                ..Default::default()
            },
            chat_thinking: ModelTypeDefaults {
                model: Some("  refact/o4-mini-deep-research  ".to_string()),
                ..Default::default()
            },
            completion_model: Some("refact/qwen2.5-coder".to_string()),
            ..Default::default()
        };

        assert!(defaults.clear_legacy_refact_models());

        assert_eq!(defaults.chat.model.as_deref(), Some("openai/gpt-5"));
        assert_eq!(defaults.chat_model_2.model.as_deref(), Some(""));
        assert_eq!(defaults.task_planner_agent_model.model.as_deref(), Some(""));
        assert_eq!(defaults.chat_light.model.as_deref(), Some(""));
        assert_eq!(defaults.chat_thinking.model.as_deref(), Some(""));
        assert_eq!(defaults.completion_model.as_deref(), Some(""));
    }

    #[test]
    fn defaults_for_model_prefers_chat_when_default_slots_share_model() {
        let defaults = ProviderDefaults {
            chat: ModelTypeDefaults {
                temperature: Some(0.1),
                ..Default::default()
            },
            chat_model_2: ModelTypeDefaults {
                temperature: Some(0.2),
                ..Default::default()
            },
            task_planner_agent_model: ModelTypeDefaults {
                temperature: Some(0.3),
                ..Default::default()
            },
            ..Default::default()
        };

        let selected =
            defaults.defaults_for_model("shared/model", "shared/model", "shared/model", "", "", "");

        assert_eq!(selected.temperature, Some(0.1));
    }

    #[test]
    fn defaults_for_task_planner_agent_model_prefers_task_slot_when_model_is_shared() {
        let defaults = ProviderDefaults {
            chat: ModelTypeDefaults {
                temperature: Some(0.1),
                ..Default::default()
            },
            chat_model_2: ModelTypeDefaults {
                temperature: Some(0.2),
                ..Default::default()
            },
            task_planner_agent_model: ModelTypeDefaults {
                temperature: Some(0.3),
                ..Default::default()
            },
            ..Default::default()
        };

        let selected = defaults.defaults_for_task_planner_agent_model(
            "shared/model",
            "shared/model",
            "shared/model",
            "shared/model",
            "",
            "",
            "",
        );

        assert_eq!(selected.temperature, Some(0.3));
    }
}
