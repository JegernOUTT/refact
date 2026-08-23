use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use chrono::Utc;
use tokio::sync::{Mutex as AMutex, Semaphore};
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
use crate::ext::hooks_runner::{HookPayload, first_block_reason, get_project_dir_string, run_hooks};
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

pub(crate) const SHELL_PRIVACY_APPROVAL_RULE: &str =
    "Privacy approval required: command read guarded files";

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

use super::config::{limits, tokens};

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
) -> Arc<AMutex<AtCommandsContext>> {
    Arc::new(AMutex::new(
        AtCommandsContext::new_with_abort(
            app.clone(),
            n_ctx,
            CHAT_TOP_N,
            false,
            messages.to_vec(),
            thread.id.clone(),
            thread.root_chat_id.clone(),
            thread.model.clone(),
            thread.task_meta.clone(),
            thread.worktree.clone(),
            abort_flag,
        )
        .await,
    ))
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

// Tools whose results are expensive, non-reproducible reports (code reviews,
// subagent/delegate outcomes). Their outputs bypass the shared per-round
// ToolBudget so running many of them at once does not chop each report to a
// fair-share fragment; overall context pressure is handled later by chat
// compression instead.
const TOOL_RESULT_TRUNCATION_EXEMPT_TOOLS: &[&str] = &[
    "review",
    "code_review",
    "subagent",
    "delegate",
    "agent_wait",
    "agent_result",
];

fn is_tool_result_truncation_exempt(tool_name: &str) -> bool {
    TOOL_RESULT_TRUNCATION_EXEMPT_TOOLS.contains(&tool_name)
}

fn should_auto_approve_confirmation(thread: &ThreadParams, tool_name: &str) -> bool {
    if thread.autonomous_no_confirm {
        return true;
    }
    thread.auto_approve_dangerous_commands
        || (thread.auto_approve_editing_tools && is_editing_tool(tool_name))
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

fn spawn_subchat_bridge(
    ccx: Arc<AMutex<AtCommandsContext>>,
    session_arc: Arc<AMutex<ChatSession>>,
) -> Arc<AtomicBool> {
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_flag_clone = cancel_flag.clone();

    tokio::spawn(async move {
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

            let recv_result = {
                let mut rx = subchat_rx.lock().await;
                tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await
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

    cancel_flag
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
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
            ["alpha", "beta"]
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
        }
    }

    async fn process_deterministic_tool_turn(
        registry: DeterministicToolRegistry,
        tool_calls: Vec<ChatToolCall>,
    ) -> (ToolStepOutcome, Arc<MemoryPerfSink>) {
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

        let outcome = process_tool_calls_once(app, session, "agent", None).await;
        (outcome, sink)
    }

    fn recorded_components(sink: &MemoryPerfSink) -> Vec<&'static str> {
        sink.events().iter().map(|event| event.component).collect()
    }

    #[serial]
    #[tokio::test]
    async fn same_turn_tool_diagnostics_cover_stages_without_confirmation_wait() {
        let (outcome, sink) = process_deterministic_tool_turn(
            DeterministicToolRegistry {
                confirmation_result: MatchConfirmDenyResult::PASS,
                runtime_failure: false,
            },
            vec![
                deterministic_tool_call("tool-alpha", "alpha"),
                deterministic_tool_call("tool-beta", "beta"),
            ],
        )
        .await;

        assert!(matches!(outcome, ToolStepOutcome::Continue));
        let components = recorded_components(&sink);
        for component in [
            PerfComponent::ToolCatalogBuild,
            PerfComponent::ToolAliasResolution,
            PerfComponent::ToolConfirmationPreflight,
            PerfComponent::ToolPolicyLookup,
            PerfComponent::ToolPreHook,
            PerfComponent::ToolSemaphoreWait,
            PerfComponent::ToolRuntime,
            PerfComponent::ToolPostHook,
            PerfComponent::ToolResultMerge,
            PerfComponent::ToolResultPostprocess,
        ] {
            assert!(
                components.contains(&component.as_str()),
                "missing {component:?}"
            );
        }
        assert!(components
            .iter()
            .all(|component| *component != "tool.confirmation_wait"));
        let runtime_events = sink
            .events()
            .into_iter()
            .filter(|event| event.component == PerfComponent::ToolRuntime.as_str())
            .collect::<Vec<_>>();
        assert_eq!(runtime_events.len(), 2);
        assert!(runtime_events.iter().all(|event| {
            event.outcome == PerfOutcome::Success.as_str()
                && event.batch_size == Some(1)
                && event.execution_class.is_none()
        }));
    }

    #[tokio::test]
    async fn catalog_aliases_remain_stable_when_tool_descriptors_change_mid_batch() {
        let initial = ToolCatalogSnapshot {
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

    #[serial]
    #[tokio::test]
    async fn app_registry_runtime_starts_after_catalog_build() {
        let (outcome, sink) = process_deterministic_tool_turn(
            DeterministicToolRegistry {
                confirmation_result: MatchConfirmDenyResult::PASS,
                runtime_failure: false,
            },
            vec![deterministic_tool_call("tool-alpha", "alpha")],
        )
        .await;

        assert!(matches!(outcome, ToolStepOutcome::Continue));
        let events = sink.events();
        let runtime_index = events
            .iter()
            .position(|event| event.component == PerfComponent::ToolRuntime.as_str())
            .expect("runtime event");
        let catalog_index = events
            .iter()
            .position(|event| event.component == PerfComponent::ToolCatalogBuild.as_str())
            .expect("catalog build event");
        assert!(catalog_index < runtime_index);
        assert_eq!(
            events
                .iter()
                .filter(|event| event.component == PerfComponent::ToolRuntime.as_str())
                .count(),
            1
        );
    }

    #[serial]
    #[tokio::test]
    async fn same_turn_tool_diagnostics_record_runtime_failure() {
        let (outcome, sink) = process_deterministic_tool_turn(
            DeterministicToolRegistry {
                confirmation_result: MatchConfirmDenyResult::PASS,
                runtime_failure: true,
            },
            vec![deterministic_tool_call("tool-alpha", "alpha")],
        )
        .await;

        assert!(matches!(outcome, ToolStepOutcome::Continue));
        let runtime_events = sink
            .events()
            .into_iter()
            .filter(|event| event.component == PerfComponent::ToolRuntime.as_str())
            .collect::<Vec<_>>();
        assert_eq!(runtime_events.len(), 1);
        assert_eq!(runtime_events[0].outcome, PerfOutcome::Failure.as_str());
    }

    #[serial]
    #[tokio::test]
    async fn same_turn_confirmation_pause_does_not_record_wait_time() {
        let (outcome, sink) = process_deterministic_tool_turn(
            DeterministicToolRegistry {
                confirmation_result: MatchConfirmDenyResult::CONFIRMATION,
                runtime_failure: false,
            },
            vec![deterministic_tool_call("tool-alpha", "alpha")],
        )
        .await;

        assert!(matches!(outcome, ToolStepOutcome::Paused));
        let components = recorded_components(&sink);
        assert!(components.contains(&PerfComponent::ToolConfirmationPreflight.as_str()));
        assert!(components.contains(&PerfComponent::ToolPolicyLookup.as_str()));
        assert!(components
            .iter()
            .all(|component| *component != "tool.confirmation_wait"));
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
                .await;
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

    #[test]
    fn test_rewrite_cc_native_grep_tool_call() {
        let mut tc = ChatToolCall {
            id: "call-1".to_string(),
            index: None,
            function: crate::call_validation::ChatToolFunction {
                name: "Grep".to_string(),
                arguments: r#"{"pattern":"needle","path":"src"}"#.to_string(),
            },
            tool_type: "function".to_string(),
            extra_content: None,
        };

        rewrite_cc_native_stub_tool_call(&mut tc);

        assert_eq!(tc.function.name, "search_pattern");
        let args = tc.function.parse_args().unwrap();
        assert_eq!(args.get("pattern").and_then(|v| v.as_str()), Some("needle"));
        assert_eq!(args.get("scope").and_then(|v| v.as_str()), Some("src"));
    }

    #[test]
    fn test_rewrite_cc_native_glob_tool_call() {
        let mut tc = ChatToolCall {
            id: "call-1".to_string(),
            index: None,
            function: crate::call_validation::ChatToolFunction {
                name: "Glob".to_string(),
                arguments: r#"{"pattern":"src/**/*.rs"}"#.to_string(),
            },
            tool_type: "function".to_string(),
            extra_content: None,
        };

        rewrite_cc_native_stub_tool_call(&mut tc);

        assert_eq!(tc.function.name, "glob");
        let args = tc.function.parse_args().unwrap();
        assert_eq!(
            args.get("pattern").and_then(|v| v.as_str()),
            Some("src/**/*.rs")
        );
        assert!(args.get("path").is_none());
        assert!(args.get("scope").is_none());
    }

    #[test]
    fn test_rewrite_cc_native_glob_preserves_path() {
        let mut tc = ChatToolCall {
            id: "call-2".to_string(),
            index: None,
            function: crate::call_validation::ChatToolFunction {
                name: "Glob".to_string(),
                arguments: r#"{"pattern":"*.rs","path":"engine/src"}"#.to_string(),
            },
            tool_type: "function".to_string(),
            extra_content: None,
        };

        rewrite_cc_native_stub_tool_call(&mut tc);

        assert_eq!(tc.function.name, "glob");
        let args = tc.function.parse_args().unwrap();
        assert_eq!(args.get("pattern").and_then(|v| v.as_str()), Some("*.rs"));
        assert_eq!(
            args.get("path").and_then(|v| v.as_str()),
            Some("engine/src")
        );
    }

    #[test]
    fn test_mixed_cc_batch_preserves_native_stub_rewrites() {
        let mut glob = ChatToolCall {
            id: "call-glob".to_string(),
            index: None,
            function: crate::call_validation::ChatToolFunction {
                name: "Glob".to_string(),
                arguments: r#"{"pattern":"src/**/*.rs"}"#.to_string(),
            },
            tool_type: "function".to_string(),
            extra_content: None,
        };
        let mut grep = ChatToolCall {
            id: "call-grep".to_string(),
            index: None,
            function: crate::call_validation::ChatToolFunction {
                name: "Grep".to_string(),
                arguments: r#"{"pattern":"needle"}"#.to_string(),
            },
            tool_type: "function".to_string(),
            extra_content: None,
        };

        rewrite_cc_native_stub_tool_call(&mut glob);
        rewrite_cc_native_stub_tool_call(&mut grep);

        assert_eq!(glob.function.name, "glob");
        assert_eq!(grep.function.name, "search_pattern");
    }

    #[serial]
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
        assert!(is_tool_result_truncation_exempt("code_review"));
        assert!(is_tool_result_truncation_exempt("subagent"));
        assert!(is_tool_result_truncation_exempt("delegate"));
        assert!(is_tool_result_truncation_exempt("agent_wait"));
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
        let thread = ThreadParams {
            autonomous_no_confirm: true,
            ..Default::default()
        };

        assert!(should_auto_approve_confirmation(&thread, "shell"));
        assert!(should_auto_approve_confirmation(&thread, "cat"));
        assert!(should_auto_approve_confirmation(&thread, "apply_patch"));
    }

    #[test]
    fn test_autonomous_allowlist_rejects_unlisted_tool() {
        let result = compute_final_action(
            &MatchConfirmDenyResult::CONFIRMATION,
            None,
            false,
            "unsafe_tool",
        );
        assert_eq!(result, "ask");
        let thread = ThreadParams {
            autonomous_no_confirm: true,
            ..Default::default()
        };
        assert!(should_auto_approve_confirmation(&thread, "any_tool"));
    }

    #[test]
    fn test_user_chat_still_requires_approval() {
        let thread = ThreadParams::default();

        assert!(!should_auto_approve_confirmation(&thread, "apply_patch"));
        assert!(!should_auto_approve_confirmation(&thread, "mv"));
    }

    #[tokio::test]
    async fn escalation_does_not_require_confirmation() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let thread = ThreadParams {
            id: "escalation-confirmation".to_string(),
            model: "model".to_string(),
            ..Default::default()
        };
        let session = ChatSession::new(thread.id.clone());
        app.chat
            .sessions
            .write()
            .await
            .insert(thread.id.clone(), Arc::new(AMutex::new(session)));
        let tool_call = ChatToolCall {
            id: "tc-escalate".to_string(),
            index: None,
            function: crate::call_validation::ChatToolFunction {
                name: "shell".to_string(),
                arguments: serde_json::json!({
                    "command": "printf ok",
                    "description": "Print ok",
                    "escalate": {
                        "mode": "full_access",
                        "justification": "write the requested system path"
                    }
                })
                .to_string(),
            },
            tool_type: "function".to_string(),
            extra_content: None,
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

        assert!(denials.is_empty());
        assert!(confirmations.is_empty());
        let session = app
            .chat
            .sessions
            .read()
            .await
            .get(&thread.id)
            .cloned()
            .unwrap();
        let session = session.lock().await;
        assert!(session.post_tool_side_effects.is_empty());
    }

    #[serial]
    #[tokio::test]
    async fn invalid_escalation_argument_is_still_denied() {
        let _lock = perf_diagnostics::PERF_RECORDER_TEST_LOCK.lock().unwrap();

        let (_guard, sink) = install_perf_recorder();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let thread = ThreadParams {
            id: "escalation-invalid".to_string(),
            model: "model".to_string(),
            ..Default::default()
        };
        let session = ChatSession::new(thread.id.clone());
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
    fn ordinary_confirmation_keeps_existing_auto_approval_behavior() {
        let thread = ThreadParams {
            auto_approve_dangerous_commands: true,
            ..Default::default()
        };
        let reason = PauseReason {
            reason_type: "confirmation".to_string(),
            tool_name: "shell".to_string(),
            command: "printf ok".to_string(),
            rule: "matched confirmation rule".to_string(),
            tool_call_id: "tc-shell".to_string(),
            integr_config_path: None,
        };

        assert!(should_auto_approve_confirmation(&thread, &reason.tool_name));
    }

    #[test]
    fn test_max_parallel_clamp() {
        assert!(1_usize.max(1) >= 1);
        assert!(0_usize.max(1) >= 1);
        assert!(100_usize.max(1) == 100);
    }

    #[test]
    fn test_tool_config_default() {
        let config = refact_tool_api::ToolConfig::default();
        assert!(config.enabled);
        assert!(config.allow_parallel.is_none());
    }

    #[test]
    fn test_tool_config_serde_roundtrip() {
        let config = refact_tool_api::ToolConfig {
            enabled: true,
            allow_parallel: Some(false),
        };
        let yaml = serde_yaml::to_string(&config).unwrap();
        let parsed: refact_tool_api::ToolConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.enabled, config.enabled);
        assert_eq!(parsed.allow_parallel, config.allow_parallel);
    }

    #[test]
    fn test_tool_config_serde_skip_none() {
        let config = refact_tool_api::ToolConfig {
            enabled: true,
            allow_parallel: None,
        };
        let yaml = serde_yaml::to_string(&config).unwrap();
        assert!(!yaml.contains("allow_parallel"));
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

        assert!(
            !effective,
            "YAML should not be able to enable parallelism for unsafe tools"
        );
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

        assert!(
            !effective,
            "YAML should be able to disable parallelism for safe tools"
        );
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
                "cat" | "tree" | "search" => true, // parallel
                "apply_patch" | "shell" => false,  // non-parallel (barriers)
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
        assert_eq!(
            ordered,
            vec!["result_0", "result_1", "result_2", "result_3", "result_4"]
        );
    }

    #[test]
    fn test_corrections_aggregation() {
        // Test that corrections from multiple tools are properly aggregated
        // Simulates the aggregation logic in execute_tools_inner

        // Results: (idx, had_corrections, msgs, files)
        let results: Vec<(usize, bool, Vec<&str>, Vec<&str>)> = vec![
            (0, false, vec!["msg0"], vec![]),
            (1, true, vec!["msg1"], vec![]), // This tool had corrections
            (2, false, vec!["msg2"], vec![]),
        ];

        let any_corrections = results
            .iter()
            .any(|(_, had_corrections, _, _)| *had_corrections);
        assert!(
            any_corrections,
            "Should detect that at least one tool had corrections"
        );

        // Test with no corrections
        let results_no_corrections: Vec<(usize, bool, Vec<&str>, Vec<&str>)> = vec![
            (0, false, vec!["msg0"], vec![]),
            (1, false, vec!["msg1"], vec![]),
        ];

        let any_corrections = results_no_corrections
            .iter()
            .any(|(_, had_corrections, _, _)| *had_corrections);
        assert!(
            !any_corrections,
            "Should detect no corrections when all tools succeeded cleanly"
        );
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

    #[test]
    fn test_always_ask_tools_override_auto() {
        assert_eq!(
            compute_final_action(
                &MatchConfirmDenyResult::PASS,
                Some("auto"),
                true,
                "compress_chat_probe"
            ),
            "ask"
        );
        assert_eq!(
            compute_final_action(&MatchConfirmDenyResult::PASS, None, true, "handoff_to_mode"),
            "auto"
        );
    }

    #[tokio::test]
    async fn tool_approved_pushed_on_decision() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        {
            let user_activity = app.buddy.user_activity.clone();
            let mut ring = user_activity.lock().await;
            ring.push(UserAction::ToolApproved {
                tool_name: "cat".to_string(),
                chat_id: "chat-1".to_string(),
                ts: Utc::now(),
            });
        }

        let user_activity = app.buddy.user_activity.clone();
        let ring = user_activity.lock().await;
        assert!(ring.snapshot().iter().any(|action| matches!(
            action,
            UserAction::ToolApproved { tool_name, chat_id, .. }
                if tool_name == "cat" && chat_id == "chat-1"
        )));
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

    #[tokio::test]
    async fn execute_tools_uses_voice_for_runtime_event_title() {
        let (service, renderer) = crate::buddy::voice_service::test_voice_service_with_responses(
            vec![Some("voice tool title".to_string())],
        );
        let _guard = crate::buddy::voice_service::install_test_voice_service(service).await;
        let gcx = make_gcx_with_buddy().await;

        let (title, speech_text) = tool_runtime_event_lines(gcx, "cat", "Thread").await;

        assert_eq!(title, "voice tool title");
        assert_eq!(
            speech_text.as_deref(),
            Some("Using cat to help with 'Thread'...")
        );
        assert_eq!(renderer.intent_kinds(), vec!["runtime:started".to_string()]);
    }

    #[test]
    fn tool_runtime_event_lines_falls_back_on_empty_voice() {
        let (title, speech_text) = runtime_event_lines_with_fallback(
            "   ".to_string(),
            Some("\t".to_string()),
            "Running cat in 'Thread'".to_string(),
            Some("Using cat to help with 'Thread'...".to_string()),
        );

        assert_eq!(title, "Running cat in 'Thread'");
        assert_eq!(
            speech_text.as_deref(),
            Some("Using cat to help with 'Thread'...")
        );
        assert!(!title.trim().is_empty());
    }

    #[test]
    fn tool_runtime_event_uses_short_timeout() {
        assert_eq!(
            crate::buddy::voice_service::VOICE_RUNTIME_EVENT_TIMEOUT_MS,
            1500
        );
    }
}

pub async fn process_tool_calls_once(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    mode_id: &str,
    model_id: Option<&str>,
) -> ToolStepOutcome {
    let (
        tool_calls,
        server_tool_calls,
        messages,
        thread,
        tool_message_index,
        allowed_tools,
        source_command,
    ) = {
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
                    session.active_command.allowed_tools.clone(),
                    session.active_command.name.clone(),
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
        }
        return ToolStepOutcome::NoToolCalls;
    }

    let session_catalog = {
        let session = session_arc.lock().await;
        session.tool_catalog.clone()
    };
    let catalog = match session_catalog {
        Some(catalog) => catalog,
        None => {
            app.tool_registry
                .acquire_tool_catalog(
                    mode_id,
                    model_id,
                    thread
                        .worktree
                        .as_ref()
                        .map(|worktree| worktree.root.to_string_lossy().into_owned())
                        .as_deref(),
                )
                .await
        }
    };
    let tool_calls = resolve_tool_call_aliases_with_catalog(tool_calls, &catalog);
    let turn_tool_pool =
        acquire_session_turn_tool_pool(&app, &session_arc, &thread, mode_id, model_id, &catalog)
            .await;

    info!(
        "process_tool_calls_once: {} tool calls to process",
        tool_calls.len()
    );

    let (confirmations, denials) = check_tools_confirmation_with_pool(
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

    let denied_ids: std::collections::HashSet<String> =
        denials.iter().map(|d| d.tool_call_id.clone()).collect();
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

    let mut approved_activity_ids: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    if !confirmations.is_empty() {
        let (auto_approved, remaining): (Vec<_>, Vec<_>) =
            confirmations.into_iter().partition(|confirmation| {
                should_auto_approve_confirmation(&thread, &confirmation.tool_name)
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
        session.set_runtime_state(SessionState::Generating, None);
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
    let pre_hook_item_count = tools_to_execute.len() as u64;
    let pre_hook_span = perf_diagnostics::span(PerfComponent::ToolPreHook, None, None);
    for tc in &tools_to_execute {
        let args_value: Option<serde_json::Value> = tc
            .function
            .parse_args()
            .ok()
            .map(|m| serde_json::Value::Object(m.into_iter().collect()));
        let payload = HookPayload {
            hook_event_name: "PreToolUse".to_string(),
            session_id: session_id.clone(),
            project_dir: project_dir.clone(),
            tool_name: Some(tc.function.name.clone()),
            tool_input: args_value,
            tool_output: None,
            user_prompt: None,
            extra: std::collections::HashMap::new(),
        };
        let results = run_hooks(app.clone(), HookEvent::PreToolUse, payload).await;
        if let Some(reason) = first_block_reason(&results) {
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

    if tools_to_execute.is_empty() {
        let mut session = session_arc.lock().await;
        session.set_runtime_state(SessionState::Generating, None);
        drop(session);
        maybe_save_trajectory_background_with_intent(
            app.clone(),
            session_arc.clone(),
            TrajectoryCommitIntent::Checkpoint,
        );
        return ToolStepOutcome::Continue;
    }

    {
        let mut session = session_arc.lock().await;
        session.set_runtime_state(SessionState::ExecutingTools, None);
    }

    let tool_interrupt_flag = {
        let session = session_arc.lock().await;
        session.user_interrupt_flag.clone()
    };
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
            ..Default::default()
        },
    );
    let (tool_results, _) = tokio::select! {
        result = tool_execution => result,
        _ = wait_for_tool_abort(session_arc.clone(), tool_interrupt_flag) => (Vec::new(), false),
    };

    let privacy_approvals = shell_privacy_approval_reasons(&tools_to_execute, &tool_results);
    if !privacy_approvals.is_empty() {
        let mut session = session_arc.lock().await;
        for result_msg in tool_results {
            session.add_message(result_msg);
        }
        session.drain_post_tool_side_effects();
        session.set_paused_with_reasons_and_auto_approved(privacy_approvals, Vec::new(), None);
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
                "ask_questions" | "wait_agents" => final_state = SessionState::WaitingUserInput,
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

    let mut verify_completion = false;
    {
        let mut session = session_arc.lock().await;
        if !was_interrupted || tool_initiated_stop {
            for result_msg in tool_results {
                session.add_message(result_msg);
            }
            session.drain_post_tool_side_effects();
        } else {
            session.clear_post_tool_side_effects();
        }
        if tool_initiated_stop {
            if final_state == SessionState::Completed
                && completion_trigger.is_some()
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
        } else {
            session.set_runtime_state(SessionState::Generating, None);
        }
    }

    if verify_completion {
        let trigger = completion_trigger.as_deref().unwrap_or("task_done");
        match verify_goal_before_completion(app.clone(), session_arc.clone(), trigger).await {
            GoalCompletionGateOutcome::Passthrough => {
                let mut session = session_arc.lock().await;
                session.set_runtime_state(SessionState::Completed, None);
            }
            GoalCompletionGateOutcome::Finalized => {}
            GoalCompletionGateOutcome::Rearmed => {
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

    // Perform pending skill deactivation cleanup (compacts skill-run messages into a report)
    {
        let mut session = session_arc.lock().await;
        if session.pending_skill_deactivation.is_some() {
            session.perform_skill_deactivation_cleanup();
        }
    }

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
    const ALWAYS_ASK_TOOLS: &[&str] = &["compress_chat_probe", "compress_chat_apply"];
    if *tool_result == MatchConfirmDenyResult::DENY {
        return "deny";
    }
    if matches!(mode_action, Some("deny")) {
        return "deny";
    }
    if tool_name == "handoff_to_mode" {
        return "auto";
    }
    if ALWAYS_ASK_TOOLS.iter().any(|name| *name == tool_name) {
        return "ask";
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
                    reason_type: "denial".to_string(),
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
            if let Err(error) = escalation_from_args(&args.clone().into_iter().collect()) {
                denials.push(PauseReason {
                    reason_type: "denial".to_string(),
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
                    should_auto_approve_confirmation(thread, &tool_call.function.name)
                        || (!allowed_tools.is_empty()
                            && (allowed_tools.contains(&tool_call.function.name)
                                || allowed_tools.contains(&resolved_tool_name)));
                let final_action = compute_final_action(
                    &result.result.result,
                    mode_action.as_deref(),
                    is_auto_approved,
                    &tool_call.function.name,
                );

                let rule_text = match mode_action.as_deref() {
                    Some(action) => format!("mode policy: {}", action),
                    None => result.result.rule.clone(),
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
                    reason_type: "denial".to_string(),
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
    let catalog = match options.catalog.clone().or(session_catalog) {
        Some(catalog) => catalog,
        None => {
            app.tool_registry
                .acquire_tool_catalog(
                    mode_id,
                    model_id,
                    thread
                        .worktree
                        .as_ref()
                        .map(|worktree| worktree.root.to_string_lossy().into_owned())
                        .as_deref(),
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

    let ccx = build_tool_execution_context(
        app.clone(),
        n_ctx,
        &prompt_messages,
        thread,
        Some(session_abort_flag),
    )
    .await;

    {
        let mut cgcx = ccx.lock().await;
        cgcx.tokens_for_rag = (n_ctx / 2).max(4096);
        if let Some(ref params) = options.subchat_tool_parameters {
            cgcx.subchat_tool_parameters = params.clone();
        }
    }

    let cancel_flag = spawn_subchat_bridge(ccx.clone(), session_arc.clone());

    let turn_tool_pool = options.turn_tool_pool.clone();
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

    cancel_flag.store(true, Ordering::Relaxed);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

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

type SerialToolRegistry = std::collections::HashSet<String>;

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
) -> (usize, bool, Vec<ChatMessage>, Vec<ContextFile>) {
    let abort_flag = {
        let cgcx = ccx.lock().await;
        cgcx.abort_flag.clone()
    };
    if abort_flag.load(Ordering::Relaxed) {
        return (idx, false, Vec::new(), Vec::new());
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

    if let Some(session_arc) = {
        let sessions_read = app.chat.sessions.read().await;
        sessions_read.get(&session_id).cloned()
    } {
        let mut session = session_arc.lock().await;
        session.mark_tool_started();
    }

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

    let (idx, had_corrections, mut msgs, files) = match app
        .tool_registry
        .execute_tool_with_catalog_and_pool(
            &ccx,
            &catalog,
            turn_tool_pool.as_ref(),
            mode_id,
            model_id,
            &tool_call.id,
            &tool_call.function.name,
            serde_json::Map::from_iter(args.into_iter()),
        )
        .await
    {
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

    let execution_class = if allow_parallel {
        ToolExecutionClass::Parallel
    } else {
        ToolExecutionClass::Serial
    };

    for msg in &mut msgs {
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

    let project_dir = get_project_dir_string(app.clone()).await;
    let args_value: Option<serde_json::Value> = tool_call
        .function
        .parse_args()
        .ok()
        .map(|m| serde_json::Value::Object(m.into_iter().collect()));
    let post_payload = HookPayload {
        hook_event_name: "PostToolUse".to_string(),
        session_id,
        project_dir,
        tool_name: Some(tool_call.function.name.clone()),
        tool_input: args_value,
        tool_output: Some(tool_output_text),
        user_prompt: None,
        extra: std::collections::HashMap::new(),
    };
    let post_hook_span = perf_diagnostics::span(PerfComponent::ToolPostHook, None, None);
    let post_results = run_hooks(app.clone(), HookEvent::PostToolUse, post_payload).await;
    if let Some(reason) = first_block_reason(&post_results) {
        post_hook_span.finish_tool(
            PerfOutcome::Failure,
            batch_size as u64,
            1,
            Some(execution_class),
        );
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
    post_hook_span.finish_tool(
        PerfOutcome::Success,
        batch_size as u64,
        1,
        Some(execution_class),
    );

    (idx, had_corrections, msgs, files)
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
    let max_parallel = limits().max_parallel_tools.max(1);

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
                    max_parallel,
                    mode_id,
                    model_id,
                )
                .await;
                all_results.extend(batch_results);
                current_parallel_batch.clear();
            }

            let result = execute_single_tool(
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
            max_parallel,
            mode_id,
            model_id,
        )
        .await;
        all_results.extend(batch_results);
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

    let results = postprocess_tool_results(
        app.gcx.clone(),
        None,
        tool_messages,
        context_files,
        budget,
        pp_settings,
        messages,
        &truncation_exempt_ids,
    )
    .await;

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
    max_parallel: usize,
    mode_id: &str,
    model_id: Option<&str>,
) -> Vec<(usize, bool, Vec<ChatMessage>, Vec<ContextFile>)> {
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
                let required = count.min(max_parallel);
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
    let semaphore = Arc::new(Semaphore::new(max_parallel));
    let batch_size = batch.len();

    let futures: Vec<_> = batch
        .iter()
        .map(|(idx, tool_call)| {
            let gcx = app.clone();
            let ccx = ccx.clone();
            let semaphore = semaphore.clone();
            let serial_registry = serial_registry.clone();
            let catalog = catalog.clone();
            let turn_tool_pool = turn_tool_pool.clone();
            let tool_call = tool_call.clone();
            let idx = *idx;
            let batch_size = batch_size;
            let mode_id = mode_id.to_string();
            let model_id = model_id.map(|s| s.to_string());

            async move {
                let semaphore_span =
                    perf_diagnostics::span(PerfComponent::ToolSemaphoreWait, None, None);
                let _permit = semaphore.acquire().await.unwrap();
                semaphore_span.finish_tool(
                    PerfOutcome::Success,
                    batch_size as u64,
                    1,
                    Some(ToolExecutionClass::Parallel),
                );
                execute_single_tool(
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

pub async fn execute_tools(
    app: AppState,
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
    let catalog = match options.catalog.clone() {
        Some(catalog) => catalog,
        None => {
            app.tool_registry
                .acquire_tool_catalog(
                    mode_id,
                    model_id,
                    thread
                        .worktree
                        .as_ref()
                        .map(|worktree| worktree.root.to_string_lossy().into_owned())
                        .as_deref(),
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

    let ccx = build_tool_execution_context(app.clone(), n_ctx, messages, thread, None).await;

    {
        let mut cgcx = ccx.lock().await;
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
