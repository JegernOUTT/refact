#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use reqwest::Url;

use crate::caps::model_caps::model_caps_from_models_dev_catalog;
use crate::caps::models_dev::{
    model_cost_to_pricing, ModelsDevCatalog, ModelsDevModel, ModelsDevModelProvider,
    ModelsDevProvider,
};
use crate::llm::adapter::WireFormat;
use crate::providers::config::is_legacy_refact_model;
use crate::providers::traits::{merge_custom_models, AvailableModel, CustomModelConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelsDevProviderFamily {
    Qwen,
    Kimi,
    Zai,
    MiniMax,
    GitHubCopilot,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelsDevEndpointSource {
    Catalog,
    UserConfigured,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelsDevProviderConfig {
    pub provider_id: String,
    pub family: ModelsDevProviderFamily,
    pub endpoint_override: Option<String>,
    pub wire_format_override: Option<WireFormat>,
    pub user_allowed_hosts: Vec<String>,
}

impl ModelsDevProviderConfig {
    pub fn new(provider_id: impl Into<String>, family: ModelsDevProviderFamily) -> Self {
        Self {
            provider_id: provider_id.into(),
            family,
            endpoint_override: None,
            wire_format_override: None,
            user_allowed_hosts: Vec::new(),
        }
    }

    pub fn with_endpoint_override(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint_override = Some(endpoint.into());
        self
    }

    pub fn with_wire_format_override(mut self, wire_format: WireFormat) -> Self {
        self.wire_format_override = Some(wire_format);
        self
    }

    pub fn with_user_allowed_hosts(mut self, hosts: Vec<String>) -> Self {
        self.user_allowed_hosts = hosts;
        self
    }
}

pub fn resolve_models_dev_provider<'a>(
    catalog: &'a ModelsDevCatalog,
    provider_id: &str,
) -> Option<(&'a str, &'a ModelsDevProvider)> {
    if let Some((key, provider)) = catalog.get_key_value(provider_id) {
        return Some((key.as_str(), provider));
    }

    catalog
        .iter()
        .find(|(_, provider)| provider.id == provider_id)
        .map(|(key, provider)| (key.as_str(), provider))
}

pub fn build_models_dev_available_models(
    catalog: &ModelsDevCatalog,
    config: &ModelsDevProviderConfig,
    enabled_models: &[String],
    custom_models: &HashMap<String, CustomModelConfig>,
) -> Result<Vec<AvailableModel>, String> {
    let enabled_set: HashSet<&str> = enabled_models.iter().map(|s| s.as_str()).collect();
    let mut models = Vec::new();

    if let Some((provider_key, provider)) =
        resolve_models_dev_provider(catalog, &config.provider_id)
    {
        let model_caps = match model_caps_from_models_dev_catalog(catalog) {
            Ok(model_caps) => model_caps,
            Err(e) if e.contains("produced no model capabilities") => HashMap::new(),
            Err(e) => return Err(e),
        };
        let provider_aliases = provider_aliases(provider_key, provider);
        for (model_key, model) in &provider.models {
            let model_aliases = model_aliases(model_key, model);
            let Some(caps) = active_model_caps(&model_caps, &provider_aliases, &model_aliases)
            else {
                continue;
            };
            let model_id = model.id.trim();
            if model_id.is_empty() || is_legacy_refact_model(model_id) {
                continue;
            }
            let enabled = is_model_enabled(&enabled_set, &provider_aliases, &model_aliases);
            let pricing = model_cost_to_pricing(model).or_else(|| caps.pricing.clone());
            let mut available = AvailableModel::from_caps(model_id, caps, enabled, pricing);
            available.display_name = non_empty_string(&model.name);
            available.wire_format_override = model_wire_format(provider, model, config);
            available.endpoint_override = model_endpoint_override(provider, model, config)?;
            models.push(available);
        }
    }

    merge_custom_models(&mut models, custom_models, &enabled_set);
    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(models)
}

pub fn models_dev_runtime_endpoint(
    catalog: &ModelsDevCatalog,
    config: &ModelsDevProviderConfig,
) -> Result<String, String> {
    let wire_format = models_dev_provider_wire_format(catalog, config);
    let (api, source) = if let Some(endpoint) = config.endpoint_override.as_deref() {
        (endpoint, ModelsDevEndpointSource::UserConfigured)
    } else {
        let (_, provider) =
            resolve_models_dev_provider(catalog, &config.provider_id).ok_or_else(|| {
                format!(
                    "models.dev provider '{}' is missing; configure an explicit endpoint",
                    config.provider_id
                )
            })?;
        let api = provider.api.as_deref().ok_or_else(|| {
            format!(
                "models.dev provider '{}' has no api endpoint; configure an explicit endpoint",
                config.provider_id
            )
        })?;
        (api, ModelsDevEndpointSource::Catalog)
    };
    let endpoint = derive_models_dev_endpoint(api, wire_format);
    validate_models_dev_endpoint(&endpoint, config.family, source, &config.user_allowed_hosts)?;
    Ok(endpoint)
}

pub fn models_dev_provider_wire_format(
    catalog: &ModelsDevCatalog,
    config: &ModelsDevProviderConfig,
) -> WireFormat {
    if let Some(wire_format) = config.wire_format_override {
        return wire_format;
    }

    resolve_models_dev_provider(catalog, &config.provider_id)
        .and_then(|(_, provider)| {
            infer_models_dev_wire_format(provider.npm.as_deref(), provider.api.as_deref())
        })
        .unwrap_or(WireFormat::OpenaiChatCompletions)
}

pub fn infer_models_dev_wire_format(npm: Option<&str>, api: Option<&str>) -> Option<WireFormat> {
    if let Some(api) = api {
        let api_lower = api.to_ascii_lowercase();
        if api_lower.ends_with("/api/chat") {
            return Some(WireFormat::OllamaNative);
        }
        if api_lower.ends_with("/messages") || api_lower.contains("/anthropic/") {
            return Some(WireFormat::AnthropicMessages);
        }
        if api_lower.ends_with("/responses") {
            return Some(WireFormat::OpenaiResponses);
        }
        if api_lower.ends_with("/chat/completions") || api_lower.contains("openai") {
            return Some(WireFormat::OpenaiChatCompletions);
        }
    }

    let npm = npm?.to_ascii_lowercase();
    if npm.contains("anthropic") {
        Some(WireFormat::AnthropicMessages)
    } else if npm.contains("responses") {
        Some(WireFormat::OpenaiResponses)
    } else if npm.contains("openai") {
        Some(WireFormat::OpenaiChatCompletions)
    } else {
        None
    }
}

pub fn derive_models_dev_endpoint(api: &str, wire_format: WireFormat) -> String {
    let trimmed = api.trim().trim_end_matches('/');
    if is_complete_endpoint(trimmed) {
        return trimmed.to_string();
    }
    let suffix = match wire_format {
        WireFormat::OpenaiChatCompletions => "chat/completions",
        WireFormat::OpenaiResponses => "responses",
        WireFormat::AnthropicMessages => "messages",
        WireFormat::OllamaNative => "api/chat",
    };
    format!("{trimmed}/{suffix}")
}

pub fn validate_models_dev_endpoint(
    endpoint: &str,
    family: ModelsDevProviderFamily,
    source: ModelsDevEndpointSource,
    user_allowed_hosts: &[String],
) -> Result<(), String> {
    let url = Url::parse(endpoint)
        .map_err(|e| format!("Invalid models.dev endpoint '{endpoint}': {e}"))?;
    if url.scheme() != "https" {
        return Err(format!(
            "models.dev endpoint '{endpoint}' must use https before credentials are sent"
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!(
            "models.dev endpoint '{endpoint}' must not include userinfo before credentials are sent"
        ));
    }
    if url.query().is_some() {
        return Err(format!(
            "models.dev endpoint '{endpoint}' must not include a query string before credentials are sent"
        ));
    }
    if url.fragment().is_some() {
        return Err(format!(
            "models.dev endpoint '{endpoint}' must not include a fragment before credentials are sent"
        ));
    }
    if url.port().is_some_and(|port| port != 443) {
        return Err(format!(
            "models.dev endpoint '{endpoint}' must not use a non-default port before credentials are sent"
        ));
    }
    let host = normalized_host(&url).ok_or_else(|| {
        format!("models.dev endpoint '{endpoint}' has no hostname and cannot be trusted")
    })?;
    if is_local_or_private_host(&host) {
        return Err(format!(
            "models.dev endpoint '{endpoint}' resolves to local or private host '{host}' and cannot receive credentials"
        ));
    }

    if family == ModelsDevProviderFamily::Unknown && source == ModelsDevEndpointSource::Catalog {
        return Err(format!(
            "models.dev catalog endpoint host '{host}' is not trusted for unknown provider; configure an explicit endpoint"
        ));
    }

    let allowed = allowed_hosts(family);
    let user_allowed = user_allowed_hosts
        .iter()
        .map(|host| normalize_host_for_compare(host))
        .any(|allowed_host| allowed_host == host);
    if !allowed.is_empty() && !allowed.contains(&host.as_str()) && !user_allowed {
        return Err(format!(
            "models.dev endpoint host '{host}' is not allowlisted for provider family {:?}",
            family
        ));
    }

    Ok(())
}

fn model_wire_format(
    provider: &ModelsDevProvider,
    model: &ModelsDevModel,
    config: &ModelsDevProviderConfig,
) -> Option<WireFormat> {
    if config.wire_format_override.is_some() {
        return None;
    }
    model
        .provider
        .as_ref()
        .and_then(|provider_profile| {
            infer_models_dev_wire_format(
                provider_profile.npm.as_deref(),
                provider_profile.api.as_deref(),
            )
        })
        .or_else(|| infer_models_dev_wire_format(provider.npm.as_deref(), provider.api.as_deref()))
}

fn model_endpoint_override(
    provider: &ModelsDevProvider,
    model: &ModelsDevModel,
    config: &ModelsDevProviderConfig,
) -> Result<Option<String>, String> {
    if config.endpoint_override.is_some() {
        return Ok(None);
    }
    let Some(model_provider) = model.provider.as_ref() else {
        return Ok(None);
    };
    let Some(api) = model_provider.api.as_deref() else {
        return Ok(None);
    };
    if !is_absolute_url(api) {
        return Ok(None);
    }
    let wire_format = model_wire_format_from_profile(provider, model_provider, config);
    let endpoint = api.trim().trim_end_matches('/').to_string();
    let endpoint = if is_complete_endpoint(&endpoint) {
        endpoint
    } else {
        derive_models_dev_endpoint(&endpoint, wire_format)
    };
    validate_models_dev_endpoint(
        &endpoint,
        config.family,
        ModelsDevEndpointSource::Catalog,
        &config.user_allowed_hosts,
    )?;
    Ok(Some(endpoint))
}

fn model_wire_format_from_profile(
    provider: &ModelsDevProvider,
    model_provider: &ModelsDevModelProvider,
    config: &ModelsDevProviderConfig,
) -> WireFormat {
    config
        .wire_format_override
        .or_else(|| {
            infer_models_dev_wire_format(
                model_provider.npm.as_deref(),
                model_provider.api.as_deref(),
            )
        })
        .or_else(|| infer_models_dev_wire_format(provider.npm.as_deref(), provider.api.as_deref()))
        .unwrap_or(WireFormat::OpenaiChatCompletions)
}

fn active_model_caps<'a>(
    model_caps: &'a HashMap<String, crate::caps::model_caps::ModelCapabilities>,
    provider_aliases: &[String],
    model_aliases: &[String],
) -> Option<&'a crate::caps::model_caps::ModelCapabilities> {
    for provider_alias in provider_aliases {
        for model_alias in model_aliases {
            let key = format!("{provider_alias}/{model_alias}");
            if let Some(caps) = model_caps.get(&key) {
                return Some(caps);
            }
        }
    }

    for model_alias in model_aliases {
        if let Some(caps) = model_caps.get(model_alias) {
            return Some(caps);
        }
    }

    None
}

fn is_model_enabled(
    enabled_set: &HashSet<&str>,
    provider_aliases: &[String],
    model_aliases: &[String],
) -> bool {
    for model_alias in model_aliases {
        if enabled_set.contains(model_alias.as_str()) {
            return true;
        }
        for provider_alias in provider_aliases {
            let qualified = format!("{provider_alias}/{model_alias}");
            if enabled_set.contains(qualified.as_str()) {
                return true;
            }
        }
    }
    false
}

fn provider_aliases(provider_key: &str, provider: &ModelsDevProvider) -> Vec<String> {
    unique_non_empty_aliases([
        provider_key.to_string(),
        provider.id.clone(),
        provider_key.replace('-', "_"),
        provider.id.replace('-', "_"),
    ])
}

fn model_aliases(model_key: &str, model: &ModelsDevModel) -> Vec<String> {
    unique_non_empty_aliases([model_key.to_string(), model.id.clone()])
}

fn unique_non_empty_aliases<const N: usize>(aliases: [String; N]) -> Vec<String> {
    let mut seen = HashSet::new();
    aliases
        .into_iter()
        .filter(|alias| !alias.trim().is_empty())
        .filter(|alias| seen.insert(alias.clone()))
        .collect()
}

fn non_empty_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn is_complete_endpoint(endpoint: &str) -> bool {
    [
        "/chat/completions",
        "/responses",
        "/messages",
        "/api/chat",
        "/v1/chat/completions",
        "/v1/responses",
        "/v1/messages",
    ]
    .iter()
    .any(|suffix| endpoint.ends_with(suffix))
}

fn is_absolute_url(value: &str) -> bool {
    Url::parse(value)
        .map(|url| matches!(url.scheme(), "http" | "https"))
        .unwrap_or(false)
}

fn normalized_host(url: &Url) -> Option<String> {
    url.host_str().map(normalize_host_for_compare)
}

fn normalize_host_for_compare(host: &str) -> String {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    host.strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(&host)
        .to_string()
}

fn allowed_hosts(family: ModelsDevProviderFamily) -> &'static [&'static str] {
    match family {
        ModelsDevProviderFamily::Qwen => &[
            "dashscope.aliyuncs.com",
            "dashscope-intl.aliyuncs.com",
            "coding-intl.dashscope.aliyuncs.com",
            "coding.dashscope.aliyuncs.com",
        ],
        ModelsDevProviderFamily::Kimi => &["api.moonshot.ai", "api.moonshot.cn"],
        ModelsDevProviderFamily::Zai => &["api.z.ai", "open.bigmodel.cn"],
        ModelsDevProviderFamily::MiniMax => &["api.minimax.io", "api.minimaxi.com"],
        ModelsDevProviderFamily::GitHubCopilot => &["api.githubcopilot.com"],
        ModelsDevProviderFamily::Unknown => &[],
    }
}

fn is_local_or_private_host(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    IpAddr::from_str(host)
        .map(|ip| match ip {
            IpAddr::V4(ip) => is_private_ipv4(ip),
            IpAddr::V6(ip) => is_private_ipv6(ip),
        })
        .unwrap_or(false)
}

fn is_private_ipv4(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || octets[0] == 0
        || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        || (octets[0] == 198 && (18..=19).contains(&octets[1]))
}

fn is_private_ipv6(ip: Ipv6Addr) -> bool {
    ip.to_ipv4_mapped().map_or(false, is_private_ipv4)
        || ip.is_loopback()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        || ip.is_unspecified()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::models_dev::{
        ModelsDevCost, ModelsDevLimit, ModelsDevModalities, ModelsDevModelProvider,
    };
    use crate::providers::traits::ModelPricing;

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

    fn catalog_with_provider(provider_id: &str, provider: ModelsDevProvider) -> ModelsDevCatalog {
        HashMap::from([(provider_id.to_string(), provider)])
    }

    fn openai_provider(api: &str) -> ModelsDevProvider {
        ModelsDevProvider {
            id: "alibaba".to_string(),
            name: "Alibaba".to_string(),
            api: Some(api.to_string()),
            npm: Some("@ai-sdk/openai-compatible".to_string()),
            models: HashMap::from([("qwen-max".to_string(), text_chat_model("qwen-max"))]),
            ..Default::default()
        }
    }

    fn anthropic_provider(api: &str) -> ModelsDevProvider {
        ModelsDevProvider {
            id: "minimax".to_string(),
            name: "MiniMax".to_string(),
            api: Some(api.to_string()),
            npm: Some("@ai-sdk/anthropic".to_string()),
            models: HashMap::from([("MiniMax-M2".to_string(), text_chat_model("MiniMax-M2"))]),
            ..Default::default()
        }
    }

    #[test]
    fn models_dev_provider_endpoint_derivation_for_supported_wire_formats() {
        let cases = [
            (
                "https://dashscope.aliyuncs.com/v1",
                WireFormat::OpenaiChatCompletions,
                "https://dashscope.aliyuncs.com/v1/chat/completions",
            ),
            (
                "https://dashscope.aliyuncs.com/v1/",
                WireFormat::OpenaiChatCompletions,
                "https://dashscope.aliyuncs.com/v1/chat/completions",
            ),
            (
                "https://dashscope.aliyuncs.com/compatible-mode/v1/",
                WireFormat::OpenaiChatCompletions,
                "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions",
            ),
            (
                "https://api.openai.com/v1",
                WireFormat::OpenaiResponses,
                "https://api.openai.com/v1/responses",
            ),
            (
                "https://api.openai.com/v1/responses",
                WireFormat::OpenaiResponses,
                "https://api.openai.com/v1/responses",
            ),
            (
                "https://api.openai.com/v1/chat/completions",
                WireFormat::OpenaiChatCompletions,
                "https://api.openai.com/v1/chat/completions",
            ),
            (
                "https://api.minimax.io/v1",
                WireFormat::AnthropicMessages,
                "https://api.minimax.io/v1/messages",
            ),
            (
                "https://api.minimax.io/anthropic/v1",
                WireFormat::AnthropicMessages,
                "https://api.minimax.io/anthropic/v1/messages",
            ),
            (
                "https://ollama.example.com",
                WireFormat::OllamaNative,
                "https://ollama.example.com/api/chat",
            ),
            (
                "https://ollama.example.com/api/chat",
                WireFormat::OllamaNative,
                "https://ollama.example.com/api/chat",
            ),
        ];

        for (api, wire_format, expected) in cases {
            assert_eq!(derive_models_dev_endpoint(api, wire_format), expected);
        }
    }

    #[test]
    fn models_dev_provider_runtime_endpoint_avoids_double_v1() {
        let catalog = catalog_with_provider(
            "alibaba",
            openai_provider("https://dashscope.aliyuncs.com/compatible-mode/v1"),
        );
        let config = ModelsDevProviderConfig::new("alibaba", ModelsDevProviderFamily::Qwen);

        let endpoint = models_dev_runtime_endpoint(&catalog, &config).unwrap();

        assert_eq!(
            endpoint,
            "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions"
        );
        assert!(!endpoint.contains("/v1/v1/"));
    }

    #[test]
    fn models_dev_provider_custom_model_merge_and_enabled_state() {
        let catalog = catalog_with_provider(
            "alibaba",
            openai_provider("https://dashscope.aliyuncs.com/compatible-mode/v1"),
        );
        let config = ModelsDevProviderConfig::new("alibaba", ModelsDevProviderFamily::Qwen);
        let mut custom_models = HashMap::new();
        custom_models.insert(
            "qwen-custom".to_string(),
            CustomModelConfig {
                n_ctx: Some(4096),
                supports_tools: Some(true),
                pricing: Some(ModelPricing {
                    prompt: 1.0,
                    generated: 2.0,
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        custom_models.insert(
            "qwen-max".to_string(),
            CustomModelConfig {
                n_ctx: Some(64_000),
                ..Default::default()
            },
        );
        let enabled = vec!["qwen-max".to_string(), "qwen-custom".to_string()];

        let models =
            build_models_dev_available_models(&catalog, &config, &enabled, &custom_models).unwrap();

        let qwen_max = models.iter().find(|model| model.id == "qwen-max").unwrap();
        assert!(qwen_max.enabled);
        assert!(qwen_max.is_custom);
        assert_eq!(qwen_max.n_ctx, 64_000);
        let custom = models
            .iter()
            .find(|model| model.id == "qwen-custom")
            .unwrap();
        assert!(custom.enabled);
        assert!(custom.is_custom);
        assert_eq!(custom.pricing.as_ref().unwrap().generated, 2.0);
    }

    #[test]
    fn models_dev_provider_missing_provider_returns_custom_only_list() {
        let catalog = ModelsDevCatalog::new();
        let config = ModelsDevProviderConfig::new("doubao", ModelsDevProviderFamily::Unknown);
        let mut custom_models = HashMap::new();
        custom_models.insert(
            "doubao-custom".to_string(),
            CustomModelConfig {
                n_ctx: Some(8192),
                ..Default::default()
            },
        );
        let enabled = vec!["doubao-custom".to_string()];

        let models =
            build_models_dev_available_models(&catalog, &config, &enabled, &custom_models).unwrap();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "doubao-custom");
        assert!(models[0].enabled);
    }

    #[test]
    fn models_dev_provider_pricing_is_copied_from_catalog_cost() {
        let mut model = text_chat_model("qwen-max");
        model.cost = Some(ModelsDevCost {
            input: Some(1.6),
            output: Some(6.4),
            cache_read: Some(0.8),
            cache_write: Some(2.4),
            ..Default::default()
        });
        let provider = ModelsDevProvider {
            id: "alibaba".to_string(),
            name: "Alibaba".to_string(),
            api: Some("https://dashscope.aliyuncs.com/compatible-mode/v1".to_string()),
            npm: Some("@ai-sdk/openai-compatible".to_string()),
            models: HashMap::from([("qwen-max".to_string(), model)]),
            ..Default::default()
        };
        let catalog = catalog_with_provider("alibaba", provider);
        let config = ModelsDevProviderConfig::new("alibaba", ModelsDevProviderFamily::Qwen);

        let models =
            build_models_dev_available_models(&catalog, &config, &[], &HashMap::new()).unwrap();

        let pricing = models[0].pricing.as_ref().unwrap();
        assert_eq!(pricing.prompt, 1.6);
        assert_eq!(pricing.generated, 6.4);
        assert_eq!(pricing.cache_read, Some(0.8));
        assert_eq!(pricing.cache_creation, Some(2.4));
    }

    #[test]
    fn models_dev_provider_model_level_api_override_takes_precedence() {
        let mut model = text_chat_model("qwen-max");
        model.provider = Some(ModelsDevModelProvider {
            api: Some(
                "https://dashscope.aliyuncs.com/model-specific/v1/chat/completions".to_string(),
            ),
            npm: Some("@ai-sdk/openai-compatible".to_string()),
        });
        let provider = ModelsDevProvider {
            id: "alibaba".to_string(),
            name: "Alibaba".to_string(),
            api: Some("https://dashscope-intl.aliyuncs.com/compatible-mode/v1".to_string()),
            npm: Some("@ai-sdk/openai-compatible".to_string()),
            models: HashMap::from([("qwen-max".to_string(), model)]),
            ..Default::default()
        };
        let catalog = catalog_with_provider("alibaba", provider);
        let config = ModelsDevProviderConfig::new("alibaba", ModelsDevProviderFamily::Qwen);

        let models =
            build_models_dev_available_models(&catalog, &config, &[], &HashMap::new()).unwrap();

        assert_eq!(
            models[0].endpoint_override.as_deref(),
            Some("https://dashscope.aliyuncs.com/model-specific/v1/chat/completions")
        );
        assert_eq!(
            models_dev_runtime_endpoint(&catalog, &config).unwrap(),
            "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/chat/completions"
        );
    }

    #[test]
    fn models_dev_provider_model_level_full_responses_endpoint_remains_final() {
        let mut model = text_chat_model("qwen-max");
        model.provider = Some(ModelsDevModelProvider {
            api: Some("https://dashscope.aliyuncs.com/model-specific/v1/responses".to_string()),
            npm: Some("@ai-sdk/openai-responses".to_string()),
        });
        let provider = ModelsDevProvider {
            id: "alibaba".to_string(),
            name: "Alibaba".to_string(),
            api: Some("https://dashscope.aliyuncs.com/compatible-mode/v1".to_string()),
            npm: Some("@ai-sdk/openai-compatible".to_string()),
            models: HashMap::from([("qwen-max".to_string(), model)]),
            ..Default::default()
        };
        let catalog = catalog_with_provider("alibaba", provider);
        let config = ModelsDevProviderConfig::new("alibaba", ModelsDevProviderFamily::Qwen);

        let models =
            build_models_dev_available_models(&catalog, &config, &[], &HashMap::new()).unwrap();

        assert_eq!(
            models[0].endpoint_override.as_deref(),
            Some("https://dashscope.aliyuncs.com/model-specific/v1/responses")
        );
        assert_eq!(
            models[0].wire_format_override,
            Some(WireFormat::OpenaiResponses)
        );
    }

    #[test]
    fn models_dev_provider_wire_format_inference_is_deterministic() {
        assert_eq!(
            infer_models_dev_wire_format(Some("@ai-sdk/openai-compatible"), None),
            Some(WireFormat::OpenaiChatCompletions)
        );
        assert_eq!(
            infer_models_dev_wire_format(Some("@ai-sdk/anthropic"), None),
            Some(WireFormat::AnthropicMessages)
        );
        assert_eq!(
            infer_models_dev_wire_format(None, Some("https://api.example.com/v1/responses")),
            Some(WireFormat::OpenaiResponses)
        );
        assert_eq!(
            infer_models_dev_wire_format(None, Some("https://ollama.example.com/api/chat")),
            Some(WireFormat::OllamaNative)
        );
        let provider = anthropic_provider("https://api.minimax.io/anthropic/v1");
        let catalog = catalog_with_provider("minimax", provider);
        let config = ModelsDevProviderConfig::new("minimax", ModelsDevProviderFamily::MiniMax);
        assert_eq!(
            models_dev_provider_wire_format(&catalog, &config),
            WireFormat::AnthropicMessages
        );
    }

    #[test]
    fn models_dev_provider_endpoint_validation_rejects_untrusted_catalog_urls() {
        for endpoint in [
            "http://dashscope.aliyuncs.com/v1/chat/completions",
            "https://localhost/v1/chat/completions",
            "https://127.0.0.1/v1/chat/completions",
            "https://192.168.1.10/v1/chat/completions",
            "https://evil.example.com/v1/chat/completions",
        ] {
            assert!(
                validate_models_dev_endpoint(
                    endpoint,
                    ModelsDevProviderFamily::Qwen,
                    ModelsDevEndpointSource::Catalog,
                    &[],
                )
                .is_err(),
                "{endpoint} should be rejected"
            );
        }
        assert!(validate_models_dev_endpoint(
            "https://api.example.com/v1/chat/completions",
            ModelsDevProviderFamily::Unknown,
            ModelsDevEndpointSource::Catalog,
            &[],
        )
        .unwrap_err()
        .contains("unknown provider"));
    }

    #[test]
    fn models_dev_provider_endpoint_validation_rejects_tricky_url_forms() {
        let cases = [
            (
                ModelsDevProviderFamily::Kimi,
                "https://api.moonshot.ai@evil.example/v1/chat/completions",
                "userinfo",
            ),
            (
                ModelsDevProviderFamily::Kimi,
                "https://api.moonshot.ai.evil.example/v1/chat/completions",
                "allowlisted",
            ),
            (
                ModelsDevProviderFamily::Qwen,
                "https://dashscope.aliyuncs.com.evil.example/v1/chat/completions",
                "allowlisted",
            ),
            (
                ModelsDevProviderFamily::Kimi,
                "https://api.moonshot.ai:8443/v1/chat/completions",
                "non-default port",
            ),
            (
                ModelsDevProviderFamily::Kimi,
                "https://api.moonshot.ai/v1/chat/completions?target=evil",
                "query string",
            ),
            (
                ModelsDevProviderFamily::Kimi,
                "https://api.moonshot.ai/v1/chat/completions#fragment",
                "fragment",
            ),
            (
                ModelsDevProviderFamily::Qwen,
                "https://10.0.0.1/v1/chat/completions",
                "local or private",
            ),
            (
                ModelsDevProviderFamily::Qwen,
                "https://172.16.0.1/v1/chat/completions",
                "local or private",
            ),
            (
                ModelsDevProviderFamily::Qwen,
                "https://169.254.1.1/v1/chat/completions",
                "local or private",
            ),
            (
                ModelsDevProviderFamily::Qwen,
                "https://[::1]/v1/chat/completions",
                "local or private",
            ),
            (
                ModelsDevProviderFamily::Qwen,
                "https://[fc00::1]/v1/chat/completions",
                "local or private",
            ),
            (
                ModelsDevProviderFamily::Qwen,
                "https://[::ffff:127.0.0.1]/v1/chat/completions",
                "local or private",
            ),
        ];

        for (family, endpoint, expected) in cases {
            let error = validate_models_dev_endpoint(
                endpoint,
                family,
                ModelsDevEndpointSource::Catalog,
                &[],
            )
            .unwrap_err();
            assert!(
                error.contains(expected),
                "{endpoint} should fail with '{expected}', got '{error}'"
            );
        }
    }

    #[test]
    fn models_dev_provider_endpoint_validation_accepts_known_hosts() {
        let cases = [
            (
                ModelsDevProviderFamily::Qwen,
                "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions",
            ),
            (
                ModelsDevProviderFamily::Qwen,
                "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/chat/completions",
            ),
            (
                ModelsDevProviderFamily::Qwen,
                "https://coding-intl.dashscope.aliyuncs.com/v1/chat/completions",
            ),
            (
                ModelsDevProviderFamily::Qwen,
                "https://coding.dashscope.aliyuncs.com/v1/chat/completions",
            ),
            (
                ModelsDevProviderFamily::Kimi,
                "https://api.moonshot.ai/v1/chat/completions",
            ),
            (
                ModelsDevProviderFamily::Kimi,
                "https://api.moonshot.ai:443/v1/chat/completions",
            ),
            (
                ModelsDevProviderFamily::Kimi,
                "https://api.moonshot.cn/v1/chat/completions",
            ),
            (
                ModelsDevProviderFamily::Zai,
                "https://api.z.ai/api/paas/v4/chat/completions",
            ),
            (
                ModelsDevProviderFamily::Zai,
                "https://open.bigmodel.cn/api/paas/v4/chat/completions",
            ),
            (
                ModelsDevProviderFamily::MiniMax,
                "https://api.minimax.io/anthropic/v1/messages",
            ),
            (
                ModelsDevProviderFamily::MiniMax,
                "https://api.minimaxi.com/anthropic/v1/messages",
            ),
            (
                ModelsDevProviderFamily::GitHubCopilot,
                "https://api.githubcopilot.com/chat/completions",
            ),
        ];
        for (family, endpoint) in cases {
            validate_models_dev_endpoint(endpoint, family, ModelsDevEndpointSource::Catalog, &[])
                .unwrap();
        }
    }

    #[test]
    fn models_dev_provider_filters_inactive_non_chat_and_deprecated_models() {
        let mut deprecated = text_chat_model("deprecated-chat");
        deprecated.status = Some("deprecated".to_string());
        let mut image = text_chat_model("image-output");
        image.modalities = Some(ModelsDevModalities {
            input: vec!["text".to_string()],
            output: vec!["image".to_string()],
        });
        let mut embedding = text_chat_model("text-embedding-3-large");
        embedding.name = "Text Embedding 3 Large".to_string();
        let provider = ModelsDevProvider {
            id: "alibaba".to_string(),
            name: "Alibaba".to_string(),
            api: Some("https://dashscope.aliyuncs.com/compatible-mode/v1".to_string()),
            npm: Some("@ai-sdk/openai-compatible".to_string()),
            models: HashMap::from([
                ("qwen-max".to_string(), text_chat_model("qwen-max")),
                ("deprecated-chat".to_string(), deprecated),
                ("image-output".to_string(), image),
                ("text-embedding-3-large".to_string(), embedding),
            ]),
            ..Default::default()
        };
        let catalog = catalog_with_provider("alibaba", provider);
        let config = ModelsDevProviderConfig::new("alibaba", ModelsDevProviderFamily::Qwen);

        let models =
            build_models_dev_available_models(&catalog, &config, &[], &HashMap::new()).unwrap();
        let ids: Vec<&str> = models.iter().map(|model| model.id.as_str()).collect();

        assert_eq!(ids, vec!["qwen-max"]);
    }
}
