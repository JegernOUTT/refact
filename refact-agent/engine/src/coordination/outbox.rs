//! Durable delivery outbox for chat deliveries (M3).
//!
//! Transport is **at-least-once**: a delivery is written to the coordination
//! store before it is considered accepted, and anything that never reached its
//! boundary is redelivered after a restart. Insertion is **logically
//! exactly-once** because the recipient session deduplicates by delivery id —
//! the same envelope arriving twice is reported `Duplicate` and its messages
//! are appended once.
//!
//! This is not exactly-once delivery of external side effects and nothing here
//! claims it is. A message that was dispatched without its outcome being
//! recorded is left in `InFlight`, reported as needing inspection, and never
//! replayed automatically.
//!
//! Durability is additive. A store that cannot be opened or written degrades
//! delivery to in-memory at-least-once instead of failing it, and says so
//! through [`note_durability_unavailable`]; a message that was not persisted
//! is never reported as durable.
//!
//! Only deliveries that the chat-session transport owns are persisted here.
//! Deliveries routed to a live background-agent runner keep their single owner
//! in the agent registry; mirroring them would be a second transport for the
//! same message.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use refact_work_core::{
    default_store_path, CoordinationSettings, CoordinationStore, DeliveryOutboxRecord,
    DeliveryState, ProjectKey, StoreError, StoreWarning,
};
use tokio::sync::{Mutex as AMutex, OnceCell};

use crate::app_state::AppState;
use crate::chat::types::{DeliveryStatus, PendingDelivery};

/// Scope for chats that belong to no project checkout. It is a literal, so it
/// can never collide with a project identity hash.
pub const UNSCOPED_PROJECT: &str = "no-project";

const SHUTDOWN_POLL: Duration = Duration::from_millis(200);

/// Distinct durability failures retained for reporting; a store that keeps
/// failing must not grow this without bound.
const MAX_DEGRADED_WARNINGS: usize = 32;

#[derive(Debug)]
pub struct RecoveredDelivery {
    pub recipient: String,
    pub delivery: PendingDelivery,
}

/// What a restart found in the outbox: deliveries that provably never reached
/// the recipient, and messages whose outcome nobody recorded.
#[derive(Debug)]
pub struct OutboxRecovery {
    pub redeliver: Vec<RecoveredDelivery>,
    pub needs_inspection: Vec<DeliveryOutboxRecord>,
}

pub struct DeliveryOutbox {
    store: Arc<CoordinationStore>,
    access: refact_work_core::ProjectAccess,
}

impl DeliveryOutbox {
    pub async fn open(
        path: &Path,
        project: ProjectKey,
        settings: &CoordinationSettings,
    ) -> Result<Self, StoreError> {
        let store = Arc::new(CoordinationStore::open(path, settings).await?);
        Ok(DeliveryOutbox::with_store(store, project))
    }

    pub fn with_store(store: Arc<CoordinationStore>, project: ProjectKey) -> Self {
        let access = store.authorize(project);
        DeliveryOutbox { store, access }
    }

    /// Persist an accepted delivery as `Pending`. An existing row is left
    /// exactly as it is: a settled message must not be resurrected, and a
    /// message already in flight must not lose its unknown outcome.
    pub async fn persist_accepted(
        &self,
        recipient: &str,
        delivery: &PendingDelivery,
    ) -> Result<(), StoreError> {
        if self
            .store
            .get_outbox_message(&self.access, &delivery.id)
            .await?
            .is_some()
        {
            return Ok(());
        }
        let payload = serde_json::to_string(delivery)
            .map_err(|error| StoreError::Decode(format!("encode delivery payload: {error}")))?;
        let now = refact_work_core::now_ms();
        self.store
            .put_outbox_message(
                &self.access,
                &DeliveryOutboxRecord {
                    project: self.access.project().clone(),
                    message_id: delivery.id.clone(),
                    recipient: recipient.to_string(),
                    state: DeliveryState::Pending,
                    attempts: 0,
                    created_at_ms: now,
                    updated_at_ms: now,
                    payload,
                },
            )
            .await
    }

    /// Hand-off point. Everything between here and the recorded outcome is an
    /// unknown-outcome window that recovery refuses to replay.
    pub async fn mark_dispatched(&self, delivery_id: &str) -> Result<(), StoreError> {
        self.transition(delivery_id, DeliveryState::InFlight, true)
            .await
    }

    /// Persist and hand off in one step. `false` means durability could not be
    /// achieved: the caller proceeds without it rather than refusing to
    /// deliver, and the failure is reported through the store warning channel.
    pub async fn accept_for_dispatch(&self, recipient: &str, delivery: &PendingDelivery) -> bool {
        if let Err(error) = self.persist_accepted(recipient, delivery).await {
            note_durability_unavailable(self.store.path(), error.to_string());
            return false;
        }
        if let Err(error) = self.mark_dispatched(&delivery.id).await {
            note_durability_unavailable(self.store.path(), error.to_string());
            return false;
        }
        true
    }

    /// Record what the recipient itself reports. `Unknown` writes nothing: an
    /// outcome nobody knows must not be invented.
    pub async fn record_status(
        &self,
        delivery_id: &str,
        status: DeliveryStatus,
    ) -> Result<(), StoreError> {
        match state_for_status(status) {
            Some(state) => self.transition(delivery_id, state, false).await,
            None => Ok(()),
        }
    }

    async fn transition(
        &self,
        delivery_id: &str,
        state: DeliveryState,
        bump_attempts: bool,
    ) -> Result<(), StoreError> {
        let Some(mut record) = self
            .store
            .get_outbox_message(&self.access, delivery_id)
            .await?
        else {
            return Ok(());
        };
        if record.state.is_terminal() || record.state == state {
            let reason = match record.state == state {
                true => "already_in_state",
                false => "already_terminal",
            };
            super::metrics::record_delivery_duplicate(reason);
            return Ok(());
        }
        record.state = state;
        if bump_attempts {
            record.attempts = record.attempts.saturating_add(1);
        }
        let now_ms = refact_work_core::now_ms();
        super::metrics::record_delivery_lag(
            "transition",
            now_ms.saturating_sub(record.created_at_ms).max(0),
        );
        record.updated_at_ms = now_ms;
        self.store.put_outbox_message(&self.access, &record).await
    }

    pub async fn status_of(&self, delivery_id: &str) -> Result<Option<DeliveryState>, StoreError> {
        Ok(self
            .store
            .get_outbox_message(&self.access, delivery_id)
            .await?
            .map(|record| record.state))
    }

    /// Everything a restart has to act on. A row whose payload cannot be
    /// decoded is reported for inspection rather than dropped or guessed at.
    pub async fn recover(&self) -> Result<OutboxRecovery, StoreError> {
        let mut recovery = OutboxRecovery {
            redeliver: Vec::new(),
            needs_inspection: self
                .store
                .list_outbox_messages_in_states(&self.access, &[DeliveryState::InFlight])
                .await?,
        };
        for record in self
            .store
            .list_outbox_messages_in_states(&self.access, &[DeliveryState::Pending])
            .await?
        {
            match serde_json::from_str::<PendingDelivery>(&record.payload) {
                Ok(delivery) => {
                    super::metrics::record_recovery_decision("redeliver");
                    recovery.redeliver.push(RecoveredDelivery {
                        recipient: record.recipient.clone(),
                        delivery,
                    })
                }
                Err(error) => {
                    super::metrics::record_recovery_decision("needs_inspection");
                    tracing::warn!(
                        message_id = %record.message_id,
                        %error,
                        "outbox payload cannot be decoded; reporting it for inspection"
                    );
                    recovery.needs_inspection.push(record);
                }
            }
        }
        Ok(recovery)
    }

    /// Delete settled messages older than the retention window. A message with
    /// an unknown outcome is never deleted by age.
    pub async fn prune_settled(
        &self,
        now_ms: i64,
        retention_secs: u64,
    ) -> Result<usize, StoreError> {
        let window = i64::try_from(retention_secs.saturating_mul(1000)).unwrap_or(i64::MAX);
        let pruned = self
            .store
            .prune_settled_outbox_messages(&self.access, now_ms.saturating_sub(window))
            .await?;
        super::metrics::record_delivery_expiry("retention_window", pruned as u64);
        Ok(pruned)
    }
}

fn state_for_status(status: DeliveryStatus) -> Option<DeliveryState> {
    match status {
        DeliveryStatus::Pending => Some(DeliveryState::Pending),
        DeliveryStatus::Delivered => Some(DeliveryState::Delivered),
        DeliveryStatus::Consumed => Some(DeliveryState::Consumed),
        DeliveryStatus::Expired => Some(DeliveryState::Expired),
        DeliveryStatus::Superseded => Some(DeliveryState::Superseded),
        DeliveryStatus::Sealed | DeliveryStatus::Failed => Some(DeliveryState::Failed),
        DeliveryStatus::Unknown => None,
    }
}

type StoreCell = Arc<OnceCell<Arc<CoordinationStore>>>;

static STORES: OnceLock<AMutex<HashMap<PathBuf, StoreCell>>> = OnceLock::new();

/// One store per path per process. The registry lock is released before the
/// open, so no lock is ever held across store I/O.
pub(super) async fn store_for_path(
    path: &Path,
    settings: &CoordinationSettings,
) -> Result<Arc<CoordinationStore>, StoreError> {
    let cell = {
        let mut stores = STORES
            .get_or_init(|| AMutex::new(HashMap::new()))
            .lock()
            .await;
        stores
            .entry(path.to_path_buf())
            .or_insert_with(|| Arc::new(OnceCell::new()))
            .clone()
    };
    cell.get_or_try_init(|| async {
        let store = CoordinationStore::open(path, settings).await?;
        for warning in store.warnings() {
            tracing::warn!("coordination store: {warning}");
        }
        Ok(Arc::new(store))
    })
    .await
    .map(Arc::clone)
}

/// Report that durability is unavailable, through the store's own warning
/// channel. Nothing downstream may then describe the affected work as
/// persisted.
pub fn note_durability_unavailable(path: &Path, reason: String) {
    let warning = StoreWarning::DurabilityUnavailable {
        path: path.display().to_string(),
        reason,
    };
    tracing::warn!("coordination store: {warning}");
    let mut warnings = degraded_warnings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !warnings.contains(&warning) && warnings.len() < MAX_DEGRADED_WARNINGS {
        warnings.push(warning);
    }
}

/// Every distinct durability failure this process has reported.
pub fn durability_warnings() -> Vec<StoreWarning> {
    degraded_warnings()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn degraded_warnings() -> &'static Mutex<Vec<StoreWarning>> {
    static WARNINGS: OnceLock<Mutex<Vec<StoreWarning>>> = OnceLock::new();
    WARNINGS.get_or_init(|| Mutex::new(Vec::new()))
}

/// The engine's durable outbox for `project`, or `None` when there is none to
/// use. Durability is additive: coordination being disabled, and a store that
/// cannot be opened, both leave the caller with in-memory delivery rather than
/// no delivery. An unopenable store is reported, never passed off as working.
pub async fn outbox_for(app: &AppState, project: ProjectKey) -> Option<DeliveryOutbox> {
    let settings = crate::file_activity::settings();
    if !settings.enabled {
        return None;
    }
    let path = default_store_path(&app.paths.cache_dir);
    match store_for_path(&path, &settings).await {
        Ok(store) => Some(DeliveryOutbox::with_store(store, project)),
        Err(error) => {
            note_durability_unavailable(&path, error.to_string());
            None
        }
    }
}

pub async fn project_key_for_root(app: &AppState, source_root: Option<PathBuf>) -> ProjectKey {
    let root = match source_root {
        Some(root) => Some(root),
        None => crate::files_correction::get_project_dirs(app.gcx.clone())
            .await
            .into_iter()
            .next(),
    };
    root.map(|root| ProjectKey::new(crate::coordination::roots::project_identity(&root)))
        .unwrap_or_else(|| ProjectKey::new(UNSCOPED_PROJECT))
}

/// Redeliver everything that provably never landed and report what cannot be
/// retried without a human looking at it.
pub async fn recover_and_redeliver(app: &AppState, outbox: &DeliveryOutbox) -> usize {
    let recovery = match outbox.recover().await {
        Ok(recovery) => recovery,
        Err(error) => {
            tracing::warn!("outbox recovery failed: {error}");
            return 0;
        }
    };
    for record in &recovery.needs_inspection {
        tracing::warn!(
            message_id = %record.message_id,
            recipient = %record.recipient,
            attempts = record.attempts,
            "outbox message was dispatched without a recorded outcome; \
             it needs inspection and is not retried automatically"
        );
    }
    let mut redelivered = 0;
    for recovered in recovery.redeliver {
        match crate::chat::deliver_to_chat(
            app.clone(),
            &recovered.recipient,
            recovered.delivery.clone(),
        )
        .await
        {
            Ok(_) => redelivered += 1,
            Err(error) => tracing::warn!(
                message_id = %recovered.delivery.id,
                recipient = %recovered.recipient,
                %error,
                "outbox redelivery failed; the message stays in the outbox"
            ),
        }
    }
    redelivered
}

async fn wait_for_shutdown(shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::SeqCst) {
        tokio::time::sleep(SHUTDOWN_POLL).await;
    }
}

/// Owner of the recovery and retention work: recovers once at startup, then
/// prunes settled messages until the shutdown flag is raised.
pub async fn outbox_recovery_background_task(app: AppState, shutdown: Arc<AtomicBool>) {
    if shutdown.load(Ordering::SeqCst) {
        return;
    }
    let settings = crate::file_activity::settings();
    let project = project_key_for_root(&app, None).await;
    let Some(outbox) = outbox_for(&app, project).await else {
        return;
    };
    if shutdown.load(Ordering::SeqCst) {
        return;
    }
    let redelivered = recover_and_redeliver(&app, &outbox).await;
    if redelivered > 0 {
        tracing::info!("outbox redelivered {redelivered} messages after restart");
    }
    let interval = Duration::from_secs(settings.outbox_prune_interval_secs);
    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = wait_for_shutdown(shutdown.clone()) => return,
        }
        if shutdown.load(Ordering::SeqCst) {
            return;
        }
        match outbox
            .prune_settled(refact_work_core::now_ms(), settings.outbox_retention_secs)
            .await
        {
            Ok(pruned) if pruned > 0 => {
                tracing::info!("outbox retention pruned {pruned} settled messages")
            }
            Ok(_) => {}
            Err(error) => tracing::warn!("outbox retention failed: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::{ChatContent, ChatMessage};
    use crate::chat::types::{ChatSession, DeliveryOutcome, PushMode, ThreadParams};

    fn settings() -> CoordinationSettings {
        CoordinationSettings::default()
    }

    fn delivery(id: &str) -> PendingDelivery {
        PendingDelivery::with_id(
            id,
            vec![ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText(format!("durable {id}")),
                ..Default::default()
            }],
            PushMode::Append,
            "test",
            false,
        )
    }

    async fn open(path: &Path, project: &str) -> DeliveryOutbox {
        DeliveryOutbox::open(path, ProjectKey::new(project), &settings())
            .await
            .unwrap()
    }

    async fn reopen(outbox: DeliveryOutbox, path: &Path, project: &str) -> DeliveryOutbox {
        let DeliveryOutbox { store, .. } = outbox;
        let store = Arc::into_inner(store).expect("test holds the only store handle");
        store.close().await.unwrap();
        open(path, project).await
    }

    #[tokio::test]
    async fn outbox_restart_redelivers_a_pending_message_exactly_once_logically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("coordination.sqlite");
        let outbox = open(&path, "alpha").await;
        outbox
            .persist_accepted("chat-1", &delivery("accepted"))
            .await
            .unwrap();

        let outbox = reopen(outbox, &path, "alpha").await;
        let recovery = outbox.recover().await.unwrap();
        assert_eq!(recovery.redeliver.len(), 1);
        assert!(recovery.needs_inspection.is_empty());
        assert_eq!(recovery.redeliver[0].recipient, "chat-1");

        let mut session = ChatSession::new("chat-1".to_string());
        assert_eq!(
            session
                .enqueue_delivery(recovery.redeliver[0].delivery.clone())
                .unwrap(),
            DeliveryOutcome::Delivered
        );
        outbox
            .record_status("accepted", DeliveryStatus::Delivered)
            .await
            .unwrap();

        let restarted = reopen(outbox, &path, "alpha").await;
        let after = restarted.recover().await.unwrap();
        assert!(after.redeliver.is_empty());
        assert!(after.needs_inspection.is_empty());
        assert_eq!(session.messages.len(), 1);
    }

    #[tokio::test]
    async fn outbox_duplicate_redelivery_is_deduplicated_at_the_recipient() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("coordination.sqlite");
        let outbox = open(&path, "alpha").await;
        let envelope = delivery("repeat");
        outbox.persist_accepted("chat-1", &envelope).await.unwrap();

        let mut session = ChatSession::new("chat-1".to_string());
        assert_eq!(
            session.enqueue_delivery(envelope.clone()).unwrap(),
            DeliveryOutcome::Delivered
        );
        assert_eq!(
            session.enqueue_delivery(envelope.clone()).unwrap(),
            DeliveryOutcome::Duplicate
        );
        assert_eq!(session.messages.len(), 1);

        let recovered = reopen(outbox, &path, "alpha")
            .await
            .recover()
            .await
            .unwrap();
        let mut restarted = ChatSession::new_with_trajectory(
            "chat-1".to_string(),
            session.messages.clone(),
            ThreadParams::default(),
            "2026-01-01T00:00:00Z".to_string(),
            None,
            Vec::new(),
            None,
        );
        restarted.restore_pending_deliveries(Vec::new());
        assert_eq!(
            restarted
                .enqueue_delivery(recovered.redeliver[0].delivery.clone())
                .unwrap(),
            DeliveryOutcome::Duplicate,
            "a redelivered message must be inserted once logically"
        );
        assert_eq!(restarted.messages.len(), 1);
    }

    #[tokio::test]
    async fn outbox_settled_messages_stay_settled_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("coordination.sqlite");
        let outbox = open(&path, "alpha").await;
        for (id, status) in [
            ("gone", DeliveryStatus::Expired),
            ("replaced", DeliveryStatus::Superseded),
            ("landed", DeliveryStatus::Delivered),
        ] {
            outbox
                .persist_accepted("chat-1", &delivery(id))
                .await
                .unwrap();
            outbox.mark_dispatched(id).await.unwrap();
            outbox.record_status(id, status).await.unwrap();
        }
        outbox
            .record_status("replaced", DeliveryStatus::Delivered)
            .await
            .unwrap();

        let outbox = reopen(outbox, &path, "alpha").await;
        let recovery = outbox.recover().await.unwrap();
        assert!(recovery.redeliver.is_empty());
        assert!(recovery.needs_inspection.is_empty());
        assert_eq!(
            outbox.status_of("replaced").await.unwrap(),
            Some(DeliveryState::Superseded),
            "a settled outcome is never overwritten by a later one"
        );
        assert_eq!(
            outbox.status_of("gone").await.unwrap(),
            Some(DeliveryState::Expired)
        );
    }

    #[tokio::test]
    async fn outbox_unknown_outcome_is_surfaced_instead_of_being_retried() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("coordination.sqlite");
        let outbox = open(&path, "alpha").await;
        outbox
            .persist_accepted("chat-1", &delivery("dispatched"))
            .await
            .unwrap();
        outbox.mark_dispatched("dispatched").await.unwrap();

        let outbox = reopen(outbox, &path, "alpha").await;
        let recovery = outbox.recover().await.unwrap();
        assert!(
            recovery.redeliver.is_empty(),
            "a dispatched message with an unknown outcome is never replayed"
        );
        assert_eq!(recovery.needs_inspection.len(), 1);
        assert_eq!(recovery.needs_inspection[0].message_id, "dispatched");
        assert_eq!(recovery.needs_inspection[0].attempts, 1);
        assert_eq!(
            outbox.status_of("dispatched").await.unwrap(),
            Some(DeliveryState::InFlight)
        );
    }

    #[tokio::test]
    async fn outbox_retention_prunes_settled_but_never_unknown_outcomes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("coordination.sqlite");
        let outbox = open(&path, "alpha").await;
        for id in ["settled", "unknown", "queued"] {
            outbox
                .persist_accepted("chat-1", &delivery(id))
                .await
                .unwrap();
        }
        outbox.mark_dispatched("settled").await.unwrap();
        outbox
            .record_status("settled", DeliveryStatus::Delivered)
            .await
            .unwrap();
        outbox.mark_dispatched("unknown").await.unwrap();

        let future = refact_work_core::now_ms() + 10_000;
        assert_eq!(outbox.prune_settled(future, 1).await.unwrap(), 1);
        assert_eq!(outbox.status_of("settled").await.unwrap(), None);
        assert_eq!(
            outbox.status_of("unknown").await.unwrap(),
            Some(DeliveryState::InFlight)
        );
        assert_eq!(
            outbox.status_of("queued").await.unwrap(),
            Some(DeliveryState::Pending)
        );
    }

    #[tokio::test]
    async fn outbox_is_isolated_per_project() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("coordination.sqlite");
        let alpha = open(&path, "alpha").await;
        alpha
            .persist_accepted("chat-alpha", &delivery("alpha-only"))
            .await
            .unwrap();
        let beta = DeliveryOutbox::with_store(alpha.store.clone(), ProjectKey::new("beta"));
        beta.persist_accepted("chat-beta", &delivery("beta-only"))
            .await
            .unwrap();

        let alpha_recovery = alpha.recover().await.unwrap();
        let beta_recovery = beta.recover().await.unwrap();
        assert_eq!(alpha_recovery.redeliver.len(), 1);
        assert_eq!(alpha_recovery.redeliver[0].delivery.id, "alpha-only");
        assert_eq!(beta_recovery.redeliver.len(), 1);
        assert_eq!(beta_recovery.redeliver[0].delivery.id, "beta-only");
        assert_eq!(beta.status_of("alpha-only").await.unwrap(), None);
    }

    #[tokio::test]
    async fn outbox_deliver_to_chat_persists_before_accepting_and_settles() {
        let workspace = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            workspace.path().join("cache"),
            config.path().to_path_buf(),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let app = AppState::from_gcx(gcx).await;
        let session = Arc::new(AMutex::new(ChatSession::new("durable-chat".to_string())));
        app.chat
            .sessions
            .write()
            .await
            .insert("durable-chat".to_string(), session.clone());

        let queued = {
            session.lock().await.start_stream().unwrap();
            crate::chat::deliver_to_chat(app.clone(), "durable-chat", delivery("queued"))
                .await
                .unwrap()
        };
        assert_eq!(queued, DeliveryOutcome::Queued);
        session.lock().await.finish_stream(Some("stop".to_string()));
        assert_eq!(
            crate::chat::deliver_to_chat(app.clone(), "durable-chat", delivery("landed"))
                .await
                .unwrap(),
            DeliveryOutcome::Delivered
        );

        {
            let mut guard = session.lock().await;
            guard.start_stream().unwrap();
            let mut index = 0;
            while guard
                .enqueue_delivery(delivery(&format!("filler-{index}")))
                .is_ok()
            {
                index += 1;
                assert!(index < 1_000, "the pending queue must be bounded");
            }
        }
        assert!(
            crate::chat::deliver_to_chat(app.clone(), "durable-chat", delivery("refused"))
                .await
                .is_err()
        );

        let project = project_key_for_root(&app, None).await;
        let outbox = outbox_for(&app, project).await.unwrap();
        assert_eq!(
            outbox.status_of("refused").await.unwrap(),
            Some(DeliveryState::Failed),
            "a refused delivery has a known outcome and is never left dispatched"
        );
        assert_eq!(
            outbox.status_of("queued").await.unwrap(),
            Some(DeliveryState::Pending),
            "a delivery waiting for its boundary stays redeliverable"
        );
        assert_eq!(
            outbox.status_of("landed").await.unwrap(),
            Some(DeliveryState::Delivered)
        );
    }
}
