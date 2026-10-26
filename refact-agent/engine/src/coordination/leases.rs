//! Durable leases with owner tokens and fencing generations (M3).
//!
//! Owning work is a *term*, not a flag: an acquisition hands out an owner token
//! and a fencing generation, and every coordinator-mediated write carries that
//! generation. Reclaiming a lease alone would not protect anything, because the
//! previous holder may still be running and still writing — so the generation
//! travels with the write and the store rejects a stale one.
//!
//! Three rules keep this honest:
//!
//! * Renewal extends the term and never changes the generation; it fences nobody.
//! * Takeover bumps the generation, which is exactly what fences the old holder.
//! * An expired lease has no authority at all. The next action fails with
//!   [`LeaseError::Expired`] instead of quietly continuing.
//!
//! Liveness is never re-derived here. Whether a stale holder may be taken over
//! is asked of engine presence (M3 / `presence.rs`), and anything it cannot
//! classify refuses the takeover: an unknown engine is not a dead one.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use refact_work_core::identity::RunKey;
use refact_work_core::settings::CoordinationSettings;
use refact_work_core::store::records::{
    now_ms, IntentState, LeaseAcquisition, LeaseAuthority, LeaseRecord, RunIntentRecord,
};
use refact_work_core::default_store_path;
use refact_work_core::store::{CoordinationStore, ProjectAccess, StoreError};
use serde::Serialize;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::presence::{self, PresenceState};

use crate::app_state::AppState;

/// Distinct takeovers kept in memory for inspection.
const MAX_TAKEOVER_JOURNAL: usize = 64;

/// A held lease: the owner token proves identity, the generation fences writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LeaseHolder {
    pub lease_key: String,
    pub holder_instance: String,
    pub owner_token: String,
    pub fencing_generation: i64,
    pub acquired_at_ms: i64,
    pub expires_at_ms: i64,
}

impl LeaseHolder {
    fn from_record(record: LeaseRecord) -> LeaseHolder {
        LeaseHolder {
            lease_key: record.lease_key,
            holder_instance: record.holder_instance,
            owner_token: record.owner_token,
            fencing_generation: record.fencing_generation,
            acquired_at_ms: record.acquired_at_ms,
            expires_at_ms: record.expires_at_ms,
        }
    }

    pub fn authority(&self) -> LeaseAuthority {
        LeaseAuthority {
            lease_key: self.lease_key.clone(),
            owner_token: self.owner_token.clone(),
            fencing_generation: self.fencing_generation,
        }
    }

    pub fn is_expired(&self, at_ms: i64) -> bool {
        at_ms >= self.expires_at_ms
    }
}

/// Who is in the way, since when, and what presence says about them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StaleHolder {
    pub lease_key: String,
    pub holder_instance: String,
    pub fencing_generation: i64,
    pub acquired_at_ms: i64,
    pub expires_at_ms: i64,
    pub held_for_ms: i64,
    /// Zero while the term is still running.
    pub expired_for_ms: i64,
    /// `None` when presence cannot classify the holder at all, which is not
    /// the same as classifying it dead.
    pub presence: Option<PresenceState>,
}

/// Why a takeover was refused. Ambiguity refuses; it never guesses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TakeoverRefusal {
    HolderAlive,
    HolderUnclassified,
    LeaseStillHeld,
}

impl TakeoverRefusal {
    pub fn as_str(&self) -> &'static str {
        match self {
            TakeoverRefusal::HolderAlive => "the holder is alive",
            TakeoverRefusal::HolderUnclassified => {
                "presence cannot classify the holder, and unknown is not dead"
            }
            TakeoverRefusal::LeaseStillHeld => "the lease has not been expired long enough",
        }
    }
}

/// Who took a lease over, from whom, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TakeoverRecord {
    pub project: String,
    pub lease_key: String,
    pub from: StaleHolder,
    pub to_instance: String,
    pub to_owner_token: String,
    pub generation_before: i64,
    pub generation_after: i64,
    pub reason: String,
    pub at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Takeover {
    pub lease: LeaseHolder,
    pub record: TakeoverRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseError {
    /// Someone else's lease is in the way; taking it needs an explicit takeover.
    Held(Box<StaleHolder>),
    NotHeld {
        lease_key: String,
    },
    Expired {
        lease_key: String,
        expires_at_ms: i64,
        at_ms: i64,
    },
    Fenced {
        lease_key: String,
        presented: i64,
        held: i64,
    },
    TakeoverRefused {
        refusal: TakeoverRefusal,
        holder: Box<StaleHolder>,
    },
    Store(StoreError),
}

impl std::fmt::Display for LeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LeaseError::Held(holder) => write!(
                f,
                "lease `{}` is held by engine `{}` at generation {} since {}ms ago",
                holder.lease_key,
                holder.holder_instance,
                holder.fencing_generation,
                holder.held_for_ms
            ),
            LeaseError::NotHeld { lease_key } => {
                write!(f, "lease `{lease_key}` is not held by this owner token")
            }
            LeaseError::Expired {
                lease_key,
                expires_at_ms,
                at_ms,
            } => write!(
                f,
                "lease `{lease_key}` expired at {expires_at_ms} and it is now {at_ms}; \
                 authority was lost and must be acquired again"
            ),
            LeaseError::Fenced {
                lease_key,
                presented,
                held,
            } => write!(
                f,
                "lease `{lease_key}` is at generation {held}; this write carried generation \
                 {presented} and was rejected"
            ),
            LeaseError::TakeoverRefused { refusal, holder } => write!(
                f,
                "refusing to take lease `{}` from engine `{}`: {}",
                holder.lease_key,
                holder.holder_instance,
                refusal.as_str()
            ),
            LeaseError::Store(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for LeaseError {}

impl From<StoreError> for LeaseError {
    fn from(error: StoreError) -> LeaseError {
        let rejected = match error {
            StoreError::LeaseNotHeld { lease_key } => LeaseError::NotHeld { lease_key },
            StoreError::LeaseExpired {
                lease_key,
                expires_at_ms,
                now_ms,
            } => LeaseError::Expired {
                lease_key,
                expires_at_ms,
                at_ms: now_ms,
            },
            StoreError::LeaseFenced {
                lease_key,
                presented,
                held,
            } => LeaseError::Fenced {
                lease_key,
                presented,
                held,
            },
            other => LeaseError::Store(other),
        };
        record_rejection(&rejected);
        rejected
    }
}

/// A write the store refused. Fencing and expiry are the two ways authority is
/// lost, and they stay apart: one is a stale generation, the other is a lease
/// that ran out.
fn record_rejection(error: &LeaseError) {
    use super::metrics::{explain, record_stale_write_rejection, Decision, Explanation};
    let (reason, rule, detail) = match error {
        LeaseError::Fenced {
            presented, held, ..
        } => (
            "fencing_generation",
            "leases.fencing_generation_behind_holder",
            format!("write carried generation {presented}, holder is at {held}"),
        ),
        LeaseError::Expired {
            expires_at_ms,
            at_ms,
            ..
        } => (
            "lease_expired",
            "leases.authority_expired",
            format!("lease expired at {expires_at_ms}, now {at_ms}"),
        ),
        LeaseError::NotHeld { .. } => (
            "not_held",
            "leases.owner_token_does_not_hold_lease",
            "the presented owner token does not hold this lease".to_string(),
        ),
        _ => return,
    };
    record_stale_write_rejection(reason);
    explain(Explanation::new(
        Decision::Denial,
        "rejected",
        rule,
        vec![
            ("reason".to_string(), detail),
            ("result".to_string(), reason.to_string()),
        ],
    ));
}

/// Leases for one project, owned by one engine instance.
pub struct LeaseManager {
    store: Arc<CoordinationStore>,
    access: ProjectAccess,
    instance_id: String,
    ttl_ms: i64,
    renew_interval: Duration,
    takeover_grace_ms: i64,
}

impl LeaseManager {
    pub fn new(
        store: Arc<CoordinationStore>,
        access: ProjectAccess,
        instance_id: impl Into<String>,
        settings: &CoordinationSettings,
    ) -> Result<LeaseManager, LeaseError> {
        settings
            .validate()
            .map_err(|error| LeaseError::Store(StoreError::Settings(error)))?;
        Ok(LeaseManager {
            store,
            access,
            instance_id: instance_id.into(),
            ttl_ms: settings.lease_ttl_ms as i64,
            renew_interval: Duration::from_millis(settings.lease_renew_interval_ms),
            takeover_grace_ms: settings.lease_takeover_grace_ms as i64,
        })
    }

    pub fn project(&self) -> &str {
        self.access.project().as_str()
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub fn renew_interval(&self) -> Duration {
        self.renew_interval
    }

    pub fn launch_protocol(&self) -> super::reconcile::LaunchProtocol {
        super::reconcile::LaunchProtocol::new(self.store.clone(), self.access.project().clone())
    }

    fn term_ends(&self, at_ms: i64) -> i64 {
        at_ms.saturating_add(self.ttl_ms)
    }

    /// Acquire `lease_key`, or report the holder standing in the way. An
    /// expired lease belonging to another engine is reported, never seized:
    /// that is what [`LeaseManager::take_over`] is for.
    pub async fn acquire(&self, lease_key: &str, at_ms: i64) -> Result<LeaseHolder, LeaseError> {
        let outcome = self
            .store
            .try_acquire_lease(
                &self.access,
                lease_key,
                &self.instance_id,
                &Uuid::new_v4().to_string(),
                at_ms,
                self.term_ends(at_ms),
            )
            .await?;
        match outcome {
            LeaseAcquisition::Acquired(record) => Ok(LeaseHolder::from_record(record)),
            LeaseAcquisition::Held(record) => {
                Err(LeaseError::Held(Box::new(self.diagnose(&record, at_ms))))
            }
        }
    }

    /// Extend the term. The generation is unchanged, so nothing is fenced.
    pub async fn renew(&self, holder: &LeaseHolder, at_ms: i64) -> Result<LeaseHolder, LeaseError> {
        let renewed = self
            .store
            .renew_lease(
                &self.access,
                &holder.authority(),
                at_ms,
                self.term_ends(at_ms),
            )
            .await?;
        Ok(LeaseHolder::from_record(renewed))
    }

    /// Give the lease up. `false` means this holder's row was already gone,
    /// which is what a fenced-out holder sees.
    pub async fn release(&self, holder: &LeaseHolder) -> Result<bool, LeaseError> {
        Ok(self
            .store
            .release_lease(&self.access, &holder.authority())
            .await?)
    }

    /// What is known about whoever holds `lease_key` right now.
    pub async fn inspect(
        &self,
        lease_key: &str,
        at_ms: i64,
    ) -> Result<Option<StaleHolder>, LeaseError> {
        Ok(self
            .store
            .get_lease(&self.access, lease_key)
            .await?
            .map(|record| self.diagnose(&record, at_ms)))
    }

    /// Take a lease from a holder presence says is gone. Refused while the
    /// holder is alive or unclassifiable, and always recorded when it happens.
    pub async fn take_over(
        &self,
        lease_key: &str,
        reason: &str,
        at_ms: i64,
    ) -> Result<Takeover, LeaseError> {
        let held = self
            .store
            .get_lease(&self.access, lease_key)
            .await?
            .ok_or_else(|| LeaseError::NotHeld {
                lease_key: lease_key.to_string(),
            })?;
        let holder = self.diagnose(&held, at_ms);
        if let Err(refusal) = authorize_takeover(&holder, at_ms, self.takeover_grace_ms) {
            return Err(LeaseError::TakeoverRefused {
                refusal,
                holder: Box::new(holder),
            });
        }
        let owner_token = Uuid::new_v4().to_string();
        let taken = self
            .store
            .take_over_lease(
                &self.access,
                lease_key,
                holder.fencing_generation,
                &self.instance_id,
                &owner_token,
                at_ms,
                self.term_ends(at_ms),
            )
            .await?;
        let record = TakeoverRecord {
            project: self.project().to_string(),
            lease_key: lease_key.to_string(),
            generation_before: holder.fencing_generation,
            generation_after: taken.fencing_generation,
            from: holder,
            to_instance: self.instance_id.clone(),
            to_owner_token: owner_token,
            reason: reason.to_string(),
            at_ms,
        };
        let abandoned = match record.from.expired_for_ms > 0 {
            true => "expired_holder_taken_over",
            false => "live_holder_taken_over",
        };
        super::metrics::record_abandoned_claim(abandoned);
        journal_takeover(&record);
        Ok(Takeover {
            lease: LeaseHolder::from_record(taken),
            record,
        })
    }

    /// A coordinator-mediated write, stamped with the holder's generation. The
    /// lease check and the write share one transaction, so a takeover cannot
    /// slip in between them.
    pub async fn set_run_intent(
        &self,
        holder: &LeaseHolder,
        run: &RunKey,
        expected_generation: i64,
        next_state: IntentState,
        at_ms: i64,
    ) -> Result<RunIntentRecord, LeaseError> {
        Ok(self
            .store
            .compare_and_set_run_intent_fenced(
                &self.access,
                &holder.authority(),
                at_ms,
                run,
                expected_generation,
                next_state,
                at_ms,
            )
            .await?)
    }

    fn diagnose(&self, record: &LeaseRecord, at_ms: i64) -> StaleHolder {
        StaleHolder {
            lease_key: record.lease_key.clone(),
            fencing_generation: record.fencing_generation,
            acquired_at_ms: record.acquired_at_ms,
            expires_at_ms: record.expires_at_ms,
            held_for_ms: at_ms.saturating_sub(record.acquired_at_ms).max(0),
            expired_for_ms: at_ms.saturating_sub(record.expires_at_ms).max(0),
            presence: holder_presence(self.project(), &record.holder_instance),
            holder_instance: record.holder_instance.clone(),
        }
    }
}

pub async fn leases_for(app: &AppState) -> Option<Arc<LeaseManager>> {
    let settings = crate::file_activity::settings();
    leases_for_settings(app, &settings).await
}

pub(super) async fn leases_for_settings(
    app: &AppState,
    settings: &CoordinationSettings,
) -> Option<Arc<LeaseManager>> {
    if !settings.enabled {
        return None;
    }
    let path = default_store_path(&app.paths.cache_dir);
    let project = super::outbox::project_key_for_root(app, None).await;
    match super::outbox::store_for_path(&path, &settings).await {
        Ok(store) => {
            let access = store.authorize(project);
            match LeaseManager::new(store, access, super::engine_instance_id(), &settings) {
                Ok(manager) => Some(Arc::new(manager)),
                Err(error) => {
                    super::outbox::note_durability_unavailable(&path, error.to_string());
                    None
                }
            }
        }
        Err(error) => {
            super::outbox::note_durability_unavailable(&path, error.to_string());
            None
        }
    }
}

/// What engine presence says about the engine behind a lease. `None` covers
/// both "no presence data" and "not a peer we know about"; neither is a claim.
fn holder_presence(project: &str, holder_instance: &str) -> Option<PresenceState> {
    presence::project_presence(project)?
        .peers
        .into_iter()
        .find(|peer| peer.instance_id == holder_instance)
        .map(|peer| peer.state)
}

/// The takeover policy, kept pure so every refusal is testable.
fn authorize_takeover(
    holder: &StaleHolder,
    at_ms: i64,
    grace_ms: i64,
) -> Result<(), TakeoverRefusal> {
    match holder.presence {
        Some(state) if state.is_alive() => Err(TakeoverRefusal::HolderAlive),
        None | Some(PresenceState::Unknown) => Err(TakeoverRefusal::HolderUnclassified),
        Some(_) if at_ms < holder.expires_at_ms.saturating_add(grace_ms) => {
            Err(TakeoverRefusal::LeaseStillHeld)
        }
        Some(_) => Ok(()),
    }
}

fn takeover_journal() -> &'static Mutex<VecDeque<TakeoverRecord>> {
    static JOURNAL: OnceLock<Mutex<VecDeque<TakeoverRecord>>> = OnceLock::new();
    JOURNAL.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn journal_takeover(record: &TakeoverRecord) {
    tracing::warn!(
        lease_key = %record.lease_key,
        from = %record.from.holder_instance,
        to = %record.to_instance,
        generation_before = record.generation_before,
        generation_after = record.generation_after,
        reason = %record.reason,
        "lease taken over from an engine presence reports as gone"
    );
    let mut journal = takeover_journal()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if journal.len() >= MAX_TAKEOVER_JOURNAL {
        journal.pop_front();
    }
    journal.push_back(record.clone());
}

/// Takeovers this engine performed for `project`, most recent last.
pub fn recent_takeovers(project: &str) -> Vec<TakeoverRecord> {
    takeover_journal()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .filter(|record| record.project == project)
        .cloned()
        .collect()
}

/// Owns the renewal task: one cancellation path, joined on shutdown.
pub struct LeaseRenewal {
    manager: Arc<LeaseManager>,
    holder: LeaseHolder,
    cancel: Arc<Notify>,
    task: Option<JoinHandle<()>>,
}

impl LeaseRenewal {
    pub fn spawn(manager: Arc<LeaseManager>, holder: LeaseHolder) -> LeaseRenewal {
        let cancel = Arc::new(Notify::new());
        let task = tokio::spawn(renewal_loop(
            manager.clone(),
            holder.clone(),
            cancel.clone(),
        ));
        LeaseRenewal {
            manager,
            holder,
            cancel,
            task: Some(task),
        }
    }

    pub fn holder(&self) -> &LeaseHolder {
        &self.holder
    }

    /// Cancel the loop, join it, then release the lease.
    pub async fn shutdown(&mut self) -> Result<bool, LeaseError> {
        self.cancel.notify_one();
        if let Some(task) = self.task.take() {
            if let Err(error) = task.await {
                tracing::warn!("lease renewal task failed to join: {error}");
            }
        }
        self.manager.release(&self.holder).await
    }
}

impl Drop for LeaseRenewal {
    fn drop(&mut self) {
        self.cancel.notify_one();
    }
}

async fn renewal_loop(manager: Arc<LeaseManager>, holder: LeaseHolder, cancel: Arc<Notify>) {
    loop {
        tokio::select! {
            _ = cancel.notified() => return,
            _ = tokio::time::sleep(manager.renew_interval()) => {}
        }
        match manager.renew(&holder, now_ms()).await {
            Ok(_) => {}
            Err(LeaseError::Store(error)) => {
                tracing::warn!(
                    lease_key = %holder.lease_key,
                    "lease renewal could not reach the coordination store: {error}"
                );
            }
            Err(error) => {
                tracing::warn!(
                    lease_key = %holder.lease_key,
                    "lease authority lost, renewal stops: {error}"
                );
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordination::presence::PeerPresence;
    use refact_work_core::identity::{ProjectKey, WorkKey};
    use refact_work_core::store::records::RunIntentRecord;

    const NOW_MS: i64 = 1_800_000_000_000;
    const TTL_MS: i64 = 1_000;
    const GRACE_MS: i64 = 500;

    fn settings() -> CoordinationSettings {
        CoordinationSettings {
            lease_ttl_ms: TTL_MS as u64,
            lease_renew_interval_ms: 20,
            lease_takeover_grace_ms: GRACE_MS as u64,
            ..CoordinationSettings::default()
        }
    }

    fn project_id() -> String {
        format!("project-{}", Uuid::new_v4())
    }

    async fn store() -> Arc<CoordinationStore> {
        Arc::new(
            CoordinationStore::open_in_memory(&settings())
                .await
                .unwrap(),
        )
    }

    fn manager(store: Arc<CoordinationStore>, project: &str, instance: &str) -> LeaseManager {
        let access = store.authorize(ProjectKey::new(project));
        LeaseManager::new(store, access, instance, &settings()).unwrap()
    }

    fn publish(project: &str, instance: &str, state: PresenceState) {
        presence::publish(
            project,
            vec![PeerPresence {
                instance_id: instance.to_string(),
                state,
            }],
            NOW_MS,
        );
    }

    fn run() -> RunKey {
        RunKey::new(WorkKey::trajectory("run-1"), 1)
    }

    async fn seed_intent(store: &CoordinationStore, project: &str) {
        let access = store.authorize(ProjectKey::new(project));
        store
            .put_run_intent(
                &access,
                &RunIntentRecord {
                    project: ProjectKey::new(project),
                    run: run(),
                    generation: 0,
                    state: IntentState::Persisted,
                    updated_at_ms: NOW_MS,
                },
            )
            .await
            .unwrap();
    }

    async fn stored_intent(store: &CoordinationStore, project: &str) -> RunIntentRecord {
        let access = store.authorize(ProjectKey::new(project));
        store
            .get_run_intent(&access, &run())
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn two_acquirers_race_and_exactly_one_owns_the_lease() {
        let project = project_id();
        let store = store().await;
        let one = manager(store.clone(), &project, "engine-1");
        let two = manager(store.clone(), &project, "engine-2");

        let (first, second) =
            tokio::join!(one.acquire("build", NOW_MS), two.acquire("build", NOW_MS));

        let winners = u8::from(first.is_ok()) + u8::from(second.is_ok());
        assert_eq!(winners, 1, "exactly one acquirer may own the lease");
        let loser = if first.is_err() {
            first.unwrap_err()
        } else {
            second.unwrap_err()
        };
        match loser {
            LeaseError::Held(holder) => {
                assert_eq!(holder.lease_key, "build");
                assert_eq!(holder.fencing_generation, 1);
                assert!(holder.holder_instance.starts_with("engine-"));
            }
            other => panic!("the loser must get a structured error, got {other}"),
        }
    }

    #[tokio::test]
    async fn a_replaced_holders_fenced_write_is_rejected_and_never_lands() {
        let project = project_id();
        let store = store().await;
        seed_intent(&store, &project).await;
        let old = manager(store.clone(), &project, "engine-1");
        let new = manager(store.clone(), &project, "engine-2");
        let held = old.acquire("run-1", NOW_MS).await.unwrap();
        let landed = old
            .set_run_intent(&held, &run(), 0, IntentState::Dispatched, NOW_MS + 1)
            .await
            .unwrap();

        publish(&project, "engine-1", PresenceState::Unreachable);
        let takeover_at = NOW_MS + TTL_MS + GRACE_MS;
        let taken = new
            .take_over("run-1", "holder unreachable for a full term", takeover_at)
            .await
            .unwrap();
        let stale = old
            .set_run_intent(
                &held,
                &run(),
                landed.generation,
                IntentState::Terminal,
                takeover_at + 1,
            )
            .await
            .unwrap_err();
        let after_rejection = stored_intent(&store, &project).await;
        let fresh = new
            .set_run_intent(
                &taken.lease,
                &run(),
                landed.generation,
                IntentState::Acknowledged,
                takeover_at + 2,
            )
            .await
            .unwrap();
        presence::forget(&project);

        assert_eq!(landed.state, IntentState::Dispatched);
        assert_eq!(taken.lease.fencing_generation, held.fencing_generation + 1);
        assert!(
            matches!(
                stale,
                LeaseError::Fenced {
                    presented: 1,
                    held: 2,
                    ..
                }
            ),
            "got {stale}"
        );
        assert_eq!(
            after_rejection, landed,
            "the fenced write must not land, not even partially"
        );
        assert_eq!(fresh.state, IntentState::Acknowledged);
    }

    #[tokio::test]
    async fn an_expired_lease_loses_authority_at_the_next_action() {
        let project = project_id();
        let store = store().await;
        seed_intent(&store, &project).await;
        let engine = manager(store.clone(), &project, "engine-1");
        let held = engine.acquire("run-1", NOW_MS).await.unwrap();
        let after_expiry = held.expires_at_ms;

        let write = engine
            .set_run_intent(&held, &run(), 0, IntentState::Dispatched, after_expiry)
            .await
            .unwrap_err();
        let renewal = engine.renew(&held, after_expiry).await.unwrap_err();
        let unchanged = stored_intent(&store, &project).await;

        assert!(
            matches!(write, LeaseError::Expired { expires_at_ms, .. } if expires_at_ms == after_expiry),
            "got {write}"
        );
        assert!(
            matches!(renewal, LeaseError::Expired { .. }),
            "got {renewal}"
        );
        assert_eq!(unchanged.state, IntentState::Persisted);
        assert_eq!(unchanged.generation, 0);
    }

    #[tokio::test]
    async fn takeover_is_refused_from_a_live_or_unclassified_holder_and_recorded_otherwise() {
        let project = project_id();
        let store = store().await;
        let old = manager(store.clone(), &project, "engine-1");
        let new = manager(store.clone(), &project, "engine-2");
        let held = old.acquire("build", NOW_MS).await.unwrap();
        let takeover_at = NOW_MS + TTL_MS + GRACE_MS;

        let unpublished = new
            .take_over("build", "no presence data", takeover_at)
            .await;
        publish(&project, "engine-1", PresenceState::AliveActive);
        let alive = new
            .take_over("build", "holder looks stuck", takeover_at)
            .await;
        publish(&project, "engine-1", PresenceState::Unknown);
        let unsure = new
            .take_over("build", "holder might be gone", takeover_at)
            .await;
        publish(&project, "engine-1", PresenceState::Unreachable);
        let too_early = new
            .take_over("build", "holder unreachable", takeover_at - 1)
            .await;
        let taken = new
            .take_over("build", "holder unreachable for a full term", takeover_at)
            .await
            .unwrap();
        let journal = recent_takeovers(&project);
        presence::forget(&project);

        for (outcome, expected) in [
            (unpublished, TakeoverRefusal::HolderUnclassified),
            (alive, TakeoverRefusal::HolderAlive),
            (unsure, TakeoverRefusal::HolderUnclassified),
            (too_early, TakeoverRefusal::LeaseStillHeld),
        ] {
            match outcome.unwrap_err() {
                LeaseError::TakeoverRefused { refusal, holder } => {
                    assert_eq!(refusal, expected);
                    assert_eq!(holder.holder_instance, "engine-1");
                }
                other => panic!("expected {expected:?}, got {other}"),
            }
        }
        assert_eq!(taken.record.from.holder_instance, "engine-1");
        assert_eq!(taken.record.from.presence, Some(PresenceState::Unreachable));
        assert_eq!(taken.record.to_instance, "engine-2");
        assert_eq!(taken.record.generation_before, held.fencing_generation);
        assert_eq!(taken.record.generation_after, held.fencing_generation + 1);
        assert_eq!(taken.record.reason, "holder unreachable for a full term");
        assert_eq!(journal, vec![taken.record]);
    }

    #[tokio::test]
    async fn another_projects_lease_of_the_same_name_is_untouched() {
        let alpha = project_id();
        let beta = project_id();
        let store = store().await;
        let one = manager(store.clone(), &alpha, "engine-1");
        let two = manager(store.clone(), &beta, "engine-2");

        let here = one.acquire("build", NOW_MS).await.unwrap();
        let there = two.acquire("build", NOW_MS).await.unwrap();
        assert!(one.release(&here).await.unwrap());

        assert_ne!(here.owner_token, there.owner_token);
        assert_eq!(here.fencing_generation, there.fencing_generation);
        assert_eq!(one.inspect("build", NOW_MS).await.unwrap(), None);
        assert_eq!(
            two.inspect("build", NOW_MS)
                .await
                .unwrap()
                .unwrap()
                .holder_instance,
            "engine-2"
        );
        assert!(recent_takeovers(&alpha).is_empty());
    }

    #[tokio::test]
    async fn the_renewal_task_is_cancelled_and_joined_on_shutdown() {
        let project = project_id();
        let store = store().await;
        let engine = Arc::new(manager(store.clone(), &project, "engine-1"));
        let holder = engine.acquire("build", now_ms()).await.unwrap();
        let mut renewal = LeaseRenewal::spawn(engine.clone(), holder.clone());

        let mut renewed = holder.clone();
        while renewed.expires_at_ms == holder.expires_at_ms {
            tokio::time::sleep(Duration::from_millis(25)).await;
            renewed = engine
                .inspect("build", now_ms())
                .await
                .unwrap()
                .map(|stale| LeaseHolder {
                    expires_at_ms: stale.expires_at_ms,
                    ..holder.clone()
                })
                .unwrap();
        }
        tokio::time::timeout(Duration::from_secs(5), renewal.shutdown())
            .await
            .expect("the renewal task must be joined, not detached")
            .unwrap();
        let after_shutdown = engine.inspect("build", now_ms()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert_eq!(renewed.fencing_generation, holder.fencing_generation);
        assert_eq!(after_shutdown, None, "shutdown releases the lease it held");
        assert_eq!(
            engine.inspect("build", now_ms()).await.unwrap(),
            None,
            "a joined renewal task cannot write after shutdown"
        );
    }
}
