use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::Mutex;
use refact_work_core::identity::{RunKey, WorkKey};
use refact_work_core::store::records::{now_ms, IntentState, RunIntentRecord};
use refact_work_core::CoordinationSettings;

use super::leases::{LeaseError, LeaseManager, LeaseRenewal};
use super::reconcile::{run_lease_key, LaunchProtocol, ProtocolError};
use crate::app_state::AppState;

pub struct LaunchIntent {
    protocol: LaunchProtocol,
    renewal: LeaseRenewal,
    run: RunKey,
    current: RunIntentRecord,
}

#[derive(Clone)]
pub struct LaunchIntentSlot {
    inner: Arc<LaunchIntentSlotInner>,
}

struct LaunchIntentSlotInner {
    intent: Mutex<Option<LaunchIntent>>,
    occupied: AtomicBool,
}

impl Drop for LaunchIntentSlotInner {
    fn drop(&mut self) {
        if let Some(intent) = self.intent.get_mut().as_ref() {
            tracing::warn!(run = ?intent.run(), "launch intent slot dropped before terminal settlement");
        }
    }
}

impl LaunchIntentSlot {
    pub async fn persist_for_agent(app: &AppState, agent_id: &str, generation: u64) -> Self {
        let settings = crate::file_activity::settings();
        Self::persist_for_agent_with_settings(app, agent_id, generation, &settings).await
    }

    async fn persist_for_agent_with_settings(
        app: &AppState,
        agent_id: &str,
        generation: u64,
        settings: &CoordinationSettings,
    ) -> Self {
        match LaunchIntent::persist_for_agent_with_settings(app, agent_id, generation, settings)
            .await
        {
            Some(intent) => Self::with_intent(intent),
            None => Self::empty(),
        }
    }

    pub(crate) fn with_intent(intent: LaunchIntent) -> Self {
        Self {
            inner: Arc::new(LaunchIntentSlotInner {
                intent: Mutex::new(Some(intent)),
                occupied: AtomicBool::new(true),
            }),
        }
    }

    pub fn empty() -> Self {
        Self {
            inner: Arc::new(LaunchIntentSlotInner {
                intent: Mutex::new(None),
                occupied: AtomicBool::new(false),
            }),
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.inner.occupied.load(Ordering::Acquire)
    }

    pub async fn dispatched(&self) {
        let mut intent = self.inner.intent.lock().await;
        if let Some(intent) = intent.as_mut() {
            if let Err(error) = intent.dispatched(now_ms()).await {
                tracing::warn!(run = ?intent.run(), %error, "failed to advance launch intent to dispatched");
            }
        }
    }

    pub async fn acknowledged(&self) {
        let mut intent = self.inner.intent.lock().await;
        if let Some(intent) = intent.as_mut() {
            if let Err(error) = intent.acknowledged(now_ms()).await {
                tracing::warn!(run = ?intent.run(), %error, "failed to advance launch intent to acknowledged");
            }
        }
    }

    pub async fn settle(&self) {
        let intent = self.inner.intent.lock().await.take();
        self.inner.occupied.store(false, Ordering::Release);
        if let Some(intent) = intent {
            if let Err(error) = intent.terminal(now_ms()).await {
                tracing::warn!(%error, "failed to settle launch intent");
            }
        }
    }
}

#[derive(Debug)]
pub enum LaunchIntentError {
    Lease(LeaseError),
    Protocol(ProtocolError),
}

impl std::fmt::Display for LaunchIntentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchIntentError::Lease(error) => write!(f, "{error}"),
            LaunchIntentError::Protocol(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for LaunchIntentError {}

impl From<LeaseError> for LaunchIntentError {
    fn from(error: LeaseError) -> Self {
        LaunchIntentError::Lease(error)
    }
}

impl From<ProtocolError> for LaunchIntentError {
    fn from(error: ProtocolError) -> Self {
        LaunchIntentError::Protocol(error)
    }
}

impl LaunchIntent {
    pub async fn persist(
        leases: Arc<LeaseManager>,
        run: RunKey,
        at_ms: i64,
    ) -> Result<LaunchIntent, LaunchIntentError> {
        let lease = leases.acquire(&run_lease_key(&run), at_ms).await?;
        let mut renewal = LeaseRenewal::spawn(leases.clone(), lease);
        let protocol = leases.launch_protocol();
        let current = match protocol.persist_intent(renewal.holder(), &run, at_ms).await {
            Ok(current) => current,
            Err(error) => {
                let _ = renewal.shutdown().await;
                return Err(error.into());
            }
        };
        Ok(LaunchIntent {
            protocol,
            renewal,
            run,
            current,
        })
    }

    pub async fn persist_for_agent(
        app: &AppState,
        agent_id: &str,
        generation: u64,
    ) -> Option<LaunchIntent> {
        let settings = crate::file_activity::settings();
        Self::persist_for_agent_with_settings(app, agent_id, generation, &settings).await
    }

    async fn persist_for_agent_with_settings(
        app: &AppState,
        agent_id: &str,
        generation: u64,
        settings: &CoordinationSettings,
    ) -> Option<LaunchIntent> {
        let path = refact_work_core::default_store_path(&app.paths.cache_dir);
        if !settings.enabled {
            super::outbox::note_durability_unavailable(
                &path,
                "coordination is disabled".to_string(),
            );
            return None;
        }
        let Some(leases) = super::leases::leases_for_settings(app, settings).await else {
            return None;
        };
        let run = WorkKey::background_agent(agent_id).run(generation);
        match Self::persist(leases, run, now_ms()).await {
            Ok(intent) => Some(intent),
            Err(error) => {
                super::outbox::note_durability_unavailable(&path, error.to_string());
                None
            }
        }
    }

    pub async fn dispatched(&mut self, at_ms: i64) -> Result<(), LaunchIntentError> {
        self.advance(IntentState::Dispatched, at_ms).await
    }

    pub async fn acknowledged(&mut self, at_ms: i64) -> Result<(), LaunchIntentError> {
        self.advance(IntentState::Acknowledged, at_ms).await
    }

    pub async fn terminal(self, at_ms: i64) -> Result<(), LaunchIntentError> {
        let LaunchIntent {
            protocol,
            mut renewal,
            run: _,
            current,
        } = self;
        let advanced = protocol
            .advance(renewal.holder(), &current, IntentState::Terminal, at_ms)
            .await;
        let released = renewal.shutdown().await;
        advanced.map(|_| ())?;
        released?;
        Ok(())
    }

    pub fn state(&self) -> IntentState {
        self.current.state
    }

    pub fn run(&self) -> &RunKey {
        &self.run
    }

    async fn advance(&mut self, next: IntentState, at_ms: i64) -> Result<(), LaunchIntentError> {
        self.current = self
            .protocol
            .advance(self.renewal.holder(), &self.current, next, at_ms)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordination::leases::LeaseRenewal;
    use refact_work_core::identity::ProjectKey;
    use refact_work_core::store::CoordinationStore;
    use uuid::Uuid;

    const NOW_MS: i64 = 1_800_000_000_000;

    fn project_id() -> String {
        format!("launch-{}", Uuid::new_v4())
    }

    fn settings() -> CoordinationSettings {
        CoordinationSettings {
            enabled: true,
            lease_ttl_ms: 5_000,
            lease_renew_interval_ms: 4_000,
            ..CoordinationSettings::default()
        }
    }

    async fn store(settings: &CoordinationSettings) -> Arc<CoordinationStore> {
        Arc::new(CoordinationStore::open_in_memory(settings).await.unwrap())
    }

    fn manager(
        store: Arc<CoordinationStore>,
        project: &str,
        settings: &CoordinationSettings,
    ) -> Arc<LeaseManager> {
        let access = store.authorize(ProjectKey::new(project));
        Arc::new(LeaseManager::new(store, access, "test-engine", settings).unwrap())
    }

    fn run(agent_id: &str, generation: u64) -> RunKey {
        WorkKey::background_agent(agent_id).run(generation)
    }

    async fn intent_record(
        store: &CoordinationStore,
        project: &str,
        run: &RunKey,
    ) -> RunIntentRecord {
        store
            .get_run_intent(&store.authorize(ProjectKey::new(project)), run)
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn persist_is_idempotent_without_rewinding_progressed_intents() {
        let project = project_id();
        let settings = settings();
        let store = store(&settings).await;
        let leases = manager(store.clone(), &project, &settings);
        let run = run("agent-1", 1);
        let mut intent = LaunchIntent::persist(leases.clone(), run.clone(), NOW_MS)
            .await
            .unwrap();
        assert_eq!(intent.state(), IntentState::Persisted);
        intent.dispatched(NOW_MS + 1).await.unwrap();
        let progressed = intent_record(&store, &project, &run).await;
        intent.renewal.shutdown().await.unwrap();

        let duplicate = LaunchIntent::persist(leases, run.clone(), NOW_MS + 2)
            .await
            .unwrap();
        assert_eq!(duplicate.state(), IntentState::Dispatched);
        assert_eq!(intent_record(&store, &project, &run).await, progressed);
        duplicate.terminal(NOW_MS + 3).await.unwrap();
        assert_eq!(
            intent_record(&store, &project, &run).await.state,
            IntentState::Terminal
        );
    }

    #[tokio::test]
    async fn boundaries_advance_in_order_and_allow_an_early_terminal_state() {
        let project = project_id();
        let settings = settings();
        let store = store(&settings).await;
        let leases = manager(store.clone(), &project, &settings);
        let run_key = run("agent-2", 1);
        let mut intent = LaunchIntent::persist(leases.clone(), run_key.clone(), NOW_MS)
            .await
            .unwrap();
        intent.dispatched(NOW_MS + 1).await.unwrap();
        assert!(matches!(
            intent.dispatched(NOW_MS + 2).await,
            Err(LaunchIntentError::Protocol(
                ProtocolError::OutOfOrder { .. }
            ))
        ));
        intent.acknowledged(NOW_MS + 3).await.unwrap();
        assert!(matches!(
            intent.dispatched(NOW_MS + 4).await,
            Err(LaunchIntentError::Protocol(
                ProtocolError::OutOfOrder { .. }
            ))
        ));
        intent.terminal(NOW_MS + 5).await.unwrap();
        assert_eq!(
            intent_record(&store, &project, &run_key).await.state,
            IntentState::Terminal
        );

        let early = run("agent-3", 1);
        LaunchIntent::persist(leases, early.clone(), NOW_MS + 5)
            .await
            .unwrap()
            .terminal(NOW_MS + 6)
            .await
            .unwrap();
        assert_eq!(
            intent_record(&store, &project, &early).await.state,
            IntentState::Terminal
        );
    }

    #[tokio::test]
    async fn terminal_releases_the_lease() {
        let project = project_id();
        let settings = settings();
        let store = store(&settings).await;
        let leases = manager(store, &project, &settings);
        let run_key = run("agent-4", 1);
        let intent = LaunchIntent::persist(leases.clone(), run_key.clone(), NOW_MS)
            .await
            .unwrap();
        intent.terminal(NOW_MS + 1).await.unwrap();
        assert!(leases
            .inspect(&run_lease_key(&run_key), NOW_MS + 2)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn dropped_renewal_stops_renewing_and_lease_expires() {
        let project = project_id();
        let settings = CoordinationSettings {
            lease_ttl_ms: 80,
            lease_renew_interval_ms: 10,
            ..CoordinationSettings::default()
        };
        let store = store(&settings).await;
        let leases = manager(store, &project, &settings);
        let run = run("agent-5", 1);
        let holder = leases
            .acquire(&run_lease_key(&run), now_ms())
            .await
            .unwrap();
        let renewal = LeaseRenewal::spawn(leases.clone(), holder);
        drop(renewal);
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        let stale = leases
            .inspect(&run_lease_key(&run), now_ms())
            .await
            .unwrap()
            .unwrap();
        assert!(stale.expired_for_ms > 0);
    }

    #[tokio::test]
    async fn persist_for_agent_returns_none_when_coordination_is_disabled() {
        let workspace = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            workspace.path().join("cache"),
            config.path().to_path_buf(),
        )
        .await;
        let app = AppState::from_gcx(gcx).await;
        let disabled = CoordinationSettings {
            enabled: false,
            ..CoordinationSettings::default()
        };

        let slot =
            LaunchIntentSlot::persist_for_agent_with_settings(&app, "agent-disabled", 1, &disabled)
                .await;
        assert!(slot.is_empty());
        slot.dispatched().await;
        slot.acknowledged().await;
        slot.settle().await;
        let path = refact_work_core::default_store_path(&app.paths.cache_dir);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn slot_persists_advances_settles_and_releases_lease() {
        let project = project_id();
        let settings = settings();
        let store = store(&settings).await;
        let leases = manager(store.clone(), &project, &settings);
        let run_key = run("slot-lifecycle", 1);
        let intent = LaunchIntent::persist(leases.clone(), run_key.clone(), NOW_MS)
            .await
            .unwrap();
        let slot = LaunchIntentSlot::with_intent(intent);

        assert!(!slot.is_empty());
        assert_eq!(
            intent_record(&store, &project, &run_key).await.state,
            IntentState::Persisted
        );
        slot.dispatched().await;
        assert_eq!(
            intent_record(&store, &project, &run_key).await.state,
            IntentState::Dispatched
        );
        slot.acknowledged().await;
        assert_eq!(
            intent_record(&store, &project, &run_key).await.state,
            IntentState::Acknowledged
        );
        slot.settle().await;
        assert!(slot.is_empty());
        assert_eq!(
            intent_record(&store, &project, &run_key).await.state,
            IntentState::Terminal
        );
        assert!(leases
            .inspect(&run_lease_key(&run_key), NOW_MS + 3)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn concurrent_slot_settlement_transitions_once_and_later_boundaries_are_noops() {
        let project = project_id();
        let settings = settings();
        let store = store(&settings).await;
        let leases = manager(store.clone(), &project, &settings);
        let run_key = run("slot-concurrent", 1);
        let intent = LaunchIntent::persist(leases.clone(), run_key.clone(), NOW_MS)
            .await
            .unwrap();
        let slot = LaunchIntentSlot::with_intent(intent);
        let first = slot.clone();
        let second = slot.clone();

        tokio::join!(first.settle(), second.settle());
        slot.dispatched().await;

        assert!(slot.is_empty());
        assert_eq!(
            intent_record(&store, &project, &run_key).await.state,
            IntentState::Terminal
        );
        assert!(leases
            .inspect(&run_lease_key(&run_key), NOW_MS + 3)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn acknowledged_before_dispatched_does_not_panic_and_settles() {
        let project = project_id();
        let settings = settings();
        let store = store(&settings).await;
        let leases = manager(store.clone(), &project, &settings);
        let run_key = run("slot-out-of-order", 1);
        let intent = LaunchIntent::persist(leases, run_key.clone(), NOW_MS)
            .await
            .unwrap();
        let slot = LaunchIntentSlot::with_intent(intent);

        slot.acknowledged().await;
        assert_eq!(
            intent_record(&store, &project, &run_key).await.state,
            IntentState::Acknowledged
        );
        slot.settle().await;
        assert_eq!(
            intent_record(&store, &project, &run_key).await.state,
            IntentState::Terminal
        );
    }

    #[tokio::test]
    async fn registry_slot_clone_settles_shared_state_and_forget_removes_it() {
        let project = project_id();
        let settings = settings();
        let store = store(&settings).await;
        let leases = manager(store.clone(), &project, &settings);
        let run_key = run("slot-registry", 1);
        let intent = LaunchIntent::persist(leases, run_key.clone(), NOW_MS)
            .await
            .unwrap();
        let slot = LaunchIntentSlot::with_intent(intent);
        let registry = crate::agents::execution::ExecutionRegistry::default();

        registry.remember_launch_intent("slot-registry", slot.clone());
        let lookup = registry.launch_intent("slot-registry").unwrap();
        lookup.settle().await;
        assert!(slot.is_empty());
        assert_eq!(
            intent_record(&store, &project, &run_key).await.state,
            IntentState::Terminal
        );
        registry.forget_launch_intent("slot-registry");
        assert!(registry.launch_intent("slot-registry").is_none());
    }

    #[tokio::test]
    async fn background_agent_run_key_matches_reconciliation_record_lookup() {
        let project = project_id();
        let settings = settings();
        let store = store(&settings).await;
        let leases = manager(store.clone(), &project, &settings);
        let run_key = run("agent-roundtrip", 7);
        let intent = LaunchIntent::persist(leases, run_key.clone(), NOW_MS)
            .await
            .unwrap();
        assert_eq!(
            intent.run().work.runtime,
            refact_work_core::RuntimeKind::BackgroundAgent
        );
        assert_eq!(intent.run().work.id, "agent-roundtrip");
        assert_eq!(intent.run().attempt, 7);
        assert_eq!(intent_record(&store, &project, &run_key).await.run, run_key);
        intent.terminal(NOW_MS + 1).await.unwrap();
    }
}
