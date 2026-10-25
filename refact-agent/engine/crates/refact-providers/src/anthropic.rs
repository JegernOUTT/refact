use std::any::Any;
use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use refact_core::model_caps::ModelCapabilities;
use refact_core::llm_types::WireFormat;
use crate::claude_code::ClaudeCodeProvider;
use refact_core::claude_backend::ClaudeBackend;
use crate::traits::{
    AvailableModel, CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
    available_models_from_caps_for_provider, merge_custom_models, set_model_enabled_impl,
};

const SUPPORTS_CACHE_CONTROL: bool = true;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct AnthropicProvider {
    pub api_key: String,
    pub enabled: bool,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
}

impl std::fmt::Debug for AnthropicProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("enabled", &self.enabled)
            .finish_non_exhaustive()
    }
}

impl AnthropicProvider {
    fn shared(&self) -> ClaudeCodeProvider {
        ClaudeCodeProvider {
            enabled: self.enabled,
            enabled_models: self.enabled_models.clone(),
            custom_models: self.custom_models.clone(),
            transport: refact_core::claude_backend::ClaudeTransportConfig {
                backend: ClaudeBackend::Anthropic,
                api_key: Some(self.api_key.clone()),
                base_url: Some("https://api.anthropic.com".into()),
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

#[async_trait]
impl ProviderTrait for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn display_name(&self) -> &str {
        "Anthropic"
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
        WireFormat::AnthropicMessages
    }

    fn model_filter_regex(&self) -> Option<&'static str> {
        Some(r"^claude-")
    }

    fn provider_schema(&self) -> &'static str {
        r#"
fields:
  api_key:
    f_type: string_long
    f_label: "API Key"
    f_secret: true
    f_desc: "Anthropic API key from console.anthropic.com. Accepts $ENV; empty uses ANTHROPIC_API_KEY."
    f_placeholder: "sk-ant-..."
description: |
  Direct access to Anthropic Claude models. Existing Anthropic model and privacy identities are preserved.
available:
  on_your_laptop_possible: true
  when_isolated_possible: true
"#
    }

    fn provider_settings_apply(&mut self, yaml: serde_yaml::Value) -> Result<(), String> {
        if let Some(backend) = yaml.get("backend") {
            if backend.as_str() != Some("anthropic") {
                return Err("Legacy Anthropic instances use the anthropic backend; create a Claude Code instance for other backends".into());
            }
        }
        let mut shared = self.shared();
        shared.provider_settings_apply(yaml)?;
        self.api_key = shared.transport.api_key.unwrap_or_default();
        self.enabled = shared.enabled;
        self.enabled_models = shared.enabled_models;
        self.custom_models = shared.custom_models;
        Ok(())
    }

    fn provider_settings_as_json(&self) -> serde_json::Value {
        self.shared().provider_settings_as_json()
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let mut runtime = self.shared().build_runtime()?;
        runtime.name = self.name().into();
        runtime.display_name = self.display_name().into();
        Ok(runtime)
    }

    fn has_credentials(&self) -> bool {
        self.shared().has_credentials()
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

    fn get_available_models_from_caps(
        &self,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let enabled_set: std::collections::HashSet<_> =
            self.enabled_models.iter().map(|s| s.as_str()).collect();
        let custom_models = self.custom_models();
        let mut models = available_models_from_caps_for_provider(self, model_caps);
        for model in &mut models {
            model.supports_cache_control = SUPPORTS_CACHE_CONTROL;
        }
        merge_custom_models(&mut models, custom_models, &enabled_set);
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

    fn custom_model_pricing(&self, model_id: &str) -> Option<ModelPricing> {
        self.custom_models
            .get(model_id)
            .and_then(|config| config.pricing.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_runtime_enables_cache_control() {
        let provider = AnthropicProvider {
            api_key: "sk-ant-test".to_string(),
            enabled: true,
            enabled_models: vec!["claude-sonnet-4".to_string()],
            ..Default::default()
        };

        let runtime = provider.build_runtime().unwrap();

        assert!(runtime.supports_cache_control);
    }

    #[test]
    fn anthropic_available_models_enable_cache_control_even_when_caps_omit_it() {
        let provider = AnthropicProvider {
            enabled_models: vec!["claude-sonnet-4".to_string()],
            ..Default::default()
        };
        let mut model_caps = HashMap::new();
        model_caps.insert(
            "anthropic/claude-sonnet-4".to_string(),
            ModelCapabilities {
                n_ctx: 200_000,
                tokenizer: "claude".to_string(),
                ..Default::default()
            },
        );

        let models = provider.get_available_models_from_caps(&model_caps);
        let model = models
            .iter()
            .find(|model| model.id == "claude-sonnet-4")
            .expect("anthropic model should be available");

        assert!(model.supports_cache_control);
    }
}
