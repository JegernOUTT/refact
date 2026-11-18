use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use indexmap::IndexMap;

use crate::caps::{BaseModelRecord, ChatModelRecord, CodeAssistantCaps};
use crate::custom_error::YamlError;

#[cfg(test)]
use refact_core::llm_types::WireFormat;
use refact_providers::identity::provider_identity_from_yaml;

pub use refact_caps_core::provider_config::{
    CapsProvider, default_endpoint_style, extend_collection, extend_model_collection,
    set_field_if_exists,
};

const LITELLM_PROVIDER_TEMPLATE: &str = r#"
wire_format: openai_chat_completions
"#;

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
    ("litellm", LITELLM_PROVIDER_TEMPLATE),
    (
        "minimax",
        include_str!("../yaml_configs/default_providers/minimax.yaml"),
    ),
    (
        "ollama",
        include_str!("../yaml_configs/default_providers/ollama.yaml"),
    ),
    (
        "opencode",
        include_str!("../yaml_configs/default_providers/opencode.yaml"),
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
    let catalog_models: std::collections::HashSet<String> =
        provider.running_models.iter().cloned().collect();
    add_running_models(provider);
    populate_model_records(provider, experimental, Some(&catalog_models));
    apply_models_dict_patch(provider);
    add_name_and_id_to_model_records(provider);
    if !include_disabled_models {
        provider.chat_models.retain(|_, model| model.base.enabled);
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

        let supports_command_credential =
            matches!(identity.base_provider.as_str(), "custom" | "litellm");
        if !supports_command_credential && config_file_value.get("credential").is_some() {
            error_log.push(YamlError {
                path: yaml_path.to_string_lossy().to_string(),
                error_line: 0,
                error_msg:
                    "command credentials are supported only for custom and litellm providers"
                        .to_string(),
            });
            continue;
        }

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

        if provider.credential.is_some()
            && !matches!(provider.base_provider.as_str(), "custom" | "litellm")
        {
            error_log.push(YamlError {
                path: yaml_path.to_string_lossy().to_string(),
                error_line: 0,
                error_msg:
                    "command credentials are supported only for custom and litellm providers"
                        .to_string(),
            });
            continue;
        }
        if let Err(error_msg) = provider.validate_credential_config() {
            error_log.push(YamlError {
                path: yaml_path.to_string_lossy().to_string(),
                error_line: 0,
                error_msg,
            });
            continue;
        }

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

pub fn add_models_to_caps(caps: &mut CodeAssistantCaps, providers: Vec<CapsProvider>) {
    fn add_provider_details_to_model(
        base_model_rec: &mut BaseModelRecord,
        provider: &CapsProvider,
        model_name: &str,
        endpoint: &str,
    ) {
        base_model_rec.endpoint = endpoint.replace("$MODEL", model_name);
        base_model_rec.endpoint_style = provider.endpoint_style.clone();
        base_model_rec.wire_format = provider.wire_format;
        base_model_rec.supports_cache_control =
            base_model_rec.supports_cache_control && provider.supports_cache_control;
    }

    fn inherit_provider_auth(base_model_rec: &mut BaseModelRecord, provider: &CapsProvider) {
        if base_model_rec.api_key.is_empty() && base_model_rec.credential.is_none() {
            base_model_rec.api_key = provider.api_key.clone();
            base_model_rec.credential = provider.credential.clone();
        }
        if base_model_rec.tokenizer_api_key.is_empty() {
            base_model_rec.tokenizer_api_key = provider.tokenizer_api_key.clone();
        }
        let model_headers = std::mem::take(&mut base_model_rec.extra_headers);
        base_model_rec.extra_headers = provider.extra_headers.clone();
        base_model_rec.extra_headers.extend(model_headers);
    }

    for mut provider in providers {
        let chat_models = std::mem::take(&mut provider.chat_models);
        for (model_name, mut model_rec) in chat_models {
            inherit_provider_auth(&mut model_rec.base, &provider);
            model_rec.base.supports_cache_control =
                model_rec.base.supports_cache_control && provider.supports_cache_control;
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

        caps.defaults
            .apply_override(&provider.defaults(), Some(&provider.name));
    }
}

fn add_name_and_id_to_model_records(provider: &mut CapsProvider) {
    for (model_name, model_rec) in &mut provider.chat_models {
        model_rec.base.name = model_name.to_string();
        model_rec.base.id = format!("{}/{}", provider.name, model_name);
    }
}

fn apply_models_dict_patch(provider: &mut CapsProvider) {
    for (model_name, rec_patched) in provider.models_dict_patch.iter() {
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

fn populate_model_records(
    provider: &mut CapsProvider,
    _experimental: bool,
    catalog_models: Option<&std::collections::HashSet<String>>,
) {
    let had_declared_chat_models = !provider.chat_models.is_empty();

    for model_name in &provider.running_models {
        if !provider.chat_models.contains_key(model_name) {
            let backfilled_default = catalog_models
                .map(|catalog| !catalog.contains(model_name))
                .unwrap_or(false);
            if backfilled_default && had_declared_chat_models {
                tracing::warn!(
                    "provider {}: configured default model '{}' is not in the provider's model list, leaving it unresolved",
                    provider.name,
                    model_name
                );
                continue;
            }
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
    async fn providers_d_rejects_invalid_and_dual_litellm_and_unsupported_credentials() {
        let temp = tempfile::tempdir().unwrap();
        for (name, yaml) in [
            (
                "invalid_litellm.yaml",
                "base_provider: litellm\ncredential:\n  type: command\n  command: ''\n",
            ),
            (
                "dual_litellm.yaml",
                "base_provider: litellm\napi_key: secret\ncredential:\n  type: command\n  command: helper\n",
            ),
            (
                "openai_command.yaml",
                "base_provider: openai\ncredential:\n  type: command\n  command: helper\n",
            ),
        ] {
            write_provider_config(&temp, name, yaml).await;
        }

        let (providers, errors) = read_providers_d(Vec::new(), temp.path(), false).await;

        assert!(providers.is_empty());
        assert_eq!(errors.len(), 3);
        let messages: Vec<&str> = errors
            .iter()
            .map(|error| error.error_msg.as_str())
            .collect();
        assert!(messages
            .iter()
            .any(|message| message.contains("must not be empty")));
        assert!(messages
            .iter()
            .any(|message| message.contains("mutually exclusive")));
        assert!(messages.iter().any(|message| *message
            == "command credentials are supported only for custom and litellm providers"));
    }
}
