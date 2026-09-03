use std::collections::{HashMap, HashSet};
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;

use crate::chat::trajectories::{
    TrajectoryData, TrajectoryListCandidate, TrajectoryMeta, calculate_line_changes_from_messages,
    calculate_task_progress_from_messages, calculate_token_totals_from_messages,
    trajectory_list_data_is_displayable_chat, trajectory_meta_title,
};
use crate::chat::types::{TrajectorySourceIdentity, WorktreeMeta};
use crate::chat::perf_diagnostics::{self, PerfComponent, PerfOutcome};

pub const TRAJECTORY_INDEX_SCHEMA_VERSION: u32 = 1;
pub const TRAJECTORY_INDEX_FILE: &str = "index.json";
pub const TRAJECTORY_INDEX_COORDINATOR_ENV: &str = "REFACT_TRAJECTORY_INDEX_COORDINATOR";
pub const TRAJECTORY_INDEX_LOCK_ORDER: &str =
    "release_global_and_session_locks_before_trajectory_index_io";

const MAX_CACHED_TRAJECTORY_DIRECTORIES: usize = 1024;

static TRAJECTORY_INDEX_CACHE_ACCESS_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_trajectory_index_cache_access_stamp() -> u64 {
    TRAJECTORY_INDEX_CACHE_ACCESS_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum TrajectoryIndexListingCaller {
    ChatApi,
    Sidebar,
    ChatPathLookup,
    StaleEntryCleanup,
    AllTrajectoriesMetadata,
    TaskTrajectoryApi,
    TaskPlannerIdAllocation,
    TaskAgentMonitor,
}

impl TrajectoryIndexListingCaller {
    const ALL: [Self; 8] = [
        Self::ChatApi,
        Self::Sidebar,
        Self::ChatPathLookup,
        Self::StaleEntryCleanup,
        Self::AllTrajectoriesMetadata,
        Self::TaskTrajectoryApi,
        Self::TaskPlannerIdAllocation,
        Self::TaskAgentMonitor,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ChatApi => "chat_api",
            Self::Sidebar => "sidebar",
            Self::ChatPathLookup => "chat_path_lookup",
            Self::StaleEntryCleanup => "stale_entry_cleanup",
            Self::AllTrajectoriesMetadata => "all_trajectories_metadata",
            Self::TaskTrajectoryApi => "task_trajectory_api",
            Self::TaskPlannerIdAllocation => "task_planner_id_allocation",
            Self::TaskAgentMonitor => "task_agent_monitor",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrajectoryIndexListingCounters {
    pub calls_by_caller: Vec<(&'static str, u64)>,
    pub cache_hits: u64,
    pub index_reads: u64,
    pub directory_scans: u64,
}

struct TrajectoryIndexListingCounterState {
    calls_by_caller: [AtomicU64; TrajectoryIndexListingCaller::ALL.len()],
    cache_hits: AtomicU64,
    index_reads: AtomicU64,
    directory_scans: AtomicU64,
}

impl TrajectoryIndexListingCounterState {
    fn new() -> Self {
        Self {
            calls_by_caller: std::array::from_fn(|_| AtomicU64::new(0)),
            cache_hits: AtomicU64::new(0),
            index_reads: AtomicU64::new(0),
            directory_scans: AtomicU64::new(0),
        }
    }

    fn record_listing_caller(&self, caller: TrajectoryIndexListingCaller) {
        self.calls_by_caller[caller as usize].fetch_add(1, Ordering::Relaxed);
    }

    fn record_listing_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    fn record_index_read(&self) {
        self.index_reads.fetch_add(1, Ordering::Relaxed);
    }

    fn record_directory_scan(&self) {
        self.directory_scans.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> TrajectoryIndexListingCounters {
        TrajectoryIndexListingCounters {
            calls_by_caller: TrajectoryIndexListingCaller::ALL
                .iter()
                .map(|caller| {
                    (
                        caller.as_str(),
                        self.calls_by_caller[*caller as usize].load(Ordering::Relaxed),
                    )
                })
                .collect(),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            index_reads: self.index_reads.load(Ordering::Relaxed),
            directory_scans: self.directory_scans.load(Ordering::Relaxed),
        }
    }

    #[cfg(test)]
    fn reset(&self) {
        for counter in &self.calls_by_caller {
            counter.store(0, Ordering::Relaxed);
        }
        self.cache_hits.store(0, Ordering::Relaxed);
        self.index_reads.store(0, Ordering::Relaxed);
        self.directory_scans.store(0, Ordering::Relaxed);
    }
}

static TRAJECTORY_INDEX_LISTING_COUNTERS: std::sync::OnceLock<TrajectoryIndexListingCounterState> =
    std::sync::OnceLock::new();

#[derive(Clone)]
enum TrajectoryIndexListingCounterScope {
    Global,
    #[cfg(test)]
    Local(Arc<TrajectoryIndexListingCounterState>),
}

static GLOBAL_TRAJECTORY_INDEX_LISTING_COUNTER_SCOPE: TrajectoryIndexListingCounterScope =
    TrajectoryIndexListingCounterScope::Global;

fn listing_counters() -> &'static TrajectoryIndexListingCounterState {
    TRAJECTORY_INDEX_LISTING_COUNTERS.get_or_init(TrajectoryIndexListingCounterState::new)
}

impl TrajectoryIndexListingCounterScope {
    fn state(&self) -> &TrajectoryIndexListingCounterState {
        match self {
            Self::Global => listing_counters(),
            #[cfg(test)]
            Self::Local(counters) => counters,
        }
    }

    fn record_listing_caller(&self, caller: TrajectoryIndexListingCaller) {
        self.state().record_listing_caller(caller);
    }

    fn record_listing_cache_hit(&self) {
        self.state().record_listing_cache_hit();
    }

    fn record_index_read(&self) {
        self.state().record_index_read();
    }

    fn record_directory_scan(&self) {
        self.state().record_directory_scan();
    }

    #[cfg(test)]
    fn local() -> Self {
        Self::Local(Arc::new(TrajectoryIndexListingCounterState::new()))
    }

    #[cfg(test)]
    fn snapshot(&self) -> TrajectoryIndexListingCounters {
        self.state().snapshot()
    }

    #[cfg(test)]
    fn reset(&self) {
        self.state().reset();
    }

    #[cfg(test)]
    fn is_global(&self) -> bool {
        matches!(self, Self::Global)
    }
}

pub fn trajectory_index_listing_counters() -> TrajectoryIndexListingCounters {
    listing_counters().snapshot()
}

pub fn trajectory_index_coordinator_rollout_enabled() -> bool {
    trajectory_index_coordinator_rollout_enabled_for(
        std::env::var(TRAJECTORY_INDEX_COORDINATOR_ENV)
            .ok()
            .as_deref()
            .or_else(|| {
                crate::runtime_settings::current()
                    .trajectory_index_coordinator_enabled
                    .then_some("1")
            }),
    )
}

pub(crate) fn trajectory_index_coordinator_rollout_enabled_for(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        let value = value.trim();
        value == "1"
            || value.eq_ignore_ascii_case("true")
            || value.eq_ignore_ascii_case("yes")
            || value.eq_ignore_ascii_case("on")
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TrajectoryIndex {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub entries: Vec<TrajectoryIndexEntry>,
    #[serde(default)]
    pub skipped_files: Vec<TrajectoryIndexSkippedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrajectoryIndexSkippedFile {
    pub file_name: String,
    pub file_len: u64,
    pub file_modified_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryIndexEntry {
    pub id: String,
    pub file_name: String,
    #[serde(default)]
    pub source: TrajectoryIndexSource,
    pub created_at: String,
    pub updated_at: String,
    pub title: String,
    pub model: String,
    pub mode: String,
    pub message_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_chat_id: Option<String>,
    #[serde(default)]
    pub is_title_generated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<WorktreeMeta>,
    #[serde(default)]
    pub total_lines_added: i64,
    #[serde(default)]
    pub total_lines_removed: i64,
    #[serde(default)]
    pub tasks_total: i32,
    #[serde(default)]
    pub tasks_done: i32,
    #[serde(default)]
    pub tasks_failed: i32,
    #[serde(default)]
    pub total_prompt_tokens: u64,
    #[serde(default)]
    pub total_completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub total_cache_read_tokens: u64,
    #[serde(default)]
    pub total_cache_creation_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_cost_usd: Option<f64>,
    #[serde(default)]
    pub displayable_chat: bool,
    #[serde(default)]
    pub waiting_for_card_ids: Vec<String>,
    #[serde(default)]
    pub file_len: u64,
    #[serde(default)]
    pub file_modified_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrajectoryIndexSource {
    Normal,
    Task {
        task_id: String,
        role: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        card_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        planner_chat_id: Option<String>,
    },
    Buddy,
}

impl Default for TrajectoryIndexSource {
    fn default() -> Self {
        Self::Normal
    }
}

impl From<&TrajectorySourceIdentity> for TrajectoryIndexSource {
    fn from(value: &TrajectorySourceIdentity) -> Self {
        match value {
            TrajectorySourceIdentity::Normal => Self::Normal,
            TrajectorySourceIdentity::Buddy => Self::Buddy,
            TrajectorySourceIdentity::Task {
                task_id,
                role,
                agent_id,
                card_id,
                planner_chat_id,
            } => Self::Task {
                task_id: task_id.clone(),
                role: role.clone(),
                agent_id: agent_id.clone(),
                card_id: card_id.clone(),
                planner_chat_id: planner_chat_id.clone(),
            },
        }
    }
}

impl From<&TrajectoryIndexSource> for TrajectorySourceIdentity {
    fn from(value: &TrajectoryIndexSource) -> Self {
        match value {
            TrajectoryIndexSource::Normal => Self::Normal,
            TrajectoryIndexSource::Buddy => Self::Buddy,
            TrajectoryIndexSource::Task {
                task_id,
                role,
                agent_id,
                card_id,
                planner_chat_id,
            } => Self::Task {
                task_id: task_id.clone(),
                role: role.clone(),
                agent_id: agent_id.clone(),
                card_id: card_id.clone(),
                planner_chat_id: planner_chat_id.clone(),
            },
        }
    }
}

fn default_schema_version() -> u32 {
    TRAJECTORY_INDEX_SCHEMA_VERSION
}

static TRAJECTORY_INDEX_LOCKS: std::sync::OnceLock<AMutex<HashMap<String, Arc<AMutex<()>>>>> =
    std::sync::OnceLock::new();

#[derive(Clone)]
struct CachedLegacyTrajectoryIndex {
    index: TrajectoryIndex,
    generation: DirectoryGeneration,
    last_access: u64,
}

static LEGACY_TRAJECTORY_INDEX_CACHE: std::sync::OnceLock<
    AMutex<HashMap<PathBuf, CachedLegacyTrajectoryIndex>>,
> = std::sync::OnceLock::new();

fn legacy_trajectory_index_cache() -> &'static AMutex<HashMap<PathBuf, CachedLegacyTrajectoryIndex>>
{
    LEGACY_TRAJECTORY_INDEX_CACHE.get_or_init(|| AMutex::new(HashMap::new()))
}

async fn cache_legacy_trajectory_index(dir: &Path, index: TrajectoryIndex) {
    let generation = match directory_generation(dir, index.clone()).await {
        Ok(generation) => generation,
        Err(_) => {
            invalidate_legacy_trajectory_index_cache(dir).await;
            return;
        }
    };
    let mut cache = legacy_trajectory_index_cache().lock().await;
    let key = dir.to_path_buf();
    let last_access = next_trajectory_index_cache_access_stamp();
    if let Some(entry) = cache.get_mut(&key) {
        entry.index = index;
        entry.generation = generation;
        entry.last_access = last_access;
        return;
    }
    if !cache.contains_key(&key) && cache.len() >= MAX_CACHED_TRAJECTORY_DIRECTORIES {
        if let Some(stale) = cache
            .iter()
            .min_by_key(|(_, entry)| entry.last_access)
            .map(|(path, _)| path.clone())
        {
            cache.remove(&stale);
        }
    }
    cache.insert(
        key,
        CachedLegacyTrajectoryIndex {
            index,
            generation,
            last_access,
        },
    );
}

async fn invalidate_legacy_trajectory_index_cache(dir: &Path) {
    legacy_trajectory_index_cache().lock().await.remove(dir);
}

fn get_trajectory_index_locks() -> &'static AMutex<HashMap<String, Arc<AMutex<()>>>> {
    TRAJECTORY_INDEX_LOCKS.get_or_init(|| AMutex::new(HashMap::new()))
}

async fn get_trajectory_index_lock(dir: &Path) -> Arc<AMutex<()>> {
    let key = dir.to_string_lossy().to_string();
    let mut locks = get_trajectory_index_locks().lock().await;
    locks
        .entry(key)
        .or_insert_with(|| Arc::new(AMutex::new(())))
        .clone()
}

const INDEX_LOCK_TIMEOUT: Duration = Duration::from_secs(2);
const INDEX_LOCK_RETRY: Duration = Duration::from_millis(25);
const COORDINATOR_FLUSH_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone)]
enum TrajectoryIndexMutation {
    Upsert(TrajectoryIndexEntry),
    Remove(HashSet<String>),
    Reconcile(Option<TrajectorySourceIdentity>),
}

#[derive(Clone)]
struct PendingTrajectoryIndexMutation {
    sequence: u64,
    mutation: TrajectoryIndexMutation,
}

#[derive(Default)]
struct TrajectoryIndexDirectoryState {
    index: Option<TrajectoryIndex>,
    generation: Option<DirectoryGeneration>,
    loaded: bool,
    recovery_flush_required: bool,
    last_access: u64,
    next_sequence: u64,
    pending: Vec<PendingTrajectoryIndexMutation>,
}

#[derive(Clone)]
pub struct TrajectoryIndexCoordinator {
    directories: Arc<AMutex<HashMap<PathBuf, Arc<AMutex<TrajectoryIndexDirectoryState>>>>>,
    lock_timeout: Duration,
    lock_retry: Duration,
    listing_counter_scope: TrajectoryIndexListingCounterScope,
}

impl Default for TrajectoryIndexCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

fn coordinator_span(component: PerfComponent, dir: &Path) -> Option<perf_diagnostics::PerfSpan> {
    if !perf_diagnostics::is_enabled() {
        return None;
    }
    let index_path = trajectory_index_path(dir);
    Some(perf_diagnostics::span(component, None, Some(&index_path)))
}

fn finish_coordinator_span(
    span: Option<perf_diagnostics::PerfSpan>,
    outcome: PerfOutcome,
    item_count: Option<u64>,
) {
    if let Some(span) = span {
        span.finish(outcome, None, item_count, None, None);
    }
}

fn record_coordinator_count(component: PerfComponent) {
    if !perf_diagnostics::is_enabled() {
        return;
    }
    perf_diagnostics::record(
        component,
        None,
        PerfOutcome::Success,
        0,
        None,
        Some(1),
        None,
    );
}

impl TrajectoryIndexCoordinator {
    pub fn new() -> Self {
        Self {
            directories: Arc::new(AMutex::new(HashMap::new())),
            lock_timeout: INDEX_LOCK_TIMEOUT,
            lock_retry: INDEX_LOCK_RETRY,
            listing_counter_scope: TrajectoryIndexListingCounterScope::Global,
        }
    }

    #[cfg(test)]
    fn with_timing(lock_timeout: Duration, lock_retry: Duration) -> Self {
        Self {
            directories: Arc::new(AMutex::new(HashMap::new())),
            lock_timeout,
            lock_retry,
            listing_counter_scope: TrajectoryIndexListingCounterScope::Global,
        }
    }

    #[cfg(test)]
    fn with_listing_counter_scope(
        listing_counter_scope: TrajectoryIndexListingCounterScope,
    ) -> Self {
        Self {
            directories: Arc::new(AMutex::new(HashMap::new())),
            lock_timeout: INDEX_LOCK_TIMEOUT,
            lock_retry: INDEX_LOCK_RETRY,
            listing_counter_scope,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_local_listing_counters() -> Self {
        Self::with_listing_counter_scope(TrajectoryIndexListingCounterScope::local())
    }

    #[cfg(test)]
    pub(crate) fn listing_counters_for_test(&self) -> TrajectoryIndexListingCounters {
        self.listing_counter_scope.snapshot()
    }

    fn record_listing_caller(&self, caller: TrajectoryIndexListingCaller) {
        self.listing_counter_scope.record_listing_caller(caller);
    }

    async fn directory_state(
        &self,
        dir: &Path,
    ) -> Result<Arc<AMutex<TrajectoryIndexDirectoryState>>, String> {
        let key = crate::files_correction::canonicalize_normalized_path(dir.to_path_buf());
        let mut directories = self.directories.lock().await;
        if let Some(state) = directories.get(&key) {
            if let Ok(mut state_guard) = state.try_lock() {
                state_guard.last_access = next_trajectory_index_cache_access_stamp();
            }
            return Ok(state.clone());
        }
        if directories.len() >= MAX_CACHED_TRAJECTORY_DIRECTORIES {
            let stale = directories
                .iter()
                .filter_map(|(path, state)| {
                    let state = state.try_lock().ok()?;
                    state
                        .pending
                        .is_empty()
                        .then_some((path.clone(), state.last_access))
                })
                .min_by_key(|(_, last_access)| *last_access)
                .map(|(path, _)| path);
            let Some(stale) = stale else {
                return Err(
                    "Trajectory index directory cache is full with pending writes".to_string(),
                );
            };
            directories.remove(&stale);
        }
        let state = Arc::new(AMutex::new(TrajectoryIndexDirectoryState {
            last_access: next_trajectory_index_cache_access_stamp(),
            ..Default::default()
        }));
        directories.insert(key, state.clone());
        Ok(state)
    }

    async fn ensure_loaded(
        &self,
        dir: &Path,
        source_hint: Option<TrajectorySourceIdentity>,
    ) -> Result<Arc<AMutex<TrajectoryIndexDirectoryState>>, String> {
        let state = self.directory_state(dir).await?;
        if state.lock().await.loaded {
            return Ok(state);
        }

        let load_span = coordinator_span(PerfComponent::TrajectoryIndexCoordinatorLoad, dir);
        let index_read_span = coordinator_span(PerfComponent::TrajectoryIndexRead, dir);
        let listing_counter_scope = self.listing_counter_scope.clone();
        let dir = dir.to_path_buf();
        let loaded = tokio::task::spawn_blocking(move || {
            let result = load_trajectory_index_for_coordinator_sync(
                &dir,
                source_hint,
                index_read_span,
                &listing_counter_scope,
            );
            let item_count = result
                .as_ref()
                .ok()
                .map(|loaded| loaded.index.entries.len() as u64);
            finish_coordinator_span(
                load_span,
                if result.is_ok() {
                    PerfOutcome::Success
                } else {
                    PerfOutcome::Failure
                },
                item_count,
            );
            result
        })
        .await
        .map_err(|error| format!("Trajectory index coordinator load task failed: {error}"))??;

        let mut state_guard = state.lock().await;
        if !state_guard.loaded {
            let mut index = loaded.index;
            for pending in &state_guard.pending {
                if !matches!(pending.mutation, TrajectoryIndexMutation::Reconcile(_)) {
                    apply_mutation_to_index(&mut index, &pending.mutation)?;
                }
            }
            state_guard.index = Some(index);
            state_guard.generation = Some(loaded.generation);
            state_guard.loaded = true;
            if loaded.needs_flush {
                push_pending_mutation(
                    &mut state_guard,
                    TrajectoryIndexMutation::Reconcile(loaded.source_hint),
                );
                state_guard.recovery_flush_required = true;
            }
        }
        drop(state_guard);
        Ok(state)
    }

    pub async fn snapshot(
        &self,
        dir: &Path,
        source_hint: Option<TrajectorySourceIdentity>,
    ) -> Result<TrajectoryIndex, String> {
        let state = self.ensure_loaded(dir, source_hint).await?;
        let snapshot = state
            .lock()
            .await
            .index
            .clone()
            .ok_or_else(|| "Trajectory index coordinator did not load an index".to_string());
        snapshot
    }

    pub async fn upsert(&self, dir: &Path, entry: TrajectoryIndexEntry) -> Result<(), String> {
        validate_trajectory_index_entry(&entry)?;
        let state = self.directory_state(dir).await?;
        let mut state_guard = state.lock().await;
        if let Some(index) = state_guard.index.as_mut() {
            apply_mutation_to_index(index, &TrajectoryIndexMutation::Upsert(entry.clone()))?;
        }
        state_guard.generation = None;
        push_pending_mutation(&mut state_guard, TrajectoryIndexMutation::Upsert(entry));
        Ok(())
    }

    pub async fn remove(&self, dir: &Path, chat_ids: HashSet<String>) -> Result<(), String> {
        if chat_ids.is_empty() {
            return Ok(());
        }
        let state = self.directory_state(dir).await?;
        let mut state_guard = state.lock().await;
        if let Some(index) = state_guard.index.as_mut() {
            apply_mutation_to_index(index, &TrajectoryIndexMutation::Remove(chat_ids.clone()))?;
        }
        state_guard.generation = None;
        push_pending_mutation(&mut state_guard, TrajectoryIndexMutation::Remove(chat_ids));
        Ok(())
    }

    pub async fn reconcile(
        &self,
        dir: &Path,
        source_hint: Option<TrajectorySourceIdentity>,
    ) -> Result<TrajectoryIndex, String> {
        let state = self.ensure_loaded(dir, source_hint.clone()).await?;
        let index = state
            .lock()
            .await
            .index
            .clone()
            .ok_or_else(|| "Trajectory index coordinator did not load an index".to_string())?;
        let reconcile_span =
            coordinator_span(PerfComponent::TrajectoryIndexCoordinatorReconcile, dir);
        let dir = dir.to_path_buf();
        let reconcile_dir = dir.clone();
        let source_hint_for_reconcile = source_hint.clone();
        let listing_counter_scope = self.listing_counter_scope.clone();
        let reconciled = tokio::task::spawn_blocking(move || {
            let result = reconcile_trajectory_index_sync(
                &reconcile_dir,
                index,
                source_hint_for_reconcile,
                &listing_counter_scope,
            );
            let item_count = result
                .as_ref()
                .ok()
                .map(|(index, _)| index.entries.len() as u64);
            finish_coordinator_span(
                reconcile_span,
                if result.is_ok() {
                    PerfOutcome::Success
                } else {
                    PerfOutcome::Failure
                },
                item_count,
            );
            result
        })
        .await
        .map_err(|error| {
            format!("Trajectory index coordinator reconcile task failed: {error}")
        })??;

        let mut state_guard = state.lock().await;
        let mut index = reconciled.0;
        for pending in &state_guard.pending {
            if !matches!(pending.mutation, TrajectoryIndexMutation::Reconcile(_)) {
                apply_mutation_to_index(&mut index, &pending.mutation)?;
            }
        }
        state_guard.index = Some(index.clone());
        if reconciled.1 {
            push_pending_mutation(
                &mut state_guard,
                TrajectoryIndexMutation::Reconcile(source_hint),
            );
        }
        drop(state_guard);
        let generation = directory_generation(dir.as_path(), index.clone()).await?;
        state.lock().await.generation = Some(generation);
        Ok(index)
    }

    pub async fn list_entries(
        &self,
        dir: &Path,
        source_hint: Option<TrajectorySourceIdentity>,
    ) -> Result<Vec<TrajectoryIndexEntry>, String> {
        let state = self.ensure_loaded(dir, source_hint.clone()).await?;
        if state.lock().await.recovery_flush_required {
            self.flush_directory(dir).await?;
        }
        let (index, generation) = {
            let state_guard = state.lock().await;
            (
                state_guard.index.clone().ok_or_else(|| {
                    "Trajectory index coordinator did not load an index".to_string()
                })?,
                state_guard.generation.clone(),
            )
        };
        let fresh = match generation {
            Some(generation) => cached_index_is_fresh(dir, index.clone(), generation)
                .await
                .unwrap_or(false),
            None => false,
        };
        if !fresh {
            let recovery_flush_required = matches!(
                read_trajectory_index_with_counter_scope(dir, &self.listing_counter_scope).await,
                Ok(None) | Err(_)
            );
            let recovery_source_hint = source_hint.clone();
            let entries = self.reconcile(dir, source_hint).await?.entries;
            if recovery_flush_required {
                let mut state_guard = state.lock().await;
                push_pending_mutation(
                    &mut state_guard,
                    TrajectoryIndexMutation::Reconcile(recovery_source_hint),
                );
                state_guard.recovery_flush_required = true;
                drop(state_guard);
                self.flush_directory(dir).await?;
            }
            return Ok(entries);
        }
        self.listing_counter_scope.record_listing_cache_hit();
        record_coordinator_count(PerfComponent::TrajectoryIndexCacheHit);
        Ok(index.entries)
    }

    pub async fn flush_directory(&self, dir: &Path) -> Result<(), String> {
        let state = self.ensure_loaded(dir, None).await?;
        let (pending, last_sequence) = {
            let state_guard = state.lock().await;
            let Some(last) = state_guard.pending.last() else {
                return Ok(());
            };
            (state_guard.pending.clone(), last.sequence)
        };
        let dir = dir.to_path_buf();
        let lock_timeout = self.lock_timeout;
        let lock_retry = self.lock_retry;
        let flush_span = coordinator_span(PerfComponent::TrajectoryIndexCoordinatorFlush, &dir);
        let reconcile_span = pending
            .iter()
            .any(|pending| matches!(pending.mutation, TrajectoryIndexMutation::Reconcile(_)))
            .then(|| coordinator_span(PerfComponent::TrajectoryIndexCoordinatorReconcile, &dir))
            .flatten();
        let index_read_span = coordinator_span(PerfComponent::TrajectoryIndexRead, &dir);
        let lock_wait_span = coordinator_span(PerfComponent::TrajectoryIndexLockWait, &dir);
        let index_write_span = coordinator_span(PerfComponent::TrajectoryIndexWrite, &dir);
        let listing_counter_scope = self.listing_counter_scope.clone();
        let flushed_index = tokio::task::spawn_blocking(move || {
            let result = flush_trajectory_index_mutations_sync(
                &dir,
                pending,
                lock_timeout,
                lock_retry,
                index_read_span,
                lock_wait_span,
                index_write_span,
                reconcile_span,
                &listing_counter_scope,
            );
            let item_count = result.as_ref().ok().map(|index| index.entries.len() as u64);
            finish_coordinator_span(
                flush_span,
                if result.is_ok() {
                    PerfOutcome::Success
                } else {
                    PerfOutcome::Failure
                },
                item_count,
            );
            result
        })
        .await
        .map_err(|error| format!("Trajectory index coordinator flush task failed: {error}"))??;

        let mut state_guard = state.lock().await;
        state_guard
            .pending
            .retain(|pending| pending.sequence > last_sequence);
        let mut index = flushed_index;
        for pending in &state_guard.pending {
            if !matches!(pending.mutation, TrajectoryIndexMutation::Reconcile(_)) {
                apply_mutation_to_index(&mut index, &pending.mutation)?;
            }
        }
        state_guard.index = Some(index);
        state_guard.generation = None;
        state_guard.recovery_flush_required = false;
        Ok(())
    }

    pub async fn flush_all(&self) -> Result<(), String> {
        let directories: Vec<PathBuf> = self.directories.lock().await.keys().cloned().collect();
        let mut first_error = None;
        for directory in directories {
            if let Err(error) = self.flush_directory(&directory).await {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

pub async fn trajectory_index_coordinator_background_task(
    coordinator: Arc<TrajectoryIndexCoordinator>,
    shutdown_flag: Arc<std::sync::atomic::AtomicBool>,
) {
    while !shutdown_flag.load(std::sync::atomic::Ordering::Relaxed) {
        tokio::time::sleep(COORDINATOR_FLUSH_INTERVAL).await;
        if let Err(error) = coordinator.flush_all().await {
            tracing::warn!("trajectory index coordinator flush failed: {error}");
        }
    }
    if let Err(error) = coordinator.flush_all().await {
        tracing::warn!("trajectory index coordinator shutdown flush failed: {error}");
    }
}

fn push_pending_mutation(
    state: &mut TrajectoryIndexDirectoryState,
    mutation: TrajectoryIndexMutation,
) {
    state.next_sequence += 1;
    state.pending.push(PendingTrajectoryIndexMutation {
        sequence: state.next_sequence,
        mutation,
    });
    record_coordinator_count(PerfComponent::TrajectoryIndexEnqueue);
}

pub fn trajectory_index_path(dir: &Path) -> PathBuf {
    dir.join(TRAJECTORY_INDEX_FILE)
}

pub fn trajectory_file_path_for_entry(dir: &Path, entry: &TrajectoryIndexEntry) -> PathBuf {
    dir.join(&entry.file_name)
}

struct CoordinatorLoadResult {
    index: TrajectoryIndex,
    generation: DirectoryGeneration,
    needs_flush: bool,
    source_hint: Option<TrajectorySourceIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileGeneration {
    file_len: u64,
    file_modified_at: SystemTime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DirectoryContentsGeneration {
    metadata: FileGeneration,
    entry_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DirectoryGeneration {
    root: Option<DirectoryContentsGeneration>,
    index: Option<FileGeneration>,
    children: Vec<(String, DirectoryContentsGeneration)>,
    indexed_files: Vec<(String, Option<FileGeneration>)>,
}

fn path_generation(path: &Path) -> Result<Option<FileGeneration>, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Failed to read metadata {:?}: {error}", path)),
    };
    if metadata.file_type().is_symlink() {
        return Ok(None);
    }
    Ok(Some(FileGeneration {
        file_len: metadata.len(),
        file_modified_at: metadata
            .modified()
            .map_err(|error| format!("Failed to read modified time for {:?}: {error}", path))?,
    }))
}

fn regular_file_generation(path: &Path) -> Result<Option<FileGeneration>, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Failed to read metadata {:?}: {error}", path)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Ok(None);
    }
    Ok(Some(FileGeneration {
        file_len: metadata.len(),
        file_modified_at: metadata
            .modified()
            .map_err(|error| format!("Failed to read modified time for {:?}: {error}", path))?,
    }))
}

fn directory_contents_generation(
    path: &Path,
) -> Result<Option<DirectoryContentsGeneration>, String> {
    let Some(metadata) = path_generation(path)? else {
        return Ok(None);
    };
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Failed to read directory {:?}: {error}", path)),
    };
    let mut entry_count = 0;
    for entry in entries {
        entry.map_err(|error| format!("Failed to read directory entry in {:?}: {error}", path))?;
        entry_count += 1;
    }
    Ok(Some(DirectoryContentsGeneration {
        metadata,
        entry_count,
    }))
}

fn directory_generation_sync(
    dir: &Path,
    index: &TrajectoryIndex,
) -> Result<DirectoryGeneration, String> {
    let root = directory_contents_generation(dir)?;
    let mut child_names = Vec::new();
    if root.is_some() {
        let entries = std::fs::read_dir(dir)
            .map_err(|error| format!("Failed to read trajectory directory {:?}: {error}", dir))?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                format!(
                    "Failed to read trajectory directory entry in {:?}: {error}",
                    dir
                )
            })?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| format!("Trajectory directory has non-UTF-8 entry in {:?}", dir))?;
            let file_type = entry.file_type().map_err(|error| {
                format!(
                    "Failed to read trajectory directory entry type in {:?}: {error}",
                    dir
                )
            })?;
            if file_type.is_dir() && !file_type.is_symlink() && !name.starts_with('.') {
                child_names.push(name);
            }
        }
    }
    child_names.sort();
    child_names.dedup();

    let mut indexed_file_names: Vec<String> = index
        .entries
        .iter()
        .map(|entry| entry.file_name.as_str())
        .chain(
            index
                .skipped_files
                .iter()
                .map(|entry| entry.file_name.as_str()),
        )
        .map(ToString::to_string)
        .collect();
    indexed_file_names.sort();
    indexed_file_names.dedup();

    let mut children = Vec::with_capacity(child_names.len());
    for child in child_names {
        let Some(generation) = directory_contents_generation(&dir.join(&child))? else {
            return Err(format!(
                "Trajectory child directory changed while checking freshness: {:?}",
                dir.join(&child)
            ));
        };
        children.push((child, generation));
    }

    let indexed_files = indexed_file_names
        .into_iter()
        .map(|file_name| {
            regular_file_generation(&dir.join(&file_name)).map(|generation| (file_name, generation))
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(DirectoryGeneration {
        root,
        index: path_generation(&trajectory_index_path(dir))?,
        children,
        indexed_files,
    })
}

async fn directory_generation(
    dir: &Path,
    index: TrajectoryIndex,
) -> Result<DirectoryGeneration, String> {
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || directory_generation_sync(&dir, &index))
        .await
        .map_err(|error| format!("Trajectory index generation task failed: {error}"))?
}

fn cached_index_is_fresh_sync(
    dir: &Path,
    index: &TrajectoryIndex,
    generation: &DirectoryGeneration,
) -> Result<bool, String> {
    if &directory_generation_sync(dir, index)? != generation {
        return Ok(false);
    }
    Ok(generation
        .indexed_files
        .iter()
        .all(|(_, generation)| generation.is_some()))
}

async fn cached_index_is_fresh(
    dir: &Path,
    index: TrajectoryIndex,
    generation: DirectoryGeneration,
) -> Result<bool, String> {
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || cached_index_is_fresh_sync(&dir, &index, &generation))
        .await
        .map_err(|error| format!("Trajectory index freshness task failed: {error}"))?
}

fn load_trajectory_index_for_coordinator_sync(
    dir: &Path,
    source_hint: Option<TrajectorySourceIdentity>,
    index_read_span: Option<perf_diagnostics::PerfSpan>,
    listing_counter_scope: &TrajectoryIndexListingCounterScope,
) -> Result<CoordinatorLoadResult, String> {
    let index_read = read_trajectory_index_sync(dir);
    let item_count = index_read
        .as_ref()
        .ok()
        .and_then(|index| index.as_ref())
        .map(|index| index.entries.len() as u64);
    finish_coordinator_span(
        index_read_span,
        if index_read.is_ok() {
            PerfOutcome::Success
        } else {
            PerfOutcome::Failure
        },
        item_count,
    );
    match index_read {
        Ok(Some(index)) => {
            let generation = directory_generation_sync(dir, &index)?;
            Ok(CoordinatorLoadResult {
                index,
                generation,
                needs_flush: false,
                source_hint,
            })
        }
        Ok(None) | Err(_) => {
            let (entries, skipped_files) =
                scan_trajectory_index_data_sync(dir, source_hint.clone(), listing_counter_scope)?;
            let index = TrajectoryIndex {
                schema_version: TRAJECTORY_INDEX_SCHEMA_VERSION,
                updated_at: Utc::now().to_rfc3339(),
                entries,
                skipped_files,
            };
            let generation = directory_generation_sync(dir, &index)?;
            Ok(CoordinatorLoadResult {
                index,
                generation,
                needs_flush: true,
                source_hint,
            })
        }
    }
}

fn read_trajectory_index_sync(dir: &Path) -> Result<Option<TrajectoryIndex>, String> {
    let path = trajectory_index_path(dir);
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Failed to read trajectory index {:?}: {error}",
                path
            ))
        }
    };
    let index = serde_json::from_str::<TrajectoryIndex>(&content)
        .map_err(|error| format!("Failed to parse trajectory index {:?}: {error}", path))?;
    if index.schema_version != TRAJECTORY_INDEX_SCHEMA_VERSION {
        return Err(format!(
            "Unsupported trajectory index schema version {} in {:?}",
            index.schema_version, path
        ));
    }
    Ok(Some(index))
}

fn apply_mutation_to_index(
    index: &mut TrajectoryIndex,
    mutation: &TrajectoryIndexMutation,
) -> Result<(), String> {
    match mutation {
        TrajectoryIndexMutation::Upsert(entry) => {
            validate_trajectory_index_entry(entry)?;
            index.entries.retain(|existing| existing.id != entry.id);
            index
                .skipped_files
                .retain(|skipped| skipped.file_name != entry.file_name);
            index.entries.push(entry.clone());
        }
        TrajectoryIndexMutation::Remove(chat_ids) => {
            index.entries.retain(|entry| !chat_ids.contains(&entry.id));
        }
        TrajectoryIndexMutation::Reconcile(_) => {}
    }
    index.schema_version = TRAJECTORY_INDEX_SCHEMA_VERSION;
    index.updated_at = Utc::now().to_rfc3339();
    Ok(())
}

fn validate_trajectory_index_entry(entry: &TrajectoryIndexEntry) -> Result<(), String> {
    if !index_entry_file_name_is_valid(&entry.file_name)
        || Path::new(&entry.file_name)
            .file_stem()
            .and_then(|stem| stem.to_str())
            != Some(entry.id.as_str())
    {
        return Err(format!(
            "Invalid trajectory index entry path: {}",
            entry.file_name
        ));
    }
    Ok(())
}

fn reconcile_trajectory_index_sync(
    dir: &Path,
    index: TrajectoryIndex,
    source_hint: Option<TrajectorySourceIdentity>,
    listing_counter_scope: &TrajectoryIndexListingCounterScope,
) -> Result<(TrajectoryIndex, bool), String> {
    let disk_files = scan_trajectory_dir_files_sync(dir, listing_counter_scope)?;
    let skipped_by_file: HashMap<String, TrajectoryIndexSkippedFile> = index
        .skipped_files
        .iter()
        .cloned()
        .map(|skipped| (skipped.file_name.clone(), skipped))
        .collect();
    let entries_by_file: HashMap<String, TrajectoryIndexEntry> = index
        .entries
        .iter()
        .cloned()
        .map(|entry| (entry.file_name.clone(), entry))
        .collect();
    let mut entries = Vec::with_capacity(disk_files.len());
    let mut skipped_files = Vec::new();
    let mut changed = false;

    for disk in &disk_files {
        if let Some(entry) = entries_by_file.get(&disk.file_name) {
            if entry.file_len == disk.file_len
                && entry.file_modified_unix_ms == disk.file_modified_unix_ms
            {
                entries.push(entry.clone());
                continue;
            }
        }
        if let Some(skipped) = skipped_by_file.get(&disk.file_name) {
            if skipped.file_len == disk.file_len
                && skipped.file_modified_unix_ms == disk.file_modified_unix_ms
            {
                skipped_files.push(skipped.clone());
                continue;
            }
        }
        let path = dir.join(&disk.file_name);
        match read_and_index_single_trajectory_sync(dir, &path, source_hint.clone()) {
            SingleTrajectoryIndexResult::Indexed(entry) => {
                entries.push(entry);
                changed = true;
            }
            SingleTrajectoryIndexResult::Skipped => {
                skipped_files.push(TrajectoryIndexSkippedFile {
                    file_name: disk.file_name.clone(),
                    file_len: disk.file_len,
                    file_modified_unix_ms: disk.file_modified_unix_ms,
                });
                changed = true;
            }
            SingleTrajectoryIndexResult::TransientFailure => {
                if let Some(entry) = entries_by_file.get(&disk.file_name) {
                    entries.push(entry.clone());
                } else if let Some(skipped) = skipped_by_file.get(&disk.file_name) {
                    skipped_files.push(skipped.clone());
                }
            }
        }
    }

    let disk_names: HashSet<&str> = disk_files
        .iter()
        .map(|disk| disk.file_name.as_str())
        .collect();
    changed |= entries_by_file
        .keys()
        .chain(skipped_by_file.keys())
        .any(|file_name| !disk_names.contains(file_name.as_str()));
    let entries = dedupe_entries_by_id(entries);
    Ok((
        TrajectoryIndex {
            schema_version: TRAJECTORY_INDEX_SCHEMA_VERSION,
            updated_at: if changed {
                Utc::now().to_rfc3339()
            } else {
                index.updated_at
            },
            entries,
            skipped_files,
        },
        changed,
    ))
}

fn with_trajectory_index_file_lock<T>(
    dir: &Path,
    timeout: Duration,
    retry: Duration,
    lock_wait_span: Option<perf_diagnostics::PerfSpan>,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let lock_path = dir.join(format!(".{TRAJECTORY_INDEX_FILE}.lock"));
    let lock = crate::daemon::lock::open_lock(&lock_path).map_err(|error| {
        format!(
            "Failed to open trajectory index lock {:?}: {error}",
            lock_path
        )
    });
    let mut lock = match lock {
        Ok(lock) => lock,
        Err(error) => {
            finish_coordinator_span(lock_wait_span, PerfOutcome::Failure, None);
            return Err(error);
        }
    };
    let deadline = Instant::now() + timeout;
    loop {
        match crate::daemon::lock::try_lock(&mut lock) {
            Ok(_guard) => {
                finish_coordinator_span(lock_wait_span, PerfOutcome::Success, None);
                return operation();
            }
            Err(error)
                if crate::daemon::lock::is_already_locked(&error) && Instant::now() < deadline =>
            {
                std::thread::sleep(retry);
            }
            Err(error) if crate::daemon::lock::is_already_locked(&error) => {
                finish_coordinator_span(lock_wait_span, PerfOutcome::Failure, None);
                return Err(format!(
                    "Timed out waiting for trajectory index lock {:?}",
                    lock_path
                ));
            }
            Err(error) => {
                finish_coordinator_span(lock_wait_span, PerfOutcome::Failure, None);
                return Err(format!(
                    "Failed to acquire trajectory index lock {:?}: {error}",
                    lock_path
                ));
            }
        }
    }
}

fn flush_trajectory_index_mutations_sync(
    dir: &Path,
    pending: Vec<PendingTrajectoryIndexMutation>,
    lock_timeout: Duration,
    lock_retry: Duration,
    index_read_span: Option<perf_diagnostics::PerfSpan>,
    lock_wait_span: Option<perf_diagnostics::PerfSpan>,
    index_write_span: Option<perf_diagnostics::PerfSpan>,
    reconcile_span: Option<perf_diagnostics::PerfSpan>,
    listing_counter_scope: &TrajectoryIndexListingCounterScope,
) -> Result<TrajectoryIndex, String> {
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("Failed to create trajectory directory {:?}: {error}", dir))?;
    with_trajectory_index_file_lock(dir, lock_timeout, lock_retry, lock_wait_span, || {
        let mut index = load_trajectory_index_for_coordinator_sync(
            dir,
            None,
            index_read_span,
            listing_counter_scope,
        )?
        .index;
        let mut reconcile_span = reconcile_span;
        for pending in pending {
            match pending.mutation {
                TrajectoryIndexMutation::Reconcile(source_hint) => {
                    let reconciled = reconcile_trajectory_index_sync(
                        dir,
                        index,
                        source_hint,
                        listing_counter_scope,
                    );
                    let item_count = reconciled
                        .as_ref()
                        .ok()
                        .map(|(index, _)| index.entries.len() as u64);
                    finish_coordinator_span(
                        reconcile_span.take(),
                        if reconciled.is_ok() {
                            PerfOutcome::Success
                        } else {
                            PerfOutcome::Failure
                        },
                        item_count,
                    );
                    index = reconciled?.0;
                }
                mutation => apply_mutation_to_index(&mut index, &mutation)?,
            }
        }
        let item_count = index.entries.len() as u64;
        let write_result = write_trajectory_index_atomic_sync(dir, &index);
        finish_coordinator_span(
            index_write_span,
            if write_result.is_ok() {
                PerfOutcome::Success
            } else {
                PerfOutcome::Failure
            },
            Some(item_count),
        );
        write_result?;
        Ok(index)
    })
}

fn write_trajectory_index_atomic_sync(dir: &Path, index: &TrajectoryIndex) -> Result<(), String> {
    let path = trajectory_index_path(dir);
    let tmp_path = dir.join(format!(".{}.tmp-{}", TRAJECTORY_INDEX_FILE, Uuid::new_v4()));
    let content = serde_json::to_string_pretty(index)
        .map_err(|error| format!("Failed to serialize trajectory index {:?}: {error}", path))?;
    let result = (|| {
        std::fs::write(&tmp_path, content).map_err(|error| {
            format!(
                "Failed to write temporary trajectory index {:?}: {error}",
                tmp_path
            )
        })?;
        #[cfg(windows)]
        if path.exists() {
            let backup_path = dir.join(format!(
                ".{}.replace-{}",
                TRAJECTORY_INDEX_FILE,
                Uuid::new_v4()
            ));
            std::fs::rename(&path, &backup_path).map_err(|error| {
                format!("Failed to move trajectory index aside {:?}: {error}", path)
            })?;
            if let Err(error) = std::fs::rename(&tmp_path, &path) {
                let _ = std::fs::rename(&backup_path, &path);
                return Err(format!(
                    "Failed to replace trajectory index {:?}: {error}",
                    path
                ));
            }
            let _ = std::fs::remove_file(backup_path);
            return Ok(());
        }
        std::fs::rename(&tmp_path, &path)
            .map_err(|error| format!("Failed to rename trajectory index {:?}: {error}", path))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    result
}

fn unix_modified_ms(metadata: &std::fs::Metadata) -> Result<i64, String> {
    let modified = metadata
        .modified()
        .map_err(|e| format!("Failed to read modified time: {e}"))?;
    let duration = modified
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Modified time predates Unix epoch: {e}"))?;
    i64::try_from(duration.as_millis()).map_err(|_| "Modified time is too large".to_string())
}

fn file_metadata(path: &Path) -> Result<(u64, i64), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|e| format!("Failed to read trajectory metadata {:?}: {e}", path))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("Trajectory path is not a regular file: {:?}", path));
    }
    Ok((metadata.len(), unix_modified_ms(&metadata)?))
}

pub fn source_from_hint_or_value(
    value: &serde_json::Value,
    source_hint: Option<TrajectorySourceIdentity>,
) -> TrajectorySourceIdentity {
    match TrajectorySourceIdentity::from_json(value) {
        Ok(TrajectorySourceIdentity::Normal) => {
            source_hint.unwrap_or(TrajectorySourceIdentity::Normal)
        }
        Ok(source) => source,
        Err(_) => source_hint.unwrap_or(TrajectorySourceIdentity::Normal),
    }
}

pub fn entry_from_trajectory_value(
    dir: &Path,
    path: &Path,
    value: &serde_json::Value,
    source_hint: Option<TrajectorySourceIdentity>,
) -> Result<TrajectoryIndexEntry, String> {
    let data = serde_json::from_value::<TrajectoryData>(value.clone())
        .map_err(|e| format!("Failed to parse trajectory {:?}: {e}", path))?;
    if data.id.is_empty() || data.created_at.is_empty() {
        return Err(format!(
            "Trajectory {:?} is missing required metadata",
            path
        ));
    }
    let bare_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("Trajectory path has no UTF-8 file name: {:?}", path))?
        .to_string();
    if bare_name == TRAJECTORY_INDEX_FILE || !bare_name.ends_with(".json") {
        return Err(format!("Not a trajectory JSON file: {:?}", path));
    }
    let file_name = path
        .strip_prefix(dir)
        .ok()
        .and_then(|rel| rel.to_str())
        .map(|rel| rel.replace('\\', "/"))
        .filter(|rel| index_entry_file_name_is_valid(rel))
        .unwrap_or(bare_name);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("Trajectory path has no UTF-8 file stem: {:?}", path))?;
    if stem != data.id {
        return Err(format!(
            "Trajectory id mismatch for {:?}: expected {}, found {}",
            path, stem, data.id
        ));
    }
    let (file_len, file_modified_unix_ms) = file_metadata(path)?;
    let (total_lines_added, total_lines_removed) =
        calculate_line_changes_from_messages(&data.messages);
    let (tasks_total, tasks_done, tasks_failed) =
        calculate_task_progress_from_messages(&data.messages);
    let token_totals = calculate_token_totals_from_messages(&data.messages);
    let source = source_from_hint_or_value(value, source_hint);
    let parent_id = data
        .extra
        .get("parent_id")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let link_type = data
        .extra
        .get("link_type")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let root_chat_id = data
        .extra
        .get("root_chat_id")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let is_title_generated = data
        .extra
        .get("isTitleGenerated")
        .or_else(|| data.extra.get("is_title_generated"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let worktree = data
        .extra
        .get("worktree")
        .and_then(|v| serde_json::from_value::<WorktreeMeta>(v.clone()).ok());
    let waiting_for_card_ids = data
        .extra
        .get("waiting_for_card_ids")
        .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
        .unwrap_or_default();
    let list_data = crate::chat::trajectories::TrajectoryListData {
        id: data.id.clone(),
        updated_at: data.updated_at.clone(),
        mode: Some(data.mode.clone()),
        extra: data.extra.clone(),
    };

    Ok(TrajectoryIndexEntry {
        id: data.id,
        file_name,
        source: TrajectoryIndexSource::from(&source),
        created_at: data.created_at.clone(),
        updated_at: if data.updated_at.is_empty() {
            data.created_at.clone()
        } else {
            data.updated_at
        },
        title: trajectory_meta_title(&data.title),
        model: data.model,
        mode: data.mode,
        message_count: data.messages.len(),
        parent_id,
        link_type,
        root_chat_id,
        is_title_generated,
        worktree,
        total_lines_added,
        total_lines_removed,
        tasks_total,
        tasks_done,
        tasks_failed,
        total_prompt_tokens: token_totals.prompt_tokens,
        total_completion_tokens: token_totals.completion_tokens,
        total_tokens: token_totals.total_tokens,
        total_cache_read_tokens: token_totals.cache_read_tokens,
        total_cache_creation_tokens: token_totals.cache_creation_tokens,
        total_cost_usd: token_totals.cost_usd,
        displayable_chat: trajectory_list_data_is_displayable_chat(&list_data),
        waiting_for_card_ids,
        file_len,
        file_modified_unix_ms,
    })
}

pub async fn read_trajectory_index(dir: &Path) -> Result<Option<TrajectoryIndex>, String> {
    read_trajectory_index_with_counter_scope(dir, &GLOBAL_TRAJECTORY_INDEX_LISTING_COUNTER_SCOPE)
        .await
}

async fn read_trajectory_index_with_counter_scope(
    dir: &Path,
    listing_counter_scope: &TrajectoryIndexListingCounterScope,
) -> Result<Option<TrajectoryIndex>, String> {
    if !perf_diagnostics::is_enabled() {
        return read_trajectory_index_inner(dir, listing_counter_scope).await;
    }
    let path = trajectory_index_path(dir);
    let span = perf_diagnostics::span(PerfComponent::TrajectoryIndexRead, None, Some(&path));
    let result = read_trajectory_index_inner(dir, listing_counter_scope).await;
    span.finish(
        if result.is_ok() {
            PerfOutcome::Success
        } else {
            PerfOutcome::Failure
        },
        None,
        result
            .as_ref()
            .ok()
            .and_then(|index| index.as_ref())
            .map(|index| index.entries.len() as u64),
        None,
        None,
    );
    result
}

async fn read_trajectory_index_inner(
    dir: &Path,
    listing_counter_scope: &TrajectoryIndexListingCounterScope,
) -> Result<Option<TrajectoryIndex>, String> {
    listing_counter_scope.record_index_read();
    let path = trajectory_index_path(dir);
    let content = match fs::read_to_string(&path).await {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Failed to read trajectory index {:?}: {e}", path)),
    };
    let parse_path = path.clone();
    let index = tokio::task::spawn_blocking(move || {
        serde_json::from_str::<TrajectoryIndex>(&content)
            .map_err(|e| format!("Failed to parse trajectory index {:?}: {e}", parse_path))
    })
    .await
    .map_err(|e| format!("Trajectory index parse task failed for {:?}: {e}", path))??;
    if index.schema_version != TRAJECTORY_INDEX_SCHEMA_VERSION {
        return Err(format!(
            "Unsupported trajectory index schema version {} in {:?}",
            index.schema_version, path
        ));
    }
    Ok(Some(index))
}

pub async fn write_trajectory_index_atomic(
    dir: &Path,
    index: &TrajectoryIndex,
) -> Result<(), String> {
    write_trajectory_index_atomic_owned(dir, index.clone()).await
}

async fn write_trajectory_index_atomic_owned(
    dir: &Path,
    index: TrajectoryIndex,
) -> Result<(), String> {
    let result = if !perf_diagnostics::is_enabled() {
        write_trajectory_index_atomic_owned_inner(dir, index).await
    } else {
        let path = trajectory_index_path(dir);
        let entry_count = index.entries.len() as u64;
        let span = perf_diagnostics::span(PerfComponent::TrajectoryIndexWrite, None, Some(&path));
        let result = write_trajectory_index_atomic_owned_inner(dir, index).await;
        span.finish(
            if result.is_ok() {
                PerfOutcome::Success
            } else {
                PerfOutcome::Failure
            },
            None,
            Some(entry_count),
            None,
            None,
        );
        result
    };
    if result.is_ok() {
        invalidate_legacy_trajectory_index_cache(dir).await;
    }
    result
}

async fn write_trajectory_index_atomic_owned_inner(
    dir: &Path,
    index: TrajectoryIndex,
) -> Result<(), String> {
    write_trajectory_index_atomic_owned_inner_with_rename_failure(dir, index, false).await
}

async fn write_trajectory_index_atomic_owned_inner_with_rename_failure(
    dir: &Path,
    index: TrajectoryIndex,
    force_rename_failure: bool,
) -> Result<(), String> {
    fs::create_dir_all(dir)
        .await
        .map_err(|e| format!("Failed to create trajectory directory {:?}: {e}", dir))?;
    let path = trajectory_index_path(dir);
    let tmp_path = dir.join(format!(".{}.tmp-{}", TRAJECTORY_INDEX_FILE, Uuid::new_v4()));
    let serialization_path = path.clone();
    let content = tokio::task::spawn_blocking(move || {
        serde_json::to_string_pretty(&index).map_err(|e| {
            format!(
                "Failed to serialize trajectory index {:?}: {e}",
                serialization_path
            )
        })
    })
    .await
    .map_err(|e| {
        format!(
            "Trajectory index serialization task failed for {:?}: {e}",
            path
        )
    })??;
    if let Err(error) = fs::write(&tmp_path, content).await {
        let _ = fs::remove_file(&tmp_path).await;
        return Err(format!(
            "Failed to write temporary trajectory index {:?}: {error}",
            tmp_path
        ));
    }
    let rename_result = if force_rename_failure {
        Err("Failed to rename: injected rename failure".to_string())
    } else {
        crate::chat::trajectories::atomic_write_file(&tmp_path, &path).await
    };
    if let Err(error) = rename_result {
        let _ = fs::remove_file(&tmp_path).await;
        return Err(error);
    }
    Ok(())
}

pub fn index_entry_file_name_is_valid(file_name: &str) -> bool {
    if file_name.contains('\\')
        || file_name.contains(':')
        || file_name.starts_with('/')
        || !file_name.ends_with(".json")
    {
        return false;
    }
    let mut parts = file_name.split('/');
    let (Some(first), second, None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let (folder, name) = match second {
        Some(name) => (Some(first), name),
        None => (None, first),
    };
    if name.is_empty() || name == TRAJECTORY_INDEX_FILE || name.starts_with('.') {
        return false;
    }
    match folder {
        Some(folder) => {
            !folder.is_empty() && folder != "." && folder != ".." && !folder.contains('/')
        }
        None => true,
    }
}

pub async fn trajectory_index_entry_is_fresh(dir: &Path, entry: &TrajectoryIndexEntry) -> bool {
    if !index_entry_file_name_is_valid(&entry.file_name) {
        return false;
    }
    let path = trajectory_file_path_for_entry(dir, entry);
    let stem_matches = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem == entry.id);
    if !stem_matches {
        return false;
    }
    let metadata = match fs::symlink_metadata(&path).await {
        Ok(metadata) => metadata,
        Err(_) => return false,
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return false;
    }
    let modified_ms = match unix_modified_ms(&metadata) {
        Ok(ms) => ms,
        Err(_) => return false,
    };
    metadata.len() == entry.file_len && modified_ms == entry.file_modified_unix_ms
}

pub async fn upsert_trajectory_index_entry(
    dir: &Path,
    entry: TrajectoryIndexEntry,
) -> Result<(), String> {
    let diagnostics_enabled = perf_diagnostics::is_enabled();
    let lock_span = diagnostics_enabled.then(|| {
        let index_path = trajectory_index_path(dir);
        perf_diagnostics::span(
            PerfComponent::TrajectoryIndexLockWait,
            Some(&entry.id),
            Some(&index_path),
        )
    });
    let lock = get_trajectory_index_lock(dir).await;
    let _guard = lock.lock().await;
    if let Some(lock_span) = lock_span {
        lock_span.finish(PerfOutcome::Success, None, None, None, None);
    }
    let mut index = match read_trajectory_index(dir).await {
        Ok(Some(index)) => index,
        Ok(None) | Err(_) => {
            let (entries, skipped_files) = scan_trajectory_index_data(dir, None).await?;
            TrajectoryIndex {
                schema_version: TRAJECTORY_INDEX_SCHEMA_VERSION,
                updated_at: Utc::now().to_rfc3339(),
                entries,
                skipped_files,
            }
        }
    };
    index.entries.retain(|existing| existing.id != entry.id);
    index
        .skipped_files
        .retain(|skipped| skipped.file_name != entry.file_name);
    index.entries.push(entry);
    index.updated_at = Utc::now().to_rfc3339();
    write_trajectory_index_atomic_owned(dir, index).await
}

pub async fn upsert_trajectory_index_entry_with_rollout(
    coordinator: &TrajectoryIndexCoordinator,
    dir: &Path,
    entry: TrajectoryIndexEntry,
) -> Result<(), String> {
    if trajectory_index_coordinator_rollout_enabled() {
        coordinator.upsert(dir, entry).await
    } else {
        upsert_trajectory_index_entry(dir, entry).await
    }
}

pub async fn upsert_trajectory_index_entry_from_owned_value_with_rollout(
    coordinator: &TrajectoryIndexCoordinator,
    dir: &Path,
    path: &Path,
    value: serde_json::Value,
    source_hint: Option<TrajectorySourceIdentity>,
) -> Result<(), String> {
    let dir_owned = dir.to_path_buf();
    let path_owned = path.to_path_buf();
    let entry = tokio::task::spawn_blocking(move || {
        entry_from_trajectory_value(&dir_owned, &path_owned, &value, source_hint)
    })
    .await
    .map_err(|e| format!("Trajectory index entry task failed for {:?}: {e}", path))??;
    upsert_trajectory_index_entry_with_rollout(coordinator, dir, entry).await
}

pub async fn upsert_trajectory_index_entry_from_value(
    dir: &Path,
    path: &Path,
    value: &serde_json::Value,
    source_hint: Option<TrajectorySourceIdentity>,
) -> Result<(), String> {
    let entry = entry_from_trajectory_value(dir, path, value, source_hint)?;
    upsert_trajectory_index_entry(dir, entry).await
}

pub async fn upsert_trajectory_index_entry_from_owned_value(
    dir: &Path,
    path: &Path,
    value: serde_json::Value,
    source_hint: Option<TrajectorySourceIdentity>,
) -> Result<(), String> {
    let dir_owned = dir.to_path_buf();
    let path_owned = path.to_path_buf();
    let entry = tokio::task::spawn_blocking(move || {
        entry_from_trajectory_value(&dir_owned, &path_owned, &value, source_hint)
    })
    .await
    .map_err(|e| format!("Trajectory index entry task failed for {:?}: {e}", path))??;
    upsert_trajectory_index_entry(dir, entry).await
}

pub async fn remove_trajectory_index_entries(
    dir: &Path,
    chat_ids: &HashSet<String>,
) -> Result<(), String> {
    if chat_ids.is_empty() {
        return Ok(());
    }
    let diagnostics_enabled = perf_diagnostics::is_enabled();
    let lock_span = diagnostics_enabled.then(|| {
        let index_path = trajectory_index_path(dir);
        perf_diagnostics::span(
            PerfComponent::TrajectoryIndexLockWait,
            None,
            Some(&index_path),
        )
    });
    let lock = get_trajectory_index_lock(dir).await;
    let _guard = lock.lock().await;
    if let Some(lock_span) = lock_span {
        lock_span.finish(PerfOutcome::Success, None, None, None, None);
    }
    let mut index = match read_trajectory_index(dir).await? {
        Some(index) => index,
        None => return Ok(()),
    };
    let before = index.entries.len();
    index.entries.retain(|entry| !chat_ids.contains(&entry.id));
    if index.entries.len() == before {
        return Ok(());
    }
    index.updated_at = Utc::now().to_rfc3339();
    write_trajectory_index_atomic_owned(dir, index).await
}

pub async fn remove_trajectory_index_entry(dir: &Path, chat_id: &str) -> Result<(), String> {
    let diagnostics_enabled = perf_diagnostics::is_enabled();
    let lock_span = diagnostics_enabled.then(|| {
        let index_path = trajectory_index_path(dir);
        perf_diagnostics::span(
            PerfComponent::TrajectoryIndexLockWait,
            Some(chat_id),
            Some(&index_path),
        )
    });
    let lock = get_trajectory_index_lock(dir).await;
    let _guard = lock.lock().await;
    if let Some(lock_span) = lock_span {
        lock_span.finish(PerfOutcome::Success, None, None, None, None);
    }
    let mut index = match read_trajectory_index(dir).await? {
        Some(index) => index,
        None => return Ok(()),
    };
    let before = index.entries.len();
    index.entries.retain(|entry| entry.id != chat_id);
    if index.entries.len() == before {
        return Ok(());
    }
    index.updated_at = Utc::now().to_rfc3339();
    write_trajectory_index_atomic_owned(dir, index).await
}

pub async fn remove_trajectory_index_entries_with_rollout(
    coordinator: &TrajectoryIndexCoordinator,
    dir: &Path,
    chat_ids: &HashSet<String>,
) -> Result<(), String> {
    if trajectory_index_coordinator_rollout_enabled() {
        coordinator.remove(dir, chat_ids.clone()).await
    } else {
        remove_trajectory_index_entries(dir, chat_ids).await
    }
}

pub async fn remove_trajectory_index_entry_with_rollout(
    coordinator: &TrajectoryIndexCoordinator,
    dir: &Path,
    chat_id: &str,
) -> Result<(), String> {
    remove_trajectory_index_entries_with_rollout(
        coordinator,
        dir,
        &HashSet::from([chat_id.to_string()]),
    )
    .await
}

pub async fn list_trajectory_entries_with_rollout(
    coordinator: &TrajectoryIndexCoordinator,
    dir: &Path,
    source_hint: Option<TrajectorySourceIdentity>,
) -> Result<Vec<TrajectoryIndexEntry>, String> {
    list_trajectory_entries_with_rollout_for(
        coordinator,
        dir,
        source_hint,
        TrajectoryIndexListingCaller::ChatApi,
    )
    .await
}

pub async fn list_trajectory_entries_with_rollout_for(
    coordinator: &TrajectoryIndexCoordinator,
    dir: &Path,
    source_hint: Option<TrajectorySourceIdentity>,
    caller: TrajectoryIndexListingCaller,
) -> Result<Vec<TrajectoryIndexEntry>, String> {
    coordinator.record_listing_caller(caller);
    if trajectory_index_coordinator_rollout_enabled() {
        coordinator.list_entries(dir, source_hint).await
    } else {
        list_trajectory_entries_from_index_or_rebuild_with_counter_scope(
            dir,
            source_hint,
            &coordinator.listing_counter_scope,
        )
        .await
    }
}

fn dedupe_entries_by_id(entries: Vec<TrajectoryIndexEntry>) -> Vec<TrajectoryIndexEntry> {
    let mut position_by_id: HashMap<String, usize> = HashMap::new();
    let mut deduped: Vec<TrajectoryIndexEntry> = Vec::with_capacity(entries.len());
    for entry in entries {
        match position_by_id.get(&entry.id) {
            Some(&position) => {
                if entry.file_modified_unix_ms > deduped[position].file_modified_unix_ms {
                    deduped[position] = entry;
                }
            }
            None => {
                position_by_id.insert(entry.id.clone(), deduped.len());
                deduped.push(entry);
            }
        }
    }
    deduped
}

struct DiskTrajectoryFile {
    file_name: String,
    file_len: u64,
    file_modified_unix_ms: i64,
}

fn scan_trajectory_dir_files_sync(
    dir: &Path,
    listing_counter_scope: &TrajectoryIndexListingCounterScope,
) -> Result<Vec<DiskTrajectoryFile>, String> {
    listing_counter_scope.record_directory_scan();
    let read_dir = match std::fs::read_dir(dir) {
        Ok(read_dir) => read_dir,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "Failed to read trajectory directory {:?}: {error}",
                dir
            ))
        }
    };
    let mut scan_targets = vec![(dir.to_path_buf(), None)];
    for entry in read_dir.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() && !file_type.is_symlink() && !name.starts_with('.') {
            scan_targets.push((entry.path(), Some(name)));
        }
    }
    let mut files = Vec::new();
    for (scan_dir, prefix) in scan_targets {
        let Ok(entries) = std::fs::read_dir(scan_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_name) = entry.file_name().into_string() else {
                continue;
            };
            if file_name == TRAJECTORY_INDEX_FILE
                || file_name.starts_with('.')
                || !file_name.ends_with(".json")
            {
                continue;
            }
            let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                continue;
            }
            let Ok(file_modified_unix_ms) = unix_modified_ms(&metadata) else {
                continue;
            };
            let file_name = prefix
                .as_ref()
                .map(|prefix| format!("{prefix}/{file_name}"))
                .unwrap_or(file_name);
            files.push(DiskTrajectoryFile {
                file_name,
                file_len: metadata.len(),
                file_modified_unix_ms,
            });
        }
    }
    Ok(files)
}

fn read_and_index_single_trajectory_sync(
    dir: &Path,
    path: &Path,
    source_hint: Option<TrajectorySourceIdentity>,
) -> SingleTrajectoryIndexResult {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => metadata,
        _ => return SingleTrajectoryIndexResult::TransientFailure,
    };
    if metadata.len() == 0 {
        return SingleTrajectoryIndexResult::TransientFailure;
    }
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return SingleTrajectoryIndexResult::TransientFailure,
    };
    let value = match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!("Failed to parse trajectory {:?}: {}", path, error);
            return SingleTrajectoryIndexResult::Skipped;
        }
    };
    if value
        .get("link_type")
        .and_then(|value| value.as_str())
        .is_some_and(|link_type| link_type.starts_with("internal:"))
    {
        return SingleTrajectoryIndexResult::Skipped;
    }
    match entry_from_trajectory_value(dir, path, &value, source_hint) {
        Ok(entry) => SingleTrajectoryIndexResult::Indexed(entry),
        Err(error) => {
            tracing::debug!("Skipping non-indexable trajectory {:?}: {}", path, error);
            SingleTrajectoryIndexResult::Skipped
        }
    }
}

fn scan_trajectory_index_data_sync(
    dir: &Path,
    source_hint: Option<TrajectorySourceIdentity>,
    listing_counter_scope: &TrajectoryIndexListingCounterScope,
) -> Result<(Vec<TrajectoryIndexEntry>, Vec<TrajectoryIndexSkippedFile>), String> {
    let disk_files = scan_trajectory_dir_files_sync(dir, listing_counter_scope)?;
    let mut entries = Vec::new();
    let mut skipped_files = Vec::new();
    for disk in disk_files {
        let path = dir.join(&disk.file_name);
        match read_and_index_single_trajectory_sync(dir, &path, source_hint.clone()) {
            SingleTrajectoryIndexResult::Indexed(entry) => entries.push(entry),
            SingleTrajectoryIndexResult::Skipped => {
                skipped_files.push(TrajectoryIndexSkippedFile {
                    file_name: disk.file_name,
                    file_len: disk.file_len,
                    file_modified_unix_ms: disk.file_modified_unix_ms,
                })
            }
            SingleTrajectoryIndexResult::TransientFailure => {}
        }
    }
    Ok((dedupe_entries_by_id(entries), skipped_files))
}

async fn scan_trajectory_dir_files(
    dir: &Path,
    listing_counter_scope: &TrajectoryIndexListingCounterScope,
) -> Result<Vec<DiskTrajectoryFile>, String> {
    listing_counter_scope.record_directory_scan();
    let dir = dir.to_path_buf();
    let dir_for_err = dir.clone();
    tokio::task::spawn_blocking(move || {
        let read_dir = match std::fs::read_dir(&dir) {
            Ok(read_dir) => read_dir,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => {
                return Err(format!(
                    "Failed to read trajectory directory {:?}: {e}",
                    dir
                ))
            }
        };
        let mut scan_targets: Vec<(std::path::PathBuf, Option<String>)> = vec![(dir.clone(), None)];
        for entry in read_dir.flatten() {
            let name = match entry.file_name().into_string() {
                Ok(name) => name,
                Err(_) => continue,
            };
            if name.starts_with('.') {
                continue;
            }
            match entry.file_type() {
                Ok(file_type) if file_type.is_dir() && !file_type.is_symlink() => {
                    scan_targets.push((entry.path(), Some(name)));
                }
                _ => {}
            }
        }

        let mut files = Vec::new();
        for (scan_dir, prefix) in scan_targets {
            let scan_read_dir = match std::fs::read_dir(&scan_dir) {
                Ok(read_dir) => read_dir,
                Err(_) => continue,
            };
            for entry in scan_read_dir {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(_) => continue,
                };
                let file_name = match entry.file_name().into_string() {
                    Ok(name) => name,
                    Err(_) => continue,
                };
                if file_name == TRAJECTORY_INDEX_FILE
                    || file_name.starts_with('.')
                    || !file_name.ends_with(".json")
                {
                    continue;
                }
                let metadata = match std::fs::symlink_metadata(entry.path()) {
                    Ok(metadata) => metadata,
                    Err(_) => continue,
                };
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    continue;
                }
                let file_modified_unix_ms = match unix_modified_ms(&metadata) {
                    Ok(ms) => ms,
                    Err(_) => continue,
                };
                let relative_name = match &prefix {
                    Some(prefix) => format!("{prefix}/{file_name}"),
                    None => file_name,
                };
                files.push(DiskTrajectoryFile {
                    file_name: relative_name,
                    file_len: metadata.len(),
                    file_modified_unix_ms,
                });
            }
        }
        Ok(files)
    })
    .await
    .map_err(|e| format!("Failed to scan trajectory directory {:?}: {e}", dir_for_err))?
}

async fn read_and_index_single_trajectory(
    dir: &Path,
    path: &Path,
    source_hint: Option<TrajectorySourceIdentity>,
) -> SingleTrajectoryIndexResult {
    const INTERNAL_LINK_TAIL_BYTES: u64 = 64 * 1024;

    let mut file = match fs::File::open(path).await {
        Ok(file) => file,
        Err(_) => return SingleTrajectoryIndexResult::TransientFailure,
    };
    let file_len = match file.metadata().await {
        Ok(metadata) => metadata.len(),
        Err(_) => return SingleTrajectoryIndexResult::TransientFailure,
    };
    let tail_start = file_len.saturating_sub(INTERNAL_LINK_TAIL_BYTES);
    if file.seek(SeekFrom::Start(tail_start)).await.is_err() {
        return SingleTrajectoryIndexResult::TransientFailure;
    }
    let mut tail = Vec::new();
    if file.read_to_end(&mut tail).await.is_err() {
        return SingleTrajectoryIndexResult::TransientFailure;
    }
    let tail = String::from_utf8_lossy(&tail);
    if tail.lines().any(|line| {
        let Some(value) = line
            .strip_suffix(',')
            .unwrap_or(line)
            .strip_prefix("  \"link_type\": ")
        else {
            return false;
        };
        serde_json::from_str::<String>(value).is_ok_and(|value| value.starts_with("internal:"))
    }) {
        return SingleTrajectoryIndexResult::Skipped;
    }

    let content = match fs::read_to_string(path).await {
        Ok(content) => content,
        Err(_) => return SingleTrajectoryIndexResult::TransientFailure,
    };
    let dir = dir.to_path_buf();
    let path = path.to_path_buf();
    match tokio::task::spawn_blocking(move || {
        let value = serde_json::from_str::<serde_json::Value>(&content).map_err(|e| {
            tracing::warn!("Failed to parse trajectory {:?}: {}", path, e);
        })?;
        if value
            .get("link_type")
            .and_then(|value| value.as_str())
            .is_some_and(|link_type| link_type.starts_with("internal:"))
        {
            return Err(());
        }
        entry_from_trajectory_value(&dir, &path, &value, source_hint).map_err(|e| {
            tracing::debug!("Skipping non-indexable trajectory {:?}: {}", path, e);
        })
    })
    .await
    {
        Ok(Ok(entry)) => SingleTrajectoryIndexResult::Indexed(entry),
        Ok(Err(())) => SingleTrajectoryIndexResult::Skipped,
        Err(_) => SingleTrajectoryIndexResult::TransientFailure,
    }
}

enum SingleTrajectoryIndexResult {
    Indexed(TrajectoryIndexEntry),
    Skipped,
    TransientFailure,
}

async fn scan_trajectory_index_data(
    dir: &Path,
    source_hint: Option<TrajectorySourceIdentity>,
) -> Result<(Vec<TrajectoryIndexEntry>, Vec<TrajectoryIndexSkippedFile>), String> {
    let disk_files =
        scan_trajectory_dir_files(dir, &GLOBAL_TRAJECTORY_INDEX_LISTING_COUNTER_SCOPE).await?;
    let mut entries = Vec::new();
    let mut skipped_files = Vec::new();
    for disk in disk_files {
        let path = dir.join(&disk.file_name);
        match read_and_index_single_trajectory(dir, &path, source_hint.clone()).await {
            SingleTrajectoryIndexResult::Indexed(entry) => entries.push(entry),
            SingleTrajectoryIndexResult::Skipped => {
                skipped_files.push(TrajectoryIndexSkippedFile {
                    file_name: disk.file_name,
                    file_len: disk.file_len,
                    file_modified_unix_ms: disk.file_modified_unix_ms,
                });
            }
            SingleTrajectoryIndexResult::TransientFailure => {}
        }
    }
    Ok((dedupe_entries_by_id(entries), skipped_files))
}

pub async fn rebuild_trajectory_index_from_disk(
    dir: &Path,
    source_hint: Option<TrajectorySourceIdentity>,
) -> Result<Vec<TrajectoryIndexEntry>, String> {
    if !perf_diagnostics::is_enabled() {
        return rebuild_trajectory_index_from_disk_inner(dir, source_hint).await;
    }
    let index_path = trajectory_index_path(dir);
    let span = perf_diagnostics::span(
        PerfComponent::TrajectoryIndexRebuild,
        None,
        Some(&index_path),
    );
    let result = rebuild_trajectory_index_from_disk_inner(dir, source_hint).await;
    span.finish(
        if result.is_ok() {
            PerfOutcome::Success
        } else {
            PerfOutcome::Failure
        },
        None,
        result.as_ref().ok().map(|entries| entries.len() as u64),
        None,
        None,
    );
    result
}

async fn rebuild_trajectory_index_from_disk_inner(
    dir: &Path,
    source_hint: Option<TrajectorySourceIdentity>,
) -> Result<Vec<TrajectoryIndexEntry>, String> {
    let diagnostics_enabled = perf_diagnostics::is_enabled();
    let lock_span = diagnostics_enabled.then(|| {
        let index_path = trajectory_index_path(dir);
        perf_diagnostics::span(
            PerfComponent::TrajectoryIndexLockWait,
            None,
            Some(&index_path),
        )
    });
    let lock = get_trajectory_index_lock(dir).await;
    let _guard = lock.lock().await;
    if let Some(lock_span) = lock_span {
        lock_span.finish(PerfOutcome::Success, None, None, None, None);
    }
    let (entries, skipped_files) = scan_trajectory_index_data(dir, source_hint).await?;
    let index = TrajectoryIndex {
        schema_version: TRAJECTORY_INDEX_SCHEMA_VERSION,
        updated_at: Utc::now().to_rfc3339(),
        entries: entries.clone(),
        skipped_files,
    };
    write_trajectory_index_atomic_owned(dir, index).await?;
    Ok(entries)
}

pub async fn list_trajectory_entries_from_index_or_rebuild(
    dir: &Path,
    source_hint: Option<TrajectorySourceIdentity>,
) -> Result<Vec<TrajectoryIndexEntry>, String> {
    list_trajectory_entries_from_index_or_rebuild_with_counter_scope(
        dir,
        source_hint,
        &GLOBAL_TRAJECTORY_INDEX_LISTING_COUNTER_SCOPE,
    )
    .await
}

async fn list_trajectory_entries_from_index_or_rebuild_with_counter_scope(
    dir: &Path,
    source_hint: Option<TrajectorySourceIdentity>,
    listing_counter_scope: &TrajectoryIndexListingCounterScope,
) -> Result<Vec<TrajectoryIndexEntry>, String> {
    let cached = {
        let mut cache = legacy_trajectory_index_cache().lock().await;
        if let Some(entry) = cache.get_mut(dir) {
            entry.last_access = next_trajectory_index_cache_access_stamp();
            Some(entry.clone())
        } else {
            None
        }
    };
    if let Some(cached) = cached {
        if let Ok(true) =
            cached_index_is_fresh(dir, cached.index.clone(), cached.generation.clone()).await
        {
            listing_counter_scope.record_listing_cache_hit();
            return Ok(cached.index.entries);
        }
    }
    let diagnostics_enabled = perf_diagnostics::is_enabled();
    let lock_span = diagnostics_enabled.then(|| {
        let index_path = trajectory_index_path(dir);
        perf_diagnostics::span(
            PerfComponent::TrajectoryIndexLockWait,
            None,
            Some(&index_path),
        )
    });
    let lock = get_trajectory_index_lock(dir).await;
    let _guard = lock.lock().await;
    if let Some(lock_span) = lock_span {
        lock_span.finish(PerfOutcome::Success, None, None, None, None);
    }
    let disk_files = scan_trajectory_dir_files(dir, listing_counter_scope).await?;

    let (existing_entries, existing_skipped, index_unreadable) =
        match read_trajectory_index_with_counter_scope(dir, listing_counter_scope).await {
            Ok(Some(index)) => (index.entries, index.skipped_files, false),
            Ok(None) => (Vec::new(), Vec::new(), false),
            Err(_) => (Vec::new(), Vec::new(), true),
        };

    let skipped_by_file: HashMap<String, TrajectoryIndexSkippedFile> = existing_skipped
        .into_iter()
        .map(|skipped| (skipped.file_name.clone(), skipped))
        .collect();

    let by_file: HashMap<String, TrajectoryIndexEntry> = existing_entries
        .into_iter()
        .map(|entry| (entry.file_name.clone(), entry))
        .collect();

    let mut new_entries: Vec<TrajectoryIndexEntry> = Vec::with_capacity(disk_files.len());
    let mut new_skipped = Vec::new();
    let mut content_changed = index_unreadable;

    for disk in &disk_files {
        if let Some(entry) = by_file.get(&disk.file_name) {
            if entry.file_len == disk.file_len
                && entry.file_modified_unix_ms == disk.file_modified_unix_ms
            {
                new_entries.push(entry.clone());
                continue;
            }
        }
        if let Some(skipped) = skipped_by_file.get(&disk.file_name) {
            if skipped.file_len == disk.file_len
                && skipped.file_modified_unix_ms == disk.file_modified_unix_ms
            {
                new_skipped.push(skipped.clone());
                continue;
            }
        }
        let path = dir.join(&disk.file_name);
        match read_and_index_single_trajectory(dir, &path, source_hint.clone()).await {
            SingleTrajectoryIndexResult::Indexed(entry) => {
                new_entries.push(entry);
                content_changed = true;
            }
            SingleTrajectoryIndexResult::Skipped => {
                new_skipped.push(TrajectoryIndexSkippedFile {
                    file_name: disk.file_name.clone(),
                    file_len: disk.file_len,
                    file_modified_unix_ms: disk.file_modified_unix_ms,
                });
                content_changed = true;
            }
            SingleTrajectoryIndexResult::TransientFailure => {
                if let Some(entry) = by_file.get(&disk.file_name) {
                    new_entries.push(entry.clone());
                } else if let Some(skipped) = skipped_by_file.get(&disk.file_name) {
                    new_skipped.push(skipped.clone());
                }
            }
        }
    }

    if !content_changed {
        let disk_names: HashSet<&str> = disk_files
            .iter()
            .map(|disk| disk.file_name.as_str())
            .collect();
        content_changed = by_file
            .keys()
            .chain(skipped_by_file.keys())
            .any(|file_name| !disk_names.contains(file_name.as_str()));
    }

    let new_entries = dedupe_entries_by_id(new_entries);

    let index = if content_changed {
        let index = TrajectoryIndex {
            schema_version: TRAJECTORY_INDEX_SCHEMA_VERSION,
            updated_at: Utc::now().to_rfc3339(),
            entries: new_entries.clone(),
            skipped_files: new_skipped,
        };
        write_trajectory_index_atomic_owned(dir, index.clone()).await?;
        index
    } else {
        let existing = read_trajectory_index_with_counter_scope(dir, listing_counter_scope)
            .await?
            .unwrap_or_default();
        TrajectoryIndex {
            schema_version: TRAJECTORY_INDEX_SCHEMA_VERSION,
            updated_at: existing.updated_at,
            entries: new_entries.clone(),
            skipped_files: new_skipped,
        }
    };
    cache_legacy_trajectory_index(dir, index).await;
    Ok(new_entries)
}

pub(crate) fn list_candidate_from_entry(
    dir: &Path,
    entry: &TrajectoryIndexEntry,
) -> TrajectoryListCandidate {
    TrajectoryListCandidate {
        id: entry.id.clone(),
        updated_at: entry.updated_at.clone(),
        path: trajectory_file_path_for_entry(dir, entry),
        indexed_meta: Some(meta_from_entry(dir, entry)),
        indexed_file_len: Some(entry.file_len),
        indexed_file_modified_unix_ms: Some(entry.file_modified_unix_ms),
    }
}

pub fn meta_from_entry(_dir: &Path, entry: &TrajectoryIndexEntry) -> TrajectoryMeta {
    let source = TrajectorySourceIdentity::from(&entry.source);
    let (task_id, task_role, agent_id, card_id) = match &entry.source {
        TrajectoryIndexSource::Task {
            task_id,
            role,
            agent_id,
            card_id,
            ..
        } => (
            Some(task_id.clone()),
            Some(role.clone()),
            agent_id.clone(),
            card_id.clone(),
        ),
        _ => (None, None, None, None),
    };
    TrajectoryMeta {
        id: entry.id.clone(),
        title: entry.title.clone(),
        created_at: entry.created_at.clone(),
        updated_at: entry.updated_at.clone(),
        model: entry.model.clone(),
        mode: entry.mode.clone(),
        message_count: entry.message_count,
        parent_id: entry.parent_id.clone(),
        link_type: entry.link_type.clone(),
        task_id,
        task_role,
        agent_id,
        card_id,
        session_state: None,
        root_chat_id: entry.root_chat_id.clone(),
        worktree: entry.worktree.clone(),
        total_lines_added: entry.total_lines_added,
        total_lines_removed: entry.total_lines_removed,
        tasks_total: entry.tasks_total,
        tasks_done: entry.tasks_done,
        tasks_failed: entry.tasks_failed,
        total_prompt_tokens: entry.total_prompt_tokens,
        total_completion_tokens: entry.total_completion_tokens,
        total_tokens: entry.total_tokens,
        total_cache_read_tokens: entry.total_cache_read_tokens,
        total_cache_creation_tokens: entry.total_cache_creation_tokens,
        total_cost_usd: entry.total_cost_usd,
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::perf_diagnostics::{self, MemoryPerfSink, PerfClock, PerfComponent, PerfRecorder};
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestClock {
        now: AtomicU64,
    }

    impl TestClock {
        fn new() -> Self {
            Self {
                now: AtomicU64::new(0),
            }
        }
    }

    impl PerfClock for TestClock {
        fn now_us(&self) -> u64 {
            self.now.fetch_add(1, Ordering::SeqCst)
        }
    }

    fn install_perf_recorder() -> (perf_diagnostics::TestRecorderGuard, Arc<MemoryPerfSink>) {
        let sink = Arc::new(MemoryPerfSink::new());
        let recorder = Arc::new(PerfRecorder::with_salt(
            Arc::new(TestClock::new()),
            sink.clone(),
            [3; 32],
        ));
        (perf_diagnostics::install_test_recorder(recorder), sink)
    }

    async fn write_trajectory(dir: &Path, id: &str, title: &str, mode: &str) -> PathBuf {
        fs::create_dir_all(dir).await.unwrap();
        let path = dir.join(format!("{id}.json"));
        let value = json!({
            "id": id,
            "title": title,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:01Z",
            "model": "test-model",
            "mode": mode,
            "tool_use": "agent",
            "messages": [{"role":"user","content":"hello"}],
            "root_chat_id": id
        });
        fs::write(&path, serde_json::to_string_pretty(&value).unwrap())
            .await
            .unwrap();
        path
    }

    #[tokio::test]
    async fn rebuild_creates_index_from_files_and_skips_index_json() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        write_trajectory(&dir, "chat-1", "One", "agent").await;
        write_trajectory(&dir, "chat-2", "Two", "agent").await;
        fs::write(trajectory_index_path(&dir), "{}").await.unwrap();

        let entries = rebuild_trajectory_index_from_disk(&dir, None)
            .await
            .unwrap();

        assert_eq!(entries.len(), 2);
        let index = read_trajectory_index(&dir).await.unwrap().unwrap();
        assert_eq!(index.entries.len(), 2);
        assert!(index
            .entries
            .iter()
            .all(|entry| entry.file_name != TRAJECTORY_INDEX_FILE));
    }

    #[serial_test::serial(trajectory_perf)]
    #[tokio::test]
    async fn performance_diagnostics_cover_index_success_and_failure_boundaries() {
        let _lock = perf_diagnostics::PERF_RECORDER_TEST_LOCK.lock().unwrap();
        let (_guard, sink) = install_perf_recorder();
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        write_trajectory(&dir, "chat-1", "One", "agent").await;

        let entry = entry_from_trajectory_value(
            &dir,
            &dir.join("chat-1.json"),
            &serde_json::from_str(&fs::read_to_string(dir.join("chat-1.json")).await.unwrap())
                .unwrap(),
            None,
        )
        .unwrap();
        upsert_trajectory_index_entry(&dir, entry).await.unwrap();
        rebuild_trajectory_index_from_disk(&dir, None)
            .await
            .unwrap();

        fs::write(trajectory_index_path(&dir), "not-json")
            .await
            .unwrap();
        assert!(read_trajectory_index(&dir).await.is_err());

        let events = sink.events();
        let components: Vec<_> = events.iter().map(|event| event.component).collect();
        assert!(components.contains(&PerfComponent::TrajectoryIndexLockWait.as_str()));
        assert!(components.contains(&PerfComponent::TrajectoryIndexRead.as_str()));
        assert!(components.contains(&PerfComponent::TrajectoryIndexWrite.as_str()));
        assert!(components.contains(&PerfComponent::TrajectoryIndexRebuild.as_str()));
        assert!(events.iter().any(|event| event.outcome == "failure"));
        assert!(events.iter().all(|event| event.path_hash.is_some()));
    }

    #[serial_test::serial(trajectory_perf)]
    #[tokio::test]
    async fn coordinator_performance_diagnostics_cover_load_reconcile_flush_and_enqueue() {
        let _lock = perf_diagnostics::PERF_RECORDER_TEST_LOCK.lock().unwrap();
        let (_guard, sink) = install_perf_recorder();
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        let path_1 = write_trajectory(&dir, "chat-1", "One", "agent").await;
        let coordinator = TrajectoryIndexCoordinator::new();

        coordinator
            .upsert(&dir, entry_for_path(&dir, &path_1).await)
            .await
            .unwrap();
        coordinator.list_entries(&dir, None).await.unwrap();

        write_trajectory(&dir, "chat-2", "Two", "agent").await;
        coordinator.reconcile(&dir, None).await.unwrap();
        coordinator.flush_all().await.unwrap();

        let events = sink.events();
        let components: Vec<_> = events.iter().map(|event| event.component).collect();
        for component in [
            PerfComponent::TrajectoryIndexCoordinatorLoad,
            PerfComponent::TrajectoryIndexCoordinatorReconcile,
            PerfComponent::TrajectoryIndexCoordinatorFlush,
            PerfComponent::TrajectoryIndexEnqueue,
            PerfComponent::TrajectoryIndexRead,
            PerfComponent::TrajectoryIndexLockWait,
            PerfComponent::TrajectoryIndexWrite,
        ] {
            assert!(
                components.contains(&component.as_str()),
                "missing {component:?}"
            );
        }
        assert!(events.iter().all(|event| {
            event.path_hash.is_some()
                || matches!(
                    event.component,
                    "trajectory.index_enqueue" | "trajectory.index_cache_hit"
                )
        }));
    }

    #[serial_test::serial(trajectory_perf)]
    #[tokio::test]
    async fn coordinator_cache_hit_records_a_counter_without_an_index_read() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        let path = write_trajectory(&dir, "chat-1", "One", "agent").await;
        let coordinator = TrajectoryIndexCoordinator::new();
        coordinator
            .upsert(&dir, entry_for_path(&dir, &path).await)
            .await
            .unwrap();
        coordinator.flush_all().await.unwrap();
        coordinator.list_entries(&dir, None).await.unwrap();

        let _lock = perf_diagnostics::PERF_RECORDER_TEST_LOCK.lock().unwrap();
        let (_guard, sink) = install_perf_recorder();
        assert_eq!(coordinator.list_entries(&dir, None).await.unwrap().len(), 1);

        let events = sink.events();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].component,
            PerfComponent::TrajectoryIndexCacheHit.as_str()
        );
        assert_eq!(events[0].item_count, Some(1));
    }

    #[serial_test::serial(trajectory_perf)]
    #[tokio::test]
    async fn coordinator_advisory_lock_wait_is_recorded_for_a_contended_flush() {
        let _lock = perf_diagnostics::PERF_RECORDER_TEST_LOCK.lock().unwrap();
        let (_guard, sink) = install_perf_recorder();
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        let path = write_trajectory(&dir, "chat-1", "One", "agent").await;
        let blocker = crate::daemon::lock::open_lock(&dir.join(".index.json.lock")).unwrap();
        let mut blocker = blocker;
        let blocker_guard = crate::daemon::lock::try_lock(&mut blocker).unwrap();
        let coordinator = TrajectoryIndexCoordinator::with_timing(
            Duration::from_millis(25),
            Duration::from_millis(5),
        );
        coordinator
            .upsert(&dir, entry_for_path(&dir, &path).await)
            .await
            .unwrap();

        assert!(coordinator.flush_all().await.is_err());

        let events = sink.events();
        assert!(events.iter().any(|event| {
            event.component == PerfComponent::TrajectoryIndexLockWait.as_str()
                && event.outcome == PerfOutcome::Failure.as_str()
                && event.path_hash.is_some()
        }));
        drop(blocker_guard);
    }

    #[serial_test::serial(trajectory_perf)]
    #[tokio::test]
    async fn coordinator_diagnostics_are_inactive_without_a_recorder() {
        let _lock = perf_diagnostics::PERF_RECORDER_TEST_LOCK.lock().unwrap();
        let telemetry = perf_diagnostics::process_telemetry();
        let was_enabled = telemetry.enabled();
        telemetry.set_enabled(false);
        telemetry.reset();

        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        let path = write_trajectory(&dir, "chat-1", "One", "agent").await;
        let coordinator = TrajectoryIndexCoordinator::new();
        coordinator
            .upsert(&dir, entry_for_path(&dir, &path).await)
            .await
            .unwrap();
        coordinator.list_entries(&dir, None).await.unwrap();
        coordinator.flush_all().await.unwrap();

        assert!(telemetry
            .snapshot()
            .components
            .iter()
            .all(|component| component.sample_count == 0));
        telemetry.set_enabled(was_enabled);
    }

    #[tokio::test]
    async fn corrupt_index_rebuilds_from_files() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        write_trajectory(&dir, "chat-1", "One", "agent").await;
        fs::write(trajectory_index_path(&dir), "not json")
            .await
            .unwrap();

        let entries = list_trajectory_entries_from_index_or_rebuild(&dir, None)
            .await
            .unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "chat-1");
    }

    #[tokio::test]
    async fn valid_index_rebuilds_when_new_trajectory_appears_on_disk() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        write_trajectory(&dir, "chat-1", "One", "agent").await;
        assert_eq!(
            list_trajectory_entries_from_index_or_rebuild(&dir, None)
                .await
                .unwrap()
                .len(),
            1
        );

        write_trajectory(&dir, "chat-2", "Two", "agent").await;
        let entries = list_trajectory_entries_from_index_or_rebuild(&dir, None)
            .await
            .unwrap();

        assert_eq!(entries.len(), 2);
        let index = read_trajectory_index(&dir).await.unwrap().unwrap();
        assert_eq!(index.entries.len(), 2);
    }

    async fn write_raw_trajectory(dir: &Path, file_stem: &str, id: &str) -> PathBuf {
        fs::create_dir_all(dir).await.unwrap();
        let path = dir.join(format!("{file_stem}.json"));
        let value = json!({
            "id": id,
            "title": "Backup",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:01Z",
            "model": "test-model",
            "mode": "agent",
            "tool_use": "agent",
            "messages": [{"role":"user","content":"hi"}]
        });
        fs::write(&path, serde_json::to_string_pretty(&value).unwrap())
            .await
            .unwrap();
        path
    }

    #[tokio::test]
    async fn non_indexable_backup_files_do_not_force_rebuild() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        write_trajectory(&dir, "chat-1", "One", "agent").await;
        write_raw_trajectory(&dir, "chat-1_initial", "chat-1").await;

        let first = list_trajectory_entries_from_index_or_rebuild(&dir, None)
            .await
            .unwrap();
        assert_eq!(first.len(), 1, "only the real trajectory is indexed");
        assert_eq!(first[0].id, "chat-1");

        let index_after_first = read_trajectory_index(&dir).await.unwrap().unwrap();
        let updated_at_marker = index_after_first.updated_at.clone();
        assert_eq!(index_after_first.entries.len(), 1);

        for _ in 0..3 {
            let entries = list_trajectory_entries_from_index_or_rebuild(&dir, None)
                .await
                .unwrap();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].id, "chat-1");
            let index = read_trajectory_index(&dir).await.unwrap().unwrap();
            assert_eq!(
                index.updated_at, updated_at_marker,
                "index must not be rewritten when nothing indexable changed"
            );
        }
    }

    #[tokio::test]
    async fn internal_trace_skip_is_cached_and_changed_visible_file_is_indexed() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join("trace.json");
        let internal = format!(
            concat!(
                "{{\n",
                "  \"id\": \"trace\",\n",
                "  \"title\": \"Trace\",\n",
                "  \"created_at\": \"2026-01-01T00:00:00Z\",\n",
                "  \"updated_at\": \"2026-01-01T00:00:01Z\",\n",
                "  \"model\": \"test-model\",\n",
                "  \"mode\": \"agent\",\n",
                "  \"tool_use\": \"agent\",\n",
                "  \"messages\": [{{\"role\":\"user\",\"content\":\"{}\"}}],\n",
                "  \"link_type\": \"internal:tool-trace\"\n",
                "}}"
            ),
            "x".repeat(128 * 1024)
        );
        fs::write(&path, internal).await.unwrap();

        assert!(list_trajectory_entries_from_index_or_rebuild(&dir, None)
            .await
            .unwrap()
            .is_empty());
        let first_index = read_trajectory_index(&dir).await.unwrap().unwrap();
        assert_eq!(first_index.skipped_files.len(), 1);
        assert_eq!(first_index.skipped_files[0].file_name, "trace.json");
        let marker = first_index.updated_at;

        assert!(list_trajectory_entries_from_index_or_rebuild(&dir, None)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            read_trajectory_index(&dir)
                .await
                .unwrap()
                .unwrap()
                .updated_at,
            marker
        );

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        write_trajectory(&dir, "trace", "Visible", "agent").await;
        let entries = list_trajectory_entries_from_index_or_rebuild(&dir, None)
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "Visible");
        let final_index = read_trajectory_index(&dir).await.unwrap().unwrap();
        assert!(final_index.skipped_files.is_empty());
        assert_eq!(final_index.entries.len(), 1);
    }

    #[tokio::test]
    async fn unchanged_directory_does_not_rewrite_index() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        write_trajectory(&dir, "chat-1", "One", "agent").await;
        write_trajectory(&dir, "chat-2", "Two", "agent").await;

        assert_eq!(
            list_trajectory_entries_from_index_or_rebuild(&dir, None)
                .await
                .unwrap()
                .len(),
            2
        );
        let marker = read_trajectory_index(&dir)
            .await
            .unwrap()
            .unwrap()
            .updated_at;

        let entries = list_trajectory_entries_from_index_or_rebuild(&dir, None)
            .await
            .unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            read_trajectory_index(&dir)
                .await
                .unwrap()
                .unwrap()
                .updated_at,
            marker
        );
    }

    #[tokio::test]
    async fn modified_trajectory_is_reindexed() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        write_trajectory(&dir, "chat-1", "One", "agent").await;
        let entries = list_trajectory_entries_from_index_or_rebuild(&dir, None)
            .await
            .unwrap();
        assert_eq!(entries[0].title, "One");

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let path = dir.join("chat-1.json");
        let value = json!({
            "id": "chat-1",
            "title": "One Renamed",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-02T00:00:01Z",
            "model": "test-model",
            "mode": "agent",
            "tool_use": "agent",
            "messages": [{"role":"user","content":"hello again"}]
        });
        fs::write(&path, serde_json::to_string_pretty(&value).unwrap())
            .await
            .unwrap();

        let entries = list_trajectory_entries_from_index_or_rebuild(&dir, None)
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "One Renamed");
        let index = read_trajectory_index(&dir).await.unwrap().unwrap();
        assert_eq!(index.entries.len(), 1);
        assert_eq!(index.entries[0].title, "One Renamed");
    }

    #[tokio::test]
    async fn upsert_after_missing_index_preserves_existing_siblings() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        let path_1 = write_trajectory(&dir, "chat-1", "One", "agent").await;
        write_trajectory(&dir, "chat-2", "Two", "agent").await;
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path_1).await.unwrap()).unwrap();

        upsert_trajectory_index_entry_from_value(&dir, &path_1, &value, None)
            .await
            .unwrap();
        let entries = list_trajectory_entries_from_index_or_rebuild(&dir, None)
            .await
            .unwrap();

        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|entry| entry.id == "chat-2"));
    }

    #[tokio::test]
    async fn stale_entry_removed_when_file_deleted() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        let path = write_trajectory(&dir, "chat-1", "One", "agent").await;
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).await.unwrap()).unwrap();
        upsert_trajectory_index_entry_from_value(&dir, &path, &value, None)
            .await
            .unwrap();
        fs::remove_file(path).await.unwrap();

        let entries = list_trajectory_entries_from_index_or_rebuild(&dir, None)
            .await
            .unwrap();

        assert!(entries.is_empty());
        let index = read_trajectory_index(&dir).await.unwrap().unwrap();
        assert!(index.entries.is_empty());
    }

    #[tokio::test]
    async fn extraction_rejects_mismatched_id_vs_filename() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join("chat-1.json");
        let value = json!({
            "id": "chat-2",
            "title": "Bad",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:01Z",
            "model": "test-model",
            "mode": "agent",
            "tool_use": "agent",
            "messages": []
        });
        fs::write(&path, serde_json::to_string(&value).unwrap())
            .await
            .unwrap();

        assert!(entry_from_trajectory_value(&dir, &path, &value, None).is_err());
    }

    #[tokio::test]
    async fn displayable_filter_marks_task_buddy_and_child_links_non_displayable() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        for (id, mode, extra) in [
            ("task-agent", "task_agent", json!({})),
            ("task-planner", "task_planner", json!({})),
            ("buddy", "agent", json!({"buddy_meta": {"x": true}})),
            (
                "child",
                "agent",
                json!({"parent_id": "root", "link_type": "subagent"}),
            ),
        ] {
            fs::create_dir_all(&dir).await.unwrap();
            let path = dir.join(format!("{id}.json"));
            let mut value = json!({
                "id": id,
                "title": id,
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:01Z",
                "model": "test-model",
                "mode": mode,
                "tool_use": "agent",
                "messages": []
            });
            for (k, v) in extra.as_object().unwrap() {
                value[k] = v.clone();
            }
            fs::write(&path, serde_json::to_string(&value).unwrap())
                .await
                .unwrap();
            let entry = entry_from_trajectory_value(&dir, &path, &value, None).unwrap();
            assert!(!entry.displayable_chat, "{id} should be hidden");
        }
    }

    #[tokio::test]
    async fn task_source_roundtrips_context_fields() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("task/trajectories/planner");
        fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join("planner-chat.json");
        let value = json!({
            "id": "planner-chat",
            "title": "Planner",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:01Z",
            "model": "test-model",
            "mode": "task_planner",
            "tool_use": "agent",
            "messages": [],
            "task_meta": {
                "task_id": "task-1",
                "role": "planner",
                "agent_id": null,
                "card_id": "card-1",
                "planner_chat_id": "planner-chat"
            }
        });
        fs::write(&path, serde_json::to_string(&value).unwrap())
            .await
            .unwrap();

        let entry = entry_from_trajectory_value(&dir, &path, &value, None).unwrap();
        let meta = meta_from_entry(&dir, &entry);

        assert_eq!(meta.task_id.as_deref(), Some("task-1"));
        assert_eq!(meta.task_role.as_deref(), Some("planner"));
        assert_eq!(meta.card_id.as_deref(), Some("card-1"));
        assert!(matches!(entry.source, TrajectoryIndexSource::Task { .. }));
    }

    async fn entry_for_path(dir: &Path, path: &Path) -> TrajectoryIndexEntry {
        let value = serde_json::from_str(&fs::read_to_string(path).await.unwrap()).unwrap();
        entry_from_trajectory_value(dir, path, &value, None).unwrap()
    }

    #[tokio::test]
    async fn coordinator_coalesces_last_upsert_and_shutdown_flush() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        let path = write_trajectory(&dir, "chat-1", "One", "agent").await;
        let coordinator = TrajectoryIndexCoordinator::new();
        coordinator
            .upsert(&dir, entry_for_path(&dir, &path).await)
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(20)).await;
        write_trajectory(&dir, "chat-1", "Renamed", "agent").await;
        coordinator
            .upsert(&dir, entry_for_path(&dir, &path).await)
            .await
            .unwrap();
        coordinator.flush_all().await.unwrap();

        let index = read_trajectory_index(&dir).await.unwrap().unwrap();
        assert_eq!(index.entries.len(), 1);
        assert_eq!(index.entries[0].title, "Renamed");
    }

    #[tokio::test]
    async fn coordinator_enqueues_before_loading_or_writing_the_index() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        let path = write_trajectory(&dir, "chat-1", "One", "agent").await;
        let coordinator = TrajectoryIndexCoordinator::new();

        coordinator
            .upsert(&dir, entry_for_path(&dir, &path).await)
            .await
            .unwrap();

        assert!(!trajectory_index_path(&dir).exists());
        assert_eq!(coordinator.directories.lock().await.len(), 1);
        coordinator.flush_all().await.unwrap();
        assert_eq!(
            read_trajectory_index(&dir)
                .await
                .unwrap()
                .unwrap()
                .entries
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn coordinator_recovers_an_unflushed_committed_trajectory_on_restart() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        let path = write_trajectory(&dir, "chat-1", "One", "agent").await;
        let coordinator = TrajectoryIndexCoordinator::new();
        coordinator
            .upsert(&dir, entry_for_path(&dir, &path).await)
            .await
            .unwrap();
        drop(coordinator);

        let restarted = TrajectoryIndexCoordinator::new();
        let entries = restarted.list_entries(&dir, None).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "chat-1");
        restarted.flush_all().await.unwrap();
        assert_eq!(
            read_trajectory_index(&dir)
                .await
                .unwrap()
                .unwrap()
                .entries
                .len(),
            1
        );
    }

    #[test]
    fn coordinator_rollout_switch_defaults_on_and_retains_legacy_fallback() {
        assert!(
            crate::runtime_settings::TrajectoryRuntimeSettings::default()
                .trajectory_index_coordinator_enabled
        );
        assert!(trajectory_index_coordinator_rollout_enabled_for(Some("1")));
        for disabled in [Some("0"), Some("false"), Some("off")] {
            assert!(!trajectory_index_coordinator_rollout_enabled_for(disabled));
        }
        for enabled in [Some("1"), Some("true"), Some("YES"), Some("on")] {
            assert!(trajectory_index_coordinator_rollout_enabled_for(enabled));
        }
    }

    #[tokio::test]
    async fn coordinator_orders_remove_then_upsert_and_upsert_then_remove() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        let path = write_trajectory(&dir, "chat-1", "One", "agent").await;
        let entry = entry_for_path(&dir, &path).await;
        let coordinator = TrajectoryIndexCoordinator::new();
        let ids = HashSet::from(["chat-1".to_string()]);

        coordinator.remove(&dir, ids.clone()).await.unwrap();
        coordinator.upsert(&dir, entry.clone()).await.unwrap();
        coordinator.flush_all().await.unwrap();
        assert_eq!(
            read_trajectory_index(&dir)
                .await
                .unwrap()
                .unwrap()
                .entries
                .len(),
            1
        );

        coordinator.upsert(&dir, entry).await.unwrap();
        coordinator.remove(&dir, ids).await.unwrap();
        coordinator.flush_all().await.unwrap();
        assert!(read_trajectory_index(&dir)
            .await
            .unwrap()
            .unwrap()
            .entries
            .is_empty());
    }

    #[tokio::test]
    async fn coordinator_failed_flush_retains_pending_mutations_for_retry() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        let path = write_trajectory(&dir, "chat-1", "One", "agent").await;
        let coordinator = TrajectoryIndexCoordinator::new();
        coordinator
            .upsert(&dir, entry_for_path(&dir, &path).await)
            .await
            .unwrap();

        let blocker = crate::daemon::lock::open_lock(&dir.join(".index.json.lock")).unwrap();
        let mut blocker = blocker;
        let _guard = crate::daemon::lock::try_lock(&mut blocker).unwrap();
        let fast = TrajectoryIndexCoordinator::with_timing(
            Duration::from_millis(25),
            Duration::from_millis(5),
        );
        fast.upsert(&dir, entry_for_path(&dir, &path).await)
            .await
            .unwrap();
        assert!(fast.flush_all().await.is_err());
        drop(_guard);
        fast.flush_all().await.unwrap();

        assert_eq!(
            read_trajectory_index(&dir)
                .await
                .unwrap()
                .unwrap()
                .entries
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn coordinator_reconciles_external_files_and_reuses_clean_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        let path_1 = write_trajectory(&dir, "chat-1", "One", "agent").await;
        let coordinator = TrajectoryIndexCoordinator::new();
        coordinator
            .upsert(&dir, entry_for_path(&dir, &path_1).await)
            .await
            .unwrap();
        coordinator.flush_all().await.unwrap();
        let first = coordinator.snapshot(&dir, None).await.unwrap();
        let second = coordinator.snapshot(&dir, None).await.unwrap();
        assert_eq!(first.updated_at, second.updated_at);

        write_trajectory(&dir, "chat-2", "Two", "agent").await;
        let reconciled = coordinator.reconcile(&dir, None).await.unwrap();
        assert_eq!(reconciled.entries.len(), 2);
        coordinator.flush_all().await.unwrap();
        assert_eq!(
            read_trajectory_index(&dir)
                .await
                .unwrap()
                .unwrap()
                .entries
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn coordinator_snapshot_does_not_reparse_clean_indexed_trajectories() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        let path = write_trajectory(&dir, "chat-1", "One", "agent").await;
        let coordinator = TrajectoryIndexCoordinator::new();
        coordinator
            .upsert(&dir, entry_for_path(&dir, &path).await)
            .await
            .unwrap();
        coordinator.flush_all().await.unwrap();
        fs::write(&path, "invalid JSON").await.unwrap();

        let snapshot = coordinator.snapshot(&dir, None).await.unwrap();
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].id, "chat-1");
    }

    #[tokio::test]
    async fn legacy_cache_lru_evicts_oldest_directory() {
        let temp = tempfile::tempdir().unwrap();
        legacy_trajectory_index_cache().lock().await.clear();

        for index in 0..MAX_CACHED_TRAJECTORY_DIRECTORIES {
            let dir = temp.path().join(format!("dir-{index}"));
            cache_legacy_trajectory_index(&dir, TrajectoryIndex::default()).await;
        }

        let oldest = temp.path().join("dir-0");
        let newest = temp
            .path()
            .join(format!("dir-{}", MAX_CACHED_TRAJECTORY_DIRECTORIES - 1));
        let extra = temp.path().join("dir-extra");
        cache_legacy_trajectory_index(&extra, TrajectoryIndex::default()).await;

        let cache = legacy_trajectory_index_cache().lock().await;
        assert!(!cache.contains_key(&oldest));
        assert!(cache.contains_key(&newest));
        assert!(cache.contains_key(&extra));
    }

    #[tokio::test]
    async fn coordinator_directory_state_lru_evicts_oldest_clean_directory() {
        let coordinator = TrajectoryIndexCoordinator::new();
        let temp = tempfile::tempdir().unwrap();

        for index in 0..MAX_CACHED_TRAJECTORY_DIRECTORIES {
            let dir = temp.path().join(format!("dir-{index}"));
            coordinator.directory_state(&dir).await.unwrap();
        }

        let extra = temp.path().join("dir-extra");
        coordinator.directory_state(&extra).await.unwrap();

        let directories = coordinator.directories.lock().await;
        assert!(!directories.contains_key(&temp.path().join("dir-0")));
        assert!(directories.contains_key(
            &temp
                .path()
                .join(format!("dir-{}", MAX_CACHED_TRAJECTORY_DIRECTORIES - 1))
        ));
        assert!(directories.contains_key(&extra));
    }

    #[tokio::test]
    async fn write_trajectory_index_atomic_owned_inner_cleans_temp_file_on_failure() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        fs::create_dir_all(&dir).await.unwrap();
        let error = write_trajectory_index_atomic_owned_inner_with_rename_failure(
            &dir,
            TrajectoryIndex::default(),
            true,
        )
        .await
        .unwrap_err();
        assert!(error.contains("Failed to rename"));

        let mut entries = tokio::fs::read_dir(&dir).await.unwrap();
        while let Some(entry) = entries.next_entry().await.unwrap() {
            assert!(!entry
                .file_name()
                .to_string_lossy()
                .contains(".index.json.tmp-"));
        }
    }

    #[tokio::test]
    async fn coordinator_listing_avoids_reads_and_scans_for_unchanged_directory() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        let path = write_trajectory(&dir, "chat-1", "One", "agent").await;
        let listing_counter_scope = TrajectoryIndexListingCounterScope::local();
        let coordinator =
            TrajectoryIndexCoordinator::with_listing_counter_scope(listing_counter_scope.clone());
        coordinator
            .upsert(&dir, entry_for_path(&dir, &path).await)
            .await
            .unwrap();
        coordinator.flush_all().await.unwrap();
        coordinator.list_entries(&dir, None).await.unwrap();

        listing_counter_scope.reset();
        let entries = coordinator.list_entries(&dir, None).await.unwrap();
        let counters = listing_counter_scope.snapshot();

        assert_eq!(entries.len(), 1);
        assert_eq!(counters.index_reads, 0);
        assert_eq!(counters.directory_scans, 0);
        assert_eq!(counters.cache_hits, 1);
    }

    #[tokio::test]
    async fn legacy_listing_avoids_reads_and_scans_for_unchanged_directory() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        write_trajectory(&dir, "chat-1", "One", "agent").await;
        let listing_counter_scope = TrajectoryIndexListingCounterScope::local();
        list_trajectory_entries_from_index_or_rebuild_with_counter_scope(
            &dir,
            None,
            &listing_counter_scope,
        )
        .await
        .unwrap();

        listing_counter_scope.reset();
        let entries = list_trajectory_entries_from_index_or_rebuild_with_counter_scope(
            &dir,
            None,
            &listing_counter_scope,
        )
        .await
        .unwrap();
        let counters = listing_counter_scope.snapshot();

        assert_eq!(entries.len(), 1);
        assert_eq!(counters.index_reads, 0);
        assert_eq!(counters.directory_scans, 0);
        assert_eq!(counters.cache_hits, 1);
    }

    #[tokio::test]
    async fn legacy_cached_listing_reflects_external_creation_and_deletion() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        let path = write_trajectory(&dir, "chat-1", "One", "agent").await;

        assert_eq!(
            list_trajectory_entries_from_index_or_rebuild(&dir, None)
                .await
                .unwrap()
                .len(),
            1
        );

        write_trajectory(&dir, "chat-2", "Two", "agent").await;
        assert_eq!(
            list_trajectory_entries_from_index_or_rebuild(&dir, None)
                .await
                .unwrap()
                .len(),
            2
        );

        fs::remove_file(path).await.unwrap();
        let entries = list_trajectory_entries_from_index_or_rebuild(&dir, None)
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "chat-2");
    }

    #[tokio::test]
    async fn cached_listings_reflect_created_modified_and_deleted_trajectories() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        let path = write_trajectory(&dir, "chat-1", "One", "agent").await;
        let coordinator = TrajectoryIndexCoordinator::new();
        coordinator
            .upsert(&dir, entry_for_path(&dir, &path).await)
            .await
            .unwrap();
        coordinator.flush_all().await.unwrap();
        assert_eq!(coordinator.list_entries(&dir, None).await.unwrap().len(), 1);

        write_trajectory(&dir, "chat-2", "Two", "agent").await;
        assert_eq!(coordinator.list_entries(&dir, None).await.unwrap().len(), 2);

        tokio::time::sleep(Duration::from_millis(20)).await;
        write_trajectory(&dir, "chat-1", "Renamed", "agent").await;
        let entries = coordinator.list_entries(&dir, None).await.unwrap();
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.id == "chat-1")
                .unwrap()
                .title,
            "Renamed"
        );

        fs::remove_file(dir.join("chat-2.json")).await.unwrap();
        let entries = coordinator.list_entries(&dir, None).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "chat-1");
    }

    #[tokio::test]
    async fn coordinator_scans_only_changed_directories_across_repeated_polls() {
        const DIRECTORY_COUNT: usize = 24;
        const POLL_COUNT: usize = 12;

        let temp = tempfile::tempdir().unwrap();
        let listing_counter_scope = TrajectoryIndexListingCounterScope::local();
        let coordinator =
            TrajectoryIndexCoordinator::with_listing_counter_scope(listing_counter_scope.clone());
        let mut dirs = Vec::new();
        for number in 0..DIRECTORY_COUNT {
            let dir = temp.path().join(format!("trajectories-{number}"));
            write_trajectory(&dir, "chat-1", "One", "agent").await;
            coordinator.list_entries(&dir, None).await.unwrap();
            dirs.push(dir);
        }

        listing_counter_scope.reset();
        for _ in 0..POLL_COUNT {
            for dir in &dirs {
                assert_eq!(coordinator.list_entries(dir, None).await.unwrap().len(), 1);
            }
        }
        let unchanged = listing_counter_scope.snapshot();
        assert_eq!(unchanged.directory_scans, 0);
        assert_eq!(
            unchanged.cache_hits,
            u64::try_from(DIRECTORY_COUNT * POLL_COUNT).unwrap()
        );

        write_trajectory(&dirs[0], "chat-2", "Two", "agent").await;
        assert_eq!(
            coordinator
                .list_entries(&dirs[0], None)
                .await
                .unwrap()
                .len(),
            2
        );
        for dir in &dirs[1..] {
            assert_eq!(coordinator.list_entries(dir, None).await.unwrap().len(), 1);
        }
        let changed = listing_counter_scope.snapshot();
        assert_eq!(changed.directory_scans, 1);
        assert_eq!(
            changed.cache_hits,
            u64::try_from(DIRECTORY_COUNT * POLL_COUNT + DIRECTORY_COUNT - 1).unwrap()
        );
    }

    #[tokio::test]
    async fn listing_counters_attribute_callers() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        write_trajectory(&dir, "chat-1", "One", "agent").await;
        assert!(TrajectoryIndexCoordinator::new()
            .listing_counter_scope
            .is_global());
        let listing_counter_scope = TrajectoryIndexListingCounterScope::local();
        let coordinator =
            TrajectoryIndexCoordinator::with_listing_counter_scope(listing_counter_scope.clone());

        listing_counter_scope.reset();
        list_trajectory_entries_with_rollout_for(
            &coordinator,
            &dir,
            None,
            TrajectoryIndexListingCaller::Sidebar,
        )
        .await
        .unwrap();
        list_trajectory_entries_with_rollout_for(
            &coordinator,
            &dir,
            None,
            TrajectoryIndexListingCaller::TaskAgentMonitor,
        )
        .await
        .unwrap();
        let counters = listing_counter_scope.snapshot();

        assert_eq!(
            counters
                .calls_by_caller
                .iter()
                .find(|(caller, _)| *caller == "sidebar")
                .unwrap()
                .1,
            1
        );
        assert_eq!(
            counters
                .calls_by_caller
                .iter()
                .find(|(caller, _)| *caller == "task_agent_monitor")
                .unwrap()
                .1,
            1
        );
    }

    #[tokio::test]
    async fn coordinator_merges_two_process_owned_instances_without_lost_entries() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        let path_1 = write_trajectory(&dir, "chat-1", "One", "agent").await;
        let path_2 = write_trajectory(&dir, "chat-2", "Two", "agent").await;
        let first = TrajectoryIndexCoordinator::new();
        let second = TrajectoryIndexCoordinator::new();
        first
            .upsert(&dir, entry_for_path(&dir, &path_1).await)
            .await
            .unwrap();
        second
            .upsert(&dir, entry_for_path(&dir, &path_2).await)
            .await
            .unwrap();

        let (first_result, second_result) = tokio::join!(first.flush_all(), second.flush_all());
        first_result.unwrap();
        second_result.unwrap();
        let index = read_trajectory_index(&dir).await.unwrap().unwrap();
        assert_eq!(index.entries.len(), 2);
        assert!(index.entries.iter().any(|entry| entry.id == "chat-1"));
        assert!(index.entries.iter().any(|entry| entry.id == "chat-2"));
    }

    #[tokio::test]
    async fn coordinator_cross_process_child_writer() {
        let Ok(dir) = std::env::var("REFACT_TRAJECTORY_INDEX_CHILD_DIR") else {
            return;
        };
        let id = std::env::var("REFACT_TRAJECTORY_INDEX_CHILD_ID").unwrap();
        let dir = PathBuf::from(dir);
        let path = dir.join(format!("{id}.json"));
        let coordinator = TrajectoryIndexCoordinator::new();
        coordinator
            .upsert(&dir, entry_for_path(&dir, &path).await)
            .await
            .unwrap();
        fs::write(dir.join(format!(".ready-{id}")), "ready")
            .await
            .unwrap();
        let release = dir.join(".release");
        tokio::time::timeout(Duration::from_secs(5), async {
            while !release.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        coordinator.flush_all().await.unwrap();
    }

    #[tokio::test]
    async fn coordinator_cross_process_concurrent_writers_lose_zero_durable_entries() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        write_trajectory(&dir, "chat-1", "One", "agent").await;
        write_trajectory(&dir, "chat-2", "Two", "agent").await;
        let executable = std::env::current_exe().unwrap();
        let child_dir = dir.to_string_lossy().into_owned();
        let spawn_child = |id: &'static str| {
            let executable = executable.clone();
            let child_dir = child_dir.clone();
            tokio::task::spawn_blocking(move || {
                std::process::Command::new(executable)
                    .args([
                        "--exact",
                        "chat::trajectory_index::tests::coordinator_cross_process_child_writer",
                        "--nocapture",
                        "--test-threads=1",
                    ])
                    .env("REFACT_TRAJECTORY_INDEX_CHILD_DIR", child_dir)
                    .env("REFACT_TRAJECTORY_INDEX_CHILD_ID", id)
                    .status()
                    .unwrap()
            })
        };
        let first = spawn_child("chat-1");
        let second = spawn_child("chat-2");
        tokio::time::timeout(Duration::from_secs(10), async {
            while !dir.join(".ready-chat-1").exists() || !dir.join(".ready-chat-2").exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        fs::write(dir.join(".release"), "release").await.unwrap();
        assert!(first.await.unwrap().success());
        assert!(second.await.unwrap().success());
        let index = read_trajectory_index(&dir).await.unwrap().unwrap();
        assert_eq!(index.entries.len(), 2);
    }

    #[tokio::test]
    async fn coordinator_matches_legacy_upsert_result() {
        let temp = tempfile::tempdir().unwrap();
        let legacy_dir = temp.path().join("legacy");
        let coordinator_dir = temp.path().join("coordinator");
        let legacy_path = write_trajectory(&legacy_dir, "chat-1", "One", "agent").await;
        let coordinator_path = write_trajectory(&coordinator_dir, "chat-1", "One", "agent").await;
        upsert_trajectory_index_entry(&legacy_dir, entry_for_path(&legacy_dir, &legacy_path).await)
            .await
            .unwrap();
        let coordinator = TrajectoryIndexCoordinator::new();
        coordinator
            .upsert(
                &coordinator_dir,
                entry_for_path(&coordinator_dir, &coordinator_path).await,
            )
            .await
            .unwrap();
        coordinator.flush_all().await.unwrap();

        let legacy = read_trajectory_index(&legacy_dir).await.unwrap().unwrap();
        let coordinated = read_trajectory_index(&coordinator_dir)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(legacy.entries.len(), coordinated.entries.len());
        assert_eq!(legacy.entries[0].id, coordinated.entries[0].id);
        assert_eq!(
            legacy.entries[0].file_name,
            coordinated.entries[0].file_name
        );
    }

    #[tokio::test]
    async fn coordinator_rebuilds_corrupt_index_and_retains_skipped_files() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        write_trajectory(&dir, "chat-1", "One", "agent").await;
        fs::write(
            dir.join("internal.json"),
            json!({
                "id": "internal",
                "title": "Internal",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:01Z",
                "model": "test-model",
                "mode": "agent",
                "tool_use": "agent",
                "messages": [],
                "link_type": "internal:test"
            })
            .to_string(),
        )
        .await
        .unwrap();
        fs::write(trajectory_index_path(&dir), "corrupt")
            .await
            .unwrap();
        let coordinator = TrajectoryIndexCoordinator::new();
        let snapshot = coordinator.snapshot(&dir, None).await.unwrap();
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.skipped_files.len(), 1);
        coordinator.flush_all().await.unwrap();
        let persisted = read_trajectory_index(&dir).await.unwrap().unwrap();
        assert_eq!(persisted.entries.len(), 1);
        assert_eq!(persisted.skipped_files.len(), 1);
    }

    #[tokio::test]
    async fn coordinator_listing_repairs_a_corrupt_cached_index() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        let path = write_trajectory(&dir, "chat-1", "One", "agent").await;
        let coordinator = TrajectoryIndexCoordinator::new();
        coordinator
            .upsert(&dir, entry_for_path(&dir, &path).await)
            .await
            .unwrap();
        coordinator.flush_all().await.unwrap();
        coordinator.list_entries(&dir, None).await.unwrap();

        fs::write(trajectory_index_path(&dir), "corrupt")
            .await
            .unwrap();

        let entries = coordinator.list_entries(&dir, None).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "chat-1");
        assert_eq!(
            read_trajectory_index(&dir)
                .await
                .unwrap()
                .unwrap()
                .entries
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn coordinator_flush_replaces_a_parseable_schema_v1_index() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("trajectories");
        let path = write_trajectory(&dir, "chat-1", "One", "agent").await;
        let coordinator = TrajectoryIndexCoordinator::new();
        coordinator
            .upsert(&dir, entry_for_path(&dir, &path).await)
            .await
            .unwrap();
        coordinator.flush_all().await.unwrap();

        let raw = fs::read_to_string(trajectory_index_path(&dir))
            .await
            .unwrap();
        let parsed: TrajectoryIndex = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed.schema_version, TRAJECTORY_INDEX_SCHEMA_VERSION);
        assert!(!dir
            .read_dir()
            .expect("directory exists")
            .any(|entry| entry.is_ok_and(|entry| entry
                .file_name()
                .to_string_lossy()
                .contains(".index.json.tmp"))));
    }
}
