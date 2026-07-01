use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use indexmap::IndexMap;
use tracing::{info, warn};

use crate::global_context::GlobalContext;
use crate::caps::providers::{
    add_models_to_caps, post_process_provider, read_providers_d, resolve_provider_api_key,
    CapsProvider,
};
use refact_core::provider_types::{ModelTypeDefaults, ProviderDefaults, is_legacy_refact_model};
use crate::caps::model_caps::{
    get_model_caps, model_caps_pricing_metadata, resolve_model_caps, ModelCapabilities,
};
use refact_core::provider_types::AvailableModel;

const PROVIDER_MODEL_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(8);

pub use refact_core::llm_types::{
    BaseModelRecord, EmbeddingModelRecord, HasBaseModelRecord, WireFormat, default_embedding_batch,
    default_rejection_threshold, default_true,
};

pub use refact_caps_core::model_records::{
    CapsMetadata, ChatModelRecord, CompletionModelFamily, CompletionModelRecord, DefaultModels,
    default_chat_scratchpad, default_completion_scratchpad, default_completion_scratchpad_patch,
    default_hf_tokenizer_template, default_pricing, normalize_string,
};

pub use refact_caps_core::code_assistant_caps::CodeAssistantCaps;

fn resolve_model_caps_for_provider_model(
    model_caps: &HashMap<String, ModelCapabilities>,
    model_id: &str,
) -> Option<crate::caps::model_caps::ResolvedCaps> {
    resolve_model_caps_for_provider_model_with_bases(model_caps, model_id, &[])
}

fn resolve_model_caps_for_provider_model_with_bases(
    model_caps: &HashMap<String, ModelCapabilities>,
    model_id: &str,
    base_provider_names: &[String],
) -> Option<crate::caps::model_caps::ResolvedCaps> {
    if let Some(resolved) = resolve_model_caps(model_caps, model_id) {
        return Some(resolved);
    }

    let Some((provider_name, bare_model_id)) = model_id.split_once('/') else {
        return None;
    };

    let mut aliases = model_caps_provider_aliases(provider_name);
    for base_provider_name in base_provider_names {
        aliases.extend(model_caps_provider_aliases(base_provider_name));
    }
    aliases.sort();
    aliases.dedup();

    for provider_alias in aliases {
        let qualified = format!("{provider_alias}/{bare_model_id}");
        if let Some(resolved) = resolve_model_caps(model_caps, &qualified) {
            return Some(resolved);
        }
    }

    None
}

fn model_caps_provider_aliases(provider_name: &str) -> Vec<String> {
    let mut aliases = vec![provider_name.replace('_', "-")];
    for suffix in ["_responses", "-responses"] {
        if let Some(stripped) = provider_name.strip_suffix(suffix) {
            aliases.push(stripped.to_string());
            aliases.push(stripped.replace('_', "-"));
        }
    }
    if provider_name == "google_gemini" {
        aliases.push("google".to_string());
    }
    aliases.sort();
    aliases.dedup();
    aliases
}

/// Build ChatModelRecord from an AvailableModel and provider runtime info
fn build_chat_model_record(
    provider_name: &str,
    base_provider_names: &[String],
    model: &AvailableModel,
    model_caps: &HashMap<String, ModelCapabilities>,
    runtime_wire_format: WireFormat,
    runtime_endpoint: &str,
    runtime_api_key: &str,
    runtime_auth_token: &str,
    runtime_tokenizer_api_key: &str,
    runtime_extra_headers: &HashMap<String, String>,
    runtime_supports_cache_control: bool,
) -> ChatModelRecord {
    let prefix = format!("{}/", provider_name);
    let model_id = if model.id.starts_with(&prefix) {
        model.id.clone()
    } else {
        format!("{}/{}", provider_name, model.id)
    };

    let resolved_caps = if provider_name == "vllm" {
        model
            .base_model
            .as_deref()
            .map(str::trim)
            .filter(|base_model| !base_model.is_empty())
            .and_then(|base_model| resolve_model_caps(model_caps, base_model))
    } else {
        None
    }
    .or_else(|| {
        resolve_model_caps_for_provider_model_with_bases(model_caps, &model_id, base_provider_names)
    })
    .or_else(|| {
        if model_id.starts_with("openrouter/") {
            None
        } else {
            resolve_model_caps(model_caps, &model.id)
        }
    });

    let (
        n_ctx,
        supports_tools,
        supports_multimodality,
        reasoning_effort_options,
        supports_thinking_budget,
        supports_adaptive_thinking_budget,
        tokenizer,
        supports_clicks,
        max_output_tokens,
        supports_parallel_tools,
        supports_strict_tools,
    ) = if let Some(ref resolved) = resolved_caps {
        let caps = &resolved.caps;
        if model.is_custom {
            let clamped_n_ctx = if caps.n_ctx > 0 {
                model.n_ctx.min(caps.n_ctx)
            } else {
                model.n_ctx
            };
            let clamped_max_output = model.max_output_tokens.map(|v| {
                if caps.max_output_tokens > 0 {
                    v.min(caps.max_output_tokens)
                } else {
                    v
                }
            });
            let tok = model
                .tokenizer
                .clone()
                .unwrap_or_else(|| caps.tokenizer.clone());
            (
                clamped_n_ctx,
                model.supports_tools,
                model.supports_multimodality,
                model.reasoning_effort_options.clone(),
                model.supports_thinking_budget,
                model.supports_adaptive_thinking_budget,
                tok,
                caps.supports_clicks,
                clamped_max_output,
                model.supports_parallel_tools,
                model.supports_strict_tools,
            )
        } else {
            let effective_n_ctx = if model.n_ctx > 0 && caps.n_ctx > 0 {
                model.n_ctx.min(caps.n_ctx)
            } else if caps.n_ctx > 0 {
                caps.n_ctx
            } else {
                model.n_ctx
            };
            let effective_max_output = if caps.max_output_tokens > 0 {
                model
                    .max_output_tokens
                    .map(|v| v.min(caps.max_output_tokens))
                    .or(Some(caps.max_output_tokens))
            } else {
                model.max_output_tokens
            };
            (
                effective_n_ctx,
                caps.supports_tools,
                caps.supports_vision
                    || caps.supports_video
                    || caps.supports_audio
                    || caps.supports_pdf,
                caps.reasoning_effort_options.clone(),
                caps.supports_thinking_budget,
                caps.supports_adaptive_thinking_budget,
                model
                    .tokenizer
                    .clone()
                    .unwrap_or_else(|| caps.tokenizer.clone()),
                caps.supports_clicks,
                effective_max_output,
                caps.supports_parallel_tools,
                caps.supports_strict_tools,
            )
        }
    } else {
        // No registry entry for this model: trust whatever the provider reported.
        // supports_clicks defaults to false because click support is a UI-level
        // capability that no local provider currently reports.
        (
            model.n_ctx,
            model.supports_tools,
            model.supports_multimodality,
            model.reasoning_effort_options.clone(),
            model.supports_thinking_budget,
            model.supports_adaptive_thinking_budget,
            model.tokenizer.clone().unwrap_or_default(),
            false,
            model.max_output_tokens,
            model.supports_parallel_tools,
            model.supports_strict_tools,
        )
    };

    let supports_agent = supports_tools;
    let supports_cache_control = model.supports_cache_control
        || resolved_caps
            .as_ref()
            .is_some_and(|resolved| resolved.caps.supports_cache_control);
    let effective_wire_format = model.wire_format_override.unwrap_or(runtime_wire_format);
    let effective_endpoint = model
        .endpoint_override
        .as_deref()
        .unwrap_or(runtime_endpoint);
    let endpoint = effective_endpoint.replace("$MODEL", &model.id);

    let endpoint_style = match effective_wire_format {
        WireFormat::AnthropicMessages => "anthropic",
        _ => "openai",
    }
    .to_string();
    let tokenizer = if tokenizer.trim().is_empty() {
        base_provider_names
            .iter()
            .find_map(|base_provider| {
                refact_core::model_caps::predefined_cloud_tokenizer_for_model(
                    base_provider,
                    &model_id,
                )
            })
            .or_else(|| {
                refact_core::model_caps::predefined_cloud_tokenizer_for_model(
                    provider_name,
                    &model_id,
                )
            })
            .unwrap_or("fake")
            .to_string()
    } else {
        refact_core::model_caps::normalize_tokenizer_or_default(
            provider_name,
            &model_id,
            &tokenizer,
        )
    };

    ChatModelRecord {
        base: BaseModelRecord {
            n_ctx,
            name: model.id.clone(),
            id: model_id,
            endpoint,
            endpoint_style,
            completion_endpoint_style: String::new(),
            embedding_endpoint_style: String::new(),
            wire_format: effective_wire_format,
            api_key: runtime_api_key.to_string(),
            auth_token: runtime_auth_token.to_string(),
            tokenizer_api_key: runtime_tokenizer_api_key.to_string(),
            extra_headers: runtime_extra_headers.clone(),
            similar_models: Vec::new(),
            tokenizer,
            enabled: model.enabled,
            experimental: false,
            supports_max_completion_tokens: resolved_caps
                .as_ref()
                .map(|r| r.caps.supports_max_completion_tokens)
                .unwrap_or(false),
            eof_is_done: false,
            supports_web_search: resolved_caps
                .as_ref()
                .map(|r| r.caps.supports_web_search)
                .unwrap_or(false),
            supports_cache_control: runtime_supports_cache_control && supports_cache_control,
            removable: model.is_custom,
            user_configured: model.is_custom,
        },
        scratchpad: String::new(),
        scratchpad_patch: serde_json::Value::Null,
        supports_tools,
        supports_multimodality,
        supports_clicks,
        supports_agent,
        reasoning_effort_options,
        supports_thinking_budget,
        supports_adaptive_thinking_budget,
        max_thinking_tokens: resolved_caps
            .as_ref()
            .and_then(|r| r.caps.max_thinking_tokens),
        default_temperature: resolved_caps
            .as_ref()
            .and_then(|r| r.caps.default_temperature),
        default_frequency_penalty: None,
        default_max_tokens: resolved_caps
            .as_ref()
            .and_then(|r| r.caps.default_max_tokens),
        max_output_tokens,
        supports_parallel_tools,
        supports_strict_tools: resolved_caps
            .as_ref()
            .map(|r| {
                if model.is_custom {
                    supports_strict_tools
                } else {
                    r.caps.supports_strict_tools
                }
            })
            .unwrap_or(supports_strict_tools),
        supports_temperature: resolved_caps
            .as_ref()
            .map(|r| r.caps.supports_temperature)
            .unwrap_or(true),
        available_providers: model.available_providers.clone(),
        selected_provider: model.selected_provider.clone(),
    }
}

pub async fn populate_chat_models_from_providers(
    caps: &mut CodeAssistantCaps,
    gcx: Arc<GlobalContext>,
) {
    let model_caps = &*caps.model_caps;

    let (http_client, providers_snapshot) = {
        let registry = gcx.providers.read().await;
        let snapshot: Vec<Box<dyn crate::providers::traits::ProviderTrait>> =
            registry.iter().map(|(_, p)| p.clone_box()).collect();
        (gcx.http_client.clone(), snapshot)
    };

    let mut pricing_map = caps.metadata.pricing.as_object_mut();

    for provider in &providers_snapshot {
        let runtime = match provider.build_runtime() {
            Ok(r) => r,
            Err(e) => {
                warn!(
                    "Failed to build runtime for provider '{}': {}",
                    provider.name(),
                    e
                );
                continue;
            }
        };

        if !runtime.enabled {
            continue;
        }

        let mut base_provider_names = vec![provider.base_provider_name().to_string()];
        if runtime.name != provider.name() {
            base_provider_names.push(provider.name().to_string());
        }
        base_provider_names.sort();
        base_provider_names.dedup();

        let available_models = match tokio::time::timeout(
            PROVIDER_MODEL_DISCOVERY_TIMEOUT,
            provider.fetch_available_models(&http_client, model_caps),
        )
        .await
        {
            Ok(models) => models,
            Err(_) => {
                warn!(
                    "Timed out fetching available models for provider '{}'; using model_caps fallback",
                    provider.name()
                );
                provider.get_available_models_from_caps(model_caps)
            }
        };

        for model in available_models {
            if !model.enabled {
                continue;
            }

            let chat_record = build_chat_model_record(
                &runtime.name,
                &base_provider_names,
                &model,
                model_caps,
                runtime.wire_format,
                &runtime.chat_endpoint,
                &runtime.api_key,
                &runtime.auth_token,
                &runtime.tokenizer_api_key,
                &runtime.extra_headers,
                runtime.supports_cache_control,
            );

            let model_id = chat_record.base.id.clone();

            if let Some(ref pricing) = model.pricing {
                if let Some(map) = pricing_map.as_mut() {
                    if let Ok(pricing_value) = serde_json::to_value(pricing) {
                        map.insert(model_id.clone(), pricing_value.clone());
                        if !map.contains_key(&model.id) {
                            map.insert(model.id.clone(), pricing_value);
                        }
                    }
                }
            }

            let map = caps
                .provider_base_names
                .entry(runtime.name.clone())
                .or_default();
            map.extend(base_provider_names.iter().cloned());
            map.sort();
            map.dedup();

            caps.chat_models.insert(model_id, Arc::new(chat_record));
        }
    }

    // Chat default model slots are intentionally not auto-filled. The Default Models UI can
    // leave each slot unset, and tools that require a model type report a setup error instead
    // of silently falling back to another model type.

    if !caps.completion_models.is_empty() {
        let need_new_default = caps.defaults.completion_default_model.is_empty()
            || !caps
                .completion_models
                .contains_key(&caps.defaults.completion_default_model);

        if need_new_default {
            let mut candidates: Vec<&String> = caps.completion_models.keys().collect();
            candidates.sort();
            if let Some(first_model_id) = candidates.first() {
                info!(
                    "Auto-selecting default completion model: {}",
                    first_model_id
                );
                caps.defaults.completion_default_model = (*first_model_id).clone();
            }
        }
    }
}

fn resolve_user_default_chat_model(
    model: &str,
    chat_models: &IndexMap<String, Arc<ChatModelRecord>>,
) -> Option<String> {
    if model.is_empty() {
        return None;
    }
    if chat_models.contains_key(model) {
        return Some(model.to_string());
    }
    if !model.contains('/') {
        for key in chat_models.keys() {
            if let Some(name) = key.split('/').last() {
                if name == model {
                    return Some(key.clone());
                }
            }
        }
    }
    None
}

fn apply_user_default_chat_model(
    target: &mut String,
    defaults: &ModelTypeDefaults,
    label: &str,
    chat_models: &IndexMap<String, Arc<ChatModelRecord>>,
) {
    let Some(model) = defaults.model.as_deref() else {
        return;
    };

    let model = model.trim();
    if model.is_empty() {
        target.clear();
        return;
    }

    if is_legacy_refact_model(model) {
        warn!(
            "Legacy Refact Cloud {} default '{}' was reset to none",
            label, model
        );
        target.clear();
        return;
    }

    match resolve_user_default_chat_model(model, chat_models) {
        Some(resolved) => *target = resolved,
        None => {
            warn!(
                "User default {} model '{}' not found in available models; keeping configured value for setup diagnostics",
                label, model
            );
            *target = model.to_string();
        }
    }
}

fn resolve_user_default_completion_model(
    model: &str,
    completion_models: &IndexMap<String, Arc<CompletionModelRecord>>,
) -> Option<String> {
    if model.is_empty() {
        return None;
    }
    if completion_models.contains_key(model) {
        return Some(model.to_string());
    }
    if !model.contains('/') {
        for key in completion_models.keys() {
            if let Some(name) = key.split('/').last() {
                if name == model {
                    return Some(key.clone());
                }
            }
        }
    }
    None
}

fn apply_user_default_completion_model(
    target: &mut String,
    model: Option<&str>,
    completion_models: &IndexMap<String, Arc<CompletionModelRecord>>,
) {
    let Some(model) = model else {
        return;
    };

    let model = model.trim();
    if model.is_empty() {
        target.clear();
        return;
    }

    if is_legacy_refact_model(model) {
        warn!(
            "Legacy Refact Cloud completion default '{}' was reset to none",
            model
        );
        target.clear();
        return;
    }

    match resolve_user_default_completion_model(model, completion_models) {
        Some(resolved) => *target = resolved,
        None => {
            warn!(
                "User default completion model '{}' not found in available models; keeping configured value for setup diagnostics",
                model
            );
            *target = model.to_string();
        }
    }
}

fn resolve_user_default_embedding_model(
    model: &str,
    embedding_models: &[EmbeddingModelRecord],
) -> Option<EmbeddingModelRecord> {
    if model.is_empty() {
        return None;
    }

    let mut matches: Vec<&EmbeddingModelRecord> = embedding_models
        .iter()
        .filter(|record| record.base.id == model || record.base.name == model)
        .collect();
    matches.sort_by(|a, b| a.base.id.cmp(&b.base.id));
    matches.first().map(|record| (*record).clone())
}

fn apply_user_default_embedding_model(
    target: &mut EmbeddingModelRecord,
    model: Option<&str>,
    embedding_models: &[EmbeddingModelRecord],
) {
    let Some(model) = model else {
        return;
    };

    let model = model.trim();
    if model.is_empty() {
        *target = EmbeddingModelRecord::default();
        return;
    }

    if is_legacy_refact_model(model) {
        warn!(
            "Legacy Refact Cloud embedding default '{}' was reset to none",
            model
        );
        *target = EmbeddingModelRecord::default();
        return;
    }

    match resolve_user_default_embedding_model(model, embedding_models) {
        Some(resolved) => *target = resolved,
        None => warn!(
            "User default embedding model '{}' not found in available models; keeping configured embedding model for setup diagnostics",
            model
        ),
    }
}

fn clear_legacy_refact_chat_defaults(caps: &mut CodeAssistantCaps) {
    let defaults = &mut caps.defaults;
    clear_legacy_refact_chat_default("chat", &mut defaults.chat_default_model);
    clear_legacy_refact_chat_default("chat 2", &mut defaults.chat_model_2);
    clear_legacy_refact_chat_default("task planner agent", &mut defaults.task_planner_agent_model);
    clear_legacy_refact_chat_default("light", &mut defaults.chat_light_model);
    clear_legacy_refact_chat_default("thinking", &mut defaults.chat_thinking_model);
    clear_legacy_refact_chat_default("buddy", &mut defaults.chat_buddy_model);
}

fn clear_legacy_refact_chat_default(label: &str, value: &mut String) {
    if is_legacy_refact_model(value) {
        warn!(
            "Legacy Refact Cloud {} default '{}' was reset to none",
            label, value
        );
        value.clear();
    }
}

fn remove_legacy_refact_models_from_caps(caps: &mut CodeAssistantCaps) {
    caps.chat_models.retain(|model_id, _| {
        let keep = !is_legacy_refact_model(model_id);
        if !keep {
            warn!(
                "Legacy Refact Cloud chat model '{}' was removed from caps",
                model_id
            );
        }
        keep
    });

    caps.completion_models.retain(|model_id, _| {
        let keep = !is_legacy_refact_model(model_id);
        if !keep {
            warn!(
                "Legacy Refact Cloud completion model '{}' was removed from caps",
                model_id
            );
        }
        keep
    });

    if is_legacy_refact_model(&caps.embedding_model.base.id)
        || is_legacy_refact_model(&caps.embedding_model.base.name)
    {
        warn!(
            "Legacy Refact Cloud embedding model '{}' was reset to none",
            caps.embedding_model.base.id
        );
        caps.embedding_model = EmbeddingModelRecord::default();
    }

    clear_legacy_refact_chat_defaults(caps);

    if is_legacy_refact_model(&caps.defaults.completion_default_model)
        || (!caps.defaults.completion_default_model.is_empty()
            && !caps
                .completion_models
                .contains_key(&caps.defaults.completion_default_model))
    {
        warn!(
            "Completion default model '{}' was reset to none because it is no longer available",
            caps.defaults.completion_default_model
        );
        caps.defaults.completion_default_model.clear();
    }

    if caps.defaults.completion_default_model.is_empty() && !caps.completion_models.is_empty() {
        let mut candidates: Vec<&String> = caps.completion_models.keys().collect();
        candidates.sort();
        if let Some(first_model_id) = candidates.first() {
            info!(
                "Auto-selecting default completion model after legacy cleanup: {}",
                first_model_id
            );
            caps.defaults.completion_default_model = (*first_model_id).clone();
        }
    }
}

async fn take_models_dev_startup_refresh_flag(gcx: Arc<GlobalContext>) -> bool {
    let caps_state = gcx.caps_state.clone();
    let mut caps_state = caps_state.write().await;
    if caps_state.models_dev_startup_refresh_attempted {
        false
    } else {
        caps_state.models_dev_startup_refresh_attempted = true;
        true
    }
}

pub async fn load_caps(
    _cmdline: crate::global_context::CommandLine,
    gcx: Arc<GlobalContext>,
) -> Result<Arc<CodeAssistantCaps>, String> {
    let (config_dir, cmdline_api_key, experimental) = {
        (
            gcx.config_dir.clone(),
            String::new(),
            gcx.cmdline.experimental,
        )
    };

    let mut caps = CodeAssistantCaps::default();
    let server_providers = Vec::new();

    let (mut providers, error_log): (Vec<CapsProvider>, Vec<_>) =
        read_providers_d(server_providers, &config_dir, experimental).await;
    providers.retain(|p| p.enabled);
    for e in error_log {
        tracing::error!("{e}");
    }
    for provider in &mut providers {
        post_process_provider(provider, false, experimental);
        provider.api_key = resolve_provider_api_key(&provider, &cmdline_api_key);
        if !provider.base_provider.is_empty() && provider.base_provider != provider.name {
            caps.provider_base_names
                .entry(provider.name.clone())
                .or_default()
                .push(provider.base_provider.clone());
        }
    }

    let force_models_dev_refresh = take_models_dev_startup_refresh_flag(gcx.clone()).await;
    if force_models_dev_refresh {
        info!("Refreshing models.dev catalog on engine startup");
    }
    let model_caps_map = get_model_caps(gcx.clone(), force_models_dev_refresh).await.map_err(|e| {
        format!("Failed to load models.dev capabilities. Check the bundled snapshot or runtime cache: {e}")
    })?;
    caps.metadata.pricing = model_caps_pricing_metadata(&model_caps_map);
    caps.metadata
        .features
        .push("models_dev_base_text_pricing".to_string());
    caps.model_caps = Arc::new(model_caps_map);

    // Clear chat models from legacy CapsProviders that have a new ProviderTrait implementation.
    // The new system (populate_chat_models_from_providers) is the sole source of truth for
    // chat models — it respects enabled_models selection. Legacy running_models from YAML
    // templates would otherwise bypass model selection, showing all template models.
    // Only chat_models are cleared; completion_models and embedding_model are preserved
    // since the new system doesn't handle those yet.
    {
        let registry = gcx.providers.read().await;
        for p in &mut providers {
            if registry.get(&p.name).is_some() {
                p.chat_models.clear();
            }
        }
    }

    let embedding_models = add_models_to_caps(&mut caps, providers);
    populate_chat_models_from_providers(&mut caps, gcx.clone()).await;
    apply_model_caps_to_all_chat_models(&mut caps);
    remove_legacy_refact_models_from_caps(&mut caps);

    match ProviderDefaults::load(&config_dir).await {
        Ok(user_defaults) => {
            apply_user_default_chat_model(
                &mut caps.defaults.chat_default_model,
                &user_defaults.chat,
                "chat",
                &caps.chat_models,
            );
            apply_user_default_chat_model(
                &mut caps.defaults.chat_model_2,
                &user_defaults.chat_model_2,
                "chat 2",
                &caps.chat_models,
            );
            apply_user_default_chat_model(
                &mut caps.defaults.task_planner_agent_model,
                &user_defaults.task_planner_agent_model,
                "task planner agent",
                &caps.chat_models,
            );
            apply_user_default_chat_model(
                &mut caps.defaults.chat_light_model,
                &user_defaults.chat_light,
                "light",
                &caps.chat_models,
            );
            apply_user_default_chat_model(
                &mut caps.defaults.chat_buddy_model,
                &user_defaults.chat_buddy,
                "buddy",
                &caps.chat_models,
            );
            apply_user_default_chat_model(
                &mut caps.defaults.chat_thinking_model,
                &user_defaults.chat_thinking,
                "thinking",
                &caps.chat_models,
            );
            apply_user_default_completion_model(
                &mut caps.defaults.completion_default_model,
                user_defaults.completion_model.as_deref(),
                &caps.completion_models,
            );
            apply_user_default_embedding_model(
                &mut caps.embedding_model,
                user_defaults.embedding_model.as_deref(),
                &embedding_models,
            );
            caps.user_defaults = user_defaults;
        }
        Err(e) => {
            warn!(
                "Failed to load user defaults from providers.d/defaults.yaml: {}",
                e
            );
        }
    }

    validate_default_models(&caps)?;

    Ok(Arc::new(caps))
}

fn validate_default_models(caps: &CodeAssistantCaps) -> Result<(), String> {
    validate_default_chat_model(caps, "chat", &caps.defaults.chat_default_model);
    validate_default_chat_model(caps, "chat 2", &caps.defaults.chat_model_2);
    validate_default_chat_model(
        caps,
        "task planner agent",
        &caps.defaults.task_planner_agent_model,
    );
    validate_default_chat_model(caps, "thinking", &caps.defaults.chat_thinking_model);
    validate_default_chat_model(caps, "buddy", &caps.defaults.chat_buddy_model);
    validate_default_chat_model(caps, "light", &caps.defaults.chat_light_model);
    validate_default_completion_model(caps, &caps.defaults.completion_default_model);
    validate_default_embedding_model(caps);
    Ok(())
}

fn validate_default_chat_model(caps: &CodeAssistantCaps, label: &str, model: &str) {
    if model.is_empty() || caps.chat_models.contains_key(model) {
        return;
    }
    if resolve_model_caps_for_provider_model(&caps.model_caps, model).is_none() {
        warn!(
            "Default {} model '{}' is not in chat_models and not found in model capabilities registry",
            label, model
        );
    }
}

fn validate_default_completion_model(caps: &CodeAssistantCaps, model: &str) {
    if model.is_empty() || caps.completion_models.contains_key(model) {
        return;
    }
    warn!(
        "Default completion model '{}' is not in completion_models",
        model
    );
}

fn validate_default_embedding_model(caps: &CodeAssistantCaps) {
    let model = caps.embedding_model.base.id.as_str();
    if model.is_empty() {
        return;
    }
    if is_legacy_refact_model(model) || is_legacy_refact_model(&caps.embedding_model.base.name) {
        warn!(
            "Default embedding model '{}' is legacy or unavailable",
            model
        );
    }
}

pub fn strip_model_from_finetune(model: &str) -> String {
    model.split(":").next().unwrap().to_string()
}

pub fn resolve_model<'a, T>(
    models: &'a IndexMap<String, Arc<T>>,
    model_id: &str,
) -> Result<Arc<T>, String> {
    models
        .get(model_id)
        .or_else(|| models.get(&strip_model_from_finetune(model_id)))
        .cloned()
        .ok_or(format!(
            "Model '{}' not found. Server has the following models: {:?}",
            model_id,
            models.keys()
        ))
}

pub fn resolve_chat_model(
    caps: Arc<CodeAssistantCaps>,
    requested_model_id: &str,
) -> Result<Arc<ChatModelRecord>, String> {
    let model_id = if !requested_model_id.is_empty() {
        requested_model_id
    } else {
        &caps.defaults.chat_default_model
    };

    let base_record = resolve_model(&caps.chat_models, model_id)?;

    let base_provider_names = model_id
        .split_once('/')
        .and_then(|(provider_name, _)| caps.provider_base_names.get(provider_name))
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let resolved = resolve_model_caps_for_provider_model_with_bases(
        &caps.model_caps,
        model_id,
        base_provider_names,
    );

    match resolved {
        Some(resolved_caps) => {
            tracing::debug!(
                "Model '{}' resolved via {:?}, matched key: '{}'",
                model_id,
                resolved_caps.source,
                resolved_caps.matched_key
            );
            let mut effective = (*base_record).clone();
            apply_registry_caps_to_chat_model(&mut effective, &resolved_caps.caps);
            Ok(Arc::new(effective))
        }
        None => {
            // Model not in registry (e.g., custom model) - use base_record as-is
            // The base_record already has capabilities from build_chat_model_record
            tracing::debug!(
                "Model '{}' not in model_caps registry, using configured capabilities",
                model_id
            );
            Ok(base_record)
        }
    }
}

fn apply_model_caps_to_all_chat_models(caps: &mut CodeAssistantCaps) {
    let model_ids: Vec<String> = caps.chat_models.keys().cloned().collect();
    for model_id in model_ids {
        let base_provider_names = model_id
            .split_once('/')
            .and_then(|(provider_name, _)| caps.provider_base_names.get(provider_name))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if let Some(resolved) = resolve_model_caps_for_provider_model_with_bases(
            &caps.model_caps,
            &model_id,
            base_provider_names,
        ) {
            if let Some(record) = caps.chat_models.get(&model_id) {
                let mut updated = (**record).clone();
                apply_registry_caps_to_chat_model(&mut updated, &resolved.caps);
                caps.chat_models.insert(model_id, Arc::new(updated));
            }
        }
    }
}

fn apply_registry_caps_to_chat_model(record: &mut ChatModelRecord, caps: &ModelCapabilities) {
    if record.base.user_configured {
        if caps.n_ctx > 0 {
            record.base.n_ctx = record.base.n_ctx.min(caps.n_ctx);
        }
        if caps.max_output_tokens > 0 {
            record.max_output_tokens = record
                .max_output_tokens
                .map(|v| v.min(caps.max_output_tokens))
                .or(Some(caps.max_output_tokens));
        }
        if record.base.tokenizer.is_empty() && !caps.tokenizer.is_empty() {
            record.base.tokenizer = caps.tokenizer.clone();
        }
        if record.default_temperature.is_none() {
            record.default_temperature = caps.default_temperature;
        }
        if record.default_max_tokens.is_none() {
            record.default_max_tokens = caps.default_max_tokens;
        }
        record.base.supports_max_completion_tokens = caps.supports_max_completion_tokens;
        return;
    }

    if caps.n_ctx > 0 {
        record.base.n_ctx = if record.base.n_ctx > 0 {
            record.base.n_ctx.min(caps.n_ctx)
        } else {
            caps.n_ctx
        };
    }
    record.base.supports_max_completion_tokens = caps.supports_max_completion_tokens;

    // For live provider-discovered models (ollama, vllm, lmstudio), the provider
    // already reported these booleans accurately. The registry should only add
    // capability knowledge the provider omitted, never remove what the provider reported.
    // For cloud/catalog models the registry is authoritative, and build_chat_model_record
    // already set these from registry caps before this point — so ||= is safe for both.
    record.supports_tools = record.supports_tools || caps.supports_tools;
    record.supports_parallel_tools = record.supports_parallel_tools || caps.supports_parallel_tools;
    record.supports_strict_tools = record.supports_strict_tools || caps.supports_strict_tools;
    record.supports_multimodality = record.supports_multimodality
        || caps.supports_vision
        || caps.supports_video
        || caps.supports_audio
        || caps.supports_pdf;
    record.supports_clicks = record.supports_clicks || caps.supports_clicks;
    record.default_temperature = caps.default_temperature;
    record.default_max_tokens = caps.default_max_tokens;
    if caps.max_output_tokens > 0 {
        record.max_output_tokens = record
            .max_output_tokens
            .map(|v| v.min(caps.max_output_tokens))
            .or(Some(caps.max_output_tokens));
    }

    if !caps.tokenizer.is_empty() {
        record.base.tokenizer = caps.tokenizer.clone();
    }

    record.reasoning_effort_options = caps.reasoning_effort_options.clone();
    record.supports_thinking_budget = caps.supports_thinking_budget;
    record.supports_adaptive_thinking_budget = caps.supports_adaptive_thinking_budget;
    record.supports_agent = record.supports_tools;
    record.supports_temperature = caps.supports_temperature;
    record.base.supports_web_search = caps.supports_web_search;
}

pub fn resolve_completion_model<'a>(
    caps: Arc<CodeAssistantCaps>,
    requested_model_id: &str,
) -> Result<Arc<CompletionModelRecord>, String> {
    let model_id = if !requested_model_id.is_empty() {
        requested_model_id
    } else {
        &caps.defaults.completion_default_model
    };

    resolve_model(&caps.completion_models, model_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;

    fn create_test_caps() -> CodeAssistantCaps {
        let mut caps = CodeAssistantCaps::default();

        let test_model = ChatModelRecord {
            base: BaseModelRecord {
                id: "test-provider/test-model".to_string(),
                n_ctx: 8192,
                ..Default::default()
            },
            ..Default::default()
        };

        caps.chat_models
            .insert("test-provider/test-model".to_string(), Arc::new(test_model));

        caps.defaults.chat_default_model = "test-provider/test-model".to_string();

        caps
    }

    #[test]
    fn test_resolve_chat_model_with_explicit_model() {
        let caps = Arc::new(create_test_caps());
        let result = resolve_chat_model(caps, "test-provider/test-model");

        assert!(result.is_ok());
        let model = result.unwrap();
        assert_eq!(model.base.id, "test-provider/test-model");
    }

    #[test]
    fn test_resolve_chat_model_with_empty_string_uses_default() {
        let caps = Arc::new(create_test_caps());
        let result = resolve_chat_model(caps, "");

        assert!(result.is_ok());
        let model = result.unwrap();
        assert_eq!(model.base.id, "test-provider/test-model");
    }

    #[test]
    fn test_resolve_chat_model_with_nonexistent_model() {
        let caps = Arc::new(create_test_caps());
        let result = resolve_chat_model(caps, "nonexistent-provider/nonexistent-model");

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Model"));
    }

    #[test]
    fn test_sorted_model_selection_is_deterministic() {
        let mut caps = CodeAssistantCaps::default();

        let model_z = ChatModelRecord {
            base: BaseModelRecord {
                id: "provider/zzz-model".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        let model_a = ChatModelRecord {
            base: BaseModelRecord {
                id: "provider/aaa-model".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        caps.chat_models
            .insert("provider/zzz-model".to_string(), Arc::new(model_z));
        caps.chat_models
            .insert("provider/aaa-model".to_string(), Arc::new(model_a));

        let mut sorted_model_ids: Vec<&String> = caps.chat_models.keys().collect();
        sorted_model_ids.sort();

        assert_eq!(sorted_model_ids[0], "provider/aaa-model");
        assert_eq!(sorted_model_ids[1], "provider/zzz-model");
    }

    #[test]
    fn test_resolve_model_generic() {
        let mut models = IndexMap::new();
        let test_model = ChatModelRecord {
            base: BaseModelRecord {
                id: "test/model".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        models.insert("test/model".to_string(), Arc::new(test_model));

        let result = resolve_model(&models, "test/model");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().base.id, "test/model");

        let result = resolve_model(&models, "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn user_completion_default_resolves_provider_qualified_and_bare_models() {
        let mut completion_models = IndexMap::new();
        completion_models.insert(
            "openai/starcoder".to_string(),
            Arc::new(CompletionModelRecord {
                base: BaseModelRecord {
                    id: "openai/starcoder".to_string(),
                    name: "starcoder".to_string(),
                    ..Default::default()
                },
                ..Default::default()
            }),
        );

        let mut target = String::new();
        apply_user_default_completion_model(&mut target, Some("starcoder"), &completion_models);
        assert_eq!(target, "openai/starcoder");

        apply_user_default_completion_model(
            &mut target,
            Some("openai/starcoder"),
            &completion_models,
        );
        assert_eq!(target, "openai/starcoder");
    }

    #[test]
    fn user_embedding_default_selects_matching_embedding_record() {
        let embedding_models = vec![
            EmbeddingModelRecord {
                base: BaseModelRecord {
                    id: "openai/text-embedding-3-small".to_string(),
                    name: "text-embedding-3-small".to_string(),
                    ..Default::default()
                },
                embedding_size: 1536,
                ..Default::default()
            },
            EmbeddingModelRecord {
                base: BaseModelRecord {
                    id: "openai_2/text-embedding-3-small".to_string(),
                    name: "text-embedding-3-small".to_string(),
                    ..Default::default()
                },
                embedding_size: 3072,
                ..Default::default()
            },
        ];
        let mut target = EmbeddingModelRecord::default();

        apply_user_default_embedding_model(
            &mut target,
            Some("openai_2/text-embedding-3-small"),
            &embedding_models,
        );
        assert_eq!(target.base.id, "openai_2/text-embedding-3-small");
        assert_eq!(target.embedding_size, 3072);
    }

    #[tokio::test]
    async fn test_models_dev_startup_refresh_flag_is_consumed_once() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let caps_state = gcx.caps_state.clone();
        caps_state
            .write()
            .await
            .models_dev_startup_refresh_attempted = false;

        assert!(take_models_dev_startup_refresh_flag(gcx.clone()).await);
        assert!(!take_models_dev_startup_refresh_flag(gcx).await);
    }

    #[test]
    fn test_instance_prefixed_model_cap_resolution_uses_base_provider() {
        let mut model_caps = HashMap::new();
        model_caps.insert(
            "openai/gpt-4.1".to_string(),
            ModelCapabilities {
                n_ctx: 128_000,
                supports_tools: true,
                supports_strict_tools: true,
                tokenizer: "openai-tokenizer".to_string(),
                ..Default::default()
            },
        );

        let model = AvailableModel {
            id: "gpt-4.1".to_string(),
            display_name: None,
            n_ctx: 0,
            supports_tools: false,
            supports_parallel_tools: false,
            supports_strict_tools: false,
            supports_multimodality: false,
            reasoning_effort_options: None,
            supports_thinking_budget: false,
            supports_adaptive_thinking_budget: false,
            supports_cache_control: true,
            tokenizer: None,
            enabled: true,
            is_custom: false,
            pricing: None,
            available_providers: Vec::new(),
            selected_provider: None,
            max_output_tokens: None,
            provider_variants: Vec::new(),
            wire_format_override: None,
            endpoint_override: None,
            base_model: None,
        };

        let record = build_chat_model_record(
            "openai_2",
            &["openai".to_string()],
            &model,
            &model_caps,
            WireFormat::OpenaiChatCompletions,
            "https://api.openai.com/v1/chat/completions",
            "sk-test",
            "",
            "",
            &HashMap::new(),
            true,
        );

        assert_eq!(record.base.id, "openai_2/gpt-4.1");
        assert_eq!(record.base.n_ctx, 128_000);
        assert_eq!(record.base.tokenizer, "openai-tokenizer");
        assert!(record.supports_tools);
        assert!(record.supports_strict_tools);
    }

    #[test]
    fn test_build_chat_model_record_migrates_missing_tokenizers_to_defaults() {
        let model = AvailableModel {
            id: "gpt-4.1".to_string(),
            display_name: None,
            n_ctx: 128_000,
            supports_tools: false,
            supports_parallel_tools: false,
            supports_strict_tools: false,
            supports_multimodality: false,
            reasoning_effort_options: None,
            supports_thinking_budget: false,
            supports_adaptive_thinking_budget: false,
            supports_cache_control: false,
            tokenizer: None,
            enabled: true,
            is_custom: false,
            pricing: None,
            available_providers: Vec::new(),
            selected_provider: None,
            max_output_tokens: None,
            provider_variants: Vec::new(),
            wire_format_override: None,
            endpoint_override: None,
            base_model: None,
        };

        let openai_record = build_chat_model_record(
            "openai",
            &[],
            &model,
            &HashMap::new(),
            WireFormat::OpenaiChatCompletions,
            "https://api.openai.com/v1/chat/completions",
            "sk-test",
            "",
            "",
            &HashMap::new(),
            true,
        );
        assert_eq!(openai_record.base.tokenizer, "fake");

        let mut instance_model = model.clone();
        instance_model.id = "custom-named-model".to_string();
        let instance_record = build_chat_model_record(
            "openai_2",
            &["openai".to_string()],
            &instance_model,
            &HashMap::new(),
            WireFormat::OpenaiChatCompletions,
            "https://api.openai.com/v1/chat/completions",
            "sk-test",
            "",
            "",
            &HashMap::new(),
            true,
        );
        assert_eq!(instance_record.base.tokenizer, "fake");

        let mut explicit_model = model.clone();
        explicit_model.tokenizer = Some("hf://custom/tokenizer".to_string());
        let mut model_caps = HashMap::new();
        model_caps.insert(
            "openai/gpt-4.1".to_string(),
            ModelCapabilities {
                tokenizer: "fake".to_string(),
                ..Default::default()
            },
        );
        let explicit_record = build_chat_model_record(
            "openai",
            &[],
            &explicit_model,
            &model_caps,
            WireFormat::OpenaiChatCompletions,
            "https://api.openai.com/v1/chat/completions",
            "sk-test",
            "",
            "",
            &HashMap::new(),
            true,
        );
        assert_eq!(explicit_record.base.tokenizer, "hf://custom/tokenizer");

        let mut unknown_model = model;
        unknown_model.id = "unknown-model".to_string();
        let unknown_record = build_chat_model_record(
            "custom_provider",
            &[],
            &unknown_model,
            &HashMap::new(),
            WireFormat::OpenaiChatCompletions,
            "http://localhost/v1/chat/completions",
            "",
            "",
            "",
            &HashMap::new(),
            true,
        );
        assert_eq!(unknown_record.base.tokenizer, "fake");
    }

    #[test]
    fn test_build_chat_model_record_uses_registry_cache_control_with_runtime_gate() {
        let mut model_caps = HashMap::new();
        model_caps.insert(
            "anthropic/claude-sonnet-4".to_string(),
            ModelCapabilities {
                n_ctx: 200_000,
                supports_tools: true,
                supports_cache_control: true,
                tokenizer: "fake".to_string(),
                ..Default::default()
            },
        );

        let model = AvailableModel {
            id: "claude-sonnet-4".to_string(),
            display_name: None,
            n_ctx: 0,
            supports_tools: false,
            supports_parallel_tools: false,
            supports_strict_tools: false,
            supports_multimodality: false,
            reasoning_effort_options: None,
            supports_thinking_budget: false,
            supports_adaptive_thinking_budget: false,
            supports_cache_control: false,
            tokenizer: None,
            enabled: true,
            is_custom: false,
            pricing: None,
            available_providers: Vec::new(),
            selected_provider: None,
            max_output_tokens: None,
            provider_variants: Vec::new(),
            wire_format_override: None,
            endpoint_override: None,
            base_model: None,
        };

        let record = build_chat_model_record(
            "anthropic",
            &["anthropic".to_string()],
            &model,
            &model_caps,
            WireFormat::AnthropicMessages,
            "https://api.anthropic.com/v1/messages",
            "sk-ant",
            "",
            "",
            &HashMap::new(),
            true,
        );

        assert!(record.base.supports_cache_control);

        let unsupported_runtime_record = build_chat_model_record(
            "anthropic_proxy",
            &["anthropic".to_string()],
            &model,
            &model_caps,
            WireFormat::AnthropicMessages,
            "https://proxy.example/v1/messages",
            "sk-ant",
            "",
            "",
            &HashMap::new(),
            false,
        );

        assert!(!unsupported_runtime_record.base.supports_cache_control);
    }

    #[test]
    fn test_vllm_caps_resolution_prefers_base_model_root() {
        let mut model_caps = HashMap::new();
        model_caps.insert(
            "served-alias".to_string(),
            ModelCapabilities {
                n_ctx: 1024,
                supports_tools: false,
                tokenizer: "alias-tokenizer".to_string(),
                ..Default::default()
            },
        );
        model_caps.insert(
            "Qwen/Qwen3.6-27B-FP8".to_string(),
            ModelCapabilities {
                n_ctx: 200_000,
                supports_tools: true,
                tokenizer: "root-tokenizer".to_string(),
                ..Default::default()
            },
        );

        let model = AvailableModel {
            id: "served-alias".to_string(),
            display_name: Some("Served Alias".to_string()),
            n_ctx: 131_072,
            supports_tools: false,
            supports_parallel_tools: false,
            supports_strict_tools: false,
            supports_multimodality: false,
            reasoning_effort_options: None,
            supports_thinking_budget: false,
            supports_adaptive_thinking_budget: false,
            supports_cache_control: true,
            tokenizer: None,
            enabled: true,
            is_custom: false,
            pricing: None,
            available_providers: Vec::new(),
            selected_provider: None,
            max_output_tokens: None,
            provider_variants: Vec::new(),
            wire_format_override: None,
            endpoint_override: None,
            base_model: Some("Qwen/Qwen3.6-27B-FP8".to_string()),
        };

        let record = build_chat_model_record(
            "vllm",
            &[],
            &model,
            &model_caps,
            WireFormat::OpenaiChatCompletions,
            "http://localhost:8000/v1/chat/completions",
            "",
            "",
            "",
            &HashMap::new(),
            false,
        );

        assert_eq!(record.base.id, "vllm/served-alias");
        assert_eq!(record.base.name, "served-alias");
        assert_eq!(record.base.tokenizer, "root-tokenizer");
        assert_eq!(record.base.n_ctx, 131_072);
        assert!(record.supports_tools);
    }

    #[test]
    fn test_build_chat_model_record_uses_models_dev_available_model_runtime_overrides() {
        let mut model = AvailableModel::from_caps(
            "qwen-override",
            &ModelCapabilities {
                n_ctx: 128_000,
                supports_tools: true,
                tokenizer: "fake".to_string(),
                ..Default::default()
            },
            true,
            None,
        );
        model.wire_format_override = Some(WireFormat::OpenaiResponses);
        model.endpoint_override =
            Some("https://dashscope.aliyuncs.com/model-specific/v1/responses".to_string());

        let record = build_chat_model_record(
            "qwen",
            &[],
            &model,
            &HashMap::new(),
            WireFormat::OpenaiChatCompletions,
            "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions",
            "test-key",
            "",
            "",
            &HashMap::new(),
            true,
        );

        assert_eq!(record.base.id, "qwen/qwen-override");
        assert_eq!(record.base.wire_format, WireFormat::OpenaiResponses);
        assert_eq!(
            record.base.endpoint,
            "https://dashscope.aliyuncs.com/model-specific/v1/responses"
        );
        assert_eq!(record.base.api_key, "test-key");
    }
}
