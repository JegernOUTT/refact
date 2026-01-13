use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};
use tracing::{info, warn};
use uuid::Uuid;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{
    ChatContent, ChatMessage, ChatMeta, ChatMode, ChatUsage, SamplingParameters,
};
use crate::global_context::GlobalContext;
use crate::scratchpad_abstract::HasTokenizerAndEot;
use crate::constants::CHAT_TOP_N;
use crate::http::routers::v1::knowledge_enrichment::enrich_messages_with_knowledge;

use super::types::*;
use super::trajectories::{maybe_save_trajectory, check_external_reload_pending};
use super::tools::{process_tool_calls_once, ToolStepOutcome};
use super::prepare::{prepare_chat_passthrough, ChatPrepareOptions};
use super::prompts::prepend_the_right_system_prompt_and_maybe_more_initial_messages;
use super::stream_core::{run_llm_stream, StreamRunParams, StreamCollector, normalize_tool_call};
use super::queue::inject_priority_messages_if_any;
use super::config::tokens;

pub fn parse_chat_mode(mode: &str) -> ChatMode {
    match mode.to_uppercase().as_str() {
        "AGENT" => ChatMode::AGENT,
        "NO_TOOLS" => ChatMode::NO_TOOLS,
        "EXPLORE" => ChatMode::EXPLORE,
        "CONFIGURE" => ChatMode::CONFIGURE,
        "PROJECT_SUMMARY" => ChatMode::PROJECT_SUMMARY,
        "TASK_PLANNER" => ChatMode::TASK_PLANNER,
        "TASK_AGENT" => ChatMode::TASK_AGENT,
        _ => ChatMode::AGENT,
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
                let Some(tcs) = m.tool_calls.as_ref() else { return false };
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
            let (messages, thread, chat_id) = {
                let session = session_arc.lock().await;
                (
                    session.messages.clone(),
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
                messages,
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

            let chat_mode = {
                let session = session_arc.lock().await;
                parse_chat_mode(&session.thread.mode)
            };

            match process_tool_calls_once(gcx.clone(), session_arc.clone(), chat_mode).await {
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
    messages: Vec<ChatMessage>,
    thread: ThreadParams,
    chat_id: String,
    abort_flag: Arc<AtomicBool>,
) -> Result<(), String> {
    let chat_mode = parse_chat_mode(&thread.mode);

    let tools: Vec<crate::tools::tools_description::ToolDesc> = {
        let all_tools: Vec<_> = crate::tools::tools_list::get_available_tools_by_chat_mode(gcx.clone(), chat_mode)
            .await
            .into_iter()
            .map(|tool| tool.tool_description())
            .collect();

        if thread.tool_use.is_empty() || thread.tool_use == "agent" {
            all_tools
        } else {
            let allowed: std::collections::HashSet<String> = thread.tool_use
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            all_tools.into_iter().filter(|t| allowed.contains(&t.name)).collect()
        }
    };

    info!("session generation: tools count = {}", tools.len());

    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| e.message)?;
    let model_rec = crate::caps::resolve_chat_model(caps, &thread.model)?;

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
        chat_mode,
        chat_remote: false,
        current_config_file: String::new(),
        context_tokens_cap: thread.context_tokens_cap,
        include_project_info: thread.include_project_info,
        request_attempt_id: Uuid::new_v4().to_string(),
    };

    let mut messages = messages;

    let (session_has_system, session_has_project_context) = {
        let session = session_arc.lock().await;
        let has_system = session
            .messages
            .first()
            .map(|m| m.role == "system")
            .unwrap_or(false);
        let has_project_ctx = session.messages.iter().any(|m| {
            m.role == "context_file"
                && m.tool_call_id == crate::chat::system_context::PROJECT_CONTEXT_MARKER
        });
        (has_system, has_project_ctx)
    };

    let needs_preamble =
        !session_has_system || (!session_has_project_context && thread.include_project_info);

    if needs_preamble {
        let tool_names: std::collections::HashSet<String> =
            tools.iter().map(|t| t.name.clone()).collect();
        let mut has_rag_results = crate::scratchpads::scratchpad_utils::HasRagResults::new();
        let messages_with_preamble =
            prepend_the_right_system_prompt_and_maybe_more_initial_messages(
                gcx.clone(),
                messages.clone(),
                &meta,
                &mut has_rag_results,
                tool_names,
            )
            .await;

        let first_conv_idx_in_new = messages_with_preamble
            .iter()
            .position(|m| m.role == "user" || m.role == "assistant")
            .unwrap_or(messages_with_preamble.len());

        if first_conv_idx_in_new > 0 {
            let mut session = session_arc.lock().await;
            let first_conv_idx_in_session = session
                .messages
                .iter()
                .position(|m| m.role == "user" || m.role == "assistant")
                .unwrap_or(session.messages.len());

            let mut inserted = 0;
            for msg in messages_with_preamble.iter().take(first_conv_idx_in_new) {
                if msg.role == "assistant" {
                    continue;
                }
                if msg.role == "system" && session.messages.first().map(|m| m.role == "system").unwrap_or(false) {
                    continue;
                }
                if msg.role == "cd_instruction" && session.messages.iter().any(|m| m.role == "cd_instruction") {
                    continue;
                }
                if msg.role == "context_file" && session.messages.iter().any(|m| {
                    m.role == "context_file" && m.tool_call_id == msg.tool_call_id
                }) {
                    continue;
                }
                let mut msg_with_id = msg.clone();
                if msg_with_id.message_id.is_empty() {
                    msg_with_id.message_id = Uuid::new_v4().to_string();
                }
                let insert_idx = first_conv_idx_in_session + inserted;
                session.messages.insert(insert_idx, msg_with_id.clone());
                session.emit(ChatEvent::MessageAdded {
                    message: msg_with_id,
                    index: insert_idx,
                });
                inserted += 1;
            }
            if inserted > 0 {
                session.increment_version();
                info!("Saved {} preamble messages to session", inserted);
            }
        }
        messages = messages_with_preamble;
    }

    let last_is_user = messages.last().map(|m| m.role == "user").unwrap_or(false);
    if chat_mode == ChatMode::AGENT && last_is_user {
        let msg_count_before = messages.len();
        enrich_messages_with_knowledge(gcx.clone(), &mut messages, Some(&chat_id)).await;
        if messages.len() > msg_count_before {
            let mut session = session_arc.lock().await;
            let session_last_user_idx = session
                .messages
                .iter()
                .rposition(|m| m.role == "user")
                .unwrap_or(0);
            let local_last_user_idx = messages.iter().rposition(|m| m.role == "user").unwrap_or(0);
            if local_last_user_idx > 0 {
                let enriched_msg = &messages[local_last_user_idx - 1];
                if enriched_msg.role == "context_file" {
                    let mut msg_with_id = enriched_msg.clone();
                    if msg_with_id.message_id.is_empty() {
                        msg_with_id.message_id = Uuid::new_v4().to_string();
                    }
                    session
                        .messages
                        .insert(session_last_user_idx, msg_with_id.clone());
                    session.emit(ChatEvent::MessageAdded {
                        message: msg_with_id,
                        index: session_last_user_idx,
                    });
                    session.increment_version();
                    info!(
                        "Saved knowledge enrichment context_file to session at index {}",
                        session_last_user_idx
                    );
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

    let code_workdir = {
        let session = session_arc.lock().await;
        let task_meta = session.thread.task_meta.clone();
        drop(session);
        
        if let Some(tm) = task_meta {
            match crate::tasks::storage::load_board(gcx.clone(), &tm.task_id).await {
                Ok(board) => {
                    board.get_card(&tm.card_id.as_ref().unwrap_or(&String::new()))
                        .and_then(|card| card.agent_worktree.as_ref().map(|p| std::path::PathBuf::from(p)))
                }
                Err(_) => None,
            }
        } else {
            None
        }
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
        thread.task_meta.clone(),
        code_workdir,
    )
    .await;
    let ccx_arc = Arc::new(AMutex::new(ccx));

    let options = ChatPrepareOptions {
        prepend_system_prompt: false,
        allow_at_commands: true,
        allow_tool_prerun: true,
        supports_tools: model_rec.supports_tools,
        ..Default::default()
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
    )
    .await?;

    {
        let mut session = session_arc.lock().await;
        session.last_prompt_messages = prepared.limited_messages.clone();
    }

    run_streaming_generation(
        gcx,
        session_arc,
        prepared.prompt,
        model_rec.base.clone(),
        parameters,
        abort_flag,
        chat_mode,
    )
    .await
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

    let (chat_id, context_tokens_cap, include_project_info) = {
        let session = session_arc.lock().await;
        (
            session.chat_id.clone(),
            session.thread.context_tokens_cap,
            session.thread.include_project_info,
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
    });

    let params = StreamRunParams {
        prompt,
        model_rec,
        sampling: parameters,
        meta,
        abort_flag: Some(abort_flag),
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

        session.finish_stream(result.finish_reason);
    }

    Ok(())
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
        let messages = vec![
            make_user_msg("hello"),
            make_assistant_msg("response"),
        ];
        assert!(!tail_needs_assistant(&messages));
    }

    #[test]
    fn test_tail_needs_assistant_ends_with_user() {
        let messages = vec![
            make_user_msg("hello"),
        ];
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
