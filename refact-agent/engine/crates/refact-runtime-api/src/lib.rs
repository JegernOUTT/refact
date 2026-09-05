use std::any::Any;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use refact_buddy_core::snapshot::BuddySnapshot;
use refact_buddy_core::types::{BuddyRuntimeEvent, BuddySuggestion};
use refact_buddy_core::user_action::UserAction;
use refact_chat_api::{
    ChatCommand, ChatMessage, ContextFile, DeliveryOutcome, GoalSnapshot, PauseReason,
    PendingDelivery, PushMode, ThreadParams,
};
use refact_chat_history::trajectory_snapshot::TrajectorySnapshot;
use refact_tool_api::{build_registry_from_names, ToolAliasRegistry, ToolDesc};

pub use refact_chat_api::{DeliveryOutcome as RuntimeDeliveryOutcome, SessionState, TaskMeta};
pub use refact_chat_api::{PendingDelivery as RuntimePendingDelivery, PushMode as RuntimePushMode};
pub use refact_tool_api::ToolDesc as RuntimeToolDesc;
pub use refact_buddy_core::types::BuddyRuntimeEvent as RuntimeBuddyEvent;
pub use refact_buddy_core::user_action::UserAction as RuntimeUserAction;
pub use refact_chat_history::trajectory_snapshot::TrajectorySnapshot as RuntimeTrajectorySnapshot;

#[async_trait]
pub trait ActivitySink: Send + Sync {
    async fn record_user_action(&self, action: UserAction);
}

#[async_trait]
pub trait BuddyEventSink: Send + Sync {
    async fn enqueue_event(&self, event: BuddyRuntimeEvent);
    async fn complete_event(&self, dedupe_key: &str, status: &str);
    async fn snapshot(&self) -> Option<BuddySnapshot>;
    async fn apply_chat_completion(&self, event: BuddyRuntimeEvent, xp: u64, mood: String);
    async fn report_error(
        &self,
        error_type: &str,
        error_msg: &str,
        source: Option<&str>,
        chat_id: Option<&str>,
    );
    async fn report_error_with_model(
        &self,
        error_type: &str,
        error_msg: &str,
        source: Option<&str>,
        chat_id: Option<&str>,
        model_id: Option<&str>,
    ) {
        let _ = model_id;
        self.report_error(error_type, error_msg, source, chat_id)
            .await;
    }
    async fn mark_chat_error(&self, event: BuddyRuntimeEvent);
    async fn maybe_add_suggestion(&self, suggestion: BuddySuggestion);
    async fn build_pulse_message(&self) -> Option<ChatMessage>;
    async fn render_runtime_event_fast(
        &self,
        workflow_id: &str,
        workflow_summary: &str,
        status: &str,
    ) -> Option<(String, Option<String>)>;
}

#[derive(Clone)]
pub struct ToolRegistryIndex {
    pub tools: Vec<ToolDesc>,
    pub mcp_lazy_mode: bool,
    pub mcp_total_count: usize,
    pub mcp_tool_index: Vec<(String, String)>,
}

#[derive(Clone)]
pub struct ToolCatalogSnapshot {
    pub index: ToolRegistryIndex,
    pub policy: Vec<ToolPolicyInfo>,
    pub aliases: ToolAliasRegistry,
}

#[derive(Clone)]
pub struct TurnToolPool {
    inner: Arc<dyn Any + Send + Sync>,
    initial_vector_builds: Arc<AtomicU64>,
    fallback_vector_builds: Arc<AtomicU64>,
}

impl TurnToolPool {
    pub fn new<T>(inner: T) -> Self
    where
        T: Any + Send + Sync,
    {
        Self {
            inner: Arc::new(inner),
            initial_vector_builds: Arc::new(AtomicU64::new(0)),
            fallback_vector_builds: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn downcast_ref<T>(&self) -> Option<&T>
    where
        T: Any,
    {
        self.inner.downcast_ref()
    }

    pub fn record_initial_vector_build(&self) {
        self.initial_vector_builds.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_fallback_vector_build(&self) {
        self.fallback_vector_builds.fetch_add(1, Ordering::Relaxed);
    }

    pub fn initial_vector_builds(&self) -> u64 {
        self.initial_vector_builds.load(Ordering::Relaxed)
    }

    pub fn fallback_vector_builds(&self) -> u64 {
        self.fallback_vector_builds.load(Ordering::Relaxed)
    }
}

#[derive(Clone)]
pub struct ToolConfirmationCheck {
    pub tool_name: String,
    pub result: refact_tool_api::MatchConfirmDeny,
    pub integr_config_path: Option<String>,
}

#[derive(Clone)]
pub struct ToolPolicyInfo {
    pub name: String,
    pub effective_allow_parallel: bool,
}

#[derive(Clone)]
pub struct ToolExecutionResult {
    pub had_corrections: bool,
    pub messages: Vec<ChatMessage>,
    pub context_files: Vec<ContextFile>,
}

#[async_trait]
pub trait ToolRegistry: Send + Sync {
    async fn get_tools_for_mode(&self, mode: &str, model_id: Option<&str>) -> Vec<ToolDesc>;
    async fn get_tools_index_for_mode(
        &self,
        mode: &str,
        model_id: Option<&str>,
    ) -> ToolRegistryIndex;
    async fn get_tools_index_for_mode_and_scope(
        &self,
        mode: &str,
        model_id: Option<&str>,
        execution_scope: Option<&str>,
    ) -> ToolRegistryIndex {
        let _ = execution_scope;
        self.get_tools_index_for_mode(mode, model_id).await
    }
    async fn acquire_tool_catalog(
        &self,
        mode: &str,
        model_id: Option<&str>,
        execution_scope: Option<&str>,
    ) -> std::sync::Arc<ToolCatalogSnapshot> {
        let index = self
            .get_tools_index_for_mode_and_scope(mode, model_id, execution_scope)
            .await;
        let names = index
            .tools
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>();
        let policy = self.get_tool_policy_info(mode, model_id).await;
        std::sync::Arc::new(ToolCatalogSnapshot {
            index,
            policy,
            aliases: build_registry_from_names(&names),
        })
    }
    async fn acquire_turn_tool_pool(
        &self,
        mode: &str,
        model_id: Option<&str>,
        execution_scope: Option<&str>,
        catalog: &ToolCatalogSnapshot,
    ) -> Option<TurnToolPool> {
        let _ = (mode, model_id, execution_scope, catalog);
        None
    }
    async fn prepare_turn_tool_pool(
        &self,
        pool: &TurnToolPool,
        catalog: &ToolCatalogSnapshot,
        mode: &str,
        model_id: Option<&str>,
        tool_slots: &[(ToolDesc, usize)],
    ) -> Result<(), String> {
        let _ = (pool, catalog, mode, model_id, tool_slots);
        Ok(())
    }
    async fn check_tool_confirmation(
        &self,
        ccx: &(dyn std::any::Any + Send + Sync),
        mode: &str,
        model_id: Option<&str>,
        tool_name: &str,
        args: serde_json::Map<String, serde_json::Value>,
    ) -> Option<Result<ToolConfirmationCheck, String>>;
    async fn check_tool_confirmation_with_catalog(
        &self,
        ccx: &(dyn std::any::Any + Send + Sync),
        catalog: &ToolCatalogSnapshot,
        mode: &str,
        model_id: Option<&str>,
        tool_name: &str,
        args: serde_json::Map<String, serde_json::Value>,
    ) -> Option<Result<ToolConfirmationCheck, String>> {
        let _ = catalog;
        self.check_tool_confirmation(ccx, mode, model_id, tool_name, args)
            .await
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
        let _ = pool;
        self.check_tool_confirmation_with_catalog(ccx, catalog, mode, model_id, tool_name, args)
            .await
    }
    async fn get_tool_policy_info(&self, mode: &str, model_id: Option<&str>)
        -> Vec<ToolPolicyInfo>;
    async fn execute_tool(
        &self,
        ccx: &(dyn std::any::Any + Send + Sync),
        mode: &str,
        model_id: Option<&str>,
        tool_call_id: &str,
        tool_name: &str,
        args: serde_json::Map<String, serde_json::Value>,
    ) -> Result<Option<ToolExecutionResult>, String>;
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
        let _ = catalog;
        self.execute_tool(ccx, mode, model_id, tool_call_id, tool_name, args)
            .await
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
        let _ = pool;
        self.execute_tool_with_catalog(ccx, catalog, mode, model_id, tool_call_id, tool_name, args)
            .await
    }
    async fn load_task_memories(&self, task_id: &str) -> Result<Vec<(PathBuf, String)>, String>;
}

#[derive(Clone)]
pub struct ChatSessionSnapshot {
    pub messages: Vec<ChatMessage>,
    pub thread: ThreadParams,
    pub session_state: SessionState,
    pub pause_reasons: Vec<PauseReason>,
    pub goal: Option<GoalSnapshot>,
}

#[derive(Clone)]
pub struct ChatSessionUpdate {
    pub messages: Vec<ChatMessage>,
    pub previous_response_id: Option<String>,
}

#[derive(Clone)]
pub struct CreateSessionRequest {
    pub chat_id: String,
    pub thread: ThreadParams,
    pub messages: Vec<ChatMessage>,
}

#[async_trait]
pub trait ChatSessionFacade: Send + Sync {
    async fn session_snapshot(&self, chat_id: &str) -> Result<ChatSessionSnapshot, String>;
    async fn update_session(&self, chat_id: &str, update: ChatSessionUpdate) -> Result<(), String>;
    async fn create_session(&self, request: CreateSessionRequest) -> Result<(), String>;
    async fn push_command(&self, chat_id: &str, command: ChatCommand) -> Result<(), String>;
    async fn push_priority_command(
        &self,
        chat_id: &str,
        command: ChatCommand,
    ) -> Result<(), String> {
        self.push_command(chat_id, command).await
    }
    /// Unified delivery entry point for producers. The default implementation
    /// only enqueues the command (so existing mocks keep compiling); the engine
    /// implementation reports the real outcome.
    async fn deliver_messages(
        &self,
        chat_id: &str,
        delivery: PendingDelivery,
    ) -> Result<DeliveryOutcome, String> {
        self.push_command(chat_id, ChatCommand::DeliverMessages { delivery })
            .await?;
        Ok(DeliveryOutcome::Queued)
    }
    /// Reprioritize or cancel a pending delivery. Default implementation routes
    /// through the command queue.
    async fn update_pending_delivery(
        &self,
        chat_id: &str,
        delivery_id: &str,
        push: Option<PushMode>,
        cancel: bool,
    ) -> Result<(), String> {
        self.push_command(
            chat_id,
            ChatCommand::UpdatePendingDelivery {
                delivery_id: delivery_id.to_string(),
                push,
                cancel,
            },
        )
        .await
    }
    async fn session_state(&self, chat_id: &str) -> Result<Option<SessionState>, String>;
    async fn maybe_save_session(&self, chat_id: &str) -> Result<(), String>;
    async fn save_trajectory_snapshot(&self, snapshot: TrajectorySnapshot) -> Result<(), String>;
}
