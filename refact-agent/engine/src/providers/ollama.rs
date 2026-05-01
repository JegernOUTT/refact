use std::any::Any;
use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::caps::model_caps::ModelCapabilities;
use crate::llm::adapter::WireFormat;
use crate::providers::traits::{
    derive_endpoint_from_chat_url, merge_custom_models, normalize_endpoint, parse_custom_models,
    parse_enabled_models, set_model_enabled_impl, AvailableModel, CustomModelConfig, ModelPricing,
    ModelSource, ProviderRuntime, ProviderTrait,
};

const DEFAULT_OLLAMA_N_CTX: usize = 32_768;

#[derive(Debug, Clone, Default)]
struct OllamaModelMetadata {
    n_ctx: Option<usize>,
    supports_tools: Option<bool>,
    supports_multimodality: Option<bool>,
    family: Option<String>,
    families: Vec<String>,
    parameter_size: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaProvider {
    pub endpoint: String,
    pub api_key: String,
    pub enabled: bool,
    #[serde(default)]
    pub supports_cache_control: bool,
    #[serde(default)]
    pub num_ctx: Option<usize>,
    #[serde(default)]
    pub keep_alive: Option<String>,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
}

impl Default for OllamaProvider {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:11434".to_string(),
            api_key: String::new(),
            enabled: false,
            supports_cache_control: false,
            num_ctx: None,
            keep_alive: None,
            enabled_models: Vec::new(),
            custom_models: HashMap::new(),
        }
    }
}

impl OllamaProvider {
    fn parse_ollama_model(
        model: &Value,
        show_model: Option<&Value>,
        enabled: bool,
    ) -> Option<AvailableModel> {
        let name = Self::model_name(model)?.to_string();
        let tags_metadata = Self::parse_model_metadata(model);
        let show_metadata = show_model.map(Self::parse_model_metadata);

        let n_ctx = show_metadata
            .as_ref()
            .and_then(|m| m.n_ctx)
            .or(tags_metadata.n_ctx)
            .unwrap_or(DEFAULT_OLLAMA_N_CTX);
        let family = show_metadata
            .as_ref()
            .and_then(|m| m.family.as_deref())
            .or(tags_metadata.family.as_deref())
            .unwrap_or("");
        let families = show_metadata
            .as_ref()
            .filter(|m| !m.families.is_empty())
            .map(|m| m.families.as_slice())
            .unwrap_or(tags_metadata.families.as_slice());
        let parameter_size = show_metadata
            .as_ref()
            .and_then(|m| m.parameter_size.as_deref())
            .or(tags_metadata.parameter_size.as_deref())
            .unwrap_or("");

        let supports_tools = show_metadata
            .as_ref()
            .and_then(|m| m.supports_tools)
            .or(tags_metadata.supports_tools)
            .unwrap_or_else(|| Self::family_supports_tools(family, families));
        let supports_multimodality = show_metadata
            .as_ref()
            .and_then(|m| m.supports_multimodality)
            .or(tags_metadata.supports_multimodality)
            .unwrap_or_else(|| Self::family_supports_vision(family, families));

        let display_name = if parameter_size.is_empty() {
            None
        } else {
            Some(format!("{} ({})", name, parameter_size))
        };

        Some(AvailableModel {
            id: name,
            display_name,
            n_ctx,
            supports_tools,
            supports_parallel_tools: supports_tools,
            supports_strict_tools: false,
            supports_multimodality,
            reasoning_effort_options: None,
            supports_thinking_budget: false,
            supports_adaptive_thinking_budget: false,
            supports_cache_control: false,
            tokenizer: None,
            enabled,
            is_custom: false,
            pricing: None,
            available_providers: Vec::new(),
            selected_provider: None,
            max_output_tokens: None,
            provider_variants: Vec::new(),
            wire_format_override: None,
            endpoint_override: None,
            base_model: None,
        })
    }

    fn model_name(model: &Value) -> Option<&str> {
        model
            .get("name")
            .or_else(|| model.get("model"))
            .and_then(|v| v.as_str())
    }

    fn parse_model_metadata(model: &Value) -> OllamaModelMetadata {
        let details = model.get("details");
        let (supports_tools, supports_multimodality) = Self::parse_capabilities(model);

        OllamaModelMetadata {
            n_ctx: Self::parse_model_context(model),
            supports_tools,
            supports_multimodality,
            family: details
                .and_then(|d| d.get("family"))
                .and_then(|v| v.as_str())
                .filter(|v| !v.is_empty())
                .map(ToString::to_string),
            families: details
                .and_then(|d| d.get("families"))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .filter(|v| !v.is_empty())
                        .map(ToString::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            parameter_size: details
                .and_then(|d| d.get("parameter_size"))
                .and_then(|v| v.as_str())
                .filter(|v| !v.is_empty())
                .map(ToString::to_string),
        }
    }

    fn parse_capabilities(model: &Value) -> (Option<bool>, Option<bool>) {
        let capabilities = match model.get("capabilities").and_then(|v| v.as_array()) {
            Some(capabilities) => capabilities,
            None => return (None, None),
        };

        let supports_tools = capabilities
            .iter()
            .any(|c| Self::capability_matches(c, &["tools"]));
        let supports_multimodality = capabilities
            .iter()
            .any(|c| Self::capability_matches(c, &["vision", "image"]));

        (Some(supports_tools), Some(supports_multimodality))
    }

    fn capability_matches(capability: &Value, names: &[&str]) -> bool {
        matches!(
            capability.as_str(),
            Some(capability) if names.iter().any(|name| capability.eq_ignore_ascii_case(name))
        )
    }

    fn parse_model_context(model: &Value) -> Option<usize> {
        Self::parse_model_info_context(model)
            .or_else(|| {
                model
                    .get("context_length")
                    .and_then(Self::parse_usize_value)
            })
            .or_else(|| {
                model
                    .get("details")
                    .and_then(|d| d.get("context_length"))
                    .and_then(Self::parse_usize_value)
            })
            .or_else(|| {
                model
                    .get("parameters")
                    .and_then(|v| v.as_str())
                    .and_then(Self::parse_num_ctx_parameter)
            })
    }

    fn parse_model_info_context(model: &Value) -> Option<usize> {
        model
            .get("model_info")
            .and_then(|v| v.as_object())
            .and_then(|model_info| {
                model_info.iter().find_map(|(key, value)| {
                    if key.to_ascii_lowercase().contains("context_length") {
                        Self::parse_usize_value(value)
                    } else {
                        None
                    }
                })
            })
    }

    fn parse_num_ctx_parameter(parameters: &str) -> Option<usize> {
        parameters.lines().find_map(|line| {
            let mut parts = line.split_whitespace();
            let key = parts.next()?;
            if key != "num_ctx" {
                return None;
            }
            let value = parts.next()?.trim_matches('"');
            value.parse::<usize>().ok().filter(|v| *v > 0)
        })
    }

    fn parse_usize_value(value: &Value) -> Option<usize> {
        if let Some(value) = value.as_u64() {
            return usize::try_from(value).ok().filter(|v| *v > 0);
        }
        value
            .as_str()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 0)
    }

    fn family_supports_tools(family: &str, families: &[String]) -> bool {
        let tool_families = [
            "llama",
            "qwen2",
            "qwen3",
            "command-r",
            "mistral",
            "gemma2",
            "gemma3",
            "phi3",
            "phi4",
            "deepseek2",
        ];
        let check = |f: &str| tool_families.iter().any(|tf| f.contains(tf));
        check(family) || families.iter().any(|f| check(f))
    }

    fn family_supports_vision(family: &str, families: &[String]) -> bool {
        let vision_families = ["llava", "gemma3", "phi4"];
        let check = |f: &str| vision_families.iter().any(|vf| f.contains(vf));
        check(family) || families.iter().any(|f| check(f))
    }

    fn apply_auth(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.api_key.is_empty() {
            request
        } else {
            request.header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.api_key),
            )
        }
    }

    async fn fetch_ollama_show_model(
        &self,
        http_client: &reqwest::Client,
        show_url: &str,
        model_name: &str,
    ) -> Option<Value> {
        if let Ok(model) = self
            .fetch_ollama_show_model_body(http_client, show_url, json!({ "model": model_name }))
            .await
        {
            return Some(model);
        }

        match self
            .fetch_ollama_show_model_body(http_client, show_url, json!({ "name": model_name }))
            .await
        {
            Ok(model) => Some(model),
            Err(e) => {
                tracing::warn!("Ollama: /api/show failed for {}: {}", model_name, e);
                None
            }
        }
    }

    async fn fetch_ollama_show_model_body(
        &self,
        http_client: &reqwest::Client,
        show_url: &str,
        body: Value,
    ) -> Result<Value, String> {
        let request = self.apply_auth(
            http_client
                .post(show_url)
                .timeout(Duration::from_secs(5))
                .json(&body),
        );
        let response = request.send().await.map_err(|e| e.to_string())?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("status {}", status));
        }
        response.json().await.map_err(|e| e.to_string())
    }
}

#[async_trait]
impl ProviderTrait for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    fn display_name(&self) -> &str {
        "Ollama"
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
        WireFormat::OllamaNative
    }

    fn model_filter_regex(&self) -> Option<&'static str> {
        None
    }

    fn provider_schema(&self) -> &'static str {
        r#"
fields:
  endpoint:
    f_type: string_long
    f_desc: "Ollama server endpoint"
    f_placeholder: "http://localhost:11434"
    f_label: "Endpoint"
    f_default: "http://localhost:11434"
  api_key:
    f_type: string_long
    f_desc: "API key (optional, for reverse proxy auth)"
    f_placeholder: ""
    f_label: "API Key"
    f_default: ""
  supports_cache_control:
    f_type: boolean
    f_desc: "Send Anthropic-style cache-control fields to the Ollama server"
    f_label: "Enable Cache Control"
    f_default: false
    f_extra: true
  num_ctx:
    f_type: string
    f_desc: "Optional Ollama num_ctx runtime option"
    f_label: "Context Window Override"
    f_default: ""
    f_extra: true
  keep_alive:
    f_type: string
    f_desc: "Optional Ollama keep_alive value, for example 5m or -1"
    f_label: "Keep Alive"
    f_default: ""
    f_extra: true
description: |
  Local Ollama server for running open-source models.
available:
  on_your_laptop_possible: true
  when_isolated_possible: true
"#
    }

    fn provider_settings_apply(&mut self, yaml: serde_yaml::Value) -> Result<(), String> {
        if let Some(endpoint) = yaml.get("endpoint").and_then(|v| v.as_str()) {
            self.endpoint = normalize_endpoint(endpoint);
        } else if let Some(chat_ep) = yaml.get("chat_endpoint").and_then(|v| v.as_str()) {
            if let Some(derived) = derive_endpoint_from_chat_url(chat_ep) {
                self.endpoint = derived;
            }
        }
        if let Some(api_key) = yaml.get("api_key").and_then(|v| v.as_str()) {
            if api_key != "***" && api_key != "any-will-work" {
                self.api_key = api_key.to_string();
            }
        }
        if let Some(enabled) = yaml.get("enabled").and_then(|v| v.as_bool()) {
            self.enabled = enabled;
        }
        if let Some(supports_cache_control) =
            yaml.get("supports_cache_control").and_then(|v| v.as_bool())
        {
            self.supports_cache_control = supports_cache_control;
        }
        if let Some(num_ctx) = yaml
            .get("num_ctx")
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .and_then(|v| usize::try_from(v).ok())
            .filter(|v| *v > 0)
        {
            self.num_ctx = Some(num_ctx);
        } else if yaml.get("num_ctx").is_some() {
            self.num_ctx = None;
        }
        if let Some(keep_alive) = yaml
            .get("keep_alive")
            .and_then(|v| v.as_str())
            .map(str::trim)
        {
            self.keep_alive = (!keep_alive.is_empty()).then(|| keep_alive.to_string());
        }
        parse_enabled_models(&yaml, &mut self.enabled_models);
        parse_custom_models(&yaml, &mut self.custom_models);
        Ok(())
    }

    fn provider_settings_as_json(&self) -> serde_json::Value {
        json!({
            "endpoint": self.endpoint,
            "api_key": if self.api_key.is_empty() { "" } else { "***" },
            "enabled": self.enabled,
            "supports_cache_control": self.supports_cache_control,
            "num_ctx": self.num_ctx,
            "keep_alive": self.keep_alive,
            "enabled_models": self.enabled_models,
            "custom_models": self.custom_models
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let base_url = normalize_endpoint(&self.endpoint);
        let mut extra_headers = HashMap::new();
        if let Some(num_ctx) = self.num_ctx {
            extra_headers.insert(
                crate::llm::adapters::ollama::OLLAMA_NUM_CTX_HEADER.to_string(),
                num_ctx.to_string(),
            );
        }
        if let Some(keep_alive) = self.keep_alive.as_deref().filter(|v| !v.trim().is_empty()) {
            extra_headers.insert(
                crate::llm::adapters::ollama::OLLAMA_KEEP_ALIVE_HEADER.to_string(),
                keep_alive.trim().to_string(),
            );
        }

        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: self.enabled && !self.endpoint.is_empty() && !self.enabled_models.is_empty(),
            readonly: false,
            wire_format: self.default_wire_format(),
            chat_endpoint: format!("{}/api/chat", base_url),
            completion_endpoint: format!("{}/v1/completions", base_url),
            embedding_endpoint: format!("{}/v1/embeddings", base_url),
            api_key: self.api_key.clone(),
            auth_token: String::new(),
            tokenizer_api_key: String::new(),
            extra_headers,
            supports_cache_control: self.supports_cache_control,
            chat_models: Vec::new(),
            completion_models: Vec::new(),
            embedding_model: None,
        })
    }

    fn has_credentials(&self) -> bool {
        !self.endpoint.is_empty()
    }

    fn model_source(&self) -> ModelSource {
        ModelSource::Api
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
            .and_then(|c| c.pricing.clone())
    }

    async fn fetch_available_models(
        &self,
        http_client: &reqwest::Client,
        _model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let base_url = normalize_endpoint(&self.endpoint);
        let tags_url = format!("{}/api/tags", base_url);

        let request = self.apply_auth(http_client.get(&tags_url).timeout(Duration::from_secs(5)));

        let response = match request.send().await {
            Ok(resp) => resp,
            Err(e) => {
                tracing::warn!("Ollama: server not reachable at {}: {}", tags_url, e);
                return self.get_custom_models_only();
            }
        };

        if !response.status().is_success() {
            tracing::warn!("Ollama: /api/tags returned status {}", response.status());
            return self.get_custom_models_only();
        }

        let json: Value = match response.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("Ollama: failed to parse /api/tags response: {}", e);
                return self.get_custom_models_only();
            }
        };

        let enabled_set: std::collections::HashSet<&str> =
            self.enabled_models.iter().map(|s| s.as_str()).collect();

        let tag_models = json
            .get("models")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let show_url = format!("{}/api/show", base_url);
        let mut models = Vec::new();

        for tag_model in tag_models {
            let name = match Self::model_name(&tag_model) {
                Some(name) => name.to_string(),
                None => continue,
            };
            let enabled = enabled_set.contains(name.as_str());
            let show_model = self
                .fetch_ollama_show_model(http_client, &show_url, &name)
                .await;
            if let Some(model) = Self::parse_ollama_model(&tag_model, show_model.as_ref(), enabled)
            {
                models.push(model);
            }
        }

        merge_custom_models(&mut models, &self.custom_models, &enabled_set);
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ollama_runtime_disables_cache_control_by_default() {
        let runtime = OllamaProvider::default().build_runtime().unwrap();

        assert!(!runtime.supports_cache_control);
    }

    #[test]
    fn ollama_runtime_can_enable_cache_control() {
        let mut provider = OllamaProvider::default();
        provider
            .provider_settings_apply(serde_yaml::from_str("supports_cache_control: true").unwrap())
            .unwrap();
        let runtime = provider.build_runtime().unwrap();

        assert!(runtime.supports_cache_control);
    }

    #[test]
    fn ollama_runtime_uses_native_chat_endpoint_and_wire_format() {
        let mut provider = OllamaProvider::default();
        provider.endpoint = "http://localhost:11434/".to_string();
        provider.api_key = "secret".to_string();
        provider.enabled = true;
        provider.enabled_models = vec!["llama3.1:8b".to_string()];

        let runtime = provider.build_runtime().unwrap();

        assert!(runtime.enabled);
        assert_eq!(runtime.wire_format, WireFormat::OllamaNative);
        assert_eq!(runtime.chat_endpoint, "http://localhost:11434/api/chat");
        assert_eq!(
            runtime.completion_endpoint,
            "http://localhost:11434/v1/completions"
        );
        assert_eq!(
            runtime.embedding_endpoint,
            "http://localhost:11434/v1/embeddings"
        );
        assert_eq!(runtime.api_key, "secret");
    }

    #[test]
    fn ollama_runtime_stores_options_as_internal_headers() {
        let mut provider = OllamaProvider::default();
        provider
            .provider_settings_apply(
                serde_yaml::from_str("num_ctx: 65536\nkeep_alive: 30m\n").unwrap(),
            )
            .unwrap();

        let runtime = provider.build_runtime().unwrap();

        assert_eq!(
            runtime
                .extra_headers
                .get(crate::llm::adapters::ollama::OLLAMA_NUM_CTX_HEADER)
                .map(String::as_str),
            Some("65536")
        );
        assert_eq!(
            runtime
                .extra_headers
                .get(crate::llm::adapters::ollama::OLLAMA_KEEP_ALIVE_HEADER)
                .map(String::as_str),
            Some("30m")
        );
        let settings = provider.provider_settings_as_json();
        assert_eq!(settings["num_ctx"], 65536);
        assert_eq!(settings["keep_alive"], "30m");
    }

    #[test]
    fn ollama_default_wire_format_is_native() {
        assert_eq!(
            OllamaProvider::default().default_wire_format(),
            WireFormat::OllamaNative
        );
    }

    #[test]
    fn ollama_show_model_info_context_length_sets_n_ctx() {
        let tags = json!({
            "name": "llama3.1:8b",
            "details": {
                "family": "llama",
                "parameter_size": "8B"
            }
        });
        let show = json!({
            "model_info": {
                "llama.context_length": 131072
            },
            "details": {
                "family": "llama",
                "parameter_size": "8B"
            }
        });

        let model = OllamaProvider::parse_ollama_model(&tags, Some(&show), false).unwrap();

        assert_eq!(model.n_ctx, 131_072);
    }

    #[test]
    fn ollama_show_parameters_num_ctx_sets_n_ctx() {
        let tags = json!({
            "name": "mistral:7b",
            "details": {
                "family": "mistral",
                "context_length": 4096
            }
        });
        let show = json!({
            "parameters": "num_ctx 65536\nstop \"</s>\"",
            "details": {
                "family": "mistral",
                "parameter_size": "7B"
            }
        });

        let model = OllamaProvider::parse_ollama_model(&tags, Some(&show), false).unwrap();

        assert_eq!(model.n_ctx, 65_536);
    }

    #[test]
    fn ollama_show_capabilities_set_tools_and_vision() {
        let tags = json!({
            "name": "llava:latest",
            "details": {
                "family": "unknown"
            }
        });
        let show = json!({
            "capabilities": ["tools", "vision"],
            "details": {
                "family": "unknown"
            }
        });

        let model = OllamaProvider::parse_ollama_model(&tags, Some(&show), false).unwrap();

        assert!(model.supports_tools);
        assert!(model.supports_parallel_tools);
        assert!(model.supports_multimodality);
    }

    #[test]
    fn ollama_show_empty_capabilities_disable_family_heuristic() {
        let tags = json!({
            "name": "gemma3:4b",
            "details": {
                "family": "gemma3",
                "families": ["gemma3"]
            }
        });
        let show = json!({
            "capabilities": [],
            "details": {
                "family": "gemma3",
                "families": ["gemma3"]
            }
        });

        let model = OllamaProvider::parse_ollama_model(&tags, Some(&show), false).unwrap();

        assert!(!model.supports_tools);
        assert!(!model.supports_parallel_tools);
        assert!(!model.supports_multimodality);
    }

    #[test]
    fn ollama_tags_only_fixture_uses_fallback_behavior() {
        let tags = json!({
            "name": "llama3.1:8b",
            "details": {
                "family": "llama",
                "families": ["llama"],
                "parameter_size": "8B",
                "context_length": 4096
            }
        });

        let model = OllamaProvider::parse_ollama_model(&tags, None, true).unwrap();

        assert_eq!(model.id, "llama3.1:8b");
        assert_eq!(model.display_name.as_deref(), Some("llama3.1:8b (8B)"));
        assert_eq!(model.n_ctx, 4096);
        assert!(model.supports_tools);
        assert!(model.supports_parallel_tools);
        assert!(!model.supports_multimodality);
        assert!(model.enabled);
    }
}
