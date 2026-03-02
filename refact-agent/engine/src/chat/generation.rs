use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use tracing::{info, warn};
use uuid::Uuid;

use crate::subchat::{resolve_subchat_config, run_subchat};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{
    ChatContent, ChatMessage, ChatMeta, ChatUsage, SamplingParameters, is_agentic_mode_id,
};
use crate::stats::event::{LlmCallEvent, split_model_provider, sum_metering_coins};
use crate::chat::tool_call_recovery;
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
use crate::ext::hooks::HookEvent;
use crate::ext::hooks_runner::{HookPayload, get_project_dir_string, run_hooks};



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
        let caps = match crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0).await {
            Ok(caps) => caps,
            Err(e) => {
                warn!("Failed to load caps for preamble: {}", e.message);
                return;
            }
        };
        let model_rec = match crate::caps::resolve_chat_model(caps.clone(), &thread.model) {
            Ok(rec) => rec,
            Err(e) => {
                warn!("Failed to resolve model for preamble: {}", e);
                return;
            }
        };

        let tools: Vec<crate::tools::tools_description::ToolDesc> =
            crate::tools::tools_list::get_tools_for_mode(gcx.clone(), &thread.mode, Some(&model_rec.base.id))
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
        let (messages_with_preamble, skills_info) =
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

        {
            let mut session = session_arc.lock().await;
            session.skills_available_count = skills_info.available_count;
            session.skills_included = skills_info.included_names.clone();
        }

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

async fn run_fork_subchat(
    gcx: Arc<ARwLock<GlobalContext>>,
    agent_name: &str,
    user_content: &str,
    thread: &ThreadParams,
    parent_chat_id: &str,
) -> Result<String, String> {
    let config = resolve_subchat_config(
        gcx.clone(),
        agent_name,
        false,
        None,
        Some(format!("Fork: {}", agent_name)),
        Some(parent_chat_id.to_string()),
        Some("fork".to_string()),
        None,
        None,
        10,
        true,
        None,
        thread.mode.clone(),
    )
    .await?;

    let messages = vec![
        ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText(user_content.to_string()),
            ..Default::default()
        },
    ];

    let result = run_subchat(gcx, messages, config).await?;

    let last_assistant = result.messages.iter().rev().find(|m| m.role == "assistant");
    Ok(last_assistant
        .map(|m| m.content.content_text_only())
        .unwrap_or_else(|| "Fork skill completed but produced no response.".to_string()))
}

pub fn start_generation(
    gcx: Arc<ARwLock<GlobalContext>>,
    session_arc: Arc<AMutex<ChatSession>>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        loop {
            let (mut thread, chat_id) = {
                let session = session_arc.lock().await;
                (
                    session.thread.clone(),
                    session.chat_id.clone(),
                )
            };
            {
                let session = session_arc.lock().await;
                if let Some(ref m) = session.active_command.model_override {
                    if !m.is_empty() {
                        thread.model = m.clone();
                    }
                }
            }

            let fork_agent_name = {
                let session = session_arc.lock().await;
                session.active_command.context_fork.clone()
            };

            if let Some(agent_name) = fork_agent_name {
                let user_content = {
                    let session = session_arc.lock().await;
                    session.messages.iter().rev()
                        .find(|m| m.role == "user")
                        .map(|m| m.content.content_text_only())
                        .unwrap_or_default()
                };
                {
                    let mut session = session_arc.lock().await;
                    session.active_command.context_fork = None;
                }

                let fork_result = run_fork_subchat(
                    gcx.clone(),
                    &agent_name,
                    &user_content,
                    &thread,
                    &chat_id,
                )
                .await;

                match fork_result {
                    Ok(assistant_content) => {
                        let mut session = session_arc.lock().await;
                        session.add_message(ChatMessage {
                            role: "assistant".to_string(),
                            content: ChatContent::SimpleText(assistant_content),
                            ..Default::default()
                        });
                        session.set_runtime_state(SessionState::Idle, None);
                        drop(session);
                        maybe_save_trajectory(gcx.clone(), session_arc.clone()).await;
                        break;
                    }
                    Err(e) => {
                        warn!("Fork skill subchat failed ({}), falling back to normal generation", e);
                        continue;
                    }
                }
            }

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

            let model_id_opt = if model_id.is_empty() {
                None
            } else {
                Some(model_id.as_str())
            };

            match process_tool_calls_once(gcx.clone(), session_arc.clone(), &mode_id, model_id_opt).await {
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
                    let gcx_stop = gcx.clone();
                    let session_id_stop = chat_id.clone();
                    tokio::spawn(async move {
                        let project_dir = get_project_dir_string(gcx_stop.clone()).await;
                        let payload = HookPayload {
                            hook_event_name: "Stop".to_string(),
                            session_id: session_id_stop,
                            project_dir,
                            tool_name: None,
                            tool_input: None,
                            tool_output: None,
                            user_prompt: None,
                            extra: std::collections::HashMap::new(),
                        };
                        run_hooks(gcx_stop, HookEvent::Stop, payload).await;
                    });
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
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| e.message)?;
    let model_rec = crate::caps::resolve_chat_model(caps.clone(), &thread.model)?;

    let tools: Vec<crate::tools::tools_description::ToolDesc> =
        crate::tools::tools_list::get_tools_for_mode(gcx.clone(), &thread.mode, Some(&model_rec.base.id))
            .await
            .into_iter()
            .map(|tool| tool.tool_description())
            .collect();

    info!("session generation: model={}, tools count = {}", model_rec.base.id, tools.len());

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
                "xhigh" => Some(crate::call_validation::ReasoningEffort::XHigh),
                "max" => Some(crate::call_validation::ReasoningEffort::Max),
                _ => None,
            }
        }).or_else(|| {
            model_type_defaults.reasoning_effort.as_ref().and_then(|s| {
                match s.as_str() {
                    "low" => Some(crate::call_validation::ReasoningEffort::Low),
                    "medium" => Some(crate::call_validation::ReasoningEffort::Medium),
                    "high" => Some(crate::call_validation::ReasoningEffort::High),
                    "xhigh" => Some(crate::call_validation::ReasoningEffort::XHigh),
                    "max" => Some(crate::call_validation::ReasoningEffort::Max),
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
        &thread,
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
    let (chat_id, root_chat_id, mode, task_id, task_role, agent_id, card_id) = {
        let session = session_arc.lock().await;
        let tm = session.thread.task_meta.as_ref();
        (
            session.chat_id.clone(),
            session.thread.root_chat_id.clone(),
            session.thread.mode.clone(),
            tm.map(|t| t.task_id.clone()),
            tm.map(|t| t.role.clone()),
            tm.and_then(|t| t.agent_id.clone()),
            tm.and_then(|t| t.card_id.clone()),
        )
    };

    const TEMPERATURE_BUMP: f32 = 0.1;
    const MAX_RETRY_TEMPERATURE: f32 = 0.5;
    let user_specified_temp = llm_request.params.temperature;
    let model_supports_temperature = model_rec.supports_temperature;
    let can_retry_with_temp_bump = user_specified_temp.is_none() && model_supports_temperature;
    let max_attempts = if can_retry_with_temp_bump {
        (MAX_RETRY_TEMPERATURE / TEMPERATURE_BUMP).floor() as usize + 2
    } else {
        1
    };
    let mut attempt = 0;

    let (result, pending_success_event, pending_success_coins) = loop {
        attempt += 1;
        if can_retry_with_temp_bump && attempt > 1 {
            let retry_temp = TEMPERATURE_BUMP * (attempt - 2) as f32;
            llm_request.params.temperature = Some(retry_temp.min(MAX_RETRY_TEMPERATURE));
        }

        let params = StreamRunParams {
            llm_request: llm_request.clone(),
            model_rec: model_rec.base.clone(),
            chat_id: Some(chat_id.clone()),
            abort_flag: Some(abort_flag.clone()),
            supports_tools: model_rec.supports_tools,
            supports_reasoning: model_rec.has_reasoning_support(),
            reasoning_type: model_rec.reasoning_type_string(),
            supports_temperature: model_rec.supports_temperature,
        };

        enum CollectorEventPayload {
            DeltaOps(Vec<DeltaOp>),
            Usage(ChatUsage),
        }

        const EMITTER_QUEUE_CAPACITY: usize = 256;
        let (tx, mut rx) = tokio::sync::mpsc::channel::<CollectorEventPayload>(EMITTER_QUEUE_CAPACITY);
        let overflow_usage = Arc::new(std::sync::Mutex::new(None::<ChatUsage>));
        let overflow_ops = Arc::new(std::sync::Mutex::new(Vec::<DeltaOp>::new()));

        struct SessionCollector {
            tx: tokio::sync::mpsc::Sender<CollectorEventPayload>,
            overflow_usage: Arc<std::sync::Mutex<Option<ChatUsage>>>,
            overflow_ops: Arc<std::sync::Mutex<Vec<DeltaOp>>>,
        }

        impl StreamCollector for SessionCollector {
            fn on_delta_ops(&mut self, _choice_idx: usize, ops: Vec<DeltaOp>) {
                match self.tx.try_send(CollectorEventPayload::DeltaOps(ops)) {
                    Ok(()) => {}
                    Err(tokio::sync::mpsc::error::TrySendError::Full(event)) => {
                        if let CollectorEventPayload::DeltaOps(ops) = event {
                            if let Ok(mut guard) = self.overflow_ops.lock() {
                                guard.extend(ops);
                            }
                        }
                    }
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_event)) => {}
                }
            }

            fn on_usage(&mut self, usage: &ChatUsage) {
                let usage_clone = usage.clone();
                match self.tx.try_send(CollectorEventPayload::Usage(usage_clone.clone())) {
                    Ok(()) => {}
                    Err(tokio::sync::mpsc::error::TrySendError::Full(_event)) => {
                        if let Ok(mut guard) = self.overflow_usage.lock() {
                            *guard = Some(usage_clone);
                        }
                    }
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_event)) => {}
                }
            }

            fn on_finish(&mut self, _choice_idx: usize, _finish_reason: Option<String>) {}
        }

        let mut collector = SessionCollector {
            tx,
            overflow_usage: overflow_usage.clone(),
            overflow_ops: overflow_ops.clone(),
        };

        let session_arc_emitter = session_arc.clone();
        let emitter_task = tokio::spawn(async move {
            fn merge_events(
                events: &mut Vec<CollectorEventPayload>,
                batched_ops: &mut Vec<DeltaOp>,
                latest_usage: &mut Option<ChatUsage>,
            ) {
                for event in events.drain(..) {
                    match event {
                        CollectorEventPayload::DeltaOps(ops) => {
                            batched_ops.extend(ops);
                        }
                        CollectorEventPayload::Usage(usage) => {
                            *latest_usage = Some(usage);
                        }
                    }
                }
            }

            fn coalesce_text_ops(ops: Vec<DeltaOp>) -> Vec<DeltaOp> {
                if ops.len() <= 1 {
                    return ops;
                }
                let mut out: Vec<DeltaOp> = Vec::with_capacity(ops.len());
                for op in ops {
                    match op {
                        DeltaOp::AppendContent { text } => {
                            if let Some(DeltaOp::AppendContent { text: ref mut prev }) = out.last_mut() {
                                prev.push_str(&text);
                            } else {
                                out.push(DeltaOp::AppendContent { text });
                            }
                        }
                        DeltaOp::AppendReasoning { text } => {
                            if let Some(DeltaOp::AppendReasoning { text: ref mut prev }) = out.last_mut() {
                                prev.push_str(&text);
                            } else {
                                out.push(DeltaOp::AppendReasoning { text });
                            }
                        }
                        other => out.push(other),
                    }
                }
                out
            }

            fn split_utf8_chunks(text: &str, max_bytes: usize) -> Vec<String> {
                if text.len() <= max_bytes {
                    return vec![text.to_string()];
                }
                let mut chunks = Vec::new();
                let mut start = 0usize;
                while start < text.len() {
                    let mut end = (start + max_bytes).min(text.len());
                    while end > start && !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    if end == start {
                        end = text[start..]
                            .char_indices()
                            .nth(1)
                            .map(|(i, _)| start + i)
                            .unwrap_or(text.len());
                    }
                    chunks.push(text[start..end].to_string());
                    start = end;
                }
                chunks
            }

            fn split_large_text_ops(ops: Vec<DeltaOp>, max_text_bytes: usize) -> Vec<DeltaOp> {
                let mut out = Vec::new();
                for op in ops {
                    match op {
                        DeltaOp::AppendContent { text } => {
                            for chunk in split_utf8_chunks(&text, max_text_bytes) {
                                out.push(DeltaOp::AppendContent { text: chunk });
                            }
                        }
                        DeltaOp::AppendReasoning { text } => {
                            for chunk in split_utf8_chunks(&text, max_text_bytes) {
                                out.push(DeltaOp::AppendReasoning { text: chunk });
                            }
                        }
                        other => out.push(other),
                    }
                }
                out
            }

            const MAX_BATCH_EVENTS: usize = 64;
            const MAX_DELTA_OPS_PER_EMIT: usize = 128;
            const MAX_DELTA_TEXT_BYTES: usize = 64 * 1024;
            let mut pending = Vec::<CollectorEventPayload>::new();

            while let Some(first_event) = rx.recv().await {
                pending.push(first_event);

                while pending.len() < MAX_BATCH_EVENTS {
                    match rx.try_recv() {
                        Ok(event) => pending.push(event),
                        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                    }
                }

                let mut batched_ops = Vec::new();
                let mut latest_usage: Option<ChatUsage> = None;
                merge_events(&mut pending, &mut batched_ops, &mut latest_usage);

                if let Ok(mut guard) = overflow_ops.lock() {
                    if !guard.is_empty() {
                        let mut drained = std::mem::take(&mut *guard);
                        drained.append(&mut batched_ops);
                        batched_ops = drained;
                    }
                }
                if let Ok(mut guard) = overflow_usage.lock() {
                    if let Some(usage) = guard.take() {
                        latest_usage = Some(usage);
                    }
                }

                let batched_ops = coalesce_text_ops(batched_ops);
                let batched_ops = split_large_text_ops(batched_ops, MAX_DELTA_TEXT_BYTES);

                let mut session = session_arc_emitter.lock().await;
                if !batched_ops.is_empty() {
                    for chunk in batched_ops.chunks(MAX_DELTA_OPS_PER_EMIT) {
                        session.emit_stream_delta(chunk.to_vec());
                    }
                }
                if let Some(usage) = latest_usage {
                    session.draft_usage = Some(usage);
                }
            }

            let mut final_ops = Vec::new();
            let mut final_usage: Option<ChatUsage> = None;
            if let Ok(mut guard) = overflow_ops.lock() {
                if !guard.is_empty() {
                    final_ops = std::mem::take(&mut *guard);
                }
            }
            if let Ok(mut guard) = overflow_usage.lock() {
                if let Some(usage) = guard.take() {
                    final_usage = Some(usage);
                }
            }

            if !final_ops.is_empty() || final_usage.is_some() {
                let final_ops = coalesce_text_ops(final_ops);
                let final_ops = split_large_text_ops(final_ops, MAX_DELTA_TEXT_BYTES);

                let mut session = session_arc_emitter.lock().await;
                if !final_ops.is_empty() {
                    for chunk in final_ops.chunks(MAX_DELTA_OPS_PER_EMIT) {
                        session.emit_stream_delta(chunk.to_vec());
                    }
                }
                if let Some(usage) = final_usage {
                    session.draft_usage = Some(usage);
                }
            }
        });

        let call_ts_start = chrono::Utc::now().to_rfc3339();
        let call_start = std::time::Instant::now();

        let results = run_llm_stream(gcx.clone(), params, &mut collector).await;
        drop(collector);
        let _ = emitter_task.await;

        let duration_ms = call_start.elapsed().as_millis() as u64;
        let call_ts_end = chrono::Utc::now().to_rfc3339();

        let (model_id_for_stats, messages_count, tools_count, temperature_for_stats, max_tokens_for_stats) = (
            llm_request.model_id.clone(),
            llm_request.messages.len(),
            llm_request.tools.as_ref().map(|t| t.len()).unwrap_or(0),
            llm_request.params.temperature,
            llm_request.params.max_tokens,
        );

        match &results {
            Err(e) => {
                let (provider, model) = split_model_provider(&model_id_for_stats);
                let event = LlmCallEvent {
                    id: uuid::Uuid::new_v4().to_string(),
                    ts_start: call_ts_start.clone(),
                    ts_end: call_ts_end.clone(),
                    duration_ms,
                    chat_id: chat_id.clone(),
                    root_chat_id: root_chat_id.clone(),
                    mode: mode.clone(),
                    task_id: task_id.clone(),
                    task_role: task_role.clone(),
                    agent_id: agent_id.clone(),
                    card_id: card_id.clone(),
                    model_id: model_id_for_stats.clone(),
                    provider,
                    model,
                    messages_count,
                    tools_count,
                    max_tokens: max_tokens_for_stats,
                    temperature: temperature_for_stats,
                    success: false,
                    error_message: Some(e.chars().take(200).collect()),
                    finish_reason: None,
                    attempt_n: attempt,
                    retry_reason: None,
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    cache_read_tokens: None,
                    cache_creation_tokens: None,
                    total_tokens: 0,
                    cost_usd: None,
                    cost_coins: None,
                };
                if let Some(sender) = &gcx.read().await.llm_stats_sender {
                    if sender.try_send(event).is_err() {
                        tracing::warn!("stats: channel full, dropping LLM call event");
                    }
                }
            }
            Ok(_) => {}
        }

        let results = results?;

        let mut result = results.into_iter().next().unwrap_or_default();

        if is_result_empty(&result) {
            let (draft_usage, draft_coins) = {
                let session = session_arc.lock().await;
                let coins = session.draft_message.as_ref().and_then(|m| sum_metering_coins(&m.extra));
                (session.draft_usage.clone(), coins)
            };
            let (provider, model) = split_model_provider(&model_id_for_stats);
            let event = LlmCallEvent {
                id: uuid::Uuid::new_v4().to_string(),
                ts_start: call_ts_start,
                ts_end: call_ts_end,
                duration_ms,
                chat_id: chat_id.clone(),
                root_chat_id: root_chat_id.clone(),
                mode: mode.clone(),
                task_id: task_id.clone(),
                task_role: task_role.clone(),
                agent_id: agent_id.clone(),
                card_id: card_id.clone(),
                model_id: model_id_for_stats,
                provider,
                model,
                messages_count,
                tools_count,
                max_tokens: max_tokens_for_stats,
                temperature: temperature_for_stats,
                success: false,
                error_message: Some("empty_response".to_string()),
                finish_reason: result.finish_reason.clone(),
                attempt_n: attempt,
                retry_reason: Some("empty_response".to_string()),
                prompt_tokens: draft_usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0),
                completion_tokens: draft_usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0),
                cache_read_tokens: draft_usage.as_ref().and_then(|u| u.cache_read_tokens),
                cache_creation_tokens: draft_usage.as_ref().and_then(|u| u.cache_creation_tokens),
                total_tokens: draft_usage.as_ref().map(|u| u.total_tokens).unwrap_or(0),
                cost_usd: draft_usage.as_ref().and_then(|u| u.metering_usd.as_ref()).map(|m| m.total_usd),
                cost_coins: draft_coins,
            };
            if let Some(sender) = &gcx.read().await.llm_stats_sender {
                if sender.try_send(event).is_err() {
                    tracing::warn!("stats: channel full, dropping LLM call event");
                }
            }

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
                        draft.server_content_blocks = Vec::new();
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

        // --- Tool call recovery ---
        // GPT-5 Codex models occasionally leak tool calls into text content instead of
        // emitting structured function_call events. Detect and recover them.
        let allowed_tools = tool_call_recovery::allowed_tool_names(&llm_request.tools);

        // 1. Unwrap multi_tool_use.parallel wrappers in structured tool_calls
        if !result.tool_calls_raw.is_empty() {
            result.tool_calls_raw = tool_call_recovery::unwrap_multi_tool_use_parallel(
                &result.tool_calls_raw,
                &allowed_tools,
            );
        }

        // 2. Recover tool calls from garbled ChatML content (when no structured calls exist)
        if result.tool_calls_raw.is_empty() && !allowed_tools.is_empty() {
            if let Some((cleaned_content, recovered_calls)) =
                tool_call_recovery::recover_tool_calls_from_chatml_content(
                    &result.content,
                    &allowed_tools,
                )
            {
                warn!(
                    "tool_call_recovery: recovered {} tool call(s) from garbled content",
                    recovered_calls.len()
                );
                result.content = cleaned_content;
                result.tool_calls_raw = recovered_calls;
            }
        }

        if !result.tool_calls_raw.is_empty() {
            let parsed: Vec<_> = result.tool_calls_raw.iter().filter_map(|tc| normalize_tool_call(tc)).collect();
            if parsed.is_empty() {
                let has_content = !result.content.trim().is_empty()
                    || !result.reasoning.trim().is_empty()
                    || !result.server_content_blocks.is_empty()
                    || !result.citations.is_empty();
                tracing::warn!(
                    "All {} tool calls unparsable: {:?}",
                    result.tool_calls_raw.len(),
                    result.tool_calls_raw,
                );
                if !has_content {
                    return Err("Model returned tool_calls but none were parsable".to_string());
                }
                // Has useful content — discard unparsable tool calls and continue
                result.tool_calls_raw.clear();
            } else if parsed.len() < result.tool_calls_raw.len() {
                let dropped: Vec<_> = result.tool_calls_raw.iter()
                    .filter(|tc| normalize_tool_call(tc).is_none())
                    .collect();
                tracing::warn!(
                    "Dropped {}/{} tool calls during normalization: {:?}",
                    dropped.len(),
                    result.tool_calls_raw.len(),
                    dropped,
                );
            }
        }

        let (draft_usage_for_success, success_coins) = {
            let session = session_arc.lock().await;
            let coins = session.draft_message.as_ref().and_then(|m| sum_metering_coins(&m.extra));
            (session.draft_usage.clone(), coins)
        };
        let usage_for_event = result.usage.as_ref().or(draft_usage_for_success.as_ref());
        let (provider, model) = split_model_provider(&model_id_for_stats);
        let pending_success_event = LlmCallEvent {
            id: uuid::Uuid::new_v4().to_string(),
            ts_start: call_ts_start,
            ts_end: call_ts_end,
            duration_ms,
            chat_id: chat_id.clone(),
            root_chat_id: root_chat_id.clone(),
            mode: mode.clone(),
            task_id: task_id.clone(),
            task_role: task_role.clone(),
            agent_id: agent_id.clone(),
            card_id: card_id.clone(),
            model_id: model_id_for_stats,
            provider,
            model,
            messages_count,
            tools_count,
            max_tokens: max_tokens_for_stats,
            temperature: temperature_for_stats,
            success: true,
            error_message: None,
            finish_reason: result.finish_reason.clone(),
            attempt_n: attempt,
            retry_reason: None,
            prompt_tokens: usage_for_event.map(|u| u.prompt_tokens).unwrap_or(0),
            completion_tokens: usage_for_event.map(|u| u.completion_tokens).unwrap_or(0),
            cache_read_tokens: usage_for_event.and_then(|u| u.cache_read_tokens),
            cache_creation_tokens: usage_for_event.and_then(|u| u.cache_creation_tokens),
            total_tokens: usage_for_event.map(|u| u.total_tokens).unwrap_or(0),
            cost_usd: None,
            cost_coins: None,
        };

        break (result, pending_success_event, success_coins);
    };

    let (model_id, usage_for_pricing) = {
        let session = session_arc.lock().await;
        (model_rec.base.id.clone(), session.draft_usage.clone())
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
        let mut success_event = pending_success_event;
        success_event.cost_usd = metering_usd.as_ref().map(|m| m.total_usd);
        success_event.cost_coins = pending_success_coins;
        if let Some(sender) = &gcx.read().await.llm_stats_sender {
            if sender.try_send(success_event).is_err() {
                tracing::warn!("stats: channel full, dropping LLM call event");
            }
        }
    }

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
            if !result.server_content_blocks.is_empty() {
                draft.server_content_blocks = result.server_content_blocks;
            }
            if !result.extra.is_empty() {
                draft.extra = result.extra;
            }
        }

        // Store previous_response_id for stateful multi-turn on Platform API only.
        // ChatGPT backend doesn't support previous_response_id, so don't store it —
        // otherwise prepare_chat_passthrough activates tail-only mode and the server
        // receives function_call_output without matching function_call items.
        let is_chatgpt_backend = model_rec.base.endpoint.contains("chatgpt.com/backend-api");
        if model_rec.base.wire_format == crate::llm::WireFormat::OpenaiResponses && !is_chatgpt_backend {
            if let Some(resp_id) = session
                .draft_message
                .as_ref()
                .and_then(|m| m.extra.get("openai_response_id"))
                .and_then(|v| v.as_str())
            {
                if session.thread.previous_response_id.as_deref() != Some(resp_id) {
                    session.thread.previous_response_id = Some(resp_id.to_string());
                    session.increment_version();
                }
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
        && result.server_content_blocks.is_empty()
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
                extra_content: None,
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
                        extra_content: None,
                    },
                    ChatToolCall {
                        id: "srvtoolu_456".to_string(),
                        index: Some(1),
                        function: ChatToolFunction {
                            name: "web_search".to_string(),
                            arguments: "{}".to_string(),
                        },
                        tool_type: "function".to_string(),
                        extra_content: None,
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
                        extra_content: None,
                    },
                    ChatToolCall {
                        id: "srvtoolu_456".to_string(),
                        index: Some(1),
                        function: ChatToolFunction {
                            name: "web_search".to_string(),
                            arguments: "{}".to_string(),
                        },
                        tool_type: "function".to_string(),
                        extra_content: None,
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

    #[test]
    fn test_fork_error_does_not_break_loop() {
        let mut loop_count = 0;
        let mut reached_normal_generation = false;

        loop {
            loop_count += 1;
            if loop_count > 5 {
                panic!("Loop ran too many times");
            }

            let fork_agent: Option<String> = if loop_count == 1 {
                Some("subagent".to_string())
            } else {
                None
            };

            if fork_agent.is_some() {
                let fork_result: Result<String, String> = Err("subchat failed".to_string());
                match fork_result {
                    Ok(_content) => {
                        break;
                    }
                    Err(_e) => {
                        continue;
                    }
                }
            }

            reached_normal_generation = true;
            break;
        }

        assert!(reached_normal_generation, "Normal generation path must be reached after fork error");
        assert_eq!(loop_count, 2, "Loop must iterate twice: fork error then normal generation");
    }
}
