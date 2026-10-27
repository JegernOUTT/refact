use std::sync::Arc;
use std::collections::HashSet;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::execute_at::run_at_commands_locally;
use crate::call_validation::{ChatContent, ChatMessage, ChatMeta, ReasoningEffort, SamplingParameters};
use crate::caps::{resolve_chat_model, ChatModelRecord};
use crate::global_context::GlobalContext;
use crate::llm::{LlmRequest, CanonicalToolChoice, CommonParams, ReasoningIntent, WireFormat};
use crate::llm::params::CacheControl;
use crate::scratchpad_abstract::HasTokenizerAndEot;
use crate::scratchpads::scratchpad_utils::HasRagResults;
use refact_chat_api::FrozenRequestPrefix;
use refact_chat_history::trajectory_ops::COMPRESSION_REPORT_ROLE;
use refact_tool_api::{build_registry_from_names, ToolAliasRegistry, ToolDesc};
use super::tools::execute_tools;
use super::types::ThreadParams;

use super::diagnostics::filter_ui_only_messages;
use super::history_limit::fix_and_limit_messages_history;
use super::linearize::apply_summarization_linearize;
use super::prompts::prepend_the_right_system_prompt_and_maybe_more_initial_messages;
use super::config::tokens;

pub struct CanonicalOpenAiTools {
    pub tools: Vec<Value>,
    pub alias_registry: ToolAliasRegistry,
}

pub fn build_canonical_openai_tools_from_aliases(
    tools: &[ToolDesc],
    aliases: ToolAliasRegistry,
    mode_supports_strict: bool,
    supports_tools: bool,
) -> CanonicalOpenAiTools {
    let filtered_tools: Vec<ToolDesc> = if supports_tools {
        tools.to_vec()
    } else {
        vec![]
    };
    let openai_tools = filtered_tools
        .iter()
        .map(|tool| {
            let alias = aliases
                .get_alias(&tool.name)
                .unwrap_or(&tool.name)
                .to_string();
            let mut value = tool.clone().into_openai_style(mode_supports_strict);
            if alias != tool.name {
                if let Some(function) = value.get_mut("function") {
                    function["name"] = serde_json::Value::String(alias);
                }
            }
            value
        })
        .collect();
    CanonicalOpenAiTools {
        tools: openai_tools,
        alias_registry: aliases,
    }
}

pub async fn build_canonical_openai_tools(
    gcx: Arc<GlobalContext>,
    tools: &[ToolDesc],
    mode_supports_strict: bool,
    supports_tools: bool,
) -> CanonicalOpenAiTools {
    let filtered_tools: Vec<ToolDesc> = if supports_tools {
        tools.to_vec()
    } else {
        vec![]
    };
    let tool_names: Vec<String> = filtered_tools.iter().map(|t| t.name.clone()).collect();
    let alias_registry = build_registry_from_names(&tool_names);
    let mut openai_tools: Vec<Value> = filtered_tools
        .iter()
        .map(|tool| {
            let alias = alias_registry
                .get_alias(&tool.name)
                .unwrap_or(&tool.name)
                .to_string();
            let mut v = tool.clone().into_openai_style(mode_supports_strict);
            if alias != tool.name {
                if let Some(func) = v.get_mut("function") {
                    func["name"] = serde_json::Value::String(alias);
                }
            }
            v
        })
        .collect();

    if supports_tools {
        let handoff_alias = alias_registry
            .get_alias("handoff_to_mode")
            .unwrap_or("handoff_to_mode");
        if let Some(idx) = openai_tools.iter().position(|t| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .map(|n| n == handoff_alias)
                .unwrap_or(false)
        }) {
            if let Some(registry) =
                crate::yaml_configs::customization_registry::get_project_registry(gcx.clone()).await
            {
                let mut mode_lines = Vec::new();
                let mut mode_ids = Vec::new();
                let mut modes: Vec<_> = registry.modes.values().collect();
                modes.sort_by(|a, b| a.id.cmp(&b.id));
                for mode in modes {
                    if mode.specific {
                        continue;
                    }
                    let title = if mode.title.is_empty() {
                        mode.id.clone()
                    } else {
                        mode.title.clone()
                    };
                    let mut desc = mode.description.clone();
                    if desc.len() > 120 {
                        desc = format!("{}...", desc.chars().take(120).collect::<String>());
                    }
                    mode_lines.push(format!(
                        "- {}: {}",
                        mode.id,
                        if desc.is_empty() { title } else { desc }
                    ));
                    mode_ids.push(mode.id.clone());
                }
                let mode_list = mode_lines.join("\n");
                if let Some(func) = openai_tools[idx].get_mut("function") {
                    if let Some(desc_val) = func.get_mut("description") {
                        let desc = desc_val.as_str().unwrap_or("");
                        let enriched = format!("{}\n\nAvailable modes:\n{}", desc, mode_list);
                        *desc_val = serde_json::Value::String(enriched);
                    }
                    if let Some(params) = func.get_mut("parameters") {
                        if let Some(props) = params.get_mut("properties") {
                            if let Some(target_mode) = props.get_mut("target_mode") {
                                let desc =
                                    format!("Target mode ID. Available modes:\n{}", mode_list);
                                target_mode["description"] = serde_json::Value::String(desc);
                                target_mode["enum"] = serde_json::Value::Array(
                                    mode_ids
                                        .into_iter()
                                        .map(serde_json::Value::String)
                                        .collect(),
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    CanonicalOpenAiTools {
        tools: openai_tools,
        alias_registry,
    }
}

fn responses_stateful_tail(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    // For stateful Responses API (previous_response_id), we should send only *new* items.
    // In our chat representation, those are the messages *after* the last assistant message
    // (tool outputs, context_files, new user message, etc.).
    if let Some(last_asst) = messages.iter().rposition(|m| m.role == "assistant") {
        if last_asst + 1 < messages.len() {
            return messages[last_asst + 1..].to_vec();
        }
        return vec![];
    }
    // If we don't have an assistant message yet, keep whatever we have (first turn).
    messages
}

fn last_system_message(messages: &[ChatMessage]) -> Option<ChatMessage> {
    messages.iter().rev().find(|m| m.role == "system").cloned()
}

fn remove_visualization_only_messages(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    messages
        .into_iter()
        .filter(|message| !matches!(message.role.as_str(), "error" | COMPRESSION_REPORT_ROLE))
        .collect()
}

pub struct PreparedChat {
    pub llm_request: LlmRequest,
    pub limited_messages: Vec<ChatMessage>,
    pub rag_results: Vec<serde_json::Value>,
}

pub struct ChatPrepareOptions {
    pub prepend_system_prompt: bool,
    pub allow_at_commands: bool,
    pub allow_tool_prerun: bool,
    pub supports_tools: bool,
    pub tool_choice: Option<ToolChoice>,
    pub parallel_tool_calls: Option<bool>,
    pub cache_control: CacheControl,
    pub frozen_request_prefix: Option<FrozenRequestPrefix>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ToolChoice {
    Auto,
    None,
    Required,
    #[serde(rename = "function")]
    Function {
        name: String,
    },
}

impl Default for ChatPrepareOptions {
    fn default() -> Self {
        Self {
            prepend_system_prompt: true,
            allow_at_commands: true,
            allow_tool_prerun: true,
            supports_tools: true,
            tool_choice: None,
            parallel_tool_calls: None,
            cache_control: CacheControl::Off,
            frozen_request_prefix: None,
        }
    }
}

fn frozen_openai_tools(prefix: Option<&FrozenRequestPrefix>) -> Option<Vec<Value>> {
    prefix
        .and_then(|prefix| prefix.tools_canonical.as_ref())
        .and_then(|value| value.as_array())
        .map(|tools| tools.to_vec())
}

async fn select_canonical_openai_tools(
    gcx: Arc<GlobalContext>,
    tools: &[ToolDesc],
    mode_supports_strict: bool,
    supports_tools: bool,
    frozen_request_prefix: Option<&FrozenRequestPrefix>,
) -> CanonicalOpenAiTools {
    if let Some(openai_tools) = frozen_openai_tools(frozen_request_prefix) {
        let tool_names: Vec<String> = tools.iter().map(|tool| tool.name.clone()).collect();
        return CanonicalOpenAiTools {
            tools: openai_tools,
            alias_registry: build_registry_from_names(&tool_names),
        };
    }

    build_canonical_openai_tools(gcx, tools, mode_supports_strict, supports_tools).await
}

fn apply_frozen_system_prompt(
    mut messages: Vec<ChatMessage>,
    frozen_request_prefix: Option<&FrozenRequestPrefix>,
) -> Vec<ChatMessage> {
    let Some(system_prompt) = frozen_request_prefix
        .and_then(|prefix| prefix.system_prompt.as_ref())
        .filter(|text| !text.is_empty())
    else {
        return messages;
    };

    match messages.iter().position(|message| message.role == "system") {
        Some(0) => {
            messages[0].content = ChatContent::SimpleText(system_prompt.clone());
        }
        Some(idx) => {
            let mut system_message = messages.remove(idx);
            system_message.content = ChatContent::SimpleText(system_prompt.clone());
            messages.insert(0, system_message);
        }
        None => {
            messages.insert(
                0,
                ChatMessage::new("system".to_string(), system_prompt.clone()),
            );
        }
    }

    messages
}

pub async fn prepare_chat_passthrough(
    gcx: Arc<GlobalContext>,
    ccx: Arc<AMutex<AtCommandsContext>>,
    t: &HasTokenizerAndEot,
    messages: Vec<ChatMessage>,
    thread: &ThreadParams,
    model_id: &str,
    mode_id: &str,
    tools: Vec<ToolDesc>,
    meta: &ChatMeta,
    sampling_parameters: &mut SamplingParameters,
    options: &ChatPrepareOptions,
) -> Result<PreparedChat, String> {
    // Boundary projection must precede UI filtering, enrichment, and provider conversion.
    let messages = apply_summarization_linearize(messages)?;
    let mut has_rag_results = HasRagResults::new();
    let messages = filter_ui_only_messages(messages);
    let messages = remove_visualization_only_messages(messages);
    let tool_names: HashSet<String> = tools.iter().map(|x| x.name.clone()).collect();

    // 1. Resolve model early to get reasoning params before history limiting
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| e.message)?;
    let model_record = resolve_chat_model(caps, model_id)?;

    let model_n_ctx = if model_record.base.n_ctx > 0 {
        model_record.base.n_ctx
    } else {
        tokens().default_n_ctx
    };
    let effective_n_ctx = if let Some(cap) = meta.context_tokens_cap {
        if cap == 0 {
            model_n_ctx
        } else {
            cap.min(model_n_ctx)
        }
    } else {
        model_n_ctx
    };

    // 2. Adapt sampling parameters for reasoning models BEFORE history limiting
    adapt_sampling_for_reasoning_models(sampling_parameters, &model_record);

    // 3. System prompt injection (decoupled from allow_at_commands)
    let prompt_tool_names = if options.allow_at_commands {
        tool_names.clone()
    } else {
        HashSet::new()
    };
    let task_meta = ccx.lock().await.task_meta.clone();
    let app = AppState::from_gcx(gcx.clone()).await;
    let messages = if options.prepend_system_prompt {
        let (msgs, _) = prepend_the_right_system_prompt_and_maybe_more_initial_messages(
            app.clone(),
            messages,
            meta,
            &task_meta,
            &mut has_rag_results,
            prompt_tool_names,
            mode_id,
            model_id,
        )
        .await;
        msgs
    } else {
        messages
    };

    // 4. Run @-commands
    let (mut messages, _) = if options.allow_at_commands {
        run_at_commands_locally(
            ccx.clone(),
            t.tokenizer.clone(),
            sampling_parameters.max_new_tokens,
            messages,
            &mut has_rag_results,
        )
        .await
    } else {
        (messages, false)
    };

    // 5. Tool prerun - restricted to allowed tools only
    // Safety: Only execute tool calls from the last message if:
    //   - It's an assistant message with pending tool calls
    //   - The tool calls have not been answered yet (no subsequent tool result messages)
    // This prevents executing tools from injected/external assistant messages.
    if options.supports_tools && options.allow_tool_prerun {
        if let Some(last_msg) = messages.last() {
            if last_msg.role == "assistant" {
                if let Some(ref tool_calls) = last_msg.tool_calls {
                    // Verify these tool calls are pending (no tool results exist for them)
                    let pending_call_ids: HashSet<String> =
                        tool_calls.iter().map(|tc| tc.id.clone()).collect();
                    let answered_call_ids: HashSet<String> = messages
                        .iter()
                        .filter(|m| m.role == "tool" || m.role == "diff")
                        .map(|m| m.tool_call_id.clone())
                        .collect();
                    let unanswered_calls: Vec<_> = tool_calls
                        .iter()
                        .filter(|tc| !answered_call_ids.contains(&tc.id))
                        .filter(|tc| tool_names.contains(&tc.function.name))
                        .cloned()
                        .collect();

                    if !unanswered_calls.is_empty()
                        && pending_call_ids.len()
                            == unanswered_calls.len()
                                + answered_call_ids
                                    .iter()
                                    .filter(|id| pending_call_ids.contains(*id))
                                    .count()
                    {
                        let mut prerun_thread = thread.clone();
                        prerun_thread.context_tokens_cap = Some(effective_n_ctx);
                        prerun_thread.model = model_id.to_string();
                        let (tool_results, _) = execute_tools(
                            app.clone(),
                            &unanswered_calls,
                            &messages,
                            &prerun_thread,
                            "agent",
                            Some(&prerun_thread.model),
                            super::tools::ExecuteToolsOptions::default(),
                        )
                        .await;
                        messages.extend(tool_results);
                    }
                }
            }
        }
    }

    let canonical_tools = select_canonical_openai_tools(
        gcx.clone(),
        &tools,
        model_record.supports_strict_tools,
        options.supports_tools,
        options.frozen_request_prefix.as_ref(),
    )
    .await;
    let openai_tools = canonical_tools.tools;
    let alias_registry = canonical_tools.alias_registry;

    // 7. Linearize summarization messages first (suppress summarized sources,
    // inline summaries) so history validation, repair and token budgeting all
    // operate on exactly what reaches the model. Running the limiter first
    // mutates/reorders messages, which previously made suppression unreliable
    // and silently re-sent every summarized source under context pressure.
    let linearized_msgs = apply_summarization_linearize(messages)?;

    // 7.5. History validation and fixing
    let limited_msgs = fix_and_limit_messages_history(&linearized_msgs, sampling_parameters)?;

    // 8. Strip thinking blocks if thinking is disabled
    let mut limited_adapted_msgs =
        strip_thinking_blocks_if_disabled(limited_msgs, sampling_parameters, &model_record);
    limited_adapted_msgs = replace_images_for_text_only_model(limited_adapted_msgs, &model_record);

    // OpenAI Responses API stateful multi-turn: when we chain with previous_response_id,
    // we should send only the new tail items (tool outputs and/or new user message).
    if model_record.base.wire_format == WireFormat::OpenaiResponses
        && thread
            .previous_response_id
            .as_ref()
            .is_some_and(|s| !s.is_empty())
    {
        let tail = responses_stateful_tail(limited_adapted_msgs.clone());
        let mut stitched = Vec::new();
        if let Some(sys) = last_system_message(&limited_adapted_msgs) {
            stitched.push(sys);
        }
        stitched.extend(tail);
        limited_adapted_msgs = stitched;
    }

    limited_adapted_msgs =
        apply_frozen_system_prompt(limited_adapted_msgs, options.frozen_request_prefix.as_ref());
    limited_adapted_msgs = fit_outbound_images_to_budget(
        limited_adapted_msgs,
        refact_core::image_policy::OUTBOUND_IMAGE_BASE64_BUDGET,
    )?;

    // 10. Build LlmRequest
    // Enforce n=1 for chat - multi-choice not supported in streaming accumulation
    let common_params = CommonParams {
        n_ctx: Some(effective_n_ctx),
        max_tokens: sampling_parameters.max_new_tokens,
        temperature: sampling_parameters.temperature,
        top_p: sampling_parameters.top_p,
        frequency_penalty: sampling_parameters.frequency_penalty,
        stop: sampling_parameters.stop.clone(),
        n: Some(1),
    };

    let reasoning = sampling_params_to_reasoning_intent(sampling_parameters, &model_record);

    let tool_choice = options.tool_choice.as_ref().map(|tc| match tc {
        ToolChoice::Auto => CanonicalToolChoice::Auto,
        ToolChoice::None => CanonicalToolChoice::None,
        ToolChoice::Required => CanonicalToolChoice::Required,
        ToolChoice::Function { name } => {
            let aliased_name = alias_registry.get_alias(name).unwrap_or(name).to_string();
            CanonicalToolChoice::Function { name: aliased_name }
        }
    });

    let mut llm_request = LlmRequest::new(model_id.to_string(), limited_adapted_msgs.clone())
        .with_params(common_params)
        .with_tools(openai_tools, tool_choice)
        .with_reasoning(reasoning)
        .with_parallel_tool_calls(
            options.parallel_tool_calls.unwrap_or(false) && model_record.supports_parallel_tools,
        )
        .with_cache_control(options.cache_control);

    if model_record.base.wire_format == WireFormat::OpenaiResponses {
        llm_request = llm_request.with_previous_response_id(thread.previous_response_id.clone());
    }

    if model_record.base.id.starts_with("openrouter/")
        && !model_record.available_providers.is_empty()
    {
        if let Some(selected_provider) = model_record.selected_provider.as_ref() {
            let mut extra_body = llm_request.extra_body.unwrap_or_default();
            extra_body.insert(
                "provider".to_string(),
                serde_json::json!({"order": [selected_provider]}),
            );
            llm_request.extra_body = Some(extra_body);
        }
    }

    Ok(PreparedChat {
        llm_request,
        limited_messages: limited_adapted_msgs,
        rag_results: has_rag_results.in_json,
    })
}

fn adapt_sampling_for_reasoning_models(
    sampling_parameters: &mut SamplingParameters,
    model_record: &ChatModelRecord,
) {
    let user_set_max_tokens = sampling_parameters.max_new_tokens > 0;

    if !user_set_max_tokens {
        sampling_parameters.max_new_tokens = model_record
            .default_max_tokens
            .or(model_record.max_output_tokens)
            .unwrap_or(4096);
    }

    if let Some(max_output) = model_record.max_output_tokens {
        if sampling_parameters.max_new_tokens > max_output {
            sampling_parameters.max_new_tokens = max_output;
        }
    }

    if sampling_parameters.temperature.is_none() {
        sampling_parameters.temperature = model_record.default_temperature;
    }

    if sampling_parameters.frequency_penalty.is_none() {
        sampling_parameters.frequency_penalty = model_record.default_frequency_penalty;
    }

    let has_reasoning_support = model_record
        .reasoning_effort_options
        .as_ref()
        .is_some_and(|options| !options.is_empty())
        || model_record.supports_thinking_budget
        || model_record.supports_adaptive_thinking_budget;

    if !has_reasoning_support {
        sampling_parameters.reasoning_effort = None;
        sampling_parameters.thinking = None;
        sampling_parameters.thinking_budget = None;
        sampling_parameters.enable_thinking = None;
        return;
    }

    if let Some(ref options) = model_record.reasoning_effort_options {
        if sampling_parameters
            .reasoning_effort
            .as_ref()
            .is_some_and(|effort| {
                !options
                    .iter()
                    .any(|option| ReasoningEffort::from_str_opt(option).as_ref() == Some(effort))
            })
        {
            sampling_parameters.reasoning_effort = None;
        }
    }

    if sampling_parameters.boost_reasoning {
        if model_record.supports_thinking_budget && sampling_parameters.thinking_budget.is_none() {
            let min_budget = tokens().min_budget_tokens;
            let budget = if sampling_parameters.max_new_tokens > min_budget {
                (sampling_parameters.max_new_tokens / 2).max(min_budget)
            } else {
                min_budget
            };
            sampling_parameters.thinking_budget = Some(budget);
        }

        if let Some(ref options) = model_record.reasoning_effort_options {
            if sampling_parameters.reasoning_effort.is_none() {
                let default_effort = if options.iter().any(|option| option == "medium") {
                    Some(ReasoningEffort::Medium)
                } else {
                    options
                        .last()
                        .and_then(|option| ReasoningEffort::from_str_opt(option))
                };
                sampling_parameters.reasoning_effort = default_effort;
            }
        }
    }

    if !model_record
        .reasoning_effort_options
        .as_ref()
        .is_some_and(|options| !options.is_empty())
    {
        sampling_parameters.reasoning_effort = None;
    }
    if !model_record.supports_thinking_budget && !model_record.supports_adaptive_thinking_budget {
        sampling_parameters.thinking_budget = None;
    }
    sampling_parameters.thinking = None;
    sampling_parameters.enable_thinking = None;
}

fn sampling_params_to_reasoning_intent(
    sampling_parameters: &SamplingParameters,
    model_record: &ChatModelRecord,
) -> ReasoningIntent {
    let has_reasoning_support = model_record
        .reasoning_effort_options
        .as_ref()
        .is_some_and(|options| !options.is_empty())
        || model_record.supports_thinking_budget
        || model_record.supports_adaptive_thinking_budget;

    if !has_reasoning_support {
        return ReasoningIntent::Off;
    }

    if let Some(budget) = sampling_parameters.thinking_budget {
        return ReasoningIntent::BudgetTokens(budget);
    }

    if let Some(ref effort) = sampling_parameters.reasoning_effort {
        return match effort {
            ReasoningEffort::NoReasoning => ReasoningIntent::NoReasoning,
            ReasoningEffort::Minimal => ReasoningIntent::Minimal,
            ReasoningEffort::Low => ReasoningIntent::Low,
            ReasoningEffort::Medium => ReasoningIntent::Medium,
            ReasoningEffort::High => ReasoningIntent::High,
            ReasoningEffort::XHigh => ReasoningIntent::XHigh,
            ReasoningEffort::Max => ReasoningIntent::Max,
        };
    }

    if let Some(ref thinking) = sampling_parameters.thinking {
        if thinking.get("type").and_then(|t| t.as_str()) == Some("enabled") {
            if let Some(budget) = thinking.get("budget_tokens").and_then(|b| b.as_u64()) {
                return ReasoningIntent::BudgetTokens(budget as usize);
            }
            return ReasoningIntent::Medium;
        }
    }

    if sampling_parameters.enable_thinking == Some(true) {
        return ReasoningIntent::Medium;
    }

    if sampling_parameters.boost_reasoning {
        return ReasoningIntent::Medium;
    }

    ReasoningIntent::Off
}

fn is_thinking_enabled(sampling_parameters: &SamplingParameters) -> bool {
    sampling_parameters
        .thinking
        .as_ref()
        .and_then(|t| t.get("type"))
        .and_then(|t| t.as_str())
        .map(|t| t == "enabled")
        .unwrap_or(false)
        || sampling_parameters.reasoning_effort.is_some()
        || sampling_parameters.thinking_budget.is_some()
        || sampling_parameters.enable_thinking == Some(true)
}

fn strip_thinking_blocks_if_disabled(
    messages: Vec<ChatMessage>,
    sampling_parameters: &SamplingParameters,
    model_record: &ChatModelRecord,
) -> Vec<ChatMessage> {
    let has_reasoning = model_record
        .reasoning_effort_options
        .as_ref()
        .is_some_and(|options| !options.is_empty())
        || model_record.supports_thinking_budget
        || model_record.supports_adaptive_thinking_budget;
    if !has_reasoning || !is_thinking_enabled(sampling_parameters) {
        let preserve_cloud_code_signatures =
            model_record.base.wire_format == WireFormat::GoogleCloudCode;
        messages
            .into_iter()
            .map(|mut msg| {
                msg.thinking_blocks = if preserve_cloud_code_signatures {
                    msg.thinking_blocks
                        .take()
                        .map(|blocks| {
                            blocks
                                .into_iter()
                                .filter(|block| {
                                    block.get("provider").and_then(Value::as_str)
                                        == Some("google_cloud_code")
                                        && block
                                            .get("signature")
                                            .and_then(Value::as_str)
                                            .is_some_and(|signature| !signature.is_empty())
                                })
                                .collect::<Vec<_>>()
                        })
                        .filter(|blocks| !blocks.is_empty())
                } else {
                    None
                };
                msg.reasoning_content = None;
                msg
            })
            .collect()
    } else {
        messages
    }
}

pub(crate) const TEXT_ONLY_IMAGE_PLACEHOLDER: &str =
    "[image removed: model does not support image input]";

/// Replace image (and other non-text) multimodal elements with a text
/// placeholder when the target model has no multimodality support, so
/// text-only providers never receive `image_url` content parts.
fn replace_images_for_text_only_model(
    messages: Vec<ChatMessage>,
    model_record: &ChatModelRecord,
) -> Vec<ChatMessage> {
    if model_record.supports_multimodality {
        return messages;
    }
    messages
        .into_iter()
        .map(|mut msg| {
            if let ChatContent::Multimodal(elements) = &mut msg.content {
                for element in elements.iter_mut() {
                    if element.m_type != "text" {
                        element.m_type = "text".to_string();
                        element.m_content = TEXT_ONLY_IMAGE_PLACEHOLDER.to_string();
                    }
                }
            }
            msg
        })
        .collect()
}

fn fit_outbound_images_to_budget(
    mut messages: Vec<ChatMessage>,
    budget: usize,
) -> Result<Vec<ChatMessage>, String> {
    for (message_index, message) in messages.iter_mut().enumerate() {
        let ChatContent::Multimodal(elements) = &mut message.content else {
            continue;
        };
        for element in elements.iter_mut() {
            if !element.m_type.starts_with("image/") {
                continue;
            }
            let fitted = refact_core::image_policy::fit_image_base64_to_budget(
                &element.m_type,
                &element.m_content,
                budget,
            )
            .map_err(|error| {
                format!(
                    "outbound image in {} message #{message_index}: {error}",
                    message.role
                )
            })?;
            if let Some((mime, payload)) = fitted {
                element.m_type = mime;
                element.m_content = payload;
            }
        }
    }
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    fn make_model_record_no_reasoning() -> ChatModelRecord {
        ChatModelRecord {
            base: Default::default(),
            default_temperature: Some(0.7),
            ..Default::default()
        }
    }

    fn make_sampling_params() -> SamplingParameters {
        SamplingParameters {
            max_new_tokens: 4096,
            temperature: Some(1.0),
            reasoning_effort: None,
            thinking: None,
            enable_thinking: None,
            boost_reasoning: false,
            ..Default::default()
        }
    }

    fn plan_message(mode: &str, version: u32, content: &str) -> ChatMessage {
        crate::chat::internal_roles::plan(mode, version, content, None)
    }

    #[tokio::test]
    async fn prepare_fits_oversized_outbound_image_without_touching_small_one() {
        use base64::Engine;
        use refact_core::chat_types::MultimodalElement;

        let model_id = "test/vision";
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut caps = crate::caps::CodeAssistantCaps::default();
        caps.chat_models.insert(
            model_id.to_string(),
            Arc::new(ChatModelRecord {
                base: crate::caps::BaseModelRecord {
                    id: model_id.to_string(),
                    name: model_id.to_string(),
                    n_ctx: 8192,
                    tokenizer: "fake".to_string(),
                    ..Default::default()
                },
                supports_multimodality: true,
                ..Default::default()
            }),
        );
        {
            let app = AppState::from_gcx(gcx.clone()).await;
            let mut state = app.model.caps.write().await;
            state.caps = Some(Arc::new(caps));
            state.last_attempted_ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
        }

        let mut seed = 0x9e37_79b9u32;
        let noise = image::RgbImage::from_fn(1500, 1000, |_, _| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let [r, g, b, _] = seed.to_le_bytes();
            image::Rgb([r, g, b])
        });
        let mut png = Vec::new();
        noise
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let large = base64::engine::general_purpose::STANDARD.encode(png);
        assert!(large.len() > 5_000_000);
        let mut small_png = Vec::new();
        image::RgbaImage::from_pixel(4, 3, image::Rgba([10, 20, 30, 128]))
            .write_to(
                &mut std::io::Cursor::new(&mut small_png),
                image::ImageFormat::Png,
            )
            .unwrap();
        let small = base64::engine::general_purpose::STANDARD.encode(small_png);
        let element = |m_type: &str, m_content: &str| MultimodalElement {
            m_type: m_type.to_string(),
            m_content: m_content.to_string(),
        };
        let user = ChatMessage {
            role: "user".to_string(),
            content: ChatContent::Multimodal(vec![
                element("text", "look"),
                element("image/png", &large),
                element("image/png", &small),
            ]),
            ..Default::default()
        };
        let messages = vec![user];

        let app = AppState::from_gcx(gcx.clone()).await;
        let ccx = AtCommandsContext::new_from_app(
            app,
            8192,
            1,
            false,
            messages.clone(),
            "image-budget-chat".to_string(),
            None,
            model_id.to_string(),
            None,
            None,
        )
        .await;
        let tokenizer = crate::tokens::cached_tokenizer(
            gcx.clone(),
            &crate::caps::BaseModelRecord {
                id: model_id.to_string(),
                name: model_id.to_string(),
                tokenizer: "fake".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let options = ChatPrepareOptions {
            prepend_system_prompt: false,
            allow_at_commands: false,
            allow_tool_prerun: false,
            ..Default::default()
        };
        let prepared = prepare_chat_passthrough(
            gcx,
            Arc::new(AMutex::new(ccx)),
            &HasTokenizerAndEot::new(tokenizer),
            messages,
            &ThreadParams {
                model: model_id.to_string(),
                ..Default::default()
            },
            model_id,
            "agent",
            vec![],
            &ChatMeta {
                chat_id: "image-budget-chat".to_string(),
                request_attempt_id: "attempt".to_string(),
                chat_remote: false,
                chat_mode: "agent".to_string(),
                current_config_file: String::new(),
                include_project_info: false,
                context_tokens_cap: Some(8192),
                worktree: None,
            },
            &mut make_sampling_params(),
            &options,
        )
        .await
        .unwrap();

        let sent = prepared
            .llm_request
            .messages
            .iter()
            .find(|message| message.role == "user")
            .unwrap();
        let ChatContent::Multimodal(elements) = &sent.content else {
            panic!("expected multimodal user content");
        };
        assert_eq!(elements.len(), 3);
        assert_eq!(
            (elements[0].m_type.as_str(), elements[0].m_content.as_str()),
            ("text", "look")
        );
        assert_eq!(elements[1].m_type, "image/jpeg");
        assert!(elements[1].m_content.len() <= 5_000_000);
        let fitted = base64::engine::general_purpose::STANDARD
            .decode(&elements[1].m_content)
            .unwrap();
        assert_eq!(
            refact_core::chat_types::probe_image_dimensions(&fitted).unwrap(),
            (1500, 1000)
        );
        assert_eq!(
            (elements[2].m_type.as_str(), elements[2].m_content.as_str()),
            ("image/png", small.as_str())
        );
    }

    #[test]
    fn outbound_image_budget_propagates_tool_image_errors() {
        let message = ChatMessage {
            role: "tool".to_string(),
            tool_call_id: "screenshot".to_string(),
            content: ChatContent::Multimodal(vec![refact_core::chat_types::MultimodalElement {
                m_type: "image/png".to_string(),
                m_content: "A".repeat(400),
            }]),
            ..Default::default()
        };
        let error = fit_outbound_images_to_budget(vec![message], 100).unwrap_err();
        assert!(
            error.starts_with(
                "outbound image in tool message #0: cannot fit image/png image (400 base64 bytes) into 100"
            ),
            "{error}"
        );
    }

    #[test]
    fn test_strip_thinking_blocks_preserves_cloud_code_signatures() {
        let mut model = make_model_record_no_reasoning();
        model.base.wire_format = WireFormat::GoogleCloudCode;
        let params = make_sampling_params();
        let msgs = vec![ChatMessage {
            reasoning_content: Some("hidden reasoning".into()),
            thinking_blocks: Some(vec![
                serde_json::json!({
                    "type": "thinking",
                    "provider": "google_cloud_code",
                    "thinking": "signed reasoning",
                    "signature": "sig-google"
                }),
                serde_json::json!({
                    "type": "thinking",
                    "thinking": "unsigned reasoning"
                }),
            ]),
            content: ChatContent::SimpleText("answer".into()),
            ..Default::default()
        }];

        let result = strip_thinking_blocks_if_disabled(msgs, &params, &model);

        assert_eq!(result[0].reasoning_content, None);
        let blocks = result[0].thinking_blocks.as_ref().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["provider"], "google_cloud_code");
        assert_eq!(blocks[0]["signature"], "sig-google");
    }
}
