use std::collections::{HashMap, VecDeque};
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
    ToolCatalogSnapshot, ToolRegistry, ToolRegistryIndex, TurnToolPool,
};

pub const TOOL_CATALOG_SNAPSHOTS_ENV: &str = "REFACT_TOOL_CATALOG_SNAPSHOTS";
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
    pub indexing_everywhere: Arc<StdRwLock<Arc<IndexingEverywhere>>>,
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

type MutableTool = Box<dyn crate::tools::tools_description::Tool + Send>;

struct AppTurnToolPool {
    tools: AMutex<HashMap<String, VecDeque<MutableTool>>>,
}

impl AppTurnToolPool {
    fn tool_key(desc: &refact_tool_api::ToolDesc) -> String {
        format!(
            "{}\u{1f}{:?}\u{1f}{}",
            crate::llm::adapters::claude_code_compat::cc_resolve_tool_name(&desc.name),
            desc.source.source_type,
            desc.source.config_path
        )
    }

    fn from_tools(tools: Vec<MutableTool>) -> Self {
        let mut grouped: HashMap<String, VecDeque<MutableTool>> = HashMap::new();
        for tool in tools {
            grouped
                .entry(Self::tool_key(&tool.tool_description()))
                .or_default()
                .push_back(tool);
        }
        Self {
            tools: AMutex::new(grouped),
        }
    }

    async fn take(&self, desc: &refact_tool_api::ToolDesc) -> Option<MutableTool> {
        self.tools
            .lock()
            .await
            .get_mut(&Self::tool_key(desc))
            .and_then(VecDeque::pop_front)
    }

    async fn return_tool(&self, tool: MutableTool) {
        let name = Self::tool_key(&tool.tool_description());
        self.tools
            .lock()
            .await
            .entry(name)
            .or_default()
            .push_back(tool);
    }

    async fn add_missing_tools(&self, tools: Vec<MutableTool>) {
        let mut grouped = self.tools.lock().await;
        for tool in tools {
            let name = Self::tool_key(&tool.tool_description());
            grouped.entry(name).or_default().push_back(tool);
        }
    }
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
        tool_catalog_snapshot_rollout_enabled()
    }

    #[cfg(test)]
    fn snapshot_cache_enabled_for(value: Option<&str>) -> bool {
        tool_catalog_snapshot_rollout_enabled_for(value)
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
        let descriptions = tools
            .tools
            .iter()
            .map(|tool| tool.tool_description())
            .collect::<Vec<_>>();
        let names = descriptions
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>();
        ToolCatalogSnapshot {
            index: ToolRegistryIndex {
                tools: descriptions,
                mcp_lazy_mode: tools.mcp_lazy_mode,
                mcp_total_count: tools.mcp_total_count,
                mcp_tool_index: tools.mcp_tool_index,
            },
            policy,
            aliases: refact_tool_api::build_registry_from_names(&names),
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

    async fn fresh_mutable_tools(
        &self,
        gcx: SharedGlobalContext,
        mode: &str,
        model_id: Option<&str>,
        catalog: &ToolCatalogSnapshot,
        component: PerfComponent,
    ) -> Vec<Box<dyn crate::tools::tools_description::Tool + Send>> {
        let span = perf_diagnostics::span(component, None, None);
        let tools = crate::tools::tools_list::apply_mcp_lazy_filter(
            self.tools_for_mode(gcx, mode, model_id).await,
        )
        .tools;
        span.finish_tool(
            PerfOutcome::Success,
            1,
            catalog.index.tools.len() as u64,
            None,
        );
        tools
    }

    async fn build_turn_tool_pool(
        &self,
        gcx: SharedGlobalContext,
        mode: &str,
        model_id: Option<&str>,
        catalog: &ToolCatalogSnapshot,
    ) -> TurnToolPool {
        let tools = self
            .fresh_mutable_tools(
                gcx,
                mode,
                model_id,
                catalog,
                PerfComponent::ToolMutableVectorBuild,
            )
            .await;
        let pool = TurnToolPool::new(AppTurnToolPool::from_tools(tools));
        pool.record_initial_vector_build();
        pool
    }

    fn app_turn_tool_pool<'a>(pool: &'a TurnToolPool) -> Result<&'a AppTurnToolPool, String> {
        pool.downcast_ref::<AppTurnToolPool>()
            .ok_or_else(|| "invalid TurnToolPool passed to AppToolRegistry".to_string())
    }

    async fn add_turn_tool_pool_fallback(
        &self,
        pool: &TurnToolPool,
        gcx: SharedGlobalContext,
        mode: &str,
        model_id: Option<&str>,
        catalog: &ToolCatalogSnapshot,
    ) -> Result<(), String> {
        let tools = self
            .fresh_mutable_tools(
                gcx,
                mode,
                model_id,
                catalog,
                PerfComponent::ToolPoolParallelExpansion,
            )
            .await;
        Self::app_turn_tool_pool(pool)?
            .add_missing_tools(tools)
            .await;
        pool.record_fallback_vector_build();
        Ok(())
    }

    async fn take_turn_tool(
        &self,
        pool: &TurnToolPool,
        gcx: SharedGlobalContext,
        mode: &str,
        model_id: Option<&str>,
        catalog: &ToolCatalogSnapshot,
        tool_name: &str,
    ) -> Result<Option<MutableTool>, String> {
        let catalog_desc = match Self::find_catalog_descriptor(catalog, tool_name) {
            Some(desc) => desc,
            None => return Ok(None),
        };
        let pool_impl = Self::app_turn_tool_pool(pool)?;
        if let Some(tool) = pool_impl.take(catalog_desc).await {
            return Ok(Some(tool));
        }
        self.add_turn_tool_pool_fallback(pool, gcx, mode, model_id, catalog)
            .await?;
        Ok(pool_impl.take(catalog_desc).await)
    }

    fn find_catalog_descriptor<'a>(
        catalog: &'a ToolCatalogSnapshot,
        tool_name: &str,
    ) -> Option<&'a refact_tool_api::ToolDesc> {
        let resolved = crate::llm::adapters::claude_code_compat::cc_resolve_tool_name(tool_name);
        catalog
            .index
            .tools
            .iter()
            .find(|desc| desc.name == tool_name || desc.name == resolved.as_str())
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

pub(crate) fn tool_catalog_snapshot_rollout_enabled() -> bool {
    std::env::var(TOOL_CATALOG_SNAPSHOTS_ENV)
        .ok()
        .as_deref()
        .or_else(|| {
            crate::runtime_settings::current()
                .tool_catalog_snapshots_enabled
                .then_some("1")
        })
        .is_some_and(|value| tool_catalog_snapshot_rollout_enabled_for(Some(value)))
}

pub(crate) fn tool_catalog_snapshot_rollout_enabled_for(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        let value = value.trim();
        value == "1"
            || value.eq_ignore_ascii_case("true")
            || value.eq_ignore_ascii_case("yes")
            || value.eq_ignore_ascii_case("on")
    })
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

    async fn acquire_tool_catalog(
        &self,
        mode: &str,
        model_id: Option<&str>,
        execution_scope: Option<&str>,
    ) -> Arc<ToolCatalogSnapshot> {
        self.snapshot_for_mode_with_scope(
            self.gcx.clone(),
            mode,
            model_id,
            execution_scope.map(str::to_string),
        )
        .await
    }

    async fn acquire_turn_tool_pool(
        &self,
        mode: &str,
        model_id: Option<&str>,
        _execution_scope: Option<&str>,
        catalog: &ToolCatalogSnapshot,
    ) -> Option<TurnToolPool> {
        if !Self::snapshot_cache_enabled() {
            return None;
        }
        Some(
            self.build_turn_tool_pool(self.gcx.clone(), mode, model_id, catalog)
                .await,
        )
    }

    async fn prepare_turn_tool_pool(
        &self,
        pool: &TurnToolPool,
        catalog: &ToolCatalogSnapshot,
        mode: &str,
        model_id: Option<&str>,
        tool_slots: &[(refact_tool_api::ToolDesc, usize)],
    ) -> Result<(), String> {
        let gcx = self.gcx.clone();
        for (tool_desc, required_slots) in tool_slots {
            let pool_impl = Self::app_turn_tool_pool(pool)?;
            let existing = {
                let tools = pool_impl.tools.lock().await;
                tools
                    .get(&AppTurnToolPool::tool_key(tool_desc))
                    .map_or(0, VecDeque::len)
            };
            for _ in existing..*required_slots {
                self.add_turn_tool_pool_fallback(pool, gcx.clone(), mode, model_id, catalog)
                    .await?;
            }
        }
        Ok(())
    }

    async fn check_tool_confirmation(
        &self,
        ccx: &(dyn std::any::Any + Send + Sync),
        mode: &str,
        model_id: Option<&str>,
        tool_name: &str,
        args: serde_json::Map<String, serde_json::Value>,
    ) -> Option<Result<ToolConfirmationCheck, String>> {
        let execution_scope = match ccx
            .downcast_ref::<Arc<AMutex<crate::at_commands::at_commands::AtCommandsContext>>>()
        {
            Some(ccx) => Self::execution_scope_from_ccx(ccx).await,
            None => None,
        };
        let catalog = self
            .acquire_tool_catalog(mode, model_id, execution_scope.as_deref())
            .await;
        self.check_tool_confirmation_with_catalog(ccx, &catalog, mode, model_id, tool_name, args)
            .await
    }

    async fn check_tool_confirmation_with_catalog(
        &self,
        ccx: &(dyn std::any::Any + Send + Sync),
        catalog: &ToolCatalogSnapshot,
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
        let tools = self
            .fresh_mutable_tools(
                self.gcx.clone(),
                mode,
                model_id,
                catalog,
                PerfComponent::ToolMutableVectorBuild,
            )
            .await;
        let catalog_desc = Self::find_catalog_descriptor(catalog, tool_name)?;
        let resolved = crate::llm::adapters::claude_code_compat::cc_resolve_tool_name(tool_name);
        for tool in tools {
            let desc = tool.tool_description();
            if desc.name == tool_name || desc.name == resolved.as_str() {
                let mut coerced_args: HashMap<String, serde_json::Value> =
                    args.into_iter().collect();
                refact_tool_api::coerce_hashmap_to_schema(
                    &mut coerced_args,
                    &catalog_desc.input_schema,
                );
                let integr_config_path = (!catalog_desc.source.config_path.is_empty())
                    .then(|| catalog_desc.source.config_path.clone());
                return Some(
                    tool.match_against_confirm_deny(ccx, &coerced_args)
                        .await
                        .map(|result| ToolConfirmationCheck {
                            tool_name: catalog_desc.name.clone(),
                            result,
                            integr_config_path,
                        }),
                );
            }
        }
        None
    }

    async fn check_tool_confirmation_with_catalog_and_pool(
        &self,
        ccx: &(dyn std::any::Any + Send + Sync),
        catalog: &ToolCatalogSnapshot,
        pool: Option<&TurnToolPool>,
        mode: &str,
        model_id: Option<&str>,
        tool_name: &str,
        args: serde_json::Map<String, serde_json::Value>,
    ) -> Option<Result<ToolConfirmationCheck, String>> {
        let Some(pool) = pool else {
            return self
                .check_tool_confirmation_with_catalog(ccx, catalog, mode, model_id, tool_name, args)
                .await;
        };
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
        let catalog_desc = Self::find_catalog_descriptor(catalog, tool_name)?;
        let gcx = {
            let cgcx = ccx.lock().await;
            cgcx.app.gcx.clone()
        };
        let Some(tool) = (match self
            .take_turn_tool(pool, gcx, mode, model_id, catalog, tool_name)
            .await
        {
            Ok(tool) => tool,
            Err(error) => return Some(Err(error)),
        }) else {
            return None;
        };
        let mut coerced_args: HashMap<String, serde_json::Value> = args.into_iter().collect();
        refact_tool_api::coerce_hashmap_to_schema(&mut coerced_args, &catalog_desc.input_schema);
        let integr_config_path = (!catalog_desc.source.config_path.is_empty())
            .then(|| catalog_desc.source.config_path.clone());
        let result = tool
            .match_against_confirm_deny(ccx, &coerced_args)
            .await
            .map(|result| ToolConfirmationCheck {
                tool_name: catalog_desc.name.clone(),
                result,
                integr_config_path,
            });
        Self::app_turn_tool_pool(pool)
            .expect("pool was validated before tool lease")
            .return_tool(tool)
            .await;
        Some(result)
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
        let execution_scope = match ccx
            .downcast_ref::<Arc<AMutex<crate::at_commands::at_commands::AtCommandsContext>>>()
        {
            Some(ccx) => Self::execution_scope_from_ccx(ccx).await,
            None => None,
        };
        let catalog = self
            .acquire_tool_catalog(mode, model_id, execution_scope.as_deref())
            .await;
        self.execute_tool_with_catalog(ccx, &catalog, mode, model_id, tool_call_id, tool_name, args)
            .await
    }

    async fn execute_tool_with_catalog(
        &self,
        ccx: &(dyn std::any::Any + Send + Sync),
        catalog: &ToolCatalogSnapshot,
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
        let lookup_span = perf_diagnostics::span(PerfComponent::ToolExecutionLookup, None, None);
        let gcx = {
            let cgcx = ccx.lock().await;
            cgcx.app.gcx.clone()
        };
        let tools = self
            .fresh_mutable_tools(
                gcx.clone(),
                mode,
                model_id,
                catalog,
                PerfComponent::ToolMutableVectorBuild,
            )
            .await;
        let catalog_desc = match Self::find_catalog_descriptor(catalog, tool_name) {
            Some(desc) => desc,
            None => return Ok(None),
        };
        let resolved = crate::llm::adapters::claude_code_compat::cc_resolve_tool_name(tool_name);
        for mut tool in tools {
            let desc = tool.tool_description();
            if desc.name == tool_name || desc.name == resolved.as_str() {
                let mut coerced_args: HashMap<String, serde_json::Value> =
                    args.into_iter().collect();
                let coercion_notes = refact_tool_api::coerce_hashmap_to_schema(
                    &mut coerced_args,
                    &catalog_desc.input_schema,
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
                lookup_span.finish_tool(PerfOutcome::Success, 1, 1, None);
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
        lookup_span.finish_tool(PerfOutcome::Failure, 1, 1, None);
        Ok(None)
    }

    async fn execute_tool_with_catalog_and_pool(
        &self,
        ccx: &(dyn std::any::Any + Send + Sync),
        catalog: &ToolCatalogSnapshot,
        pool: Option<&TurnToolPool>,
        mode: &str,
        model_id: Option<&str>,
        tool_call_id: &str,
        tool_name: &str,
        args: serde_json::Map<String, serde_json::Value>,
    ) -> Result<Option<ToolExecutionResult>, String> {
        let Some(pool) = pool else {
            return self
                .execute_tool_with_catalog(
                    ccx,
                    catalog,
                    mode,
                    model_id,
                    tool_call_id,
                    tool_name,
                    args,
                )
                .await;
        };
        let ccx = ccx
            .downcast_ref::<Arc<AMutex<crate::at_commands::at_commands::AtCommandsContext>>>()
            .ok_or_else(|| "invalid AtCommandsContext passed to ToolRegistry".to_string())?
            .clone();
        let lookup_span = perf_diagnostics::span(PerfComponent::ToolExecutionLookup, None, None);
        let catalog_desc = match Self::find_catalog_descriptor(catalog, tool_name) {
            Some(desc) => desc,
            None => return Ok(None),
        };
        let gcx = {
            let cgcx = ccx.lock().await;
            cgcx.app.gcx.clone()
        };
        let Some(mut tool) = self
            .take_turn_tool(pool, gcx.clone(), mode, model_id, catalog, tool_name)
            .await?
        else {
            return Ok(None);
        };
        let mut coerced_args: HashMap<String, serde_json::Value> = args.into_iter().collect();
        let coercion_notes = refact_tool_api::coerce_hashmap_to_schema(
            &mut coerced_args,
            &catalog_desc.input_schema,
        );
        if !coercion_notes.is_empty() {
            tracing::info!(
                "Coerced arguments for tool {}: {:?}",
                tool.tool_description().name,
                coercion_notes
            );
        }
        {
            let mut cgcx = ccx.lock().await;
            cgcx.app = AppState::from_gcx(gcx).await;
        }
        lookup_span.finish_tool(PerfOutcome::Success, 1, 1, None);
        let runtime_span = perf_diagnostics::span(PerfComponent::ToolRuntime, None, None);
        let result = tool
            .tool_execute(ccx, &tool_call_id.to_string(), &coerced_args)
            .await;
        let result = match result {
            Ok(result) => {
                runtime_span.finish_tool(PerfOutcome::Success, 1, 1, None);
                result
            }
            Err(error) => {
                runtime_span.finish_tool(PerfOutcome::Failure, 1, 1, None);
                Self::app_turn_tool_pool(pool)
                    .expect("pool was validated before tool lease")
                    .return_tool(tool)
                    .await;
                return Err(error);
            }
        };
        Self::app_turn_tool_pool(pool)
            .expect("pool was validated before tool lease")
            .return_tool(tool)
            .await;
        let mut messages = Vec::new();
        let mut context_files = Vec::new();
        for item in result.1 {
            match item {
                crate::call_validation::ContextEnum::ChatMessage(message) => messages.push(message),
                crate::call_validation::ContextEnum::ContextFile(file) => context_files.push(file),
            }
        }
        Ok(Some(ToolExecutionResult {
            had_corrections: result.0,
            messages,
            context_files,
        }))
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
    use serial_test::serial;

    struct ToolCatalogSnapshotsEnvGuard(Option<std::ffi::OsString>);

    impl ToolCatalogSnapshotsEnvGuard {
        fn enable() -> Self {
            let previous = std::env::var_os(TOOL_CATALOG_SNAPSHOTS_ENV);
            std::env::set_var(TOOL_CATALOG_SNAPSHOTS_ENV, "1");
            Self(previous)
        }
    }

    impl Drop for ToolCatalogSnapshotsEnvGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.0.take() {
                std::env::set_var(TOOL_CATALOG_SNAPSHOTS_ENV, previous);
            } else {
                std::env::remove_var(TOOL_CATALOG_SNAPSHOTS_ENV);
            }
        }
    }

    struct FixtureTool {
        _build_number: usize,
        name: String,
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
                name: self.name.clone(),
                experimental: false,
                allow_parallel: true,
                description: format!("fixture {}", self._build_number),
                input_schema: serde_json::json!({"type":"object"}),
                output_schema: None,
                annotations: None,
                display_name: self.name.clone(),
                source: ToolSource {
                    source_type: ToolSourceType::Builtin,
                    config_path: String::new(),
                },
            }
        }
    }

    fn fixture_registry(gcx: SharedGlobalContext, builds: Arc<AtomicUsize>) -> AppToolRegistry {
        fixture_registry_with_names(gcx, builds, vec!["fixture".to_string()])
    }

    fn fixture_registry_with_names(
        gcx: SharedGlobalContext,
        builds: Arc<AtomicUsize>,
        names: Vec<String>,
    ) -> AppToolRegistry {
        AppToolRegistry::with_fixture_tool_factory(
            gcx,
            Arc::new(move || {
                let build_number = builds.fetch_add(1, Ordering::SeqCst);
                names
                    .iter()
                    .map(|name| {
                        Box::new(FixtureTool {
                            _build_number: build_number,
                            name: name.clone(),
                        })
                            as Box<dyn crate::tools::tools_description::Tool + Send>
                    })
                    .collect()
            }),
        )
    }

    #[serial]
    #[tokio::test]
    async fn tool_catalog_snapshot_single_flights_and_reuses_descriptors() {
        let _env = ToolCatalogSnapshotsEnvGuard::enable();
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

    #[serial]
    #[tokio::test]
    async fn tool_catalog_snapshot_warm_acquisition_stays_below_two_milliseconds() {
        let _env = ToolCatalogSnapshotsEnvGuard::enable();
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

    #[serial]
    #[tokio::test]
    async fn tool_catalog_generations_create_next_turn_snapshot_without_mutating_old_one() {
        let _env = ToolCatalogSnapshotsEnvGuard::enable();
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

    #[serial]
    #[tokio::test]
    async fn tool_catalog_snapshot_cache_is_bounded_across_generations() {
        let _env = ToolCatalogSnapshotsEnvGuard::enable();
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

    #[serial]
    #[tokio::test]
    async fn tool_catalog_key_isolates_mode_and_model_and_keeps_mutable_instances_fresh() {
        let _env = ToolCatalogSnapshotsEnvGuard::enable();
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

    #[serial]
    #[tokio::test]
    async fn tool_catalog_key_isolates_execution_scopes() {
        let _env = ToolCatalogSnapshotsEnvGuard::enable();
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

    #[serial]
    #[tokio::test]
    async fn turn_tool_pool_unique_batch_uses_one_mutable_vector() {
        let _env = ToolCatalogSnapshotsEnvGuard::enable();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let builds = Arc::new(AtomicUsize::new(0));
        let names = (0..200).map(|index| format!("tool-{index}")).collect();
        let registry = fixture_registry_with_names(gcx.clone(), builds.clone(), names);
        let catalog = registry
            .snapshot_for_mode_for_test("agent", Some("provider/model"))
            .await;
        let pool = registry
            .build_turn_tool_pool(gcx, "agent", Some("provider/model"), &catalog)
            .await;
        let builds_after_pool = builds.load(Ordering::SeqCst);
        let slots = catalog
            .index
            .tools
            .iter()
            .cloned()
            .map(|tool| (tool, 1))
            .collect::<Vec<_>>();

        registry
            .prepare_turn_tool_pool(&pool, &catalog, "agent", Some("provider/model"), &slots)
            .await
            .unwrap();

        assert_eq!(pool.initial_vector_builds(), 1);
        assert_eq!(pool.fallback_vector_builds(), 0);
        assert_eq!(builds.load(Ordering::SeqCst), builds_after_pool);
    }

    #[serial]
    #[tokio::test]
    async fn turn_tool_pool_reuses_one_instance_for_sequential_calls() {
        let _env = ToolCatalogSnapshotsEnvGuard::enable();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let builds = Arc::new(AtomicUsize::new(0));
        let registry = fixture_registry(gcx.clone(), builds.clone());
        let catalog = registry
            .snapshot_for_mode_for_test("agent", Some("provider/model"))
            .await;
        let pool = registry
            .build_turn_tool_pool(gcx, "agent", Some("provider/model"), &catalog)
            .await;
        let builds_after_pool = builds.load(Ordering::SeqCst);

        for _ in 0..50 {
            let tool = registry
                .take_turn_tool(
                    &pool,
                    registry.gcx.clone(),
                    "agent",
                    Some("provider/model"),
                    &catalog,
                    "fixture",
                )
                .await
                .unwrap()
                .unwrap();
            AppToolRegistry::app_turn_tool_pool(&pool)
                .unwrap()
                .return_tool(tool)
                .await;
        }

        assert_eq!(pool.initial_vector_builds(), 1);
        assert_eq!(pool.fallback_vector_builds(), 0);
        assert_eq!(builds.load(Ordering::SeqCst), builds_after_pool);
    }

    #[serial]
    #[tokio::test]
    async fn turn_tool_pool_returns_the_confirmation_instance_to_execution() {
        let _env = ToolCatalogSnapshotsEnvGuard::enable();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let builds = Arc::new(AtomicUsize::new(0));
        let registry = fixture_registry(gcx.clone(), builds);
        let catalog = registry
            .snapshot_for_mode_for_test("agent", Some("provider/model"))
            .await;
        let pool = registry
            .build_turn_tool_pool(gcx, "agent", Some("provider/model"), &catalog)
            .await;

        let confirmation_tool = registry
            .take_turn_tool(
                &pool,
                registry.gcx.clone(),
                "agent",
                Some("provider/model"),
                &catalog,
                "fixture",
            )
            .await
            .unwrap()
            .unwrap();
        let confirmation_instance = confirmation_tool.tool_description().description;
        AppToolRegistry::app_turn_tool_pool(&pool)
            .unwrap()
            .return_tool(confirmation_tool)
            .await;
        let execution_tool = registry
            .take_turn_tool(
                &pool,
                registry.gcx.clone(),
                "agent",
                Some("provider/model"),
                &catalog,
                "fixture",
            )
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            confirmation_instance,
            execution_tool.tool_description().description
        );
    }

    #[serial]
    #[tokio::test]
    async fn turn_tool_pool_same_name_parallelism_is_bounded_by_multiplicity() {
        let _env = ToolCatalogSnapshotsEnvGuard::enable();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let builds = Arc::new(AtomicUsize::new(0));
        let registry = fixture_registry(gcx.clone(), builds);
        let catalog = registry
            .snapshot_for_mode_for_test("agent", Some("provider/model"))
            .await;
        let pool = registry
            .build_turn_tool_pool(gcx, "agent", Some("provider/model"), &catalog)
            .await;
        let parallelism = 8;

        registry
            .prepare_turn_tool_pool(
                &pool,
                &catalog,
                "agent",
                Some("provider/model"),
                &[(catalog.index.tools[0].clone(), parallelism)],
            )
            .await
            .unwrap();

        assert_eq!(pool.initial_vector_builds(), 1);
        assert_eq!(pool.fallback_vector_builds(), parallelism as u64 - 1);
        assert_eq!(
            pool.initial_vector_builds() + pool.fallback_vector_builds(),
            parallelism as u64
        );
    }

    #[serial]
    #[tokio::test]
    async fn turn_tool_pools_are_isolated_between_turns() {
        let _env = ToolCatalogSnapshotsEnvGuard::enable();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let builds = Arc::new(AtomicUsize::new(0));
        let registry = fixture_registry(gcx.clone(), builds.clone());
        let catalog = registry
            .snapshot_for_mode_for_test("agent", Some("provider/model"))
            .await;
        let first = registry
            .build_turn_tool_pool(gcx.clone(), "agent", Some("provider/model"), &catalog)
            .await;
        let second = registry
            .build_turn_tool_pool(gcx, "agent", Some("provider/model"), &catalog)
            .await;

        assert_eq!(first.initial_vector_builds(), 1);
        assert_eq!(second.initial_vector_builds(), 1);
        assert_eq!(builds.load(Ordering::SeqCst), 3);
    }

    #[serial]
    #[test]
    fn tool_catalog_snapshot_rollout_switch_defaults_on_and_keeps_the_cold_fallback_available() {
        assert!(
            crate::runtime_settings::TrajectoryRuntimeSettings::default()
                .tool_catalog_snapshots_enabled
        );
        for disabled in [Some("0"), Some("false"), Some("no"), Some("off")] {
            assert!(!AppToolRegistry::snapshot_cache_enabled_for(disabled));
        }
        for enabled in [Some("1"), Some("true"), Some("yes")] {
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
