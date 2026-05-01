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
    build_models_dev_available_models, derive_models_dev_endpoint, resolve_models_dev_provider,
    validate_models_dev_endpoint, ModelsDevEndpointSource, ModelsDevProviderConfig,
    ModelsDevProviderFamily,
};
use crate::providers::traits::{
    parse_custom_models, parse_enabled_models, set_model_enabled_impl, AvailableModel,
    CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
};

const DEFAULT_DOUBAO_BASE_URL: &str = "https://ark.cn-beijing.volces.com/api/v3";
const DEFAULT_DOUBAO_MODELS_DEV_PROVIDER_ID: &str = "volcengine-cn";

fn default_base_url() -> String {
    DEFAULT_DOUBAO_BASE_URL.to_string()
}

fn default_models_dev_provider_id() -> String {
    DEFAULT_DOUBAO_MODELS_DEV_PROVIDER_ID.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoubaoProvider {
    pub api_key: String,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_models_dev_provider_id")]
    pub models_dev_provider_id: String,
    pub enabled: bool,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
}

impl Default for DoubaoProvider {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: default_base_url(),
            models_dev_provider_id: default_models_dev_provider_id(),
            enabled: false,
            enabled_models: Vec::new(),
            custom_models: HashMap::new(),
        }
    }
}

impl DoubaoProvider {
    fn effective_base_url(&self) -> &str {
        let base_url = self.base_url.trim().trim_end_matches('/');
        if base_url.is_empty() {
            DEFAULT_DOUBAO_BASE_URL
        } else {
            base_url
        }
    }

    fn effective_models_dev_provider_id(&self) -> &str {
        let provider_id = self.models_dev_provider_id.trim();
        if provider_id.is_empty() {
            DEFAULT_DOUBAO_MODELS_DEV_PROVIDER_ID
        } else {
            provider_id
        }
    }

    fn models_dev_config(&self) -> ModelsDevProviderConfig {
        ModelsDevProviderConfig::new(
            self.effective_models_dev_provider_id(),
            ModelsDevProviderFamily::Unknown,
        )
        .with_endpoint_override(self.effective_base_url())
        .with_wire_format_override(WireFormat::OpenaiChatCompletions)
    }

    fn catalog_unavailable_message(&self) -> String {
        format!(
            "first-party Doubao catalog is unavailable until Volcengine/Ark provider '{}' is present in models.dev; add Ark deployment or endpoint IDs as custom models",
            self.effective_models_dev_provider_id()
        )
    }

    fn catalog_provider_missing(&self, catalog: &ModelsDevCatalog) -> bool {
        resolve_models_dev_provider(catalog, self.effective_models_dev_provider_id()).is_none()
    }

    fn available_models_from_catalog(
        &self,
        catalog: &ModelsDevCatalog,
    ) -> Result<Vec<AvailableModel>, String> {
        if self.catalog_provider_missing(catalog) {
            tracing::warn!("Doubao: {}", self.catalog_unavailable_message());
        }
        build_models_dev_available_models(
            catalog,
            &self.models_dev_config(),
            &self.enabled_models,
            &self.custom_models,
        )
    }

    fn chat_endpoint(&self) -> Result<String, String> {
        let endpoint = derive_models_dev_endpoint(
            self.effective_base_url(),
            WireFormat::OpenaiChatCompletions,
        );
        validate_models_dev_endpoint(
            &endpoint,
            ModelsDevProviderFamily::Unknown,
            ModelsDevEndpointSource::UserConfigured,
            &[],
        )?;
        Ok(endpoint)
    }

    fn build_runtime_from_catalog(
        &self,
        catalog: &ModelsDevCatalog,
    ) -> Result<ProviderRuntime, String> {
        let api_key = resolve_env_var(&self.api_key, "", "doubao api_key");
        let selected_model_available = self
            .available_models_from_catalog(catalog)?
            .iter()
            .any(|model| model.enabled);

        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: self.enabled && !api_key.is_empty() && selected_model_available,
            readonly: false,
            wire_format: WireFormat::OpenaiChatCompletions,
            chat_endpoint: self.chat_endpoint()?,
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
impl ProviderTrait for DoubaoProvider {
    fn name(&self) -> &str {
        "doubao"
    }

    fn display_name(&self) -> &str {
        "Doubao / Volcengine Ark"
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
    f_desc: "Volcengine Ark API key. You can set $DOUBAO_API_KEY or paste an API key from the Volcengine Ark console."
    f_placeholder: "sk-... or $DOUBAO_API_KEY"
    f_label: "API Key"
    smartlinks:
      - sl_label: "Open Volcengine Ark Console"
        sl_goto: "https://console.volcengine.com/ark"
  base_url:
    f_type: string_long
    f_desc: "OpenAI-compatible Ark base URL. The chat/completions path is added automatically when omitted."
    f_default: "https://ark.cn-beijing.volces.com/api/v3"
    f_label: "Base URL"
  models_dev_provider_id:
    f_type: string
    f_desc: "models.dev provider id to use for first-party Volcengine/Ark catalog models when available. Built-in models may be absent until Volcengine/Ark appears in models.dev; use custom models with Ark deployment or endpoint IDs meanwhile."
    f_default: "volcengine-cn"
    f_label: "models.dev Provider ID"
description: |
  Doubao / Volcengine Ark uses the OpenAI Chat Completions wire format. First-party built-in models may be absent until Volcengine/Ark is present in models.dev. Add custom model IDs for your Ark deployments or endpoint IDs and enable them to use Doubao today.
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
        if let Some(base_url) = yaml.get("base_url").and_then(|v| v.as_str()) {
            let base_url = base_url.trim().trim_end_matches('/');
            self.base_url = if base_url.is_empty() {
                default_base_url()
            } else {
                base_url.to_string()
            };
        }
        if let Some(provider_id) = yaml.get("models_dev_provider_id").and_then(|v| v.as_str()) {
            let provider_id = provider_id.trim();
            self.models_dev_provider_id = if provider_id.is_empty() {
                default_models_dev_provider_id()
            } else {
                provider_id.to_string()
            };
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
            "base_url": self.effective_base_url(),
            "models_dev_provider_id": self.effective_models_dev_provider_id(),
            "enabled": self.enabled,
            "enabled_models": self.enabled_models,
            "custom_models": self.custom_models
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let catalog = match load_models_dev_snapshot_catalog() {
            Ok(catalog) => catalog,
            Err(e) => {
                tracing::warn!("Doubao: failed to load models.dev catalog: {e}");
                ModelsDevCatalog::new()
            }
        };
        self.build_runtime_from_catalog(&catalog)
    }

    fn has_credentials(&self) -> bool {
        let key = resolve_env_var(&self.api_key, "", "doubao api_key");
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
        let catalog = match load_models_dev_snapshot_catalog() {
            Ok(catalog) => catalog,
            Err(e) => {
                tracing::warn!("Doubao: failed to load models.dev catalog: {e}");
                ModelsDevCatalog::new()
            }
        };
        match self.available_models_from_catalog(&catalog) {
            Ok(models) => models,
            Err(e) => {
                tracing::warn!("Doubao: failed to build models.dev model list: {e}");
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

    fn doubao_catalog() -> ModelsDevCatalog {
        HashMap::from([(
            "volcengine-cn".to_string(),
            ModelsDevProvider {
                id: "volcengine-cn".to_string(),
                name: "Volcengine Ark".to_string(),
                api: Some("https://ark.cn-beijing.volces.com/api/v3".to_string()),
                npm: Some("@ai-sdk/openai-compatible".to_string()),
                models: HashMap::from([(
                    "doubao-catalog".to_string(),
                    text_chat_model("doubao-catalog"),
                )]),
                ..Default::default()
            },
        )])
    }

    #[test]
    fn doubao_default_settings_use_volcengine_cn_and_ark_base_url() {
        let provider = DoubaoProvider::default();

        assert_eq!(provider.effective_models_dev_provider_id(), "volcengine-cn");
        assert_eq!(
            provider.effective_base_url(),
            "https://ark.cn-beijing.volces.com/api/v3"
        );
        assert_eq!(
            provider.default_wire_format(),
            WireFormat::OpenaiChatCompletions
        );
    }

    #[test]
    fn doubao_missing_models_dev_provider_returns_custom_only_available_models() {
        let mut provider = DoubaoProvider::default();
        provider.custom_models.insert(
            "ep-20260430-demo".to_string(),
            CustomModelConfig {
                n_ctx: Some(8192),
                supports_tools: Some(true),
                ..Default::default()
            },
        );
        provider.set_model_enabled("ep-20260430-demo", true);

        let models = provider
            .available_models_from_catalog(&ModelsDevCatalog::new())
            .unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "ep-20260430-demo");
        assert!(models[0].enabled);
        assert!(models[0].is_custom);
        assert!(models[0].supports_tools);
    }

    #[test]
    fn doubao_configured_custom_model_can_be_enabled_and_removed() {
        let mut provider = DoubaoProvider::default();
        provider.add_custom_model(
            "ark-deployment-id".to_string(),
            CustomModelConfig {
                n_ctx: Some(32_768),
                supports_tools: Some(true),
                ..Default::default()
            },
        );
        provider.set_model_enabled("ark-deployment-id", true);

        let models = provider
            .available_models_from_catalog(&ModelsDevCatalog::new())
            .unwrap();
        let custom = models
            .iter()
            .find(|model| model.id == "ark-deployment-id")
            .unwrap();
        assert!(custom.enabled);
        assert!(custom.is_custom);
        assert_eq!(custom.n_ctx, 32_768);

        assert!(provider.remove_custom_model("ark-deployment-id"));
        let models = provider
            .available_models_from_catalog(&ModelsDevCatalog::new())
            .unwrap();
        assert!(models.is_empty());
    }

    #[test]
    fn doubao_uses_models_dev_catalog_when_provider_exists() {
        let mut provider = DoubaoProvider::default();
        provider.set_model_enabled("doubao-catalog", true);
        provider.add_custom_model(
            "ark-custom".to_string(),
            CustomModelConfig {
                n_ctx: Some(4096),
                ..Default::default()
            },
        );

        let models = provider
            .available_models_from_catalog(&doubao_catalog())
            .unwrap();
        let catalog_model = models
            .iter()
            .find(|model| model.id == "doubao-catalog")
            .unwrap();
        let custom = models
            .iter()
            .find(|model| model.id == "ark-custom")
            .unwrap();

        assert!(catalog_model.enabled);
        assert!(!catalog_model.is_custom);
        assert!(!custom.enabled);
        assert!(custom.is_custom);
    }

    #[test]
    fn doubao_runtime_endpoint_uses_base_url_chat_completions_normalization() {
        let provider = DoubaoProvider {
            api_key: "sk-test".to_string(),
            base_url: "https://ark.cn-beijing.volces.com/api/v3/".to_string(),
            enabled: true,
            enabled_models: vec!["ep-20260430-demo".to_string()],
            custom_models: HashMap::from([(
                "ep-20260430-demo".to_string(),
                CustomModelConfig::default(),
            )]),
            ..Default::default()
        };

        let runtime = provider
            .build_runtime_from_catalog(&ModelsDevCatalog::new())
            .unwrap();

        assert!(runtime.enabled);
        assert_eq!(runtime.wire_format, WireFormat::OpenaiChatCompletions);
        assert_eq!(
            runtime.chat_endpoint,
            "https://ark.cn-beijing.volces.com/api/v3/chat/completions"
        );
    }

    #[test]
    fn doubao_runtime_accepts_full_chat_completions_url() {
        let provider = DoubaoProvider {
            base_url: "https://ark.cn-beijing.volces.com/api/v3/chat/completions".to_string(),
            ..Default::default()
        };

        assert_eq!(
            provider.chat_endpoint().unwrap(),
            "https://ark.cn-beijing.volces.com/api/v3/chat/completions"
        );
    }

    #[test]
    fn doubao_runtime_requires_enabled_credentials_and_selected_available_model() {
        let catalog = ModelsDevCatalog::new();
        let mut provider = DoubaoProvider {
            api_key: "sk-test".to_string(),
            enabled: true,
            enabled_models: vec!["ark-custom".to_string()],
            custom_models: HashMap::from([(
                "ark-custom".to_string(),
                CustomModelConfig::default(),
            )]),
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
        provider.enabled_models = vec!["missing-custom".to_string()];
        assert!(
            !provider
                .build_runtime_from_catalog(&catalog)
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn doubao_settings_apply_and_as_json_preserve_secret_redaction() {
        let mut provider = DoubaoProvider {
            api_key: "old-secret".to_string(),
            ..Default::default()
        };
        provider
            .provider_settings_apply(
                serde_yaml::from_str(
                    "api_key: '***'\nbase_url: https://ark.cn-beijing.volces.com/api/v3/\nmodels_dev_provider_id: volcengine\nenabled: true\nenabled_models:\n  - ark-custom\n",
                )
                .unwrap(),
            )
            .unwrap();

        assert_eq!(provider.api_key, "old-secret");
        assert_eq!(
            provider.base_url,
            "https://ark.cn-beijing.volces.com/api/v3"
        );
        assert_eq!(provider.models_dev_provider_id, "volcengine");
        assert!(provider.enabled);
        assert_eq!(provider.enabled_models, vec!["ark-custom"]);

        let settings = provider.provider_settings_as_json();
        assert_eq!(settings["api_key"], "***");
        assert_eq!(
            settings["base_url"],
            "https://ark.cn-beijing.volces.com/api/v3"
        );
        assert_eq!(settings["models_dev_provider_id"], "volcengine");
    }

    #[test]
    fn doubao_schema_exposes_actionable_catalog_warning() {
        let provider = DoubaoProvider::default();
        let schema = provider.provider_schema();
        let warning = provider.catalog_unavailable_message();

        assert!(schema.contains("built-in models may be absent"));
        assert!(schema.contains("custom model IDs"));
        assert!(schema.contains("deployment"));
        assert!(warning.contains("Volcengine/Ark"));
        assert!(warning.contains("models.dev"));
        assert!(warning.contains("custom models"));
    }
}
