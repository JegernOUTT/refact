use std::any::Any;
use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use refact_core::llm_types::WireFormat;
use refact_core::model_caps::{resolve_model_caps, ModelCapabilities};
use refact_core::provider_types::{available_model_from_catalog_and_live, LiveModelFields};
use crate::config::resolve_env_var;
use crate::traits::{
    AvailableModel, CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
    merge_custom_models, parse_enabled_models, parse_custom_models, set_model_enabled_impl,
};

const GROQ_MODELS_URL: &str = "https://api.groq.com/openai/v1/models";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GroqProvider {
    pub api_key: String,
    pub enabled: bool,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
}

#[async_trait]
impl ProviderTrait for GroqProvider {
    fn name(&self) -> &str {
        "groq"
    }

    fn display_name(&self) -> &str {
        "Groq"
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
        Some(r"^(llama-|mixtral-|gemma-)")
    }

    fn provider_schema(&self) -> &'static str {
        r#"
fields:
  api_key:
    f_type: string_long
    f_desc: "Groq API key from console.groq.com"
    f_placeholder: "gsk_..."
    f_label: "API Key"
    smartlinks:
      - sl_label: "Get API Key"
        sl_goto: "https://console.groq.com/keys"
description: |
  Groq fast inference for open-source models.
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
            "enabled": self.enabled,
            "enabled_models": self.enabled_models,
            "custom_models": self.custom_models
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let api_key = resolve_env_var(&self.api_key, "", "groq api_key");

        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: self.enabled && !api_key.is_empty() && !self.enabled_models.is_empty(),
            readonly: false,
            wire_format: self.default_wire_format(),
            chat_endpoint: "https://api.groq.com/openai/v1/chat/completions".to_string(),
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

    fn has_credentials(&self) -> bool {
        let key = resolve_env_var(&self.api_key, "", "groq api_key");
        !key.is_empty()
    }

    fn model_source(&self) -> ModelSource {
        ModelSource::Api // Groq has an API for models
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
        http_client: &reqwest::Client,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let api_key = resolve_env_var(&self.api_key, "", "groq api_key");
        if api_key.is_empty() {
            return self.get_custom_models_only();
        }

        let response = match http_client
            .get(GROQ_MODELS_URL)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {api_key}"))
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!("Groq: failed to fetch models: {error}");
                return self.get_custom_models_only();
            }
        };
        if !response.status().is_success() {
            tracing::warn!(
                "Groq: models endpoint returned status {}",
                response.status()
            );
            return self.get_custom_models_only();
        }
        let json = match response.json::<serde_json::Value>().await {
            Ok(json) => json,
            Err(error) => {
                tracing::warn!("Groq: failed to parse models response: {error}");
                return self.get_custom_models_only();
            }
        };

        self.parse_available_models(&json, model_caps)
    }
}

impl GroqProvider {
    fn is_chat_model(model: &serde_json::Value, id: &str) -> bool {
        const NON_CHAT_MARKERS: &[&str] = &[
            "whisper",
            "transcription",
            "transcribe",
            "audio",
            "embedding",
            "embeddings",
            "embed",
            "speech",
            "tts",
        ];

        let metadata = ["mode", "type"]
            .into_iter()
            .filter_map(|field| model.get(field).and_then(|value| value.as_str()))
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>();
        if metadata.iter().any(|value| {
            NON_CHAT_MARKERS.iter().any(|marker| {
                value
                    .split(|c: char| !c.is_ascii_alphanumeric())
                    .any(|part| part == *marker)
            })
        }) {
            return false;
        }
        if metadata.iter().any(|value| {
            matches!(
                value.as_str(),
                "chat" | "completion" | "chat_completion" | "text_generation"
            )
        }) {
            return true;
        }

        let id = id.to_ascii_lowercase();
        !id.split(|c: char| !c.is_ascii_alphanumeric())
            .any(|part| NON_CHAT_MARKERS.contains(&part))
    }

    fn parse_available_models(
        &self,
        json: &serde_json::Value,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let enabled_set: std::collections::HashSet<&str> =
            self.enabled_models.iter().map(|id| id.as_str()).collect();
        let mut models = Vec::new();

        for model in json
            .get("data")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
        {
            let Some(id) = model.get("id").and_then(|value| value.as_str()) else {
                continue;
            };
            if !Self::is_chat_model(model, id) {
                continue;
            }
            let resolved = resolve_model_caps(model_caps, &format!("groq/{id}"))
                .or_else(|| resolve_model_caps(model_caps, id));
            models.push(available_model_from_catalog_and_live(
                id,
                resolved.as_ref().map(|resolved| &resolved.caps),
                &LiveModelFields::default(),
                enabled_set.contains(id),
                8_192,
            ));
        }

        merge_custom_models(&mut models, &self.custom_models, &enabled_set);
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models.dedup_by(|a, b| a.id == b.id);
        models
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn groq_uses_api_source_and_server_ids_for_catalog_enriched_discovery() {
        let provider = GroqProvider::default();
        assert!(matches!(provider.model_source(), ModelSource::Api));

        let mut caps = HashMap::new();
        caps.insert(
            "groq/server-model".to_string(),
            ModelCapabilities {
                n_ctx: 131_072,
                supports_tools: true,
                ..Default::default()
            },
        );
        caps.insert(
            "groq/catalog-only".to_string(),
            ModelCapabilities {
                n_ctx: 1,
                ..Default::default()
            },
        );

        let models = provider.parse_available_models(
            &json!({"object": "list", "data": [{"id": "server-model"}, {"id": "unknown-model", "owned_by": "Groq"}]}),
            &caps,
        );

        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["server-model", "unknown-model"]
        );
        assert_eq!(models[0].n_ctx, 131_072);
        assert!(models[0].supports_tools);
        assert_eq!(models[1].n_ctx, 8_192);
        assert!(!models[1].supports_tools);
    }

    #[test]
    fn groq_discovery_url_is_openai_models_endpoint() {
        assert_eq!(GROQ_MODELS_URL, "https://api.groq.com/openai/v1/models");
    }

    #[test]
    fn groq_excludes_non_chat_models_and_preserves_llm_ids() {
        let provider = GroqProvider::default();
        let models = provider.parse_available_models(
            &json!({"data": [
                {"id": "whisper-large-v3"},
                {"id": "playai-tts"},
                {"id": "nomic-embed-text"},
                {"id": "model-with-opaque-id", "mode": "audio_transcription"},
                {"id": "llama-3.3-70b-versatile"},
                {"id": "compound-beta"},
                {"id": "speechless-llama"},
                {"id": "audio-named-chat-model", "type": "chat"}
            ]}),
            &HashMap::new(),
        );

        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "audio-named-chat-model",
                "compound-beta",
                "llama-3.3-70b-versatile",
                "speechless-llama"
            ]
        );
    }
}
