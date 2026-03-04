use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::caps::{
    BaseModelRecord, ChatModelRecord, CodeAssistantCaps, CompletionModelRecord, DefaultModels,
    EmbeddingModelRecord, HasBaseModelRecord, default_embedding_batch, default_rejection_threshold,
    strip_model_from_finetune, normalize_string,
};
use crate::custom_error::{MapErrToString, YamlError};

use crate::llm::adapter::WireFormat;

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CapsProvider {
    #[serde(alias = "cloud_name", default, deserialize_with = "normalize_string")]
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub supports_completion: bool,

    #[serde(default)]
    pub wire_format: WireFormat,

    #[serde(default = "default_endpoint_style")]
    pub endpoint_style: String,

    // These aliases are for backward compatibility with cloud and self-hosted caps
    #[serde(default, alias = "endpoint_template")]
    pub completion_endpoint: String,
    #[serde(default, alias = "endpoint_chat_passthrough")]
    pub chat_endpoint: String,
    #[serde(default, alias = "endpoint_embeddings_template")]
    pub embedding_endpoint: String,

    #[serde(default)]
    pub api_key: String,

    #[serde(default)]
    pub tokenizer_api_key: String,

    #[serde(default)]
    pub extra_headers: std::collections::HashMap<String, String>,

    #[serde(default)]
    pub code_completion_n_ctx: usize,

    #[serde(default)]
    pub support_metadata: bool,

    #[serde(default)]
    pub completion_models: IndexMap<String, CompletionModelRecord>,
    #[serde(default)]
    pub chat_models: IndexMap<String, ChatModelRecord>,
    #[serde(default, alias = "default_embeddings_model")]
    pub embedding_model: EmbeddingModelRecord,

    #[serde(default)]
    pub models_dict_patch: IndexMap<String, serde_json::Value>, // Used to patch some params from cloud, like n_ctx for pro/free users

    // Default model selections — inlined directly instead of using #[serde(flatten)]
    // on DefaultModels, because serde's flatten uses content buffering which breaks
    // #[serde(alias)] resolution and can silently drop sibling fields.
    #[serde(default, alias = "code_completion_default_model", alias = "completion_model")]
    pub completion_default_model: String,
    #[serde(default, alias = "code_chat_default_model", alias = "chat_model")]
    pub chat_default_model: String,
    #[serde(default)]
    pub chat_thinking_model: String,
    #[serde(default)]
    pub chat_light_model: String,

    #[serde(default)]
    pub running_models: Vec<String>,
}

impl CapsProvider {
    /// Construct a `DefaultModels` from the inline fields for use by consumers
    /// that need the grouped struct (e.g., `caps.defaults.apply_override()`).
    pub fn defaults(&self) -> DefaultModels {
        DefaultModels {
            completion_default_model: self.completion_default_model.clone(),
            chat_default_model: self.chat_default_model.clone(),
            chat_thinking_model: self.chat_thinking_model.clone(),
            chat_light_model: self.chat_light_model.clone(),
        }
    }

    pub fn apply_override(&mut self, value: serde_yaml::Value) -> Result<(), String> {
        set_field_if_exists::<bool>(&mut self.enabled, "enabled", &value)?;
        set_field_if_exists::<WireFormat>(&mut self.wire_format, "wire_format", &value)?;
        set_field_if_exists::<String>(&mut self.endpoint_style, "endpoint_style", &value)?;
        set_field_if_exists::<String>(
            &mut self.completion_endpoint,
            "completion_endpoint",
            &value,
        )?;
        set_field_if_exists::<String>(&mut self.chat_endpoint, "chat_endpoint", &value)?;
        set_field_if_exists::<String>(&mut self.embedding_endpoint, "embedding_endpoint", &value)?;
        set_field_if_exists::<String>(&mut self.api_key, "api_key", &value)?;
        set_field_if_exists::<String>(&mut self.tokenizer_api_key, "tokenizer_api_key", &value)?;
        set_field_if_exists::<EmbeddingModelRecord>(
            &mut self.embedding_model,
            "embedding_model",
            &value,
        )?;
        if value.get("embedding_model").is_some() {
            self.embedding_model.base.removable = true;
            self.embedding_model.base.user_configured = true;
        }

        // New provider system writes `enabled_models` when user toggles models via UI.
        // If present, replace template running_models with user's explicit selection.
        if value.get("enabled_models").is_some() {
            self.running_models.clear();
            extend_collection::<Vec<String>>(&mut self.running_models, "enabled_models", &value)?;
        }
        extend_collection::<Vec<String>>(&mut self.running_models, "running_models", &value)?;
        extend_model_collection::<ChatModelRecord>(
            &mut self.chat_models,
            "chat_models",
            &value,
            &self.running_models,
        )?;
        extend_model_collection::<CompletionModelRecord>(
            &mut self.completion_models,
            "completion_models",
            &value,
            &self.running_models,
        )?;

        // Deserialize as DefaultModels (standalone, so aliases work correctly)
        // and merge non-empty fields into our inline fields.
        match serde_yaml::from_value::<DefaultModels>(value) {
            Ok(dm) => {
                if !dm.completion_default_model.is_empty() {
                    self.completion_default_model = dm.completion_default_model;
                }
                if !dm.chat_default_model.is_empty() {
                    self.chat_default_model = dm.chat_default_model;
                }
                if !dm.chat_thinking_model.is_empty() {
                    self.chat_thinking_model = dm.chat_thinking_model;
                }
                if !dm.chat_light_model.is_empty() {
                    self.chat_light_model = dm.chat_light_model;
                }
            }
            Err(e) => return Err(e.to_string()),
        }

        Ok(())
    }
}

fn set_field_if_exists<T: for<'de> serde::Deserialize<'de>>(
    target: &mut T,
    field: &str,
    value: &serde_yaml::Value,
) -> Result<(), String> {
    if let Some(val) = value.get(field) {
        *target = serde_yaml::from_value(val.clone())
            .map_err(|_| format!("Field '{}' has incorrect type", field))?;
    }
    Ok(())
}

fn extend_collection<C: for<'de> serde::Deserialize<'de> + Extend<C::Item> + IntoIterator>(
    target: &mut C,
    field: &str,
    value: &serde_yaml::Value,
) -> Result<(), String> {
    if let Some(value) = value.get(field) {
        let imported_collection = serde_yaml::from_value::<C>(value.clone())
            .map_err(|_| format!("Invalid format for {field}"))?;

        target.extend(imported_collection);
    }
    Ok(())
}

// Special implementation for ChatModelRecord and CompletionModelRecord collections
// that sets removable=true for newly added models
fn extend_model_collection<T: for<'de> serde::Deserialize<'de> + HasBaseModelRecord>(
    target: &mut IndexMap<String, T>,
    field: &str,
    value: &serde_yaml::Value,
    prev_running_models: &Vec<String>,
) -> Result<(), String> {
    if let Some(value) = value.get(field) {
        let imported_collection = serde_yaml::from_value::<IndexMap<String, T>>(value.clone())
            .map_err(|_| format!("Invalid format for {field}"))?;

        for (key, mut model) in imported_collection {
            model.base_mut().user_configured = true;
            if !target.contains_key(&key) && !prev_running_models.contains(&key) {
                model.base_mut().removable = true;
            }
            target.insert(key, model);
        }
    }
    Ok(())
}

fn default_endpoint_style() -> String {
    "openai".to_string()
}

fn default_true() -> bool {
    true
}

impl<'de> serde::Deserialize<'de> for EmbeddingModelRecord {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Input {
            String(String),
            Full(EmbeddingModelRecordHelper),
        }

        #[derive(Deserialize)]
        struct EmbeddingModelRecordHelper {
            #[serde(flatten)]
            base: BaseModelRecord,
            #[serde(default)]
            embedding_size: i32,
            #[serde(default = "default_rejection_threshold")]
            rejection_threshold: f32,
            #[serde(default = "default_embedding_batch")]
            embedding_batch: usize,
        }

        match Input::deserialize(deserializer)? {
            Input::String(name) => Ok(EmbeddingModelRecord {
                base: BaseModelRecord {
                    name,
                    ..Default::default()
                },
                ..Default::default()
            }),
            Input::Full(mut helper) => {
                if helper.embedding_batch > 256 {
                    tracing::warn!("embedding_batch can't be higher than 256");
                    helper.embedding_batch = default_embedding_batch();
                }

                Ok(EmbeddingModelRecord {
                    base: helper.base,
                    embedding_batch: helper.embedding_batch,
                    rejection_threshold: helper.rejection_threshold,
                    embedding_size: helper.embedding_size,
                })
            }
        }
    }
}



const PROVIDER_TEMPLATES: &[(&str, &str)] = &[
    (
        "anthropic",
        include_str!("../yaml_configs/default_providers/anthropic.yaml"),
    ),
    (
        "custom",
        include_str!("../yaml_configs/default_providers/custom.yaml"),
    ),
    (
        "deepseek",
        include_str!("../yaml_configs/default_providers/deepseek.yaml"),
    ),
    (
        "google_gemini",
        include_str!("../yaml_configs/default_providers/google_gemini.yaml"),
    ),
    (
        "groq",
        include_str!("../yaml_configs/default_providers/groq.yaml"),
    ),
    (
        "lmstudio",
        include_str!("../yaml_configs/default_providers/lmstudio.yaml"),
    ),
    (
        "ollama",
        include_str!("../yaml_configs/default_providers/ollama.yaml"),
    ),
    (
        "openai",
        include_str!("../yaml_configs/default_providers/openai.yaml"),
    ),
    (
        "openai_responses",
        include_str!("../yaml_configs/default_providers/openai_responses.yaml"),
    ),
    (
        "openrouter",
        include_str!("../yaml_configs/default_providers/openrouter.yaml"),
    ),
    (
        "refact",
        include_str!("../yaml_configs/default_providers/refact.yaml"),
    ),
    (
        "vllm",
        include_str!("../yaml_configs/default_providers/vllm.yaml"),
    ),
    (
        "xai",
        include_str!("../yaml_configs/default_providers/xai.yaml"),
    ),
    (
        "xai_responses",
        include_str!("../yaml_configs/default_providers/xai_responses.yaml"),
    ),
];
static PARSED_PROVIDERS: OnceLock<IndexMap<String, CapsProvider>> = OnceLock::new();

pub fn get_provider_templates() -> &'static IndexMap<String, CapsProvider> {
    PARSED_PROVIDERS.get_or_init(|| {
        let mut map = IndexMap::new();
        for (name, yaml) in PROVIDER_TEMPLATES {
            if let Ok(mut provider) = serde_yaml::from_str::<CapsProvider>(yaml) {
                provider.name = name.to_string();
                map.insert(name.to_string(), provider);
            } else {
                panic!("Failed to parse template for provider {}", name);
            }
        }
        map
    })
}

/// Returns yaml files from providers.d directory, and list of errors from reading
/// directory or listing files
pub async fn get_provider_yaml_paths(config_dir: &Path) -> (Vec<PathBuf>, Vec<String>) {
    let providers_dir = config_dir.join("providers.d");
    let mut yaml_paths = Vec::new();
    let mut errors = Vec::new();

    let mut entries = match tokio::fs::read_dir(&providers_dir).await {
        Ok(entries) => entries,
        Err(e) => {
            errors.push(format!("Failed to read providers directory: {e}"));
            return (yaml_paths, errors);
        }
    };

    while let Some(entry_result) = entries.next_entry().await.transpose() {
        match entry_result {
            Ok(entry) => {
                let path = entry.path();

                if path.is_file()
                    && path
                        .extension()
                        .map_or(false, |ext| ext == "yaml" || ext == "yml")
                {
                    yaml_paths.push(path);
                }
            }
            Err(e) => {
                errors.push(format!("Error reading directory entry: {e}"));
            }
        }
    }

    yaml_paths.sort();

    (yaml_paths, errors)
}

pub fn post_process_provider(
    provider: &mut CapsProvider,
    include_disabled_models: bool,
    experimental: bool,
) {
    add_running_models(provider);
    populate_model_records(provider, experimental);
    apply_models_dict_patch(provider);
    add_name_and_id_to_model_records(provider);
    if !include_disabled_models {
        provider.chat_models.retain(|_, model| model.base.enabled);
        provider
            .completion_models
            .retain(|_, model| model.base.enabled);
    }
}

pub async fn read_providers_d(
    prev_providers: Vec<CapsProvider>,
    config_dir: &Path,
    experimental: bool,
) -> (Vec<CapsProvider>, Vec<YamlError>) {
    let providers_dir = config_dir.join("providers.d");
    let mut providers = prev_providers;
    let mut error_log = Vec::new();

    let (yaml_paths, read_errors) = get_provider_yaml_paths(config_dir).await;
    for error in read_errors {
        error_log.push(YamlError {
            path: providers_dir.to_string_lossy().to_string(),
            error_line: 0,
            error_msg: error.to_string(),
        });
    }

    let provider_templates = get_provider_templates();
    let mut seen_provider_names = std::collections::HashSet::new();

    for yaml_path in yaml_paths {
        let provider_name = match yaml_path.file_stem() {
            Some(name) => name.to_string_lossy().to_string(),
            None => continue,
        };

        if !seen_provider_names.insert(provider_name.clone()) {
            error_log.push(YamlError {
                path: yaml_path.to_string_lossy().to_string(),
                error_line: 0,
                error_msg: format!(
                    "Duplicate provider name '{}' (another file with the same stem was already processed)",
                    provider_name
                ),
            });
            continue;
        }

        if provider_templates.contains_key(&provider_name) {
            match get_provider_from_template_and_config_file(
                config_dir,
                &provider_name,
                false,
                false,
                experimental,
            )
            .await
            {
                Ok(provider) => {
                    providers.push(provider);
                }
                Err(e) => {
                    error_log.push(YamlError {
                        path: yaml_path.to_string_lossy().to_string(),
                        error_line: 0,
                        error_msg: e,
                    });
                }
            }
        } else {
            let content = match tokio::fs::read_to_string(&yaml_path).await {
                Ok(content) => content,
                Err(e) => {
                    error_log.push(YamlError {
                        path: yaml_path.to_string_lossy().to_string(),
                        error_line: 0,
                        error_msg: format!("Failed to read file: {}", e),
                    });
                    continue;
                }
            };

            let mut provider: CapsProvider = match serde_yaml::from_str(&content) {
                Ok(provider) => provider,
                Err(e) => {
                    error_log.push(YamlError {
                        path: yaml_path.to_string_lossy().to_string(),
                        error_line: e.location().map_or(0, |loc| loc.line()),
                        error_msg: format!("Failed to parse YAML: {}", e),
                    });
                    continue;
                }
            };
            provider.name = provider_name;
            providers.push(provider);
        }
    }

    (providers, error_log)
}

fn add_running_models(provider: &mut CapsProvider) {
    let models_to_add = vec![
        &provider.chat_default_model,
        &provider.chat_light_model,
        &provider.chat_thinking_model,
        &provider.completion_default_model,
    ];

    for model in models_to_add {
        if !model.is_empty() && !provider.running_models.contains(model) {
            provider.running_models.push(model.clone());
        }
    }
}

/// Returns the latest modification timestamp in seconds of any YAML file in the providers.d directory
pub async fn get_latest_provider_mtime(config_dir: &Path) -> Option<u64> {
    let (yaml_paths, reading_errors) = get_provider_yaml_paths(config_dir).await;

    for error in reading_errors {
        tracing::error!("{error}");
    }

    let mut latest_mtime = None;
    for path in yaml_paths {
        match tokio::fs::metadata(&path).await {
            Ok(metadata) => {
                if let Ok(mtime) = metadata.modified() {
                    latest_mtime = match latest_mtime {
                        Some(current_latest) if mtime > current_latest => Some(mtime),
                        None => Some(mtime),
                        _ => latest_mtime,
                    };
                }
            }
            Err(e) => {
                tracing::error!("Failed to get metadata for {}: {}", path.display(), e);
            }
        }
    }

    latest_mtime.map(|mtime| {
        mtime
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    })
}

pub fn add_models_to_caps(caps: &mut CodeAssistantCaps, providers: Vec<CapsProvider>) {
    fn add_provider_details_to_model(
        base_model_rec: &mut BaseModelRecord,
        provider: &CapsProvider,
        model_name: &str,
        endpoint: &str,
    ) {
        base_model_rec.api_key = provider.api_key.clone();
        base_model_rec.tokenizer_api_key = provider.tokenizer_api_key.clone();
        base_model_rec.endpoint = endpoint.replace("$MODEL", model_name);
        base_model_rec.support_metadata = provider.support_metadata;
        base_model_rec.endpoint_style = provider.endpoint_style.clone();
        base_model_rec.wire_format = provider.wire_format;
        base_model_rec.extra_headers = provider.extra_headers.clone();
    }

    for mut provider in providers {
        let completion_models = std::mem::take(&mut provider.completion_models);
        for (model_name, mut model_rec) in completion_models {
            if model_rec.base.endpoint.is_empty() {
                add_provider_details_to_model(
                    &mut model_rec.base,
                    &provider,
                    &model_name,
                    &provider.completion_endpoint,
                );

                if provider.code_completion_n_ctx > 0 {
                    if model_rec.base.n_ctx == 0 || provider.code_completion_n_ctx < model_rec.base.n_ctx {
                        model_rec.base.n_ctx = provider.code_completion_n_ctx;
                    }
                }
            }

            caps.completion_models
                .insert(model_rec.base.id.clone(), Arc::new(model_rec));
        }

        let chat_models = std::mem::take(&mut provider.chat_models);
        for (model_name, mut model_rec) in chat_models {
            if model_rec.base.endpoint.is_empty() {
                add_provider_details_to_model(
                    &mut model_rec.base,
                    &provider,
                    &model_name,
                    &provider.chat_endpoint,
                );
            }

            caps.chat_models
                .insert(model_rec.base.id.clone(), Arc::new(model_rec));
        }

        if provider.embedding_model.is_configured() && provider.embedding_model.base.enabled {
            let mut embedding_model = std::mem::take(&mut provider.embedding_model);

            if embedding_model.base.endpoint.is_empty() {
                let model_name = embedding_model.base.name.clone();
                add_provider_details_to_model(
                    &mut embedding_model.base,
                    &provider,
                    &model_name,
                    &provider.embedding_endpoint,
                );
            }
            caps.embedding_model = embedding_model;
        }

        caps.defaults
            .apply_override(&provider.defaults(), Some(&provider.name));
    }
}

fn add_name_and_id_to_model_records(provider: &mut CapsProvider) {
    for (model_name, model_rec) in &mut provider.completion_models {
        model_rec.base.name = model_name.to_string();
        model_rec.base.id = format!("{}/{}", provider.name, model_name);
    }

    for (model_name, model_rec) in &mut provider.chat_models {
        model_rec.base.name = model_name.to_string();
        model_rec.base.id = format!("{}/{}", provider.name, model_name);
    }

    if provider.embedding_model.is_configured() {
        provider.embedding_model.base.id =
            format!("{}/{}", provider.name, provider.embedding_model.base.name);
    }
}

fn apply_models_dict_patch(provider: &mut CapsProvider) {
    for (model_name, rec_patched) in provider.models_dict_patch.iter() {
        if let Some(completion_rec) = provider.completion_models.get_mut(model_name) {
            if let Some(n_ctx) = rec_patched.get("n_ctx").and_then(|v| v.as_u64()) {
                completion_rec.base.n_ctx = n_ctx as usize;
            }
        }

        if let Some(chat_rec) = provider.chat_models.get_mut(model_name) {
            if let Some(n_ctx) = rec_patched.get("n_ctx").and_then(|v| v.as_u64()) {
                chat_rec.base.n_ctx = n_ctx as usize;
            }

            if let Some(supports_tools) =
                rec_patched.get("supports_tools").and_then(|v| v.as_bool())
            {
                chat_rec.supports_tools = supports_tools;
            }
            if let Some(supports_multimodality) = rec_patched
                .get("supports_multimodality")
                .and_then(|v| v.as_bool())
            {
                chat_rec.supports_multimodality = supports_multimodality;
            }
        }
    }
}

#[derive(Deserialize)]
pub struct CompletionPresets {
    pub completion_models: IndexMap<String, CompletionModelRecord>,
}

#[derive(Deserialize)]
pub struct EmbeddingPresets {
    pub embedding_models: IndexMap<String, EmbeddingModelRecord>,
}

const UNPARSED_COMPLETION_PRESETS: &str = include_str!("../completion_presets.json");
const UNPARSED_EMBEDDING_PRESETS: &str = include_str!("../embedding_presets.json");

static COMPLETION_PRESETS: OnceLock<CompletionPresets> = OnceLock::new();
static EMBEDDING_PRESETS: OnceLock<EmbeddingPresets> = OnceLock::new();

pub fn get_completion_presets() -> &'static CompletionPresets {
    COMPLETION_PRESETS.get_or_init(|| {
        serde_json::from_str::<CompletionPresets>(UNPARSED_COMPLETION_PRESETS)
            .unwrap_or_else(|e| {
                let up_to_line = UNPARSED_COMPLETION_PRESETS
                    .lines()
                    .take(e.line())
                    .collect::<Vec<&str>>()
                    .join("\n");
                panic!("{}\nfailed to parse COMPLETION_PRESETS: {}", up_to_line, e);
            })
    })
}

pub fn get_embedding_presets() -> &'static EmbeddingPresets {
    EMBEDDING_PRESETS.get_or_init(|| {
        serde_json::from_str::<EmbeddingPresets>(UNPARSED_EMBEDDING_PRESETS)
            .unwrap_or_else(|e| {
                let up_to_line = UNPARSED_EMBEDDING_PRESETS
                    .lines()
                    .take(e.line())
                    .collect::<Vec<&str>>()
                    .join("\n");
                panic!("{}\nfailed to parse EMBEDDING_PRESETS: {}", up_to_line, e);
            })
    })
}

/// Augment an existing completion model with scratchpad data from a matching preset.
/// Models created by `convert_self_hosted_caps_if_needed` have correct endpoint/tokenizer
/// but lack FIM token configuration (scratchpad_patch). This fills in the missing data
/// from completion_presets.json without overwriting cloud-provided fields.
fn augment_completion_model_from_preset(
    model: &mut CompletionModelRecord,
    model_name: &str,
    known_presets: &IndexMap<String, CompletionModelRecord>,
    experimental: bool,
) {
    // Skip if model already has FIM-specific scratchpad configuration
    if model.scratchpad_patch.get("fim_prefix").is_some() {
        return;
    }

    let name_owned = model_name.to_string();
    if let Some(preset) = find_model_match(
        &name_owned,
        &IndexMap::new(),
        known_presets,
        experimental,
    ) {
        model.scratchpad_patch = preset.scratchpad_patch.clone();
        model.scratchpad = preset.scratchpad.clone();
        if model.model_family.is_none() {
            model.model_family = preset.model_family;
        }
        if model.base.tokenizer.is_empty() {
            model.base.tokenizer = preset.base.tokenizer.clone();
        }
        if model.base.n_ctx == 0 && preset.base.n_ctx > 0 {
            model.base.n_ctx = preset.base.n_ctx;
        }
    }
}

fn populate_model_records(provider: &mut CapsProvider, experimental: bool) {
    let completion_presets = get_completion_presets();
    let embedding_presets = get_embedding_presets();

    for model_name in &provider.running_models {
        if provider.supports_completion {
            if !provider.completion_models.contains_key(model_name) {
                if let Some(model_rec) = find_model_match(
                    model_name,
                    &provider.completion_models,
                    &completion_presets.completion_models,
                    experimental,
                ) {
                    provider
                        .completion_models
                        .insert(model_name.clone(), model_rec);
                }
            } else {
                // Model already exists (e.g. from convert_self_hosted_caps_if_needed)
                // but may lack scratchpad data (FIM tokens). Augment from preset.
                augment_completion_model_from_preset(
                    provider.completion_models.get_mut(model_name).unwrap(),
                    model_name,
                    &completion_presets.completion_models,
                    experimental,
                );
            }
        }

        if !provider.chat_models.contains_key(model_name) {
            let placeholder = ChatModelRecord {
                base: BaseModelRecord {
                    enabled: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            provider.chat_models.insert(model_name.clone(), placeholder);
        }
    }

    // Augment all completion models that lack FIM tokens with preset scratchpad data.
    // This covers models from convert_self_hosted_caps_if_needed that aren't in running_models.
    if provider.supports_completion {
        let model_names: Vec<String> = provider.completion_models.keys().cloned().collect();
        for model_name in &model_names {
            augment_completion_model_from_preset(
                provider.completion_models.get_mut(model_name).unwrap(),
                model_name,
                &completion_presets.completion_models,
                experimental,
            );
        }
    }

    if !provider.embedding_model.is_configured() && !provider.embedding_model.base.name.is_empty() {
        let model_name = provider.embedding_model.base.name.clone();
        if let Some(model_rec) = find_model_match(
            &model_name,
            &IndexMap::new(),
            &embedding_presets.embedding_models,
            experimental,
        ) {
            provider.embedding_model = model_rec;
            provider.embedding_model.base.name = model_name;
        } else {
            tracing::warn!(
                "Unknown embedding model '{}', maybe configure it or update this binary",
                model_name
            );
        }
    }

    if provider.embedding_model.is_configured() {
        let model_name = provider.embedding_model.base.name.clone();
        if let Some(preset) = find_model_match(
            &model_name,
            &IndexMap::new(),
            &embedding_presets.embedding_models,
            experimental,
        ) {
            if provider.embedding_model.base.tokenizer.is_empty() {
                provider.embedding_model.base.tokenizer = preset.base.tokenizer.clone();
            }
            if !provider.embedding_model.base.user_configured {
                if provider.embedding_model.base.n_ctx == 0 {
                    provider.embedding_model.base.n_ctx = preset.base.n_ctx;
                }
                if provider.embedding_model.embedding_size == 0 {
                    provider.embedding_model.embedding_size = preset.embedding_size;
                }
                if provider.embedding_model.rejection_threshold == 0.0 {
                    provider.embedding_model.rejection_threshold = preset.rejection_threshold;
                }
                if provider.embedding_model.embedding_batch == 0 {
                    provider.embedding_model.embedding_batch = preset.embedding_batch;
                }
            }
        }
        if provider.embedding_model.base.tokenizer.is_empty() {
            tracing::warn!(
                "Embedding model '{}' has no tokenizer configured and no preset match; VecDB may fail to start",
                provider.embedding_model.base.name
            );
        }
    }
}

fn find_model_match<T: Clone + HasBaseModelRecord>(
    model_name: &String,
    provider_models: &IndexMap<String, T>,
    known_models: &IndexMap<String, T>,
    experimental: bool,
) -> Option<T> {
    let model_stripped = strip_model_from_finetune(model_name);

    if let Some(model) = provider_models
        .get(model_name)
        .or_else(|| provider_models.get(&model_stripped))
    {
        if !model.base().experimental || experimental {
            return Some(model.clone());
        }
    }

    for model in provider_models.values() {
        if model.base().similar_models.contains(model_name)
            || model.base().similar_models.contains(&model_stripped)
        {
            if !model.base().experimental || experimental {
                return Some(model.clone());
            }
        }
    }

    if let Some(model) = known_models
        .get(model_name)
        .or_else(|| known_models.get(&model_stripped))
    {
        if !model.base().experimental || experimental {
            return Some(model.clone());
        }
    }

    for model in known_models.values() {
        if model
            .base()
            .similar_models
            .contains(&model_name.to_string())
            || model.base().similar_models.contains(&model_stripped)
        {
            if !model.base().experimental || experimental {
                return Some(model.clone());
            }
        }
    }

    None
}

pub fn resolve_api_key(
    provider: &CapsProvider,
    key: &str,
    fallback: &str,
    key_name: &str,
) -> String {
    match key {
        k if k.is_empty() => fallback.to_string(),
        k if k.starts_with("$") => match std::env::var(&k[1..]) {
            Ok(env_val) => env_val,
            Err(e) => {
                tracing::error!(
                    "tried to read {} from env var {} for provider {}, but failed: {}",
                    key_name,
                    k,
                    provider.name,
                    e
                );
                fallback.to_string()
            }
        },
        k => k.to_string(),
    }
}

pub fn resolve_provider_api_key(provider: &CapsProvider, cmdline_api_key: &str) -> String {
    resolve_api_key(provider, &provider.api_key, &cmdline_api_key, "API key")
}

pub async fn get_provider_from_template_and_config_file(
    config_dir: &Path,
    name: &str,
    config_file_must_exist: bool,
    post_process: bool,
    experimental: bool,
) -> Result<CapsProvider, String> {
    let mut provider = get_provider_templates()
        .get(name)
        .cloned()
        .ok_or("Provider template not found")?;

    let provider_path = config_dir.join("providers.d").join(format!("{name}.yaml"));
    let config_file_value = match tokio::fs::read_to_string(&provider_path).await {
        Ok(content) => serde_yaml::from_str::<serde_yaml::Value>(&content)
            .map_err_with_prefix(format!("Error parsing file {}:", provider_path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !config_file_must_exist => {
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
        }
        Err(e) => {
            return Err(format!(
                "Failed to read file {}: {}",
                provider_path.display(),
                e
            ));
        }
    };

    provider.apply_override(config_file_value)?;

    if post_process {
        post_process_provider(&mut provider, true, experimental);
    }

    Ok(provider)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_provider_templates() {
        let _ = get_provider_templates(); // This will panic if any template fails to parse
    }

    #[test]
    fn test_parse_completion_presets() {
        let _ = get_completion_presets(); // This will panic if any preset fails to parse
    }

    #[test]
    fn test_parse_embedding_presets() {
        let _ = get_embedding_presets(); // This will panic if any preset fails to parse
    }

    #[test]
    fn test_embedding_tokenizer_prefill_from_preset() {
        let mut provider = CapsProvider {
            name: "test".to_string(),
            embedding_model: EmbeddingModelRecord {
                base: BaseModelRecord {
                    name: "text-embedding-3-small".to_string(),
                    n_ctx: 8191,
                    tokenizer: String::new(),
                    enabled: true,
                    ..Default::default()
                },
                embedding_size: 1536,
                ..Default::default()
            },
            ..Default::default()
        };
        populate_model_records(&mut provider, false);
        assert!(
            !provider.embedding_model.base.tokenizer.is_empty(),
            "tokenizer should have been filled from embedding presets"
        );
        assert_eq!(
            provider.embedding_model.base.tokenizer,
            "hf://Xenova/text-embedding-ada-002"
        );
    }

    #[test]
    fn test_cloud_caps_json_deserialization() {
        // Simulate the cloud caps JSON structure from inference.smallcloud.ai
        let cloud_json = serde_json::json!({
            "cloud_name": "Refact",
            "endpoint_template": "/v1/completions",
            "endpoint_style": "openai",
            "endpoint_chat_passthrough": "/v1/chat/completions",
            "code_completion_default_model": "Refact/1.6B",
            "code_completion_n_ctx": 2048,
            "code_chat_default_model": "gpt-3.5-turbo",
            "chat_light_model": "grok-4-1-fast-non-reasoning",
            "chat_thinking_model": "gpt-5.2",
            "default_embeddings_model": "thenlper/gte-base",
            "endpoint_embeddings_template": "/v1/embeddings",
            "running_models": [
                "smallcloudai/Refact-1_6B-fim",
                "Refact/1.6B",
                "qwen2.5/coder/1.5b/base",
                "thenlper/gte-base"
            ],
            "support_metadata": true
        });

        let provider: CapsProvider = serde_json::from_value(cloud_json).unwrap();

        // Verify running_models parsed correctly (NOT eaten by serde flatten)
        assert_eq!(provider.running_models.len(), 4, "running_models should have 4 items");
        assert_eq!(provider.running_models[0], "smallcloudai/Refact-1_6B-fim");

        // Verify default model fields parsed via aliases
        assert_eq!(provider.completion_default_model, "Refact/1.6B",
            "code_completion_default_model alias should work");
        assert_eq!(provider.chat_default_model, "gpt-3.5-turbo",
            "code_chat_default_model alias should work");
        assert_eq!(provider.chat_light_model, "grok-4-1-fast-non-reasoning");
        assert_eq!(provider.chat_thinking_model, "gpt-5.2");

        // Verify embedding model parsed via alias
        assert_eq!(provider.embedding_model.base.name, "thenlper/gte-base",
            "default_embeddings_model alias should work");

        // Verify endpoint aliases
        assert_eq!(provider.completion_endpoint, "/v1/completions",
            "endpoint_template alias should work");
        assert_eq!(provider.chat_endpoint, "/v1/chat/completions",
            "endpoint_chat_passthrough alias should work");
        assert_eq!(provider.embedding_endpoint, "/v1/embeddings",
            "endpoint_embeddings_template alias should work");

        // Verify other fields
        assert_eq!(provider.code_completion_n_ctx, 2048);
        assert!(provider.support_metadata);
    }

    #[test]
    fn test_qwen_completion_model_scratchpad_patch_preserved() {
        // Simulate the Refact cloud caps flow: cloud JSON → CapsProvider → post_process → add_models_to_caps
        // This tests that the qwen2.5-coder-base preset's scratchpad_patch with <|fim_prefix|> tokens
        // is correctly applied and preserved through the entire flow.
        let cloud_json = serde_json::json!({
            "cloud_name": "Refact",
            "endpoint_template": "/v1/completions",
            "endpoint_style": "openai",
            "endpoint_chat_passthrough": "/v1/chat/completions",
            "code_completion_default_model": "Refact/1.6B",
            "code_completion_n_ctx": 2048,
            "code_chat_default_model": "gpt-3.5-turbo",
            "default_embeddings_model": "thenlper/gte-base",
            "endpoint_embeddings_template": "/v1/embeddings",
            "models_dict_patch": {
                "qwen2.5/coder/1.5b/base": { "n_ctx": 4096 }
            },
            "running_models": [
                "smallcloudai/Refact-1_6B-fim",
                "Refact/1.6B",
                "qwen2.5/coder/1.5b/base",
                "thenlper/gte-base"
            ],
            "support_metadata": true
        });

        let mut provider: CapsProvider = serde_json::from_value(cloud_json).unwrap();
        assert!(provider.completion_models.is_empty(), "cloud JSON should not have completion_models");

        // Step 1: post_process_provider populates models from presets
        post_process_provider(&mut provider, false, false);

        // Step 2: Verify qwen2.5 completion model has correct scratchpad_patch from preset
        let qwen_model = provider.completion_models.get("qwen2.5/coder/1.5b/base")
            .expect("qwen2.5/coder/1.5b/base should be in completion_models after populate");

        assert_eq!(qwen_model.scratchpad, "FIM-PSM", "scratchpad should be FIM-PSM");
        assert_eq!(
            qwen_model.scratchpad_patch.get("fim_prefix").and_then(|v| v.as_str()),
            Some("<|fim_prefix|>"),
            "scratchpad_patch should have <|fim_prefix|> from qwen2.5-coder-base preset, got: {:?}",
            qwen_model.scratchpad_patch
        );
        assert_eq!(
            qwen_model.scratchpad_patch.get("fim_suffix").and_then(|v| v.as_str()),
            Some("<|fim_suffix|>"),
            "scratchpad_patch should have <|fim_suffix|>"
        );
        assert_eq!(
            qwen_model.scratchpad_patch.get("fim_middle").and_then(|v| v.as_str()),
            Some("<|fim_middle|>"),
            "scratchpad_patch should have <|fim_middle|>"
        );
        assert_eq!(
            qwen_model.base.tokenizer,
            "hf://Qwen/Qwen2.5-Coder-0.5B",
            "tokenizer should come from preset"
        );
        // n_ctx should be patched by models_dict_patch (code_completion_n_ctx applied later in add_models_to_caps)
        assert_eq!(qwen_model.base.n_ctx, 4096,
            "n_ctx should be min(preset 8192, models_dict_patch 4096) = 4096 at this stage");

        // Step 3: Verify Refact/1.6B model
        let refact_model = provider.completion_models.get("Refact/1.6B")
            .expect("Refact/1.6B should be in completion_models");
        assert_eq!(refact_model.scratchpad, "FIM-SPM", "Refact model should use FIM-SPM");

        // Step 4: add_models_to_caps and verify final state
        let mut caps = CodeAssistantCaps::default();
        add_models_to_caps(&mut caps, vec![provider]);

        let final_qwen = caps.completion_models.get("refact/qwen2.5/coder/1.5b/base")
            .expect("refact/qwen2.5/coder/1.5b/base should be in caps");
        assert_eq!(
            final_qwen.scratchpad_patch.get("fim_prefix").and_then(|v| v.as_str()),
            Some("<|fim_prefix|>"),
            "scratchpad_patch should survive add_models_to_caps, got: {:?}",
            final_qwen.scratchpad_patch
        );
    }

    #[test]
    fn test_completion_model_scratchpad_patch_survives_serde_flatten() {
        // CompletionModelRecord uses #[serde(flatten)] on base: BaseModelRecord.
        // Verify that scratchpad_patch (a serde_json::Value) is correctly deserialized
        // through serde's content buffering and not lost or corrupted.
        let json = serde_json::json!({
            "n_ctx": 8192,
            "scratchpad_patch": {
                "fim_prefix": "<|fim_prefix|>",
                "fim_suffix": "<|fim_suffix|>",
                "fim_middle": "<|fim_middle|>",
                "eot": "<|endoftext|>",
                "extra_stop_tokens": ["<|repo_name|>", "<|file_sep|>"],
                "context_format": "qwen2.5",
                "rag_ratio": 0.5
            },
            "tokenizer": "hf://Qwen/Qwen2.5-Coder-0.5B",
            "scratchpad": "FIM-PSM",
            "similar_models": ["qwen2.5/coder/1.5b/base"]
        });

        let model: CompletionModelRecord = serde_json::from_value(json).unwrap();

        assert_eq!(model.scratchpad, "FIM-PSM");
        assert_eq!(model.base.n_ctx, 8192);
        assert_eq!(model.base.tokenizer, "hf://Qwen/Qwen2.5-Coder-0.5B");
        assert_eq!(model.base.similar_models, vec!["qwen2.5/coder/1.5b/base"]);

        // Critical: scratchpad_patch must survive #[serde(flatten)] content buffering
        let patch = &model.scratchpad_patch;
        assert_eq!(patch.get("fim_prefix").and_then(|v| v.as_str()), Some("<|fim_prefix|>"),
            "fim_prefix should be <|fim_prefix|>, got: {:?}", patch);
        assert_eq!(patch.get("fim_suffix").and_then(|v| v.as_str()), Some("<|fim_suffix|>"));
        assert_eq!(patch.get("fim_middle").and_then(|v| v.as_str()), Some("<|fim_middle|>"));
        assert_eq!(patch.get("eot").and_then(|v| v.as_str()), Some("<|endoftext|>"));
        assert_eq!(patch.get("context_format").and_then(|v| v.as_str()), Some("qwen2.5"));
    }

    #[test]
    fn test_nested_caps_embedding_conversion() {
        // The authenticated cloud response uses a nested format with
        // "embedding": {"endpoint": "...", "models": {...}, "default_model": "..."}
        // convert_self_hosted_caps_if_needed must extract these into flat fields
        let nested_json = serde_json::json!({
            "cloud_name": "Refact",
            "chat": {
                "endpoint": "/v1/chat/completions",
                "models": {
                    "gpt-4.1": {"n_ctx": 128000}
                },
                "default_model": "gpt-4.1",
                "default_light_model": "gpt-4.1-mini",
                "default_thinking_model": "o3-mini"
            },
            "completion": {
                "endpoint": "/v1/completions",
                "models": {
                    "Refact/1.6B": {"n_ctx": 4096}
                }
            },
            "embedding": {
                "endpoint": "v1/embeddings",
                "models": {
                    "thenlper/gte-base": {"n_ctx": 512, "size": 768}
                },
                "default_model": "thenlper/gte-base"
            },
            "telemetry_endpoints": {
                "telemetry_basic_endpoint": "https://example.com/telemetry-basic"
            },
            "tokenizer_endpoints": {},
            "support_metadata": true,
            "metadata": {"pricing": {}, "features": []}
        });

        let caps_url = "https://inference.smallcloud.ai/v1/model-catalog";
        let converted = crate::caps::caps::convert_self_hosted_caps_if_needed(
            nested_json, caps_url, "test-key"
        ).unwrap();

        let obj = converted.as_object().unwrap();

        assert_eq!(
            obj.get("embedding_endpoint").and_then(|v| v.as_str()),
            Some("https://inference.smallcloud.ai/v1/embeddings"),
            "embedding.endpoint must be resolved and extracted as embedding_endpoint"
        );

        // Embedding model must be extracted as a full object with n_ctx and embedding_size
        let emb_model = obj.get("default_embeddings_model").expect("default_embeddings_model must exist");
        assert_eq!(emb_model.get("name").and_then(|v| v.as_str()), Some("thenlper/gte-base"));
        assert_eq!(emb_model.get("n_ctx").and_then(|v| v.as_u64()), Some(512));
        assert_eq!(emb_model.get("embedding_size").and_then(|v| v.as_u64()), Some(768));

        // Chat fields must also be extracted (existing behavior)
        assert_eq!(obj.get("chat_default_model").and_then(|v| v.as_str()), Some("gpt-4.1"));
        assert_eq!(obj.get("chat_light_model").and_then(|v| v.as_str()), Some("gpt-4.1-mini"));

        // CapsProvider deserialization must work with the converted JSON
        let provider: CapsProvider = serde_json::from_value(converted).unwrap();
        assert_eq!(provider.embedding_model.base.name, "thenlper/gte-base");
        assert_eq!(provider.embedding_model.base.n_ctx, 512);
        assert_eq!(provider.embedding_model.embedding_size, 768);
        assert_eq!(provider.embedding_endpoint, "https://inference.smallcloud.ai/v1/embeddings");
        assert_eq!(provider.chat_default_model, "gpt-4.1");
    }

    #[test]
    fn test_nested_caps_embedding_fallback_no_default_model() {
        // When embedding.default_model is missing, fall back to first model in models
        let nested_json = serde_json::json!({
            "cloud_name": "Test",
            "embedding": {
                "endpoint": "v1/embeddings",
                "models": {
                    "some-new-model": {"n_ctx": 1024, "size": 512}
                }
            },
            "support_metadata": false,
            "metadata": {}
        });

        let caps_url = "https://example.com/caps.json";
        let converted = crate::caps::caps::convert_self_hosted_caps_if_needed(
            nested_json, caps_url, ""
        ).unwrap();

        let provider: CapsProvider = serde_json::from_value(converted).unwrap();
        assert_eq!(provider.embedding_model.base.name, "some-new-model",
            "should fall back to first model when default_model is missing");
        assert_eq!(provider.embedding_model.base.n_ctx, 1024);
        assert_eq!(provider.embedding_model.embedding_size, 512);
    }

    #[test]
    fn test_embedding_prefill_respects_user_configured() {
        let mut provider = CapsProvider {
            name: "test".to_string(),
            embedding_model: EmbeddingModelRecord {
                base: BaseModelRecord {
                    name: "text-embedding-3-small".to_string(),
                    n_ctx: 4096,
                    tokenizer: String::new(),
                    enabled: true,
                    user_configured: true,
                    ..Default::default()
                },
                embedding_size: 0,
                rejection_threshold: 0.0,
                embedding_batch: 0,
            },
            ..Default::default()
        };
        populate_model_records(&mut provider, false);
        assert_eq!(
            provider.embedding_model.base.tokenizer,
            "hf://Xenova/text-embedding-ada-002",
            "tokenizer should always be filled even for user-configured models"
        );
        assert_eq!(
            provider.embedding_model.base.n_ctx, 4096,
            "user-configured n_ctx should NOT be overwritten"
        );
        assert_eq!(
            provider.embedding_model.embedding_size, 0,
            "user-configured zero embedding_size should NOT be overwritten"
        );
        assert_eq!(
            provider.embedding_model.rejection_threshold, 0.0,
            "user-configured zero rejection_threshold should NOT be overwritten"
        );
    }

    #[test]
    fn test_supports_completion_false_blocks_completion_models() {
        let mut provider = CapsProvider {
            name: "test".to_string(),
            supports_completion: false,
            running_models: vec!["qwen2.5/coder/1.5b/base".to_string()],
            ..Default::default()
        };
        populate_model_records(&mut provider, false);
        assert!(
            provider.completion_models.is_empty(),
            "supports_completion=false should prevent completion model population"
        );
        assert!(
            !provider.chat_models.is_empty(),
            "chat models should still be populated regardless of supports_completion"
        );
    }

    #[test]
    fn test_qualify_model_no_double_prefix() {
        use crate::caps::DefaultModels;

        let mut defaults = DefaultModels::default();
        let other = DefaultModels {
            completion_default_model: "Refact/1.6B".to_string(),
            chat_default_model: "gpt-4.1".to_string(),
            chat_thinking_model: "refact/o3-mini".to_string(),
            chat_light_model: "".to_string(),
        };

        defaults.apply_override(&other, Some("refact"));

        // "Refact/1.6B" doesn't start with "refact/" (case mismatch), so gets prefixed
        assert_eq!(defaults.completion_default_model, "refact/Refact/1.6B",
            "model names with / but wrong provider prefix should get prefixed");
        assert_eq!(defaults.chat_default_model, "refact/gpt-4.1",
            "unqualified model should get provider prefix");
        assert_eq!(defaults.chat_thinking_model, "refact/o3-mini",
            "model already prefixed with same provider should stay unchanged");
        assert_eq!(defaults.chat_light_model, "",
            "empty model should stay empty");
    }

    #[test]
    fn test_augment_completion_model_from_nested_caps() {
        // Simulates the flow when cloud sends nested caps format:
        // convert_self_hosted_caps_if_needed creates completion_models entries with
        // endpoint/tokenizer but NO scratchpad_patch (gets default).
        // populate_model_records should augment with preset FIM tokens.
        let nested_json = serde_json::json!({
            "cloud_name": "Refact",
            "chat": {
                "endpoint": "/v1/chat/completions",
                "models": {"gpt-4.1": {"n_ctx": 128000}},
                "default_model": "gpt-4.1"
            },
            "completion": {
                "endpoint": "/v1/completions",
                "models": {
                    "qwen2.5/coder/1.5b/base": {"n_ctx": 4096}
                }
            },
            "tokenizer_endpoints": {
                "qwen2.5/coder/1.5b/base": "hf://Qwen/Qwen2.5-Coder-1.5B"
            },
            "support_metadata": true,
            "metadata": {"pricing": {}, "features": []}
        });

        let caps_url = "https://inference.smallcloud.ai/v1/model-catalog";
        let converted = crate::caps::caps::convert_self_hosted_caps_if_needed(
            nested_json, caps_url, "test-key"
        ).unwrap();

        let mut provider: CapsProvider = serde_json::from_value(converted).unwrap();

        // Before post_process: model exists with tokenizer from tokenizer_endpoints
        // but scratchpad_patch is default (no FIM tokens)
        assert!(provider.completion_models.contains_key("qwen2.5/coder/1.5b/base"),
            "completion model should exist from conversion");
        let model_before = provider.completion_models.get("qwen2.5/coder/1.5b/base").unwrap();
        assert!(model_before.scratchpad_patch.get("fim_prefix").is_none(),
            "scratchpad_patch should lack fim_prefix before augmentation");
        assert_eq!(model_before.base.tokenizer, "hf://Qwen/Qwen2.5-Coder-1.5B",
            "tokenizer should come from tokenizer_endpoints");

        // post_process_provider should augment the model with preset scratchpad data
        post_process_provider(&mut provider, false, false);

        let model_after = provider.completion_models.get("qwen2.5/coder/1.5b/base")
            .expect("model should still exist after post_process");

        // FIM tokens from qwen2.5-coder-base preset must be applied
        assert_eq!(
            model_after.scratchpad_patch.get("fim_prefix").and_then(|v| v.as_str()),
            Some("<|fim_prefix|>"),
            "scratchpad_patch should have <|fim_prefix|> from preset after augmentation"
        );
        assert_eq!(
            model_after.scratchpad_patch.get("fim_suffix").and_then(|v| v.as_str()),
            Some("<|fim_suffix|>"),
        );
        assert_eq!(
            model_after.scratchpad_patch.get("fim_middle").and_then(|v| v.as_str()),
            Some("<|fim_middle|>"),
        );
        assert_eq!(
            model_after.scratchpad_patch.get("context_format").and_then(|v| v.as_str()),
            Some("qwen2.5"),
            "context_format should come from preset (not default 'chat')"
        );

        // Cloud-provided tokenizer should be preserved (NOT overwritten by preset)
        assert_eq!(model_after.base.tokenizer, "hf://Qwen/Qwen2.5-Coder-1.5B",
            "cloud-provided tokenizer should be preserved, not overwritten by preset 0.5B");

        // n_ctx should use the smaller of cloud value and preset value
        // Cloud provides 4096, preset has 8192, but models_dict_patch not involved here
        assert_eq!(model_after.base.n_ctx, 4096,
            "cloud-provided n_ctx should be preserved when non-zero");
    }

    #[test]
    fn test_qualify_model_with_slashes() {
        use crate::caps::DefaultModels;

        // OpenRouter-style models: "openai/gpt-4.1" under provider "openrouter"
        let mut defaults = DefaultModels::default();
        let other = DefaultModels {
            chat_default_model: "openai/gpt-4.1".to_string(),
            chat_light_model: "openrouter/openai/gpt-4.1".to_string(),
            ..Default::default()
        };
        defaults.apply_override(&other, Some("openrouter"));
        assert_eq!(defaults.chat_default_model, "openrouter/openai/gpt-4.1",
            "cross-provider model names must get the provider prefix");
        assert_eq!(defaults.chat_light_model, "openrouter/openai/gpt-4.1",
            "already correctly prefixed model should stay unchanged");

        // No provider name
        let mut defaults2 = DefaultModels::default();
        let other2 = DefaultModels {
            chat_default_model: "gpt-4.1".to_string(),
            ..Default::default()
        };
        defaults2.apply_override(&other2, None);
        assert_eq!(defaults2.chat_default_model, "gpt-4.1",
            "no provider name should return model as-is");
    }
}
