use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::RwLock as ARwLock;
use tracing::warn;

use crate::caps::models_dev::{
    load_models_dev_catalog, models_dev_catalog_to_model_caps, ModelsDevCatalog,
};
use crate::global_context::GlobalContext;
use crate::providers::traits::ModelPricing;

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
    #[serde(default = "default_true")]
    pub supports_cache_control: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_cost: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

fn default_true() -> bool {
    true
}

const MAX_REASONABLE_N_CTX: usize = 10_000_000;
const MAX_REASONABLE_OUTPUT_TOKENS: usize = 1_000_000;

fn normalize_tokenizer(tokenizer: &str) -> String {
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

fn validate_model_caps(caps: &mut HashMap<String, ModelCapabilities>) {
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
        cap.tokenizer = normalize_tokenizer(&cap.tokenizer);
    }
}

pub fn model_caps_from_models_dev_catalog(
    catalog: &ModelsDevCatalog,
) -> Result<HashMap<String, ModelCapabilities>, String> {
    let mut models = models_dev_catalog_to_model_caps(catalog)?;
    validate_model_caps(&mut models);
    Ok(models)
}

pub async fn get_model_caps(
    gcx: Arc<ARwLock<GlobalContext>>,
    force_refresh: bool,
) -> Result<HashMap<String, ModelCapabilities>, String> {
    let catalog = load_models_dev_catalog(gcx, force_refresh)
        .await
        .map_err(|e| format!("Failed to load models.dev model capabilities: {e}"))?;
    model_caps_from_models_dev_catalog(&catalog)
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

/// Known suffixes added by cloud providers that don't change model capabilities.
/// Stripping these allows matching e.g. "gemini-3-flash-preview" → "gemini-3-flash".
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

/// Normalize a model name for fuzzy matching:
/// - lowercase
/// - strip known ignorable suffixes (repeatedly, to handle e.g. "-preview-cheap")
/// - replace '.' with '-' (e.g. "claude-opus-4.6" → "claude-opus-4-6")
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

    // Phase 1: Exact case-sensitive match
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

    // Phase 2: Normalized matching (case-insensitive + suffix stripping + dot→dash)
    let normalized_names: Vec<String> = names_to_try
        .iter()
        .map(|n| normalize_model_name_for_matching(n))
        .collect();

    // Deduplicate normalized names while preserving order
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

    // Phase 3: Wildcard pattern matching (case-sensitive first)
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

    // Phase 4: Wildcard pattern matching with normalized names
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
    use crate::caps::models_dev::{
        get_model, load_models_dev_snapshot_catalog, write_models_dev_cache, ModelsDevCost,
        ModelsDevCostTier, ModelsDevLimit, ModelsDevModalities, ModelsDevModel, ModelsDevProvider,
    };

    fn catalog_with_providers(
        providers: Vec<(&str, Vec<(&str, ModelsDevModel)>)>,
    ) -> ModelsDevCatalog {
        providers
            .into_iter()
            .map(|(provider_id, models)| {
                let provider_models = models
                    .into_iter()
                    .map(|(model_key, model)| (model_key.to_string(), model))
                    .collect();
                (
                    provider_id.to_string(),
                    ModelsDevProvider {
                        id: provider_id.to_string(),
                        name: provider_id.to_string(),
                        models: provider_models,
                        ..Default::default()
                    },
                )
            })
            .collect()
    }

    fn models_dev_model(model_id: &str) -> ModelsDevModel {
        ModelsDevModel {
            id: model_id.to_string(),
            name: model_id.to_string(),
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

    #[test]
    fn test_models_dev_translation_maps_limits_booleans_modalities_and_pricing() {
        let catalog = catalog_with_providers(vec![(
            "provider-a",
            vec![(
                "model-a",
                ModelsDevModel {
                    id: "model-a".to_string(),
                    limit: Some(ModelsDevLimit {
                        context: Some(128_000),
                        output: Some(16_384),
                        ..Default::default()
                    }),
                    tool_call: Some(true),
                    temperature: None,
                    reasoning: Some(true),
                    modalities: Some(ModelsDevModalities {
                        input: vec![
                            "text".to_string(),
                            "image".to_string(),
                            "video".to_string(),
                            "audio".to_string(),
                            "pdf".to_string(),
                        ],
                        output: vec!["text".to_string()],
                    }),
                    cost: Some(ModelsDevCost {
                        input: Some(2.5),
                        output: Some(10.0),
                        cache_read: Some(1.25),
                        cache_write: Some(3.75),
                        context_over_200k: Some(ModelsDevCostTier {
                            input: Some(5.0),
                            output: Some(20.0),
                            ..Default::default()
                        }),
                    }),
                    ..Default::default()
                },
            )],
        )]);

        let caps = model_caps_from_models_dev_catalog(&catalog).unwrap();
        let resolved = resolve_model_caps(&caps, "provider-a/model-a").unwrap();
        let model = resolved.caps;

        assert_eq!(model.n_ctx, 128_000);
        assert_eq!(model.max_output_tokens, 16_384);
        assert!(model.supports_tools);
        assert!(model.supports_temperature);
        assert!(model.supports_vision);
        assert!(model.supports_video);
        assert!(model.supports_audio);
        assert!(model.supports_pdf);
        assert!(model.reasoning_effort_options.is_none());
        assert!(!model.supports_thinking_budget);
        assert!(!model.supports_adaptive_thinking_budget);
        assert_eq!(model.tokenizer, "fake");
        let pricing = model.pricing.unwrap();
        assert_eq!(pricing.prompt, 2.5);
        assert_eq!(pricing.generated, 10.0);
        assert_eq!(pricing.cache_read, Some(1.25));
        assert_eq!(pricing.cache_creation, Some(3.75));
        assert_eq!(
            model.raw_cost.unwrap()["context_over_200k"]["input"],
            serde_json::json!(5.0)
        );
    }

    #[test]
    fn test_models_dev_option_bool_defaults_are_explicit() {
        let catalog = catalog_with_providers(vec![(
            "provider-a",
            vec![
                ("missing-bools", models_dev_model("missing-bools")),
                (
                    "false-temperature",
                    ModelsDevModel {
                        temperature: Some(false),
                        tool_call: Some(false),
                        reasoning: None,
                        ..models_dev_model("false-temperature")
                    },
                ),
            ],
        )]);

        let caps = model_caps_from_models_dev_catalog(&catalog).unwrap();
        let missing = resolve_model_caps(&caps, "missing-bools").unwrap().caps;
        assert!(!missing.supports_tools);
        assert!(missing.supports_temperature);
        assert!(missing.reasoning_effort_options.is_none());
        assert!(!missing.supports_thinking_budget);

        let explicit_false = resolve_model_caps(&caps, "provider-a/false-temperature")
            .unwrap()
            .caps;
        assert!(!explicit_false.supports_tools);
        assert!(!explicit_false.supports_temperature);
        assert!(explicit_false.reasoning_effort_options.is_none());
    }

    #[test]
    fn test_models_dev_translation_adds_qualified_and_unambiguous_bare_keys() {
        let catalog = catalog_with_providers(vec![(
            "provider-a",
            vec![("unique-model", models_dev_model("unique-model"))],
        )]);

        let caps = model_caps_from_models_dev_catalog(&catalog).unwrap();

        assert!(caps.contains_key("provider-a/unique-model"));
        assert!(caps.contains_key("unique-model"));
        assert_eq!(
            resolve_model_caps(&caps, "provider-a/unique-model")
                .unwrap()
                .matched_key,
            "provider-a/unique-model"
        );
        assert_eq!(
            resolve_model_caps(&caps, "unique-model")
                .unwrap()
                .matched_key,
            "unique-model"
        );
    }

    #[test]
    fn test_models_dev_translation_skips_conflicting_bare_keys() {
        let catalog = catalog_with_providers(vec![
            (
                "provider-a",
                vec![("shared-model", models_dev_model("shared-model"))],
            ),
            (
                "provider-b",
                vec![("shared-model", models_dev_model("shared-model"))],
            ),
        ]);

        let caps = model_caps_from_models_dev_catalog(&catalog).unwrap();

        assert!(caps.contains_key("provider-a/shared-model"));
        assert!(caps.contains_key("provider-b/shared-model"));
        assert!(!caps.contains_key("shared-model"));
        assert!(resolve_model_caps(&caps, "shared-model").is_none());
    }

    #[test]
    fn test_models_dev_non_chat_modalities_are_excluded() {
        let mut image_output = models_dev_model("image-model");
        image_output.modalities = Some(ModelsDevModalities {
            input: vec!["text".to_string()],
            output: vec!["image".to_string()],
        });
        let mut text_and_image_output = models_dev_model("text-image-model");
        text_and_image_output.modalities = Some(ModelsDevModalities {
            input: vec!["text".to_string(), "image".to_string()],
            output: vec!["text".to_string(), "image".to_string()],
        });
        let mut audio_output = models_dev_model("tts-model");
        audio_output.modalities = Some(ModelsDevModalities {
            input: vec!["text".to_string()],
            output: vec!["audio".to_string()],
        });
        let mut asr = models_dev_model("asr-model");
        asr.modalities = Some(ModelsDevModalities {
            input: vec!["audio".to_string()],
            output: vec!["text".to_string()],
        });
        let catalog = catalog_with_providers(vec![(
            "provider-a",
            vec![
                ("normal-chat", models_dev_model("normal-chat")),
                ("image-model", image_output),
                ("text-image-model", text_and_image_output),
                ("tts-model", audio_output),
                ("asr-model", asr),
            ],
        )]);

        let caps = model_caps_from_models_dev_catalog(&catalog).unwrap();

        assert!(resolve_model_caps(&caps, "provider-a/normal-chat").is_some());
        for model in ["image-model", "text-image-model", "tts-model", "asr-model"] {
            assert!(
                resolve_model_caps(&caps, &format!("provider-a/{model}")).is_none(),
                "{model} should be excluded from chat capabilities"
            );
        }
    }

    #[test]
    fn test_models_dev_special_purpose_text_models_are_excluded() {
        let catalog = catalog_with_providers(vec![(
            "provider-a",
            vec![
                (
                    "text-embedding-3-large",
                    models_dev_model("text-embedding-3-large"),
                ),
                ("qwen3-reranker-4b", models_dev_model("qwen3-reranker-4b")),
                ("safety-guard", models_dev_model("safety-guard")),
                ("normal-chat", models_dev_model("normal-chat")),
            ],
        )]);

        let caps = model_caps_from_models_dev_catalog(&catalog).unwrap();

        assert!(resolve_model_caps(&caps, "provider-a/normal-chat").is_some());
        for model in [
            "text-embedding-3-large",
            "qwen3-reranker-4b",
            "safety-guard",
        ] {
            assert!(resolve_model_caps(&caps, &format!("provider-a/{model}")).is_none());
        }
    }

    #[test]
    fn test_models_dev_multimodal_text_output_chat_models_are_included() {
        let mut multimodal = models_dev_model("vision-chat");
        multimodal.modalities = Some(ModelsDevModalities {
            input: vec!["text".to_string(), "image".to_string(), "pdf".to_string()],
            output: vec!["text".to_string()],
        });
        let catalog =
            catalog_with_providers(vec![("provider-a", vec![("vision-chat", multimodal)])]);

        let caps = model_caps_from_models_dev_catalog(&catalog).unwrap();
        let model = resolve_model_caps(&caps, "provider-a/vision-chat")
            .unwrap()
            .caps;

        assert!(model.supports_vision);
        assert!(model.supports_pdf);
    }

    #[test]
    fn test_models_dev_status_policy_excludes_deprecated_and_marks_beta() {
        let mut deprecated = models_dev_model("deprecated-chat");
        deprecated.status = Some("deprecated".to_string());
        let mut beta = models_dev_model("beta-chat");
        beta.status = Some("beta".to_string());
        let catalog = catalog_with_providers(vec![(
            "provider-a",
            vec![("deprecated-chat", deprecated), ("beta-chat", beta)],
        )]);

        let caps = model_caps_from_models_dev_catalog(&catalog).unwrap();

        assert!(resolve_model_caps(&caps, "provider-a/deprecated-chat").is_none());
        let beta = resolve_model_caps(&caps, "provider-a/beta-chat")
            .unwrap()
            .caps;
        assert_eq!(beta.status.as_deref(), Some("beta"));
    }

    #[test]
    fn test_models_dev_openai_providers_do_not_expose_gpt_image_models() {
        let mut gpt_image = models_dev_model("gpt-image-1");
        gpt_image.modalities = Some(ModelsDevModalities {
            input: vec!["text".to_string(), "image".to_string()],
            output: vec!["image".to_string()],
        });
        let catalog = catalog_with_providers(vec![(
            "openai",
            vec![
                ("gpt-4o", models_dev_model("gpt-4o")),
                ("gpt-image-1", gpt_image),
            ],
        )]);
        let caps = model_caps_from_models_dev_catalog(&catalog).unwrap();

        let openai_models = crate::providers::create_provider("openai")
            .unwrap()
            .get_available_models_from_caps(&caps);
        let openai_responses_models = crate::providers::create_provider("openai_responses")
            .unwrap()
            .get_available_models_from_caps(&caps);

        assert!(openai_models.iter().any(|model| model.id == "gpt-4o"));
        assert!(openai_responses_models
            .iter()
            .any(|model| model.id == "gpt-4o"));
        assert!(!openai_models
            .iter()
            .any(|model| model.id.contains("gpt-image")));
        assert!(!openai_responses_models
            .iter()
            .any(|model| model.id.contains("gpt-image")));
    }

    #[test]
    fn test_models_dev_pricing_metadata_labels_base_tier_and_preserves_raw_cost() {
        let catalog = catalog_with_providers(vec![(
            "provider-a",
            vec![(
                "priced-model",
                ModelsDevModel {
                    cost: Some(ModelsDevCost {
                        input: Some(1.0),
                        output: Some(2.0),
                        cache_read: Some(0.5),
                        cache_write: Some(0.75),
                        context_over_200k: Some(ModelsDevCostTier {
                            input: Some(10.0),
                            output: Some(20.0),
                            ..Default::default()
                        }),
                    }),
                    ..models_dev_model("priced-model")
                },
            )],
        )]);
        let caps = model_caps_from_models_dev_catalog(&catalog).unwrap();
        let metadata = model_caps_pricing_metadata(&caps);
        let pricing = &metadata["provider-a/priced-model"];

        assert_eq!(pricing["prompt"], serde_json::json!(1.0));
        assert_eq!(pricing["generated"], serde_json::json!(2.0));
        assert_eq!(pricing["cache_read"], serde_json::json!(0.5));
        assert_eq!(pricing["cache_creation"], serde_json::json!(0.75));
        assert_eq!(pricing["source"], serde_json::json!("models.dev"));
        assert_eq!(pricing["tier"], serde_json::json!("base_text_tokens"));
        assert_eq!(
            pricing["raw_cost"]["context_over_200k"]["output"],
            serde_json::json!(20.0)
        );
    }

    #[tokio::test]
    async fn test_get_model_caps_loads_models_dev_snapshot_and_cache() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let snapshot_caps = get_model_caps(gcx.clone(), false).await.unwrap();
        assert!(resolve_model_caps(&snapshot_caps, "openai/gpt-4o").is_some());

        let mut catalog = load_models_dev_snapshot_catalog().unwrap();
        catalog.get_mut("openai").unwrap().models.insert(
            "refact-cache-test-model".to_string(),
            ModelsDevModel {
                limit: Some(ModelsDevLimit {
                    context: Some(42_000),
                    output: Some(4_200),
                    ..Default::default()
                }),
                tool_call: Some(true),
                ..models_dev_model("refact-cache-test-model")
            },
        );
        let cache_dir = { gcx.read().await.cache_dir.clone() };
        let contents = serde_json::to_string(&catalog).unwrap();
        write_models_dev_cache(&cache_dir, &contents).await.unwrap();

        let cache_caps = get_model_caps(gcx, false).await.unwrap();
        let resolved = resolve_model_caps(&cache_caps, "openai/refact-cache-test-model")
            .unwrap()
            .caps;
        assert_eq!(resolved.n_ctx, 42_000);
        assert_eq!(resolved.max_output_tokens, 4_200);
        assert!(resolved.supports_tools);
    }

    #[test]
    fn test_models_dev_snapshot_resolves_representative_planned_providers() {
        let catalog = load_models_dev_snapshot_catalog().unwrap();
        let caps = model_caps_from_models_dev_catalog(&catalog).unwrap();

        for model in [
            "openai/gpt-4o",
            "anthropic/claude-3-5-sonnet-20241022",
            "deepseek/deepseek-chat",
            "alibaba/qwen-max",
            "moonshotai/kimi-k2-thinking",
            "zai/glm-4.6",
            "zhipuai/glm-4.6",
            "minimax/MiniMax-M2.1",
            "github-copilot/claude-opus-4.6",
        ] {
            assert!(
                resolve_model_caps(&caps, model).is_some(),
                "models.dev snapshot should resolve {model}"
            );
        }
    }

    #[test]
    fn test_models_dev_snapshot_filters_representative_real_catalog_entries() {
        let catalog = load_models_dev_snapshot_catalog().unwrap();
        let caps = model_caps_from_models_dev_catalog(&catalog).unwrap();

        for (provider, model) in [
            ("openai", "gpt-4o"),
            ("openai", "gpt-5.1"),
            ("anthropic", "claude-3-5-sonnet-20241022"),
            ("deepseek", "deepseek-chat"),
            ("alibaba", "qwen-plus"),
            ("moonshotai", "kimi-k2-thinking"),
            ("zai", "glm-4.6"),
            ("zhipuai", "glm-4.6"),
            ("minimax", "MiniMax-M2.1"),
        ] {
            assert!(
                get_model(&catalog, provider, model).is_some(),
                "snapshot fixture should contain {provider}/{model}"
            );
            assert!(
                resolve_model_caps(&caps, &format!("{provider}/{model}")).is_some(),
                "models.dev caps should include {provider}/{model}"
            );
        }

        for (provider, model) in [
            ("openai", "gpt-image-1"),
            ("alibaba", "qwen-omni-turbo"),
            ("alibaba", "qwen3-asr-flash"),
            ("alibaba", "qwen-vl-ocr"),
            ("openai", "text-embedding-3-large"),
            ("azure", "cohere-embed-v-4-0"),
            ("regolo-ai", "qwen3-reranker-4b"),
            ("groq", "meta-llama/llama-prompt-guard-2-22m"),
            ("azure", "model-router"),
        ] {
            assert!(
                get_model(&catalog, provider, model).is_some(),
                "snapshot fixture should contain {provider}/{model}"
            );
            assert!(
                resolve_model_caps(&caps, &format!("{provider}/{model}")).is_none(),
                "models.dev caps should exclude {provider}/{model}"
            );
        }
    }

    #[test]
    fn test_model_capability_lookup() {
        let mut caps = HashMap::new();
        caps.insert(
            "gpt-4o".to_string(),
            ModelCapabilities {
                n_ctx: 128000,
                max_output_tokens: 16384,
                supports_tools: true,
                supports_vision: true,
                ..Default::default()
            },
        );
        caps.insert(
            "claude-3-5-sonnet".to_string(),
            ModelCapabilities {
                n_ctx: 200000,
                max_output_tokens: 8192,
                supports_tools: true,
                supports_vision: true,
                supports_pdf: true,
                ..Default::default()
            },
        );

        assert!(resolve_model_caps(&caps, "gpt-4o").is_some());
        assert!(resolve_model_caps(&caps, "openai/gpt-4o").is_some());
        assert!(resolve_model_caps(&caps, "gpt-4o:v2").is_some());
        assert!(resolve_model_caps(&caps, "claude-3-5-sonnet").is_some());
        assert!(resolve_model_caps(&caps, "unknown-model").is_none());
    }

    #[test]
    fn test_canonicalize_model_name() {
        let parts = canonicalize_model_name("openai/gpt-4o");
        assert_eq!(parts.provider_stripped, "gpt-4o");
        assert_eq!(parts.base_model, "gpt-4o");
        assert_eq!(parts.last_segment, "gpt-4o");
        assert!(!parts.is_finetune);

        let parts = canonicalize_model_name("gpt-4o:ft-abc123");
        assert_eq!(parts.provider_stripped, "gpt-4o:ft-abc123");
        assert_eq!(parts.base_model, "gpt-4o");
        assert!(parts.is_finetune);

        let parts = canonicalize_model_name("anthropic/claude-3-5-sonnet:ft-xyz");
        assert_eq!(parts.provider_stripped, "claude-3-5-sonnet:ft-xyz");
        assert_eq!(parts.base_model, "claude-3-5-sonnet");
        assert!(parts.is_finetune);

        let parts = canonicalize_model_name("openrouter/anthropic/claude-3.7-sonnet");
        assert_eq!(parts.provider_stripped, "anthropic/claude-3.7-sonnet");
        assert_eq!(parts.base_model, "anthropic/claude-3.7-sonnet");
        assert_eq!(parts.last_segment, "claude-3.7-sonnet");
        assert_eq!(parts.last_segment_base, "claude-3.7-sonnet");
        assert!(!parts.is_finetune);

        let parts = canonicalize_model_name("models/gemini-2.0-flash");
        assert_eq!(parts.provider_stripped, "gemini-2.0-flash");
        assert_eq!(parts.last_segment, "gemini-2.0-flash");
    }

    #[test]
    fn test_pattern_matching() {
        let mut caps = HashMap::new();
        caps.insert(
            "claude-3-7-sonnet*".to_string(),
            ModelCapabilities {
                n_ctx: 200000,
                max_output_tokens: 16384,
                supports_tools: true,
                ..Default::default()
            },
        );
        caps.insert(
            "gpt-4*".to_string(),
            ModelCapabilities {
                n_ctx: 128000,
                max_output_tokens: 8192,
                supports_tools: true,
                ..Default::default()
            },
        );

        let resolved = resolve_model_caps(&caps, "claude-3-7-sonnet-latest").unwrap();
        assert_eq!(resolved.matched_key, "claude-3-7-sonnet*");
        assert_eq!(resolved.caps.n_ctx, 200000);

        let resolved = resolve_model_caps(&caps, "gpt-4o").unwrap();
        assert_eq!(resolved.matched_key, "gpt-4*");
    }

    #[test]
    fn test_finetune_source() {
        let mut caps = HashMap::new();
        caps.insert(
            "gpt-4o".to_string(),
            ModelCapabilities {
                n_ctx: 128000,
                max_output_tokens: 16384,
                ..Default::default()
            },
        );

        let resolved = resolve_model_caps(&caps, "gpt-4o:ft-abc123").unwrap();
        assert_eq!(resolved.source, ModelCapsSource::Finetune);
        assert_eq!(resolved.matched_key, "gpt-4o");
    }

    #[test]
    fn test_reasoning_effort_options_serde() {
        let caps = ModelCapabilities {
            n_ctx: 128000,
            max_output_tokens: 16384,
            reasoning_effort_options: Some(vec![
                "low".to_string(),
                "medium".to_string(),
                "high".to_string(),
            ]),
            supports_thinking_budget: true,
            supports_adaptive_thinking_budget: false,
            ..Default::default()
        };
        let json = serde_json::to_string(&caps).unwrap();
        assert!(json.contains("\"reasoning_effort_options\":[\"low\",\"medium\",\"high\"]"));
        assert!(json.contains("\"supports_thinking_budget\":true"));
        assert!(json.contains("\"supports_adaptive_thinking_budget\":false"));

        let parsed: ModelCapabilities = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.reasoning_effort_options,
            Some(vec![
                "low".to_string(),
                "medium".to_string(),
                "high".to_string()
            ])
        );
        assert!(parsed.supports_thinking_budget);
        assert!(!parsed.supports_adaptive_thinking_budget);
    }

    #[test]
    fn test_caching_type_serde() {
        let json = serde_json::to_string(&CachingType::Explicit).unwrap();
        assert_eq!(json, "\"explicit\"");

        let parsed: CachingType = serde_json::from_str("\"auto\"").unwrap();
        assert_eq!(parsed, CachingType::Auto);
    }

    #[test]
    fn test_multi_slash_openrouter_models() {
        let mut caps = HashMap::new();
        caps.insert(
            "claude-3.7-sonnet".to_string(),
            ModelCapabilities {
                n_ctx: 200000,
                max_output_tokens: 16384,
                supports_tools: true,
                ..Default::default()
            },
        );

        let resolved = resolve_model_caps(&caps, "openrouter/anthropic/claude-3.7-sonnet");
        assert!(resolved.is_some());
        let resolved = resolved.unwrap();
        assert_eq!(resolved.matched_key, "claude-3.7-sonnet");
        assert_eq!(resolved.caps.n_ctx, 200000);
    }

    #[test]
    fn test_gemini_models_prefix() {
        let mut caps = HashMap::new();
        caps.insert(
            "gemini-2.0-flash".to_string(),
            ModelCapabilities {
                n_ctx: 1000000,
                max_output_tokens: 8192,
                supports_tools: true,
                supports_vision: true,
                ..Default::default()
            },
        );

        let resolved = resolve_model_caps(&caps, "models/gemini-2.0-flash");
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().matched_key, "gemini-2.0-flash");
    }

    #[test]
    fn test_capability_fields_completeness() {
        let caps = ModelCapabilities {
            n_ctx: 128000,
            max_output_tokens: 16384,
            supports_tools: true,
            supports_strict_tools: true,
            supports_vision: true,
            supports_max_completion_tokens: true,
            reasoning_effort_options: Some(vec![
                "low".to_string(),
                "medium".to_string(),
                "high".to_string(),
            ]),
            supports_thinking_budget: true,
            supports_temperature: false,
            ..Default::default()
        };

        assert!(caps.supports_strict_tools);
        assert!(caps.supports_max_completion_tokens);
        assert!(!caps.supports_temperature);
        assert_eq!(
            caps.reasoning_effort_options,
            Some(vec![
                "low".to_string(),
                "medium".to_string(),
                "high".to_string()
            ])
        );
        assert!(caps.supports_thinking_budget);
    }

    #[test]
    fn test_validation_clamps_values() {
        let mut caps = HashMap::new();
        caps.insert(
            "test-model".to_string(),
            ModelCapabilities {
                n_ctx: 999_999_999,
                max_output_tokens: 999_999_999,
                ..Default::default()
            },
        );

        validate_model_caps(&mut caps);

        let model = caps.get("test-model").unwrap();
        assert_eq!(model.n_ctx, MAX_REASONABLE_N_CTX);
        assert_eq!(model.max_output_tokens, MAX_REASONABLE_OUTPUT_TOKENS);
    }

    #[test]
    fn test_pattern_specificity_tiebreaking() {
        let mut caps = HashMap::new();
        caps.insert(
            "gpt-*".to_string(),
            ModelCapabilities {
                n_ctx: 100000,
                ..Default::default()
            },
        );
        caps.insert(
            "gpt-4*".to_string(),
            ModelCapabilities {
                n_ctx: 128000,
                ..Default::default()
            },
        );
        caps.insert(
            "gpt-4o*".to_string(),
            ModelCapabilities {
                n_ctx: 200000,
                ..Default::default()
            },
        );

        let resolved = resolve_model_caps(&caps, "gpt-4o-mini").unwrap();
        assert_eq!(resolved.matched_key, "gpt-4o*");
        assert_eq!(resolved.caps.n_ctx, 200000);
    }

    #[test]
    fn test_exact_match_over_pattern() {
        let mut caps = HashMap::new();
        caps.insert(
            "gpt-4o".to_string(),
            ModelCapabilities {
                n_ctx: 128000,
                ..Default::default()
            },
        );
        caps.insert(
            "gpt-4*".to_string(),
            ModelCapabilities {
                n_ctx: 100000,
                ..Default::default()
            },
        );

        let resolved = resolve_model_caps(&caps, "gpt-4o").unwrap();
        assert_eq!(resolved.matched_key, "gpt-4o");
        assert_eq!(resolved.caps.n_ctx, 128000);
    }

    #[test]
    fn test_normalize_tokenizer() {
        assert_eq!(normalize_tokenizer(""), "");
        assert_eq!(
            normalize_tokenizer("hf://Xenova/claude-tokenizer"),
            "hf://Xenova/claude-tokenizer"
        );
        assert_eq!(
            normalize_tokenizer("http://example.com/tokenizer.json"),
            "http://example.com/tokenizer.json"
        );
        assert_eq!(
            normalize_tokenizer("https://example.com/tokenizer.json"),
            "https://example.com/tokenizer.json"
        );
        assert_eq!(
            normalize_tokenizer("file:///path/to/tokenizer.json"),
            "file:///path/to/tokenizer.json"
        );
        assert_eq!(normalize_tokenizer("fake"), "fake");
        assert_eq!(normalize_tokenizer("fake-tokenizer"), "fake-tokenizer");
        assert_eq!(
            normalize_tokenizer("Xenova/claude-tokenizer"),
            "hf://Xenova/claude-tokenizer"
        );
        assert_eq!(
            normalize_tokenizer("meta-llama/Llama-3.3-70B"),
            "hf://meta-llama/Llama-3.3-70B"
        );
        assert_eq!(
            normalize_tokenizer("deepseek-ai/DeepSeek-V3"),
            "hf://deepseek-ai/DeepSeek-V3"
        );
        assert_eq!(normalize_tokenizer("local-tokenizer"), "local-tokenizer");
    }

    #[test]
    fn test_validate_normalizes_tokenizer() {
        let mut caps = HashMap::new();
        caps.insert(
            "test-model".to_string(),
            ModelCapabilities {
                n_ctx: 128000,
                max_output_tokens: 16384,
                tokenizer: "Xenova/claude-tokenizer".to_string(),
                ..Default::default()
            },
        );

        validate_model_caps(&mut caps);

        let model = caps.get("test-model").unwrap();
        assert_eq!(model.tokenizer, "hf://Xenova/claude-tokenizer");
    }

    #[test]
    fn test_normalize_model_name_for_matching() {
        assert_eq!(
            normalize_model_name_for_matching("claude-3-7-sonnet-latest"),
            "claude-3-7-sonnet"
        );
        assert_eq!(
            normalize_model_name_for_matching("gemini-3-pro-preview-cheap"),
            "gemini-3-pro"
        );
        assert_eq!(
            normalize_model_name_for_matching("o4-mini-deep-research"),
            "o4-mini"
        );
        assert_eq!(
            normalize_model_name_for_matching("claude-opus-4.6"),
            "claude-opus-4-6"
        );
        assert_eq!(
            normalize_model_name_for_matching("Kimi-K2-Instruct"),
            "kimi-k2-instruct"
        );
        assert_eq!(
            normalize_model_name_for_matching("MiniMax-M2.1"),
            "minimax-m2-1"
        );
        assert_eq!(
            normalize_model_name_for_matching("llama-3-70b-fp8"),
            "llama-3-70b"
        );
        assert_eq!(normalize_model_name_for_matching("gpt-4o"), "gpt-4o");
    }

    #[test]
    fn test_case_insensitive_matching() {
        let mut caps = HashMap::new();
        caps.insert(
            "kimi-k2-instruct".to_string(),
            ModelCapabilities {
                n_ctx: 131000,
                max_output_tokens: 32768,
                ..Default::default()
            },
        );

        let resolved = resolve_model_caps(&caps, "Kimi-K2-Instruct");
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().matched_key, "kimi-k2-instruct");
    }

    #[test]
    fn test_suffix_stripping_latest() {
        let mut caps = HashMap::new();
        caps.insert(
            "claude-3-7-sonnet".to_string(),
            ModelCapabilities {
                n_ctx: 200000,
                max_output_tokens: 16384,
                ..Default::default()
            },
        );

        let resolved = resolve_model_caps(&caps, "claude-3-7-sonnet-latest");
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().matched_key, "claude-3-7-sonnet");
    }

    #[test]
    fn test_suffix_stripping_compound() {
        let mut caps = HashMap::new();
        caps.insert(
            "gemini-3-pro".to_string(),
            ModelCapabilities {
                n_ctx: 1000000,
                max_output_tokens: 64000,
                ..Default::default()
            },
        );

        let resolved = resolve_model_caps(&caps, "gemini-3-pro-preview-cheap");
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().matched_key, "gemini-3-pro");
    }

    #[test]
    fn test_dot_to_dash_normalization() {
        let mut caps = HashMap::new();
        caps.insert(
            "claude-opus-4-6".to_string(),
            ModelCapabilities {
                n_ctx: 200000,
                max_output_tokens: 128000,
                ..Default::default()
            },
        );

        let resolved = resolve_model_caps(&caps, "claude-opus-4.6");
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().matched_key, "claude-opus-4-6");
    }

    #[test]
    fn test_exact_match_preferred_over_normalized() {
        let mut caps = HashMap::new();
        caps.insert(
            "gpt-4o".to_string(),
            ModelCapabilities {
                n_ctx: 128000,
                max_output_tokens: 16384,
                ..Default::default()
            },
        );
        caps.insert(
            "gpt-4o-latest".to_string(),
            ModelCapabilities {
                n_ctx: 200000,
                max_output_tokens: 32768,
                ..Default::default()
            },
        );

        // Exact match should win over suffix-stripped
        let resolved = resolve_model_caps(&caps, "gpt-4o-latest").unwrap();
        assert_eq!(resolved.matched_key, "gpt-4o-latest");
        assert_eq!(resolved.caps.n_ctx, 200000);
    }

    #[test]
    fn test_fp_suffix_stripping() {
        let mut caps = HashMap::new();
        caps.insert(
            "llama-3-70b".to_string(),
            ModelCapabilities {
                n_ctx: 128000,
                max_output_tokens: 8192,
                ..Default::default()
            },
        );

        let resolved = resolve_model_caps(&caps, "llama-3-70b-fp8");
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().matched_key, "llama-3-70b");
    }

    #[test]
    fn test_provider_prefix_with_case_mismatch() {
        let mut caps = HashMap::new();
        caps.insert(
            "minimax-m2.1".to_string(),
            ModelCapabilities {
                n_ctx: 196000,
                max_output_tokens: 16384,
                ..Default::default()
            },
        );

        // Both "refact/MiniMax-M2.1" and "MiniMax-M2.1" should resolve
        let resolved = resolve_model_caps(&caps, "refact/MiniMax-M2.1");
        assert!(resolved.is_some());

        let resolved = resolve_model_caps(&caps, "MiniMax-M2.1");
        assert!(resolved.is_some());
    }

    #[test]
    fn test_models_dev_snapshot_loads_representative_bare_models() {
        let catalog = load_models_dev_snapshot_catalog().unwrap();
        let caps = model_caps_from_models_dev_catalog(&catalog).unwrap();

        for model in [
            "gpt-4o-2024-05-13",
            "claude-3-opus-20240229",
            "qwen3.6-35b-a3b",
            "gemini-1.5-flash",
            "grok-3-fast",
        ] {
            assert!(
                resolve_model_caps(&caps, model).is_some(),
                "models.dev caps should include unambiguous bare model {model}"
            );
        }
    }
}
