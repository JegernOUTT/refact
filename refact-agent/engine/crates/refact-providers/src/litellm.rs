use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use refact_core::llm_types::WireFormat;
use refact_core::model_caps::{resolve_model_caps, ModelCapabilities};
use refact_core::provider_types::{
    available_model_from_catalog_and_live, CredentialSpec, LiveModelFields,
};

use crate::config::resolve_env_var;
use crate::credential;
use crate::traits::{
    extra_headers_mapping_to_hash_map, merge_custom_models, parse_custom_models,
    parse_enabled_models, parse_extra_headers_value, set_model_enabled_impl, AvailableModel,
    CustomModelConfig, ModelPricing, ModelSource, ProviderRuntime, ProviderTrait,
};

const DEFAULT_ENDPOINT: &str = "http://localhost:4000";
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);
const METADATA_ENRICHMENT_TIMEOUT: Duration = Duration::from_secs(2);
const BUDGET_QUERY_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_BUDGET_BODY_BYTES: usize = 64 * 1024;
const MAX_DISCOVERY_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_DISCOVERED_CONTEXT_TOKENS: usize = 10_000_000;
const MAX_DISCOVERED_OUTPUT_TOKENS: usize = 1_000_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiteLLMProvider {
    pub endpoint: String,
    pub api_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<CredentialSpec>,
    pub enabled: bool,
    #[serde(default)]
    pub enabled_models: Vec<String>,
    #[serde(default)]
    pub disabled_models: Vec<String>,
    #[serde(default)]
    pub custom_models: HashMap<String, CustomModelConfig>,
    #[serde(default)]
    pub extra_headers: HashMap<String, String>,
    #[serde(default)]
    pub supports_cache_control: bool,
}

impl Default for LiteLLMProvider {
    fn default() -> Self {
        Self {
            endpoint: DEFAULT_ENDPOINT.to_string(),
            api_key: String::new(),
            admin_url: None,
            admin_api_key: None,
            credential: None,
            enabled: false,
            enabled_models: Vec::new(),
            disabled_models: Vec::new(),
            custom_models: HashMap::new(),
            extra_headers: HashMap::new(),
            supports_cache_control: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiteLLMBudget {
    pub spend: f64,
    pub max_budget: f64,
    pub remaining: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LiteLLMBudgetUnavailable {
    NotConfigured,
    BudgetNotConfigured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LiteLLMBudgetError {
    InvalidAdminUrl,
    RequestFailed,
    HttpStatus(u16),
    ResponseTooLarge,
    InvalidResponse,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LiteLLMBudgetQueryResult {
    Available(LiteLLMBudget),
    Unavailable(LiteLLMBudgetUnavailable),
    Error(LiteLLMBudgetError),
}

#[derive(Clone, Debug, Default)]
struct DeploymentMetadata {
    live: LiveModelFields,
    input_capacity: Option<usize>,
    total_context: Option<usize>,
    prompt_price_explicit: bool,
    generated_price_explicit: bool,
    routed_model: Option<String>,
    mode: Option<String>,
    routing_mode_missing: bool,
    base_model_conflict: bool,
    tokenizer_conflict: bool,
    reasoning_conflict: bool,
    routed_model_conflict: bool,
    mode_conflict: bool,
}

struct DiscoveryAuth {
    provider_id: String,
    bearer: String,
    refreshed_after_rejection: bool,
}

impl LiteLLMProvider {
    fn base_url(&self) -> String {
        normalize_base_url(&self.endpoint)
    }

    fn bearer_from_static(&self) -> String {
        resolve_env_var(&self.api_key, "", "litellm api_key")
    }

    fn validated_admin_url(value: &str) -> Result<String, LiteLLMBudgetError> {
        let normalized = normalize_base_url(value);
        let url =
            reqwest::Url::parse(&normalized).map_err(|_| LiteLLMBudgetError::InvalidAdminUrl)?;
        if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
            return Err(LiteLLMBudgetError::InvalidAdminUrl);
        }
        let host = url.host_str().ok_or(LiteLLMBudgetError::InvalidAdminUrl)?;
        let permitted_scheme = url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]"));
        if !permitted_scheme {
            return Err(LiteLLMBudgetError::InvalidAdminUrl);
        }
        Ok(normalized)
    }

    fn admin_url_for_settings(&self) -> Option<String> {
        let value = self.admin_url.as_deref()?;
        let mut url = reqwest::Url::parse(value).ok()?;
        if !url.username().is_empty() || url.password().is_some() {
            url.set_username("").ok()?;
            url.set_password(None).ok()?;
        }
        Some(normalize_base_url(url.as_str()))
    }

    fn parse_budget_response(value: &Value) -> LiteLLMBudgetQueryResult {
        let info = value
            .get("info")
            .filter(|value| value.is_object())
            .unwrap_or(value);
        if matches!(info.get("max_budget"), None | Some(Value::Null)) {
            return LiteLLMBudgetQueryResult::Unavailable(
                LiteLLMBudgetUnavailable::BudgetNotConfigured,
            );
        }
        let Some(spend) = Self::budget_number(info.get("spend")).filter(|value| *value >= 0.0)
        else {
            return LiteLLMBudgetQueryResult::Error(LiteLLMBudgetError::InvalidResponse);
        };
        let Some(max_budget) =
            Self::budget_number(info.get("max_budget")).filter(|value| *value >= 0.0)
        else {
            return LiteLLMBudgetQueryResult::Error(LiteLLMBudgetError::InvalidResponse);
        };
        let remaining = Self::budget_number(info.get("remaining"));
        LiteLLMBudgetQueryResult::Available(LiteLLMBudget {
            spend,
            max_budget,
            remaining,
        })
    }

    fn budget_number(value: Option<&Value>) -> Option<f64> {
        value
            .and_then(|value| {
                value.as_f64().or_else(|| {
                    value
                        .as_str()
                        .and_then(|text| text.trim().parse::<f64>().ok())
                })
            })
            .filter(|number| number.is_finite())
    }

    pub async fn query_budget(&self, http_client: &reqwest::Client) -> LiteLLMBudgetQueryResult {
        let Some(configured_url) = self
            .admin_url
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        else {
            return LiteLLMBudgetQueryResult::Unavailable(LiteLLMBudgetUnavailable::NotConfigured);
        };
        let admin_url = match Self::validated_admin_url(configured_url) {
            Ok(value) => value,
            Err(error) => return LiteLLMBudgetQueryResult::Error(error),
        };
        let Some(configured_key) = self
            .admin_api_key
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        else {
            return LiteLLMBudgetQueryResult::Unavailable(LiteLLMBudgetUnavailable::NotConfigured);
        };
        let admin_api_key = resolve_env_var(configured_key, "", "litellm admin_api_key");
        if admin_api_key.is_empty() {
            return LiteLLMBudgetQueryResult::Unavailable(LiteLLMBudgetUnavailable::NotConfigured);
        }
        let response = match http_client
            .get(format!("{admin_url}/key/info"))
            .bearer_auth(admin_api_key)
            .timeout(BUDGET_QUERY_TIMEOUT)
            .send()
            .await
        {
            Ok(response) => response,
            Err(_) => {
                return LiteLLMBudgetQueryResult::Error(LiteLLMBudgetError::RequestFailed);
            }
        };
        if !response.status().is_success() {
            return LiteLLMBudgetQueryResult::Error(LiteLLMBudgetError::HttpStatus(
                response.status().as_u16(),
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_BUDGET_BODY_BYTES as u64)
        {
            return LiteLLMBudgetQueryResult::Error(LiteLLMBudgetError::ResponseTooLarge);
        }
        let mut response = response;
        let mut body = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if body.len().saturating_add(chunk.len()) > MAX_BUDGET_BODY_BYTES {
                        return LiteLLMBudgetQueryResult::Error(
                            LiteLLMBudgetError::ResponseTooLarge,
                        );
                    }
                    body.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(_) => {
                    return LiteLLMBudgetQueryResult::Error(LiteLLMBudgetError::RequestFailed);
                }
            }
        }
        match serde_json::from_slice(&body) {
            Ok(value) => Self::parse_budget_response(&value),
            Err(_) => LiteLLMBudgetQueryResult::Error(LiteLLMBudgetError::InvalidResponse),
        }
    }

    async fn discovery_auth(&self, provider_id: &str) -> Result<DiscoveryAuth, String> {
        match &self.credential {
            Some(spec) => Ok(DiscoveryAuth {
                provider_id: provider_id.to_string(),
                bearer: credential::resolve(provider_id, spec, false).await?,
                refreshed_after_rejection: false,
            }),
            None => Ok(DiscoveryAuth {
                provider_id: provider_id.to_string(),
                bearer: self.bearer_from_static(),
                refreshed_after_rejection: false,
            }),
        }
    }

    fn request(
        &self,
        http_client: &reqwest::Client,
        url: &str,
        bearer: &str,
    ) -> reqwest::RequestBuilder {
        let mut request = http_client.get(url).timeout(DISCOVERY_TIMEOUT);
        if !bearer.is_empty() {
            request = request.bearer_auth(bearer);
        }
        for (name, value) in &self.extra_headers {
            request = request.header(name.as_str(), value.as_str());
        }
        request
    }

    async fn get_json_with_refresh(
        &self,
        http_client: &reqwest::Client,
        url: &str,
        auth: &mut DiscoveryAuth,
    ) -> Result<Value, String> {
        let mut response = self
            .request(http_client, url, &auth.bearer)
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;
        if matches!(response.status().as_u16(), 401 | 403) && !auth.refreshed_after_rejection {
            if let Some(spec) = &self.credential {
                auth.bearer =
                    credential::refresh_after_rejection(&auth.provider_id, spec, &auth.bearer)
                        .await?;
                auth.refreshed_after_rejection = true;
                response = self
                    .request(http_client, url, &auth.bearer)
                    .send()
                    .await
                    .map_err(|e| format!("retry failed: {e}"))?;
            }
        }
        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_DISCOVERY_BODY_BYTES as u64)
        {
            return Err("response body is too large".to_string());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| format!("failed to read response: {e}"))?
        {
            if body.len().saturating_add(chunk.len()) > MAX_DISCOVERY_BODY_BYTES {
                return Err("response body is too large".to_string());
            }
            body.extend_from_slice(&chunk);
        }
        let mut value: Value =
            serde_json::from_slice(&body).map_err(|e| format!("invalid JSON response: {e}"))?;
        Self::redact_json_strings(&mut value, &auth.bearer);
        Ok(value)
    }

    fn redact_json_strings(value: &mut Value, secret: &str) {
        if secret.is_empty() {
            return;
        }
        match value {
            Value::String(text) => {
                if text.contains(secret) {
                    *text = text.replace(secret, "[REDACTED]");
                }
            }
            Value::Array(values) => {
                for value in values {
                    Self::redact_json_strings(value, secret);
                }
            }
            Value::Object(values) => {
                for value in values.values_mut() {
                    Self::redact_json_strings(value, secret);
                }
            }
            _ => {}
        }
    }

    async fn discover_with_fallback(
        &self,
        http_client: &reqwest::Client,
        paths: &[&str],
        auth: &mut DiscoveryAuth,
        require_rows_shape: bool,
    ) -> Option<Value> {
        for path in paths {
            let url = format!("{}{}", self.base_url(), path);
            if let Ok(value) = self.get_json_with_refresh(http_client, &url, auth).await {
                if !require_rows_shape || Self::has_rows_shape(&value) {
                    return Some(value);
                }
            }
        }
        None
    }

    fn metadata_object<'a>(row: &'a Value) -> &'a Value {
        row.get("model_info")
            .filter(|v| v.is_object())
            .unwrap_or(row)
    }

    fn number(value: Option<&Value>) -> Option<f64> {
        value.and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
        })
    }

    fn non_negative_number(value: Option<&Value>) -> Option<f64> {
        Self::number(value).filter(|number| {
            number.is_finite() && *number >= 0.0 && (*number * 1_000_000.0).is_finite()
        })
    }

    fn safe_tokenizer(value: String) -> Option<String> {
        let lower = value.to_ascii_lowercase();
        let secret_shaped = lower.contains("key=")
            || lower.contains("token=")
            || lower.contains("authorization: bearer")
            || lower.contains("bearer ")
            || lower.starts_with("sk-")
            || lower.starts_with("sk_")
            || (value.starts_with("eyJ") && value.matches('.').count() == 2);
        (!secret_shaped).then_some(value)
    }

    fn routing_mode(row: &Value) -> (Option<String>, bool) {
        let info = row.get("model_info").filter(|value| value.is_object());
        let params = row.get("litellm_params").filter(|value| value.is_object());
        let mut values = Vec::new();
        for object in info.into_iter().chain(std::iter::once(row)).chain(params) {
            for name in ["mode", "api_mode"] {
                if let Some(value) = Self::string_field(object, &[name]) {
                    let normalized = value.to_ascii_lowercase();
                    if !values.contains(&normalized) {
                        values.push(normalized);
                    }
                }
            }
        }
        (values.first().cloned(), values.len() > 1)
    }

    fn usize_field(object: &Value, names: &[&str]) -> Option<usize> {
        names.iter().find_map(|name| {
            Self::number(object.get(*name)).and_then(|n| {
                if n.is_finite() && n > 0.0 {
                    Some(n as usize)
                } else {
                    None
                }
            })
        })
    }

    fn bool_field(object: &Value, names: &[&str]) -> Option<bool> {
        names.iter().find_map(|name| {
            object.get(*name).and_then(|v| {
                v.as_bool().or_else(|| match v.as_str() {
                    Some("true") => Some(true),
                    Some("false") => Some(false),
                    _ => None,
                })
            })
        })
    }

    fn string_field(object: &Value, names: &[&str]) -> Option<String> {
        names.iter().find_map(|name| {
            object
                .get(*name)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
    }

    fn string_field_allow_empty(object: &Value, names: &[&str]) -> Option<String> {
        names.iter().find_map(|name| {
            object
                .get(*name)
                .and_then(Value::as_str)
                .map(str::trim)
                .map(str::to_string)
        })
    }

    fn supported_parameters_array(row: &Value) -> Option<&Vec<Value>> {
        let info = Self::metadata_object(row);
        row.get("supported_parameters")
            .and_then(Value::as_array)
            .or_else(|| row.get("supported_openai_params").and_then(Value::as_array))
            .or_else(|| info.get("supported_parameters").and_then(Value::as_array))
            .or_else(|| {
                info.get("supported_openai_params")
                    .and_then(Value::as_array)
            })
    }

    fn supported_parameters(row: &Value) -> HashSet<&str> {
        Self::supported_parameters_array(row)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect()
    }

    fn parse_pricing(info: &Value) -> Option<ModelPricing> {
        let prompt = Self::non_negative_number(
            info.get("input_cost_per_token")
                .or_else(|| info.get("prompt_cost_per_token")),
        );
        let generated = Self::non_negative_number(
            info.get("output_cost_per_token")
                .or_else(|| info.get("completion_cost_per_token")),
        );
        let cache_read = Self::non_negative_number(
            info.get("cache_read_input_token_cost")
                .or_else(|| info.get("cache_read_cost_per_token")),
        );
        let cache_creation = Self::non_negative_number(
            info.get("cache_creation_input_token_cost")
                .or_else(|| info.get("cache_creation_cost_per_token")),
        );
        if prompt.is_none()
            && generated.is_none()
            && cache_read.is_none()
            && cache_creation.is_none()
        {
            return None;
        }
        Some(ModelPricing {
            prompt: prompt.unwrap_or(0.0) * 1_000_000.0,
            generated: generated.unwrap_or(0.0) * 1_000_000.0,
            cache_read: cache_read.map(|value| value * 1_000_000.0),
            cache_creation: cache_creation.map(|value| value * 1_000_000.0),
            ..Default::default()
        })
    }

    fn string_list_field(object: &Value, names: &[&str]) -> Option<Vec<String>> {
        names.iter().find_map(|name| {
            object.get(*name).and_then(Value::as_array).map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .collect()
            })
        })
    }

    fn f32_field(object: &Value, names: &[&str]) -> Option<f32> {
        names
            .iter()
            .find_map(|name| Self::number(object.get(*name)))
            .filter(|value| value.is_finite())
            .map(|value| value as f32)
    }

    fn is_sonnet_five(row: &Value, info: &Value) -> bool {
        let routed_model = row
            .get("litellm_params")
            .and_then(|params| Self::string_field(params, &["model"]));
        [
            Self::string_field(row, &["id"]),
            Self::string_field(row, &["model_name"]),
            Self::string_field(row, &["model"]),
            routed_model,
            Self::string_field(info, &["base_model"]),
            Self::string_field(info, &["model"]),
        ]
        .into_iter()
        .flatten()
        .any(|model| model.to_ascii_lowercase().contains("sonnet-5"))
    }

    fn parse_reasoning(
        row: &Value,
        info: &Value,
        parameters: &HashSet<&str>,
    ) -> Option<Vec<String>> {
        if Self::bool_field(row, &["supports_reasoning"]) == Some(false) {
            return Some(Vec::new());
        }
        let explicit = Self::string_list_field(
            row,
            &["reasoning_effort_options", "supported_reasoning_efforts"],
        )
        .or_else(|| {
            Self::string_list_field(
                info,
                &["reasoning_effort_options", "supported_reasoning_efforts"],
            )
        });
        if explicit.is_some() {
            return explicit;
        }
        let supports_reasoning = Self::bool_field(row, &["supports_reasoning"])
            .or_else(|| Self::bool_field(info, &["supports_reasoning"]));
        if supports_reasoning == Some(false) {
            return Some(Vec::new());
        }
        if supports_reasoning != Some(true)
            && !parameters.contains("reasoning_effort")
            && !Self::is_sonnet_five(row, info)
        {
            return None;
        }
        let mut efforts = Vec::new();
        let mut has_explicit_effort_capability = false;
        for (name, supported) in [
            ("none", "supports_none_reasoning_effort"),
            ("minimal", "supports_minimal_reasoning_effort"),
            ("low", "supports_low_reasoning_effort"),
            ("medium", "supports_medium_reasoning_effort"),
            ("high", "supports_high_reasoning_effort"),
            ("xhigh", "supports_xhigh_reasoning_effort"),
            ("max", "supports_max_reasoning_effort"),
        ] {
            if let Some(is_supported) =
                Self::bool_field(row, &[supported]).or_else(|| Self::bool_field(info, &[supported]))
            {
                has_explicit_effort_capability = true;
                if is_supported {
                    efforts.push(name.to_string());
                }
            }
        }
        if efforts.is_empty() && !has_explicit_effort_capability {
            if Self::is_sonnet_five(row, info) {
                efforts.extend(["high".to_string(), "xhigh".to_string()]);
            } else {
                efforts.extend(["low".to_string(), "medium".to_string(), "high".to_string()]);
            }
        }
        Some(efforts)
    }

    fn parse_metadata(row: &Value) -> DeploymentMetadata {
        let info = Self::metadata_object(row);
        let parameters = Self::supported_parameters(row);
        let supported_parameters = Self::supported_parameters_array(row).map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        });
        let has_parameters = supported_parameters.is_some();
        let modalities = info
            .get("input_modalities")
            .or_else(|| info.get("modalities"))
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<HashSet<_>>()
            });
        let routed_model = row
            .get("litellm_params")
            .and_then(|value| value.get("model"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let supports_tools = Self::bool_field(
            info,
            &[
                "supports_function_calling",
                "supports_tools",
                "supports_tool_choice",
            ],
        )
        .or_else(|| {
            has_parameters.then(|| {
                parameters
                    .iter()
                    .any(|parameter| matches!(*parameter, "tools" | "functions" | "tool_choice"))
            })
        });
        let multimodal_flags = [
            "supports_vision",
            "supports_audio_input",
            "supports_pdf_input",
            "supports_video_input",
        ]
        .iter()
        .filter_map(|name| Self::bool_field(info, &[*name]))
        .collect::<Vec<_>>();
        let supports_multimodality = (!multimodal_flags.is_empty())
            .then(|| multimodal_flags.iter().any(|value| *value))
            .or_else(|| {
                modalities.as_ref().map(|modalities| {
                    modalities
                        .iter()
                        .any(|modality| matches!(*modality, "image" | "audio" | "pdf" | "video"))
                })
            });
        let max_output_tokens = Self::usize_field(
            info,
            &["max_output_tokens", "max_tokens", "max_completion_tokens"],
        );
        let input_capacity = Self::usize_field(info, &["max_input_tokens"]);
        let total_context = Self::usize_field(
            info,
            &[
                "context_length",
                "max_context_length",
                "max_context_window",
                "max_model_len",
            ],
        );
        let (mode, mode_conflict) = Self::routing_mode(row);
        let routing_mode_missing = mode.is_none();
        let reasoning_opted_out = Self::bool_field(row, &["supports_reasoning"])
            .or_else(|| Self::bool_field(info, &["supports_reasoning"]))
            == Some(false);
        let supports_thinking =
            Self::bool_field(row, &["supports_thinking", "supports_thinking_budget"]).or_else(
                || Self::bool_field(info, &["supports_thinking", "supports_thinking_budget"]),
            );
        let supports_adaptive_thinking = Self::bool_field(
            row,
            &[
                "supports_adaptive_thinking",
                "supports_adaptive_thinking_budget",
            ],
        )
        .or_else(|| {
            Self::bool_field(
                info,
                &[
                    "supports_adaptive_thinking",
                    "supports_adaptive_thinking_budget",
                ],
            )
        });
        let max_thinking_tokens =
            Self::usize_field(row, &["max_thinking_tokens", "max_thinking_budget"]).or_else(|| {
                Self::usize_field(info, &["max_thinking_tokens", "max_thinking_budget"])
            });
        let live = LiveModelFields {
            display_name: Self::string_field(info, &["display_name", "model_name"]),
            n_ctx: None,
            max_output_tokens,
            supports_tools,
            supports_parallel_tools: Self::bool_field(
                info,
                &[
                    "supports_parallel_function_calling",
                    "supports_parallel_tools",
                ],
            )
            .or_else(|| has_parameters.then(|| parameters.contains("parallel_tool_calls"))),
            supports_strict_tools: Self::bool_field(
                info,
                &[
                    "supports_response_schema",
                    "supports_strict_tools",
                    "supports_json_schema",
                ],
            )
            .or_else(|| {
                has_parameters.then(|| {
                    parameters
                        .iter()
                        .any(|parameter| matches!(*parameter, "response_format" | "json_schema"))
                })
            }),
            supports_multimodality,
            supports_clicks: Self::bool_field(
                info,
                &["supports_computer_use", "supports_clicks", "supports_click"],
            ),
            reasoning_effort_options: Self::parse_reasoning(row, info, &parameters),
            supports_thinking_budget: if reasoning_opted_out {
                Some(false)
            } else {
                supports_thinking.or_else(|| max_thinking_tokens.map(|_| true))
            },
            supports_adaptive_thinking_budget: if reasoning_opted_out {
                Some(false)
            } else {
                supports_adaptive_thinking
            },
            max_thinking_tokens,
            supports_cache_control: Self::bool_field(
                row,
                &[
                    "supports_prompt_caching",
                    "supports_cache_control",
                    "supports_caching",
                ],
            )
            .or_else(|| {
                Self::bool_field(
                    info,
                    &[
                        "supports_prompt_caching",
                        "supports_cache_control",
                        "supports_caching",
                    ],
                )
            }),
            tokenizer: Self::string_field_allow_empty(info, &["tokenizer"])
                .and_then(Self::safe_tokenizer),
            pricing: Self::parse_pricing(info),
            supports_temperature: Self::bool_field(info, &["supports_temperature"])
                .or_else(|| has_parameters.then(|| parameters.contains("temperature"))),
            default_temperature: Self::f32_field(info, &["default_temperature"]),
            default_max_tokens: Self::usize_field(
                info,
                &["default_max_tokens", "default_max_completion_tokens"],
            ),
            supports_web_search: Self::bool_field(
                info,
                &["supports_web_search", "supports_web_search_preview"],
            )
            .or_else(|| has_parameters.then(|| parameters.contains("web_search_options"))),
            supports_max_completion_tokens: Self::bool_field(
                info,
                &["supports_max_completion_tokens"],
            )
            .or_else(|| has_parameters.then(|| parameters.contains("max_completion_tokens"))),
            supported_parameters,
            wire_format_override: None,
            endpoint_override: None,
            base_model: Self::string_field(info, &["base_model", "model"])
                .or_else(|| routed_model.clone()),
            upstream_provider: Self::string_field(
                info,
                &["litellm_provider", "custom_llm_provider", "provider"],
            )
            .or_else(|| {
                row.get("litellm_params").and_then(|params| {
                    Self::string_field(params, &["custom_llm_provider", "litellm_provider"])
                })
            })
            .or_else(|| Self::string_field(row, &["litellm_provider", "provider"])),
            api_mode: mode.clone(),
            // LiteLLM's /model/info does not report per-modality detail or a
            // minimum thinking budget; left absent so the catalog can answer.
            ..Default::default()
        };
        DeploymentMetadata {
            live,
            input_capacity,
            total_context,
            prompt_price_explicit: Self::non_negative_number(
                info.get("input_cost_per_token")
                    .or_else(|| info.get("prompt_cost_per_token")),
            )
            .is_some(),
            generated_price_explicit: Self::non_negative_number(
                info.get("output_cost_per_token")
                    .or_else(|| info.get("completion_cost_per_token")),
            )
            .is_some(),
            routed_model,
            mode,
            routing_mode_missing,
            mode_conflict,
            ..Default::default()
        }
    }

    fn merge_metadata(existing: &mut DeploymentMetadata, next: DeploymentMetadata) {
        fn min_some<T: Ord>(a: Option<T>, b: Option<T>) -> Option<T> {
            match (a, b) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, None) => a,
                (None, b) => b,
            }
        }
        fn conservative_bool(a: Option<bool>, b: Option<bool>) -> Option<bool> {
            match (a, b) {
                (Some(a), Some(b)) => Some(a && b),
                (Some(a), None) => Some(a),
                (None, b) => b,
            }
        }
        fn max_optional(a: Option<f64>, b: Option<f64>) -> Option<f64> {
            match (a, b) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, None) => a,
                (None, b) => b,
            }
        }
        fn merge_equal<T: PartialEq>(a: Option<T>, b: Option<T>) -> Option<T> {
            match (a, b) {
                (Some(a), Some(b)) if a == b => Some(a),
                (Some(_), Some(_)) => None,
                (a, None) => a,
                (None, b) => b,
            }
        }
        macro_rules! merge_bool {
            ($field:ident) => {
                existing.live.$field = conservative_bool(existing.live.$field, next.live.$field)
            };
        }
        macro_rules! merge_min {
            ($field:ident) => {
                existing.live.$field = min_some(existing.live.$field, next.live.$field)
            };
        }
        existing.input_capacity = min_some(existing.input_capacity, next.input_capacity);
        existing.total_context = min_some(existing.total_context, next.total_context);
        existing.prompt_price_explicit |= next.prompt_price_explicit;
        existing.generated_price_explicit |= next.generated_price_explicit;
        merge_min!(max_output_tokens);
        merge_min!(max_thinking_tokens);
        merge_min!(default_max_tokens);
        merge_bool!(supports_tools);
        merge_bool!(supports_parallel_tools);
        merge_bool!(supports_strict_tools);
        merge_bool!(supports_multimodality);
        merge_bool!(supports_clicks);
        merge_bool!(supports_thinking_budget);
        merge_bool!(supports_adaptive_thinking_budget);
        merge_bool!(supports_cache_control);
        merge_bool!(supports_temperature);
        merge_bool!(supports_web_search);
        merge_bool!(supports_max_completion_tokens);
        match (&mut existing.live.pricing, next.live.pricing) {
            (Some(old), Some(new)) => {
                old.prompt = old.prompt.max(new.prompt);
                old.generated = old.generated.max(new.generated);
                old.cache_read = max_optional(old.cache_read, new.cache_read);
                old.cache_creation = max_optional(old.cache_creation, new.cache_creation);
            }
            (None, pricing) => existing.live.pricing = pricing,
            _ => {}
        }
        if existing.live.base_model.is_some()
            && next.live.base_model.is_some()
            && existing.live.base_model != next.live.base_model
        {
            existing.base_model_conflict = true;
        }
        existing.live.base_model = if existing.base_model_conflict {
            None
        } else {
            merge_equal(existing.live.base_model.take(), next.live.base_model)
        };
        if existing.live.tokenizer.is_some()
            && next.live.tokenizer.is_some()
            && existing.live.tokenizer != next.live.tokenizer
        {
            existing.tokenizer_conflict = true;
        }
        existing.live.tokenizer = if existing.tokenizer_conflict {
            None
        } else {
            merge_equal(existing.live.tokenizer.take(), next.live.tokenizer)
        };
        if let (Some(old), Some(new)) = (
            existing.live.reasoning_effort_options.as_ref(),
            next.live.reasoning_effort_options.as_ref(),
        ) {
            if !old.iter().any(|effort| new.contains(effort)) {
                existing.reasoning_conflict = true;
            }
        }
        existing.live.reasoning_effort_options = if existing.reasoning_conflict {
            None
        } else {
            match (
                existing.live.reasoning_effort_options.take(),
                next.live.reasoning_effort_options,
            ) {
                (Some(mut old), Some(new)) => {
                    old.retain(|effort| new.contains(effort));
                    Some(old)
                }
                (old, None) => old,
                (None, new) => new,
            }
        };
        existing.live.supported_parameters = match (
            existing.live.supported_parameters.take(),
            next.live.supported_parameters,
        ) {
            (Some(mut old), Some(new)) => {
                old.retain(|parameter| new.contains(parameter));
                Some(old)
            }
            (old, None) => old,
            (None, new) => new,
        };
        existing.live.display_name =
            merge_equal(existing.live.display_name.take(), next.live.display_name);
        existing.live.default_temperature = merge_equal(
            existing.live.default_temperature.take(),
            next.live.default_temperature,
        );
        existing.live.upstream_provider = merge_equal(
            existing.live.upstream_provider.take(),
            next.live.upstream_provider,
        );
        existing.live.api_mode = merge_equal(existing.live.api_mode.take(), next.live.api_mode);
        if existing.routed_model.is_some()
            && next.routed_model.is_some()
            && existing.routed_model != next.routed_model
        {
            existing.routed_model_conflict = true;
        }
        existing.routed_model = if existing.routed_model_conflict {
            None
        } else {
            merge_equal(existing.routed_model.take(), next.routed_model)
        };
        if existing.mode.is_some()
            && next.mode.is_some()
            && !existing
                .mode
                .as_ref()
                .unwrap()
                .eq_ignore_ascii_case(next.mode.as_ref().unwrap())
        {
            existing.mode_conflict = true;
        }
        let existing_responses = existing
            .mode
            .as_deref()
            .is_some_and(|mode| mode.eq_ignore_ascii_case("responses"));
        let next_responses = next
            .mode
            .as_deref()
            .is_some_and(|mode| mode.eq_ignore_ascii_case("responses"));
        if (existing.routing_mode_missing && next_responses)
            || (next.routing_mode_missing && existing_responses)
        {
            existing.mode_conflict = true;
        }
        existing.mode_conflict |= next.mode_conflict;
        existing.routing_mode_missing |= next.routing_mode_missing;
        existing.mode = if existing.mode_conflict {
            None
        } else {
            existing.mode.take().or(next.mode)
        };
    }

    fn rows(value: &Value) -> &[Value] {
        value
            .get("data")
            .and_then(Value::as_array)
            .or_else(|| value.as_array())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    fn has_rows_shape(value: &Value) -> bool {
        value.is_array() || value.get("data").is_some_and(Value::is_array)
    }

    fn public_aliases(models: &Value) -> Vec<String> {
        let mut seen = HashSet::new();
        Self::rows(models)
            .iter()
            .filter_map(|row| {
                let id = row
                    .get("id")
                    .or_else(|| row.get("model_name"))?
                    .as_str()?
                    .trim();
                (!id.is_empty() && seen.insert(id.to_string())).then(|| id.to_string())
            })
            .collect()
    }

    fn info_by_public_alias(
        info: Option<&Value>,
        aliases: &HashSet<&str>,
    ) -> HashMap<String, DeploymentMetadata> {
        let mut result = HashMap::new();
        for row in info.into_iter().flat_map(Self::rows) {
            let Some(alias) = row
                .get("model_name")
                .or_else(|| row.get("id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| aliases.contains(*s))
            else {
                continue;
            };
            let metadata = Self::parse_metadata(row);
            result
                .entry(alias.to_string())
                .and_modify(|old| Self::merge_metadata(old, metadata.clone()))
                .or_insert(metadata);
        }
        result
    }

    fn excluded_mode(mode: Option<&str>) -> bool {
        matches!(
            mode.map(|s| s.to_ascii_lowercase()).as_deref(),
            Some(
                "embedding"
                    | "embeddings"
                    | "rerank"
                    | "image_generation"
                    | "image"
                    | "audio"
                    | "speech"
                    | "transcription"
                    | "completion"
                    | "text_completion"
                    | "audio_speech"
                    | "audio_transcription"
            )
        )
    }

    fn clearly_non_chat_id(id: &str) -> bool {
        let normalized = id
            .to_ascii_lowercase()
            .replace('/', "-")
            .replace('_', "-")
            .replace('.', "-");
        normalized.split('-').any(|part| {
            matches!(
                part,
                "embedding"
                    | "embeddings"
                    | "embed"
                    | "rerank"
                    | "reranker"
                    | "image"
                    | "audio"
                    | "speech"
                    | "transcription"
                    | "tts"
            )
        })
    }

    fn available_from_discovery(
        &self,
        models_json: &Value,
        info_json: Option<&Value>,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let aliases = Self::public_aliases(models_json);
        let alias_set = aliases.iter().map(String::as_str).collect::<HashSet<_>>();
        let metadata = Self::info_by_public_alias(info_json, &alias_set);
        let configured_enabled = self
            .enabled_models
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let disabled = self
            .disabled_models
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let mut models = Vec::new();
        for id in aliases {
            let meta = metadata.get(&id).cloned().unwrap_or_else(|| {
                let mut meta = DeploymentMetadata::default();
                if id.to_ascii_lowercase().contains("sonnet-5") {
                    meta.live.reasoning_effort_options =
                        Some(vec!["high".to_string(), "xhigh".to_string()]);
                }
                meta
            });
            if meta.mode_conflict
                || meta.routed_model_conflict
                || Self::excluded_mode(meta.mode.as_deref())
                || (meta.mode.is_none() && Self::clearly_non_chat_id(&id))
            {
                continue;
            }
            let tokenizer_conflict = meta.tokenizer_conflict;
            let reasoning_conflict = meta.reasoning_conflict;
            let prompt_price_explicit = meta.prompt_price_explicit;
            let generated_price_explicit = meta.generated_price_explicit;
            let resolved = meta
                .routed_model
                .as_deref()
                .and_then(|model| resolve_model_caps(model_caps, model))
                .or_else(|| {
                    meta.live
                        .base_model
                        .as_deref()
                        .and_then(|model| resolve_model_caps(model_caps, model))
                })
                .or_else(|| resolve_model_caps(model_caps, &id));
            let mut live = meta.live;
            if let (Some(live_pricing), Some(catalog_pricing)) = (
                live.pricing.as_mut(),
                resolved
                    .as_ref()
                    .and_then(|resolved| resolved.caps.pricing.as_ref()),
            ) {
                if !prompt_price_explicit {
                    live_pricing.prompt = catalog_pricing.prompt;
                }
                if !generated_price_explicit {
                    live_pricing.generated = catalog_pricing.generated;
                }
                if live_pricing.cache_read.is_none() {
                    live_pricing.cache_read = catalog_pricing.cache_read;
                }
                if live_pricing.cache_creation.is_none() {
                    live_pricing.cache_creation = catalog_pricing.cache_creation;
                }
            }
            live.n_ctx = match (
                meta.total_context,
                meta.input_capacity,
                live.max_output_tokens,
            ) {
                (Some(total), _, _) => Some(total.min(MAX_DISCOVERED_CONTEXT_TOKENS)),
                (None, Some(input), Some(output)) => Some(
                    input
                        .saturating_add(output)
                        .min(MAX_DISCOVERED_CONTEXT_TOKENS),
                ),
                (None, Some(input), None) if resolved.is_none() => {
                    Some(input.min(MAX_DISCOVERED_CONTEXT_TOKENS))
                }
                _ => None,
            };
            live.max_output_tokens = live.max_output_tokens.map(|value| {
                value
                    .min(MAX_DISCOVERED_OUTPUT_TOKENS)
                    .min(live.n_ctx.unwrap_or(usize::MAX))
            });
            if meta
                .mode
                .as_deref()
                .is_some_and(|mode| mode.eq_ignore_ascii_case("responses"))
                || live
                    .api_mode
                    .as_deref()
                    .is_some_and(|mode| mode.eq_ignore_ascii_case("responses"))
            {
                live.wire_format_override = Some(WireFormat::OpenaiResponses);
                live.endpoint_override = Some(format!("{}/v1/responses", self.base_url()));
            }
            let is_enabled = !disabled.contains(id.as_str());
            let mut model = available_model_from_catalog_and_live(
                &id,
                resolved.as_ref().map(|resolved| &resolved.caps),
                &live,
                is_enabled,
                32_768,
            );
            if tokenizer_conflict {
                model.tokenizer = None;
                model.live_fields.tokenizer = Some(String::new());
            }
            if reasoning_conflict {
                model.reasoning_effort_options = None;
                model.live_fields.reasoning_effort_options = Some(Vec::new());
            }
            models.push(model);
        }
        merge_custom_models(&mut models, &self.custom_models, &configured_enabled);
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models
    }
}

fn normalize_base_url(endpoint: &str) -> String {
    let mut value = endpoint.trim().trim_end_matches('/').to_string();
    for suffix in [
        "/v1/chat/completions",
        "/v1/responses",
        "/v1/completions",
        "/v1/embeddings",
        "/v1/models",
        "/models",
        "/v1",
    ] {
        if value.ends_with(suffix) {
            value.truncate(value.len() - suffix.len());
            break;
        }
    }
    value.trim_end_matches('/').to_string()
}

#[async_trait]
impl ProviderTrait for LiteLLMProvider {
    fn name(&self) -> &str {
        "litellm"
    }
    fn display_name(&self) -> &str {
        "LiteLLM"
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
    fn supported_wire_formats(&self) -> Vec<WireFormat> {
        vec![
            WireFormat::OpenaiChatCompletions,
            WireFormat::OpenaiResponses,
        ]
    }
    fn model_filter_regex(&self) -> Option<&'static str> {
        None
    }

    fn provider_schema(&self) -> &'static str {
        r#"
fields:
  enabled:
    f_type: boolean
    f_desc: "Enable this LiteLLM provider and expose its enabled models in global model selectors"
    f_label: "Enable Provider"
    f_default: false
  endpoint:
    f_type: string_long
    f_desc: "LiteLLM Proxy base URL. Models and deployment metadata are discovered dynamically from the proxy."
    f_placeholder: "http://localhost:4000"
    f_label: "Endpoint"
    f_default: "http://localhost:4000"
  api_key:
    f_type: string_long
    f_desc: "LiteLLM Proxy API key (literal value or $ENV_VAR)"
    f_label: "API Key"
  admin_url:
    f_type: string_long
    f_desc: "Optional LiteLLM Proxy admin base URL. The /key/info query reports the configured admin key's accessible budget; it may not represent an inference key unless both use the same credential. HTTPS is required except for loopback HTTP."
    f_placeholder: "http://localhost:4000"
    f_label: "Admin URL"
    f_extra: true
  admin_api_key:
    f_type: string_long
    f_desc: "Optional static LiteLLM admin API key (literal value or $ENV_VAR), used only for budget queries"
    f_label: "Admin API Key"
    f_extra: true
  credential:
    f_type: string_long
    f_object: true
    f_desc: "Optional command-backed bearer credential. Clear API Key before enabling it. The output is never stored or logged."
    f_label: "Credential Command"
    f_placeholder: "type: command\ncommand: credential-helper\nargs: [token]"
    f_confirmation: true
    f_extra: true
  extra_headers:
    f_type: string_long
    f_desc: "Advanced JSON/YAML object of additional HTTP headers. Values are redacted when read back."
    f_label: "Extra Headers"
    f_extra: true
  supports_cache_control:
    f_type: boolean
    f_desc: "Allow cache-control fields for models routed through this proxy"
    f_label: "Enable Cache Control"
    f_default: false
    f_extra: true
description: |
  LiteLLM Proxy with dynamic discovery of public model aliases and safe model metadata.
available:
  on_your_laptop_possible: true
  when_isolated_possible: true
"#
    }

    fn provider_settings_apply(&mut self, yaml: serde_yaml::Value) -> Result<(), String> {
        let mut endpoint = self.endpoint.clone();
        let mut api_key = self.api_key.clone();
        let mut admin_url = self.admin_url.clone();
        let mut admin_api_key = self.admin_api_key.clone();
        let mut credential_spec = self.credential.clone();
        let mut enabled = self.enabled;
        let mut supports_cache_control = self.supports_cache_control;
        let mut extra_headers = self.extra_headers.clone();
        let mut enabled_models = self.enabled_models.clone();
        let mut disabled_models = self.disabled_models.clone();
        let mut custom_models = self.custom_models.clone();
        if let Some(value) = yaml.get("api_key").and_then(|v| v.as_str()) {
            if value != "***" {
                api_key = value.to_string();
            }
        }
        let admin_key_was_deliberately_set = match yaml.get("admin_api_key") {
            Some(value) if value.is_null() => true,
            Some(value) => value.as_str().is_some_and(|value| value != "***"),
            None => false,
        };
        if let Some(value) = yaml.get("admin_api_key") {
            admin_api_key = if value.is_null() {
                None
            } else if let Some(value) = value.as_str() {
                if value == "***" {
                    admin_api_key
                } else if value.trim().is_empty() {
                    None
                } else {
                    Some(value.to_string())
                }
            } else {
                return Err("admin_api_key must be a string".to_string());
            };
        }
        if let Some(value) = yaml.get("admin_url") {
            admin_url = if value.is_null() {
                None
            } else if let Some(value) = value.as_str() {
                if value.trim().is_empty() {
                    None
                } else {
                    Some(Self::validated_admin_url(value).map_err(|_| {
                        "admin_url must be HTTPS, except for localhost/127.0.0.1/[::1] HTTP, and must not contain userinfo or a fragment".to_string()
                    })?)
                }
            } else {
                return Err("admin_url must be a string".to_string());
            };
        }
        let admin_url_changed = match (self.admin_url.as_deref(), admin_url.as_deref()) {
            (Some(old), Some(new)) => Self::validated_admin_url(old).ok().as_deref() != Some(new),
            (None, None) => false,
            _ => true,
        };
        if admin_url_changed && !admin_key_was_deliberately_set {
            admin_api_key = None;
        }
        if let Some(value) = yaml.get("credential") {
            credential_spec = if value.is_null() {
                None
            } else {
                let spec: CredentialSpec = serde_yaml::from_value(value.clone())
                    .map_err(|_| "invalid credential configuration".to_string())?;
                spec.validate()?;
                Some(spec)
            };
        }
        if !api_key.trim().is_empty() && credential_spec.is_some() {
            return Err("api_key and credential are mutually exclusive".to_string());
        }
        if let Some(value) = yaml.get("endpoint").and_then(|v| v.as_str()) {
            let normalized = normalize_base_url(value);
            if normalized.is_empty() {
                return Err("endpoint must not be empty".to_string());
            }
            endpoint = normalized;
        }
        if let Some(value) = yaml.get("enabled").and_then(|v| v.as_bool()) {
            enabled = value;
        }
        if let Some(value) = yaml.get("supports_cache_control").and_then(|v| v.as_bool()) {
            supports_cache_control = value;
        }
        if let Some(value) = yaml.get("extra_headers") {
            let parsed = parse_extra_headers_value(value)?;
            extra_headers = extra_headers_mapping_to_hash_map(Some(&extra_headers), &parsed);
        }
        parse_enabled_models(&yaml, &mut enabled_models);
        if let Some(values) = yaml
            .get("disabled_models")
            .and_then(|value| value.as_sequence())
        {
            disabled_models = values
                .iter()
                .filter_map(|value| value.as_str())
                .map(str::to_string)
                .collect();
        }
        parse_custom_models(&yaml, &mut custom_models);

        self.endpoint = endpoint;
        self.api_key = api_key;
        self.admin_url = admin_url;
        self.admin_api_key = admin_api_key;
        self.credential = credential_spec;
        self.enabled = enabled;
        self.supports_cache_control = supports_cache_control;
        self.extra_headers = extra_headers;
        self.enabled_models = enabled_models;
        self.disabled_models = disabled_models;
        self.custom_models = custom_models;
        Ok(())
    }

    fn provider_settings_as_json(&self) -> Value {
        let headers = self
            .extra_headers
            .keys()
            .map(|key| (key.clone(), "***".to_string()))
            .collect::<HashMap<_, _>>();
        let admin_url = self.admin_url_for_settings();
        json!({
            "endpoint": self.endpoint, "api_key": if self.api_key.is_empty() { "" } else { "***" },
            "admin_url": admin_url, "admin_api_key": if self.admin_api_key.is_some() { "***" } else { "" },
            "credential": self.credential, "enabled": self.enabled,
            "enabled_models": self.enabled_models, "disabled_models": self.disabled_models,
            "custom_models": self.custom_models,
            "extra_headers": headers, "supports_cache_control": self.supports_cache_control
        })
    }

    fn build_runtime(&self) -> Result<ProviderRuntime, String> {
        if !self.api_key.trim().is_empty() && self.credential.is_some() {
            return Err("api_key and credential are mutually exclusive".into());
        }
        if let Some(spec) = &self.credential {
            spec.validate()?;
        }
        let base = self.base_url();
        Ok(ProviderRuntime {
            name: self.name().into(),
            display_name: self.display_name().into(),
            enabled: self.enabled && !base.is_empty(),
            readonly: false,
            wire_format: self.default_wire_format(),
            chat_endpoint: format!("{base}/v1/chat/completions"),
            completion_endpoint: format!("{base}/v1/completions"),
            embedding_endpoint: format!("{base}/v1/embeddings"),
            api_key: self.bearer_from_static(),
            auth_token: String::new(),
            tokenizer_api_key: String::new(),
            extra_headers: self.extra_headers.clone(),
            supports_cache_control: self.supports_cache_control,
            chat_models: Vec::new(),
            completion_models: Vec::new(),
            embedding_model: None,
        })
    }

    fn credential(&self) -> Option<&CredentialSpec> {
        self.credential.as_ref()
    }
    fn has_credentials(&self) -> bool {
        !self.endpoint.trim().is_empty()
    }
    fn model_source(&self) -> ModelSource {
        ModelSource::Api
    }
    fn enabled_models(&self) -> &[String] {
        &self.enabled_models
    }
    fn disabled_models(&self) -> &[String] {
        &self.disabled_models
    }
    fn custom_models(&self) -> &HashMap<String, CustomModelConfig> {
        &self.custom_models
    }
    fn set_model_enabled(&mut self, model_id: &str, enabled: bool) {
        set_model_enabled_impl(&mut self.enabled_models, model_id, enabled);
        if enabled {
            self.disabled_models.retain(|disabled| disabled != model_id);
        } else if !self
            .disabled_models
            .iter()
            .any(|disabled| disabled == model_id)
        {
            self.disabled_models.push(model_id.to_string());
        }
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
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        self.fetch_available_models_for_instance(self.name(), http_client, model_caps)
            .await
    }

    async fn fetch_available_models_for_instance(
        &self,
        instance_id: &str,
        http_client: &reqwest::Client,
        model_caps: &HashMap<String, ModelCapabilities>,
    ) -> Vec<AvailableModel> {
        let mut auth = match self.discovery_auth(instance_id).await {
            Ok(value) => value,
            Err(_) => return self.get_custom_models_only(),
        };
        let Some(models) = self
            .discover_with_fallback(http_client, &["/v1/models", "/models"], &mut auth, true)
            .await
        else {
            return self.get_custom_models_only();
        };
        let info = tokio::time::timeout(
            METADATA_ENRICHMENT_TIMEOUT,
            self.discover_with_fallback(
                http_client,
                &["/v1/model/info", "/model/info"],
                &mut auth,
                true,
            ),
        )
        .await
        .ok()
        .flatten();
        self.available_from_discovery(&models, info.as_ref(), model_caps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_and_config_validate_and_redact() {
        let mut provider = LiteLLMProvider::default();
        assert!(provider
            .provider_schema()
            .contains("f_label: \"Enable Provider\""));
        assert!(provider.provider_schema().contains("f_confirmation: true"));
        assert!(provider
            .provider_schema()
            .contains("discovered dynamically"));
        provider
            .provider_settings_apply(
                serde_yaml::from_str("api_key: secret\nextra_headers:\n  X-Token: hidden\n")
                    .unwrap(),
            )
            .unwrap();
        let settings = provider.provider_settings_as_json();
        assert_eq!(settings["api_key"], "***");
        assert_eq!(settings["extra_headers"]["X-Token"], "***");
        let error = provider
            .provider_settings_apply(
                serde_yaml::from_str("credential:\n  type: command\n  command: helper\n").unwrap(),
            )
            .unwrap_err();
        assert!(error.contains("mutually exclusive"));
    }

    #[test]
    fn endpoint_normalization_and_runtime_paths() {
        let mut provider = LiteLLMProvider::default();
        provider
            .provider_settings_apply(
                serde_yaml::from_str("endpoint: http://host:4000/v1/chat/completions\n").unwrap(),
            )
            .unwrap();
        assert_eq!(provider.endpoint, "http://host:4000");
        let runtime = provider.build_runtime().unwrap();
        assert_eq!(
            runtime.chat_endpoint,
            "http://host:4000/v1/chat/completions"
        );
        assert_eq!(
            runtime.completion_endpoint,
            "http://host:4000/v1/completions"
        );
        assert_eq!(runtime.embedding_endpoint, "http://host:4000/v1/embeddings");
    }

    #[test]
    fn legacy_config_and_masked_admin_key_are_compatible_for_same_url() {
        let legacy: LiteLLMProvider = serde_yaml::from_str(
            "endpoint: http://proxy:4000\napi_key: inference\nenabled: false\n",
        )
        .unwrap();
        assert!(legacy.admin_url.is_none() && legacy.admin_api_key.is_none());

        let mut provider = LiteLLMProvider::default();
        provider
            .provider_settings_apply(
                serde_yaml::from_str(
                    "admin_url: https://admin.example/v1/\nadmin_api_key: admin-secret\n",
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(provider.admin_url.as_deref(), Some("https://admin.example"));
        assert_eq!(provider.provider_settings_as_json()["admin_api_key"], "***");
        provider
            .provider_settings_apply(
                serde_yaml::from_str("admin_url: https://admin.example/v1\nadmin_api_key: '***'\n")
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(provider.admin_api_key.as_deref(), Some("admin-secret"));
        assert_eq!(provider.admin_url.as_deref(), Some("https://admin.example"));
        assert!(provider.provider_schema().contains("Admin API Key"));
        assert!(provider
            .provider_schema()
            .contains("may not represent an inference key"));
        assert_eq!(
            provider.provider_schema().matches("f_extra: true").count(),
            5
        );
    }

    #[test]
    fn admin_url_policy_accepts_https_and_loopback_http_only() {
        for value in [
            "https://admin.example",
            "https://admin.example:8443/proxy/v1",
            "http://localhost:4000",
            "http://127.0.0.1:4000/v1",
            "http://[::1]:4000/v1/",
        ] {
            assert!(
                LiteLLMProvider::validated_admin_url(value).is_ok(),
                "expected accepted URL: {value}"
            );
        }
        for value in [
            "http://admin.example",
            "ftp://admin.example",
            "not a url",
            "https://",
            "https://user@admin.example",
            "https://user:password@admin.example",
            "https://admin.example/#fragment",
        ] {
            assert_eq!(
                LiteLLMProvider::validated_admin_url(value),
                Err(LiteLLMBudgetError::InvalidAdminUrl),
                "expected rejected URL: {value}"
            );
        }
    }

    #[test]
    fn admin_key_is_not_rebound_when_admin_url_changes() {
        let mut provider = LiteLLMProvider {
            admin_url: Some("https://old.example".into()),
            admin_api_key: Some("admin-secret".into()),
            ..Default::default()
        };
        provider
            .provider_settings_apply(
                serde_yaml::from_str("admin_url: https://new.example\nadmin_api_key: '***'\n")
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(provider.admin_url.as_deref(), Some("https://new.example"));
        assert!(provider.admin_api_key.is_none());

        provider.admin_api_key = Some("second-secret".into());
        provider
            .provider_settings_apply(
                serde_yaml::from_str("admin_url: https://third.example\n").unwrap(),
            )
            .unwrap();
        assert!(provider.admin_api_key.is_none());

        provider
            .provider_settings_apply(
                serde_yaml::from_str(
                    "admin_url: https://fourth.example\nadmin_api_key: deliberate-secret\n",
                )
                .unwrap(),
            )
            .unwrap();
        assert_eq!(provider.admin_api_key.as_deref(), Some("deliberate-secret"));
    }

    #[test]
    fn settings_never_expose_admin_url_userinfo() {
        let provider = LiteLLMProvider {
            admin_url: Some("https://username:password@admin.example/v1".into()),
            ..Default::default()
        };
        let settings = provider.provider_settings_as_json();
        assert_eq!(settings["admin_url"], "https://admin.example");
        assert!(!settings.to_string().contains("username"));
        assert!(!settings.to_string().contains("password"));

        let mut provider = LiteLLMProvider::default();
        assert!(provider
            .provider_settings_apply(
                serde_yaml::from_str("admin_url: https://username@admin.example\n").unwrap()
            )
            .is_err());
    }

    #[test]
    fn budget_response_normalizes_numbers_and_unavailable_cases() {
        assert_eq!(
            LiteLLMProvider::parse_budget_response(
                &json!({"info":{"spend":"12.5","max_budget":100,"remaining":"87.5"}}),
            ),
            LiteLLMBudgetQueryResult::Available(LiteLLMBudget {
                spend: 12.5,
                max_budget: 100.0,
                remaining: Some(87.5),
            })
        );
        assert_eq!(
            LiteLLMProvider::parse_budget_response(&json!({"spend":1})),
            LiteLLMBudgetQueryResult::Unavailable(LiteLLMBudgetUnavailable::BudgetNotConfigured)
        );
        assert_eq!(
            LiteLLMProvider::parse_budget_response(
                &json!({"spend":"not-a-number","max_budget":"100"}),
            ),
            LiteLLMBudgetQueryResult::Error(LiteLLMBudgetError::InvalidResponse)
        );
    }

    #[tokio::test]
    async fn budget_query_uses_proxy_root_and_static_admin_bearer() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 4096];
            let size = socket.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..size]);
            assert!(request.starts_with("GET /key/info "));
            assert!(request
                .to_ascii_lowercase()
                .contains("authorization: bearer admin-only"));
            assert!(!request.contains("inference-only"));
            let body = r#"{"spend":"2.5","max_budget":"10","remaining":7.5}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let provider = LiteLLMProvider {
            api_key: "inference-only".into(),
            admin_url: Some(format!("http://{address}/v1")),
            admin_api_key: Some("admin-only".into()),
            ..Default::default()
        };
        assert_eq!(
            provider.query_budget(&reqwest::Client::new()).await,
            LiteLLMBudgetQueryResult::Available(LiteLLMBudget {
                spend: 2.5,
                max_budget: 10.0,
                remaining: Some(7.5),
            })
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn budget_query_is_unavailable_without_complete_opt_in() {
        let client = reqwest::Client::new();
        assert_eq!(
            LiteLLMProvider::default().query_budget(&client).await,
            LiteLLMBudgetQueryResult::Unavailable(LiteLLMBudgetUnavailable::NotConfigured)
        );
        let provider = LiteLLMProvider {
            admin_url: Some("http://localhost:1".into()),
            ..Default::default()
        };
        assert_eq!(
            provider.query_budget(&client).await,
            LiteLLMBudgetQueryResult::Unavailable(LiteLLMBudgetUnavailable::NotConfigured)
        );
    }

    #[tokio::test]
    async fn rejected_admin_url_returns_typed_error_without_request() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let provider = LiteLLMProvider {
            admin_url: Some(format!("http://user@{address}")),
            admin_api_key: Some("admin-secret".into()),
            ..Default::default()
        };

        assert_eq!(
            provider.query_budget(&reqwest::Client::new()).await,
            LiteLLMBudgetQueryResult::Error(LiteLLMBudgetError::InvalidAdminUrl)
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(25), listener.accept())
                .await
                .is_err()
        );
    }

    #[test]
    fn filters_info_to_public_aliases_and_deduplicates_deployments() {
        let provider = LiteLLMProvider::default();
        let models = json!({"data": [{"id":"public"}, {"id":"public"}]});
        let info = json!({"data": [
            {"model_name":"public", "model_info":{"max_input_tokens":8192}},
            {"model_name":"public", "model_info":{"max_input_tokens":4096}},
            {"model_name":"private", "model_info":{"max_input_tokens":999999}}
        ]});
        let result = provider.available_from_discovery(&models, Some(&info), &HashMap::new());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "public");
        assert_eq!(result[0].n_ctx, 4096);
    }

    #[test]
    fn maps_metadata_pricing_capabilities_and_modes() {
        let provider = LiteLLMProvider::default();
        let models = json!({"data":[{"id":"response-model"},{"id":"embedding-model"}]});
        let info = json!({"data":[
            {"model_name":"response-model", "model_info":{
                "mode":"responses", "max_input_tokens":100000, "max_output_tokens":12000,
                "input_cost_per_token":"0.000002", "output_cost_per_token":0.000006,
                "supports_function_calling":true, "supports_parallel_function_calling":true,
                "supports_response_schema":true, "supports_vision":true,
                "supports_prompt_caching":true,
                "supported_openai_params":["tools","parallel_tool_calls","response_format","reasoning_effort"],
                "supports_low_reasoning_effort":true, "supports_high_reasoning_effort":true,
                "supports_xhigh_reasoning_effort":true,
                "base_model":"openai/gpt-5", "tokenizer":"cl100k_base"
            }},
            {"model_name":"embedding-model", "model_info":{"mode":"embedding"}}
        ]});
        let result = provider.available_from_discovery(&models, Some(&info), &HashMap::new());
        assert_eq!(result.len(), 1);
        let model = &result[0];
        assert_eq!(model.n_ctx, 112000);
        assert_eq!(model.max_output_tokens, Some(12000));
        assert_eq!(model.pricing.as_ref().unwrap().prompt, 2.0);
        assert_eq!(model.pricing.as_ref().unwrap().generated, 6.0);
        assert!(
            model.supports_tools && model.supports_parallel_tools && model.supports_strict_tools
        );
        assert!(model.supports_multimodality && model.supports_cache_control);
        assert_eq!(
            model.reasoning_effort_options.as_deref(),
            Some(&["low".to_string(), "high".to_string(), "xhigh".to_string()][..])
        );
        assert_eq!(
            model.wire_format_override,
            Some(WireFormat::OpenaiResponses)
        );
        assert_eq!(
            model.endpoint_override.as_deref(),
            Some("http://localhost:4000/v1/responses")
        );
    }

    #[test]
    fn discovers_row_level_reasoning_efforts_exactly() {
        let provider = LiteLLMProvider::default();
        let models = json!({"data":[{"id":"reasoning-model"}]});
        let info = json!({"data":[{
            "model_name":"reasoning-model",
            "supported_reasoning_efforts":["high","xhigh"],
            "supports_prompt_caching":true,
            "model_info":{
                "mode":"chat",
                "supported_reasoning_efforts":["low","medium"],
                "supports_prompt_caching":false
            }
        }]});

        let model = provider
            .available_from_discovery(&models, Some(&info), &HashMap::new())
            .pop()
            .unwrap();

        assert_eq!(
            model.reasoning_effort_options.as_deref(),
            Some(&["high".to_string(), "xhigh".to_string()][..])
        );
        assert!(model.supports_cache_control);
    }

    #[test]
    fn row_level_reasoning_opt_out_overrides_nested_reasoning_capabilities() {
        let provider = LiteLLMProvider::default();
        let model = provider
            .available_from_discovery(
                &json!({"data":[{"id":"reasoning-model"}]}),
                Some(&json!({"data":[{
                    "model_name":"reasoning-model",
                    "supports_reasoning":false,
                    "model_info":{
                        "supports_reasoning":true,
                        "supported_reasoning_efforts":["low","high"],
                        "supports_thinking":true,
                        "supports_adaptive_thinking":true,
                        "max_thinking_tokens":4096
                    }
                }]})),
                &HashMap::new(),
            )
            .pop()
            .unwrap();

        assert_eq!(model.reasoning_effort_options.as_deref(), Some(&[][..]));
        assert_eq!(model.live_fields.supports_thinking_budget, Some(false));
        assert_eq!(
            model.live_fields.supports_adaptive_thinking_budget,
            Some(false)
        );
    }

    #[test]
    fn nested_reasoning_opt_out_disables_thinking_capabilities() {
        let metadata = LiteLLMProvider::parse_metadata(&json!({
            "model_name":"reasoning-model",
            "model_info":{
                "supports_reasoning":false,
                "supports_thinking":true,
                "supports_adaptive_thinking":true,
                "max_thinking_tokens":4096
            }
        }));

        assert_eq!(
            metadata.live.reasoning_effort_options.as_deref(),
            Some(&[][..])
        );
        assert_eq!(metadata.live.supports_thinking_budget, Some(false));
        assert_eq!(metadata.live.supports_adaptive_thinking_budget, Some(false));
    }

    #[test]
    fn row_level_thinking_capabilities_override_nested_metadata() {
        let metadata = LiteLLMProvider::parse_metadata(&json!({
            "model_name":"reasoning-model",
            "supports_thinking_budget":false,
            "supports_adaptive_thinking_budget":false,
            "max_thinking_budget":2048,
            "model_info":{
                "supports_thinking":true,
                "supports_adaptive_thinking":true,
                "max_thinking_tokens":4096
            }
        }));

        assert_eq!(metadata.live.supports_thinking_budget, Some(false));
        assert_eq!(metadata.live.supports_adaptive_thinking_budget, Some(false));
        assert_eq!(metadata.live.max_thinking_tokens, Some(2048));
    }

    #[test]
    fn row_level_supported_parameters_override_nested_generic_metadata() {
        let provider = LiteLLMProvider::default();
        let model = provider
            .available_from_discovery(
                &json!({"data":[{"id":"reasoning-model"}]}),
                Some(&json!({"data":[{
                    "model_name":"reasoning-model",
                    "supported_openai_params":["reasoning_effort"],
                    "model_info":{"supported_openai_params":["tools"]}
                }]})),
                &HashMap::new(),
            )
            .pop()
            .unwrap();

        assert_eq!(
            model.reasoning_effort_options.as_deref(),
            Some(&["low".to_string(), "medium".to_string(), "high".to_string()][..])
        );
        assert_eq!(
            model.supported_parameters.as_deref(),
            Some(&["reasoning_effort".to_string()][..])
        );
        assert!(!model.supports_tools);
    }

    #[test]
    fn canonical_supported_parameters_skip_invalid_candidates_consistently() {
        let metadata = LiteLLMProvider::parse_metadata(&json!({
            "model_name":"reasoning-model",
            "supported_parameters":null,
            "supported_openai_params":["reasoning_effort"],
            "model_info":{
                "supported_parameters":["tools"],
                "supported_openai_params":["temperature"]
            }
        }));

        assert_eq!(
            metadata.live.supported_parameters.as_deref(),
            Some(&["reasoning_effort".to_string()][..])
        );
        assert_eq!(
            metadata.live.reasoning_effort_options.as_deref(),
            Some(&["low".to_string(), "medium".to_string(), "high".to_string()][..])
        );
        assert_eq!(metadata.live.supports_tools, Some(false));

        let metadata = LiteLLMProvider::parse_metadata(&json!({
            "supported_parameters":"invalid",
            "supported_openai_params":null,
            "model_info":{
                "supported_parameters":"invalid",
                "supported_openai_params":["tools"]
            }
        }));
        assert_eq!(
            metadata.live.supported_parameters.as_deref(),
            Some(&["tools".to_string()][..])
        );
        assert_eq!(metadata.live.supports_tools, Some(true));
    }

    #[test]
    fn canonical_supported_parameters_prefer_row_and_generic_arrays() {
        let metadata = LiteLLMProvider::parse_metadata(&json!({
            "supported_parameters":["tools"],
            "supported_openai_params":["reasoning_effort"],
            "model_info":{
                "supported_parameters":["temperature"],
                "supported_openai_params":["max_completion_tokens"]
            }
        }));

        assert_eq!(
            metadata.live.supported_parameters.as_deref(),
            Some(&["tools".to_string()][..])
        );
        assert_eq!(metadata.live.supports_tools, Some(true));
        assert!(metadata.live.reasoning_effort_options.is_none());
    }

    #[test]
    fn sonnet_five_defaults_to_high_and_xhigh_reasoning_efforts() {
        let provider = LiteLLMProvider::default();
        let model = provider
            .available_from_discovery(
                &json!({"data":[{"id":"sonnet-5"}]}),
                Some(&json!({"data":[{
                    "model_name":"sonnet-5",
                    "supports_reasoning":true,
                    "model_info":{"mode":"chat"}
                }]})),
                &HashMap::new(),
            )
            .pop()
            .unwrap();

        assert_eq!(
            model.reasoning_effort_options.as_deref(),
            Some(&["high".to_string(), "xhigh".to_string()][..])
        );
    }

    #[test]
    fn sonnet_five_identifiers_default_without_reasoning_advertisement() {
        for row in [
            json!({"model_name":"team-sonnet-5-alias","supported_parameters":["tools"]}),
            json!({"model_name":"alias","litellm_params":{"model":"anthropic/sonnet-5-latest"}}),
            json!({"model_name":"alias","model_info":{"base_model":"claude-sonnet-5-preview"}}),
        ] {
            assert_eq!(
                LiteLLMProvider::parse_metadata(&row)
                    .live
                    .reasoning_effort_options
                    .as_deref(),
                Some(&["high".to_string(), "xhigh".to_string()][..])
            );
        }
    }

    #[test]
    fn sonnet_five_detection_checks_id_and_model_name_independently() {
        let metadata = LiteLLMProvider::parse_metadata(&json!({
            "id":"unrelated-model",
            "model_name":"team-sonnet-5-alias",
            "supported_parameters":["tools"]
        }));

        assert_eq!(
            metadata.live.reasoning_effort_options.as_deref(),
            Some(&["high".to_string(), "xhigh".to_string()][..])
        );
    }

    #[test]
    fn sonnet_five_respects_explicit_reasoning_data_and_row_false() {
        for (row, expected) in [
            (
                json!({"model_name":"sonnet-5-alias","reasoning_effort_options":[]}),
                Vec::<String>::new(),
            ),
            (
                json!({"model_name":"sonnet-5-alias","supported_reasoning_efforts":["medium"]}),
                vec!["medium".to_string()],
            ),
            (
                json!({
                    "model_name":"sonnet-5-alias",
                    "supports_reasoning":false,
                    "model_info":{"supports_reasoning":true}
                }),
                Vec::<String>::new(),
            ),
            (
                json!({
                    "model_name":"sonnet-5-alias",
                    "supports_high_reasoning_effort":false,
                    "model_info":{"supports_high_reasoning_effort":true}
                }),
                Vec::<String>::new(),
            ),
        ] {
            assert_eq!(
                LiteLLMProvider::parse_metadata(&row)
                    .live
                    .reasoning_effort_options,
                Some(expected)
            );
        }
    }

    #[test]
    fn metadata_free_sonnet_five_defaults_to_high_and_xhigh_reasoning_efforts() {
        let provider = LiteLLMProvider::default();
        let model = provider
            .available_from_discovery(
                &json!({"data":[{"id":"team-sonnet-5-latest"}]}),
                Some(&json!({"data":[{
                    "model_name":"another-model",
                    "reasoning_effort_options":["low"]
                }]})),
                &HashMap::new(),
            )
            .pop()
            .unwrap();

        assert_eq!(
            model.reasoning_effort_options.as_deref(),
            Some(&["high".to_string(), "xhigh".to_string()][..])
        );
    }

    #[test]
    fn missing_info_keeps_unknown_public_model_available_and_merges_custom() {
        let mut provider = LiteLLMProvider::default();
        provider.custom_models.insert(
            "custom".into(),
            CustomModelConfig {
                n_ctx: Some(2048),
                ..Default::default()
            },
        );
        let result = provider.available_from_discovery(
            &json!({"data":[{"id":"unknown"}]}),
            None,
            &HashMap::new(),
        );
        assert_eq!(
            result.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["custom", "unknown"]
        );
        assert!(
            !result
                .iter()
                .find(|m| m.id == "unknown")
                .unwrap()
                .supports_tools
        );
        assert!(result.iter().find(|m| m.id == "unknown").unwrap().enabled);

        provider.set_model_enabled("unknown", false);
        let result = provider.available_from_discovery(
            &json!({"data":[{"id":"unknown"}]}),
            None,
            &HashMap::new(),
        );
        assert!(!result.iter().find(|m| m.id == "unknown").unwrap().enabled);
        assert_eq!(
            provider.provider_settings_as_json()["disabled_models"],
            json!(["unknown"])
        );
    }

    #[test]
    fn catalog_fields_survive_missing_live_fields_and_input_capacity_alone() {
        let provider = LiteLLMProvider::default();
        let caps = HashMap::from([(
            "openai/base".to_string(),
            ModelCapabilities {
                n_ctx: 128_000,
                max_output_tokens: 16_000,
                supports_tools: true,
                tokenizer: "openai".to_string(),
                ..Default::default()
            },
        )]);
        let models = json!({"data":[{"id":"alias"}]});
        let info = json!({"data":[{"model_name":"alias","litellm_params":{"model":"openai/base"},"model_info":{"max_input_tokens":64_000}}]});

        let model = provider
            .available_from_discovery(&models, Some(&info), &caps)
            .pop()
            .unwrap();

        assert_eq!(model.n_ctx, 128_000);
        assert_eq!(model.max_output_tokens, Some(16_000));
        assert!(model.supports_tools);
        assert_eq!(model.tokenizer.as_deref(), Some("openai"));
    }

    #[test]
    fn explicit_false_live_fields_override_catalog_true() {
        let provider = LiteLLMProvider::default();
        let caps = HashMap::from([(
            "base".to_string(),
            ModelCapabilities {
                n_ctx: 32_000,
                supports_tools: true,
                supports_parallel_tools: true,
                supports_strict_tools: true,
                supports_vision: true,
                supports_temperature: true,
                supports_cache_control: true,
                ..Default::default()
            },
        )]);
        let info = json!({"data":[{"model_name":"alias","model_info":{
            "base_model":"base", "supports_function_calling":false,
            "supports_parallel_function_calling":false, "supports_response_schema":false,
            "supports_vision":false, "supports_temperature":false,
            "supports_prompt_caching":false
        }}]});
        let model = provider
            .available_from_discovery(&json!({"data":[{"id":"alias"}]}), Some(&info), &caps)
            .pop()
            .unwrap();

        assert!(!model.supports_tools);
        assert!(!model.supports_parallel_tools);
        assert!(!model.supports_strict_tools);
        assert!(!model.supports_multimodality);
        assert!(!model.supports_temperature);
        assert!(!model.supports_cache_control);
    }

    #[test]
    fn conflicting_deployment_routing_is_excluded_and_metadata_is_cleared() {
        let provider = LiteLLMProvider::default();
        let models = json!({"data":[{"id":"mode-conflict"},{"id":"route-conflict"},{"id":"metadata-conflict"}]});
        let info = json!({"data":[
            {"model_name":"mode-conflict","model_info":{"mode":"chat"}},
            {"model_name":"mode-conflict","model_info":{"mode":"embedding"}},
            {"model_name":"route-conflict","litellm_params":{"model":"a"},"model_info":{"mode":"chat"}},
            {"model_name":"route-conflict","litellm_params":{"model":"b"},"model_info":{"mode":"chat"}},
            {"model_name":"metadata-conflict","model_info":{"base_model":"a","tokenizer":"tok-a","reasoning_effort_options":["low"]}},
            {"model_name":"metadata-conflict","model_info":{"base_model":"b","tokenizer":"tok-b","reasoning_effort_options":["high"]}}
        ]});

        let result = provider.available_from_discovery(&models, Some(&info), &HashMap::new());

        assert_eq!(result.len(), 1);
        let model = &result[0];
        assert_eq!(model.id, "metadata-conflict");
        assert!(model.base_model.is_none());
        assert!(model.tokenizer.is_none());
        assert!(model.reasoning_effort_options.is_none());
    }

    #[test]
    fn extracts_real_litellm_names_into_typed_live_fields() {
        let provider = LiteLLMProvider::default();
        let info = json!({"data":[{"model_name":"real","litellm_params":{
            "model":"anthropic/claude", "custom_llm_provider":"anthropic"
        },"model_info":{
            "mode":"responses", "max_input_tokens":1000, "max_output_tokens":200,
            "cache_read_input_token_cost":0.000001, "cache_creation_input_token_cost":0.000002,
            "supports_computer_use":true, "supports_reasoning":true,
            "supported_reasoning_efforts":["low","high"], "supports_thinking":true,
            "supports_adaptive_thinking":true, "max_thinking_tokens":4096,
            "default_temperature":0.2, "default_max_tokens":123,
            "supports_web_search":true,
            "supported_openai_params":["tools","temperature","max_completion_tokens"]
        }}]});
        let model = provider
            .available_from_discovery(
                &json!({"data":[{"id":"real"}]}),
                Some(&info),
                &HashMap::new(),
            )
            .pop()
            .unwrap();

        assert_eq!(model.n_ctx, 1200);
        assert!(model.supports_clicks && model.supports_tools);
        assert!(model.supports_thinking_budget && model.supports_adaptive_thinking_budget);
        assert_eq!(model.max_thinking_tokens, Some(4096));
        assert_eq!(model.default_temperature, Some(0.2));
        assert_eq!(model.default_max_tokens, Some(123));
        assert!(model.supports_web_search && model.supports_max_completion_tokens);
        assert_eq!(model.upstream_provider.as_deref(), Some("anthropic"));
        assert_eq!(model.api_mode.as_deref(), Some("responses"));
        assert_eq!(
            model.wire_format_override,
            Some(WireFormat::OpenaiResponses)
        );
        assert_eq!(
            model.supported_parameters.as_deref(),
            Some(
                &[
                    "tools".to_string(),
                    "temperature".to_string(),
                    "max_completion_tokens".to_string()
                ][..]
            )
        );
    }

    #[test]
    fn tokenizer_validation_accepts_identifiers_and_rejects_secret_shapes() {
        for tokenizer in ["tiktoken", "my-tokenizer", "cl100k_base"] {
            assert_eq!(
                LiteLLMProvider::parse_metadata(&json!({"model_info":{"tokenizer":tokenizer}}))
                    .live
                    .tokenizer
                    .as_deref(),
                Some(tokenizer)
            );
        }
        for tokenizer in ["token=secret", "provider?key=secret", "Bearer abc123"] {
            assert!(LiteLLMProvider::parse_metadata(
                &json!({"model_info":{"tokenizer":tokenizer}})
            )
            .live
            .tokenizer
            .is_none());
        }
    }

    #[test]
    fn pricing_rejects_invalid_components_and_allows_catalog_fallback() {
        let provider = LiteLLMProvider::default();
        let caps = HashMap::from([(
            "base".to_string(),
            ModelCapabilities {
                pricing: Some(ModelPricing {
                    prompt: 3.0,
                    generated: 7.0,
                    ..Default::default()
                }),
                ..Default::default()
            },
        )]);
        let info = json!({"data":[{"model_name":"alias","model_info":{
            "base_model":"base", "input_cost_per_token":null,
            "output_cost_per_token":"invalid", "cache_read_input_token_cost":-1,
            "cache_creation_input_token_cost":"NaN"
        }}]});
        let model = provider
            .available_from_discovery(&json!({"data":[{"id":"alias"}]}), Some(&info), &caps)
            .pop()
            .unwrap();
        let pricing = model.pricing.unwrap();
        assert_eq!(pricing.prompt, 3.0);
        assert_eq!(pricing.generated, 7.0);
        assert!(pricing.cache_read.is_none() && pricing.cache_creation.is_none());
    }

    #[test]
    fn routing_modes_are_collected_from_all_locations_and_ambiguity_is_excluded() {
        let provider = LiteLLMProvider::default();
        let models = json!({"data":[
            {"id":"params-response"}, {"id":"internal-conflict"},
            {"id":"api-conflict"}, {"id":"response-plus-missing"}
        ]});
        let info = json!({"data":[
            {"model_name":"params-response","litellm_params":{"api_mode":"responses"}},
            {"model_name":"internal-conflict","mode":"chat","model_info":{"api_mode":"responses"}},
            {"model_name":"api-conflict","model_info":{"api_mode":"chat"}},
            {"model_name":"api-conflict","litellm_params":{"api_mode":"responses"}},
            {"model_name":"response-plus-missing","model_info":{"mode":"responses"}},
            {"model_name":"response-plus-missing","model_info":{"max_input_tokens":1000}}
        ]});
        let result = provider.available_from_discovery(&models, Some(&info), &HashMap::new());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "params-response");
        assert_eq!(
            result[0].wire_format_override,
            Some(WireFormat::OpenaiResponses)
        );
    }

    #[test]
    fn missing_metadata_filters_obvious_non_chat_names_but_keeps_unknown_chat_candidates() {
        let provider = LiteLLMProvider::default();
        let models = json!({"data":[
            {"id":"text-embedding-3-small"}, {"id":"rerank-v3"},
            {"id":"image_generation"}, {"id":"audio/tts-1"}, {"id":"company-model-v2"}
        ]});
        let result = provider.available_from_discovery(&models, None, &HashMap::new());
        assert_eq!(
            result
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["company-model-v2"]
        );
    }

    #[test]
    fn explicit_total_context_wins_and_clamps_output() {
        let provider = LiteLLMProvider::default();
        let info = json!({"data":[{"model_name":"alias","model_info":{
            "context_length":8000, "max_input_tokens":10000, "max_output_tokens":9000
        }}]});
        let model = provider
            .available_from_discovery(
                &json!({"data":[{"id":"alias"}]}),
                Some(&info),
                &HashMap::new(),
            )
            .pop()
            .unwrap();
        assert_eq!(model.n_ctx, 8000);
        assert_eq!(model.max_output_tokens, Some(8000));
    }

    #[test]
    fn conflict_clearing_preserves_explicit_empty_live_provenance() {
        let provider = LiteLLMProvider::default();
        let caps = HashMap::from([(
            "alias".to_string(),
            ModelCapabilities {
                tokenizer: "catalog-tokenizer".into(),
                reasoning_effort_options: Some(vec!["medium".into()]),
                ..Default::default()
            },
        )]);
        let info = json!({"data":[
            {"model_name":"alias","model_info":{"tokenizer":"one","reasoning_effort_options":["low"]}},
            {"model_name":"alias","model_info":{"tokenizer":"two","reasoning_effort_options":["high"]}}
        ]});
        let model = provider
            .available_from_discovery(&json!({"data":[{"id":"alias"}]}), Some(&info), &caps)
            .pop()
            .unwrap();
        assert!(model.tokenizer.is_none() && model.reasoning_effort_options.is_none());
        assert_eq!(model.live_fields.tokenizer.as_deref(), Some(""));
        assert_eq!(
            model.live_fields.reasoning_effort_options.as_deref(),
            Some(&[][..])
        );
    }

    #[test]
    fn settings_apply_is_transactional_and_uses_explicit_opt_outs() {
        let mut provider = LiteLLMProvider::default();
        provider.endpoint = "http://original".into();
        provider.enabled = false;
        let error = provider
            .provider_settings_apply(
                serde_yaml::from_str(
                    "endpoint: http://changed\nenabled: true\nextra_headers: []\n",
                )
                .unwrap(),
            )
            .unwrap_err();
        assert!(!error.is_empty());
        assert_eq!(provider.endpoint, "http://original");
        assert!(!provider.enabled);

        provider
            .provider_settings_apply(serde_yaml::from_str("enabled_models: [allowed]\n").unwrap())
            .unwrap();
        let models = provider.available_from_discovery(
            &json!({"data":[{"id":"allowed"},{"id":"other"}]}),
            None,
            &HashMap::new(),
        );
        assert!(
            models
                .iter()
                .find(|model| model.id == "allowed")
                .unwrap()
                .enabled
        );
        assert!(
            models
                .iter()
                .find(|model| model.id == "other")
                .unwrap()
                .enabled
        );

        provider.set_model_enabled("other", false);
        let disabled_models = provider.available_from_discovery(
            &json!({"data":[{"id":"allowed"},{"id":"other"}]}),
            None,
            &HashMap::new(),
        );
        assert!(
            disabled_models
                .iter()
                .find(|model| model.id == "allowed")
                .unwrap()
                .enabled
        );
        assert!(
            !disabled_models
                .iter()
                .find(|model| model.id == "other")
                .unwrap()
                .enabled
        );
        assert_eq!(provider.disabled_models, vec!["other".to_string()]);
    }

    #[test]
    fn discovery_redacts_reflected_bearer_from_exposed_metadata() {
        let secret = "sk-reflected-live-secret";
        let mut value = json!({
            "data": [{
                "model_name": "alias",
                "model_info": {
                    "display_name": format!("model {secret}"),
                    "tokenizer": secret,
                    "mode": "chat"
                }
            }]
        });
        LiteLLMProvider::redact_json_strings(&mut value, secret);

        let provider = LiteLLMProvider::default();
        let models = provider.available_from_discovery(
            &json!({"data":[{"id":"alias"}]}),
            Some(&value),
            &HashMap::new(),
        );
        let serialized = serde_json::to_string(&models).unwrap();
        assert!(!serialized.contains(secret));
    }

    #[tokio::test]
    async fn model_discovery_falls_back_after_malformed_success_shape() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for (path, status, body) in [
                ("/v1/models", "200 OK", r#"{"object":"list"}"#),
                ("/models", "200 OK", r#"[{"id":"fallback-chat"}]"#),
                ("/v1/model/info", "404 Not Found", "{}"),
                ("/model/info", "404 Not Found", "{}"),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0_u8; 4096];
                let size = socket.read(&mut request).await.unwrap();
                assert!(
                    String::from_utf8_lossy(&request[..size]).starts_with(&format!("GET {path} "))
                );
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let provider = LiteLLMProvider {
            endpoint: format!("http://{address}"),
            ..Default::default()
        };
        let models = provider
            .fetch_available_models(&reqwest::Client::new(), &HashMap::new())
            .await;
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "fallback-chat");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn discovery_rejects_oversized_response_before_json_parsing() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0_u8; 1024];
            socket.read(&mut request).await.unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                MAX_DISCOVERY_BODY_BYTES + 1
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let provider = LiteLLMProvider::default();
        let mut auth = provider.discovery_auth("oversized-test").await.unwrap();
        let error = provider
            .get_json_with_refresh(
                &reqwest::Client::new(),
                &format!("http://{address}/models"),
                &mut auth,
            )
            .await
            .unwrap_err();
        assert!(error.contains("too large"));
        server.await.unwrap();
    }

    #[cfg(unix)]
    fn incrementing_credential(counter: &std::path::Path) -> CredentialSpec {
        CredentialSpec::Command {
            command: "sh".to_string(),
            args: vec![
                "-c".to_string(),
                format!(
                    "n=$(cat '{}' 2>/dev/null || echo 0); n=$((n+1)); printf %s $n > '{}'; printf %s $n",
                    counter.display(),
                    counter.display()
                ),
            ],
            timeout_ms: 5_000,
            refresh_interval_ms: 300_000,
            cwd: None,
            env_passthrough: Vec::new(),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn discovery_credentials_are_isolated_by_provider_instance() {
        let temp = tempfile::tempdir().unwrap();
        let counter = temp.path().join("count");
        let provider = LiteLLMProvider {
            credential: Some(incrementing_credential(&counter)),
            ..Default::default()
        };
        let first_id = format!("litellm_first_{}", uuid::Uuid::new_v4());
        let second_id = format!("litellm_second_{}", uuid::Uuid::new_v4());

        assert_eq!(
            provider.discovery_auth(&first_id).await.unwrap().bearer,
            "1"
        );
        assert_eq!(
            provider.discovery_auth(&second_id).await.unwrap().bearer,
            "2"
        );

        credential::invalidate(&first_id);
        credential::invalidate(&second_id);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn discovery_refreshes_command_credential_only_once_across_fallbacks() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for (expected_path, expected_token, status, body) in [
                ("/v1/models", "1", "401 Unauthorized", "{}"),
                (
                    "/v1/models",
                    "2",
                    "200 OK",
                    r#"{"object":"list","data":[{"id":"chat-model"}]}"#,
                ),
                ("/v1/model/info", "2", "401 Unauthorized", "{}"),
                ("/model/info", "2", "401 Unauthorized", "{}"),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0_u8; 8_192];
                let size = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..size]);
                assert!(request.starts_with(&format!("GET {expected_path} ")));
                assert!(request
                    .to_ascii_lowercase()
                    .contains(&format!("authorization: bearer {expected_token}")));
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });

        let temp = tempfile::tempdir().unwrap();
        let counter = temp.path().join("count");
        let provider_id = format!("litellm_refresh_{}", uuid::Uuid::new_v4());
        let provider = LiteLLMProvider {
            endpoint: format!("http://{address}"),
            credential: Some(incrementing_credential(&counter)),
            ..Default::default()
        };

        let models = provider
            .fetch_available_models_for_instance(
                &provider_id,
                &reqwest::Client::new(),
                &HashMap::new(),
            )
            .await;

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "chat-model");
        assert_eq!(tokio::fs::read_to_string(&counter).await.unwrap(), "2");
        server.await.unwrap();
        credential::invalidate(&provider_id);
    }

    #[test]
    fn accepts_command_credential_and_registry_constructs_provider() {
        let mut provider = LiteLLMProvider::default();
        provider
            .provider_settings_apply(
                serde_yaml::from_str(
                    "credential:\n  type: command\n  command: helper\n  args: [token]\n",
                )
                .unwrap(),
            )
            .unwrap();
        assert!(matches!(
            provider.credential,
            Some(CredentialSpec::Command { .. })
        ));
        let registered = crate::create_provider("litellm").unwrap();
        assert_eq!(registered.name(), "litellm");
    }
}
