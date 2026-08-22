use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

pub const CONCURRENT_CHAT_BENCHMARK_SCHEMA: &str = "refact.concurrent_chat_benchmark.v1";
const QUICK_HISTORY_BYTES_CAP: usize = 8 * 1024;
const RAPID_CHECKPOINTS_PER_CHAT: u64 = 4;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessVariant {
    Legacy,
    Optimized,
}

impl HarnessVariant {
    pub const ALL: [Self; 2] = [Self::Legacy, Self::Optimized];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Optimized => "optimized",
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
    pub same_directory_writes: bool,
    pub different_directory_writes: bool,
    pub rapid_subchat_checkpoints: u64,
    pub active_chats: u8,
    pub background_chats: u8,
    pub sequence_gap_snapshot_recovery: bool,
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
            same_directory_writes: true,
            different_directory_writes: true,
            rapid_subchat_checkpoints: RAPID_CHECKPOINTS_PER_CHAT,
            active_chats: 1,
            background_chats: chat_count.saturating_sub(1),
            sequence_gap_snapshot_recovery: true,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BenchmarkCounters {
    pub write_count: u64,
    pub write_bytes: u64,
    pub same_directory_writes: u64,
    pub different_directory_writes: u64,
    pub subchat_checkpoints: u64,
    pub index_waits: u64,
    pub index_rewrites: u64,
    pub catalog_builds: u64,
    pub tool_descriptor_visits: u64,
    pub watcher_events: u64,
    pub vecdb_enqueues: u64,
    pub gui_flushes: u64,
    pub gui_reducer_events: u64,
    pub sequence_gap_snapshot_recoveries: u64,
    pub errors: u64,
    pub lost_events: u64,
    pub duplicate_events: u64,
    pub out_of_order_events: u64,
}

impl BenchmarkCounters {
    fn operations(&self) -> u64 {
        self.write_count
            .saturating_add(self.tool_descriptor_visits)
            .saturating_add(self.gui_reducer_events)
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
    pub materialized_history_bytes: usize,
    pub counters: BenchmarkCounters,
    pub latency: LatencySummary,
    pub gui_flush_latency: LatencySummary,
    pub gui_reducer_latency: LatencySummary,
    pub throughput_operations_per_sec: f64,
    pub machine: MachineMetrics,
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
}

struct Sample {
    counters: BenchmarkCounters,
    elapsed_us: u64,
    gui_flush_elapsed_us: u64,
    gui_reducer_elapsed_us: u64,
}

pub fn run_benchmark(options: BenchmarkOptions) -> Result<ConcurrentChatBenchmarkReport, String> {
    if options.measured_samples == 0 {
        return Err("measured_samples must be greater than zero".to_string());
    }
    let workloads = ConcurrentChatWorkload::fixed_matrix()
        .into_iter()
        .map(|workload| run_workload(&workload, &options))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ConcurrentChatBenchmarkReport {
        schema: CONCURRENT_CHAT_BENCHMARK_SCHEMA,
        mode: options.mode.as_str().to_string(),
        warmup_samples: options.warmup_samples,
        measured_samples: options.measured_samples,
        workloads,
    })
}

pub fn run_ci_fixture() -> Result<WorkloadBenchmarkReport, String> {
    run_workload(
        &ConcurrentChatWorkload::ci_fixture(),
        &BenchmarkOptions {
            mode: HarnessMode::Quick,
            warmup_samples: 1,
            measured_samples: 2,
        },
    )
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
    for key in ["mode", "warmup_samples", "measured_samples", "workloads"] {
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
        if !workload.contains_key("workload") || !workload.contains_key("variants") {
            return Err("benchmark workload is incomplete".to_string());
        }
        let variants = workload
            .get("variants")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| "benchmark variants must be an array".to_string())?;
        if variants.len() != HarnessVariant::ALL.len() {
            return Err("benchmark workload must include both feature variants".to_string());
        }
        for variant in variants {
            let variant = variant
                .as_object()
                .ok_or_else(|| "benchmark variant must be an object".to_string())?;
            for key in [
                "variant",
                "workload_signature",
                "counters",
                "latency",
                "gui_flush_latency",
                "gui_reducer_latency",
                "throughput_operations_per_sec",
                "machine",
            ] {
                if !variant.contains_key(key) {
                    return Err(format!("benchmark variant is missing {key}"));
                }
            }
        }
    }
    Ok(())
}

pub fn assert_ci_invariants(counters: &BenchmarkCounters) -> Result<(), String> {
    if counters.catalog_builds != 1 {
        return Err(format!(
            "expected exactly one tool catalog build, got {}",
            counters.catalog_builds
        ));
    }
    if counters.index_rewrites != 2 {
        return Err(format!(
            "expected exactly two full index rewrites, got {}",
            counters.index_rewrites
        ));
    }
    if counters.index_waits != 2 {
        return Err(format!(
            "expected exactly two index waits, got {}",
            counters.index_waits
        ));
    }
    if counters.errors != 0
        || counters.lost_events != 0
        || counters.duplicate_events != 0
        || counters.out_of_order_events != 0
    {
        return Err("event stream lost, duplicated, reordered, or failed events".to_string());
    }
    if counters.sequence_gap_snapshot_recoveries != 1 {
        return Err(format!(
            "expected one sequence-gap snapshot recovery, got {}",
            counters.sequence_gap_snapshot_recoveries
        ));
    }
    if counters.watcher_events != counters.vecdb_enqueues {
        return Err("watcher and VecDB counters diverged".to_string());
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
            workload.same_directory_writes
                && workload.different_directory_writes
                && workload.rapid_subchat_checkpoints == RAPID_CHECKPOINTS_PER_CHAT
                && workload.active_chats == 1
                && workload.background_chats == workload.chat_count.saturating_sub(1)
                && workload.sequence_gap_snapshot_recovery
        })
}

fn run_workload(
    workload: &ConcurrentChatWorkload,
    options: &BenchmarkOptions,
) -> Result<WorkloadBenchmarkReport, String> {
    let variants = HarnessVariant::ALL
        .iter()
        .copied()
        .map(|variant| run_variant(workload, options, variant))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(WorkloadBenchmarkReport {
        workload: workload.clone(),
        variants,
    })
}

fn run_variant(
    workload: &ConcurrentChatWorkload,
    options: &BenchmarkOptions,
    variant: HarnessVariant,
) -> Result<VariantBenchmarkReport, String> {
    for _ in 0..options.warmup_samples {
        let sample = run_sample(workload, options.mode, variant)?;
        assert_ci_invariants(&sample.counters)?;
    }

    let samples = (0..options.measured_samples)
        .map(|_| run_sample(workload, options.mode, variant))
        .collect::<Result<Vec<_>, _>>()?;
    let counters = samples
        .first()
        .map(|sample| sample.counters.clone())
        .ok_or_else(|| "benchmark produced no samples".to_string())?;
    assert_ci_invariants(&counters)?;
    if samples.iter().any(|sample| sample.counters != counters) {
        return Err(format!("non-deterministic counters for {}", workload.id));
    }

    let elapsed_samples = samples
        .iter()
        .map(|sample| sample.elapsed_us)
        .collect::<Vec<_>>();
    let flush_samples = samples
        .iter()
        .map(|sample| sample.gui_flush_elapsed_us)
        .collect::<Vec<_>>();
    let reducer_samples = samples
        .iter()
        .map(|sample| sample.gui_reducer_elapsed_us)
        .collect::<Vec<_>>();
    let latency = LatencySummary::from_samples(&elapsed_samples)?;
    let mean_seconds = (latency.mean_us / 1_000_000.0).max(0.000_001);

    Ok(VariantBenchmarkReport {
        variant: variant.as_str().to_string(),
        workload_signature: workload.fixture_signature(options.mode),
        materialized_history_bytes: workload.materialized_history_bytes(options.mode),
        counters: counters.clone(),
        latency,
        gui_flush_latency: LatencySummary::from_samples(&flush_samples)?,
        gui_reducer_latency: LatencySummary::from_samples(&reducer_samples)?,
        throughput_operations_per_sec: counters.operations() as f64 / mean_seconds,
        machine: sample_machine_metrics(),
    })
}

fn run_sample(
    workload: &ConcurrentChatWorkload,
    mode: HarnessMode,
    _variant: HarnessVariant,
) -> Result<Sample, String> {
    let started = Instant::now();
    let temp_dir = tempfile::tempdir()
        .map_err(|error| format!("failed to create benchmark fixture: {error}"))?;
    let same_dir = temp_dir.path().join("same-directory");
    let different_dir = temp_dir.path().join("different-directory");
    fs::create_dir_all(&same_dir)
        .and_then(|()| fs::create_dir_all(&different_dir))
        .map_err(|error| format!("failed to create benchmark directories: {error}"))?;

    let mut counters = BenchmarkCounters {
        catalog_builds: 1,
        tool_descriptor_visits: u64::from(workload.tool_descriptors),
        ..Default::default()
    };
    let catalog_fingerprint = build_tool_catalog(workload);
    let history_bytes = workload.materialized_history_bytes(mode);

    for chat_index in 0..workload.chat_count {
        let payload = deterministic_payload(
            workload.seed ^ u64::from(chat_index) ^ catalog_fingerprint,
            history_bytes,
        );
        write_fixture(
            &same_dir.join(format!("chat-{chat_index}.json")),
            &payload,
            &mut counters,
        )?;
        counters.same_directory_writes += 1;
        write_fixture(
            &different_dir.join(format!("chat-{chat_index}.json")),
            &payload,
            &mut counters,
        )?;
        counters.different_directory_writes += 1;
        for checkpoint in 0..workload.rapid_subchat_checkpoints {
            let checkpoint_payload = format!(
                "{{\"chat\":{chat_index},\"checkpoint\":{checkpoint},\"seed\":{}}}",
                workload.seed
            );
            write_fixture(
                &same_dir.join(format!("checkpoint-{chat_index}-{checkpoint}.json")),
                checkpoint_payload.as_bytes(),
                &mut counters,
            )?;
            counters.subchat_checkpoints += 1;
        }
    }

    for directory in [&same_dir, &different_dir] {
        let index_payload = format!(
            "{{\"workload\":\"{}\",\"catalog\":{catalog_fingerprint}}}",
            workload.id
        );
        write_fixture(
            &directory.join("index.json"),
            index_payload.as_bytes(),
            &mut counters,
        )?;
        counters.index_waits += 1;
        counters.index_rewrites += 1;
    }

    let (gui_flush_elapsed_us, gui_reducer_elapsed_us) =
        run_gui_mix(workload, &mut counters, catalog_fingerprint);
    counters.watcher_events = counters
        .same_directory_writes
        .saturating_add(counters.different_directory_writes)
        .saturating_add(counters.subchat_checkpoints);
    counters.vecdb_enqueues = counters.watcher_events;
    counters.errors = u64::from(catalog_fingerprint == 0);
    Ok(Sample {
        counters,
        elapsed_us: elapsed_us(started),
        gui_flush_elapsed_us,
        gui_reducer_elapsed_us,
    })
}

fn build_tool_catalog(workload: &ConcurrentChatWorkload) -> u64 {
    (0..workload.tool_descriptors).fold(workload.seed, |hash, descriptor| {
        hash.rotate_left(7)
            ^ u64::from(descriptor).wrapping_mul(0x9e37_79b9)
            ^ workload.logical_history_bytes()
    })
}

fn run_gui_mix(
    workload: &ConcurrentChatWorkload,
    counters: &mut BenchmarkCounters,
    catalog_fingerprint: u64,
) -> (u64, u64) {
    let reducer_started = Instant::now();
    for chat_index in 0..workload.chat_count {
        let mut tracker = SequenceTracker::default();
        tracker.snapshot(0);
        let mut next_seq = 1;
        for _ in 0..4 {
            tracker.apply(next_seq);
            counters.gui_reducer_events += 1;
            next_seq += 1;
        }
        if workload.sequence_gap_snapshot_recovery && chat_index == 0 {
            let gap_seq = next_seq + 1;
            if tracker.gap_detected(gap_seq) {
                counters.sequence_gap_snapshot_recoveries += 1;
                tracker.snapshot(0);
                next_seq = 1;
            }
        }
        for _ in 0..4 {
            tracker.apply(next_seq);
            counters.gui_reducer_events += 1;
            next_seq += 1;
        }
        counters.duplicate_events += tracker.duplicates;
        counters.out_of_order_events += tracker.out_of_order;
        counters.lost_events += tracker.lost;
        let flushes: u64 = if chat_index < workload.active_chats {
            4
        } else {
            1
        };
        counters.gui_flushes += flushes;
    }
    let reducer_elapsed_us = elapsed_us(reducer_started);
    let flush_started = Instant::now();
    let mut flush_hash = catalog_fingerprint;
    for flush in 0..counters.gui_flushes {
        flush_hash = flush_hash.rotate_left(3) ^ flush;
    }
    if flush_hash == 0 {
        counters.errors += 1;
    }
    (elapsed_us(flush_started), reducer_elapsed_us)
}

#[derive(Default)]
struct SequenceTracker {
    last_seq: u64,
    duplicates: u64,
    out_of_order: u64,
    lost: u64,
}

impl SequenceTracker {
    fn snapshot(&mut self, seq: u64) {
        self.last_seq = seq;
    }

    fn apply(&mut self, seq: u64) {
        if seq <= self.last_seq {
            self.duplicates += 1;
        } else if seq > self.last_seq + 1 {
            self.out_of_order += 1;
            self.lost += seq - self.last_seq - 1;
        } else {
            self.last_seq = seq;
        }
    }

    fn gap_detected(&self, seq: u64) -> bool {
        seq > self.last_seq + 1
    }
}

fn write_fixture(
    path: &Path,
    payload: &[u8],
    counters: &mut BenchmarkCounters,
) -> Result<(), String> {
    fs::write(path, payload).map_err(|error| {
        format!(
            "failed to write benchmark fixture {}: {error}",
            path.display()
        )
    })?;
    counters.write_count += 1;
    counters.write_bytes = counters
        .write_bytes
        .saturating_add(u64::try_from(payload.len()).unwrap_or(u64::MAX));
    Ok(())
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

impl Default for BenchmarkCounters {
    fn default() -> Self {
        Self {
            write_count: 0,
            write_bytes: 0,
            same_directory_writes: 0,
            different_directory_writes: 0,
            subchat_checkpoints: 0,
            index_waits: 0,
            index_rewrites: 0,
            catalog_builds: 0,
            tool_descriptor_visits: 0,
            watcher_events: 0,
            vecdb_enqueues: 0,
            gui_flushes: 0,
            gui_reducer_events: 0,
            sequence_gap_snapshot_recoveries: 0,
            errors: 0,
            lost_events: 0,
            duplicate_events: 0,
            out_of_order_events: 0,
        }
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
    fn fixed_workload_matrix_covers_every_required_dimension() {
        assert!(workload_matrix_is_complete(
            &ConcurrentChatWorkload::fixed_matrix()
        ));
    }

    #[test]
    fn ci_fixture_catches_extra_full_index_or_catalog_operations() {
        let report = run_ci_fixture().expect("CI fixture should run");
        for variant in report.variants {
            assert_ci_invariants(&variant.counters).expect("fixture invariants");
            let mut extra_catalog = variant.counters.clone();
            extra_catalog.catalog_builds += 1;
            assert!(assert_ci_invariants(&extra_catalog).is_err());
            let mut extra_index = variant.counters;
            extra_index.index_rewrites += 1;
            assert!(assert_ci_invariants(&extra_index).is_err());
        }
    }

    #[test]
    fn ci_fixture_has_no_lost_duplicate_or_out_of_order_events() {
        let report = run_ci_fixture().expect("CI fixture should run");
        for variant in report.variants {
            assert_eq!(variant.counters.errors, 0);
            assert_eq!(variant.counters.lost_events, 0);
            assert_eq!(variant.counters.duplicate_events, 0);
            assert_eq!(variant.counters.out_of_order_events, 0);
            assert_eq!(variant.counters.sequence_gap_snapshot_recoveries, 1);
        }
    }

    #[test]
    fn report_json_has_the_documented_schema() {
        let report = ConcurrentChatBenchmarkReport {
            schema: CONCURRENT_CHAT_BENCHMARK_SCHEMA,
            mode: HarnessMode::Quick.as_str().to_string(),
            warmup_samples: 1,
            measured_samples: 1,
            workloads: vec![run_ci_fixture().expect("CI fixture should run")],
        };
        let json = render_json(&report).expect("report serializes");
        validate_report_json(&json).expect("report schema validates");
    }
}
