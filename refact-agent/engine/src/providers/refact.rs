use std::any::Any;
use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::caps::model_caps::{ModelCapabilities, resolve_model_caps};
use crate::llm::adapter::WireFormat;
use crate::providers::config::resolve_env_var;
use crate::providers::traits::{AvailableModel, ModelSource, ProviderRuntime, ProviderTrait};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RefactProvider {
    pub address_url: String,
    pub api_key: String,
    pub enabled: bool,
    #[serde(default)]
    pub disabled_models: Vec<String>,
    #[serde(skip)]
    pub running_models: Vec<String>,
}

impl RefactProvider {
    pub fn from_cli(address_url: String, api_key: String) -> Self {
        Self {
            address_url,
            api_key,
            enabled: true,
            disabled_models: Vec::new(),
            running_models: Vec::new(),
        }
    }
}

#[async_trait]
impl ProviderTrait for RefactProvider {
    fn name(&self) -> &'static str {
        "refact"
    }

    fn display_name(&self) -> &'static str {
        "Refact Cloud"
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
        WireFormat::Refact
    }

    fn model_filter_regex(&self) -> Option<&'static str> {
        None
    }

    fn provider_schema(&self) -> &'static str {
        r#"
fields:
  api_key:
    f_type: string_long
    f_desc: "API key (usually set via --api-key CLI argument)"
    f_label: "API Key"
    f_extra: true
description: |
  Refact Cloud provider. Settings are typically configured via CLI arguments.
available:
  on_your_laptop_possible: true
  when_isolated_possible: false
"#
    }

    fn provider_settings_apply(&mut self, yaml: serde_yaml::Value) -> Result<(), String> {
        if let Some(api_key) = yaml.get("api_key").and_then(|v| v.as_str()) {
            if api_key != "***" {
                self.api_key = api_key.to_string();
            }
        }
        if let Some(enabled) = yaml.get("enabled").and_then(|v| v.as_bool()) {
            self.enabled = enabled;
        }
        crate::providers::traits::parse_disabled_models(&yaml, &mut self.disabled_models);
        Ok(())
    }

    fn provider_settings_as_json(&self) -> serde_json::Value {
        json!({
            "address_url": self.address_url,
            "api_key": if self.api_key.is_empty() { "" } else { "***" },
            "enabled": self.enabled,
            "disabled_models": self.disabled_models
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let api_key = resolve_env_var(&self.api_key, "", "refact api_key");
        let base_url = if self.address_url.is_empty()
            || self.address_url.to_lowercase() == "refact"
        {
            "https://inference.smallcloud.ai".to_string()
        } else {
            self.address_url.trim_end_matches('/').to_string()
        };

        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: self.enabled && !self.running_models.is_empty(),
            readonly: false,
            wire_format: self.default_wire_format(),
            chat_endpoint: format!("{}/v1/chat/completions", base_url),
            completion_endpoint: format!("{}/v1/completions", base_url),
            embedding_endpoint: format!("{}/v1/embeddings", base_url),
            api_key,
            auth_token: String::new(),
            tokenizer_api_key: String::new(),
            extra_headers: HashMap::new(),
            support_metadata: true,
            chat_models: Vec::new(),
            completion_models: Vec::new(),
            embedding_model: None,
        })
    }

    fn is_readonly(&self) -> bool {
        false
    }

    fn has_credentials(&self) -> bool {
        true
    }

    fn model_source(&self) -> ModelSource {
        ModelSource::ModelCaps
    }

    fn selected_model_count(&self) -> usize {
        if self.running_models.is_empty() {
            return 0;
        }
        self.running_models.iter()
            .filter(|m| !self.disabled_models.contains(m))
            .count()
    }

    fn disabled_models(&self) -> &[String] {
        &self.disabled_models
    }

    fn set_model_enabled(&mut self, model_id: &str, enabled: bool) {
        crate::providers::traits::set_model_disabled_impl(&mut self.disabled_models, model_id, enabled);
    }

    fn set_running_models(&mut self, running_models: Vec<String>) {
        self.running_models = running_models;
    }

    fn get_available_models_from_caps(
        &self,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        if self.running_models.is_empty() {
            return Vec::new();
        }

        let mut models: Vec<AvailableModel> = Vec::new();

        for running_model in &self.running_models {
            if let Some(resolved) = resolve_model_caps(model_caps, running_model) {
                let disabled = self.disabled_models.contains(running_model);
                let pricing = self.model_pricing(running_model);
                let mut model = AvailableModel::from_caps(running_model, &resolved.caps, !disabled, pricing);
                if running_model != &resolved.matched_key {
                    model.display_name = Some(running_model.clone());
                }
                models.push(model);
            } else {
                tracing::warn!(
                    "Refact running model '{}' not found in model capabilities, adding with defaults",
                    running_model
                );
                let disabled = self.disabled_models.contains(running_model);
                models.push(AvailableModel {
                    id: running_model.clone(),
                    display_name: None,
                    n_ctx: 4096,
                    supports_tools: false,
                    supports_multimodality: false,
                    reasoning_effort_options: None,
                    supports_thinking_budget: false,
                    supports_adaptive_thinking_budget: false,
                    tokenizer: None,
                    enabled: !disabled,
                    is_custom: false,
                    pricing: None,
                    available_providers: Vec::new(),
                    selected_provider: None,
                    max_output_tokens: None,
                    provider_variants: Vec::new(),
                });
            }
        }

        models.sort_by(|a, b| a.id.cmp(&b.id));
        models
    }
}

