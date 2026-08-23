use std::collections::HashMap;
use std::env;
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

use crate::agents::registry::BackgroundAgentRegistry;
use crate::buddy::actor::BuddyService;
use crate::buddy::events::BuddyEvent;
use crate::buddy::user_activity::UserActivityRing;
use crate::chat::perf_diagnostics::{self, PerfComponent, PerfOutcome};
use crate::chat::trajectory_index::TrajectoryIndexCoordinator;
use crate::chat::types::EnqueueCommandOutcome;
use crate::chat::trajectories::{self, TrajectoryEvent};
use crate::chat::{self, process_command_queue, SessionsMap};
use crate::completion_cache::CompletionCache;
use crate::exec::ExecRegistry;
use crate::files_blocklist::IndexingEverywhere;
use crate::files_in_workspace::DocumentsState;
use crate::global_context::{AtCommandsPreviewCache, CommandLine, SharedGlobalContext};
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
    pub knowledge_index: Arc<AMutex<KnowledgeIndex>>,
    pub at_commands_preview_cache: Arc<AMutex<AtCommandsPreviewCache>>,
}

#[derive(Clone)]
pub struct ChatServices {
    pub sessions: SessionsMap,
    pub facade: Arc<dyn ChatSessionFacade>,
    pub trajectory_index_coordinator: Arc<TrajectoryIndexCoordinator>,
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
            trajectories::maybe_save_trajectory_with_intent(
                AppState::from_gcx(self.gcx.clone()).await,
                session_arc,
                crate::chat::types::TrajectoryCommitIntent::Required,
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
    #[cfg(any(test, feature = "bench"))]
    fixture_tool_factory: Option<FixtureToolFactory>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ToolCatalogKey {
    workspace_scope: String,
    execution_scope: Option<String>,
    mode: String,
    model: Option<String>,
    customization_generation: u64,
    integration_generation: u64,
    mcp_generation: u64,
    privacy_generation: u64,
    capability_generation: u64,
    extension_generation: u64,
}

#[derive(Clone)]
pub struct ToolCatalogSnapshot {
    pub index: ToolRegistryIndex,
    pub policy: Vec<ToolPolicyInfo>,
}

const TOOL_CATALOG_SNAPSHOT_CACHE_LIMIT: usize = 128;

#[derive(Default)]
pub struct ToolCatalogCache {
    snapshots: ARwLock<HashMap<ToolCatalogKey, Arc<ToolCatalogSnapshot>>>,
    build_locks: AMutex<HashMap<ToolCatalogKey, Arc<AMutex<()>>>>,
}

impl ToolCatalogCache {
    async fn acquire_build_lock(&self, key: &ToolCatalogKey) -> Arc<AMutex<()>> {
        let mut locks = self.build_locks.lock().await;
        locks
            .entry(key.clone())
            .or_insert_with(|| Arc::new(AMutex::new(())))
            .clone()
    }

    async fn get(&self, key: &ToolCatalogKey) -> Option<Arc<ToolCatalogSnapshot>> {
        self.snapshots.read().await.get(key).cloned()
    }

    async fn insert(&self, key: ToolCatalogKey, snapshot: Arc<ToolCatalogSnapshot>) {
        let mut snapshots = self.snapshots.write().await;
        if snapshots.len() >= TOOL_CATALOG_SNAPSHOT_CACHE_LIMIT && !snapshots.contains_key(&key) {
            if let Some(evicted) = snapshots.keys().next().cloned() {
                snapshots.remove(&evicted);
            }
        }
        snapshots.insert(key, snapshot);
    }

    async fn release_build_lock(&self, key: &ToolCatalogKey) {
        self.build_locks.lock().await.remove(key);
    }

    #[cfg(test)]
    async fn snapshot_count(&self) -> usize {
        self.snapshots.read().await.len()
    }

    #[cfg(test)]
    async fn build_lock_count(&self) -> usize {
        self.build_locks.lock().await.len()
    }
}

#[cfg(any(test, feature = "bench"))]
pub type FixtureToolFactory =
    Arc<dyn Fn() -> Vec<Box<dyn crate::tools::tools_description::Tool + Send>> + Send + Sync>;

impl AppToolRegistry {
    pub fn new(gcx: SharedGlobalContext) -> Self {
        Self {
            gcx,
            #[cfg(any(test, feature = "bench"))]
            fixture_tool_factory: None,
        }
    }

    #[cfg(any(test, feature = "bench"))]
    pub fn with_fixture_tool_factory(
        gcx: SharedGlobalContext,
        fixture_tool_factory: FixtureToolFactory,
    ) -> Self {
        Self {
            gcx,
            fixture_tool_factory: Some(fixture_tool_factory),
        }
    }

    async fn tools_for_mode(
        &self,
        gcx: SharedGlobalContext,
        mode: &str,
        model_id: Option<&str>,
    ) -> Vec<Box<dyn crate::tools::tools_description::Tool + Send>> {
        #[cfg(any(test, feature = "bench"))]
        if let Some(fixture_tool_factory) = &self.fixture_tool_factory {
            return fixture_tool_factory();
        }
        crate::tools::tools_list::get_tools_for_mode(gcx, mode, model_id).await
    }

    fn snapshot_cache_enabled() -> bool {
        Self::snapshot_cache_enabled_for(
            env::var("REFACT_TOOL_CATALOG_SNAPSHOTS")
                .ok()
                .as_deref()
                .map(str::trim),
        )
    }

    fn snapshot_cache_enabled_for(value: Option<&str>) -> bool {
        !matches!(value, Some("0") | Some("false") | Some("no") | Some("off"))
    }

    async fn catalog_key_with_scope(
        &self,
        gcx: SharedGlobalContext,
        mode: &str,
        model_id: Option<&str>,
        execution_scope: Option<String>,
    ) -> ToolCatalogKey {
        let workspace_scope = crate::files_correction::get_active_project_path(gcx.clone())
            .await
            .unwrap_or_else(|| gcx.config_dir.clone())
            .to_string_lossy()
            .to_string();
        let generations = &gcx.tool_catalog_generations;
        ToolCatalogKey {
            workspace_scope,
            execution_scope,
            mode: mode.to_string(),
            model: model_id.map(str::to_string),
            customization_generation: generations.customization.load(Ordering::Acquire),
            integration_generation: generations.integrations.load(Ordering::Acquire),
            mcp_generation: generations.mcp.load(Ordering::Acquire),
            privacy_generation: generations.privacy.load(Ordering::Acquire),
            capability_generation: generations.capabilities.load(Ordering::Acquire),
            extension_generation: gcx.ext_cache_generation.load(Ordering::Acquire),
        }
    }

    async fn build_snapshot(
        &self,
        gcx: SharedGlobalContext,
        mode: &str,
        model_id: Option<&str>,
    ) -> ToolCatalogSnapshot {
        let tools = crate::tools::tools_list::apply_mcp_lazy_filter(
            self.tools_for_mode(gcx, mode, model_id).await,
        );
        let policy = crate::tools::tools_list::catalog_policy_for_tools(&tools.tools);
        ToolCatalogSnapshot {
            index: ToolRegistryIndex {
                tools: tools
                    .tools
                    .into_iter()
                    .map(|tool| tool.tool_description())
                    .collect(),
                mcp_lazy_mode: tools.mcp_lazy_mode,
                mcp_total_count: tools.mcp_total_count,
                mcp_tool_index: tools.mcp_tool_index,
            },
            policy,
        }
    }

    async fn snapshot_for_mode(
        &self,
        gcx: SharedGlobalContext,
        mode: &str,
        model_id: Option<&str>,
    ) -> Arc<ToolCatalogSnapshot> {
        self.snapshot_for_mode_with_scope(gcx, mode, model_id, None)
            .await
    }

    async fn execution_scope_from_ccx(
        ccx: &Arc<AMutex<crate::at_commands::at_commands::AtCommandsContext>>,
    ) -> Option<String> {
        ccx.lock()
            .await
            .execution_scope
            .as_ref()
            .map(|scope| scope.effective_root().to_string_lossy().to_string())
    }

    async fn fresh_tools_for_scope(
        &self,
        gcx: SharedGlobalContext,
        mode: &str,
        model_id: Option<&str>,
        execution_scope: Option<String>,
    ) -> Vec<Box<dyn crate::tools::tools_description::Tool + Send>> {
        let snapshot = self
            .snapshot_for_mode_with_scope(gcx.clone(), mode, model_id, execution_scope)
            .await;
        let span = perf_diagnostics::span(PerfComponent::ToolCatalogBuild, None, None);
        let tools = crate::tools::tools_list::apply_mcp_lazy_filter(
            self.tools_for_mode(gcx, mode, model_id).await,
        )
        .tools;
        span.finish_tool(
            PerfOutcome::Success,
            1,
            snapshot.index.tools.len() as u64,
            None,
        );
        tools
    }

    async fn snapshot_for_mode_with_scope(
        &self,
        gcx: SharedGlobalContext,
        mode: &str,
        model_id: Option<&str>,
        execution_scope: Option<String>,
    ) -> Arc<ToolCatalogSnapshot> {
        if !Self::snapshot_cache_enabled() {
            let span = perf_diagnostics::span(PerfComponent::ToolCatalogBuild, None, None);
            let snapshot = Arc::new(self.build_snapshot(gcx, mode, model_id).await);
            span.finish_tool(
                PerfOutcome::Success,
                1,
                snapshot.index.tools.len() as u64,
                None,
            );
            return snapshot;
        }
        let key = self
            .catalog_key_with_scope(gcx.clone(), mode, model_id, execution_scope)
            .await;
        if let Some(snapshot) = gcx.tool_catalog_cache.get(&key).await {
            return snapshot;
        }
        let build_lock = gcx.tool_catalog_cache.acquire_build_lock(&key).await;
        let _build_guard = build_lock.lock().await;
        if let Some(snapshot) = gcx.tool_catalog_cache.get(&key).await {
            return snapshot;
        }
        let span = perf_diagnostics::span(PerfComponent::ToolCatalogBuild, None, None);
        let snapshot = Arc::new(self.build_snapshot(gcx.clone(), mode, model_id).await);
        span.finish_tool(
            PerfOutcome::Success,
            1,
            snapshot.index.tools.len() as u64,
            None,
        );
        gcx.tool_catalog_cache
            .insert(key.clone(), snapshot.clone())
            .await;
        gcx.tool_catalog_cache.release_build_lock(&key).await;
        snapshot
    }

    #[cfg(any(test, feature = "bench"))]
    #[allow(dead_code)]
    async fn snapshot_for_mode_for_test(
        &self,
        mode: &str,
        model_id: Option<&str>,
    ) -> Arc<ToolCatalogSnapshot> {
        self.snapshot_for_mode(self.gcx.clone(), mode, model_id)
            .await
    }

    #[cfg(any(test, feature = "bench"))]
    #[allow(dead_code)]
    async fn snapshot_for_mode_with_scope_for_test(
        &self,
        mode: &str,
        model_id: Option<&str>,
        execution_scope: Option<String>,
    ) -> Arc<ToolCatalogSnapshot> {
        self.snapshot_for_mode_with_scope(self.gcx.clone(), mode, model_id, execution_scope)
            .await
    }
}

#[async_trait]
impl ToolRegistry for AppToolRegistry {
    async fn get_tools_for_mode(
        &self,
        mode: &str,
        model_id: Option<&str>,
    ) -> Vec<refact_tool_api::ToolDesc> {
        let tools = self
            .snapshot_for_mode(self.gcx.clone(), mode, model_id)
            .await
            .index
            .tools
            .clone();
        tools
    }

    async fn get_tools_index_for_mode(
        &self,
        mode: &str,
        model_id: Option<&str>,
    ) -> ToolRegistryIndex {
        let index = self
            .snapshot_for_mode(self.gcx.clone(), mode, model_id)
            .await
            .index
            .clone();
        index
    }

    async fn get_tools_index_for_mode_and_scope(
        &self,
        mode: &str,
        model_id: Option<&str>,
        execution_scope: Option<&str>,
    ) -> ToolRegistryIndex {
        self.snapshot_for_mode_with_scope(
            self.gcx.clone(),
            mode,
            model_id,
            execution_scope.map(str::to_string),
        )
        .await
        .index
        .clone()
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
        let execution_scope = Self::execution_scope_from_ccx(&ccx).await;
        let tools = self
            .fresh_tools_for_scope(self.gcx.clone(), mode, model_id, execution_scope)
            .await;
        let resolved = crate::llm::adapters::claude_code_compat::cc_resolve_tool_name(tool_name);
        for tool in tools {
            let desc = tool.tool_description();
            if desc.name == tool_name || desc.name == resolved.as_str() {
                let mut coerced_args: HashMap<String, serde_json::Value> =
                    args.into_iter().collect();
                refact_tool_api::coerce_hashmap_to_schema(&mut coerced_args, &desc.input_schema);
                let integr_config_path = tool.has_config_path();
                return Some(
                    tool.match_against_confirm_deny(ccx, &coerced_args)
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
        let policy = self
            .snapshot_for_mode(self.gcx.clone(), mode, model_id)
            .await
            .policy
            .clone();
        policy
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
        let execution_scope = Self::execution_scope_from_ccx(&ccx).await;
        let tools = self
            .fresh_tools_for_scope(gcx.clone(), mode, model_id, execution_scope)
            .await;
        let resolved = crate::llm::adapters::claude_code_compat::cc_resolve_tool_name(tool_name);
        for mut tool in tools {
            let desc = tool.tool_description();
            if desc.name == tool_name || desc.name == resolved.as_str() {
                let mut coerced_args: HashMap<String, serde_json::Value> =
                    args.into_iter().collect();
                let coercion_notes = refact_tool_api::coerce_hashmap_to_schema(
                    &mut coerced_args,
                    &desc.input_schema,
                );
                if !coercion_notes.is_empty() {
                    tracing::info!(
                        "Coerced arguments for tool {}: {:?}",
                        desc.name,
                        coercion_notes
                    );
                }
                {
                    let mut cgcx = ccx.lock().await;
                    cgcx.app = AppState::from_gcx(gcx.clone()).await;
                }
                let tool_call_id = tool_call_id.to_string();
                let runtime_span = perf_diagnostics::span(PerfComponent::ToolRuntime, None, None);
                let result = tool.tool_execute(ccx, &tool_call_id, &coerced_args).await;
                let result = match result {
                    Ok(result) => {
                        runtime_span.finish_tool(PerfOutcome::Success, 1, 1, None);
                        result
                    }
                    Err(error) => {
                        runtime_span.finish_tool(PerfOutcome::Failure, 1, 1, None);
                        return Err(error);
                    }
                };
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

    struct FixtureTool {
        _build_number: usize,
    }

    #[async_trait]
    impl Tool for FixtureTool {
        async fn tool_execute(
            &mut self,
            _ccx: Arc<AMutex<crate::at_commands::at_commands::AtCommandsContext>>,
            _tool_call_id: &String,
            _args: &HashMap<String, serde_json::Value>,
        ) -> Result<(bool, Vec<crate::call_validation::ContextEnum>), String> {
            Ok((false, Vec::new()))
        }

        fn tool_description(&self) -> ToolDesc {
            ToolDesc {
                name: "fixture".to_string(),
                experimental: false,
                allow_parallel: true,
                description: format!("fixture {}", self._build_number),
                input_schema: serde_json::json!({"type":"object"}),
                output_schema: None,
                annotations: None,
                display_name: "fixture".to_string(),
                source: ToolSource {
                    source_type: ToolSourceType::Builtin,
                    config_path: String::new(),
                },
            }
        }
    }

    fn fixture_registry(gcx: SharedGlobalContext, builds: Arc<AtomicUsize>) -> AppToolRegistry {
        AppToolRegistry::with_fixture_tool_factory(
            gcx,
            Arc::new(move || {
                let build_number = builds.fetch_add(1, Ordering::SeqCst);
                vec![Box::new(FixtureTool {
                    _build_number: build_number,
                })]
            }),
        )
    }

    #[tokio::test]
    async fn tool_catalog_snapshot_single_flights_and_reuses_descriptors() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let builds = Arc::new(AtomicUsize::new(0));
        let registry = Arc::new(fixture_registry(gcx.clone(), builds.clone()));
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let registry = registry.clone();
            tasks.push(tokio::spawn(async move {
                registry
                    .snapshot_for_mode_for_test("agent", Some("provider/model"))
                    .await
            }));
        }
        let snapshots = futures::future::join_all(tasks)
            .await
            .into_iter()
            .map(Result::unwrap)
            .collect::<Vec<_>>();

        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert!(snapshots
            .iter()
            .all(|snapshot| Arc::ptr_eq(snapshot, &snapshots[0])));
        assert_eq!(gcx.tool_catalog_cache.snapshot_count().await, 1);
    }

    #[tokio::test]
    async fn tool_catalog_snapshot_warm_acquisition_stays_below_two_milliseconds() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let builds = Arc::new(AtomicUsize::new(0));
        let registry = fixture_registry(gcx, builds.clone());
        registry
            .snapshot_for_mode_for_test("agent", Some("provider/model"))
            .await;
        let mut samples = Vec::new();
        for _ in 0..16 {
            let started = std::time::Instant::now();
            registry
                .snapshot_for_mode_for_test("agent", Some("provider/model"))
                .await;
            samples.push(started.elapsed());
        }
        samples.sort_unstable();

        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert!(samples[15] < std::time::Duration::from_millis(2));
    }

    #[tokio::test]
    async fn tool_catalog_generations_create_next_turn_snapshot_without_mutating_old_one() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let builds = Arc::new(AtomicUsize::new(0));
        let registry = fixture_registry(gcx.clone(), builds.clone());
        let first = registry
            .snapshot_for_mode_for_test("agent", Some("provider/model"))
            .await;
        gcx.tool_catalog_generations.advance_privacy();
        let second = registry
            .snapshot_for_mode_for_test("agent", Some("provider/model"))
            .await;
        gcx.ext_cache_generation.fetch_add(1, Ordering::SeqCst);
        let third = registry
            .snapshot_for_mode_for_test("agent", Some("provider/model"))
            .await;

        assert_eq!(builds.load(Ordering::SeqCst), 3);
        assert!(!Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&second, &third));
        assert_eq!(first.index.tools[0].name, "fixture");
        assert_eq!(second.index.tools[0].name, "fixture");
        assert_eq!(third.index.tools[0].name, "fixture");
    }

    #[tokio::test]
    async fn tool_catalog_snapshot_cache_is_bounded_across_generations() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let builds = Arc::new(AtomicUsize::new(0));
        let registry = fixture_registry(gcx.clone(), builds);

        for _ in 0..=TOOL_CATALOG_SNAPSHOT_CACHE_LIMIT {
            registry
                .snapshot_for_mode_for_test("agent", Some("provider/model"))
                .await;
            gcx.tool_catalog_generations.advance_integrations();
        }

        assert_eq!(
            gcx.tool_catalog_cache.snapshot_count().await,
            TOOL_CATALOG_SNAPSHOT_CACHE_LIMIT
        );
        assert_eq!(gcx.tool_catalog_cache.build_lock_count().await, 0);
    }

    #[tokio::test]
    async fn tool_catalog_key_isolates_mode_and_model_and_keeps_mutable_instances_fresh() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let builds = Arc::new(AtomicUsize::new(0));
        let registry = fixture_registry(gcx.clone(), builds.clone());
        let first = registry
            .snapshot_for_mode_for_test("agent", Some("provider/model-a"))
            .await;
        let second = registry
            .snapshot_for_mode_for_test("task_agent", Some("provider/model-a"))
            .await;
        let third = registry
            .snapshot_for_mode_for_test("agent", Some("provider/model-b"))
            .await;
        let mutable_a = registry
            .tools_for_mode(gcx.clone(), "agent", Some("provider/model-a"))
            .await;
        let mutable_b = registry
            .tools_for_mode(gcx.clone(), "agent", Some("provider/model-a"))
            .await;

        assert!(!Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&first, &third));
        assert_eq!(builds.load(Ordering::SeqCst), 5);
        assert_eq!(gcx.tool_catalog_cache.snapshot_count().await, 3);
        assert_ne!(
            mutable_a[0].tool_description().description,
            mutable_b[0].tool_description().description
        );
    }

    #[tokio::test]
    async fn tool_catalog_key_isolates_execution_scopes() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let builds = Arc::new(AtomicUsize::new(0));
        let registry = fixture_registry(gcx, builds.clone());
        let first = registry
            .snapshot_for_mode_with_scope_for_test(
                "agent",
                Some("provider/model"),
                Some("/workspace/one".to_string()),
            )
            .await;
        let second = registry
            .snapshot_for_mode_with_scope_for_test(
                "agent",
                Some("provider/model"),
                Some("/workspace/two".to_string()),
            )
            .await;

        assert_eq!(builds.load(Ordering::SeqCst), 2);
        assert!(!Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn tool_catalog_snapshot_rollout_switch_keeps_the_cold_fallback_available() {
        for disabled in ["0", "false", "no", "off"] {
            assert!(!AppToolRegistry::snapshot_cache_enabled_for(Some(disabled)));
        }
        for enabled in [None, Some("1"), Some("true"), Some("yes")] {
            assert!(AppToolRegistry::snapshot_cache_enabled_for(enabled));
        }
    }

    #[test]
    fn tool_catalog_generations_are_independent_for_every_invalidation_source() {
        let generations = crate::global_context::ToolCatalogGenerations::default();

        generations.advance_customization();
        generations.advance_integrations();
        generations.advance_mcp();
        generations.advance_privacy();
        generations.advance_capabilities();

        assert_eq!(generations.customization.load(Ordering::Acquire), 1);
        assert_eq!(generations.integrations.load(Ordering::Acquire), 1);
        assert_eq!(generations.mcp.load(Ordering::Acquire), 1);
        assert_eq!(generations.privacy.load(Ordering::Acquire), 1);
        assert_eq!(generations.capabilities.load(Ordering::Acquire), 1);
    }
}
