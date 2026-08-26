use std::collections::{HashMap, HashSet};
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

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
const COORDINATOR_RECONCILE_INTERVAL: Duration = Duration::from_secs(1);

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
    loaded: bool,
    last_reconciled_at: Option<Instant>,
    next_sequence: u64,
    pending: Vec<PendingTrajectoryIndexMutation>,
}

#[derive(Clone)]
pub struct TrajectoryIndexCoordinator {
    directories: Arc<AMutex<HashMap<PathBuf, Arc<AMutex<TrajectoryIndexDirectoryState>>>>>,
    lock_timeout: Duration,
    lock_retry: Duration,
}

impl Default for TrajectoryIndexCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl TrajectoryIndexCoordinator {
    pub fn new() -> Self {
        Self {
            directories: Arc::new(AMutex::new(HashMap::new())),
            lock_timeout: INDEX_LOCK_TIMEOUT,
            lock_retry: INDEX_LOCK_RETRY,
        }
    }

    #[cfg(test)]
    fn with_timing(lock_timeout: Duration, lock_retry: Duration) -> Self {
        Self {
            directories: Arc::new(AMutex::new(HashMap::new())),
            lock_timeout,
            lock_retry,
        }
    }

    async fn directory_state(&self, dir: &Path) -> Arc<AMutex<TrajectoryIndexDirectoryState>> {
        let key = dir.to_path_buf();
        let mut directories = self.directories.lock().await;
        directories
            .entry(key)
            .or_insert_with(|| Arc::new(AMutex::new(TrajectoryIndexDirectoryState::default())))
            .clone()
    }

    async fn ensure_loaded(
        &self,
        dir: &Path,
        source_hint: Option<TrajectorySourceIdentity>,
    ) -> Result<Arc<AMutex<TrajectoryIndexDirectoryState>>, String> {
        let state = self.directory_state(dir).await;
        if state.lock().await.loaded {
            return Ok(state);
        }

        let dir = dir.to_path_buf();
        let loaded = tokio::task::spawn_blocking(move || {
            load_trajectory_index_for_coordinator_sync(&dir, source_hint)
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
            state_guard.loaded = true;
            if loaded.needs_flush {
                push_pending_mutation(
                    &mut state_guard,
                    TrajectoryIndexMutation::Reconcile(loaded.source_hint),
                );
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
        let state = self.directory_state(dir).await;
        let mut state_guard = state.lock().await;
        if let Some(index) = state_guard.index.as_mut() {
            apply_mutation_to_index(index, &TrajectoryIndexMutation::Upsert(entry.clone()))?;
        }
        push_pending_mutation(&mut state_guard, TrajectoryIndexMutation::Upsert(entry));
        Ok(())
    }

    pub async fn remove(&self, dir: &Path, chat_ids: HashSet<String>) -> Result<(), String> {
        if chat_ids.is_empty() {
            return Ok(());
        }
        let state = self.directory_state(dir).await;
        let mut state_guard = state.lock().await;
        if let Some(index) = state_guard.index.as_mut() {
            apply_mutation_to_index(index, &TrajectoryIndexMutation::Remove(chat_ids.clone()))?;
        }
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
        let dir = dir.to_path_buf();
        let source_hint_for_reconcile = source_hint.clone();
        let reconciled = tokio::task::spawn_blocking(move || {
            reconcile_trajectory_index_sync(&dir, index, source_hint_for_reconcile)
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
        state_guard.last_reconciled_at = Some(Instant::now());
        if reconciled.1 {
            push_pending_mutation(
                &mut state_guard,
                TrajectoryIndexMutation::Reconcile(source_hint),
            );
        }
        Ok(index)
    }

    pub async fn list_entries(
        &self,
        dir: &Path,
        source_hint: Option<TrajectorySourceIdentity>,
    ) -> Result<Vec<TrajectoryIndexEntry>, String> {
        let state = self.ensure_loaded(dir, source_hint.clone()).await?;
        let reconcile_due = {
            let state_guard = state.lock().await;
            state_guard
                .last_reconciled_at
                .is_none_or(|last| last.elapsed() >= COORDINATOR_RECONCILE_INTERVAL)
        };
        if reconcile_due {
            return Ok(self.reconcile(dir, source_hint).await?.entries);
        }
        let entries = state
            .lock()
            .await
            .index
            .as_ref()
            .ok_or_else(|| "Trajectory index coordinator did not load an index".to_string())?
            .entries
            .clone();
        Ok(entries)
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
        let flushed_index = tokio::task::spawn_blocking(move || {
            flush_trajectory_index_mutations_sync(&dir, pending, lock_timeout, lock_retry)
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
}

pub fn trajectory_index_path(dir: &Path) -> PathBuf {
    dir.join(TRAJECTORY_INDEX_FILE)
}

pub fn trajectory_file_path_for_entry(dir: &Path, entry: &TrajectoryIndexEntry) -> PathBuf {
    dir.join(&entry.file_name)
}

struct CoordinatorLoadResult {
    index: TrajectoryIndex,
    needs_flush: bool,
    source_hint: Option<TrajectorySourceIdentity>,
}

fn load_trajectory_index_for_coordinator_sync(
    dir: &Path,
    source_hint: Option<TrajectorySourceIdentity>,
) -> Result<CoordinatorLoadResult, String> {
    match read_trajectory_index_sync(dir) {
        Ok(Some(index)) => Ok(CoordinatorLoadResult {
            index,
            needs_flush: false,
            source_hint,
        }),
        Ok(None) | Err(_) => {
            let (entries, skipped_files) =
                scan_trajectory_index_data_sync(dir, source_hint.clone())?;
            Ok(CoordinatorLoadResult {
                index: TrajectoryIndex {
                    schema_version: TRAJECTORY_INDEX_SCHEMA_VERSION,
                    updated_at: Utc::now().to_rfc3339(),
                    entries,
                    skipped_files,
                },
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
) -> Result<(TrajectoryIndex, bool), String> {
    let disk_files = scan_trajectory_dir_files_sync(dir)?;
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
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let lock_path = dir.join(format!(".{TRAJECTORY_INDEX_FILE}.lock"));
    let mut lock = crate::daemon::lock::open_lock(&lock_path).map_err(|error| {
        format!(
            "Failed to open trajectory index lock {:?}: {error}",
            lock_path
        )
    })?;
    let deadline = Instant::now() + timeout;
    loop {
        match crate::daemon::lock::try_lock(&mut lock) {
            Ok(_guard) => return operation(),
            Err(error)
                if crate::daemon::lock::is_already_locked(&error) && Instant::now() < deadline =>
            {
                std::thread::sleep(retry);
            }
            Err(error) if crate::daemon::lock::is_already_locked(&error) => {
                return Err(format!(
                    "Timed out waiting for trajectory index lock {:?}",
                    lock_path
                ));
            }
            Err(error) => {
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
) -> Result<TrajectoryIndex, String> {
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("Failed to create trajectory directory {:?}: {error}", dir))?;
    with_trajectory_index_file_lock(dir, lock_timeout, lock_retry, || {
        let mut index = load_trajectory_index_for_coordinator_sync(dir, None)?.index;
        for pending in pending {
            match pending.mutation {
                TrajectoryIndexMutation::Reconcile(source_hint) => {
                    index = reconcile_trajectory_index_sync(dir, index, source_hint)?.0;
                }
                mutation => apply_mutation_to_index(&mut index, &mutation)?,
            }
        }
        write_trajectory_index_atomic_sync(dir, &index)?;
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
    if !perf_diagnostics::is_enabled() {
        return read_trajectory_index_inner(dir).await;
    }
    let path = trajectory_index_path(dir);
    let span = perf_diagnostics::span(PerfComponent::TrajectoryIndexRead, None, Some(&path));
    let result = read_trajectory_index_inner(dir).await;
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

async fn read_trajectory_index_inner(dir: &Path) -> Result<Option<TrajectoryIndex>, String> {
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
    if !perf_diagnostics::is_enabled() {
        return write_trajectory_index_atomic_owned_inner(dir, index).await;
    }
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
}

async fn write_trajectory_index_atomic_owned_inner(
    dir: &Path,
    index: TrajectoryIndex,
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
    fs::write(&tmp_path, content).await.map_err(|e| {
        format!(
            "Failed to write temporary trajectory index {:?}: {e}",
            tmp_path
        )
    })?;
    crate::chat::trajectories::atomic_write_file(&tmp_path, &path).await
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
    if trajectory_index_coordinator_rollout_enabled() {
        coordinator.list_entries(dir, source_hint).await
    } else {
        list_trajectory_entries_from_index_or_rebuild(dir, source_hint).await
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

fn scan_trajectory_dir_files_sync(dir: &Path) -> Result<Vec<DiskTrajectoryFile>, String> {
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
) -> Result<(Vec<TrajectoryIndexEntry>, Vec<TrajectoryIndexSkippedFile>), String> {
    let disk_files = scan_trajectory_dir_files_sync(dir)?;
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

async fn scan_trajectory_dir_files(dir: &Path) -> Result<Vec<DiskTrajectoryFile>, String> {
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
    let disk_files = scan_trajectory_dir_files(dir).await?;
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
    let disk_files = scan_trajectory_dir_files(dir).await?;

    let (existing_entries, existing_skipped, index_unreadable) =
        match read_trajectory_index(dir).await {
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

    if content_changed {
        let index = TrajectoryIndex {
            schema_version: TRAJECTORY_INDEX_SCHEMA_VERSION,
            updated_at: Utc::now().to_rfc3339(),
            entries: new_entries.clone(),
            skipped_files: new_skipped,
        };
        write_trajectory_index_atomic_owned(dir, index).await?;
    }

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

    #[serial_test::serial]
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
