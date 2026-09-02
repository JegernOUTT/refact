use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use rand::RngCore;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::perf_telemetry::PerformanceTelemetry;

pub const PERFORMANCE_DIAGNOSTICS_ENV: &str = "REFACT_PERF_DIAGNOSTICS";
pub const PERFORMANCE_DIAGNOSTICS_SCHEMA_VERSION: u8 = 1;
const ID_HASH_HEX_CHARS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PerfComponent {
    TrajectorySnapshot,
    TrajectorySerialize,
    TrajectoryAtomicWrite,
    TrajectoryCommit,
    TrajectoryIndexLockWait,
    TrajectoryIndexRead,
    TrajectoryIndexWrite,
    TrajectoryIndexRebuild,
    CommandQueueWait,
    StreamFirstDelta,
    SseSerialize,
    SseBroadcast,
    SseLagged,
    ToolConfirmationWait,
    ToolCatalogBuild,
    ToolMutableVectorBuild,
    ToolPoolParallelExpansion,
    ToolSessionExtraction,
    ToolCatalogPoolAcquire,
    ToolAliasResolution,
    ToolConfirmationPreflight,
    ToolPolicyLookup,
    ToolExecutionWait,
    ToolExecutionLookup,
    ToolSemaphoreWait,
    ToolRuntime,
    ToolPreHook,
    ToolPostHook,
    ToolResultPostprocess,
    ToolResultMerge,
    ToolSessionMergeEvents,
    ToolCheckpointScheduling,
    EnrichmentAttempt,
    EnrichmentDecisionFirstUser,
    EnrichmentDecisionForced,
    EnrichmentDecisionSignaled,
    EnrichmentSkipNoUser,
    EnrichmentSkipAlreadyPresent,
    EnrichmentSkipEmptyQuery,
    EnrichmentSkipCommand,
    EnrichmentSkipThreshold,
    EnrichmentSessionSnapshot,
    EnrichmentExistingContextScan,
    EnrichmentQueryNormalize,
    EnrichmentRootDiscovery,
    EnrichmentCurrentRootResolve,
    EnrichmentVecdbLockWait,
    EnrichmentVecdbLockHold,
    EnrichmentEmbedding,
    EnrichmentScopedSearch,
    EnrichmentMergeDedup,
    EnrichmentFileReread,
    EnrichmentFallback,
    EnrichmentCardBuild,
    EnrichmentCacheMiss,
    EnrichmentCacheHit,
    EnrichmentCacheCoalesced,
    EnrichmentInsertion,
    EnrichmentInsertionStale,
    EnrichmentPersistenceScheduling,
    TrajectoryIndexCoordinatorLoad,
    TrajectoryIndexCoordinatorReconcile,
    TrajectoryIndexCoordinatorFlush,
    TrajectoryIndexEnqueue,
    TrajectoryIndexCacheHit,
    StreamPrepare,
    StreamTokenCountRequest,
    StreamRequestSend,
    StreamProviderTtft,
    StreamFirstContentDelta,
    TrajectorySaveMutexWait,
    TrajectoryMetricScan,
    ToolIntegrationToolsBuild,
}

impl PerfComponent {
    pub const ALL: [Self; 73] = [
        Self::TrajectorySnapshot,
        Self::TrajectorySerialize,
        Self::TrajectoryAtomicWrite,
        Self::TrajectoryCommit,
        Self::TrajectoryIndexLockWait,
        Self::TrajectoryIndexRead,
        Self::TrajectoryIndexWrite,
        Self::TrajectoryIndexRebuild,
        Self::CommandQueueWait,
        Self::StreamFirstDelta,
        Self::SseSerialize,
        Self::SseBroadcast,
        Self::SseLagged,
        Self::ToolConfirmationWait,
        Self::ToolCatalogBuild,
        Self::ToolMutableVectorBuild,
        Self::ToolPoolParallelExpansion,
        Self::ToolSessionExtraction,
        Self::ToolCatalogPoolAcquire,
        Self::ToolAliasResolution,
        Self::ToolConfirmationPreflight,
        Self::ToolPolicyLookup,
        Self::ToolExecutionWait,
        Self::ToolExecutionLookup,
        Self::ToolSemaphoreWait,
        Self::ToolRuntime,
        Self::ToolPreHook,
        Self::ToolPostHook,
        Self::ToolResultPostprocess,
        Self::ToolResultMerge,
        Self::ToolSessionMergeEvents,
        Self::ToolCheckpointScheduling,
        Self::EnrichmentAttempt,
        Self::EnrichmentDecisionFirstUser,
        Self::EnrichmentDecisionForced,
        Self::EnrichmentDecisionSignaled,
        Self::EnrichmentSkipNoUser,
        Self::EnrichmentSkipAlreadyPresent,
        Self::EnrichmentSkipEmptyQuery,
        Self::EnrichmentSkipCommand,
        Self::EnrichmentSkipThreshold,
        Self::EnrichmentSessionSnapshot,
        Self::EnrichmentExistingContextScan,
        Self::EnrichmentQueryNormalize,
        Self::EnrichmentRootDiscovery,
        Self::EnrichmentCurrentRootResolve,
        Self::EnrichmentVecdbLockWait,
        Self::EnrichmentVecdbLockHold,
        Self::EnrichmentEmbedding,
        Self::EnrichmentScopedSearch,
        Self::EnrichmentMergeDedup,
        Self::EnrichmentFileReread,
        Self::EnrichmentFallback,
        Self::EnrichmentCardBuild,
        Self::EnrichmentCacheMiss,
        Self::EnrichmentCacheHit,
        Self::EnrichmentCacheCoalesced,
        Self::EnrichmentInsertion,
        Self::EnrichmentInsertionStale,
        Self::EnrichmentPersistenceScheduling,
        Self::TrajectoryIndexCoordinatorLoad,
        Self::TrajectoryIndexCoordinatorReconcile,
        Self::TrajectoryIndexCoordinatorFlush,
        Self::TrajectoryIndexEnqueue,
        Self::TrajectoryIndexCacheHit,
        Self::StreamPrepare,
        Self::StreamTokenCountRequest,
        Self::StreamRequestSend,
        Self::StreamProviderTtft,
        Self::StreamFirstContentDelta,
        Self::TrajectorySaveMutexWait,
        Self::TrajectoryMetricScan,
        Self::ToolIntegrationToolsBuild,
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
            Self::CommandQueueWait => "command.queue_wait",
            Self::StreamFirstDelta => "stream.first_delta",
            Self::StreamPrepare => "stream.prepare",
            Self::StreamTokenCountRequest => "stream.token_count_request",
            Self::StreamRequestSend => "stream.request_send",
            Self::StreamProviderTtft => "stream.provider_ttft",
            Self::StreamFirstContentDelta => "stream.first_content_delta",
            Self::SseSerialize => "sse.serialize",
            Self::SseBroadcast => "sse.broadcast",
            Self::SseLagged => "sse.lagged",
            Self::ToolConfirmationWait => "tool.confirmation_wait",
            Self::ToolCatalogBuild => "tool.catalog_build",
            Self::ToolMutableVectorBuild => "tool.mutable_vector_build",
            Self::ToolPoolParallelExpansion => "tool.pool_parallel_expansion",
            Self::ToolSessionExtraction => "tool.session_extraction",
            Self::ToolCatalogPoolAcquire => "tool.catalog_pool_acquire",
            Self::ToolAliasResolution => "tool.alias_resolution",
            Self::ToolConfirmationPreflight => "tool.confirmation_preflight",
            Self::ToolPolicyLookup => "tool.policy_lookup",
            Self::ToolExecutionWait => "tool.execution_wait",
            Self::ToolExecutionLookup => "tool.execution_lookup",
            Self::ToolSemaphoreWait => "tool.semaphore_wait",
            Self::ToolRuntime => "tool.runtime",
            Self::ToolPreHook => "tool.pre_hook",
            Self::ToolPostHook => "tool.post_hook",
            Self::ToolResultPostprocess => "tool.result_postprocess",
            Self::ToolResultMerge => "tool.result_merge",
            Self::ToolSessionMergeEvents => "tool.session_merge_events",
            Self::ToolCheckpointScheduling => "tool.checkpoint_scheduling",
            Self::EnrichmentAttempt => "enrichment.attempt",
            Self::EnrichmentDecisionFirstUser => "enrichment.decision.first_user",
            Self::EnrichmentDecisionForced => "enrichment.decision.forced",
            Self::EnrichmentDecisionSignaled => "enrichment.decision.signaled",
            Self::EnrichmentSkipNoUser => "enrichment.skip.no_user",
            Self::EnrichmentSkipAlreadyPresent => "enrichment.skip.already_present",
            Self::EnrichmentSkipEmptyQuery => "enrichment.skip.empty_query",
            Self::EnrichmentSkipCommand => "enrichment.skip.command",
            Self::EnrichmentSkipThreshold => "enrichment.skip.threshold",
            Self::EnrichmentSessionSnapshot => "enrichment.session_snapshot",
            Self::EnrichmentExistingContextScan => "enrichment.existing_context_scan",
            Self::EnrichmentQueryNormalize => "enrichment.query_normalize",
            Self::EnrichmentRootDiscovery => "enrichment.root_discovery",
            Self::EnrichmentCurrentRootResolve => "enrichment.current_root_resolve",
            Self::EnrichmentVecdbLockWait => "enrichment.vecdb_lock_wait",
            Self::EnrichmentVecdbLockHold => "enrichment.vecdb_lock_hold",
            Self::EnrichmentEmbedding => "enrichment.embedding",
            Self::EnrichmentScopedSearch => "enrichment.scoped_search",
            Self::EnrichmentMergeDedup => "enrichment.merge_dedup",
            Self::EnrichmentFileReread => "enrichment.file_reread",
            Self::EnrichmentFallback => "enrichment.fallback",
            Self::EnrichmentCardBuild => "enrichment.card_build",
            Self::EnrichmentCacheMiss => "enrichment.cache_miss",
            Self::EnrichmentCacheHit => "enrichment.cache_hit",
            Self::EnrichmentCacheCoalesced => "enrichment.cache_coalesced",
            Self::EnrichmentInsertion => "enrichment.insertion",
            Self::EnrichmentInsertionStale => "enrichment.insertion_stale",
            Self::EnrichmentPersistenceScheduling => "enrichment.persistence_scheduling",
            Self::TrajectoryIndexCoordinatorLoad => "trajectory.index_coordinator_load",
            Self::TrajectoryIndexCoordinatorReconcile => "trajectory.index_coordinator_reconcile",
            Self::TrajectoryIndexCoordinatorFlush => "trajectory.index_coordinator_flush",
            Self::TrajectoryIndexEnqueue => "trajectory.index_enqueue",
            Self::TrajectoryIndexCacheHit => "trajectory.index_cache_hit",
            Self::TrajectorySaveMutexWait => "trajectory.save_mutex_wait",
            Self::TrajectoryMetricScan => "trajectory.metric_scan",
            Self::ToolIntegrationToolsBuild => "tool.integration_tools_build",
        }
    }

    pub const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ToolExecutionClass {
    Serial = 0,
    Parallel = 1,
}

impl ToolExecutionClass {
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PerfOutcome {
    Success,
    Failure,
    Skipped,
    Cancelled,
}

impl PerfOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Skipped => "skipped",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PerfEvent {
    pub schema_version: u8,
    pub component: &'static str,
    #[serde(skip)]
    pub(crate) component_index: u8,
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
    pub batch_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_class: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_tokens: Option<u64>,
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
            batch_size = ?event.batch_size,
            execution_class = ?event.execution_class,
            estimated_tokens = ?event.estimated_tokens,
            chat_id_hash = ?event.chat_id_hash,
            path_hash = ?event.path_hash,
            "trajectory_performance"
        );
    }
}

struct ProcessPerfSink {
    telemetry: Arc<PerformanceTelemetry>,
    tracing_enabled: bool,
}

impl PerfSink for ProcessPerfSink {
    fn record(&self, event: PerfEvent) {
        self.telemetry.record(&event);
        if self.tracing_enabled {
            TracingSink.record(event);
        }
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

    #[cfg(any(test, feature = "bench"))]
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
    finished: bool,
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
        self.finish_with_metrics(
            outcome,
            size_bytes,
            item_count,
            trajectory_version,
            queue_depth,
            None,
            None,
        );
    }

    pub fn finish_tool(
        self,
        outcome: PerfOutcome,
        batch_size: u64,
        item_count: u64,
        execution_class: Option<ToolExecutionClass>,
    ) {
        self.finish_with_metrics(
            outcome,
            None,
            Some(item_count),
            None,
            None,
            Some(batch_size),
            execution_class.map(ToolExecutionClass::as_u8),
        );
    }

    fn finish_with_metrics(
        mut self,
        outcome: PerfOutcome,
        size_bytes: Option<u64>,
        item_count: Option<u64>,
        trajectory_version: Option<u64>,
        queue_depth: Option<u64>,
        batch_size: Option<u64>,
        execution_class: Option<u8>,
    ) {
        let Self::Active(active) = &mut self else {
            return;
        };
        active.finished = true;
        let elapsed_us = active.accumulated_us.saturating_add(
            active
                .active_started_us
                .map(|started_us| active.recorder.clock.now_us().saturating_sub(started_us))
                .unwrap_or(0),
        );
        active.recorder.sink.record(PerfEvent {
            schema_version: PERFORMANCE_DIAGNOSTICS_SCHEMA_VERSION,
            component: active.component.as_str(),
            component_index: active.component.index() as u8,
            outcome: outcome.as_str(),
            elapsed_us,
            size_bytes,
            item_count,
            trajectory_version,
            queue_depth,
            batch_size,
            execution_class,
            estimated_tokens: None,
            chat_id_hash: active.chat_id_hash.take(),
            path_hash: active.path_hash.take(),
        });
    }
}

impl Drop for PerfSpan {
    fn drop(&mut self) {
        let Self::Active(active) = self else {
            return;
        };
        if active.finished {
            return;
        }
        active.finished = true;
        let elapsed_us = active.accumulated_us.saturating_add(
            active
                .active_started_us
                .map(|started_us| active.recorder.clock.now_us().saturating_sub(started_us))
                .unwrap_or(0),
        );
        active.recorder.sink.record(PerfEvent {
            schema_version: PERFORMANCE_DIAGNOSTICS_SCHEMA_VERSION,
            component: active.component.as_str(),
            component_index: active.component.index() as u8,
            outcome: PerfOutcome::Cancelled.as_str(),
            elapsed_us,
            size_bytes: None,
            item_count: None,
            trajectory_version: None,
            queue_depth: None,
            batch_size: None,
            execution_class: None,
            estimated_tokens: None,
            chat_id_hash: active.chat_id_hash.take(),
            path_hash: active.path_hash.take(),
        });
    }
}

static PROCESS_RECORDER: OnceLock<Arc<PerfRecorder>> = OnceLock::new();
static PROCESS_TELEMETRY: OnceLock<Arc<PerformanceTelemetry>> = OnceLock::new();

#[cfg(any(test, feature = "bench"))]
thread_local! {
    static TEST_RECORDERS: std::cell::RefCell<Vec<Arc<PerfRecorder>>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

pub fn initialize_from_environment() {
    let _ = process_telemetry();
}

pub fn process_telemetry() -> Arc<PerformanceTelemetry> {
    PROCESS_TELEMETRY
        .get_or_init(|| {
            let enabled = diagnostics_enabled_from_environment();
            Arc::new(PerformanceTelemetry::new(enabled))
        })
        .clone()
}

fn diagnostics_enabled_from_environment() -> bool {
    std::env::var(PERFORMANCE_DIAGNOSTICS_ENV)
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

#[cfg(test)]
pub(crate) fn clear_process_telemetry_for_test() {
    let telemetry = process_telemetry();
    telemetry.reset();
}

fn process_recorder() -> Arc<PerfRecorder> {
    PROCESS_RECORDER
        .get_or_init(|| {
            Arc::new(PerfRecorder::new(
                Arc::new(MonotonicClock::default()),
                Arc::new(ProcessPerfSink {
                    telemetry: process_telemetry(),
                    tracing_enabled: diagnostics_enabled_from_environment(),
                }),
            ))
        })
        .clone()
}

fn active_recorder() -> Option<Arc<PerfRecorder>> {
    #[cfg(any(test, feature = "bench"))]
    if let Some(recorder) = TEST_RECORDERS.with(|recorders| recorders.borrow().last().cloned()) {
        return Some(recorder);
    }
    PROCESS_TELEMETRY
        .get()
        .filter(|telemetry| telemetry.enabled())
        .map(|_| process_recorder())
}

pub fn is_enabled() -> bool {
    active_recorder().is_some()
}

pub fn span(component: PerfComponent, chat_id: Option<&str>, path: Option<&Path>) -> PerfSpan {
    span_with_recorder(active_recorder(), component, chat_id, path)
}

pub fn record(
    component: PerfComponent,
    chat_id: Option<&str>,
    outcome: PerfOutcome,
    elapsed_us: u64,
    size_bytes: Option<u64>,
    item_count: Option<u64>,
    queue_depth: Option<u64>,
) {
    let Some(recorder) = active_recorder() else {
        return;
    };
    recorder.sink.record(PerfEvent {
        schema_version: PERFORMANCE_DIAGNOSTICS_SCHEMA_VERSION,
        component: component.as_str(),
        component_index: component.index() as u8,
        outcome: outcome.as_str(),
        elapsed_us,
        size_bytes,
        item_count,
        trajectory_version: None,
        queue_depth,
        batch_size: None,
        execution_class: None,
        estimated_tokens: None,
        chat_id_hash: chat_id.map(|chat_id| recorder.hash_bytes(chat_id.as_bytes())),
        path_hash: None,
    });
}

pub fn record_enrichment(
    component: PerfComponent,
    chat_id: &str,
    outcome: PerfOutcome,
    elapsed_us: u64,
    size_bytes: Option<u64>,
    item_count: Option<u64>,
    estimated_tokens: Option<u64>,
) {
    let Some(recorder) = active_recorder() else {
        return;
    };
    recorder.sink.record(PerfEvent {
        schema_version: PERFORMANCE_DIAGNOSTICS_SCHEMA_VERSION,
        component: component.as_str(),
        component_index: component.index() as u8,
        outcome: outcome.as_str(),
        elapsed_us,
        size_bytes,
        item_count,
        trajectory_version: None,
        queue_depth: None,
        batch_size: None,
        execution_class: None,
        estimated_tokens,
        chat_id_hash: Some(recorder.hash_bytes(chat_id.as_bytes())),
        path_hash: None,
    });
}

#[cfg(any(test, feature = "bench"))]
pub(crate) struct TestRecorderLock(std::sync::Mutex<()>);

#[cfg(any(test, feature = "bench"))]
impl TestRecorderLock {
    pub(crate) const fn new() -> Self {
        Self(std::sync::Mutex::new(()))
    }

    pub(crate) fn lock(&self) -> Result<std::sync::MutexGuard<'_, ()>, std::convert::Infallible> {
        Ok(match self.0.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        })
    }
}

#[cfg(any(test, feature = "bench"))]
pub(crate) static PERF_RECORDER_TEST_LOCK: TestRecorderLock = TestRecorderLock::new();

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
        finished: false,
    })
}

#[cfg(any(test, feature = "bench"))]
pub(crate) struct TestRecorderGuard {
    _not_send: std::marker::PhantomData<std::rc::Rc<()>>,
}

#[cfg(any(test, feature = "bench"))]
impl Drop for TestRecorderGuard {
    fn drop(&mut self) {
        TEST_RECORDERS.with(|recorders| {
            recorders.borrow_mut().pop();
        });
    }
}

#[cfg(any(test, feature = "bench"))]
pub(crate) fn install_test_recorder(recorder: Arc<PerfRecorder>) -> TestRecorderGuard {
    TEST_RECORDERS.with(|recorders| recorders.borrow_mut().push(recorder));
    TestRecorderGuard {
        _not_send: std::marker::PhantomData,
    }
}

#[cfg(any(test, feature = "bench"))]
pub(crate) struct MemoryPerfSink {
    events: std::sync::Mutex<Vec<PerfEvent>>,
}

#[cfg(any(test, feature = "bench"))]
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

#[cfg(any(test, feature = "bench"))]
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

    #[test]
    fn disabled_process_telemetry_keeps_the_default_recording_path_inactive() {
        let _lock = PERF_RECORDER_TEST_LOCK.lock().unwrap();
        let telemetry = process_telemetry();
        let was_enabled = telemetry.enabled();
        telemetry.set_enabled(false);
        telemetry.reset();
        assert!(!is_enabled());
        record(
            PerfComponent::TrajectoryCommit,
            Some("private-chat"),
            PerfOutcome::Success,
            1,
            None,
            None,
            None,
        );
        assert_eq!(process_telemetry().snapshot().components[3].sample_count, 0);
        telemetry.set_enabled(was_enabled);
    }

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
        let _lock = PERF_RECORDER_TEST_LOCK.lock().unwrap();
        let span = span_with_recorder(None, PerfComponent::TrajectoryCommit, None, None);
        assert!(matches!(span, PerfSpan::Disabled));
        span.finish(PerfOutcome::Success, None, None, None, None);
    }

    #[test]
    fn recorder_hashes_identity_and_records_injected_elapsed_time() {
        let _lock = PERF_RECORDER_TEST_LOCK.lock().unwrap();
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
        let _lock = PERF_RECORDER_TEST_LOCK.lock().unwrap();
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
    fn dropping_unfinished_span_records_cancelled_active_time_once() {
        let _lock = PERF_RECORDER_TEST_LOCK.lock().unwrap();
        let clock = Arc::new(TestClock::new(100));
        let sink = Arc::new(MemoryPerfSink::new());
        let recorder = Arc::new(PerfRecorder::with_salt(
            clock.clone(),
            sink.clone(),
            [8; 32],
        ));
        let mut span = span_with_recorder(Some(recorder), PerfComponent::ToolRuntime, None, None);

        clock.advance(7);
        span.pause();
        clock.advance(100);
        drop(span);

        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].outcome, PerfOutcome::Cancelled.as_str());
        assert_eq!(events[0].elapsed_us, 7);
        assert!(events[0].elapsed_us > 0);
    }

    #[test]
    fn finishing_span_does_not_record_cancelled_on_drop() {
        let _lock = PERF_RECORDER_TEST_LOCK.lock().unwrap();
        let clock = Arc::new(TestClock::new(100));
        let sink = Arc::new(MemoryPerfSink::new());
        let recorder = Arc::new(PerfRecorder::with_salt(
            clock.clone(),
            sink.clone(),
            [9; 32],
        ));
        let span = span_with_recorder(Some(recorder), PerfComponent::ToolRuntime, None, None);

        clock.advance(11);
        span.finish(PerfOutcome::Success, None, None, None, None);

        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].outcome, PerfOutcome::Success.as_str());
    }

    #[test]
    fn record_accepts_injected_elapsed_time_without_a_cross_turn_span() {
        let _lock = PERF_RECORDER_TEST_LOCK.lock().unwrap();
        let clock = Arc::new(TestClock::new(100));
        let sink = Arc::new(MemoryPerfSink::new());
        let recorder = Arc::new(PerfRecorder::with_salt(
            clock.clone(),
            sink.clone(),
            [7; 32],
        ));
        let _guard = install_test_recorder(recorder);

        clock.advance(42);
        record(
            PerfComponent::CommandQueueWait,
            None,
            PerfOutcome::Success,
            42,
            None,
            Some(1),
            Some(2),
        );

        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].component, "command.queue_wait");
        assert_eq!(events[0].elapsed_us, 42);
        assert_eq!(events[0].item_count, Some(1));
        assert_eq!(events[0].queue_depth, Some(2));
    }

    #[test]
    fn component_and_outcome_labels_are_bounded_to_the_schema() {
        let _lock = PERF_RECORDER_TEST_LOCK.lock().unwrap();
        let labels: Vec<_> = PerfComponent::ALL
            .iter()
            .map(|component| component.as_str())
            .collect();
        let unique_labels: std::collections::BTreeSet<_> = labels.iter().copied().collect();
        let expected_labels: std::collections::BTreeSet<_> = [
            "trajectory.snapshot",
            "trajectory.serialize",
            "trajectory.atomic_write",
            "trajectory.commit",
            "trajectory.index_lock_wait",
            "trajectory.index_read",
            "trajectory.index_write",
            "trajectory.index_rebuild",
            "command.queue_wait",
            "stream.first_delta",
            "stream.prepare",
            "stream.token_count_request",
            "stream.request_send",
            "stream.provider_ttft",
            "stream.first_content_delta",
            "sse.serialize",
            "sse.broadcast",
            "sse.lagged",
            "tool.confirmation_wait",
            "tool.catalog_build",
            "tool.mutable_vector_build",
            "tool.pool_parallel_expansion",
            "tool.session_extraction",
            "tool.catalog_pool_acquire",
            "tool.alias_resolution",
            "tool.confirmation_preflight",
            "tool.policy_lookup",
            "tool.execution_wait",
            "tool.execution_lookup",
            "tool.semaphore_wait",
            "tool.runtime",
            "tool.pre_hook",
            "tool.post_hook",
            "tool.result_postprocess",
            "tool.result_merge",
            "tool.session_merge_events",
            "tool.checkpoint_scheduling",
            "enrichment.attempt",
            "enrichment.decision.first_user",
            "enrichment.decision.forced",
            "enrichment.decision.signaled",
            "enrichment.skip.no_user",
            "enrichment.skip.already_present",
            "enrichment.skip.empty_query",
            "enrichment.skip.command",
            "enrichment.skip.threshold",
            "enrichment.session_snapshot",
            "enrichment.existing_context_scan",
            "enrichment.query_normalize",
            "enrichment.root_discovery",
            "enrichment.current_root_resolve",
            "enrichment.vecdb_lock_wait",
            "enrichment.vecdb_lock_hold",
            "enrichment.embedding",
            "enrichment.scoped_search",
            "enrichment.merge_dedup",
            "enrichment.file_reread",
            "enrichment.fallback",
            "enrichment.card_build",
            "enrichment.cache_miss",
            "enrichment.cache_hit",
            "enrichment.cache_coalesced",
            "enrichment.insertion",
            "enrichment.insertion_stale",
            "enrichment.persistence_scheduling",
            "trajectory.index_coordinator_load",
            "trajectory.index_coordinator_reconcile",
            "trajectory.index_coordinator_flush",
            "trajectory.index_enqueue",
            "trajectory.index_cache_hit",
            "trajectory.save_mutex_wait",
            "trajectory.metric_scan",
            "tool.integration_tools_build",
        ]
        .into_iter()
        .collect();
        assert_eq!(labels.len(), PerfComponent::ALL.len());
        assert_eq!(unique_labels.len(), labels.len());
        assert_eq!(unique_labels, expected_labels);
        assert!(labels.iter().all(|label| label.len() <= 40));
        assert!(labels.contains(&"command.queue_wait"));
        assert!(labels.contains(&"stream.first_delta"));
        assert!(labels.contains(&"stream.prepare"));
        assert!(labels.contains(&"stream.token_count_request"));
        assert!(labels.contains(&"stream.request_send"));
        assert!(labels.contains(&"stream.provider_ttft"));
        assert!(labels.contains(&"stream.first_content_delta"));
        assert!(labels.contains(&"sse.serialize"));
        assert!(labels.contains(&"sse.broadcast"));
        assert!(labels.contains(&"sse.lagged"));
        assert!(labels.contains(&"tool.confirmation_wait"));
        assert!(labels.contains(&"tool.mutable_vector_build"));
        assert!(labels.contains(&"tool.pool_parallel_expansion"));
        assert!(labels.contains(&"tool.session_extraction"));
        assert!(labels.contains(&"tool.catalog_pool_acquire"));
        assert!(labels.contains(&"tool.execution_wait"));
        assert!(labels.contains(&"tool.execution_lookup"));
        assert!(labels.contains(&"tool.session_merge_events"));
        assert!(labels.contains(&"tool.checkpoint_scheduling"));
        assert!(labels.contains(&"enrichment.embedding"));
        assert!(labels.contains(&"enrichment.vecdb_lock_wait"));
        assert!(labels.contains(&"enrichment.fallback"));
        assert!(labels.contains(&"enrichment.insertion_stale"));
        assert!(labels.contains(&"trajectory.save_mutex_wait"));
        assert!(labels.contains(&"trajectory.metric_scan"));
        assert!(labels.contains(&"tool.integration_tools_build"));
        assert_eq!(PerfOutcome::Success.as_str(), "success");
        assert_eq!(PerfOutcome::Failure.as_str(), "failure");
        assert_eq!(PerfOutcome::Skipped.as_str(), "skipped");
        assert_eq!(PerfOutcome::Cancelled.as_str(), "cancelled");
    }

    #[test]
    fn enrichment_metrics_hash_identity_and_never_store_content() {
        let _lock = PERF_RECORDER_TEST_LOCK.lock().unwrap();
        let sink = Arc::new(MemoryPerfSink::new());
        let recorder = Arc::new(PerfRecorder::with_salt(
            Arc::new(TestClock::new(0)),
            sink.clone(),
            [23; 32],
        ));
        let _guard = install_test_recorder(recorder);
        let chat_id = "private-chat-id";
        let query = "private query secret";

        record_enrichment(
            PerfComponent::EnrichmentQueryNormalize,
            chat_id,
            PerfOutcome::Success,
            7,
            Some(query.len() as u64),
            Some(1),
            Some(5),
        );

        let event = sink.events().pop().expect("enrichment event");
        assert_eq!(event.component, "enrichment.query_normalize");
        assert_eq!(event.size_bytes, Some(query.len() as u64));
        assert_eq!(event.estimated_tokens, Some(5));
        let rendered = serde_json::to_string(&event).unwrap();
        assert!(!rendered.contains(chat_id));
        assert!(!rendered.contains(query));
        assert!(event.chat_id_hash.is_some());
        assert!(event.path_hash.is_none());
    }

    #[test]
    fn tool_span_records_numeric_batch_and_execution_class() {
        let clock = Arc::new(TestClock::new(100));
        let sink = Arc::new(MemoryPerfSink::new());
        let recorder = Arc::new(PerfRecorder::with_salt(
            clock.clone(),
            sink.clone(),
            [7; 32],
        ));
        let span = span_with_recorder(Some(recorder), PerfComponent::ToolRuntime, None, None);

        clock.advance(7);
        span.finish_tool(
            PerfOutcome::Success,
            3,
            1,
            Some(ToolExecutionClass::Parallel),
        );

        let event = sink.events().pop().expect("tool event");
        assert_eq!(event.component, "tool.runtime");
        assert_eq!(event.elapsed_us, 7);
        assert_eq!(event.batch_size, Some(3));
        assert_eq!(event.item_count, Some(1));
        assert_eq!(
            event.execution_class,
            Some(ToolExecutionClass::Parallel.as_u8())
        );
    }

    #[test]
    fn test_recorders_isolate_former_parallel_failures_and_restore_after_panics() {
        use std::sync::Barrier;

        let panic_sink = Arc::new(MemoryPerfSink::new());
        let panic_recorder = Arc::new(PerfRecorder::with_salt(
            Arc::new(TestClock::new(0)),
            panic_sink.clone(),
            [41; 32],
        ));
        let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = install_test_recorder(panic_recorder);
            record(
                PerfComponent::TrajectoryCommit,
                Some("panic-restoration"),
                PerfOutcome::Failure,
                1,
                None,
                None,
                None,
            );
            panic!("test recorder scope panic");
        }));
        assert!(panic_result.is_err());
        assert_eq!(panic_sink.events().len(), 1);

        let barrier = Arc::new(Barrier::new(2));
        let failed_commit_sink = Arc::new(MemoryPerfSink::new());
        let full_soak_sink = Arc::new(MemoryPerfSink::new());
        let failed_commit_recorder = Arc::new(PerfRecorder::with_salt(
            Arc::new(TestClock::new(0)),
            failed_commit_sink.clone(),
            [42; 32],
        ));
        let full_soak_recorder = Arc::new(PerfRecorder::with_salt(
            Arc::new(TestClock::new(0)),
            full_soak_sink.clone(),
            [43; 32],
        ));

        let failed_commit_thread = std::thread::spawn({
            let barrier = barrier.clone();
            move || {
                let _guard = install_test_recorder(failed_commit_recorder);
                barrier.wait();
                for _ in 0..100 {
                    record(
                        PerfComponent::TrajectoryCommit,
                        Some("failed-trajectory-commit"),
                        PerfOutcome::Failure,
                        1,
                        None,
                        None,
                        None,
                    );
                }
            }
        });
        let full_soak_thread = std::thread::spawn({
            let barrier = barrier.clone();
            move || {
                let _guard = install_test_recorder(full_soak_recorder);
                barrier.wait();
                for _ in 0..100 {
                    record(
                        PerfComponent::ToolRuntime,
                        Some("full-soak-fixture"),
                        PerfOutcome::Success,
                        1,
                        None,
                        None,
                        None,
                    );
                }
            }
        });
        failed_commit_thread.join().unwrap();
        full_soak_thread.join().unwrap();

        assert!(failed_commit_sink.events().iter().all(|event| {
            event.component == PerfComponent::TrajectoryCommit.as_str()
                && event.outcome == PerfOutcome::Failure.as_str()
        }));
        assert_eq!(failed_commit_sink.events().len(), 100);
        assert!(full_soak_sink.events().iter().all(|event| {
            event.component == PerfComponent::ToolRuntime.as_str()
                && event.outcome == PerfOutcome::Success.as_str()
        }));
        assert_eq!(full_soak_sink.events().len(), 100);
    }
}
