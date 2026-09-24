use std::sync::{Arc, Mutex};
use std::time::Duration;

use refact_work_core::identity::{RunKey, WorkKey};
use refact_work_core::settings::CoordinationSettings;
use refact_work_core::store::records::{IntentState, RunIntentRecord};
use refact_work_core::store::CoordinationStore;
use refact_work_core::ProjectKey;
use uuid::Uuid;

use super::*;
use crate::coordination::leases::{self, LeaseError, LeaseManager};
use crate::coordination::presence::{self, PeerPresence, PresenceState};

const NOW_MS: i64 = 1_800_000_000_000;
const TTL_MS: i64 = 1_000;
const GRACE_MS: i64 = 500;
/// When recovery runs: after the crashed owner's whole term plus its grace,
/// which is the earliest a takeover is allowed at all.
const RECOVER_AT_MS: i64 = NOW_MS + TTL_MS + GRACE_MS;

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

fn run(id: &str) -> RunKey {
    RunKey::new(WorkKey::background_agent(id), 1)
}

async fn store() -> Arc<CoordinationStore> {
    Arc::new(
        CoordinationStore::open_in_memory(&settings())
            .await
            .unwrap(),
    )
}

fn manager(store: Arc<CoordinationStore>, project: &str, instance: &str) -> Arc<LeaseManager> {
    let access = store.authorize(ProjectKey::new(project));
    Arc::new(LeaseManager::new(store, access, instance, &settings()).unwrap())
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

/// A probe that answers the same way for every run.
fn probe(runner_live: Observation, progress_observed: Observation) -> EvidenceProbe {
    Arc::new(move |_| LaunchEvidence::new(runner_live, progress_observed))
}

/// Counts what reconciliation actually started, so "exactly one child" is a
/// measured fact rather than a claim.
#[derive(Clone, Default)]
struct StartLog(Arc<Mutex<Vec<RunKey>>>);

impl StartLog {
    fn action(&self) -> StartAction {
        let log = self.0.clone();
        Arc::new(move |run, _holder| {
            log.lock().unwrap().push(run);
            Box::pin(async { Ok(()) })
        })
    }

    fn started(&self) -> Vec<RunKey> {
        self.0.lock().unwrap().clone()
    }
}

fn setup(
    project: &str,
    instance: &str,
    evidence: EvidenceProbe,
    start: StartAction,
) -> ReconcilerSetup {
    ReconcilerSetup {
        project: ProjectKey::new(project),
        instance_id: instance.to_string(),
        settings: settings(),
        evidence,
        start,
    }
}

/// Walk a launch to `boundary` as an engine that then stops existing: the
/// lease it held is deliberately left behind, exactly as a crash leaves it.
async fn crashed_launch(
    store: &Arc<CoordinationStore>,
    project: &str,
    run: &RunKey,
    boundary: IntentState,
) -> (LeaseHolder, RunIntentRecord) {
    let leases = manager(store.clone(), project, "engine-crashed");
    let protocol = LaunchProtocol::new(store.clone(), ProjectKey::new(project));
    let holder = leases.acquire(&run_lease_key(run), NOW_MS).await.unwrap();
    let mut record = protocol.persist_intent(&holder, run, NOW_MS).await.unwrap();
    for step in [
        IntentState::Dispatched,
        IntentState::Acknowledged,
        IntentState::Terminal,
    ] {
        if boundary_ordinal(boundary) < boundary_ordinal(step) {
            break;
        }
        record = protocol
            .advance(&holder, &record, step, NOW_MS)
            .await
            .unwrap();
    }
    assert_eq!(record.state, boundary);
    (holder, record)
}

async fn state_of(store: &Arc<CoordinationStore>, project: &str, run: &RunKey) -> IntentState {
    LaunchProtocol::new(store.clone(), ProjectKey::new(project))
        .state_of(run)
        .await
        .unwrap()
        .unwrap()
        .state
}

/// Persist an intent and hand the lease back cleanly, leaving a record whose
/// owner is gone without a stale lease standing in the way.
async fn seed_released_intent(store: &Arc<CoordinationStore>, project: &str, run: &RunKey) {
    let leases = manager(store.clone(), project, "engine-previous");
    let protocol = LaunchProtocol::new(store.clone(), ProjectKey::new(project));
    let at_ms = refact_work_core::now_ms();
    let holder = leases.acquire(&run_lease_key(run), at_ms).await.unwrap();
    protocol.persist_intent(&holder, run, at_ms).await.unwrap();
    assert!(leases.release(&holder).await.unwrap());
}

#[tokio::test]
async fn surface_only_reconciliation_surfaces_persisted_and_unknown_without_starting() {
    let project = project_id();
    let store = store().await;
    let persisted = run("persisted-without-policy");
    let dispatched = run("dispatched-unknown");
    seed_released_intent(&store, &project, &persisted).await;
    crashed_launch(&store, &project, &dispatched, IntentState::Dispatched).await;
    publish(&project, "engine-crashed", PresenceState::Unreachable);

    let log = StartLog::default();
    let reconciler = Reconciler::new_surface_only(
        store.clone(),
        &setup(
            &project,
            "engine-recovering",
            probe(Observation::Unknown, Observation::Unknown),
            log.action(),
        ),
    )
    .unwrap();
    let report = reconciler
        .reconcile(ReconcileTrigger::Startup, RECOVER_AT_MS)
        .await;
    presence::forget(&project);

    assert!(log.started().is_empty());
    assert_eq!(report.needs_inspection.len(), 2, "{report:?}");
    assert!(report.needs_inspection.iter().any(|item| {
        item.run == persisted
            && item.state == IntentState::Persisted
            && item.reason == InspectionReason::NoAuthorizedResumePolicy
    }));
    assert!(report.needs_inspection.iter().any(|item| {
        item.run == dispatched
            && item.state == IntentState::Dispatched
            && item.reason == InspectionReason::UnacknowledgedLaunch
            && item.evidence == LaunchEvidence::unknown()
    }));
    assert_eq!(
        state_of(&store, &project, &persisted).await,
        IntentState::Persisted
    );
    assert_eq!(
        state_of(&store, &project, &dispatched).await,
        IntentState::Dispatched
    );
}

#[serial_test::serial(runtime_settings)]
#[tokio::test]
async fn production_startup_owns_reconciliation_and_skips_when_disabled() {
    crate::runtime_settings::reset_for_test();
    let workspace = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
        workspace.path().join("cache"),
        config.path().to_path_buf(),
    )
    .await;
    *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.path().to_path_buf()];
    let app = crate::app_state::AppState::from_gcx(gcx).await;

    let service = super::start_background_reconciliation(&app)
        .await
        .expect("enabled coordination starts reconciliation");
    tokio::time::timeout(Duration::from_secs(5), service.shutdown())
        .await
        .expect("shutdown joins reconciliation");

    let disabled = crate::tools::settings_guard::SettingsGuard::install(|settings| {
        settings.coordination.enabled = false;
    });
    assert!(super::start_background_reconciliation(&app).await.is_none());
    drop(disabled);
    crate::runtime_settings::reset_for_test();
}

#[test]
fn every_boundary_and_every_piece_of_evidence_has_one_decided_resolution() {
    use InspectionReason::*;
    use IntentState::*;
    use Observation::{No, Unknown, Yes};

    let table = [
        // Nothing happened yet, so starting is the only thing that is safe.
        (Persisted, No, No, Resolution::StartFresh),
        (Persisted, No, Unknown, Resolution::StartFresh),
        (Persisted, Unknown, No, Resolution::StartFresh),
        // ... unless something is demonstrably running that the record says
        // was never dispatched. That contradiction is repaired, not guessed.
        (
            Persisted,
            Yes,
            No,
            Resolution::NeedsInspection(ContradictoryRecord),
        ),
        (
            Persisted,
            No,
            Yes,
            Resolution::NeedsInspection(ContradictoryRecord),
        ),
        // The unknown-effect window: no live runner and no recorded progress
        // means nobody can say whether a child started.
        (
            Dispatched,
            No,
            No,
            Resolution::NeedsInspection(UnacknowledgedLaunch),
        ),
        (
            Dispatched,
            Unknown,
            Unknown,
            Resolution::NeedsInspection(UnacknowledgedLaunch),
        ),
        (
            Dispatched,
            Unknown,
            No,
            Resolution::NeedsInspection(UnacknowledgedLaunch),
        ),
        (
            Dispatched,
            No,
            Unknown,
            Resolution::NeedsInspection(UnacknowledgedLaunch),
        ),
        // Evidence closes the window in one of two directions, never by guess.
        (Dispatched, Yes, No, Resolution::LeaveRunning),
        (Dispatched, No, Yes, Resolution::RecoverInterrupted),
        (Acknowledged, No, No, Resolution::RecoverInterrupted),
        (
            Acknowledged,
            Unknown,
            Unknown,
            Resolution::RecoverInterrupted,
        ),
        (Acknowledged, Yes, Yes, Resolution::LeaveRunning),
        (Terminal, No, No, Resolution::Settle),
        (Terminal, Yes, Yes, Resolution::Settle),
    ];

    for (state, runner_live, progress, expected) in table {
        let evidence = LaunchEvidence::new(runner_live, progress);
        assert_eq!(
            classify(state, evidence),
            expected,
            "{state:?} with {evidence:?}"
        );
        assert_eq!(
            classify(state, evidence),
            classify(state, evidence),
            "classification must not depend on when it is asked"
        );
    }

    assert!(
        !table.iter().any(|(state, runner, progress, resolution)| {
            *state == Dispatched
                && !(*runner == Yes || *progress == Yes)
                && *resolution == Resolution::StartFresh
        }),
        "an unacknowledged launch may never be started again"
    );
}

#[tokio::test]
async fn launch_boundaries_are_idempotent_forward_only_and_never_repeated() {
    let project = project_id();
    let store = store().await;
    let leases = manager(store.clone(), &project, "engine-1");
    let protocol = LaunchProtocol::new(store.clone(), ProjectKey::new(&project));
    let run = run("ordered");
    let holder = leases.acquire(&run_lease_key(&run), NOW_MS).await.unwrap();

    let persisted = protocol
        .persist_intent(&holder, &run, NOW_MS)
        .await
        .unwrap();
    let dispatched = protocol
        .advance(&holder, &persisted, IntentState::Dispatched, NOW_MS + 1)
        .await
        .unwrap();
    let replayed = protocol
        .persist_intent(&holder, &run, NOW_MS + 2)
        .await
        .unwrap();

    assert_eq!(persisted.state, IntentState::Persisted);
    assert_eq!(persisted.generation, 0);
    assert_eq!(dispatched.state, IntentState::Dispatched);
    assert_eq!(dispatched.generation, 1);
    assert_eq!(
        replayed, dispatched,
        "replaying the first boundary must never rewind a run that moved on"
    );

    for backwards in [IntentState::Persisted, IntentState::Dispatched] {
        assert_eq!(
            protocol
                .advance(&holder, &dispatched, backwards, NOW_MS + 3)
                .await
                .unwrap_err(),
            ProtocolError::OutOfOrder {
                from: IntentState::Dispatched,
                to: backwards,
            }
        );
    }
    assert_eq!(
        state_of(&store, &project, &run).await,
        IntentState::Dispatched,
        "a refused transition must leave the record exactly as it was"
    );
    let settled = protocol
        .advance(&holder, &dispatched, IntentState::Terminal, NOW_MS + 4)
        .await
        .unwrap();
    assert_eq!(
        settled.state,
        IntentState::Terminal,
        "a launch refused before acknowledgement still has a known outcome"
    );
}

#[tokio::test]
async fn each_crash_boundary_is_resolved_by_what_it_proves_and_nothing_more() {
    let project = project_id();
    let store = store().await;
    let intent_only = run("after-intent");
    let dispatched = run("after-record");
    let acknowledged = run("after-ack");
    let terminal = run("after-terminal");
    for (run, boundary) in [
        (&intent_only, IntentState::Persisted),
        (&dispatched, IntentState::Dispatched),
        (&acknowledged, IntentState::Acknowledged),
        (&terminal, IntentState::Terminal),
    ] {
        crashed_launch(&store, &project, run, boundary).await;
    }
    publish(&project, "engine-crashed", PresenceState::Unreachable);

    let log = StartLog::default();
    let reconciler = Reconciler::new(
        store.clone(),
        &setup(
            &project,
            "engine-recovering",
            probe(Observation::No, Observation::No),
            log.action(),
        ),
    )
    .unwrap();
    let report = reconciler
        .reconcile(ReconcileTrigger::Startup, RECOVER_AT_MS)
        .await;
    presence::forget(&project);

    assert_eq!(report.started, vec![intent_only.clone()]);
    assert_eq!(
        log.started(),
        vec![intent_only.clone()],
        "only the boundary that provably had no effect may be started"
    );
    assert_eq!(report.recovered, vec![acknowledged.clone()]);
    assert_eq!(report.settled, vec![terminal.clone()]);
    assert!(report.left_running.is_empty());
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(report.needs_inspection.len(), 1);
    let surfaced = &report.needs_inspection[0];
    assert_eq!(surfaced.run, dispatched);
    assert_eq!(surfaced.state, IntentState::Dispatched);
    assert_eq!(surfaced.reason, InspectionReason::UnacknowledgedLaunch);
    assert_eq!(
        state_of(&store, &project, &dispatched).await,
        IntentState::Dispatched,
        "an unknown launch is left exactly as it was, not advanced or reset"
    );
    assert_eq!(
        state_of(&store, &project, &acknowledged).await,
        IntentState::Acknowledged,
        "recovery is the owning runtime's job; reconciliation rewrites nothing"
    );
    assert_eq!(
        state_of(&store, &project, &intent_only).await,
        IntentState::Dispatched,
        "the runtime claim is recorded before the start has any effect"
    );
    assert_eq!(report.pruned, 0);
    assert_eq!(
        state_of(&store, &project, &terminal).await,
        IntentState::Terminal,
        "a settled run inside its retention window is kept, not resurrected"
    );
}

#[tokio::test]
async fn running_reconciliation_again_starts_nothing_twice_and_reaches_a_fixpoint() {
    let project = project_id();
    let store = store().await;
    let only = run("launch-once");
    crashed_launch(&store, &project, &only, IntentState::Persisted).await;
    publish(&project, "engine-crashed", PresenceState::Unreachable);

    let log = StartLog::default();
    let launched = log.clone();
    let launching_store = store.clone();
    let launching_project = project.clone();
    // A real launcher acknowledges its own start under the authority it was
    // handed, which is what closes the unknown-effect window.
    let start: StartAction = Arc::new(move |run: RunKey, holder: LeaseHolder| {
        let launched = launched.clone();
        let protocol =
            LaunchProtocol::new(launching_store.clone(), ProjectKey::new(&launching_project));
        Box::pin(async move {
            let dispatched = protocol.state_of(&run).await.unwrap().unwrap();
            protocol
                .advance(
                    &holder,
                    &dispatched,
                    IntentState::Acknowledged,
                    RECOVER_AT_MS,
                )
                .await
                .map_err(|error| error.to_string())?;
            launched.0.lock().unwrap().push(run);
            Ok(())
        })
    });

    let running = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = running.clone();
    let evidence: EvidenceProbe = Arc::new(move |_| {
        LaunchEvidence::new(
            if observed.load(std::sync::atomic::Ordering::SeqCst) {
                Observation::Yes
            } else {
                Observation::No
            },
            Observation::No,
        )
    });
    let reconciler = Reconciler::new(
        store.clone(),
        &setup(&project, "engine-recovering", evidence, start),
    )
    .unwrap();

    let first = reconciler
        .reconcile(ReconcileTrigger::Startup, RECOVER_AT_MS)
        .await;
    running.store(true, std::sync::atomic::Ordering::SeqCst);
    let second = reconciler
        .reconcile(ReconcileTrigger::Takeover, RECOVER_AT_MS + 1)
        .await;
    let third = reconciler
        .reconcile(ReconcileTrigger::Takeover, RECOVER_AT_MS + 2)
        .await;
    presence::forget(&project);

    assert_eq!(first.started, vec![only.clone()]);
    assert_eq!(
        state_of(&store, &project, &only).await,
        IntentState::Acknowledged
    );
    assert!(second.started.is_empty());
    assert_eq!(second.left_running, vec![only.clone()]);
    assert_eq!(
        second, third,
        "reconciliation must be a fixpoint, not a source of new work"
    );
    assert_eq!(
        log.started(),
        vec![only],
        "a second pass must never produce a second child"
    );
}

#[tokio::test]
async fn reconciliation_never_touches_a_record_a_live_engine_still_owns() {
    let project = project_id();
    let store = store().await;
    let owned = run("someone-elses");
    crashed_launch(&store, &project, &owned, IntentState::Persisted).await;
    publish(&project, "engine-crashed", PresenceState::AliveActive);

    let log = StartLog::default();
    let reconciler = Reconciler::new(
        store.clone(),
        &setup(
            &project,
            "engine-recovering",
            probe(Observation::No, Observation::No),
            log.action(),
        ),
    )
    .unwrap();
    let alive = reconciler
        .reconcile(ReconcileTrigger::Startup, RECOVER_AT_MS)
        .await;
    publish(&project, "engine-crashed", PresenceState::Unknown);
    let unclassified = reconciler
        .reconcile(ReconcileTrigger::Startup, RECOVER_AT_MS)
        .await;
    presence::forget(&project);
    let unpublished = reconciler
        .reconcile(ReconcileTrigger::Startup, RECOVER_AT_MS)
        .await;

    for (report, refusal) in [
        (&alive, TakeoverRefusal::HolderAlive),
        (&unclassified, TakeoverRefusal::HolderUnclassified),
        (&unpublished, TakeoverRefusal::HolderUnclassified),
    ] {
        assert_eq!(
            report.owned_elsewhere,
            vec![ForeignRecord {
                run: owned.clone(),
                holder_instance: "engine-crashed".to_string(),
                refusal,
            }]
        );
        assert!(report.started.is_empty());
        assert!(report.needs_inspection.is_empty());
        assert!(report.failed.is_empty(), "{:?}", report.failed);
    }
    assert!(
        log.started().is_empty(),
        "work owned by a live engine is never restarted"
    );
    assert_eq!(
        state_of(&store, &project, &owned).await,
        IntentState::Persisted,
        "a record owned elsewhere must come out of reconciliation unchanged"
    );
    assert!(leases::recent_takeovers(&project).is_empty());
}

#[tokio::test]
async fn a_replaced_owners_write_is_fenced_out_while_reconciliation_holds_the_lease() {
    let project = project_id();
    let store = store().await;
    let taken = run("taken-over");
    let (stale_holder, persisted) =
        crashed_launch(&store, &project, &taken, IntentState::Persisted).await;
    publish(&project, "engine-crashed", PresenceState::Unreachable);

    let rejected = Arc::new(Mutex::new(None));
    let captured = rejected.clone();
    let stale_store = store.clone();
    let stale_project = project.clone();
    // The crashed owner is still running somewhere and tries to continue its
    // launch while reconciliation holds the lease it lost.
    let start: StartAction = Arc::new(move |_run, _holder| {
        let captured = captured.clone();
        let stale_holder = stale_holder.clone();
        let persisted = persisted.clone();
        let protocol = LaunchProtocol::new(stale_store.clone(), ProjectKey::new(&stale_project));
        Box::pin(async move {
            let outcome = protocol
                .advance(
                    &stale_holder,
                    &persisted,
                    IntentState::Acknowledged,
                    RECOVER_AT_MS,
                )
                .await;
            *captured.lock().unwrap() = Some(outcome.unwrap_err());
            Ok(())
        })
    });

    let reconciler = Reconciler::new(
        store.clone(),
        &setup(
            &project,
            "engine-recovering",
            probe(Observation::No, Observation::No),
            start,
        ),
    )
    .unwrap();
    let report = reconciler
        .reconcile(ReconcileTrigger::Startup, RECOVER_AT_MS)
        .await;
    let takeovers = leases::recent_takeovers(&project);
    presence::forget(&project);

    assert_eq!(report.started, vec![taken.clone()]);
    assert!(
        matches!(
            rejected.lock().unwrap().clone().unwrap(),
            ProtocolError::Lease(LeaseError::Fenced {
                presented: 1,
                held: 2,
                ..
            })
        ),
        "{:?}",
        rejected.lock().unwrap()
    );
    assert_eq!(
        state_of(&store, &project, &taken).await,
        IntentState::Dispatched,
        "the fenced write must not land, not even partially"
    );
    assert_eq!(takeovers.len(), 1);
    assert_eq!(takeovers[0].from.holder_instance, "engine-crashed");
    assert_eq!(takeovers[0].to_instance, "engine-recovering");
    assert_eq!(takeovers[0].generation_after, 2);
}

#[tokio::test]
async fn retention_prunes_a_settled_run_and_never_one_with_an_unknown_outcome() {
    let project = project_id();
    let store = store().await;
    let settled = run("settled");
    let unknown = run("unknown");
    crashed_launch(&store, &project, &settled, IntentState::Terminal).await;
    crashed_launch(&store, &project, &unknown, IntentState::Dispatched).await;
    publish(&project, "engine-crashed", PresenceState::Unreachable);

    let log = StartLog::default();
    let mut config = setup(
        &project,
        "engine-recovering",
        probe(Observation::No, Observation::No),
        log.action(),
    );
    config.settings.run_intent_retention_secs = 1;
    let reconciler = Reconciler::new(store.clone(), &config).unwrap();
    let report = reconciler
        .reconcile(ReconcileTrigger::Startup, NOW_MS + 60_000)
        .await;
    presence::forget(&project);

    assert_eq!(report.settled, vec![settled.clone()]);
    assert_eq!(report.pruned, 1);
    assert!(
        LaunchProtocol::new(store.clone(), ProjectKey::new(&project))
            .state_of(&settled)
            .await
            .unwrap()
            .is_none(),
        "a settled run past retention is pruned"
    );
    assert_eq!(
        state_of(&store, &project, &unknown).await,
        IntentState::Dispatched,
        "a run whose outcome nobody recorded is never pruned by age"
    );
    assert_eq!(report.needs_inspection.len(), 1);
    assert!(
        log.started().is_empty(),
        "a settled run is never resurrected"
    );
}

#[tokio::test]
async fn an_unusable_store_skips_reconciliation_with_a_reported_reason() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not-a-database.sqlite");
    std::fs::write(&path, b"this is emphatically not a SQLite database").unwrap();
    let log = StartLog::default();
    let outcome = reconcile_at_path(
        &path,
        &setup(
            &project_id(),
            "engine-recovering",
            probe(Observation::No, Observation::No),
            log.action(),
        ),
        ReconcileTrigger::Startup,
        NOW_MS,
    )
    .await;

    let reason = outcome
        .skip_reason()
        .expect("an unusable store must skip out loud, not silently")
        .to_string();
    assert!(outcome.report().is_none());
    assert!(reason.contains("coordination store"), "{reason}");
    assert!(
        crate::coordination::outbox::durability_warnings()
            .iter()
            .any(|warning| warning.to_string().contains("not-a-database.sqlite")),
        "the skip must reach the durability reporting channel"
    );
    assert!(log.started().is_empty());
}

#[tokio::test]
async fn shutdown_cancels_and_joins_the_reconciliation_task() {
    let project = project_id();
    let store = store().await;
    let pending = run("startup-pass");
    seed_released_intent(&store, &project, &pending).await;

    let log = StartLog::default();
    let reconciler = Arc::new(
        Reconciler::new(
            store.clone(),
            &setup(
                &project,
                "engine-recovering",
                probe(Observation::No, Observation::No),
                log.action(),
            ),
        )
        .unwrap(),
    );
    let service = ReconciliationService::spawn(reconciler);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while log.started().is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the startup pass must run"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    tokio::time::timeout(Duration::from_secs(5), service.shutdown())
        .await
        .expect("the reconciliation task must be joined, not detached");
    let after_shutdown = log.started();
    service_cannot_write_after_shutdown(&log, after_shutdown.len()).await;

    assert_eq!(
        after_shutdown,
        vec![pending],
        "the startup pass starts the one record that had no effect yet"
    );
}

async fn service_cannot_write_after_shutdown(log: &StartLog, before: usize) {
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        log.started().len(),
        before,
        "a joined reconciliation task cannot act after shutdown"
    );
}
