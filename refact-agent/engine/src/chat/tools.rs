use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock, Semaphore};
use tracing::info;
use uuid::Uuid;
use futures::future::join_all;

use indexmap::IndexMap;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{
    ChatContent, ChatMessage, ChatToolCall, ContextFile, PostprocessSettings,
    SubchatParameters,
};
use crate::global_context::GlobalContext;
use crate::constants::CHAT_TOP_N;
use crate::postprocessing::pp_tool_results::{postprocess_tool_results, ToolBudget};
use crate::yaml_configs::customization_registry::{get_mode_config, map_legacy_mode_to_id, match_tool_confirm_action};

#[derive(Default)]
pub struct ExecuteToolsOptions {
    pub subchat_tool_parameters: Option<IndexMap<String, SubchatParameters>>,
    pub postprocess_settings: Option<PostprocessSettings>,
}

pub enum ToolStepOutcome {
    NoToolCalls,
    Paused,
    Continue,
}

use super::types::*;
use super::trajectories::maybe_save_trajectory;
use super::config::{limits, tokens};

async fn get_effective_n_ctx(gcx: Arc<ARwLock<GlobalContext>>, thread: &ThreadParams) -> usize {
    let default_n_ctx = tokens().default_n_ctx;
    let model_n_ctx =
        match crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0).await {
            Ok(caps) => match crate::caps::resolve_chat_model(caps, &thread.model) {
                Ok(model_rec) if model_rec.base.n_ctx > 0 => model_rec.base.n_ctx,
                _ => default_n_ctx,
            },
            Err(_) => default_n_ctx,
        };
    match thread.context_tokens_cap {
        Some(cap) if cap > 0 => cap.min(model_n_ctx),
        _ => model_n_ctx,
    }
}

fn is_server_executed_tool(tool_call_id: &str) -> bool {
    tool_call_id.starts_with("srvtoolu_")
}

const PATCH_LIKE_TOOLS: &[&str] = &[
    "patch",
    "text_edit",
    "create_textdoc",
    "update_textdoc",
    "replace_textdoc",
    "update_textdoc_regex",
    "update_textdoc_by_lines",
    "update_textdoc_anchored",
    "apply_patch",
    "undo_textdoc",
];

fn is_patch_like_tool(command: &str) -> bool {
    PATCH_LIKE_TOOLS.contains(&command)
}

fn get_context_files_from_messages(messages: &[ChatMessage]) -> Vec<String> {
    let mut paths = Vec::new();
    for msg in messages {
        if msg.role == "context_file" {
            match &msg.content {
                ChatContent::ContextFiles(files) => {
                    for file in files {
                        if !paths.contains(&file.file_name) {
                            paths.push(file.file_name.clone());
                        }
                    }
                }
                ChatContent::SimpleText(text) => {
                    if let Ok(files) = serde_json::from_str::<Vec<ContextFile>>(text) {
                        for file in files {
                            if !paths.contains(&file.file_name) {
                                paths.push(file.file_name.clone());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    paths
}

fn spawn_subchat_bridge(
    ccx: Arc<AMutex<AtCommandsContext>>,
    session_arc: Arc<AMutex<ChatSession>>,
) -> Arc<AtomicBool> {
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_flag_clone = cancel_flag.clone();

    tokio::spawn(async move {
        let (subchat_rx, abort_flag) = {
            let ccx_locked = ccx.lock().await;
            (ccx_locked.subchat_rx.clone(), ccx_locked.abort_flag.clone())
        };
        info!("spawn_subchat_bridge: started listening for subchat messages");

        let mut active_tool_call_ids: Vec<String> = Vec::new();

        loop {
            let should_stop =
                cancel_flag_clone.load(Ordering::Relaxed) || abort_flag.load(Ordering::Relaxed);

            if should_stop {
                info!(
                    "spawn_subchat_bridge: cancelled, sending cleanup events for {} active tools",
                    active_tool_call_ids.len()
                );
                let mut session = session_arc.lock().await;
                for tool_call_id in active_tool_call_ids.drain(..) {
                    session.emit(ChatEvent::SubchatUpdate {
                        tool_call_id,
                        subchat_id: String::new(),
                        attached_files: vec![],
                    });
                }
                break;
            }

            let recv_result = {
                let mut rx = subchat_rx.lock().await;
                tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await
            };

            match recv_result {
                Ok(Some(value)) => {
                    info!("spawn_subchat_bridge: received message: {:?}", value);
                    let tool_call_id = value.get("tool_call_id").and_then(|v| v.as_str());
                    let subchat_id = value.get("subchat_id").and_then(|v| v.as_str());

                    if let (Some(tool_call_id), Some(subchat_id)) = (tool_call_id, subchat_id) {
                        info!("spawn_subchat_bridge: emitting SubchatUpdate for tool_call_id={}, subchat_id={}", tool_call_id, subchat_id);

                        if !active_tool_call_ids.contains(&tool_call_id.to_string()) {
                            active_tool_call_ids.push(tool_call_id.to_string());
                        }

                        let mut attached_files: Vec<String> = value
                            .get("attached_files")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                                    .collect()
                            })
                            .unwrap_or_default();

                        {
                            let session = session_arc.lock().await;
                            for f in get_context_files_from_messages(&session.messages) {
                                if !attached_files.contains(&f) {
                                    attached_files.push(f);
                                }
                            }
                        }

                        let mut session = session_arc.lock().await;
                        session.emit(ChatEvent::SubchatUpdate {
                            tool_call_id: tool_call_id.to_string(),
                            subchat_id: subchat_id.to_string(),
                            attached_files,
                        });
                    }
                }
                Ok(None) => break,
                Err(_) => {}
            }
        }
    });

    cancel_flag
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_server_executed_tool_with_prefix() {
        assert!(is_server_executed_tool("srvtoolu_abc123"));
        assert!(is_server_executed_tool("srvtoolu_"));
        assert!(is_server_executed_tool("srvtoolu_very_long_id_here"));
    }

    #[test]
    fn test_is_server_executed_tool_without_prefix() {
        assert!(!is_server_executed_tool("call_abc123"));
        assert!(!is_server_executed_tool("toolu_abc123"));
        assert!(!is_server_executed_tool(""));
        assert!(!is_server_executed_tool("srvtoolu"));
        assert!(!is_server_executed_tool("SRVTOOLU_abc"));
    }

    #[test]
    fn test_is_patch_like_tool() {
        assert!(is_patch_like_tool("patch"));
        assert!(is_patch_like_tool("text_edit"));
        assert!(is_patch_like_tool("create_textdoc"));
        assert!(is_patch_like_tool("update_textdoc"));
        assert!(is_patch_like_tool("update_textdoc_regex"));
        assert!(is_patch_like_tool("update_textdoc_by_lines"));
        assert!(is_patch_like_tool("undo_textdoc"));
    }

    #[test]
    fn test_is_not_patch_like_tool() {
        assert!(!is_patch_like_tool("shell"));
        assert!(!is_patch_like_tool("cat"));
        assert!(!is_patch_like_tool("search"));
        assert!(!is_patch_like_tool(""));
        assert!(!is_patch_like_tool("PATCH"));
    }
}

pub async fn process_tool_calls_once(
    gcx: Arc<ARwLock<GlobalContext>>,
    session_arc: Arc<AMutex<ChatSession>>,
    mode_id: &str,
    model_id: Option<&str>,
) -> ToolStepOutcome {
    let (tool_calls, server_tool_calls, messages, thread) = {
        let session = session_arc.lock().await;
        let last_msg = session.messages.last();
        match last_msg {
            Some(m) if m.role == "assistant" && m.tool_calls.is_some() => {
                let all_calls = m.tool_calls.clone().unwrap();
                let (server_calls, client_calls): (Vec<_>, Vec<_>) = all_calls
                    .into_iter()
                    .partition(|tc| is_server_executed_tool(&tc.id));
                (
                    client_calls,
                    server_calls,
                    session.messages.clone(),
                    session.thread.clone(),
                )
            }
            _ => return ToolStepOutcome::NoToolCalls,
        }
    };

    // Add synthetic tool results for server-executed tools (e.g., Anthropic's web_search).
    // These tools are executed by the LLM provider and their results are embedded in the
    // assistant's response as citations. However, the Anthropic API requires exactly one
    // tool_result per tool_use, so we add placeholder results to satisfy this requirement.
    // Without these, the LLM may regenerate similar responses because it sees tool_calls
    // without corresponding results.
    if !server_tool_calls.is_empty() {
        let mut session = session_arc.lock().await;
        for tc in &server_tool_calls {
            // Check if a tool result already exists for this tool call
            let result_exists = session
                .messages
                .iter()
                .any(|m| m.role == "tool" && m.tool_call_id == tc.id);
            if !result_exists {
                let tool_message = ChatMessage {
                    message_id: Uuid::new_v4().to_string(),
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(format!(
                        "[Results from '{}' are included in the assistant's response above]",
                        tc.function.name
                    )),
                    tool_call_id: tc.id.clone(),
                    tool_failed: Some(false),
                    ..Default::default()
                };
                session.add_message(tool_message);
            }
        }
    }

    if tool_calls.is_empty() {
        return ToolStepOutcome::NoToolCalls;
    }

    info!(
        "process_tool_calls_once: {} tool calls to process",
        tool_calls.len()
    );

    let (confirmations, denials) =
        check_tools_confirmation(gcx.clone(), &tool_calls, &messages, mode_id, model_id).await;

    let denied_ids: Vec<String> = denials.iter().map(|d| d.tool_call_id.clone()).collect();
    if !denials.is_empty() {
        let mut session = session_arc.lock().await;
        for denial in &denials {
            let tool_message = ChatMessage {
                message_id: Uuid::new_v4().to_string(),
                role: "tool".to_string(),
                content: ChatContent::SimpleText(format!("Denied by policy: {}", denial.rule)),
                tool_call_id: denial.tool_call_id.clone(),
                tool_failed: Some(true),
                ..Default::default()
            };
            session.add_message(tool_message);
        }
    }

    if !confirmations.is_empty() {
        let dominated_by_patch =
            thread.automatic_patch && confirmations.iter().all(|c| is_patch_like_tool(&c.command));

        if !dominated_by_patch {
            let mut session = session_arc.lock().await;
            session.set_paused_with_reasons(confirmations);
            return ToolStepOutcome::Paused;
        }
    }

    let tools_to_execute: Vec<_> = tool_calls
        .iter()
        .filter(|tc| !denied_ids.contains(&tc.id))
        .cloned()
        .collect();

    if tools_to_execute.is_empty() {
        return ToolStepOutcome::Continue;
    }

    {
        let mut session = session_arc.lock().await;
        session.set_runtime_state(SessionState::ExecutingTools, None);
    }

    let (tool_results, _) = execute_tools_with_session(
        gcx.clone(),
        session_arc.clone(),
        &tools_to_execute,
        &messages,
        &thread,
        mode_id,
        model_id,
        ExecuteToolsOptions::default(),
    )
    .await;

    {
        let mut session = session_arc.lock().await;
        for result_msg in tool_results {
            session.add_message(result_msg);
        }
        session.set_runtime_state(SessionState::Idle, None);
    }

    maybe_save_trajectory(gcx.clone(), session_arc.clone()).await;
    ToolStepOutcome::Continue
}

pub async fn check_tools_confirmation(
    gcx: Arc<ARwLock<GlobalContext>>,
    tool_calls: &[crate::call_validation::ChatToolCall],
    messages: &[ChatMessage],
    mode_id: &str,
    model_id: Option<&str>,
) -> (Vec<PauseReason>, Vec<PauseReason>) {
    use crate::tools::tools_description::MatchConfirmDenyResult;

    let mut confirmations = Vec::new();
    let mut denials = Vec::new();

    let ccx = Arc::new(AMutex::new(
        AtCommandsContext::new(
            gcx.clone(),
            1000,
            1,
            false,
            messages.to_vec(),
            String::new(),
            None,
            String::new(),
            None,
        )
        .await,
    ));

    let mode_id = map_legacy_mode_to_id(mode_id);
    let mode_config = get_mode_config(gcx.clone(), mode_id, model_id).await;
    let tool_confirm_rules = mode_config
        .as_ref()
        .map(|m| m.tool_confirm.rules.as_slice())
        .unwrap_or(&[]);

    let all_tools =
        crate::tools::tools_list::get_tools_for_mode(gcx.clone(), mode_id, model_id)
            .await
            .into_iter()
            .map(|tool| {
                let spec = tool.tool_description();
                (spec.name, tool)
            })
            .collect::<indexmap::IndexMap<_, _>>();

    for tool_call in tool_calls {
        let tool = match all_tools.get(&tool_call.function.name) {
            Some(t) => t,
            None => {
                info!(
                    "Unknown tool: {}, skipping confirmation check",
                    tool_call.function.name
                );
                continue;
            }
        };

        let args: std::collections::HashMap<String, serde_json::Value> =
            match serde_json::from_str(&tool_call.function.arguments) {
                Ok(a) => a,
                Err(e) => {
                    denials.push(PauseReason {
                        reason_type: "denial".to_string(),
                        command: tool_call.function.name.clone(),
                        rule: format!("Failed to parse arguments: {}", e),
                        tool_call_id: tool_call.id.clone(),
                        integr_config_path: tool.has_config_path(),
                    });
                    continue;
                }
            };

        let tool_result = tool.match_against_confirm_deny(ccx.clone(), &args).await;
        let mode_action = match_tool_confirm_action(tool_confirm_rules, &tool_call.function.name);

        match tool_result {
            Ok(result) => {
                if result.result == MatchConfirmDenyResult::DENY {
                    denials.push(PauseReason {
                        reason_type: "denial".to_string(),
                        command: result.command,
                        rule: result.rule,
                        tool_call_id: tool_call.id.clone(),
                        integr_config_path: tool.has_config_path(),
                    });
                    continue;
                }

                let final_action = match mode_action.as_deref() {
                    Some("deny") => "deny",
                    Some("ask") => "ask",
                    Some("auto") => "auto",
                    _ => match result.result {
                        MatchConfirmDenyResult::CONFIRMATION => "ask",
                        MatchConfirmDenyResult::PASS => "auto",
                        MatchConfirmDenyResult::DENY => "deny",
                    },
                };

                let rule_text = match mode_action.as_deref() {
                    Some(action) => format!("mode policy: {}", action),
                    None => result.rule.clone(),
                };

                match final_action {
                    "deny" => {
                        denials.push(PauseReason {
                            reason_type: "denial".to_string(),
                            command: result.command,
                            rule: rule_text,
                            tool_call_id: tool_call.id.clone(),
                            integr_config_path: tool.has_config_path(),
                        });
                    }
                    "ask" => {
                        confirmations.push(PauseReason {
                            reason_type: "confirmation".to_string(),
                            command: result.command,
                            rule: rule_text,
                            tool_call_id: tool_call.id.clone(),
                            integr_config_path: tool.has_config_path(),
                        });
                    }
                    _ => {}
                }
            }
            Err(e) => {
                denials.push(PauseReason {
                    reason_type: "denial".to_string(),
                    command: tool_call.function.name.clone(),
                    rule: format!("confirmation check failed: {}", e),
                    tool_call_id: tool_call.id.clone(),
                    integr_config_path: tool.has_config_path(),
                });
            }
        }
    }

    (confirmations, denials)
}

pub async fn execute_tools_with_session(
    gcx: Arc<ARwLock<GlobalContext>>,
    session_arc: Arc<AMutex<ChatSession>>,
    tool_calls: &[ChatToolCall],
    messages: &[ChatMessage],
    thread: &ThreadParams,
    mode_id: &str,
    model_id: Option<&str>,
    options: ExecuteToolsOptions,
) -> (Vec<ChatMessage>, bool) {
    if tool_calls.is_empty() {
        return (vec![], false);
    }

    let (prompt_messages, session_abort_flag) = {
        let session = session_arc.lock().await;
        let msgs = if session.last_prompt_messages.is_empty() {
            messages.to_vec()
        } else {
            session.last_prompt_messages.clone()
        };
        (msgs, session.abort_flag.clone())
    };

    let n_ctx = get_effective_n_ctx(gcx.clone(), thread).await;
    let budget = match ToolBudget::try_from_n_ctx(n_ctx) {
        Ok(b) => b,
        Err(e) => {
            let error_messages: Vec<ChatMessage> = tool_calls
                .iter()
                .map(|tc| ChatMessage {
                    message_id: Uuid::new_v4().to_string(),
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(format!("Error: {}", e)),
                    tool_call_id: tc.id.clone(),
                    tool_failed: Some(true),
                    ..Default::default()
                })
                .collect();
            return (error_messages, false);
        }
    };

    let ccx = Arc::new(AMutex::new(
        AtCommandsContext::new_with_abort(
            gcx.clone(),
            n_ctx,
            CHAT_TOP_N,
            false,
            messages.to_vec(),
            thread.id.clone(),
            thread.root_chat_id.clone(),
            thread.model.clone(),
            thread.task_meta.clone(),
            Some(session_abort_flag),
        )
        .await,
    ));

    {
        let mut ccx_locked = ccx.lock().await;
        ccx_locked.tokens_for_rag = (n_ctx / 2).max(4096);
        if let Some(ref params) = options.subchat_tool_parameters {
            ccx_locked.subchat_tool_parameters = params.clone();
        }
    }

    let cancel_flag = spawn_subchat_bridge(ccx.clone(), session_arc.clone());

    let result = execute_tools_inner(
        gcx,
        ccx,
        tool_calls,
        mode_id,
        model_id,
        budget,
        options,
        &prompt_messages,
    )
    .await;

    cancel_flag.store(true, Ordering::Relaxed);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let context_files = get_context_files_from_messages(&result.0);
    if !context_files.is_empty() {
        let mut session = session_arc.lock().await;
        for tc in tool_calls {
            session.emit(ChatEvent::SubchatUpdate {
                tool_call_id: tc.id.clone(),
                subchat_id: "/tool:files".to_string(),
                attached_files: context_files.clone(),
            });
        }
    }

    result
}

async fn execute_tools_inner(
    gcx: Arc<ARwLock<GlobalContext>>,
    ccx: Arc<AMutex<AtCommandsContext>>,
    tool_calls: &[ChatToolCall],
    mode_id: &str,
    model_id: Option<&str>,
    budget: ToolBudget,
    options: ExecuteToolsOptions,
    messages: &[ChatMessage],
) -> (Vec<ChatMessage>, bool) {
    let max_parallel = limits().max_parallel_tools;

    let available_tool_names: std::collections::HashSet<String> =
        crate::tools::tools_list::get_tools_for_mode(gcx.clone(), mode_id, model_id)
            .await
            .into_iter()
            .map(|tool| tool.tool_description().name)
            .collect();

    let semaphore = Arc::new(Semaphore::new(max_parallel));

    let futures: Vec<_> = tool_calls
        .iter()
        .enumerate()
        .map(|(idx, tool_call)| {
            let gcx = gcx.clone();
            let ccx = ccx.clone();
            let semaphore = semaphore.clone();
            let available_tool_names = available_tool_names.clone();
            let tool_call = tool_call.clone();

            async move {
                let _permit = semaphore.acquire().await.unwrap();

                if !available_tool_names.contains(&tool_call.function.name) {
                    return (
                        idx,
                        vec![ChatMessage {
                            message_id: Uuid::new_v4().to_string(),
                            role: "tool".to_string(),
                            content: ChatContent::SimpleText(format!(
                                "Error: tool '{}' not found",
                                tool_call.function.name
                            )),
                            tool_call_id: tool_call.id.clone(),
                            tool_failed: Some(true),
                            ..Default::default()
                        }],
                        vec![],
                    );
                }

                let tool_opt = crate::tools::tools_list::get_available_tools(gcx.clone())
                    .await
                    .into_iter()
                    .find(|t| t.tool_description().name == tool_call.function.name);

                let mut tool = match tool_opt {
                    Some(t) => t,
                    None => {
                        return (
                            idx,
                            vec![ChatMessage {
                                message_id: Uuid::new_v4().to_string(),
                                role: "tool".to_string(),
                                content: ChatContent::SimpleText(format!(
                                    "Error: tool '{}' not found",
                                    tool_call.function.name
                                )),
                                tool_call_id: tool_call.id.clone(),
                                tool_failed: Some(true),
                                ..Default::default()
                            }],
                            vec![],
                        );
                    }
                };

                let args: std::collections::HashMap<String, serde_json::Value> =
                    match serde_json::from_str(&tool_call.function.arguments) {
                        Ok(a) => a,
                        Err(e) => {
                            return (
                                idx,
                                vec![ChatMessage {
                                    message_id: Uuid::new_v4().to_string(),
                                    role: "tool".to_string(),
                                    content: ChatContent::SimpleText(format!(
                                        "Error parsing arguments: {}",
                                        e
                                    )),
                                    tool_call_id: tool_call.id.clone(),
                                    tool_failed: Some(true),
                                    ..Default::default()
                                }],
                                vec![],
                            );
                        }
                    };

                info!("Executing tool: {}({:?})", tool_call.function.name, args);

                match tool.tool_execute(ccx, &tool_call.id, &args).await {
                    Ok((_corrections, results)) => {
                        let mut msgs = Vec::new();
                        let mut files = Vec::new();
                        for result in results {
                            match result {
                                crate::call_validation::ContextEnum::ChatMessage(mut msg) => {
                                    if msg.message_id.is_empty() {
                                        msg.message_id = Uuid::new_v4().to_string();
                                    }
                                    if msg.tool_failed.is_none() {
                                        msg.tool_failed = Some(false);
                                    }
                                    msgs.push(msg);
                                }
                                crate::call_validation::ContextEnum::ContextFile(cf) => {
                                    files.push(cf);
                                }
                            }
                        }
                        (idx, msgs, files)
                    }
                    Err(e) => {
                        info!("Tool execution failed: {}: {}", tool_call.function.name, e);
                        (
                            idx,
                            vec![ChatMessage {
                                message_id: Uuid::new_v4().to_string(),
                                role: "tool".to_string(),
                                content: ChatContent::SimpleText(format!("Error: {}", e)),
                                tool_call_id: tool_call.id.clone(),
                                tool_failed: Some(true),
                                ..Default::default()
                            }],
                            vec![],
                        )
                    }
                }
            }
        })
        .collect();

    let mut results: Vec<_> = join_all(futures).await;
    results.sort_by_key(|(idx, _, _)| *idx);

    let mut tool_messages: Vec<ChatMessage> = Vec::new();
    let mut context_files: Vec<ContextFile> = Vec::new();
    for (_, msgs, files) in results {
        tool_messages.extend(msgs);
        context_files.extend(files);
    }

    let pp_settings = options.postprocess_settings.unwrap_or_default();

    let results = postprocess_tool_results(
        gcx,
        None,
        tool_messages,
        context_files,
        budget,
        pp_settings,
        messages,
    )
    .await;

    (results, true)
}

pub async fn execute_tools(
    gcx: Arc<ARwLock<GlobalContext>>,
    tool_calls: &[ChatToolCall],
    messages: &[ChatMessage],
    thread: &ThreadParams,
    mode_id: &str,
    model_id: Option<&str>,
    options: ExecuteToolsOptions,
) -> (Vec<ChatMessage>, bool) {
    if tool_calls.is_empty() {
        return (vec![], false);
    }

    let n_ctx = get_effective_n_ctx(gcx.clone(), thread).await;
    let budget = match ToolBudget::try_from_n_ctx(n_ctx) {
        Ok(b) => b,
        Err(e) => {
            let error_messages: Vec<ChatMessage> = tool_calls
                .iter()
                .map(|tc| ChatMessage {
                    message_id: Uuid::new_v4().to_string(),
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(format!("Error: {}", e)),
                    tool_call_id: tc.id.clone(),
                    tool_failed: Some(true),
                    ..Default::default()
                })
                .collect();
            return (error_messages, false);
        }
    };

    let ccx = Arc::new(AMutex::new(
        AtCommandsContext::new(
            gcx.clone(),
            n_ctx,
            CHAT_TOP_N,
            false,
            messages.to_vec(),
            thread.id.clone(),
            thread.root_chat_id.clone(),
            thread.model.clone(),
            thread.task_meta.clone(),
        )
        .await,
    ));

    {
        let mut ccx_locked = ccx.lock().await;
        ccx_locked.tokens_for_rag = (n_ctx / 2).max(4096);
        if let Some(params) = options.subchat_tool_parameters.clone() {
            ccx_locked.subchat_tool_parameters = params;
        }
    }

    execute_tools_inner(gcx, ccx, tool_calls, mode_id, model_id, budget, options, messages).await
}
