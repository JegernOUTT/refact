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
enum QwenRegion {
    International,
    China,
}

impl Default for QwenRegion {
    fn default() -> Self {
        Self::International
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum QwenEndpointType {
    Standard,
    CodingPlan,
}

impl Default for QwenEndpointType {
    fn default() -> Self {
        Self::Standard
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QwenProvider {
    pub api_key: String,
    #[serde(default)]
    region: QwenRegion,
    #[serde(default)]
    endpoint_type: QwenEndpointType,
    pub enabled: bool,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
}

impl QwenProvider {
    fn models_dev_provider_id_for(
        region: QwenRegion,
        endpoint_type: QwenEndpointType,
    ) -> &'static str {
        match (region, endpoint_type) {
            (QwenRegion::International, QwenEndpointType::Standard) => "alibaba",
            (QwenRegion::China, QwenEndpointType::Standard) => "alibaba-cn",
            (QwenRegion::International, QwenEndpointType::CodingPlan) => "alibaba-coding-plan",
            (QwenRegion::China, QwenEndpointType::CodingPlan) => "alibaba-coding-plan-cn",
        }
    }

    fn models_dev_provider_id(&self) -> &'static str {
        Self::models_dev_provider_id_for(self.region, self.endpoint_type)
    }

    fn models_dev_config(&self) -> ModelsDevProviderConfig {
        ModelsDevProviderConfig::new(self.models_dev_provider_id(), ModelsDevProviderFamily::Qwen)
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
        let api_key = resolve_env_var(&self.api_key, "", "qwen api_key");
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
impl ProviderTrait for QwenProvider {
    fn name(&self) -> &str {
        "qwen"
    }

    fn display_name(&self) -> &str {
        "Qwen / Alibaba"
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
    f_desc: "DashScope API key. You can set $DASHSCOPE_API_KEY for standard endpoints or $ALIBABA_CODING_PLAN_API_KEY for coding plan endpoints."
    f_placeholder: "sk-... or $DASHSCOPE_API_KEY"
    f_label: "API Key"
    smartlinks:
      - sl_label: "Get DashScope API Key"
        sl_goto: "https://bailian.console.aliyun.com/?tab=model#/api-key"
  region:
    f_type: string
    f_desc: "Region: international or china"
    f_default: "international"
    f_label: "Region"
  endpoint_type:
    f_type: string
    f_desc: "Endpoint type: standard or coding_plan"
    f_default: "standard"
    f_label: "Endpoint Type"
description: |
  Qwen models from Alibaba Cloud DashScope using models.dev catalog metadata.
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
                .map_err(|e| format!("invalid qwen region: {e}"))?;
        }
        if let Some(endpoint_type) = yaml.get("endpoint_type") {
            self.endpoint_type = serde_yaml::from_value(endpoint_type.clone())
                .map_err(|e| format!("invalid qwen endpoint_type: {e}"))?;
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
            "endpoint_type": self.endpoint_type,
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
        let key = resolve_env_var(&self.api_key, "", "qwen api_key");
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
                    tracing::warn!("Qwen: failed to build models.dev model list: {e}");
                    self.get_custom_models_only()
                }
            },
            Err(e) => {
                tracing::warn!("Qwen: failed to load models.dev catalog: {e}");
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
    use crate::providers::models_dev_provider::{validate_models_dev_endpoint, ModelsDevEndpointSource};

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

    fn qwen_catalog() -> ModelsDevCatalog {
        HashMap::from([
            (
                "alibaba".to_string(),
                provider(
                    "alibaba",
                    "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
                    "qwen-plus",
                ),
            ),
            (
                "alibaba-cn".to_string(),
                provider(
                    "alibaba-cn",
                    "https://dashscope.aliyuncs.com/compatible-mode/v1",
                    "qwen-cn",
                ),
            ),
            (
                "alibaba-coding-plan".to_string(),
                provider(
                    "alibaba-coding-plan",
                    "https://coding-intl.dashscope.aliyuncs.com/v1",
                    "qwen-coder-plan",
                ),
            ),
            (
                "alibaba-coding-plan-cn".to_string(),
                provider(
                    "alibaba-coding-plan-cn",
                    "https://coding.dashscope.aliyuncs.com/v1",
                    "qwen-coder-plan-cn",
                ),
            ),
        ])
    }

    #[test]
    fn qwen_region_and_endpoint_type_map_to_models_dev_provider_id() {
        assert_eq!(
            QwenProvider::models_dev_provider_id_for(
                QwenRegion::International,
                QwenEndpointType::Standard,
            ),
            "alibaba"
        );
        assert_eq!(
            QwenProvider::models_dev_provider_id_for(QwenRegion::China, QwenEndpointType::Standard),
            "alibaba-cn"
        );
        assert_eq!(
            QwenProvider::models_dev_provider_id_for(
                QwenRegion::International,
                QwenEndpointType::CodingPlan,
            ),
            "alibaba-coding-plan"
        );
        assert_eq!(
            QwenProvider::models_dev_provider_id_for(
                QwenRegion::China,
                QwenEndpointType::CodingPlan,
            ),
            "alibaba-coding-plan-cn"
        );
    }

    #[test]
    fn qwen_settings_apply_and_as_json_preserve_secret_redaction() {
        let mut provider = QwenProvider {
            api_key: "old-secret".to_string(),
            ..Default::default()
        };
        provider
            .provider_settings_apply(serde_yaml::from_str("api_key: '***'\nregion: china\nendpoint_type: coding_plan\nenabled: true\nenabled_models:\n  - qwen-plus\n").unwrap())
            .unwrap();

        assert_eq!(provider.api_key, "old-secret");
        assert_eq!(provider.region, QwenRegion::China);
        assert_eq!(provider.endpoint_type, QwenEndpointType::CodingPlan);
        assert!(provider.enabled);
        assert_eq!(provider.enabled_models, vec!["qwen-plus"]);

        let settings = provider.provider_settings_as_json();
        assert_eq!(settings["api_key"], "***");
        assert_eq!(settings["region"], "china");
        assert_eq!(settings["endpoint_type"], "coding_plan");
    }

    #[test]
    fn qwen_runtime_requires_enabled_credentials_and_selected_model() {
        let catalog = qwen_catalog();
        let mut provider = QwenProvider {
            api_key: "sk-test".to_string(),
            enabled: true,
            enabled_models: vec!["qwen-plus".to_string()],
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
    fn qwen_available_models_use_models_dev_helper_and_custom_models() {
        let catalog = qwen_catalog();
        let mut provider = QwenProvider {
            enabled_models: vec!["qwen-plus".to_string(), "qwen-custom".to_string()],
            ..Default::default()
        };
        provider.custom_models.insert(
            "qwen-custom".to_string(),
            CustomModelConfig {
                n_ctx: Some(4096),
                supports_tools: Some(true),
                ..Default::default()
            },
        );

        let models = provider.available_models_from_catalog(&catalog).unwrap();
        let qwen_plus = models.iter().find(|model| model.id == "qwen-plus").unwrap();
        let custom = models
            .iter()
            .find(|model| model.id == "qwen-custom")
            .unwrap();

        assert!(qwen_plus.enabled);
        assert!(!qwen_plus.is_custom);
        assert!(custom.enabled);
        assert!(custom.is_custom);
    }

    #[test]
    fn qwen_standard_and_coding_endpoints_validate_through_helper_allowlist() {
        let catalog = qwen_catalog();
        for (region, endpoint_type, expected_endpoint) in [
            (
                QwenRegion::International,
                QwenEndpointType::Standard,
                "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/chat/completions",
            ),
            (
                QwenRegion::China,
                QwenEndpointType::Standard,
                "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions",
            ),
            (
                QwenRegion::International,
                QwenEndpointType::CodingPlan,
                "https://coding-intl.dashscope.aliyuncs.com/v1/chat/completions",
            ),
            (
                QwenRegion::China,
                QwenEndpointType::CodingPlan,
                "https://coding.dashscope.aliyuncs.com/v1/chat/completions",
            ),
        ] {
            let provider = QwenProvider {
                region,
                endpoint_type,
                ..Default::default()
            };
            let runtime = provider.build_runtime_from_catalog(&catalog).unwrap();
            assert_eq!(runtime.chat_endpoint, expected_endpoint);
            validate_models_dev_endpoint(
                &runtime.chat_endpoint,
                ModelsDevProviderFamily::Qwen,
                ModelsDevEndpointSource::Catalog,
                &[],
            )
            .unwrap();
        }
    }

    #[test]
    fn qwen_missing_catalog_provider_returns_custom_models_only() {
        let mut provider = QwenProvider {
            enabled_models: vec!["qwen-custom".to_string()],
            ..Default::default()
        };
        provider.custom_models.insert(
            "qwen-custom".to_string(),
            CustomModelConfig {
                n_ctx: Some(8192),
                ..Default::default()
            },
        );

        let models = provider
            .available_models_from_catalog(&ModelsDevCatalog::new())
            .unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "qwen-custom");
        assert!(models[0].enabled);
    }
}
