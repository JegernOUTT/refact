use std::sync::Arc;
use std::collections::HashSet;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::execute_at::run_at_commands_locally;
use crate::call_validation::{ChatMessage, ChatMeta, ReasoningEffort, SamplingParameters};
use crate::caps::{resolve_chat_model, ChatModelRecord};
use crate::global_context::GlobalContext;
use crate::llm::{LlmRequest, CanonicalToolChoice, CommonParams, ReasoningIntent, WireFormat};
use crate::llm::params::CacheControl;
use crate::scratchpad_abstract::HasTokenizerAndEot;
use crate::scratchpads::scratchpad_utils::HasRagResults;
use refact_tool_api::{build_registry_from_names, ToolDesc};
use super::tools::execute_tools;
use super::types::ThreadParams;

use super::diagnostics::filter_ui_only_messages;
use super::history_limit::fix_and_limit_messages_history;
use super::linearize::apply_summarization_linearize;
use super::prompts::prepend_the_right_system_prompt_and_maybe_more_initial_messages;
use super::config::tokens;

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
        .filter(|message| message.role != "error")
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
        }
    }
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

    // 6. Build tools list with alias layer to ensure provider-safe names (≤64 chars)
    let filtered_tools: Vec<ToolDesc> = if options.supports_tools {
        tools.to_vec()
    } else {
        vec![]
    };
    let strict_tools = model_record.supports_strict_tools;
    let tool_names: Vec<String> = filtered_tools.iter().map(|t| t.name.clone()).collect();
    let alias_registry = build_registry_from_names(&tool_names);
    let mut openai_tools: Vec<Value> = filtered_tools
        .iter()
        .map(|tool| {
            let alias = alias_registry
                .get_alias(&tool.name)
                .unwrap_or(&tool.name)
                .to_string();
            let mut v = tool.clone().into_openai_style(strict_tools);
            if alias != tool.name {
                if let Some(func) = v.get_mut("function") {
                    func["name"] = serde_json::Value::String(alias);
                }
            }
            v
        })
        .collect();

    // 6b. Enrich handoff_to_mode tool with dynamic mode list
    if options.supports_tools {
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

    // 7. History validation and fixing
    let limited_msgs = fix_and_limit_messages_history(&messages, sampling_parameters)?;

    // 7.5. Linearize summarization messages (replace summarized ranges with summary content)
    let limited_msgs = apply_summarization_linearize(limited_msgs);

    // 8. Strip thinking blocks if thinking is disabled
    let mut limited_adapted_msgs =
        strip_thinking_blocks_if_disabled(limited_msgs, sampling_parameters, &model_record);

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

    let has_reasoning_support = model_record.reasoning_effort_options.is_some()
        || model_record.supports_thinking_budget
        || model_record.supports_adaptive_thinking_budget;

    if !has_reasoning_support {
        sampling_parameters.reasoning_effort = None;
        sampling_parameters.thinking = None;
        sampling_parameters.thinking_budget = None;
        sampling_parameters.enable_thinking = None;
        return;
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
            if sampling_parameters.reasoning_effort.is_none() && !options.is_empty() {
                let default_effort = if options.contains(&"medium".to_string()) {
                    ReasoningEffort::Medium
                } else {
                    ReasoningEffort::from_str_opt(&options[options.len() - 1])
                        .unwrap_or(ReasoningEffort::Medium)
                };
                sampling_parameters.reasoning_effort = Some(default_effort);
            }
        }
    }

    if model_record.reasoning_effort_options.is_none() {
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
    let has_reasoning_support = model_record.reasoning_effort_options.is_some()
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
    let has_reasoning = model_record.reasoning_effort_options.is_some()
        || model_record.supports_thinking_budget
        || model_record.supports_adaptive_thinking_budget;
    if !has_reasoning || !is_thinking_enabled(sampling_parameters) {
        messages
            .into_iter()
            .map(|mut msg| {
                msg.thinking_blocks = None;
                msg.reasoning_content = None;
                msg
            })
            .collect()
    } else {
        messages
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::ChatContent;

    fn make_model_record_effort(effort_options: Option<Vec<&str>>) -> ChatModelRecord {
        ChatModelRecord {
            base: Default::default(),
            default_temperature: Some(0.7),
            reasoning_effort_options: effort_options
                .map(|opts| opts.into_iter().map(|s| s.to_string()).collect()),
            ..Default::default()
        }
    }

    fn make_model_record_thinking_budget() -> ChatModelRecord {
        ChatModelRecord {
            base: Default::default(),
            default_temperature: Some(0.7),
            supports_thinking_budget: true,
            ..Default::default()
        }
    }

    fn make_model_record_adaptive() -> ChatModelRecord {
        ChatModelRecord {
            base: Default::default(),
            default_temperature: Some(0.7),
            supports_adaptive_thinking_budget: true,
            reasoning_effort_options: Some(vec![
                "low".to_string(),
                "medium".to_string(),
                "high".to_string(),
                "max".to_string(),
            ]),
            ..Default::default()
        }
    }

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

    #[test]
    fn test_is_thinking_enabled_with_thinking_json() {
        let mut params = make_sampling_params();
        params.thinking = Some(serde_json::json!({"type": "enabled", "budget_tokens": 1024}));
        assert!(is_thinking_enabled(&params));
    }

    #[test]
    fn test_is_thinking_enabled_with_thinking_disabled() {
        let mut params = make_sampling_params();
        params.thinking = Some(serde_json::json!({"type": "disabled"}));
        assert!(!is_thinking_enabled(&params));
    }

    #[test]
    fn test_is_thinking_enabled_with_reasoning_effort() {
        let mut params = make_sampling_params();
        params.reasoning_effort = Some(ReasoningEffort::Medium);
        assert!(is_thinking_enabled(&params));
    }

    #[test]
    fn test_is_thinking_enabled_with_enable_thinking_true() {
        let mut params = make_sampling_params();
        params.enable_thinking = Some(true);
        assert!(is_thinking_enabled(&params));
    }

    #[test]
    fn test_is_thinking_enabled_with_enable_thinking_false() {
        let mut params = make_sampling_params();
        params.enable_thinking = Some(false);
        assert!(!is_thinking_enabled(&params));
    }

    #[test]
    fn test_is_thinking_enabled_all_none() {
        let params = make_sampling_params();
        assert!(!is_thinking_enabled(&params));
    }

    #[test]
    fn test_strip_thinking_blocks_when_no_reasoning_support() {
        let model = make_model_record_no_reasoning();
        let params = make_sampling_params();
        let msgs = vec![ChatMessage {
            thinking_blocks: Some(vec![serde_json::json!({"type": "thinking"})]),
            content: ChatContent::SimpleText("hello".into()),
            ..Default::default()
        }];
        let result = strip_thinking_blocks_if_disabled(msgs, &params, &model);
        assert!(result[0].thinking_blocks.is_none());
    }

    #[test]
    fn test_strip_thinking_blocks_when_thinking_disabled() {
        let model = make_model_record_thinking_budget();
        let params = make_sampling_params();
        let msgs = vec![ChatMessage {
            thinking_blocks: Some(vec![serde_json::json!({"type": "thinking"})]),
            content: ChatContent::SimpleText("hello".into()),
            ..Default::default()
        }];
        let result = strip_thinking_blocks_if_disabled(msgs, &params, &model);
        assert!(result[0].thinking_blocks.is_none());
    }

    #[test]
    fn test_strip_thinking_blocks_preserves_when_enabled() {
        let model = make_model_record_thinking_budget();
        let mut params = make_sampling_params();
        params.thinking = Some(serde_json::json!({"type": "enabled", "budget_tokens": 1024}));
        let msgs = vec![ChatMessage {
            thinking_blocks: Some(vec![serde_json::json!({"type": "thinking"})]),
            content: ChatContent::SimpleText("hello".into()),
            ..Default::default()
        }];
        let result = strip_thinking_blocks_if_disabled(msgs, &params, &model);
        assert!(result[0].thinking_blocks.is_some());
    }

    #[test]
    fn test_strip_thinking_blocks_preserves_other_fields() {
        let model = make_model_record_no_reasoning();
        let params = make_sampling_params();
        let msgs = vec![ChatMessage {
            role: "assistant".into(),
            content: ChatContent::SimpleText("hello".into()),
            reasoning_content: Some("reasoning".into()),
            thinking_blocks: Some(vec![serde_json::json!({"type": "thinking"})]),
            citations: vec![serde_json::json!({"url": "http://x"})],
            ..Default::default()
        }];
        let result = strip_thinking_blocks_if_disabled(msgs, &params, &model);
        assert_eq!(result[0].role, "assistant");
        assert_eq!(result[0].reasoning_content, None);
        assert_eq!(result[0].citations.len(), 1);
        assert!(result[0].thinking_blocks.is_none());
    }

    #[test]
    fn test_adapt_sampling_effort_boost_reasoning() {
        let mut params = make_sampling_params();
        params.boost_reasoning = true;
        params.temperature = None;
        let model = make_model_record_effort(Some(vec!["low", "medium", "high"]));
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.reasoning_effort, Some(ReasoningEffort::Medium));
        assert_eq!(params.temperature, Some(0.7));
    }

    #[test]
    fn test_adapt_sampling_effort_preserves_user_temperature() {
        let mut params = make_sampling_params();
        params.boost_reasoning = true;
        params.temperature = Some(0.3);
        let model = make_model_record_effort(Some(vec!["low", "medium", "high"]));
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.reasoning_effort, Some(ReasoningEffort::Medium));
        assert_eq!(params.temperature, Some(0.3));
    }

    #[test]
    fn test_adapt_sampling_effort_takes_precedence() {
        let mut params = make_sampling_params();
        params.boost_reasoning = true;
        params.reasoning_effort = Some(ReasoningEffort::High);
        let model = make_model_record_effort(Some(vec!["low", "medium", "high"]));
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.reasoning_effort, Some(ReasoningEffort::High));
    }

    #[test]
    fn test_adapt_sampling_thinking_budget_boost_reasoning() {
        let mut params = make_sampling_params();
        params.boost_reasoning = true;
        params.max_new_tokens = 4096;
        let model = make_model_record_thinking_budget();
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert!(params.thinking_budget.is_some());
        assert!(params.thinking_budget.unwrap() > 0);
        assert!(params.reasoning_effort.is_none());
        assert!(params.thinking.is_none());
        assert!(params.enable_thinking.is_none());
    }

    #[test]
    fn test_adapt_sampling_thinking_budget_explicit_preserved() {
        let mut params = make_sampling_params();
        params.thinking_budget = Some(5000);
        let model = make_model_record_thinking_budget();
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.thinking_budget, Some(5000));
        assert!(params.reasoning_effort.is_none());
        assert!(params.thinking.is_none());
    }

    #[test]
    fn test_adapt_sampling_thinking_budget_no_boost_no_budget() {
        let mut params = make_sampling_params();
        let model = make_model_record_thinking_budget();
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert!(params.thinking_budget.is_none());
        assert!(params.reasoning_effort.is_none());
    }

    #[test]
    fn test_adapt_sampling_adaptive_boost_reasoning() {
        let mut params = make_sampling_params();
        params.boost_reasoning = true;
        let model = make_model_record_adaptive();
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.reasoning_effort, Some(ReasoningEffort::Medium));
        assert!(params.thinking.is_none());
        assert!(params.enable_thinking.is_none());
    }

    #[test]
    fn test_adapt_sampling_adaptive_preserves_reasoning_effort() {
        let mut params = make_sampling_params();
        params.reasoning_effort = Some(ReasoningEffort::High);
        let model = make_model_record_adaptive();
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.reasoning_effort, Some(ReasoningEffort::High));
    }

    #[test]
    fn test_adapt_sampling_no_reasoning_clears_all() {
        let mut params = make_sampling_params();
        params.reasoning_effort = Some(ReasoningEffort::High);
        params.thinking = Some(serde_json::json!({"type": "enabled"}));
        params.enable_thinking = Some(true);
        let model = make_model_record_no_reasoning();
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert!(params.reasoning_effort.is_none());
        assert!(params.thinking.is_none());
        assert!(params.enable_thinking.is_none());
    }

    #[test]
    fn test_adapt_sampling_effort_default_to_last_option() {
        let mut params = make_sampling_params();
        params.boost_reasoning = true;
        let model = make_model_record_effort(Some(vec!["low", "high"]));
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.reasoning_effort, Some(ReasoningEffort::High));
    }

    #[test]
    fn test_no_reasoning_intent_for_no_support() {
        let model = make_model_record_no_reasoning();
        let mut params = make_sampling_params();
        params.boost_reasoning = true;
        let intent = sampling_params_to_reasoning_intent(&params, &model);
        assert_eq!(intent, ReasoningIntent::Off);

        params.reasoning_effort = Some(ReasoningEffort::High);
        let intent = sampling_params_to_reasoning_intent(&params, &model);
        assert_eq!(intent, ReasoningIntent::Off);
    }

    #[test]
    fn test_ui_only_messages_are_filtered_before_prepare_steps() {
        let visible = ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText("visible".to_string()),
            ..Default::default()
        };
        let hidden = crate::chat::diagnostics::make_ui_only_error_message(
            "context_length_exceeded: too large",
        );

        let filtered = filter_ui_only_messages(vec![hidden, visible.clone()]);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].role, "user");
        assert_eq!(filtered[0].content.content_text_only(), "visible");
    }

    #[test]
    fn test_chat_prepare_options_default() {
        let opts = ChatPrepareOptions::default();
        assert!(opts.prepend_system_prompt);
        assert!(opts.allow_at_commands);
        assert!(opts.allow_tool_prerun);
        assert!(opts.supports_tools);
    }
}
