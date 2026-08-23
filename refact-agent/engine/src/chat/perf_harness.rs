use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tokio::runtime::Builder;
use tokio::sync::Mutex as AMutex;

use crate::app_state::{AppState, AppToolRegistry, FixtureToolFactory};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ChatToolCall, ChatToolFunction, ContextEnum};
use crate::chat::tools::{
    process_tool_calls_once, resolve_tool_call_aliases_with_catalog, ToolStepOutcome,
};
use crate::chat::perf_diagnostics::{
    self, MemoryPerfSink, PerfClock, PerfComponent, PerfEvent, PerfRecorder,
};
use crate::chat::prepare::build_canonical_openai_tools;
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
const QUICK_HISTORY_BYTES_CAP: usize = 8 * 1024;
const RAPID_CHECKPOINTS_PER_CHAT: u64 = 4;
pub const TURN_MEMORY_FLEET_CHAT_COUNTS: [usize; 2] = [10, 100];

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
}

impl HarnessMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Soak => "soak",
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
            HarnessMode::Soak => {
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
    if options.measured_samples == 0 {
        return Err("measured_samples must be greater than zero".to_string());
    }
    Builder::new_multi_thread()
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
    Builder::new_multi_thread()
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
    Builder::new_multi_thread()
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

pub fn render_json(report: &ConcurrentChatBenchmarkReport) -> Result<String, String> {
    serde_json::to_string_pretty(report)
        .map_err(|error| format!("failed to serialize benchmark report: {error}"))
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

fn sample_machine_metrics() -> MachineMetrics {
    let pid = Pid::from_u32(std::process::id());
    let pids = [pid];
    let mut system = System::new();
    let refresh_kind = ProcessRefreshKind::nothing()
        .with_memory()
        .with_cpu()
        .without_tasks();
    system.refresh_processes_specifics(ProcessesToUpdate::Some(&pids), true, refresh_kind);
    match system.process(pid) {
        Some(process) => MachineMetrics {
            rss_bytes: Some(process.memory()),
            cpu_percent: Some(process.cpu_usage()),
        },
        None => MachineMetrics {
            rss_bytes: None,
            cpu_percent: None,
        },
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
    fn fixed_fixture_repeats_structural_counters_without_requiring_identical_wall_clock() {
        let first = run_ci_fixture().expect("first fixture run succeeds");
        let second = run_ci_fixture().expect("second fixture run succeeds");
        let first_variant = &first.variants[0];
        let second_variant = &second.variants[0];
        assert_eq!(first.variants.len(), 2);
        assert_eq!(second.variants.len(), 2);
        assert_eq!(first.workload, second.workload);
        assert_eq!(first_variant.counters, second_variant.counters);
        assert_eq!(first_variant.diagnostics, second_variant.diagnostics);
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
    fn repeated_real_saves_scale_observed_save_and_diagnostic_counts() {
        let mut workload = ConcurrentChatWorkload::ci_fixture();
        workload.rapid_same_chat_checkpoints = 7;
        let report = Builder::new_multi_thread()
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
    fn extra_real_operation_changes_measured_counters() {
        let report = run_ci_fixture().expect("CI fixture should run");
        let variant = &report.variants[0];
        let mut diagnostics = variant.diagnostics.clone();
        diagnostics.tool_catalog_build += 1;
        assert!(assert_ci_invariants(&variant.counters, &diagnostics).is_err());
    }

    #[test]
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
}
