use std::any::Any;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::llm::adapter::WireFormat;
use crate::providers::config::resolve_env_var;
use crate::providers::traits::{ModelSource, ProviderRuntime, ProviderTrait};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RefactProvider {
    pub address_url: String,
    pub api_key: String,
    pub enabled: bool,
    #[serde(default)]
    pub disabled_models: Vec<String>,
}

impl RefactProvider {
    pub fn from_cli(address_url: String, api_key: String) -> Self {
        Self {
            address_url,
            api_key,
            enabled: true,
            disabled_models: Vec::new(),
        }
    }
}

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
            enabled: self.enabled,
            readonly: false,
            wire_format: self.default_wire_format(),
            chat_endpoint: format!("{}/v1/chat/completions", base_url),
            completion_endpoint: format!("{}/v1/completions", base_url),
            embedding_endpoint: format!("{}/v1/embeddings", base_url),
            api_key,
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

    fn model_source(&self) -> ModelSource {
        ModelSource::ModelCaps
    }

    fn disabled_models(&self) -> &[String] {
        &self.disabled_models
    }

    fn set_model_enabled(&mut self, model_id: &str, enabled: bool) {
        crate::providers::traits::set_model_disabled_impl(&mut self.disabled_models, model_id, enabled);
    }
}

