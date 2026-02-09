use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use tracing::{info, warn};
use uuid::Uuid;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{
    ChatContent, ChatMessage, ChatMeta, ChatUsage, SamplingParameters, is_agentic_mode_id,
};
use crate::global_context::GlobalContext;
use crate::llm::LlmRequest;
use crate::llm::params::CacheControl;
use crate::scratchpad_abstract::HasTokenizerAndEot;
use crate::constants::CHAT_TOP_N;
use crate::http::routers::v1::knowledge_enrichment::enrich_messages_with_knowledge;

use super::types::*;
use super::trajectories::{maybe_save_trajectory, check_external_reload_pending};
use super::tools::{process_tool_calls_once, ToolStepOutcome};
use super::prepare::{prepare_chat_passthrough, ChatPrepareOptions};
use super::prompts::prepend_the_right_system_prompt_and_maybe_more_initial_messages;
use super::stream_core::{run_llm_stream, StreamRunParams, StreamCollector, normalize_tool_call, ChoiceFinal};
use super::queue::inject_priority_messages_if_any;
use super::config::tokens;



pub async fn prepare_session_preamble_and_knowledge(
    gcx: Arc<ARwLock<GlobalContext>>,
    session_arc: Arc<AMutex<ChatSession>>,
) {
    let (thread, chat_id, has_system, has_project_context) = {
        let session = session_arc.lock().await;
        let has_sys = session.messages.first().map(|m| m.role == "system").unwrap_or(false);
        let has_proj = session.messages.iter().any(|m| {
            m.role == "context_file"
                && m.tool_call_id == crate::chat::system_context::PROJECT_CONTEXT_MARKER
        });
        (session.thread.clone(), session.chat_id.clone(), has_sys, has_proj)
    };

    let needs_preamble = !has_system || (!has_project_context && thread.include_project_info);

    if needs_preamble {
        let tools: Vec<crate::tools::tools_description::ToolDesc> =
            crate::tools::tools_list::get_tools_for_mode(gcx.clone(), &thread.mode, Some(&thread.model))
                .await
                .into_iter()
                .map(|tool| tool.tool_description())
                .collect();
        let tool_names: std::collections::HashSet<String> =
            tools.iter().map(|t| t.name.clone()).collect();

        let meta = ChatMeta {
            chat_id: chat_id.clone(),
            chat_mode: thread.mode.clone(),
            chat_remote: false,
            current_config_file: String::new(),
            context_tokens_cap: thread.context_tokens_cap,
            include_project_info: thread.include_project_info,
            request_attempt_id: Uuid::new_v4().to_string(),
        };

        let messages = {
            let session = session_arc.lock().await;
            session.messages.clone()
        };
        let mut has_rag_results = crate::scratchpads::scratchpad_utils::HasRagResults::new();
        let messages_with_preamble =
            prepend_the_right_system_prompt_and_maybe_more_initial_messages(
                gcx.clone(),
                messages,
                &meta,
                &thread.task_meta,
                &mut has_rag_results,
                tool_names,
                &thread.mode,
                &thread.model,
            )
            .await;

        let first_conv_idx = messages_with_preamble
            .iter()
            .position(|m| m.role == "user" || m.role == "assistant")
            .unwrap_or(messages_with_preamble.len());

        if first_conv_idx > 0 {
            let mut session = session_arc.lock().await;

            let mut system_insert_idx = 0;
            let mut context_insert_idx = session
                .messages
                .iter()
                .position(|m| m.role == "system")
                .map(|i| i + 1)
                .unwrap_or(0);

            let mut inserted = 0;
            for msg in messages_with_preamble.iter().take(first_conv_idx) {
                if msg.role == "assistant" {
                    continue;
                }
                if msg.role == "system"
                    && session.messages.first().map(|m| m.role == "system").unwrap_or(false)
                {
                    continue;
                }
                if msg.role == "cd_instruction"
                    && session.messages.iter().any(|m| m.role == "cd_instruction")
                {
                    continue;
                }
                if msg.role == "context_file"
                    && session
                        .messages
                        .iter()
                        .any(|m| m.role == "context_file" && m.tool_call_id == msg.tool_call_id)
                {
                    continue;
                }
                let insert_idx = if msg.role == "system" {
                    let idx = system_insert_idx;
                    system_insert_idx += 1;
                    context_insert_idx += 1;
                    idx
                } else {
                    let idx = context_insert_idx;
                    context_insert_idx += 1;
                    idx
                };
                session.insert_message(insert_idx, msg.clone());
                inserted += 1;
            }
            if inserted > 0 {
                info!("Saved {} preamble messages to session", inserted);
            }
        }
    }

    // Knowledge enrichment for agentic mode
    let last_is_user = {
        let session = session_arc.lock().await;
        session.messages.last().map(|m| m.role == "user").unwrap_or(false)
    };
    if is_agentic_mode_id(&thread.mode) && last_is_user {
        let mut messages = {
            let session = session_arc.lock().await;
            session.messages.clone()
        };
        let msg_count_before = messages.len();
        enrich_messages_with_knowledge(gcx.clone(), &mut messages, Some(&chat_id)).await;
        if messages.len() > msg_count_before {
            let local_last_user_idx = messages.iter().rposition(|m| m.role == "user").unwrap_or(0);
            if local_last_user_idx > 0 {
                let enriched_msg = &messages[local_last_user_idx - 1];
                if enriched_msg.role == "context_file" {
                    let mut session = session_arc.lock().await;
                    let session_last_user_idx = session
                        .messages
                        .iter()
                        .rposition(|m| m.role == "user")
                        .unwrap_or(0);
                    session.insert_message(session_last_user_idx, enriched_msg.clone());
                    info!(
                        "Saved knowledge enrichment context_file to session at index {}",
                        session_last_user_idx
                    );
                }
            }
        }
    }
}

pub fn save_rag_results_to_session(
    session: &mut ChatSession,
    rag_results: &[serde_json::Value],
) {
    let last_user_idx = session.messages.iter().rposition(|m| m.role == "user");
    if let Some(insert_idx) = last_user_idx {
        let existing_content: std::collections::HashSet<String> = session.messages.iter()
            .filter(|m| m.role == "context_file" || m.role == "plain_text")
            .map(|m| m.content.content_text_only())
            .collect();
        let mut offset = 0;
        for rag_msg_json in rag_results {
            if let Ok(msg) = serde_json::from_value::<ChatMessage>(rag_msg_json.clone()) {
                if (msg.role == "context_file" || msg.role == "plain_text")
                    && !existing_content.contains(&msg.content.content_text_only())
                {
                    session.insert_message(insert_idx + offset, msg);
                    offset += 1;
                }
            }
        }
    }
}

fn tail_needs_assistant(messages: &[ChatMessage]) -> bool {
    let mut saw_toolish = false;

    for m in messages.iter().rev() {
        match m.role.as_str() {
            "assistant" => {
                if !saw_toolish {
                    return false;
                }
                let Some(tcs) = m.tool_calls.as_ref() else {
                    return false;
                };
                if tcs.is_empty() {
                    return false;
                }
                return tcs.iter().any(|tc| !tc.id.starts_with("srvtoolu_"));
            }
            "user" => return true,
            "tool" | "context_file" => saw_toolish = true,
            _ => {}
        }
    }

    false
}

pub fn start_generation(
    gcx: Arc<ARwLock<GlobalContext>>,
    session_arc: Arc<AMutex<ChatSession>>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        loop {
            let (thread, chat_id) = {
                let session = session_arc.lock().await;
                (
                    session.thread.clone(),
                    session.chat_id.clone(),
                )
            };

            let abort_flag = {
                let mut session = session_arc.lock().await;
                match session.start_stream() {
                    Some((_message_id, abort_flag)) => abort_flag,
                    None => {
                        warn!(
                            "Cannot start generation for {}: already generating",
                            chat_id
                        );
                        break;
                    }
                }
            };

            let generation_result = run_llm_generation(
                gcx.clone(),
                session_arc.clone(),
                thread,
                chat_id.clone(),
                abort_flag.clone(),
            )
            .await;

            if let Err(e) = generation_result {
                let task_meta_opt = {
                    let mut session = session_arc.lock().await;
                    if !session.abort_flag.load(Ordering::SeqCst) {
                        session.finish_stream_with_error(e);
                    }
                    session.thread.task_meta.clone()
                };

                if let Some(task_meta) = task_meta_opt {
                    let error_msg = {
                        let session = session_arc.lock().await;
                        session.task_agent_error.clone()
                    };
                    if let Some(error) = error_msg {
                        super::task_agent_monitor::handle_agent_streaming_error(
                            gcx.clone(),
                            &task_meta,
                            &error,
                        )
                        .await;
                    }
                }
                break;
            }

            if abort_flag.load(Ordering::SeqCst) {
                break;
            }

            maybe_save_trajectory(gcx.clone(), session_arc.clone()).await;

            let (mode_id, model_id) = {
                let session = session_arc.lock().await;
                (session.thread.mode.clone(), session.thread.model.clone())
            };

            match process_tool_calls_once(gcx.clone(), session_arc.clone(), &mode_id, Some(&model_id)).await {
                ToolStepOutcome::NoToolCalls => {
                    if inject_priority_messages_if_any(gcx.clone(), session_arc.clone()).await {
                        continue;
                    }
                    let should_continue = {
                        let session = session_arc.lock().await;
                        tail_needs_assistant(&session.messages)
                    };
                    if should_continue {
                        continue;
                    }
                    break;
                }
                ToolStepOutcome::Paused => break,
                ToolStepOutcome::Stop => break,
                ToolStepOutcome::Continue => {
                    inject_priority_messages_if_any(gcx.clone(), session_arc.clone()).await;
                }
            }
        }

        check_external_reload_pending(gcx.clone(), session_arc.clone()).await;

        {
            let session = session_arc.lock().await;
            session.abort_flag.store(false, Ordering::SeqCst);
            session.queue_notify.notify_one();
        }
    })
}

pub async fn run_llm_generation(
    gcx: Arc<ARwLock<GlobalContext>>,
    session_arc: Arc<AMutex<ChatSession>>,
    thread: ThreadParams,
    chat_id: String,
    abort_flag: Arc<AtomicBool>,
) -> Result<(), String> {
    let tools: Vec<crate::tools::tools_description::ToolDesc> =
        crate::tools::tools_list::get_tools_for_mode(gcx.clone(), &thread.mode, Some(&thread.model))
            .await
            .into_iter()
            .map(|tool| tool.tool_description())
            .collect();

    info!("session generation: tools count = {}", tools.len());

    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| e.message)?;
    let model_rec = crate::caps::resolve_chat_model(caps.clone(), &thread.model)?;

    let model_n_ctx = if model_rec.base.n_ctx > 0 {
        model_rec.base.n_ctx
    } else {
        tokens().default_n_ctx
    };
    let effective_n_ctx = match thread.context_tokens_cap {
        Some(cap) if cap > 0 => cap.min(model_n_ctx),
        _ => model_n_ctx,
    };
    let tokenizer_arc = crate::tokens::cached_tokenizer(gcx.clone(), &model_rec.base).await?;
    let t = HasTokenizerAndEot::new(tokenizer_arc);

    let meta = ChatMeta {
        chat_id: chat_id.clone(),
        chat_mode: thread.mode.clone(),
        chat_remote: false,
        current_config_file: String::new(),
        context_tokens_cap: thread.context_tokens_cap,
        include_project_info: thread.include_project_info,
        request_attempt_id: Uuid::new_v4().to_string(),
    };

    let messages = {
        let session = session_arc.lock().await;
        session.messages.clone()
    };
    let model_type_defaults = caps.user_defaults.defaults_for_model(
        &model_rec.base.id,
        &caps.defaults.chat_default_model,
        &caps.defaults.chat_light_model,
        &caps.defaults.chat_thinking_model,
    );
    let mut parameters = SamplingParameters {
        temperature: thread.temperature
            .or(model_type_defaults.temperature),
        frequency_penalty: thread.frequency_penalty,
        max_new_tokens: thread.max_tokens
            .or(model_type_defaults.max_new_tokens)
            .unwrap_or(0),
        boost_reasoning: thread.boost_reasoning
            .unwrap_or_else(|| model_type_defaults.boost_reasoning.unwrap_or(false)),
        reasoning_effort: thread.reasoning_effort.as_ref().and_then(|s| {
            match s.as_str() {
                "low" => Some(crate::call_validation::ReasoningEffort::Low),
                "medium" => Some(crate::call_validation::ReasoningEffort::Medium),
                "high" => Some(crate::call_validation::ReasoningEffort::High),
                _ => None,
            }
        }).or_else(|| {
            model_type_defaults.reasoning_effort.as_ref().and_then(|s| {
                match s.as_str() {
                    "low" => Some(crate::call_validation::ReasoningEffort::Low),
                    "medium" => Some(crate::call_validation::ReasoningEffort::Medium),
                    "high" => Some(crate::call_validation::ReasoningEffort::High),
                    _ => None,
                }
            })
        }),
        thinking_budget: thread.thinking_budget
            .or(model_type_defaults.thinking_budget),
        ..Default::default()
    };

    let ccx = AtCommandsContext::new(
        gcx.clone(),
        effective_n_ctx,
        CHAT_TOP_N,
        false,
        messages.clone(),
        chat_id.clone(),
        thread.root_chat_id.clone(),
        model_rec.base.id.clone(),
        thread.task_meta.clone(),
    )
    .await;
    let ccx_arc = Arc::new(AMutex::new(ccx));

    let options = ChatPrepareOptions {
        prepend_system_prompt: false,
        allow_at_commands: true,
        allow_tool_prerun: true,
        supports_tools: model_rec.supports_tools,
        parallel_tool_calls: thread.parallel_tool_calls,
        cache_control: CacheControl::Ephemeral,
        ..Default::default()
    };

    let prepared = prepare_chat_passthrough(
        gcx.clone(),
        ccx_arc.clone(),
        &t,
        messages,
        &model_rec.base.id,
        &thread.mode,
        tools,
        &meta,
        &mut parameters,
        &options,
    )
    .await?;

    {
        let mut session = session_arc.lock().await;
        session.last_prompt_messages = prepared.limited_messages.clone();
        save_rag_results_to_session(&mut session, &prepared.rag_results);
    }

    run_streaming_generation(
        gcx,
        session_arc,
        prepared.llm_request,
        &model_rec,
        abort_flag,
    )
    .await
}

async fn run_streaming_generation(
    gcx: Arc<ARwLock<GlobalContext>>,
    session_arc: Arc<AMutex<ChatSession>>,
    mut llm_request: LlmRequest,
    model_rec: &crate::caps::ChatModelRecord,
    abort_flag: Arc<AtomicBool>,
) -> Result<(), String> {
    info!("session generation: model={}, messages={}", llm_request.model_id, llm_request.messages.len());

    const TEMPERATURE_BUMP: f32 = 0.1;
    const MAX_RETRY_TEMPERATURE: f32 = 0.5;
    let user_specified_temp = llm_request.params.temperature;
    let model_supports_temperature = model_rec.supports_reasoning.is_none();
    let can_retry_with_temp_bump = user_specified_temp.is_none() && model_supports_temperature;
    let max_attempts = if can_retry_with_temp_bump {
        (MAX_RETRY_TEMPERATURE / TEMPERATURE_BUMP).floor() as usize + 2
    } else {
        1
    };
    let mut attempt = 0;

    let result = loop {
        attempt += 1;
        if can_retry_with_temp_bump && attempt > 1 {
            let retry_temp = TEMPERATURE_BUMP * (attempt - 2) as f32;
            llm_request.params.temperature = Some(retry_temp.min(MAX_RETRY_TEMPERATURE));
        }

        let params = StreamRunParams {
            llm_request: llm_request.clone(),
            model_rec: model_rec.base.clone(),
            abort_flag: Some(abort_flag.clone()),
            supports_tools: model_rec.supports_tools,
            supports_reasoning: model_rec.supports_reasoning.is_some(),
            reasoning_type: model_rec.supports_reasoning.clone(),
            supports_temperature: model_rec.supports_temperature,
        };

        enum CollectorEvent {
            DeltaOps(Vec<DeltaOp>),
            Usage(ChatUsage),
        }

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<CollectorEvent>();

        struct SessionCollector {
            tx: tokio::sync::mpsc::UnboundedSender<CollectorEvent>,
        }

        impl StreamCollector for SessionCollector {
            fn on_delta_ops(&mut self, _choice_idx: usize, ops: Vec<DeltaOp>) {
                let _ = self.tx.send(CollectorEvent::DeltaOps(ops));
            }

            fn on_usage(&mut self, usage: &ChatUsage) {
                let _ = self.tx.send(CollectorEvent::Usage(usage.clone()));
            }

            fn on_finish(&mut self, _choice_idx: usize, _finish_reason: Option<String>) {}
        }

        let mut collector = SessionCollector { tx };

        let session_arc_emitter = session_arc.clone();
        let emitter_task = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let mut session = session_arc_emitter.lock().await;
                match event {
                    CollectorEvent::DeltaOps(ops) => {
                        session.emit_stream_delta(ops);
                    }
                    CollectorEvent::Usage(usage) => {
                        session.draft_usage = Some(usage);
                    }
                }
            }
        });

        let results = run_llm_stream(gcx.clone(), params, &mut collector).await;
        drop(collector);
        let _ = emitter_task.await;
        let results = results?;

        let result = results.into_iter().next().unwrap_or_default();

        if is_result_empty(&result) {
            if attempt < max_attempts && can_retry_with_temp_bump {
                let current_temp_display = if attempt == 1 {
                    "default".to_string()
                } else {
                    format!("{:.1}", TEMPERATURE_BUMP * (attempt - 2) as f32)
                };
                let next_temp = (TEMPERATURE_BUMP * (attempt - 1) as f32).min(MAX_RETRY_TEMPERATURE);
                warn!(
                    "Empty assistant response at T={}, retrying with T={:.1} (attempt {}/{})",
                    current_temp_display, next_temp, attempt, max_attempts
                );
                {
                    let mut session = session_arc.lock().await;
                    if let Some(ref mut draft) = session.draft_message {
                        draft.content = ChatContent::SimpleText(String::new());
                        draft.tool_calls = None;
                        draft.reasoning_content = None;
                        draft.thinking_blocks = None;
                        draft.citations = Vec::new();
                        draft.extra = serde_json::Map::new();
                    }
                    session.draft_usage = None;
                }
                continue;
            } else {
                let effective_temp = llm_request.params.temperature.unwrap_or(0.0);
                return Err(format!(
                    "Empty assistant response after {} attempts (T={:.1})",
                    max_attempts, effective_temp
                ));
            }
        }

        if !result.tool_calls_raw.is_empty() {
            let parsed: Vec<_> = result.tool_calls_raw.iter().filter_map(|tc| normalize_tool_call(tc)).collect();
            if parsed.is_empty() {
                return Err("Model returned tool_calls but none were parsable".to_string());
            }
        }

        break result;
    };

    let (model_id, usage_for_pricing) = {
        let session = session_arc.lock().await;
        (session.thread.model.clone(), session.draft_usage.clone())
    };
    let metering_usd = if let Some(ref usage) = usage_for_pricing {
        if let Some(pricing) = get_model_pricing(&gcx, &model_id).await {
            crate::providers::pricing::compute_cost(usage, &pricing)
        } else {
            None
        }
    } else {
        None
    };

    {
        let mut session = session_arc.lock().await;
        if let Some(ref mut draft) = session.draft_message {
            draft.content = ChatContent::SimpleText(result.content);

            if !result.tool_calls_raw.is_empty() {
                info!(
                    "Parsing {} accumulated tool calls",
                    result.tool_calls_raw.len()
                );
                let parsed: Vec<_> = result
                    .tool_calls_raw
                    .iter()
                    .filter_map(|tc| normalize_tool_call(tc))
                    .collect();
                info!("Successfully parsed {} tool calls", parsed.len());
                if !parsed.is_empty() {
                    draft.tool_calls = Some(parsed);
                }
            }

            if !result.reasoning.is_empty() {
                draft.reasoning_content = Some(result.reasoning);
            }
            if !result.thinking_blocks.is_empty() {
                draft.thinking_blocks = Some(result.thinking_blocks);
            }
            if !result.citations.is_empty() {
                draft.citations = result.citations;
            }
            if !result.extra.is_empty() {
                draft.extra = result.extra;
            }
        }

        if let Some(ref mut usage) = session.draft_usage {
            usage.metering_usd = metering_usd;
        }

        session.finish_stream(result.finish_reason);
    }

    Ok(())
}

async fn get_model_pricing(
    gcx: &Arc<ARwLock<GlobalContext>>,
    model_id: &str,
) -> Option<crate::providers::traits::ModelPricing> {
    let parts: Vec<&str> = model_id.splitn(2, '/').collect();
    if parts.len() != 2 {
        return None;
    }
    let provider_name = parts[0];
    let model_name = parts[1];

    let gcx_locked = gcx.read().await;
    let registry = gcx_locked.providers.read().await;

    if let Some(provider) = registry.get(provider_name) {
        return provider.model_pricing(model_name);
    }

    None
}

fn is_result_empty(result: &ChoiceFinal) -> bool {
    result.content.trim().is_empty()
        && result.tool_calls_raw.is_empty()
        && result.reasoning.trim().is_empty()
        && result.thinking_blocks.is_empty()
        && result.citations.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::{ChatToolCall, ChatToolFunction};

    fn make_user_msg(content: &str) -> ChatMessage {
        ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn make_assistant_msg(content: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn make_assistant_with_tool_call(tool_call_id: &str, tool_name: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("".to_string()),
            tool_calls: Some(vec![ChatToolCall {
                id: tool_call_id.to_string(),
                index: Some(0),
                function: ChatToolFunction {
                    name: tool_name.to_string(),
                    arguments: "{}".to_string(),
                },
                tool_type: "function".to_string(),
            }]),
            ..Default::default()
        }
    }

    fn make_tool_msg(tool_call_id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            tool_call_id: tool_call_id.to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn make_context_file_msg() -> ChatMessage {
        ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::SimpleText("file content".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn test_tail_needs_assistant_ends_with_assistant_no_tools() {
        let messages = vec![make_user_msg("hello"), make_assistant_msg("response")];
        assert!(!tail_needs_assistant(&messages));
    }

    #[test]
    fn test_tail_needs_assistant_ends_with_user() {
        let messages = vec![make_user_msg("hello")];
        assert!(tail_needs_assistant(&messages));
    }

    #[test]
    fn test_tail_needs_assistant_ends_with_tool_from_client() {
        let messages = vec![
            make_user_msg("hello"),
            make_assistant_with_tool_call("call_123", "cat"),
            make_tool_msg("call_123", "file content"),
        ];
        assert!(tail_needs_assistant(&messages));
    }

    #[test]
    fn test_tail_needs_assistant_ends_with_tool_from_server() {
        let messages = vec![
            make_user_msg("hello"),
            make_assistant_with_tool_call("srvtoolu_123", "web_search"),
            make_tool_msg("srvtoolu_123", "search results"),
        ];
        assert!(!tail_needs_assistant(&messages));
    }

    #[test]
    fn test_tail_needs_assistant_empty_assistant_discarded() {
        let messages = vec![
            make_user_msg("hello"),
            make_assistant_with_tool_call("call_123", "cat"),
            make_tool_msg("call_123", "file content"),
        ];
        assert!(tail_needs_assistant(&messages));
    }

    #[test]
    fn test_tail_needs_assistant_context_file_after_tool() {
        let messages = vec![
            make_user_msg("hello"),
            make_assistant_with_tool_call("call_123", "cat"),
            make_tool_msg("call_123", "file content"),
            make_context_file_msg(),
        ];
        assert!(tail_needs_assistant(&messages));
    }

    #[test]
    fn test_tail_needs_assistant_multiple_tool_calls_mixed() {
        let messages = vec![
            make_user_msg("hello"),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![
                    ChatToolCall {
                        id: "call_123".to_string(),
                        index: Some(0),
                        function: ChatToolFunction {
                            name: "cat".to_string(),
                            arguments: "{}".to_string(),
                        },
                        tool_type: "function".to_string(),
                    },
                    ChatToolCall {
                        id: "srvtoolu_456".to_string(),
                        index: Some(1),
                        function: ChatToolFunction {
                            name: "web_search".to_string(),
                            arguments: "{}".to_string(),
                        },
                        tool_type: "function".to_string(),
                    },
                ]),
                ..Default::default()
            },
            make_tool_msg("call_123", "file content"),
            make_tool_msg("srvtoolu_456", "search results"),
        ];
        assert!(tail_needs_assistant(&messages));
    }

    #[test]
    fn test_tail_needs_assistant_only_server_tools() {
        let messages = vec![
            make_user_msg("hello"),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("".to_string()),
                tool_calls: Some(vec![
                    ChatToolCall {
                        id: "srvtoolu_123".to_string(),
                        index: Some(0),
                        function: ChatToolFunction {
                            name: "web_search".to_string(),
                            arguments: "{}".to_string(),
                        },
                        tool_type: "function".to_string(),
                    },
                    ChatToolCall {
                        id: "srvtoolu_456".to_string(),
                        index: Some(1),
                        function: ChatToolFunction {
                            name: "web_search".to_string(),
                            arguments: "{}".to_string(),
                        },
                        tool_type: "function".to_string(),
                    },
                ]),
                ..Default::default()
            },
            make_tool_msg("srvtoolu_123", "search results 1"),
            make_tool_msg("srvtoolu_456", "search results 2"),
        ];
        assert!(!tail_needs_assistant(&messages));
    }

    #[test]
    fn test_tail_needs_assistant_empty_messages() {
        let messages: Vec<ChatMessage> = vec![];
        assert!(!tail_needs_assistant(&messages));
    }

    #[test]
    fn test_tail_needs_assistant_assistant_with_empty_tool_calls() {
        let messages = vec![
            make_user_msg("hello"),
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("response".to_string()),
                tool_calls: Some(vec![]),
                ..Default::default()
            },
        ];
        assert!(!tail_needs_assistant(&messages));
    }
}
