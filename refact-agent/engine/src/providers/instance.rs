use std::any::Any;
use std::collections::HashMap;
use std::path::Path;

use async_trait::async_trait;

use crate::caps::model_caps::ModelCapabilities;
use crate::llm::adapter::WireFormat;
use crate::providers::traits::{
    AvailableModel, CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
};

pub struct ProviderInstance {
    pub instance_id: String,
    pub base_provider: String,
    pub display_name: String,
    pub inner: Box<dyn ProviderTrait>,
}

impl ProviderInstance {
    pub fn new(
        instance_id: impl Into<String>,
        base_provider: impl Into<String>,
        display_name: impl Into<String>,
        inner: Box<dyn ProviderTrait>,
    ) -> Self {
        let display_name = display_name.into();
        let display_name = if display_name.trim().is_empty() {
            inner.display_name().to_string()
        } else {
            display_name
        };
        Self {
            instance_id: instance_id.into(),
            base_provider: base_provider.into(),
            display_name,
            inner,
        }
    }

    pub fn from_inner(instance_id: impl Into<String>, inner: Box<dyn ProviderTrait>) -> Self {
        let base_provider = inner.base_provider_name().to_string();
        Self::new(instance_id, base_provider, "", inner)
    }
}

impl Clone for ProviderInstance {
    fn clone(&self) -> Self {
        Self {
            instance_id: self.instance_id.clone(),
            base_provider: self.base_provider.clone(),
            display_name: self.display_name.clone(),
            inner: self.inner.clone_box(),
        }
    }
}

#[async_trait]
impl ProviderTrait for ProviderInstance {
    fn name(&self) -> &str {
        &self.instance_id
    }

    fn display_name(&self) -> &str {
        &self.display_name
    }

    fn base_provider_name(&self) -> &str {
        &self.base_provider
    }

    fn as_any(&self) -> &dyn Any {
        self.inner.as_any()
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self.inner.as_any_mut()
    }

    fn clone_box(&self) -> Box<dyn ProviderTrait> {
        Box::new(self.clone())
    }

    fn default_wire_format(&self) -> WireFormat {
        self.inner.default_wire_format()
    }

    fn supported_wire_formats(&self) -> Vec<WireFormat> {
        self.inner.supported_wire_formats()
    }

    fn model_filter_regex(&self) -> Option<&'static str> {
        self.inner.model_filter_regex()
    }

    fn provider_schema(&self) -> &'static str {
        self.inner.provider_schema()
    }

    fn provider_settings_apply(&mut self, yaml: serde_yaml::Value) -> Result<(), String> {
        self.inner.provider_settings_apply(yaml)
    }

    fn provider_settings_as_json(&self) -> serde_json::Value {
        let mut settings = self.inner.provider_settings_as_json();
        if let serde_json::Value::Object(map) = &mut settings {
            map.insert(
                "base_provider".to_string(),
                serde_json::Value::String(self.base_provider.clone()),
            );
            map.insert(
                "display_name".to_string(),
                serde_json::Value::String(self.display_name.clone()),
            );
        }
        settings
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let mut runtime = self.inner.build_runtime()?;
        runtime.name = self.instance_id.clone();
        runtime.display_name = self.display_name.clone();
        Ok(runtime)
    }

    fn is_readonly(&self) -> bool {
        self.inner.is_readonly()
    }

    fn is_hidden_from_list(&self) -> bool {
        self.inner.is_hidden_from_list()
    }

    fn has_credentials(&self) -> bool {
        self.inner.has_credentials()
    }

    fn selected_model_count(&self) -> usize {
        self.inner.selected_model_count()
    }

    fn model_source(&self) -> ModelSource {
        self.inner.model_source()
    }

    fn enabled_models(&self) -> &[String] {
        self.inner.enabled_models()
    }

    fn disabled_models(&self) -> &[String] {
        self.inner.disabled_models()
    }

    fn custom_models(&self) -> &HashMap<String, CustomModelConfig> {
        self.inner.custom_models()
    }

    fn set_model_enabled(&mut self, model_id: &str, enabled: bool) {
        self.inner.set_model_enabled(model_id, enabled);
    }

    fn set_selected_provider(&mut self, model_id: &str, provider: Option<String>) {
        self.inner.set_selected_provider(model_id, provider);
    }

    fn selected_providers(&self) -> &HashMap<String, String> {
        self.inner.selected_providers()
    }

    fn add_custom_model(&mut self, model_id: String, config: CustomModelConfig) {
        self.inner.add_custom_model(model_id, config);
    }

    fn remove_custom_model(&mut self, model_id: &str) -> bool {
        self.inner.remove_custom_model(model_id)
    }

    fn custom_model_pricing(&self, model_id: &str) -> Option<ModelPricing> {
        self.inner.custom_model_pricing(model_id)
    }

    async fn fetch_available_models(
        &self,
        http_client: &reqwest::Client,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        self.inner
            .fetch_available_models(http_client, model_caps)
            .await
    }

    async fn startup_refresh_and_sync(
        &mut self,
        http_client: &reqwest::Client,
        config_dir: &Path,
    ) -> Result<(), String> {
        self.inner
            .startup_refresh_and_sync(http_client, config_dir)
            .await
    }

    fn get_available_models_from_caps(
        &self,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        self.inner.get_available_models_from_caps(model_caps)
    }

    fn get_custom_models_only(&self) -> Vec<AvailableModel> {
        self.inner.get_custom_models_only()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::custom::CustomProvider;

    fn custom_provider() -> CustomProvider {
        CustomProvider {
            enabled: true,
            enabled_models: vec!["test-model".to_string()],
            chat_endpoint: "https://example.com/v1/chat/completions".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn wrapper_reports_instance_id_base_and_display() {
        let provider = ProviderInstance::new(
            "custom_2",
            "custom",
            "",
            Box::new(CustomProvider::default()),
        );

        assert_eq!(provider.name(), "custom_2");
        assert_eq!(provider.base_provider_name(), "custom");
        assert_eq!(provider.display_name(), "Custom");
        let settings = provider.provider_settings_as_json();
        assert_eq!(settings["base_provider"], "custom");
        assert_eq!(settings["display_name"], "Custom");
    }

    #[test]
    fn wrapper_build_runtime_patches_runtime_identity() {
        let provider = ProviderInstance::new(
            "custom_work",
            "custom",
            "Work Custom",
            Box::new(custom_provider()),
        );

        let runtime = provider.build_runtime().unwrap();

        assert_eq!(runtime.name, "custom_work");
        assert_eq!(runtime.display_name, "Work Custom");
        assert_eq!(
            runtime.chat_endpoint,
            "https://example.com/v1/chat/completions"
        );
    }

    #[test]
    fn wrapper_as_any_downcasts_to_inner_concrete_provider_type() {
        let provider = ProviderInstance::new(
            "custom_2",
            "custom",
            "Custom 2",
            Box::new(CustomProvider::default()),
        );

        assert!(provider.as_any().downcast_ref::<CustomProvider>().is_some());
    }

    #[test]
    fn clone_box_preserves_wrapper_identity() {
        let provider = ProviderInstance::new(
            "custom_2",
            "custom",
            "Custom 2",
            Box::new(CustomProvider::default()),
        );

        let cloned = provider.clone_box();

        assert_eq!(cloned.name(), "custom_2");
        assert_eq!(cloned.base_provider_name(), "custom");
        assert_eq!(cloned.display_name(), "Custom 2");
        assert!(cloned.as_any().downcast_ref::<CustomProvider>().is_some());
    }
}
