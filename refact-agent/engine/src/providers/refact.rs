use std::any::Any;
use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::caps::model_caps::{ModelCapabilities, resolve_model_caps};
use crate::llm::adapter::WireFormat;
use crate::providers::config::resolve_env_var;
use crate::providers::traits::{
    AvailableModel, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
};

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
    fn config_path(config_dir: &std::path::Path) -> std::path::PathBuf {
        config_dir.join("providers.d").join("refact.yaml")
    }

    async fn save_config(&self, config_dir: &std::path::Path) -> Result<(), String> {
        let providers_dir = config_dir.join("providers.d");
        tokio::fs::create_dir_all(&providers_dir)
            .await
            .map_err(|e| format!("Failed to create providers.d: {}", e))?;

        let config_path = Self::config_path(config_dir);
        let payload = serde_yaml::to_string(
            &serde_yaml::to_value(json!({
                "enabled": self.enabled,
                "disabled_models": self.disabled_models,
                "running_models": self.running_models,
            }))
            .map_err(|e| format!("Failed to serialize refact provider settings: {}", e))?,
        )
        .map_err(|e| format!("Failed to render refact provider yaml: {}", e))?;

        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let temp_path = config_path.with_extension(format!(
            "yaml.tmp.{}.{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));

        tokio::fs::write(&temp_path, payload)
            .await
            .map_err(|e| format!("Failed to write temporary refact config: {}", e))?;
        tokio::fs::rename(&temp_path, &config_path)
            .await
            .map_err(|e| format!("Failed to finalize refact config: {}", e))?;
        Ok(())
    }

    pub fn from_cli(address_url: String, api_key: String) -> Self {
        Self {
            address_url,
            api_key,
            enabled: true,
            disabled_models: Vec::new(),
            running_models: Vec::new(),
        }
    }

    fn base_url(&self) -> String {
        if self.address_url.is_empty() || self.address_url.to_lowercase() == "refact" {
            "https://inference.smallcloud.ai".to_string()
        } else {
            self.address_url.trim_end_matches('/').to_string()
        }
    }

    fn model_catalog_url(&self) -> String {
        format!("{}/v1/model-catalog", self.base_url())
    }

    fn parse_model_pricing_from_json(value: &serde_json::Value) -> Option<ModelPricing> {
        let prompt = value.get("prompt").and_then(|v| v.as_f64())?;
        let generated = value.get("generated").and_then(|v| v.as_f64())?;
        let pricing = ModelPricing {
            prompt,
            generated,
            cache_read: value.get("cache_read").and_then(|v| v.as_f64()),
            cache_creation: value.get("cache_creation").and_then(|v| v.as_f64()),
        };
        if pricing.is_valid() {
            Some(pricing)
        } else {
            None
        }
    }

    fn model_is_disabled(&self, model_id: &str) -> bool {
        self.disabled_models.contains(&model_id.to_string())
            || self
                .disabled_models
                .contains(&format!("refact/{}", model_id))
    }

    pub fn extract_chat_model_ids_from_catalog(catalog: &serde_json::Value) -> Vec<String> {
        let mut ids: Vec<String> = catalog
            .get("chat")
            .and_then(|v| v.get("models"))
            .and_then(|v| v.as_object())
            .map(|models| models.keys().cloned().collect())
            .unwrap_or_default();
        ids.sort();
        ids
    }

    pub async fn fetch_model_catalog(
        &self,
        http_client: &reqwest::Client,
    ) -> Result<serde_json::Value, String> {
        let mut request = http_client.get(self.model_catalog_url()).header(
            reqwest::header::USER_AGENT,
            format!("refact-lsp {}", crate::version::build::PKG_VERSION),
        );

        let api_key = resolve_env_var(&self.api_key, "", "refact api_key");
        if !api_key.is_empty() {
            request = request.bearer_auth(api_key);
        }

        let response = request
            .send()
            .await
            .map_err(|e| format!("Failed to fetch Refact model catalog: {}", e))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_else(|_| String::new());
            return Err(format!(
                "Refact model catalog fetch failed: HTTP {} {}",
                status, body
            ));
        }

        let payload: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Invalid Refact model catalog JSON: {}", e))?;

        let cloud_name = payload
            .get("cloud_name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_lowercase();
        if cloud_name != "refact" {
            return Err("Model catalog response is not a Refact catalog".to_string());
        }

        Ok(payload)
    }

    pub async fn sync_running_models_from_catalog(
        &mut self,
        http_client: &reqwest::Client,
    ) -> Result<(), String> {
        let catalog = self.fetch_model_catalog(http_client).await?;
        let catalog_models = Self::extract_chat_model_ids_from_catalog(&catalog);

        let mut disabled: std::collections::HashSet<String> =
            self.disabled_models.iter().cloned().collect();
        disabled.retain(|m| {
            let bare = m.strip_prefix("refact/").unwrap_or(m);
            catalog_models.iter().any(|x| x == bare)
        });

        self.running_models = catalog_models;
        self.disabled_models = disabled.into_iter().collect();
        self.disabled_models.sort();
        Ok(())
    }

    fn extract_available_models_from_catalog(
        &self,
        catalog: &serde_json::Value,
    ) -> Result<Vec<AvailableModel>, String> {
        let chat_models = catalog
            .get("chat")
            .and_then(|v| v.get("models"))
            .and_then(|v| v.as_object())
            .ok_or_else(|| "Model catalog response missing chat.models".to_string())?;

        let pricing_map = catalog
            .get("metadata")
            .and_then(|v| v.get("pricing"))
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();

        let tokenizer_endpoints = catalog
            .get("tokenizer_endpoints")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();

        let mut models: Vec<AvailableModel> = Vec::new();
        for (model_id, model_info) in chat_models {
            let n_ctx = model_info
                .get("n_ctx")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .unwrap_or(4096);
            let supports_tools = model_info
                .get("supports_tools")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let supports_multimodality = model_info
                .get("supports_multimodality")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let max_output_tokens = model_info
                .get("max_output_tokens")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);

            let tokenizer = tokenizer_endpoints
                .get(model_id)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let pricing = pricing_map
                .get(model_id)
                .and_then(Self::parse_model_pricing_from_json);

            models.push(AvailableModel {
                id: model_id.clone(),
                display_name: None,
                n_ctx,
                supports_tools,
                supports_parallel_tools: supports_tools,
                supports_strict_tools: false,
                supports_multimodality,
                reasoning_effort_options: None,
                supports_thinking_budget: false,
                supports_adaptive_thinking_budget: false,
                tokenizer,
                enabled: !self.model_is_disabled(model_id),
                is_custom: false,
                pricing,
                available_providers: Vec::new(),
                selected_provider: None,
                max_output_tokens,
                provider_variants: Vec::new(),
            });
        }

        models.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(models)
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
        if let Some(models) = yaml.get("running_models").and_then(|v| v.as_sequence()) {
            self.running_models = models
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            self.running_models.sort();
            self.running_models.dedup();
        }
        Ok(())
    }

    fn provider_settings_as_json(&self) -> serde_json::Value {
        json!({
            "address_url": self.address_url,
            "api_key": if self.api_key.is_empty() { "" } else { "***" },
            "enabled": self.enabled,
            "disabled_models": self.disabled_models,
            "running_models": self.running_models
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        let api_key = resolve_env_var(&self.api_key, "", "refact api_key");
        let base_url = self.base_url();

        Ok(ProviderRuntime {
            name: self.name().to_string(),
            display_name: self.display_name().to_string(),
            enabled: self.enabled && !api_key.is_empty(),
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
            supports_cache_control: true,
            chat_models: Vec::new(),
            completion_models: Vec::new(),
            embedding_model: None,
        })
    }

    fn is_readonly(&self) -> bool {
        false
    }

    fn has_credentials(&self) -> bool {
        let resolved = resolve_env_var(&self.api_key, "", "refact api_key");
        !resolved.is_empty()
    }

    fn model_source(&self) -> ModelSource {
        ModelSource::Api
    }

    fn selected_model_count(&self) -> usize {
        if self.running_models.is_empty() {
            return 0;
        }
        self.running_models
            .iter()
            .filter(|m| !self.model_is_disabled(m))
            .count()
    }

    fn disabled_models(&self) -> &[String] {
        &self.disabled_models
    }

    fn set_model_enabled(&mut self, model_id: &str, enabled: bool) {
        crate::providers::traits::set_model_disabled_impl(
            &mut self.disabled_models,
            model_id,
            enabled,
        );
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
                let disabled = self.model_is_disabled(running_model);
                let pricing = self.model_pricing(running_model);
                let mut model =
                    AvailableModel::from_caps(running_model, &resolved.caps, !disabled, pricing);
                if running_model != &resolved.matched_key {
                    model.display_name = Some(running_model.clone());
                }
                models.push(model);
            } else {
                tracing::warn!(
                    "Refact running model '{}' not found in model capabilities, adding with defaults",
                    running_model
                );
                let disabled = self.model_is_disabled(running_model);
                models.push(AvailableModel {
                    id: running_model.clone(),
                    display_name: None,
                    n_ctx: 4096,
                    supports_tools: false,
                    supports_parallel_tools: false,
                    supports_strict_tools: false,
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

    async fn fetch_available_models(
        &self,
        http_client: &reqwest::Client,
        _model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        match self.fetch_model_catalog(http_client).await {
            Ok(catalog) => match self.extract_available_models_from_catalog(&catalog) {
                Ok(models) => models,
                Err(e) => {
                    tracing::warn!("Refact model catalog parse failed: {}", e);
                    Vec::new()
                }
            },
            Err(e) => {
                tracing::warn!("Refact model catalog fetch failed: {}", e);
                Vec::new()
            }
        }
    }

    async fn startup_refresh_and_sync(
        &mut self,
        http_client: &reqwest::Client,
        config_dir: &std::path::Path,
    ) -> Result<(), String> {
        self.sync_running_models_from_catalog(http_client).await?;
        self.save_config(config_dir).await
    }
}
