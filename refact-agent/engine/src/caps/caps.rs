use std::collections::HashMap;
use std::sync::Arc;

use indexmap::IndexMap;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::RwLock as ARwLock;
use url::Url;
use tracing::{info, warn};

use crate::custom_error::MapErrToString;
use crate::global_context::CommandLine;
use crate::global_context::GlobalContext;
use crate::caps::providers::{
    add_models_to_caps, read_providers_d, resolve_provider_api_key, post_process_provider,
    CapsProvider,
};
use crate::providers::config::ProviderDefaults;
use crate::caps::model_caps::{ModelCapabilities, get_model_caps, resolve_model_caps};
use crate::llm::WireFormat;
use crate::providers::traits::AvailableModel;

pub const CAPS_FILENAME: &str = "refact-caps";
pub const CAPS_FILENAME_FALLBACK: &str = "coding_assistant_caps.json";

#[derive(Debug, Serialize, Clone, Deserialize, Default, PartialEq)]
pub struct BaseModelRecord {
    #[serde(default)]
    pub n_ctx: usize,

    /// Actual model name, e.g. "gpt-4o"
    #[serde(default)]
    pub name: String,
    /// provider/model_name, e.g. "openai/gpt-4o"
    #[serde(skip_deserializing)]
    pub id: String,

    #[serde(default, skip_serializing)]
    pub endpoint: String,
    #[serde(default, skip_serializing)]
    pub endpoint_style: String,
    #[serde(default, skip_serializing)]
    pub wire_format: WireFormat,
    #[serde(default, skip_serializing)]
    pub api_key: String,
    #[serde(default, skip_serializing)]
    pub auth_token: String,
    #[serde(default, skip_serializing)]
    pub tokenizer_api_key: String,

    #[serde(default, skip_serializing)]
    pub support_metadata: bool,
    #[serde(default, skip_serializing)]
    pub extra_headers: std::collections::HashMap<String, String>,
    #[serde(default, skip_serializing)]
    pub similar_models: Vec<String>,
    #[serde(default)]
    pub tokenizer: String,

    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub experimental: bool,

    /// Use max_completion_tokens instead of max_tokens (required for OpenAI o1/o3 models)
    #[serde(default)]
    pub supports_max_completion_tokens: bool,

    /// Treat stream EOF as completion (for endpoints that don't send explicit Done signal)
    #[serde(default)]
    pub eof_is_done: bool,

    /// Enable Anthropic's server-side web_search tool
    #[serde(default)]
    pub supports_web_search: bool,

    // Fields used for Config/UI management
    #[serde(skip_deserializing)]
    pub removable: bool,
    #[serde(skip_deserializing)]
    pub user_configured: bool,
}

fn default_true() -> bool {
    true
}

pub trait HasBaseModelRecord {
    fn base(&self) -> &BaseModelRecord;
    fn base_mut(&mut self) -> &mut BaseModelRecord;
}

#[derive(Debug, Serialize, Clone, Deserialize, Default)]
pub struct ChatModelRecord {
    #[serde(flatten)]
    pub base: BaseModelRecord,

    #[allow(dead_code)] // Deserialized from API but not used internally
    #[serde(default = "default_chat_scratchpad", skip_serializing)]
    pub scratchpad: String,
    #[allow(dead_code)] // Deserialized from API but not used internally
    #[serde(default, skip_serializing)]
    pub scratchpad_patch: serde_json::Value,

    #[serde(default)]
    pub supports_tools: bool,
    #[serde(default)]
    pub supports_multimodality: bool,
    #[serde(default)]
    pub supports_clicks: bool,
    #[serde(default)]
    pub supports_agent: bool,
    #[serde(default)]
    pub reasoning_effort_options: Option<Vec<String>>,
    #[serde(default)]
    pub supports_thinking_budget: bool,
    #[serde(default)]
    pub supports_adaptive_thinking_budget: bool,
    #[serde(default)]
    pub max_thinking_tokens: Option<usize>,
    #[serde(default)]
    pub default_temperature: Option<f32>,
    #[serde(default)]
    pub default_frequency_penalty: Option<f32>,
    #[serde(default)]
    pub default_max_tokens: Option<usize>,
    #[serde(default)]
    pub max_output_tokens: Option<usize>,
    #[serde(default)]
    pub supports_strict_tools: bool,
    #[serde(default = "default_true")]
    pub supports_temperature: bool,
}

pub fn default_chat_scratchpad() -> String {
    String::new()
}

impl ChatModelRecord {
    pub fn has_reasoning_support(&self) -> bool {
        self.reasoning_effort_options.is_some()
            || self.supports_thinking_budget
            || self.supports_adaptive_thinking_budget
    }

    pub fn reasoning_type_string(&self) -> Option<String> {
        if self.supports_adaptive_thinking_budget {
            Some("anthropic_effort".to_string())
        } else if self.supports_thinking_budget {
            Some("anthropic_budget".to_string())
        } else {
            None
        }
    }
}

impl HasBaseModelRecord for ChatModelRecord {
    fn base(&self) -> &BaseModelRecord {
        &self.base
    }
    fn base_mut(&mut self) -> &mut BaseModelRecord {
        &mut self.base
    }
}

#[derive(Debug, Serialize, Clone, Deserialize, Default)]
pub struct CompletionModelRecord {
    #[serde(flatten)]
    pub base: BaseModelRecord,

    #[serde(default = "default_completion_scratchpad")]
    pub scratchpad: String,
    #[serde(default = "default_completion_scratchpad_patch")]
    pub scratchpad_patch: serde_json::Value,

    pub model_family: Option<CompletionModelFamily>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionModelFamily {
    #[serde(rename = "qwen2.5-coder-base")]
    Qwen2_5CoderBase,
    #[serde(rename = "starcoder")]
    Starcoder,
    #[serde(rename = "deepseek-coder")]
    DeepseekCoder,
}

pub fn default_completion_scratchpad() -> String {
    "FIM-PSM".to_string()
}

pub fn default_completion_scratchpad_patch() -> serde_json::Value {
    serde_json::json!({
        "context_format": "chat",
        "rag_ratio": 0.5
    })
}

impl HasBaseModelRecord for CompletionModelRecord {
    fn base(&self) -> &BaseModelRecord {
        &self.base
    }
    fn base_mut(&mut self) -> &mut BaseModelRecord {
        &mut self.base
    }
}

#[derive(Debug, Serialize, Clone, Default, PartialEq)]
pub struct EmbeddingModelRecord {
    #[serde(flatten)]
    pub base: BaseModelRecord,

    pub embedding_size: i32,
    pub rejection_threshold: f32,
    pub embedding_batch: usize,
}

pub fn default_rejection_threshold() -> f32 {
    0.63
}

pub fn default_embedding_batch() -> usize {
    64
}

impl HasBaseModelRecord for EmbeddingModelRecord {
    fn base(&self) -> &BaseModelRecord {
        &self.base
    }
    fn base_mut(&mut self) -> &mut BaseModelRecord {
        &mut self.base
    }
}

impl EmbeddingModelRecord {
    pub fn is_configured(&self) -> bool {
        !self.base.name.is_empty()
            && (self.embedding_size > 0 || self.embedding_batch > 0 || self.base.n_ctx > 0)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CapsMetadata {
    #[serde(default = "default_pricing")]
    pub pricing: serde_json::Value,
    #[serde(default)]
    pub features: Vec<String>,
}

fn default_pricing() -> serde_json::Value {
    serde_json::json!({})
}

impl Default for CapsMetadata {
    fn default() -> Self {
        Self {
            pricing: default_pricing(),
            features: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CodeAssistantCaps {
    #[serde(deserialize_with = "normalize_string")]
    pub cloud_name: String,

    #[serde(default = "default_telemetry_basic_dest")]
    pub telemetry_basic_dest: String,
    #[serde(default = "default_telemetry_retrieve_my_own")]
    pub telemetry_basic_retrieve_my_own: String,

    #[serde(skip_deserializing)]
    pub completion_models: IndexMap<String, Arc<CompletionModelRecord>>,
    #[serde(skip_deserializing)]
    pub chat_models: IndexMap<String, Arc<ChatModelRecord>>,
    #[serde(skip_deserializing)]
    pub embedding_model: EmbeddingModelRecord,

    #[serde(flatten, skip_deserializing)]
    pub defaults: DefaultModels,

    #[serde(default)]
    pub caps_version: i64,

    #[serde(default)]
    pub customization: String,

    #[serde(default = "default_hf_tokenizer_template")]
    pub hf_tokenizer_template: String,

    #[serde(default)]
    pub metadata: CapsMetadata,

    #[serde(skip)]
    pub model_caps: Arc<HashMap<String, ModelCapabilities>>,

    #[serde(skip)]
    pub user_defaults: ProviderDefaults,
}

fn default_telemetry_retrieve_my_own() -> String {
    "https://www.smallcloud.ai/v1/telemetry-retrieve-my-own-stats".to_string()
}

pub fn default_hf_tokenizer_template() -> String {
    "https://huggingface.co/$HF_MODEL/resolve/main/tokenizer.json".to_string()
}

fn default_telemetry_basic_dest() -> String {
    "https://www.smallcloud.ai/v1/telemetry-basic".to_string()
}

pub fn normalize_string<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let s: String = String::deserialize(deserializer)?;
    Ok(s.chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect())
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct DefaultModels {
    #[serde(
        default,
        alias = "code_completion_default_model",
        alias = "completion_model"
    )]
    pub completion_default_model: String,
    #[serde(default, alias = "code_chat_default_model", alias = "chat_model")]
    pub chat_default_model: String,
    #[serde(default)]
    pub chat_thinking_model: String,
    #[serde(default)]
    pub chat_light_model: String,
}

impl DefaultModels {
    pub fn apply_override(&mut self, other: &DefaultModels, provider_name: Option<&str>) {
        if !other.completion_default_model.is_empty() {
            self.completion_default_model = match provider_name {
                Some(provider) => format!("{}/{}", provider, other.completion_default_model),
                None => other.completion_default_model.clone(),
            };
        }
        if !other.chat_default_model.is_empty() {
            self.chat_default_model = match provider_name {
                Some(provider) => format!("{}/{}", provider, other.chat_default_model),
                None => other.chat_default_model.clone(),
            };
        }
        if !other.chat_thinking_model.is_empty() {
            self.chat_thinking_model = match provider_name {
                Some(provider) => format!("{}/{}", provider, other.chat_thinking_model),
                None => other.chat_thinking_model.clone(),
            };
        }
        if !other.chat_light_model.is_empty() {
            self.chat_light_model = match provider_name {
                Some(provider) => format!("{}/{}", provider, other.chat_light_model),
                None => other.chat_light_model.clone(),
            };
        }
    }
}

pub async fn load_caps_value_from_url(
    cmdline: CommandLine,
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Result<(serde_json::Value, String), String> {
    let caps_urls = if cmdline.address_url.to_lowercase() == "refact" {
        vec!["https://app.refact.ai/coding_assistant_caps.json".to_string()]
    } else {
        let base_url = Url::parse(&cmdline.address_url)
            .map_err(|_| "failed to parse address url".to_string())?;

        vec![
            base_url
                .join(&CAPS_FILENAME)
                .map_err(|_| "failed to join caps URL".to_string())?
                .to_string(),
            base_url
                .join(&CAPS_FILENAME_FALLBACK)
                .map_err(|_| "failed to join fallback caps URL".to_string())?
                .to_string(),
        ]
    };

    let http_client = gcx.read().await.http_client.clone();
    let mut headers = reqwest::header::HeaderMap::new();

    if !cmdline.api_key.is_empty() {
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", cmdline.api_key)).unwrap(),
        );
        headers.insert(
            reqwest::header::USER_AGENT,
            reqwest::header::HeaderValue::from_str(&format!(
                "refact-lsp {}",
                crate::version::build::PKG_VERSION
            ))
            .unwrap(),
        );
    }

    let mut last_status = 0;
    let mut last_response_json: Option<serde_json::Value> = None;

    for url in &caps_urls {
        info!("fetching caps from {}", url);
        let response = http_client
            .get(url)
            .headers(headers.clone())
            .send()
            .await
            .map_err(|e| e.to_string())?;

        last_status = response.status().as_u16();

        if let Ok(json_value) = response.json::<serde_json::Value>().await {
            if last_status == 200 {
                return Ok((json_value, url.clone()));
            }
            last_response_json = Some(json_value.clone());
            warn!(
                "status={}; server responded with:\n{}",
                last_status, json_value
            );
        }
    }

    if let Some(json_value) = last_response_json {
        if let Some(detail) = json_value.get("detail").and_then(|d| d.as_str()) {
            return Err(detail.to_string());
        }
    }

    Err(format!("cannot fetch caps, status={}", last_status))
}

/// Build ChatModelRecord from an AvailableModel and provider runtime info
fn build_chat_model_record(
    provider_name: &str,
    model: &AvailableModel,
    model_caps: &HashMap<String, ModelCapabilities>,
    runtime_wire_format: WireFormat,
    runtime_endpoint: &str,
    runtime_api_key: &str,
    runtime_auth_token: &str,
    runtime_tokenizer_api_key: &str,
    runtime_support_metadata: bool,
    runtime_extra_headers: &HashMap<String, String>,
) -> ChatModelRecord {
    let prefix = format!("{}/", provider_name);
    let model_id = if model.id.starts_with(&prefix) {
        model.id.clone()
    } else {
        format!("{}/{}", provider_name, model.id)
    };

    let resolved_caps = resolve_model_caps(model_caps, &model_id)
        .or_else(|| resolve_model_caps(model_caps, &model.id));

    // Get capabilities from model_caps or use the model's own values (for custom models)
    let (
        n_ctx,
        supports_tools,
        supports_multimodality,
        reasoning_effort_options,
        supports_thinking_budget,
        supports_adaptive_thinking_budget,
        tokenizer,
        supports_clicks,
    ) = if let Some(ref resolved) = resolved_caps {
        let caps = &resolved.caps;
        (
            caps.n_ctx,
            caps.supports_tools,
            caps.supports_vision,
            caps.reasoning_effort_options.clone(),
            caps.supports_thinking_budget,
            caps.supports_adaptive_thinking_budget,
            caps.tokenizer.clone(),
            caps.supports_clicks,
        )
    } else {
        (
            model.n_ctx,
            model.supports_tools,
            model.supports_multimodality,
            model.reasoning_effort_options.clone(),
            model.supports_thinking_budget,
            model.supports_adaptive_thinking_budget,
            model.tokenizer.clone().unwrap_or_default(),
            false,
        )
    };

    let supports_agent = supports_tools;
    let endpoint = runtime_endpoint.replace("$MODEL", &model.id);

    let endpoint_style = match runtime_wire_format {
        WireFormat::AnthropicMessages => "anthropic",
        _ => "openai",
    }
    .to_string();

    ChatModelRecord {
        base: BaseModelRecord {
            n_ctx,
            name: model.id.clone(),
            id: model_id,
            endpoint,
            endpoint_style,
            wire_format: runtime_wire_format,
            api_key: runtime_api_key.to_string(),
            auth_token: runtime_auth_token.to_string(),
            tokenizer_api_key: runtime_tokenizer_api_key.to_string(),
            support_metadata: runtime_support_metadata,
            extra_headers: runtime_extra_headers.clone(),
            similar_models: Vec::new(),
            tokenizer,
            enabled: model.enabled,
            experimental: false,
            supports_max_completion_tokens: resolved_caps
                .as_ref()
                .map(|r| r.caps.supports_max_completion_tokens)
                .unwrap_or(false),
            eof_is_done: false,
            supports_web_search: resolved_caps
                .as_ref()
                .map(|r| r.caps.supports_web_search)
                .unwrap_or(false),
            removable: model.is_custom,
            user_configured: model.is_custom,
        },
        scratchpad: String::new(),
        scratchpad_patch: serde_json::Value::Null,
        supports_tools,
        supports_multimodality,
        supports_clicks,
        supports_agent,
        reasoning_effort_options,
        supports_thinking_budget,
        supports_adaptive_thinking_budget,
        max_thinking_tokens: resolved_caps
            .as_ref()
            .and_then(|r| r.caps.max_thinking_tokens),
        default_temperature: resolved_caps
            .as_ref()
            .and_then(|r| r.caps.default_temperature),
        default_frequency_penalty: None,
        default_max_tokens: resolved_caps
            .as_ref()
            .and_then(|r| r.caps.default_max_tokens),
        max_output_tokens: resolved_caps
            .as_ref()
            .map(|r| r.caps.max_output_tokens)
            .filter(|&t| t > 0),
        supports_strict_tools: resolved_caps
            .as_ref()
            .map(|r| r.caps.supports_strict_tools)
            .unwrap_or(false),
        supports_temperature: resolved_caps
            .as_ref()
            .map(|r| r.caps.supports_temperature)
            .unwrap_or(false),
        
    }
}

pub async fn populate_chat_models_from_providers(
    caps: &mut CodeAssistantCaps,
    gcx: Arc<ARwLock<GlobalContext>>,
) {
    let model_caps = &*caps.model_caps;

    let (http_client, providers_snapshot) = {
        let gcx_locked = gcx.read().await;
        let registry = gcx_locked.providers.read().await;
        let snapshot: Vec<Box<dyn crate::providers::traits::ProviderTrait>> =
            registry.iter().map(|(_, p)| p.clone_box()).collect();
        (gcx_locked.http_client.clone(), snapshot)
    };

    let mut pricing_map = caps.metadata.pricing.as_object_mut();

    for provider in &providers_snapshot {
        let runtime = match provider.build_runtime() {
            Ok(r) => r,
            Err(e) => {
                warn!(
                    "Failed to build runtime for provider '{}': {}",
                    provider.name(),
                    e
                );
                continue;
            }
        };

        if !runtime.enabled {
            continue;
        }

        let available_models = provider.fetch_available_models(&http_client, model_caps).await;

        for model in available_models {
            if !model.enabled {
                continue;
            }

            let chat_record = build_chat_model_record(
                &runtime.name,
                &model,
                model_caps,
                runtime.wire_format,
                &runtime.chat_endpoint,
                &runtime.api_key,
                &runtime.auth_token,
                &runtime.tokenizer_api_key,
                runtime.support_metadata,
                &runtime.extra_headers,
            );

            let model_id = chat_record.base.id.clone();

            if let Some(ref pricing) = model.pricing {
                if let Some(map) = pricing_map.as_mut() {
                    if let Ok(pricing_value) = serde_json::to_value(pricing) {
                        map.insert(model_id.clone(), pricing_value.clone());
                        if !map.contains_key(&model.id) {
                            map.insert(model.id.clone(), pricing_value);
                        }
                    }
                }
            }

            caps.chat_models.insert(model_id, Arc::new(chat_record));
        }
    }

    if !caps.chat_models.is_empty() {
        let need_new_default = caps.defaults.chat_default_model.is_empty()
            || !caps
                .chat_models
                .contains_key(&caps.defaults.chat_default_model);

        if need_new_default {
            if let Some((first_model_id, _)) = caps.chat_models.first() {
                info!("Auto-selecting default chat model: {}", first_model_id);
                caps.defaults.chat_default_model = first_model_id.clone();
            }
        }

        let need_new_light = caps.defaults.chat_light_model.is_empty()
            || !caps
                .chat_models
                .contains_key(&caps.defaults.chat_light_model);
        if need_new_light && !caps.defaults.chat_default_model.is_empty() {
            info!(
                "Light model '{}' not available, falling back to default '{}'",
                caps.defaults.chat_light_model, caps.defaults.chat_default_model
            );
            caps.defaults.chat_light_model = caps.defaults.chat_default_model.clone();
        }

        let need_new_thinking = caps.defaults.chat_thinking_model.is_empty()
            || !caps
                .chat_models
                .contains_key(&caps.defaults.chat_thinking_model);
        if need_new_thinking && !caps.defaults.chat_default_model.is_empty() {
            info!(
                "Thinking model '{}' not available, falling back to default '{}'",
                caps.defaults.chat_thinking_model, caps.defaults.chat_default_model
            );
            caps.defaults.chat_thinking_model = caps.defaults.chat_default_model.clone();
        }
    }
}

fn convert_self_hosted_caps_if_needed(
    caps_value: serde_json::Value,
    caps_url: &str,
    cmdline_api_key: &str,
) -> Result<serde_json::Value, String> {
    let obj = match caps_value.as_object() {
        Some(o) => o,
        None => return Ok(caps_value),
    };

    let has_nested_chat = obj.get("chat").and_then(|v| v.get("models")).is_some();
    if !has_nested_chat {
        return Ok(caps_value);
    }

    let support_metadata = obj.get("support_metadata")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let tokenizer_endpoints = obj.get("tokenizer_endpoints")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let mut chat_models = serde_json::Map::new();
    if let Some(chat) = obj.get("chat").and_then(|v| v.as_object()) {
        let endpoint = chat.get("endpoint").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(models) = chat.get("models").and_then(|v| v.as_object()) {
            for (model_name, model_val) in models {
                let mut record = model_val.clone();
                if let Some(rec) = record.as_object_mut() {
                    rec.insert("name".to_string(), serde_json::json!(model_name));
                    let model_endpoint = endpoint.replace("$MODEL", model_name);
                    let full_endpoint = relative_to_full_url(caps_url, &model_endpoint)
                        .unwrap_or(model_endpoint);
                    rec.insert("endpoint".to_string(), serde_json::json!(full_endpoint));
                    rec.insert("endpoint_style".to_string(), serde_json::json!("openai"));
                    rec.insert("enabled".to_string(), serde_json::json!(true));
                    rec.insert("support_metadata".to_string(), serde_json::json!(support_metadata));
                    if !cmdline_api_key.is_empty() {
                        rec.insert("api_key".to_string(), serde_json::json!(cmdline_api_key));
                    }
                    if let Some(tok_url) = tokenizer_endpoints.get(model_name) {
                        if let Some(tok_str) = tok_url.as_str() {
                            let full_tok = relative_to_full_url(caps_url, tok_str)
                                .unwrap_or(tok_str.to_string());
                            rec.insert("tokenizer".to_string(), serde_json::json!(full_tok));
                        }
                    }
                    chat_models.insert(model_name.clone(), record);
                }
            }
        }
    }

    let mut completion_models = serde_json::Map::new();
    if let Some(completion) = obj.get("completion").and_then(|v| v.as_object()) {
        let endpoint = completion.get("endpoint").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(models) = completion.get("models").and_then(|v| v.as_object()) {
            for (model_name, model_val) in models {
                let mut record = model_val.clone();
                if let Some(rec) = record.as_object_mut() {
                    rec.insert("name".to_string(), serde_json::json!(model_name));
                    let model_endpoint = endpoint.replace("$MODEL", model_name);
                    let full_endpoint = relative_to_full_url(caps_url, &model_endpoint)
                        .unwrap_or(model_endpoint);
                    rec.insert("endpoint".to_string(), serde_json::json!(full_endpoint));
                    rec.insert("endpoint_style".to_string(), serde_json::json!("openai"));
                    rec.insert("enabled".to_string(), serde_json::json!(true));
                    if !cmdline_api_key.is_empty() {
                        rec.insert("api_key".to_string(), serde_json::json!(cmdline_api_key));
                    }
                    if let Some(tok_url) = tokenizer_endpoints.get(model_name) {
                        if let Some(tok_str) = tok_url.as_str() {
                            let full_tok = relative_to_full_url(caps_url, tok_str)
                                .unwrap_or(tok_str.to_string());
                            rec.insert("tokenizer".to_string(), serde_json::json!(full_tok));
                        }
                    }
                    completion_models.insert(model_name.clone(), record);
                }
            }
        }
    }

    let mut result = caps_value.clone();
    if let Some(result_obj) = result.as_object_mut() {
        result_obj.insert("chat_models".to_string(), serde_json::Value::Object(chat_models));
        result_obj.insert("completion_models".to_string(), serde_json::Value::Object(completion_models));

        if let Some(chat) = obj.get("chat").and_then(|v| v.as_object()) {
            let chat_endpoint = chat.get("endpoint").and_then(|v| v.as_str()).unwrap_or("");
            let full_chat_endpoint = relative_to_full_url(caps_url, chat_endpoint)
                .unwrap_or(chat_endpoint.to_string());
            result_obj.insert("chat_endpoint".to_string(), serde_json::json!(full_chat_endpoint));

            if let Some(dm) = chat.get("default_model").and_then(|v| v.as_str()) {
                if !dm.is_empty() {
                    result_obj.insert("chat_default_model".to_string(), serde_json::json!(dm));
                }
            }
            if let Some(dm) = chat.get("default_light_model").and_then(|v| v.as_str()) {
                if !dm.is_empty() {
                    result_obj.insert("chat_light_model".to_string(), serde_json::json!(dm));
                }
            }
            if let Some(dm) = chat.get("default_thinking_model").and_then(|v| v.as_str()) {
                if !dm.is_empty() {
                    result_obj.insert("chat_thinking_model".to_string(), serde_json::json!(dm));
                }
            }
        }

        if let Some(completion) = obj.get("completion").and_then(|v| v.as_object()) {
            let comp_endpoint = completion.get("endpoint").and_then(|v| v.as_str()).unwrap_or("");
            let full_comp_endpoint = relative_to_full_url(caps_url, comp_endpoint)
                .unwrap_or(comp_endpoint.to_string());
            result_obj.insert("completion_endpoint".to_string(), serde_json::json!(full_comp_endpoint));
        }

        if let Some(telem) = obj.get("telemetry_endpoints").and_then(|v| v.as_object()) {
            if let Some(basic) = telem.get("telemetry_basic_endpoint").and_then(|v| v.as_str()) {
                result_obj.insert("telemetry_basic_dest".to_string(), serde_json::json!(basic));
            }
            if let Some(own) = telem.get("telemetry_basic_retrieve_my_own_endpoint").and_then(|v| v.as_str()) {
                result_obj.insert("telemetry_basic_retrieve_my_own".to_string(), serde_json::json!(own));
            }
        }
    }

    Ok(result)
}

pub async fn load_caps(
    cmdline: crate::global_context::CommandLine,
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Result<Arc<CodeAssistantCaps>, String> {
    let (config_dir, cmdline_api_key, experimental) = {
        let gcx_locked = gcx.read().await;
        (
            gcx_locked.config_dir.clone(),
            gcx_locked.cmdline.api_key.clone(),
            gcx_locked.cmdline.experimental,
        )
    };

    let (caps_value, caps_url) = load_caps_value_from_url(cmdline, gcx.clone()).await?;

    let caps_value = convert_self_hosted_caps_if_needed(caps_value, &caps_url, &cmdline_api_key)?;

    let mut caps = serde_json::from_value::<CodeAssistantCaps>(caps_value.clone())
        .map_err_with_prefix("Failed to parse caps:")?;
    let mut server_provider = serde_json::from_value::<CapsProvider>(caps_value)
        .map_err_with_prefix("Failed to parse caps provider:")?;
    resolve_relative_urls(&mut server_provider, &caps_url)?;
    if caps.cloud_name == "refact" {
        server_provider.wire_format = WireFormat::Refact;
        server_provider.support_metadata = true;
        if let Some(pricing_obj) = caps.metadata.pricing.as_object() {
            for model_name in pricing_obj.keys() {
                if !server_provider.running_models.contains(model_name) {
                    server_provider.running_models.push(model_name.clone());
                }
            }
        }
    }
    let server_providers = vec![server_provider];

    caps.telemetry_basic_dest = relative_to_full_url(&caps_url, &caps.telemetry_basic_dest)?;
    caps.telemetry_basic_retrieve_my_own =
        relative_to_full_url(&caps_url, &caps.telemetry_basic_retrieve_my_own)?;

    let (mut providers, error_log) =
        read_providers_d(server_providers, &config_dir, experimental).await;
    providers.retain(|p| p.enabled);
    for e in error_log {
        tracing::error!("{e}");
    }
    for provider in &mut providers {
        post_process_provider(provider, false, experimental);
        provider.api_key = resolve_provider_api_key(&provider, &cmdline_api_key);
    }

    let address_url = gcx.read().await.cmdline.address_url.clone();
    let model_caps_map = get_model_caps(gcx.clone(), &address_url, false).await?;
    caps.model_caps = Arc::new(model_caps_map);
    if caps.cloud_name == "refact" {
        let running_models: Vec<String> = if let Some(pricing_obj) = caps.metadata.pricing.as_object() {
            pricing_obj.keys().cloned().collect()
        } else {
            Vec::new()
        };
        if !running_models.is_empty() {
            let gcx_locked = gcx.write().await;
            let mut registry = gcx_locked.providers.write().await;
            if let Some(provider) = registry.get_mut("refact") {
                provider.set_running_models(running_models);
            }
            drop(registry);
            drop(gcx_locked);
        }
    }

    add_models_to_caps(&mut caps, providers);
    populate_chat_models_from_providers(&mut caps, gcx.clone()).await;
    apply_model_caps_to_all_chat_models(&mut caps);

    match ProviderDefaults::load(&config_dir).await {
        Ok(user_defaults) => {
            let resolve_user_model = |model: &str, chat_models: &IndexMap<String, Arc<ChatModelRecord>>| -> Option<String> {
                if model.is_empty() {
                    return None;
                }
                if chat_models.contains_key(model) {
                    return Some(model.to_string());
                }
                if !model.contains('/') {
                    for key in chat_models.keys() {
                        if let Some(name) = key.split('/').last() {
                            if name == model {
                                return Some(key.clone());
                            }
                        }
                    }
                }
                None
            };

            if let Some(model) = &user_defaults.chat.model {
                match resolve_user_model(model, &caps.chat_models) {
                    Some(resolved) => caps.defaults.chat_default_model = resolved,
                    None if !model.is_empty() => warn!(
                        "User default chat model '{}' not found in available models, ignoring", model
                    ),
                    _ => {}
                }
            }
            if let Some(model) = &user_defaults.chat_light.model {
                match resolve_user_model(model, &caps.chat_models) {
                    Some(resolved) => caps.defaults.chat_light_model = resolved,
                    None if !model.is_empty() => warn!(
                        "User default light model '{}' not found in available models, ignoring", model
                    ),
                    _ => {}
                }
            }
            if let Some(model) = &user_defaults.chat_thinking.model {
                match resolve_user_model(model, &caps.chat_models) {
                    Some(resolved) => caps.defaults.chat_thinking_model = resolved,
                    None if !model.is_empty() => warn!(
                        "User default thinking model '{}' not found in available models, ignoring", model
                    ),
                    _ => {}
                }
            }
            caps.user_defaults = user_defaults;
        }
        Err(e) => {
            warn!(
                "Failed to load user defaults from providers.d/defaults.yaml: {}",
                e
            );
        }
    }

    validate_default_models(&caps)?;

    Ok(Arc::new(caps))
}

fn validate_default_models(caps: &CodeAssistantCaps) -> Result<(), String> {
    if !caps.defaults.chat_default_model.is_empty() {
        if !caps
            .chat_models
            .contains_key(&caps.defaults.chat_default_model)
        {
            if resolve_model_caps(&caps.model_caps, &caps.defaults.chat_default_model).is_none() {
                warn!(
                    "Default chat model '{}' is not in chat_models and not found in model capabilities registry",
                    caps.defaults.chat_default_model
                );
            }
        }
    }
    if !caps.defaults.chat_thinking_model.is_empty() {
        if !caps
            .chat_models
            .contains_key(&caps.defaults.chat_thinking_model)
        {
            if resolve_model_caps(&caps.model_caps, &caps.defaults.chat_thinking_model).is_none() {
                warn!(
                    "Default thinking model '{}' is not in chat_models and not found in model capabilities registry",
                    caps.defaults.chat_thinking_model
                );
            }
        }
    }
    if !caps.defaults.chat_light_model.is_empty() {
        if !caps
            .chat_models
            .contains_key(&caps.defaults.chat_light_model)
        {
            if resolve_model_caps(&caps.model_caps, &caps.defaults.chat_light_model).is_none() {
                warn!(
                    "Default light model '{}' is not in chat_models and not found in model capabilities registry",
                    caps.defaults.chat_light_model
                );
            }
        }
    }
    Ok(())
}

pub fn resolve_relative_urls(provider: &mut CapsProvider, caps_url: &str) -> Result<(), String> {
    provider.chat_endpoint = relative_to_full_url(caps_url, &provider.chat_endpoint)?;
    provider.completion_endpoint = relative_to_full_url(caps_url, &provider.completion_endpoint)?;
    provider.embedding_endpoint = relative_to_full_url(caps_url, &provider.embedding_endpoint)?;
    Ok(())
}

pub fn strip_model_from_finetune(model: &str) -> String {
    model.split(":").next().unwrap().to_string()
}

pub fn relative_to_full_url(caps_url: &str, maybe_relative_url: &str) -> Result<String, String> {
    if maybe_relative_url.starts_with("http") {
        Ok(maybe_relative_url.to_string())
    } else if maybe_relative_url.is_empty() {
        Ok("".to_string())
    } else {
        let base_url =
            Url::parse(caps_url).map_err(|_| format!("failed to parse caps url: {}", caps_url))?;
        let joined_url = base_url
            .join(maybe_relative_url)
            .map_err(|_| format!("failed to join url: {}", maybe_relative_url))?;
        Ok(joined_url.to_string())
    }
}

pub fn resolve_model<'a, T>(
    models: &'a IndexMap<String, Arc<T>>,
    model_id: &str,
) -> Result<Arc<T>, String> {
    models
        .get(model_id)
        .or_else(|| models.get(&strip_model_from_finetune(model_id)))
        .cloned()
        .ok_or(format!(
            "Model '{}' not found. Server has the following models: {:?}",
            model_id,
            models.keys()
        ))
}

pub fn resolve_chat_model(
    caps: Arc<CodeAssistantCaps>,
    requested_model_id: &str,
) -> Result<Arc<ChatModelRecord>, String> {
    let model_id = if !requested_model_id.is_empty() {
        requested_model_id
    } else {
        &caps.defaults.chat_default_model
    };

    let base_record = resolve_model(&caps.chat_models, model_id)?;

    let resolved = resolve_model_caps(&caps.model_caps, model_id);

    match resolved {
        Some(resolved_caps) => {
            tracing::debug!(
                "Model '{}' resolved via {:?}, matched key: '{}'",
                model_id,
                resolved_caps.source,
                resolved_caps.matched_key
            );
            let mut effective = (*base_record).clone();
            apply_registry_caps_to_chat_model(&mut effective, &resolved_caps.caps);
            Ok(Arc::new(effective))
        }
        None => {
            // Model not in registry (e.g., custom model) - use base_record as-is
            // The base_record already has capabilities from build_chat_model_record
            tracing::debug!(
                "Model '{}' not in model_caps registry, using configured capabilities",
                model_id
            );
            Ok(base_record)
        }
    }
}

fn apply_model_caps_to_all_chat_models(caps: &mut CodeAssistantCaps) {
    let model_ids: Vec<String> = caps.chat_models.keys().cloned().collect();
    for model_id in model_ids {
        if let Some(resolved) = resolve_model_caps(&caps.model_caps, &model_id) {
            if let Some(record) = caps.chat_models.get(&model_id) {
                let mut updated = (**record).clone();
                apply_registry_caps_to_chat_model(&mut updated, &resolved.caps);
                caps.chat_models.insert(model_id, Arc::new(updated));
            }
        }
    }
}

fn apply_registry_caps_to_chat_model(record: &mut ChatModelRecord, caps: &ModelCapabilities) {
    record.base.n_ctx = caps.n_ctx;
    record.base.supports_max_completion_tokens = caps.supports_max_completion_tokens;

    record.supports_tools = caps.supports_tools;
    record.supports_strict_tools = caps.supports_strict_tools;
    record.supports_multimodality = caps.supports_vision;
    record.supports_clicks = caps.supports_clicks;
    record.default_temperature = caps.default_temperature;
    record.default_max_tokens = caps.default_max_tokens;
    record.max_output_tokens = if caps.max_output_tokens > 0 {
        Some(caps.max_output_tokens)
    } else {
        None
    };

    if !caps.tokenizer.is_empty() {
        record.base.tokenizer = caps.tokenizer.clone();
    }

    record.reasoning_effort_options = caps.reasoning_effort_options.clone();
    record.supports_thinking_budget = caps.supports_thinking_budget;
    record.supports_adaptive_thinking_budget = caps.supports_adaptive_thinking_budget;
    record.supports_agent = caps.supports_tools;
    record.supports_temperature = caps.supports_temperature;
    record.base.supports_web_search = caps.supports_web_search;
}

pub fn resolve_completion_model<'a>(
    caps: Arc<CodeAssistantCaps>,
    requested_model_id: &str,
    try_refact_fallbacks: bool,
) -> Result<Arc<CompletionModelRecord>, String> {
    let model_id = if !requested_model_id.is_empty() {
        requested_model_id
    } else {
        &caps.defaults.completion_default_model
    };

    match resolve_model(&caps.completion_models, model_id) {
        Ok(model) => Ok(model),
        Err(first_err) if try_refact_fallbacks => {
            if let Ok(model) = resolve_model(&caps.completion_models, &format!("refact/{model_id}"))
            {
                return Ok(model);
            }
            Err(first_err)
        }
        Err(err) => Err(err),
    }
}

#[allow(dead_code)]
pub fn is_cloud_model(model_id: &str) -> bool {
    model_id.starts_with("refact/")
}
