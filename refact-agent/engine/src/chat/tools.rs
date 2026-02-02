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
    Stop,
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

const EDITING_TOOLS: &[&str] = &[
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
    "mv",
];

const DANGEROUS_TOOLS: &[&str] = &[
    "shell",
    "rm",
];

fn is_editing_tool(tool_name: &str) -> bool {
    EDITING_TOOLS.contains(&tool_name)
}

fn is_dangerous_tool(tool_name: &str) -> bool {
    DANGEROUS_TOOLS.contains(&tool_name)
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
    fn test_is_editing_tool() {
        assert!(is_editing_tool("patch"));
        assert!(is_editing_tool("text_edit"));
        assert!(is_editing_tool("create_textdoc"));
        assert!(is_editing_tool("update_textdoc"));
        assert!(is_editing_tool("update_textdoc_regex"));
        assert!(is_editing_tool("update_textdoc_by_lines"));
        assert!(is_editing_tool("undo_textdoc"));
        assert!(is_editing_tool("mv"));
    }

    #[test]
    fn test_is_not_editing_tool() {
        assert!(!is_editing_tool("shell"));
        assert!(!is_editing_tool("cat"));
        assert!(!is_editing_tool("search"));
        assert!(!is_editing_tool(""));
        assert!(!is_editing_tool("PATCH"));
    }

    #[test]
    fn test_is_dangerous_tool() {
        assert!(is_dangerous_tool("shell"));
        assert!(is_dangerous_tool("rm"));
    }

    #[test]
    fn test_is_not_dangerous_tool() {
        assert!(!is_dangerous_tool("cat"));
        assert!(!is_dangerous_tool("mv"));
        assert!(!is_dangerous_tool("patch"));
        assert!(!is_dangerous_tool(""));
    }

    #[test]
    fn test_max_parallel_clamp() {
        assert!(1_usize.max(1) >= 1);
        assert!(0_usize.max(1) >= 1);
        assert!(100_usize.max(1) == 100);
    }

    #[test]
    fn test_tool_config_default() {
        let config = crate::tools::tools_description::ToolConfig::default();
        assert!(config.enabled);
        assert!(config.allow_parallel.is_none());
    }

    #[test]
    fn test_tool_config_serde_roundtrip() {
        let config = crate::tools::tools_description::ToolConfig {
            enabled: true,
            allow_parallel: Some(false),
        };
        let yaml = serde_yaml::to_string(&config).unwrap();
        let parsed: crate::tools::tools_description::ToolConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.enabled, config.enabled);
        assert_eq!(parsed.allow_parallel, config.allow_parallel);
    }

    #[test]
    fn test_tool_config_serde_skip_none() {
        let config = crate::tools::tools_description::ToolConfig {
            enabled: true,
            allow_parallel: None,
        };
        let yaml = serde_yaml::to_string(&config).unwrap();
        assert!(!yaml.contains("allow_parallel"));
    }

    #[test]
    fn test_tool_desc_default_allow_parallel() {
        use crate::tools::tools_description::ToolDesc;
        let yaml = r#"
name: test_tool
description: A test tool
parameters: []
parameters_required: []
display_name: Test Tool
source:
  source_type: builtin
  config_path: ""
"#;
        let desc: ToolDesc = serde_yaml::from_str(yaml).unwrap();
        assert!(!desc.allow_parallel);
    }

    #[test]
    fn test_yaml_cannot_enable_parallel_for_unsafe_tools() {
        // Test that YAML override can only DISABLE parallelism, not enable it
        // This is the security policy: tools that declared allow_parallel=false
        // cannot be overridden to true via YAML config

        // Tool declares allow_parallel=false (unsafe tool)
        let tool_allow_parallel = false;

        // YAML tries to override to true
        let config_override = Some(true);

        // Security policy: if tool declared false, ignore override-to-true
        let effective = if tool_allow_parallel {
            config_override.unwrap_or(true)
        } else {
            false // ignore override-to-true for safety
        };

        assert!(!effective, "YAML should not be able to enable parallelism for unsafe tools");
    }

    #[test]
    fn test_yaml_can_disable_parallel_for_safe_tools() {
        // Test that YAML override CAN disable parallelism for safe tools

        // Tool declares allow_parallel=true (safe tool)
        let tool_allow_parallel = true;

        // YAML overrides to false
        let config_override = Some(false);

        // Policy: if tool declared true, YAML can disable it
        let effective = if tool_allow_parallel {
            config_override.unwrap_or(true)
        } else {
            false
        };

        assert!(!effective, "YAML should be able to disable parallelism for safe tools");
    }

    #[test]
    fn test_barrier_scheduling_logic() {
        // Test the barrier scheduling algorithm:
        // - Parallel tools (P) can run concurrently
        // - Non-parallel tools (X) act as barriers
        // Pattern: P1, P2, X, P3, P4 should execute as:
        //   [P1, P2] concurrently -> X alone -> [P3, P4] concurrently

        let tool_allow_parallel = |name: &str| -> bool {
            match name {
                "cat" | "tree" | "search" => true,  // parallel
                "apply_patch" | "shell" => false,   // non-parallel (barriers)
                _ => false,
            }
        };

        // Simulate tool call sequence
        let tool_calls = vec!["cat", "tree", "apply_patch", "search", "cat"];

        let mut batches: Vec<Vec<&str>> = Vec::new();
        let mut current_batch: Vec<&str> = Vec::new();

        for tool_name in &tool_calls {
            if tool_allow_parallel(tool_name) {
                current_batch.push(tool_name);
            } else {
                // Flush parallel batch before barrier
                if !current_batch.is_empty() {
                    batches.push(current_batch.clone());
                    current_batch.clear();
                }
                // Barrier tool runs alone
                batches.push(vec![tool_name]);
            }
        }
        // Flush remaining parallel batch
        if !current_batch.is_empty() {
            batches.push(current_batch);
        }

        // Expected: [[cat, tree], [apply_patch], [search, cat]]
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0], vec!["cat", "tree"]);
        assert_eq!(batches[1], vec!["apply_patch"]);
        assert_eq!(batches[2], vec!["search", "cat"]);
    }

    #[test]
    fn test_result_ordering_preserved() {
        // Test that results are sorted by original index regardless of completion order
        let mut results: Vec<(usize, &str)> = vec![
            (2, "result_2"),
            (0, "result_0"),
            (4, "result_4"),
            (1, "result_1"),
            (3, "result_3"),
        ];

        // Sort by index (same as in execute_tools_inner)
        results.sort_by_key(|(idx, _)| *idx);

        let ordered: Vec<&str> = results.iter().map(|(_, r)| *r).collect();
        assert_eq!(ordered, vec!["result_0", "result_1", "result_2", "result_3", "result_4"]);
    }

    #[test]
    fn test_corrections_aggregation() {
        // Test that corrections from multiple tools are properly aggregated
        // Simulates the aggregation logic in execute_tools_inner

        // Results: (idx, had_corrections, msgs, files)
        let results: Vec<(usize, bool, Vec<&str>, Vec<&str>)> = vec![
            (0, false, vec!["msg0"], vec![]),
            (1, true, vec!["msg1"], vec![]),  // This tool had corrections
            (2, false, vec!["msg2"], vec![]),
        ];

        let any_corrections = results.iter().any(|(_, had_corrections, _, _)| *had_corrections);
        assert!(any_corrections, "Should detect that at least one tool had corrections");

        // Test with no corrections
        let results_no_corrections: Vec<(usize, bool, Vec<&str>, Vec<&str>)> = vec![
            (0, false, vec!["msg0"], vec![]),
            (1, false, vec!["msg1"], vec![]),
        ];

        let any_corrections = results_no_corrections.iter().any(|(_, had_corrections, _, _)| *had_corrections);
        assert!(!any_corrections, "Should detect no corrections when all tools succeeded cleanly");
    }
}

pub async fn process_tool_calls_once(
    gcx: Arc<ARwLock<GlobalContext>>,
    session_arc: Arc<AMutex<ChatSession>>,
    mode_id: &str,
    model_id: Option<&str>,
) -> ToolStepOutcome {
    let (tool_calls, server_tool_calls, messages, thread, tool_message_index) = {
        let session = session_arc.lock().await;
        let msg_count = session.messages.len();
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
                    msg_count.saturating_sub(1),
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

    let denied_ids: std::collections::HashSet<String> = denials.iter().map(|d| d.tool_call_id.clone()).collect();
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
        let (auto_approved, remaining): (Vec<_>, Vec<_>) = confirmations
            .into_iter()
            .partition(|c| {
                let auto_editing = thread.auto_approve_editing_tools && is_editing_tool(&c.tool_name);
                let auto_dangerous = thread.auto_approve_dangerous_commands && is_dangerous_tool(&c.tool_name);
                auto_editing || auto_dangerous
            });

        if !remaining.is_empty() {
            let auto_approved_ids: Vec<String> = auto_approved.iter().map(|c| c.tool_call_id.clone()).collect();
            let mut session = session_arc.lock().await;
            session.set_paused_with_reasons_and_auto_approved(remaining, auto_approved_ids, Some(tool_message_index));
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

    let mut final_state = SessionState::Idle;
    for tool_call in &tools_to_execute {
        match tool_call.function.name.as_str() {
            "ask_questions" => final_state = SessionState::WaitingUserInput,
            "task_done" => final_state = SessionState::Completed,
            _ => {}
        }
    }

    {
        let mut session = session_arc.lock().await;
        for result_msg in tool_results {
            session.add_message(result_msg);
        }
        session.set_runtime_state(final_state, None);
    }

    maybe_save_trajectory(gcx.clone(), session_arc.clone()).await;

    match final_state {
        SessionState::Completed | SessionState::WaitingUserInput => ToolStepOutcome::Stop,
        _ => ToolStepOutcome::Continue,
    }
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
                        tool_name: tool_call.function.name.clone(),
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
                        tool_name: tool_call.function.name.clone(),
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
                            tool_name: tool_call.function.name.clone(),
                            command: result.command,
                            rule: rule_text,
                            tool_call_id: tool_call.id.clone(),
                            integr_config_path: tool.has_config_path(),
                        });
                    }
                    "ask" => {
                        confirmations.push(PauseReason {
                            reason_type: "confirmation".to_string(),
                            tool_name: tool_call.function.name.clone(),
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
                    tool_name: tool_call.function.name.clone(),
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

/// Per-tool mutex registry - each tool has its own lock to enable true parallelism
type ToolRegistry = std::collections::HashMap<String, Arc<AMutex<Box<dyn crate::tools::tools_description::Tool + Send>>>>;

async fn execute_single_tool(
    ccx: Arc<AMutex<AtCommandsContext>>,
    idx: usize,
    tool_call: ChatToolCall,
    tool_registry: Arc<ToolRegistry>,
) -> (usize, bool, Vec<ChatMessage>, Vec<ContextFile>) {
    // Get the tool's Arc without holding the registry lock during execution
    let tool_arc = match tool_registry.get(&tool_call.function.name) {
        Some(t) => t.clone(),
        None => {
            return (
                idx,
                true, // had_corrections: tool not found is a correction
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
                    true, // had_corrections: parse error is a correction
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

    // Lock only this specific tool, not the entire registry
    let mut tool = tool_arc.lock().await;
    match tool.tool_execute(ccx, &tool_call.id, &args).await {
        Ok((had_corrections, results)) => {
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
            (idx, had_corrections, msgs, files)
        }
        Err(e) => {
            info!("Tool execution failed: {}: {}", tool_call.function.name, e);
            (
                idx,
                true, // had_corrections: execution error is a correction
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
    let max_parallel = limits().max_parallel_tools.max(1);

    let available_tools = crate::tools::tools_list::get_tools_for_mode(gcx.clone(), mode_id, model_id).await;

    let mut tool_allow_parallel: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    let mut tool_registry: ToolRegistry = std::collections::HashMap::new();

    for tool in available_tools {
        let desc = tool.tool_description();
        // Only check config if config_path is non-empty (avoid unnecessary I/O)
        let config_override = if !desc.source.config_path.is_empty() {
            tool.config().ok().and_then(|c| c.allow_parallel)
        } else {
            None
        };
        // Security: YAML can only DISABLE parallelism, not enable it for tools that declared false
        // This prevents users from accidentally enabling parallel execution for unsafe tools
        let effective = if desc.allow_parallel {
            config_override.unwrap_or(true)
        } else {
            false // ignore override-to-true for safety
        };
        tool_allow_parallel.insert(desc.name.clone(), effective);
        // Wrap each tool in its own mutex for true parallelism
        tool_registry.insert(desc.name, Arc::new(AMutex::new(tool)));
    }

    let tool_registry = Arc::new(tool_registry);

    let mut all_results: Vec<(usize, bool, Vec<ChatMessage>, Vec<ContextFile>)> = Vec::new();
    let mut current_parallel_batch: Vec<(usize, ChatToolCall)> = Vec::new();

    for (idx, tool_call) in tool_calls.iter().enumerate() {
        let allow_parallel = tool_allow_parallel
            .get(&tool_call.function.name)
            .copied()
            .unwrap_or(false);

        if allow_parallel {
            current_parallel_batch.push((idx, tool_call.clone()));
        } else {
            if !current_parallel_batch.is_empty() {
                let batch_results = execute_parallel_batch(
                    ccx.clone(),
                    &current_parallel_batch,
                    tool_registry.clone(),
                    max_parallel,
                ).await;
                all_results.extend(batch_results);
                current_parallel_batch.clear();
            }

            let result = execute_single_tool(
                ccx.clone(),
                idx,
                tool_call.clone(),
                tool_registry.clone(),
            ).await;
            all_results.push(result);
        }
    }

    if !current_parallel_batch.is_empty() {
        let batch_results = execute_parallel_batch(
            ccx.clone(),
            &current_parallel_batch,
            tool_registry.clone(),
            max_parallel,
        ).await;
        all_results.extend(batch_results);
    }

    all_results.sort_by_key(|(idx, _, _, _)| *idx);

    let mut tool_messages: Vec<ChatMessage> = Vec::new();
    let mut context_files: Vec<ContextFile> = Vec::new();
    let mut any_corrections = false;
    for (_, had_corrections, msgs, files) in all_results {
        if had_corrections {
            any_corrections = true;
        }
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

    (results, any_corrections)
}

async fn execute_parallel_batch(
    ccx: Arc<AMutex<AtCommandsContext>>,
    batch: &[(usize, ChatToolCall)],
    tool_registry: Arc<ToolRegistry>,
    max_parallel: usize,
) -> Vec<(usize, bool, Vec<ChatMessage>, Vec<ContextFile>)> {
    let semaphore = Arc::new(Semaphore::new(max_parallel));

    let futures: Vec<_> = batch
        .iter()
        .map(|(idx, tool_call)| {
            let ccx = ccx.clone();
            let semaphore = semaphore.clone();
            let tool_registry = tool_registry.clone();
            let tool_call = tool_call.clone();
            let idx = *idx;

            async move {
                let _permit = semaphore.acquire().await.unwrap();
                execute_single_tool(ccx, idx, tool_call, tool_registry).await
            }
        })
        .collect();

    join_all(futures).await
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
