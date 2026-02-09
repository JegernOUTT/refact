use std::sync::Arc;
use std::collections::HashSet;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::execute_at::run_at_commands_locally;
use crate::call_validation::{ChatMessage, ChatMeta, ReasoningEffort, SamplingParameters};
use crate::caps::{resolve_chat_model, ChatModelRecord};
use crate::global_context::GlobalContext;
use crate::llm::{LlmRequest, CanonicalToolChoice, CommonParams, ReasoningIntent};
use crate::llm::params::CacheControl;
use crate::scratchpad_abstract::HasTokenizerAndEot;
use crate::scratchpads::scratchpad_utils::HasRagResults;
use crate::tools::tools_description::ToolDesc;
use super::tools::execute_tools;
use super::types::ThreadParams;

use super::history_limit::fix_and_limit_messages_history;
use super::prompts::prepend_the_right_system_prompt_and_maybe_more_initial_messages;
use super::config::tokens;

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
    gcx: Arc<ARwLock<GlobalContext>>,
    ccx: Arc<AMutex<AtCommandsContext>>,
    t: &HasTokenizerAndEot,
    messages: Vec<ChatMessage>,
    model_id: &str,
    mode_id: &str,
    tools: Vec<ToolDesc>,
    meta: &ChatMeta,
    sampling_parameters: &mut SamplingParameters,
    options: &ChatPrepareOptions,
) -> Result<PreparedChat, String> {
    let mut has_rag_results = HasRagResults::new();
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
    let messages = if options.prepend_system_prompt {
        prepend_the_right_system_prompt_and_maybe_more_initial_messages(
            gcx.clone(),
            messages,
            meta,
            &task_meta,
            &mut has_rag_results,
            prompt_tool_names,
            mode_id,
            model_id,
        )
        .await
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
                    let pending_call_ids: HashSet<String> = tool_calls
                        .iter()
                        .map(|tc| tc.id.clone())
                        .collect();
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

                    if !unanswered_calls.is_empty() && pending_call_ids.len() == unanswered_calls.len() + answered_call_ids.iter().filter(|id| pending_call_ids.contains(*id)).count() {
                        let thread = ThreadParams {
                            id: meta.chat_id.clone(),
                            model: model_id.to_string(),
                            context_tokens_cap: Some(effective_n_ctx),
                            ..Default::default()
                        };
                        let (tool_results, _) = execute_tools(
                            gcx.clone(),
                            &unanswered_calls,
                            &messages,
                            &thread,
                            "agent",
                            Some(&thread.model),
                            super::tools::ExecuteToolsOptions::default(),
                        )
                        .await;
                        messages.extend(tool_results);
                    }
                }
            }
        }
    }

    // 6. Build tools list
    let filtered_tools: Vec<ToolDesc> = if options.supports_tools {
        tools
            .iter()
            .filter(|x| x.is_supported_by(model_id))
            .cloned()
            .collect()
    } else {
        vec![]
    };
    let strict_tools = model_record.supports_strict_tools;
    let openai_tools: Vec<Value> = filtered_tools
        .iter()
        .map(|tool| tool.clone().into_openai_style(strict_tools))
        .collect();

    // 7. History validation and fixing
    let limited_msgs = fix_and_limit_messages_history(&messages, sampling_parameters)?;

    // 8. Strip thinking blocks if thinking is disabled
    let limited_adapted_msgs =
        strip_thinking_blocks_if_disabled(limited_msgs, sampling_parameters, &model_record);

    // 9. Linearize thread: merge consecutive user-like messages for cache-friendly
    //    strict role alternation (system/user/assistant/user/assistant/...)
    let linearized_msgs = super::linearize::linearize_thread_for_llm(&limited_adapted_msgs);

    // 10. Build LlmRequest
    // Enforce n=1 for chat - multi-choice not supported in streaming accumulation
    let common_params = CommonParams {
        max_tokens: sampling_parameters.max_new_tokens,
        temperature: sampling_parameters.temperature,
        frequency_penalty: sampling_parameters.frequency_penalty,
        stop: sampling_parameters.stop.clone(),
        n: Some(1),
    };

    let reasoning = sampling_params_to_reasoning_intent(sampling_parameters, &model_record);

    let tool_choice = options.tool_choice.as_ref().map(|tc| match tc {
        ToolChoice::Auto => CanonicalToolChoice::Auto,
        ToolChoice::None => CanonicalToolChoice::None,
        ToolChoice::Required => CanonicalToolChoice::Required,
        ToolChoice::Function { name } => CanonicalToolChoice::Function { name: name.clone() },
    });

    let mut llm_request = LlmRequest::new(model_id.to_string(), linearized_msgs.clone())
        .with_params(common_params)
        .with_tools(openai_tools, tool_choice)
        .with_reasoning(reasoning)
        .with_parallel_tool_calls(options.parallel_tool_calls.unwrap_or(false))
        .with_cache_control(options.cache_control);

    // Add meta for Refact cloud when support_metadata is enabled
    if model_record.base.support_metadata {
        llm_request = llm_request.with_meta(meta.clone());
    }

    Ok(PreparedChat {
        llm_request,
        limited_messages: linearized_msgs,
        rag_results: has_rag_results.in_json,
    })
}

fn adapt_sampling_for_reasoning_models(
    sampling_parameters: &mut SamplingParameters,
    model_record: &ChatModelRecord,
) {
    let user_set_max_tokens = sampling_parameters.max_new_tokens > 0;

    if !user_set_max_tokens {
        sampling_parameters.max_new_tokens = model_record.default_max_tokens
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

    let Some(ref supports_reasoning) = model_record.supports_reasoning else {
        sampling_parameters.reasoning_effort = None;
        sampling_parameters.thinking = None;
        sampling_parameters.enable_thinking = None;
        return;
    };

    match supports_reasoning.as_ref() {
        "openai" => {
            if sampling_parameters.reasoning_effort.is_none()
                && model_record.supports_boost_reasoning
                && sampling_parameters.boost_reasoning
            {
                sampling_parameters.reasoning_effort = Some(ReasoningEffort::Medium);
            }
            if !user_set_max_tokens && sampling_parameters.max_new_tokens <= 8192 {
                sampling_parameters.max_new_tokens *= 2;
            }
            sampling_parameters.thinking = None;
            sampling_parameters.enable_thinking = None;
        }
        "anthropic_budget" => {
            if sampling_parameters.thinking_budget.is_none()
                && model_record.supports_boost_reasoning
                && sampling_parameters.boost_reasoning
            {
                let min_budget = tokens().min_budget_tokens;
                let budget = if sampling_parameters.max_new_tokens > min_budget {
                    (sampling_parameters.max_new_tokens / 2).max(min_budget)
                } else {
                    min_budget
                };
                sampling_parameters.thinking_budget = Some(budget);
            }
            sampling_parameters.reasoning_effort = None;
            sampling_parameters.thinking = None;
            sampling_parameters.enable_thinking = None;
        }
        "anthropic_effort" => {
            if sampling_parameters.reasoning_effort.is_none()
                && model_record.supports_boost_reasoning
                && sampling_parameters.boost_reasoning
            {
                sampling_parameters.reasoning_effort = Some(ReasoningEffort::High);
            }
            sampling_parameters.thinking = None;
            sampling_parameters.enable_thinking = None;
        }
        "qwen" => {
            sampling_parameters.enable_thinking =
                Some(model_record.supports_boost_reasoning && sampling_parameters.boost_reasoning);
            sampling_parameters.reasoning_effort = None;
            sampling_parameters.thinking = None;
        }
        "deepseek" => {
            if !user_set_max_tokens && sampling_parameters.max_new_tokens <= 8192 {
                sampling_parameters.max_new_tokens *= 2;
            }
            sampling_parameters.reasoning_effort = None;
            sampling_parameters.thinking = None;
            sampling_parameters.enable_thinking = None;
        }
        _ => {
            sampling_parameters.reasoning_effort = None;
            sampling_parameters.thinking = None;
            sampling_parameters.enable_thinking = None;
        }
    };
}

fn sampling_params_to_reasoning_intent(
    sampling_parameters: &SamplingParameters,
    model_record: &ChatModelRecord,
) -> ReasoningIntent {
    // If model doesn't support reasoning, return Off
    let Some(ref reasoning_type) = model_record.supports_reasoning else {
        return ReasoningIntent::Off;
    };

    // DeepSeek handles reasoning automatically in responses - never send reasoning_effort
    // This prevents accidentally sending OpenAI-style params that DeepSeek may reject
    if reasoning_type == "deepseek" {
        return ReasoningIntent::Off;
    }

    // Check thinking_budget (from frontend UI) — explicit tokens always win
    if let Some(budget) = sampling_parameters.thinking_budget {
        return ReasoningIntent::BudgetTokens(budget);
    }

    // Check reasoning_effort (OpenAI-style or Anthropic effort)
    if let Some(ref effort) = sampling_parameters.reasoning_effort {
        return match effort {
            ReasoningEffort::Low => ReasoningIntent::Low,
            ReasoningEffort::Medium => ReasoningIntent::Medium,
            ReasoningEffort::High => ReasoningIntent::High,
        };
    }

    // Check Anthropic-style thinking with budget_tokens (legacy API)
    if let Some(ref thinking) = sampling_parameters.thinking {
        if thinking.get("type").and_then(|t| t.as_str()) == Some("enabled") {
            if let Some(budget) = thinking.get("budget_tokens").and_then(|b| b.as_u64()) {
                return ReasoningIntent::BudgetTokens(budget as usize);
            }
            return ReasoningIntent::Medium;
        }
    }

    // Check Qwen-style enable_thinking
    if sampling_parameters.enable_thinking == Some(true) {
        return ReasoningIntent::Medium;
    }

    // Check boost_reasoning flag (only for providers that support it via API params)
    // DeepSeek is already handled above
    if sampling_parameters.boost_reasoning && model_record.supports_boost_reasoning {
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
    if model_record.supports_reasoning.is_none() || !is_thinking_enabled(sampling_parameters) {
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

    fn make_model_record(supports_reasoning: Option<&str>) -> ChatModelRecord {
        ChatModelRecord {
            base: Default::default(),
            default_temperature: Some(0.7),
            supports_reasoning: supports_reasoning.map(|s| s.to_string()),
            supports_boost_reasoning: true,
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
        let model = make_model_record(None);
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
        let model = make_model_record(Some("anthropic_budget"));
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
        let model = make_model_record(Some("anthropic_budget"));
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
        let model = make_model_record(None);
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
    fn test_adapt_sampling_openai_boost_reasoning() {
        let mut params = make_sampling_params();
        params.boost_reasoning = true;
        params.temperature = None;
        let model = make_model_record(Some("openai"));
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.reasoning_effort, Some(ReasoningEffort::Medium));
        assert_eq!(params.temperature, Some(0.7)); // Filled from model default_temperature
    }

    #[test]
    fn test_adapt_sampling_openai_preserves_user_temperature() {
        let mut params = make_sampling_params();
        params.boost_reasoning = true;
        params.temperature = Some(0.3); // User explicitly set temperature
        let model = make_model_record(Some("openai"));
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.reasoning_effort, Some(ReasoningEffort::Medium));
        assert_eq!(params.temperature, Some(0.3)); // User value preserved
    }

    #[test]
    fn test_adapt_sampling_openai_reasoning_effort_takes_precedence() {
        let mut params = make_sampling_params();
        params.boost_reasoning = true;
        params.reasoning_effort = Some(ReasoningEffort::High); // User set reasoning_effort
        let model = make_model_record(Some("openai"));
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.reasoning_effort, Some(ReasoningEffort::High)); // User value preserved, not overwritten to Medium
    }

    #[test]
    fn test_adapt_sampling_openai_doubles_tokens_when_default() {
        let mut params = make_sampling_params();
        params.max_new_tokens = 0;
        let mut model = make_model_record(Some("openai"));
        model.default_max_tokens = Some(4096);
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.max_new_tokens, 8192);
    }

    #[test]
    fn test_adapt_sampling_openai_no_double_when_user_set() {
        let mut params = make_sampling_params();
        params.max_new_tokens = 4096;
        let model = make_model_record(Some("openai"));
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.max_new_tokens, 4096);
    }

    #[test]
    fn test_adapt_sampling_openai_no_double_above_8192() {
        let mut params = make_sampling_params();
        params.max_new_tokens = 16384;
        let model = make_model_record(Some("openai"));
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.max_new_tokens, 16384);
    }

    #[test]
    fn test_adapt_sampling_anthropic_budget_boost_reasoning() {
        let mut params = make_sampling_params();
        params.boost_reasoning = true;
        params.max_new_tokens = 4096;
        let model = make_model_record(Some("anthropic_budget"));
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert!(params.thinking_budget.is_some());
        assert!(params.thinking_budget.unwrap() > 0);
        assert!(params.reasoning_effort.is_none());
        assert!(params.thinking.is_none());
        assert!(params.enable_thinking.is_none());
    }

    #[test]
    fn test_adapt_sampling_anthropic_budget_explicit_budget_preserved() {
        let mut params = make_sampling_params();
        params.thinking_budget = Some(5000);
        let model = make_model_record(Some("anthropic_budget"));
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.thinking_budget, Some(5000));
        assert!(params.reasoning_effort.is_none());
        assert!(params.thinking.is_none());
    }

    #[test]
    fn test_adapt_sampling_anthropic_budget_no_boost_no_budget() {
        let mut params = make_sampling_params();
        let model = make_model_record(Some("anthropic_budget"));
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert!(params.thinking_budget.is_none());
        assert!(params.reasoning_effort.is_none());
    }

    #[test]
    fn test_adapt_sampling_anthropic_effort_boost_reasoning() {
        let mut params = make_sampling_params();
        params.boost_reasoning = true;
        let model = make_model_record(Some("anthropic_effort"));
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.reasoning_effort, Some(ReasoningEffort::Medium));
        assert!(params.thinking.is_none());
        assert!(params.enable_thinking.is_none());
    }

    #[test]
    fn test_adapt_sampling_anthropic_effort_preserves_reasoning_effort() {
        let mut params = make_sampling_params();
        params.reasoning_effort = Some(ReasoningEffort::High);
        let model = make_model_record(Some("anthropic_effort"));
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.reasoning_effort, Some(ReasoningEffort::High));
    }

    #[test]
    fn test_adapt_sampling_qwen_enable_thinking() {
        let mut params = make_sampling_params();
        params.boost_reasoning = true;
        params.temperature = None;
        let model = make_model_record(Some("qwen"));
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.enable_thinking, Some(true));
        assert_eq!(params.temperature, Some(0.7)); // Filled from model default_temperature
    }

    #[test]
    fn test_adapt_sampling_qwen_preserves_user_temperature() {
        let mut params = make_sampling_params();
        params.boost_reasoning = true;
        params.temperature = Some(0.5); // User explicitly set temperature
        let model = make_model_record(Some("qwen"));
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.enable_thinking, Some(true));
        assert_eq!(params.temperature, Some(0.5)); // User value preserved
    }

    #[test]
    fn test_adapt_sampling_qwen_no_boost() {
        let mut params = make_sampling_params();
        params.boost_reasoning = false;
        let model = make_model_record(Some("qwen"));
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.enable_thinking, Some(false));
    }

    #[test]
    fn test_adapt_sampling_no_reasoning_clears_all() {
        let mut params = make_sampling_params();
        params.reasoning_effort = Some(ReasoningEffort::High);
        params.thinking = Some(serde_json::json!({"type": "enabled"}));
        params.enable_thinking = Some(true);
        let model = make_model_record(None);
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert!(params.reasoning_effort.is_none());
        assert!(params.thinking.is_none());
        assert!(params.enable_thinking.is_none());
    }

    #[test]
    fn test_adapt_sampling_unknown_provider() {
        let mut params = make_sampling_params();
        params.boost_reasoning = true;
        params.temperature = None;
        let model = make_model_record(Some("unknown_provider"));
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.temperature, Some(0.7)); // Filled from model default_temperature
        assert!(params.reasoning_effort.is_none());
    }

    #[test]
    fn test_adapt_sampling_deepseek_doubles_tokens_when_default() {
        let mut params = make_sampling_params();
        params.max_new_tokens = 0;
        params.temperature = None;
        let mut model = make_model_record(Some("deepseek"));
        model.default_max_tokens = Some(4096);
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.max_new_tokens, 8192);
        assert!(params.reasoning_effort.is_none());
        assert!(params.thinking.is_none());
        assert!(params.enable_thinking.is_none());
        assert_eq!(params.temperature, Some(0.7)); // Filled from model default_temperature
    }

    #[test]
    fn test_adapt_sampling_deepseek_no_double_when_user_set() {
        let mut params = make_sampling_params();
        params.max_new_tokens = 4096;
        let model = make_model_record(Some("deepseek"));
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.max_new_tokens, 4096);
    }

    #[test]
    fn test_adapt_sampling_deepseek_no_double_above_8192() {
        let mut params = make_sampling_params();
        params.max_new_tokens = 0;
        let mut model = make_model_record(Some("deepseek"));
        model.default_max_tokens = Some(16384);
        adapt_sampling_for_reasoning_models(&mut params, &model);
        assert_eq!(params.max_new_tokens, 16384);
    }

    #[test]
    fn test_deepseek_reasoning_intent_always_off() {
        // DeepSeek handles reasoning automatically - never send reasoning_effort to API
        let model = make_model_record(Some("deepseek"));

        // Even with boost_reasoning enabled, should return Off
        let mut params = make_sampling_params();
        params.boost_reasoning = true;
        let intent = sampling_params_to_reasoning_intent(&params, &model);
        assert_eq!(intent, ReasoningIntent::Off);

        // Even with reasoning_effort set (shouldn't happen but defensive)
        params.reasoning_effort = Some(ReasoningEffort::High);
        let intent = sampling_params_to_reasoning_intent(&params, &model);
        assert_eq!(intent, ReasoningIntent::Off);
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
