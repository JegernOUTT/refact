//! Explicit launch protocol with crash reconciliation at every boundary (M3).
//!
//! A launch crosses four durable boundaries, each one recorded before the next
//! has any effect:
//!
//! 1. [`IntentState::Persisted`] — the intent exists and nothing has happened.
//! 2. [`IntentState::Dispatched`] — a runtime record/claim exists; a child may
//!    now start.
//! 3. [`IntentState::Acknowledged`] — the child confirmed that it started.
//! 4. [`IntentState::Terminal`] — the outcome is recorded.
//!
//! The coordination store and the board/trajectory stores commit separately.
//! **There is no cross-store transaction and nothing here claims one.** What
//! makes the sequence safe is not atomicity but two other properties: every
//! boundary write is idempotent, and reconciliation classifies whatever a crash
//! left behind by what is *provable* rather than by what is likely.
//!
//! One rule shapes the whole module: an operation whose external effect is
//! unknown is surfaced for inspection, never retried. The window between
//! `Dispatched` and `Acknowledged` is exactly such an operation — the child may
//! or may not have started — so reconciliation refuses to start a second one
//! there, and says so instead.
//!
//! Ownership comes before any action. A record is only touched while this
//! engine holds its lease or may legitimately take it over under the M3 rules,
//! so reconciliation can never "clean up" work a live engine is still doing.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use refact_work_core::identity::RunKey;
use refact_work_core::settings::CoordinationSettings;
use refact_work_core::store::records::{now_ms, IntentState, RunIntentRecord};
use refact_work_core::store::{CoordinationStore, ProjectAccess, StoreError};
use refact_work_core::ProjectKey;
use serde::Serialize;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use super::leases::{LeaseError, LeaseHolder, LeaseManager, TakeoverRefusal};

/// What prompted a reconciliation pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileTrigger {
    Startup,
    Takeover,
}

/// An observation about a run, kept in three states on purpose. `Unknown` is
/// never collapsed into `No`: not knowing whether a child is running is not the
/// same as knowing that none is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Observation {
    Yes,
    No,
    Unknown,
}

/// What can be observed about a launch whose acknowledgement never landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LaunchEvidence {
    /// Whether a runner for this run is live right now, per M3 presence.
    pub runner_live: Observation,
    /// Whether the run's trajectory shows recorded progress, which proves the
    /// child did start.
    pub progress_observed: Observation,
}

impl LaunchEvidence {
    /// Nothing could be established either way.
    pub fn unknown() -> LaunchEvidence {
        LaunchEvidence {
            runner_live: Observation::Unknown,
            progress_observed: Observation::Unknown,
        }
    }

    pub fn new(runner_live: Observation, progress_observed: Observation) -> LaunchEvidence {
        LaunchEvidence {
            runner_live,
            progress_observed,
        }
    }
}

/// Why a record was handed to a human instead of being acted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectionReason {
    /// A runtime record exists and the launch was never acknowledged. Whether
    /// a child started is unknown, so nothing is started and nothing is
    /// retried.
    UnacknowledgedLaunch,
    /// The record and the world disagree — something is running that the
    /// record says was never dispatched. Which of the two is wrong cannot be
    /// derived, so it is surfaced for repair rather than guessed at.
    ContradictoryRecord,
}

impl InspectionReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            InspectionReason::UnacknowledgedLaunch => {
                "a runtime record exists but the launch was never acknowledged; \
                 whether a child started is unknown"
            }
            InspectionReason::ContradictoryRecord => {
                "work appears to be running for a record that says nothing was dispatched"
            }
        }
    }
}

/// What reconciliation decided about one record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// The intent is persisted and provably had no effect yet, so starting it
    /// is safe and starts exactly one child.
    StartFresh,
    /// A live runner owns this run. Reconciliation leaves it alone.
    LeaveRunning,
    /// The launch is known to have happened and no outcome was recorded. The
    /// owning runtime resumes it from its last completed durable boundary;
    /// reconciliation replays nothing itself.
    RecoverInterrupted,
    /// The outcome is recorded. Nothing to do beyond retention.
    Settle,
    /// Surfaced for inspection, never retried automatically.
    NeedsInspection(InspectionReason),
}

/// The one place a crash boundary turns into a decision.
///
/// Pure, so every row of the table below is asserted directly.
pub fn classify(state: IntentState, evidence: LaunchEvidence) -> Resolution {
    let running = evidence.runner_live == Observation::Yes;
    match state {
        // Nothing was dispatched, so nothing can be running. If something is,
        // the record is wrong and a human has to say which way.
        IntentState::Persisted => {
            if running || evidence.progress_observed == Observation::Yes {
                Resolution::NeedsInspection(InspectionReason::ContradictoryRecord)
            } else {
                Resolution::StartFresh
            }
        }
        // The unknown-effect window. A live runner or recorded progress proves
        // the launch landed; anything less leaves it unknown, and unknown is
        // never replayed.
        IntentState::Dispatched => {
            if running {
                Resolution::LeaveRunning
            } else if evidence.progress_observed == Observation::Yes {
                Resolution::RecoverInterrupted
            } else {
                Resolution::NeedsInspection(InspectionReason::UnacknowledgedLaunch)
            }
        }
        IntentState::Acknowledged => {
            if running {
                Resolution::LeaveRunning
            } else {
                Resolution::RecoverInterrupted
            }
        }
        IntentState::Terminal => Resolution::Settle,
    }
}

/// Position of a boundary in the launch sequence.
fn boundary_ordinal(state: IntentState) -> u8 {
    match state {
        IntentState::Persisted => 0,
        IntentState::Dispatched => 1,
        IntentState::Acknowledged => 2,
        IntentState::Terminal => 3,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    /// Boundaries only move forward. Repeating or regressing one would record
    /// a launch that did not happen.
    OutOfOrder { from: IntentState, to: IntentState },
    /// Every boundary write is lease-fenced, so every failure of one is
    /// expressed in lease terms — including a plain store failure.
    Lease(LeaseError),
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtocolError::OutOfOrder { from, to } => write!(
                f,
                "launch boundary `{}` cannot move to `{}`; boundaries only move forward",
                from.as_str(),
                to.as_str()
            ),
            ProtocolError::Lease(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<LeaseError> for ProtocolError {
    fn from(error: LeaseError) -> ProtocolError {
        ProtocolError::Lease(error)
    }
}

impl From<StoreError> for ProtocolError {
    fn from(error: StoreError) -> ProtocolError {
        ProtocolError::Lease(LeaseError::from(error))
    }
}

/// Writes the four launch boundaries, each one under the holder's fencing
/// generation so a replaced owner's write is rejected by the store.
pub struct LaunchProtocol {
    store: Arc<CoordinationStore>,
    access: ProjectAccess,
}

impl LaunchProtocol {
    pub fn new(store: Arc<CoordinationStore>, project: ProjectKey) -> LaunchProtocol {
        let access = store.authorize(project);
        LaunchProtocol { store, access }
    }

    /// Boundary 1. Idempotent by construction: an intent that already exists is
    /// returned exactly as it stands, so replaying this after a crash can never
    /// rewind a run that already moved on.
    pub async fn persist_intent(
        &self,
        holder: &LeaseHolder,
        run: &RunKey,
        at_ms: i64,
    ) -> Result<RunIntentRecord, ProtocolError> {
        Ok(self
            .store
            .insert_run_intent_fenced(&self.access, &holder.authority(), at_ms, run, at_ms)
            .await?)
    }

    /// Boundaries 2 to 4. An early settle is allowed — a launch refused before
    /// acknowledgement still has a known outcome — but a boundary is never
    /// repeated and never walked back.
    pub async fn advance(
        &self,
        holder: &LeaseHolder,
        current: &RunIntentRecord,
        to: IntentState,
        at_ms: i64,
    ) -> Result<RunIntentRecord, ProtocolError> {
        if boundary_ordinal(to) <= boundary_ordinal(current.state) {
            return Err(ProtocolError::OutOfOrder {
                from: current.state,
                to,
            });
        }
        Ok(self
            .store
            .compare_and_set_run_intent_fenced(
                &self.access,
                &holder.authority(),
                at_ms,
                &current.run,
                current.generation,
                to,
                at_ms,
            )
            .await?)
    }

    pub async fn state_of(&self, run: &RunKey) -> Result<Option<RunIntentRecord>, StoreError> {
        self.store.get_run_intent(&self.access, run).await
    }
}

/// Looks up what can be observed about a run right now.
pub type EvidenceProbe = Arc<dyn Fn(&RunKey) -> LaunchEvidence + Send + Sync>;

/// Starts a run that provably had no effect yet. Only ever called for
/// [`Resolution::StartFresh`].
///
/// The holder is handed over with the run so the launch can record its own
/// later boundaries under the same term of authority. Reconciliation already
/// wrote the runtime claim; acknowledging it belongs to whoever starts it.
pub type StartAction = Arc<
    dyn Fn(RunKey, LeaseHolder) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>>
        + Send
        + Sync,
>;

/// A record left alone because another engine legitimately owns it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ForeignRecord {
    pub run: RunKey,
    pub holder_instance: String,
    pub refusal: TakeoverRefusal,
}

/// A record handed to a human rather than acted on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InspectionItem {
    pub run: RunKey,
    pub state: IntentState,
    pub reason: InspectionReason,
    pub evidence: LaunchEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReconciliationReport {
    pub trigger: ReconcileTrigger,
    /// Runs that had no side effect yet and were started, exactly once each.
    pub started: Vec<RunKey>,
    /// Runs the owning runtime may resume from their last completed boundary.
    pub recovered: Vec<RunKey>,
    /// Runs whose outcome is already recorded.
    pub settled: Vec<RunKey>,
    /// Terminal records deleted because they aged past retention.
    pub pruned: usize,
    /// Runs a live runner is still executing.
    pub left_running: Vec<RunKey>,
    pub owned_elsewhere: Vec<ForeignRecord>,
    pub needs_inspection: Vec<InspectionItem>,
    /// Runs whose reconciliation could not complete. Their records are left
    /// where they stand so the next pass classifies them by evidence rather
    /// than by assumption.
    pub failed: Vec<(RunKey, String)>,
}

impl ReconciliationReport {
    fn new(trigger: ReconcileTrigger) -> ReconciliationReport {
        ReconciliationReport {
            trigger,
            started: Vec::new(),
            recovered: Vec::new(),
            settled: Vec::new(),
            pruned: 0,
            left_running: Vec::new(),
            owned_elsewhere: Vec::new(),
            needs_inspection: Vec::new(),
            failed: Vec::new(),
        }
    }

    pub fn acted_on(&self) -> usize {
        self.started.len() + self.recovered.len() + self.settled.len()
    }

    /// Say out loud what a human has to look at. An unknown outcome that nobody
    /// hears about is indistinguishable from one that never happened.
    pub fn report(&self) {
        for item in &self.needs_inspection {
            tracing::warn!(
                run = %item.run,
                state = item.state.as_str(),
                "reconciliation needs inspection: {}; it was not retried",
                item.reason.as_str()
            );
        }
        for foreign in &self.owned_elsewhere {
            tracing::debug!(
                run = %foreign.run,
                holder = %foreign.holder_instance,
                "reconciliation left a record owned by another engine alone: {}",
                foreign.refusal.as_str()
            );
        }
        for (run, error) in &self.failed {
            tracing::warn!(%run, "reconciliation could not finish this record: {error}");
        }
        if self.acted_on() > 0 || self.pruned > 0 {
            tracing::info!(
                "reconciliation ({:?}): {} started, {} recoverable, {} settled, {} pruned",
                self.trigger,
                self.started.len(),
                self.recovered.len(),
                self.settled.len(),
                self.pruned
            );
        }
    }
}

/// Whether reconciliation ran at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationOutcome {
    Ran(Box<ReconciliationReport>),
    /// The store could not be used. The engine keeps running without it and
    /// reconciliation says why instead of silently not happening.
    Skipped {
        reason: String,
    },
}

impl ReconciliationOutcome {
    pub fn report(&self) -> Option<&ReconciliationReport> {
        match self {
            ReconciliationOutcome::Ran(report) => Some(report),
            ReconciliationOutcome::Skipped { .. } => None,
        }
    }

    pub fn skip_reason(&self) -> Option<&str> {
        match self {
            ReconciliationOutcome::Skipped { reason } => Some(reason),
            ReconciliationOutcome::Ran(_) => None,
        }
    }
}

enum Ownership {
    Held(Box<LeaseHolder>),
    Foreign(Box<ForeignRecord>),
    Unavailable(String),
}

/// The lease one run's reconciliation is serialized on.
pub fn run_lease_key(run: &RunKey) -> String {
    format!("run:{run}")
}

/// Everything a reconciler needs that is not the store itself.
pub struct ReconcilerSetup {
    pub project: ProjectKey,
    pub instance_id: String,
    pub settings: CoordinationSettings,
    pub evidence: EvidenceProbe,
    pub start: StartAction,
}

pub struct Reconciler {
    store: Arc<CoordinationStore>,
    access: ProjectAccess,
    leases: Arc<LeaseManager>,
    protocol: LaunchProtocol,
    evidence: EvidenceProbe,
    start: StartAction,
    retention_ms: i64,
}

impl Reconciler {
    pub fn new(
        store: Arc<CoordinationStore>,
        setup: &ReconcilerSetup,
    ) -> Result<Reconciler, LeaseError> {
        let access = store.authorize(setup.project.clone());
        let leases = LeaseManager::new(
            store.clone(),
            access.clone(),
            setup.instance_id.clone(),
            &setup.settings,
        )?;
        Ok(Reconciler {
            protocol: LaunchProtocol::new(store.clone(), setup.project.clone()),
            store,
            access,
            leases: Arc::new(leases),
            evidence: setup.evidence.clone(),
            start: setup.start.clone(),
            retention_ms: i64::try_from(
                setup
                    .settings
                    .run_intent_retention_secs
                    .saturating_mul(1000),
            )
            .unwrap_or(i64::MAX),
        })
    }

    pub fn leases(&self) -> &Arc<LeaseManager> {
        &self.leases
    }

    pub fn protocol(&self) -> &LaunchProtocol {
        &self.protocol
    }

    /// One pass over every incomplete record of this project.
    ///
    /// Running it again produces the same result: the records it started have
    /// moved past `Persisted` and are classified by evidence on the next pass,
    /// and every other resolution is already a fixpoint.
    pub async fn reconcile(&self, trigger: ReconcileTrigger, at_ms: i64) -> ReconciliationReport {
        let mut report = ReconciliationReport::new(trigger);
        let records = match self.store.list_run_intents(&self.access).await {
            Ok(records) => records,
            Err(error) => {
                tracing::warn!("reconciliation could not read run intents: {error}");
                return report;
            }
        };
        for record in records {
            self.reconcile_one(record, at_ms, &mut report).await;
        }
        match self
            .store
            .prune_settled_run_intents(&self.access, at_ms.saturating_sub(self.retention_ms))
            .await
        {
            Ok(pruned) => report.pruned = pruned,
            Err(error) => tracing::warn!("reconciliation retention failed: {error}"),
        }
        report
    }

    async fn reconcile_one(
        &self,
        record: RunIntentRecord,
        at_ms: i64,
        report: &mut ReconciliationReport,
    ) {
        let run = record.run.clone();
        let ownership = self.own(&run, at_ms).await;
        let holder = match ownership {
            Ownership::Held(holder) => *holder,
            Ownership::Foreign(foreign) => {
                report.owned_elsewhere.push(*foreign);
                return;
            }
            Ownership::Unavailable(error) => {
                report.failed.push((run, error));
                return;
            }
        };
        let evidence = (self.evidence)(&run);
        match classify(record.state, evidence) {
            Resolution::StartFresh => self.start_fresh(&holder, &record, at_ms, report).await,
            Resolution::LeaveRunning => report.left_running.push(run),
            Resolution::RecoverInterrupted => report.recovered.push(run),
            Resolution::Settle => report.settled.push(run),
            Resolution::NeedsInspection(reason) => report.needs_inspection.push(InspectionItem {
                run,
                state: record.state,
                reason,
                evidence,
            }),
        }
        if let Err(error) = self.leases.release(&holder).await {
            tracing::debug!(lease_key = %holder.lease_key, "reconciliation lease release: {error}");
        }
    }

    /// Record the runtime claim first, then start. A crash in between leaves
    /// the unknown-effect window, which the next pass refuses to replay — that
    /// is the cost of never starting a second child, and it is the right one.
    async fn start_fresh(
        &self,
        holder: &LeaseHolder,
        record: &RunIntentRecord,
        at_ms: i64,
        report: &mut ReconciliationReport,
    ) {
        if let Err(error) = self
            .protocol
            .advance(holder, record, IntentState::Dispatched, at_ms)
            .await
        {
            report.failed.push((record.run.clone(), error.to_string()));
            return;
        }
        match (self.start)(record.run.clone(), holder.clone()).await {
            Ok(()) => report.started.push(record.run.clone()),
            Err(error) => report.failed.push((record.run.clone(), error)),
        }
    }

    /// Acquire this run's lease, or take it over from an engine presence says
    /// is gone. A holder that is alive, or that presence cannot classify, keeps
    /// its work: reconciliation never cleans up after a live engine.
    async fn own(&self, run: &RunKey, at_ms: i64) -> Ownership {
        let lease_key = run_lease_key(run);
        match self.leases.acquire(&lease_key, at_ms).await {
            Ok(holder) => Ownership::Held(Box::new(holder)),
            Err(LeaseError::Held(_)) => {
                match self
                    .leases
                    .take_over(&lease_key, "reconciling an incomplete launch", at_ms)
                    .await
                {
                    Ok(taken) => Ownership::Held(Box::new(taken.lease)),
                    Err(LeaseError::TakeoverRefused { refusal, holder }) => {
                        Ownership::Foreign(Box::new(ForeignRecord {
                            run: run.clone(),
                            holder_instance: holder.holder_instance.clone(),
                            refusal,
                        }))
                    }
                    Err(error) => Ownership::Unavailable(error.to_string()),
                }
            }
            Err(error) => Ownership::Unavailable(error.to_string()),
        }
    }
}

/// Reconcile the store at `path`, or report why it could not happen.
///
/// An unusable store costs durability, not functionality: the engine keeps
/// running and the skip is reported through the same channel every other
/// durability failure uses.
pub async fn reconcile_at_path(
    path: &Path,
    setup: &ReconcilerSetup,
    trigger: ReconcileTrigger,
    at_ms: i64,
) -> ReconciliationOutcome {
    let store = match super::outbox::store_for_path(path, &setup.settings).await {
        Ok(store) => store,
        Err(error) => {
            let reason = error.to_string();
            super::outbox::note_durability_unavailable(path, reason.clone());
            return ReconciliationOutcome::Skipped { reason };
        }
    };
    match Reconciler::new(store, setup) {
        Ok(reconciler) => {
            let report = reconciler.reconcile(trigger, at_ms).await;
            report.report();
            ReconciliationOutcome::Ran(Box::new(report))
        }
        Err(error) => {
            let reason = error.to_string();
            super::outbox::note_durability_unavailable(path, reason.clone());
            ReconciliationOutcome::Skipped { reason }
        }
    }
}

/// Owns the reconciliation task: one startup pass, then a pass per requested
/// takeover, with one cancellation path that shutdown joins.
pub struct ReconciliationService {
    cancel: Arc<Notify>,
    wake: Arc<Notify>,
    task: JoinHandle<()>,
}

impl ReconciliationService {
    pub fn spawn(reconciler: Arc<Reconciler>) -> ReconciliationService {
        let cancel = Arc::new(Notify::new());
        let wake = Arc::new(Notify::new());
        let task = tokio::spawn(reconcile_loop(reconciler, cancel.clone(), wake.clone()));
        ReconciliationService { cancel, wake, task }
    }

    /// Ask for a pass because a lease changed hands.
    pub fn request_takeover_pass(&self) {
        self.wake.notify_one();
    }

    /// Cancel the loop and join it. A detached reconciliation task could write
    /// after shutdown decided the engine was done.
    pub async fn shutdown(self) {
        self.cancel.notify_one();
        if let Err(error) = self.task.await {
            tracing::warn!("reconciliation task failed to join: {error}");
        }
    }
}

async fn reconcile_loop(reconciler: Arc<Reconciler>, cancel: Arc<Notify>, wake: Arc<Notify>) {
    let mut trigger = ReconcileTrigger::Startup;
    loop {
        reconciler.reconcile(trigger, now_ms()).await.report();
        let woken = wake.notified();
        tokio::pin!(woken);
        woken.as_mut().enable();
        tokio::select! {
            _ = cancel.notified() => return,
            _ = &mut woken => trigger = ReconcileTrigger::Takeover,
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, target_os = "linux"))]
mod crash_tests;
