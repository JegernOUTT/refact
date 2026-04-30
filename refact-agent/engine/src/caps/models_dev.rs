use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use reqwest::header::USER_AGENT;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use tracing::warn;

use crate::caps::model_caps::ModelCapabilities;
use crate::global_context::GlobalContext;
use crate::providers::traits::{ModelPricing, ModelPricingTier};

pub const MODELS_DEV_API_URL: &str = "https://models.dev/api.json";
const MODELS_DEV_CACHE_DIR: &str = "models_dev";
const MODELS_DEV_CACHE_FILE: &str = "api.json";
const FETCH_TIMEOUT_SECS: u64 = 10;
const MODELS_DEV_MAX_CATALOG_BYTES: usize = 25 * 1024 * 1024;
const MODELS_DEV_SNAPSHOT: &str = include_str!("models_dev_snapshot.json");
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
    let supports_reasoning = model.reasoning == Some(true);
    let reasoning_effort_options = supports_openai_reasoning_effort(provider_key, provider, model)
        .then(|| vec!["low".to_string(), "medium".to_string(), "high".to_string()]);
    let supports_thinking_budget = supports_reasoning
        && reasoning_effort_options.is_none()
        && supports_anthropic_thinking_budget(provider_key, provider, model);
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
        reasoning_effort_options,
        supports_thinking_budget,
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

fn supports_openai_reasoning_effort(
    provider_key: &str,
    provider: &ModelsDevProvider,
    model: &ModelsDevModel,
) -> bool {
    if model.reasoning != Some(true) {
        return false;
    }

    let model_id = model.id.to_ascii_lowercase();
    let provider_id = provider.id.to_ascii_lowercase();
    let provider_key = provider_key.to_ascii_lowercase();

    provider_key == "openai"
        || provider_id == "openai"
        || provider_key == "openrouter"
        || provider_id == "openrouter"
        || model_id.starts_with("gpt-")
        || model_id.starts_with("o1")
        || model_id.starts_with("o3")
        || model_id.starts_with("o4")
        || model_id.starts_with("o5")
}

fn supports_anthropic_thinking_budget(
    provider_key: &str,
    provider: &ModelsDevProvider,
    model: &ModelsDevModel,
) -> bool {
    if model.reasoning != Some(true) {
        return false;
    }

    let model_id = model.id.to_ascii_lowercase();
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

    provider_key == "anthropic"
        || provider_id == "anthropic"
        || provider_npm.contains("anthropic")
        || model_npm.contains("anthropic")
        || model_api.contains("/anthropic/")
        || model_api.ends_with("/messages")
        || model_id.starts_with("claude-")
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

pub async fn load_models_dev_catalog(
    gcx: Arc<ARwLock<GlobalContext>>,
    force_refresh: bool,
) -> Result<ModelsDevCatalog, String> {
    let (cache_dir, http_client) = {
        let gcx_locked = gcx.read().await;
        (gcx_locked.cache_dir.clone(), gcx_locked.http_client.clone())
    };

    if force_refresh {
        match fetch_models_dev_catalog(&http_client).await {
            Ok((catalog, body)) => {
                if let Err(e) = write_models_dev_cache(&cache_dir, &body).await {
                    warn!("Failed to write models.dev runtime cache: {e}");
                }
                Ok(catalog)
            }
            Err(e) => {
                warn!("Failed to refresh models.dev catalog: {e}; using cache or snapshot");
                load_models_dev_catalog_from_cache_or_snapshot(&cache_dir).await
            }
        }
    } else {
        load_models_dev_catalog_from_cache_or_snapshot(&cache_dir).await
    }
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

pub async fn fetch_models_dev_catalog(
    http_client: &reqwest::Client,
) -> Result<(ModelsDevCatalog, String), String> {
    tokio::time::timeout(Duration::from_secs(FETCH_TIMEOUT_SECS), async {
        let response = http_client
            .get(MODELS_DEV_API_URL)
            .header(USER_AGENT, "refact-lsp models.dev catalog")
            .send()
            .await
            .map_err(|e| format!("Failed to request models.dev catalog: {e}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("models.dev catalog returned HTTP {status}"));
        }
        let body = read_models_dev_response_body(response).await?;
        let catalog = parse_required_project_catalog_json(&body, "models.dev live catalog")?;
        Ok((catalog, body))
    })
    .await
    .map_err(|_| "Timed out fetching models.dev catalog".to_string())?
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

async fn read_models_dev_response_body(mut response: reqwest::Response) -> Result<String, String> {
    validate_models_dev_content_length(response.content_length())?;
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("Failed to read models.dev catalog response: {e}"))?
    {
        let next_len = body
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| "models.dev catalog response is too large".to_string())?;
        validate_models_dev_body_size(next_len)?;
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|e| format!("models.dev catalog response is not UTF-8: {e}"))
}

fn validate_models_dev_content_length(content_length: Option<u64>) -> Result<(), String> {
    if let Some(content_length) = content_length {
        let content_length = usize::try_from(content_length).map_err(|_| {
            format!(
                "models.dev catalog is too large: {content_length} bytes exceeds {} byte limit",
                MODELS_DEV_MAX_CATALOG_BYTES
            )
        })?;
        validate_models_dev_body_size(content_length)?;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_catalog_json() -> &'static str {
        r#"
        {
            "openai": {
                "id": "openai",
                "name": "OpenAI",
                "env": ["OPENAI_API_KEY"],
                "api": "https://api.openai.com/v1",
                "npm": "@ai-sdk/openai",
                "models": {
                    "gpt-4o": {
                        "id": "gpt-4o",
                        "name": "GPT-4o",
                        "family": "gpt",
                        "reasoning": false,
                        "temperature": true,
                        "tool_call": true,
                        "cost": {
                            "input": 2.5,
                            "output": 10.0,
                            "cache_read": 1.25,
                            "cache_write": 3.75
                        },
                        "limit": {
                            "context": 128000,
                            "output": 16384
                        },
                        "modalities": {
                            "input": ["text", "image"],
                            "output": ["text"]
                        },
                        "provider": {
                            "api": "gpt-4o"
                        },
                        "status": "beta",
                        "experimental": {
                            "modes": {}
                        },
                        "unknown_future_field": true
                    }
                },
                "unknown_provider_field": "ignored"
            }
        }
        "#
    }

    fn provider_with_model(provider_id: &str) -> ModelsDevProvider {
        let model_id = format!("{provider_id}-model");
        ModelsDevProvider {
            id: provider_id.to_string(),
            name: provider_id.to_string(),
            models: HashMap::from([(
                model_id.clone(),
                ModelsDevModel {
                    id: model_id,
                    name: format!("{provider_id} model"),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        }
    }

    fn required_catalog() -> ModelsDevCatalog {
        let mut catalog = ModelsDevCatalog::new();
        for provider_id in REQUIRED_MODELS_DEV_PROVIDERS {
            catalog.insert((*provider_id).to_string(), provider_with_model(provider_id));
        }
        catalog.insert("zai".to_string(), provider_with_model("zai"));
        catalog
    }

    fn required_catalog_json() -> String {
        serde_json::to_string(&required_catalog()).unwrap()
    }
    #[test]
    fn minimal_catalog_parses_successfully() {
        let catalog = parse_catalog_json(minimal_catalog_json()).unwrap();
        let provider = get_provider(&catalog, "openai").unwrap();
        assert_eq!(provider.name, "OpenAI");
        assert_eq!(provider.env, vec!["OPENAI_API_KEY"]);
        let model = get_model(&catalog, "openai", "gpt-4o").unwrap();
        assert_eq!(model.name, "GPT-4o");
        assert_eq!(model.reasoning, Some(false));
        assert_eq!(model.temperature, Some(true));
        assert_eq!(model.tool_call, Some(true));
        assert_eq!(model.family.as_deref(), Some("gpt"));
        assert_eq!(model.status.as_deref(), Some("beta"));
        assert_eq!(
            model.limit.as_ref().and_then(|limit| limit.context),
            Some(128000)
        );
    }

    #[test]
    fn provider_and_model_lookup_uses_ids() {
        let catalog = parse_catalog_json(
            r#"
            {
                "provider-key": {
                    "id": "provider-id",
                    "name": "Provider",
                    "models": {
                        "model-key": {
                            "id": "model-id",
                            "name": "Model"
                        }
                    }
                }
            }
            "#,
        )
        .unwrap();

        assert!(get_provider(&catalog, "provider-key").is_some());
        assert!(get_provider(&catalog, "provider-id").is_some());
        assert!(get_model(&catalog, "provider-id", "model-key").is_some());
        assert!(get_model(&catalog, "provider-id", "model-id").is_some());
        assert!(get_model(&catalog, "provider-id", "missing").is_none());
    }

    #[test]
    fn provider_without_models_object_is_rejected() {
        let error = parse_catalog_json(
            r#"
            {
                "openai": {
                    "id": "openai",
                    "name": "OpenAI"
                }
            }
            "#,
        )
        .unwrap_err();

        assert!(error.contains("missing models object"));
    }

    #[test]
    fn missing_boolean_capabilities_parse_as_unknown() {
        let catalog = parse_catalog_json(
            r#"
            {
                "openai": {
                    "id": "openai",
                    "models": {
                        "gpt-4o": {
                            "id": "gpt-4o",
                            "name": "GPT-4o"
                        }
                    }
                }
            }
            "#,
        )
        .unwrap();
        let model = get_model(&catalog, "openai", "gpt-4o").unwrap();

        assert_eq!(model.reasoning, None);
        assert_eq!(model.temperature, None);
        assert_eq!(model.tool_call, None);
    }

    #[test]
    fn provider_with_empty_key_or_id_is_rejected() {
        let empty_key_error = parse_catalog_json(
            r#"
            {
                "": {
                    "id": "openai",
                    "models": {
                        "gpt-4o": { "id": "gpt-4o" }
                    }
                }
            }
            "#,
        )
        .unwrap_err();
        assert!(empty_key_error.contains("provider key"));

        let empty_id_error = parse_catalog_json(
            r#"
            {
                "openai": {
                    "id": "",
                    "models": {
                        "gpt-4o": { "id": "gpt-4o" }
                    }
                }
            }
            "#,
        )
        .unwrap_err();
        assert!(empty_id_error.contains("provider 'openai' id"));
    }

    #[test]
    fn model_with_empty_key_or_id_is_rejected() {
        let empty_key_error = parse_catalog_json(
            r#"
            {
                "openai": {
                    "id": "openai",
                    "models": {
                        "": { "id": "gpt-4o" }
                    }
                }
            }
            "#,
        )
        .unwrap_err();
        assert!(empty_key_error.contains("model key in provider 'openai'"));

        let empty_id_error = parse_catalog_json(
            r#"
            {
                "openai": {
                    "id": "openai",
                    "models": {
                        "gpt-4o": { "id": "" }
                    }
                }
            }
            "#,
        )
        .unwrap_err();
        assert!(empty_id_error.contains("model 'gpt-4o' id in provider 'openai'"));
    }

    #[test]
    fn duplicate_provider_id_is_rejected() {
        let error = parse_catalog_json(
            r#"
            {
                "openai": {
                    "id": "duplicate",
                    "models": {
                        "gpt-4o": { "id": "gpt-4o" }
                    }
                },
                "anthropic": {
                    "id": "duplicate",
                    "models": {
                        "claude": { "id": "claude" }
                    }
                }
            }
            "#,
        )
        .unwrap_err();

        assert!(error.contains("duplicate provider alias"));
        assert!(error.contains("duplicate"));
    }

    #[test]
    fn duplicate_model_id_within_provider_is_rejected() {
        let error = parse_catalog_json(
            r#"
            {
                "openai": {
                    "id": "openai",
                    "models": {
                        "gpt-4o": { "id": "duplicate" },
                        "gpt-4.1": { "id": "duplicate" }
                    }
                }
            }
            "#,
        )
        .unwrap_err();

        assert!(error.contains("duplicate model in provider 'openai' alias"));
        assert!(error.contains("duplicate"));
    }

    #[test]
    fn duplicate_raw_provider_json_key_is_rejected() {
        let error = parse_catalog_json(
            r#"
            {
                "openai": {
                    "id": "openai",
                    "models": {
                        "gpt-4o": { "id": "gpt-4o" }
                    }
                },
                "openai": {
                    "id": "openai-duplicate",
                    "models": {
                        "gpt-4.1": { "id": "gpt-4.1" }
                    }
                }
            }
            "#,
        )
        .unwrap_err();

        assert!(error.contains("duplicate JSON object key 'openai'"));
    }

    #[test]
    fn duplicate_raw_provider_object_key_is_rejected() {
        let error = parse_catalog_json(
            r#"
            {
                "openai": {
                    "id": "openai",
                    "id": "openai-duplicate",
                    "models": {
                        "gpt-4o": { "id": "gpt-4o" }
                    }
                }
            }
            "#,
        )
        .unwrap_err();

        assert!(error.contains("duplicate JSON object key 'id'"));
    }

    #[test]
    fn duplicate_raw_model_json_key_inside_provider_is_rejected() {
        let error = parse_catalog_json(
            r#"
            {
                "openai": {
                    "id": "openai",
                    "models": {
                        "gpt-4o": { "id": "gpt-4o" },
                        "gpt-4o": { "id": "gpt-4o-duplicate" }
                    }
                }
            }
            "#,
        )
        .unwrap_err();

        assert!(error.contains("duplicate JSON object key 'gpt-4o'"));
    }

    #[test]
    fn duplicate_raw_model_object_key_is_rejected() {
        let error = parse_catalog_json(
            r#"
            {
                "openai": {
                    "id": "openai",
                    "models": {
                        "gpt-4o": {
                            "id": "gpt-4o",
                            "name": "GPT-4o",
                            "name": "GPT-4o duplicate"
                        }
                    }
                }
            }
            "#,
        )
        .unwrap_err();

        assert!(error.contains("duplicate JSON object key 'name'"));
    }

    #[tokio::test]
    async fn corrupt_cache_falls_back_to_snapshot() {
        let tempdir = tempfile::tempdir().unwrap();
        let cache_path = models_dev_cache_path(tempdir.path());
        std::fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        std::fs::write(&cache_path, "not json").unwrap();

        let catalog = load_models_dev_catalog_from_cache_or_snapshot(tempdir.path())
            .await
            .unwrap();

        assert!(!catalog.is_empty());
    }

    #[tokio::test]
    async fn incomplete_cache_missing_required_providers_falls_back_to_snapshot() {
        let tempdir = tempfile::tempdir().unwrap();
        write_models_dev_cache_atomic(tempdir.path(), minimal_catalog_json())
            .await
            .unwrap();

        let catalog = load_models_dev_catalog_from_cache_or_snapshot(tempdir.path())
            .await
            .unwrap();

        assert!(get_provider(&catalog, "anthropic").is_some());
        assert!(validate_required_project_providers(&catalog).is_ok());
    }

    #[tokio::test]
    async fn incomplete_live_catalog_is_rejected_and_not_cached() {
        let tempdir = tempfile::tempdir().unwrap();
        let error =
            parse_required_project_catalog_json(minimal_catalog_json(), "models.dev live catalog")
                .unwrap_err();

        assert!(error.contains("models.dev live catalog is incomplete"));
        assert!(error.contains("required provider 'anthropic' is missing"));

        let cache_error = write_models_dev_cache(tempdir.path(), minimal_catalog_json())
            .await
            .unwrap_err();

        assert!(cache_error.contains("models.dev runtime cache is incomplete"));
        assert!(!models_dev_cache_path(tempdir.path()).exists());
    }

    #[tokio::test]
    async fn public_cache_write_rejects_invalid_catalog() {
        let tempdir = tempfile::tempdir().unwrap();
        let error = write_models_dev_cache(tempdir.path(), "not json")
            .await
            .unwrap_err();

        assert!(error.contains("models.dev runtime cache is invalid"));
        assert!(!models_dev_cache_path(tempdir.path()).exists());
    }

    #[test]
    fn required_provider_with_empty_models_is_rejected() {
        let mut catalog = required_catalog();
        catalog.get_mut("openai").unwrap().models.clear();

        let error = validate_required_project_providers(&catalog).unwrap_err();

        assert!(error.contains("required provider 'openai' has no models"));
    }

    #[test]
    fn required_zai_provider_group_with_empty_models_is_rejected() {
        let mut catalog = required_catalog();
        catalog.get_mut("zai").unwrap().models.clear();

        let error = validate_required_project_providers(&catalog).unwrap_err();

        assert!(error.contains("required provider group 'zai or zhipuai' has no models"));
    }
    #[test]
    fn cost_conversion_maps_to_model_pricing() {
        let catalog = parse_catalog_json(minimal_catalog_json()).unwrap();
        let model = get_model(&catalog, "openai", "gpt-4o").unwrap();
        let pricing = model_cost_to_pricing(model).unwrap();

        assert_eq!(pricing.prompt, 2.5);
        assert_eq!(pricing.generated, 10.0);
        assert_eq!(pricing.cache_read, Some(1.25));
        assert_eq!(pricing.cache_creation, Some(3.75));
    }

    #[test]
    fn tiered_pricing_keeps_base_tier_for_flat_model_pricing() {
        let cost = ModelsDevCost {
            input: Some(1.0),
            output: Some(2.0),
            cache_read: Some(0.5),
            cache_write: Some(0.75),
            context_over_200k: Some(ModelsDevCostTier {
                input: Some(10.0),
                output: Some(20.0),
                cache_read: Some(5.0),
                cache_write: Some(7.5),
            }),
        };

        let pricing = cost_to_pricing(&cost).unwrap();

        assert_eq!(pricing.prompt, 1.0);
        assert_eq!(pricing.generated, 2.0);
        assert_eq!(pricing.cache_read, Some(0.5));
        assert_eq!(pricing.cache_creation, Some(0.75));
    }

    #[test]
    fn incomplete_cost_does_not_convert_to_pricing() {
        let cost = ModelsDevCost {
            input: Some(1.0),
            output: None,
            ..Default::default()
        };

        assert!(cost_to_pricing(&cost).is_none());
    }

    #[test]
    fn generated_snapshot_parses() {
        let catalog = load_models_dev_snapshot_catalog().unwrap();

        assert!(!catalog.is_empty());
        assert!(get_provider(&catalog, "openai").is_some());
    }

    #[test]
    fn generated_snapshot_contains_required_project_providers() {
        let catalog = load_models_dev_snapshot_catalog().unwrap();

        validate_required_project_providers(&catalog).unwrap();
    }

    #[test]
    fn oversized_catalog_size_is_rejected() {
        assert!(validate_models_dev_body_size(MODELS_DEV_MAX_CATALOG_BYTES).is_ok());
        let error = validate_models_dev_body_size(MODELS_DEV_MAX_CATALOG_BYTES + 1).unwrap_err();

        assert!(error.contains("too large"));
    }

    #[test]
    fn oversized_content_length_is_rejected() {
        assert!(
            validate_models_dev_content_length(Some(MODELS_DEV_MAX_CATALOG_BYTES as u64)).is_ok()
        );
        let error =
            validate_models_dev_content_length(Some(MODELS_DEV_MAX_CATALOG_BYTES as u64 + 1))
                .unwrap_err();

        assert!(error.contains("too large"));
    }

    #[test]
    fn unique_cache_tmp_paths_are_distinct() {
        let cache_path = Path::new("/tmp/refact-models-dev/api.json");
        let first = unique_models_dev_cache_tmp_path(cache_path);
        let second = unique_models_dev_cache_tmp_path(cache_path);

        assert_ne!(first, second);
        assert_eq!(first.parent(), cache_path.parent());
        assert!(first
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap()
            .starts_with("api.json.tmp."));
    }

    #[tokio::test]
    async fn cache_replace_fallback_restores_existing_file_when_new_file_is_missing() {
        let tempdir = tempfile::tempdir().unwrap();
        let cache_path = models_dev_cache_path(tempdir.path());
        tokio::fs::create_dir_all(cache_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&cache_path, "existing").await.unwrap();
        let missing_tmp_path = cache_path.with_extension("json.tmp.missing");

        let error = replace_models_dev_cache_file(&missing_tmp_path, &cache_path)
            .await
            .unwrap_err();

        assert!(error.contains("Failed to replace models.dev cache file"));
        let contents = tokio::fs::read_to_string(&cache_path).await.unwrap();
        assert_eq!(contents, "existing");

        let mut dir = tokio::fs::read_dir(cache_path.parent().unwrap())
            .await
            .unwrap();
        while let Some(entry) = dir.next_entry().await.unwrap() {
            let file_name = entry.file_name().to_string_lossy().to_string();
            assert!(!file_name.contains(".backup."));
        }
    }

    #[tokio::test]
    async fn concurrent_validated_cache_writes_leave_valid_cache_and_no_artifacts() {
        let tempdir = tempfile::tempdir().unwrap();
        let write_count = 8usize;
        let barrier = Arc::new(tokio::sync::Barrier::new(write_count));
        let mut expected_contents = Vec::new();
        let mut handles = Vec::new();

        for idx in 0..write_count {
            let mut catalog = required_catalog();
            let provider_id = format!("extra-{idx}");
            catalog.insert(provider_id.clone(), provider_with_model(&provider_id));
            catalog.get_mut("openai").unwrap().name = format!("OpenAI {idx}");
            let contents = serde_json::to_string(&catalog).unwrap();
            expected_contents.push(contents.clone());

            let cache_dir = tempdir.path().to_path_buf();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                write_models_dev_cache(&cache_dir, &contents).await.unwrap();
            }));
        }

        for handle in handles {
            handle.await.unwrap();
        }

        let cache_path = models_dev_cache_path(tempdir.path());
        let contents = tokio::fs::read_to_string(&cache_path).await.unwrap();
        assert!(expected_contents.contains(&contents));
        let catalog =
            parse_required_project_catalog_json(&contents, "models.dev runtime cache").unwrap();
        validate_required_project_providers(&catalog).unwrap();

        let mut dir = tokio::fs::read_dir(cache_path.parent().unwrap())
            .await
            .unwrap();
        while let Some(entry) = dir.next_entry().await.unwrap() {
            let file_name = entry.file_name().to_string_lossy().to_string();
            assert!(!file_name.contains(".tmp."));
            assert!(!file_name.contains(".backup."));
        }
    }

    #[tokio::test]
    async fn write_cache_replaces_file_and_leaves_no_temp_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let first = required_catalog_json();
        let mut second_catalog = required_catalog();
        second_catalog.insert("zhipuai".to_string(), provider_with_model("zhipuai"));
        let second = serde_json::to_string(&second_catalog).unwrap();
        write_models_dev_cache(tempdir.path(), &first)
            .await
            .unwrap();
        write_models_dev_cache(tempdir.path(), &second)
            .await
            .unwrap();

        let cache_path = models_dev_cache_path(tempdir.path());
        let contents = tokio::fs::read_to_string(&cache_path).await.unwrap();
        assert_eq!(contents, second);

        let mut dir = tokio::fs::read_dir(cache_path.parent().unwrap())
            .await
            .unwrap();
        while let Some(entry) = dir.next_entry().await.unwrap() {
            let file_name = entry.file_name().to_string_lossy().to_string();
            assert!(!file_name.contains(".tmp."));
            assert!(!file_name.contains(".backup."));
        }
    }
}
