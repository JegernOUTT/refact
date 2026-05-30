use std::any::Any;
use std::collections::HashMap;
use std::sync::OnceLock;

use async_trait::async_trait;
use regex::Regex;

pub use refact_core::model_caps::ModelCapabilities;
pub use refact_core::provider_types::{
    AvailableModel, CustomModelConfig, ModelPricing, ModelPricingTier, ModelSource,
    ModelTypeDefaults, ProviderDefaults, ProviderModel, ProviderRuntime, ProviderVariant,
    derive_endpoint_from_chat_url, extra_headers_mapping_to_hash_map, is_legacy_refact_model,
    merge_custom_models, normalize_endpoint, parse_custom_models, parse_enabled_models,
    parse_extra_headers_value, resolve_env_var, set_model_enabled_impl,
};
pub use refact_core::llm_types::WireFormat;

static REGEX_CACHE: OnceLock<std::sync::RwLock<HashMap<&'static str, Regex>>> = OnceLock::new();

fn get_cached_regex(pattern: &'static str) -> Option<Regex> {
    let cache = REGEX_CACHE.get_or_init(|| std::sync::RwLock::new(HashMap::new()));

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

#[async_trait]
pub trait ProviderTrait: Send + Sync {
    fn name(&self) -> &str;

    fn display_name(&self) -> &str;

    fn base_provider_name(&self) -> &str {
        self.name()
    }

    #[allow(dead_code)]
    fn as_any(&self) -> &dyn Any;

    #[allow(dead_code)]
    fn as_any_mut(&mut self) -> &mut dyn Any;

    fn clone_box(&self) -> Box<dyn ProviderTrait>;

    fn default_wire_format(&self) -> WireFormat;

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

    fn is_hidden_from_list(&self) -> bool {
        false
    }

    fn has_credentials(&self) -> bool {
        false
    }

    fn selected_model_count(&self) -> usize {
        self.enabled_models().len()
    }

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

    fn set_model_enabled(&mut self, _model_id: &str, _enabled: bool) {}

    fn set_selected_provider(&mut self, _model_id: &str, _provider: Option<String>) {}

    fn selected_providers(&self) -> &HashMap<String, String> {
        static EMPTY: OnceLock<HashMap<String, String>> = OnceLock::new();
        EMPTY.get_or_init(HashMap::new)
    }

    fn add_custom_model(&mut self, _model_id: String, _config: CustomModelConfig) {}

    fn remove_custom_model(&mut self, _model_id: &str) -> bool {
        false
    }

    fn apply_oauth_refresh_tokens(
        &mut self,
        _access_token: &str,
        _refresh_token: &str,
        _expires_at: i64,
    ) {
    }

    fn custom_model_pricing(&self, _model_id: &str) -> Option<ModelPricing> {
        None
    }

    async fn fetch_available_models(
        &self,
        http_client: &reqwest::Client,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let _ = http_client;
        self.get_available_models_from_caps(model_caps)
    }

    async fn startup_refresh_and_sync(
        &mut self,
        _http_client: &reqwest::Client,
        _config_dir: &std::path::Path,
        _instance_id: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    fn get_available_models_from_caps(
        &self,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        available_models_from_caps_for_provider(self, model_caps)
    }

    fn get_custom_models_only(&self) -> Vec<AvailableModel> {
        let enabled_set: std::collections::HashSet<_> =
            self.enabled_models().iter().map(|s| s.as_str()).collect();

        let mut models: Vec<AvailableModel> = self
            .custom_models()
            .iter()
            .filter(|(id, _)| !is_legacy_refact_model(id))
            .map(|(id, config)| {
                let enabled = enabled_set.contains(id.as_str());
                AvailableModel::from_custom(id, config, enabled)
            })
            .collect();

        models.sort_by(|a, b| a.id.cmp(&b.id));
        models
    }
}

pub fn available_models_from_caps_for_provider<P: ProviderTrait + ?Sized>(
    provider: &P,
    model_caps: &HashMap<String, ModelCapabilities>,
) -> Vec<AvailableModel> {
    let enabled_set: std::collections::HashSet<_> = provider
        .enabled_models()
        .iter()
        .map(|s| s.as_str())
        .collect();
    let custom_models = provider.custom_models();

    let mut models_map: HashMap<String, AvailableModel> = HashMap::new();

    let regex_opt: Option<Regex> = provider.model_filter_regex().and_then(get_cached_regex);
    let provider_aliases = model_caps_provider_aliases(provider.base_provider_name());
    let has_provider_qualified_caps = model_caps
        .keys()
        .any(|key| model_caps_key_has_provider_alias(key, &provider_aliases));

    for (name, caps) in model_caps {
        let Some(model_id) =
            model_caps_provider_model_id(&provider_aliases, has_provider_qualified_caps, name)
        else {
            continue;
        };
        if is_legacy_refact_model(model_id) {
            continue;
        }
        let matches = match &regex_opt {
            Some(regex) => regex.is_match(model_id),
            None => true,
        };
        if matches {
            let disabled = provider
                .disabled_models()
                .iter()
                .any(|disabled| disabled == name || disabled.as_str() == model_id);
            let enabled = if disabled {
                false
            } else {
                enabled_set.contains(name.as_str()) || enabled_set.contains(model_id)
            };
            let pricing = provider
                .custom_model_pricing(model_id)
                .or_else(|| provider.custom_model_pricing(name));
            models_map.insert(
                model_id.to_string(),
                AvailableModel::from_caps(model_id, caps, enabled, pricing),
            );
        }
    }

    let mut models: Vec<AvailableModel> = models_map.into_values().collect();
    merge_custom_models(&mut models, custom_models, &enabled_set);
    models.sort_by(|a, b| a.id.cmp(&b.id));
    models
}

fn model_caps_provider_model_id<'a>(
    provider_aliases: &[String],
    has_provider_qualified_caps: bool,
    capability_key: &'a str,
) -> Option<&'a str> {
    if !capability_key.contains('/') {
        return (!has_provider_qualified_caps).then_some(capability_key);
    }

    for provider_alias in provider_aliases {
        let prefix = format!("{provider_alias}/");
        if let Some(model_id) = capability_key.strip_prefix(&prefix) {
            return Some(model_id);
        }
    }

    None
}

fn model_caps_key_has_provider_alias(key: &str, provider_aliases: &[String]) -> bool {
    provider_aliases
        .iter()
        .any(|provider_alias| key.starts_with(&format!("{provider_alias}/")))
}

fn model_caps_provider_aliases(provider_name: &str) -> Vec<String> {
    let mut aliases = vec![provider_name.to_string(), provider_name.replace('_', "-")];
    for suffix in ["_responses", "-responses"] {
        if let Some(stripped) = provider_name.strip_suffix(suffix) {
            aliases.push(stripped.to_string());
            aliases.push(stripped.replace('_', "-"));
        }
    }
    aliases.sort();
    aliases.dedup();
    aliases
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn models_dev_available_model_omits_empty_override_fields_when_serialized() {
        let model = AvailableModel::from_caps(
            "test-model",
            &ModelCapabilities {
                n_ctx: 4096,
                supports_tools: true,
                ..Default::default()
            },
            true,
            None,
        );

        let value = serde_json::to_value(model).unwrap();

        assert!(value.get("wire_format_override").is_none());
        assert!(value.get("endpoint_override").is_none());
    }

    #[test]
    fn models_dev_available_model_deserializes_without_override_fields() {
        let model: AvailableModel = serde_json::from_value(json!({
            "id": "old-model",
            "display_name": null,
            "n_ctx": 8192,
            "supports_tools": true,
            "supports_multimodality": false,
            "tokenizer": null,
            "enabled": true,
            "is_custom": false
        }))
        .unwrap();

        assert_eq!(model.id, "old-model");
        assert!(model.wire_format_override.is_none());
        assert!(model.endpoint_override.is_none());
    }
}
