use std::path::Path;
use std::sync::{Arc, OnceLock};
#[cfg(test)]
use std::sync::RwLock;
use std::time::Instant;

use rand::RngCore;
use serde::Serialize;
use sha2::{Digest, Sha256};

pub const PERFORMANCE_DIAGNOSTICS_ENV: &str = "REFACT_PERF_DIAGNOSTICS";
pub const PERFORMANCE_DIAGNOSTICS_SCHEMA_VERSION: u8 = 1;
const ID_HASH_HEX_CHARS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PerfComponent {
    TrajectorySnapshot,
    TrajectorySerialize,
    TrajectoryAtomicWrite,
    TrajectoryCommit,
    TrajectoryIndexLockWait,
    TrajectoryIndexRead,
    TrajectoryIndexWrite,
    TrajectoryIndexRebuild,
}

impl PerfComponent {
    pub const ALL: [Self; 8] = [
        Self::TrajectorySnapshot,
        Self::TrajectorySerialize,
        Self::TrajectoryAtomicWrite,
        Self::TrajectoryCommit,
        Self::TrajectoryIndexLockWait,
        Self::TrajectoryIndexRead,
        Self::TrajectoryIndexWrite,
        Self::TrajectoryIndexRebuild,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TrajectorySnapshot => "trajectory.snapshot",
            Self::TrajectorySerialize => "trajectory.serialize",
            Self::TrajectoryAtomicWrite => "trajectory.atomic_write",
            Self::TrajectoryCommit => "trajectory.commit",
            Self::TrajectoryIndexLockWait => "trajectory.index_lock_wait",
            Self::TrajectoryIndexRead => "trajectory.index_read",
            Self::TrajectoryIndexWrite => "trajectory.index_write",
            Self::TrajectoryIndexRebuild => "trajectory.index_rebuild",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PerfOutcome {
    Success,
    Failure,
}

impl PerfOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PerfEvent {
    pub schema_version: u8,
    pub component: &'static str,
    pub outcome: &'static str,
    pub elapsed_us: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trajectory_version: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_depth: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_id_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_hash: Option<String>,
}

pub trait PerfClock: Send + Sync {
    fn now_us(&self) -> u64;
}

pub trait PerfSink: Send + Sync {
    fn record(&self, event: PerfEvent);
}

struct MonotonicClock {
    origin: Instant,
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl PerfClock for MonotonicClock {
    fn now_us(&self) -> u64 {
        self.origin
            .elapsed()
            .as_micros()
            .try_into()
            .unwrap_or(u64::MAX)
    }
}

struct TracingSink;

impl PerfSink for TracingSink {
    fn record(&self, event: PerfEvent) {
        tracing::info!(
            target: "perf_diagnostics",
            schema_version = event.schema_version,
            component = event.component,
            outcome = event.outcome,
            elapsed_us = event.elapsed_us,
            size_bytes = ?event.size_bytes,
            item_count = ?event.item_count,
            trajectory_version = ?event.trajectory_version,
            queue_depth = ?event.queue_depth,
            chat_id_hash = ?event.chat_id_hash,
            path_hash = ?event.path_hash,
            "trajectory_performance"
        );
    }
}

pub struct PerfRecorder {
    clock: Arc<dyn PerfClock>,
    sink: Arc<dyn PerfSink>,
    salt: [u8; 32],
}

impl PerfRecorder {
    pub fn new(clock: Arc<dyn PerfClock>, sink: Arc<dyn PerfSink>) -> Self {
        let mut salt = [0; 32];
        rand::thread_rng().fill_bytes(&mut salt);
        Self { clock, sink, salt }
    }

    #[cfg(test)]
    pub(crate) fn with_salt(
        clock: Arc<dyn PerfClock>,
        sink: Arc<dyn PerfSink>,
        salt: [u8; 32],
    ) -> Self {
        Self { clock, sink, salt }
    }

    fn hash_bytes(&self, value: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.salt);
        hasher.update(value);
        let hash = hex::encode(hasher.finalize());
        hash[..ID_HASH_HEX_CHARS].to_string()
    }

    #[cfg(test)]
    pub(crate) fn hash_identity_for_test(&self, value: &str) -> String {
        self.hash_bytes(value.as_bytes())
    }

    fn hash_path(&self, path: &Path) -> String {
        self.hash_bytes(path.as_os_str().to_string_lossy().as_bytes())
    }
}

pub struct ActivePerfSpan {
    recorder: Arc<PerfRecorder>,
    component: PerfComponent,
    active_started_us: Option<u64>,
    accumulated_us: u64,
    chat_id_hash: Option<String>,
    path_hash: Option<String>,
}

pub enum PerfSpan {
    Disabled,
    Active(ActivePerfSpan),
}

impl PerfSpan {
    pub fn pause(&mut self) {
        let Self::Active(active) = self else {
            return;
        };
        let Some(started_us) = active.active_started_us.take() else {
            return;
        };
        active.accumulated_us = active
            .accumulated_us
            .saturating_add(active.recorder.clock.now_us().saturating_sub(started_us));
    }

    pub fn resume(&mut self) {
        let Self::Active(active) = self else {
            return;
        };
        if active.active_started_us.is_none() {
            active.active_started_us = Some(active.recorder.clock.now_us());
        }
    }

    pub fn finish(
        self,
        outcome: PerfOutcome,
        size_bytes: Option<u64>,
        item_count: Option<u64>,
        trajectory_version: Option<u64>,
        queue_depth: Option<u64>,
    ) {
        let Self::Active(active) = self else {
            return;
        };
        let elapsed_us = active.accumulated_us.saturating_add(
            active
                .active_started_us
                .map(|started_us| active.recorder.clock.now_us().saturating_sub(started_us))
                .unwrap_or(0),
        );
        active.recorder.sink.record(PerfEvent {
            schema_version: PERFORMANCE_DIAGNOSTICS_SCHEMA_VERSION,
            component: active.component.as_str(),
            outcome: outcome.as_str(),
            elapsed_us,
            size_bytes,
            item_count,
            trajectory_version,
            queue_depth,
            chat_id_hash: active.chat_id_hash,
            path_hash: active.path_hash,
        });
    }
}

static PROCESS_RECORDER: OnceLock<Arc<PerfRecorder>> = OnceLock::new();
static PROCESS_RECORDER_INITIALIZED: OnceLock<()> = OnceLock::new();

#[cfg(test)]
static TEST_RECORDER: OnceLock<RwLock<Option<Arc<PerfRecorder>>>> = OnceLock::new();

#[cfg(test)]
fn test_recorder_slot() -> &'static RwLock<Option<Arc<PerfRecorder>>> {
    TEST_RECORDER.get_or_init(|| RwLock::new(None))
}

pub fn initialize_from_environment() {
    PROCESS_RECORDER_INITIALIZED.get_or_init(|| {
        let enabled = std::env::var(PERFORMANCE_DIAGNOSTICS_ENV)
            .ok()
            .is_some_and(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "YES"));
        if enabled {
            let _ = PROCESS_RECORDER.set(Arc::new(PerfRecorder::new(
                Arc::new(MonotonicClock::default()),
                Arc::new(TracingSink),
            )));
        }
    });
}

fn active_recorder() -> Option<Arc<PerfRecorder>> {
    PROCESS_RECORDER.get().cloned().or_else(|| {
        #[cfg(test)]
        {
            return test_recorder_slot()
                .read()
                .ok()
                .and_then(|recorder| recorder.clone());
        }
        #[cfg(not(test))]
        None
    })
}

pub fn span(component: PerfComponent, chat_id: Option<&str>, path: Option<&Path>) -> PerfSpan {
    span_with_recorder(active_recorder(), component, chat_id, path)
}

fn span_with_recorder(
    recorder: Option<Arc<PerfRecorder>>,
    component: PerfComponent,
    chat_id: Option<&str>,
    path: Option<&Path>,
) -> PerfSpan {
    let Some(recorder) = recorder else {
        return PerfSpan::Disabled;
    };
    let chat_id_hash = chat_id.map(|chat_id| recorder.hash_bytes(chat_id.as_bytes()));
    let path_hash = path.map(|path| recorder.hash_path(path));
    PerfSpan::Active(ActivePerfSpan {
        active_started_us: Some(recorder.clock.now_us()),
        accumulated_us: 0,
        recorder,
        component,
        chat_id_hash,
        path_hash,
    })
}

#[cfg(test)]
pub(crate) struct TestRecorderGuard {
    previous: Option<Arc<PerfRecorder>>,
}

#[cfg(test)]
impl Drop for TestRecorderGuard {
    fn drop(&mut self) {
        *test_recorder_slot()
            .write()
            .expect("performance recorder lock poisoned") = self.previous.take();
    }
}

#[cfg(test)]
pub(crate) fn install_test_recorder(recorder: Arc<PerfRecorder>) -> TestRecorderGuard {
    let mut slot = test_recorder_slot()
        .write()
        .expect("performance recorder lock poisoned");
    let previous = slot.replace(recorder);
    TestRecorderGuard { previous }
}

#[cfg(test)]
pub(crate) struct MemoryPerfSink {
    events: std::sync::Mutex<Vec<PerfEvent>>,
}

#[cfg(test)]
impl MemoryPerfSink {
    pub(crate) fn new() -> Self {
        Self {
            events: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn events(&self) -> Vec<PerfEvent> {
        self.events
            .lock()
            .expect("performance event lock poisoned")
            .clone()
    }
}

#[cfg(test)]
impl PerfSink for MemoryPerfSink {
    fn record(&self, event: PerfEvent) {
        self.events
            .lock()
            .expect("performance event lock poisoned")
            .push(event);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    use super::*;

    struct TestClock {
        now: AtomicU64,
    }

    impl TestClock {
        fn new(now: u64) -> Self {
            Self {
                now: AtomicU64::new(now),
            }
        }

        fn advance(&self, elapsed_us: u64) {
            self.now.fetch_add(elapsed_us, Ordering::SeqCst);
        }
    }

    impl PerfClock for TestClock {
        fn now_us(&self) -> u64 {
            self.now.load(Ordering::SeqCst)
        }
    }

    #[test]
    fn disabled_span_does_not_touch_clock_or_sink() {
        let span = span_with_recorder(None, PerfComponent::TrajectoryCommit, None, None);
        assert!(matches!(span, PerfSpan::Disabled));
        span.finish(PerfOutcome::Success, None, None, None, None);
    }

    #[test]
    fn recorder_hashes_identity_and_records_injected_elapsed_time() {
        let clock = Arc::new(TestClock::new(100));
        let sink = Arc::new(MemoryPerfSink::new());
        let recorder = Arc::new(PerfRecorder::with_salt(
            clock.clone(),
            sink.clone(),
            [7; 32],
        ));
        let chat_id = "chat-secret-123";
        let path =
            Path::new("/home/example/private/project/.refact/trajectories/chat-secret-123.json");
        let span = span_with_recorder(
            Some(recorder.clone()),
            PerfComponent::TrajectorySerialize,
            Some(chat_id),
            Some(path),
        );
        clock.advance(42);
        span.finish(PerfOutcome::Success, Some(512), Some(3), Some(9), Some(2));

        let events = sink.events();
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.elapsed_us, 42);
        assert_eq!(event.component, "trajectory.serialize");
        let expected_chat_hash = recorder.hash_identity_for_test(chat_id);
        let expected_path_hash = recorder.hash_path(path);
        assert_eq!(
            event.chat_id_hash.as_deref(),
            Some(expected_chat_hash.as_str())
        );
        assert_eq!(
            event.path_hash.as_deref(),
            Some(expected_path_hash.as_str())
        );
        assert_eq!(event.size_bytes, Some(512));
        assert_eq!(event.item_count, Some(3));
        assert_eq!(event.trajectory_version, Some(9));
        assert_eq!(event.queue_depth, Some(2));
        let rendered = format!("{event:?}");
        assert!(!rendered.contains(chat_id));
        assert!(!rendered.contains(path.to_str().unwrap()));
    }

    #[test]
    fn paused_span_accumulates_only_active_time() {
        let clock = Arc::new(TestClock::new(100));
        let sink = Arc::new(MemoryPerfSink::new());
        let recorder = Arc::new(PerfRecorder::with_salt(
            clock.clone(),
            sink.clone(),
            [7; 32],
        ));
        let mut span = span_with_recorder(
            Some(recorder),
            PerfComponent::TrajectorySerialize,
            None,
            None,
        );

        clock.advance(7);
        span.pause();
        clock.advance(100);
        span.resume();
        clock.advance(11);
        span.finish(PerfOutcome::Success, None, None, None, None);

        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].elapsed_us, 18);
    }

    #[test]
    fn component_and_outcome_labels_are_bounded_to_the_schema() {
        let labels: Vec<_> = PerfComponent::ALL
            .iter()
            .map(|component| component.as_str())
            .collect();
        assert_eq!(labels.len(), 8);
        assert!(labels.iter().all(|label| label.len() <= 32));
        assert_eq!(PerfOutcome::Success.as_str(), "success");
        assert_eq!(PerfOutcome::Failure.as_str(), "failure");
    }
}
