use std::any::Any;
use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::caps::model_caps::ModelCapabilities;
use crate::caps::models_dev::{load_models_dev_snapshot_catalog, ModelsDevCatalog};
use crate::llm::adapter::WireFormat;
use crate::providers::config::resolve_env_var;
use crate::providers::models_dev_provider::{
    build_models_dev_available_models, models_dev_provider_wire_format,
    models_dev_runtime_endpoint, ModelsDevProviderConfig, ModelsDevProviderFamily,
};
use crate::providers::traits::{
    AvailableModel, CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
    parse_enabled_models, parse_custom_models, set_model_enabled_impl,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum KimiRegion {
    International,
    China,
}

impl Default for KimiRegion {
    fn default() -> Self {
        Self::International
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KimiProvider {
    pub api_key: String,
    #[serde(default)]
    region: KimiRegion,
    pub enabled: bool,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
}

impl KimiProvider {
    fn models_dev_provider_id_for(region: KimiRegion) -> &'static str {
        match region {
            KimiRegion::International => "moonshotai",
            KimiRegion::China => "moonshotai-cn",
        }
    }

    fn models_dev_provider_id(&self) -> &'static str {
        Self::models_dev_provider_id_for(self.region)
    }

    fn models_dev_config(&self) -> ModelsDevProviderConfig {
        ModelsDevProviderConfig::new(self.models_dev_provider_id(), ModelsDevProviderFamily::Kimi)
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
        let api_key = resolve_env_var(&self.api_key, "", "kimi api_key");
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
}

#[async_trait]
impl ProviderTrait for KimiProvider {
    fn name(&self) -> &str {
        "kimi"
    }

    fn display_name(&self) -> &str {
        "Moonshot / Kimi"
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
    f_desc: "Moonshot API key. You can set $MOONSHOT_API_KEY or paste a key from platform.moonshot.ai."
    f_placeholder: "sk-... or $MOONSHOT_API_KEY"
    f_label: "API Key"
    smartlinks:
      - sl_label: "Get Moonshot API Key"
        sl_goto: "https://platform.moonshot.ai/console/api-keys"
  region:
    f_type: string
    f_desc: "Region: international or china"
    f_default: "international"
    f_label: "Region"
description: |
  Moonshot / Kimi models using models.dev catalog metadata.
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
        if let Some(region) = yaml.get("region") {
            self.region = serde_yaml::from_value(region.clone())
                .map_err(|e| format!("invalid kimi region: {e}"))?;
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
            "region": self.region,
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
        let key = resolve_env_var(&self.api_key, "", "kimi api_key");
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
                    tracing::warn!("Kimi: failed to build models.dev model list: {e}");
                    self.get_custom_models_only()
                }
            },
            Err(e) => {
                tracing::warn!("Kimi: failed to load models.dev catalog: {e}");
                self.get_custom_models_only()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::models_dev::{
        ModelsDevLimit, ModelsDevModalities, ModelsDevModel, ModelsDevProvider,
    };

    fn text_chat_model(model_id: &str) -> ModelsDevModel {
        ModelsDevModel {
            id: model_id.to_string(),
            name: model_id.to_string(),
            tool_call: Some(true),
            limit: Some(ModelsDevLimit {
                context: Some(128_000),
                output: Some(16_384),
                ..Default::default()
            }),
            modalities: Some(ModelsDevModalities {
                input: vec!["text".to_string()],
                output: vec!["text".to_string()],
            }),
            ..Default::default()
        }
    }

    fn provider(provider_id: &str, api: &str, model_id: &str) -> ModelsDevProvider {
        ModelsDevProvider {
            id: provider_id.to_string(),
            name: provider_id.to_string(),
            api: Some(api.to_string()),
            npm: Some("@ai-sdk/openai-compatible".to_string()),
            models: HashMap::from([(model_id.to_string(), text_chat_model(model_id))]),
            ..Default::default()
        }
    }

    fn kimi_catalog() -> ModelsDevCatalog {
        HashMap::from([
            (
                "moonshotai".to_string(),
                provider("moonshotai", "https://api.moonshot.ai/v1", "kimi-k2"),
            ),
            (
                "moonshotai-cn".to_string(),
                provider("moonshotai-cn", "https://api.moonshot.cn/v1", "kimi-k2-cn"),
            ),
        ])
    }

    #[test]
    fn kimi_region_maps_to_models_dev_provider_id() {
        assert_eq!(
            KimiProvider::models_dev_provider_id_for(KimiRegion::International),
            "moonshotai"
        );
        assert_eq!(
            KimiProvider::models_dev_provider_id_for(KimiRegion::China),
            "moonshotai-cn"
        );
    }

    #[test]
    fn kimi_settings_apply_and_as_json_preserve_secret_redaction() {
        let mut provider = KimiProvider {
            api_key: "old-secret".to_string(),
            ..Default::default()
        };
        provider
            .provider_settings_apply(
                serde_yaml::from_str(
                    "api_key: '***'\nregion: china\nenabled: true\nenabled_models:\n  - kimi-k2\n",
                )
                .unwrap(),
            )
            .unwrap();

        assert_eq!(provider.api_key, "old-secret");
        assert_eq!(provider.region, KimiRegion::China);
        assert!(provider.enabled);
        assert_eq!(provider.enabled_models, vec!["kimi-k2"]);

        let settings = provider.provider_settings_as_json();
        assert_eq!(settings["api_key"], "***");
        assert_eq!(settings["region"], "china");
    }

    #[test]
    fn kimi_runtime_requires_enabled_credentials_and_selected_model() {
        let catalog = kimi_catalog();
        let mut provider = KimiProvider {
            api_key: "sk-test".to_string(),
            enabled: true,
            enabled_models: vec!["kimi-k2".to_string()],
            ..Default::default()
        };
        assert!(
            provider
                .build_runtime_from_catalog(&catalog)
                .unwrap()
                .enabled
        );

        provider.enabled = false;
        assert!(
            !provider
                .build_runtime_from_catalog(&catalog)
                .unwrap()
                .enabled
        );
        provider.enabled = true;
        provider.api_key.clear();
        assert!(
            !provider
                .build_runtime_from_catalog(&catalog)
                .unwrap()
                .enabled
        );
        provider.api_key = "sk-test".to_string();
        provider.enabled_models.clear();
        assert!(
            !provider
                .build_runtime_from_catalog(&catalog)
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn kimi_available_models_use_models_dev_helper_and_custom_models() {
        let catalog = kimi_catalog();
        let mut provider = KimiProvider {
            enabled_models: vec!["kimi-k2".to_string(), "kimi-custom".to_string()],
            ..Default::default()
        };
        provider.custom_models.insert(
            "kimi-custom".to_string(),
            CustomModelConfig {
                n_ctx: Some(4096),
                supports_tools: Some(true),
                ..Default::default()
            },
        );

        let models = provider.available_models_from_catalog(&catalog).unwrap();
        let kimi_k2 = models.iter().find(|model| model.id == "kimi-k2").unwrap();
        let custom = models
            .iter()
            .find(|model| model.id == "kimi-custom")
            .unwrap();

        assert!(kimi_k2.enabled);
        assert!(!kimi_k2.is_custom);
        assert!(custom.enabled);
        assert!(custom.is_custom);
    }

    #[test]
    fn kimi_region_endpoints_validate_through_helper_allowlist() {
        let catalog = kimi_catalog();
        for (region, expected_endpoint) in [
            (
                KimiRegion::International,
                "https://api.moonshot.ai/v1/chat/completions",
            ),
            (
                KimiRegion::China,
                "https://api.moonshot.cn/v1/chat/completions",
            ),
        ] {
            let provider = KimiProvider {
                region,
                ..Default::default()
            };
            let runtime = provider.build_runtime_from_catalog(&catalog).unwrap();
            assert_eq!(runtime.chat_endpoint, expected_endpoint);
        }
    }

    #[test]
    fn kimi_missing_catalog_provider_returns_custom_models_only() {
        let mut provider = KimiProvider {
            enabled_models: vec!["kimi-custom".to_string()],
            ..Default::default()
        };
        provider.custom_models.insert(
            "kimi-custom".to_string(),
            CustomModelConfig {
                n_ctx: Some(8192),
                ..Default::default()
            },
        );

        let models = provider
            .available_models_from_catalog(&ModelsDevCatalog::new())
            .unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "kimi-custom");
        assert!(models[0].enabled);
    }
}
