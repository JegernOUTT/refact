//! Kill the owning engine at each launch boundary and reconcile what it left.
//!
//! A subprocess walks the protocol to one boundary, publishes its own engine
//! presence record, then hangs. The parent SIGKILLs it and reconciles the same
//! on-disk store, so every assertion is about a process that really died rather
//! than one that was asked politely to stop.
//!
//! Linux only: recovery depends on classifying the dead owner through
//! `/proc`-backed process identity, and an engine presence cannot classify is
//! never treated as a dead one.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use refact_work_core::identity::{RunKey, WorkKey};
use refact_work_core::settings::CoordinationSettings;
use refact_work_core::store::records::{now_ms, IntentState};
use refact_work_core::store::CoordinationStore;
use refact_work_core::ProjectKey;

use super::*;
use crate::coordination::presence::{EngineIdentity, EnginePresence};

const BOUNDARY_ENV: &str = "REFACT_TEST_LAUNCH_BOUNDARY";
const ROOT_ENV: &str = "REFACT_TEST_LAUNCH_ROOT";
const CRASHED_INSTANCE: &str = "engine-crashed";
const RECOVERING_INSTANCE: &str = "engine-recovering";
const TTL_MS: u64 = 500;
const GRACE_MS: u64 = 250;

fn settings() -> CoordinationSettings {
    CoordinationSettings {
        lease_ttl_ms: TTL_MS,
        lease_renew_interval_ms: 100,
        lease_takeover_grace_ms: GRACE_MS,
        presence_heartbeat_interval_ms: 1_000,
        presence_stale_after_ms: 3_000,
        ..CoordinationSettings::default()
    }
}

fn run() -> RunKey {
    RunKey::new(WorkKey::background_agent("crashing-child"), 1)
}

fn store_path(root: &Path) -> PathBuf {
    root.join("coordination.sqlite")
}

/// One project per checkout directory, so crash tests running side by side in
/// this process never read each other's published engine presence.
fn project(root: &Path) -> ProjectKey {
    ProjectKey::new(format!(
        "crash-{}",
        root.file_name().unwrap().to_string_lossy()
    ))
}

async fn open(root: &Path) -> Arc<CoordinationStore> {
    Arc::new(
        CoordinationStore::open(&store_path(root), &settings())
            .await
            .unwrap(),
    )
}

fn boundary_of(raw: &str) -> Option<IntentState> {
    match raw {
        "intent" => Some(IntentState::Persisted),
        "record" => Some(IntentState::Dispatched),
        "ack" => Some(IntentState::Acknowledged),
        "terminal" => Some(IntentState::Terminal),
        _ => None,
    }
}

/// The engine that dies. It walks the protocol to exactly one boundary, says
/// it got there, and then never does anything again.
#[test]
fn subprocess_launch_to_boundary() {
    let (Ok(root), Ok(boundary)) = (std::env::var(ROOT_ENV), std::env::var(BOUNDARY_ENV)) else {
        return;
    };
    let root = PathBuf::from(root);
    let boundary = boundary_of(&boundary).expect("a named launch boundary");
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async move {
            let store = open(&root).await;
            let access = store.authorize(project(&root));
            let presence = EnginePresence::new(
                store.clone(),
                access,
                EngineIdentity::detect(CRASHED_INSTANCE),
                Arc::new(|| true),
                &settings(),
            )
            .unwrap();
            presence.heartbeat(now_ms()).await.unwrap();

            let leases = Arc::new(
                LeaseManager::new(
                    store.clone(),
                    store.authorize(project(&root)),
                    CRASHED_INSTANCE,
                    &settings(),
                )
                .unwrap(),
            );
            let protocol = LaunchProtocol::new(store.clone(), project(&root));
            let run = run();
            let at_ms = now_ms();
            let holder = leases.acquire(&run_lease_key(&run), at_ms).await.unwrap();
            let mut record = protocol.persist_intent(&holder, &run, at_ms).await.unwrap();
            for step in [
                IntentState::Dispatched,
                IntentState::Acknowledged,
                IntentState::Terminal,
            ] {
                if boundary_ordinal(boundary) < boundary_ordinal(step) {
                    break;
                }
                record = protocol
                    .advance(&holder, &record, step, now_ms())
                    .await
                    .unwrap();
            }
            assert_eq!(record.state, boundary);
            std::fs::write(root.join("reached"), boundary.as_str()).unwrap();
            std::future::pending::<()>().await;
        });
}

/// Run a launch to `boundary` in a real process and SIGKILL it there.
fn kill_at_boundary(root: &Path, boundary: &str) {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "coordination::reconcile::crash_tests::subprocess_launch_to_boundary",
            "--nocapture",
        ])
        .env(ROOT_ENV, root)
        .env(BOUNDARY_ENV, boundary)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let reached = root.join("reached");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !reached.exists() {
        if Instant::now() >= deadline || child.try_wait().unwrap().is_some() {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the launching subprocess never reached the `{boundary}` boundary");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(
        std::fs::read_to_string(&reached).unwrap(),
        boundary_of(boundary).unwrap().as_str()
    );
}

/// Everything the surviving engine needs to reconcile after the kill.
struct Recovery {
    root: PathBuf,
    store: Arc<CoordinationStore>,
    reconciler: Reconciler,
    started: Arc<std::sync::Mutex<Vec<RunKey>>>,
    at_ms: i64,
}

impl Recovery {
    async fn open(root: &Path, evidence: EvidenceProbe) -> Recovery {
        let store = open(root).await;
        // Classify the engine that died from its own record, the same way a
        // restarting engine would: nothing about its death is asserted here.
        EnginePresence::new(
            store.clone(),
            store.authorize(project(&root)),
            EngineIdentity::detect(RECOVERING_INSTANCE),
            Arc::new(|| false),
            &settings(),
        )
        .unwrap()
        .refresh(now_ms())
        .await
        .unwrap();
        let started = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = started.clone();
        let start: StartAction = Arc::new(move |run, _holder| {
            log.lock().unwrap().push(run);
            Box::pin(async { Ok(()) })
        });
        let reconciler = Reconciler::new(
            store.clone(),
            &ReconcilerSetup {
                project: project(&root),
                instance_id: RECOVERING_INSTANCE.to_string(),
                settings: settings(),
                evidence,
                start,
            },
        )
        .unwrap();
        Recovery {
            root: root.to_path_buf(),
            store,
            reconciler,
            started,
            // The dead owner's term and its whole grace window are over: this
            // is the earliest moment a takeover is allowed at all.
            at_ms: now_ms() + (TTL_MS + GRACE_MS) as i64 + 1,
        }
    }

    async fn reconcile(&self) -> ReconciliationReport {
        self.reconciler
            .reconcile(ReconcileTrigger::Startup, self.at_ms)
            .await
    }

    fn started(&self) -> Vec<RunKey> {
        self.started.lock().unwrap().clone()
    }

    async fn state(&self) -> Option<IntentState> {
        LaunchProtocol::new(self.store.clone(), project(&self.root))
            .state_of(&run())
            .await
            .unwrap()
            .map(|record| record.state)
    }
}

fn saw_nothing() -> EvidenceProbe {
    Arc::new(|_| LaunchEvidence::new(Observation::No, Observation::No))
}

#[tokio::test]
async fn killed_after_the_intent_starts_exactly_one_child() {
    let temp = tempfile::tempdir().unwrap();
    kill_at_boundary(temp.path(), "intent");
    let recovery = Recovery::open(temp.path(), saw_nothing()).await;

    let report = recovery.reconcile().await;

    assert_eq!(
        report.started,
        vec![run()],
        "an intent that provably had no effect is safe to start, {report:?}"
    );
    assert_eq!(recovery.started(), vec![run()], "exactly one child");
    assert!(report.needs_inspection.is_empty());
    assert_eq!(
        recovery.state().await,
        Some(IntentState::Dispatched),
        "the runtime claim is recorded before the start has any effect"
    );
}

#[tokio::test]
async fn killed_after_the_runtime_record_never_starts_a_second_child() {
    let temp = tempfile::tempdir().unwrap();
    kill_at_boundary(temp.path(), "record");
    let recovery = Recovery::open(temp.path(), saw_nothing()).await;

    let report = recovery.reconcile().await;

    assert!(
        recovery.started().is_empty(),
        "whether a child started is unknown, so no second one may be launched"
    );
    assert!(report.started.is_empty());
    assert!(report.recovered.is_empty());
    assert_eq!(report.needs_inspection.len(), 1, "{report:?}");
    assert_eq!(report.needs_inspection[0].run, run());
    assert_eq!(
        report.needs_inspection[0].reason,
        InspectionReason::UnacknowledgedLaunch
    );
    assert_eq!(
        recovery.state().await,
        Some(IntentState::Dispatched),
        "an unknown launch is left exactly where the crash left it"
    );
}

#[tokio::test]
async fn killed_after_the_acknowledgement_is_recoverable_without_replay() {
    let temp = tempfile::tempdir().unwrap();
    kill_at_boundary(temp.path(), "ack");
    let recovery = Recovery::open(temp.path(), saw_nothing()).await;

    let report = recovery.reconcile().await;

    assert_eq!(
        report.recovered,
        vec![run()],
        "an acknowledged launch without an outcome is resumable, {report:?}"
    );
    assert!(
        recovery.started().is_empty(),
        "resuming is not restarting; nothing may be replayed"
    );
    assert!(report.needs_inspection.is_empty());
    assert_eq!(
        recovery.state().await,
        Some(IntentState::Acknowledged),
        "the last completed durable boundary stands untouched"
    );
}

#[tokio::test]
async fn killed_after_the_outcome_settles_and_never_resurrects() {
    let temp = tempfile::tempdir().unwrap();
    kill_at_boundary(temp.path(), "terminal");
    let recovery = Recovery::open(temp.path(), saw_nothing()).await;

    let first = recovery.reconcile().await;
    let second = recovery.reconcile().await;

    assert_eq!(first.settled, vec![run()]);
    assert!(first.started.is_empty());
    assert!(first.recovered.is_empty());
    assert!(first.needs_inspection.is_empty());
    assert!(
        recovery.started().is_empty(),
        "a settled run never restarts"
    );
    assert_eq!(
        first, second,
        "reconciling a settled run twice must change nothing"
    );
    assert_eq!(recovery.state().await, Some(IntentState::Terminal));
}

#[tokio::test]
async fn reconciling_a_killed_launch_twice_produces_one_child_and_one_outcome() {
    let temp = tempfile::tempdir().unwrap();
    kill_at_boundary(temp.path(), "intent");
    let recovery = Recovery::open(temp.path(), saw_nothing()).await;

    let first = recovery.reconcile().await;
    let second = recovery.reconcile().await;
    let third = recovery.reconcile().await;

    assert_eq!(first.started, vec![run()]);
    assert_eq!(
        recovery.started(),
        vec![run()],
        "a second pass must never produce a second child"
    );
    assert_eq!(
        second.needs_inspection.len(),
        1,
        "the start left an unacknowledged launch, which is surfaced not replayed"
    );
    assert_eq!(
        second, third,
        "reconciliation must settle into a fixpoint, not keep acting"
    );
}
