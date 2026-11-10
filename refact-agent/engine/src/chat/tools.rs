use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use chrono::Utc;
use tokio::sync::Mutex as AMutex;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tracing::info;
use uuid::Uuid;
use futures::future::join_all;

use indexmap::IndexMap;

use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use refact_buddy_core::user_action::UserAction;
use crate::call_validation::{
    ChatContent, ChatMessage, ChatToolCall, ContextFile, PostprocessSettings, SubchatParameters,
};
use crate::constants::CHAT_TOP_N;
use crate::postprocessing::pp_tool_results::{postprocess_tool_results, ToolBudget};
use crate::chat::perf_diagnostics::{self, PerfComponent, PerfOutcome, ToolExecutionClass};
use crate::yaml_configs::customization_registry::{
    get_mode_config, map_legacy_mode_to_id, match_tool_confirm_action,
};
use crate::ext::hooks::HookEvent;
use crate::ext::hooks_runner::{
    ControlVerdict, HookPayload, get_project_dir_string, revalidate_rewritten_tool_call,
    run_control_hooks,
};
use refact_runtime_api::{ToolCatalogSnapshot, TurnToolPool};
use refact_tool_api::MatchConfirmDenyResult;
use crate::exec::command_policy::escalation_from_args;

fn make_runtime_event(
    signal_type: &str,
    title: &str,
    source: &str,
    dedupe_key: &str,
    status: &str,
    priority: Option<&str>,
) -> refact_buddy_core::types::BuddyRuntimeEvent {
    refact_buddy_core::types::BuddyRuntimeEvent {
        id: Uuid::new_v4().to_string(),
        signal_type: signal_type.to_string(),
        title: title.to_string(),
        description: None,
        source: source.to_string(),
        status: status.to_string(),
        failure_category: None,
        failure_summary: None,
        progress: None,
        dedupe_key: Some(dedupe_key.to_string()),
        priority: priority.unwrap_or("normal").to_string(),
        created_at: Utc::now().to_rfc3339(),
        ttl_ms: None,
        bubble_policy: None,
        speech_text: None,
        scene: None,
        duration_hint: None,
        persistent: false,
        controls: Vec::new(),
        chat_id: None,
        dismissed: false,
    }
}

#[derive(Default)]
pub struct ExecuteToolsOptions {
    pub subchat_tool_parameters: Option<IndexMap<String, SubchatParameters>>,
    pub postprocess_settings: Option<PostprocessSettings>,
    pub allowed_tools: Vec<String>,
    pub catalog: Option<Arc<ToolCatalogSnapshot>>,
    pub turn_tool_pool: Option<TurnToolPool>,
    pub(crate) execution_context: Option<Arc<AMutex<AtCommandsContext>>>,
    /// Present only for authority-supervised native subchats. The gate is
    /// checked immediately before every native tool invocation, rather than
    /// only before the enclosing tool turn.
    pub native_pause: Option<Arc<refact_agents_core::registry::PauseGate>>,
    /// The runtime-owned abort flag paired with `native_pause`.
    pub native_abort: Option<Arc<AtomicBool>>,
}

fn tool_execution_messages(
    messages: &[ChatMessage],
    last_prompt_messages: &[ChatMessage],
) -> Vec<ChatMessage> {
    if last_prompt_messages.is_empty() {
        messages.to_vec()
    } else {
        last_prompt_messages.to_vec()
    }
}

fn tool_timestamp_ms() -> Option<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .filter(|timestamp| *timestamp > 0)
}

pub(crate) const SHELL_PRIVACY_APPROVAL_RULE: &str =
    "Privacy approval required: command read guarded files";

pub(crate) fn requires_actual_human_approval(reason: &str) -> bool {
    reason == crate::scheduler::automation_approval::HUMAN_AUTOMATION_APPROVAL
}

pub(crate) fn is_shell_privacy_approval(reason: &PauseReason) -> bool {
    reason.rule == SHELL_PRIVACY_APPROVAL_RULE
}

pub(crate) fn shell_privacy_approval_reasons(
    tool_calls: &[ChatToolCall],
    tool_results: &[ChatMessage],
) -> Vec<PauseReason> {
    tool_results
        .iter()
        .filter(|message| crate::privacy::records::shell_ask_pending(message))
        .filter_map(|message| {
            let tool_call = tool_calls
                .iter()
                .find(|tool_call| tool_call.id == message.tool_call_id)?;
            Some(PauseReason {
                reason_type: "confirmation".to_string(),
                tool_name: tool_call.function.name.clone(),
                command: tool_call.function.name.clone(),
                rule: SHELL_PRIVACY_APPROVAL_RULE.to_string(),
                tool_call_id: tool_call.id.clone(),
                integr_config_path: None,
            })
        })
        .collect()
}

pub enum ToolStepOutcome {
    NoToolCalls,
    Paused,
    Continue,
    Stop,
}

use super::types::*;
use super::trajectories::{
    maybe_save_trajectory_with_intent, maybe_save_trajectory_background_with_intent,
};
use super::goal_verifier::{
    should_verify_goal_on_done, verify_goal_before_completion, GoalCompletionGateOutcome,
};

use super::config::tokens;

async fn get_effective_n_ctx(app: AppState, thread: &ThreadParams) -> usize {
    let default_n_ctx = tokens().default_n_ctx;
    let model_n_ctx =
        match crate::global_context::try_load_caps_quickly_if_not_present(app.gcx.clone(), 0).await
        {
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

async fn build_tool_execution_context(
    app: AppState,
    n_ctx: usize,
    messages: &[ChatMessage],
    thread: &ThreadParams,
    abort_flag: Option<Arc<AtomicBool>>,
) -> Result<Arc<AMutex<AtCommandsContext>>, String> {
    let messages = refact_core::active_context::active_context(messages)
        .map_err(|error| error.to_string())?
        .messages;
    let ccx = AtCommandsContext::new_with_abort(
        app.clone(),
        n_ctx,
        CHAT_TOP_N,
        false,
        messages,
        thread.id.clone(),
        thread.root_chat_id.clone(),
        thread.model.clone(),
        thread.task_meta.clone(),
        thread.worktree.clone(),
        abort_flag,
    )
    .await;
    Ok(Arc::new(AMutex::new(ccx)))
}

fn context_error_results(tool_calls: &[ChatToolCall], error: String) -> (Vec<ChatMessage>, bool) {
    (
        tool_calls
            .iter()
            .map(|call| ChatMessage {
                message_id: Uuid::new_v4().to_string(),
                role: "tool".to_string(),
                content: ChatContent::SimpleText(format!("Invalid active context: {error}")),
                tool_call_id: call.id.clone(),
                tool_failed: Some(true),
                ..Default::default()
            })
            .collect(),
        false,
    )
}

fn execution_scope_for_thread(thread: &ThreadParams) -> Option<String> {
    thread
        .worktree
        .as_ref()
        .map(|worktree| worktree.root.to_string_lossy().into_owned())
}

pub(crate) async fn acquire_session_turn_tool_pool(
    app: &AppState,
    session_arc: &Arc<AMutex<ChatSession>>,
    thread: &ThreadParams,
    mode_id: &str,
    model_id: Option<&str>,
    catalog: &ToolCatalogSnapshot,
) -> Option<TurnToolPool> {
    let existing = { session_arc.lock().await.turn_tool_pool.clone() };
    if existing.is_some() {
        return existing;
    }
    let pool = app
        .tool_registry
        .acquire_turn_tool_pool(
            mode_id,
            model_id,
            execution_scope_for_thread(thread).as_deref(),
            catalog,
        )
        .await;
    if let Some(pool) = &pool {
        session_arc.lock().await.turn_tool_pool = Some(pool.clone());
    }
    pool
}

fn is_server_executed_tool(tool_call_id: &str) -> bool {
    tool_call_id.starts_with("srvtoolu_")
}

fn rewrite_cc_native_stub_tool_call(tc: &mut ChatToolCall) {
    let name = tc.function.name.as_str();
    if name != "Grep" && name != "Glob" {
        return;
    }

    let args = tc.function.parse_args().unwrap_or_default();
    let mut mapped = serde_json::Map::new();
    let canonical_name = match name {
        "Grep" => {
            let pattern = args
                .get("pattern")
                .cloned()
                .unwrap_or_else(|| serde_json::Value::String(String::new()));
            let scope = args
                .get("path")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("workspace");
            mapped.insert("pattern".to_string(), pattern);
            mapped.insert(
                "scope".to_string(),
                serde_json::Value::String(scope.to_string()),
            );
            "search_pattern"
        }
        "Glob" => {
            let pattern = args
                .get("pattern")
                .cloned()
                .unwrap_or_else(|| serde_json::Value::String(String::new()));
            mapped.insert("pattern".to_string(), pattern);
            if let Some(path) = args
                .get("path")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                mapped.insert(
                    "path".to_string(),
                    serde_json::Value::String(path.to_string()),
                );
            }
            "glob"
        }
        _ => return,
    };

    tc.function.name = canonical_name.to_string();
    tc.function.arguments = serde_json::Value::Object(mapped).to_string();
}

pub fn resolve_tool_call_aliases_with_catalog(
    tool_calls: Vec<ChatToolCall>,
    catalog: &ToolCatalogSnapshot,
) -> Vec<ChatToolCall> {
    let tool_names = &catalog.index.tools;
    let registry = &catalog.aliases;
    let needs_cc = tool_calls.iter().any(|tc| {
        tc.function
            .name
            .starts_with(crate::llm::adapters::claude_code_compat::MCP_TOOL_PREFIX)
    });
    let needs_cc_native = tool_calls
        .iter()
        .any(|tc| matches!(tc.function.name.as_str(), "Grep" | "Glob"));
    let item_count = tool_calls.len() as u64;
    let span = perf_diagnostics::span(PerfComponent::ToolAliasResolution, None, None);
    if !registry.needs_aliasing() && !needs_cc && !needs_cc_native {
        span.finish_tool(PerfOutcome::Success, 1, item_count, None);
        return tool_calls;
    }
    let resolved = tool_calls
        .into_iter()
        .map(|mut tc| {
            if matches!(tc.function.name.as_str(), "Grep" | "Glob") {
                rewrite_cc_native_stub_tool_call(&mut tc);
                return tc;
            }
            if tc
                .function
                .name
                .starts_with(crate::llm::adapters::claude_code_compat::MCP_TOOL_PREFIX)
            {
                let cc_resolved = crate::llm::adapters::claude_code_compat::cc_resolve_tool_name(
                    &tc.function.name,
                );
                let lookup_name = if cc_resolved != tc.function.name {
                    &cc_resolved
                } else {
                    &tc.function.name
                };
                if let Some(internal_name) = registry.resolve_alias(lookup_name) {
                    tc.function.name = internal_name.to_string();
                } else {
                    tc.function.name = cc_resolved;
                }
            } else if let Some(internal_name) = registry.resolve_alias(&tc.function.name) {
                tc.function.name = internal_name.to_string();
            } else if needs_cc && !tool_names.iter().any(|tool| tool.name == tc.function.name) {
                let cc_resolved = crate::llm::adapters::claude_code_compat::cc_resolve_tool_name(
                    &tc.function.name,
                );
                if let Some(internal_name) = registry.resolve_alias(&cc_resolved) {
                    tc.function.name = internal_name.to_string();
                } else {
                    tc.function.name = cc_resolved;
                }
            }
            tc
        })
        .collect::<Vec<_>>();
    span.finish_tool(PerfOutcome::Success, 1, item_count, None);
    resolved
}

pub async fn resolve_tool_call_aliases(
    app: AppState,
    tool_calls: Vec<ChatToolCall>,
    mode_id: &str,
    model_id: Option<&str>,
) -> Vec<ChatToolCall> {
    let catalog = app
        .tool_registry
        .acquire_tool_catalog(mode_id, model_id, None)
        .await;
    resolve_tool_call_aliases_with_catalog(tool_calls, &catalog)
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

fn is_editing_tool(tool_name: &str) -> bool {
    EDITING_TOOLS.contains(&tool_name)
}

// Tools whose results are expensive, non-reproducible subagent outcomes bypass the shared per-round
// ToolBudget so running many of them at once does not chop each report to a
// fair-share fragment; overall context pressure is handled later by chat
// compression instead.
const TOOL_RESULT_TRUNCATION_EXEMPT_TOOLS: &[&str] = &["subagent", "wait", "agent_result"];

fn is_tool_result_truncation_exempt(tool_name: &str) -> bool {
    TOOL_RESULT_TRUNCATION_EXEMPT_TOOLS.contains(&tool_name)
}

fn should_auto_approve_confirmation(thread: &ThreadParams, tool_name: &str) -> bool {
    if thread.auto_approve_dangerous_commands {
        return true;
    }
    if matches!(
        tool_name,
        "agent_approval" | "agent_lifecycle" | "pause" | "resume"
    ) {
        return false;
    }
    if thread.autonomous_no_confirm {
        return true;
    }
    thread.auto_approve_editing_tools && is_editing_tool(tool_name)
}

async fn record_tool_activity(
    app: AppState,
    tool_calls: &[ChatToolCall],
    chat_id: &str,
    approved_ids: &std::collections::HashSet<String>,
    denied_ids: &std::collections::HashSet<String>,
) {
    if approved_ids.is_empty() && denied_ids.is_empty() {
        return;
    }
    for tc in tool_calls {
        if denied_ids.contains(&tc.id) {
            app.activity_sink
                .record_user_action(UserAction::ToolRejected {
                    tool_name: tc.function.name.clone(),
                    chat_id: chat_id.to_string(),
                    ts: Utc::now(),
                })
                .await;
        } else if approved_ids.contains(&tc.id) {
            app.activity_sink
                .record_user_action(UserAction::ToolApproved {
                    tool_name: tc.function.name.clone(),
                    chat_id: chat_id.to_string(),
                    ts: Utc::now(),
                })
                .await;
        }
    }
}

fn tool_pool_key(desc: &refact_tool_api::ToolDesc) -> String {
    format!(
        "{}\u{1f}{:?}\u{1f}{}",
        crate::llm::adapters::claude_code_compat::cc_resolve_tool_name(&desc.name),
        desc.source.source_type,
        desc.source.config_path
    )
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

struct SubchatBridge {
    cancel_flag: Arc<AtomicBool>,
    cancel_notify: Arc<Notify>,
    task: Option<JoinHandle<()>>,
}

impl SubchatBridge {
    async fn shutdown(mut self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
        self.cancel_notify.notify_one();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for SubchatBridge {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

fn spawn_subchat_bridge(
    ccx: Arc<AMutex<AtCommandsContext>>,
    session_arc: Arc<AMutex<ChatSession>>,
) -> SubchatBridge {
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_flag_clone = cancel_flag.clone();
    let cancel_notify = Arc::new(Notify::new());
    let cancel_notify_clone = cancel_notify.clone();

    let task = tokio::spawn(async move {
        let (subchat_rx, abort_flag) = {
            let cgcx = ccx.lock().await;
            (cgcx.subchat_rx.clone(), cgcx.abort_flag.clone())
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

            let recv_result = tokio::select! {
                _ = cancel_notify_clone.notified() => {
                    cancel_flag_clone.store(true, Ordering::Relaxed);
                    continue;
                }
                recv_result = async {
                    let mut rx = subchat_rx.lock().await;
                    tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await
                } => recv_result,
            };

            match recv_result {
                Ok(Some(value)) => {
                    info!(
                        "spawn_subchat_bridge: received message: {}b",
                        value.to_string().len()
                    );
                    let tool_call_id = value.get("tool_call_id").and_then(|v| v.as_str());
                    let subchat_id = value.get("subchat_id").and_then(|v| v.as_str());

                    if let (Some(tool_call_id), Some(subchat_id)) = (tool_call_id, subchat_id) {
                        info!(
                            "spawn_subchat_bridge: emitting SubchatUpdate for tool_call_id={}, subchat_id={}b",
                            tool_call_id,
                            subchat_id.len()
                        );

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
                        session.mark_tool_progress();
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

    SubchatBridge {
        cancel_flag,
        cancel_notify,
        task: Some(task),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
    use crate::chat::perf_diagnostics::{MemoryPerfSink, PerfClock, PerfRecorder};
    use serial_test::serial;

    struct TestPerfClock {
        now: AtomicU64,
    }

    impl TestPerfClock {
        fn new() -> Self {
            Self {
                now: AtomicU64::new(0),
            }
        }
    }

    impl PerfClock for TestPerfClock {
        fn now_us(&self) -> u64 {
            self.now.fetch_add(1, AtomicOrdering::SeqCst)
        }
    }

    fn install_perf_recorder() -> (perf_diagnostics::TestRecorderGuard, Arc<MemoryPerfSink>) {
        let sink = Arc::new(MemoryPerfSink::new());
        let recorder = Arc::new(PerfRecorder::with_salt(
            Arc::new(TestPerfClock::new()),
            sink.clone(),
            [9; 32],
        ));
        (perf_diagnostics::install_test_recorder(recorder), sink)
    }

    #[derive(Clone)]
    struct DeterministicToolRegistry {
        confirmation_result: MatchConfirmDenyResult,
        runtime_failure: bool,
    }

    fn deterministic_tool_desc(name: &str) -> refact_tool_api::ToolDesc {
        refact_tool_api::ToolDesc {
            name: name.to_string(),
            experimental: false,
            allow_parallel: true,
            description: format!("Deterministic {name} tool"),
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
            output_schema: None,
            annotations: None,
            display_name: name.to_string(),
            source: refact_tool_api::ToolSource {
                source_type: refact_tool_api::ToolSourceType::Builtin,
                config_path: String::new(),
            },
        }
    }

    struct DeterministicTool {
        name: String,
        confirmation_result: MatchConfirmDenyResult,
        runtime_failure: bool,
    }

    #[async_trait::async_trait]
    impl crate::tools::tools_description::Tool for DeterministicTool {
        async fn tool_execute(
            &mut self,
            _ccx: Arc<AMutex<AtCommandsContext>>,
            tool_call_id: &String,
            _args: &std::collections::HashMap<String, serde_json::Value>,
        ) -> Result<(bool, Vec<crate::call_validation::ContextEnum>), String> {
            if self.runtime_failure {
                return Err("deterministic runtime failure".to_string());
            }
            let delay = if self.name == "wait" {
                std::time::Duration::from_secs(8)
            } else {
                std::time::Duration::from_millis(20)
            };
            tokio::time::sleep(delay).await;
            let mut message =
                ChatMessage::new("tool".to_string(), "deterministic result".to_string());
            message.tool_call_id = tool_call_id.clone();
            Ok((
                false,
                vec![crate::call_validation::ContextEnum::ChatMessage(message)],
            ))
        }

        fn tool_description(&self) -> refact_tool_api::ToolDesc {
            deterministic_tool_desc(&self.name)
        }

        async fn match_against_confirm_deny(
            &self,
            _ccx: Arc<AMutex<AtCommandsContext>>,
            _args: &std::collections::HashMap<String, serde_json::Value>,
        ) -> Result<refact_tool_api::MatchConfirmDeny, String> {
            Ok(refact_tool_api::MatchConfirmDeny {
                result: self.confirmation_result.clone(),
                command: self.name.clone(),
                rule: "deterministic confirmation".to_string(),
            })
        }
    }

    impl DeterministicToolRegistry {
        fn tools(&self) -> Vec<Box<dyn crate::tools::tools_description::Tool + Send>> {
            ["alpha", "beta", "wait"]
                .into_iter()
                .map(|name| {
                    Box::new(DeterministicTool {
                        name: name.to_string(),
                        confirmation_result: self.confirmation_result.clone(),
                        runtime_failure: self.runtime_failure,
                    }) as Box<dyn crate::tools::tools_description::Tool + Send>
                })
                .collect()
        }
    }

    #[derive(Clone)]
    struct BarrierToolRegistry {
        started: Arc<AtomicUsize>,
        barrier: Arc<tokio::sync::Barrier>,
    }

    struct BarrierTool {
        started: Arc<AtomicUsize>,
        barrier: Arc<tokio::sync::Barrier>,
    }

    #[async_trait::async_trait]
    impl crate::tools::tools_description::Tool for BarrierTool {
        async fn tool_execute(
            &mut self,
            _ccx: Arc<AMutex<AtCommandsContext>>,
            tool_call_id: &String,
            _args: &std::collections::HashMap<String, serde_json::Value>,
        ) -> Result<(bool, Vec<crate::call_validation::ContextEnum>), String> {
            self.started.fetch_add(1, AtomicOrdering::SeqCst);
            self.barrier.wait().await;
            let mut message = ChatMessage::new("tool".to_string(), "released".to_string());
            message.tool_call_id = tool_call_id.clone();
            Ok((
                false,
                vec![crate::call_validation::ContextEnum::ChatMessage(message)],
            ))
        }

        fn tool_description(&self) -> refact_tool_api::ToolDesc {
            deterministic_tool_desc("parallel_barrier")
        }

        async fn match_against_confirm_deny(
            &self,
            _ccx: Arc<AMutex<AtCommandsContext>>,
            _args: &std::collections::HashMap<String, serde_json::Value>,
        ) -> Result<refact_tool_api::MatchConfirmDeny, String> {
            Ok(refact_tool_api::MatchConfirmDeny {
                result: MatchConfirmDenyResult::PASS,
                command: "parallel_barrier".to_string(),
                rule: "parallel test".to_string(),
            })
        }
    }

    impl BarrierToolRegistry {
        fn tools(&self) -> Vec<Box<dyn crate::tools::tools_description::Tool + Send>> {
            vec![
                Box::new(BarrierTool {
                    started: self.started.clone(),
                    barrier: self.barrier.clone(),
                }),
                Box::new(crate::tools::tool_tool_job::ToolToolJob {
                    config_path: String::new(),
                }),
            ]
        }
    }

    #[derive(Clone)]
    struct SerialPauseToolRegistry {
        control: Arc<SerialPauseToolControl>,
    }

    struct SerialPauseToolControl {
        invocations: AtomicUsize,
        first_started: AMutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release_first: AMutex<tokio::sync::oneshot::Receiver<()>>,
    }

    struct SerialPauseTool {
        control: Arc<SerialPauseToolControl>,
    }

    #[async_trait::async_trait]
    impl crate::tools::tools_description::Tool for SerialPauseTool {
        async fn tool_execute(
            &mut self,
            _ccx: Arc<AMutex<AtCommandsContext>>,
            tool_call_id: &String,
            _args: &std::collections::HashMap<String, serde_json::Value>,
        ) -> Result<(bool, Vec<crate::call_validation::ContextEnum>), String> {
            if self
                .control
                .invocations
                .fetch_add(1, AtomicOrdering::SeqCst)
                == 0
            {
                if let Some(first_started) = self.control.first_started.lock().await.take() {
                    let _ = first_started.send(());
                }
                let mut release_first = self.control.release_first.lock().await;
                let _ = (&mut *release_first).await;
            }
            let mut message = ChatMessage::new("tool".to_string(), "serial result".to_string());
            message.tool_call_id = tool_call_id.clone();
            Ok((
                false,
                vec![crate::call_validation::ContextEnum::ChatMessage(message)],
            ))
        }

        fn tool_description(&self) -> refact_tool_api::ToolDesc {
            let mut desc = deterministic_tool_desc("serial_pause");
            desc.allow_parallel = false;
            desc
        }

        async fn match_against_confirm_deny(
            &self,
            _ccx: Arc<AMutex<AtCommandsContext>>,
            _args: &std::collections::HashMap<String, serde_json::Value>,
        ) -> Result<refact_tool_api::MatchConfirmDeny, String> {
            Ok(refact_tool_api::MatchConfirmDeny {
                result: MatchConfirmDenyResult::PASS,
                command: "serial_pause".to_string(),
                rule: "serial pause test".to_string(),
            })
        }
    }

    impl SerialPauseToolRegistry {
        fn tools(&self) -> Vec<Box<dyn crate::tools::tools_description::Tool + Send>> {
            vec![Box::new(SerialPauseTool {
                control: self.control.clone(),
            })]
        }
    }

    async fn serial_pause_execute_fixture() -> (
        AppState,
        ThreadParams,
        Arc<SerialPauseToolControl>,
        tokio::sync::oneshot::Receiver<()>,
        tokio::sync::oneshot::Sender<()>,
    ) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let base_app = AppState::from_gcx(gcx).await;
        let (first_started, first_started_rx) = tokio::sync::oneshot::channel();
        let (release_first, release_first_rx) = tokio::sync::oneshot::channel();
        let control = Arc::new(SerialPauseToolControl {
            invocations: AtomicUsize::new(0),
            first_started: AMutex::new(Some(first_started)),
            release_first: AMutex::new(release_first_rx),
        });
        let fixture = SerialPauseToolRegistry {
            control: control.clone(),
        };
        let app = AppState {
            tool_registry: Arc::new(
                crate::app_state::AppToolRegistry::with_fixture_tool_factory(
                    base_app.gcx.clone(),
                    Arc::new(move || fixture.tools()),
                ),
            ),
            ..base_app
        };
        let thread = ThreadParams {
            id: "serial-pause-tools".to_string(),
            model: "model".to_string(),
            ..Default::default()
        };
        (app, thread, control, first_started_rx, release_first)
    }

    fn deterministic_tool_call(id: &str, name: &str) -> ChatToolCall {
        ChatToolCall {
            id: id.to_string(),
            index: None,
            function: crate::call_validation::ChatToolFunction {
                name: name.to_string(),
                arguments: "{}".to_string(),
            },
            tool_type: "function".to_string(),
            extra_content: None,
            started_at_ms: None,
            completed_at_ms: None,
        }
    }

    async fn process_deterministic_tool_turn(
        registry: DeterministicToolRegistry,
        tool_calls: Vec<ChatToolCall>,
    ) -> (
        ToolStepOutcome,
        Arc<MemoryPerfSink>,
        Arc<AMutex<ChatSession>>,
    ) {
        let _lock = perf_diagnostics::PERF_RECORDER_TEST_LOCK.lock().unwrap();
        let (_guard, sink) = install_perf_recorder();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let base_app = AppState::from_gcx(gcx).await;
        let test_tools = registry.clone();
        let app = AppState {
            tool_registry: Arc::new(
                crate::app_state::AppToolRegistry::with_fixture_tool_factory(
                    base_app.gcx.clone(),
                    Arc::new(move || test_tools.tools()),
                ),
            ),
            ..base_app
        };
        let session = Arc::new(AMutex::new(ChatSession::new("same-turn-tools".to_string())));
        {
            let mut locked = session.lock().await;
            locked.thread.model = "model".to_string();
            let mut assistant = ChatMessage::new("assistant".to_string(), String::new());
            assistant.tool_calls = Some(tool_calls);
            locked.add_message(assistant);
        }
        app.chat
            .sessions
            .write()
            .await
            .insert("same-turn-tools".to_string(), session.clone());

        let outcome = process_tool_calls_once(app, session.clone(), "agent", None).await;
        (outcome, sink, session)
    }

    fn recorded_components(sink: &MemoryPerfSink) -> Vec<&'static str> {
        sink.events().iter().map(|event| event.component).collect()
    }

    #[test]
    fn job_policy_exempts_control_not_ordinary_work() {
        for name in [
            "ask_questions",
            "tool_job",
            "wait",
            "worktree_merge",
            "set_review_context",
            "subagent",
            "spawn_agent",
            "merge_agent",
            "agent_lifecycle",
        ] {
            assert!(tool_job_foreground_reason(name).is_some(), "{name}");
        }
        for name in [
            "cat",
            "code_health",
            "web",
            "apply_patch",
            "parallel_barrier",
        ] {
            assert!(tool_job_foreground_reason(name).is_none(), "{name}");
        }
    }

    #[tokio::test]
    async fn actual_dispatch_review_activation_keeps_live_scope_and_does_not_replay() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        fs::create_dir(&source).unwrap();
        let oid = {
            let repo = git2::Repository::init(&source).unwrap();
            let mut tree = repo.treebuilder(None).unwrap();
            tree.insert(
                "tracked.txt",
                repo.blob(b"review input\n").unwrap(),
                0o100644,
            )
            .unwrap();
            let tree = repo.find_tree(tree.write().unwrap()).unwrap();
            let sig = git2::Signature::now("Review", "review@localhost").unwrap();
            let oid = repo
                .commit(Some("HEAD"), &sig, &sig, "fixture", &tree, &[])
                .unwrap();
            repo.checkout_head(None).unwrap();
            oid.to_string()
        };
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            temp.path().join("cache"),
            temp.path().join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        let base = AppState::from_gcx(gcx).await;
        let app = AppState {
            tool_registry: Arc::new(
                crate::app_state::AppToolRegistry::with_fixture_tool_factory(
                    base.gcx.clone(),
                    Arc::new(|| {
                        vec![Box::new(crate::tools::tool_review::ToolReview(
                            crate::tools::tool_review::ReviewToolKind::Context,
                        ))]
                    }),
                ),
            ),
            ..base
        };
        let mut live = ChatSession::new("review-actual-dispatch".into());
        live.thread.model = "model".into();
        live.work_project_root = Some(source.clone());
        live.set_runtime_state(SessionState::ExecutingTools, None);
        let mut user = ChatMessage::new(
            "user".into(),
            "Review tracked.txt in this local comparison".into(),
        );
        user.message_id = "dispatch-review-user".into();
        live.add_message(user);
        let mut source_call = deterministic_tool_call("dispatch-source", "cat");
        source_call.function.arguments =
            serde_json::json!({"paths":source.join("tracked.txt")}).to_string();
        let mut source_invocation = ChatMessage::new("assistant".into(), String::new());
        source_invocation.tool_calls = Some(vec![source_call]);
        live.add_message(source_invocation);
        // Captured native IO fixture with actual file content.
        let mut source_result = ChatMessage::new(
            "tool".into(),
            fs::read_to_string(source.join("tracked.txt")).unwrap(),
        );
        source_result.tool_call_id = "dispatch-source".into();
        source_result.tool_failed = Some(false);
        live.add_message(source_result);
        let thread = live.thread.clone();
        let abort = live.abort_flag.clone();
        let session = Arc::new(AMutex::new(live));
        app.chat
            .sessions
            .write()
            .await
            .insert(thread.id.clone(), session.clone());
        let context =
            build_tool_execution_context(app.clone(), 4096, &[], &thread, Some(abort.clone()))
                .await
                .unwrap();
        let mut call = deterministic_tool_call("review-activation", "set_review_context");
        call.function.arguments = serde_json::json!({
            "repository":session.lock().await.work_project_root,
            "base_ref":oid,"candidate_ref":oid,"request_key":"dispatch-capture"
        })
        .to_string();

        let mut first = None;
        let mut first_attachment = None;
        for _ in 0..2 {
            call.id = format!("review-activation-{}", uuid::Uuid::new_v4());
            let mut invocation = ChatMessage::new("assistant".into(), String::new());
            invocation.tool_calls = Some(vec![call.clone()]);
            session.lock().await.add_message(invocation);
            let thread = session.lock().await.thread.clone();
            let (messages, corrections) = tokio::time::timeout(
                std::time::Duration::from_secs(20),
                execute_tools(
                    app.clone(),
                    &[call.clone()],
                    &[],
                    &thread,
                    "agent",
                    None,
                    ExecuteToolsOptions {
                        allowed_tools: vec!["set_review_context".into()],
                        execution_context: Some(context.clone()),
                        ..Default::default()
                    },
                ),
            )
            .await
            .expect("scope writer must not wait for its own job lease");
            assert!(!corrections, "{messages:?}");
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].tool_call_id, call.id);
            assert_eq!(messages[0].tool_failed, Some(false), "{messages:?}");
            session.lock().await.add_message(messages[0].clone());
            assert!(!messages[0].extra.contains_key("tool_job"));
            assert_eq!(
                messages[0].extra["tool_execution_policy"]["foreground_reason"],
                "scope_writer"
            );
            let value: serde_json::Value =
                serde_json::from_str(&messages[0].content.content_text_only()).unwrap();
            let review_id = value["review"]["id"].as_str().unwrap();
            let candidate_id = value["candidate_id"].as_str().unwrap();
            assert!(!review_id.is_empty());
            assert!(!candidate_id.is_empty());
            let identity = (review_id.to_string(), candidate_id.to_string());
            if let Some(first) = &first {
                assert_eq!(first, &identity);
            } else {
                first = Some(identity);
            }
            let c = context.lock().await;
            assert!(Arc::ptr_eq(&c.abort_flag, &abort));
            let attached = session.lock().await.thread.worktree.clone().unwrap();
            assert_eq!(attached.kind, "review");
            assert_eq!(c.execution_scope_worktree(), Some(attached.clone()));
            if let Some(first) = &first_attachment {
                assert_eq!(first, &attached);
            } else {
                first_attachment = Some(attached);
            }
        }
        app.runtime.tool_jobs.shutdown().await;
    }
    #[tokio::test]
    async fn actual_dispatch_returns_receipt_then_retains_one_native_result() {
        let started = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let fixture = BarrierToolRegistry {
            started: started.clone(),
            barrier: barrier.clone(),
        };
        let base = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let app = AppState {
            tool_registry: Arc::new(
                crate::app_state::AppToolRegistry::with_fixture_tool_factory(
                    base.gcx.clone(),
                    Arc::new(move || fixture.tools()),
                ),
            ),
            ..base
        };
        app.runtime.tool_jobs.set_test_inline_budget(None);
        let thread = ThreadParams {
            id: "owned-job-dispatch".into(),
            model: "model".into(),
            ..Default::default()
        };
        let call = deterministic_tool_call("owned-job-call", "parallel_barrier");
        let (messages, corrections) = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            execute_tools(
                app.clone(),
                &[call],
                &[],
                &thread,
                "agent",
                None,
                ExecuteToolsOptions::default(),
            ),
        )
        .await
        .unwrap();
        assert!(!corrections);
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].tool_failed, None,
            "acceptance must not claim execution success"
        );
        assert_eq!(messages[0].tool_call_id, "owned-job-call");
        let id = messages[0].extra["tool_job"]["job_id"].as_str().unwrap();
        wait_for_count(&started, 1).await;
        assert_eq!(started.load(AtomicOrdering::SeqCst), 1);
        barrier.wait().await;
        let snapshot = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            app.runtime.tool_jobs.wait_terminal(&thread.id, id),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(snapshot.status, crate::tool_jobs::JobStatus::Succeeded);
        assert_eq!(
            snapshot.result.unwrap().messages[0]
                .content
                .content_text_only(),
            "released"
        );
        assert_eq!(started.load(AtomicOrdering::SeqCst), 1);
        assert!(app.runtime.tool_jobs.get("another-chat", id).is_none());
        app.runtime.tool_jobs.shutdown().await;
    }

    #[tokio::test]
    async fn custom_catalog_without_job_control_returns_receipt_and_auto_lands() {
        let chat = "custom-no-control";
        let (_temp, base, session, old_receipt) =
            super::super::tool_job_delivery::tests::policy_race_fixture(chat).await;
        session.lock().await.pending_deliveries.clear();
        let old = base
            .runtime
            .tool_jobs
            .wait_terminal(chat, &old_receipt.job_id)
            .await
            .unwrap();
        base.runtime
            .tool_jobs
            .ack_delivered(chat, &old_receipt.job_id, old.revision)
            .await
            .unwrap();
        let mode = "custom_no_job_control";
        let mut registry =
            crate::yaml_configs::customization_registry::get_project_registry(base.gcx.clone())
                .await
                .unwrap();
        let mut custom = registry.modes.get("agent").unwrap().clone();
        custom.id = mode.into();
        custom.tools = vec!["parallel_barrier".into()];
        custom.allow_integrations = false;
        custom.allow_mcp = false;
        custom.allow_subagents = false;
        registry.modes.insert(mode.into(), custom);
        let root = session.lock().await.work_project_root.clone().unwrap();
        base.gcx
            .project_registry_cache
            .write()
            .unwrap()
            .insert(root, registry);
        base.gcx.tool_catalog_generations.advance_customization();
        session.lock().await.thread.mode = mode.into();
        let started = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let fixture = BarrierToolRegistry {
            started: started.clone(),
            barrier: barrier.clone(),
        };
        let app = AppState {
            tool_registry: Arc::new(
                crate::app_state::AppToolRegistry::with_fixture_tool_factory(
                    base.gcx.clone(),
                    Arc::new(move || {
                        fixture
                            .tools()
                            .into_iter()
                            .filter(|tool| tool.tool_description().name != "tool_job")
                            .collect()
                    }),
                ),
            ),
            ..base
        };
        app.runtime.tool_jobs.set_test_inline_budget(None);
        let thread = session.lock().await.thread.clone();
        let catalog = app
            .tool_registry
            .acquire_tool_catalog(mode, Some("model"), None)
            .await;
        assert_eq!(catalog.index.tools.len(), 1);
        assert_eq!(catalog.index.tools[0].name, "parallel_barrier");
        assert!(!catalog
            .index
            .tools
            .iter()
            .any(|tool| tool.name == "tool_job"));
        let (messages, _) = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            execute_tools(
                app.clone(),
                &[deterministic_tool_call("custom-call", "parallel_barrier")],
                &[],
                &thread,
                mode,
                Some("model"),
                ExecuteToolsOptions {
                    allowed_tools: vec!["parallel_barrier".into()],
                    catalog: Some(catalog),
                    ..Default::default()
                },
            ),
        )
        .await
        .expect("receipt must arrive while tool remains behind barrier");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tool_failed, None);
        assert_eq!(messages[0].tool_call_id, "custom-call");
        assert_eq!(messages[0].extra["tool_job"]["status"], "accepted");
        let text = messages[0].content.content_text_only();
        assert!(text.contains("arrive automatically"));
        assert!(!text.contains("Use tool_job"));
        let id = messages[0].extra["tool_job"]["job_id"]
            .as_str()
            .unwrap()
            .to_string();
        wait_for_count(&started, 1).await;
        assert_eq!(started.load(AtomicOrdering::SeqCst), 1);
        session.lock().await.add_message(messages[0].clone());
        barrier.wait().await;
        let terminal = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            app.runtime.tool_jobs.wait_terminal(chat, &id),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(terminal.status, crate::tool_jobs::JobStatus::Succeeded);
        assert_eq!(terminal.receipt.binding.chat_id, chat);
        assert_eq!(terminal.receipt.binding.tool_call_id, "custom-call");
        assert!(app.runtime.tool_jobs.get("another-chat", &id).is_none());
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let locked = session.lock().await;
                if locked
                    .messages
                    .iter()
                    .any(|message| message.content.content_text_only() == "released")
                {
                    assert_eq!(
                        locked
                            .messages
                            .iter()
                            .filter(|message| message.content.content_text_only() == "released")
                            .count(),
                        1
                    );
                    let landed = locked
                        .messages
                        .iter()
                        .find(|message| message.content.content_text_only() == "released")
                        .unwrap();
                    assert_eq!(landed.role, "event");
                    assert!(landed.tool_call_id.is_empty());
                    assert_eq!(landed.extra["tool_job_result"]["job_id"], id);
                    assert_eq!(
                        landed.extra["tool_job_result"]["tool_call_id"],
                        "custom-call"
                    );
                    assert_eq!(landed.extra["tool_job_result"]["original_role"], "tool");
                    assert_eq!(landed.extra["tool_job_result"]["actor"], "tool_completion");
                    assert_eq!(landed.extra["event"]["source"], "tool_job.completed");
                    break;
                }
                drop(locked);
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("completion bridge automatically lands without a control tool");
        let current = app
            .tool_registry
            .acquire_tool_catalog(mode, Some("model"), None)
            .await;
        assert!(!current
            .index
            .tools
            .iter()
            .any(|tool| tool.name == "tool_job"));
        assert_eq!(started.load(AtomicOrdering::SeqCst), 1);
        app.runtime.tool_jobs.shutdown().await;
    }

    #[tokio::test]
    async fn native_owned_adapter_revocation_while_paused_is_unknown_without_tool_poll() {
        for revoke in [false, true] {
            let started = Arc::new(AtomicUsize::new(0));
            let fixture = BarrierToolRegistry {
                started: started.clone(),
                barrier: Arc::new(tokio::sync::Barrier::new(2)),
            };
            let base =
                AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
            let app = AppState {
                tool_registry: Arc::new(
                    crate::app_state::AppToolRegistry::with_fixture_tool_factory(
                        base.gcx.clone(),
                        Arc::new(move || fixture.tools()),
                    ),
                ),
                ..base
            };
            app.runtime.tool_jobs.set_test_inline_budget(None);
            let child = format!("subchat-supervised-{}", Uuid::new_v4());
            let (fence, _owner) =
                crate::buddy::supervision_authority::test_native_job_owner(&child);
            let thread = ThreadParams {
                id: child.clone(),
                model: "model".into(),
                ..Default::default()
            };
            let context = build_tool_execution_context(app.clone(), 4096, &[], &thread, None)
                .await
                .unwrap();
            context.lock().await.native_job_fence = Some(fence);
            let gate = Arc::new(refact_agents_core::registry::PauseGate::default());
            gate.request();
            let abort = Arc::new(AtomicBool::new(false));
            let (messages, _) = execute_tools(
                app.clone(),
                &[deterministic_tool_call("native-paused", "parallel_barrier")],
                &[],
                &thread,
                "agent",
                None,
                ExecuteToolsOptions {
                    execution_context: Some(context),
                    native_pause: Some(gate.clone()),
                    native_abort: Some(abort.clone()),
                    ..Default::default()
                },
            )
            .await;
            let id = messages[0].extra["tool_job"]["job_id"]
                .as_str()
                .expect("opaque guard activates owned admission");
            gate.quiesced().await;
            if revoke {
                crate::buddy::supervision_authority::invalidate_chat(&child).unwrap();
            } else {
                abort.store(true, Ordering::Release);
                gate.wake_for_abort();
            }
            let snapshot = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                app.runtime.tool_jobs.wait_terminal(&child, id),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(snapshot.status, crate::tool_jobs::JobStatus::Unknown);
            gate.release();
            assert_eq!(
                started.load(AtomicOrdering::SeqCst),
                0,
                "revoked parked tool must never be polled"
            );
            app.runtime.tool_jobs.shutdown().await;
        }
    }

    #[tokio::test]
    async fn serial_job_receipt_does_not_release_serial_execution_barrier() {
        let (base, thread, control, first_started, release_first) =
            serial_pause_execute_fixture().await;
        let fixture = SerialPauseToolRegistry {
            control: control.clone(),
        };
        let app = AppState {
            tool_registry: Arc::new(
                crate::app_state::AppToolRegistry::with_fixture_tool_factory(
                    base.gcx.clone(),
                    Arc::new(move || {
                        let mut tools = fixture.tools();
                        tools.push(Box::new(crate::tools::tool_tool_job::ToolToolJob {
                            config_path: String::new(),
                        }));
                        tools
                    }),
                ),
            ),
            ..base
        };
        app.runtime.tool_jobs.set_test_inline_budget(None);
        let calls = [
            deterministic_tool_call("fifo-first", "serial_pause"),
            deterministic_tool_call("fifo-second", "serial_pause"),
        ];
        let (messages, _) = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            execute_tools(
                app.clone(),
                &calls,
                &[],
                &thread,
                "agent",
                None,
                ExecuteToolsOptions::default(),
            ),
        )
        .await
        .unwrap();
        first_started.await.unwrap();
        assert_eq!(messages.len(), 2);
        assert!(messages
            .iter()
            .all(|message| message.extra.contains_key("tool_job")));
        assert_eq!(
            control.invocations.load(AtomicOrdering::SeqCst),
            1,
            "receipt must not release serial gate"
        );
        release_first.send(()).unwrap();
        for message in &messages {
            let id = message.extra["tool_job"]["job_id"].as_str().unwrap();
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                app.runtime.tool_jobs.wait_terminal(&thread.id, id),
            )
            .await
            .unwrap()
            .unwrap();
        }
        assert_eq!(control.invocations.load(AtomicOrdering::SeqCst), 2);
        app.runtime.tool_jobs.shutdown().await;
    }

    #[tokio::test]
    async fn companion_tool_calls_never_reach_execution_catalog() {
        let base_app =
            AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let catalog_reads = Arc::new(AtomicUsize::new(0));
        let reads = catalog_reads.clone();
        let app = AppState {
            tool_registry: Arc::new(
                crate::app_state::AppToolRegistry::with_fixture_tool_factory(
                    base_app.gcx.clone(),
                    Arc::new(move || {
                        reads.fetch_add(1, AtomicOrdering::SeqCst);
                        DeterministicToolRegistry {
                            confirmation_result: MatchConfirmDenyResult::PASS,
                            runtime_failure: false,
                        }
                        .tools()
                    }),
                ),
            ),
            ..base_app
        };
        let mut session = ChatSession::new("companion-tool-guard".into());
        session.thread.mode = "agent".into();
        session.thread.model = "model".into();
        session.thread.buddy_meta = Some(BuddyThreadMeta {
            is_buddy_chat: true,
            buddy_chat_kind: "companion".into(),
            workflow_id: None,
            source_chat_id: Some("source-chat".into()),
            source_project: None,
            source_created_at: None,
        });
        let call = deterministic_tool_call("unexpected-call", "alpha");
        let mut assistant = ChatMessage::new("assistant".into(), String::new());
        assistant.tool_calls = Some(vec![call.clone()]);
        session.add_message(assistant);
        let session = Arc::new(AMutex::new(session));
        let outcome = process_tool_calls_once(app.clone(), session.clone(), "agent", None).await;
        assert!(matches!(outcome, ToolStepOutcome::Stop));
        assert_eq!(catalog_reads.load(AtomicOrdering::SeqCst), 0);
        let stale_thread = ThreadParams {
            id: "companion-tool-guard".into(),
            ..Default::default()
        };
        let (results, _) = execute_tools_with_session(
            app,
            session,
            &[call],
            &[],
            &stale_thread,
            "agent",
            None,
            ExecuteToolsOptions::default(),
        )
        .await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].tool_call_id, "unexpected-call");
        assert_eq!(results[0].tool_failed, Some(true));
        assert_eq!(
            results[0].content.content_text_only(),
            "Invalid active context: Buddy companions cannot execute tools"
        );
        assert_eq!(catalog_reads.load(AtomicOrdering::SeqCst), 0);
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn invalid_arguments_fail_tool_step_without_policy_denial_or_execution() {
        let mut malformed = deterministic_tool_call("malformed", "alpha");
        malformed.function.arguments = "{".into();
        let missing_command = deterministic_tool_call("missing", "shell");
        let raw = serde_json::json!({"id":"truncated-shell","type":"function","function":{"name":"shell","arguments":"{\"command\":\"cat <<EOF"}});
        let truncated =
            super::super::stream_core::normalize_tool_call_with_finish(&raw, Some("max_tokens"))
                .unwrap();
        assert!(super::super::stream_core::truncated_tool_arguments(
            &truncated
        ));
        assert_eq!(truncated.function.arguments, "{}");
        let valid_raw = serde_json::json!({"id":"valid","type":"function","function":{"name":"shell","arguments":"{\"command\":\"true\"}"}});
        let valid = super::super::stream_core::normalize_tool_call_with_finish(
            &valid_raw,
            Some("max_tokens"),
        )
        .unwrap();
        assert!(!super::super::stream_core::truncated_tool_arguments(&valid));
        assert_eq!(valid.function.arguments, "{\"command\":\"true\"}");
        let ordinary =
            super::super::stream_core::normalize_tool_call_with_finish(&raw, Some("stop")).unwrap();
        assert!(!super::super::stream_core::truncated_tool_arguments(
            &ordinary
        ));
        assert_eq!(ordinary.function.arguments, "{\"command\":\"cat <<EOF");
        let (_, sink, session) = process_deterministic_tool_turn(
            DeterministicToolRegistry {
                confirmation_result: MatchConfirmDenyResult::PASS,
                runtime_failure: false,
            },
            vec![malformed, missing_command, truncated],
        )
        .await;
        let session = session.lock().await;
        for id in ["malformed", "missing"] {
            let message = session
                .messages
                .iter()
                .find(|message| message.role == "tool" && message.tool_call_id == id)
                .unwrap();
            assert_eq!(message.tool_failed, Some(true));
            assert_eq!(message.extra["error_kind"], "invalid_arguments");
            assert!(message
                .content
                .content_text_only()
                .starts_with("Invalid arguments for "));
        }
        let truncated_result = session
            .messages
            .iter()
            .find(|message| message.role == "tool" && message.tool_call_id == "truncated-shell")
            .unwrap();
        assert_eq!(truncated_result.tool_failed, Some(true));
        assert!(truncated_result
            .content
            .content_text_only()
            .contains(super::super::stream_core::TRUNCATED_TOOL_ARGUMENTS_ERROR));
        assert!(session.runtime.pause_reasons.is_empty());
        assert!(sink
            .events()
            .iter()
            .all(|event| event.component != PerfComponent::ToolRuntime.as_str()));
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn more_than_two_hundred_fifty_six_parallel_safe_tools_start_together() {
        const TOOL_COUNT: usize = 300;
        let started = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(tokio::sync::Barrier::new(TOOL_COUNT + 1));
        let registry = BarrierToolRegistry {
            started: started.clone(),
            barrier: barrier.clone(),
        };
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let base_app = AppState::from_gcx(gcx).await;
        let fixture = registry.clone();
        let app = AppState {
            tool_registry: Arc::new(
                crate::app_state::AppToolRegistry::with_fixture_tool_factory(
                    base_app.gcx.clone(),
                    Arc::new(move || fixture.tools()),
                ),
            ),
            ..base_app
        };
        let session = Arc::new(AMutex::new(ChatSession::new(
            "unbounded-parallel-tools".to_string(),
        )));
        {
            let mut locked = session.lock().await;
            locked.thread.model = "model".to_string();
            let mut assistant = ChatMessage::new("assistant".to_string(), String::new());
            assistant.tool_calls = Some(
                (0..TOOL_COUNT)
                    .map(|index| {
                        deterministic_tool_call(&format!("parallel-{index}"), "parallel_barrier")
                    })
                    .collect(),
            );
            locked.add_message(assistant);
        }
        app.chat
            .sessions
            .write()
            .await
            .insert("unbounded-parallel-tools".to_string(), session.clone());

        let execution = tokio::spawn(process_tool_calls_once(
            app.clone(),
            session.clone(),
            "agent",
            None,
        ));
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            while started.load(AtomicOrdering::SeqCst) < TOOL_COUNT {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("all parallel-safe tools started before any completed");
        assert_eq!(started.load(AtomicOrdering::SeqCst), TOOL_COUNT);
        assert!(
            !execution.is_finished(),
            "no tool may finish before every call has started"
        );

        barrier.wait().await;
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(30), execution)
            .await
            .expect("tool execution finished")
            .expect("tool execution task");
        assert!(matches!(outcome, ToolStepOutcome::Continue));
        assert_eq!(started.load(AtomicOrdering::SeqCst), TOOL_COUNT);
        let locked = session.lock().await;
        let results: Vec<_> = locked
            .messages
            .iter()
            .filter(|message| message.role == "tool")
            .collect();
        assert_eq!(results.len(), TOOL_COUNT);
        for (index, message) in results.iter().enumerate() {
            assert_eq!(message.tool_call_id, format!("parallel-{index}"));
            assert_ne!(message.tool_failed, Some(true));
            assert!(!message.extra.contains_key("tool_job"));
            assert_eq!(message.content.content_text_only(), "released");
        }
        assert_eq!(locked.runtime.state, SessionState::Idle);
        drop(locked);
        app.runtime.tool_jobs.shutdown().await;
    }

    async fn wait_for_count(counter: &AtomicUsize, expected: usize) {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while counter.load(AtomicOrdering::SeqCst) < expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("tool invocation started");
    }

    async fn drive_until(mut condition: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !condition() {
            assert!(
                std::time::Instant::now() < deadline,
                "condition not reached"
            );
            tokio::task::yield_now().await;
        }
    }

    struct GateControl {
        invocations: AtomicUsize,
        completed: AtomicUsize,
        started: AMutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release: AMutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    }

    struct GateTool {
        control: Arc<GateControl>,
    }

    #[async_trait::async_trait]
    impl crate::tools::tools_description::Tool for GateTool {
        async fn tool_execute(
            &mut self,
            _ccx: Arc<AMutex<AtCommandsContext>>,
            tool_call_id: &String,
            _args: &std::collections::HashMap<String, serde_json::Value>,
        ) -> Result<(bool, Vec<crate::call_validation::ContextEnum>), String> {
            self.control
                .invocations
                .fetch_add(1, AtomicOrdering::SeqCst);
            if let Some(started) = self.control.started.lock().await.take() {
                let _ = started.send(());
            }
            let release = self
                .control
                .release
                .lock()
                .await
                .take()
                .ok_or("gate tool replayed")?;
            release
                .await
                .map_err(|_| "gate release dropped".to_string())?;
            self.control.completed.fetch_add(1, AtomicOrdering::SeqCst);
            let mut message = ChatMessage::new("tool".to_string(), "native result".to_string());
            message.tool_call_id = tool_call_id.clone();
            Ok((
                false,
                vec![crate::call_validation::ContextEnum::ChatMessage(message)],
            ))
        }

        fn tool_description(&self) -> refact_tool_api::ToolDesc {
            deterministic_tool_desc("gate_tool")
        }

        async fn match_against_confirm_deny(
            &self,
            _ccx: Arc<AMutex<AtCommandsContext>>,
            _args: &std::collections::HashMap<String, serde_json::Value>,
        ) -> Result<refact_tool_api::MatchConfirmDeny, String> {
            Ok(refact_tool_api::MatchConfirmDeny {
                result: MatchConfirmDenyResult::PASS,
                command: "gate_tool".to_string(),
                rule: "gate test".to_string(),
            })
        }
    }

    struct GateDispatch {
        app: AppState,
        thread: ThreadParams,
        control: Arc<GateControl>,
        events: tokio::sync::broadcast::Receiver<crate::tool_jobs::JobEvent>,
        release: tokio::sync::oneshot::Sender<()>,
        origin: tokio::time::Instant,
        handle: tokio::task::JoinHandle<(usize, bool, Vec<ChatMessage>, Vec<ContextFile>)>,
    }

    async fn start_gate_dispatch() -> GateDispatch {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release, release_rx) = tokio::sync::oneshot::channel();
        let control = Arc::new(GateControl {
            invocations: AtomicUsize::new(0),
            completed: AtomicUsize::new(0),
            started: AMutex::new(Some(started_tx)),
            release: AMutex::new(Some(release_rx)),
        });
        let factory_control = control.clone();
        let base = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let mut app = AppState {
            tool_registry: Arc::new(
                crate::app_state::AppToolRegistry::with_fixture_tool_factory(
                    base.gcx.clone(),
                    Arc::new(move || {
                        vec![Box::new(GateTool {
                            control: factory_control.clone(),
                        })
                            as Box<dyn crate::tools::tools_description::Tool + Send>]
                    }),
                ),
            ),
            ..base
        };
        app.runtime.tool_jobs = crate::tool_jobs::JobRuntime::new(8, 65536);
        let thread = ThreadParams {
            id: "gate-dispatch".into(),
            model: "model".into(),
            ..Default::default()
        };
        let ccx = build_tool_execution_context(app.clone(), 16384, &[], &thread, None)
            .await
            .unwrap();
        let catalog = app
            .tool_registry
            .acquire_tool_catalog("agent", None, None)
            .await;
        let events = app.runtime.tool_jobs.subscribe();
        tokio::time::pause();
        let origin = tokio::time::Instant::now();
        let handle = tokio::spawn(execute_single_tool(
            app.clone(),
            ccx,
            0,
            deterministic_tool_call("gate-call", "gate_tool"),
            catalog,
            None,
            Arc::new(SerialToolRegistry::new()),
            true,
            1,
            "agent",
            None,
            None,
            None,
            None,
        ));
        drive_until(|| control.invocations.load(AtomicOrdering::SeqCst) == 1).await;
        started_rx.await.expect("gate start signal");
        assert_eq!(origin.elapsed(), std::time::Duration::ZERO);
        GateDispatch {
            app,
            thread,
            control,
            events,
            release,
            origin,
            handle,
        }
    }

    #[test]
    fn shipping_inline_budget_is_one_minute() {
        assert_eq!(TOOL_JOB_INLINE_BUDGET, std::time::Duration::from_secs(60));
    }

    #[tokio::test]
    async fn shipping_dispatch_returns_inline_result_when_tool_finishes_before_sixty_seconds() {
        let mut run = start_gate_dispatch().await;
        tokio::time::advance(std::time::Duration::from_secs(59)).await;
        assert!(!run.handle.is_finished(), "pending through virtual 59s");
        assert_eq!(run.control.invocations.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(run.control.completed.load(AtomicOrdering::SeqCst), 0);
        run.release.send(()).unwrap();
        drive_until(|| run.handle.is_finished()).await;
        let (idx, corrections, messages, _) = run.handle.await.unwrap();
        assert_eq!(idx, 0);
        assert!(!corrections);
        assert!(run.origin.elapsed() < std::time::Duration::from_secs(60));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tool_call_id, "gate-call");
        assert_eq!(messages[0].tool_failed, Some(false));
        assert!(!messages[0].extra.contains_key("tool_job"));
        assert_eq!(messages[0].content.content_text_only(), "native result");
        assert_eq!(run.control.invocations.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(run.control.completed.load(AtomicOrdering::SeqCst), 1);
        assert!(matches!(
            run.events.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
        run.app.runtime.tool_jobs.shutdown().await;
    }

    #[tokio::test]
    async fn shipping_dispatch_hands_off_at_sixty_seconds_and_same_future_completes_once() {
        let mut run = start_gate_dispatch().await;
        tokio::time::advance(std::time::Duration::from_secs(59)).await;
        assert!(!run.handle.is_finished(), "pending through virtual 59s");
        tokio::time::advance(std::time::Duration::from_millis(1001)).await;
        drive_until(|| run.handle.is_finished()).await;
        let (idx, corrections, messages, _) = run.handle.await.unwrap();
        assert_eq!(idx, 0);
        assert!(!corrections);
        assert!(run.origin.elapsed() >= std::time::Duration::from_secs(60));
        assert!(run.origin.elapsed() <= std::time::Duration::from_millis(60001));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tool_call_id, "gate-call");
        assert_eq!(messages[0].tool_failed, None);
        assert_eq!(messages[0].extra["tool_job"]["status"], "accepted");
        assert_eq!(run.control.invocations.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(run.control.completed.load(AtomicOrdering::SeqCst), 0);
        let id = messages[0].extra["tool_job"]["job_id"]
            .as_str()
            .unwrap()
            .to_string();
        let jobs = run.app.runtime.tool_jobs.clone();
        let chat = run.thread.id.clone();
        assert_eq!(
            jobs.get(&chat, &id).unwrap().status,
            crate::tool_jobs::JobStatus::Running
        );

        run.release.send(()).unwrap();
        drive_until(|| {
            jobs.get(&chat, &id)
                .is_some_and(|snapshot| snapshot.status == crate::tool_jobs::JobStatus::Succeeded)
        })
        .await;
        let terminal = jobs.get(&chat, &id).unwrap();
        assert_eq!(
            terminal.result.unwrap().messages[0]
                .content
                .content_text_only(),
            "native result"
        );
        assert_eq!(run.control.invocations.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(run.control.completed.load(AtomicOrdering::SeqCst), 1);
        let mut terminal_events = 0;
        while let Ok(event) = run.events.try_recv() {
            assert_eq!(event.job_id, id);
            if event.status == crate::tool_jobs::JobStatus::Succeeded {
                terminal_events += 1;
            }
        }
        assert_eq!(terminal_events, 1, "exactly one terminal notification");
        tokio::time::advance(std::time::Duration::from_secs(120)).await;
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        assert_eq!(run.control.invocations.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(run.control.completed.load(AtomicOrdering::SeqCst), 1);
        assert!(matches!(
            run.events.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
        jobs.shutdown().await;
    }

    #[tokio::test]
    async fn execute_tools_native_pause_parks_between_serial_tools_and_resume_retains_results() {
        let (app, thread, control, first_started, release_first) =
            serial_pause_execute_fixture().await;
        let gate = Arc::new(refact_agents_core::registry::PauseGate::default());
        let abort = Arc::new(AtomicBool::new(false));
        let calls = vec![
            deterministic_tool_call("serial-first", "serial_pause"),
            deterministic_tool_call("serial-second", "serial_pause"),
        ];
        let execution = tokio::spawn({
            let gate = gate.clone();
            async move {
                execute_tools(
                    app,
                    &calls,
                    &[],
                    &thread,
                    "agent",
                    None,
                    ExecuteToolsOptions {
                        native_pause: Some(gate),
                        native_abort: Some(abort),
                        ..Default::default()
                    },
                )
                .await
            }
        });

        first_started.await.expect("first serial tool started");
        gate.request();
        release_first.send(()).expect("release first serial tool");
        gate.quiesced().await;
        assert_eq!(control.invocations.load(AtomicOrdering::SeqCst), 1);
        assert!(
            !execution.is_finished(),
            "second serial tool remains parked"
        );

        gate.release();
        let (results, had_corrections) = execution.await.expect("tool execution task");
        assert!(!had_corrections);
        assert_eq!(control.invocations.load(AtomicOrdering::SeqCst), 2);
        assert_eq!(
            results
                .iter()
                .map(|message| message.tool_call_id.as_str())
                .collect::<Vec<_>>(),
            vec!["serial-first", "serial-second"],
        );
        assert!(results
            .iter()
            .all(|message| message.tool_failed == Some(false)));
    }

    #[tokio::test]
    async fn execute_tools_native_cancel_while_parked_skips_second_serial_tool_without_success() {
        let (app, thread, control, first_started, release_first) =
            serial_pause_execute_fixture().await;
        let gate = Arc::new(refact_agents_core::registry::PauseGate::default());
        let abort = Arc::new(AtomicBool::new(false));
        let calls = vec![
            deterministic_tool_call("serial-first", "serial_pause"),
            deterministic_tool_call("serial-second", "serial_pause"),
        ];
        let execution = tokio::spawn({
            let gate = gate.clone();
            let native_abort = abort.clone();
            async move {
                execute_tools(
                    app,
                    &calls,
                    &[],
                    &thread,
                    "agent",
                    None,
                    ExecuteToolsOptions {
                        native_pause: Some(gate),
                        native_abort: Some(native_abort),
                        ..Default::default()
                    },
                )
                .await
            }
        });

        first_started.await.expect("first serial tool started");
        gate.request();
        release_first.send(()).expect("release first serial tool");
        gate.quiesced().await;
        assert_eq!(control.invocations.load(AtomicOrdering::SeqCst), 1);
        assert!(
            !execution.is_finished(),
            "second serial tool remains parked"
        );

        abort.store(true, Ordering::SeqCst);
        gate.wake_for_abort();
        let (results, had_corrections) = execution.await.expect("tool execution task");
        assert!(!had_corrections);
        assert_eq!(control.invocations.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(
            results
                .iter()
                .map(|message| message.tool_call_id.as_str())
                .collect::<Vec<_>>(),
            vec!["serial-first"],
        );
        assert!(results
            .iter()
            .all(|message| message.tool_failed == Some(false)));
        assert!(results
            .iter()
            .all(|message| message.tool_call_id != "serial-second"));
    }

    #[tokio::test]
    async fn catalog_aliases_remain_stable_when_tool_descriptors_change_mid_batch() {
        let initial = ToolCatalogSnapshot {
            intent: refact_runtime_api::ToolCatalogIntent::Policy,
            index: refact_runtime_api::ToolRegistryIndex {
                tools: vec![deterministic_tool_desc(
                    "very_long_tool_name_that_exceeds_the_64_character_limit_for_provider_apis",
                )],
                mcp_lazy_mode: false,
                mcp_total_count: 0,
                mcp_tool_index: Vec::new(),
            },
            policy: Vec::new(),
            aliases: refact_tool_api::build_registry_from_names(&vec![
                "very_long_tool_name_that_exceeds_the_64_character_limit_for_provider_apis"
                    .to_string(),
            ]),
        };
        let alias = initial
            .aliases
            .get_alias(&initial.index.tools[0].name)
            .unwrap();
        let call = deterministic_tool_call("call-1", alias);

        let resolved = resolve_tool_call_aliases_with_catalog(vec![call], &initial);

        assert_eq!(resolved[0].function.name, initial.index.tools[0].name);
    }

    #[test]
    fn runner_wait_recognition_requires_builtin_identity() {
        let mut catalog = ToolCatalogSnapshot {
            intent: refact_runtime_api::ToolCatalogIntent::Policy,
            index: refact_runtime_api::ToolRegistryIndex {
                tools: vec![
                    deterministic_tool_desc("wait"),
                    deterministic_tool_desc("shell"),
                ],
                mcp_lazy_mode: false,
                mcp_total_count: 0,
                mcp_tool_index: Vec::new(),
            },
            policy: Vec::new(),
            aliases: refact_tool_api::build_registry_from_names(&vec![
                "wait".into(),
                "shell".into(),
            ]),
        };
        let call = deterministic_tool_call("wait", "wait");
        assert!(recognized_runner_wait(&catalog, &call));
        assert!(!recognized_runner_wait(
            &catalog,
            &deterministic_tool_call("shell", "shell")
        ));
        catalog.index.tools[0].source.source_type = refact_tool_api::ToolSourceType::Integration;
        assert!(!recognized_runner_wait(&catalog, &call));
    }

    #[test]
    fn runner_capacity_refusal_keeps_tool_call_pairing() {
        let call = deterministic_tool_call("paired", "wait");
        let (index, _, messages, files) = runner_capacity_error(2, &call, "cancelled");
        assert_eq!(index, 2);
        assert!(files.is_empty());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tool_call_id, "paired");
        assert_eq!(messages[0].tool_failed, Some(true));
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn sleep_tool_call_records_eight_second_duration() {
        let (outcome, _, session) = process_deterministic_tool_turn(
            DeterministicToolRegistry {
                confirmation_result: MatchConfirmDenyResult::PASS,
                runtime_failure: false,
            },
            vec![ChatToolCall {
                id: "tool-sleep".to_string(),
                index: None,
                function: crate::call_validation::ChatToolFunction {
                    name: "wait".to_string(),
                    arguments: r#"{"target":{"kind":"duration","duration_ms":8000},"description":"Wait eight seconds"}"#
                        .to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
                started_at_ms: None,
                completed_at_ms: None,
            }],
        )
        .await;

        assert!(matches!(outcome, ToolStepOutcome::Continue));
        let session = session.lock().await;
        let tool_call = session.messages[0]
            .tool_calls
            .as_ref()
            .unwrap()
            .first()
            .unwrap();
        let duration_ms = tool_call
            .completed_at_ms
            .unwrap()
            .saturating_sub(tool_call.started_at_ms.unwrap());
        assert!(
            (7_500..=9_000).contains(&duration_ms),
            "duration: {duration_ms}ms"
        );
    }

    fn sample_worktree() -> (tempfile::TempDir, crate::worktrees::types::WorktreeMeta) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("worktree");
        let source = temp.path().join("source");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&source).unwrap();
        let root = dunce::simplified(&fs::canonicalize(root).unwrap()).to_path_buf();
        let source = dunce::simplified(&fs::canonicalize(source).unwrap()).to_path_buf();
        (
            temp,
            crate::worktrees::types::WorktreeMeta {
                id: "wt-tools".to_string(),
                kind: "chat".to_string(),
                root,
                source_workspace_root: source.clone(),
                repo_root: source,
                branch: Some("feature".to_string()),
                base_branch: Some("main".to_string()),
                base_commit: Some("base".to_string()),
                task_id: Some("task-1".to_string()),
                card_id: Some("card-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                enforce: true,
            },
        )
    }

    #[tokio::test]
    async fn subchat_worktree_tool_execution_context_inherits_thread_scope() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (_temp, worktree) = sample_worktree();
        let task_meta = TaskMeta {
            task_id: "task-1".to_string(),
            role: "agents".to_string(),
            agent_id: Some("agent-1".to_string()),
            card_id: Some("card-1".to_string()),
            planner_chat_id: Some("planner-task-1-1".to_string()),
        };
        let thread = ThreadParams {
            id: "chat-tools".to_string(),
            model: "model".to_string(),
            task_meta: Some(task_meta.clone()),
            worktree: Some(worktree.clone()),
            ..Default::default()
        };
        let ccx =
            build_tool_execution_context(AppState::from_gcx(gcx).await, 4096, &[], &thread, None)
                .await
                .unwrap();
        let ccx = ccx.lock().await;

        assert_eq!(ccx.task_meta, Some(task_meta));
        assert_eq!(ccx.execution_scope_root(), Some(worktree.root.clone()));
        assert_eq!(ccx.execution_scope_worktree(), Some(worktree));
    }

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
    fn tool_execution_messages_prefers_prepared_prompt_history() {
        let raw = vec![ChatMessage::new("user".to_string(), "raw".to_string())];
        let prepared = vec![ChatMessage::new("user".to_string(), "prepared".to_string())];

        let selected_prepared = tool_execution_messages(&raw, &prepared);
        let selected_raw = tool_execution_messages(&raw, &[]);

        assert_eq!(selected_prepared[0].content.content_text_only(), "prepared");
        assert_eq!(selected_raw[0].content.content_text_only(), "raw");
    }

    #[test]
    fn shell_privacy_pending_result_uses_confirmation_pause() {
        let tool_call = ChatToolCall {
            id: "tc-shell".to_string(),
            index: None,
            function: crate::call_validation::ChatToolFunction {
                name: "shell".to_string(),
                arguments: "{}".to_string(),
            },
            tool_type: "function".to_string(),
            extra_content: None,
            started_at_ms: None,
            completed_at_ms: None,
        };
        let mut result = ChatMessage::new("tool".to_string(), "pending".to_string());
        result.tool_call_id = tool_call.id.clone();
        result.extra.insert(
            "privacy_shell".to_string(),
            serde_json::json!({"ask_pending": true}),
        );

        let reasons = shell_privacy_approval_reasons(&[tool_call], &[result]);

        assert_eq!(reasons.len(), 1);
        assert!(is_shell_privacy_approval(&reasons[0]));
        assert_eq!(reasons[0].tool_call_id, "tc-shell");
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn test_resolve_tool_call_aliases_handles_mixed_cc_batch_per_call() {
        let _lock = perf_diagnostics::PERF_RECORDER_TEST_LOCK.lock().unwrap();
        let (_guard, sink) = install_perf_recorder();

        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let calls = vec![
            ChatToolCall {
                id: "call-cat".to_string(),
                index: None,
                function: crate::call_validation::ChatToolFunction {
                    name: "t_cat".to_string(),
                    arguments: r#"{"paths":"src/lib.rs"}"#.to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
                started_at_ms: None,
                completed_at_ms: None,
            },
            ChatToolCall {
                id: "call-tree".to_string(),
                index: None,
                function: crate::call_validation::ChatToolFunction {
                    name: "tree".to_string(),
                    arguments: "{}".to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
                started_at_ms: None,
                completed_at_ms: None,
            },
            ChatToolCall {
                id: "call-glob".to_string(),
                index: None,
                function: crate::call_validation::ChatToolFunction {
                    name: "Glob".to_string(),
                    arguments: r#"{"pattern":"src/**/*.rs"}"#.to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
                started_at_ms: None,
                completed_at_ms: None,
            },
        ];

        let resolved = resolve_tool_call_aliases(app, calls, "agent", None).await;
        let names = resolved
            .into_iter()
            .map(|call| call.function.name)
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["cat", "tree", "glob"]);
        let components: Vec<_> = sink.events().iter().map(|event| event.component).collect();
        assert!(components.contains(&PerfComponent::ToolCatalogBuild.as_str()));
        assert!(components.contains(&PerfComponent::ToolAliasResolution.as_str()));
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
    fn test_tool_result_truncation_exempt_tools() {
        assert!(is_tool_result_truncation_exempt("subagent"));
        assert!(is_tool_result_truncation_exempt("wait"));
        assert!(is_tool_result_truncation_exempt("agent_result"));
        assert!(!is_tool_result_truncation_exempt("cat"));
        assert!(!is_tool_result_truncation_exempt("shell"));
        assert!(!is_tool_result_truncation_exempt("web_search"));
        assert!(!is_tool_result_truncation_exempt(""));
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
    fn test_auto_approve_dangerous_commands_approves_all_tools() {
        let thread = ThreadParams {
            auto_approve_dangerous_commands: true,
            ..Default::default()
        };

        assert!(should_auto_approve_confirmation(&thread, "shell"));
        assert!(should_auto_approve_confirmation(&thread, "rm"));
        assert!(should_auto_approve_confirmation(&thread, "cat"));
        assert!(should_auto_approve_confirmation(&thread, "ask_questions"));
    }

    #[test]
    fn test_auto_approve_editing_tools_stays_limited_to_editing_tools() {
        let thread = ThreadParams {
            auto_approve_editing_tools: true,
            ..Default::default()
        };

        assert!(should_auto_approve_confirmation(&thread, "apply_patch"));
        assert!(should_auto_approve_confirmation(&thread, "mv"));
        assert!(!should_auto_approve_confirmation(&thread, "cat"));
        assert!(!should_auto_approve_confirmation(&thread, "shell"));
    }

    #[test]
    fn test_autonomous_no_confirm_bypasses_confirmation() {
        let mut thread = ThreadParams::default();
        let (changed, _) = crate::chat::queue::apply_setparams_patch(
            &mut thread,
            &serde_json::json!({"autonomous_no_confirm": true}),
        );

        assert!(changed);
        assert!(should_auto_approve_confirmation(&thread, "shell"));
        assert!(should_auto_approve_confirmation(&thread, "cat"));
        assert!(should_auto_approve_confirmation(&thread, "apply_patch"));
    }

    #[test]
    fn child_approval_auto_approve_requires_dangerous_consent() {
        let mut thread = ThreadParams {
            autonomous_no_confirm: true,
            auto_approve_editing_tools: true,
            ..Default::default()
        };
        assert!(!should_auto_approve_confirmation(&thread, "agent_approval"));
        assert_eq!(
            compute_final_action(
                &MatchConfirmDenyResult::CONFIRMATION,
                Some("auto"),
                false,
                "agent_approval"
            ),
            "ask"
        );
        thread.auto_approve_dangerous_commands = true;
        for name in ["agent_approval", "agent_lifecycle", "pause", "resume"] {
            assert!(should_auto_approve_confirmation(&thread, name));
        }
        for mode in [None, Some("auto"), Some("ask")] {
            assert_eq!(
                compute_final_action(
                    &MatchConfirmDenyResult::CONFIRMATION,
                    mode,
                    true,
                    "agent_approval"
                ),
                "auto"
            );
        }
        assert_eq!(
            compute_final_action(
                &MatchConfirmDenyResult::CONFIRMATION,
                Some("deny"),
                true,
                "agent_approval"
            ),
            "deny"
        );
        assert_eq!(
            compute_final_action(
                &MatchConfirmDenyResult::DENY,
                Some("auto"),
                true,
                "agent_approval"
            ),
            "deny"
        );
        assert_eq!(
            compute_final_action(
                &MatchConfirmDenyResult::PASS,
                Some("auto"),
                false,
                "agent_approval"
            ),
            "auto"
        );
    }

    #[test]
    fn test_user_chat_still_requires_approval() {
        let thread = ThreadParams::default();

        assert!(!should_auto_approve_confirmation(&thread, "apply_patch"));
        assert!(!should_auto_approve_confirmation(&thread, "mv"));
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn invalid_escalation_argument_is_still_denied() {
        let _lock = perf_diagnostics::PERF_RECORDER_TEST_LOCK.lock().unwrap();

        let (_guard, sink) = install_perf_recorder();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let thread = ThreadParams {
            id: "escalation-invalid".to_string(),
            auto_approve_dangerous_commands: true,
            model: "model".to_string(),
            ..Default::default()
        };
        let mut session = ChatSession::new(thread.id.clone());
        session.thread = thread.clone();
        app.chat
            .sessions
            .write()
            .await
            .insert(thread.id.clone(), Arc::new(AMutex::new(session)));
        let tool_call = ChatToolCall {
            id: "tc-escalate-invalid".to_string(),
            index: None,
            function: crate::call_validation::ChatToolFunction {
                name: "shell".to_string(),
                arguments: serde_json::json!({
                    "command": "printf ok",
                    "description": "Print ok",
                    "escalate": {"mode": "full_access", "justification": "   "}
                })
                .to_string(),
            },
            tool_type: "function".to_string(),
            extra_content: None,
            started_at_ms: None,
            completed_at_ms: None,
        };

        let (confirmations, denials) = check_tools_confirmation(
            app.clone(),
            &[tool_call],
            &[],
            &thread,
            None,
            "agent",
            None,
            &[],
            "",
        )
        .await;

        assert!(confirmations.is_empty());
        assert_eq!(denials.len(), 1);
        assert!(denials[0].rule.contains("justification"));
        let components: Vec<_> = sink.events().iter().map(|event| event.component).collect();
        assert!(components.contains(&PerfComponent::ToolConfirmationPreflight.as_str()));
        assert!(components.contains(&PerfComponent::ToolPolicyLookup.as_str()));
    }

    #[test]
    fn test_tool_desc_default_allow_parallel() {
        let yaml = r#"
name: test_tool
description: A test tool
input_schema:
  type: object
  properties: {}
  required: []
display_name: Test Tool
source:
  source_type: builtin
  config_path: ""
"#;
        let desc: refact_tool_api::ToolDesc = serde_yaml::from_str(yaml).unwrap();
        assert!(!desc.allow_parallel);
    }

    #[test]
    fn test_allowed_tool_still_denied_if_tool_says_deny() {
        assert_eq!(
            compute_final_action(&MatchConfirmDenyResult::DENY, None, true, "shell"),
            "deny"
        );
        assert_eq!(
            compute_final_action(&MatchConfirmDenyResult::DENY, Some("auto"), true, "shell"),
            "deny"
        );
        assert_eq!(
            compute_final_action(&MatchConfirmDenyResult::DENY, Some("ask"), true, "shell"),
            "deny"
        );
    }

    #[test]
    fn test_allowed_tool_auto_approves_confirmation() {
        assert_eq!(
            compute_final_action(&MatchConfirmDenyResult::CONFIRMATION, None, true, "shell"),
            "auto"
        );
        assert_eq!(
            compute_final_action(
                &MatchConfirmDenyResult::CONFIRMATION,
                Some("ask"),
                true,
                "shell"
            ),
            "auto"
        );
    }

    #[test]
    fn test_allowed_tool_respects_mode_deny() {
        assert_eq!(
            compute_final_action(
                &MatchConfirmDenyResult::CONFIRMATION,
                Some("deny"),
                true,
                "shell"
            ),
            "deny"
        );
        assert_eq!(
            compute_final_action(&MatchConfirmDenyResult::PASS, Some("deny"), true, "shell"),
            "deny"
        );
    }

    #[test]
    fn test_empty_allowed_tools_no_change() {
        assert_eq!(
            compute_final_action(&MatchConfirmDenyResult::CONFIRMATION, None, false, "shell"),
            "ask"
        );
        assert_eq!(
            compute_final_action(&MatchConfirmDenyResult::PASS, None, false, "shell"),
            "auto"
        );
        assert_eq!(
            compute_final_action(
                &MatchConfirmDenyResult::CONFIRMATION,
                Some("ask"),
                false,
                "shell"
            ),
            "ask"
        );
        assert_eq!(
            compute_final_action(
                &MatchConfirmDenyResult::CONFIRMATION,
                Some("auto"),
                false,
                "shell"
            ),
            "auto"
        );
        assert_eq!(
            compute_final_action(
                &MatchConfirmDenyResult::CONFIRMATION,
                Some("deny"),
                false,
                "shell"
            ),
            "deny"
        );
    }

    async fn make_gcx_with_buddy() -> AppState {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let (tx, _) = tokio::sync::broadcast::channel(16);
        let mut state = refact_buddy_core::state::default_buddy_state();
        state.identity.name = "Pixel".to_string();
        let service = crate::buddy::actor::BuddyService::new(
            std::env::temp_dir().join(format!("buddy-tool-voice-test-{}", uuid::Uuid::new_v4())),
            state,
            refact_buddy_core::settings::BuddySettings::default(),
            Vec::new(),
            refact_buddy_core::runtime_queue::RuntimeQueue::new(),
            tx,
            None,
        );
        let buddy_arc = app.buddy.buddy.clone();
        *buddy_arc.lock().await = Some(service);
        app
    }
}

pub async fn process_tool_calls_once(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    mode_id: &str,
    model_id: Option<&str>,
) -> ToolStepOutcome {
    if crate::buddy::companion::is_companion(&session_arc.lock().await.thread) {
        return ToolStepOutcome::Stop;
    }
    if session_arc.lock().await.planner_pause_at_boundary() {
        return ToolStepOutcome::Stop;
    }
    let Ok(mut operation) = super::session_runtime::begin_board_operation(&app, &session_arc).await
    else {
        return ToolStepOutcome::Stop;
    };
    let session_extraction_span =
        perf_diagnostics::span(PerfComponent::ToolSessionExtraction, None, None);
    let (
        tool_calls,
        server_tool_calls,
        messages,
        thread,
        tool_message_id,
        allowed_tools,
        source_command,
    ) = {
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
                    m.message_id.clone(),
                    session.active_command.allowed_tools.clone(),
                    session.active_command.name.clone(),
                )
            }
            _ => return ToolStepOutcome::NoToolCalls,
        }
    };
    session_extraction_span.finish_tool(
        PerfOutcome::Success,
        1,
        tool_calls.len().saturating_add(server_tool_calls.len()) as u64,
        None,
    );

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
                let content = if tc.function.name.starts_with("openai_") {
                    format_openai_server_tool_result(tc)
                } else {
                    format!(
                        "[Results from '{}' are included in the assistant's response above]",
                        tc.function.name
                    )
                };
                let tool_message = ChatMessage {
                    message_id: Uuid::new_v4().to_string(),
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(content),
                    tool_call_id: tc.id.clone(),
                    tool_failed: Some(false),
                    ..Default::default()
                };
                session.add_message(tool_message);
            }
        }
    }

    if tool_calls.is_empty() {
        if !server_tool_calls.is_empty() {
            let mut session = session_arc.lock().await;
            session.set_runtime_state(SessionState::Idle, None);
            session.release_turn_only_state();
        }
        return ToolStepOutcome::NoToolCalls;
    }

    let catalog_pool_span =
        perf_diagnostics::span(PerfComponent::ToolCatalogPoolAcquire, None, None);
    let session_catalog = {
        let session = session_arc.lock().await;
        session.tool_catalog.clone()
    };
    let intent = super::generation::tool_catalog_intent_for_thread(&app, &thread).await;
    let catalog = match session_catalog.filter(|catalog| catalog.intent == intent) {
        Some(catalog) => catalog,
        None => {
            app.tool_registry
                .acquire_tool_catalog_with_intent(
                    mode_id,
                    model_id,
                    thread
                        .worktree
                        .as_ref()
                        .map(|worktree| worktree.root.to_string_lossy().into_owned())
                        .as_deref(),
                    intent,
                )
                .await
        }
    };
    let turn_tool_pool =
        acquire_session_turn_tool_pool(&app, &session_arc, &thread, mode_id, model_id, &catalog)
            .await;
    catalog_pool_span.finish_tool(PerfOutcome::Success, 1, tool_calls.len() as u64, None);
    let tool_calls = resolve_tool_call_aliases_with_catalog(tool_calls, &catalog);

    info!(
        "process_tool_calls_once: {} tool calls to process",
        tool_calls.len()
    );

    let (mut confirmations, mut denials) = check_tools_confirmation_with_pool(
        app.clone(),
        &tool_calls,
        &messages,
        &thread,
        Some(&catalog),
        mode_id,
        model_id,
        &allowed_tools,
        &source_command,
        turn_tool_pool.as_ref(),
    )
    .await;

    let mut approval_context = None;
    if !confirmations.is_empty()
        && thread
            .task_meta
            .as_ref()
            .is_some_and(|meta| matches!(meta.role.as_str(), "agent" | "agents"))
    {
        let abort = session_arc.lock().await.abort_flag.clone();
        match build_tool_execution_context(
            app.clone(),
            get_effective_n_ctx(app.clone(), &thread).await,
            &messages,
            &thread,
            Some(abort),
        )
        .await
        {
            Ok(context) => {
                drop(operation.take());
                for confirmation in confirmations.drain(..) {
                    if let Some(call) = tool_calls
                        .iter()
                        .find(|call| call.id == confirmation.tool_call_id)
                    {
                        let decision = crate::agents::approval::request(
                            &app,
                            &context,
                            call,
                            &confirmation.rule,
                        )
                        .await;
                        if !matches!(decision, Ok(true)) {
                            denials.push(PauseReason {
                                rule: decision.err().unwrap_or_else(|| {
                                    "Child command approval rejected or expired".into()
                                }),
                                reason_type: "denial".into(),
                                ..confirmation
                            });
                        }
                    }
                }
                approval_context = Some(context);
                operation =
                    match super::session_runtime::begin_board_operation(&app, &session_arc).await {
                        Ok(operation) => operation,
                        Err(_) => return ToolStepOutcome::Stop,
                    };
            }
            // The approval round-trip needs a real execution context. When it
            // cannot be built we fail closed: every pending confirmation turns
            // into a denial instead of being silently approved or dropped.
            Err(error) => {
                for confirmation in confirmations.drain(..) {
                    denials.push(PauseReason {
                        rule: format!("Invalid active context: {error}"),
                        reason_type: "denial".into(),
                        ..confirmation
                    });
                }
            }
        }
    }

    let denied_ids: std::collections::HashSet<String> =
        denials.iter().map(|d| d.tool_call_id.clone()).collect();
    if !denials.is_empty() {
        let mut session = session_arc.lock().await;
        for denial in &denials {
            let tool_message = ChatMessage {
                message_id: Uuid::new_v4().to_string(),
                role: "tool".to_string(),
                content: ChatContent::SimpleText(match denial.reason_type.as_str() {
                    "invalid_arguments" => format!(
                        "Invalid arguments for {}: {}; tool was not run",
                        denial.tool_name, denial.rule
                    ),
                    "preflight_unavailable" => {
                        format!("Preflight check failed: {}; tool was not run", denial.rule)
                    }
                    _ => format!("Denied by policy: {}", denial.rule),
                }),
                extra: serde_json::Map::from_iter([(
                    "error_kind".into(),
                    serde_json::json!(match denial.reason_type.as_str() {
                        "invalid_arguments" => "invalid_arguments",
                        "preflight_unavailable" => "preflight_unavailable",
                        _ => "policy_denied",
                    }),
                )]),
                tool_call_id: denial.tool_call_id.clone(),
                tool_failed: Some(true),
                ..Default::default()
            };
            session.add_message(tool_message);
        }
    }

    let mut approved_activity_ids: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    if !confirmations.is_empty() {
        let mut current_thread = thread.clone();
        current_thread.auto_approve_dangerous_commands =
            crate::agents::approval::dangerous_confirmation_enabled(&app, &thread.id).await;
        let (auto_approved, remaining): (Vec<_>, Vec<_>) =
            confirmations.into_iter().partition(|confirmation| {
                !requires_actual_human_approval(&confirmation.rule)
                    && should_auto_approve_confirmation(&current_thread, &confirmation.tool_name)
            });

        for confirmation in &auto_approved {
            approved_activity_ids.insert(confirmation.tool_call_id.clone());
        }

        if !remaining.is_empty() {
            let mut auto_approved_ids: Vec<String> = auto_approved
                .iter()
                .map(|c| c.tool_call_id.clone())
                .collect();
            let paused_ids: std::collections::HashSet<&str> =
                remaining.iter().map(|r| r.tool_call_id.as_str()).collect();
            for tc in &tool_calls {
                if !denied_ids.contains(&tc.id)
                    && !paused_ids.contains(tc.id.as_str())
                    && !auto_approved_ids.contains(&tc.id)
                {
                    auto_approved_ids.push(tc.id.clone());
                }
            }

            let approved_set: std::collections::HashSet<String> =
                auto_approved_ids.iter().cloned().collect();
            record_tool_activity(
                app.clone(),
                &tool_calls,
                &thread.id,
                &approved_set,
                &denied_ids,
            )
            .await;

            let mut session = session_arc.lock().await;
            let Some(tool_message_index) = session
                .messages
                .iter()
                .position(|message| message.message_id == tool_message_id)
            else {
                return ToolStepOutcome::Stop;
            };
            session.set_paused_with_reasons_and_auto_approved(
                remaining,
                auto_approved_ids,
                Some(tool_message_index),
            );
            return ToolStepOutcome::Paused;
        }
    }

    for tc in &tool_calls {
        if !denied_ids.contains(&tc.id) {
            approved_activity_ids.insert(tc.id.clone());
        }
    }
    record_tool_activity(
        app.clone(),
        &tool_calls,
        &thread.id,
        &approved_activity_ids,
        &denied_ids,
    )
    .await;

    let mut tools_to_execute: Vec<_> = tool_calls
        .iter()
        .filter(|tc| !denied_ids.contains(&tc.id))
        .cloned()
        .collect();

    if tools_to_execute.is_empty() {
        let mut session = session_arc.lock().await;
        session.release_turn_only_state();
        session.set_runtime_state(SessionState::Idle, None);
        drop(session);
        maybe_save_trajectory_background_with_intent(
            app.clone(),
            session_arc.clone(),
            TrajectoryCommitIntent::Checkpoint,
        );
        return ToolStepOutcome::Continue;
    }

    let (session_id, project_dir) = {
        let session = session_arc.lock().await;
        let session_id = session.chat_id.clone();
        drop(session);
        (session_id, get_project_dir_string(app.clone()).await)
    };
    let mut pre_hook_blocked_ids: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut pre_hook_rewrites: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let hook_authority = (!allowed_tools.is_empty()).then(|| {
        crate::subchat::ToolsPolicy::Only(
            allowed_tools
                .iter()
                .map(|name| crate::agents::tool_authority::canonical(name))
                .collect(),
        )
    });
    let pre_hook_item_count = tools_to_execute.len() as u64;
    let pre_hook_span = perf_diagnostics::span(PerfComponent::ToolPreHook, None, None);
    for tc in &tools_to_execute {
        let args_value: Option<serde_json::Value> = tc
            .function
            .parse_args()
            .ok()
            .map(|m| serde_json::Value::Object(m.into_iter().collect()));
        let payload = HookPayload::new(
            &HookEvent::PreToolUse,
            session_id.clone(),
            project_dir.clone(),
        )
        .with_tool(&tc.function.name, args_value);
        let verdict = run_control_hooks(app.clone(), HookEvent::PreToolUse, payload).await;
        let denial = match verdict {
            ControlVerdict::Allow => None,
            ControlVerdict::Deny(reason) => Some(reason),
            ControlVerdict::Rewrite(value) => match revalidate_rewritten_tool_call(
                &tc.function.name,
                &value,
                hook_authority.as_ref(),
            ) {
                Ok(args) => match serde_json::to_string(&serde_json::Value::Object(args)) {
                    Ok(encoded) => {
                        if encoded != tc.function.arguments {
                            pre_hook_rewrites.insert(tc.id.clone(), encoded);
                        }
                        None
                    }
                    Err(e) => Some(format!("hook rewrite could not be encoded: {e}")),
                },
                Err(e) => Some(e.to_string()),
            },
        };
        if let Some(reason) = denial {
            pre_hook_blocked_ids.insert(tc.id.clone());
            let block_message = ChatMessage {
                message_id: Uuid::new_v4().to_string(),
                role: "tool".to_string(),
                content: ChatContent::SimpleText(format!("Blocked by hook: {}", reason)),
                tool_call_id: tc.id.clone(),
                tool_failed: Some(true),
                ..Default::default()
            };
            let mut session = session_arc.lock().await;
            session.add_message(block_message);
        }
    }
    pre_hook_span.finish_tool(
        if pre_hook_blocked_ids.is_empty() {
            PerfOutcome::Success
        } else {
            PerfOutcome::Failure
        },
        pre_hook_item_count,
        pre_hook_item_count,
        Some(ToolExecutionClass::Serial),
    );
    if !pre_hook_blocked_ids.is_empty() {
        tools_to_execute.retain(|tc| !pre_hook_blocked_ids.contains(&tc.id));
    }
    let rewritten_ids: std::collections::HashSet<_> = pre_hook_rewrites.keys().cloned().collect();
    if !pre_hook_rewrites.is_empty() {
        for tc in tools_to_execute.iter_mut() {
            if let Some(arguments) = pre_hook_rewrites.remove(&tc.id) {
                tc.function.arguments = arguments;
            }
        }
    }

    if tools_to_execute.is_empty() {
        let mut session = session_arc.lock().await;
        session.release_turn_only_state();
        session.set_runtime_state(SessionState::Idle, None);
        drop(session);
        maybe_save_trajectory_background_with_intent(
            app.clone(),
            session_arc.clone(),
            TrajectoryCommitIntent::Checkpoint,
        );
        return ToolStepOutcome::Continue;
    }

    if !rewritten_ids.is_empty() {
        // Approval is bound to FINAL arguments, not to the call id alone. A hook
        // rewrite invalidates earlier command approval and must pass policy again.
        let rewritten: Vec<_> = tools_to_execute
            .iter()
            .filter(|call| rewritten_ids.contains(&call.id))
            .cloned()
            .collect();
        if let Some(context) = approval_context.as_ref() {
            let mut context = context.lock().await;
            for id in &rewritten_ids {
                context.approved_executions.remove(id);
            }
        }
        {
            let mut session = session_arc.lock().await;
            if let Some(message) = session
                .messages
                .iter_mut()
                .find(|message| message.message_id == tool_message_id)
            {
                if let Some(calls) = message.tool_calls.as_mut() {
                    for call in calls {
                        if let Some(final_call) =
                            rewritten.iter().find(|final_call| final_call.id == call.id)
                        {
                            *call = final_call.clone();
                        }
                    }
                }
            }
        }
        let (confirmations, denials) = check_tools_confirmation_with_pool(
            app.clone(),
            &rewritten,
            &messages,
            &thread,
            Some(&catalog),
            mode_id,
            model_id,
            &allowed_tools,
            &source_command,
            turn_tool_pool.as_ref(),
        )
        .await;
        if !confirmations.is_empty() {
            let mut session = session_arc.lock().await;
            let index = session
                .messages
                .iter()
                .position(|message| message.message_id == tool_message_id);
            session.set_paused_with_reasons_and_auto_approved(confirmations, Vec::new(), index);
            return ToolStepOutcome::Paused;
        }
        let denied: std::collections::HashSet<_> = denials
            .iter()
            .map(|reason| reason.tool_call_id.clone())
            .collect();
        for denial in denials {
            let mut message = ChatMessage::new(
                "tool".into(),
                format!("Rewritten invocation denied: {}", denial.rule),
            );
            message.tool_call_id = denial.tool_call_id;
            message.tool_failed = Some(true);
            session_arc.lock().await.add_message(message);
        }
        tools_to_execute.retain(|call| !denied.contains(&call.id));
    }

    {
        let mut session = session_arc.lock().await;
        session.set_runtime_state(SessionState::ExecutingTools, None);
    }

    let tool_interrupt_flag = {
        let session = session_arc.lock().await;
        session.user_interrupt_flag.clone()
    };
    let execution_wait_span = perf_diagnostics::span(PerfComponent::ToolExecutionWait, None, None);
    drop(operation);
    let tool_execution = execute_tools_with_session(
        app.clone(),
        session_arc.clone(),
        &tools_to_execute,
        &messages,
        &thread,
        mode_id,
        model_id,
        ExecuteToolsOptions {
            allowed_tools: allowed_tools.clone(),
            catalog: Some(catalog),
            turn_tool_pool,
            execution_context: approval_context,
            ..Default::default()
        },
    );
    let (tool_results, _) = tokio::select! {
        result = tool_execution => result,
        _ = wait_for_tool_abort(session_arc.clone(), tool_interrupt_flag) => (Vec::new(), false),
    };
    if super::session_runtime::board_execution_interrupted(&*session_arc.lock().await) {
        let mut session = session_arc.lock().await;
        for message in tool_results {
            session.add_message(message);
        }
        return ToolStepOutcome::Stop;
    }
    execution_wait_span.finish_tool(PerfOutcome::Success, 1, tools_to_execute.len() as u64, None);

    let privacy_approvals = shell_privacy_approval_reasons(&tools_to_execute, &tool_results);
    if !privacy_approvals.is_empty() {
        let session_merge_span =
            perf_diagnostics::span(PerfComponent::ToolSessionMergeEvents, None, None);
        let mut session = session_arc.lock().await;
        for result_msg in tool_results {
            session.add_message(result_msg);
        }
        session.drain_post_tool_side_effects();
        session.set_paused_with_reasons_and_auto_approved(privacy_approvals, Vec::new(), None);
        session_merge_span.finish_tool(
            PerfOutcome::Success,
            1,
            tools_to_execute.len() as u64,
            None,
        );
        return ToolStepOutcome::Paused;
    }

    // Determine tool-requested final state before checking abort, since ask_questions,
    // task_done, and agent_finish set abort_flag=true as part of their normal operation to prevent
    // further LLM generation — but they still need their state transition applied.
    // Only apply stop state if the tool actually succeeded (tool_failed != Some(true)), otherwise
    // let the loop continue so the LLM can see the error and retry with correct arguments.
    let mut final_state = SessionState::Idle;
    let mut completion_trigger: Option<String> = None;
    for tool_call in &tools_to_execute {
        let failed = tool_results
            .iter()
            .any(|r| r.tool_call_id == tool_call.id && r.tool_failed == Some(true));
        if !failed {
            let tool_name =
                crate::llm::adapters::claude_code_compat::cc_normalize_internal_tool_name(
                    &tool_call.function.name,
                );
            match tool_name.as_str() {
                "ask_questions" => final_state = SessionState::WaitingUserInput,
                "wait"
                    if tool_results.iter().any(|m| {
                        m.tool_call_id == tool_call.id
                            && m.tool_failed != Some(true)
                            && serde_json::from_str::<serde_json::Value>(
                                &m.content.content_text_only(),
                            )
                            .is_ok_and(|body| body["outcome"] == "parked")
                    }) =>
                {
                    final_state = SessionState::WaitingUserInput
                }
                "task_done" | "finish" => {
                    final_state = SessionState::Completed;
                    completion_trigger = Some(tool_name.to_string());
                }
                "agent_finish" => {
                    final_state = SessionState::Completed;
                    completion_trigger = Some(tool_name.to_string());
                }
                "handoff_to_mode" => final_state = SessionState::Completed,
                _ => {}
            }
        }
    }

    let tool_initiated_stop = matches!(
        final_state,
        SessionState::Completed | SessionState::WaitingUserInput
    );

    let was_interrupted = {
        let session = session_arc.lock().await;
        session.user_interrupt_flag.load(Ordering::Relaxed)
    };

    // Check if we were aborted during tool execution (user stop or tool-initiated).
    let was_aborted = {
        let session = session_arc.lock().await;
        session.abort_flag.load(Ordering::Relaxed)
    };

    let session_merge_span =
        perf_diagnostics::span(PerfComponent::ToolSessionMergeEvents, None, None);
    let chat_id = session_arc.lock().await.chat_id.clone();
    let sealed = crate::agents::seal::is_sealed(&app, &chat_id).await;
    let mut verify_completion = false;
    {
        let mut session = session_arc.lock().await;
        if !was_interrupted || tool_initiated_stop {
            for result_msg in tool_results {
                session.add_message(result_msg);
            }
            session.drain_post_tool_side_effects();
        } else {
            session.drain_post_tool_side_effects();
        }
        if tool_initiated_stop {
            if final_state == SessionState::Completed
                && completion_trigger.is_some()
                && !sealed
                && should_verify_goal_on_done(&session)
            {
                session.set_runtime_state(SessionState::ExecutingTools, None);
                verify_completion = true;
            } else {
                session.set_runtime_state(final_state, None);
            }
        } else if was_aborted {
            // User abort during regular tools: transition to Idle so UI stops animating
            session.set_runtime_state(SessionState::Idle, None);
            session.release_turn_only_state();
        } else {
            session.release_turn_only_state();
            session.set_runtime_state(SessionState::Idle, None);
        }
    }
    session_merge_span.finish_tool(PerfOutcome::Success, 1, tools_to_execute.len() as u64, None);

    if verify_completion {
        let trigger = completion_trigger.as_deref().unwrap_or("task_done");
        match verify_goal_before_completion(app.clone(), session_arc.clone(), trigger).await {
            GoalCompletionGateOutcome::Passthrough => {
                let mut session = session_arc.lock().await;
                session.set_runtime_state(SessionState::Completed, None);
            }
            GoalCompletionGateOutcome::Finalized => {}
            GoalCompletionGateOutcome::Rearmed | GoalCompletionGateOutcome::Waiting => {
                maybe_save_trajectory_with_intent(
                    app.clone(),
                    session_arc.clone(),
                    TrajectoryCommitIntent::Required,
                )
                .await;
                return ToolStepOutcome::Stop;
            }
            GoalCompletionGateOutcome::BudgetExhausted(_) => {}
            GoalCompletionGateOutcome::VerificationUnavailable
            | GoalCompletionGateOutcome::VerificationStalled => {
                let mut session = session_arc.lock().await;
                session.set_runtime_state(SessionState::Completed, None);
            }
            GoalCompletionGateOutcome::Aborted => return ToolStepOutcome::Stop,
        }
    }

    let checkpoint_span =
        perf_diagnostics::span(PerfComponent::ToolCheckpointScheduling, None, None);
    if was_aborted || tool_initiated_stop {
        maybe_save_trajectory_with_intent(
            app.clone(),
            session_arc.clone(),
            TrajectoryCommitIntent::Required,
        )
        .await;
    } else {
        maybe_save_trajectory_background_with_intent(
            app.clone(),
            session_arc.clone(),
            TrajectoryCommitIntent::Checkpoint,
        );
    }
    checkpoint_span.finish_tool(PerfOutcome::Success, 1, 1, None);

    if was_aborted || tool_initiated_stop {
        ToolStepOutcome::Stop
    } else {
        ToolStepOutcome::Continue
    }
}

fn format_openai_server_tool_result(tc: &ChatToolCall) -> String {
    let mut out = String::new();
    out.push_str("## Server tool (OpenAI Responses)\n\n");
    out.push_str(&format!("**{}**\n\n", tc.function.name));

    // Best-effort pretty JSON of the tool call arguments (usually a whole output item).
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&tc.function.arguments) {
        if let Ok(pretty) = serde_json::to_string_pretty(&v) {
            out.push_str("### Raw item\n\n```json\n");
            out.push_str(&pretty);
            out.push_str("\n```\n");
            return out;
        }
    }

    out.push_str("### Raw item\n\n```\n");
    out.push_str(&tc.function.arguments);
    out.push_str("\n```\n");
    out
}

fn compute_final_action(
    tool_result: &MatchConfirmDenyResult,
    mode_action: Option<&str>,
    is_auto_approved: bool,
    tool_name: &str,
) -> &'static str {
    if *tool_result == MatchConfirmDenyResult::DENY {
        return "deny";
    }
    if matches!(mode_action, Some("deny")) {
        return "deny";
    }
    if matches!(
        tool_name,
        "agent_approval" | "agent_lifecycle" | "pause" | "resume"
    ) && *tool_result == MatchConfirmDenyResult::CONFIRMATION
        && !is_auto_approved
    {
        return "ask";
    }
    if tool_name == "handoff_to_mode" {
        return "auto";
    }
    match mode_action {
        Some("deny") => "deny",
        _ if is_auto_approved => "auto",
        Some("ask") => "ask",
        Some("auto") => "auto",
        _ => match tool_result {
            MatchConfirmDenyResult::CONFIRMATION => "ask",
            MatchConfirmDenyResult::PASS => "auto",
            MatchConfirmDenyResult::DENY => "deny",
        },
    }
}

pub async fn check_tools_confirmation(
    app: AppState,
    tool_calls: &[crate::call_validation::ChatToolCall],
    messages: &[ChatMessage],
    thread: &ThreadParams,
    catalog: Option<&ToolCatalogSnapshot>,
    mode_id: &str,
    model_id: Option<&str>,
    allowed_tools: &[String],
    _source_command: &str,
) -> (Vec<PauseReason>, Vec<PauseReason>) {
    check_tools_confirmation_with_pool(
        app,
        tool_calls,
        messages,
        thread,
        catalog,
        mode_id,
        model_id,
        allowed_tools,
        _source_command,
        None,
    )
    .await
}

async fn check_tools_confirmation_with_pool(
    app: AppState,
    tool_calls: &[crate::call_validation::ChatToolCall],
    messages: &[ChatMessage],
    thread: &ThreadParams,
    catalog: Option<&ToolCatalogSnapshot>,
    mode_id: &str,
    model_id: Option<&str>,
    allowed_tools: &[String],
    _source_command: &str,
    turn_tool_pool: Option<&TurnToolPool>,
) -> (Vec<PauseReason>, Vec<PauseReason>) {
    let mut effective_thread = thread.clone();
    effective_thread.auto_approve_dangerous_commands &=
        crate::agents::approval::dangerous_confirmation_enabled(&app, &thread.id).await;
    let thread = &effective_thread;
    let preflight_span =
        perf_diagnostics::span(PerfComponent::ToolConfirmationPreflight, None, None);
    let item_count = tool_calls.len() as u64;
    let mut confirmations = Vec::new();
    let mut denials = Vec::new();

    let ccx = Arc::new(AMutex::new(
        AtCommandsContext::new_from_app(
            app.clone(),
            1000,
            1,
            false,
            messages.to_vec(),
            thread.id.clone(),
            thread.root_chat_id.clone(),
            thread.model.clone(),
            thread.task_meta.clone(),
            thread.worktree.clone(),
        )
        .await,
    ));

    let mode_id = map_legacy_mode_to_id(mode_id);
    let policy_span = perf_diagnostics::span(PerfComponent::ToolPolicyLookup, None, None);
    let mode_config = get_mode_config(app.gcx.clone(), mode_id, model_id).await;
    let tool_confirm_rules = mode_config
        .as_ref()
        .map(|m| m.tool_confirm.rules.as_slice())
        .unwrap_or(&[]);
    policy_span.finish_tool(
        PerfOutcome::Success,
        1,
        tool_confirm_rules.len() as u64,
        None,
    );

    for tool_call in tool_calls {
        if super::stream_core::truncated_tool_arguments(tool_call) {
            denials.push(PauseReason {
                reason_type: "invalid_arguments".into(),
                tool_call_id: tool_call.id.clone(),
                tool_name: tool_call.function.name.clone(),
                command: String::new(),
                rule: super::stream_core::TRUNCATED_TOOL_ARGUMENTS_ERROR.into(),
                integr_config_path: None,
            });
            continue;
        }
        let child_allowed = ccx
            .lock()
            .await
            .child_tool_ceiling
            .as_ref()
            .is_none_or(|ceiling| {
                crate::agents::tool_authority::allows_dispatch(ceiling, &tool_call.function.name)
            });
        if !child_allowed {
            denials.push(PauseReason {
                reason_type: "denial".into(),
                tool_call_id: tool_call.id.clone(),
                tool_name: tool_call.function.name.clone(),
                command: String::new(),
                rule: "outside accepted child tool authority".into(),
                integr_config_path: None,
            });
            continue;
        }
        if thread.autonomous_no_confirm && !allowed_tools.is_empty() {
            let resolved = crate::llm::adapters::claude_code_compat::cc_resolve_tool_name(
                &tool_call.function.name,
            );
            let in_allowlist = allowed_tools
                .iter()
                .any(|t| t == &tool_call.function.name || t == &resolved);
            if !in_allowlist {
                denials.push(PauseReason {
                    reason_type: "denial".to_string(),
                    tool_name: tool_call.function.name.clone(),
                    command: tool_call.function.name.clone(),
                    rule: format!(
                        "Tool `{}` is not allowed in this autonomous workflow",
                        tool_call.function.name
                    ),
                    tool_call_id: tool_call.id.clone(),
                    integr_config_path: None,
                });
                continue;
            }
        }

        let args = match tool_call.function.parse_args() {
            Ok(a) => serde_json::Map::from_iter(a.into_iter()),
            Err(e) => {
                denials.push(PauseReason {
                    reason_type: "invalid_arguments".to_string(),
                    tool_name: tool_call.function.name.clone(),
                    command: tool_call.function.name.clone(),
                    rule: format!("Failed to parse arguments: {}", e),
                    tool_call_id: tool_call.id.clone(),
                    integr_config_path: None,
                });
                continue;
            }
        };

        if matches!(tool_call.function.name.as_str(), "shell" | "process_start") {
            let invalid = ["command", "description"].into_iter().find(|key| {
                args.get(*key)
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(|value| value.trim().is_empty())
            });
            if let Some(key) = invalid {
                denials.push(PauseReason {
                    reason_type: "invalid_arguments".into(),
                    tool_name: tool_call.function.name.clone(),
                    command: tool_call.function.name.clone(),
                    rule: format!("Missing or invalid '{key}'"),
                    tool_call_id: tool_call.id.clone(),
                    integr_config_path: None,
                });
                continue;
            }
            if let Err(error) = escalation_from_args(&args.clone().into_iter().collect()) {
                denials.push(PauseReason {
                    reason_type: "invalid_arguments".to_string(),
                    tool_name: tool_call.function.name.clone(),
                    command: tool_call.function.name.clone(),
                    rule: error,
                    tool_call_id: tool_call.id.clone(),
                    integr_config_path: None,
                });
                continue;
            }
        }

        let Some(tool_result) = (match catalog {
            Some(catalog) => {
                app.tool_registry
                    .check_tool_confirmation_with_catalog_and_pool(
                        &ccx,
                        catalog,
                        turn_tool_pool,
                        mode_id,
                        model_id,
                        &tool_call.function.name,
                        args,
                    )
                    .await
            }
            None => {
                app.tool_registry
                    .check_tool_confirmation(
                        &ccx,
                        mode_id,
                        model_id,
                        &tool_call.function.name,
                        args,
                    )
                    .await
            }
        }) else {
            info!(
                "Unknown tool: {}, skipping confirmation check",
                tool_call.function.name
            );
            continue;
        };
        let mode_action = match_tool_confirm_action(tool_confirm_rules, &tool_call.function.name);

        match tool_result {
            Ok(result) => {
                let integr_config_path = result.integr_config_path.clone();
                let resolved_tool_name = result.tool_name.clone();
                if result.result.result == MatchConfirmDenyResult::DENY {
                    denials.push(PauseReason {
                        reason_type: "denial".to_string(),
                        tool_name: tool_call.function.name.clone(),
                        command: result.result.command,
                        rule: result.result.rule,
                        tool_call_id: tool_call.id.clone(),
                        integr_config_path,
                    });
                    continue;
                }

                let is_auto_approved =
                    should_auto_approve_confirmation(thread, &resolved_tool_name)
                        || (!matches!(
                            resolved_tool_name.as_str(),
                            "agent_approval" | "agent_lifecycle" | "pause" | "resume"
                        ) && !allowed_tools.is_empty()
                            && (allowed_tools.contains(&tool_call.function.name)
                                || allowed_tools.contains(&resolved_tool_name)));
                let forced_automation_confirmation =
                    match crate::scheduler::automation_approval::inspect_automation_approval(
                        &ccx,
                        tool_call,
                        &tool_call.function.parse_args().unwrap_or_default(),
                    )
                    .await
                    {
                        Ok(forced) => forced,
                        Err(error) => {
                            denials.push(PauseReason {
                                reason_type: "denial".into(),
                                tool_name: tool_call.function.name.clone(),
                                command: result.result.command,
                                rule: error,
                                tool_call_id: tool_call.id.clone(),
                                integr_config_path,
                            });
                            continue;
                        }
                    };
                let mut final_action = compute_final_action(
                    &result.result.result,
                    mode_action.as_deref(),
                    is_auto_approved,
                    &resolved_tool_name,
                );
                if crate::tools::tool_app::family_for_tool(&resolved_tool_name).is_some()
                    && final_action == "ask"
                {
                    final_action = "allow";
                }
                if forced_automation_confirmation && final_action != "deny" {
                    final_action = "ask";
                }
                if final_action != "deny"
                    && !is_auto_approved
                    && result.result.result == MatchConfirmDenyResult::CONFIRMATION
                    && matches!(resolved_tool_name.as_str(), "shell" | "process_start")
                    && thread
                        .task_meta
                        .as_ref()
                        .is_some_and(|meta| matches!(meta.role.as_str(), "agent" | "agents"))
                {
                    final_action = "ask";
                }

                let rule_text = if forced_automation_confirmation {
                    crate::scheduler::automation_approval::HUMAN_AUTOMATION_APPROVAL.into()
                } else {
                    match mode_action.as_deref() {
                        Some(action) => format!("mode policy: {}", action),
                        None => result.result.rule.clone(),
                    }
                };

                match final_action {
                    "deny" => {
                        denials.push(PauseReason {
                            reason_type: "denial".to_string(),
                            tool_name: tool_call.function.name.clone(),
                            command: result.result.command,
                            rule: rule_text,
                            tool_call_id: tool_call.id.clone(),
                            integr_config_path: integr_config_path.clone(),
                        });
                    }
                    "ask" => {
                        confirmations.push(PauseReason {
                            reason_type: "confirmation".to_string(),
                            tool_name: tool_call.function.name.clone(),
                            command: result.result.command,
                            rule: rule_text,
                            tool_call_id: tool_call.id.clone(),
                            integr_config_path: integr_config_path.clone(),
                        });
                    }
                    _ => {}
                }
            }
            Err(e) => {
                denials.push(PauseReason {
                    reason_type: "preflight_unavailable".to_string(),
                    tool_name: tool_call.function.name.clone(),
                    command: tool_call.function.name.clone(),
                    rule: format!("confirmation check failed: {}", e),
                    tool_call_id: tool_call.id.clone(),
                    integr_config_path: None,
                });
            }
        }
    }

    preflight_span.finish_tool(PerfOutcome::Success, item_count, item_count, None);
    (confirmations, denials)
}

pub async fn execute_tools_with_session(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    tool_calls: &[ChatToolCall],
    messages: &[ChatMessage],
    thread: &ThreadParams,
    mode_id: &str,
    model_id: Option<&str>,
    mut options: ExecuteToolsOptions,
) -> (Vec<ChatMessage>, bool) {
    if tool_calls.is_empty() {
        return (vec![], false);
    }

    if crate::buddy::companion::is_companion(thread)
        || crate::buddy::companion::is_companion(&session_arc.lock().await.thread)
    {
        return context_error_results(tool_calls, "Buddy companions cannot execute tools".into());
    }

    let Ok(operation) = super::session_runtime::begin_board_operation(&app, &session_arc).await
    else {
        return (Vec::new(), false);
    };
    let (
        prompt_messages,
        session_abort_flag,
        session_allowed_tools,
        session_catalog,
        session_turn_tool_pool,
    ) = {
        let session = session_arc.lock().await;
        let msgs = tool_execution_messages(messages, &session.last_prompt_messages);
        (
            msgs,
            session.abort_flag.clone(),
            session.active_command.allowed_tools.clone(),
            session.tool_catalog.clone(),
            session.turn_tool_pool.clone(),
        )
    };
    if options.allowed_tools.is_empty() {
        options.allowed_tools = session_allowed_tools;
    }
    let intent = super::generation::tool_catalog_intent_for_thread(&app, thread).await;
    let catalog = match options
        .catalog
        .clone()
        .or(session_catalog)
        .filter(|catalog| catalog.intent == intent)
    {
        Some(catalog) => catalog,
        None => {
            app.tool_registry
                .acquire_tool_catalog_with_intent(
                    mode_id,
                    model_id,
                    thread
                        .worktree
                        .as_ref()
                        .map(|worktree| worktree.root.to_string_lossy().into_owned())
                        .as_deref(),
                    intent,
                )
                .await
        }
    };
    if options.turn_tool_pool.is_none() {
        options.turn_tool_pool = session_turn_tool_pool;
    }
    if options.turn_tool_pool.is_none() {
        options.turn_tool_pool =
            acquire_session_turn_tool_pool(&app, &session_arc, thread, mode_id, model_id, &catalog)
                .await;
    }

    let n_ctx = get_effective_n_ctx(app.clone(), thread).await;
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

    let ccx = match options.execution_context.take() {
        Some(context) => context,
        None => match build_tool_execution_context(
            app.clone(),
            n_ctx,
            &prompt_messages,
            thread,
            Some(session_abort_flag),
        )
        .await
        {
            Ok(context) => context,
            Err(error) => return context_error_results(tool_calls, error),
        },
    };

    {
        let mut cgcx = ccx.lock().await;
        cgcx.tokens_for_rag = (n_ctx / 2).max(4096);
        if let Some(ref params) = options.subchat_tool_parameters {
            cgcx.subchat_tool_parameters = params.clone();
        }
    }

    let subchat_bridge = spawn_subchat_bridge(ccx.clone(), session_arc.clone());

    let turn_tool_pool = options.turn_tool_pool.clone();
    drop(operation);
    let result = execute_tools_inner(
        app.clone(),
        ccx,
        tool_calls,
        mode_id,
        model_id,
        budget,
        options,
        &prompt_messages,
        catalog,
        turn_tool_pool,
    )
    .await;

    subchat_bridge.shutdown().await;

    let context_files = get_context_files_from_messages(&result.0);
    if !context_files.is_empty() {
        let mut session = session_arc.lock().await;
        session.mark_tool_progress();
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

async fn wait_for_tool_abort(session_arc: Arc<AMutex<ChatSession>>, abort_flag: Arc<AtomicBool>) {
    loop {
        if abort_flag.load(Ordering::Relaxed) {
            return;
        }

        let notify = {
            let session = session_arc.lock().await;
            session.queue_notify.clone()
        };
        notify.notified().await;
    }
}

/// Owns the runtime wait marker even if execution errors, panics or is cancelled.
struct InterruptibleWaitGuard(Option<Arc<AMutex<ChatSession>>>);

impl InterruptibleWaitGuard {
    async fn begin(session: Arc<AMutex<ChatSession>>) -> Self {
        session.lock().await.begin_interruptible_wait();
        Self(Some(session))
    }

    async fn finish(&mut self) {
        if let Some(session) = &self.0 {
            session.lock().await.end_interruptible_wait();
        }
        self.0 = None;
    }
}

impl Drop for InterruptibleWaitGuard {
    fn drop(&mut self) {
        if let Some(session) = self.0.take() {
            if let Ok(mut locked) = session.try_lock() {
                locked.end_interruptible_wait();
                return;
            }
            tokio::spawn(async move {
                session.lock().await.end_interruptible_wait();
            });
        }
    }
}

type SerialToolRegistry = std::collections::HashSet<String>;

pub(crate) fn redact_tool_message_urls(message: &mut ChatMessage) {
    fn scrub_value(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::String(text) => {
                *text = refact_core::string_utils::redact_url_credentials(text)
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(scrub_value),
            serde_json::Value::Object(items) => {
                let original = std::mem::take(items);
                for (key, mut value) in original {
                    scrub_value(&mut value);
                    let base = refact_core::string_utils::redact_url_credentials(&key);
                    let mut name = base.clone();
                    let mut n = 2;
                    while items.contains_key(&name) {
                        name = format!("{base} [redacted key {n}]");
                        n += 1;
                    }
                    items.insert(name, value);
                }
            }
            _ => {}
        }
    }
    fn scrub_text(text: &mut String) {
        if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(text) {
            let original = value.clone();
            scrub_value(&mut value);
            if value != original {
                *text = value.to_string();
            }
        } else {
            *text = refact_core::string_utils::redact_url_credentials(text);
        }
    }
    let process_event = message.role == "event"
        && message
            .extra
            .get("event")
            .and_then(|event| event.get("source"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|source| source == "exec.subscribe" || source == "exec.process_completed");
    if message.role != "tool" && !process_event {
        return;
    }
    if process_event {
        if let Some(event) = message.extra.get_mut("event") {
            if let Some(payload) = event.get_mut("payload") {
                scrub_value(payload);
            }
        }
    }
    match &mut message.content {
        ChatContent::SimpleText(text) => scrub_text(text),
        ChatContent::Multimodal(parts) => {
            for part in parts {
                if part.m_type == "text" {
                    scrub_text(&mut part.m_content);
                }
            }
        }
        ChatContent::ContextFiles(_) => {}
    }
}

#[cfg(test)]
#[test]
fn tool_url_scrubbing_preserves_json_keys_and_metadata() {
    let mut message = ChatMessage {
        role: "tool".into(),
        tool_call_id: "id".into(),
        tool_failed: Some(true),
        content: ChatContent::SimpleText(r#"{"https://key:FAKE@host/x":{"output":"https://user:FAKE@host/repo?q=1","path":"/home/test/x"},"https://host/x":"other"}"#.into()),
        ..Default::default()
    };
    redact_tool_message_urls(&mut message);
    let value: serde_json::Value =
        serde_json::from_str(&message.content.content_text_only()).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 2);
    assert!(!value.to_string().contains("FAKE"));
    assert!(value.to_string().contains("/home/test/x"));
    assert_eq!(message.tool_call_id, "id");
    assert_eq!(message.tool_failed, Some(true));
}

/// Procedural/control tools must remain in the foreground: they can park the
/// turn or change the very authority domain a job would hold. Everything else
/// goes through owned admission; an unknown authority scope is an explicit error.
fn tool_job_foreground_reason(name: &str) -> Option<&'static str> {
    match name {
        "ask_questions" | "task_done" | "handoff_to_mode" => Some("interactive_turn_control"),
        "wait" | "tool_job" => Some("interruptible_job_control"),
        "shell" | "shell_service" | "process_start" => Some("shell_privacy_confirmation"),
        "set_goal" | "update_goal" | "get_goal" | "validate_goal" | "tasks_set" => {
            Some("goal_control")
        }
        "pause" | "resume" | "agent_cancel" | "agent_lifecycle" | "agent_approval"
        | "agent_peer_grant" | "team_role_grant" => Some("authority_writer"),
        "worktree_create"
        | "worktree_merge"
        | "worktree_sync"
        | "set_review_context"
        | "add_workspace_folder"
        | "task_update"
        | "task_manage"
        | "subagent"
        | "spawn_agent"
        | "spawn_agents_batch"
        | "restart_agent"
        | "merge_agent"
        | "merge_ready_in_order"
        | "task_start"
        | "assign_agent"
        | "agent_complete"
        | "agent_fail"
        | "agent_finish"
        | "board_delete"
        | "board_move"
        | "board_claim"
        | "task_board_authority" => Some("scope_writer"),
        _ => None,
    }
}

#[cfg(test)]
async fn execute_single_tool(
    app: AppState,
    ccx: Arc<AMutex<AtCommandsContext>>,
    idx: usize,
    tool_call: ChatToolCall,
    catalog: Arc<ToolCatalogSnapshot>,
    turn_tool_pool: Option<TurnToolPool>,
    serial_registry: Arc<SerialToolRegistry>,
    allow_parallel: bool,
    batch_size: usize,
    mode_id: &str,
    model_id: Option<&str>,
    native_pause: Option<Arc<refact_agents_core::registry::PauseGate>>,
    native_abort: Option<Arc<AtomicBool>>,
    expected_execution: Option<Arc<crate::agents::execution::RunExecution>>,
) -> (usize, bool, Vec<ChatMessage>, Vec<ContextFile>) {
    execute_admitted_tool(
        app,
        ccx,
        idx,
        tool_call,
        catalog,
        turn_tool_pool,
        serial_registry,
        allow_parallel,
        batch_size,
        mode_id,
        model_id,
        native_pause,
        native_abort,
        expected_execution,
        Vec::new(),
    )
    .await
}

pub(crate) const TOOL_JOB_INLINE_BUDGET: std::time::Duration = std::time::Duration::from_secs(60);

struct ShellOperationGuards(
    std::sync::Mutex<Vec<Option<crate::agents::execution::OperationGuard>>>,
);
impl std::fmt::Debug for ShellOperationGuards {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ShellOperationGuards")
    }
}
impl refact_exec::ExecLifecycleGuard for ShellOperationGuards {
    fn settle(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}
impl Drop for ShellOperationGuards {
    fn drop(&mut self) {
        // Unknown monitor ownership is not proof that native effects stopped.
        for guard in self
            .0
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
        {
            std::mem::forget(guard);
        }
    }
}

async fn execute_admitted_tool(
    app: AppState,
    ccx: Arc<AMutex<AtCommandsContext>>,
    idx: usize,
    tool_call: ChatToolCall,
    catalog: Arc<ToolCatalogSnapshot>,
    turn_tool_pool: Option<TurnToolPool>,
    serial_registry: Arc<SerialToolRegistry>,
    allow_parallel: bool,
    batch_size: usize,
    mode_id: &str,
    model_id: Option<&str>,
    native_pause: Option<Arc<refact_agents_core::registry::PauseGate>>,
    native_abort: Option<Arc<AtomicBool>>,
    expected_execution: Option<Arc<crate::agents::execution::RunExecution>>,
    operation_guards: Vec<Option<crate::agents::execution::OperationGuard>>,
) -> (usize, bool, Vec<ChatMessage>, Vec<ContextFile>) {
    let unsupported_scope = {
        let context = ccx.lock().await;
        context.tool_access_bypass
            || (context.native_job_fence.is_none()
                && (context.task_meta.is_some()
                    || context.chat_id.starts_with("subchat-supervised-")
                    || native_pause.is_some()))
    };
    let foreground_reason = tool_job_foreground_reason(&tool_call.function.name)
        .or(unsupported_scope.then_some("authority_scope_not_async_fenced"));
    if let Some(reason) = foreground_reason {
        let guards = operation_guards;
        let shell_binding = (tool_call.function.name == "shell").then(|| {
            (
                tool_call.id.clone(),
                ccx.clone(),
                app.runtime.exec_registry.clone(),
            )
        });
        let mut result = execute_single_tool_owned(
            app,
            ccx,
            idx,
            tool_call,
            catalog,
            turn_tool_pool,
            serial_registry,
            allow_parallel,
            batch_size,
            mode_id,
            model_id,
            native_pause,
            native_abort,
            expected_execution,
        )
        .await;
        if let Some((tool_id, context, registry)) = shell_binding {
            let chat_id = context.lock().await.chat_id.clone();
            let snapshots = registry
                .list(refact_exec::ExecProcessFilter {
                    chat_id: Some(chat_id),
                    tool_call_id: Some(tool_id),
                    ..Default::default()
                })
                .await;
            if snapshots.len() == 1 {
                let guard = Arc::new(ShellOperationGuards(std::sync::Mutex::new(guards)));
                if let Some(handle) = registry.execution_handle(&snapshots[0].meta).await {
                    if let Err(guard) = registry.attach_execution_guard(&handle, guard).await {
                        std::mem::forget(guard);
                    }
                } else {
                    std::mem::forget(guard);
                }
            } else if !snapshots.is_empty() {
                std::mem::forget(guards);
            }
        }
        for message in &mut result.2 {
            message.extra.insert(
                "tool_execution_policy".into(),
                serde_json::json!({"foreground_reason":reason}),
            );
        }
        return result;
    }
    let (chat_id, model, scope_keys, turn_id) = {
        let context = ccx.lock().await;
        let mut keys = vec![app.runtime.tool_jobs.chat_scope_key(&context.chat_id)];
        if let Some(worktree) = context.execution_scope_worktree() {
            keys.push(crate::tool_jobs::ScopeKey::Worktree {
                project: worktree.source_workspace_root,
                root: worktree.root,
            });
        } else {
            // Freeze trusted configured primary workspace roots, not model path
            // arguments. Canonical identity matches the central service fence.
            let roots = context
                .global_context
                .documents_state
                .workspace_folders
                .lock()
                .unwrap()
                .clone();
            for root in roots {
                match std::fs::canonicalize(&root) {
                    Ok(root) => {
                        let root = dunce::simplified(&root).to_path_buf();
                        keys.push(crate::tool_jobs::ScopeKey::Worktree {
                            project: root.clone(),
                            root,
                        });
                    }
                    Err(_) => {
                        return runner_capacity_error(
                            idx,
                            &tool_call,
                            "primary workspace scope identity unavailable",
                        )
                    }
                }
            }
        }
        let turn_id = context
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "assistant")
            .map(|m| m.message_id.clone())
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| tool_call.id.clone());
        (
            context.chat_id.clone(),
            context.current_model.clone(),
            keys,
            turn_id,
        )
    };
    let dispatch_gate = {
        type Gates =
            std::collections::HashMap<(usize, String), std::sync::Weak<tokio::sync::RwLock<()>>>;
        static GATES: std::sync::OnceLock<std::sync::Mutex<Gates>> = std::sync::OnceLock::new();
        let mut gates = GATES
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        gates.retain(|_, gate| gate.strong_count() > 0);
        gates
            .entry((
                Arc::as_ptr(&app.runtime.tool_jobs) as usize,
                chat_id.clone(),
            ))
            .or_default()
            .upgrade()
            .unwrap_or_else(|| {
                let gate = Arc::new(tokio::sync::RwLock::new(()));
                gates.insert(
                    (
                        Arc::as_ptr(&app.runtime.tool_jobs) as usize,
                        chat_id.clone(),
                    ),
                    Arc::downgrade(&gate),
                );
                gate
            })
    };
    super::tool_job_delivery::ensure_bridge(&app);
    let binding = crate::tool_jobs::JobBinding {
        chat_id: chat_id.clone(),
        tool_call_id: tool_call.id.clone(),
        tool_name: tool_call.function.name.clone(),
        model,
        turn_id,
        scope_keys,
    };
    let mode_id = mode_id.to_owned();
    let model_id = model_id.map(str::to_owned);
    let call_for_error = tool_call.clone();
    let runtime = app.runtime.tool_jobs.clone();
    let result = runtime
        .submit(binding, TOOL_JOB_INLINE_BUDGET, move |cancel| async move {
            // submit acquires the lifecycle lease before this future is created/polled.
            let _guards = operation_guards;
            let frozen = ccx
                .lock()
                .await
                .freeze_tool_job(&tool_call.id, cancel.clone());
            let frozen = match frozen {
                Ok(context) => Arc::new(AMutex::new(context)),
                Err(error) => {
                    let (_, _, messages, context_files) =
                        runner_capacity_error(idx, &tool_call, &error);
                    return (
                        crate::tool_jobs::JobStatus::Failed,
                        crate::tool_jobs::JobResult {
                            had_corrections: false,
                            messages,
                            context_files,
                        },
                    );
                }
            };
            let (_parallel_guard, _serial_guard) = if allow_parallel {
                (Some(dispatch_gate.read_owned().await), None)
            } else {
                (None, Some(dispatch_gate.write_owned().await))
            };
            let (n_ctx, pp, history) = {
                let context = frozen.lock().await;
                (
                    context.n_ctx,
                    context.postprocess_parameters.clone(),
                    context.messages.clone(),
                )
            };
            let session = app.chat.sessions.read().await.get(&chat_id).cloned();
            let bridge = session.map(|session| spawn_subchat_bridge(frozen.clone(), session));
            let native_fence = frozen.lock().await.native_job_fence.clone();
            let owner_abort = native_abort.clone().unwrap_or_else(|| cancel.clone());
            let invocation = execute_single_tool_owned(
                app.clone(),
                frozen,
                idx,
                tool_call.clone(),
                catalog,
                turn_tool_pool,
                serial_registry,
                allow_parallel,
                batch_size,
                &mode_id,
                model_id.as_deref(),
                native_pause,
                native_abort,
                expected_execution,
            );
            let invocation_result = match native_fence {
                Some(fence) => {
                    fence
                        .run(&cancel, &owner_abort, async { Ok(invocation.await) })
                        .await
                }
                None => Ok(invocation.await),
            };
            let (_, corrections, messages, files) = match invocation_result {
                Ok(result) => result,
                Err(error) => {
                    if let Some(bridge) = bridge {
                        bridge.shutdown().await;
                    }
                    let (_, _, messages, context_files) = runner_capacity_error(
                        idx,
                        &tool_call,
                        &format!("{error}; effects unknown, never replay"),
                    );
                    return (
                        crate::tool_jobs::JobStatus::Unknown,
                        crate::tool_jobs::JobResult {
                            had_corrections: false,
                            messages,
                            context_files,
                        },
                    );
                }
            };
            if let Some(bridge) = bridge {
                bridge.shutdown().await;
            }
            let mut exempt = std::collections::HashSet::new();
            if is_tool_result_truncation_exempt(&tool_call.function.name) {
                exempt.insert(tool_call.id.clone());
            }
            let mut messages = match ToolBudget::try_from_n_ctx(n_ctx) {
                Ok(budget) => {
                    crate::file_activity::TOOL_ACTOR
                        .scope(
                            chat_id,
                            postprocess_tool_results(
                                app.gcx.clone(),
                                None,
                                messages,
                                files,
                                budget,
                                pp,
                                &history,
                                &exempt,
                            ),
                        )
                        .await
                }
                Err(error) => runner_capacity_error(idx, &tool_call, &error).2,
            };
            for message in &mut messages {
                message
                    .extra
                    .insert("tool_job_postprocessed".into(), true.into());
            }
            let status = if messages.is_empty() || cancel.load(Ordering::Acquire) {
                crate::tool_jobs::JobStatus::Unknown
            } else if messages.iter().any(|m| m.tool_failed == Some(true)) {
                crate::tool_jobs::JobStatus::Failed
            } else {
                crate::tool_jobs::JobStatus::Succeeded
            };
            (
                status,
                crate::tool_jobs::JobResult {
                    had_corrections: corrections,
                    messages,
                    context_files: Vec::new(),
                },
            )
        })
        .await;
    match result {
        Ok(crate::tool_jobs::Submission::Inline(result)) => (
            idx,
            result.had_corrections,
            result.messages,
            result.context_files,
        ),
        Ok(crate::tool_jobs::Submission::Accepted(receipt)) => (
            idx,
            false,
            vec![super::tool_job_delivery::receipt_message(&receipt)],
            Vec::new(),
        ),
        Err(error) => runner_capacity_error(idx, &call_for_error, &error),
    }
}

async fn execute_single_tool_owned(
    app: AppState,
    ccx: Arc<AMutex<AtCommandsContext>>,
    idx: usize,
    tool_call: ChatToolCall,
    catalog: Arc<ToolCatalogSnapshot>,
    turn_tool_pool: Option<TurnToolPool>,
    serial_registry: Arc<SerialToolRegistry>,
    allow_parallel: bool,
    batch_size: usize,
    mode_id: &str,
    model_id: Option<&str>,
    native_pause: Option<Arc<refact_agents_core::registry::PauseGate>>,
    native_abort: Option<Arc<AtomicBool>>,
    expected_execution: Option<Arc<crate::agents::execution::RunExecution>>,
) -> (usize, bool, Vec<ChatMessage>, Vec<ContextFile>) {
    let context_abort_flag = {
        let cgcx = ccx.lock().await;
        cgcx.abort_flag.clone()
    };
    let abort_flag = native_abort.unwrap_or(context_abort_flag);
    if abort_flag.load(Ordering::Relaxed) {
        return (idx, false, Vec::new(), Vec::new());
    }
    if let Some(gate) = native_pause {
        if gate.hold(Some(abort_flag.as_ref())).await || abort_flag.load(Ordering::Relaxed) {
            return (idx, false, Vec::new(), Vec::new());
        }
    }

    let child_allowed = ccx
        .lock()
        .await
        .child_tool_ceiling
        .as_ref()
        .is_none_or(|ceiling| {
            crate::agents::tool_authority::allows_dispatch(ceiling, &tool_call.function.name)
        });
    if !child_allowed {
        return (
            idx,
            false,
            vec![ChatMessage {
                role: "tool".into(),
                tool_call_id: tool_call.id.clone(),
                tool_failed: Some(true),
                content: ChatContent::SimpleText("Tool is outside accepted child authority".into()),
                ..Default::default()
            }],
            Vec::new(),
        );
    }

    if super::stream_core::truncated_tool_arguments(&tool_call) {
        return (
            idx,
            true,
            vec![ChatMessage {
                message_id: Uuid::new_v4().to_string(),
                role: "tool".into(),
                tool_call_id: tool_call.id.clone(),
                tool_failed: Some(true),
                content: ChatContent::SimpleText(
                    super::stream_core::TRUNCATED_TOOL_ARGUMENTS_ERROR.into(),
                ),
                ..Default::default()
            }],
            Vec::new(),
        );
    }

    let args: std::collections::HashMap<String, serde_json::Value> =
        match tool_call.function.parse_args() {
            Ok(a) => a,
            Err(e) => {
                return (
                    idx,
                    true, // had_corrections: parse error is a correction
                    vec![ChatMessage {
                        message_id: Uuid::new_v4().to_string(),
                        role: "tool".to_string(),
                        content: ChatContent::SimpleText(format!("Error parsing arguments: {}", e)),
                        tool_call_id: tool_call.id.clone(),
                        tool_failed: Some(true),
                        ..Default::default()
                    }],
                    vec![],
                );
            }
        };

    info!("Executing tool: {}({:?})", tool_call.function.name, args);

    let session_id = {
        let cgcx = ccx.lock().await;
        cgcx.chat_id.clone()
    };

    if !allow_parallel {
        let resolved_name = crate::llm::adapters::claude_code_compat::cc_resolve_tool_name(
            &tool_call.function.name,
        );
        if !serial_registry.contains(&tool_call.function.name)
            && !serial_registry.contains(resolved_name.as_str())
        {
            return (
                idx,
                true,
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
    }

    if let Some(session_arc) = {
        let sessions_read = app.chat.sessions.read().await;
        sessions_read.get(&session_id).cloned()
    } {
        let mut session = session_arc.lock().await;
        session.mark_tool_started();
        session.stamp_tool_call_timing(&tool_call.id, tool_timestamp_ms(), None);
    }

    let interruptible = catalog
        .index
        .tools
        .iter()
        .any(|desc| desc.name == tool_call.function.name && desc.is_interruptible_wait());
    let mut wait_guard = if interruptible {
        let session = app.chat.sessions.read().await.get(&session_id).cloned();
        match session {
            Some(session) => Some(InterruptibleWaitGuard::begin(session).await),
            None => None,
        }
    } else {
        None
    };
    let current = match current_runner_execution(&app, &ccx).await {
        Ok(current) => current,
        Err(_) => return (idx, false, Vec::new(), Vec::new()),
    };
    if !match (expected_execution.as_ref(), current.as_ref()) {
        (Some(expected), Some(current)) => Arc::ptr_eq(expected, current),
        (None, None) => true,
        _ => false,
    } {
        return (idx, false, Vec::new(), Vec::new());
    }
    let started = std::time::Instant::now();
    let execution_future = crate::file_activity::TOOL_ACTOR.scope(
        session_id.clone(),
        app.tool_registry.execute_tool_with_catalog_and_pool(
            &ccx,
            &catalog,
            turn_tool_pool.as_ref(),
            mode_id,
            model_id,
            &tool_call.id,
            &tool_call.function.name,
            serde_json::Map::from_iter(args.into_iter()),
        ),
    );
    let execution = if interruptible && tool_call.function.name != "wait" {
        let interrupt = crate::tools::tools_description::WaitInterrupt::from_context(&ccx).await;
        tokio::select! {
            result = execution_future => result,
            _ = interrupt.wait() => Ok(Some(refact_runtime_api::ToolExecutionResult {
                had_corrections: false,
                messages: vec![ChatMessage {
                    role: "tool".into(),
                    tool_call_id: tool_call.id.clone(),
                    tool_failed: Some(false),
                    content: ChatContent::SimpleText(serde_json::json!({
                        "interrupted": true,
                        "reason": interrupt.reason(started.elapsed().as_secs_f64()),
                    }).to_string()),
                    ..Default::default()
                }],
                context_files: vec![],
            })),
        }
    } else {
        execution_future.await
    };
    if let Some(guard) = &mut wait_guard {
        guard.finish().await;
    }

    if let Some(session_arc) = {
        let sessions_read = app.chat.sessions.read().await;
        sessions_read.get(&session_id).cloned()
    } {
        let mut session = session_arc.lock().await;
        session.stamp_tool_call_timing(&tool_call.id, None, tool_timestamp_ms());
    }

    let (idx, had_corrections, mut msgs, files) = match execution {
        Ok(Some(result)) => (
            idx,
            result.had_corrections,
            result.messages,
            result.context_files,
        ),
        Ok(None) => {
            return (
                idx,
                true,
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
        Err(e) => {
            let e = refact_core::string_utils::redact_url_credentials(&e.to_string());
            info!("Tool execution failed: {}: {}", tool_call.function.name, e);
            return (
                idx,
                true,
                vec![ChatMessage {
                    message_id: Uuid::new_v4().to_string(),
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(format!("Error: {}", e)),
                    tool_call_id: tool_call.id.clone(),
                    tool_failed: Some(true),
                    ..Default::default()
                }],
                vec![],
            );
        }
    };

    let post_hook_runner = recognized_runner_wait(&catalog, &tool_call)
        .then(|| app.agent_executions.get(&session_id))
        .flatten();
    let post_hook_permit = match post_hook_runner.as_ref() {
        Some(runner) => {
            match runner_capacity_wait(&app, &ccx, &runner, runner.begin_operation()).await {
                Ok(guard) => Some(guard),
                Err(error) => return runner_capacity_error(idx, &tool_call, &error),
            }
        }
        None => None,
    };

    let execution_class = if allow_parallel {
        ToolExecutionClass::Parallel
    } else {
        ToolExecutionClass::Serial
    };

    for msg in &mut msgs {
        redact_tool_message_urls(msg);
        if msg.message_id.is_empty() {
            msg.message_id = Uuid::new_v4().to_string();
        }
        if msg.tool_failed.is_none() {
            msg.tool_failed = Some(false);
        }
    }

    let tool_output_text = msgs
        .iter()
        .filter(|m| m.role == "tool")
        .map(|m| m.content.content_text_only())
        .collect::<Vec<_>>()
        .join("\n");

    let frozen_project = ccx
        .lock()
        .await
        .execution_scope
        .as_ref()
        .map(|scope| scope.effective_root().to_string_lossy().into_owned());
    let project_dir = match frozen_project {
        Some(project) => project,
        None => get_project_dir_string(app.clone()).await,
    };
    let args_value: Option<serde_json::Value> = tool_call
        .function
        .parse_args()
        .ok()
        .map(|m| serde_json::Value::Object(m.into_iter().collect()));
    let post_payload = HookPayload::new(&HookEvent::PostToolUse, session_id, project_dir)
        .with_tool(&tool_call.function.name, args_value)
        .with_tool_output(tool_output_text);
    let post_hook_span = perf_diagnostics::span(PerfComponent::ToolPostHook, None, None);
    let post_verdict = run_control_hooks(app.clone(), HookEvent::PostToolUse, post_payload).await;
    post_hook_span.finish_tool(
        if matches!(&post_verdict, ControlVerdict::Deny(_)) {
            PerfOutcome::Failure
        } else {
            PerfOutcome::Success
        },
        batch_size as u64,
        1,
        Some(execution_class),
    );
    drop(post_hook_permit);
    if let Some(runner) = post_hook_runner {
        if let Err(error) =
            runner_capacity_wait(&app, &ccx, &runner, runner.suspend_for_wait()).await
        {
            return runner_capacity_error(idx, &tool_call, &error);
        }
    }
    if let ControlVerdict::Deny(reason) = post_verdict {
        return (
            idx,
            true,
            vec![ChatMessage {
                message_id: Uuid::new_v4().to_string(),
                role: "tool".to_string(),
                content: ChatContent::SimpleText(format!("Blocked by hook: {}", reason)),
                tool_call_id: tool_call.id.clone(),
                tool_failed: Some(true),
                ..Default::default()
            }],
            vec![],
        );
    }

    (idx, had_corrections, msgs, files)
}

async fn runner_capacity_wait<T>(
    app: &AppState,
    ccx: &Arc<AMutex<AtCommandsContext>>,
    execution: &crate::agents::execution::RunExecution,
    future: impl std::future::Future<Output = Result<T, refact_work_core::AdmissionError>>,
) -> Result<T, String> {
    let span = perf_diagnostics::span(PerfComponent::ToolRunnerCapacityWait, None, None);
    let abort = ccx.lock().await.abort_flag.clone();
    tokio::pin!(future);
    loop {
        if abort.load(Ordering::Relaxed) || app.runtime.shutdown_flag.load(Ordering::Relaxed) {
            execution.cancel();
            span.finish_tool(PerfOutcome::Cancelled, 1, 1, None);
            return Err("Aborted while waiting for execution capacity".into());
        }
        tokio::select! {
            result = &mut future => {
                span.finish_tool(
                    if result.is_ok() { PerfOutcome::Success } else { PerfOutcome::Failure },
                    1, 1, None,
                );
                return result.map_err(|error| error.to_string());
            },
            _ = tokio::time::sleep(refact_work_core::coordination_settings().lock_poll_interval()) => {}
        }
    }
}

async fn current_runner_execution(
    app: &AppState,
    ccx: &Arc<AMutex<AtCommandsContext>>,
) -> Result<Option<Arc<crate::agents::execution::RunExecution>>, String> {
    let (chat_id, board_context, internal) = {
        let context = ccx.lock().await;
        (
            context.chat_id.clone(),
            context
                .task_meta
                .as_ref()
                .is_some_and(|meta| matches!(meta.role.as_str(), "agent" | "agents")),
            context.subchat_depth > 0 && context.background_agent_id.is_none(),
        )
    };
    let session = app.chat.sessions.read().await.get(&chat_id).cloned();
    if let Some(session) = session {
        match super::session_runtime::board_execution(app, &session).await {
            Ok(Some(execution)) => return Ok(Some(execution)),
            Ok(None) => {}
            Err(crate::tasks::admission::ExecutionUnavailable::Held) => {
                return Err("session busy; retry the tool call".into());
            }
            Err(reason) => {
                super::session_runtime::interrupt_board_execution(
                    &mut *session.lock().await,
                    reason,
                );
                return Err(reason.recovery_message());
            }
        }
    } else if board_context && !internal {
        return Err(crate::tasks::admission::ExecutionUnavailable::Missing.recovery_message());
    }
    Ok(app.agent_executions.get(&chat_id))
}

/// Tools whose whole job is to block: the runner parks its execution permit for
/// the duration instead of occupying a slot while idle.
pub(crate) fn is_recognized_runner_wait_name(name: &str) -> bool {
    matches!(name, "wait")
}

fn recognized_runner_wait(catalog: &ToolCatalogSnapshot, call: &ChatToolCall) -> bool {
    let name = if catalog
        .index
        .tools
        .iter()
        .any(|desc| desc.name == call.function.name)
    {
        call.function.name.clone()
    } else {
        crate::llm::adapters::claude_code_compat::cc_resolve_tool_name(&call.function.name)
    };
    is_recognized_runner_wait_name(&name)
        && catalog.index.tools.iter().any(|desc| {
            desc.name == name
                && matches!(
                    desc.source.source_type,
                    crate::tools::tools_description::ToolSourceType::Builtin
                )
        })
}

fn runner_capacity_error(
    idx: usize,
    call: &ChatToolCall,
    error: &str,
) -> (usize, bool, Vec<ChatMessage>, Vec<ContextFile>) {
    (
        idx,
        false,
        vec![ChatMessage {
            message_id: Uuid::new_v4().to_string(),
            role: "tool".into(),
            content: ChatContent::SimpleText(format!("Execution capacity unavailable: {error}")),
            tool_call_id: call.id.clone(),
            tool_failed: Some(true),
            ..Default::default()
        }],
        Vec::new(),
    )
}

async fn execute_tools_inner(
    app: AppState,
    ccx: Arc<AMutex<AtCommandsContext>>,
    tool_calls: &[ChatToolCall],
    mode_id: &str,
    model_id: Option<&str>,
    budget: ToolBudget,
    options: ExecuteToolsOptions,
    messages: &[ChatMessage],
    catalog: Arc<ToolCatalogSnapshot>,
    turn_tool_pool: Option<TurnToolPool>,
) -> (Vec<ChatMessage>, bool) {
    ccx.lock().await.postprocess_parameters =
        options.postprocess_settings.clone().unwrap_or_default();
    let policy_span = perf_diagnostics::span(PerfComponent::ToolPolicyLookup, None, None);
    let available_tools = &catalog.policy;
    policy_span.finish_tool(PerfOutcome::Success, 1, available_tools.len() as u64, None);

    let mut tool_allow_parallel: std::collections::HashMap<String, bool> =
        std::collections::HashMap::new();
    let mut serial_registry = SerialToolRegistry::new();

    for tool in available_tools {
        tool_allow_parallel.insert(tool.name.clone(), tool.effective_allow_parallel);

        // Parallel tools are instantiated per call (no shared mutex).
        // Sequential tools are cached and protected by a single mutex.
        if tool.effective_allow_parallel {
            continue;
        }
        serial_registry.insert(tool.name.clone());
    }

    let serial_registry = Arc::new(serial_registry);

    let mut all_results: Vec<(usize, bool, Vec<ChatMessage>, Vec<ContextFile>)> = Vec::new();
    let mut current_parallel_batch: Vec<(usize, ChatToolCall)> = Vec::new();
    let mut prepared_parallel_slots: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();

    for (idx, tool_call) in tool_calls.iter().enumerate() {
        let is_aborted = {
            let cgcx = ccx.lock().await;
            cgcx.abort_flag.load(Ordering::Relaxed)
        };
        if is_aborted {
            break;
        }

        let options_allowed_tools = &options.allowed_tools;
        if !options_allowed_tools.is_empty() {
            let resolved = crate::llm::adapters::claude_code_compat::cc_resolve_tool_name(
                &tool_call.function.name,
            );
            let in_allowlist = options_allowed_tools
                .iter()
                .any(|t| t == &tool_call.function.name || t == &resolved);
            if !in_allowlist {
                all_results.push((
                    idx,
                    false,
                    vec![ChatMessage {
                        message_id: Uuid::new_v4().to_string(),
                        role: "tool".to_string(),
                        content: ChatContent::SimpleText(format!(
                            "Tool `{}` is not allowed in this autonomous workflow. Allowed: {}",
                            tool_call.function.name,
                            options_allowed_tools.join(", ")
                        )),
                        tool_call_id: tool_call.id.clone(),
                        tool_failed: Some(true),
                        ..Default::default()
                    }],
                    vec![],
                ));
                continue;
            }
        }

        let resolved_name = crate::llm::adapters::claude_code_compat::cc_resolve_tool_name(
            &tool_call.function.name,
        );
        let allow_parallel = tool_allow_parallel
            .get(&tool_call.function.name)
            .or_else(|| tool_allow_parallel.get(resolved_name.as_str()))
            .copied()
            .unwrap_or(false);

        if allow_parallel {
            current_parallel_batch.push((idx, tool_call.clone()));
        } else {
            if !current_parallel_batch.is_empty() {
                let batch_results = execute_parallel_batch(
                    app.clone(),
                    ccx.clone(),
                    &current_parallel_batch,
                    catalog.clone(),
                    turn_tool_pool.clone(),
                    &mut prepared_parallel_slots,
                    serial_registry.clone(),
                    mode_id,
                    model_id,
                    options.native_pause.clone(),
                    options.native_abort.clone(),
                )
                .await;
                all_results.extend(batch_results);
                current_parallel_batch.clear();
            }

            let execution = match current_runner_execution(&app, &ccx).await {
                Ok(execution) => execution,
                Err(_) => break,
            };
            let is_wait = recognized_runner_wait(&catalog, tool_call);
            let operation_guards = if let Some(execution) = execution.as_ref() {
                match runner_capacity_wait(
                    &app,
                    &ccx,
                    execution,
                    execution.begin_tool_batch(&[is_wait]),
                )
                .await
                {
                    Ok(guards) => guards,
                    Err(error) => {
                        all_results.push(runner_capacity_error(idx, tool_call, &error));
                        continue;
                    }
                }
            } else {
                Vec::new()
            };
            if is_wait {
                if let Some(execution) = execution.as_ref() {
                    if let Err(error) =
                        runner_capacity_wait(&app, &ccx, execution, execution.suspend_for_wait())
                            .await
                    {
                        all_results.push(runner_capacity_error(idx, tool_call, &error));
                        continue;
                    }
                }
            }
            let result = execute_admitted_tool(
                app.clone(),
                ccx.clone(),
                idx,
                tool_call.clone(),
                catalog.clone(),
                turn_tool_pool.clone(),
                serial_registry.clone(),
                false,
                1,
                mode_id,
                model_id,
                options.native_pause.clone(),
                options.native_abort.clone(),
                execution.clone(),
                operation_guards,
            )
            .await;
            all_results.push(result);
        }
    }

    if !current_parallel_batch.is_empty() {
        let batch_results = execute_parallel_batch(
            app.clone(),
            ccx.clone(),
            &current_parallel_batch,
            catalog,
            turn_tool_pool,
            &mut prepared_parallel_slots,
            serial_registry.clone(),
            mode_id,
            model_id,
            options.native_pause.clone(),
            options.native_abort.clone(),
        )
        .await;
        all_results.extend(batch_results);
    }

    let chat_id = ccx.lock().await.chat_id.clone();
    if let Some(execution) = app.agent_executions.get(&chat_id) {
        if runner_capacity_wait(&app, &ccx, &execution, execution.resume())
            .await
            .is_err()
        {
            ccx.lock().await.abort_flag.store(true, Ordering::Relaxed);
        }
    }
    let merge_span = perf_diagnostics::span(PerfComponent::ToolResultMerge, None, None);
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
    let merged_item_count = tool_messages.len().saturating_add(context_files.len()) as u64;
    merge_span.finish_tool(
        PerfOutcome::Success,
        tool_calls.len() as u64,
        merged_item_count,
        None,
    );

    let pp_settings = options.postprocess_settings.unwrap_or_default();

    let truncation_exempt_ids: std::collections::HashSet<String> = tool_calls
        .iter()
        .filter(|tc| {
            let resolved =
                crate::llm::adapters::claude_code_compat::cc_resolve_tool_name(&tc.function.name);
            is_tool_result_truncation_exempt(&tc.function.name)
                || is_tool_result_truncation_exempt(resolved.as_str())
        })
        .map(|tc| tc.id.clone())
        .collect();

    let (ready_messages, tool_messages): (Vec<_>, Vec<_>) =
        tool_messages.into_iter().partition(|message| {
            message
                .extra
                .get("tool_job_postprocessed")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
                || message.extra.contains_key("tool_job")
        });
    let mut results = crate::file_activity::TOOL_ACTOR
        .scope(
            chat_id,
            postprocess_tool_results(
                app.gcx.clone(),
                None,
                tool_messages,
                context_files,
                budget,
                pp_settings,
                messages,
                &truncation_exempt_ids,
            ),
        )
        .await;

    results.extend(ready_messages);
    results.sort_by_key(|message| {
        tool_calls
            .iter()
            .position(|call| call.id == message.tool_call_id)
            .unwrap_or(usize::MAX)
    });
    (results, any_corrections)
}

async fn execute_parallel_batch(
    app: AppState,
    ccx: Arc<AMutex<AtCommandsContext>>,
    batch: &[(usize, ChatToolCall)],
    catalog: Arc<ToolCatalogSnapshot>,
    turn_tool_pool: Option<TurnToolPool>,
    prepared_parallel_slots: &mut std::collections::HashMap<String, usize>,
    serial_registry: Arc<SerialToolRegistry>,
    mode_id: &str,
    model_id: Option<&str>,
    native_pause: Option<Arc<refact_agents_core::registry::PauseGate>>,
    native_abort: Option<Arc<AtomicBool>>,
) -> Vec<(usize, bool, Vec<ChatMessage>, Vec<ContextFile>)> {
    if current_runner_execution(&app, &ccx).await.is_err() {
        return Vec::new();
    }
    if let Some(pool) = turn_tool_pool.as_ref() {
        let mut required_slots: std::collections::HashMap<
            String,
            (refact_tool_api::ToolDesc, usize),
        > = std::collections::HashMap::new();
        for (_, tool_call) in batch {
            let resolved_name = crate::llm::adapters::claude_code_compat::cc_resolve_tool_name(
                &tool_call.function.name,
            );
            let Some(desc) = catalog
                .index
                .tools
                .iter()
                .find(|desc| desc.name == resolved_name)
            else {
                continue;
            };
            let entry = required_slots
                .entry(tool_pool_key(desc))
                .or_insert_with(|| (desc.clone(), 0));
            entry.1 += 1;
        }
        let requested_slots = required_slots
            .into_iter()
            .filter_map(|(key, (desc, count))| {
                let required = count;
                let prepared = prepared_parallel_slots
                    .get(&key)
                    .copied()
                    .unwrap_or_default();
                (required > prepared).then(|| (key, desc, required))
            })
            .collect::<Vec<_>>();
        let descriptors = requested_slots
            .iter()
            .map(|(_, desc, count)| (desc.clone(), *count))
            .collect::<Vec<_>>();
        if let Err(error) = app
            .tool_registry
            .prepare_turn_tool_pool(pool, &catalog, mode_id, model_id, &descriptors)
            .await
        {
            return batch
                .iter()
                .map(|(idx, tool_call)| {
                    (
                        *idx,
                        true,
                        vec![ChatMessage {
                            message_id: Uuid::new_v4().to_string(),
                            role: "tool".to_string(),
                            content: ChatContent::SimpleText(format!(
                                "Error preparing tool pool: {error}"
                            )),
                            tool_call_id: tool_call.id.clone(),
                            tool_failed: Some(true),
                            ..Default::default()
                        }],
                        Vec::new(),
                    )
                })
                .collect();
        }
        for (key, _, count) in requested_slots {
            prepared_parallel_slots.insert(key, count);
        }
    }
    let batch_size = batch.len();
    let execution = match current_runner_execution(&app, &ccx).await {
        Ok(execution) => execution,
        Err(_) => return Vec::new(),
    };
    let waits: Vec<_> = batch
        .iter()
        .map(|(_, call)| recognized_runner_wait(&catalog, call))
        .collect();
    let guards = if let Some(execution) = execution.as_ref() {
        match runner_capacity_wait(&app, &ccx, execution, execution.begin_tool_batch(&waits)).await
        {
            Ok(guards) => guards,
            Err(error) => {
                return batch
                    .iter()
                    .map(|(idx, call)| runner_capacity_error(*idx, call, &error))
                    .collect()
            }
        }
    } else {
        (0..batch_size).map(|_| None).collect()
    };

    let futures: Vec<_> = batch
        .iter()
        .zip(guards)
        .zip(waits)
        .map(|(((idx, tool_call), operation_guard), is_wait)| {
            let gcx = app.clone();
            let ccx = ccx.clone();
            let execution = execution.clone();
            let serial_registry = serial_registry.clone();
            let catalog = catalog.clone();
            let turn_tool_pool = turn_tool_pool.clone();
            let tool_call = tool_call.clone();
            let idx = *idx;
            let batch_size = batch_size;
            let mode_id = mode_id.to_string();
            let model_id = model_id.map(|s| s.to_string());
            let native_pause = native_pause.clone();
            let native_abort = native_abort.clone();

            async move {
                if is_wait {
                    if let Some(execution) = execution.as_ref() {
                        if let Err(error) = runner_capacity_wait(
                            &gcx,
                            &ccx,
                            execution,
                            execution.suspend_for_wait(),
                        )
                        .await
                        {
                            return runner_capacity_error(idx, &tool_call, &error);
                        }
                    }
                }
                execute_admitted_tool(
                    gcx,
                    ccx,
                    idx,
                    tool_call,
                    catalog,
                    turn_tool_pool,
                    serial_registry,
                    true,
                    batch_size,
                    &mode_id,
                    model_id.as_deref(),
                    native_pause.clone(),
                    native_abort,
                    execution.clone(),
                    vec![operation_guard],
                )
                .await
            }
        })
        .collect();

    join_all(futures).await
}

#[cfg(test)]
async fn tool_runtime_event_lines(
    app: AppState,
    tool_name: &str,
    chat_label: &str,
) -> (String, Option<String>) {
    let (fallback_title, fallback_speech) =
        fallback_tool_runtime_event_lines(tool_name, chat_label);
    let workflow_summary = format!("Using {} to help with '{}'...", tool_name, chat_label);
    let Some((title, speech)) = app
        .buddy_event_sink
        .render_runtime_event_fast(tool_name, &workflow_summary, "started")
        .await
    else {
        return (fallback_title, fallback_speech);
    };
    runtime_event_lines_with_fallback(title, speech, fallback_title, fallback_speech)
}

fn fallback_tool_runtime_event_lines(
    tool_name: &str,
    chat_label: &str,
) -> (String, Option<String>) {
    (
        format!("Running {} in '{}'", tool_name, chat_label),
        Some(format!(
            "Using {} to help with '{}'...",
            tool_name, chat_label
        )),
    )
}

#[cfg(test)]
fn runtime_event_lines_with_fallback(
    title: String,
    speech: Option<String>,
    fallback_title: String,
    fallback_speech: Option<String>,
) -> (String, Option<String>) {
    let title = if title.trim().is_empty() {
        fallback_title
    } else {
        title
    };
    let speech = match speech {
        Some(s) if !s.trim().is_empty() => Some(s),
        _ => fallback_speech,
    };
    (title, speech)
}

async fn begin_thread_board_preparation(
    app: &AppState,
    thread: &ThreadParams,
    context: Option<&Arc<AMutex<AtCommandsContext>>>,
) -> Result<Option<crate::chat::session_runtime::BoardOperation>, String> {
    let session = app.chat.sessions.read().await.get(&thread.id).cloned();
    if let Some(session) = session {
        return super::session_runtime::begin_board_operation(app, &session)
            .await
            .map_err(|reason| reason.recovery_message());
    }
    if thread
        .task_meta
        .as_ref()
        .is_some_and(|meta| matches!(meta.role.as_str(), "agent" | "agents"))
    {
        if let Some(context) = context {
            let context = context.lock().await;
            if context.chat_id == thread.id
                && context.subchat_depth > 0
                && context.background_agent_id.is_none()
                && context.task_meta == thread.task_meta
            {
                return Ok(None);
            }
        }
        return Err(crate::tasks::admission::ExecutionUnavailable::Missing.recovery_message());
    }
    Ok(None)
}

pub async fn execute_tools(
    app: AppState,
    tool_calls: &[ChatToolCall],
    messages: &[ChatMessage],
    thread: &ThreadParams,
    mode_id: &str,
    model_id: Option<&str>,
    mut options: ExecuteToolsOptions,
) -> (Vec<ChatMessage>, bool) {
    if tool_calls.is_empty() {
        return (vec![], false);
    }
    let operation = match begin_thread_board_preparation(
        &app,
        thread,
        options.execution_context.as_ref(),
    )
    .await
    {
        Ok(operation) => operation,
        Err(error) => return context_error_results(tool_calls, error),
    };
    let intent = super::generation::tool_catalog_intent_for_thread(&app, thread).await;
    let catalog = match options
        .catalog
        .clone()
        .filter(|catalog| catalog.intent == intent)
    {
        Some(catalog) => catalog,
        None => {
            app.tool_registry
                .acquire_tool_catalog_with_intent(
                    mode_id,
                    model_id,
                    thread
                        .worktree
                        .as_ref()
                        .map(|worktree| worktree.root.to_string_lossy().into_owned())
                        .as_deref(),
                    intent,
                )
                .await
        }
    };
    let turn_tool_pool = app
        .tool_registry
        .acquire_turn_tool_pool(
            mode_id,
            model_id,
            execution_scope_for_thread(thread).as_deref(),
            &catalog,
        )
        .await;

    let n_ctx = get_effective_n_ctx(app.clone(), thread).await;
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

    let background_agent_id = app.agents.find_agent_id_by_child_chat_id(&thread.id).await;
    let abort = match background_agent_id.as_deref() {
        Some(agent_id) => app.agents.abort_flag(agent_id).await,
        None => None,
    };
    let ccx = match options.execution_context.take() {
        Some(context) => context,
        None => {
            match build_tool_execution_context(app.clone(), n_ctx, messages, thread, abort).await {
                Ok(context) => context,
                Err(error) => return context_error_results(tool_calls, error),
            }
        }
    };

    {
        let mut cgcx = ccx.lock().await;
        cgcx.background_agent_id = background_agent_id;
        if let Some(agent_id) = cgcx.background_agent_id.as_deref() {
            if let Some(config) = app.agent_executions.saved_config(agent_id) {
                cgcx.subchat_depth = config.subchat_depth;
            }
        }
        cgcx.tokens_for_rag = (n_ctx / 2).max(4096);
        if let Some(params) = options.subchat_tool_parameters.clone() {
            cgcx.subchat_tool_parameters = params;
        }
    }

    let app2 = app.clone();
    let chat_id = thread.id.clone();
    let chat_label = {
        let t = thread.title.trim().to_string();
        if t.is_empty() || t == "New Chat" {
            "Untitled chat".to_string()
        } else {
            t.chars().take(60).collect()
        }
    };
    let tool_meta: Vec<(String, String)> = tool_calls
        .iter()
        .map(|tc| (tc.id.clone(), format!("tool_{}", tc.id)))
        .collect();
    for (tc, (_, dedupe_key)) in tool_calls.iter().zip(tool_meta.iter()) {
        let (title, speech_text) =
            fallback_tool_runtime_event_lines(&tc.function.name, &chat_label);
        let mut ev = make_runtime_event("tool_used", &title, "tool", dedupe_key, "started", None);
        ev.speech_text = speech_text;
        ev.scene = Some("working".to_string());
        ev.chat_id = Some(chat_id.to_string());
        app.buddy_event_sink.enqueue_event(ev).await;
    }

    drop(operation);
    let (result_msgs, had_corrections) = execute_tools_inner(
        app,
        ccx,
        tool_calls,
        mode_id,
        model_id,
        budget,
        options,
        messages,
        catalog,
        turn_tool_pool,
    )
    .await;

    for (tool_call_id, dedupe_key) in &tool_meta {
        if result_msgs
            .iter()
            .any(|m| &m.tool_call_id == tool_call_id && m.extra.contains_key("tool_job"))
        {
            continue;
        }
        let failed = result_msgs
            .iter()
            .any(|m| &m.tool_call_id == tool_call_id && m.tool_failed == Some(true));
        if failed {
            app2.buddy_event_sink
                .complete_event(dedupe_key, "failed")
                .await;
        } else {
            app2.buddy_event_sink
                .complete_event(dedupe_key, "completed")
                .await;
        }
    }

    (result_msgs, had_corrections)
}

#[cfg(test)]
mod interruptible_wait_tests {
    use super::*;

    #[tokio::test]
    async fn live_sleep_execution_yields_to_all_queued_events() {
        use crate::tools::tools_description::Tool;
        use refact_core::chat_types::{PendingDelivery, PushMode};
        for (push, source, fenced, process) in [
            (Some(PushMode::Append), "test", false, false),
            (Some(PushMode::WhenIdle), "test", false, false),
            (None, "test", false, false),
            (Some(PushMode::Preempt), "test", false, false),
            (
                Some(PushMode::Append),
                crate::agents::push::NOTICE_PROGRESS,
                false,
                false,
            ),
            (
                Some(PushMode::Append),
                "exec.process_completed",
                true,
                false,
            ),
            (Some(PushMode::Append), "exec.subscribe", false, true),
        ] {
            let gcx = crate::global_context::tests::make_test_gcx().await;
            let base_app = AppState::from_gcx(gcx).await;
            let app = AppState {
                tool_registry: Arc::new(
                    crate::app_state::AppToolRegistry::with_fixture_tool_factory(
                        base_app.gcx.clone(),
                        Arc::new(|| {
                            vec![Box::new(crate::tools::tool_wait::ToolWait {
                                config_path: String::new(),
                            })
                                as Box<dyn crate::tools::tools_description::Tool + Send>]
                        }),
                    ),
                ),
                ..base_app
            };
            let registry = app.runtime.exec_registry.clone();
            let process_id = registry
                .register(
                    refact_exec::ExecProcessMeta::new(
                        refact_exec::ExecMode::Background,
                        "pending".into(),
                    )
                    .with_chat_id("live-wait"),
                    1024,
                )
                .await
                .meta
                .process_id;
            let session = Arc::new(AMutex::new(ChatSession::new("live-wait".into())));
            app.chat
                .sessions
                .write()
                .await
                .insert("live-wait".into(), session.clone());
            let ccx = Arc::new(AMutex::new(
                AtCommandsContext::new_from_app(
                    app.clone(),
                    4096,
                    20,
                    false,
                    vec![],
                    "live-wait".into(),
                    None,
                    "model".into(),
                    None,
                    None,
                )
                .await,
            ));
            let desc = crate::tools::tool_wait::ToolWait {
                config_path: String::new(),
            }
            .tool_description();
            let catalog = Arc::new(ToolCatalogSnapshot {
                intent: refact_runtime_api::ToolCatalogIntent::Policy,
                index: refact_runtime_api::ToolRegistryIndex {
                    tools: vec![desc],
                    mcp_lazy_mode: false,
                    mcp_total_count: 0,
                    mcp_tool_index: vec![],
                },
                policy: vec![],
                aliases: refact_tool_api::ToolAliasRegistry::new(),
            });
            let tool_call = ChatToolCall {
                id: "live-sleep".into(),
                index: Some(0),
                tool_type: "function".into(),
                function: crate::call_validation::ChatToolFunction {
                    name: "wait".into(),
                    arguments: if process {
                        serde_json::json!({"target":{"kind":"process","process_id": process_id}})
                            .to_string()
                    } else {
                        serde_json::json!({"target":{"kind":"duration","duration_ms":30000},"description":"wait"}).to_string()
                    },
                },
                extra_content: None,
                started_at_ms: None,
                completed_at_ms: None,
            };
            {
                let mut locked = session.lock().await;
                locked.turn_depth = 1;
                locked.add_message(ChatMessage {
                    role: "assistant".into(),
                    tool_calls: Some(vec![tool_call.clone()]),
                    ..Default::default()
                });
            }
            let run = tokio::spawn(async move {
                execute_single_tool(
                    app,
                    ccx,
                    0,
                    tool_call,
                    catalog,
                    None,
                    Arc::new(Default::default()),
                    true,
                    1,
                    "agent",
                    None,
                    None,
                    None,
                    None,
                )
                .await
            });
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    if session.lock().await.runtime.waiting_interruptible {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("sleep did not enter wait wrapper");
            {
                let mut locked = session.lock().await;
                if let Some(push) = push {
                    let mut delivery = PendingDelivery::with_id(
                        "incoming",
                        vec![ChatMessage {
                            role: "user".into(),
                            content: ChatContent::SimpleText("wake up".into()),
                            ..Default::default()
                        }],
                        push,
                        source,
                        true,
                    );
                    if fenced {
                        delivery.after_tool_call_id = Some("live-sleep".into());
                    }
                    locked.enqueue_delivery(delivery).unwrap();
                } else {
                    assert_eq!(
                        locked.enqueue_command(crate::chat::types::CommandRequest {
                            native_approval_guard: None,
                            supervision_guard: None,
                            client_request_id: "command".into(),
                            priority: false,
                            command: crate::chat::types::ChatCommand::SetParams {
                                patch: serde_json::json!({})
                            },
                        }),
                        crate::chat::types::EnqueueCommandOutcome::Accepted
                    );
                }
            }
            let (_, corrected, messages, _) =
                tokio::time::timeout(std::time::Duration::from_secs(10), run)
                    .await
                    .unwrap()
                    .unwrap();
            assert!(!corrected);
            if process {
                assert!(messages[0]
                    .content
                    .content_text_only()
                    .contains("outcome: interrupted"));
                assert!(matches!(
                    registry.get(&process_id).await.unwrap().status,
                    refact_exec::ExecStatus::Starting
                ));
            } else {
                assert_eq!(messages[0].extra["wait"]["interrupted"], true);
                assert!(messages[0].extra["wait"]["reason"]
                    .as_str()
                    .unwrap()
                    .contains("message arrived"));
            }
            let mut locked = session.lock().await;
            assert!(!locked.abort_flag.load(Ordering::SeqCst));
            assert!(!locked.runtime.waiting_interruptible);
            for message in messages {
                locked.add_message(message);
            }
            if push.is_some() {
                assert_eq!(
                    locked.drain_pending_deliveries(),
                    (vec!["incoming".into()], true)
                );
            } else {
                assert!(locked.yield_to_queued_commands_at_boundary());
                assert_eq!(locked.command_queue[0].client_request_id, "command");
            }
            assert_eq!(locked.drain_pending_deliveries(), (vec![], false));
            assert_eq!(locked.messages[0].role, "assistant");
            assert_eq!(locked.messages[1].role, "tool");
            if push.is_some() {
                assert!(locked
                    .messages
                    .iter()
                    .skip(2)
                    .any(|message| message.content.content_text_only().contains("wake up")));
            }
        }
    }

    #[tokio::test]
    async fn guard_clears_runtime_on_finish_and_drop() {
        let session = Arc::new(AMutex::new(ChatSession::new("wait-guard".into())));
        let mut first = InterruptibleWaitGuard::begin(session.clone()).await;
        let second = InterruptibleWaitGuard::begin(session.clone()).await;
        assert!(session.lock().await.runtime.waiting_interruptible);
        first.finish().await;
        assert!(session.lock().await.runtime.waiting_interruptible);
        drop(second);
        assert!(!session.lock().await.runtime.waiting_interruptible);
        assert_eq!(session.lock().await.interruptible_waits, 0);
    }

    #[tokio::test]
    async fn guard_cleans_up_on_panic() {
        let session = Arc::new(AMutex::new(ChatSession::new("wait-panic".into())));
        let copy = session.clone();
        assert!(tokio::spawn(async move {
            let _guard = InterruptibleWaitGuard::begin(copy).await;
            panic!("test panic");
        })
        .await
        .is_err());
        assert!(!session.lock().await.runtime.waiting_interruptible);
    }
}
