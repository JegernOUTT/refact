use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tracing::warn;

use crate::models_dev::{models_dev_catalog_to_model_caps, ModelsDevCatalog};
use crate::provider_types::ModelPricing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCapsSource {
    Registry,
    Finetune,
    Custom,
}

impl Default for ModelCapsSource {
    fn default() -> Self {
        Self::Registry
    }
}

#[derive(Debug, Clone)]
pub struct CanonicalNameParts {
    pub original: String,
    pub provider_stripped: String,
    pub base_model: String,
    pub is_finetune: bool,
    pub last_segment: String,
    pub last_segment_base: String,
}

#[derive(Debug, Clone)]
pub struct ResolvedCaps {
    pub caps: ModelCapabilities,
    pub source: ModelCapsSource,
    pub matched_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CachingType {
    None,
    Auto,
    Explicit,
    Openai,
}

impl Default for CachingType {
    fn default() -> Self {
        Self::None
    }
}

fn default_true() -> bool {
    true
}

fn default_cache_control_support() -> bool {
    false
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelCapabilities {
    pub n_ctx: usize,
    pub max_output_tokens: usize,
    #[serde(default)]
    pub supports_tools: bool,
    #[serde(default)]
    pub supports_strict_tools: bool,
    #[serde(default)]
    pub supports_vision: bool,
    #[serde(default)]
    pub supports_video: bool,
    #[serde(default)]
    pub supports_audio: bool,
    #[serde(default)]
    pub supports_pdf: bool,
    #[serde(default)]
    pub supports_clicks: bool,
    #[serde(default = "default_true")]
    pub supports_temperature: bool,
    #[serde(default = "default_true")]
    pub supports_streaming: bool,
    #[serde(default)]
    pub supports_max_completion_tokens: bool,
    #[serde(default)]
    pub reasoning_effort_options: Option<Vec<String>>,
    #[serde(default)]
    pub supports_thinking_budget: bool,
    #[serde(default)]
    pub supports_adaptive_thinking_budget: bool,
    #[serde(default)]
    pub supports_parallel_tools: bool,
    #[serde(default)]
    pub max_thinking_tokens: Option<usize>,
    #[serde(default)]
    pub caching: CachingType,
    #[serde(default)]
    pub tokenizer: String,
    #[serde(default)]
    pub default_temperature: Option<f32>,
    #[serde(default)]
    pub default_max_tokens: Option<usize>,
    #[serde(default)]
    pub supports_web_search: bool,
    #[serde(default = "default_cache_control_support")]
    pub supports_cache_control: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_cost: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

const MAX_REASONABLE_N_CTX: usize = 10_000_000;
const MAX_REASONABLE_OUTPUT_TOKENS: usize = 1_000_000;

pub const ANTHROPIC_CLOUD_TOKENIZER: &str = "anthropic";
pub const CLAUDE_CLOUD_TOKENIZER_ALIAS: &str = "claude";
pub const OPENAI_CLOUD_TOKENIZER: &str = "openai";

pub fn is_predefined_cloud_tokenizer(tokenizer: &str) -> bool {
    matches!(
        tokenizer.trim().to_ascii_lowercase().as_str(),
        ANTHROPIC_CLOUD_TOKENIZER | CLAUDE_CLOUD_TOKENIZER_ALIAS | OPENAI_CLOUD_TOKENIZER
    )
}

pub fn predefined_cloud_tokenizer_for_model(
    provider: &str,
    model_id: &str,
) -> Option<&'static str> {
    let provider = provider.to_ascii_lowercase().replace('-', "_");
    let model_id = model_id
        .rsplit('/')
        .next()
        .unwrap_or(model_id)
        .to_ascii_lowercase();

    if model_id.starts_with("claude-") {
        return Some(ANTHROPIC_CLOUD_TOKENIZER);
    }

    if matches!(provider.as_str(), "anthropic" | "claude_code") {
        return Some(ANTHROPIC_CLOUD_TOKENIZER);
    }

    None
}

fn normalize_tokenizer(tokenizer: &str) -> String {
    if is_predefined_cloud_tokenizer(tokenizer) {
        return tokenizer.trim().to_ascii_lowercase();
    }
    if tokenizer.is_empty()
        || tokenizer.starts_with("hf://")
        || tokenizer.starts_with("http://")
        || tokenizer.starts_with("https://")
        || tokenizer.starts_with("file://")
        || tokenizer.starts_with("fake")
    {
        return tokenizer.to_string();
    }
    if tokenizer.contains('/') {
        return format!("hf://{}", tokenizer);
    }
    tokenizer.to_string()
}

pub fn default_tokenizer_for_model(provider: &str, model_id: &str) -> String {
    predefined_cloud_tokenizer_for_model(provider, model_id)
        .unwrap_or("fake")
        .to_string()
}

pub fn normalize_tokenizer_or_default(provider: &str, model_id: &str, tokenizer: &str) -> String {
    let normalized = normalize_tokenizer(tokenizer.trim());
    if normalized.is_empty() {
        default_tokenizer_for_model(provider, model_id)
    } else {
        normalized
    }
}

fn provider_from_model_key(model_key: &str) -> &str {
    model_key
        .split_once('/')
        .map(|(provider, _)| provider)
        .unwrap_or("")
}

pub fn validate_model_caps(caps: &mut HashMap<String, ModelCapabilities>) {
    for (name, cap) in caps.iter_mut() {
        if cap.n_ctx > MAX_REASONABLE_N_CTX {
            warn!(
                "Model {} has unreasonable n_ctx {}, clamping to {}",
                name, cap.n_ctx, MAX_REASONABLE_N_CTX
            );
            cap.n_ctx = MAX_REASONABLE_N_CTX;
        }
        if cap.max_output_tokens > MAX_REASONABLE_OUTPUT_TOKENS {
            warn!(
                "Model {} has unreasonable max_output_tokens {}, clamping to {}",
                name, cap.max_output_tokens, MAX_REASONABLE_OUTPUT_TOKENS
            );
            cap.max_output_tokens = MAX_REASONABLE_OUTPUT_TOKENS;
        }
        if matches!(cap.caching, CachingType::Explicit) {
            cap.supports_cache_control = true;
        }
        cap.tokenizer =
            normalize_tokenizer_or_default(provider_from_model_key(name), name, &cap.tokenizer);
    }
}

pub fn model_caps_from_models_dev_catalog(
    catalog: &ModelsDevCatalog,
) -> Result<HashMap<String, ModelCapabilities>, String> {
    let mut models = models_dev_catalog_to_model_caps(catalog)?;
    validate_model_caps(&mut models);
    Ok(models)
}

pub fn model_caps_pricing_metadata(caps: &HashMap<String, ModelCapabilities>) -> Value {
    let mut map = Map::new();
    for (model_id, model_caps) in caps {
        let Some(pricing) = model_caps.pricing.as_ref() else {
            continue;
        };
        let Ok(mut value) = serde_json::to_value(pricing) else {
            continue;
        };
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "source".to_string(),
                Value::String("models.dev".to_string()),
            );
            obj.insert(
                "tier".to_string(),
                Value::String("base_text_tokens".to_string()),
            );
            if let Some(raw_cost) = model_caps.raw_cost.as_ref() {
                obj.insert("raw_cost".to_string(), raw_cost.clone());
            }
        }
        map.insert(model_id.clone(), value);
    }
    Value::Object(map)
}

pub fn is_model_supported(caps: &HashMap<String, ModelCapabilities>, model_name: &str) -> bool {
    resolve_model_caps(caps, model_name).is_some()
}

pub fn canonicalize_model_name(model_id: &str) -> CanonicalNameParts {
    let provider_stripped = if let Some(pos) = model_id.find('/') {
        model_id[pos + 1..].to_string()
    } else {
        model_id.to_string()
    };

    let (base_model, is_finetune) = if let Some(colon_pos) = provider_stripped.find(':') {
        let base = provider_stripped[..colon_pos].to_string();
        let suffix = &provider_stripped[colon_pos + 1..];
        let is_ft = suffix.starts_with("ft-") || suffix.starts_with("ft_");
        (base, is_ft)
    } else {
        (provider_stripped.clone(), false)
    };

    let last_segment = model_id.split('/').last().unwrap_or(model_id).to_string();
    let last_segment_base = if let Some(colon_pos) = last_segment.find(':') {
        last_segment[..colon_pos].to_string()
    } else {
        last_segment.clone()
    };

    CanonicalNameParts {
        original: model_id.to_string(),
        provider_stripped,
        base_model,
        is_finetune,
        last_segment,
        last_segment_base,
    }
}

const IGNORABLE_SUFFIXES: &[&str] = &[
    "-latest",
    "-preview",
    "-cheap",
    "-deep-research",
    "-fp4",
    "-fp8",
    "-fp16",
    "-int4",
    "-int8",
];

fn normalize_model_name_for_matching(name: &str) -> String {
    let mut result = name.to_lowercase();
    loop {
        let mut changed = false;
        for suffix in IGNORABLE_SUFFIXES {
            if result.ends_with(suffix) {
                result.truncate(result.len() - suffix.len());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    result = result.replace('.', "-");
    result
}

fn matches_pattern(pattern: &str, name: &str) -> bool {
    if !pattern.contains('*') {
        return pattern == name;
    }

    if pattern.ends_with('*') {
        let prefix = &pattern[..pattern.len() - 1];
        return name.starts_with(prefix);
    }

    if pattern.starts_with('*') {
        let suffix = &pattern[1..];
        return name.ends_with(suffix);
    }

    if let Some(star_pos) = pattern.find('*') {
        let prefix = &pattern[..star_pos];
        let suffix = &pattern[star_pos + 1..];
        return name.starts_with(prefix) && name.ends_with(suffix);
    }

    false
}

fn pattern_specificity(pattern: &str) -> usize {
    pattern.chars().filter(|c| *c != '*').count()
}

pub fn resolve_model_caps(
    caps: &HashMap<String, ModelCapabilities>,
    model_name: &str,
) -> Option<ResolvedCaps> {
    let canonical = canonicalize_model_name(model_name);

    let names_to_try = [
        &canonical.original,
        &canonical.provider_stripped,
        &canonical.base_model,
        &canonical.last_segment,
        &canonical.last_segment_base,
    ];

    for name in &names_to_try {
        if let Some(model_caps) = caps.get(*name) {
            let source = if canonical.is_finetune
                && (*name == &canonical.base_model || *name == &canonical.last_segment_base)
            {
                ModelCapsSource::Finetune
            } else {
                ModelCapsSource::Registry
            };
            return Some(ResolvedCaps {
                caps: model_caps.clone(),
                source,
                matched_key: (*name).clone(),
            });
        }
    }

    let normalized_names: Vec<String> = names_to_try
        .iter()
        .map(|n| normalize_model_name_for_matching(n))
        .collect();

    let mut seen = std::collections::HashSet::new();
    let unique_normalized: Vec<&String> = normalized_names
        .iter()
        .filter(|n| seen.insert(n.as_str().to_string()))
        .collect();

    for (key, model_caps) in caps.iter() {
        if key.contains('*') {
            continue;
        }
        let key_normalized = normalize_model_name_for_matching(key);
        for norm_name in &unique_normalized {
            if key_normalized == **norm_name {
                let source = if canonical.is_finetune {
                    ModelCapsSource::Finetune
                } else {
                    ModelCapsSource::Registry
                };
                return Some(ResolvedCaps {
                    caps: model_caps.clone(),
                    source,
                    matched_key: key.clone(),
                });
            }
        }
    }

    let mut best_match: Option<(&str, &ModelCapabilities, usize)> = None;

    for (pattern, model_caps) in caps.iter() {
        if !pattern.contains('*') {
            continue;
        }

        for name in &names_to_try {
            if matches_pattern(pattern, name) {
                let specificity = pattern_specificity(pattern);
                if best_match.is_none() || specificity > best_match.unwrap().2 {
                    best_match = Some((pattern, model_caps, specificity));
                } else if specificity == best_match.unwrap().2
                    && pattern.as_str() < best_match.unwrap().0
                {
                    best_match = Some((pattern, model_caps, specificity));
                }
            }
        }
    }

    if best_match.is_none() {
        for (pattern, model_caps) in caps.iter() {
            if !pattern.contains('*') {
                continue;
            }
            let pattern_normalized = normalize_model_name_for_matching(pattern);
            for norm_name in &unique_normalized {
                if matches_pattern(&pattern_normalized, norm_name) {
                    let specificity = pattern_specificity(&pattern_normalized);
                    if best_match.is_none() || specificity > best_match.unwrap().2 {
                        best_match = Some((pattern, model_caps, specificity));
                    } else if specificity == best_match.unwrap().2
                        && pattern.as_str() < best_match.unwrap().0
                    {
                        best_match = Some((pattern, model_caps, specificity));
                    }
                }
            }
        }
    }

    best_match.map(|(matched_key, model_caps, _)| {
        let source = if canonical.is_finetune {
            ModelCapsSource::Finetune
        } else {
            ModelCapsSource::Registry
        };
        ResolvedCaps {
            caps: model_caps.clone(),
            source,
            matched_key: matched_key.to_string(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_types::{ModelPricing, ModelPricingTier};
    use serde_json::json;

    fn caps_with(entries: &[(&str, usize)]) -> HashMap<String, ModelCapabilities> {
        entries
            .iter()
            .map(|(name, n_ctx)| ((*name).to_string(), model_cap(*n_ctx)))
            .collect()
    }

    fn model_cap(n_ctx: usize) -> ModelCapabilities {
        ModelCapabilities {
            n_ctx,
            max_output_tokens: n_ctx / 2,
            tokenizer: "fake".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn resolves_exact_model_match() {
        let caps = caps_with(&[("openai/gpt-4.1", 128_000)]);

        let resolved = resolve_model_caps(&caps, "openai/gpt-4.1").unwrap();

        assert_eq!(resolved.matched_key, "openai/gpt-4.1");
        assert_eq!(resolved.source, ModelCapsSource::Registry);
        assert_eq!(resolved.caps.n_ctx, 128_000);
        assert!(is_model_supported(&caps, "openai/gpt-4.1"));
        assert!(!is_model_supported(&caps, "openai/missing"));
    }

    #[test]
    fn resolves_provider_stripped_model_match() {
        let caps = caps_with(&[("gpt-4.1", 128_000)]);

        let resolved = resolve_model_caps(&caps, "openai/gpt-4.1").unwrap();

        assert_eq!(resolved.matched_key, "gpt-4.1");
        assert_eq!(resolved.source, ModelCapsSource::Registry);
        assert_eq!(resolved.caps.n_ctx, 128_000);
    }

    #[test]
    fn resolves_finetune_to_base_model_match() {
        let caps = caps_with(&[("gpt-4o", 128_000)]);
        let canonical = canonicalize_model_name("openai/gpt-4o:ft-project-123");

        let resolved = resolve_model_caps(&caps, "openai/gpt-4o:ft-project-123").unwrap();

        assert_eq!(canonical.original, "openai/gpt-4o:ft-project-123");
        assert_eq!(canonical.provider_stripped, "gpt-4o:ft-project-123");
        assert_eq!(canonical.base_model, "gpt-4o");
        assert!(canonical.is_finetune);
        assert_eq!(canonical.last_segment_base, "gpt-4o");
        assert_eq!(resolved.matched_key, "gpt-4o");
        assert_eq!(resolved.source, ModelCapsSource::Finetune);
        assert_eq!(resolved.caps.n_ctx, 128_000);
    }

    #[test]
    fn resolves_wildcard_model_match() {
        let caps = caps_with(&[("anthropic/claude-3-*", 200_000)]);

        let resolved = resolve_model_caps(&caps, "anthropic/claude-3-sonnet").unwrap();

        assert_eq!(resolved.matched_key, "anthropic/claude-3-*");
        assert_eq!(resolved.source, ModelCapsSource::Registry);
        assert_eq!(resolved.caps.n_ctx, 200_000);
    }

    #[test]
    fn resolves_normalized_suffix_model_match() {
        let caps = caps_with(&[("openai/gpt-4.1", 128_000)]);

        let resolved = resolve_model_caps(&caps, "openai/gpt-4.1-preview").unwrap();

        assert_eq!(resolved.matched_key, "openai/gpt-4.1");
        assert_eq!(resolved.source, ModelCapsSource::Registry);
        assert_eq!(resolved.caps.n_ctx, 128_000);
    }

    #[test]
    fn resolves_more_specific_wildcard_before_generic() {
        let caps = caps_with(&[
            ("provider/model-*", 4_096),
            ("provider/model-pro-*", 32_768),
        ]);

        let resolved = resolve_model_caps(&caps, "provider/model-pro-2026").unwrap();

        assert_eq!(resolved.matched_key, "provider/model-pro-*");
        assert_eq!(resolved.caps.n_ctx, 32_768);
    }

    #[test]
    fn resolves_equal_specificity_wildcard_by_lexical_key() {
        let caps = caps_with(&[("foo-b*ar", 4_096), ("foo-*bar", 8_192)]);

        let resolved = resolve_model_caps(&caps, "foo-bbar").unwrap();

        assert_eq!(resolved.matched_key, "foo-*bar");
        assert_eq!(resolved.caps.n_ctx, 8_192);
    }

    #[test]
    fn validate_model_caps_clamps_large_limits_and_normalizes_tokenizer() {
        let mut caps = HashMap::from([(
            "huge".to_string(),
            ModelCapabilities {
                n_ctx: usize::MAX,
                max_output_tokens: usize::MAX,
                tokenizer: "Qwen/Qwen3".to_string(),
                ..Default::default()
            },
        )]);

        validate_model_caps(&mut caps);

        let huge = caps.get("huge").unwrap();
        assert_eq!(huge.n_ctx, MAX_REASONABLE_N_CTX);
        assert_eq!(huge.max_output_tokens, MAX_REASONABLE_OUTPUT_TOKENS);
        assert_eq!(huge.tokenizer, "hf://Qwen/Qwen3");
    }

    #[test]
    fn validate_model_caps_preserves_predefined_cloud_tokenizer_names() {
        let mut caps = HashMap::from([(
            "claude".to_string(),
            ModelCapabilities {
                tokenizer: "Anthropic".to_string(),
                ..Default::default()
            },
        )]);

        validate_model_caps(&mut caps);

        assert_eq!(
            caps.get("claude").unwrap().tokenizer,
            ANTHROPIC_CLOUD_TOKENIZER
        );
    }

    #[test]
    fn predefined_cloud_tokenizer_matches_explicit_openai_name() {
        assert!(is_predefined_cloud_tokenizer("openai"));
        assert!(is_predefined_cloud_tokenizer(" OpenAI "));
    }

    #[test]
    fn validate_model_caps_migrates_missing_tokenizers_to_cloud_or_fake_defaults() {
        let mut caps = HashMap::from([
            (
                "openai/gpt-4.1".to_string(),
                ModelCapabilities {
                    tokenizer: String::new(),
                    ..Default::default()
                },
            ),
            (
                "anthropic/claude-sonnet-4-5".to_string(),
                ModelCapabilities {
                    tokenizer: String::new(),
                    ..Default::default()
                },
            ),
            (
                "custom/unknown-model".to_string(),
                ModelCapabilities {
                    tokenizer: String::new(),
                    ..Default::default()
                },
            ),
        ]);

        validate_model_caps(&mut caps);

        assert_eq!(caps.get("openai/gpt-4.1").unwrap().tokenizer, "fake");
        assert_eq!(
            caps.get("anthropic/claude-sonnet-4-5").unwrap().tokenizer,
            ANTHROPIC_CLOUD_TOKENIZER
        );
        assert_eq!(caps.get("custom/unknown-model").unwrap().tokenizer, "fake");
    }

    #[test]
    fn predefined_cloud_tokenizer_matches_anthropic_model_families() {
        assert_eq!(
            predefined_cloud_tokenizer_for_model("anthropic", "claude-3-5-sonnet-latest"),
            Some(ANTHROPIC_CLOUD_TOKENIZER)
        );
        assert_eq!(
            predefined_cloud_tokenizer_for_model("ollama", "llama3"),
            None
        );
    }

    #[test]
    fn validate_model_caps_derives_cache_support_from_explicit_caching() {
        let mut caps = HashMap::from([(
            "explicit-cache".to_string(),
            ModelCapabilities {
                caching: CachingType::Explicit,
                supports_cache_control: false,
                ..Default::default()
            },
        )]);

        validate_model_caps(&mut caps);

        assert!(caps.get("explicit-cache").unwrap().supports_cache_control);
    }

    #[test]
    fn model_caps_pricing_metadata_converts_pricing_and_raw_cost() {
        let mut caps = HashMap::new();
        caps.insert(
            "priced-model".to_string(),
            ModelCapabilities {
                pricing: Some(ModelPricing {
                    prompt: 1.25,
                    generated: 2.5,
                    cache_read: Some(0.25),
                    cache_creation: Some(0.75),
                    context_over_200k: Some(ModelPricingTier {
                        prompt: Some(3.0),
                        generated: Some(4.0),
                        cache_read: None,
                        cache_creation: Some(1.0),
                    }),
                }),
                raw_cost: Some(json!({ "input": 1.25, "output": 2.5 })),
                ..Default::default()
            },
        );
        caps.insert("unpriced-model".to_string(), ModelCapabilities::default());

        let metadata = model_caps_pricing_metadata(&caps);

        assert_eq!(metadata["priced-model"]["prompt"], json!(1.25));
        assert_eq!(metadata["priced-model"]["generated"], json!(2.5));
        assert_eq!(metadata["priced-model"]["cache_read"], json!(0.25));
        assert_eq!(metadata["priced-model"]["cache_creation"], json!(0.75));
        assert_eq!(
            metadata["priced-model"]["context_over_200k"]["prompt"],
            json!(3.0)
        );
        assert_eq!(
            metadata["priced-model"]["context_over_200k"]["generated"],
            json!(4.0)
        );
        assert_eq!(
            metadata["priced-model"]["context_over_200k"]["cache_creation"],
            json!(1.0)
        );
        assert_eq!(metadata["priced-model"]["source"], json!("models.dev"));
        assert_eq!(metadata["priced-model"]["tier"], json!("base_text_tokens"));
        assert_eq!(
            metadata["priced-model"]["raw_cost"],
            json!({ "input": 1.25, "output": 2.5 })
        );
        assert!(metadata.get("unpriced-model").is_none());
    }
}
