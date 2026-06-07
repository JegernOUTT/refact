use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use indexmap::IndexMap;

use crate::caps::{
    BaseModelRecord, ChatModelRecord, CodeAssistantCaps, CompletionModelRecord, DefaultModels,
    EmbeddingEndpointStyle, EmbeddingModelRecord, HasBaseModelRecord, strip_model_from_finetune,
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
        &provider.chat_model_2,
        &provider.task_planner_agent_model,
        &provider.chat_light_model,
        &provider.chat_thinking_model,
        &provider.chat_buddy_model,
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

pub fn add_models_to_caps(
    caps: &mut CodeAssistantCaps,
    providers: Vec<CapsProvider>,
) -> Vec<EmbeddingModelRecord> {
    let mut embedding_models = Vec::new();

    fn add_provider_details_to_model(
        base_model_rec: &mut BaseModelRecord,
        provider: &CapsProvider,
        model_name: &str,
        endpoint: &str,
    ) {
        base_model_rec.api_key = provider.api_key.clone();
        base_model_rec.tokenizer_api_key = provider.tokenizer_api_key.clone();
        base_model_rec.endpoint = if endpoint.is_empty() {
            base_model_rec.endpoint.clone()
        } else {
            endpoint.replace("$MODEL", model_name)
        };
        base_model_rec.endpoint_style = provider.endpoint_style.clone();
        base_model_rec.wire_format = provider.wire_format;
        base_model_rec.extra_headers = provider.extra_headers.clone();
        base_model_rec.supports_cache_control =
            base_model_rec.supports_cache_control && provider.supports_cache_control;
    }

    fn add_provider_details_to_model_preserving_endpoint(
        base_model_rec: &mut BaseModelRecord,
        provider: &CapsProvider,
        model_name: &str,
    ) {
        let endpoint = base_model_rec.endpoint.clone();
        add_provider_details_to_model(base_model_rec, provider, model_name, &endpoint);
    }

    for mut provider in providers {
        if !provider.enabled {
            continue;
        }

        let mut inserted_chat_model = false;
        let mut inserted_embedding_model = false;

        let completion_models = std::mem::take(&mut provider.completion_models);
        for (model_name, mut model_rec) in completion_models {
            let endpoint = if model_rec.base.endpoint.is_empty() {
                provider.completion_endpoint.clone()
            } else {
                model_rec.base.endpoint.clone()
            };
            model_rec.base.supports_cache_control =
                model_rec.base.supports_cache_control && provider.supports_cache_control;
            add_provider_details_to_model(&mut model_rec.base, &provider, &model_name, &endpoint);

            if provider.code_completion_n_ctx > 0 {
                if model_rec.base.n_ctx == 0
                    || provider.code_completion_n_ctx < model_rec.base.n_ctx
                {
                    model_rec.base.n_ctx = provider.code_completion_n_ctx;
                }
            }

            if model_rec.base.completion_endpoint_style.is_empty()
                && !provider.completion_endpoint_style.is_empty()
            {
                model_rec.base.completion_endpoint_style =
                    provider.completion_endpoint_style.clone();
            }

            if !completion_model_is_selectable(&provider, &model_rec) {
                continue;
            }

            caps.completion_models
                .insert(model_rec.base.id.clone(), Arc::new(model_rec));
        }

        let chat_models = std::mem::take(&mut provider.chat_models);
        for (model_name, mut model_rec) in chat_models {
            model_rec.base.supports_cache_control =
                model_rec.base.supports_cache_control && provider.supports_cache_control;
            if model_rec.base.endpoint.is_empty() {
                add_provider_details_to_model(
                    &mut model_rec.base,
                    &provider,
                    &model_name,
                    &provider.chat_endpoint,
                );
            } else {
                add_provider_details_to_model_preserving_endpoint(
                    &mut model_rec.base,
                    &provider,
                    &model_name,
                );
            }

            caps.chat_models
                .insert(model_rec.base.id.clone(), Arc::new(model_rec));
            inserted_chat_model = true;
        }

        if provider.embedding_model.is_configured() && provider.embedding_model.base.enabled {
            let mut embedding_model = std::mem::take(&mut provider.embedding_model);
            let endpoint = if embedding_model.base.endpoint.is_empty() {
                provider.embedding_endpoint.clone()
            } else {
                embedding_model.base.endpoint.clone()
            };
            embedding_model.base.supports_cache_control =
                embedding_model.base.supports_cache_control && provider.supports_cache_control;

            let model_name = embedding_model.base.name.clone();
            add_provider_details_to_model(
                &mut embedding_model.base,
                &provider,
                &model_name,
                &endpoint,
            );
            if embedding_model.base.embedding_endpoint_style.is_empty()
                && !provider.embedding_endpoint_style.is_empty()
            {
                embedding_model.base.embedding_endpoint_style =
                    provider.embedding_endpoint_style.clone();
            }
            if !embedding_model_is_selectable(&provider, &embedding_model) {
                apply_provider_defaults_for_inserted_models(
                    caps,
                    &provider,
                    inserted_chat_model,
                    false,
                );
                continue;
            }
            if provider.embedding_default_model.is_empty() {
                tracing::info!(
                    "Embedding provider '{}' has no embedding_default_model; deterministic selection may use sorted fallback",
                    provider.name
                );
            }
            embedding_models.push(embedding_model.clone());
            inserted_embedding_model = true;
        }

        apply_provider_defaults_for_inserted_models(
            caps,
            &provider,
            inserted_chat_model,
            inserted_embedding_model,
        );
    }

    select_embedding_model(caps, &embedding_models);

    embedding_models
}

fn select_embedding_model(caps: &mut CodeAssistantCaps, embedding_models: &[EmbeddingModelRecord]) {
    if embedding_models.is_empty() {
        caps.embedding_model = EmbeddingModelRecord::default();
        caps.defaults.embedding_default_model.clear();
        return;
    }

    if !caps.defaults.embedding_default_model.is_empty() {
        if let Some(model) = embedding_models
            .iter()
            .find(|model| model.base.id == caps.defaults.embedding_default_model)
            .or_else(|| {
                embedding_models
                    .iter()
                    .find(|model| model.base.name == caps.defaults.embedding_default_model)
            })
        {
            caps.embedding_model = model.clone();
            caps.defaults.embedding_default_model = model.base.id.clone();
            return;
        }
        tracing::warn!(
            "Embedding default model '{}' was not found in available embedding models; using deterministic fallback",
            caps.defaults.embedding_default_model
        );
    }

    let mut candidates: Vec<&EmbeddingModelRecord> = embedding_models.iter().collect();
    candidates.sort_by(|a, b| a.base.id.cmp(&b.base.id));
    if let Some(model) = candidates.first() {
        tracing::info!(
            "Auto-selecting embedding model by deterministic fallback: {}",
            model.base.id
        );
        caps.embedding_model = (*model).clone();
        caps.defaults.embedding_default_model = model.base.id.clone();
    }
}

fn completion_model_is_selectable(provider: &CapsProvider, model: &CompletionModelRecord) -> bool {
    let Ok(style) = model.base.effective_completion_endpoint_style() else {
        tracing::warn!(
            "Skipping completion model '{}' for provider '{}' because completion_endpoint_style is invalid",
            model.base.id,
            provider.name
        );
        return false;
    };
    if !style.is_supported() {
        tracing::warn!(
            "Skipping completion model '{}' for provider '{}' because completion_endpoint_style '{}' is not supported",
            model.base.id,
            provider.name,
            style
        );
        return false;
    }
    if !endpoint_is_selectable(&model.base.endpoint) {
        tracing::warn!(
            "Skipping completion model '{}' for provider '{}' because completion endpoint is invalid",
            model.base.id,
            provider.name
        );
        return false;
    }
    true
}

fn embedding_model_is_selectable(provider: &CapsProvider, model: &EmbeddingModelRecord) -> bool {
    let Ok(style) = model.base.effective_embedding_endpoint_style() else {
        tracing::warn!(
            "Skipping embedding model '{}' for provider '{}' because embedding_endpoint_style is invalid",
            model.base.id,
            provider.name
        );
        return false;
    };
    if !style.is_supported() {
        tracing::warn!(
            "Skipping embedding model '{}' for provider '{}' because embedding_endpoint_style '{}' is not supported",
            model.base.id,
            provider.name,
            style
        );
        return false;
    }
    if model.base.endpoint.trim().is_empty() && style != EmbeddingEndpointStyle::OllamaNative {
        tracing::warn!(
            "Skipping embedding model '{}' for provider '{}' because embedding endpoint is invalid",
            model.base.id,
            provider.name
        );
        return false;
    }
    if !model.base.endpoint.trim().is_empty() && !endpoint_is_selectable(&model.base.endpoint) {
        tracing::warn!(
            "Skipping embedding model '{}' for provider '{}' because embedding endpoint is invalid",
            model.base.id,
            provider.name
        );
        return false;
    }
    true
}

fn endpoint_is_selectable(endpoint: &str) -> bool {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return false;
    }
    url::Url::parse(endpoint)
        .map(|url| matches!(url.scheme(), "http" | "https"))
        .unwrap_or(false)
}

fn apply_provider_defaults_for_inserted_models(
    caps: &mut CodeAssistantCaps,
    provider: &CapsProvider,
    inserted_chat_model: bool,
    inserted_embedding_model: bool,
) {
    let provider_defaults = provider.defaults();
    let mut role_defaults = DefaultModels {
        chat_default_model: provider_defaults.chat_default_model,
        chat_model_2: provider_defaults.chat_model_2,
        task_planner_agent_model: provider_defaults.task_planner_agent_model,
        chat_thinking_model: provider_defaults.chat_thinking_model,
        chat_light_model: provider_defaults.chat_light_model,
        chat_buddy_model: provider_defaults.chat_buddy_model,
        ..Default::default()
    };

    if !inserted_chat_model {
        role_defaults.chat_default_model.clear();
        role_defaults.chat_model_2.clear();
        role_defaults.task_planner_agent_model.clear();
        role_defaults.chat_thinking_model.clear();
        role_defaults.chat_light_model.clear();
        role_defaults.chat_buddy_model.clear();
    }

    if !provider_defaults.completion_default_model.is_empty() {
        let qualified_completion_default =
            qualify_model_for_provider(&provider_defaults.completion_default_model, &provider.name);
        if caps
            .completion_models
            .contains_key(&qualified_completion_default)
        {
            role_defaults.completion_default_model = provider_defaults.completion_default_model;
        }
    }

    if inserted_embedding_model && !provider_defaults.embedding_default_model.is_empty() {
        role_defaults.embedding_default_model = provider_defaults.embedding_default_model;
    }

    caps.defaults
        .apply_override(&role_defaults, Some(&provider.name));
}

fn qualify_model_for_provider(model: &str, provider: &str) -> String {
    if model.is_empty() || model.starts_with(&format!("{provider}/")) {
        model.to_string()
    } else {
        format!("{provider}/{model}")
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

    if provider.supports_completion && !provider.completion_default_model.is_empty() {
        let model_name = provider.completion_default_model.clone();
        if !provider.completion_models.contains_key(&model_name) {
            if let Some(model_rec) = find_model_match(
                &model_name,
                &provider.completion_models,
                &completion_presets.completion_models,
                experimental,
            ) {
                provider.completion_models.insert(model_name, model_rec);
            }
        }
    }

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
                    supports_cache_control: provider.supports_cache_control,
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
    use crate::caps::resolve_completion_model;
    use std::collections::HashMap;

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
                dimensions: None,
                query_prefix: String::new(),
                document_prefix: String::new(),
                rejection_threshold: 0.0,
                embedding_batch: 0,
                ..Default::default()
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

    #[test]
    fn disabled_provider_contributes_no_role_models_or_defaults() {
        let provider = CapsProvider {
            name: "disabled".to_string(),
            enabled: false,
            completion_default_model: "coder".to_string(),
            embedding_default_model: "embed".to_string(),
            completion_endpoint: "https://example.com/v1/completions".to_string(),
            embedding_endpoint: "https://example.com/v1/embeddings".to_string(),
            completion_models: IndexMap::from([(
                "coder".to_string(),
                CompletionModelRecord::default(),
            )]),
            embedding_model: EmbeddingModelRecord {
                base: BaseModelRecord {
                    name: "embed".to_string(),
                    id: "disabled/embed".to_string(),
                    n_ctx: 8192,
                    enabled: true,
                    ..Default::default()
                },
                embedding_size: 1536,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut caps = CodeAssistantCaps::default();
        let embedding_models = add_models_to_caps(&mut caps, vec![provider]);

        assert!(caps.completion_models.is_empty());
        assert!(caps.chat_models.is_empty());
        assert!(embedding_models.is_empty());
        assert!(caps.embedding_model.base.id.is_empty());
        assert!(caps.defaults.completion_default_model.is_empty());
        assert!(caps.defaults.embedding_default_model.is_empty());
    }

    #[test]
    fn invalid_role_endpoints_are_omitted_from_caps() {
        let provider = CapsProvider {
            name: "custom".to_string(),
            completion_default_model: "coder".to_string(),
            embedding_default_model: "embed".to_string(),
            completion_endpoint_style: "openai_responses".to_string(),
            embedding_endpoint_style: "cohere_v2".to_string(),
            completion_endpoint: "https://example.com/v1/responses".to_string(),
            embedding_endpoint: "https://example.com/v1/embed".to_string(),
            completion_models: IndexMap::from([(
                "coder".to_string(),
                CompletionModelRecord::default(),
            )]),
            embedding_model: EmbeddingModelRecord {
                base: BaseModelRecord {
                    name: "embed".to_string(),
                    id: "custom/embed".to_string(),
                    n_ctx: 8192,
                    enabled: true,
                    ..Default::default()
                },
                embedding_size: 1536,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut caps = CodeAssistantCaps::default();
        let embedding_models = add_models_to_caps(&mut caps, vec![provider]);

        assert!(caps.completion_models.is_empty());
        assert!(embedding_models.is_empty());
        assert!(caps.embedding_model.base.id.is_empty());
    }

    #[test]
    fn multiple_embedding_providers_use_default_or_sorted_fallback() {
        let first = CapsProvider {
            name: "z_provider".to_string(),
            embedding_default_model: "z_provider/embed-z".to_string(),
            embedding_endpoint: "https://z.example/v1/embeddings".to_string(),
            embedding_model: EmbeddingModelRecord {
                base: BaseModelRecord {
                    name: "embed-z".to_string(),
                    id: "z_provider/embed-z".to_string(),
                    n_ctx: 8192,
                    enabled: true,
                    ..Default::default()
                },
                embedding_size: 1024,
                ..Default::default()
            },
            ..Default::default()
        };
        let second = CapsProvider {
            name: "a_provider".to_string(),
            embedding_endpoint: "https://a.example/v1/embeddings".to_string(),
            embedding_model: EmbeddingModelRecord {
                base: BaseModelRecord {
                    name: "embed-a".to_string(),
                    id: "a_provider/embed-a".to_string(),
                    n_ctx: 8192,
                    enabled: true,
                    ..Default::default()
                },
                embedding_size: 2048,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut caps = CodeAssistantCaps::default();
        add_models_to_caps(&mut caps, vec![second.clone(), first.clone()]);
        assert_eq!(caps.embedding_model.base.id, "z_provider/embed-z");

        let mut fallback_caps = CodeAssistantCaps::default();
        let mut no_default_first = first;
        no_default_first.embedding_default_model.clear();
        add_models_to_caps(&mut fallback_caps, vec![no_default_first, second]);
        assert_eq!(fallback_caps.embedding_model.base.id, "a_provider/embed-a");
    }

    #[test]
    fn provider_details_propagate_to_role_models_with_endpoint_overrides() {
        let provider = CapsProvider {
            name: "custom".to_string(),
            endpoint_style: "openai".to_string(),
            completion_endpoint_style: "openai_chat_completions".to_string(),
            embedding_endpoint_style: "openai".to_string(),
            api_key: "sk-role".to_string(),
            tokenizer_api_key: "tok-role".to_string(),
            extra_headers: HashMap::from([("X-Role".to_string(), "secret".to_string())]),
            supports_cache_control: false,
            completion_models: IndexMap::from([(
                "coder".to_string(),
                CompletionModelRecord {
                    base: BaseModelRecord {
                        id: "custom/coder".to_string(),
                        endpoint: "https://model.example/v1/chat/completions".to_string(),
                        supports_cache_control: true,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )]),
            embedding_endpoint: "https://provider.example/v1/embeddings".to_string(),
            embedding_model: EmbeddingModelRecord {
                base: BaseModelRecord {
                    name: "embed".to_string(),
                    id: "custom/embed".to_string(),
                    endpoint: "https://embed.example/v1/embeddings".to_string(),
                    n_ctx: 8192,
                    enabled: true,
                    supports_cache_control: true,
                    ..Default::default()
                },
                embedding_size: 1536,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut caps = CodeAssistantCaps::default();
        add_models_to_caps(&mut caps, vec![provider]);

        let completion = caps.completion_models.get("custom/coder").unwrap();
        assert_eq!(
            completion.base.endpoint,
            "https://model.example/v1/chat/completions"
        );
        assert_eq!(completion.base.api_key, "sk-role");
        assert_eq!(completion.base.tokenizer_api_key, "tok-role");
        assert_eq!(
            completion
                .base
                .extra_headers
                .get("X-Role")
                .map(String::as_str),
            Some("secret")
        );
        assert_eq!(
            completion.base.completion_endpoint_style,
            "openai_chat_completions"
        );
        assert!(!completion.base.supports_cache_control);

        assert_eq!(
            caps.embedding_model.base.endpoint,
            "https://embed.example/v1/embeddings"
        );
        assert_eq!(caps.embedding_model.base.api_key, "sk-role");
        assert_eq!(caps.embedding_model.base.tokenizer_api_key, "tok-role");
        assert_eq!(
            caps.embedding_model
                .base
                .extra_headers
                .get("X-Role")
                .map(String::as_str),
            Some("secret")
        );
        assert_eq!(caps.embedding_model.base.embedding_endpoint_style, "openai");
        assert!(!caps.embedding_model.base.supports_cache_control);
    }

    #[test]
    fn completion_only_custom_provider_does_not_create_chat_model() {
        let mut provider = CapsProvider {
            name: "custom".to_string(),
            base_provider: "custom".to_string(),
            completion_endpoint: "https://completion.example/v1/completions".to_string(),
            completion_default_model: "qwen2.5/coder/1.5b/base".to_string(),
            ..Default::default()
        };
        post_process_provider(&mut provider, false, false);

        let mut caps = CodeAssistantCaps::default();
        add_models_to_caps(&mut caps, vec![provider]);

        assert!(caps
            .completion_models
            .contains_key("custom/qwen2.5/coder/1.5b/base"));
        assert!(!caps
            .chat_models
            .contains_key("custom/qwen2.5/coder/1.5b/base"));
        assert_eq!(
            caps.defaults.completion_default_model,
            "custom/qwen2.5/coder/1.5b/base"
        );
    }

    #[test]
    fn explicit_chat_model_endpoint_inherits_provider_runtime_details() {
        let mut provider = CapsProvider {
            name: "custom".to_string(),
            base_provider: "custom".to_string(),
            api_key: "sk-provider".to_string(),
            tokenizer_api_key: "tok-provider".to_string(),
            endpoint_style: "anthropic".to_string(),
            wire_format: WireFormat::AnthropicMessages,
            supports_cache_control: false,
            extra_headers: HashMap::from([("X-Test".to_string(), "header-value".to_string())]),
            chat_models: IndexMap::from([(
                "custom-chat".to_string(),
                ChatModelRecord {
                    base: BaseModelRecord {
                        endpoint: "https://model.example/v1/messages".to_string(),
                        supports_cache_control: true,
                        enabled: true,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        post_process_provider(&mut provider, false, false);

        let mut caps = CodeAssistantCaps::default();
        add_models_to_caps(&mut caps, vec![provider]);

        let model = caps.chat_models.get("custom/custom-chat").unwrap();
        assert_eq!(model.base.endpoint, "https://model.example/v1/messages");
        assert_eq!(model.base.api_key, "sk-provider");
        assert_eq!(model.base.tokenizer_api_key, "tok-provider");
        assert_eq!(model.base.endpoint_style, "anthropic");
        assert_eq!(model.base.wire_format, WireFormat::AnthropicMessages);
        assert_eq!(
            model.base.extra_headers.get("X-Test").map(String::as_str),
            Some("header-value")
        );
        assert!(!model.base.supports_cache_control);
    }

    #[test]
    fn explicit_custom_role_config_exposes_completion_and_embedding_defaults() {
        let provider = CapsProvider {
            name: "custom".to_string(),
            completion_default_model: "qwen-coder".to_string(),
            embedding_default_model: "text-embedding-3-small".to_string(),
            completion_endpoint: "https://completion.example/v1/chat/completions".to_string(),
            embedding_endpoint: "https://embedding.example/v1/embeddings".to_string(),
            completion_endpoint_style: "openai_chat_completions".to_string(),
            completion_models: IndexMap::from([(
                "qwen-coder".to_string(),
                CompletionModelRecord {
                    base: BaseModelRecord {
                        id: "custom/qwen-coder".to_string(),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )]),
            embedding_model: EmbeddingModelRecord {
                base: BaseModelRecord {
                    name: "text-embedding-3-small".to_string(),
                    id: "custom/text-embedding-3-small".to_string(),
                    n_ctx: 8191,
                    enabled: true,
                    ..Default::default()
                },
                embedding_size: 1536,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut caps = CodeAssistantCaps::default();
        add_models_to_caps(&mut caps, vec![provider]);

        assert!(caps.completion_models.contains_key("custom/qwen-coder"));
        assert_eq!(caps.defaults.completion_default_model, "custom/qwen-coder");
        assert_eq!(
            caps.embedding_model.base.id,
            "custom/text-embedding-3-small"
        );
        assert_eq!(
            caps.defaults.embedding_default_model,
            "custom/text-embedding-3-small"
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
    async fn custom_embedding_provider_selection_preserves_embedding_config() {
        let temp = tempfile::tempdir().unwrap();
        let providers_dir = temp.path().join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(
            providers_dir.join("custom.yaml"),
            r#"
enabled: true
api_key: sk-embed
embedding_endpoint: https://embedding.example/v1/embeddings
embedding_endpoint_style: openai
extra_headers:
  X-Embedding-Tenant: tenant-a
embedding_model:
  name: upstream-embed-name
  embedding_size: 384
  dimensions: 384
  query_prefix: "query: "
  document_prefix: "passage: "
  embedding_batch: 4
"#,
        )
        .await
        .unwrap();
        tokio::fs::write(
            providers_dir.join("custom_2.yaml"),
            r#"
base_provider: custom
enabled: true
api_key: sk-other
embedding_endpoint: https://other.example/v1/embeddings
embedding_model:
  name: other-embed
  embedding_size: 999
"#,
        )
        .await
        .unwrap();

        let (mut providers, errors) = read_providers_d(Vec::new(), temp.path(), false).await;
        assert!(errors.is_empty(), "{}", errors.len());
        providers.sort_by(|a, b| a.name.cmp(&b.name));
        for provider in &mut providers {
            post_process_provider(provider, false, false);
        }
        let mut caps = CodeAssistantCaps::default();
        let embedding_models = add_models_to_caps(&mut caps, providers);

        assert_eq!(embedding_models.len(), 2);
        let selected = embedding_models
            .iter()
            .find(|model| model.base.id == "custom/upstream-embed-name")
            .unwrap();
        assert_eq!(
            selected.base.endpoint,
            "https://embedding.example/v1/embeddings"
        );
        assert_eq!(selected.base.name, "upstream-embed-name");
        assert_eq!(selected.base.api_key, "sk-embed");
        assert_eq!(selected.base.embedding_endpoint_style, "openai");
        assert_eq!(
            selected
                .base
                .extra_headers
                .get("X-Embedding-Tenant")
                .map(String::as_str),
            Some("tenant-a")
        );
        assert_eq!(selected.dimensions, Some(384));
        assert_eq!(selected.query_prefix, "query: ");
        assert_eq!(selected.document_prefix, "passage: ");
        let selected_config = refact_core::vecdb_types::EmbeddingModelConfig::from(selected);
        assert_eq!(selected_config.model_name, "upstream-embed-name");
        assert_eq!(
            selected_config.endpoint,
            "https://embedding.example/v1/embeddings"
        );
        assert_eq!(selected_config.api_key, "sk-embed");
        assert_eq!(
            selected_config
                .extra_headers
                .get("X-Embedding-Tenant")
                .map(String::as_str),
            Some("tenant-a")
        );
    }

    #[tokio::test]
    async fn custom_completion_provider_config_reaches_caps_and_default_resolution() {
        let temp = tempfile::tempdir().unwrap();
        let providers_dir = temp.path().join("providers.d");
        tokio::fs::create_dir_all(&providers_dir).await.unwrap();
        tokio::fs::write(
            providers_dir.join("custom.yaml"),
            r#"
enabled: true
api_key: sk-test
completion_endpoint: http://localhost:1234/v1/completions
completion_endpoint_style: openai_completions
completion_default_model: upstream-coder
enabled_models:
  - upstream-coder
extra_headers:
  X-Proxy-Token: secret-token
completion_models:
  upstream-coder:
    n_ctx: 4096
    scratchpad: plain
"#,
        )
        .await
        .unwrap();

        let provider =
            get_provider_from_template_and_config_file(temp.path(), "custom", true, true, false)
                .await
                .unwrap();
        let mut caps = CodeAssistantCaps::default();
        add_models_to_caps(&mut caps, vec![provider]);

        assert_eq!(
            caps.defaults.completion_default_model,
            "custom/upstream-coder"
        );
        let model = caps.completion_models.get("custom/upstream-coder").unwrap();
        assert_eq!(model.base.id, "custom/upstream-coder");
        assert_eq!(model.base.name, "upstream-coder");
        assert_eq!(model.base.endpoint, "http://localhost:1234/v1/completions");
        assert_eq!(model.base.completion_endpoint_style, "openai_completions");
        assert_eq!(model.scratchpad, "plain");
        assert_eq!(
            model
                .base
                .extra_headers
                .get("X-Proxy-Token")
                .map(String::as_str),
            Some("secret-token")
        );
        let resolved = resolve_completion_model(Arc::new(caps), "").unwrap();
        assert_eq!(resolved.base.id, "custom/upstream-coder");
    }

    #[test]
    fn provider_cache_control_false_disables_placeholder_chat_model() {
        let mut provider = CapsProvider {
            name: "anthropic_proxy".to_string(),
            base_provider: "anthropic".to_string(),
            supports_cache_control: false,
            running_models: vec!["claude-proxy".to_string()],
            ..Default::default()
        };
        populate_model_records(&mut provider, false);
        post_process_provider(&mut provider, false, false);

        let mut caps = CodeAssistantCaps::default();
        add_models_to_caps(&mut caps, vec![provider]);

        let model = caps
            .chat_models
            .get("anthropic_proxy/claude-proxy")
            .unwrap();
        assert!(!model.base.supports_cache_control);
    }

    #[test]
    fn provider_cache_control_false_disables_explicit_endpoint_chat_model() {
        let mut provider = CapsProvider {
            name: "anthropic_proxy".to_string(),
            base_provider: "anthropic".to_string(),
            supports_cache_control: false,
            chat_endpoint: "https://proxy.example/v1/messages".to_string(),
            chat_models: IndexMap::from([(
                "claude-proxy".to_string(),
                ChatModelRecord {
                    base: BaseModelRecord {
                        id: "anthropic_proxy/claude-proxy".to_string(),
                        name: "claude-proxy".to_string(),
                        endpoint: "https://proxy.example/v1/messages".to_string(),
                        supports_cache_control: true,
                        enabled: true,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        post_process_provider(&mut provider, false, false);

        let mut caps = CodeAssistantCaps::default();
        add_models_to_caps(&mut caps, vec![provider]);

        let model = caps
            .chat_models
            .get("anthropic_proxy/claude-proxy")
            .unwrap();
        assert_eq!(model.base.endpoint, "https://proxy.example/v1/messages");
        assert!(!model.base.supports_cache_control);
    }

    #[test]
    fn model_cache_control_false_stays_false_when_provider_supports_it() {
        let provider = CapsProvider {
            name: "anthropic".to_string(),
            base_provider: "anthropic".to_string(),
            supports_cache_control: true,
            chat_endpoint: "https://api.anthropic.com/v1/messages".to_string(),
            chat_models: IndexMap::from([(
                "claude".to_string(),
                ChatModelRecord {
                    base: BaseModelRecord {
                        id: "anthropic/claude".to_string(),
                        name: "claude".to_string(),
                        supports_cache_control: false,
                        enabled: true,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };

        let mut caps = CodeAssistantCaps::default();
        add_models_to_caps(&mut caps, vec![provider]);

        let model = caps.chat_models.get("anthropic/claude").unwrap();
        assert!(!model.base.supports_cache_control);
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
            embedding_default_model: String::new(),
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
