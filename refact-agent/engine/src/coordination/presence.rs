//! Engine presence and liveness (M3).
//!
//! This engine records its own presence in the coordination store and reads
//! back what other engines of the *same project on this machine* recorded.
//!
//! A PID is never proof of ownership: the OS reuses PIDs, so liveness needs the
//! PID **and** the process-start identity to match. Where that identity cannot
//! be obtained the answer is [`PresenceState::Unknown`] — never "alive".

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use refact_work_core::identity::ProjectKey;
use refact_work_core::settings::CoordinationSettings;
use refact_work_core::store::records::{now_ms, EngineInstanceRecord, PresenceMark};
use refact_work_core::store::{CoordinationStore, ProjectAccess, StoreError};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use super::types::Freshness;

pub async fn start_background_presence(
    app: &crate::app_state::AppState,
) -> Option<PresenceService> {
    let project_root = crate::files_correction::get_project_dirs(app.gcx.clone())
        .await
        .into_iter()
        .next()?;
    let settings = match crate::runtime_settings::coordination_for_project(&project_root).await {
        Ok(settings) if settings.enabled => settings,
        Ok(_) => return None,
        Err(error) => {
            tracing::warn!("engine presence settings unavailable: {error}");
            return None;
        }
    };
    let path = refact_work_core::default_store_path(&app.paths.cache_dir);
    let store = match super::outbox::store_for_path(&path, &settings).await {
        Ok(store) => store,
        Err(error) => {
            super::outbox::note_durability_unavailable(&path, error.to_string());
            return None;
        }
    };
    let project = ProjectKey::new(super::roots::project_identity(&project_root));
    let identity = EngineIdentity::detect(super::engine_instance_id());
    let access = store.authorize(project.clone());
    let activity = scheduler_activity(app.work_scheduler.clone(), project);
    match EnginePresence::new(store, access, identity, activity, &settings) {
        Ok(presence) => Some(PresenceService::spawn(Arc::new(presence))),
        Err(error) => {
            tracing::warn!("engine presence could not start: {error}");
            None
        }
    }
}

/// Stored in `process_started_at_ms` when the platform cannot tell us when a
/// process started. Such a record can never classify as alive.
pub const START_IDENTITY_UNOBTAINABLE: i64 = -1;

/// How live another engine is, with the four operational cases kept apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceState {
    /// Heartbeat is fresh, the process identity matches and work is running.
    AliveActive,
    /// The process is alive but advertises no running work.
    AliveIdle,
    /// Heartbeat is stale while the process identity still matches; unknown.
    Unreachable,
    /// The process identity no longer matches, or the record was closed.
    Dead,
    /// The start identity could not be obtained, so nothing can be claimed.
    Unknown,
}

impl PresenceState {
    pub fn is_alive(&self) -> bool {
        matches!(self, PresenceState::AliveActive | PresenceState::AliveIdle)
    }

    /// An idle engine is alive but is never advertised as running work.
    pub fn is_running_work(&self) -> bool {
        matches!(self, PresenceState::AliveActive)
    }
}

/// What probing a PID told us about the process behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartIdentity {
    /// The process exists and started this many milliseconds after boot.
    Started(i64),
    /// There is no process with that PID.
    NotRunning,
    /// The platform could not answer.
    Unknown,
}

/// The machine and boot this engine runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostIdentity {
    pub host_id: String,
    pub boot_id: String,
}

impl HostIdentity {
    pub fn local() -> HostIdentity {
        HostIdentity {
            host_id: hostname::get()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            boot_id: local_boot_id(),
        }
    }
}

#[cfg(target_os = "linux")]
fn local_boot_id() -> String {
    std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map(|value| value.trim().to_string())
        .unwrap_or_default()
}

#[cfg(not(target_os = "linux"))]
fn local_boot_id() -> String {
    String::new()
}

#[cfg(target_os = "linux")]
pub fn probe_process_start(pid: i64) -> StartIdentity {
    if pid <= 0 {
        return StartIdentity::NotRunning;
    }
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => match parse_start_ticks(&stat) {
            Some(ticks) => StartIdentity::Started(ticks_to_ms(ticks)),
            None => StartIdentity::Unknown,
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => StartIdentity::NotRunning,
        Err(_) => StartIdentity::Unknown,
    }
}

#[cfg(not(target_os = "linux"))]
pub fn probe_process_start(_pid: i64) -> StartIdentity {
    StartIdentity::Unknown
}

/// Field 22 of `/proc/<pid>/stat`, read after the `comm` field so a process
/// name containing spaces or parentheses cannot shift the offsets.
fn parse_start_ticks(stat: &str) -> Option<i64> {
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse::<i64>()
        .ok()
}

#[cfg(unix)]
fn clock_ticks_per_second() -> i64 {
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks > 0 {
        ticks as i64
    } else {
        100
    }
}

#[cfg(not(unix))]
fn clock_ticks_per_second() -> i64 {
    100
}

fn ticks_to_ms(ticks: i64) -> i64 {
    ticks.saturating_mul(1000) / clock_ticks_per_second()
}

/// Everything that identifies this engine process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineIdentity {
    pub instance_id: String,
    pub pid: i64,
    pub host: HostIdentity,
    pub start: StartIdentity,
}

impl EngineIdentity {
    pub fn detect(instance_id: impl Into<String>) -> EngineIdentity {
        let pid = std::process::id() as i64;
        EngineIdentity {
            instance_id: instance_id.into(),
            pid,
            host: HostIdentity::local(),
            start: probe_process_start(pid),
        }
    }

    pub fn started_at_ms(&self) -> i64 {
        match self.start {
            StartIdentity::Started(ms) => ms,
            _ => START_IDENTITY_UNOBTAINABLE,
        }
    }
}

/// Classify one engine record against this host.
///
/// `probe` is the result of looking the record's PID up right now.
pub fn classify(
    record: &EngineInstanceRecord,
    local: &HostIdentity,
    probe: StartIdentity,
    now_at_ms: i64,
    stale_after_ms: i64,
) -> PresenceState {
    if record.presence_mark == PresenceMark::Closed {
        return PresenceState::Dead;
    }
    if record.host_id != local.host_id {
        return PresenceState::Unknown;
    }
    if record.process_started_at_ms < 0 {
        return PresenceState::Unknown;
    }
    if !record.boot_id.is_empty() && !local.boot_id.is_empty() && record.boot_id != local.boot_id {
        return PresenceState::Dead;
    }
    match probe {
        StartIdentity::Unknown => PresenceState::Unknown,
        StartIdentity::NotRunning => PresenceState::Dead,
        StartIdentity::Started(started_at_ms) if started_at_ms != record.process_started_at_ms => {
            PresenceState::Dead
        }
        StartIdentity::Started(_) => {
            if now_at_ms.saturating_sub(record.heartbeat_at_ms) > stale_after_ms {
                PresenceState::Unreachable
            } else if record.presence_mark == PresenceMark::Active {
                PresenceState::AliveActive
            } else {
                PresenceState::AliveIdle
            }
        }
    }
}

/// One other engine of this project, as classified right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerPresence {
    pub instance_id: String,
    pub state: PresenceState,
}

/// What the other engines of a project let us say about work we do not own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerLiveness {
    /// Some other engine is running work right now.
    RunningElsewhere,
    /// Other engines exist but none advertises running work.
    IdleElsewhere,
    /// Every other engine is provably gone.
    NoOtherEngine,
    /// At least one engine could not be classified.
    Unknown,
}

pub fn peer_liveness(peers: &[PeerPresence]) -> PeerLiveness {
    if peers.iter().any(|peer| peer.state.is_running_work()) {
        return PeerLiveness::RunningElsewhere;
    }
    if peers.iter().any(|peer| {
        matches!(
            peer.state,
            PresenceState::Unreachable | PresenceState::Unknown
        )
    }) {
        return PeerLiveness::Unknown;
    }
    if peers.iter().any(|peer| peer.state.is_alive()) {
        return PeerLiveness::IdleElsewhere;
    }
    PeerLiveness::NoOtherEngine
}

/// Relabel the freshness of a row this engine does not own.
///
/// A recent-looking record on disk only means something is happening if an
/// engine is actually running; otherwise it is a leftover, not activity.
pub fn apply_to_freshness(current: Freshness, liveness: PeerLiveness) -> Freshness {
    match (current, liveness) {
        (Freshness::Recent, PeerLiveness::Unknown) => Freshness::Unknown,
        (Freshness::Recent, PeerLiveness::IdleElsewhere | PeerLiveness::NoOtherEngine) => {
            Freshness::Stale
        }
        (current, _) => current,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectPresence {
    pub peers: Vec<PeerPresence>,
    pub liveness: PeerLiveness,
    pub observed_at_ms: i64,
}

fn published() -> &'static RwLock<BTreeMap<String, ProjectPresence>> {
    static SLOT: OnceLock<RwLock<BTreeMap<String, ProjectPresence>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(BTreeMap::new()))
}

/// Publish the classification for one project. Readers only ever see the
/// project they ask for, so another project's engines stay undisclosed.
pub fn publish(project: &str, peers: Vec<PeerPresence>, observed_at_ms: i64) {
    let presence = ProjectPresence {
        liveness: peer_liveness(&peers),
        peers,
        observed_at_ms,
    };
    published()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(project.to_string(), presence);
}

pub fn forget(project: &str) {
    published()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(project);
}

pub fn project_presence(project: &str) -> Option<ProjectPresence> {
    published()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(project)
        .cloned()
}

/// `None` when this engine has no presence data for the project, so callers
/// keep whatever they already knew instead of inventing liveness.
pub fn project_liveness(project: &str) -> Option<PeerLiveness> {
    project_presence(project).map(|presence| presence.liveness)
}

/// Tells the heartbeat whether this engine is running work right now.
pub type ActivityProbe = Arc<dyn Fn() -> bool + Send + Sync>;

/// Looks a PID up and reports its start identity.
pub type ProcessProbe = Arc<dyn Fn(i64) -> StartIdentity + Send + Sync>;

/// This engine's presence record, and the classification of its peers.
pub struct EnginePresence {
    store: Arc<CoordinationStore>,
    access: ProjectAccess,
    identity: EngineIdentity,
    activity: ActivityProbe,
    process: ProcessProbe,
    heartbeat_interval: Duration,
    stale_after_ms: i64,
}

impl EnginePresence {
    pub fn new(
        store: Arc<CoordinationStore>,
        access: ProjectAccess,
        identity: EngineIdentity,
        activity: ActivityProbe,
        settings: &CoordinationSettings,
    ) -> Result<EnginePresence, StoreError> {
        settings.validate().map_err(StoreError::Settings)?;
        Ok(EnginePresence {
            store,
            access,
            identity,
            activity,
            process: Arc::new(probe_process_start),
            heartbeat_interval: Duration::from_millis(settings.presence_heartbeat_interval_ms),
            stale_after_ms: settings.presence_stale_after_ms as i64,
        })
    }

    pub fn with_process_probe(mut self, process: ProcessProbe) -> EnginePresence {
        self.process = process;
        self
    }

    pub fn project(&self) -> &str {
        self.access.project().as_str()
    }

    pub fn heartbeat_interval(&self) -> Duration {
        self.heartbeat_interval
    }

    fn record(&self, mark: PresenceMark, at_ms: i64) -> EngineInstanceRecord {
        EngineInstanceRecord {
            project: ProjectKey::new(self.project()),
            instance_id: self.identity.instance_id.clone(),
            pid: self.identity.pid,
            boot_id: self.identity.host.boot_id.clone(),
            process_started_at_ms: self.identity.started_at_ms(),
            host_id: self.identity.host.host_id.clone(),
            heartbeat_at_ms: at_ms,
            presence_mark: mark,
        }
    }

    /// Write this engine's presence, advertising work only when it is running.
    pub async fn heartbeat(&self, at_ms: i64) -> Result<(), StoreError> {
        let mark = if (self.activity)() {
            PresenceMark::Active
        } else {
            PresenceMark::Idle
        };
        self.store
            .put_engine_instance(&self.access, &self.record(mark, at_ms))
            .await
    }

    /// Clean shutdown: mark the record closed so a reader knows this engine
    /// went away on purpose instead of guessing from a stale heartbeat.
    pub async fn close(&self, at_ms: i64) -> Result<(), StoreError> {
        self.store
            .put_engine_instance(&self.access, &self.record(PresenceMark::Closed, at_ms))
            .await
    }

    /// Classify every other engine of this project.
    pub async fn peers(&self, at_ms: i64) -> Result<Vec<PeerPresence>, StoreError> {
        let records = self.store.list_engine_instances(&self.access).await?;
        Ok(records
            .into_iter()
            .filter(|record| record.instance_id != self.identity.instance_id)
            .map(|record| PeerPresence {
                state: classify(
                    &record,
                    &self.identity.host,
                    (self.process)(record.pid),
                    at_ms,
                    self.stale_after_ms,
                ),
                instance_id: record.instance_id,
            })
            .collect())
    }

    /// One heartbeat plus a fresh peer classification for readers.
    pub async fn refresh(&self, at_ms: i64) -> Result<(), StoreError> {
        self.heartbeat(at_ms).await?;
        let peers = self.peers(at_ms).await?;
        publish(self.project(), peers, at_ms);
        Ok(())
    }
}

/// Owns the heartbeat task: one cancellation path, joined on shutdown.
pub struct PresenceService {
    presence: Arc<EnginePresence>,
    cancel: Arc<Notify>,
    task: JoinHandle<()>,
}

impl PresenceService {
    pub fn spawn(presence: Arc<EnginePresence>) -> PresenceService {
        let cancel = Arc::new(Notify::new());
        let task = tokio::spawn(heartbeat_loop(presence.clone(), cancel.clone()));
        PresenceService {
            presence,
            cancel,
            task,
        }
    }

    /// Cancel the loop, join it, then close this engine's record.
    pub async fn shutdown(self) -> Result<(), StoreError> {
        self.cancel.notify_one();
        if let Err(error) = self.task.await {
            tracing::warn!("engine presence heartbeat task failed to join: {error}");
        }
        let closed = self.presence.close(now_ms()).await;
        forget(self.presence.project());
        closed
    }
}

async fn heartbeat_loop(presence: Arc<EnginePresence>, cancel: Arc<Notify>) {
    loop {
        if let Err(error) = presence.refresh(now_ms()).await {
            tracing::warn!("engine presence heartbeat failed: {error}");
        }
        tokio::select! {
            _ = cancel.notified() => return,
            _ = tokio::time::sleep(presence.heartbeat_interval()) => {}
        }
    }
}

/// Running work as the admission scheduler sees it, for this project only.
pub fn scheduler_activity(
    scheduler: Arc<refact_work_core::AdmissionScheduler>,
    project: ProjectKey,
) -> ActivityProbe {
    Arc::new(move || {
        scheduler
            .snapshot()
            .active_by_project
            .get(&project)
            .copied()
            .unwrap_or(0)
            > 0
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::coordination::roots;
    use refact_work_core::store::records::PresenceMark;
    use std::collections::HashMap;

    const STALE_AFTER_MS: i64 = 60_000;
    const NOW_MS: i64 = 1_800_000_000_000;

    fn settings() -> CoordinationSettings {
        CoordinationSettings::default()
    }

    fn host() -> HostIdentity {
        HostIdentity {
            host_id: "host-a".into(),
            boot_id: "boot-a".into(),
        }
    }

    fn identity(instance_id: &str, pid: i64, started_at_ms: i64) -> EngineIdentity {
        EngineIdentity {
            instance_id: instance_id.into(),
            pid,
            host: host(),
            start: StartIdentity::Started(started_at_ms),
        }
    }

    fn peer_record(
        project: &str,
        instance_id: &str,
        pid: i64,
        started_at_ms: i64,
        heartbeat_at_ms: i64,
        mark: PresenceMark,
    ) -> EngineInstanceRecord {
        EngineInstanceRecord {
            project: ProjectKey::new(project),
            instance_id: instance_id.into(),
            pid,
            boot_id: host().boot_id,
            process_started_at_ms: started_at_ms,
            host_id: host().host_id,
            heartbeat_at_ms,
            presence_mark: mark,
        }
    }

    fn fake_probe(results: &[(i64, StartIdentity)]) -> ProcessProbe {
        let table: HashMap<i64, StartIdentity> = results.iter().copied().collect();
        Arc::new(move |pid| {
            table
                .get(&pid)
                .copied()
                .unwrap_or(StartIdentity::NotRunning)
        })
    }

    async fn store() -> Arc<CoordinationStore> {
        Arc::new(
            CoordinationStore::open_in_memory(&settings())
                .await
                .unwrap(),
        )
    }

    fn presence(
        store: Arc<CoordinationStore>,
        project: &str,
        identity: EngineIdentity,
        active: bool,
        probe: ProcessProbe,
    ) -> EnginePresence {
        let access = store.authorize(ProjectKey::new(project));
        let mut settings = settings();
        settings.presence_heartbeat_interval_ms = 20;
        settings.presence_stale_after_ms = STALE_AFTER_MS as u64;
        EnginePresence::new(store, access, identity, Arc::new(move || active), &settings)
            .unwrap()
            .with_process_probe(probe)
    }

    #[tokio::test]
    async fn recorded_presence_reads_back_with_every_identity_field() {
        let store = store().await;
        let engine = presence(
            store.clone(),
            "alpha",
            identity("engine-1", 4242, 990),
            true,
            fake_probe(&[(4242, StartIdentity::Started(990))]),
        );
        engine.heartbeat(NOW_MS).await.unwrap();

        let access = store.authorize(ProjectKey::new("alpha"));
        let stored = store
            .get_engine_instance(&access, "engine-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.project, ProjectKey::new("alpha"));
        assert_eq!(stored.instance_id, "engine-1");
        assert_eq!(stored.pid, 4242);
        assert_eq!(stored.boot_id, host().boot_id);
        assert_eq!(stored.host_id, host().host_id);
        assert_eq!(stored.process_started_at_ms, 990);
        assert_eq!(stored.heartbeat_at_ms, NOW_MS);
        assert_eq!(stored.presence_mark, PresenceMark::Active);
    }

    #[test]
    fn a_reused_pid_classifies_dead_not_alive() {
        let record = peer_record("alpha", "engine-2", 4242, 990, NOW_MS, PresenceMark::Active);

        assert_eq!(
            classify(
                &record,
                &host(),
                StartIdentity::Started(7_100),
                NOW_MS,
                STALE_AFTER_MS
            ),
            PresenceState::Dead
        );
        assert_eq!(
            classify(
                &record,
                &host(),
                StartIdentity::NotRunning,
                NOW_MS,
                STALE_AFTER_MS
            ),
            PresenceState::Dead
        );
        assert_eq!(
            classify(
                &record,
                &host(),
                StartIdentity::Started(990),
                NOW_MS,
                STALE_AFTER_MS
            ),
            PresenceState::AliveActive
        );
    }

    #[test]
    fn an_unobtainable_start_identity_is_unknown_and_never_alive() {
        let record = peer_record("alpha", "engine-2", 4242, 990, NOW_MS, PresenceMark::Active);
        let unobtainable = peer_record(
            "alpha",
            "engine-3",
            4242,
            START_IDENTITY_UNOBTAINABLE,
            NOW_MS,
            PresenceMark::Active,
        );

        let probed = classify(
            &record,
            &host(),
            StartIdentity::Unknown,
            NOW_MS,
            STALE_AFTER_MS,
        );
        let recorded = classify(
            &unobtainable,
            &host(),
            StartIdentity::Started(990),
            NOW_MS,
            STALE_AFTER_MS,
        );

        assert_eq!(probed, PresenceState::Unknown);
        assert_eq!(recorded, PresenceState::Unknown);
        assert!(!probed.is_alive());
        assert!(!recorded.is_alive());
        assert_eq!(
            EngineIdentity {
                start: StartIdentity::Unknown,
                ..identity("engine-4", 4242, 990)
            }
            .started_at_ms(),
            START_IDENTITY_UNOBTAINABLE
        );
    }

    #[test]
    fn a_reboot_kills_the_record_but_a_foreign_host_stays_unknown() {
        let mut rebooted =
            peer_record("alpha", "engine-2", 4242, 990, NOW_MS, PresenceMark::Active);
        rebooted.boot_id = "boot-previous".into();
        let mut foreign = peer_record("alpha", "engine-3", 4242, 990, NOW_MS, PresenceMark::Active);
        foreign.host_id = "host-b".into();

        assert_eq!(
            classify(
                &rebooted,
                &host(),
                StartIdentity::Started(990),
                NOW_MS,
                STALE_AFTER_MS
            ),
            PresenceState::Dead
        );
        assert_eq!(
            classify(
                &foreign,
                &host(),
                StartIdentity::Started(990),
                NOW_MS,
                STALE_AFTER_MS
            ),
            PresenceState::Unknown
        );
    }

    #[tokio::test]
    async fn the_four_states_stay_distinguishable_end_to_end() {
        let store = store().await;
        let access = store.authorize(ProjectKey::new("alpha"));
        for record in [
            peer_record("alpha", "active", 11, 110, NOW_MS, PresenceMark::Active),
            peer_record("alpha", "idle", 12, 120, NOW_MS, PresenceMark::Idle),
            peer_record(
                "alpha",
                "unreachable",
                13,
                130,
                NOW_MS - STALE_AFTER_MS - 1,
                PresenceMark::Active,
            ),
            peer_record("alpha", "dead", 14, 140, NOW_MS, PresenceMark::Active),
            peer_record("alpha", "unsure", 15, 150, NOW_MS, PresenceMark::Active),
        ] {
            store.put_engine_instance(&access, &record).await.unwrap();
        }
        let engine = presence(
            store.clone(),
            "alpha",
            identity("engine-self", 10, 100),
            true,
            fake_probe(&[
                (10, StartIdentity::Started(100)),
                (11, StartIdentity::Started(110)),
                (12, StartIdentity::Started(120)),
                (13, StartIdentity::Started(130)),
                (14, StartIdentity::Started(999)),
                (15, StartIdentity::Unknown),
            ]),
        );

        engine.refresh(NOW_MS).await.unwrap();
        let peers = project_presence("alpha").unwrap().peers;
        let states: BTreeMap<String, PresenceState> = peers
            .iter()
            .map(|peer| (peer.instance_id.clone(), peer.state))
            .collect();
        forget("alpha");

        assert_eq!(states.len(), 5);
        assert_eq!(states["active"], PresenceState::AliveActive);
        assert_eq!(states["idle"], PresenceState::AliveIdle);
        assert_eq!(states["unreachable"], PresenceState::Unreachable);
        assert_eq!(states["dead"], PresenceState::Dead);
        assert_eq!(states["unsure"], PresenceState::Unknown);
        assert_eq!(peer_liveness(&peers), PeerLiveness::RunningElsewhere);
    }

    #[tokio::test]
    async fn another_projects_engines_are_not_disclosed() {
        let store = store().await;
        let other = store.authorize(ProjectKey::new("beta"));
        store
            .put_engine_instance(
                &other,
                &peer_record("beta", "beta-engine", 21, 210, NOW_MS, PresenceMark::Active),
            )
            .await
            .unwrap();
        let engine = presence(
            store.clone(),
            "alpha",
            identity("engine-self", 10, 100),
            true,
            fake_probe(&[
                (10, StartIdentity::Started(100)),
                (21, StartIdentity::Started(210)),
            ]),
        );

        engine.refresh(NOW_MS).await.unwrap();
        let peers = project_presence("alpha").unwrap().peers;
        let beta = project_presence("beta");
        forget("alpha");

        assert!(peers.is_empty());
        assert_eq!(peer_liveness(&peers), PeerLiveness::NoOtherEngine);
        assert_eq!(beta, None);
    }

    #[tokio::test]
    async fn the_heartbeat_task_is_cancelled_and_joined_on_shutdown() {
        let store = store().await;
        let access = store.authorize(ProjectKey::new("alpha"));
        let engine = Arc::new(presence(
            store.clone(),
            "alpha",
            identity("engine-1", 4242, 990),
            true,
            fake_probe(&[(4242, StartIdentity::Started(990))]),
        ));
        let service = PresenceService::spawn(engine.clone());
        let mut beats = 0;
        while beats < 2 {
            tokio::time::sleep(Duration::from_millis(25)).await;
            if store
                .get_engine_instance(&access, "engine-1")
                .await
                .unwrap()
                .is_some()
            {
                beats += 1;
            }
        }

        tokio::time::timeout(Duration::from_secs(5), service.shutdown())
            .await
            .unwrap()
            .unwrap();
        let after_shutdown = store
            .get_engine_instance(&access, "engine-1")
            .await
            .unwrap()
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let later = store
            .get_engine_instance(&access, "engine-1")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(after_shutdown.presence_mark, PresenceMark::Closed);
        assert_eq!(later, after_shutdown);
        assert_eq!(project_presence("alpha"), None);
    }

    #[tokio::test]
    async fn the_scheduler_decides_whether_this_engine_advertises_work() {
        let scheduler = Arc::new(refact_work_core::AdmissionScheduler::new(settings()));
        let scope = refact_work_core::WorkScope::single("alpha");
        let activity = scheduler_activity(scheduler.clone(), scope.project.clone());
        assert!(!activity());

        let ticket = scheduler
            .reserve(refact_work_core::AdmissionRequest::new(
                refact_work_core::WorkKey::trajectory("chat-1").run(1),
                scope,
            ))
            .unwrap();
        let permit = ticket.acquire().await.unwrap();
        assert!(activity());

        drop(permit);
        assert!(!activity());
    }

    #[test]
    fn a_process_name_with_spaces_and_parens_does_not_shift_the_start_field() {
        let mut fields = vec!["7".to_string(), "(we (are) here)".to_string()];
        fields.extend((3..=21).map(|field| field.to_string()));
        fields.push("8899".to_string());

        assert_eq!(parse_start_ticks(&fields.join(" ")), Some(8899));
        assert_eq!(parse_start_ticks("garbage without a comm field"), None);
    }

    #[test]
    fn this_process_is_probed_as_started_and_an_impossible_pid_is_not_running() {
        let detected = EngineIdentity::detect("engine-self");

        assert_eq!(detected.pid, std::process::id() as i64);
        if cfg!(target_os = "linux") {
            assert!(matches!(detected.start, StartIdentity::Started(_)));
            assert!(detected.started_at_ms() >= 0);
            assert_eq!(probe_process_start(i64::MAX), StartIdentity::NotRunning);
        } else {
            assert_eq!(detected.start, StartIdentity::Unknown);
            assert_eq!(detected.started_at_ms(), START_IDENTITY_UNOBTAINABLE);
        }
    }

    #[serial_test::serial(runtime_settings)]
    #[tokio::test]
    async fn production_startup_publishes_presence_and_shutdown_closes_it() {
        crate::runtime_settings::reset_for_test();
        let workspace = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            workspace.path().join("cache"),
            config.path().to_path_buf(),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let app = AppState::from_gcx(gcx.clone()).await;
        let project = roots::project_identity(workspace.path());
        forget(&project);

        let service = start_background_presence(&app)
            .await
            .expect("enabled coordination starts presence");
        tokio::time::timeout(Duration::from_secs(5), async {
            while project_liveness(&project).is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("startup heartbeat publishes project liveness");
        assert_eq!(
            project_liveness(&project),
            Some(PeerLiveness::NoOtherEngine)
        );

        let settings = crate::runtime_settings::coordination_for_project(workspace.path())
            .await
            .unwrap();
        let path = refact_work_core::default_store_path(&app.paths.cache_dir);
        let store = super::super::outbox::store_for_path(&path, &settings)
            .await
            .unwrap();
        let project_key = ProjectKey::new(project.clone());
        let access = store.authorize(project_key);
        let engine_id = crate::coordination::engine_instance_id();
        let stored = store
            .get_engine_instance(&access, &engine_id)
            .await
            .unwrap()
            .expect("startup heartbeat records this engine");
        assert!(classify(
            &stored,
            &HostIdentity::local(),
            probe_process_start(stored.pid),
            now_ms(),
            settings.presence_stale_after_ms as i64,
        )
        .is_alive());

        service.shutdown().await.unwrap();
        let closed = store
            .get_engine_instance(&access, &engine_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(closed.presence_mark, PresenceMark::Closed);
        assert_eq!(project_liveness(&project), None);
        crate::runtime_settings::reset_for_test();
    }

    #[serial_test::serial(runtime_settings)]
    #[tokio::test]
    async fn disabled_coordination_does_not_publish_presence() {
        crate::runtime_settings::reset_for_test();
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
        let project = roots::project_identity(workspace.path());
        forget(&project);
        let disabled = crate::tools::settings_guard::SettingsGuard::install(|settings| {
            settings.coordination.enabled = false;
        });

        assert!(start_background_presence(&app).await.is_none());
        assert_eq!(project_liveness(&project), None);
        drop(disabled);
        crate::runtime_settings::reset_for_test();
    }
}
