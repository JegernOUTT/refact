use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use indexmap::IndexMap;

use crate::caps::{
    BaseModelRecord, ChatModelRecord, CodeAssistantCaps, CompletionModelRecord,
    EmbeddingModelRecord, HasBaseModelRecord, strip_model_from_finetune,
};
use crate::custom_error::YamlError;

#[cfg(test)]
use refact_core::llm_types::WireFormat;
use refact_providers::identity::provider_identity_from_yaml;

pub use refact_caps_core::provider_config::{
    CapsProvider, CompletionPresets, EmbeddingPresets, default_endpoint_style, extend_collection,
    extend_model_collection, set_field_if_exists,
};

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
        "doubao",
        include_str!("../yaml_configs/default_providers/doubao.yaml"),
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
        "github_copilot",
        include_str!("../yaml_configs/default_providers/github_copilot.yaml"),
    ),
    (
        "kimi",
        include_str!("../yaml_configs/default_providers/kimi.yaml"),
    ),
    (
        "lmstudio",
        include_str!("../yaml_configs/default_providers/lmstudio.yaml"),
    ),
    (
        "minimax",
        include_str!("../yaml_configs/default_providers/minimax.yaml"),
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
        "qwen",
        include_str!("../yaml_configs/default_providers/qwen.yaml"),
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
    (
        "zhipu",
        include_str!("../yaml_configs/default_providers/zhipu.yaml"),
    ),
];
static PARSED_PROVIDERS: OnceLock<IndexMap<String, CapsProvider>> = OnceLock::new();

pub fn get_provider_templates() -> &'static IndexMap<String, CapsProvider> {
    PARSED_PROVIDERS.get_or_init(|| {
        let mut map = IndexMap::new();
        for (name, yaml) in PROVIDER_TEMPLATES {
            if let Ok(mut provider) = serde_yaml::from_str::<CapsProvider>(yaml) {
                provider.name = name.to_string();
                provider.base_provider = name.to_string();
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
    _experimental: bool,
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
        let instance_id = match yaml_path.file_stem() {
            Some(name) => name.to_string_lossy().to_string(),
            None => continue,
        };

        if instance_id == "refact" {
            tracing::warn!(
                "Legacy Refact Cloud provider config '{}' is ignored; configure a BYOK provider instead",
                yaml_path.display()
            );
            continue;
        }

        let duplicate_key = instance_id.to_ascii_lowercase();
        if !seen_provider_names.insert(duplicate_key) {
            error_log.push(YamlError {
                path: yaml_path.to_string_lossy().to_string(),
                error_line: 0,
                error_msg: format!(
                    "Duplicate provider name '{}' (another file with the same stem was already processed)",
                    instance_id
                ),
            });
            continue;
        }

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

        let config_file_value = match serde_yaml::from_str::<serde_yaml::Value>(&content) {
            Ok(value) => value,
            Err(e) => {
                error_log.push(YamlError {
                    path: yaml_path.to_string_lossy().to_string(),
                    error_line: e.location().map_or(0, |loc| loc.line()),
                    error_msg: format!("Failed to parse YAML: {}", e),
                });
                continue;
            }
        };

        let identity = match provider_identity_from_yaml(&instance_id, &config_file_value) {
            Ok(identity) => identity,
            Err(e) => {
                error_log.push(YamlError {
                    path: yaml_path.to_string_lossy().to_string(),
                    error_line: 0,
                    error_msg: e,
                });
                continue;
            }
        };

        let provider = if let Some(template) = provider_templates.get(&identity.base_provider) {
            let mut provider = template.clone();
            if let Err(e) = provider.apply_override(config_file_value) {
                error_log.push(YamlError {
                    path: yaml_path.to_string_lossy().to_string(),
                    error_line: 0,
                    error_msg: e,
                });
                continue;
            }
            provider.name = identity.instance_id;
            provider.base_provider = identity.base_provider;
            provider
        } else {
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
            provider.name = identity.instance_id;
            provider.base_provider = identity.base_provider;
            provider
        };

        providers.push(provider);
    }

    (providers, error_log)
}

fn add_running_models(provider: &mut CapsProvider) {
    let models_to_add = vec![
        &provider.chat_default_model,
        &provider.chat_light_model,
        &provider.chat_thinking_model,
        &provider.chat_buddy_model,
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
                    if model_rec.base.n_ctx == 0
                        || provider.code_completion_n_ctx < model_rec.base.n_ctx
                    {
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

const UNPARSED_COMPLETION_PRESETS: &str = include_str!("../completion_presets.json");
const UNPARSED_EMBEDDING_PRESETS: &str = include_str!("../embedding_presets.json");

static COMPLETION_PRESETS: OnceLock<CompletionPresets> = OnceLock::new();
static EMBEDDING_PRESETS: OnceLock<EmbeddingPresets> = OnceLock::new();

pub fn get_completion_presets() -> &'static CompletionPresets {
    COMPLETION_PRESETS.get_or_init(|| {
        serde_json::from_str::<CompletionPresets>(UNPARSED_COMPLETION_PRESETS).unwrap_or_else(|e| {
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
        serde_json::from_str::<EmbeddingPresets>(UNPARSED_EMBEDDING_PRESETS).unwrap_or_else(|e| {
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
/// Models imported from user/provider config can have correct endpoint/tokenizer
/// but lack FIM token configuration. This fills in the missing data from
/// completion_presets.json without overwriting configured fields.
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
    if let Some(preset) =
        find_model_match(&name_owned, &IndexMap::new(), known_presets, experimental)
    {
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
                // Model already exists but may lack scratchpad data (FIM tokens).
                // Augment from preset without overwriting configured fields.
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

#[cfg(test)]
pub async fn get_provider_from_template_and_config_file(
    config_dir: &Path,
    name: &str,
    config_file_must_exist: bool,
    post_process: bool,
    experimental: bool,
) -> Result<CapsProvider, String> {
    use crate::custom_error::MapErrToString;
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
    provider.base_provider = name.to_string();

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
        assert_eq!(
            patch.get("fim_prefix").and_then(|v| v.as_str()),
            Some("<|fim_prefix|>"),
            "fim_prefix should be <|fim_prefix|>, got: {:?}",
            patch
        );
        assert_eq!(
            patch.get("fim_suffix").and_then(|v| v.as_str()),
            Some("<|fim_suffix|>")
        );
        assert_eq!(
            patch.get("fim_middle").and_then(|v| v.as_str()),
            Some("<|fim_middle|>")
        );
        assert_eq!(
            patch.get("eot").and_then(|v| v.as_str()),
            Some("<|endoftext|>")
        );
        assert_eq!(
            patch.get("context_format").and_then(|v| v.as_str()),
            Some("qwen2.5")
        );
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
            provider.embedding_model.base.tokenizer, "hf://Xenova/text-embedding-ada-002",
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

    async fn write_provider_config(temp: &tempfile::TempDir, file_name: &str, yaml: &str) {
        let providers_dir = temp.path().join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(providers_dir.join(file_name), yaml)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn provider_instances_use_base_template_and_instance_model_ids() {
        let temp = tempfile::tempdir().unwrap();
        write_provider_config(
            &temp,
            "openai.yaml",
            "api_key: sk-main\nenabled: true\nenabled_models:\n  - gpt-4.1\n",
        )
        .await;
        write_provider_config(
            &temp,
            "openai_2.yaml",
            "base_provider: openai\napi_key: sk-two\nenabled: true\nenabled_models:\n  - gpt-4.1\n",
        )
        .await;

        let (mut providers, errors) = read_providers_d(Vec::new(), temp.path(), false).await;
        assert!(errors.is_empty(), "{}", errors.len());
        providers.sort_by(|a, b| a.name.cmp(&b.name));
        for provider in &mut providers {
            post_process_provider(provider, false, false);
        }

        let mut caps = CodeAssistantCaps::default();
        add_models_to_caps(&mut caps, providers);

        let openai = caps.chat_models.get("openai/gpt-4.1").unwrap();
        let openai_2 = caps.chat_models.get("openai_2/gpt-4.1").unwrap();
        assert_eq!(openai.base.id, "openai/gpt-4.1");
        assert_eq!(openai_2.base.id, "openai_2/gpt-4.1");
        assert_eq!(
            openai.base.endpoint,
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            openai_2.base.endpoint,
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(openai.base.wire_format, WireFormat::OpenaiChatCompletions);
        assert_eq!(openai_2.base.wire_format, WireFormat::OpenaiChatCompletions);
        assert_eq!(openai.base.api_key, "sk-main");
        assert_eq!(openai_2.base.api_key, "sk-two");
    }

    #[tokio::test]
    async fn legacy_singleton_provider_config_without_identity_fields_remains_valid() {
        let temp = tempfile::tempdir().unwrap();
        write_provider_config(
            &temp,
            "openai.yaml",
            "api_key: sk-main\nenabled: true\nenabled_models:\n  - gpt-4.1\n",
        )
        .await;

        let (mut providers, errors) = read_providers_d(Vec::new(), temp.path(), false).await;
        assert!(errors.is_empty(), "{}", errors.len());
        assert_eq!(providers.len(), 1);
        let provider = providers.get_mut(0).unwrap();
        assert_eq!(provider.name, "openai");
        assert_eq!(provider.base_provider, "openai");
        assert_eq!(provider.api_key, "sk-main");

        post_process_provider(provider, false, false);
        assert!(provider.chat_models.contains_key("gpt-4.1"));
    }

    #[tokio::test]
    async fn alias_provider_without_base_provider_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        write_provider_config(
            &temp,
            "openai_2.yaml",
            "api_key: sk-two\nenabled: true\nenabled_models:\n  - gpt-4.1\n",
        )
        .await;

        let (providers, errors) = read_providers_d(Vec::new(), temp.path(), false).await;

        assert!(providers.is_empty());
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].error_msg.contains("must set base_provider"),
            "{}",
            errors[0].error_msg
        );
    }

    #[tokio::test]
    async fn custom_provider_extra_headers_reach_caps_model_records() {
        let temp = tempfile::tempdir().unwrap();
        let providers_dir = temp.path().join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(
            providers_dir.join("custom.yaml"),
            r#"
enabled: true
api_key: sk-test
chat_endpoint: https://example.com/v1/chat/completions
enabled_models:
  - my-model
extra_headers:
  X-Proxy-Token: secret-token
  X-Tenant: team-a
"#,
        )
        .await
        .unwrap();

        let provider =
            get_provider_from_template_and_config_file(temp.path(), "custom", true, true, false)
                .await
                .unwrap();

        assert_eq!(
            provider
                .extra_headers
                .get("X-Proxy-Token")
                .map(String::as_str),
            Some("secret-token")
        );
        assert_eq!(
            provider.extra_headers.get("X-Tenant").map(String::as_str),
            Some("team-a")
        );

        let mut caps = CodeAssistantCaps::default();
        add_models_to_caps(&mut caps, vec![provider]);
        let model = caps.chat_models.get("custom/my-model").unwrap();

        assert_eq!(
            model
                .base
                .extra_headers
                .get("X-Proxy-Token")
                .map(String::as_str),
            Some("secret-token")
        );
        assert_eq!(
            model.base.extra_headers.get("X-Tenant").map(String::as_str),
            Some("team-a")
        );
    }

    #[tokio::test]
    async fn custom_provider_extra_headers_string_reaches_caps_provider() {
        let temp = tempfile::tempdir().unwrap();
        let providers_dir = temp.path().join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(
            providers_dir.join("custom.yaml"),
            r#"
enabled_models:
  - my-model
extra_headers: |
  X-String: string-secret
  X-Number: 7
"#,
        )
        .await
        .unwrap();

        let provider =
            get_provider_from_template_and_config_file(temp.path(), "custom", true, true, false)
                .await
                .unwrap();

        assert_eq!(
            provider.extra_headers.get("X-String").map(String::as_str),
            Some("string-secret")
        );
        assert!(provider.extra_headers.get("X-Number").is_none());
    }

    #[test]
    fn test_qualify_model_no_double_prefix() {
        use crate::caps::DefaultModels;

        let mut defaults = DefaultModels::default();
        let other = DefaultModels {
            completion_default_model: "Qwen/Qwen2.5-Coder-1.5B".to_string(),
            chat_default_model: "gpt-4.1".to_string(),
            chat_thinking_model: "custom/o3-mini".to_string(),
            chat_light_model: "".to_string(),
            ..Default::default()
        };

        defaults.apply_override(&other, Some("custom"));

        // Cross-provider model names must get the configured provider prefix.
        assert_eq!(
            defaults.completion_default_model, "custom/Qwen/Qwen2.5-Coder-1.5B",
            "model names with / but wrong provider prefix should get prefixed"
        );
        assert_eq!(
            defaults.chat_default_model, "custom/gpt-4.1",
            "unqualified model should get provider prefix"
        );
        assert_eq!(
            defaults.chat_thinking_model, "custom/o3-mini",
            "model already prefixed with same provider should stay unchanged"
        );
        assert_eq!(
            defaults.chat_light_model, "",
            "empty model should stay empty"
        );
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
        assert_eq!(
            defaults.chat_default_model, "openrouter/openai/gpt-4.1",
            "cross-provider model names must get the provider prefix"
        );
        assert_eq!(
            defaults.chat_light_model, "openrouter/openai/gpt-4.1",
            "already correctly prefixed model should stay unchanged"
        );

        // No provider name
        let mut defaults2 = DefaultModels::default();
        let other2 = DefaultModels {
            chat_default_model: "gpt-4.1".to_string(),
            ..Default::default()
        };
        defaults2.apply_override(&other2, None);
        assert_eq!(
            defaults2.chat_default_model, "gpt-4.1",
            "no provider name should return model as-is"
        );
    }
}
