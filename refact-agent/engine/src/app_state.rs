use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock};

use async_trait::async_trait;
use axum::extract::FromRef;
use refact_buddy_core::snapshot::BuddySnapshot;
use refact_buddy_core::types::{BuddyRuntimeEvent, BuddySuggestion};
use refact_buddy_core::user_action::UserAction;
use refact_chat_api::ChatMessage;
use refact_runtime_api::{
    ActivitySink, BuddyEventSink, ToolConfirmationCheck, ToolExecutionResult, ToolPolicyInfo,
    ToolRegistry, ToolRegistryIndex,
};
use tokio::sync::{Mutex as AMutex, RwLock as ARwLock};

use crate::ast::ast_indexer_thread::AstIndexService;
use crate::agents::registry::BackgroundAgentRegistry;
use crate::buddy::actor::BuddyService;
use crate::buddy::events::BuddyEvent;
use crate::buddy::user_activity::UserActivityRing;
use crate::chat::types::EnqueueCommandOutcome;
use crate::chat::trajectories::{self, TrajectoryEvent};
use crate::chat::{self, process_command_queue, SessionsMap};
use crate::completion_cache::CompletionCache;
use crate::exec::ExecRegistry;
use crate::files_blocklist::IndexingEverywhere;
use crate::files_in_workspace::DocumentsState;
use crate::global_context::{AtCommandsPreviewCache, CommandLine, SharedGlobalContext};
use crate::http::routers::v1::code_lens::CodeLensCache;
use crate::http::routers::v1::sidebar::NotificationEvent;
use crate::integrations::browser_runtime::BrowserRuntime;
use crate::integrations::sessions::IntegrationSession;
use crate::knowledge_index::KnowledgeIndex;
use crate::privacy::PrivacySettings;
use crate::providers::ProviderRegistry;
use crate::stats::event::LlmCallEvent;
use crate::tasks::events::TaskEventEnvelope;
use crate::voice::SharedVoiceService;
use crate::yaml_configs::customization_registry::RegistryCacheManager;
pub use refact_caps_core::caps_state::CapsState;
pub use refact_core::tokenizer_state::TokenizerState;
use refact_core::vecdb_types::VecdbSearch;
use refact_runtime_api::{
    ChatSessionFacade, ChatSessionSnapshot, ChatSessionUpdate, CreateSessionRequest,
};

#[derive(Clone)]
pub struct RuntimeServices {
    pub shutdown_flag: Arc<AtomicBool>,
    pub cmdline: Arc<CommandLine>,
    pub http_client: reqwest::Client,
    pub ask_shutdown_sender: Arc<StdMutex<std::sync::mpsc::Sender<String>>>,
    pub exec_registry: Arc<ExecRegistry>,
}

#[derive(Clone)]
pub struct PathServices {
    pub cache_dir: PathBuf,
    pub config_dir: PathBuf,
    pub app_searchable_id: String,
}

#[derive(Clone)]
pub struct ModelServices {
    pub caps: Arc<ARwLock<CapsState>>,
    pub tokenizers: Arc<StdRwLock<TokenizerState>>,
    pub providers: Arc<ARwLock<ProviderRegistry>>,
    pub llm_stats_sender: Option<tokio::sync::mpsc::Sender<LlmCallEvent>>,
}

#[derive(Clone)]
pub struct WorkspaceServices {
    pub documents_state: DocumentsState,
    pub privacy_settings: Arc<PrivacySettings>,
    pub indexing_everywhere: Arc<IndexingEverywhere>,
    pub completions_cache: Arc<StdRwLock<CompletionCache>>,
    pub vec_db: Arc<AMutex<Option<Arc<dyn VecdbSearch>>>>,
    pub vec_db_error: Arc<StdMutex<String>>,
    pub ast_service: Option<Arc<AMutex<AstIndexService>>>,
    pub knowledge_index: Arc<AMutex<KnowledgeIndex>>,
    pub at_commands_preview_cache: Arc<AMutex<AtCommandsPreviewCache>>,
}

#[derive(Clone)]
pub struct ChatServices {
    pub sessions: SessionsMap,
    pub facade: Arc<dyn ChatSessionFacade>,
    pub trajectory_events_tx: tokio::sync::broadcast::Sender<TrajectoryEvent>,
    pub workspace_changed_tx: tokio::sync::broadcast::Sender<()>,
    pub task_events_tx: tokio::sync::broadcast::Sender<TaskEventEnvelope>,
    pub task_events_seq: Arc<AtomicU64>,
    pub notification_events_tx: tokio::sync::broadcast::Sender<NotificationEvent>,
    pub voice_service: SharedVoiceService,
}

#[derive(Clone)]
pub struct BuddyServices {
    pub buddy: Arc<AMutex<Option<BuddyService>>>,
    pub buddy_events_tx: tokio::sync::broadcast::Sender<BuddyEvent>,
    pub user_activity: Arc<AMutex<UserActivityRing>>,
}

#[derive(Clone)]
pub struct IntegrationServices {
    pub integration_sessions:
        Arc<AMutex<HashMap<String, Arc<AMutex<Box<dyn IntegrationSession>>>>>>,
    pub browser_runtimes: Arc<AMutex<HashMap<String, Arc<AMutex<BrowserRuntime>>>>>,
    pub ext_cache_generation: Arc<AtomicU64>,
    pub project_registry_cache: Arc<StdRwLock<RegistryCacheManager>>,
    pub codelens_cache: Arc<AMutex<CodeLensCache>>,
    pub init_shadow_repos_lock: Arc<AMutex<bool>>,
    pub git_operations_abort_flag: Arc<AtomicBool>,
}

#[derive(Clone)]
pub struct EngineChatSessionFacade {
    gcx: SharedGlobalContext,
}

impl EngineChatSessionFacade {
    pub fn new(gcx: SharedGlobalContext) -> Self {
        Self { gcx }
    }

    async fn enqueue_command(
        &self,
        chat_id: &str,
        command: refact_chat_api::ChatCommand,
        priority: bool,
    ) -> Result<(), String> {
        let app = AppState::from_gcx(self.gcx.clone()).await;
        let session_arc = chat::get_or_create_session_with_trajectory(
            app.clone(),
            &self.gcx.chat_sessions,
            chat_id,
        )
        .await;
        let mut session = session_arc.lock().await;
        let request = refact_chat_api::CommandRequest {
            client_request_id: uuid::Uuid::new_v4().to_string(),
            priority,
            command,
        };
        let enqueue_outcome = session.enqueue_command(request);
        if enqueue_outcome == EnqueueCommandOutcome::Full {
            return Err("chat command queue is full".to_string());
        }
        let processor_running = session.queue_processor_running.clone();
        let queue_notify = session.queue_notify.clone();
        drop(session);
        if !processor_running.swap(true, Ordering::SeqCst) {
            tokio::spawn(process_command_queue(app, session_arc, processor_running));
        } else {
            queue_notify.notify_one();
        }
        Ok(())
    }
}

#[async_trait]
impl ChatSessionFacade for EngineChatSessionFacade {
    async fn session_snapshot(&self, chat_id: &str) -> Result<ChatSessionSnapshot, String> {
        let app = AppState::from_gcx(self.gcx.clone()).await;
        let session_arc =
            chat::get_or_create_session_with_trajectory(app, &self.gcx.chat_sessions, chat_id)
                .await;
        let session = session_arc.lock().await;
        Ok(ChatSessionSnapshot {
            messages: session.messages.clone(),
            thread: session.thread.clone(),
            session_state: session.runtime.state,
            pause_reasons: session.runtime.pause_reasons.clone(),
            goal: session.goal.clone(),
        })
    }

    async fn update_session(&self, chat_id: &str, update: ChatSessionUpdate) -> Result<(), String> {
        let app = AppState::from_gcx(self.gcx.clone()).await;
        let session_arc =
            chat::get_or_create_session_with_trajectory(app, &self.gcx.chat_sessions, chat_id)
                .await;
        let background_agents: Vec<_> = self
            .gcx
            .agents
            .list_for_parent(chat_id, crate::agents::types::AgentListFilter::default())
            .await
            .iter()
            .map(crate::agents::types::BackgroundAgentSummary::from)
            .collect();
        let mut session = session_arc.lock().await;
        session.replace_messages(update.messages);
        session.thread.previous_response_id = update.previous_response_id;
        session.upsert_background_agents(background_agents);
        let snapshot = session.snapshot();
        session.emit(snapshot);
        Ok(())
    }

    async fn create_session(&self, request: CreateSessionRequest) -> Result<(), String> {
        let app = AppState::from_gcx(self.gcx.clone()).await;
        let session_arc = chat::get_or_create_session_with_trajectory(
            app,
            &self.gcx.chat_sessions,
            &request.chat_id,
        )
        .await;
        let mut session = session_arc.lock().await;
        session.thread = request.thread;
        for message in request.messages {
            session.add_message(message);
        }
        session.increment_version();
        Ok(())
    }

    async fn push_command(
        &self,
        chat_id: &str,
        command: refact_chat_api::ChatCommand,
    ) -> Result<(), String> {
        self.enqueue_command(chat_id, command, false).await
    }

    async fn push_priority_command(
        &self,
        chat_id: &str,
        command: refact_chat_api::ChatCommand,
    ) -> Result<(), String> {
        self.enqueue_command(chat_id, command, true).await
    }

    async fn session_state(
        &self,
        chat_id: &str,
    ) -> Result<Option<refact_runtime_api::SessionState>, String> {
        let session_arc = {
            let sessions = self.gcx.chat_sessions.read().await;
            sessions.get(chat_id).cloned()
        };
        match session_arc {
            Some(session_arc) => Ok(Some(session_arc.lock().await.runtime.state)),
            None => Ok(None),
        }
    }

    async fn maybe_save_session(&self, chat_id: &str) -> Result<(), String> {
        let session_arc = {
            let sessions = self.gcx.chat_sessions.read().await;
            sessions.get(chat_id).cloned()
        };
        if let Some(session_arc) = session_arc {
            trajectories::maybe_save_trajectory(
                AppState::from_gcx(self.gcx.clone()).await,
                session_arc,
            )
            .await;
        }
        Ok(())
    }

    async fn save_trajectory_snapshot(
        &self,
        snapshot: refact_runtime_api::RuntimeTrajectorySnapshot,
    ) -> Result<(), String> {
        trajectories::save_trajectory_snapshot(self.gcx.clone(), snapshot).await
    }
}

#[derive(Clone)]
pub struct AppState {
    pub gcx: SharedGlobalContext,
    pub runtime: RuntimeServices,
    pub paths: PathServices,
    pub model: ModelServices,
    pub workspace: WorkspaceServices,
    pub chat: ChatServices,
    pub buddy: BuddyServices,
    pub integrations: IntegrationServices,
    pub activity_sink: Arc<dyn ActivitySink>,
    pub buddy_event_sink: Arc<dyn BuddyEventSink>,
    pub tool_registry: Arc<dyn ToolRegistry>,
    pub agents: Arc<BackgroundAgentRegistry>,
}

pub struct AppActivitySink {
    user_activity: Arc<AMutex<UserActivityRing>>,
}

impl AppActivitySink {
    pub fn new(user_activity: Arc<AMutex<UserActivityRing>>) -> Self {
        Self { user_activity }
    }
}

#[async_trait]
impl ActivitySink for AppActivitySink {
    async fn record_user_action(&self, action: UserAction) {
        if let Ok(mut ring) = self.user_activity.try_lock() {
            ring.push(action);
        }
    }
}

pub struct AppToolRegistry {
    gcx: SharedGlobalContext,
}

impl AppToolRegistry {
    pub fn new(gcx: SharedGlobalContext) -> Self {
        Self { gcx }
    }
}

#[async_trait]
impl ToolRegistry for AppToolRegistry {
    async fn get_tools_for_mode(
        &self,
        mode: &str,
        model_id: Option<&str>,
    ) -> Vec<refact_tool_api::ToolDesc> {
        crate::tools::tools_list::apply_mcp_lazy_filter(
            crate::tools::tools_list::get_tools_for_mode(self.gcx.clone(), mode, model_id).await,
        )
        .tools
        .into_iter()
        .map(|tool| tool.tool_description())
        .collect()
    }

    async fn get_tools_index_for_mode(
        &self,
        mode: &str,
        model_id: Option<&str>,
    ) -> ToolRegistryIndex {
        let tools = crate::tools::tools_list::apply_mcp_lazy_filter(
            crate::tools::tools_list::get_tools_for_mode(self.gcx.clone(), mode, model_id).await,
        );
        ToolRegistryIndex {
            tools: tools
                .tools
                .into_iter()
                .map(|tool| tool.tool_description())
                .collect(),
            mcp_lazy_mode: tools.mcp_lazy_mode,
            mcp_total_count: tools.mcp_total_count,
            mcp_tool_index: tools.mcp_tool_index,
        }
    }

    async fn check_tool_confirmation(
        &self,
        ccx: &(dyn std::any::Any + Send + Sync),
        mode: &str,
        model_id: Option<&str>,
        tool_name: &str,
        args: serde_json::Map<String, serde_json::Value>,
    ) -> Option<Result<ToolConfirmationCheck, String>> {
        let ccx = match ccx
            .downcast_ref::<Arc<AMutex<crate::at_commands::at_commands::AtCommandsContext>>>()
        {
            Some(ccx) => ccx.clone(),
            None => {
                return Some(Err(
                    "invalid AtCommandsContext passed to ToolRegistry".to_string()
                ))
            }
        };
        let args: HashMap<String, serde_json::Value> = args.into_iter().collect();
        let raw_tools =
            crate::tools::tools_list::get_tools_for_mode(self.gcx.clone(), mode, model_id).await;
        let tools = crate::tools::tools_list::apply_mcp_lazy_filter(raw_tools).tools;
        let resolved = crate::llm::adapters::claude_code_compat::cc_resolve_tool_name(tool_name);
        for tool in tools {
            let desc = tool.tool_description();
            if desc.name == tool_name || desc.name == resolved.as_str() {
                let integr_config_path = tool.has_config_path();
                return Some(
                    tool.match_against_confirm_deny(ccx, &args)
                        .await
                        .map(|result| ToolConfirmationCheck {
                            tool_name: desc.name,
                            result,
                            integr_config_path,
                        }),
                );
            }
        }
        None
    }

    async fn get_tool_policy_info(
        &self,
        mode: &str,
        model_id: Option<&str>,
    ) -> Vec<ToolPolicyInfo> {
        let raw_tools =
            crate::tools::tools_list::get_tools_for_mode(self.gcx.clone(), mode, model_id).await;
        crate::tools::tools_list::apply_mcp_lazy_filter(raw_tools)
            .tools
            .into_iter()
            .map(|tool| {
                let desc = tool.tool_description();
                let config_override = if !desc.source.config_path.is_empty() {
                    tool.config().ok().and_then(|c| c.allow_parallel)
                } else {
                    None
                };
                let effective_allow_parallel = if desc.allow_parallel {
                    config_override.unwrap_or(true)
                } else {
                    false
                };
                ToolPolicyInfo {
                    name: desc.name,
                    effective_allow_parallel,
                }
            })
            .collect()
    }

    async fn execute_tool(
        &self,
        ccx: &(dyn std::any::Any + Send + Sync),
        mode: &str,
        model_id: Option<&str>,
        tool_call_id: &str,
        tool_name: &str,
        args: serde_json::Map<String, serde_json::Value>,
    ) -> Result<Option<ToolExecutionResult>, String> {
        let ccx = ccx
            .downcast_ref::<Arc<AMutex<crate::at_commands::at_commands::AtCommandsContext>>>()
            .ok_or_else(|| "invalid AtCommandsContext passed to ToolRegistry".to_string())?
            .clone();
        let gcx = {
            let cgcx = ccx.lock().await;
            cgcx.app.gcx.clone()
        };
        let raw_tools =
            crate::tools::tools_list::get_tools_for_mode(gcx.clone(), mode, model_id).await;
        let tools = crate::tools::tools_list::apply_mcp_lazy_filter(raw_tools).tools;
        let resolved = crate::llm::adapters::claude_code_compat::cc_resolve_tool_name(tool_name);
        let args: HashMap<String, serde_json::Value> = args.into_iter().collect();
        for mut tool in tools {
            let name = tool.tool_description().name;
            if name == tool_name || name == resolved.as_str() {
                {
                    let mut cgcx = ccx.lock().await;
                    cgcx.app = AppState::from_gcx(gcx.clone()).await;
                }
                let result = tool
                    .tool_execute(ccx, &tool_call_id.to_string(), &args)
                    .await?;
                let mut messages = Vec::new();
                let mut context_files = Vec::new();
                for item in result.1 {
                    match item {
                        crate::call_validation::ContextEnum::ChatMessage(message) => {
                            messages.push(message)
                        }
                        crate::call_validation::ContextEnum::ContextFile(file) => {
                            context_files.push(file)
                        }
                    }
                }
                return Ok(Some(ToolExecutionResult {
                    had_corrections: result.0,
                    messages,
                    context_files,
                }));
            }
        }
        Ok(None)
    }

    async fn load_task_memories(&self, task_id: &str) -> Result<Vec<(PathBuf, String)>, String> {
        crate::tools::tool_task_memory::load_task_memories(self.gcx.clone(), task_id).await
    }
}

pub struct AppBuddyEventSink {
    gcx: SharedGlobalContext,
    buddy: Arc<AMutex<Option<BuddyService>>>,
}

impl AppBuddyEventSink {
    pub fn new(gcx: SharedGlobalContext, buddy: Arc<AMutex<Option<BuddyService>>>) -> Self {
        Self { gcx, buddy }
    }
}

#[async_trait]
impl BuddyEventSink for AppBuddyEventSink {
    async fn enqueue_event(&self, event: BuddyRuntimeEvent) {
        let buddy_arc = self.buddy.clone();
        let mut lock = buddy_arc.lock().await;
        if let Some(svc) = lock.as_mut() {
            svc.enqueue_runtime_event(event);
        }
    }

    async fn complete_event(&self, dedupe_key: &str, status: &str) {
        let buddy_arc = self.buddy.clone();
        let mut lock = buddy_arc.lock().await;
        if let Some(svc) = lock.as_mut() {
            svc.complete_runtime_event(dedupe_key, status);
        }
    }

    async fn snapshot(&self) -> Option<BuddySnapshot> {
        let buddy_arc = self.buddy.clone();
        let lock = buddy_arc.lock().await;
        lock.as_ref().map(|svc| svc.snapshot())
    }

    async fn apply_chat_completion(&self, event: BuddyRuntimeEvent, xp: u64, mood: String) {
        let buddy_arc = self.buddy.clone();
        let mut lock = buddy_arc.lock().await;
        let Some(svc) = lock.as_mut() else { return };
        svc.enqueue_runtime_event(event);
        if xp > 0 {
            svc.grant_xp(xp);
        }
        svc.state.semantic.mood = mood;
        svc.dirty = true;
        let _ = svc.events_tx.send(BuddyEvent::StateUpdated {
            state: svc.state.clone(),
        });
    }

    async fn report_error(
        &self,
        error_type: &str,
        error_msg: &str,
        source: Option<&str>,
        chat_id: Option<&str>,
    ) {
        let buddy_arc = self.buddy.clone();
        let mut lock = buddy_arc.lock().await;
        if let Some(svc) = lock.as_mut() {
            svc.report_error(error_type, error_msg, source, chat_id);
        }
    }

    async fn report_error_with_model(
        &self,
        error_type: &str,
        error_msg: &str,
        source: Option<&str>,
        chat_id: Option<&str>,
        model_id: Option<&str>,
    ) {
        let buddy_arc = self.buddy.clone();
        let mut lock = buddy_arc.lock().await;
        if let Some(svc) = lock.as_mut() {
            svc.report_error_with_model(error_type, error_msg, source, chat_id, model_id);
        }
    }

    async fn mark_chat_error(&self, event: BuddyRuntimeEvent) {
        let buddy_arc = self.buddy.clone();
        let mut lock = buddy_arc.lock().await;
        if let Some(svc) = lock.as_mut() {
            svc.enqueue_runtime_event(event);
            svc.state.semantic.mood = "worried".to_string();
            svc.dirty = true;
            let _ = svc.events_tx.send(BuddyEvent::StateUpdated {
                state: svc.state.clone(),
            });
        }
    }

    async fn maybe_add_suggestion(&self, suggestion: BuddySuggestion) {
        let buddy_arc = self.buddy.clone();
        let mut lock = buddy_arc.lock().await;
        if let Some(svc) = lock.as_mut() {
            svc.maybe_add_suggestion(suggestion);
        }
    }

    async fn render_runtime_event_fast(
        &self,
        workflow_id: &str,
        workflow_summary: &str,
        status: &str,
    ) -> Option<(String, Option<String>)> {
        let app = AppState::from_gcx(self.gcx.clone()).await;
        let snapshot = self.snapshot().await?;
        let pulse_one_liner = format!(
            "{} pending ops, {} recent stuck task alerts",
            snapshot.pulse.memory.pending_ops,
            snapshot.pulse.tasks.recent_stuck_alert_count_1h()
        );
        let voice_ctx = crate::buddy::voice_service::VoiceCtx {
            persona: &snapshot.state.personality,
            identity_name: snapshot.state.identity.name.as_str(),
            pulse_one_liner,
            workflow_id: Some(workflow_id),
            workflow_summary: Some(workflow_summary),
        };
        Some(
            crate::buddy::voice_service::voice_service()
                .await
                .render_runtime_event_fast(app, voice_ctx, status)
                .await,
        )
    }

    async fn build_pulse_message(&self) -> Option<ChatMessage> {
        crate::buddy::pulse_inject::build_buddy_pulse_message(
            AppState::from_gcx(self.gcx.clone()).await,
        )
        .await
    }
}

impl AppState {
    pub async fn from_gcx(gcx: SharedGlobalContext) -> Self {
        gcx.app_state(gcx.clone())
    }
}

impl FromRef<AppState> for SharedGlobalContext {
    fn from_ref(app: &AppState) -> Self {
        app.gcx.clone()
    }
}

impl From<AppState> for SharedGlobalContext {
    fn from(app: AppState) -> Self {
        app.gcx.clone()
    }
}

impl From<&AppState> for SharedGlobalContext {
    fn from(app: &AppState) -> Self {
        app.gcx.clone()
    }
}

impl FromRef<AppState> for RuntimeServices {
    fn from_ref(app: &AppState) -> Self {
        app.runtime.clone()
    }
}

impl FromRef<AppState> for PathServices {
    fn from_ref(app: &AppState) -> Self {
        app.paths.clone()
    }
}

impl FromRef<AppState> for ModelServices {
    fn from_ref(app: &AppState) -> Self {
        app.model.clone()
    }
}

impl FromRef<AppState> for WorkspaceServices {
    fn from_ref(app: &AppState) -> Self {
        app.workspace.clone()
    }
}

impl FromRef<AppState> for ChatServices {
    fn from_ref(app: &AppState) -> Self {
        app.chat.clone()
    }
}

impl FromRef<AppState> for BuddyServices {
    fn from_ref(app: &AppState) -> Self {
        app.buddy.clone()
    }
}

impl FromRef<AppState> for IntegrationServices {
    fn from_ref(app: &AppState) -> Self {
        app.integrations.clone()
    }
}
