use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AMutex;
use tracing::warn;

use crate::model_caps::ModelCapabilities;
use crate::provider_types::{ModelPricing, ModelPricingTier};

pub const MODELS_DEV_API_URL: &str = "https://models.dev/api.json";
const MODELS_DEV_CACHE_DIR: &str = "models_dev";
const MODELS_DEV_CACHE_FILE: &str = "api.json";
const MODELS_DEV_MAX_CATALOG_BYTES: usize = 25 * 1024 * 1024;
const MODELS_DEV_SNAPSHOT: &str = include_str!("models_dev_snapshot.json");
const REASONING_CONTROLS: &str = include_str!("reasoning_controls.json");
const REQUIRED_MODELS_DEV_PROVIDERS: &[&str] = &[
    "openai",
    "anthropic",
    "deepseek",
    "alibaba",
    "moonshotai",
    "minimax",
    "github-copilot",
];
const REQUIRED_ZAI_PROVIDER_ALIASES: &[&str] = &["zai", "zhipuai"];
static MODELS_DEV_CACHE_WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);
static MODELS_DEV_CACHE_WRITE_MUTEX: OnceLock<AMutex<()>> = OnceLock::new();
static REASONING_CONTROL_RULES: OnceLock<Vec<ReasoningControlRule>> = OnceLock::new();

#[derive(Debug, Clone, Deserialize)]
struct ReasoningControlRule {
    #[serde(default)]
    provider_ids: Vec<String>,
    #[serde(default)]
    family_prefixes: Vec<String>,
    #[serde(default)]
    model_prefixes: Vec<String>,
    #[serde(default)]
    reasoning_effort_options: Option<Vec<String>>,
    #[serde(default)]
    supports_thinking_budget: bool,
    #[serde(default)]
    supports_adaptive_thinking_budget: bool,
}

#[derive(Debug, Clone, Default)]
struct ReasoningControls {
    reasoning_effort_options: Option<Vec<String>>,
    supports_thinking_budget: bool,
    supports_adaptive_thinking_budget: bool,
}

pub type ModelsDevCatalog = HashMap<String, ModelsDevProvider>;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ModelsDevProvider {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub api: Option<String>,
    #[serde(default)]
    pub npm: Option<String>,
    #[serde(default)]
    pub models: HashMap<String, ModelsDevModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ModelsDevModel {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub reasoning: Option<bool>,
    #[serde(default)]
    pub temperature: Option<bool>,
    #[serde(default)]
    pub tool_call: Option<bool>,
    #[serde(default)]
    pub structured_output: Option<bool>,
    #[serde(default)]
    pub attachment: Option<bool>,
    #[serde(default)]
    pub cost: Option<ModelsDevCost>,
    #[serde(default)]
    pub limit: Option<ModelsDevLimit>,
    #[serde(default)]
    pub modalities: Option<ModelsDevModalities>,
    #[serde(default)]
    pub provider: Option<ModelsDevModelProvider>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub interleaved: Option<serde_json::Value>,
    #[serde(default)]
    pub experimental: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq)]
pub struct ModelsDevCost {
    #[serde(default)]
    pub input: Option<f64>,
    #[serde(default)]
    pub output: Option<f64>,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
    #[serde(default)]
    pub context_over_200k: Option<ModelsDevCostTier>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq)]
pub struct ModelsDevCostTier {
    #[serde(default)]
    pub input: Option<f64>,
    #[serde(default)]
    pub output: Option<f64>,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ModelsDevLimit {
    #[serde(default)]
    pub context: Option<usize>,
    #[serde(default)]
    pub input: Option<usize>,
    #[serde(default)]
    pub output: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ModelsDevModalities {
    #[serde(default)]
    pub input: Vec<String>,
    #[serde(default)]
    pub output: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ModelsDevModelProvider {
    #[serde(default)]
    pub api: Option<String>,
    #[serde(default)]
    pub npm: Option<String>,
}

struct NoDuplicateJson;

struct NoDuplicateJsonVisitor;

impl<'de> Deserialize<'de> for NoDuplicateJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(NoDuplicateJsonVisitor)
    }
}

impl<'de> Visitor<'de> for NoDuplicateJsonVisitor {
    type Value = NoDuplicateJson;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any valid JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(NoDuplicateJson)
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(NoDuplicateJson)
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(NoDuplicateJson)
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(NoDuplicateJson)
    }

    fn visit_str<E>(self, _value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(NoDuplicateJson)
    }

    fn visit_string<E>(self, _value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(NoDuplicateJson)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(NoDuplicateJson)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(NoDuplicateJson)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Deserialize::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while seq.next_element::<NoDuplicateJson>()?.is_some() {}
        Ok(NoDuplicateJson)
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(de::Error::custom(format!(
                    "duplicate JSON object key '{key}'"
                )));
            }
            let _: NoDuplicateJson = map.next_value()?;
        }
        Ok(NoDuplicateJson)
    }
}

pub fn parse_catalog_json(json: &str) -> Result<ModelsDevCatalog, String> {
    validate_models_dev_body_size(json.len())?;
    validate_catalog_raw_json_keys(json)?;
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| format!("Failed to parse models.dev catalog: {e}"))?;
    validate_catalog_value_schema(&value)?;
    let catalog: ModelsDevCatalog = serde_json::from_value(value)
        .map_err(|e| format!("Failed to parse models.dev catalog: {e}"))?;
    normalize_and_validate_catalog(catalog)
}

fn validate_catalog_raw_json_keys(json: &str) -> Result<(), String> {
    serde_json::from_str::<NoDuplicateJson>(json)
        .map(|_| ())
        .map_err(|e| format!("Failed to parse models.dev catalog: {e}"))
}

fn parse_required_project_catalog_json(
    json: &str,
    source: &str,
) -> Result<ModelsDevCatalog, String> {
    let catalog = parse_catalog_json(json).map_err(|e| format!("{source} is invalid: {e}"))?;
    validate_required_project_providers(&catalog)
        .map_err(|e| format!("{source} is incomplete: {e}"))?;
    Ok(catalog)
}

pub fn load_models_dev_snapshot_catalog() -> Result<ModelsDevCatalog, String> {
    parse_required_project_catalog_json(MODELS_DEV_SNAPSHOT, "Bundled models.dev snapshot")
}

pub fn models_dev_cache_path(cache_dir: &Path) -> PathBuf {
    cache_dir
        .join(MODELS_DEV_CACHE_DIR)
        .join(MODELS_DEV_CACHE_FILE)
}

pub fn get_provider<'a>(
    catalog: &'a ModelsDevCatalog,
    provider_id: &str,
) -> Option<&'a ModelsDevProvider> {
    catalog
        .get(provider_id)
        .or_else(|| catalog.values().find(|provider| provider.id == provider_id))
}

#[allow(dead_code)]
pub fn get_model<'a>(
    catalog: &'a ModelsDevCatalog,
    provider_id: &str,
    model_id: &str,
) -> Option<&'a ModelsDevModel> {
    let provider = get_provider(catalog, provider_id)?;
    provider
        .models
        .get(model_id)
        .or_else(|| provider.models.values().find(|model| model.id == model_id))
}

pub fn model_cost_to_pricing(model: &ModelsDevModel) -> Option<ModelPricing> {
    cost_to_pricing(model.cost.as_ref()?)
}

pub fn cost_to_pricing(cost: &ModelsDevCost) -> Option<ModelPricing> {
    let pricing = ModelPricing {
        prompt: cost.input?,
        generated: cost.output?,
        cache_read: cost.cache_read,
        cache_creation: cost.cache_write,
        context_over_200k: cost.context_over_200k.map(|tier| ModelPricingTier {
            prompt: tier.input,
            generated: tier.output,
            cache_read: tier.cache_read,
            cache_creation: tier.cache_write,
        }),
    };
    pricing.is_valid().then_some(pricing)
}

pub fn models_dev_catalog_to_model_caps(
    catalog: &ModelsDevCatalog,
) -> Result<HashMap<String, ModelCapabilities>, String> {
    let bare_alias_owners = collect_bare_alias_owners(catalog);
    let mut caps = HashMap::new();

    for (provider_key, provider) in catalog {
        let provider_aliases = provider_aliases(provider_key, provider);
        for (model_key, model) in &provider.models {
            if !is_active_chat_model(model) {
                continue;
            }

            let model_aliases = model_aliases(model_key, model);
            let owner = model_owner_key(provider_key, model_key);
            let model_caps = models_dev_model_to_model_caps(provider_key, provider, model);

            for provider_alias in &provider_aliases {
                for model_alias in &model_aliases {
                    let key = format!("{provider_alias}/{model_alias}");
                    insert_qualified_model_caps(&mut caps, key, &model_caps)?;
                }
            }

            for model_alias in &model_aliases {
                if model_alias.contains('/') {
                    continue;
                }
                let collision_key = bare_alias_collision_key(model_alias);
                if bare_alias_owners
                    .get(&collision_key)
                    .is_some_and(|owners| owners.len() == 1 && owners.contains(&owner))
                {
                    insert_bare_model_caps(&mut caps, model_alias.to_string(), &model_caps);
                }
            }
        }
    }

    if caps.is_empty() {
        return Err("models.dev catalog produced no model capabilities".to_string());
    }

    Ok(caps)
}

fn models_dev_model_to_model_caps(
    provider_key: &str,
    provider: &ModelsDevProvider,
    model: &ModelsDevModel,
) -> ModelCapabilities {
    let limit = model.limit.as_ref();
    let input_modalities = model
        .modalities
        .as_ref()
        .map(|modalities| modalities.input.as_slice())
        .unwrap_or(&[]);
    let reasoning_controls = reasoning_controls_for_model(provider_key, provider, model);
    ModelCapabilities {
        n_ctx: limit.and_then(|limit| limit.context).unwrap_or_default(),
        max_output_tokens: limit.and_then(|limit| limit.output).unwrap_or_default(),
        supports_tools: model.tool_call == Some(true),
        supports_parallel_tools: model.tool_call == Some(true),
        supports_strict_tools: model.structured_output == Some(true),
        supports_vision: has_modality(input_modalities, "image"),
        supports_video: has_modality(input_modalities, "video"),
        supports_audio: has_modality(input_modalities, "audio"),
        supports_pdf: has_modality(input_modalities, "pdf"),
        supports_temperature: model.temperature.unwrap_or(true),
        reasoning_effort_options: reasoning_controls.reasoning_effort_options,
        supports_thinking_budget: reasoning_controls.supports_thinking_budget,
        supports_adaptive_thinking_budget: reasoning_controls.supports_adaptive_thinking_budget,
        supports_cache_control: models_dev_model_supports_cache_control(model),
        tokenizer: "fake".to_string(),
        pricing: model_cost_to_pricing(model),
        raw_cost: model
            .cost
            .as_ref()
            .and_then(|cost| serde_json::to_value(cost).ok()),
        status: non_empty_status(model.status.as_deref()),
        ..Default::default()
    }
}

fn models_dev_model_supports_cache_control(model: &ModelsDevModel) -> bool {
    model.cost.as_ref().is_some_and(|cost| {
        cost.cache_read.is_some()
            || cost.cache_write.is_some()
            || cost
                .context_over_200k
                .is_some_and(|tier| tier.cache_read.is_some() || tier.cache_write.is_some())
    })
}

fn reasoning_control_rules() -> &'static [ReasoningControlRule] {
    REASONING_CONTROL_RULES
        .get_or_init(|| {
            serde_json::from_str(REASONING_CONTROLS)
                .expect("reasoning_controls.json must be valid ReasoningControlRule[]")
        })
        .as_slice()
}

fn reasoning_controls_for_model(
    provider_key: &str,
    provider: &ModelsDevProvider,
    model: &ModelsDevModel,
) -> ReasoningControls {
    if model.reasoning != Some(true) {
        return ReasoningControls::default();
    }

    reasoning_control_rules()
        .iter()
        .find(|rule| reasoning_rule_matches(rule, provider_key, provider, model))
        .map(|rule| ReasoningControls {
            reasoning_effort_options: rule.reasoning_effort_options.clone(),
            supports_thinking_budget: rule.supports_thinking_budget,
            supports_adaptive_thinking_budget: rule.supports_adaptive_thinking_budget,
        })
        .unwrap_or_default()
}

fn reasoning_rule_matches(
    rule: &ReasoningControlRule,
    provider_key: &str,
    provider: &ModelsDevProvider,
    model: &ModelsDevModel,
) -> bool {
    let model_id = model.id.to_ascii_lowercase();
    let model_name = model.name.to_ascii_lowercase();
    let family = model
        .family
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let provider_key = provider_key.to_ascii_lowercase();
    let provider_id = provider.id.to_ascii_lowercase();
    let provider_npm = provider
        .npm
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let model_npm = model
        .provider
        .as_ref()
        .and_then(|provider| provider.npm.as_deref())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let model_api = model
        .provider
        .as_ref()
        .and_then(|provider| provider.api.as_deref())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let provider_matches = rule.provider_ids.is_empty()
        || rule.provider_ids.iter().any(|expected| {
            let expected = expected.to_ascii_lowercase();
            provider_key == expected
                || provider_id == expected
                || provider_npm.contains(&expected)
                || model_npm.contains(&expected)
                || model_api.contains(&format!("/{expected}/"))
                || (expected == "anthropic" && model_id.starts_with("claude-"))
                || (expected == "google" && model_id.starts_with("gemini-"))
                || (expected == "openai" && is_openai_reasoning_model_id(&model_id))
        });

    if !provider_matches {
        return false;
    }

    let model_matches = rule.model_prefixes.is_empty()
        || rule
            .model_prefixes
            .iter()
            .any(|prefix| model_id_or_last_segment_starts_with(&model_id, &model_name, prefix));
    let family_matches = rule.family_prefixes.is_empty()
        || rule.family_prefixes.iter().any(|prefix| {
            family.starts_with(&prefix.to_ascii_lowercase())
                || model_id_or_last_segment_starts_with(&model_id, &model_name, prefix)
        });

    model_matches && family_matches
}

fn model_id_or_last_segment_starts_with(model_id: &str, model_name: &str, prefix: &str) -> bool {
    let prefix = prefix.to_ascii_lowercase();
    let model_last_segment = model_id.rsplit('/').next().unwrap_or(model_id);
    let name_last_segment = model_name.rsplit('/').next().unwrap_or(model_name);
    model_id.starts_with(&prefix)
        || model_last_segment.starts_with(&prefix)
        || model_name.starts_with(&prefix)
        || name_last_segment.starts_with(&prefix)
}

fn is_openai_reasoning_model_id(model_id: &str) -> bool {
    let model_id = model_id.rsplit('/').next().unwrap_or(model_id);
    model_id.starts_with("gpt-")
        || model_id.starts_with("o1")
        || model_id.starts_with("o3")
        || model_id.starts_with("o4")
        || model_id.starts_with("o5")
}

fn is_active_chat_model(model: &ModelsDevModel) -> bool {
    if model
        .status
        .as_deref()
        .is_some_and(|status| status.eq_ignore_ascii_case("deprecated"))
    {
        return false;
    }

    let Some(modalities) = model.modalities.as_ref() else {
        return false;
    };
    if !has_modality(&modalities.input, "text") || !has_modality(&modalities.output, "text") {
        return false;
    }
    if modalities
        .output
        .iter()
        .any(|modality| !modality.eq_ignore_ascii_case("text"))
    {
        return false;
    }

    let Some(limit) = model.limit.as_ref() else {
        return false;
    };
    if limit.context.unwrap_or_default() == 0 {
        return false;
    }

    !is_special_purpose_model(model)
}

fn is_special_purpose_model(model: &ModelsDevModel) -> bool {
    let searchable = [
        model.id.as_str(),
        model.name.as_str(),
        model.family.as_deref().unwrap_or_default(),
    ]
    .join(" ")
    .to_ascii_lowercase();

    [
        "embedding",
        "embed-",
        "-embed",
        "rerank",
        "re-rank",
        "ocr",
        "whisper",
        "transcrib",
        "speech-to-text",
        "text-to-speech",
        "tts",
        "moderation",
        "classifier",
        "classify",
        "router",
        "guard",
        "safety",
        "gpt-image",
        "dall-e",
        "stable-diffusion",
        "sdxl",
    ]
    .iter()
    .any(|marker| searchable.contains(marker))
}

fn non_empty_status(status: Option<&str>) -> Option<String> {
    status
        .map(str::trim)
        .filter(|status| !status.is_empty())
        .map(str::to_string)
}

fn has_modality(modalities: &[String], expected: &str) -> bool {
    modalities
        .iter()
        .any(|modality| modality.eq_ignore_ascii_case(expected))
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

fn collect_bare_alias_owners(catalog: &ModelsDevCatalog) -> HashMap<String, HashSet<String>> {
    let mut owners: HashMap<String, HashSet<String>> = HashMap::new();
    for (provider_key, provider) in catalog {
        for (model_key, model) in &provider.models {
            if !is_active_chat_model(model) {
                continue;
            }

            let owner = model_owner_key(provider_key, model_key);
            for alias in model_aliases(model_key, model) {
                if alias.contains('/') {
                    continue;
                }
                owners
                    .entry(bare_alias_collision_key(&alias))
                    .or_default()
                    .insert(owner.clone());
            }
        }
    }
    owners
}

fn model_owner_key(provider_key: &str, model_key: &str) -> String {
    format!("{provider_key}/{model_key}")
}

fn bare_alias_collision_key(alias: &str) -> String {
    alias.to_ascii_lowercase().replace('.', "-")
}

fn insert_qualified_model_caps(
    caps: &mut HashMap<String, ModelCapabilities>,
    key: String,
    model_caps: &ModelCapabilities,
) -> Result<(), String> {
    if caps.contains_key(&key) {
        return Err(format!(
            "models.dev capability key '{key}' would be duplicated"
        ));
    }
    caps.insert(key, model_caps.clone());
    Ok(())
}

fn insert_bare_model_caps(
    caps: &mut HashMap<String, ModelCapabilities>,
    key: String,
    model_caps: &ModelCapabilities,
) {
    if caps.contains_key(&key) {
        warn!("Skipping ambiguous models.dev bare capability key '{key}'");
        return;
    }
    caps.insert(key, model_caps.clone());
}

pub fn validate_required_project_providers(catalog: &ModelsDevCatalog) -> Result<(), String> {
    for provider_id in REQUIRED_MODELS_DEV_PROVIDERS {
        let provider = get_provider(catalog, provider_id)
            .ok_or_else(|| format!("required provider '{provider_id}' is missing"))?;
        if provider.models.is_empty() {
            return Err(format!("required provider '{provider_id}' has no models"));
        }
    }

    let zai_provider_exists = REQUIRED_ZAI_PROVIDER_ALIASES
        .iter()
        .any(|provider_id| get_provider(catalog, provider_id).is_some());
    let zai_provider_has_models = REQUIRED_ZAI_PROVIDER_ALIASES.iter().any(|provider_id| {
        get_provider(catalog, provider_id).is_some_and(|provider| !provider.models.is_empty())
    });
    if !zai_provider_has_models {
        let provider_group = REQUIRED_ZAI_PROVIDER_ALIASES.join(" or ");
        if zai_provider_exists {
            return Err(format!(
                "required provider group '{provider_group}' has no models"
            ));
        }
        return Err(format!(
            "required provider group '{provider_group}' is missing"
        ));
    }

    Ok(())
}

pub async fn load_models_dev_catalog_from_cache_or_snapshot(
    cache_dir: &Path,
) -> Result<ModelsDevCatalog, String> {
    let cache_path = models_dev_cache_path(cache_dir);
    match tokio::fs::read_to_string(&cache_path).await {
        Ok(contents) => {
            match parse_required_project_catalog_json(&contents, "models.dev runtime cache") {
                Ok(catalog) => Ok(catalog),
                Err(e) => {
                    warn!(
                        "models.dev runtime cache '{}' is invalid: {e}; using bundled snapshot",
                        cache_path.display()
                    );
                    load_models_dev_snapshot_catalog()
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => load_models_dev_snapshot_catalog(),
        Err(e) => {
            warn!(
                "Failed to read models.dev runtime cache '{}': {e}; using bundled snapshot",
                cache_path.display()
            );
            load_models_dev_snapshot_catalog()
        }
    }
}

pub async fn write_models_dev_cache(cache_dir: &Path, contents: &str) -> Result<(), String> {
    parse_required_project_catalog_json(contents, "models.dev runtime cache")?;
    write_models_dev_cache_atomic(cache_dir, contents).await
}

fn models_dev_cache_write_mutex() -> &'static AMutex<()> {
    MODELS_DEV_CACHE_WRITE_MUTEX.get_or_init(|| AMutex::new(()))
}

async fn write_models_dev_cache_atomic(cache_dir: &Path, contents: &str) -> Result<(), String> {
    validate_models_dev_body_size(contents.len())?;
    let _write_guard = models_dev_cache_write_mutex().lock().await;
    let cache_path = models_dev_cache_path(cache_dir);
    if let Some(parent) = cache_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create models.dev cache directory: {e}"))?;
    }
    let tmp_path = unique_models_dev_cache_tmp_path(&cache_path);
    if let Err(e) = tokio::fs::write(&tmp_path, contents).await {
        cleanup_models_dev_cache_tmp_path(&tmp_path).await;
        return Err(format!("Failed to write models.dev cache temp file: {e}"));
    }
    if let Err(e) = replace_models_dev_cache_file(&tmp_path, &cache_path).await {
        cleanup_models_dev_cache_tmp_path(&tmp_path).await;
        return Err(e);
    }
    Ok(())
}

async fn replace_models_dev_cache_file(tmp_path: &Path, cache_path: &Path) -> Result<(), String> {
    match tokio::fs::rename(tmp_path, cache_path).await {
        Ok(()) => Ok(()),
        Err(first_error) => {
            let backup_path = unique_models_dev_cache_backup_path(cache_path);
            if let Err(backup_error) = tokio::fs::rename(cache_path, &backup_path).await {
                return Err(format!(
                    concat!(
                        "Failed to replace models.dev cache file: {}; ",
                        "failed to move existing cache file aside: {}"
                    ),
                    first_error, backup_error
                ));
            }

            match tokio::fs::rename(tmp_path, cache_path).await {
                Ok(()) => {
                    cleanup_models_dev_cache_tmp_path(&backup_path).await;
                    Ok(())
                }
                Err(replace_error) => match tokio::fs::rename(&backup_path, cache_path).await {
                    Ok(()) => Err(format!(
                        "Failed to replace models.dev cache file after moving existing file aside: {replace_error}"
                    )),
                    Err(restore_error) => Err(format!(
                        concat!(
                            "Failed to replace models.dev cache file after moving existing file ",
                            "aside: {}; failed to restore previous cache file: {}"
                        ),
                        replace_error,
                        restore_error
                    )),
                },
            }
        }
    }
}

async fn cleanup_models_dev_cache_tmp_path(tmp_path: &Path) {
    let _ = tokio::fs::remove_file(tmp_path).await;
}

fn validate_catalog_value_schema(value: &serde_json::Value) -> Result<(), String> {
    let providers = value
        .as_object()
        .ok_or_else(|| "models.dev catalog root must be a JSON object".to_string())?;
    if providers.is_empty() {
        return Err("models.dev catalog is empty".to_string());
    }

    let mut model_count = 0usize;
    for (provider_key, provider_value) in providers {
        validate_non_empty_catalog_field("provider key", provider_key)?;
        let provider = provider_value
            .as_object()
            .ok_or_else(|| format!("models.dev provider '{provider_key}' must be a JSON object"))?;
        let models_value = provider.get("models").ok_or_else(|| {
            format!("models.dev provider '{provider_key}' is missing models object")
        })?;
        let models = models_value.as_object().ok_or_else(|| {
            format!("models.dev provider '{provider_key}' models must be a JSON object")
        })?;
        if models.is_empty() {
            return Err(format!(
                "models.dev provider '{provider_key}' has no models"
            ));
        }
        for (model_key, model_value) in models {
            validate_non_empty_catalog_field(
                &format!("model key in provider '{provider_key}'"),
                model_key,
            )?;
            model_value.as_object().ok_or_else(|| {
                format!(
                    "models.dev model '{model_key}' in provider '{provider_key}' must be a JSON object"
                )
            })?;
        }
        model_count += models.len();
    }

    if model_count == 0 {
        return Err("models.dev catalog contains no models".to_string());
    }

    Ok(())
}

fn normalize_and_validate_catalog(catalog: ModelsDevCatalog) -> Result<ModelsDevCatalog, String> {
    if catalog.is_empty() {
        return Err("models.dev catalog is empty".to_string());
    }

    let mut provider_aliases = HashMap::new();
    let mut model_count = 0usize;
    for (provider_key, provider) in catalog.iter() {
        validate_non_empty_catalog_field("provider key", provider_key)?;
        validate_non_empty_catalog_field(&format!("provider '{provider_key}' id"), &provider.id)?;
        insert_catalog_alias(
            &mut provider_aliases,
            "provider",
            provider_key,
            provider_key,
        )?;
        insert_catalog_alias(
            &mut provider_aliases,
            "provider",
            &provider.id,
            provider_key,
        )?;
        if provider.models.is_empty() {
            return Err(format!(
                "models.dev provider '{provider_key}' has no models"
            ));
        }

        let mut model_aliases = HashMap::new();
        for (model_key, model) in provider.models.iter() {
            validate_non_empty_catalog_field(
                &format!("model key in provider '{provider_key}'"),
                model_key,
            )?;
            validate_non_empty_catalog_field(
                &format!("model '{model_key}' id in provider '{provider_key}'"),
                &model.id,
            )?;
            let model_alias_context = format!("model in provider '{provider_key}'");
            insert_catalog_alias(
                &mut model_aliases,
                &model_alias_context,
                model_key,
                model_key,
            )?;
            insert_catalog_alias(
                &mut model_aliases,
                &model_alias_context,
                &model.id,
                model_key,
            )?;
            model_count += 1;
        }
    }

    if model_count == 0 {
        return Err("models.dev catalog contains no models".to_string());
    }

    Ok(catalog)
}

fn validate_non_empty_catalog_field(context: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("models.dev {context} must be non-empty"));
    }
    Ok(())
}

fn insert_catalog_alias(
    aliases: &mut HashMap<String, String>,
    context: &str,
    alias: &str,
    owner: &str,
) -> Result<(), String> {
    if let Some(existing_owner) = aliases.get(alias) {
        if existing_owner != owner {
            return Err(format!(
                "models.dev duplicate {context} alias '{alias}' for '{existing_owner}' and '{owner}'"
            ));
        }
        return Ok(());
    }
    aliases.insert(alias.to_string(), owner.to_string());
    Ok(())
}

fn validate_models_dev_body_size(size: usize) -> Result<(), String> {
    if size > MODELS_DEV_MAX_CATALOG_BYTES {
        return Err(format!(
            "models.dev catalog is too large: {size} bytes exceeds {} byte limit",
            MODELS_DEV_MAX_CATALOG_BYTES
        ));
    }
    Ok(())
}

fn unique_models_dev_cache_tmp_path(cache_path: &Path) -> PathBuf {
    let unique_id = MODELS_DEV_CACHE_WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
    cache_path.with_extension(format!("json.tmp.{}.{}", std::process::id(), unique_id))
}

fn unique_models_dev_cache_backup_path(cache_path: &Path) -> PathBuf {
    let unique_id = MODELS_DEV_CACHE_WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
    cache_path.with_extension(format!("json.backup.{}.{}", std::process::id(), unique_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_provider() -> ModelsDevProvider {
        ModelsDevProvider {
            id: "anthropic".to_string(),
            name: "Anthropic".to_string(),
            ..Default::default()
        }
    }

    fn test_model(cost: Option<ModelsDevCost>) -> ModelsDevModel {
        ModelsDevModel {
            id: "claude-test".to_string(),
            name: "claude-test".to_string(),
            temperature: Some(true),
            tool_call: Some(true),
            cost,
            ..Default::default()
        }
    }

    #[test]
    fn model_caps_enable_cache_control_when_cache_pricing_exists() {
        let model = test_model(Some(ModelsDevCost {
            input: Some(3.0),
            output: Some(15.0),
            cache_read: Some(0.3),
            cache_write: Some(3.75),
            ..Default::default()
        }));

        let caps = models_dev_model_to_model_caps("anthropic", &test_provider(), &model);

        assert!(caps.supports_cache_control);
    }

    #[test]
    fn model_caps_do_not_enable_cache_control_without_cache_pricing() {
        let model = test_model(Some(ModelsDevCost {
            input: Some(3.0),
            output: Some(15.0),
            ..Default::default()
        }));

        let caps = models_dev_model_to_model_caps("anthropic", &test_provider(), &model);

        assert!(!caps.supports_cache_control);
    }

    #[test]
    fn model_caps_enable_cache_control_when_long_context_cache_pricing_exists() {
        let model = test_model(Some(ModelsDevCost {
            input: Some(3.0),
            output: Some(15.0),
            context_over_200k: Some(ModelsDevCostTier {
                cache_read: Some(0.3),
                cache_write: None,
                ..Default::default()
            }),
            ..Default::default()
        }));

        let caps = models_dev_model_to_model_caps("anthropic", &test_provider(), &model);

        assert!(caps.supports_cache_control);
    }
}
