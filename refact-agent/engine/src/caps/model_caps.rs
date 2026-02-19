use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use tracing::{info, warn};

use crate::global_context::GlobalContext;

static REFRESH_LOCK: OnceLock<AMutex<()>> = OnceLock::new();
static FIRST_CALL: AtomicBool = AtomicBool::new(true);

fn get_refresh_lock() -> &'static AMutex<()> {
    REFRESH_LOCK.get_or_init(|| AMutex::new(()))
}

const SMALLCLOUD_MODEL_CAPS_URL: &str = "https://inference.smallcloud.ai/v1/model-capabilities";
const CACHE_FILENAME: &str = "model-capabilities.json";
const CACHE_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

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
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedModelCaps {
    pub fetched_at: u64,
    pub models: HashMap<String, ModelCapabilities>,
}

impl CachedModelCaps {
    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now - self.fetched_at > CACHE_MAX_AGE.as_secs()
    }
}

fn get_cache_path() -> PathBuf {
    let cache_dir = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("refact");
    cache_dir.join(CACHE_FILENAME)
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
            warn!("Model {} has unreasonable n_ctx {}, clamping to {}", name, cap.n_ctx, MAX_REASONABLE_N_CTX);
            cap.n_ctx = MAX_REASONABLE_N_CTX;
        }
        if cap.max_output_tokens > MAX_REASONABLE_OUTPUT_TOKENS {
            warn!("Model {} has unreasonable max_output_tokens {}, clamping to {}", name, cap.max_output_tokens, MAX_REASONABLE_OUTPUT_TOKENS);
            cap.max_output_tokens = MAX_REASONABLE_OUTPUT_TOKENS;
        }
        cap.tokenizer = normalize_tokenizer(&cap.tokenizer);
    }
}

pub async fn load_cached_model_caps() -> Option<CachedModelCaps> {
    let cache_path = get_cache_path();

    match tokio::fs::read_to_string(&cache_path).await {
        Ok(content) => match serde_json::from_str::<CachedModelCaps>(&content) {
            Ok(mut cached) => {
                validate_model_caps(&mut cached.models);
                info!("Loaded model capabilities from cache: {} models", cached.models.len());
                Some(cached)
            }
            Err(e) => {
                warn!("Failed to parse cached model capabilities (treating as cache miss): {}", e);
                None
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            warn!("Failed to read cached model capabilities: {}", e);
            None
        }
    }
}

pub async fn save_cached_model_caps(caps: &CachedModelCaps) -> Result<(), String> {
    let cache_path = get_cache_path();

    if let Some(parent) = cache_path.parent() {
        tokio::fs::create_dir_all(parent).await
            .map_err(|e| format!("Failed to create cache directory: {}", e))?;
    }

    let content = serde_json::to_string_pretty(caps)
        .map_err(|e| format!("Failed to serialize model capabilities: {}", e))?;
    tokio::fs::write(&cache_path, content).await
        .map_err(|e| format!("Failed to write model capabilities cache: {}", e))?;
    info!("Saved model capabilities to cache: {}", cache_path.display());
    Ok(())
}

fn build_model_caps_url(address_url: &str) -> Result<String, String> {
    let address_url = address_url.trim();
    if address_url.is_empty() || address_url.eq_ignore_ascii_case("refact") {
        return Ok(SMALLCLOUD_MODEL_CAPS_URL.to_string());
    }

    let base_url = url::Url::parse(address_url)
        .map_err(|e| format!("Invalid address_url '{}': {}", address_url, e))?;
    base_url
        .join("v1/model-capabilities")
        .map(|u| u.to_string())
        .map_err(|e| format!("Failed to construct model-capabilities URL: {}", e))
}

pub async fn fetch_model_caps_from_server(
    gcx: Arc<ARwLock<GlobalContext>>,
    address_url: &str,
) -> Result<HashMap<String, ModelCapabilities>, String> {
    let http_client = gcx.read().await.http_client.clone();
    let model_caps_url = build_model_caps_url(address_url)?;

    info!("Fetching model capabilities from {}", model_caps_url);

    let response = http_client
        .get(&model_caps_url)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch model capabilities: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("Server returned status {}", status));
    }

    let models: HashMap<String, ModelCapabilities> = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse model capabilities response: {}", e))?;

    info!("Fetched {} model capabilities from server", models.len());
    Ok(models)
}

pub async fn get_model_caps(
    gcx: Arc<ARwLock<GlobalContext>>,
    address_url: &str,
    force_refresh: bool,
) -> Result<HashMap<String, ModelCapabilities>, String> {
    let _refresh_guard = get_refresh_lock().lock().await;

    let first_call = FIRST_CALL.swap(false, Ordering::SeqCst);
    let should_refresh = force_refresh || first_call;

    if !should_refresh {
        if let Some(cached) = load_cached_model_caps().await {
            if !cached.is_expired() {
                return Ok(cached.models);
            }
            info!("Cached model capabilities expired, fetching fresh data");
        }
    } else if first_call {
        info!("First model capabilities request, fetching fresh data");
    }

    match fetch_model_caps_from_server(gcx, address_url).await {
        Ok(mut models) => {
            validate_model_caps(&mut models);
            let cached = CachedModelCaps {
                fetched_at: SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                models: models.clone(),
            };
            if let Err(e) = save_cached_model_caps(&cached).await {
                warn!("Failed to save model capabilities cache: {}", e);
            }
            Ok(models)
        }
        Err(e) => {
            warn!("Failed to fetch model capabilities from server: {}", e);
            if let Some(cached) = load_cached_model_caps().await {
                warn!("Using expired cached model capabilities as fallback");
                return Ok(cached.models);
            }
            Err(e)
        }
    }
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
            let source = if canonical.is_finetune && (*name == &canonical.base_model || *name == &canonical.last_segment_base) {
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
    let normalized_names: Vec<String> = names_to_try.iter()
        .map(|n| normalize_model_name_for_matching(n))
        .collect();

    // Deduplicate normalized names while preserving order
    let mut seen = std::collections::HashSet::new();
    let unique_normalized: Vec<&String> = normalized_names.iter()
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
                } else if specificity == best_match.unwrap().2 && pattern.as_str() < best_match.unwrap().0 {
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
                    } else if specificity == best_match.unwrap().2 && pattern.as_str() < best_match.unwrap().0 {
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

    #[test]
    fn test_model_capability_lookup() {
        let mut caps = HashMap::new();
        caps.insert("gpt-4o".to_string(), ModelCapabilities {
            n_ctx: 128000,
            max_output_tokens: 16384,
            supports_tools: true,
            supports_vision: true,
            ..Default::default()
        });
        caps.insert("claude-3-5-sonnet".to_string(), ModelCapabilities {
            n_ctx: 200000,
            max_output_tokens: 8192,
            supports_tools: true,
            supports_vision: true,
            supports_pdf: true,
            ..Default::default()
        });

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
        caps.insert("claude-3-7-sonnet*".to_string(), ModelCapabilities {
            n_ctx: 200000,
            max_output_tokens: 16384,
            supports_tools: true,
            ..Default::default()
        });
        caps.insert("gpt-4*".to_string(), ModelCapabilities {
            n_ctx: 128000,
            max_output_tokens: 8192,
            supports_tools: true,
            ..Default::default()
        });

        let resolved = resolve_model_caps(&caps, "claude-3-7-sonnet-latest").unwrap();
        assert_eq!(resolved.matched_key, "claude-3-7-sonnet*");
        assert_eq!(resolved.caps.n_ctx, 200000);

        let resolved = resolve_model_caps(&caps, "gpt-4o").unwrap();
        assert_eq!(resolved.matched_key, "gpt-4*");
    }

    #[test]
    fn test_finetune_source() {
        let mut caps = HashMap::new();
        caps.insert("gpt-4o".to_string(), ModelCapabilities {
            n_ctx: 128000,
            max_output_tokens: 16384,
            ..Default::default()
        });

        let resolved = resolve_model_caps(&caps, "gpt-4o:ft-abc123").unwrap();
        assert_eq!(resolved.source, ModelCapsSource::Finetune);
        assert_eq!(resolved.matched_key, "gpt-4o");
    }

    #[test]
    fn test_reasoning_effort_options_serde() {
        let caps = ModelCapabilities {
            n_ctx: 128000,
            max_output_tokens: 16384,
            reasoning_effort_options: Some(vec!["low".to_string(), "medium".to_string(), "high".to_string()]),
            supports_thinking_budget: true,
            supports_adaptive_thinking_budget: false,
            ..Default::default()
        };
        let json = serde_json::to_string(&caps).unwrap();
        assert!(json.contains("\"reasoning_effort_options\":[\"low\",\"medium\",\"high\"]"));
        assert!(json.contains("\"supports_thinking_budget\":true"));
        assert!(json.contains("\"supports_adaptive_thinking_budget\":false"));

        let parsed: ModelCapabilities = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.reasoning_effort_options, Some(vec!["low".to_string(), "medium".to_string(), "high".to_string()]));
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
        caps.insert("claude-3.7-sonnet".to_string(), ModelCapabilities {
            n_ctx: 200000,
            max_output_tokens: 16384,
            supports_tools: true,
            ..Default::default()
        });

        let resolved = resolve_model_caps(&caps, "openrouter/anthropic/claude-3.7-sonnet");
        assert!(resolved.is_some());
        let resolved = resolved.unwrap();
        assert_eq!(resolved.matched_key, "claude-3.7-sonnet");
        assert_eq!(resolved.caps.n_ctx, 200000);
    }

    #[test]
    fn test_gemini_models_prefix() {
        let mut caps = HashMap::new();
        caps.insert("gemini-2.0-flash".to_string(), ModelCapabilities {
            n_ctx: 1000000,
            max_output_tokens: 8192,
            supports_tools: true,
            supports_vision: true,
            ..Default::default()
        });

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
            reasoning_effort_options: Some(vec!["low".to_string(), "medium".to_string(), "high".to_string()]),
            supports_thinking_budget: true,
            supports_temperature: false,
            ..Default::default()
        };

        assert!(caps.supports_strict_tools);
        assert!(caps.supports_max_completion_tokens);
        assert!(!caps.supports_temperature);
        assert_eq!(caps.reasoning_effort_options, Some(vec!["low".to_string(), "medium".to_string(), "high".to_string()]));
        assert!(caps.supports_thinking_budget);
    }

    #[test]
    fn test_validation_clamps_values() {
        let mut caps = HashMap::new();
        caps.insert("test-model".to_string(), ModelCapabilities {
            n_ctx: 999_999_999,
            max_output_tokens: 999_999_999,
            ..Default::default()
        });

        validate_model_caps(&mut caps);

        let model = caps.get("test-model").unwrap();
        assert_eq!(model.n_ctx, MAX_REASONABLE_N_CTX);
        assert_eq!(model.max_output_tokens, MAX_REASONABLE_OUTPUT_TOKENS);
    }

    #[test]
    fn test_pattern_specificity_tiebreaking() {
        let mut caps = HashMap::new();
        caps.insert("gpt-*".to_string(), ModelCapabilities {
            n_ctx: 100000,
            ..Default::default()
        });
        caps.insert("gpt-4*".to_string(), ModelCapabilities {
            n_ctx: 128000,
            ..Default::default()
        });
        caps.insert("gpt-4o*".to_string(), ModelCapabilities {
            n_ctx: 200000,
            ..Default::default()
        });

        let resolved = resolve_model_caps(&caps, "gpt-4o-mini").unwrap();
        assert_eq!(resolved.matched_key, "gpt-4o*");
        assert_eq!(resolved.caps.n_ctx, 200000);
    }

    #[test]
    fn test_exact_match_over_pattern() {
        let mut caps = HashMap::new();
        caps.insert("gpt-4o".to_string(), ModelCapabilities {
            n_ctx: 128000,
            ..Default::default()
        });
        caps.insert("gpt-4*".to_string(), ModelCapabilities {
            n_ctx: 100000,
            ..Default::default()
        });

        let resolved = resolve_model_caps(&caps, "gpt-4o").unwrap();
        assert_eq!(resolved.matched_key, "gpt-4o");
        assert_eq!(resolved.caps.n_ctx, 128000);
    }

    #[test]
    fn test_normalize_tokenizer() {
        assert_eq!(normalize_tokenizer(""), "");
        assert_eq!(normalize_tokenizer("hf://Xenova/claude-tokenizer"), "hf://Xenova/claude-tokenizer");
        assert_eq!(normalize_tokenizer("http://example.com/tokenizer.json"), "http://example.com/tokenizer.json");
        assert_eq!(normalize_tokenizer("https://example.com/tokenizer.json"), "https://example.com/tokenizer.json");
        assert_eq!(normalize_tokenizer("file:///path/to/tokenizer.json"), "file:///path/to/tokenizer.json");
        assert_eq!(normalize_tokenizer("fake"), "fake");
        assert_eq!(normalize_tokenizer("fake-tokenizer"), "fake-tokenizer");
        assert_eq!(normalize_tokenizer("Xenova/claude-tokenizer"), "hf://Xenova/claude-tokenizer");
        assert_eq!(normalize_tokenizer("meta-llama/Llama-3.3-70B"), "hf://meta-llama/Llama-3.3-70B");
        assert_eq!(normalize_tokenizer("deepseek-ai/DeepSeek-V3"), "hf://deepseek-ai/DeepSeek-V3");
        assert_eq!(normalize_tokenizer("local-tokenizer"), "local-tokenizer");
    }

    #[test]
    fn test_validate_normalizes_tokenizer() {
        let mut caps = HashMap::new();
        caps.insert("test-model".to_string(), ModelCapabilities {
            n_ctx: 128000,
            max_output_tokens: 16384,
            tokenizer: "Xenova/claude-tokenizer".to_string(),
            ..Default::default()
        });

        validate_model_caps(&mut caps);

        let model = caps.get("test-model").unwrap();
        assert_eq!(model.tokenizer, "hf://Xenova/claude-tokenizer");
    }

    #[test]
    fn test_normalize_model_name_for_matching() {
        assert_eq!(normalize_model_name_for_matching("claude-3-7-sonnet-latest"), "claude-3-7-sonnet");
        assert_eq!(normalize_model_name_for_matching("gemini-3-pro-preview-cheap"), "gemini-3-pro");
        assert_eq!(normalize_model_name_for_matching("o4-mini-deep-research"), "o4-mini");
        assert_eq!(normalize_model_name_for_matching("claude-opus-4.6"), "claude-opus-4-6");
        assert_eq!(normalize_model_name_for_matching("Kimi-K2-Instruct"), "kimi-k2-instruct");
        assert_eq!(normalize_model_name_for_matching("MiniMax-M2.1"), "minimax-m2-1");
        assert_eq!(normalize_model_name_for_matching("llama-3-70b-fp8"), "llama-3-70b");
        assert_eq!(normalize_model_name_for_matching("gpt-4o"), "gpt-4o");
    }

    #[test]
    fn test_case_insensitive_matching() {
        let mut caps = HashMap::new();
        caps.insert("kimi-k2-instruct".to_string(), ModelCapabilities {
            n_ctx: 131000,
            max_output_tokens: 32768,
            ..Default::default()
        });

        let resolved = resolve_model_caps(&caps, "Kimi-K2-Instruct");
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().matched_key, "kimi-k2-instruct");
    }

    #[test]
    fn test_suffix_stripping_latest() {
        let mut caps = HashMap::new();
        caps.insert("claude-3-7-sonnet".to_string(), ModelCapabilities {
            n_ctx: 200000,
            max_output_tokens: 16384,
            ..Default::default()
        });

        let resolved = resolve_model_caps(&caps, "claude-3-7-sonnet-latest");
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().matched_key, "claude-3-7-sonnet");
    }

    #[test]
    fn test_suffix_stripping_compound() {
        let mut caps = HashMap::new();
        caps.insert("gemini-3-pro".to_string(), ModelCapabilities {
            n_ctx: 1000000,
            max_output_tokens: 64000,
            ..Default::default()
        });

        let resolved = resolve_model_caps(&caps, "gemini-3-pro-preview-cheap");
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().matched_key, "gemini-3-pro");
    }

    #[test]
    fn test_dot_to_dash_normalization() {
        let mut caps = HashMap::new();
        caps.insert("claude-opus-4-6".to_string(), ModelCapabilities {
            n_ctx: 200000,
            max_output_tokens: 128000,
            ..Default::default()
        });

        let resolved = resolve_model_caps(&caps, "claude-opus-4.6");
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().matched_key, "claude-opus-4-6");
    }

    #[test]
    fn test_exact_match_preferred_over_normalized() {
        let mut caps = HashMap::new();
        caps.insert("gpt-4o".to_string(), ModelCapabilities {
            n_ctx: 128000,
            max_output_tokens: 16384,
            ..Default::default()
        });
        caps.insert("gpt-4o-latest".to_string(), ModelCapabilities {
            n_ctx: 200000,
            max_output_tokens: 32768,
            ..Default::default()
        });

        // Exact match should win over suffix-stripped
        let resolved = resolve_model_caps(&caps, "gpt-4o-latest").unwrap();
        assert_eq!(resolved.matched_key, "gpt-4o-latest");
        assert_eq!(resolved.caps.n_ctx, 200000);
    }

    #[test]
    fn test_fp_suffix_stripping() {
        let mut caps = HashMap::new();
        caps.insert("llama-3-70b".to_string(), ModelCapabilities {
            n_ctx: 128000,
            max_output_tokens: 8192,
            ..Default::default()
        });

        let resolved = resolve_model_caps(&caps, "llama-3-70b-fp8");
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().matched_key, "llama-3-70b");
    }

    #[test]
    fn test_provider_prefix_with_case_mismatch() {
        let mut caps = HashMap::new();
        caps.insert("minimax-m2.1".to_string(), ModelCapabilities {
            n_ctx: 196000,
            max_output_tokens: 16384,
            ..Default::default()
        });

        // Both "refact/MiniMax-M2.1" and "MiniMax-M2.1" should resolve
        let resolved = resolve_model_caps(&caps, "refact/MiniMax-M2.1");
        assert!(resolved.is_some());

        let resolved = resolve_model_caps(&caps, "MiniMax-M2.1");
        assert!(resolved.is_some());
    }
}
