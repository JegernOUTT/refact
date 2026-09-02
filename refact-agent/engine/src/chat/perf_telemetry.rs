use std::array;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::perf_diagnostics::{PerfComponent, PerfEvent};
use super::trajectory_index::trajectory_index_listing_counters;

pub const PERFORMANCE_TELEMETRY_SCHEMA_VERSION: u8 = 1;
const HISTOGRAM_BUCKETS: usize = 64;

struct ComponentCounters {
    sample_count: AtomicU64,
    success_count: AtomicU64,
    failure_count: AtomicU64,
    skipped_count: AtomicU64,
    cancelled_count: AtomicU64,
    min_us: AtomicU64,
    max_us: AtomicU64,
    histogram: [AtomicU64; HISTOGRAM_BUCKETS],
    last_sample_at_ms: AtomicU64,
    size_bytes_sum: AtomicU64,
    item_count_sum: AtomicU64,
    batch_size_sum: AtomicU64,
}

impl ComponentCounters {
    fn new() -> Self {
        Self {
            sample_count: AtomicU64::new(0),
            success_count: AtomicU64::new(0),
            failure_count: AtomicU64::new(0),
            skipped_count: AtomicU64::new(0),
            cancelled_count: AtomicU64::new(0),
            min_us: AtomicU64::new(u64::MAX),
            max_us: AtomicU64::new(0),
            histogram: array::from_fn(|_| AtomicU64::new(0)),
            last_sample_at_ms: AtomicU64::new(0),
            size_bytes_sum: AtomicU64::new(0),
            item_count_sum: AtomicU64::new(0),
            batch_size_sum: AtomicU64::new(0),
        }
    }

    fn record(&self, event: &PerfEvent) {
        match event.outcome {
            "success" => {
                self.success_count.fetch_add(1, Ordering::Relaxed);
            }
            "failure" => {
                self.failure_count.fetch_add(1, Ordering::Relaxed);
            }
            "cancelled" => {
                self.cancelled_count.fetch_add(1, Ordering::Relaxed);
            }
            _ => {
                self.skipped_count.fetch_add(1, Ordering::Relaxed);
            }
        }
        update_min(&self.min_us, event.elapsed_us);
        update_max(&self.max_us, event.elapsed_us);
        self.histogram[histogram_bucket(event.elapsed_us)].fetch_add(1, Ordering::Relaxed);
        self.last_sample_at_ms.store(now_ms(), Ordering::Relaxed);
        if let Some(value) = event.size_bytes {
            saturating_add(&self.size_bytes_sum, value);
        }
        if let Some(value) = event.item_count {
            saturating_add(&self.item_count_sum, value);
        }
        if let Some(value) = event.batch_size {
            saturating_add(&self.batch_size_sum, value);
        }
        self.sample_count.fetch_add(1, Ordering::Release);
    }

    fn clear(&self) {
        self.sample_count.store(0, Ordering::Relaxed);
        self.success_count.store(0, Ordering::Relaxed);
        self.failure_count.store(0, Ordering::Relaxed);
        self.skipped_count.store(0, Ordering::Relaxed);
        self.cancelled_count.store(0, Ordering::Relaxed);
        self.min_us.store(u64::MAX, Ordering::Relaxed);
        self.max_us.store(0, Ordering::Relaxed);
        for bucket in &self.histogram {
            bucket.store(0, Ordering::Relaxed);
        }
        self.last_sample_at_ms.store(0, Ordering::Relaxed);
        self.size_bytes_sum.store(0, Ordering::Relaxed);
        self.item_count_sum.store(0, Ordering::Relaxed);
        self.batch_size_sum.store(0, Ordering::Relaxed);
    }

    fn snapshot(&self, component: Option<&'static str>) -> PerformanceComponentAggregate {
        let sample_count = self.sample_count.load(Ordering::Acquire);
        let histogram = self
            .histogram
            .iter()
            .map(|bucket| bucket.load(Ordering::Relaxed))
            .collect::<Vec<_>>();
        PerformanceComponentAggregate {
            component,
            sample_count,
            success_count: self.success_count.load(Ordering::Relaxed),
            failure_count: self.failure_count.load(Ordering::Relaxed),
            skipped_count: self.skipped_count.load(Ordering::Relaxed),
            cancelled_count: self.cancelled_count.load(Ordering::Relaxed),
            min_us: (sample_count > 0).then(|| self.min_us.load(Ordering::Relaxed)),
            max_us: (sample_count > 0).then(|| self.max_us.load(Ordering::Relaxed)),
            p50_us: percentile_bucket_us(&histogram, sample_count, 50),
            p95_us: percentile_bucket_us(&histogram, sample_count, 95),
            p99_us: percentile_bucket_us(&histogram, sample_count, 99),
            last_sample_at_ms: (sample_count > 0)
                .then(|| self.last_sample_at_ms.load(Ordering::Relaxed)),
            size_bytes_sum: self.size_bytes_sum.load(Ordering::Relaxed),
            item_count_sum: self.item_count_sum.load(Ordering::Relaxed),
            batch_size_sum: self.batch_size_sum.load(Ordering::Relaxed),
        }
    }
}

pub struct PerformanceTelemetry {
    enabled: AtomicBool,
    state_lock: RwLock<()>,
    collection_started_at_ms: AtomicU64,
    components: [ComponentCounters; PerfComponent::ALL.len()],
}

impl PerformanceTelemetry {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled: AtomicBool::new(enabled),
            state_lock: RwLock::new(()),
            collection_started_at_ms: AtomicU64::new(now_ms()),
            components: array::from_fn(|_| ComponentCounters::new()),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn set_enabled(&self, enabled: bool) {
        let _guard = lock_write(&self.state_lock);
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn record(&self, event: &PerfEvent) -> bool {
        if !self.enabled() {
            return false;
        }
        let _guard = lock_read(&self.state_lock);
        if !self.enabled() {
            return false;
        }
        let Some(component) = self.components.get(event.component_index as usize) else {
            return false;
        };
        if component_label(event.component_index) != Some(event.component) {
            return false;
        }
        component.record(event);
        true
    }

    pub fn reset(&self) {
        let _guard = lock_write(&self.state_lock);
        for component in &self.components {
            component.clear();
        }
        self.collection_started_at_ms
            .store(now_ms(), Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> PerformanceTelemetrySnapshot {
        let _guard = lock_read(&self.state_lock);
        let now_ms = now_ms();
        let collection_started_at_ms = self.collection_started_at_ms.load(Ordering::Relaxed);
        let listing_counters = trajectory_index_listing_counters();
        let listing_calls = listing_counters
            .calls_by_caller
            .iter()
            .fold(0_u64, |total, (_, calls)| total.saturating_add(*calls));
        PerformanceTelemetrySnapshot {
            schema_version: PERFORMANCE_TELEMETRY_SCHEMA_VERSION,
            enabled: self.enabled(),
            collection_started_at_ms,
            uptime_ms: now_ms.saturating_sub(collection_started_at_ms),
            components: PerfComponent::ALL
                .iter()
                .enumerate()
                .map(|(index, component)| self.components[index].snapshot(Some(component.as_str())))
                .collect(),
            rollups: PerformanceTelemetryRollups {
                advancement: self.aggregate_components(&[
                    PerfComponent::TrajectorySnapshot,
                    PerfComponent::TrajectorySerialize,
                    PerfComponent::TrajectoryAtomicWrite,
                    PerfComponent::TrajectoryCommit,
                    PerfComponent::CommandQueueWait,
                    PerfComponent::StreamFirstDelta,
                    PerfComponent::StreamPrepare,
                    PerfComponent::StreamTokenCountRequest,
                    PerfComponent::StreamRequestSend,
                    PerfComponent::StreamProviderTtft,
                    PerfComponent::StreamFirstContentDelta,
                    PerfComponent::SseSerialize,
                    PerfComponent::SseBroadcast,
                    PerfComponent::SseLagged,
                    PerfComponent::TrajectorySaveMutexWait,
                    PerfComponent::TrajectoryMetricScan,
                ]),
                tool_stages: self.aggregate_components(&[
                    PerfComponent::ToolConfirmationWait,
                    PerfComponent::ToolCatalogBuild,
                    PerfComponent::ToolMutableVectorBuild,
                    PerfComponent::ToolPoolParallelExpansion,
                    PerfComponent::ToolSessionExtraction,
                    PerfComponent::ToolCatalogPoolAcquire,
                    PerfComponent::ToolAliasResolution,
                    PerfComponent::ToolConfirmationPreflight,
                    PerfComponent::ToolPolicyLookup,
                    PerfComponent::ToolExecutionWait,
                    PerfComponent::ToolExecutionLookup,
                    PerfComponent::ToolSemaphoreWait,
                    PerfComponent::ToolRuntime,
                    PerfComponent::ToolPreHook,
                    PerfComponent::ToolPostHook,
                    PerfComponent::ToolResultPostprocess,
                    PerfComponent::ToolResultMerge,
                    PerfComponent::ToolSessionMergeEvents,
                    PerfComponent::ToolCheckpointScheduling,
                    PerfComponent::ToolIntegrationToolsBuild,
                ]),
                index_watcher_vecdb_amplification: IndexWatcherVecdbAmplification {
                    trajectory_index_reads_per_listing_call: ratio_counts(
                        listing_counters.index_reads,
                        listing_calls,
                    ),
                    calls_by_caller: listing_counters.calls_by_caller,
                    watcher_rebuilds_per_commit: self.ratio(
                        &[PerfComponent::TrajectoryIndexRebuild],
                        PerfComponent::TrajectoryCommit,
                    ),
                    vecdb_searches_per_enrichment_attempt: self.ratio(
                        &[PerfComponent::EnrichmentScopedSearch],
                        PerfComponent::EnrichmentAttempt,
                    ),
                    aggregate: self.aggregate_components(&[
                        PerfComponent::TrajectoryIndexLockWait,
                        PerfComponent::TrajectoryIndexRead,
                        PerfComponent::TrajectoryIndexWrite,
                        PerfComponent::TrajectoryIndexRebuild,
                        PerfComponent::EnrichmentVecdbLockWait,
                        PerfComponent::EnrichmentVecdbLockHold,
                        PerfComponent::EnrichmentScopedSearch,
                    ]),
                },
                enrichment_stages: self.aggregate_components(&[
                    PerfComponent::EnrichmentAttempt,
                    PerfComponent::EnrichmentDecisionFirstUser,
                    PerfComponent::EnrichmentDecisionForced,
                    PerfComponent::EnrichmentDecisionSignaled,
                    PerfComponent::EnrichmentSkipNoUser,
                    PerfComponent::EnrichmentSkipAlreadyPresent,
                    PerfComponent::EnrichmentSkipEmptyQuery,
                    PerfComponent::EnrichmentSkipCommand,
                    PerfComponent::EnrichmentSkipThreshold,
                    PerfComponent::EnrichmentSessionSnapshot,
                    PerfComponent::EnrichmentExistingContextScan,
                    PerfComponent::EnrichmentQueryNormalize,
                    PerfComponent::EnrichmentRootDiscovery,
                    PerfComponent::EnrichmentCurrentRootResolve,
                    PerfComponent::EnrichmentVecdbLockWait,
                    PerfComponent::EnrichmentVecdbLockHold,
                    PerfComponent::EnrichmentEmbedding,
                    PerfComponent::EnrichmentScopedSearch,
                    PerfComponent::EnrichmentMergeDedup,
                    PerfComponent::EnrichmentFileReread,
                    PerfComponent::EnrichmentFallback,
                    PerfComponent::EnrichmentCardBuild,
                    PerfComponent::EnrichmentCacheMiss,
                    PerfComponent::EnrichmentCacheHit,
                    PerfComponent::EnrichmentCacheCoalesced,
                    PerfComponent::EnrichmentInsertion,
                    PerfComponent::EnrichmentInsertionStale,
                    PerfComponent::EnrichmentPersistenceScheduling,
                ]),
            },
        }
    }

    pub fn fixed_storage_words(&self) -> usize {
        self.components.len() * (12 + HISTOGRAM_BUCKETS)
    }

    fn aggregate_components(&self, components: &[PerfComponent]) -> PerformanceComponentAggregate {
        let mut aggregate = AggregateAccumulator::default();
        for component in components {
            aggregate.add(&self.components[component.index()]);
        }
        aggregate.finish()
    }

    fn ratio(&self, numerator: &[PerfComponent], denominator: PerfComponent) -> f64 {
        let numerator = numerator.iter().fold(0_u64, |total, component| {
            total.saturating_add(
                self.components[component.index()]
                    .sample_count
                    .load(Ordering::Relaxed),
            )
        });
        let denominator = self.components[denominator.index()]
            .sample_count
            .load(Ordering::Relaxed);
        if denominator == 0 {
            0.0
        } else {
            numerator as f64 / denominator as f64
        }
    }
}

fn component_label(index: u8) -> Option<&'static str> {
    PerfComponent::ALL
        .get(index as usize)
        .map(|component| component.as_str())
}

fn lock_read(lock: &RwLock<()>) -> std::sync::RwLockReadGuard<'_, ()> {
    match lock.read() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn lock_write(lock: &RwLock<()>) -> std::sync::RwLockWriteGuard<'_, ()> {
    match lock.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PerformanceTelemetrySnapshot {
    pub schema_version: u8,
    pub enabled: bool,
    pub collection_started_at_ms: u64,
    pub uptime_ms: u64,
    pub components: Vec<PerformanceComponentAggregate>,
    pub rollups: PerformanceTelemetryRollups,
}

#[derive(Clone, Debug, Serialize)]
pub struct PerformanceComponentAggregate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub component: Option<&'static str>,
    pub sample_count: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub skipped_count: u64,
    pub cancelled_count: u64,
    pub min_us: Option<u64>,
    pub max_us: Option<u64>,
    pub p50_us: Option<u64>,
    pub p95_us: Option<u64>,
    pub p99_us: Option<u64>,
    pub last_sample_at_ms: Option<u64>,
    pub size_bytes_sum: u64,
    pub item_count_sum: u64,
    pub batch_size_sum: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct PerformanceTelemetryRollups {
    pub advancement: PerformanceComponentAggregate,
    pub tool_stages: PerformanceComponentAggregate,
    pub index_watcher_vecdb_amplification: IndexWatcherVecdbAmplification,
    pub enrichment_stages: PerformanceComponentAggregate,
}

#[derive(Clone, Debug, Serialize)]
pub struct IndexWatcherVecdbAmplification {
    pub trajectory_index_reads_per_listing_call: f64,
    pub calls_by_caller: Vec<(&'static str, u64)>,
    pub watcher_rebuilds_per_commit: f64,
    pub vecdb_searches_per_enrichment_attempt: f64,
    pub aggregate: PerformanceComponentAggregate,
}

struct AggregateAccumulator {
    sample_count: u64,
    success_count: u64,
    failure_count: u64,
    skipped_count: u64,
    cancelled_count: u64,
    min_us: u64,
    max_us: u64,
    histogram: [u64; HISTOGRAM_BUCKETS],
    last_sample_at_ms: u64,
    size_bytes_sum: u64,
    item_count_sum: u64,
    batch_size_sum: u64,
}

impl Default for AggregateAccumulator {
    fn default() -> Self {
        Self {
            sample_count: 0,
            success_count: 0,
            failure_count: 0,
            skipped_count: 0,
            cancelled_count: 0,
            min_us: 0,
            max_us: 0,
            histogram: [0; HISTOGRAM_BUCKETS],
            last_sample_at_ms: 0,
            size_bytes_sum: 0,
            item_count_sum: 0,
            batch_size_sum: 0,
        }
    }
}

impl AggregateAccumulator {
    fn add(&mut self, component: &ComponentCounters) {
        let sample_count = component.sample_count.load(Ordering::Relaxed);
        self.sample_count = self.sample_count.saturating_add(sample_count);
        self.success_count = self
            .success_count
            .saturating_add(component.success_count.load(Ordering::Relaxed));
        self.failure_count = self
            .failure_count
            .saturating_add(component.failure_count.load(Ordering::Relaxed));
        self.skipped_count = self
            .skipped_count
            .saturating_add(component.skipped_count.load(Ordering::Relaxed));
        self.cancelled_count = self
            .cancelled_count
            .saturating_add(component.cancelled_count.load(Ordering::Relaxed));
        if sample_count > 0 {
            self.min_us = if self.min_us == 0 {
                component.min_us.load(Ordering::Relaxed)
            } else {
                self.min_us.min(component.min_us.load(Ordering::Relaxed))
            };
            self.max_us = self.max_us.max(component.max_us.load(Ordering::Relaxed));
            self.last_sample_at_ms = self
                .last_sample_at_ms
                .max(component.last_sample_at_ms.load(Ordering::Relaxed));
        }
        for (target, source) in self.histogram.iter_mut().zip(&component.histogram) {
            *target = target.saturating_add(source.load(Ordering::Relaxed));
        }
        self.size_bytes_sum = self
            .size_bytes_sum
            .saturating_add(component.size_bytes_sum.load(Ordering::Relaxed));
        self.item_count_sum = self
            .item_count_sum
            .saturating_add(component.item_count_sum.load(Ordering::Relaxed));
        self.batch_size_sum = self
            .batch_size_sum
            .saturating_add(component.batch_size_sum.load(Ordering::Relaxed));
    }

    fn finish(self) -> PerformanceComponentAggregate {
        PerformanceComponentAggregate {
            component: None,
            sample_count: self.sample_count,
            success_count: self.success_count,
            failure_count: self.failure_count,
            skipped_count: self.skipped_count,
            cancelled_count: self.cancelled_count,
            min_us: (self.sample_count > 0).then_some(self.min_us),
            max_us: (self.sample_count > 0).then_some(self.max_us),
            p50_us: percentile_bucket_us(&self.histogram, self.sample_count, 50),
            p95_us: percentile_bucket_us(&self.histogram, self.sample_count, 95),
            p99_us: percentile_bucket_us(&self.histogram, self.sample_count, 99),
            last_sample_at_ms: (self.sample_count > 0).then_some(self.last_sample_at_ms),
            size_bytes_sum: self.size_bytes_sum,
            item_count_sum: self.item_count_sum,
            batch_size_sum: self.batch_size_sum,
        }
    }
}

fn ratio_counts(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn histogram_bucket(elapsed_us: u64) -> usize {
    if elapsed_us == 0 {
        0
    } else {
        (u64::BITS - 1 - elapsed_us.leading_zeros()) as usize
    }
}

fn percentile_bucket_us(histogram: &[u64], sample_count: u64, percentile: u64) -> Option<u64> {
    if sample_count == 0 {
        return None;
    }
    let rank = sample_count.saturating_mul(percentile).saturating_add(99) / 100;
    let mut cumulative: u64 = 0;
    for (index, count) in histogram.iter().copied().enumerate() {
        cumulative = cumulative.saturating_add(count);
        if cumulative >= rank {
            return Some(bucket_upper_bound(index));
        }
    }
    Some(u64::MAX)
}

fn bucket_upper_bound(index: usize) -> u64 {
    if index == 0 {
        1
    } else if index >= HISTOGRAM_BUCKETS - 1 {
        u64::MAX
    } else {
        (1_u64 << (index + 1)) - 1
    }
}

fn update_min(target: &AtomicU64, value: u64) {
    let mut previous = target.load(Ordering::Relaxed);
    while value < previous {
        match target.compare_exchange_weak(previous, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(actual) => previous = actual,
        }
    }
}

fn update_max(target: &AtomicU64, value: u64) {
    let mut previous = target.load(Ordering::Relaxed);
    while value > previous {
        match target.compare_exchange_weak(previous, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return,
            Err(actual) => previous = actual,
        }
    }
}

fn saturating_add(target: &AtomicU64, value: u64) {
    let mut previous = target.load(Ordering::Relaxed);
    loop {
        match target.compare_exchange_weak(
            previous,
            previous.saturating_add(value),
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return,
            Err(actual) => previous = actual,
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::perf_diagnostics::PerfOutcome;

    fn event(
        component: PerfComponent,
        outcome: PerfOutcome,
        elapsed_us: u64,
        size_bytes: Option<u64>,
        item_count: Option<u64>,
        batch_size: Option<u64>,
    ) -> PerfEvent {
        PerfEvent {
            schema_version: 1,
            component: component.as_str(),
            component_index: component.index() as u8,
            outcome: outcome.as_str(),
            elapsed_us,
            size_bytes,
            item_count,
            trajectory_version: None,
            queue_depth: None,
            batch_size,
            execution_class: None,
            estimated_tokens: None,
            chat_id_hash: Some("hashed-chat-id".to_string()),
            path_hash: Some("hashed-path".to_string()),
        }
    }

    #[test]
    fn perf_telemetry_aggregates_injected_events_and_percentile_buckets() {
        let telemetry = PerformanceTelemetry::new(true);
        telemetry.record(&event(
            PerfComponent::ToolRuntime,
            PerfOutcome::Success,
            10,
            Some(5),
            Some(2),
            Some(1),
        ));
        telemetry.record(&event(
            PerfComponent::ToolRuntime,
            PerfOutcome::Failure,
            100,
            Some(7),
            Some(3),
            Some(2),
        ));
        telemetry.record(&event(
            PerfComponent::ToolRuntime,
            PerfOutcome::Skipped,
            1_000,
            None,
            None,
            None,
        ));
        telemetry.record(&event(
            PerfComponent::ToolRuntime,
            PerfOutcome::Cancelled,
            2_000,
            None,
            None,
            None,
        ));

        let snapshot = telemetry.snapshot();
        let aggregate = snapshot
            .components
            .iter()
            .find(|component| component.component == Some("tool.runtime"))
            .unwrap();
        assert_eq!(aggregate.sample_count, 4);
        assert_eq!(aggregate.success_count, 1);
        assert_eq!(aggregate.failure_count, 1);
        assert_eq!(aggregate.skipped_count, 1);
        assert_eq!(aggregate.cancelled_count, 1);
        assert_eq!(aggregate.min_us, Some(10));
        assert_eq!(aggregate.max_us, Some(2_000));
        assert_eq!(aggregate.p50_us, Some(127));
        assert_eq!(aggregate.p95_us, Some(2_047));
        assert_eq!(aggregate.p99_us, Some(2_047));
        assert_eq!(aggregate.size_bytes_sum, 12);
        assert_eq!(aggregate.item_count_sum, 5);
        assert_eq!(aggregate.batch_size_sum, 3);
        assert_eq!(snapshot.rollups.tool_stages.sample_count, 4);
        assert_eq!(snapshot.rollups.tool_stages.cancelled_count, 1);
        let rendered = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(rendered["components"][25]["cancelled_count"], 1);
        assert!(rendered["rollups"]["index_watcher_vecdb_amplification"]
            ["trajectory_index_reads_per_listing_call"]
            .is_number());
        assert!(
            rendered["rollups"]["index_watcher_vecdb_amplification"]["calls_by_caller"].is_array()
        );
    }

    #[test]
    fn listing_read_ratio_handles_zero_and_nonzero_call_counts() {
        assert_eq!(ratio_counts(4, 2), 2.0);
        assert_eq!(ratio_counts(4, 0), 0.0);
    }

    #[test]
    fn stream_stage_component_names_round_trip_through_telemetry() {
        let telemetry = PerformanceTelemetry::new(true);
        let expected = [
            (PerfComponent::StreamPrepare, "stream.prepare"),
            (
                PerfComponent::StreamTokenCountRequest,
                "stream.token_count_request",
            ),
            (PerfComponent::StreamRequestSend, "stream.request_send"),
            (PerfComponent::StreamProviderTtft, "stream.provider_ttft"),
            (
                PerfComponent::StreamFirstContentDelta,
                "stream.first_content_delta",
            ),
        ];

        for (component, name) in expected {
            assert_eq!(component.as_str(), name);
            assert!(telemetry.record(&event(component, PerfOutcome::Success, 1, None, None, None,)));
        }

        let snapshot = telemetry.snapshot();
        for (_, name) in expected {
            let aggregate = snapshot
                .components
                .iter()
                .find(|aggregate| aggregate.component == Some(name))
                .unwrap();
            assert_eq!(aggregate.sample_count, 1, "missing telemetry for {name}");
        }
    }

    #[test]
    fn perf_telemetry_disabled_enable_disable_and_reset_are_lossless() {
        let telemetry = PerformanceTelemetry::new(false);
        let event = event(
            PerfComponent::TrajectoryCommit,
            PerfOutcome::Success,
            8,
            None,
            None,
            None,
        );
        assert!(!telemetry.record(&event));
        assert_eq!(telemetry.snapshot().components[3].sample_count, 0);

        telemetry.set_enabled(true);
        assert!(telemetry.record(&event));
        telemetry.set_enabled(false);
        assert!(!telemetry.record(&event));
        assert_eq!(telemetry.snapshot().components[3].sample_count, 1);

        telemetry.reset();
        let snapshot = telemetry.snapshot();
        assert!(!snapshot.enabled);
        assert_eq!(snapshot.components[3].sample_count, 0);
        assert_eq!(snapshot.components[3].min_us, None);
        assert_eq!(snapshot.components[3].size_bytes_sum, 0);
    }

    #[test]
    fn perf_telemetry_storage_is_fixed_after_one_million_events() {
        let telemetry = PerformanceTelemetry::new(true);
        let storage_words = telemetry.fixed_storage_words();
        let event = event(
            PerfComponent::EnrichmentScopedSearch,
            PerfOutcome::Success,
            32,
            None,
            None,
            None,
        );
        for _ in 0..1_000_000 {
            telemetry.record(&event);
        }

        let snapshot = telemetry.snapshot();
        assert_eq!(telemetry.fixed_storage_words(), storage_words);
        assert_eq!(snapshot.components.len(), PerfComponent::ALL.len());
        assert_eq!(
            snapshot
                .components
                .iter()
                .find(|component| component.component == Some("enrichment.scoped_search"))
                .unwrap()
                .sample_count,
            1_000_000
        );
    }

    #[test]
    fn perf_telemetry_snapshot_contains_only_fixed_numeric_schema_fields() {
        let telemetry = PerformanceTelemetry::new(true);
        let private_chat_id = "private-chat-id";
        let private_path = "/private/project/secret.rs";
        telemetry.record(&PerfEvent {
            chat_id_hash: Some(private_chat_id.to_string()),
            path_hash: Some(private_path.to_string()),
            ..event(
                PerfComponent::EnrichmentScopedSearch,
                PerfOutcome::Success,
                5,
                Some(99),
                Some(1),
                None,
            )
        });

        let rendered = serde_json::to_string(&telemetry.snapshot()).unwrap();
        assert!(!rendered.contains(private_chat_id));
        assert!(!rendered.contains(private_path));
        for forbidden in ["chat_id", "path", "query", "prompt", "arguments", "content"] {
            assert!(!rendered.contains(&format!("\"{forbidden}\"")));
        }
    }

    #[test]
    fn perf_telemetry_ignores_unknown_component_indices() {
        let telemetry = PerformanceTelemetry::new(true);
        let mut event = event(
            PerfComponent::ToolRuntime,
            PerfOutcome::Success,
            5,
            None,
            None,
            None,
        );
        event.component_index = u8::MAX;

        assert!(!telemetry.record(&event));
        assert_eq!(telemetry.snapshot().rollups.tool_stages.sample_count, 0);
    }
}
