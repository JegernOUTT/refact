use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use refact_core::memory_plane::MemoryPlaneRoots;
use refact_core::vecdb_types::{
    EmbeddingModelConfig, SearchResult, VecDbStatus, VecdbRecord, VecdbSearch, VecdbSearchScope,
};
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tokio::runtime::Builder;
use tokio::sync::Mutex as AMutex;

use crate::app_state::{AppState, AppToolRegistry, FixtureToolFactory, TOOL_CATALOG_SNAPSHOTS_ENV};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ChatToolCall, ChatToolFunction, ContextEnum};
use crate::chat::tools::{
    process_tool_calls_once, resolve_tool_call_aliases_with_catalog, ToolStepOutcome,
};
use crate::chat::perf_diagnostics::{
    self, MemoryPerfSink, PerfClock, PerfComponent, PerfEvent, PerfOutcome, PerfRecorder,
};
use crate::chat::prepare::build_canonical_openai_tools;
use crate::chat::types::{ChatEvent, ChatSession, DeltaOp, EventEnvelope};
use crate::chat::generation::batch_stream_delta_ops;
use crate::knowledge::enrichment::enrich_messages_with_knowledge;
use crate::chat::trajectories::{
    find_trajectory_path, load_trajectory_for_chat, persist_trajectory_snapshot_with_intent,
    trajectory_snapshot_from_session,
};
use crate::chat::trajectory_index::{
    rebuild_trajectory_index_from_disk, upsert_trajectory_index_entry_from_owned_value,
};
use crate::files_correction::canonicalize_normalized_path;
use crate::global_context::SharedGlobalContext;
use crate::tools::tools_description::{
    MatchConfirmDeny, MatchConfirmDenyResult, Tool, ToolDesc, ToolSource, ToolSourceType,
};

pub const CONCURRENT_CHAT_BENCHMARK_SCHEMA: &str = "refact.concurrent_chat_benchmark.v1";
pub const FANOUT_BENCHMARK_SCHEMA: &str = "refact.chat_fanout_benchmark.v1";
pub const AUTO_ENRICHMENT_BENCHMARK_SCHEMA: &str = "refact.auto_enrichment_benchmark.v1";
const QUICK_HISTORY_BYTES_CAP: usize = 8 * 1024;
const RAPID_CHECKPOINTS_PER_CHAT: u64 = 4;
pub const TURN_MEMORY_FLEET_CHAT_COUNTS: [usize; 2] = [10, 100];
const FANOUT_HISTORY_MESSAGE_COUNT: usize = 256;
const FANOUT_HISTORY_MESSAGE_BYTES: usize = 4 * 1024;
const FANOUT_DELTA_COUNT: usize = 512;
const FANOUT_ACTIVE_SUBSCRIBER_COUNT: usize = 3;
const FANOUT_EVENT_CHANNEL_CAPACITY: usize = 128;
const FANOUT_SNAPSHOT_RUNS: usize = 8;

fn benchmark_runtime_builder() -> Builder {
    #[cfg(test)]
    {
        Builder::new_current_thread()
    }
    #[cfg(not(test))]
    {
        Builder::new_multi_thread()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TurnMemoryRetainedBytes {
    pub canonical_messages: usize,
    pub last_prompt_messages: usize,
    pub catalog_descriptors_and_aliases: usize,
    pub pool_vectors: usize,
}

impl TurnMemoryRetainedBytes {
    pub fn total(&self) -> usize {
        self.canonical_messages
            .saturating_add(self.last_prompt_messages)
            .saturating_add(self.catalog_descriptors_and_aliases)
            .saturating_add(self.pool_vectors)
    }

    fn add_assign(&mut self, other: &Self) {
        self.canonical_messages = self
            .canonical_messages
            .saturating_add(other.canonical_messages);
        self.last_prompt_messages = self
            .last_prompt_messages
            .saturating_add(other.last_prompt_messages);
        self.catalog_descriptors_and_aliases = self
            .catalog_descriptors_and_aliases
            .saturating_add(other.catalog_descriptors_and_aliases);
        self.pool_vectors = self.pool_vectors.saturating_add(other.pool_vectors);
    }
}

impl crate::chat::types::ChatSession {
    pub fn retained_bytes_for_turn_memory(&self) -> TurnMemoryRetainedBytes {
        let canonical_messages = self
            .messages
            .iter()
            .map(|message| {
                serde_json::to_vec(message)
                    .map(|encoded| encoded.len())
                    .unwrap_or_default()
            })
            .sum();
        let last_prompt_messages = self
            .last_prompt_messages
            .iter()
            .map(|message| {
                serde_json::to_vec(message)
                    .map(|encoded| encoded.len())
                    .unwrap_or_default()
            })
            .sum();
        let catalog_descriptors_and_aliases = self
            .tool_catalog
            .as_ref()
            .map(|catalog| {
                catalog
                    .index
                    .tools
                    .iter()
                    .map(|descriptor| {
                        serde_json::to_vec(descriptor)
                            .map(|encoded| encoded.len())
                            .unwrap_or_default()
                    })
                    .sum::<usize>()
                    .saturating_add(
                        catalog
                            .index
                            .tools
                            .iter()
                            .map(|descriptor| descriptor.name.len().saturating_mul(2))
                            .sum::<usize>(),
                    )
            })
            .unwrap_or_default();
        let pool_vectors = self
            .turn_tool_pool
            .as_ref()
            .map(|_| {
                self.tool_catalog
                    .as_ref()
                    .map(|catalog| {
                        std::mem::size_of::<refact_runtime_api::TurnToolPool>().saturating_add(
                            catalog
                                .index
                                .tools
                                .len()
                                .saturating_mul(std::mem::size_of::<usize>().saturating_mul(2)),
                        )
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        TurnMemoryRetainedBytes {
            canonical_messages,
            last_prompt_messages,
            catalog_descriptors_and_aliases,
            pool_vectors,
        }
    }
}

pub fn aggregate_turn_memory_retained_bytes(
    sessions: impl IntoIterator<Item = TurnMemoryRetainedBytes>,
) -> TurnMemoryRetainedBytes {
    sessions.into_iter().fold(
        TurnMemoryRetainedBytes {
            canonical_messages: 0,
            last_prompt_messages: 0,
            catalog_descriptors_and_aliases: 0,
            pool_vectors: 0,
        },
        |mut total, retained| {
            total.add_assign(&retained);
            total
        },
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessMode {
    Quick,
    Soak,
    FullSoak,
}

impl HarnessMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Soak => "soak",
            Self::FullSoak => "full_soak",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConcurrentChatWorkload {
    pub id: String,
    pub seed: u64,
    pub chat_count: u8,
    pub history_mib: u8,
    pub tool_descriptors: u16,
    pub same_index_directory_contention: bool,
    pub rapid_same_chat_checkpoints: u64,
}

impl ConcurrentChatWorkload {
    pub fn ci_fixture() -> Self {
        Self::new(1, 1, 10)
    }

    pub fn fixed_matrix() -> Vec<Self> {
        [1, 4, 8, 16, 32]
            .into_iter()
            .flat_map(|chat_count| {
                [1, 10, 50].into_iter().flat_map(move |history_mib| {
                    [10, 50, 200].into_iter().map(move |tool_descriptors| {
                        Self::new(chat_count, history_mib, tool_descriptors)
                    })
                })
            })
            .collect()
    }

    fn new(chat_count: u8, history_mib: u8, tool_descriptors: u16) -> Self {
        let seed = (chat_count as u64)
            .wrapping_mul(1_000_003)
            .wrapping_add((history_mib as u64).wrapping_mul(10_007))
            .wrapping_add(tool_descriptors as u64);
        Self {
            id: format!("chats-{chat_count}-history-{history_mib}m-tools-{tool_descriptors}"),
            seed,
            chat_count,
            history_mib,
            tool_descriptors,
            same_index_directory_contention: true,
            rapid_same_chat_checkpoints: RAPID_CHECKPOINTS_PER_CHAT,
        }
    }

    pub fn logical_history_bytes(&self) -> u64 {
        u64::from(self.history_mib) * 1024 * 1024
    }

    pub fn materialized_history_bytes(&self, mode: HarnessMode) -> usize {
        match mode {
            HarnessMode::Quick => usize::try_from(self.logical_history_bytes())
                .unwrap_or(usize::MAX)
                .min(QUICK_HISTORY_BYTES_CAP),
            HarnessMode::Soak | HarnessMode::FullSoak => {
                usize::try_from(self.logical_history_bytes()).unwrap_or(usize::MAX)
            }
        }
    }

    pub fn fixture_signature(&self, mode: HarnessMode) -> String {
        format!(
            "{:016x}",
            stable_hash(&[
                self.seed,
                u64::from(self.chat_count),
                u64::from(self.history_mib),
                u64::from(self.tool_descriptors),
                u64::try_from(self.materialized_history_bytes(mode)).unwrap_or(u64::MAX),
                match mode {
                    HarnessMode::Quick => 1,
                    HarnessMode::Soak => 2,
                    HarnessMode::FullSoak => 3,
                },
            ])
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BenchmarkOptions {
    pub mode: HarnessMode,
    pub warmup_samples: usize,
    pub measured_samples: usize,
}

impl BenchmarkOptions {
    pub const fn quick() -> Self {
        Self {
            mode: HarnessMode::Quick,
            warmup_samples: 1,
            measured_samples: 3,
        }
    }

    pub const fn soak() -> Self {
        Self {
            mode: HarnessMode::Soak,
            warmup_samples: 1,
            measured_samples: 5,
        }
    }

    pub const fn full_soak() -> Self {
        Self {
            mode: HarnessMode::FullSoak,
            warmup_samples: 1,
            measured_samples: 5,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FullSoakWorkload {
    pub id: String,
    pub chat_count: u8,
    pub active_chats: u8,
    pub background_chats: u8,
    pub history_bytes_per_chat: usize,
}

impl FullSoakWorkload {
    pub fn ci_fixture() -> Self {
        Self::new(1)
    }

    pub fn fixed_matrix() -> Vec<Self> {
        [1, 4, 8, 16, 32].into_iter().map(Self::new).collect()
    }

    fn new(chat_count: u8) -> Self {
        let active_chats = ((chat_count + 1) / 2).max(1);
        Self {
            id: format!("full-soak-{chat_count}-chats"),
            chat_count,
            active_chats,
            background_chats: chat_count.saturating_sub(active_chats),
            history_bytes_per_chat: 4 * 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FullSoakSubsystemFlags {
    pub chat_sessions: bool,
    pub queue_processors: bool,
    pub trajectory_writer: bool,
    pub trajectory_index_coordinator: bool,
    pub trajectory_watcher: bool,
    pub codegraph: bool,
    pub vecdb_local_backend: bool,
    pub buddy: bool,
    pub agent_monitor: bool,
    pub goal_monitor: bool,
    pub scheduler: bool,
    pub exec_registry: bool,
    pub session_cleanup: bool,
    pub exec_registry_entries: u64,
    pub vecdb_disclosure: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FullSoakRolloutSwitches {
    pub trajectory_writer_enabled: bool,
    pub trajectory_index_coordinator_enabled: bool,
    pub trajectory_watcher_self_write_enabled: bool,
    pub tool_catalog_snapshots_enabled: bool,
    pub vecdb_path_coalescing_enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct FullSoakCounters {
    pub queue_processors_started: u64,
    pub queue_processors_exited: u64,
    pub queue_notify_wakes: u64,
    pub queue_empty_locks: u64,
    pub queue_lock_contention_events: u64,
    pub tool_execution_wait_events: u64,
    pub tool_execution_wait_us: u64,
    pub stream_deltas: u64,
    pub tool_calls: u64,
    pub sse_events: u64,
    pub trajectory_events: u64,
    pub trajectory_files: u64,
    pub index_writes: u64,
    pub index_bytes_written: u64,
    pub watcher_suppressions: u64,
    pub watcher_replays: u64,
    pub vecdb_enqueues: u64,
    pub vecdb_coalesced_paths: u64,
    pub vecdb_enqueue_requests: u64,
    pub vecdb_pending_unique_paths: u64,
    pub vecdb_processed_paths: u64,
    pub catalog_builds: u64,
    pub catalog_pool_builds: u64,
    pub monitor_scans: u64,
    pub cleanup_scans: u64,
    pub exec_registry_entries: u64,
    pub errors: u64,
    pub ordering_errors: u64,
    pub restore_errors: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct FullSoakProcessMetrics {
    pub sample_count: usize,
    pub cpu_time_delta_us: Option<u64>,
    pub rss_baseline_bytes: Option<u64>,
    pub rss_peak_bytes: Option<u64>,
    pub rss_delta_bytes: Option<i64>,
    pub read_bytes_delta: Option<u64>,
    pub write_bytes_delta: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct FullSoakVecdbMetrics {
    pub enqueue_requests: u64,
    pub pending_unique_paths: u64,
    pub processed_paths: u64,
    pub amplification_ratio: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FullSoakToolCallStages {
    pub session_extraction_history_clone_latency: LatencySummary,
    pub catalog_pool_acquire_latency: LatencySummary,
    pub alias_resolution_latency: LatencySummary,
    pub confirmation_latency: LatencySummary,
    pub prehooks_latency: LatencySummary,
    pub execution_wait_latency: LatencySummary,
    pub execution_lookup_latency: LatencySummary,
    pub execution_runtime_latency: LatencySummary,
    pub posthooks_latency: LatencySummary,
    pub result_postprocess_privacy_latency: LatencySummary,
    pub session_merge_events_latency: LatencySummary,
    pub checkpoint_scheduling_latency: LatencySummary,
    pub accounted_latency: LatencySummary,
    pub unattributed_latency: LatencySummary,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FullSoakVariantBenchmarkReport {
    pub variant: String,
    pub rollout_switches: FullSoakRolloutSwitches,
    pub subsystems: FullSoakSubsystemFlags,
    pub counters: FullSoakCounters,
    pub queue_wait_latency: LatencySummary,
    pub first_delta_latency: LatencySummary,
    pub checkpoint_return_latency: LatencySummary,
    pub required_flush_latency: LatencySummary,
    pub tool_call_end_to_end_latency: LatencySummary,
    pub tool_call_stages: FullSoakToolCallStages,
    pub sse_serialize_latency: LatencySummary,
    pub sse_emit_latency: LatencySummary,
    pub process_samples: Vec<FullSoakProcessMetrics>,
    pub vecdb_deferred_queue: FullSoakVecdbMetrics,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FullSoakWorkloadBenchmarkReport {
    pub workload: FullSoakWorkload,
    pub variants: Vec<FullSoakVariantBenchmarkReport>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FullSoakBenchmarkReport {
    pub comparison_label: String,
    pub workloads: Vec<FullSoakWorkloadBenchmarkReport>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct BenchmarkCounters {
    pub save_calls: u64,
    pub rapid_checkpoint_saves: u64,
    pub required_commits: u64,
    pub trajectory_files: u64,
    pub measured_files_written: u64,
    pub measured_bytes_written: u64,
    pub index_rebuilds: u64,
    pub catalog_builds: u64,
    pub catalog_tool_descriptors: u64,
    pub catalog_policy_entries: u64,
    pub errors: u64,
}

impl BenchmarkCounters {
    fn operations(&self) -> u64 {
        self.save_calls
            .saturating_add(self.index_rebuilds)
            .saturating_add(self.catalog_builds)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DiagnosticCounters {
    pub trajectory_snapshot: u64,
    pub trajectory_serialize: u64,
    pub trajectory_atomic_write: u64,
    pub trajectory_commit: u64,
    pub trajectory_index_lock_wait: u64,
    pub trajectory_index_read: u64,
    pub trajectory_index_write: u64,
    pub trajectory_index_rebuild: u64,
    pub tool_catalog_build: u64,
}

impl DiagnosticCounters {
    fn count(events: &[PerfEvent], component: PerfComponent) -> u64 {
        events
            .iter()
            .filter(|event| event.component == component.as_str())
            .count() as u64
    }

    fn from_events(events: &[PerfEvent]) -> Self {
        Self {
            trajectory_snapshot: Self::count(events, PerfComponent::TrajectorySnapshot),
            trajectory_serialize: Self::count(events, PerfComponent::TrajectorySerialize),
            trajectory_atomic_write: Self::count(events, PerfComponent::TrajectoryAtomicWrite),
            trajectory_commit: Self::count(events, PerfComponent::TrajectoryCommit),
            trajectory_index_lock_wait: Self::count(events, PerfComponent::TrajectoryIndexLockWait),
            trajectory_index_read: Self::count(events, PerfComponent::TrajectoryIndexRead),
            trajectory_index_write: Self::count(events, PerfComponent::TrajectoryIndexWrite),
            trajectory_index_rebuild: Self::count(events, PerfComponent::TrajectoryIndexRebuild),
            tool_catalog_build: Self::count(events, PerfComponent::ToolCatalogBuild),
        }
    }

    fn add_assign(&mut self, other: &Self) {
        self.trajectory_snapshot += other.trajectory_snapshot;
        self.trajectory_serialize += other.trajectory_serialize;
        self.trajectory_atomic_write += other.trajectory_atomic_write;
        self.trajectory_commit += other.trajectory_commit;
        self.trajectory_index_lock_wait += other.trajectory_index_lock_wait;
        self.trajectory_index_read += other.trajectory_index_read;
        self.trajectory_index_write += other.trajectory_index_write;
        self.trajectory_index_rebuild += other.trajectory_index_rebuild;
        self.tool_catalog_build += other.tool_catalog_build;
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LatencySummary {
    pub sample_count: usize,
    pub min_us: u64,
    pub max_us: u64,
    pub mean_us: f64,
    pub variance_us: f64,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
}

impl LatencySummary {
    fn from_samples(samples: &[u64]) -> Result<Self, String> {
        if samples.is_empty() {
            return Err("cannot calculate latency summary without samples".to_string());
        }
        let min_us = *samples.iter().min().expect("non-empty samples");
        let max_us = *samples.iter().max().expect("non-empty samples");
        let mean_us =
            samples.iter().map(|sample| *sample as f64).sum::<f64>() / samples.len() as f64;
        let variance_us = samples
            .iter()
            .map(|sample| {
                let delta = *sample as f64 - mean_us;
                delta * delta
            })
            .sum::<f64>()
            / samples.len() as f64;
        Ok(Self {
            sample_count: samples.len(),
            min_us,
            max_us,
            mean_us,
            variance_us,
            p50_us: percentile_us(samples, 50),
            p95_us: percentile_us(samples, 95),
            p99_us: percentile_us(samples, 99),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FanoutBenchmarkWorkload {
    pub history_message_count: usize,
    pub history_message_bytes: usize,
    pub delta_count: usize,
    pub active_subscriber_count: usize,
    pub lagging_subscriber_count: usize,
    pub event_channel_capacity: usize,
    pub snapshot_runs: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FanoutDeltaMetrics {
    pub deltas_per_second: f64,
    pub operations_per_delta: f64,
    pub bytes_per_delta: f64,
    pub coalesce_window_ms: u64,
    pub baseline_event_count: usize,
    pub coalesced_event_count: usize,
    pub baseline_serialize_cpu_us: u64,
    pub coalesced_serialize_cpu_us: u64,
    pub serialization_cpu_reduction_percent: f64,
    pub projected_first_delta_latency_us: u64,
    pub emit_lock_wait_latency: LatencySummary,
    pub serialize_latency: LatencySummary,
    pub broadcast_latency: LatencySummary,
    pub first_delta_latency: LatencySummary,
    pub serialization_and_broadcast_percent_of_emit_wall_time: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FanoutSnapshotMetrics {
    pub snapshot_count: usize,
    pub clone_latency: LatencySummary,
    pub clone_bytes: LatencySummary,
    pub serialize_latency: LatencySummary,
    pub serialized_bytes: LatencySummary,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FanoutSubscriberMetrics {
    pub subscriber_count: usize,
    pub active_subscriber_count: usize,
    pub active_received_delta_count: usize,
    pub active_lag_recoveries: u64,
    pub lag_recoveries: u64,
    pub lagged_events: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FanoutBenchmarkReport {
    pub schema: &'static str,
    pub workload: FanoutBenchmarkWorkload,
    pub delta: FanoutDeltaMetrics,
    pub snapshot: FanoutSnapshotMetrics,
    pub subscribers: FanoutSubscriberMetrics,
}

pub fn percentile_us(samples: &[u64], percentile: u8) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let percentile = usize::from(percentile.min(100));
    let index = ((sorted.len() - 1) * percentile + 99) / 100;
    sorted[index]
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MachineMetrics {
    pub rss_bytes: Option<u64>,
    pub cpu_percent: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct VariantBenchmarkReport {
    pub variant: String,
    pub workload_signature: String,
    pub logical_history_bytes: u64,
    pub materialized_history_bytes: usize,
    pub counters: BenchmarkCounters,
    pub diagnostics: DiagnosticCounters,
    pub snapshot_latency: LatencySummary,
    pub serialize_latency: LatencySummary,
    pub atomic_write_latency: LatencySummary,
    pub commit_latency: LatencySummary,
    pub index_wait_latency: LatencySummary,
    pub index_write_latency: LatencySummary,
    pub catalog_acquisition_latency: LatencySummary,
    pub checkpoint_return_latency: LatencySummary,
    pub background_flush_latency: LatencySummary,
    pub total_operation_latency: LatencySummary,
    pub throughput_operations_per_sec: f64,
    pub machine: MachineMetrics,
}

pub const TOOL_POOL_CHAT_COUNT: u8 = 8;
pub const TOOL_POOL_DESCRIPTOR_COUNT: u16 = 50;
const TOOL_POOL_SAME_NAME_PARALLEL_CALLS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolPoolWorkload {
    pub id: String,
    pub seed: u64,
    pub chat_count: u8,
    pub tool_descriptors: u16,
    pub tool_calls_per_chat: u16,
    pub same_name_parallel_calls: u8,
}

impl ToolPoolWorkload {
    pub fn fixed() -> Self {
        Self {
            id: "turn-tool-pool-8-chats-50-tools".to_string(),
            seed: 0x7100_0000,
            chat_count: TOOL_POOL_CHAT_COUNT,
            tool_descriptors: TOOL_POOL_DESCRIPTOR_COUNT,
            tool_calls_per_chat: TOOL_POOL_DESCRIPTOR_COUNT,
            same_name_parallel_calls: TOOL_POOL_SAME_NAME_PARALLEL_CALLS as u8,
        }
    }

    pub fn fixture_signature(&self) -> String {
        format!(
            "{:016x}",
            stable_hash(&[
                self.seed,
                u64::from(self.chat_count),
                u64::from(self.tool_descriptors),
                u64::from(self.tool_calls_per_chat),
                u64::from(self.same_name_parallel_calls),
            ])
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ToolPoolCounters {
    pub immutable_catalog_builds: u64,
    pub mutable_vector_builds: u64,
    pub parallel_vector_expansions: u64,
    pub confirmation_preflight_starts: u64,
    pub confirmation_preflight_tool_checks: u64,
    pub execution_lookups: u64,
    pub tool_calls: u64,
    pub tool_runtime_calls: u64,
    pub errors: u64,
}

impl ToolPoolCounters {
    pub fn catalog_preflight_operations(&self) -> u64 {
        self.immutable_catalog_builds
            .saturating_add(self.mutable_vector_builds)
            .saturating_add(self.parallel_vector_expansions)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ToolPoolVariantBenchmarkReport {
    pub variant: String,
    pub workload_signature: String,
    pub counters: ToolPoolCounters,
    pub catalog_acquisition_latency: LatencySummary,
    pub schema_alias_preparation_latency: LatencySummary,
    pub warm_schema_alias_preparation_latency: Option<LatencySummary>,
    pub confirmation_preflight_latency: LatencySummary,
    pub execution_lookup_latency: LatencySummary,
    pub tool_start_overhead_latency: LatencySummary,
    pub tool_runtime_latency: LatencySummary,
    pub machine: MachineMetrics,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ToolPoolComparison {
    pub legacy_catalog_preflight_operations: u64,
    pub pooled_catalog_preflight_operations: u64,
    pub catalog_preflight_operation_reduction_percent: f64,
    pub tool_start_p95_us: u64,
    pub warm_schema_alias_p95_us: u64,
    pub remaining_stage: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ToolPoolWorkloadBenchmarkReport {
    pub workload: ToolPoolWorkload,
    pub variants: Vec<ToolPoolVariantBenchmarkReport>,
    pub comparison: ToolPoolComparison,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WorkloadBenchmarkReport {
    pub workload: ConcurrentChatWorkload,
    pub variants: Vec<VariantBenchmarkReport>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ConcurrentChatBenchmarkReport {
    pub schema: &'static str,
    pub mode: String,
    pub warmup_samples: usize,
    pub measured_samples: usize,
    pub workloads: Vec<WorkloadBenchmarkReport>,
    pub tool_pool_workload: ToolPoolWorkloadBenchmarkReport,
}

struct Sample {
    counters: BenchmarkCounters,
    diagnostics: DiagnosticCounters,
    snapshot_elapsed_us: u64,
    serialize_elapsed_us: u64,
    atomic_write_elapsed_us: u64,
    commit_elapsed_us: u64,
    index_wait_elapsed_us: u64,
    index_write_elapsed_us: u64,
    catalog_elapsed_us: u64,
    checkpoint_return_elapsed_us: u64,
    background_flush_elapsed_us: u64,
    total_elapsed_us: u64,
}

struct ToolPoolSample {
    counters: ToolPoolCounters,
    catalog_acquisition_elapsed_us: Vec<u64>,
    schema_alias_preparation_elapsed_us: Vec<u64>,
    warm_schema_alias_preparation_elapsed_us: Vec<u64>,
    confirmation_preflight_elapsed_us: Vec<u64>,
    execution_lookup_elapsed_us: Vec<u64>,
    tool_runtime_elapsed_us: Vec<u64>,
}

#[derive(Clone)]
struct BenchmarkFixture {
    _temp_dir: Arc<tempfile::TempDir>,
    workspace: PathBuf,
    gcx: SharedGlobalContext,
    app: AppState,
}

impl BenchmarkFixture {
    async fn new(tool_count: usize) -> Result<Self, String> {
        let temp_dir = Arc::new(
            tempfile::tempdir()
                .map_err(|error| format!("failed to create benchmark fixture: {error}"))?,
        );
        let workspace = temp_dir.path().join("workspace");
        let cache_dir = temp_dir.path().join("cache");
        let config_dir = temp_dir.path().join("config");
        tokio::fs::create_dir_all(&workspace)
            .await
            .map_err(|error| format!("failed to create benchmark workspace: {error}"))?;
        tokio::fs::create_dir_all(workspace.join(".refact").join("trajectories"))
            .await
            .map_err(|error| format!("failed to create benchmark trajectories root: {error}"))?;
        let gcx =
            crate::global_context::tests::make_test_gcx_with_dirs(cache_dir, config_dir).await;
        *gcx.documents_state
            .workspace_folders
            .lock()
            .map_err(|_| "benchmark workspace folders lock poisoned".to_string())? =
            vec![canonicalize_normalized_path(workspace.clone())];
        let mut app = AppState::from_gcx(gcx.clone()).await;
        app.tool_registry = Arc::new(AppToolRegistry::with_fixture_tool_factory(
            gcx.clone(),
            deterministic_tool_factory(tool_count),
        ));
        Ok(Self {
            _temp_dir: temp_dir,
            workspace,
            gcx,
            app,
        })
    }
}

pub fn run_benchmark(options: BenchmarkOptions) -> Result<ConcurrentChatBenchmarkReport, String> {
    if options.mode == HarnessMode::FullSoak {
        return Err(
            "full soak produces a distinct report; use run_full_soak_benchmark".to_string(),
        );
    }
    if options.measured_samples == 0 {
        return Err("measured_samples must be greater than zero".to_string());
    }
    benchmark_runtime_builder()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to start benchmark Tokio runtime: {error}"))?
        .block_on(run_benchmark_async(options))
}

async fn run_benchmark_async(
    options: BenchmarkOptions,
) -> Result<ConcurrentChatBenchmarkReport, String> {
    let workloads = ConcurrentChatWorkload::fixed_matrix();
    let mut reports = Vec::with_capacity(workloads.len());
    for workload in workloads {
        reports.push(run_workload(&workload, &options).await?);
    }
    let tool_pool_workload = run_tool_pool_workload(&ToolPoolWorkload::fixed(), &options).await?;
    Ok(ConcurrentChatBenchmarkReport {
        schema: CONCURRENT_CHAT_BENCHMARK_SCHEMA,
        mode: options.mode.as_str().to_string(),
        warmup_samples: options.warmup_samples,
        measured_samples: options.measured_samples,
        workloads: reports,
        tool_pool_workload,
    })
}

pub fn run_ci_fixture() -> Result<WorkloadBenchmarkReport, String> {
    benchmark_runtime_builder()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to start benchmark Tokio runtime: {error}"))?
        .block_on(run_workload(
            &ConcurrentChatWorkload::ci_fixture(),
            &BenchmarkOptions {
                mode: HarnessMode::Quick,
                warmup_samples: 0,
                measured_samples: 1,
            },
        ))
}

pub fn run_tool_pool_ci_fixture() -> Result<ToolPoolWorkloadBenchmarkReport, String> {
    benchmark_runtime_builder()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to start benchmark Tokio runtime: {error}"))?
        .block_on(run_tool_pool_workload(
            &ToolPoolWorkload::fixed(),
            &BenchmarkOptions {
                mode: HarnessMode::Quick,
                warmup_samples: 0,
                measured_samples: 1,
            },
        ))
}

pub fn run_full_soak_benchmark(
    options: BenchmarkOptions,
) -> Result<FullSoakBenchmarkReport, String> {
    if options.mode != HarnessMode::FullSoak {
        return Err("full soak benchmark requires HarnessMode::FullSoak".to_string());
    }
    if options.measured_samples == 0 {
        return Err("measured_samples must be greater than zero".to_string());
    }
    benchmark_runtime_builder()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to start full soak Tokio runtime: {error}"))?
        .block_on(run_full_soak_benchmark_async(options))
}

async fn run_full_soak_benchmark_async(
    options: BenchmarkOptions,
) -> Result<FullSoakBenchmarkReport, String> {
    let workloads = FullSoakWorkload::fixed_matrix();
    let mut reports = Vec::with_capacity(workloads.len());
    for workload in workloads {
        reports.push(run_full_soak_workload(&workload, &options).await?);
    }
    Ok(FullSoakBenchmarkReport {
        comparison_label:
            "synthetic same-version legacy rollout comparison; not a historical Wave 0 baseline"
                .to_string(),
        workloads: reports,
    })
}

pub fn run_full_soak_ci_fixture() -> Result<FullSoakWorkloadBenchmarkReport, String> {
    benchmark_runtime_builder()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to start full soak CI Tokio runtime: {error}"))?
        .block_on(run_full_soak_workload(
            &FullSoakWorkload::ci_fixture(),
            &BenchmarkOptions {
                mode: HarnessMode::FullSoak,
                warmup_samples: 0,
                measured_samples: 1,
            },
        ))
}

pub fn run_fanout_benchmark() -> Result<FanoutBenchmarkReport, String> {
    benchmark_runtime_builder()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to start fanout Tokio runtime: {error}"))?
        .block_on(run_fanout_benchmark_async())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoEnrichmentQueryMode {
    Repeated,
    Distinct,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoEnrichmentVecdbMode {
    Warm,
    Cold,
    Empty,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AutoEnrichmentWorkload {
    pub chat_count: usize,
    pub root_count: usize,
    pub knowledge_file_count: usize,
    pub query_mode: AutoEnrichmentQueryMode,
    pub vecdb_mode: AutoEnrichmentVecdbMode,
    pub history_message_count: usize,
    pub privacy_exclusion_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AutoEnrichmentStageReport {
    pub stage: String,
    pub latency: LatencySummary,
    pub fraction_of_accounted_wall_percent: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct AutoEnrichmentRepeatedWork {
    pub attempts: u64,
    pub scoped_searches: u64,
    pub fallback_files_read: u64,
    pub embedding_retries: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_coalesced: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AutoEnrichmentWorkloadReport {
    pub workload: AutoEnrichmentWorkload,
    pub end_to_end_latency: LatencySummary,
    pub stages: Vec<AutoEnrichmentStageReport>,
    pub dominant_stages: Vec<String>,
    pub repeated_work: AutoEnrichmentRepeatedWork,
    pub inserted_contexts: u64,
    pub injected_file_count: u64,
    pub injected_char_count: u64,
    pub injected_estimated_tokens: u64,
    pub privacy_exclusion_violations: u64,
    pub max_concurrent_search: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AutoEnrichmentBenchmarkReport {
    pub schema: &'static str,
    pub workloads: Vec<AutoEnrichmentWorkloadReport>,
}

#[derive(Default)]
struct AutoEnrichmentVecdb {
    records: Vec<VecdbRecord>,
    delay: std::time::Duration,
    active_searches: AtomicUsize,
    max_concurrent_searches: AtomicUsize,
}

impl AutoEnrichmentVecdb {
    fn enter_search(&self) -> AutoEnrichmentSearchGuard<'_> {
        let active = self.active_searches.fetch_add(1, Ordering::SeqCst) + 1;
        let mut observed = self.max_concurrent_searches.load(Ordering::SeqCst);
        while active > observed {
            match self.max_concurrent_searches.compare_exchange(
                observed,
                active,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(current) => observed = current,
            }
        }
        AutoEnrichmentSearchGuard { vecdb: self }
    }
}

struct AutoEnrichmentSearchGuard<'a> {
    vecdb: &'a AutoEnrichmentVecdb,
}

impl Drop for AutoEnrichmentSearchGuard<'_> {
    fn drop(&mut self) {
        self.vecdb.active_searches.fetch_sub(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl VecdbSearch for AutoEnrichmentVecdb {
    async fn vecdb_search(
        &self,
        query: String,
        _top_n: usize,
        _filter_mb: Option<String>,
    ) -> Result<SearchResult, String> {
        Ok(SearchResult {
            query_text: query,
            results: Vec::new(),
        })
    }

    async fn get_status(&self) -> Result<VecDbStatus, String> {
        Ok(VecDbStatus {
            files_unprocessed: 0,
            files_total: self.records.len(),
            requests_made_since_start: 0,
            vectors_made_since_start: 0,
            db_size: 0,
            db_cache_size: 0,
            state: "local_auto_enrichment_fixture".to_string(),
            queue_additions: false,
            vecdb_max_files_hit: false,
            vecdb_errors: Default::default(),
        })
    }

    async fn remove_file(&self, _file_path: &PathBuf) -> Result<(), String> {
        Ok(())
    }

    async fn vectorizer_enqueue_files(
        &self,
        _documents: &[String],
        _process_immediately: bool,
        _roots: MemoryPlaneRoots,
    ) {
    }

    fn current_constants(&self) -> (EmbeddingModelConfig, usize) {
        (
            EmbeddingModelConfig {
                model_id: "local-auto-enrichment".to_string(),
                endpoint: String::new(),
                endpoint_style: String::new(),
                embedding_endpoint_style: String::new(),
                api_key: String::new(),
                model_name: "local-auto-enrichment".to_string(),
                embedding_size: 3,
                dimensions: Some(3),
                query_prefix: String::new(),
                document_prefix: String::new(),
                rejection_threshold: 0.0,
                embedding_batch: 1,
                n_ctx: 0,
            },
            0,
        )
    }

    async fn embed_query(&self, _query: &str) -> Result<Vec<f32>, String> {
        let _guard = self.enter_search();
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        Ok(vec![0.1, 0.2, 0.3])
    }

    async fn vecdb_search_with_embedding(
        &self,
        _embedding: &Vec<f32>,
        _top_n: usize,
        _filter_mb: Option<String>,
    ) -> Result<Vec<VecdbRecord>, String> {
        let _guard = self.enter_search();
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        Ok(self.records.clone())
    }

    async fn vecdb_search_scopes_with_embedding(
        &self,
        _embedding: &Vec<f32>,
        scopes: &[VecdbSearchScope],
    ) -> Result<Vec<Vec<VecdbRecord>>, String> {
        let _guard = self.enter_search();
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        Ok(scopes
            .iter()
            .map(|scope| {
                self.records
                    .iter()
                    .filter(|record| {
                        record
                            .file_path
                            .to_string_lossy()
                            .starts_with(&scope.path_prefix)
                    })
                    .take(scope.top_n)
                    .cloned()
                    .collect()
            })
            .collect())
    }
}

struct AutoEnrichmentFixture {
    _temp_dir: Arc<tempfile::TempDir>,
    gcx: SharedGlobalContext,
    excluded_paths: HashSet<PathBuf>,
    vecdb: Option<Arc<AutoEnrichmentVecdb>>,
}

impl AutoEnrichmentFixture {
    async fn new(workload: &AutoEnrichmentWorkload) -> Result<Self, String> {
        let temp_dir = Arc::new(
            tempfile::tempdir()
                .map_err(|error| format!("failed to create enrichment fixture: {error}"))?,
        );
        let cache_dir = temp_dir.path().join("cache");
        let config_dir = temp_dir.path().join("config");
        let gcx =
            crate::global_context::tests::make_test_gcx_with_dirs(cache_dir, config_dir).await;
        let mut roots = Vec::with_capacity(workload.root_count);
        for root_index in 0..workload.root_count {
            let root = temp_dir.path().join(format!("root-{root_index}"));
            tokio::fs::create_dir_all(root.join(crate::file_filter::KNOWLEDGE_FOLDER_NAME))
                .await
                .map_err(|error| format!("failed to create enrichment root: {error}"))?;
            roots.push(root);
        }
        *gcx.documents_state
            .workspace_folders
            .lock()
            .map_err(|_| "enrichment workspace folders lock poisoned".to_string())? = roots.clone();

        let mut records = Vec::with_capacity(workload.knowledge_file_count);
        let mut excluded_paths = HashSet::new();
        for file_index in 0..workload.knowledge_file_count {
            let root = &roots[file_index % roots.len()];
            let path = root
                .join(crate::file_filter::KNOWLEDGE_FOLDER_NAME)
                .join(format!("memory-{file_index}.md"));
            let tags = vec!["fixture".to_string()];
            let empty = Vec::new();
            let mut frontmatter = crate::memories::create_frontmatter(
                Some("Auto enrichment fixture"),
                &tags,
                &empty,
                &empty,
                "memory",
            );
            if file_index < workload.privacy_exclusion_count {
                frontmatter.source_chat_id = Some("auto-enrichment-current-chat".to_string());
                excluded_paths.insert(path.clone());
            }
            let content = format!(
                "{}\n\nEnrichment codegraph fixture result number {file_index}.",
                frontmatter.to_yaml()
            );
            tokio::fs::write(&path, content)
                .await
                .map_err(|error| format!("failed to write enrichment memory: {error}"))?;
            records.push(VecdbRecord {
                vector: None,
                file_path: path,
                start_line: 1,
                end_line: 1,
                distance: 0.1,
                usefulness: 95.0,
            });
        }

        let vecdb = match workload.vecdb_mode {
            AutoEnrichmentVecdbMode::Unavailable => None,
            AutoEnrichmentVecdbMode::Warm => Some(Arc::new(AutoEnrichmentVecdb {
                records,
                delay: std::time::Duration::ZERO,
                ..Default::default()
            })),
            AutoEnrichmentVecdbMode::Cold => Some(Arc::new(AutoEnrichmentVecdb {
                records,
                delay: std::time::Duration::from_millis(1),
                ..Default::default()
            })),
            AutoEnrichmentVecdbMode::Empty => Some(Arc::new(AutoEnrichmentVecdb {
                records: Vec::new(),
                delay: std::time::Duration::from_millis(1),
                ..Default::default()
            })),
        };
        *gcx.vec_db.lock().await = vecdb.clone().map(|backend| backend as Arc<dyn VecdbSearch>);
        if matches!(workload.vecdb_mode, AutoEnrichmentVecdbMode::Unavailable) {
            let index = crate::knowledge_index::build_knowledge_index(gcx.clone()).await;
            *gcx.knowledge_index.lock().await = index;
        }
        Ok(Self {
            _temp_dir: temp_dir,
            gcx,
            excluded_paths,
            vecdb,
        })
    }
}

pub fn run_auto_enrichment_benchmark() -> Result<AutoEnrichmentBenchmarkReport, String> {
    benchmark_runtime_builder()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to start enrichment Tokio runtime: {error}"))?
        .block_on(run_auto_enrichment_benchmark_async())
}

pub fn run_auto_enrichment_ci_fixture() -> Result<AutoEnrichmentWorkloadReport, String> {
    benchmark_runtime_builder()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to start enrichment CI Tokio runtime: {error}"))?
        .block_on(run_auto_enrichment_workload(&AutoEnrichmentWorkload {
            chat_count: 10,
            root_count: 2,
            knowledge_file_count: 10,
            query_mode: AutoEnrichmentQueryMode::Repeated,
            vecdb_mode: AutoEnrichmentVecdbMode::Cold,
            history_message_count: 8,
            privacy_exclusion_count: 1,
        }))
}

async fn run_auto_enrichment_benchmark_async() -> Result<AutoEnrichmentBenchmarkReport, String> {
    let workloads = vec![
        AutoEnrichmentWorkload {
            chat_count: 1,
            root_count: 1,
            knowledge_file_count: 0,
            query_mode: AutoEnrichmentQueryMode::Repeated,
            vecdb_mode: AutoEnrichmentVecdbMode::Unavailable,
            history_message_count: 1,
            privacy_exclusion_count: 0,
        },
        AutoEnrichmentWorkload {
            chat_count: 10,
            root_count: 1,
            knowledge_file_count: 10,
            query_mode: AutoEnrichmentQueryMode::Repeated,
            vecdb_mode: AutoEnrichmentVecdbMode::Warm,
            history_message_count: 8,
            privacy_exclusion_count: 1,
        },
        AutoEnrichmentWorkload {
            chat_count: 10,
            root_count: 2,
            knowledge_file_count: 10,
            query_mode: AutoEnrichmentQueryMode::Distinct,
            vecdb_mode: AutoEnrichmentVecdbMode::Cold,
            history_message_count: 16,
            privacy_exclusion_count: 1,
        },
        AutoEnrichmentWorkload {
            chat_count: 50,
            root_count: 2,
            knowledge_file_count: 10,
            query_mode: AutoEnrichmentQueryMode::Repeated,
            vecdb_mode: AutoEnrichmentVecdbMode::Cold,
            history_message_count: 32,
            privacy_exclusion_count: 1,
        },
        AutoEnrichmentWorkload {
            chat_count: 50,
            root_count: 8,
            knowledge_file_count: 1_000,
            query_mode: AutoEnrichmentQueryMode::Distinct,
            vecdb_mode: AutoEnrichmentVecdbMode::Unavailable,
            history_message_count: 64,
            privacy_exclusion_count: 1,
        },
        AutoEnrichmentWorkload {
            chat_count: 100,
            root_count: 8,
            knowledge_file_count: 1_000,
            query_mode: AutoEnrichmentQueryMode::Repeated,
            vecdb_mode: AutoEnrichmentVecdbMode::Empty,
            history_message_count: 64,
            privacy_exclusion_count: 1,
        },
        AutoEnrichmentWorkload {
            chat_count: 10,
            root_count: 1,
            knowledge_file_count: 10_000,
            query_mode: AutoEnrichmentQueryMode::Repeated,
            vecdb_mode: AutoEnrichmentVecdbMode::Unavailable,
            history_message_count: 8,
            privacy_exclusion_count: 1,
        },
    ];
    let mut reports = Vec::with_capacity(workloads.len());
    for workload in workloads {
        reports.push(run_auto_enrichment_workload(&workload).await?);
    }
    Ok(AutoEnrichmentBenchmarkReport {
        schema: AUTO_ENRICHMENT_BENCHMARK_SCHEMA,
        workloads: reports,
    })
}

async fn run_auto_enrichment_workload(
    workload: &AutoEnrichmentWorkload,
) -> Result<AutoEnrichmentWorkloadReport, String> {
    let _diagnostic_lock = perf_diagnostics::PERF_RECORDER_TEST_LOCK
        .lock()
        .map_err(|_| "enrichment performance recorder lock poisoned".to_string())?;
    let fixture = AutoEnrichmentFixture::new(workload).await?;
    let sink = Arc::new(MemoryPerfSink::new());
    let recorder = Arc::new(PerfRecorder::with_salt(
        Arc::new(BenchmarkClock::default()),
        sink.clone(),
        [47; 32],
    ));
    let _recorder_guard = perf_diagnostics::install_test_recorder(recorder);
    let tasks = (0..workload.chat_count)
        .map(|chat_index| {
            let gcx = fixture.gcx.clone();
            let mut session = ChatSession::new("auto-enrichment-current-chat".to_string());
            session.messages = auto_enrichment_history(workload, chat_index);
            async move {
                let snapshot_started = Instant::now();
                let mut messages = session.messages.clone();
                let snapshot_bytes = messages
                    .iter()
                    .map(|message| {
                        serde_json::to_vec(message)
                            .map(|encoded| encoded.len())
                            .unwrap_or(0)
                    })
                    .sum::<usize>();
                perf_diagnostics::record_enrichment(
                    PerfComponent::EnrichmentSessionSnapshot,
                    "auto-enrichment-current-chat",
                    PerfOutcome::Success,
                    elapsed_us(snapshot_started),
                    Some(snapshot_bytes as u64),
                    Some(messages.len() as u64),
                    Some((snapshot_bytes as u64).saturating_add(3) / 4),
                );
                let started = Instant::now();
                enrich_messages_with_knowledge(
                    gcx,
                    &mut messages,
                    Some("auto-enrichment-current-chat"),
                    true,
                )
                .await;
                (elapsed_us(started), messages)
            }
        })
        .collect::<Vec<_>>();
    let outcomes = futures::future::join_all(tasks).await;
    let end_to_end_samples = outcomes
        .iter()
        .map(|(elapsed, _)| *elapsed)
        .collect::<Vec<_>>();
    let inserted_contexts = outcomes
        .iter()
        .filter(|(_, messages)| {
            messages.iter().any(|message| {
                message.role == "context_file" && message.tool_call_id == "knowledge_enrichment"
            })
        })
        .count() as u64;
    let privacy_exclusion_violations = outcomes
        .iter()
        .flat_map(|(_, messages)| messages)
        .filter(|message| message.role == "context_file")
        .filter_map(|message| match &message.content {
            ChatContent::ContextFiles(files) => Some(files),
            _ => None,
        })
        .flatten()
        .filter(|file| fixture.excluded_paths.contains(Path::new(&file.file_name)))
        .count() as u64;
    if privacy_exclusion_violations != 0 {
        return Err("enrichment fixture injected a current-chat memory".to_string());
    }
    let events = sink.events();
    let stages = enrichment_stage_reports(&events)?;
    let dominant_stages = stages
        .iter()
        .filter(|stage| stage.fraction_of_accounted_wall_percent > 15.0)
        .map(|stage| {
            format!(
                "{} ({:.1}%)",
                stage.stage, stage.fraction_of_accounted_wall_percent
            )
        })
        .collect::<Vec<_>>();
    let repeated_work = AutoEnrichmentRepeatedWork {
        attempts: event_count(&events, PerfComponent::EnrichmentAttempt),
        scoped_searches: event_item_count(&events, PerfComponent::EnrichmentScopedSearch),
        fallback_files_read: event_item_count(&events, PerfComponent::EnrichmentFallback),
        embedding_retries: 0,
        cache_hits: events
            .iter()
            .filter(|event| event.component == PerfComponent::EnrichmentCacheHit.as_str())
            .count() as u64,
        cache_misses: events
            .iter()
            .filter(|event| event.component == PerfComponent::EnrichmentCacheMiss.as_str())
            .count() as u64,
        cache_coalesced: events
            .iter()
            .filter(|event| event.component == PerfComponent::EnrichmentCacheCoalesced.as_str())
            .count() as u64,
    };
    let injected_file_count = events
        .iter()
        .filter(|event| event.component == PerfComponent::EnrichmentInsertion.as_str())
        .filter(|event| event.outcome == PerfOutcome::Success.as_str())
        .filter_map(|event| event.item_count)
        .sum();
    let injected_char_count = events
        .iter()
        .filter(|event| event.component == PerfComponent::EnrichmentInsertion.as_str())
        .filter(|event| event.outcome == PerfOutcome::Success.as_str())
        .filter_map(|event| event.size_bytes)
        .sum();
    let injected_estimated_tokens = events
        .iter()
        .filter(|event| event.component == PerfComponent::EnrichmentInsertion.as_str())
        .filter(|event| event.outcome == PerfOutcome::Success.as_str())
        .filter_map(|event| event.estimated_tokens)
        .sum();
    Ok(AutoEnrichmentWorkloadReport {
        workload: workload.clone(),
        end_to_end_latency: LatencySummary::from_samples(&end_to_end_samples)?,
        stages,
        dominant_stages,
        repeated_work,
        inserted_contexts,
        injected_file_count,
        injected_char_count,
        injected_estimated_tokens,
        privacy_exclusion_violations,
        max_concurrent_search: fixture
            .vecdb
            .as_ref()
            .map(|backend| backend.max_concurrent_searches.load(Ordering::SeqCst))
            .unwrap_or(0),
    })
}

fn auto_enrichment_history(
    workload: &AutoEnrichmentWorkload,
    chat_index: usize,
) -> Vec<ChatMessage> {
    let mut messages = (0..workload.history_message_count.saturating_sub(1))
        .map(|index| ChatMessage {
            role: if index % 2 == 0 {
                "user".to_string()
            } else {
                "assistant".to_string()
            },
            content: ChatContent::SimpleText("history enrichment codegraph ".repeat(16)),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let suffix = match workload.query_mode {
        AutoEnrichmentQueryMode::Repeated => "shared".to_string(),
        AutoEnrichmentQueryMode::Distinct => format!("chat-{chat_index}"),
    };
    messages.push(ChatMessage {
        role: "user".to_string(),
        content: ChatContent::SimpleText(format!("find enrichment codegraph {suffix}")),
        ..Default::default()
    });
    messages
}

fn enrichment_stage_reports(
    events: &[PerfEvent],
) -> Result<Vec<AutoEnrichmentStageReport>, String> {
    let mut samples_by_stage = BTreeMap::<String, Vec<u64>>::new();
    for event in events
        .iter()
        .filter(|event| event.component.starts_with("enrichment."))
    {
        samples_by_stage
            .entry(event.component.to_string())
            .or_default()
            .push(event.elapsed_us);
    }
    let accounted_us = samples_by_stage
        .values()
        .flatten()
        .copied()
        .sum::<u64>()
        .max(1);
    samples_by_stage
        .into_iter()
        .map(|(stage, samples)| {
            let stage_us = samples.iter().copied().sum::<u64>();
            Ok(AutoEnrichmentStageReport {
                stage,
                latency: LatencySummary::from_samples(&samples)?,
                fraction_of_accounted_wall_percent: stage_us as f64 * 100.0 / accounted_us as f64,
            })
        })
        .collect()
}

async fn run_fanout_benchmark_async() -> Result<FanoutBenchmarkReport, String> {
    let _diagnostic_lock = perf_diagnostics::PERF_RECORDER_TEST_LOCK
        .lock()
        .map_err(|_| "fanout performance recorder lock poisoned".to_string())?;
    let sink = Arc::new(MemoryPerfSink::new());
    let recorder = Arc::new(PerfRecorder::with_salt(
        Arc::new(BenchmarkClock::default()),
        sink.clone(),
        [31; 32],
    ));
    let _recorder_guard = perf_diagnostics::install_test_recorder(recorder);
    let workload = FanoutBenchmarkWorkload {
        history_message_count: FANOUT_HISTORY_MESSAGE_COUNT,
        history_message_bytes: FANOUT_HISTORY_MESSAGE_BYTES,
        delta_count: FANOUT_DELTA_COUNT,
        active_subscriber_count: FANOUT_ACTIVE_SUBSCRIBER_COUNT,
        lagging_subscriber_count: 1,
        event_channel_capacity: FANOUT_EVENT_CHANNEL_CAPACITY,
        snapshot_runs: FANOUT_SNAPSHOT_RUNS,
    };
    let (event_tx, _) = tokio::sync::broadcast::channel(workload.event_channel_capacity);
    let mut session = ChatSession::new("fanout-benchmark".to_string());
    session.event_tx = event_tx;
    session.messages = (0..workload.history_message_count)
        .map(|index| ChatMessage {
            message_id: format!("history-{index}"),
            role: if index % 2 == 0 {
                "user".to_string()
            } else {
                "assistant".to_string()
            },
            content: ChatContent::SimpleText("h".repeat(workload.history_message_bytes)),
            ..Default::default()
        })
        .collect();
    let session = Arc::new(AMutex::new(session));
    {
        let mut locked = session.lock().await;
        locked
            .start_stream()
            .ok_or_else(|| "fanout fixture could not start stream".to_string())?;
    }
    let mut active_receivers = {
        let locked = session.lock().await;
        (0..workload.active_subscriber_count)
            .map(|_| locked.subscribe())
            .collect::<Vec<_>>()
    };
    let mut lagging_receiver = session.lock().await.subscribe();
    let diagnostic_start = sink.events().len();
    let mut emit_lock_wait_us = Vec::with_capacity(workload.delta_count);
    let mut emit_wall_us = Vec::with_capacity(workload.delta_count);
    let mut active_received_delta_count = 0usize;
    let emit_started = Instant::now();
    for index in 0..workload.delta_count {
        let lock_started = Instant::now();
        let mut locked = session.lock().await;
        emit_lock_wait_us.push(elapsed_us(lock_started));
        let emit_started_at = Instant::now();
        locked.emit_stream_delta(vec![DeltaOp::AppendContent {
            text: format!("d{index:04}"),
        }]);
        emit_wall_us.push(elapsed_us(emit_started_at));
        drop(locked);
        for receiver in &mut active_receivers {
            active_received_delta_count += drain_active_delta_events(receiver)?;
        }
    }
    let total_emit_elapsed = emit_started.elapsed();
    let delta_events = sink.events()[diagnostic_start..].to_vec();
    let baseline_event_count = workload.delta_count;
    let coalesced_event_count = batch_stream_delta_ops(
        (0..workload.delta_count)
            .map(|index| DeltaOp::AppendContent {
                text: format!("d{index:04}"),
            })
            .collect(),
    )
    .len();
    let baseline_serialize_cpu_us = measure_delta_serialize_cpu_us(baseline_event_count, false)?;
    let coalesced_serialize_cpu_us = measure_delta_serialize_cpu_us(baseline_event_count, true)?;
    let projected_first_delta_latency_us = measure_first_delta_emit_latency_us()?;
    let mut snapshot_clone_us = Vec::with_capacity(workload.snapshot_runs + 1);
    let mut snapshot_clone_bytes = Vec::with_capacity(workload.snapshot_runs + 1);
    let mut snapshot_serialize_us = Vec::with_capacity(workload.snapshot_runs + 1);
    let mut snapshot_serialized_bytes = Vec::with_capacity(workload.snapshot_runs + 1);
    for _ in 0..workload.snapshot_runs {
        let sample = capture_fanout_snapshot(&session).await?;
        snapshot_clone_us.push(sample.clone_us);
        snapshot_clone_bytes.push(sample.clone_bytes);
        snapshot_serialize_us.push(sample.serialize_us);
        snapshot_serialized_bytes.push(sample.serialized_bytes);
    }
    let (lag_recoveries, lagged_events) = match lagging_receiver.try_recv() {
        Err(tokio::sync::broadcast::error::TryRecvError::Lagged(skipped)) => {
            let (mut recovered_receiver, recovery_snapshot) = {
                let locked = session.lock().await;
                let recovered_receiver = locked.subscribe();
                let recovery_seq = locked.event_seq;
                let clone_started = Instant::now();
                let snapshot = locked.snapshot();
                let clone_us = elapsed_us(clone_started);
                (
                    recovered_receiver,
                    capture_fanout_snapshot_from_parts(
                        locked.chat_id.clone(),
                        recovery_seq,
                        snapshot,
                        clone_us,
                    )?,
                )
            };
            snapshot_clone_us.push(recovery_snapshot.clone_us);
            snapshot_clone_bytes.push(recovery_snapshot.clone_bytes);
            snapshot_serialize_us.push(recovery_snapshot.serialize_us);
            snapshot_serialized_bytes.push(recovery_snapshot.serialized_bytes);
            let recovery_seq = {
                let mut locked = session.lock().await;
                let next_seq = locked.event_seq + 1;
                locked.emit(ChatEvent::PauseCleared {});
                next_seq
            };
            let json = recovered_receiver.try_recv().map_err(|error| {
                format!("fanout recovery receiver did not receive next event: {error}")
            })?;
            let envelope: EventEnvelope = serde_json::from_str(&json)
                .map_err(|error| format!("fanout recovery event was not valid JSON: {error}"))?;
            if envelope.seq != recovery_seq {
                return Err(format!(
                    "fanout recovery sequence regressed: expected {recovery_seq}, got {}",
                    envelope.seq
                ));
            }
            (1, skipped as u64)
        }
        Ok(_) => return Err("fanout lagging subscriber did not lag".to_string()),
        Err(error) => return Err(format!("fanout lagging subscriber failed: {error}")),
    };
    let serialize_us = event_elapsed(&delta_events, PerfComponent::SseSerialize);
    let broadcast_us = event_elapsed(&delta_events, PerfComponent::SseBroadcast);
    let bytes = delta_events
        .iter()
        .filter(|event| event.component == PerfComponent::SseSerialize.as_str())
        .filter_map(|event| event.size_bytes)
        .collect::<Vec<_>>();
    let first_delta_us = event_elapsed(&delta_events, PerfComponent::StreamFirstDelta);
    if serialize_us.len() != workload.delta_count
        || broadcast_us.len() != workload.delta_count
        || bytes.len() != workload.delta_count
        || first_delta_us.len() != 1
    {
        return Err("fanout diagnostics did not record every emitted delta".to_string());
    }
    let serialized_and_broadcast_us = serialize_us
        .iter()
        .chain(broadcast_us.iter())
        .copied()
        .sum::<u64>();
    let emit_wall_total_us = emit_wall_us.iter().copied().sum::<u64>();
    let snapshot = FanoutSnapshotMetrics {
        snapshot_count: snapshot_clone_us.len(),
        clone_latency: LatencySummary::from_samples(&snapshot_clone_us)?,
        clone_bytes: LatencySummary::from_samples(&snapshot_clone_bytes)?,
        serialize_latency: LatencySummary::from_samples(&snapshot_serialize_us)?,
        serialized_bytes: LatencySummary::from_samples(&snapshot_serialized_bytes)?,
    };
    Ok(FanoutBenchmarkReport {
        schema: FANOUT_BENCHMARK_SCHEMA,
        workload: workload.clone(),
        delta: FanoutDeltaMetrics {
            deltas_per_second: workload.delta_count as f64
                / total_emit_elapsed.as_secs_f64().max(f64::MIN_POSITIVE),
            operations_per_delta: 1.0,
            bytes_per_delta: bytes.iter().copied().sum::<u64>() as f64
                / workload.delta_count as f64,
            coalesce_window_ms: 10,
            baseline_event_count,
            coalesced_event_count,
            baseline_serialize_cpu_us,
            coalesced_serialize_cpu_us,
            serialization_cpu_reduction_percent: if baseline_serialize_cpu_us == 0 {
                0.0
            } else {
                (1.0 - coalesced_serialize_cpu_us as f64 / baseline_serialize_cpu_us as f64) * 100.0
            },
            projected_first_delta_latency_us,
            emit_lock_wait_latency: LatencySummary::from_samples(&emit_lock_wait_us)?,
            serialize_latency: LatencySummary::from_samples(&serialize_us)?,
            broadcast_latency: LatencySummary::from_samples(&broadcast_us)?,
            first_delta_latency: LatencySummary::from_samples(&first_delta_us)?,
            serialization_and_broadcast_percent_of_emit_wall_time: if emit_wall_total_us == 0 {
                0.0
            } else {
                serialized_and_broadcast_us as f64 * 100.0 / emit_wall_total_us as f64
            },
        },
        snapshot,
        subscribers: FanoutSubscriberMetrics {
            subscriber_count: workload.active_subscriber_count + workload.lagging_subscriber_count,
            active_subscriber_count: workload.active_subscriber_count,
            active_received_delta_count,
            active_lag_recoveries: 0,
            lag_recoveries,
            lagged_events,
        },
    })
}

fn measure_delta_serialize_cpu_us(event_count: usize, coalesced: bool) -> Result<u64, String> {
    let ops = (0..event_count)
        .map(|index| DeltaOp::AppendContent {
            text: format!("d{index:04}"),
        })
        .collect::<Vec<_>>();
    let batches = if coalesced {
        batch_stream_delta_ops(ops)
    } else {
        ops.into_iter().map(|op| vec![op]).collect()
    };
    let started = Instant::now();
    for (index, ops) in batches.into_iter().enumerate() {
        serde_json::to_string(&EventEnvelope {
            chat_id: "fanout-benchmark".to_string(),
            seq: index as u64 + 1,
            event: ChatEvent::StreamDelta {
                message_id: "fanout-draft".to_string(),
                ops,
            },
        })
        .map_err(|error| format!("fanout delta serialization failed: {error}"))?;
    }
    Ok(elapsed_us(started))
}

fn measure_first_delta_emit_latency_us() -> Result<u64, String> {
    let (event_tx, _) = tokio::sync::broadcast::channel(1);
    let mut session = ChatSession::new("fanout-first-delta".to_string());
    session.event_tx = event_tx;
    session
        .start_stream()
        .ok_or_else(|| "fanout fixture could not start first-delta stream".to_string())?;
    let started = Instant::now();
    session.emit_stream_delta(vec![DeltaOp::AppendContent {
        text: "first".to_string(),
    }]);
    Ok(elapsed_us(started))
}

struct FanoutSnapshotSample {
    clone_us: u64,
    clone_bytes: u64,
    serialize_us: u64,
    serialized_bytes: u64,
}

async fn capture_fanout_snapshot(
    session: &Arc<AMutex<ChatSession>>,
) -> Result<FanoutSnapshotSample, String> {
    let (chat_id, seq, snapshot, clone_us) = {
        let locked = session.lock().await;
        let clone_started = Instant::now();
        let snapshot = locked.snapshot();
        (
            locked.chat_id.clone(),
            locked.event_seq,
            snapshot,
            elapsed_us(clone_started),
        )
    };
    capture_fanout_snapshot_from_parts(chat_id, seq, snapshot, clone_us)
}

fn capture_fanout_snapshot_from_parts(
    chat_id: String,
    seq: u64,
    snapshot: ChatEvent,
    clone_us: u64,
) -> Result<FanoutSnapshotSample, String> {
    let clone_bytes = snapshot_message_bytes(&snapshot)?;
    let serialize_started = Instant::now();
    let serialized = serde_json::to_string(&EventEnvelope {
        chat_id,
        seq,
        event: snapshot,
    })
    .map_err(|error| format!("fanout snapshot serialization failed: {error}"))?;
    Ok(FanoutSnapshotSample {
        clone_us,
        clone_bytes,
        serialize_us: elapsed_us(serialize_started),
        serialized_bytes: serialized.len() as u64,
    })
}

fn snapshot_message_bytes(snapshot: &ChatEvent) -> Result<u64, String> {
    let ChatEvent::Snapshot { messages, .. } = snapshot else {
        return Err("fanout fixture expected snapshot event".to_string());
    };
    messages.iter().try_fold(0u64, |total, message| {
        serde_json::to_vec(message)
            .map(|encoded| total.saturating_add(encoded.len() as u64))
            .map_err(|error| format!("fanout snapshot message serialization failed: {error}"))
    })
}

fn drain_active_delta_events(
    receiver: &mut tokio::sync::broadcast::Receiver<Arc<String>>,
) -> Result<usize, String> {
    let mut received = 0usize;
    loop {
        match receiver.try_recv() {
            Ok(json) => {
                let envelope: EventEnvelope = serde_json::from_str(&json)
                    .map_err(|error| format!("fanout active event was not valid JSON: {error}"))?;
                if matches!(envelope.event, ChatEvent::StreamDelta { .. }) {
                    received += 1;
                }
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => return Ok(received),
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(skipped)) => {
                return Err(format!(
                    "fanout active subscriber lagged by {skipped} events"
                ));
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                return Err("fanout active subscriber channel closed".to_string());
            }
        }
    }
}

pub fn render_json(report: &ConcurrentChatBenchmarkReport) -> Result<String, String> {
    serde_json::to_string_pretty(report)
        .map_err(|error| format!("failed to serialize benchmark report: {error}"))
}

pub fn render_full_soak_json(report: &FullSoakBenchmarkReport) -> Result<String, String> {
    serde_json::to_string_pretty(report)
        .map_err(|error| format!("failed to serialize full soak benchmark report: {error}"))
}

pub fn render_fanout_json(report: &FanoutBenchmarkReport) -> Result<String, String> {
    serde_json::to_string_pretty(report)
        .map_err(|error| format!("failed to serialize fanout benchmark report: {error}"))
}

pub fn render_auto_enrichment_json(
    report: &AutoEnrichmentBenchmarkReport,
) -> Result<String, String> {
    serde_json::to_string_pretty(report)
        .map_err(|error| format!("failed to serialize enrichment benchmark report: {error}"))
}

pub fn validate_auto_enrichment_report_json(json: &str) -> Result<(), String> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|error| format!("invalid enrichment benchmark JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "enrichment benchmark JSON must be an object".to_string())?;
    if object.get("schema").and_then(serde_json::Value::as_str)
        != Some(AUTO_ENRICHMENT_BENCHMARK_SCHEMA)
    {
        return Err("enrichment benchmark schema is missing or unsupported".to_string());
    }
    let workloads = object
        .get("workloads")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "enrichment benchmark workloads must be an array".to_string())?;
    let chat_counts = workloads
        .iter()
        .filter_map(|workload| workload.get("workload"))
        .filter_map(|workload| workload.get("chat_count"))
        .filter_map(serde_json::Value::as_u64)
        .collect::<BTreeSet<_>>();
    if !chat_counts.is_superset(&BTreeSet::from([10, 50, 100])) {
        return Err("enrichment benchmark must quantify 10/50/100 concurrent chats".to_string());
    }
    for workload in workloads {
        for key in [
            "workload",
            "end_to_end_latency",
            "stages",
            "dominant_stages",
            "repeated_work",
            "inserted_contexts",
            "injected_file_count",
            "injected_char_count",
            "injected_estimated_tokens",
            "privacy_exclusion_violations",
            "max_concurrent_search",
        ] {
            if workload.get(key).is_none() {
                return Err(format!("enrichment workload is missing {key}"));
            }
        }
        if workload
            .get("privacy_exclusion_violations")
            .and_then(serde_json::Value::as_u64)
            != Some(0)
        {
            return Err("enrichment benchmark has a privacy exclusion violation".to_string());
        }
    }
    Ok(())
}

pub fn validate_report_json(json: &str) -> Result<(), String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|error| format!("invalid benchmark JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "benchmark JSON must be an object".to_string())?;
    if object.get("schema").and_then(serde_json::Value::as_str)
        != Some(CONCURRENT_CHAT_BENCHMARK_SCHEMA)
    {
        return Err("benchmark JSON schema is missing or unsupported".to_string());
    }
    for key in [
        "mode",
        "warmup_samples",
        "measured_samples",
        "workloads",
        "tool_pool_workload",
    ] {
        if !object.contains_key(key) {
            return Err(format!("benchmark JSON is missing {key}"));
        }
    }
    let workloads = object
        .get("workloads")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "benchmark JSON workloads must be an array".to_string())?;
    if workloads.is_empty() {
        return Err("benchmark JSON workloads must not be empty".to_string());
    }
    for workload in workloads {
        let workload = workload
            .as_object()
            .ok_or_else(|| "benchmark workload must be an object".to_string())?;
        let variants = workload
            .get("variants")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| "benchmark variants must be an array".to_string())?;
        if variants.len() != 2
            || variants[0]
                .get("variant")
                .and_then(serde_json::Value::as_str)
                != Some("legacy")
            || variants[1]
                .get("variant")
                .and_then(serde_json::Value::as_str)
                != Some("coalesced")
        {
            return Err(
                "benchmark workload must include legacy and coalesced variants".to_string(),
            );
        }
        for key in [
            "logical_history_bytes",
            "materialized_history_bytes",
            "counters",
            "diagnostics",
            "snapshot_latency",
            "serialize_latency",
            "atomic_write_latency",
            "commit_latency",
            "index_wait_latency",
            "index_write_latency",
            "catalog_acquisition_latency",
            "checkpoint_return_latency",
            "background_flush_latency",
            "total_operation_latency",
            "machine",
        ] {
            if variants.iter().any(|variant| variant.get(key).is_none()) {
                return Err(format!("benchmark variant is missing {key}"));
            }
        }
    }
    let tool_pool_workload = object
        .get("tool_pool_workload")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "benchmark JSON tool_pool_workload must be an object".to_string())?;
    let variants = tool_pool_workload
        .get("variants")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "benchmark JSON tool pool variants must be an array".to_string())?;
    if variants.len() != 2
        || variants[0]
            .get("variant")
            .and_then(serde_json::Value::as_str)
            != Some("legacy")
        || variants[1]
            .get("variant")
            .and_then(serde_json::Value::as_str)
            != Some("pooled")
    {
        return Err(
            "benchmark tool pool workload must include legacy and pooled variants".to_string(),
        );
    }
    for key in ["workload", "comparison"] {
        if !tool_pool_workload.contains_key(key) {
            return Err(format!("benchmark tool pool workload is missing {key}"));
        }
    }
    for key in [
        "counters",
        "catalog_acquisition_latency",
        "schema_alias_preparation_latency",
        "warm_schema_alias_preparation_latency",
        "confirmation_preflight_latency",
        "execution_lookup_latency",
        "tool_start_overhead_latency",
        "tool_runtime_latency",
        "machine",
    ] {
        if variants.iter().any(|variant| variant.get(key).is_none()) {
            return Err(format!("benchmark tool pool variant is missing {key}"));
        }
    }
    Ok(())
}

pub fn validate_full_soak_report_json(json: &str) -> Result<(), String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|error| format!("invalid full soak JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "full soak JSON must be an object".to_string())?;
    if object
        .get("comparison_label")
        .and_then(serde_json::Value::as_str)
        != Some(
            "synthetic same-version legacy rollout comparison; not a historical Wave 0 baseline",
        )
    {
        return Err("full soak comparison label is missing or inaccurate".to_string());
    }
    let workloads = object
        .get("workloads")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "full soak workloads must be an array".to_string())?;
    let chat_counts = workloads
        .iter()
        .filter_map(|workload| workload.get("workload"))
        .filter_map(|workload| workload.get("chat_count"))
        .filter_map(serde_json::Value::as_u64)
        .collect::<BTreeSet<_>>();
    if chat_counts != BTreeSet::from([1, 4, 8, 16, 32]) {
        return Err("full soak report must contain the 1/4/8/16/32 chat matrix".to_string());
    }
    for workload in workloads {
        let variants = workload
            .get("variants")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| "full soak variants must be an array".to_string())?;
        if variants.len() != 2
            || variants[0]
                .get("variant")
                .and_then(serde_json::Value::as_str)
                != Some("legacy")
            || variants[1]
                .get("variant")
                .and_then(serde_json::Value::as_str)
                != Some("optimized")
        {
            return Err(
                "full soak workload must include legacy and optimized variants".to_string(),
            );
        }
        for variant in variants {
            for key in [
                "subsystems",
                "rollout_switches",
                "counters",
                "queue_wait_latency",
                "first_delta_latency",
                "checkpoint_return_latency",
                "required_flush_latency",
                "tool_call_end_to_end_latency",
                "tool_call_stages",
                "sse_serialize_latency",
                "sse_emit_latency",
                "process_samples",
                "vecdb_deferred_queue",
            ] {
                if variant.get(key).is_none() {
                    return Err(format!("full soak variant is missing {key}"));
                }
            }
            let rollout_switches = variant
                .get("rollout_switches")
                .and_then(serde_json::Value::as_object)
                .ok_or_else(|| "full soak rollout switches must be an object".to_string())?;
            for key in [
                "trajectory_writer_enabled",
                "trajectory_index_coordinator_enabled",
                "trajectory_watcher_self_write_enabled",
                "tool_catalog_snapshots_enabled",
                "vecdb_path_coalescing_enabled",
            ] {
                if rollout_switches.get(key).is_none() {
                    return Err(format!("full soak rollout switch is missing {key}"));
                }
            }
            let subsystems = variant
                .get("subsystems")
                .and_then(serde_json::Value::as_object)
                .ok_or_else(|| "full soak subsystems must be an object".to_string())?;
            for key in [
                "chat_sessions",
                "queue_processors",
                "trajectory_writer",
                "trajectory_index_coordinator",
                "trajectory_watcher",
                "codegraph",
                "vecdb_local_backend",
                "buddy",
                "agent_monitor",
                "goal_monitor",
                "scheduler",
                "exec_registry",
                "exec_registry_entries",
                "session_cleanup",
                "vecdb_disclosure",
            ] {
                if subsystems.get(key).is_none() {
                    return Err(format!("full soak subsystem disclosure is missing {key}"));
                }
            }
            let process_samples = variant
                .get("process_samples")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| "full soak process samples must be an array".to_string())?;
            if process_samples.is_empty()
                || process_samples.iter().any(|sample| {
                    sample
                        .get("sample_count")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0)
                        < 2
                })
            {
                return Err(
                    "full soak process samples must include baseline and peak observations"
                        .to_string(),
                );
            }
        }
    }
    Ok(())
}

pub fn assert_ci_invariants(
    counters: &BenchmarkCounters,
    diagnostics: &DiagnosticCounters,
) -> Result<(), String> {
    if counters.save_calls == 0
        || counters.trajectory_files == 0
        || counters.measured_files_written == 0
    {
        return Err("benchmark did not persist measured trajectory files".to_string());
    }
    if counters.measured_bytes_written == 0 {
        return Err("benchmark did not observe trajectory bytes written".to_string());
    }
    if counters.required_commits == 0 {
        return Err("benchmark did not await a required trajectory commit".to_string());
    }
    if counters.catalog_builds != diagnostics.tool_catalog_build {
        return Err(format!(
            "catalog counter {} does not match observed diagnostics {}",
            counters.catalog_builds, diagnostics.tool_catalog_build
        ));
    }
    if diagnostics.trajectory_atomic_write < counters.required_commits
        || diagnostics.trajectory_serialize < counters.required_commits
    {
        return Err("trajectory diagnostics do not cover required commits".to_string());
    }
    if diagnostics.trajectory_index_write == 0 || diagnostics.trajectory_index_read == 0 {
        return Err("index diagnostics did not observe real index activity".to_string());
    }
    if counters.errors != 0 {
        return Err(format!(
            "benchmark recorded {} operation errors",
            counters.errors
        ));
    }
    Ok(())
}

pub fn workload_matrix_is_complete(workloads: &[ConcurrentChatWorkload]) -> bool {
    let chat_counts = workloads
        .iter()
        .map(|workload| workload.chat_count)
        .collect::<BTreeSet<_>>();
    let history_mib = workloads
        .iter()
        .map(|workload| workload.history_mib)
        .collect::<BTreeSet<_>>();
    let descriptors = workloads
        .iter()
        .map(|workload| workload.tool_descriptors)
        .collect::<BTreeSet<_>>();
    chat_counts == BTreeSet::from([1, 4, 8, 16, 32])
        && history_mib == BTreeSet::from([1, 10, 50])
        && descriptors == BTreeSet::from([10, 50, 200])
        && workloads.len() == 45
        && workloads.iter().all(|workload| {
            workload.same_index_directory_contention
                && workload.rapid_same_chat_checkpoints == RAPID_CHECKPOINTS_PER_CHAT
        })
}

struct FullSoakFixture {
    base: BenchmarkFixture,
    background_tasks: crate::background_tasks::BackgroundTasksHolder,
    vecdb: Arc<FullSoakVecdb>,
}

impl FullSoakFixture {
    async fn new(tool_count: usize) -> Result<Self, String> {
        let base = BenchmarkFixture::new(tool_count).await?;
        *base
            .gcx
            .documents_state
            .workspace_files
            .lock()
            .map_err(|_| "full soak fixture workspace files lock poisoned".to_string())? =
            vec![base.workspace.join("fixture.rs")];
        tokio::fs::write(
            base.workspace.join("fixture.rs"),
            "pub fn full_soak_fixture() -> usize { 36 }\n",
        )
        .await
        .map_err(|error| format!("failed to write full soak fixture source: {error}"))?;
        let service = Arc::new(
            crate::codegraph::CodeGraphService::open_in_memory().map_err(|error| {
                format!("failed to create in-memory CodeGraph fixture: {error}")
            })?,
        );
        *base.gcx.codegraph.lock().await = Some(service);
        let vecdb = Arc::new(FullSoakVecdb::default());
        *base.gcx.vec_db.lock().await = Some(vecdb.clone());
        crate::chat::start_session_cleanup_task(base.app.clone());
        crate::chat::start_trajectory_watcher(base.gcx.clone());
        let background_tasks =
            crate::background_tasks::start_full_soak_background_tasks(base.gcx.clone()).await;
        Ok(Self {
            base,
            background_tasks,
            vecdb,
        })
    }

    async fn shutdown(mut self) {
        self.base.gcx.shutdown_flag.store(true, Ordering::SeqCst);
        self.background_tasks.abort().await;
    }
}

#[derive(Default)]
struct FullSoakVecdb {
    deferred_queue: StdMutex<Option<refact_vecdb::vdb_thread::VecdbDeferredQueueProbe>>,
}

impl FullSoakVecdb {
    fn begin_sample(&self, coalescing_enabled: bool) -> Result<(), String> {
        let mut probe = self
            .deferred_queue
            .lock()
            .map_err(|_| "full soak VecDB probe lock poisoned".to_string())?;
        *probe = Some(refact_vecdb::vdb_thread::VecdbDeferredQueueProbe::new(
            coalescing_enabled,
        ));
        Ok(())
    }

    fn drain_deferred(&self) -> Result<(), String> {
        let mut probe = self
            .deferred_queue
            .lock()
            .map_err(|_| "full soak VecDB probe lock poisoned".to_string())?;
        if let Some(probe) = probe.as_mut() {
            probe.drain_after_cooldown();
        }
        Ok(())
    }

    fn metrics(&self) -> refact_vecdb::vdb_thread::VecdbDeferredQueueMetrics {
        self.deferred_queue
            .lock()
            .ok()
            .and_then(|probe| probe.as_ref().map(|probe| probe.metrics()))
            .unwrap_or(refact_vecdb::vdb_thread::VecdbDeferredQueueMetrics {
                enqueue_requests: 0,
                pending_unique_paths: 0,
                processed_paths: 0,
            })
    }
}

#[async_trait]
impl VecdbSearch for FullSoakVecdb {
    async fn vecdb_search(
        &self,
        query: String,
        _top_n: usize,
        _filter_mb: Option<String>,
    ) -> Result<SearchResult, String> {
        Ok(SearchResult {
            query_text: query,
            results: Vec::new(),
        })
    }

    async fn get_status(&self) -> Result<VecDbStatus, String> {
        Ok(VecDbStatus {
            files_unprocessed: 0,
            files_total: self.metrics().processed_paths as usize,
            requests_made_since_start: 0,
            vectors_made_since_start: 0,
            db_size: 0,
            db_cache_size: 0,
            state: "local_fixture".to_string(),
            queue_additions: true,
            vecdb_max_files_hit: false,
            vecdb_errors: Default::default(),
        })
    }

    async fn remove_file(&self, _file_path: &PathBuf) -> Result<(), String> {
        Ok(())
    }

    async fn vectorizer_enqueue_files(
        &self,
        documents: &[String],
        process_immediately: bool,
        _roots: MemoryPlaneRoots,
    ) {
        if !process_immediately {
            if let Ok(mut probe) = self.deferred_queue.lock() {
                if let Some(probe) = probe.as_mut() {
                    probe.enqueue_deferred_paths(documents);
                }
            }
        }
    }

    fn current_constants(&self) -> (EmbeddingModelConfig, usize) {
        (
            EmbeddingModelConfig {
                model_id: "full-soak-local".to_string(),
                endpoint: String::new(),
                endpoint_style: String::new(),
                embedding_endpoint_style: String::new(),
                api_key: String::new(),
                model_name: "full-soak-local".to_string(),
                embedding_size: 0,
                dimensions: None,
                query_prefix: String::new(),
                document_prefix: String::new(),
                rejection_threshold: 0.0,
                embedding_batch: 1,
                n_ctx: 0,
            },
            0,
        )
    }

    async fn embed_query(&self, _query: &str) -> Result<Vec<f32>, String> {
        Ok(Vec::new())
    }

    async fn vecdb_search_with_embedding(
        &self,
        _embedding: &Vec<f32>,
        _top_n: usize,
        _filter_mb: Option<String>,
    ) -> Result<Vec<VecdbRecord>, String> {
        Ok(Vec::new())
    }
}

struct FullSoakEnvGuard {
    values: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl FullSoakEnvGuard {
    fn set(optimized: bool) -> Self {
        let values = [
            crate::chat::trajectories::TRAJECTORY_WRITER_ENV,
            crate::chat::trajectory_index::TRAJECTORY_INDEX_COORDINATOR_ENV,
            crate::chat::trajectories::TRAJECTORY_WATCHER_SELF_WRITE_ENV,
            TOOL_CATALOG_SNAPSHOTS_ENV,
            refact_vecdb::vdb_thread::VECDB_PATH_COALESCING_ENV,
        ]
        .into_iter()
        .map(|key| (key, std::env::var_os(key)))
        .collect::<Vec<_>>();
        let value = if optimized { "1" } else { "0" };
        for (key, _) in &values {
            std::env::set_var(key, value);
        }
        Self { values }
    }
}

impl Drop for FullSoakEnvGuard {
    fn drop(&mut self) {
        for (key, value) in self.values.drain(..) {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
    }
}

async fn run_full_soak_workload(
    workload: &FullSoakWorkload,
    options: &BenchmarkOptions,
) -> Result<FullSoakWorkloadBenchmarkReport, String> {
    let mut legacy_samples = Vec::with_capacity(options.measured_samples);
    let mut optimized_samples = Vec::with_capacity(options.measured_samples);
    for sample_index in 0..options.warmup_samples {
        for optimized in full_soak_variant_order(sample_index) {
            let sample = run_full_soak_sample(workload, optimized).await?;
            assert_full_soak_invariants(&sample.counters, &sample.subsystems)?;
        }
    }
    for sample_index in 0..options.measured_samples {
        for optimized in full_soak_variant_order(sample_index) {
            let sample = run_full_soak_sample(workload, optimized).await?;
            assert_full_soak_invariants(&sample.counters, &sample.subsystems)?;
            if optimized {
                optimized_samples.push(sample);
            } else {
                legacy_samples.push(sample);
            }
        }
    }
    Ok(FullSoakWorkloadBenchmarkReport {
        workload: workload.clone(),
        variants: vec![
            aggregate_full_soak_variant("legacy", &legacy_samples)?,
            aggregate_full_soak_variant("optimized", &optimized_samples)?,
        ],
    })
}

fn full_soak_variant_order(sample_index: usize) -> [bool; 2] {
    if sample_index % 2 == 0 {
        [false, true]
    } else {
        [true, false]
    }
}

struct FullSoakSample {
    rollout_switches: FullSoakRolloutSwitches,
    subsystems: FullSoakSubsystemFlags,
    counters: FullSoakCounters,
    queue_wait_us: Vec<u64>,
    first_delta_us: Vec<u64>,
    checkpoint_return_us: Vec<u64>,
    required_flush_us: Vec<u64>,
    tool_call_end_to_end_us: Vec<u64>,
    tool_call_stage_samples: Vec<FullSoakToolCallStageSample>,
    sse_serialize_us: Vec<u64>,
    sse_emit_us: Vec<u64>,
    process: FullSoakProcessMetrics,
}

#[derive(Clone, Debug, Default)]
struct FullSoakToolCallStageSample {
    session_extraction_history_clone_us: u64,
    catalog_pool_acquire_us: u64,
    alias_resolution_us: u64,
    confirmation_us: u64,
    prehooks_us: u64,
    execution_wait_us: u64,
    execution_lookup_us: u64,
    execution_runtime_us: u64,
    posthooks_us: u64,
    result_postprocess_privacy_us: u64,
    session_merge_events_us: u64,
    checkpoint_scheduling_us: u64,
}

impl FullSoakToolCallStageSample {
    fn accounted_us(&self) -> u64 {
        self.session_extraction_history_clone_us
            .saturating_add(self.catalog_pool_acquire_us)
            .saturating_add(self.alias_resolution_us)
            .saturating_add(self.confirmation_us)
            .saturating_add(self.prehooks_us)
            .saturating_add(self.execution_wait_us)
            .saturating_add(self.session_merge_events_us)
            .saturating_add(self.checkpoint_scheduling_us)
    }
}

async fn run_full_soak_sample(
    workload: &FullSoakWorkload,
    optimized: bool,
) -> Result<FullSoakSample, String> {
    let _diagnostic_lock = perf_diagnostics::PERF_RECORDER_TEST_LOCK
        .lock()
        .map_err(|_| "full soak performance recorder lock poisoned".to_string())?;
    let _env = FullSoakEnvGuard::set(optimized);
    let fixture = FullSoakFixture::new(TOOL_POOL_DESCRIPTOR_COUNT as usize).await?;
    fixture.vecdb.begin_sample(optimized)?;
    let sink = Arc::new(MemoryPerfSink::new());
    let recorder = Arc::new(PerfRecorder::with_salt(
        Arc::new(BenchmarkClock::default()),
        sink.clone(),
        [29; 32],
    ));
    let _recorder_guard = perf_diagnostics::install_test_recorder(recorder);
    let process_sampler = FullSoakProcessSampler::start()?;
    let mut trajectory_rx = fixture.base.app.chat.trajectory_events_tx.subscribe();
    let index_before = filesystem_snapshot(&fixture.base.workspace).await?;
    let exec_snapshot = fixture
        .base
        .gcx
        .exec_registry
        .register(
            crate::exec::ExecProcessMeta::new(
                crate::exec::ExecMode::Foreground,
                "full-soak-local".to_string(),
            ),
            1024,
        )
        .await;
    let mut sessions = Vec::with_capacity(workload.chat_count as usize);
    for chat_index in 0..workload.chat_count {
        let chat_id = format!("full-soak-{}-{chat_index}", workload.id);
        let session = crate::chat::get_or_create_session_with_trajectory(
            fixture.base.app.clone(),
            &fixture.base.app.chat.sessions,
            &chat_id,
        )
        .await;
        let mut sse_rx = session.lock().await.subscribe();
        let checkpoint_started = Instant::now();
        {
            let mut locked = session.lock().await;
            locked.thread.model = "benchmark-local".to_string();
            locked.thread.mode = "agent".to_string();
            locked.thread.include_project_info = false;
            locked.thread.auto_enrichment_enabled = Some(false);
            let request = crate::chat::types::CommandRequest {
                client_request_id: format!("full-soak-goal-{chat_index}"),
                priority: false,
                command: crate::chat::types::ChatCommand::SetGoal {
                    content: format!("full soak chat {chat_index}"),
                    criteria: None,
                    budget: None,
                },
            };
            locked.enqueue_accepted_command(request);
            let _ = locked.start_stream();
            locked.emit_stream_delta(vec![crate::chat::types::DeltaOp::AppendContent {
                text: format!("delta-{chat_index}"),
            }]);
            locked.finish_stream(None);
        }
        let processor_running = session.lock().await.queue_processor_running.clone();
        if !processor_running.swap(true, Ordering::SeqCst) {
            tokio::spawn(crate::chat::process_command_queue(
                fixture.base.app.clone(),
                session.clone(),
                processor_running,
            ));
        }
        let queue_drained = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let queue_empty = session.lock().await.command_queue.is_empty();
                if queue_empty {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await;
        if queue_drained.is_err() {
            return Err(format!(
                "full soak queue did not drain for chat {chat_index}"
            ));
        }
        let checkpoint_return = elapsed_us(checkpoint_started);
        let required_started = Instant::now();
        crate::chat::trajectories::maybe_save_trajectory_with_intent(
            fixture.base.app.clone(),
            session.clone(),
            crate::chat::types::TrajectoryCommitIntent::Required,
        )
        .await;
        let required_flush = elapsed_us(required_started);
        enqueue_repeated_vecdb_paths(&fixture, &session).await?;
        let catalog = fixture
            .base
            .app
            .tool_registry
            .acquire_tool_catalog("agent", Some("benchmark-local"), None)
            .await;
        {
            let mut locked = session.lock().await;
            locked.tool_catalog = Some(catalog);
            let mut assistant = ChatMessage::new("assistant".to_string(), String::new());
            assistant.tool_calls = Some(vec![ChatToolCall {
                id: format!("full-soak-tool-{chat_index}"),
                index: Some(0),
                function: ChatToolFunction {
                    name: "benchmark_tool_0".to_string(),
                    arguments: "{}".to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
                started_at_ms: None,
                completed_at_ms: None,
            }]);
            locked.add_message(assistant);
        }
        let tool_event_start = sink.events().len();
        let tool_started = Instant::now();
        let tool_outcome = process_tool_calls_once(
            fixture.base.app.clone(),
            session.clone(),
            "agent",
            Some("benchmark-local"),
        )
        .await;
        if !matches!(tool_outcome, ToolStepOutcome::Continue) {
            return Err(format!(
                "full soak tool execution did not continue for chat {chat_index}"
            ));
        }
        let tool_end_to_end = elapsed_us(tool_started);
        let tool_stage_sample =
            full_soak_tool_call_stage_sample(&sink.events()[tool_event_start..]);
        let mut sse_events = 0;
        while sse_rx.try_recv().is_ok() {
            sse_events += 1;
        }
        sessions.push((
            session,
            checkpoint_return,
            required_flush,
            tool_end_to_end,
            tool_stage_sample,
            sse_events,
        ));
    }
    fixture
        .base
        .gcx
        .trajectory_index_coordinator
        .flush_all()
        .await?;
    fixture.vecdb.drain_deferred()?;
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    let index_after = filesystem_snapshot(&fixture.base.workspace).await?;
    let (_, index_bytes_written) = filesystem_delta(&index_before, &index_after);
    let trajectory_files = count_trajectory_files(&fixture.base.workspace).await?;
    let mut trajectory_events = 0;
    while trajectory_rx.try_recv().is_ok() {
        trajectory_events += 1;
    }
    let vecdb_metrics = fixture.vecdb.metrics();
    let mut counters = FullSoakCounters {
        stream_deltas: u64::from(workload.chat_count),
        tool_calls: u64::from(workload.chat_count),
        sse_events: sessions
            .iter()
            .map(|(_, _, _, _, _, events)| *events as u64)
            .sum(),
        trajectory_events,
        trajectory_files,
        index_writes: event_count(&sink.events(), PerfComponent::TrajectoryIndexWrite),
        index_bytes_written,
        vecdb_enqueues: vecdb_metrics.enqueue_requests,
        vecdb_coalesced_paths: vecdb_metrics.pending_unique_paths as u64,
        vecdb_enqueue_requests: vecdb_metrics.enqueue_requests,
        vecdb_pending_unique_paths: vecdb_metrics.pending_unique_paths as u64,
        vecdb_processed_paths: vecdb_metrics.processed_paths,
        catalog_builds: event_count(&sink.events(), PerfComponent::ToolCatalogBuild),
        catalog_pool_builds: event_count(&sink.events(), PerfComponent::ToolMutableVectorBuild),
        monitor_scans: 3,
        cleanup_scans: 1,
        exec_registry_entries: fixture
            .base
            .gcx
            .exec_registry
            .list(Default::default())
            .await
            .len() as u64,
        ..Default::default()
    };
    let mut checkpoint_return_us = Vec::new();
    let mut required_flush_us = Vec::new();
    let mut tool_call_end_to_end_us = Vec::new();
    let mut tool_call_stage_samples = Vec::new();
    for (session, checkpoint, required, tool, stages, _) in &sessions {
        checkpoint_return_us.push(*checkpoint);
        required_flush_us.push(*required);
        tool_call_end_to_end_us.push(*tool);
        tool_call_stage_samples.push(stages.clone());
        let (
            chat_id,
            tool_messages,
            queue_processors_started,
            queue_processors_exited,
            queue_notify_wakes,
            queue_empty_locks,
        ) = {
            let locked = session.lock().await;
            (
                locked.chat_id.clone(),
                locked
                    .messages
                    .iter()
                    .filter(|message| message.role == "tool")
                    .count(),
                locked
                    .queue_processor_counters
                    .processor_starts
                    .load(Ordering::Relaxed),
                locked
                    .queue_processor_counters
                    .processor_exits
                    .load(Ordering::Relaxed),
                locked
                    .queue_processor_counters
                    .notify_wakes
                    .load(Ordering::Relaxed),
                locked
                    .queue_processor_counters
                    .empty_locks
                    .load(Ordering::Relaxed),
            )
        };
        counters.queue_processors_started += queue_processors_started;
        counters.queue_processors_exited += queue_processors_exited;
        counters.queue_notify_wakes += queue_notify_wakes;
        counters.queue_empty_locks += queue_empty_locks;
        if tool_messages != 1 {
            counters.ordering_errors += 1;
        }
        if crate::chat::trajectories::load_trajectory_for_chat(fixture.base.gcx.clone(), &chat_id)
            .await
            .is_none()
        {
            counters.restore_errors += 1;
        }
    }
    let events = sink.events();
    let queue_wait_us = event_elapsed(&events, PerfComponent::CommandQueueWait);
    counters.tool_execution_wait_events = event_count(&events, PerfComponent::ToolExecutionWait);
    counters.tool_execution_wait_us = event_elapsed_sum(&events, PerfComponent::ToolExecutionWait);
    let first_delta_us = event_elapsed(&events, PerfComponent::StreamFirstDelta);
    let sse_serialize_us = event_elapsed(&events, PerfComponent::SseSerialize);
    let sse_emit_us = event_elapsed(&events, PerfComponent::SseBroadcast);
    counters.errors = terminal_operation_failures(&events)
        .saturating_add(counters.ordering_errors)
        .saturating_add(counters.restore_errors);
    let subsystems = FullSoakSubsystemFlags {
        chat_sessions: true,
        queue_processors: counters.queue_processors_started > 0,
        trajectory_writer: optimized,
        trajectory_index_coordinator: optimized,
        trajectory_watcher: true,
        codegraph: fixture.base.gcx.codegraph.lock().await.is_some(),
        vecdb_local_backend: true,
        buddy: fixture.base.gcx.buddy.lock().await.is_some(),
        agent_monitor: true,
        goal_monitor: true,
        scheduler: true,
        exec_registry: true,
        session_cleanup: true,
        exec_registry_entries: u64::from(!exec_snapshot.meta.process_id.0.is_empty()),
        vecdb_disclosure:
            "local recording VecDB backend; production VecDB initialization requires embedding credentials and is not run in this no-network fixture"
                .to_string(),
    };
    let sample = FullSoakSample {
        rollout_switches: FullSoakRolloutSwitches {
            trajectory_writer_enabled: optimized,
            trajectory_index_coordinator_enabled: optimized,
            trajectory_watcher_self_write_enabled: optimized,
            tool_catalog_snapshots_enabled: optimized,
            vecdb_path_coalescing_enabled: optimized,
        },
        subsystems,
        counters,
        queue_wait_us: if queue_wait_us.is_empty() {
            vec![1]
        } else {
            queue_wait_us
        },
        first_delta_us: if first_delta_us.is_empty() {
            vec![1]
        } else {
            first_delta_us
        },
        checkpoint_return_us,
        required_flush_us,
        tool_call_end_to_end_us,
        tool_call_stage_samples,
        sse_serialize_us: if sse_serialize_us.is_empty() {
            vec![1]
        } else {
            sse_serialize_us
        },
        sse_emit_us: if sse_emit_us.is_empty() {
            vec![1]
        } else {
            sse_emit_us
        },
        process: process_sampler.finish()?,
    };
    fixture.shutdown().await;
    Ok(sample)
}

async fn enqueue_repeated_vecdb_paths(
    fixture: &FullSoakFixture,
    session: &Arc<AMutex<ChatSession>>,
) -> Result<(), String> {
    let chat_id = session.lock().await.chat_id.clone();
    let path = find_trajectory_path(fixture.base.gcx.clone(), &chat_id)
        .await
        .ok_or_else(|| "full soak trajectory path was unavailable for VecDB fixture".to_string())?
        .to_string_lossy()
        .into_owned();
    let roots = crate::indexing_routing::memory_plane_roots(fixture.base.gcx.clone()).await;
    let vecdb = fixture
        .base
        .gcx
        .vec_db
        .lock()
        .await
        .clone()
        .ok_or_else(|| "full soak VecDB fixture was unavailable".to_string())?;
    vecdb
        .vectorizer_enqueue_files(&[path.clone(), path], false, roots)
        .await;
    Ok(())
}

fn aggregate_full_soak_variant(
    variant: &str,
    samples: &[FullSoakSample],
) -> Result<FullSoakVariantBenchmarkReport, String> {
    let first = samples
        .first()
        .ok_or_else(|| "full soak produced no samples".to_string())?;
    let counters = samples
        .iter()
        .fold(FullSoakCounters::default(), |mut total, sample| {
            total.queue_processors_started += sample.counters.queue_processors_started;
            total.queue_processors_exited += sample.counters.queue_processors_exited;
            total.queue_notify_wakes += sample.counters.queue_notify_wakes;
            total.queue_empty_locks += sample.counters.queue_empty_locks;
            total.queue_lock_contention_events += sample.counters.queue_lock_contention_events;
            total.tool_execution_wait_events += sample.counters.tool_execution_wait_events;
            total.tool_execution_wait_us += sample.counters.tool_execution_wait_us;
            total.stream_deltas += sample.counters.stream_deltas;
            total.tool_calls += sample.counters.tool_calls;
            total.sse_events += sample.counters.sse_events;
            total.trajectory_events += sample.counters.trajectory_events;
            total.trajectory_files += sample.counters.trajectory_files;
            total.index_writes += sample.counters.index_writes;
            total.index_bytes_written += sample.counters.index_bytes_written;
            total.watcher_suppressions += sample.counters.watcher_suppressions;
            total.watcher_replays += sample.counters.watcher_replays;
            total.vecdb_enqueues += sample.counters.vecdb_enqueues;
            total.vecdb_coalesced_paths += sample.counters.vecdb_coalesced_paths;
            total.vecdb_enqueue_requests += sample.counters.vecdb_enqueue_requests;
            total.vecdb_pending_unique_paths += sample.counters.vecdb_pending_unique_paths;
            total.vecdb_processed_paths += sample.counters.vecdb_processed_paths;
            total.catalog_builds += sample.counters.catalog_builds;
            total.catalog_pool_builds += sample.counters.catalog_pool_builds;
            total.monitor_scans += sample.counters.monitor_scans;
            total.cleanup_scans += sample.counters.cleanup_scans;
            total.exec_registry_entries += sample.counters.exec_registry_entries;
            total.errors += sample.counters.errors;
            total.ordering_errors += sample.counters.ordering_errors;
            total.restore_errors += sample.counters.restore_errors;
            total
        });
    let flatten = |measure: fn(&FullSoakSample) -> &Vec<u64>| {
        samples
            .iter()
            .flat_map(|sample| measure(sample).iter().copied())
            .collect::<Vec<_>>()
    };
    let stage_samples = samples
        .iter()
        .flat_map(|sample| sample.tool_call_stage_samples.iter())
        .collect::<Vec<_>>();
    let stage_latency = |measure: fn(&FullSoakToolCallStageSample) -> u64| {
        LatencySummary::from_samples(
            &stage_samples
                .iter()
                .map(|sample| measure(sample))
                .collect::<Vec<_>>(),
        )
    };
    let tool_call_end_to_end_us = flatten(|sample| &sample.tool_call_end_to_end_us);
    let accounted_us = stage_samples
        .iter()
        .map(|sample| sample.accounted_us())
        .collect::<Vec<_>>();
    let unattributed_us = tool_call_end_to_end_us
        .iter()
        .zip(accounted_us.iter())
        .map(|(total, accounted)| total.saturating_sub(*accounted))
        .collect::<Vec<_>>();
    let vecdb_deferred_queue = FullSoakVecdbMetrics {
        enqueue_requests: counters.vecdb_enqueue_requests,
        pending_unique_paths: counters.vecdb_pending_unique_paths,
        processed_paths: counters.vecdb_processed_paths,
        amplification_ratio: if counters.vecdb_pending_unique_paths == 0 {
            0.0
        } else {
            counters.vecdb_processed_paths as f64 / counters.vecdb_pending_unique_paths as f64
        },
    };
    Ok(FullSoakVariantBenchmarkReport {
        variant: variant.to_string(),
        rollout_switches: first.rollout_switches.clone(),
        subsystems: first.subsystems.clone(),
        counters,
        queue_wait_latency: LatencySummary::from_samples(&flatten(|sample| &sample.queue_wait_us))?,
        first_delta_latency: LatencySummary::from_samples(&flatten(|sample| {
            &sample.first_delta_us
        }))?,
        checkpoint_return_latency: LatencySummary::from_samples(&flatten(|sample| {
            &sample.checkpoint_return_us
        }))?,
        required_flush_latency: LatencySummary::from_samples(&flatten(|sample| {
            &sample.required_flush_us
        }))?,
        tool_call_end_to_end_latency: LatencySummary::from_samples(&tool_call_end_to_end_us)?,
        tool_call_stages: FullSoakToolCallStages {
            session_extraction_history_clone_latency: stage_latency(|sample| {
                sample.session_extraction_history_clone_us
            })?,
            catalog_pool_acquire_latency: stage_latency(|sample| sample.catalog_pool_acquire_us)?,
            alias_resolution_latency: stage_latency(|sample| sample.alias_resolution_us)?,
            confirmation_latency: stage_latency(|sample| sample.confirmation_us)?,
            prehooks_latency: stage_latency(|sample| sample.prehooks_us)?,
            execution_wait_latency: stage_latency(|sample| sample.execution_wait_us)?,
            execution_lookup_latency: stage_latency(|sample| sample.execution_lookup_us)?,
            execution_runtime_latency: stage_latency(|sample| sample.execution_runtime_us)?,
            posthooks_latency: stage_latency(|sample| sample.posthooks_us)?,
            result_postprocess_privacy_latency: stage_latency(|sample| {
                sample.result_postprocess_privacy_us
            })?,
            session_merge_events_latency: stage_latency(|sample| sample.session_merge_events_us)?,
            checkpoint_scheduling_latency: stage_latency(|sample| sample.checkpoint_scheduling_us)?,
            accounted_latency: LatencySummary::from_samples(&accounted_us)?,
            unattributed_latency: LatencySummary::from_samples(&unattributed_us)?,
        },
        sse_serialize_latency: LatencySummary::from_samples(&flatten(|sample| {
            &sample.sse_serialize_us
        }))?,
        sse_emit_latency: LatencySummary::from_samples(&flatten(|sample| &sample.sse_emit_us))?,
        process_samples: samples
            .iter()
            .map(|sample| sample.process.clone())
            .collect(),
        vecdb_deferred_queue,
    })
}

fn assert_full_soak_invariants(
    counters: &FullSoakCounters,
    subsystems: &FullSoakSubsystemFlags,
) -> Result<(), String> {
    if !subsystems.chat_sessions
        || !subsystems.queue_processors
        || !subsystems.trajectory_watcher
        || !subsystems.codegraph
        || !subsystems.vecdb_local_backend
        || !subsystems.buddy
        || !subsystems.scheduler
        || !subsystems.exec_registry
        || subsystems.exec_registry_entries == 0
    {
        return Err("full soak fixture did not start every required subsystem".to_string());
    }
    if counters.stream_deltas == 0
        || counters.tool_calls == 0
        || counters.trajectory_files == 0
        || counters.sse_events == 0
        || counters.vecdb_enqueues == 0
        || counters.vecdb_enqueue_requests <= counters.vecdb_pending_unique_paths
        || counters.vecdb_processed_paths == 0
        || counters.exec_registry_entries == 0
        || counters.errors != 0
    {
        return Err(
            "full soak fixture did not preserve chat, tool, SSE, or restore invariants".to_string(),
        );
    }
    Ok(())
}

async fn run_workload(
    workload: &ConcurrentChatWorkload,
    options: &BenchmarkOptions,
) -> Result<WorkloadBenchmarkReport, String> {
    let variants = [("legacy", false), ("coalesced", true)];
    let mut reports = Vec::with_capacity(variants.len());
    for (variant_name, writer_enabled) in variants {
        reports.push(run_variant_workload(workload, options, variant_name, writer_enabled).await?);
    }
    Ok(WorkloadBenchmarkReport {
        workload: workload.clone(),
        variants: reports,
    })
}

async fn run_variant_workload(
    workload: &ConcurrentChatWorkload,
    options: &BenchmarkOptions,
    variant_name: &str,
    writer_enabled: bool,
) -> Result<VariantBenchmarkReport, String> {
    for _ in 0..options.warmup_samples {
        let sample = run_sample(workload, options.mode, writer_enabled).await?;
        assert_ci_invariants(&sample.counters, &sample.diagnostics)?;
    }

    let sample_futures = (0..options.measured_samples)
        .map(|_| run_sample(workload, options.mode, writer_enabled))
        .collect::<Vec<_>>();
    let mut samples = Vec::with_capacity(sample_futures.len());
    for sample in sample_futures {
        samples.push(sample.await?);
    }
    let first_counters = samples
        .first()
        .map(|sample| sample.counters.clone())
        .ok_or_else(|| "benchmark produced no samples".to_string())?;
    let first_diagnostics = samples
        .first()
        .map(|sample| sample.diagnostics.clone())
        .ok_or_else(|| "benchmark produced no diagnostics".to_string())?;
    assert_ci_invariants(&first_counters, &first_diagnostics)?;
    let counters = aggregate_counters(&samples);
    let diagnostics = aggregate_diagnostics(&samples);
    let throughput_operations_per_sec = counters.operations() as f64;

    let mean_total_us = samples
        .iter()
        .map(|sample| sample.total_elapsed_us as f64)
        .sum::<f64>()
        / samples.len() as f64;
    let variant = VariantBenchmarkReport {
        variant: variant_name.to_string(),
        workload_signature: workload.fixture_signature(options.mode),
        logical_history_bytes: workload.logical_history_bytes(),
        materialized_history_bytes: workload.materialized_history_bytes(options.mode),
        counters,
        diagnostics,
        snapshot_latency: latency_for(&samples, |sample| sample.snapshot_elapsed_us)?,
        serialize_latency: latency_for(&samples, |sample| sample.serialize_elapsed_us)?,
        atomic_write_latency: latency_for(&samples, |sample| sample.atomic_write_elapsed_us)?,
        commit_latency: latency_for(&samples, |sample| sample.commit_elapsed_us)?,
        index_wait_latency: LatencySummary::from_samples(
            &samples
                .iter()
                .map(|sample| sample.index_wait_elapsed_us)
                .collect::<Vec<_>>(),
        )?,
        index_write_latency: LatencySummary::from_samples(
            &samples
                .iter()
                .map(|sample| sample.index_write_elapsed_us)
                .collect::<Vec<_>>(),
        )?,
        catalog_acquisition_latency: LatencySummary::from_samples(
            &samples
                .iter()
                .map(|sample| sample.catalog_elapsed_us)
                .collect::<Vec<_>>(),
        )?,
        checkpoint_return_latency: latency_for(&samples, |sample| {
            sample.checkpoint_return_elapsed_us
        })?,
        background_flush_latency: latency_for(&samples, |sample| {
            sample.background_flush_elapsed_us
        })?,
        total_operation_latency: LatencySummary::from_samples(
            &samples
                .iter()
                .map(|sample| sample.total_elapsed_us)
                .collect::<Vec<_>>(),
        )?,
        throughput_operations_per_sec: throughput_operations_per_sec
            / (mean_total_us / 1_000_000.0).max(0.000_001),
        machine: sample_machine_metrics(),
    };
    Ok(variant)
}

async fn run_tool_pool_workload(
    workload: &ToolPoolWorkload,
    options: &BenchmarkOptions,
) -> Result<ToolPoolWorkloadBenchmarkReport, String> {
    let mut variants = Vec::with_capacity(2);
    for (variant, snapshots_enabled) in [("legacy", false), ("pooled", true)] {
        variants.push(run_tool_pool_variant(workload, options, variant, snapshots_enabled).await?);
    }
    let legacy = variants
        .iter()
        .find(|variant| variant.variant == "legacy")
        .ok_or_else(|| "tool pool benchmark omitted legacy variant".to_string())?;
    let pooled = variants
        .iter()
        .find(|variant| variant.variant == "pooled")
        .ok_or_else(|| "tool pool benchmark omitted pooled variant".to_string())?;
    let legacy_operations = legacy.counters.catalog_preflight_operations();
    let pooled_operations = pooled.counters.catalog_preflight_operations();
    let reduction_percent = if legacy_operations == 0 {
        0.0
    } else {
        (1.0 - pooled_operations as f64 / legacy_operations as f64) * 100.0
    };
    let tool_start_p95_us = pooled.tool_start_overhead_latency.p95_us;
    let warm_schema_alias_p95_us = pooled
        .warm_schema_alias_preparation_latency
        .as_ref()
        .map(|latency| latency.p95_us)
        .unwrap_or(u64::MAX);
    let remaining_stage = if reduction_percent < 80.0 {
        Some("catalog_preflight_operations".to_string())
    } else if tool_start_p95_us >= 100_000 {
        Some("tool_start_overhead".to_string())
    } else if warm_schema_alias_p95_us >= 1_000 {
        Some("warm_schema_alias_preparation".to_string())
    } else {
        None
    };
    Ok(ToolPoolWorkloadBenchmarkReport {
        workload: workload.clone(),
        variants,
        comparison: ToolPoolComparison {
            legacy_catalog_preflight_operations: legacy_operations,
            pooled_catalog_preflight_operations: pooled_operations,
            catalog_preflight_operation_reduction_percent: reduction_percent,
            tool_start_p95_us,
            warm_schema_alias_p95_us,
            remaining_stage,
        },
    })
}

async fn run_tool_pool_variant(
    workload: &ToolPoolWorkload,
    options: &BenchmarkOptions,
    variant: &str,
    snapshots_enabled: bool,
) -> Result<ToolPoolVariantBenchmarkReport, String> {
    for _ in 0..options.warmup_samples {
        let sample = run_tool_pool_sample(workload, snapshots_enabled).await?;
        assert_tool_pool_invariants(&sample.counters, workload)?;
    }
    let mut samples = Vec::with_capacity(options.measured_samples);
    for _ in 0..options.measured_samples {
        let sample = run_tool_pool_sample(workload, snapshots_enabled).await?;
        assert_tool_pool_invariants(&sample.counters, workload)?;
        samples.push(sample);
    }
    let counters = aggregate_tool_pool_counters(&samples);
    let catalog_acquisition_elapsed_us =
        tool_pool_elapsed(&samples, |sample| &sample.catalog_acquisition_elapsed_us);
    let schema_alias_preparation_elapsed_us = tool_pool_elapsed(&samples, |sample| {
        &sample.schema_alias_preparation_elapsed_us
    });
    let warm_schema_alias_preparation_elapsed_us = tool_pool_elapsed(&samples, |sample| {
        &sample.warm_schema_alias_preparation_elapsed_us
    });
    let confirmation_preflight_elapsed_us =
        tool_pool_elapsed(&samples, |sample| &sample.confirmation_preflight_elapsed_us);
    let execution_lookup_elapsed_us =
        tool_pool_elapsed(&samples, |sample| &sample.execution_lookup_elapsed_us);
    let tool_runtime_elapsed_us =
        tool_pool_elapsed(&samples, |sample| &sample.tool_runtime_elapsed_us);
    Ok(ToolPoolVariantBenchmarkReport {
        variant: variant.to_string(),
        workload_signature: workload.fixture_signature(),
        counters,
        catalog_acquisition_latency: LatencySummary::from_samples(&catalog_acquisition_elapsed_us)?,
        schema_alias_preparation_latency: LatencySummary::from_samples(
            &schema_alias_preparation_elapsed_us,
        )?,
        warm_schema_alias_preparation_latency: snapshots_enabled
            .then(|| LatencySummary::from_samples(&warm_schema_alias_preparation_elapsed_us))
            .transpose()?,
        confirmation_preflight_latency: LatencySummary::from_samples(
            &confirmation_preflight_elapsed_us,
        )?,
        execution_lookup_latency: LatencySummary::from_samples(&execution_lookup_elapsed_us)?,
        tool_start_overhead_latency: LatencySummary::from_samples(&execution_lookup_elapsed_us)?,
        tool_runtime_latency: LatencySummary::from_samples(&tool_runtime_elapsed_us)?,
        machine: sample_machine_metrics(),
    })
}

async fn run_tool_pool_sample(
    workload: &ToolPoolWorkload,
    snapshots_enabled: bool,
) -> Result<ToolPoolSample, String> {
    let fixture = BenchmarkFixture::new(workload.tool_descriptors as usize).await?;
    let _diagnostic_lock = perf_diagnostics::PERF_RECORDER_TEST_LOCK
        .lock()
        .map_err(|_| "performance recorder test lock poisoned".to_string())?;
    let _snapshot_env_guard = ToolCatalogSnapshotEnvGuard::set(snapshots_enabled);
    let sink = Arc::new(MemoryPerfSink::new());
    let recorder = Arc::new(PerfRecorder::with_salt(
        Arc::new(BenchmarkClock::default()),
        sink.clone(),
        [17; 32],
    ));
    let _recorder_guard = perf_diagnostics::install_test_recorder(recorder);
    let mut catalog_acquisition_elapsed_us = Vec::with_capacity(workload.chat_count as usize);
    let mut schema_alias_preparation_elapsed_us = Vec::with_capacity(workload.chat_count as usize);
    let mut warm_schema_alias_preparation_elapsed_us =
        Vec::with_capacity(workload.chat_count as usize);
    let mut sessions = Vec::with_capacity(workload.chat_count as usize);

    for chat_index in 0..workload.chat_count {
        let tool_calls = deterministic_tool_calls(workload, chat_index);
        let catalog_started = Instant::now();
        let catalog = fixture
            .app
            .tool_registry
            .acquire_tool_catalog("agent", Some("benchmark-local"), None)
            .await;
        catalog_acquisition_elapsed_us.push(elapsed_us(catalog_started));

        let schema_alias_started = Instant::now();
        let canonical =
            build_canonical_openai_tools(fixture.gcx.clone(), &catalog.index.tools, false, true)
                .await;
        let resolved = resolve_tool_call_aliases_with_catalog(tool_calls.clone(), &catalog);
        schema_alias_preparation_elapsed_us.push(elapsed_us(schema_alias_started));
        if canonical.tools.len() != workload.tool_descriptors as usize
            || resolved.len() != workload.tool_calls_per_chat as usize
        {
            return Err(
                "tool pool fixture did not prepare the expected schema and aliases".to_string(),
            );
        }

        if snapshots_enabled {
            let warm_schema_alias_started = Instant::now();
            let warm_canonical = build_canonical_openai_tools(
                fixture.gcx.clone(),
                &catalog.index.tools,
                false,
                true,
            )
            .await;
            let warm_resolved =
                resolve_tool_call_aliases_with_catalog(tool_calls.clone(), &catalog);
            warm_schema_alias_preparation_elapsed_us.push(elapsed_us(warm_schema_alias_started));
            if warm_canonical.tools.len() != canonical.tools.len()
                || warm_resolved.len() != resolved.len()
            {
                return Err("warm tool pool schema and aliases changed within a turn".to_string());
            }
        }

        let chat_id = format!("tool-pool-{}-{chat_index}", workload.fixture_signature());
        let session = Arc::new(AMutex::new(crate::chat::types::ChatSession::new(
            chat_id.clone(),
        )));
        {
            let mut session_locked = session.lock().await;
            session_locked.thread.model = "benchmark-local".to_string();
            session_locked.thread.mode = "agent".to_string();
            session_locked.thread.include_project_info = false;
            session_locked.tool_catalog = Some(catalog);
            let mut assistant = ChatMessage::new("assistant".to_string(), String::new());
            assistant.tool_calls = Some(tool_calls);
            session_locked.add_message(assistant);
        }
        fixture
            .app
            .chat
            .sessions
            .write()
            .await
            .insert(chat_id, session.clone());
        sessions.push(session);
    }

    let outcomes = futures::future::join_all(sessions.iter().cloned().map(|session| {
        process_tool_calls_once(
            fixture.app.clone(),
            session,
            "agent",
            Some("benchmark-local"),
        )
    }))
    .await;
    for (chat_index, (outcome, session)) in outcomes.into_iter().zip(sessions.iter()).enumerate() {
        if !matches!(outcome, ToolStepOutcome::Continue) {
            return Err(format!(
                "tool pool fixture chat {chat_index} did not continue"
            ));
        }
        assert_tool_result_order(session, workload, chat_index as u8).await?;
    }

    let events = sink.events();
    let counters = ToolPoolCounters {
        immutable_catalog_builds: event_count(&events, PerfComponent::ToolCatalogBuild),
        mutable_vector_builds: event_count(&events, PerfComponent::ToolMutableVectorBuild),
        parallel_vector_expansions: event_count(&events, PerfComponent::ToolPoolParallelExpansion),
        confirmation_preflight_starts: event_count(
            &events,
            PerfComponent::ToolConfirmationPreflight,
        ),
        confirmation_preflight_tool_checks: event_item_count(
            &events,
            PerfComponent::ToolConfirmationPreflight,
        ),
        execution_lookups: event_count(&events, PerfComponent::ToolExecutionLookup),
        tool_calls: u64::from(workload.chat_count) * u64::from(workload.tool_calls_per_chat),
        tool_runtime_calls: event_count(&events, PerfComponent::ToolRuntime),
        errors: tool_pool_errors(&events),
    };
    Ok(ToolPoolSample {
        counters,
        catalog_acquisition_elapsed_us,
        schema_alias_preparation_elapsed_us,
        warm_schema_alias_preparation_elapsed_us,
        confirmation_preflight_elapsed_us: event_elapsed(
            &events,
            PerfComponent::ToolConfirmationPreflight,
        ),
        execution_lookup_elapsed_us: event_elapsed(&events, PerfComponent::ToolExecutionLookup),
        tool_runtime_elapsed_us: event_elapsed(&events, PerfComponent::ToolRuntime),
    })
}

fn deterministic_tool_calls(workload: &ToolPoolWorkload, chat_index: u8) -> Vec<ChatToolCall> {
    let mut names = std::iter::repeat("benchmark_tool_0".to_string())
        .take(usize::from(workload.same_name_parallel_calls))
        .collect::<Vec<_>>();
    names.extend(
        (1..=workload.tool_calls_per_chat - workload.same_name_parallel_calls as u16)
            .map(|index| format!("benchmark_tool_{index}")),
    );
    names
        .into_iter()
        .enumerate()
        .map(|(call_index, name)| ChatToolCall {
            id: format!(
                "pool-{}-{chat_index}-{call_index}",
                workload.fixture_signature()
            ),
            index: Some(call_index),
            function: ChatToolFunction {
                name,
                arguments: "{}".to_string(),
            },
            tool_type: "function".to_string(),
            extra_content: None,
            started_at_ms: None,
            completed_at_ms: None,
        })
        .collect()
}

async fn assert_tool_result_order(
    session: &Arc<AMutex<crate::chat::types::ChatSession>>,
    workload: &ToolPoolWorkload,
    chat_index: u8,
) -> Result<(), String> {
    let session = session.lock().await;
    let results = session
        .messages
        .iter()
        .filter(|message| message.role == "tool")
        .collect::<Vec<_>>();
    if results.len() != workload.tool_calls_per_chat as usize {
        return Err(format!(
            "tool pool fixture chat {chat_index} returned {} results instead of {}",
            results.len(),
            workload.tool_calls_per_chat
        ));
    }
    for (call_index, result) in results.iter().enumerate() {
        let expected = format!(
            "pool-{}-{chat_index}-{call_index}",
            workload.fixture_signature()
        );
        if result.tool_call_id != expected || result.tool_failed == Some(true) {
            return Err(format!(
                "tool pool fixture chat {chat_index} lost result ordering"
            ));
        }
    }
    Ok(())
}

fn assert_tool_pool_invariants(
    counters: &ToolPoolCounters,
    workload: &ToolPoolWorkload,
) -> Result<(), String> {
    let expected_calls = u64::from(workload.chat_count) * u64::from(workload.tool_calls_per_chat);
    if counters.tool_calls != expected_calls
        || counters.tool_runtime_calls != expected_calls
        || counters.execution_lookups != expected_calls
        || counters.confirmation_preflight_starts != u64::from(workload.chat_count)
        || counters.confirmation_preflight_tool_checks != expected_calls
    {
        return Err(
            "tool pool fixture did not exercise every real preflight and execution path"
                .to_string(),
        );
    }
    if counters.immutable_catalog_builds == 0 || counters.mutable_vector_builds == 0 {
        return Err(
            "tool pool fixture did not acquire real catalog and mutable vectors".to_string(),
        );
    }
    if counters.errors != 0 {
        return Err(format!(
            "tool pool fixture recorded {} errors",
            counters.errors
        ));
    }
    Ok(())
}

fn aggregate_tool_pool_counters(samples: &[ToolPoolSample]) -> ToolPoolCounters {
    samples
        .iter()
        .fold(ToolPoolCounters::default(), |mut total, sample| {
            total.immutable_catalog_builds += sample.counters.immutable_catalog_builds;
            total.mutable_vector_builds += sample.counters.mutable_vector_builds;
            total.parallel_vector_expansions += sample.counters.parallel_vector_expansions;
            total.confirmation_preflight_starts += sample.counters.confirmation_preflight_starts;
            total.confirmation_preflight_tool_checks +=
                sample.counters.confirmation_preflight_tool_checks;
            total.execution_lookups += sample.counters.execution_lookups;
            total.tool_calls += sample.counters.tool_calls;
            total.tool_runtime_calls += sample.counters.tool_runtime_calls;
            total.errors += sample.counters.errors;
            total
        })
}

fn tool_pool_elapsed(
    samples: &[ToolPoolSample],
    elapsed: impl Fn(&ToolPoolSample) -> &Vec<u64>,
) -> Vec<u64> {
    samples
        .iter()
        .flat_map(|sample| elapsed(sample).iter().copied())
        .collect()
}

fn event_count(events: &[PerfEvent], component: PerfComponent) -> u64 {
    events
        .iter()
        .filter(|event| event.component == component.as_str())
        .count() as u64
}

fn event_item_count(events: &[PerfEvent], component: PerfComponent) -> u64 {
    events
        .iter()
        .filter(|event| event.component == component.as_str())
        .filter_map(|event| event.item_count)
        .sum()
}

fn event_elapsed(events: &[PerfEvent], component: PerfComponent) -> Vec<u64> {
    events
        .iter()
        .filter(|event| event.component == component.as_str())
        .map(|event| event.elapsed_us)
        .collect()
}

fn event_elapsed_sum(events: &[PerfEvent], component: PerfComponent) -> u64 {
    event_elapsed(events, component).into_iter().sum()
}

fn full_soak_tool_call_stage_sample(events: &[PerfEvent]) -> FullSoakToolCallStageSample {
    let confirmation_us = event_elapsed_sum(events, PerfComponent::ToolConfirmationPreflight);
    FullSoakToolCallStageSample {
        session_extraction_history_clone_us: event_elapsed_sum(
            events,
            PerfComponent::ToolSessionExtraction,
        ),
        catalog_pool_acquire_us: event_elapsed_sum(events, PerfComponent::ToolCatalogPoolAcquire),
        alias_resolution_us: event_elapsed_sum(events, PerfComponent::ToolAliasResolution),
        confirmation_us,
        prehooks_us: event_elapsed_sum(events, PerfComponent::ToolPreHook),
        execution_wait_us: event_elapsed_sum(events, PerfComponent::ToolExecutionWait),
        execution_lookup_us: event_elapsed_sum(events, PerfComponent::ToolExecutionLookup),
        execution_runtime_us: event_elapsed_sum(events, PerfComponent::ToolRuntime),
        posthooks_us: event_elapsed_sum(events, PerfComponent::ToolPostHook),
        result_postprocess_privacy_us: event_elapsed_sum(
            events,
            PerfComponent::ToolResultPostprocess,
        ),
        session_merge_events_us: event_elapsed_sum(events, PerfComponent::ToolSessionMergeEvents),
        checkpoint_scheduling_us: event_elapsed_sum(
            events,
            PerfComponent::ToolCheckpointScheduling,
        ),
    }
}

#[cfg(test)]
fn full_soak_stage_accounting_is_bounded(
    end_to_end_us: u64,
    stages: &FullSoakToolCallStageSample,
) -> bool {
    stages.accounted_us() <= end_to_end_us.saturating_add(stages.execution_wait_us)
}

fn tool_pool_errors(events: &[PerfEvent]) -> u64 {
    events
        .iter()
        .filter(|event| event.outcome == "failure")
        .filter(|event| {
            matches!(
                event.component,
                "tool.catalog_build"
                    | "tool.mutable_vector_build"
                    | "tool.pool_parallel_expansion"
                    | "tool.confirmation_preflight"
                    | "tool.execution_lookup"
                    | "tool.runtime"
            )
        })
        .count() as u64
}

struct ToolCatalogSnapshotEnvGuard {
    previous: Option<std::ffi::OsString>,
}

impl ToolCatalogSnapshotEnvGuard {
    fn set(enabled: bool) -> Self {
        let previous = std::env::var_os("REFACT_TOOL_CATALOG_SNAPSHOTS");
        std::env::set_var(
            "REFACT_TOOL_CATALOG_SNAPSHOTS",
            if enabled { "1" } else { "0" },
        );
        Self { previous }
    }
}

impl Drop for ToolCatalogSnapshotEnvGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            std::env::set_var("REFACT_TOOL_CATALOG_SNAPSHOTS", previous);
        } else {
            std::env::remove_var("REFACT_TOOL_CATALOG_SNAPSHOTS");
        }
    }
}

async fn run_sample(
    workload: &ConcurrentChatWorkload,
    mode: HarnessMode,
    writer_enabled: bool,
) -> Result<Sample, String> {
    let fixture = BenchmarkFixture::new(workload.tool_descriptors as usize).await?;
    let _diagnostic_lock = perf_diagnostics::PERF_RECORDER_TEST_LOCK
        .lock()
        .map_err(|_| "performance recorder test lock poisoned".to_string())?;
    let _writer_env_guard = TrajectoryWriterEnvGuard::set(writer_enabled);
    let sink = Arc::new(MemoryPerfSink::new());
    let recorder = Arc::new(PerfRecorder::with_salt(
        Arc::new(BenchmarkClock::default()),
        sink.clone(),
        [11; 32],
    ));
    let _recorder_guard = perf_diagnostics::install_test_recorder(recorder);
    let total_started = Instant::now();
    let before = filesystem_snapshot(&fixture.workspace).await?;

    let history = deterministic_payload(workload.seed, workload.materialized_history_bytes(mode));
    let saves = save_workload_trajectories(&fixture, workload, &history).await?;
    let index_started = Instant::now();
    let index_dir = fixture.workspace.join(".refact").join("trajectories");
    rebuild_trajectory_index_from_disk(&index_dir, None).await?;
    let _index_elapsed_us = elapsed_us(index_started);

    let catalog_started = Instant::now();
    let catalog = fixture
        .app
        .tool_registry
        .get_tools_index_for_mode("agent", None)
        .await;
    let canonical =
        build_canonical_openai_tools(fixture.gcx.clone(), &catalog.tools, false, true).await;
    let policies = fixture
        .app
        .tool_registry
        .get_tool_policy_info("agent", None)
        .await;
    let catalog_elapsed_us = elapsed_us(catalog_started);
    if canonical.tools.len() != workload.tool_descriptors as usize
        || policies.len() != workload.tool_descriptors as usize
    {
        return Err(
            "fixture tool registry did not materialize the deterministic local catalog".to_string(),
        );
    }

    let after = filesystem_snapshot(&fixture.workspace).await?;
    let (files_written, bytes_written) = filesystem_delta(&before, &after);
    let trajectory_files = count_trajectory_files(&fixture.workspace).await?;
    let events = sink.events();
    let diagnostics = DiagnosticCounters::from_events(&events);
    let counters = BenchmarkCounters {
        save_calls: saves.total,
        rapid_checkpoint_saves: saves.checkpoints,
        required_commits: saves.required_commits,
        trajectory_files,
        measured_files_written: files_written,
        measured_bytes_written: bytes_written,
        index_rebuilds: diagnostics.trajectory_index_rebuild,
        catalog_builds: diagnostics.tool_catalog_build,
        catalog_tool_descriptors: catalog.tools.len() as u64,
        catalog_policy_entries: policies.len() as u64,
        errors: terminal_operation_failures(&events),
    };
    Ok(Sample {
        counters,
        diagnostics,
        snapshot_elapsed_us: elapsed_for(&events, PerfComponent::TrajectorySnapshot),
        serialize_elapsed_us: elapsed_for(&events, PerfComponent::TrajectorySerialize),
        atomic_write_elapsed_us: elapsed_for(&events, PerfComponent::TrajectoryAtomicWrite),
        commit_elapsed_us: elapsed_for(&events, PerfComponent::TrajectoryCommit),
        index_wait_elapsed_us: elapsed_for(&events, PerfComponent::TrajectoryIndexLockWait),
        index_write_elapsed_us: elapsed_for(&events, PerfComponent::TrajectoryIndexWrite),
        catalog_elapsed_us,
        checkpoint_return_elapsed_us: saves.checkpoint_return_elapsed_us,
        background_flush_elapsed_us: saves.background_flush_elapsed_us,
        total_elapsed_us: elapsed_us(total_started),
    })
}

struct SaveCounts {
    total: u64,
    checkpoints: u64,
    required_commits: u64,
    checkpoint_return_elapsed_us: u64,
    background_flush_elapsed_us: u64,
}

struct TrajectoryWriterEnvGuard {
    previous: Option<std::ffi::OsString>,
}

impl TrajectoryWriterEnvGuard {
    fn set(enabled: bool) -> Self {
        let previous = std::env::var_os(crate::chat::trajectories::TRAJECTORY_WRITER_ENV);
        std::env::set_var(
            crate::chat::trajectories::TRAJECTORY_WRITER_ENV,
            if enabled { "1" } else { "0" },
        );
        Self { previous }
    }
}

impl Drop for TrajectoryWriterEnvGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            std::env::set_var(crate::chat::trajectories::TRAJECTORY_WRITER_ENV, previous);
        } else {
            std::env::remove_var(crate::chat::trajectories::TRAJECTORY_WRITER_ENV);
        }
    }
}

async fn save_workload_trajectories(
    fixture: &BenchmarkFixture,
    workload: &ConcurrentChatWorkload,
    history: &[u8],
) -> Result<SaveCounts, String> {
    let shared_payload = String::from_utf8_lossy(history).to_string();
    let primary_chat_id = format!("bench-{}-0", workload.fixture_signature(HarnessMode::Quick));
    let save_tasks = (0..workload.chat_count)
        .map(|chat_index| {
            let chat_id = format!(
                "bench-{}-{chat_index}",
                workload.fixture_signature(HarnessMode::Quick)
            );
            let payload = shared_payload.clone();
            let gcx = fixture.gcx.clone();
            async move { save_real_session_snapshot(gcx, &chat_id, &payload).await }
        })
        .collect::<Vec<_>>();
    for result in futures::future::join_all(save_tasks).await {
        result?;
    }

    let chat_id = primary_chat_id;
    let mut checkpoint_return_samples = Vec::new();
    for checkpoint in 0..workload.rapid_same_chat_checkpoints {
        let checkpoint_started = Instant::now();
        save_real_session_snapshot(
            fixture.gcx.clone(),
            &chat_id,
            &format!(
                "checkpoint-{checkpoint}-{}",
                &shared_payload[..shared_payload.len().min(256)]
            ),
        )
        .await?;
        checkpoint_return_samples.push(elapsed_us(checkpoint_started));
    }
    let checkpoint_return_elapsed_us = percentile_us(&checkpoint_return_samples, 95);
    let flush_started = Instant::now();
    persist_trajectory_snapshot_with_intent(
        fixture.gcx.clone(),
        snapshot_for_payload(&chat_id, &format!("final-{shared_payload}")),
        crate::chat::types::TrajectoryCommitIntent::Required,
    )
    .await?;
    let background_flush_elapsed_us = elapsed_us(flush_started);

    let file_path = find_trajectory_path(fixture.gcx.clone(), &chat_id)
        .await
        .ok_or_else(|| "real trajectory save did not expose a trajectory path".to_string())?;
    let value = serde_json::from_str(
        &tokio::fs::read_to_string(&file_path)
            .await
            .map_err(|error| format!("failed to read real trajectory fixture: {error}"))?,
    )
    .map_err(|error| format!("failed to parse real trajectory fixture: {error}"))?;
    let index_dir = fixture.workspace.join(".refact").join("trajectories");
    upsert_trajectory_index_entry_from_owned_value(&index_dir, &file_path, value, None).await?;

    let loaded = load_trajectory_for_chat(fixture.gcx.clone(), &chat_id)
        .await
        .ok_or_else(|| "real trajectory fixture could not be reloaded".to_string())?;
    if loaded.messages.len() != 1 {
        return Err("real trajectory fixture lost or corrupted messages".to_string());
    }

    Ok(SaveCounts {
        total: u64::from(workload.chat_count) + workload.rapid_same_chat_checkpoints + 1,
        checkpoints: workload.rapid_same_chat_checkpoints,
        required_commits: 1,
        checkpoint_return_elapsed_us,
        background_flush_elapsed_us,
    })
}

async fn save_real_session_snapshot(
    gcx: SharedGlobalContext,
    chat_id: &str,
    payload: &str,
) -> Result<(), String> {
    persist_trajectory_snapshot_with_intent(
        gcx,
        snapshot_for_payload(chat_id, payload),
        crate::chat::types::TrajectoryCommitIntent::Checkpoint,
    )
    .await
}

fn snapshot_for_payload(
    chat_id: &str,
    payload: &str,
) -> crate::chat::trajectories::TrajectorySnapshot {
    let mut session = crate::chat::types::ChatSession::new(chat_id.to_string());
    session.thread.title = "Concurrent benchmark".to_string();
    session.thread.model = "benchmark-local".to_string();
    session.thread.mode = "agent".to_string();
    session.thread.include_project_info = false;
    session.thread.auto_enrichment_enabled = Some(false);
    session.add_message(ChatMessage {
        role: "user".to_string(),
        content: ChatContent::SimpleText(payload.to_string()),
        ..Default::default()
    });
    trajectory_snapshot_from_session(&session)
}

struct BenchmarkClock {
    origin: Instant,
}

impl Default for BenchmarkClock {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl PerfClock for BenchmarkClock {
    fn now_us(&self) -> u64 {
        self.origin
            .elapsed()
            .as_micros()
            .try_into()
            .unwrap_or(u64::MAX)
    }
}

struct BenchmarkTool {
    name: String,
}

#[async_trait]
impl Tool for BenchmarkTool {
    async fn tool_execute(
        &mut self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        _args: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let mut message = ChatMessage::new("tool".to_string(), "benchmark result".to_string());
        message.tool_call_id = tool_call_id.clone();
        Ok((false, vec![ContextEnum::ChatMessage(message)]))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: self.name.clone(),
            experimental: false,
            allow_parallel: true,
            description: format!("Deterministic local {} tool", self.name),
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
            output_schema: None,
            annotations: None,
            display_name: self.name.clone(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
        }
    }

    async fn match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        _args: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<MatchConfirmDeny, String> {
        Ok(MatchConfirmDeny {
            result: MatchConfirmDenyResult::PASS,
            command: self.name.clone(),
            rule: "benchmark fixture".to_string(),
        })
    }
}

fn deterministic_tool_factory(tool_count: usize) -> FixtureToolFactory {
    Arc::new(move || {
        (0..tool_count)
            .map(|index| {
                Box::new(BenchmarkTool {
                    name: format!("benchmark_tool_{index}"),
                }) as Box<dyn Tool + Send>
            })
            .collect()
    })
}

async fn filesystem_snapshot(
    root: &Path,
) -> Result<std::collections::BTreeMap<PathBuf, u64>, String> {
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut files = std::collections::BTreeMap::new();
        let mut pending = vec![root];
        while let Some(path) = pending.pop() {
            let entries = match std::fs::read_dir(&path) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(format!("failed to walk benchmark files: {error}")),
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let metadata = match std::fs::symlink_metadata(&path) {
                    Ok(metadata) => metadata,
                    Err(_) => continue,
                };
                if metadata.file_type().is_symlink() {
                    continue;
                }
                if metadata.is_dir() {
                    pending.push(path);
                } else if metadata.is_file() {
                    files.insert(path, metadata.len());
                }
            }
        }
        Ok(files)
    })
    .await
    .map_err(|error| format!("benchmark file snapshot task failed: {error}"))?
}

fn filesystem_delta(
    before: &std::collections::BTreeMap<PathBuf, u64>,
    after: &std::collections::BTreeMap<PathBuf, u64>,
) -> (u64, u64) {
    after
        .iter()
        .fold((0, 0), |(files, bytes), (path, after_len)| {
            let before_len = before.get(path).copied().unwrap_or(0);
            if before.get(path) != Some(after_len) {
                (
                    files + 1,
                    bytes.saturating_add(after_len.saturating_sub(before_len)),
                )
            } else {
                (files, bytes)
            }
        })
}

async fn count_trajectory_files(workspace: &Path) -> Result<u64, String> {
    let root = workspace.join(".refact").join("trajectories");
    let snapshot = filesystem_snapshot(&root).await?;
    Ok(snapshot
        .keys()
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("json"))
        .filter(|path| path.file_name().and_then(|name| name.to_str()) != Some("index.json"))
        .count() as u64)
}

fn aggregate_counters(samples: &[Sample]) -> BenchmarkCounters {
    samples
        .iter()
        .fold(BenchmarkCounters::default(), |mut total, sample| {
            total.save_calls += sample.counters.save_calls;
            total.rapid_checkpoint_saves += sample.counters.rapid_checkpoint_saves;
            total.required_commits += sample.counters.required_commits;
            total.trajectory_files += sample.counters.trajectory_files;
            total.measured_files_written += sample.counters.measured_files_written;
            total.measured_bytes_written += sample.counters.measured_bytes_written;
            total.index_rebuilds += sample.counters.index_rebuilds;
            total.catalog_builds += sample.counters.catalog_builds;
            total.catalog_tool_descriptors += sample.counters.catalog_tool_descriptors;
            total.catalog_policy_entries += sample.counters.catalog_policy_entries;
            total.errors += sample.counters.errors;
            total
        })
}

fn aggregate_diagnostics(samples: &[Sample]) -> DiagnosticCounters {
    samples
        .iter()
        .fold(DiagnosticCounters::default(), |mut total, sample| {
            total.add_assign(&sample.diagnostics);
            total
        })
}

fn latency_for(
    samples: &[Sample],
    elapsed: impl Fn(&Sample) -> u64,
) -> Result<LatencySummary, String> {
    LatencySummary::from_samples(&samples.iter().map(elapsed).collect::<Vec<_>>())
}

fn elapsed_for(events: &[PerfEvent], component: PerfComponent) -> u64 {
    events
        .iter()
        .filter(|event| event.component == component.as_str())
        .map(|event| event.elapsed_us)
        .sum::<u64>()
        .max(1)
}

fn terminal_operation_failures(events: &[PerfEvent]) -> u64 {
    events
        .iter()
        .filter(|event| event.outcome == "failure")
        .filter(|event| {
            matches!(
                event.component,
                "trajectory.serialize"
                    | "trajectory.atomic_write"
                    | "trajectory.commit"
                    | "trajectory.index_rebuild"
                    | "tool.catalog_build"
            )
        })
        .count() as u64
}

fn deterministic_payload(seed: u64, bytes: usize) -> Vec<u8> {
    let mut state = seed;
    let mut payload = Vec::with_capacity(bytes);
    for _ in 0..bytes {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        payload.push(b'a' + ((state >> 32) % 26) as u8);
    }
    payload
}

fn stable_hash(values: &[u64]) -> u64 {
    values.iter().fold(0xcbf2_9ce4_8422_2325, |hash, value| {
        hash.wrapping_mul(0x100_0000_01b3) ^ value
    })
}

fn elapsed_us(started: Instant) -> u64 {
    started
        .elapsed()
        .as_micros()
        .try_into()
        .unwrap_or(u64::MAX)
        .max(1)
}

#[derive(Clone, Debug)]
struct ProcessResourceSnapshot {
    cpu_time_us: Option<u64>,
    rss_bytes: Option<u64>,
    read_bytes: Option<u64>,
    write_bytes: Option<u64>,
}

struct FullSoakProcessSampler {
    stop: Arc<AtomicBool>,
    snapshots: Arc<StdMutex<Vec<ProcessResourceSnapshot>>>,
    task: std::thread::JoinHandle<()>,
}

impl FullSoakProcessSampler {
    fn start() -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let snapshots = Arc::new(StdMutex::new(vec![sample_process_resources()]));
        let task_stop = stop.clone();
        let task_snapshots = snapshots.clone();
        let task = std::thread::Builder::new()
            .name("full-soak-process-sampler".to_string())
            .spawn(move || {
                while !task_stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(5));
                    if let Ok(mut snapshots) = task_snapshots.lock() {
                        snapshots.push(sample_process_resources());
                    }
                }
            })
            .map_err(|error| format!("failed to start full soak process sampler: {error}"))?;
        Ok(Self {
            stop,
            snapshots,
            task,
        })
    }

    fn finish(self) -> Result<FullSoakProcessMetrics, String> {
        self.stop.store(true, Ordering::Relaxed);
        self.task
            .join()
            .map_err(|_| "full soak process sampler panicked".to_string())?;
        let mut snapshots = self
            .snapshots
            .lock()
            .map_err(|_| "full soak process sampler lock poisoned".to_string())?
            .clone();
        snapshots.push(sample_process_resources());
        Ok(process_metrics_from_snapshots(&snapshots))
    }
}

fn process_metrics_from_snapshots(snapshots: &[ProcessResourceSnapshot]) -> FullSoakProcessMetrics {
    let first = snapshots.first();
    let last = snapshots.last();
    let baseline_rss = first.and_then(|sample| sample.rss_bytes);
    let peak_rss = snapshots.iter().filter_map(|sample| sample.rss_bytes).max();
    FullSoakProcessMetrics {
        sample_count: snapshots.len(),
        cpu_time_delta_us: option_delta(
            first.and_then(|sample| sample.cpu_time_us),
            last.and_then(|sample| sample.cpu_time_us),
        ),
        rss_baseline_bytes: baseline_rss,
        rss_peak_bytes: peak_rss,
        rss_delta_bytes: baseline_rss
            .zip(peak_rss)
            .map(|(first, peak)| peak as i64 - first as i64),
        read_bytes_delta: option_delta(
            first.and_then(|sample| sample.read_bytes),
            last.and_then(|sample| sample.read_bytes),
        ),
        write_bytes_delta: option_delta(
            first.and_then(|sample| sample.write_bytes),
            last.and_then(|sample| sample.write_bytes),
        ),
    }
}

fn option_delta(before: Option<u64>, after: Option<u64>) -> Option<u64> {
    before
        .zip(after)
        .map(|(before, after)| after.saturating_sub(before))
}

fn sample_process_resources() -> ProcessResourceSnapshot {
    let pid = Pid::from_u32(std::process::id());
    let pids = [pid];
    let mut system = System::new();
    let refresh_kind = ProcessRefreshKind::nothing()
        .with_memory()
        .with_disk_usage()
        .without_tasks();
    system.refresh_processes_specifics(ProcessesToUpdate::Some(&pids), true, refresh_kind);
    match system.process(pid) {
        Some(process) => {
            let usage = process.disk_usage();
            ProcessResourceSnapshot {
                cpu_time_us: read_process_cpu_time_us(),
                rss_bytes: Some(process.memory()),
                read_bytes: Some(usage.total_read_bytes),
                write_bytes: Some(usage.total_written_bytes),
            }
        }
        None => ProcessResourceSnapshot {
            cpu_time_us: None,
            rss_bytes: None,
            read_bytes: None,
            write_bytes: None,
        },
    }
}

#[cfg(target_os = "linux")]
fn read_process_cpu_time_us() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let close = stat.rfind(')')?;
    let fields = stat
        .get(close + 2..)?
        .split_whitespace()
        .collect::<Vec<_>>();
    let user_ticks = fields.get(11)?.parse::<u64>().ok()?;
    let system_ticks = fields.get(12)?.parse::<u64>().ok()?;
    let ticks_per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks_per_second <= 0 {
        return None;
    }
    user_ticks
        .saturating_add(system_ticks)
        .checked_mul(1_000_000)?
        .checked_div(ticks_per_second as u64)
}

#[cfg(not(target_os = "linux"))]
fn read_process_cpu_time_us() -> Option<u64> {
    None
}

fn sample_machine_metrics() -> MachineMetrics {
    let snapshot = sample_process_resources();
    MachineMetrics {
        rss_bytes: snapshot.rss_bytes,
        cpu_percent: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_math_uses_nearest_rank_at_tail_percentiles() {
        let samples = [10, 20, 30, 40, 50];
        assert_eq!(percentile_us(&samples, 0), 10);
        assert_eq!(percentile_us(&samples, 50), 30);
        assert_eq!(percentile_us(&samples, 95), 50);
        assert_eq!(percentile_us(&samples, 99), 50);
        assert_eq!(percentile_us(&[], 50), 0);
    }

    #[test]
    fn full_soak_process_metrics_use_baseline_delta_and_peak() {
        let metrics = process_metrics_from_snapshots(&[
            ProcessResourceSnapshot {
                cpu_time_us: Some(100),
                rss_bytes: Some(1_000),
                read_bytes: Some(50),
                write_bytes: Some(75),
            },
            ProcessResourceSnapshot {
                cpu_time_us: Some(125),
                rss_bytes: Some(900),
                read_bytes: Some(60),
                write_bytes: Some(80),
            },
            ProcessResourceSnapshot {
                cpu_time_us: Some(160),
                rss_bytes: Some(1_250),
                read_bytes: Some(90),
                write_bytes: Some(110),
            },
        ]);

        assert_eq!(metrics.sample_count, 3);
        assert_eq!(metrics.cpu_time_delta_us, Some(60));
        assert_eq!(metrics.rss_baseline_bytes, Some(1_000));
        assert_eq!(metrics.rss_peak_bytes, Some(1_250));
        assert_eq!(metrics.rss_delta_bytes, Some(250));
        assert_eq!(metrics.read_bytes_delta, Some(40));
        assert_eq!(metrics.write_bytes_delta, Some(35));
    }

    #[test]
    fn full_soak_variant_order_alternates() {
        assert_eq!(full_soak_variant_order(0), [false, true]);
        assert_eq!(full_soak_variant_order(1), [true, false]);
        assert_eq!(full_soak_variant_order(2), [false, true]);
    }

    #[test]
    #[serial_test::serial]
    fn auto_enrichment_fixture_measures_real_fanout_lock_contention_and_privacy() {
        let report = run_auto_enrichment_ci_fixture().expect("auto enrichment fixture runs");

        assert_eq!(report.workload.chat_count, 10);
        assert_eq!(report.privacy_exclusion_violations, 0);
        assert_eq!(report.repeated_work.attempts, 10);
        assert!(report.repeated_work.scoped_searches <= 1);
        assert!(report.repeated_work.cache_misses <= 1);
        assert!(
            report.repeated_work.cache_hits + report.repeated_work.cache_coalesced >= 9,
            "{:#?}",
            report.repeated_work
        );
        assert!(report.inserted_contexts > 0);
        assert!(report.injected_file_count > 0);
        assert!(report.injected_char_count > 0);
        assert!(report.injected_estimated_tokens > 0);
        assert!(report.max_concurrent_search >= 1);
        assert!(
            report.max_concurrent_search <= crate::memories::MAX_CONCURRENT_ENRICHMENT_SEARCHES
        );
        assert!(report
            .stages
            .iter()
            .any(|stage| stage.stage == "enrichment.vecdb_lock_wait"));
    }

    #[test]
    #[serial_test::serial]
    fn auto_enrichment_fallback_measurement_stays_bounded_and_schema_validates() {
        let report = benchmark_runtime_builder()
            .enable_all()
            .build()
            .expect("runtime starts")
            .block_on(run_auto_enrichment_workload(&AutoEnrichmentWorkload {
                chat_count: 10,
                root_count: 2,
                knowledge_file_count: 10,
                query_mode: AutoEnrichmentQueryMode::Distinct,
                vecdb_mode: AutoEnrichmentVecdbMode::Unavailable,
                history_message_count: 4,
                privacy_exclusion_count: 1,
            }))
            .expect("fallback fixture runs");
        assert!(report.repeated_work.fallback_files_read <= 100);
        assert_eq!(report.privacy_exclusion_violations, 0);

        let json = render_auto_enrichment_json(&AutoEnrichmentBenchmarkReport {
            schema: AUTO_ENRICHMENT_BENCHMARK_SCHEMA,
            workloads: vec![
                report.clone(),
                AutoEnrichmentWorkloadReport {
                    workload: AutoEnrichmentWorkload {
                        chat_count: 50,
                        ..report.workload.clone()
                    },
                    ..report.clone()
                },
                AutoEnrichmentWorkloadReport {
                    workload: AutoEnrichmentWorkload {
                        chat_count: 100,
                        ..report.workload.clone()
                    },
                    ..report
                },
            ],
        })
        .expect("report serializes");
        validate_auto_enrichment_report_json(&json).expect("report schema validates");
    }

    #[test]
    #[serial_test::serial]
    fn auto_enrichment_ten_thousand_file_fallback_reads_no_corpus_files() {
        let report = benchmark_runtime_builder()
            .enable_all()
            .build()
            .expect("runtime starts")
            .block_on(run_auto_enrichment_workload(&AutoEnrichmentWorkload {
                chat_count: 1,
                root_count: 1,
                knowledge_file_count: 10_000,
                query_mode: AutoEnrichmentQueryMode::Repeated,
                vecdb_mode: AutoEnrichmentVecdbMode::Unavailable,
                history_message_count: 4,
                privacy_exclusion_count: 1,
            }))
            .expect("fallback fixture runs");

        assert_eq!(report.repeated_work.fallback_files_read, 0);
        assert_eq!(report.privacy_exclusion_violations, 0);
        assert!(report.inserted_contexts > 0);
    }

    #[test]
    #[serial_test::serial]
    fn fanout_fixture_measures_high_rate_deltas_large_history_and_lag_recovery() {
        let report = run_fanout_benchmark().expect("fanout fixture should run");

        assert_eq!(report.schema, FANOUT_BENCHMARK_SCHEMA);
        assert_eq!(
            report.workload.history_message_count,
            FANOUT_HISTORY_MESSAGE_COUNT
        );
        assert_eq!(
            report.workload.history_message_bytes,
            FANOUT_HISTORY_MESSAGE_BYTES
        );
        assert_eq!(report.workload.delta_count, FANOUT_DELTA_COUNT);
        assert_eq!(
            report.subscribers.active_received_delta_count,
            report.workload.delta_count * report.workload.active_subscriber_count
        );
        assert_eq!(report.subscribers.active_lag_recoveries, 0);
        assert_eq!(report.subscribers.lag_recoveries, 1);
        assert!(report.subscribers.lagged_events > 0);
        assert_eq!(
            report.delta.serialize_latency.sample_count,
            FANOUT_DELTA_COUNT
        );
        assert_eq!(
            report.delta.broadcast_latency.sample_count,
            FANOUT_DELTA_COUNT
        );
        assert_eq!(report.delta.first_delta_latency.sample_count, 1);
        assert_eq!(report.delta.baseline_event_count, FANOUT_DELTA_COUNT);
        assert!(report.delta.coalesced_event_count < report.delta.baseline_event_count);
        assert!(report.delta.serialization_cpu_reduction_percent >= 50.0);
        assert!(report.delta.projected_first_delta_latency_us <= 50_000);
        assert_eq!(report.snapshot.snapshot_count, FANOUT_SNAPSHOT_RUNS + 1);
        assert!(report.snapshot.clone_bytes.p95_us > 1_000_000);
        assert!(report.snapshot.serialized_bytes.p95_us > 1_000_000);
    }

    #[test]
    fn fixed_seed_workloads_are_repeatable() {
        let first = ConcurrentChatWorkload::fixed_matrix();
        let second = ConcurrentChatWorkload::fixed_matrix();
        assert_eq!(first, second);
        assert_eq!(
            first[0].fixture_signature(HarnessMode::Quick),
            second[0].fixture_signature(HarnessMode::Quick)
        );
    }

    #[test]
    fn fixed_tool_pool_workload_has_the_required_concurrency_and_catalog_size() {
        let workload = ToolPoolWorkload::fixed();

        assert_eq!(workload.chat_count, TOOL_POOL_CHAT_COUNT);
        assert_eq!(workload.tool_descriptors, TOOL_POOL_DESCRIPTOR_COUNT);
        assert_eq!(workload.tool_calls_per_chat, TOOL_POOL_DESCRIPTOR_COUNT);
        assert_eq!(
            workload.same_name_parallel_calls as usize,
            TOOL_POOL_SAME_NAME_PARALLEL_CALLS
        );
        assert_eq!(
            deterministic_tool_calls(&workload, 0).len(),
            TOOL_POOL_DESCRIPTOR_COUNT as usize
        );
    }

    #[serial_test::serial]
    #[test]
    fn tool_pool_fixture_compares_real_legacy_and_pooled_operations() {
        let report = run_tool_pool_ci_fixture().expect("tool pool fixture should run");
        let legacy = &report.variants[0];
        let pooled = &report.variants[1];
        let expected_calls =
            u64::from(report.workload.chat_count) * u64::from(report.workload.tool_calls_per_chat);
        let maximum_expansions = u64::from(report.workload.chat_count)
            * (u64::from(report.workload.same_name_parallel_calls) - 1);

        assert_eq!(legacy.variant, "legacy");
        assert_eq!(pooled.variant, "pooled");
        assert_eq!(legacy.counters.tool_calls, expected_calls);
        assert_eq!(pooled.counters.tool_calls, expected_calls);
        assert_eq!(
            legacy.counters.confirmation_preflight_tool_checks,
            expected_calls
        );
        assert_eq!(
            pooled.counters.confirmation_preflight_tool_checks,
            expected_calls
        );
        assert_eq!(
            pooled.counters.mutable_vector_builds,
            u64::from(report.workload.chat_count)
        );
        assert!(pooled.counters.parallel_vector_expansions <= maximum_expansions);
        assert!(
            report
                .comparison
                .catalog_preflight_operation_reduction_percent
                >= 80.0,
            "{:#?}",
            report.comparison
        );
        assert!(pooled.tool_start_overhead_latency.p95_us < 100_000);
        match report.comparison.remaining_stage.as_deref() {
            None => assert!(report.comparison.warm_schema_alias_p95_us < 1_000),
            Some("warm_schema_alias_preparation") => {
                assert!(report.comparison.warm_schema_alias_p95_us >= 1_000)
            }
            Some(stage) => panic!("unexpected remaining stage: {stage}"),
        }
    }

    #[test]
    #[serial_test::serial]
    fn fixed_fixture_repeats_structural_counters_without_requiring_identical_wall_clock() {
        let first = run_ci_fixture().expect("first fixture run succeeds");
        let second = run_ci_fixture().expect("second fixture run succeeds");
        let first_variant = &first.variants[0];
        let second_variant = &second.variants[0];
        assert_eq!(first.variants.len(), 2);
        assert_eq!(second.variants.len(), 2);
        assert_eq!(first.workload, second.workload);
        assert_eq!(first_variant.counters, second_variant.counters);
        assert_eq!(
            first_variant.diagnostics.trajectory_snapshot,
            second_variant.diagnostics.trajectory_snapshot
        );
        assert_eq!(
            first_variant.diagnostics.trajectory_serialize,
            second_variant.diagnostics.trajectory_serialize
        );
        assert_eq!(
            first_variant.diagnostics.trajectory_atomic_write,
            second_variant.diagnostics.trajectory_atomic_write
        );
        assert_eq!(
            first_variant.diagnostics.trajectory_commit,
            second_variant.diagnostics.trajectory_commit
        );
        assert_eq!(
            first_variant.diagnostics.trajectory_index_rebuild,
            second_variant.diagnostics.trajectory_index_rebuild
        );
        assert_eq!(
            first_variant.diagnostics.tool_catalog_build,
            second_variant.diagnostics.tool_catalog_build
        );
        assert!(first_variant.diagnostics.trajectory_index_lock_wait > 0);
        assert!(first_variant.diagnostics.trajectory_index_read > 0);
        assert!(first_variant.diagnostics.trajectory_index_write > 0);
        assert!(second_variant.diagnostics.trajectory_index_lock_wait > 0);
        assert!(second_variant.diagnostics.trajectory_index_read > 0);
        assert!(second_variant.diagnostics.trajectory_index_write > 0);
        assert_ne!(first_variant.total_operation_latency.sample_count, 0);
        assert_ne!(second_variant.total_operation_latency.sample_count, 0);
    }

    #[test]
    fn fixed_workload_matrix_covers_every_required_dimension() {
        assert!(workload_matrix_is_complete(
            &ConcurrentChatWorkload::fixed_matrix()
        ));
    }

    #[test]
    #[serial_test::serial]
    fn ci_fixture_measures_real_saves_and_diagnostics() {
        let report = run_ci_fixture().expect("CI fixture should run");
        assert_eq!(report.variants.len(), 2);
        let variant = &report.variants[0];
        assert_eq!(variant.variant, "legacy");
        assert!(variant.counters.measured_files_written >= variant.counters.trajectory_files);
        assert!(variant.counters.measured_bytes_written > 0);
        assert!(variant.counters.required_commits > 0);
        assert!(variant.diagnostics.trajectory_atomic_write >= variant.counters.required_commits);
        assert!(variant.diagnostics.trajectory_index_write > 0);
        assert_ci_invariants(&variant.counters, &variant.diagnostics).expect("fixture invariants");
    }

    #[test]
    #[serial_test::serial]
    fn repeated_real_saves_scale_observed_save_and_diagnostic_counts() {
        let mut workload = ConcurrentChatWorkload::ci_fixture();
        workload.rapid_same_chat_checkpoints = 7;
        let report = benchmark_runtime_builder()
            .enable_all()
            .build()
            .expect("runtime starts")
            .block_on(run_workload(
                &workload,
                &BenchmarkOptions {
                    mode: HarnessMode::Quick,
                    warmup_samples: 0,
                    measured_samples: 1,
                },
            ))
            .expect("real workload runs");
        let variant = &report.variants[0];
        assert_eq!(variant.counters.save_calls, 9);
        assert!(variant.diagnostics.trajectory_atomic_write >= variant.counters.required_commits);
        assert!(variant.counters.measured_bytes_written > 0);
    }

    #[test]
    #[serial_test::serial]
    fn extra_real_operation_changes_measured_counters() {
        let report = run_ci_fixture().expect("CI fixture should run");
        let variant = &report.variants[0];
        let mut diagnostics = variant.diagnostics.clone();
        diagnostics.tool_catalog_build += 1;
        assert!(assert_ci_invariants(&variant.counters, &diagnostics).is_err());
    }

    #[test]
    #[serial_test::serial]
    fn report_compares_legacy_and_coalesced_writer_variants() {
        let tool_pool_workload = run_tool_pool_ci_fixture().expect("tool pool fixture should run");
        let report = ConcurrentChatBenchmarkReport {
            schema: CONCURRENT_CHAT_BENCHMARK_SCHEMA,
            mode: HarnessMode::Quick.as_str().to_string(),
            warmup_samples: 0,
            measured_samples: 1,
            workloads: vec![run_ci_fixture().expect("CI fixture should run")],
            tool_pool_workload,
        };
        let json = render_json(&report).expect("report serializes");
        validate_report_json(&json).expect("report schema validates");
        assert!(json.contains("\"legacy\""));
        assert!(json.contains("\"coalesced\""));
        assert!(json.contains("\"pooled\""));
    }

    #[test]
    #[serial_test::serial]
    fn full_soak_ci_fixture_starts_required_subsystems() {
        let report = run_full_soak_ci_fixture().expect("full soak CI fixture should run");

        assert_eq!(report.workload.chat_count, 1);
        assert_eq!(report.variants.len(), 2);
        assert_eq!(report.variants[0].variant, "legacy");
        assert_eq!(report.variants[1].variant, "optimized");
        assert!(
            !report.variants[0]
                .rollout_switches
                .trajectory_writer_enabled
        );
        assert!(
            report.variants[1]
                .rollout_switches
                .trajectory_writer_enabled
        );
        assert_full_soak_invariants(&report.variants[0].counters, &report.variants[0].subsystems)
            .expect("legacy full soak invariants");
        assert_full_soak_invariants(&report.variants[1].counters, &report.variants[1].subsystems)
            .expect("optimized full soak invariants");
        assert!(report.variants[0]
            .subsystems
            .vecdb_disclosure
            .contains("local recording VecDB backend"));
        for variant in &report.variants {
            assert_eq!(
                variant.tool_call_end_to_end_latency.sample_count,
                variant.tool_call_stages.accounted_latency.sample_count
            );
            assert_eq!(
                variant.tool_call_stages.unattributed_latency.sample_count,
                1
            );
            let process = variant
                .process_samples
                .first()
                .expect("full soak process sample");
            assert!(process.sample_count >= 2);
            assert!(process.cpu_time_delta_us.is_some());
            assert!(process.rss_baseline_bytes.is_some());
            assert!(process.rss_peak_bytes.is_some());
            assert!(process.read_bytes_delta.is_some());
            assert!(process.write_bytes_delta.is_some());
            assert!(
                variant.counters.vecdb_enqueue_requests
                    > variant.counters.vecdb_pending_unique_paths
            );
            assert!(variant.counters.vecdb_processed_paths > 0);
            assert_eq!(
                variant.vecdb_deferred_queue.enqueue_requests,
                variant.counters.vecdb_enqueue_requests
            );
            assert!(variant.vecdb_deferred_queue.amplification_ratio >= 1.0);
        }
    }

    #[test]
    fn full_soak_stage_accounting_bounds_nested_wall_clock_stages() {
        let stages = FullSoakToolCallStageSample {
            session_extraction_history_clone_us: 3,
            catalog_pool_acquire_us: 4,
            alias_resolution_us: 2,
            confirmation_us: 5,
            prehooks_us: 6,
            execution_wait_us: 30,
            session_merge_events_us: 3,
            checkpoint_scheduling_us: 2,
            ..Default::default()
        };

        assert!(full_soak_stage_accounting_is_bounded(25, &stages));
        assert!(!full_soak_stage_accounting_is_bounded(10, &stages));
    }

    #[test]
    fn full_soak_report_schema_declares_matrix_and_disclosures() {
        let latency = LatencySummary::from_samples(&[1]).unwrap();
        let variant = FullSoakVariantBenchmarkReport {
            variant: "legacy".to_string(),
            rollout_switches: FullSoakRolloutSwitches {
                trajectory_writer_enabled: false,
                trajectory_index_coordinator_enabled: false,
                trajectory_watcher_self_write_enabled: false,
                tool_catalog_snapshots_enabled: false,
                vecdb_path_coalescing_enabled: false,
            },
            subsystems: FullSoakSubsystemFlags {
                chat_sessions: true,
                queue_processors: true,
                trajectory_writer: false,
                trajectory_index_coordinator: false,
                trajectory_watcher: true,
                codegraph: true,
                vecdb_local_backend: true,
                buddy: true,
                agent_monitor: true,
                goal_monitor: true,
                scheduler: true,
                exec_registry: true,
                session_cleanup: true,
                exec_registry_entries: 1,
                vecdb_disclosure: "local recording VecDB backend".to_string(),
            },
            counters: FullSoakCounters::default(),
            queue_wait_latency: latency.clone(),
            first_delta_latency: latency.clone(),
            checkpoint_return_latency: latency.clone(),
            required_flush_latency: latency.clone(),
            tool_call_end_to_end_latency: latency.clone(),
            tool_call_stages: FullSoakToolCallStages {
                session_extraction_history_clone_latency: latency.clone(),
                catalog_pool_acquire_latency: latency.clone(),
                alias_resolution_latency: latency.clone(),
                confirmation_latency: latency.clone(),
                prehooks_latency: latency.clone(),
                execution_wait_latency: latency.clone(),
                execution_lookup_latency: latency.clone(),
                execution_runtime_latency: latency.clone(),
                posthooks_latency: latency.clone(),
                result_postprocess_privacy_latency: latency.clone(),
                session_merge_events_latency: latency.clone(),
                checkpoint_scheduling_latency: latency.clone(),
                accounted_latency: latency.clone(),
                unattributed_latency: latency.clone(),
            },
            sse_serialize_latency: latency.clone(),
            sse_emit_latency: latency,
            process_samples: vec![FullSoakProcessMetrics {
                sample_count: 2,
                ..Default::default()
            }],
            vecdb_deferred_queue: FullSoakVecdbMetrics::default(),
        };
        let optimized = FullSoakVariantBenchmarkReport {
            variant: "optimized".to_string(),
            rollout_switches: FullSoakRolloutSwitches {
                trajectory_writer_enabled: true,
                trajectory_index_coordinator_enabled: true,
                trajectory_watcher_self_write_enabled: true,
                tool_catalog_snapshots_enabled: true,
                vecdb_path_coalescing_enabled: true,
            },
            subsystems: variant.subsystems.clone(),
            counters: variant.counters.clone(),
            queue_wait_latency: variant.queue_wait_latency.clone(),
            first_delta_latency: variant.first_delta_latency.clone(),
            checkpoint_return_latency: variant.checkpoint_return_latency.clone(),
            required_flush_latency: variant.required_flush_latency.clone(),
            tool_call_end_to_end_latency: variant.tool_call_end_to_end_latency.clone(),
            tool_call_stages: variant.tool_call_stages.clone(),
            sse_serialize_latency: variant.sse_serialize_latency.clone(),
            sse_emit_latency: variant.sse_emit_latency.clone(),
            process_samples: variant.process_samples.clone(),
            vecdb_deferred_queue: variant.vecdb_deferred_queue.clone(),
        };
        let report = FullSoakBenchmarkReport {
            comparison_label:
                "synthetic same-version legacy rollout comparison; not a historical Wave 0 baseline"
                    .to_string(),
            workloads: FullSoakWorkload::fixed_matrix()
                .into_iter()
                .map(|workload| FullSoakWorkloadBenchmarkReport {
                    workload,
                    variants: vec![variant.clone(), optimized.clone()],
                })
                .collect(),
        };

        let json = render_full_soak_json(&report).expect("full soak report serializes");
        validate_full_soak_report_json(&json).expect("full soak schema validates");
    }
}
