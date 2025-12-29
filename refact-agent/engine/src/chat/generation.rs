use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use tracing::{info, warn};
use uuid::Uuid;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ChatMeta, ChatMode, ChatUsage, SamplingParameters};
use crate::global_context::GlobalContext;
use crate::scratchpad_abstract::HasTokenizerAndEot;
use crate::constants::CHAT_TOP_N;
use crate::http::routers::v1::knowledge_enrichment::enrich_messages_with_knowledge;

use super::types::*;
use super::trajectories::{maybe_save_trajectory, check_external_reload_pending};
use super::tools::check_tool_calls_and_continue;
use super::prepare::{prepare_chat_passthrough, ChatPrepareOptions};
use super::prompts::prepend_the_right_system_prompt_and_maybe_more_initial_messages;
use super::stream_core::{run_llm_stream, StreamRunParams, StreamCollector, normalize_tool_call};

pub fn parse_chat_mode(mode: &str) -> ChatMode {
    match mode.to_uppercase().as_str() {
        "AGENT" => ChatMode::AGENT,
        "NO_TOOLS" => ChatMode::NO_TOOLS,
        "EXPLORE" => ChatMode::EXPLORE,
        "CONFIGURE" => ChatMode::CONFIGURE,
        "PROJECT_SUMMARY" => ChatMode::PROJECT_SUMMARY,
        _ => ChatMode::AGENT,
    }
}

pub fn start_generation(
    gcx: Arc<ARwLock<GlobalContext>>,
    session_arc: Arc<AMutex<ChatSession>>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        let (messages, thread, chat_id) = {
            let session = session_arc.lock().await;
            (session.messages.clone(), session.thread.clone(), session.chat_id.clone())
        };

        let abort_flag = {
            let mut session = session_arc.lock().await;
            match session.start_stream() {
                Some((_message_id, abort_flag)) => abort_flag,
                None => {
                    warn!("Cannot start generation for {}: already generating", chat_id);
                    return;
                }
            }
        };

        if let Err(e) = run_llm_generation(gcx.clone(), session_arc.clone(), messages, thread, chat_id.clone(), abort_flag).await {
            let mut session = session_arc.lock().await;
            if !session.abort_flag.load(Ordering::SeqCst) {
                session.finish_stream_with_error(e);
            }
        }

        maybe_save_trajectory(gcx.clone(), session_arc.clone()).await;

        {
            let session = session_arc.lock().await;
            session.queue_notify.notify_one();
        }
    })
}

pub async fn run_llm_generation(
    gcx: Arc<ARwLock<GlobalContext>>,
    session_arc: Arc<AMutex<ChatSession>>,
    messages: Vec<ChatMessage>,
    thread: ThreadParams,
    chat_id: String,
    abort_flag: Arc<AtomicBool>,
) -> Result<(), String> {
    let chat_mode = parse_chat_mode(&thread.mode);

    let tools: Vec<crate::tools::tools_description::ToolDesc> =
        crate::tools::tools_list::get_available_tools_by_chat_mode(gcx.clone(), chat_mode).await
            .into_iter()
            .map(|tool| tool.tool_description())
            .collect();

    info!("session generation: tools count = {}", tools.len());

    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0).await
        .map_err(|e| e.message)?;
    let model_rec = crate::caps::resolve_chat_model(caps, &thread.model)?;

    let effective_n_ctx = thread.context_tokens_cap.unwrap_or(model_rec.base.n_ctx);
    let tokenizer_arc = crate::tokens::cached_tokenizer(gcx.clone(), &model_rec.base).await?;
    let t = HasTokenizerAndEot::new(tokenizer_arc);

    let meta = ChatMeta {
        chat_id: chat_id.clone(),
        chat_mode,
        chat_remote: false,
        current_config_file: String::new(),
        context_tokens_cap: thread.context_tokens_cap,
        include_project_info: thread.include_project_info,
        request_attempt_id: Uuid::new_v4().to_string(),
        use_compression: thread.use_compression,
    };

    let mut messages = messages;

    let (session_has_system, session_has_cd_instruction) = {
        let session = session_arc.lock().await;
        let has_system = session.messages.first().map(|m| m.role == "system").unwrap_or(false);
        let has_cd = session.messages.iter().any(|m| m.role == "cd_instruction");
        (has_system, has_cd)
    };

    let needs_preamble = !session_has_system || (!session_has_cd_instruction && thread.include_project_info);

    if needs_preamble {
        let tool_names: std::collections::HashSet<String> = tools.iter()
            .map(|t| t.name.clone())
            .collect();
        let mut has_rag_results = crate::scratchpads::scratchpad_utils::HasRagResults::new();
        let messages_with_preamble = prepend_the_right_system_prompt_and_maybe_more_initial_messages(
            gcx.clone(),
            messages.clone(),
            &meta,
            &mut has_rag_results,
            tool_names,
        ).await;

        let first_user_idx_in_new = messages_with_preamble.iter()
            .position(|m| m.role == "user")
            .unwrap_or(messages_with_preamble.len());

        if first_user_idx_in_new > 0 {
            let mut session = session_arc.lock().await;
            let first_user_idx_in_session = session.messages.iter()
                .position(|m| m.role == "user")
                .unwrap_or(0);

            for (i, msg) in messages_with_preamble.iter().take(first_user_idx_in_new).enumerate() {
                if session.messages.iter().any(|m| m.role == msg.role && m.role == "system") && msg.role == "system" {
                    continue;
                }
                if session.messages.iter().any(|m| m.role == "cd_instruction") && msg.role == "cd_instruction" {
                    continue;
                }
                let mut msg_with_id = msg.clone();
                if msg_with_id.message_id.is_empty() {
                    msg_with_id.message_id = Uuid::new_v4().to_string();
                }
                session.messages.insert(first_user_idx_in_session + i, msg_with_id.clone());
                session.emit(ChatEvent::MessageAdded {
                    message: msg_with_id,
                    index: first_user_idx_in_session + i,
                });
            }
            session.increment_version();
            info!("Saved preamble messages to session before first user message");
        }
        messages = messages_with_preamble;
    }

    let last_is_user = messages.last().map(|m| m.role == "user").unwrap_or(false);
    if chat_mode == ChatMode::AGENT && last_is_user {
        let msg_count_before = messages.len();
        enrich_messages_with_knowledge(gcx.clone(), &mut messages).await;
        if messages.len() > msg_count_before {
            let mut session = session_arc.lock().await;
            let session_last_user_idx = session.messages.iter().rposition(|m| m.role == "user").unwrap_or(0);
            let local_last_user_idx = messages.iter().rposition(|m| m.role == "user").unwrap_or(0);
            if local_last_user_idx > 0 {
                let enriched_msg = &messages[local_last_user_idx - 1];
                if enriched_msg.role == "context_file" {
                    let mut msg_with_id = enriched_msg.clone();
                    if msg_with_id.message_id.is_empty() {
                        msg_with_id.message_id = Uuid::new_v4().to_string();
                    }
                    session.messages.insert(session_last_user_idx, msg_with_id.clone());
                    session.emit(ChatEvent::MessageAdded {
                        message: msg_with_id,
                        index: session_last_user_idx,
                    });
                    session.increment_version();
                    info!("Saved knowledge enrichment context_file to session at index {}", session_last_user_idx);
                }
            }
        }
    }

    let mut parameters = SamplingParameters {
        temperature: Some(0.0),
        max_new_tokens: 4096.min(effective_n_ctx / 4),
        boost_reasoning: thread.boost_reasoning,
        ..Default::default()
    };

    let ccx = AtCommandsContext::new(
        gcx.clone(),
        effective_n_ctx,
        CHAT_TOP_N,
        false,
        messages.clone(),
        chat_id.clone(),
        false,
        model_rec.base.id.clone(),
    ).await;
    let ccx_arc = Arc::new(AMutex::new(ccx));

    let options = ChatPrepareOptions {
        prepend_system_prompt: false,
        allow_at_commands: true,
        allow_tool_prerun: true,
        supports_tools: model_rec.supports_tools,
        use_compression: thread.use_compression,
    };

    let prepared = prepare_chat_passthrough(
        gcx.clone(),
        ccx_arc.clone(),
        &t,
        messages,
        &model_rec.base.id,
        tools,
        &meta,
        &mut parameters,
        &options,
        &None,
    ).await?;

    run_streaming_generation(
        gcx,
        session_arc,
        prepared.prompt,
        model_rec.base.clone(),
        parameters,
        abort_flag,
        chat_mode,
    ).await
}

async fn run_streaming_generation(
    gcx: Arc<ARwLock<GlobalContext>>,
    session_arc: Arc<AMutex<ChatSession>>,
    prompt: String,
    model_rec: crate::caps::BaseModelRecord,
    parameters: SamplingParameters,
    abort_flag: Arc<AtomicBool>,
    chat_mode: ChatMode,
) -> Result<(), String> {
    info!("session generation: prompt length = {}", prompt.len());

    let (chat_id, context_tokens_cap, include_project_info, use_compression) = {
        let session = session_arc.lock().await;
        (
            session.chat_id.clone(),
            session.thread.context_tokens_cap,
            session.thread.include_project_info,
            session.thread.use_compression,
        )
    };

    let meta = Some(ChatMeta {
        chat_id,
        chat_mode,
        chat_remote: false,
        current_config_file: String::new(),
        context_tokens_cap,
        include_project_info,
        request_attempt_id: Uuid::new_v4().to_string(),
        use_compression,
    });

    let params = StreamRunParams {
        prompt,
        model_rec,
        sampling: parameters,
        meta,
        abort_flag: Some(abort_flag),
    };

    struct SessionCollector {
        session_arc: Arc<AMutex<ChatSession>>,
    }

    impl StreamCollector for SessionCollector {
        fn on_delta_ops(&mut self, _choice_idx: usize, ops: Vec<DeltaOp>) {
            let session_arc = self.session_arc.clone();
            tokio::spawn(async move {
                let mut session = session_arc.lock().await;
                session.emit_stream_delta(ops);
            });
        }

        fn on_usage(&mut self, usage: &ChatUsage) {
            let session_arc = self.session_arc.clone();
            let usage = usage.clone();
            tokio::spawn(async move {
                let mut session = session_arc.lock().await;
                session.draft_usage = Some(usage);
            });
        }

        fn on_finish(&mut self, _choice_idx: usize, _finish_reason: Option<String>) {}
    }

    let mut collector = SessionCollector { session_arc: session_arc.clone() };
    let results = run_llm_stream(gcx.clone(), params, 1, &mut collector).await?;

    let result = results.into_iter().next().unwrap_or_default();

    {
        let mut session = session_arc.lock().await;

        if let Some(ref mut draft) = session.draft_message {
            draft.content = ChatContent::SimpleText(result.content);

            if !result.tool_calls_raw.is_empty() {
                info!("Parsing {} accumulated tool calls", result.tool_calls_raw.len());
                let parsed: Vec<_> = result.tool_calls_raw.iter()
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

        session.finish_stream(result.finish_reason);
    }

    check_tool_calls_and_continue(gcx.clone(), session_arc.clone(), chat_mode).await;
    check_external_reload_pending(gcx, session_arc).await;

    Ok(())
}
