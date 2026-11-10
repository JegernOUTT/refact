use axum::extract::Path as AxumPath;
use axum::extract::State;
use axum::http::{Response, StatusCode};
use futures::FutureExt;
use hyper::Body;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use refact_chat_api::FrozenRequestPrefix;
use serde::de::{Deserializer as _, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::fs;
use tokio::sync::{broadcast, mpsc, Mutex as AMutex, Notify};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::call_validation::{ChatContent, ChatMessage};
use crate::custom_error::ScratchError;
use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
use crate::subchat::run_subchat_once;
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::WorktreeMeta;
use crate::yaml_configs::customization_registry::get_subagent_config;

use super::perf_diagnostics::{self, PerfComponent, PerfOutcome};
use refact_chat_api::{GoalLedgerEntry, GoalSnapshot, GoalStatus, PendingQuestionSet};

pub async fn atomic_write_file(tmp_path: &Path, dest_path: &Path) -> Result<(), String> {
    super::trajectory_archive::with_trajectory_mutation_lock(atomic_write_file_coordinated(
        tmp_path, dest_path,
    ))
    .await
}

async fn atomic_write_file_coordinated(tmp_path: &Path, dest_path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        if fs::try_exists(dest_path)
            .await
            .map_err(|e| format!("Failed to inspect destination file: {}", e))?
        {
            let backup_extension = format!(
                "{}.replace.{}",
                dest_path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("tmp"),
                Uuid::new_v4().simple()
            );
            let backup_path = dest_path.with_extension(backup_extension);
            fs::rename(dest_path, &backup_path)
                .await
                .map_err(|e| format!("Failed to move existing file aside: {}", e))?;
            match fs::rename(tmp_path, dest_path).await {
                Ok(()) => {
                    let _ = fs::remove_file(&backup_path).await;
                    return Ok(());
                }
                Err(e) => {
                    let _ = fs::rename(&backup_path, dest_path).await;
                    return Err(format!("Failed to rename: {}", e));
                }
            }
        }
    }
    fs::rename(tmp_path, dest_path)
        .await
        .map_err(|e| format!("Failed to rename: {}", e))
}

fn unique_trajectory_tmp_path(file_path: &Path) -> PathBuf {
    let random = Uuid::new_v4().simple().to_string();
    file_path.with_extension(format!("json.tmp.{}", &random[..8]))
}

async fn atomic_write_json_with_tmp_path(
    path: &Path,
    tmp_path: &Path,
    json_result: Result<String, String>,
    write_error_prefix: Option<&str>,
    self_write_source: Option<TrajectorySourceIdentity>,
) -> Result<(), String> {
    let write_result = async {
        let json = json_result?;
        fs::write(tmp_path, &json).await.map_err(|error| {
            write_error_prefix
                .map(|prefix| format!("{}: {}", prefix, error))
                .unwrap_or_else(|| error.to_string())
        })?;
        Ok(TrajectoryPrettyWrite {
            byte_len: json.len() as u64,
            content_hash: md5::compute(&json).0,
            source_sha256: trajectory_index::source_sha256(json.as_bytes()),
        })
    }
    .await;
    atomic_finish_trajectory_write(path, tmp_path, write_result, self_write_source).await
}

struct TrajectoryPrettyWrite {
    byte_len: u64,
    content_hash: [u8; 16],
    source_sha256: [u8; 32],
}

struct TrajectoryPrettyWriter {
    inner: std::io::BufWriter<std::fs::File>,
    context: md5::Context,
    sha256: sha2::Sha256,
    byte_len: u64,
}

impl std::io::Write for TrajectoryPrettyWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.context.consume(&buf[..written]);
        sha2::Digest::update(&mut self.sha256, &buf[..written]);
        self.byte_len += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl TrajectoryPrettyWriter {
    fn finish(mut self) -> Result<TrajectoryPrettyWrite, String> {
        use std::io::Write;
        self.inner
            .flush()
            .map_err(|error| format!("Failed to write trajectory: {}", error))?;
        Ok(TrajectoryPrettyWrite {
            byte_len: self.byte_len,
            content_hash: self.context.compute().0,
            source_sha256: sha2::Digest::finalize(self.sha256).into(),
        })
    }
}

/// Byte-identical to `serde_json::to_string_pretty` (same `PrettyFormatter`), without
/// materialising the whole document as a `String` first. The pretty on-disk layout is a hard
/// product constraint; `pretty_output_bytes_are_unchanged` pins it.
fn write_pretty_json_to_tmp_file(
    tmp_path: &Path,
    value: &serde_json::Value,
) -> Result<TrajectoryPrettyWrite, String> {
    let file = std::fs::File::create(tmp_path)
        .map_err(|error| format!("Failed to write trajectory: {}", error))?;
    let mut writer = TrajectoryPrettyWriter {
        inner: std::io::BufWriter::new(file),
        context: md5::Context::new(),
        sha256: <sha2::Sha256 as sha2::Digest>::new(),
        byte_len: 0,
    };
    serde_json::to_writer_pretty(&mut writer, value)
        .map_err(|error| format!("Failed to serialize trajectory: {}", error))?;
    writer.finish()
}

async fn atomic_finish_trajectory_write(
    path: &Path,
    tmp_path: &Path,
    write_result: Result<TrajectoryPrettyWrite, String>,
    self_write_source: Option<TrajectorySourceIdentity>,
) -> Result<(), String> {
    let result = async {
        let write = write_result?;
        let self_write = match self_write_source {
            Some(source) if trajectory_watcher_self_write_rollout_enabled() => {
                register_trajectory_self_write(path, source, write.byte_len, write.content_hash)
                    .await
            }
            _ => None,
        };
        if let Err(error) = atomic_write_file(tmp_path, path).await {
            discard_trajectory_self_write(self_write);
            return Err(error);
        }
        complete_trajectory_self_write(self_write).await;
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_file(tmp_path).await;
    }
    result
}

use super::config::timeouts;
use super::session::has_displayable_assistant_content;
use super::trajectory_index;
use super::types::{
    ChatSession, ExternalReloadPending, GoalSnapshotBudgetExt, SessionState, TaskMeta,
    ThreadParams, TrajectoryCommitIntent, TrajectorySourceIdentity,
    TrajectorySourceIdentitySessionExt,
};
use super::SessionsMap;

const TITLE_GENERATION_SUBAGENT_ID: &str = "title_generation";
pub const TRAJECTORY_WRITER_ENV: &str = "REFACT_TRAJECTORY_WRITER";
pub const TRAJECTORY_WATCHER_SELF_WRITE_ENV: &str = "REFACT_TRAJECTORY_WATCHER_SELF_WRITE";
const TRAJECTORY_FLUSH_TIMEOUT: Duration = Duration::from_secs(5);
const TRAJECTORY_COMMIT_STALL_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a `Checkpoint` save may sit in the pending slot before it is written. Every save
/// rewrites the whole trajectory, so batching a burst into one physical write is what keeps the
/// cost from growing quadratically with conversation length. Durability-critical intents bypass it.
const TRAJECTORY_CHECKPOINT_COALESCE_WINDOW: Duration = Duration::from_millis(400);
#[cfg(test)]
const TITLE_GENERATION_LLM_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);
#[cfg(not(test))]
const TITLE_GENERATION_LLM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const TRAJECTORY_META_TITLE_MAX_CHARS: usize = 120;

pub const TRAJECTORY_LABEL_MAX_CHARS: usize = TRAJECTORY_META_TITLE_MAX_CHARS;

pub use refact_chat_history::trajectory_event::TrajectoryEvent;

static TRAJECTORY_WRITER_ROLLOUT: OnceLock<bool> = OnceLock::new();

fn rollout_switch_enabled<F>(cache: &OnceLock<bool>, read: F) -> bool
where
    F: FnOnce() -> bool,
{
    *cache.get_or_init(read)
}

pub fn trajectory_writer_rollout_enabled() -> bool {
    #[cfg(any(test, feature = "bench"))]
    if let Some(enabled) = *trajectory_writer_rollout_override()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
    {
        return enabled;
    }
    rollout_switch_enabled(&TRAJECTORY_WRITER_ROLLOUT, || {
        trajectory_writer_rollout_enabled_for(
            std::env::var(TRAJECTORY_WRITER_ENV)
                .ok()
                .as_deref()
                .or_else(|| {
                    crate::runtime_settings::current()
                        .trajectory_writer_enabled
                        .then_some("1")
                }),
        )
    })
}

pub(crate) fn trajectory_writer_rollout_enabled_for(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        let value = value.trim();
        value == "1"
            || value.eq_ignore_ascii_case("true")
            || value.eq_ignore_ascii_case("yes")
            || value.eq_ignore_ascii_case("on")
    })
}

#[cfg(any(test, feature = "bench"))]
fn trajectory_writer_rollout_override() -> &'static StdMutex<Option<bool>> {
    static OVERRIDE: OnceLock<StdMutex<Option<bool>>> = OnceLock::new();
    OVERRIDE.get_or_init(|| StdMutex::new(None))
}

#[cfg(any(test, feature = "bench"))]
pub(crate) struct TrajectoryWriterRolloutGuard(Option<bool>);

#[cfg(any(test, feature = "bench"))]
impl TrajectoryWriterRolloutGuard {
    pub(crate) fn set(enabled: bool) -> Self {
        let mut override_value = trajectory_writer_rollout_override()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = *override_value;
        *override_value = Some(enabled);
        Self(previous)
    }
}

#[cfg(any(test, feature = "bench"))]
impl Drop for TrajectoryWriterRolloutGuard {
    fn drop(&mut self) {
        *trajectory_writer_rollout_override()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = self.0.take();
    }
}

pub fn trajectory_watcher_self_write_rollout_enabled() -> bool {
    trajectory_watcher_self_write_rollout_enabled_for(
        std::env::var(TRAJECTORY_WATCHER_SELF_WRITE_ENV)
            .ok()
            .as_deref()
            .or_else(|| {
                crate::runtime_settings::current()
                    .trajectory_watcher_self_write_enabled
                    .then_some("1")
            }),
    )
}

pub(crate) fn trajectory_watcher_self_write_rollout_enabled_for(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        let value = value.trim();
        value == "1"
            || value.eq_ignore_ascii_case("true")
            || value.eq_ignore_ascii_case("yes")
            || value.eq_ignore_ascii_case("on")
    })
}

const TRAJECTORY_SELF_WRITE_TTL: Duration = Duration::from_secs(10);
const TRAJECTORY_SELF_WRITE_MAX_RECORDS: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
struct TrajectoryWriteFingerprint {
    file_len: u64,
    file_modified_unix_ms: i64,
    content_hash: [u8; 16],
}

#[derive(Clone)]
struct TrajectorySelfWriteHandle {
    key: (PathBuf, TrajectorySourceIdentity),
    nonce: Uuid,
    byte_len: u64,
    content_hash: [u8; 16],
}

struct TrajectorySelfWriteRecord {
    nonce: Uuid,
    created_at: Instant,
    fingerprint: Option<TrajectoryWriteFingerprint>,
}

type TrajectorySelfWriteRegistry = std::collections::HashMap<
    (PathBuf, TrajectorySourceIdentity),
    std::collections::VecDeque<TrajectorySelfWriteRecord>,
>;

fn trajectory_self_writes() -> &'static StdMutex<TrajectorySelfWriteRegistry> {
    static WRITES: OnceLock<StdMutex<TrajectorySelfWriteRegistry>> = OnceLock::new();
    WRITES.get_or_init(|| StdMutex::new(std::collections::HashMap::new()))
}

fn watcher_trajectory_source(source: &TrajectorySourceIdentity) -> TrajectorySourceIdentity {
    match source {
        TrajectorySourceIdentity::Task {
            task_id,
            role,
            agent_id,
            ..
        } => TrajectorySourceIdentity::task(
            task_id.clone(),
            role.clone(),
            agent_id.clone(),
            None,
            None,
        ),
        source => source.clone(),
    }
}

fn prune_trajectory_self_writes(registry: &mut TrajectorySelfWriteRegistry, now: Instant) {
    registry.retain(|_, records| {
        records.retain(|record| now.duration_since(record.created_at) < TRAJECTORY_SELF_WRITE_TTL);
        !records.is_empty()
    });
    while registry
        .values()
        .map(|records| records.len())
        .sum::<usize>()
        > TRAJECTORY_SELF_WRITE_MAX_RECORDS
    {
        let oldest = registry
            .iter()
            .filter_map(|(key, records)| {
                records
                    .front()
                    .map(|record| (key.clone(), record.created_at))
            })
            .min_by_key(|(_, created_at)| *created_at);
        let Some((key, _)) = oldest else {
            break;
        };
        if let Some(records) = registry.get_mut(&key) {
            records.pop_front();
            if records.is_empty() {
                registry.remove(&key);
            }
        }
    }
}

async fn canonical_trajectory_write_path(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    let file_name = path.file_name()?;
    let parent = fs::canonicalize(parent).await.ok()?;
    Some(dunce::simplified(&parent).join(file_name))
}

/// Per-path self-write content-read counters, used only by tests. Keyed by path for the same
/// reason as the metadata-read counters below: the reads happen on detached writer and watcher
/// tasks, where a task-local scope would never observe them.
#[cfg(test)]
fn test_trajectory_self_write_read_counters(
) -> &'static StdMutex<std::collections::HashMap<PathBuf, Weak<AtomicUsize>>> {
    static COUNTERS: OnceLock<StdMutex<std::collections::HashMap<PathBuf, Weak<AtomicUsize>>>> =
        OnceLock::new();
    COUNTERS.get_or_init(|| StdMutex::new(std::collections::HashMap::new()))
}

#[cfg(test)]
fn count_test_trajectory_self_write_reads(path: &Path) -> Arc<AtomicUsize> {
    let reads = Arc::new(AtomicUsize::new(0));
    let mut counters = test_trajectory_self_write_read_counters()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    counters.retain(|_, counter| counter.strong_count() > 0);
    counters.insert(trajectory_metadata_cache_key(path), Arc::downgrade(&reads));
    reads
}

/// The only place the self-write machinery reads trajectory bytes back off disk, and it sits on
/// the watcher side: the write path fingerprints from the hash streamed while writing.
async fn read_trajectory_file_for_fingerprint(path: &Path) -> Option<Vec<u8>> {
    #[cfg(test)]
    if let Some(reads) = test_trajectory_self_write_read_counters()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&trajectory_metadata_cache_key(path))
        .and_then(Weak::upgrade)
    {
        reads.fetch_add(1, Ordering::Relaxed);
    }
    fs::read(path).await.ok()
}

async fn trajectory_fingerprint_matches(
    path: &Path,
    expected: &TrajectoryWriteFingerprint,
) -> bool {
    let before = match fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => metadata,
        _ => return false,
    };
    let before_modified = match before
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
    {
        Some(modified) => modified,
        None => return false,
    };
    if before.len() != expected.file_len || before_modified != expected.file_modified_unix_ms {
        return false;
    }
    let content = match read_trajectory_file_for_fingerprint(path).await {
        Some(content) if md5::compute(&content).0 == expected.content_hash => content,
        _ => return false,
    };
    let after = match fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => metadata,
        _ => return false,
    };
    let after_modified = match after
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
    {
        Some(modified) => modified,
        None => return false,
    };
    after.len() == before.len()
        && after_modified == before_modified
        && content.len() as u64 == after.len()
}

async fn register_trajectory_self_write(
    path: &Path,
    source: TrajectorySourceIdentity,
    byte_len: u64,
    content_hash: [u8; 16],
) -> Option<TrajectorySelfWriteHandle> {
    let key = (
        canonical_trajectory_write_path(path).await?,
        watcher_trajectory_source(&source),
    );
    let nonce = Uuid::new_v4();
    let mut registry = trajectory_self_writes()
        .lock()
        .expect("trajectory self-write registry poisoned");
    prune_trajectory_self_writes(&mut registry, Instant::now());
    registry
        .entry(key.clone())
        .or_default()
        .push_back(TrajectorySelfWriteRecord {
            nonce,
            created_at: Instant::now(),
            fingerprint: None,
        });
    prune_trajectory_self_writes(&mut registry, Instant::now());
    Some(TrajectorySelfWriteHandle {
        key,
        nonce,
        byte_len,
        content_hash,
    })
}

fn discard_trajectory_self_write(handle: Option<TrajectorySelfWriteHandle>) {
    let Some(handle) = handle else {
        return;
    };
    let mut registry = trajectory_self_writes()
        .lock()
        .expect("trajectory self-write registry poisoned");
    if let Some(records) = registry.get_mut(&handle.key) {
        records.retain(|record| record.nonce != handle.nonce);
        if records.is_empty() {
            registry.remove(&handle.key);
        }
    }
}

/// Stamps the renamed file with a single `stat`. The content hash was computed once while the
/// bytes streamed to the temp file, so nothing has to be read back here; a size that disagrees
/// with what we wrote means somebody replaced the file already, and that record is dropped rather
/// than left able to suppress their change.
async fn complete_trajectory_self_write(handle: Option<TrajectorySelfWriteHandle>) {
    let Some(handle) = handle else {
        return;
    };
    let fingerprint = trajectory_file_stamp(&handle.key.0)
        .await
        .filter(|stamp| stamp.len == handle.byte_len)
        .map(|stamp| TrajectoryWriteFingerprint {
            file_len: stamp.len,
            file_modified_unix_ms: stamp.modified_unix_ms,
            content_hash: handle.content_hash,
        });
    let mut registry = trajectory_self_writes()
        .lock()
        .expect("trajectory self-write registry poisoned");
    prune_trajectory_self_writes(&mut registry, Instant::now());
    let Some(records) = registry.get_mut(&handle.key) else {
        return;
    };
    match fingerprint {
        Some(fingerprint) => {
            if let Some(record) = records
                .iter_mut()
                .find(|record| record.nonce == handle.nonce)
            {
                record.fingerprint = Some(fingerprint);
            }
        }
        None => records.retain(|record| record.nonce != handle.nonce),
    }
    if records.is_empty() {
        registry.remove(&handle.key);
    }
}

async fn consume_trajectory_self_write(path: &Path, source: &TrajectorySourceIdentity) -> bool {
    let Some(path) = canonical_trajectory_write_path(path).await else {
        return false;
    };
    let key = (path, watcher_trajectory_source(source));
    let fingerprints = {
        let mut registry = trajectory_self_writes()
            .lock()
            .expect("trajectory self-write registry poisoned");
        prune_trajectory_self_writes(&mut registry, Instant::now());
        registry
            .get(&key)
            .map(|records| {
                records
                    .iter()
                    .filter_map(|record| record.fingerprint.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let matched = futures::future::join_all(
        fingerprints
            .iter()
            .map(|fingerprint| trajectory_fingerprint_matches(&key.0, fingerprint)),
    )
    .await
    .into_iter()
    .position(|matches| matches);
    let Some(matched) = matched else {
        return false;
    };
    let mut registry = trajectory_self_writes()
        .lock()
        .expect("trajectory self-write registry poisoned");
    prune_trajectory_self_writes(&mut registry, Instant::now());
    let Some(records) = registry.get_mut(&key) else {
        return false;
    };
    if let Some(position) = records
        .iter()
        .position(|record| record.fingerprint.as_ref() == fingerprints.get(matched))
    {
        records.remove(position);
        if records.is_empty() {
            registry.remove(&key);
        }
        true
    } else {
        false
    }
}

fn clear_trajectory_self_writes(watched_roots: &[PathBuf]) {
    let mut registry = trajectory_self_writes()
        .lock()
        .expect("trajectory self-write registry poisoned");
    prune_trajectory_self_writes(&mut registry, Instant::now());
    registry.retain(|(path, _), _| !watched_roots.iter().any(|root| path.starts_with(root)));
}

struct TrajectorySelfWriteScope {
    watched_roots: Vec<PathBuf>,
}

impl Drop for TrajectorySelfWriteScope {
    fn drop(&mut self) {
        clear_trajectory_self_writes(&self.watched_roots);
    }
}

#[cfg(test)]
fn clear_all_trajectory_self_writes() {
    trajectory_self_writes()
        .lock()
        .expect("trajectory self-write registry poisoned")
        .clear();
}

pub async fn get_session_state_for_chat(
    sessions: &SessionsMap,
    chat_id: &str,
) -> (String, Option<String>) {
    let session_arc = sessions.read().await.get(chat_id).cloned();
    match session_arc {
        Some(arc) => {
            let session = arc.lock().await;
            (
                session.runtime.state.to_string(),
                session.runtime.error.clone(),
            )
        }
        None => (SessionState::Idle.to_string(), None),
    }
}

async fn get_session_runtime_for_trajectory_source(
    sessions: &SessionsMap,
    chat_id: &str,
    source: &TrajectorySourceIdentity,
) -> (String, Option<String>, Option<WorktreeMeta>) {
    let session_arc = sessions.read().await.get(chat_id).cloned();
    match session_arc {
        Some(arc) => {
            let session = arc.lock().await;
            if !source.emits_generic_event() || !source.matches_session(&session) {
                return (SessionState::Idle.to_string(), None, None);
            }
            (
                session.runtime.state.to_string(),
                session.runtime.error.clone(),
                session.thread.worktree.clone(),
            )
        }
        None => (SessionState::Idle.to_string(), None, None),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrajectoryMeta {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub model: String,
    pub mode: String,
    pub message_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_chat_id: Option<String>,
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
    #[serde(skip)]
    pub(crate) source: TrajectorySourceIdentity,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TrajectoryData {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub model: String,
    pub mode: String,
    pub tool_use: String,
    pub messages: Vec<serde_json::Value>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub(crate) struct TrajectoryListData {
    pub(crate) id: String,
    pub(crate) updated_at: String,
    pub(crate) mode: Option<String>,
    #[serde(flatten)]
    pub(crate) extra: serde_json::Map<String, serde_json::Value>,
}

pub(crate) struct TrajectoryListCandidate {
    pub(crate) id: String,
    pub(crate) updated_at: String,
    pub(crate) path: PathBuf,
    pub(crate) indexed_meta: Option<TrajectoryMeta>,
    pub(crate) indexed_file_len: Option<u64>,
    pub(crate) indexed_file_modified_unix_ms: Option<i64>,
}

fn trajectory_list_main_link_type(link_type: Option<&str>) -> bool {
    matches!(link_type, Some("handoff" | "mode_transition" | "branch"))
}

pub fn trajectory_event_is_displayable_chat(event: &TrajectoryEvent) -> bool {
    if matches!(
        event.mode.as_deref(),
        Some("task_agent" | "task_planner" | "buddy")
    ) {
        return false;
    }
    if is_internal_trace_link_type(event.link_type.as_deref()) {
        return false;
    }
    if (event.link_type.is_some() || event.parent_id.is_some())
        && !trajectory_list_main_link_type(event.link_type.as_deref())
    {
        return false;
    }

    true
}

pub(crate) fn trajectory_list_data_is_displayable_chat(data: &TrajectoryListData) -> bool {
    if data.extra.get("buddy_meta").is_some_and(|v| !v.is_null()) {
        return false;
    }
    if matches!(data.mode.as_deref(), Some("task_agent" | "task_planner")) {
        return false;
    }

    let parent_id = data.extra.get("parent_id").and_then(|v| v.as_str());
    let link_type = data.extra.get("link_type").and_then(|v| v.as_str());
    if is_internal_trace_link_type(link_type) {
        return false;
    }
    if (link_type.is_some() || parent_id.is_some()) && !trajectory_list_main_link_type(link_type) {
        return false;
    }

    true
}

#[derive(Clone)]
pub struct LoadedTrajectory {
    pub recovery: Option<refact_core::recovery::RecoveryRecord>,
    pub source_path: PathBuf,
    pub messages: Vec<ChatMessage>,
    pub thread: ThreadParams,
    pub goal: Option<GoalSnapshot>,
    pub goal_ledger: Vec<GoalLedgerEntry>,
    pub goal_verification_blocked_until_ms: Option<u64>,
    pub pending_questions: Vec<PendingQuestionSet>,
    pub native_decision_receipts: Vec<serde_json::Value>,
    pub pending_browser_decision: Option<super::browser_decisions::PendingBrowserDecisionSnapshot>,
    pub created_at: String,
    pub updated_at: String,
    pub wake_up_at: Option<chrono::DateTime<chrono::Utc>>,
    pub waiting_for_card_ids: Vec<String>,
    pub pending_deliveries: Vec<refact_core::chat_types::PendingDelivery>,
    pub dropped_deliveries: Vec<refact_core::chat_types::DeliveryStatusRecord>,
    pub auto_approve_editing_tools_present: bool,
    pub auto_approve_dangerous_commands_present: bool,
    pub transition_identity_repaired: bool,
}

#[derive(Clone)]
pub(crate) struct TrajectoryRepairPatch {
    pub chat_id: String,
    pub source_path: PathBuf,
    pub created_at: String,
    pub frozen_request_prefix: Option<FrozenRequestPrefix>,
    pub auto_approve_editing_tools: bool,
    pub auto_approve_dangerous_commands: bool,
}

impl LoadedTrajectory {
    pub(crate) fn repair_patch(&self) -> TrajectoryRepairPatch {
        TrajectoryRepairPatch {
            chat_id: self.thread.id.clone(),
            source_path: self.source_path.clone(),
            created_at: self.created_at.clone(),
            frozen_request_prefix: self.thread.frozen_request_prefix.clone(),
            auto_approve_editing_tools: self.thread.auto_approve_editing_tools,
            auto_approve_dangerous_commands: self.thread.auto_approve_dangerous_commands,
        }
    }
}

pub use refact_chat_history::trajectory_snapshot::TrajectorySnapshot;

pub(super) fn retain_in_snapshot(message: &ChatMessage) -> bool {
    message.role != "assistant" || has_displayable_assistant_content(message)
}

pub(crate) fn trajectory_snapshot_from_session(session: &ChatSession) -> TrajectorySnapshot {
    let span = perf_diagnostics::span(
        PerfComponent::TrajectorySnapshot,
        Some(&session.chat_id),
        None,
    );
    let messages = session
        .messages
        .iter()
        .filter(|message| retain_in_snapshot(message))
        .cloned()
        .collect();

    let mut snapshot = TrajectorySnapshot::from_thread_parts(
        session.chat_id.clone(),
        &session.thread,
        messages,
        session.created_at.clone(),
        session.trajectory_version,
    );
    snapshot.recovery = session.runtime.recovery.clone();
    snapshot.wake_up_at = session.wake_up_at;
    snapshot.waiting_for_card_ids = session.waiting_for_card_ids.clone();
    snapshot.goal = session.goal.clone();
    snapshot.goal_ledger = session.goal_ledger.clone();
    snapshot.goal_verification_blocked_until_ms = session.goal_verification_blocked_until_ms;
    snapshot.pending_questions = session.pending_questions.clone();
    snapshot.native_decision_receipts = session.native_decision_receipts.clone();
    snapshot.pending_browser_decision = session.pending_browser_message.as_ref().map(|pending| {
        super::browser_decisions::snapshot_pending_browser_decision(
            pending,
            session
                .thread
                .browser_meta
                .as_ref()
                .and_then(|meta| meta.browser_runtime_id.clone()),
        )
    });
    snapshot.pending_deliveries = session.pending_deliveries_for_snapshot();
    snapshot.dropped_deliveries = session.dropped_deliveries_for_snapshot();
    span.finish(
        PerfOutcome::Success,
        None,
        Some(snapshot.messages.len() as u64),
        Some(session.trajectory_version),
        Some(session.command_queue.len() as u64),
    );
    snapshot
}

fn clamp_goal_snapshot_for_load(mut goal: GoalSnapshot) -> GoalSnapshot {
    goal.budget = goal.budget.migrate_legacy_default_hard_limits();
    if let Some(max_turns) = goal.budget.max_turns.filter(|limit| *limit > 0) {
        goal.progress.turns_used = goal.progress.turns_used.min(max_turns);
    }
    if let Some(max_tokens) = goal.budget.max_tokens.filter(|limit| *limit > 0) {
        goal.progress.tokens_used = goal.progress.tokens_used.min(max_tokens);
    }
    if let Some(no_progress_turns) = goal.budget.no_progress_turns.filter(|limit| *limit > 0) {
        goal.progress.no_progress_turns = goal.progress.no_progress_turns.min(no_progress_turns);
    }
    if goal.status == GoalStatus::Verifying {
        goal.status = if goal.active {
            GoalStatus::Active
        } else {
            GoalStatus::Paused
        };
    }
    if matches!(
        goal.status,
        GoalStatus::BudgetExhausted | GoalStatus::NoProgress
    ) && goal
        .goal_budget_exhaustion_status_at(epoch_ms_now())
        .is_none()
        && !goal_budget_has_hard_limits_for_load(&goal.budget)
    {
        goal.status = if goal.active {
            GoalStatus::Active
        } else {
            GoalStatus::Paused
        };
    }
    goal
}

fn goal_budget_has_hard_limits_for_load(budget: &refact_chat_api::GoalBudget) -> bool {
    budget.max_turns.is_some_and(|limit| limit > 0)
        || budget.max_minutes.is_some_and(|limit| limit > 0)
        || budget.max_tokens.is_some_and(|limit| limit > 0)
        || budget.max_cost_cents.is_some_and(|limit| limit > 0)
        || budget.no_progress_turns.is_some_and(|limit| limit > 0)
}

fn epoch_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub async fn apply_mode_defaults_to_thread(
    gcx: Arc<GlobalContext>,
    thread: &mut ThreadParams,
    auto_approve_editing_present: bool,
    auto_approve_dangerous_present: bool,
) {
    if auto_approve_editing_present && auto_approve_dangerous_present {
        return;
    }
    if let Some(mode_config) = crate::yaml_configs::customization_registry::get_mode_config(
        gcx.clone(),
        &thread.mode,
        None,
    )
    .await
    {
        let defaults = &mode_config.thread_defaults;
        if !auto_approve_editing_present {
            if let Some(v) = defaults.auto_approve_editing_tools {
                thread.auto_approve_editing_tools = v;
            }
        }
        if !auto_approve_dangerous_present {
            if let Some(v) = defaults.auto_approve_dangerous_commands {
                thread.auto_approve_dangerous_commands = v;
            }
        }
    }
}

pub async fn get_trajectories_dir(gcx: Arc<GlobalContext>) -> Result<PathBuf, String> {
    let project_dirs = get_project_dirs(gcx).await;
    let workspace_root = project_dirs.first().ok_or("No workspace folder found")?;
    Ok(workspace_root.join(".refact").join("trajectories"))
}

pub async fn get_global_trajectories_dir(gcx: Arc<GlobalContext>) -> PathBuf {
    let app = AppState::from_gcx(gcx).await;
    let config_dir = app.paths.config_dir.clone();
    config_dir.join("trajectories")
}

pub const INTERNAL_TRACE_LINK_PREFIX: &str = "internal:";
pub const UNATTRIBUTED_TRACES_DIR: &str = "internal";
const PRUNE_THROTTLE_MAX_ENTRIES: usize = 256;

#[derive(Default)]
struct PruneThrottle {
    entries: std::collections::HashMap<PathBuf, PruneThrottleEntry>,
    next_access: u64,
}

struct PruneThrottleEntry {
    last_pruned_unix_secs: u64,
    last_access: u64,
}

impl PruneThrottle {
    fn claim(&mut self, dir: &Path, now: u64, interval_secs: u64) -> bool {
        self.next_access = self.next_access.saturating_add(1);
        if let Some(entry) = self.entries.get_mut(dir) {
            if now.saturating_sub(entry.last_pruned_unix_secs) < interval_secs {
                entry.last_access = self.next_access;
                return false;
            }
            entry.last_pruned_unix_secs = now;
            entry.last_access = self.next_access;
            return true;
        }
        if self.entries.len() >= PRUNE_THROTTLE_MAX_ENTRIES {
            if let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_access)
                .map(|(path, _)| path.clone())
            {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(
            dir.to_path_buf(),
            PruneThrottleEntry {
                last_pruned_unix_secs: now,
                last_access: self.next_access,
            },
        );
        true
    }

    fn release(&mut self, dir: &Path, claimed_at_unix_secs: u64) {
        if self
            .entries
            .get(dir)
            .is_some_and(|entry| entry.last_pruned_unix_secs == claimed_at_unix_secs)
        {
            self.entries.remove(dir);
        }
    }
}

fn claim_prune_throttle(
    throttle: &StdMutex<PruneThrottle>,
    dir: &Path,
    now: u64,
    interval_secs: u64,
    label: &str,
) -> Result<bool, String> {
    let mut throttle = throttle
        .lock()
        .map_err(|_| format!("{label} prune throttle lock poisoned"))?;
    Ok(throttle.claim(dir, now, interval_secs))
}

fn release_prune_throttle(
    throttle: &StdMutex<PruneThrottle>,
    dir: &Path,
    claimed_at_unix_secs: u64,
) {
    if let Ok(mut throttle) = throttle.lock() {
        throttle.release(dir, claimed_at_unix_secs);
    }
}

fn internal_trace_prune_throttle() -> &'static StdMutex<PruneThrottle> {
    static LAST_PRUNE_BY_DIR: OnceLock<StdMutex<PruneThrottle>> = OnceLock::new();
    LAST_PRUNE_BY_DIR.get_or_init(|| StdMutex::new(PruneThrottle::default()))
}

const STALE_ENTRY_CLEANUP_INTERVAL_SECS: u64 = 60;

fn stale_entry_cleanup_throttle() -> &'static StdMutex<PruneThrottle> {
    static LAST_CLEANUP_BY_CHAT: OnceLock<StdMutex<PruneThrottle>> = OnceLock::new();
    LAST_CLEANUP_BY_CHAT.get_or_init(|| StdMutex::new(PruneThrottle::default()))
}

fn stale_entry_cleanup_allowed(chat_id: &str) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    claim_prune_throttle(
        stale_entry_cleanup_throttle(),
        Path::new(chat_id),
        now,
        STALE_ENTRY_CLEANUP_INTERVAL_SECS,
        "stale trajectory index entry cleanup",
    )
    .unwrap_or(true)
}

struct TopLevelInternalTraceVisitor;

impl<'de> Visitor<'de> for TopLevelInternalTraceVisitor {
    type Value = bool;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a trajectory JSON object")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut link_type = None;
        let mut link_type_seen = false;
        while let Some(key) = map.next_key::<String>()? {
            if key == "link_type" {
                if link_type_seen {
                    return Err(serde::de::Error::custom("duplicate top-level link_type"));
                }
                link_type_seen = true;
                link_type = map.next_value::<Option<String>>()?;
                if !is_internal_trace_link_type(link_type.as_deref()) {
                    // Early-out: the answer is already pinned to `false` and nothing later in the
                    // document can change it. A well-formed remainder still yields `false`, and a
                    // malformed remainder yields a parse error that the caller also maps to
                    // `false`. So we can stop reading bytes here without changing the result.
                    //
                    // We deliberately do NOT early-out on an *internal* link_type: there the
                    // remainder still matters, because a truncated or otherwise malformed file
                    // must stay classified as "not an internal trace" so that pruning never
                    // deletes a torn write. See
                    // `internal_trace_pruning_ignores_message_content_and_invalid_json`.
                    return Ok(false);
                }
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(is_internal_trace_link_type(link_type.as_deref()))
    }
}

fn top_level_link_type_is_internal_from_reader<R: std::io::Read>(reader: R) -> bool {
    // `serde_json`'s `IoRead` pulls the document through `Read::bytes()`, i.e. one `read()`
    // syscall per byte consumed. Buffering is what makes scanning a multi-megabyte trajectory
    // affordable; without it a single 144 MB folder costs ~144 million syscalls per scan.
    let mut deserializer = serde_json::Deserializer::from_reader(std::io::BufReader::new(reader));
    (&mut deserializer)
        .deserialize_map(TopLevelInternalTraceVisitor)
        .and_then(|is_internal| deserializer.end().map(|_| is_internal))
        .unwrap_or(false)
}

fn parsed_top_level_link_type_is_internal(path: PathBuf) -> bool {
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {}
        _ => return false,
    }
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    top_level_link_type_is_internal_from_reader(file)
}

pub fn is_internal_trace_link_type(link_type: Option<&str>) -> bool {
    link_type.is_some_and(|value| value.starts_with(INTERNAL_TRACE_LINK_PREFIX))
}

fn should_vectorize_trajectory(source: &TrajectorySourceIdentity, link_type: Option<&str>) -> bool {
    !matches!(source, TrajectorySourceIdentity::Buddy) && !is_internal_trace_link_type(link_type)
}

const TRAJECTORY_VECTORIZATION_QUEUE_CAPACITY: usize = 64;

#[derive(Default)]
pub struct TrajectoryVectorizationQueue {
    sender: Option<mpsc::Sender<PathBuf>>,
    worker: Option<tokio::task::JoinHandle<()>>,
}

async fn wait_for_trajectory_vectorization_shutdown(
    shutdown_flag: Arc<std::sync::atomic::AtomicBool>,
) {
    while !shutdown_flag.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn enqueue_trajectory_vectorization(gcx: Arc<GlobalContext>, file_path: PathBuf) {
    if gcx.shutdown_flag.load(Ordering::Relaxed) {
        return;
    }

    let finished_worker = {
        let mut queue = gcx.trajectory_vectorization_queue.lock().await;
        if queue
            .worker
            .as_ref()
            .is_some_and(tokio::task::JoinHandle::is_finished)
        {
            queue.sender = None;
            queue.worker.take()
        } else {
            None
        }
    };
    if let Some(worker) = finished_worker {
        if let Err(error) = worker.await {
            warn!("trajectory VecDB enqueue worker failed: {error}");
        }
    }

    let sender = {
        let mut queue = gcx.trajectory_vectorization_queue.lock().await;
        if queue.sender.is_none() {
            let (sender, receiver) = mpsc::channel(TRAJECTORY_VECTORIZATION_QUEUE_CAPACITY);
            queue.sender = Some(sender);
            queue.worker = Some(tokio::spawn(run_trajectory_vectorization_queue(
                Arc::downgrade(&gcx),
                receiver,
            )));
        }
        queue.sender.as_ref().cloned()
    };

    let Some(sender) = sender else {
        return;
    };
    if sender.try_send(file_path).is_err() {
        warn!("skipping best-effort trajectory VecDB enqueue because its queue is unavailable");
    }
}

async fn run_trajectory_vectorization_queue(
    gcx_weak: Weak<GlobalContext>,
    mut receiver: mpsc::Receiver<PathBuf>,
) {
    let Some(gcx) = gcx_weak.upgrade() else {
        return;
    };
    let shutdown_flag = gcx.shutdown_flag.clone();
    drop(gcx);

    loop {
        let file_path = tokio::select! {
            file_path = receiver.recv() => file_path,
            _ = wait_for_trajectory_vectorization_shutdown(shutdown_flag.clone()) => return,
        };
        let Some(file_path) = file_path else {
            return;
        };
        let Some(gcx) = gcx_weak.upgrade() else {
            return;
        };
        let roots = crate::indexing_routing::memory_plane_roots(gcx.clone()).await;
        if !roots.is_trajectory_file(&file_path) {
            continue;
        }
        let vecdb = gcx.vec_db.lock().await.clone();
        if let Some(vecdb) = vecdb {
            let documents = vec![file_path.to_string_lossy().to_string()];
            tokio::select! {
                result = std::panic::AssertUnwindSafe(vecdb.vectorizer_enqueue_files(
                    &documents,
                    false,
                    roots,
                )).catch_unwind() => {
                    if result.is_err() {
                        warn!("best-effort trajectory VecDB enqueue panicked");
                    }
                }
                _ = wait_for_trajectory_vectorization_shutdown(shutdown_flag.clone()) => return,
            }
        }
    }
}

pub fn internal_trace_link_type(feature: &str) -> String {
    format!("{INTERNAL_TRACE_LINK_PREFIX}{feature}")
}

async fn prune_internal_traces_in_folder(dir: &Path, keep: usize) -> Result<usize, String> {
    let mut entries = match fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(format!(
                "failed to read trace folder {}: {error}",
                dir.display()
            ))
        }
    };
    let mut traces = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let metadata = match fs::symlink_metadata(&path).await {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => metadata,
            _ => continue,
        };
        let path_for_read = path.clone();
        let is_internal = tokio::task::spawn_blocking(move || {
            parsed_top_level_link_type_is_internal(path_for_read)
        })
        .await
        .unwrap_or(false);
        if !is_internal {
            continue;
        }
        let modified = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis());
        let Some(modified) = modified else {
            continue;
        };
        traces.push((modified, path));
    }
    if traces.len() <= keep {
        return Ok(0);
    }
    traces.sort_by(|left, right| right.0.cmp(&left.0));
    let mut removed = 0;
    for (scanned_modified, path) in traces.into_iter().skip(keep) {
        let current_modified = match fs::symlink_metadata(&path).await {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis()),
            _ => None,
        };
        if current_modified != Some(scanned_modified) {
            continue;
        }
        let path_for_read = path.clone();
        if !tokio::task::spawn_blocking(move || {
            parsed_top_level_link_type_is_internal(path_for_read)
        })
        .await
        .unwrap_or(false)
        {
            continue;
        }
        if fs::remove_file(&path).await.is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

async fn prune_internal_traces_throttled(dir: &Path) -> Result<usize, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    if !claim_prune_throttle(
        internal_trace_prune_throttle(),
        dir,
        now,
        crate::runtime_settings::current().internal_trace_prune_interval_secs,
        "internal trace",
    )? {
        return Ok(0);
    }
    let result = prune_internal_traces_in_folder(
        dir,
        crate::runtime_settings::current().internal_traces_keep_per_folder,
    )
    .await;
    if result.is_err() {
        release_prune_throttle(internal_trace_prune_throttle(), dir, now);
    }
    result
}

pub(crate) async fn index_dir_for_trajectory_file(
    gcx: Arc<GlobalContext>,
    file_path: &Path,
) -> Option<PathBuf> {
    let parent = file_path.parent()?;
    let normal_dirs = get_all_trajectories_dirs(gcx.clone()).await;
    if normal_dirs.iter().any(|dir| dir == parent) {
        return Some(parent.to_path_buf());
    }
    let task_roots = get_all_task_roots(gcx).await;
    if is_task_trajectory_index_dir(parent, &task_roots).await {
        return Some(parent.to_path_buf());
    }
    if let Some(grandparent) = parent.parent() {
        if normal_dirs.iter().any(|dir| dir == grandparent)
            || is_task_trajectory_index_dir(grandparent, &task_roots).await
        {
            return Some(grandparent.to_path_buf());
        }
    }
    Some(parent.to_path_buf())
}

async fn is_task_trajectory_index_dir(dir: &Path, task_roots: &[PathBuf]) -> bool {
    for root in task_roots {
        let Ok(relative) = dir.strip_prefix(root) else {
            continue;
        };
        let components: Vec<_> = relative.components().collect();
        let [Component::Normal(task), Component::Normal(trajectories), Component::Normal(role), descendants @ ..] =
            components.as_slice()
        else {
            continue;
        };
        if *trajectories != "trajectories"
            || !["planner", "linked", "agents", "subchats"]
                .iter()
                .any(|name| *role == *name)
            || descendants
                .iter()
                .any(|part| !matches!(part, Component::Normal(_)))
            || !is_real_dir(&root.join(task)).await
            || !is_real_dir(&root.join(task).join(trajectories)).await
        {
            continue;
        }
        let mut ancestor = root.join(task).join(trajectories).join(role);
        if !is_real_dir(&ancestor).await {
            continue;
        }
        let mut valid = true;
        for component in descendants {
            ancestor.push(component.as_os_str());
            if !is_real_dir(&ancestor).await {
                valid = false;
                break;
            }
        }
        if valid {
            return true;
        }
    }
    false
}

const TRAJECTORY_ID_MISMATCH_PREFIX: &str = "Existing trajectory file id mismatch";

pub(crate) fn trajectory_id_mismatch_error(path: &Path) -> String {
    format!("{TRAJECTORY_ID_MISMATCH_PREFIX} for {}", path.display())
}

pub(crate) fn is_trajectory_id_mismatch_error(error: &str) -> bool {
    error.starts_with(TRAJECTORY_ID_MISMATCH_PREFIX)
}

pub fn chat_folder_name<'a>(chat_id: &'a str, root_chat_id: Option<&'a str>) -> &'a str {
    match root_chat_id {
        Some(root) if !root.is_empty() && validate_trajectory_id(root).is_ok() => root,
        _ => chat_id,
    }
}

pub(crate) async fn list_chat_folder_dirs(gcx: &Arc<GlobalContext>) -> Vec<PathBuf> {
    let roots = get_all_trajectories_dirs(gcx.clone()).await;
    tokio::task::spawn_blocking(move || {
        let mut dirs = Vec::new();
        for root in roots {
            let Ok(entries) = std::fs::read_dir(root) else {
                continue;
            };
            for entry in entries.filter_map(Result::ok) {
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    dirs.push(entry.path());
                }
            }
        }
        dirs
    })
    .await
    .unwrap_or_default()
}

pub async fn get_all_trajectories_dirs(gcx: Arc<GlobalContext>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for dir in get_project_dirs(gcx.clone())
        .await
        .into_iter()
        .map(|p| p.join(".refact").join("trajectories"))
    {
        if is_real_dir(&dir).await {
            dirs.push(dir);
        }
    }

    let global_dir = get_global_trajectories_dir(gcx).await;
    if is_real_dir(&global_dir).await {
        dirs.push(global_dir);
    }

    dirs
}

fn task_root_candidates(
    gcx: Arc<GlobalContext>,
) -> impl std::future::Future<Output = Vec<PathBuf>> {
    async move {
        let mut dirs: Vec<PathBuf> = get_project_dirs(gcx.clone())
            .await
            .into_iter()
            .map(|p| p.join(".refact").join("tasks"))
            .collect();

        dirs.push(crate::tasks::storage::get_global_tasks_dir(gcx).await);
        dirs
    }
}

async fn get_all_task_roots(gcx: Arc<GlobalContext>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for dir in task_root_candidates(gcx).await {
        if is_real_dir(&dir).await {
            dirs.push(dir);
        }
    }
    dirs
}

async fn get_or_create_all_task_roots(gcx: Arc<GlobalContext>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for dir in task_root_candidates(gcx).await {
        if ensure_real_dir_tree(&dir).await.is_ok() {
            dirs.push(dir);
        }
    }
    dirs
}

async fn get_all_task_roots_from_weak(gcx_weak: &Weak<GlobalContext>) -> Vec<PathBuf> {
    match gcx_weak.upgrade() {
        Some(gcx) => get_or_create_all_task_roots(gcx).await,
        None => vec![],
    }
}

pub(crate) async fn list_task_trajectory_dirs(gcx: &Arc<GlobalContext>) -> Vec<PathBuf> {
    let roots = get_all_task_roots(gcx.clone()).await;
    tokio::task::spawn_blocking(move || task_trajectory_dirs_sync(&roots))
        .await
        .unwrap_or_default()
}

fn task_trajectory_dirs_sync(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for tasks_dir in roots {
        let Ok(entries) = std::fs::read_dir(tasks_dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let trajectories = entry.path().join("trajectories");
            if !is_real_dir_sync(&trajectories) {
                continue;
            }
            for role in ["planner", "linked", "agents", "subchats"] {
                let mut pending = vec![trajectories.join(role)];
                while let Some(dir) = pending.pop() {
                    if !is_real_dir_sync(&dir) {
                        continue;
                    }
                    dirs.push(dir.clone());
                    let Ok(children) = std::fs::read_dir(&dir) else {
                        continue;
                    };
                    for child in children.filter_map(Result::ok) {
                        if child.file_type().is_ok_and(|kind| kind.is_dir()) {
                            pending.push(child.path());
                        }
                    }
                }
            }
        }
    }
    dirs
}

async fn existing_trajectory_paths_in_dirs(dirs: Vec<PathBuf>, chat_id: &str) -> Vec<PathBuf> {
    let file_name = format!("{chat_id}.json");
    let hits = tokio::task::spawn_blocking(move || existing_trajectory_dirs_sync(dirs, &file_name))
        .await
        .unwrap_or_default();
    validate_trajectory_dirs(hits, chat_id).await
}

fn existing_trajectory_dirs_sync(dirs: Vec<PathBuf>, file_name: &str) -> Vec<PathBuf> {
    dirs.into_iter()
        .filter(|dir| {
            std::fs::symlink_metadata(dir.join(file_name))
                .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        })
        .collect()
}

async fn validate_trajectory_dirs(dirs: Vec<PathBuf>, chat_id: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for dir in dirs {
        if let Some(path) = safe_trajectory_file_in_dir(&dir, chat_id).await {
            paths.push(path);
        }
    }
    paths
}

async fn task_trajectory_candidate_paths(gcx: Arc<GlobalContext>, chat_id: &str) -> Vec<PathBuf> {
    let roots = get_all_task_roots(gcx).await;
    let file_name = format!("{chat_id}.json");
    let hits = tokio::task::spawn_blocking(move || {
        existing_trajectory_dirs_sync(task_trajectory_dirs_sync(&roots), &file_name)
    })
    .await
    .unwrap_or_default();
    validate_trajectory_dirs(hits, chat_id).await
}

pub(crate) async fn list_trajectory_dirs(gcx: &Arc<GlobalContext>) -> Vec<PathBuf> {
    let mut dirs = get_all_trajectories_dirs(gcx.clone()).await;
    dirs.extend(list_task_trajectory_dirs(gcx).await);
    dirs
}

async fn get_all_trajectories_dirs_from_weak(gcx_weak: &Weak<GlobalContext>) -> Vec<PathBuf> {
    match gcx_weak.upgrade() {
        Some(gcx) => get_all_trajectories_dirs(gcx).await,
        None => vec![],
    }
}

pub async fn get_buddy_conversations_dir(gcx: Arc<GlobalContext>) -> Result<PathBuf, String> {
    let project_dirs = get_project_dirs(gcx).await;
    let workspace_root = project_dirs.first().ok_or("No workspace folder found")?;
    Ok(workspace_root
        .join(".refact")
        .join("buddy")
        .join("chats")
        .join("conversations"))
}

async fn get_or_create_buddy_conversations_dir(gcx: Arc<GlobalContext>) -> Result<PathBuf, String> {
    let dir = get_buddy_conversations_dir(gcx).await?;
    ensure_real_dir_tree(&dir).await?;
    Ok(dir)
}

fn buddy_conversation_prune_throttle() -> &'static StdMutex<PruneThrottle> {
    static LAST_PRUNE_BY_DIR: OnceLock<StdMutex<PruneThrottle>> = OnceLock::new();
    LAST_PRUNE_BY_DIR.get_or_init(|| StdMutex::new(PruneThrottle::default()))
}

async fn prune_buddy_conversations_throttled(gcx: Arc<GlobalContext>) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    let dir = match get_buddy_conversations_dir(gcx.clone()).await {
        Ok(dir) => dir,
        Err(_) => return,
    };
    let settings = crate::runtime_settings::current();
    if !claim_prune_throttle(
        buddy_conversation_prune_throttle(),
        &dir,
        now,
        settings.buddy_conversations_prune_interval_secs,
        "buddy conversation",
    )
    .unwrap_or(false)
    {
        return;
    }
    let app = AppState::from_gcx(gcx.clone()).await;
    match prune_buddy_conversations_in_dir_with_coordinator(
        &dir,
        settings.buddy_conversations_keep,
        settings.buddy_conversations_prune_min_age_secs,
        Some(&app.chat.trajectory_index_coordinator),
    )
    .await
    {
        Ok(0) => {}
        Ok(removed) => info!("pruned {} old buddy conversations from {:?}", removed, dir),
        Err(e) => {
            warn!("buddy conversations prune failed: {}", e);
            release_prune_throttle(buddy_conversation_prune_throttle(), &dir, now);
        }
    }
}

#[cfg(test)]
pub(crate) async fn prune_buddy_conversations_in_dir(
    dir: &Path,
    keep: usize,
) -> Result<usize, String> {
    prune_buddy_conversations_in_dir_with_coordinator(
        dir,
        keep,
        crate::runtime_settings::current().buddy_conversations_prune_min_age_secs,
        None,
    )
    .await
}

async fn prune_buddy_conversations_in_dir_with_coordinator(
    dir: &Path,
    keep: usize,
    min_age_secs: u64,
    coordinator: Option<&trajectory_index::TrajectoryIndexCoordinator>,
) -> Result<usize, String> {
    let mut entries = match fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => {
            return Err(format!(
                "Failed to read buddy conversations dir {:?}: {e}",
                dir
            ))
        }
    };
    let mut files: Vec<(i64, PathBuf, String)> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if !path.extension().map(|e| e == "json").unwrap_or(false) {
            continue;
        }
        if path
            .file_name()
            .is_some_and(|name| name == trajectory_index::TRAJECTORY_INDEX_FILE)
        {
            continue;
        }
        let Some(stem) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        let metadata = match fs::symlink_metadata(&path).await {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => metadata,
            Err(_) => continue,
            _ => continue,
        };
        let modified_ms = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|elapsed| elapsed.as_millis() as i64)
            .unwrap_or(0);
        files.push((modified_ms, path, stem));
    }
    if files.len() <= keep {
        return Ok(0);
    }
    files.sort_by(|a, b| b.0.cmp(&a.0));
    let min_age_cutoff_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
        .saturating_sub((min_age_secs as i64).saturating_mul(1000));
    let mut removed_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (scanned_modified_ms, path, stem) in files.drain(keep..) {
        if scanned_modified_ms >= min_age_cutoff_ms {
            continue;
        }
        let current_modified_ms = match fs::symlink_metadata(&path).await {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|elapsed| elapsed.as_millis() as i64)
                .unwrap_or(i64::MAX),
            Err(_) => continue,
            _ => continue,
        };
        if current_modified_ms != scanned_modified_ms {
            continue;
        }
        match fs::remove_file(&path).await {
            Ok(()) => {
                removed_ids.insert(stem);
            }
            Err(e) => warn!("failed to remove old buddy conversation {:?}: {}", path, e),
        }
    }
    let removed = removed_ids.len();
    if removed > 0 {
        if let Some(coordinator) = coordinator {
            trajectory_index::remove_trajectory_index_entries_with_rollout(
                coordinator,
                dir,
                &removed_ids,
            )
            .await?;
        } else {
            trajectory_index::remove_trajectory_index_entries(dir, &removed_ids).await?;
        }
    }
    Ok(removed)
}

fn normalize_system_prompt(system_prompt: Option<String>) -> Option<String> {
    system_prompt.filter(|text| !text.trim().is_empty())
}

fn frozen_prefix_has_system_prompt(prefix: &FrozenRequestPrefix) -> bool {
    prefix
        .system_prompt
        .as_ref()
        .is_some_and(|text| !text.trim().is_empty())
}

fn frozen_prefix_has_tools(prefix: &FrozenRequestPrefix) -> bool {
    prefix.tools_canonical.is_some()
}

fn is_mode_transition_or_handoff(link_type: Option<&str>) -> bool {
    matches!(link_type, Some("mode_transition" | "handoff"))
}

pub fn frozen_prefix_is_complete(prefix: &FrozenRequestPrefix) -> bool {
    frozen_prefix_has_system_prompt(prefix) && frozen_prefix_has_tools(prefix)
}

pub fn first_system_prompt(messages: &[ChatMessage]) -> Option<String> {
    messages.iter().find_map(|message| {
        if message.role == "system" {
            match &message.content {
                ChatContent::SimpleText(text) if !text.trim().is_empty() => Some(text.clone()),
                _ => None,
            }
        } else {
            None
        }
    })
}

fn parsed_rfc3339_utc(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&chrono::Utc))
}

fn raw_frozen_prefix_has_tools(value: Option<&serde_json::Value>) -> bool {
    value
        .and_then(|value| value.get("tools_canonical"))
        .is_some_and(|value| !value.is_null())
}

fn legacy_incomplete_prefix_from_messages(messages: &[ChatMessage]) -> Option<FrozenRequestPrefix> {
    first_system_prompt(messages)
        .map(|system_prompt| new_legacy_incomplete_frozen_request_prefix(Some(system_prompt)))
}

fn repair_transition_frozen_prefix(
    link_type: Option<&str>,
    messages: &[ChatMessage],
    frozen_request_prefix: Option<FrozenRequestPrefix>,
    raw_frozen_prefix_had_tools: bool,
    trajectory_created_at: Option<&str>,
    provider_state_present: bool,
) -> (Option<FrozenRequestPrefix>, bool) {
    let is_transition = is_mode_transition_or_handoff(link_type);
    let Some(prefix) = frozen_request_prefix else {
        let repaired = is_transition && (provider_state_present || raw_frozen_prefix_had_tools);
        return (legacy_incomplete_prefix_from_messages(messages), repaired);
    };
    if !is_transition {
        return (Some(prefix), false);
    }

    let first_system = first_system_prompt(messages);
    let system_mismatch = match (
        prefix
            .system_prompt
            .as_ref()
            .filter(|text| !text.trim().is_empty()),
        first_system.as_ref(),
    ) {
        (Some(frozen_system), Some(first_system)) => frozen_system != first_system,
        _ => false,
    };
    let missing_system_with_tools =
        !frozen_prefix_has_system_prompt(&prefix) && frozen_prefix_has_tools(&prefix);
    let prefix_created_at_invalid = parsed_rfc3339_utc(&prefix.created_at).is_none();
    let invalid_created_at_with_state =
        prefix_created_at_invalid && (provider_state_present || frozen_prefix_has_tools(&prefix));
    let trajectory_created_at_invalid = trajectory_created_at
        .map(|created_at| parsed_rfc3339_utc(created_at).is_none())
        .unwrap_or(true);
    let invalid_trajectory_created_at_with_state = trajectory_created_at_invalid
        && (provider_state_present || frozen_prefix_has_tools(&prefix));
    // Transition prefixes older than trajectory creation are copied by definition: new transition
    // snapshots start without a prefix, and target-chat lazy prefixes are created after trajectory creation.
    let prefix_created_before_trajectory = trajectory_created_at
        .and_then(parsed_rfc3339_utc)
        .zip(parsed_rfc3339_utc(&prefix.created_at))
        .is_some_and(|(trajectory_created_at, prefix_created_at)| {
            prefix_created_at < trajectory_created_at
        });

    if !(system_mismatch
        || prefix_created_before_trajectory
        || invalid_created_at_with_state
        || invalid_trajectory_created_at_with_state
        || missing_system_with_tools)
    {
        return (Some(prefix), false);
    }

    (
        first_system
            .map(|system_prompt| new_legacy_incomplete_frozen_request_prefix(Some(system_prompt))),
        true,
    )
}

pub fn new_frozen_request_prefix(
    system_prompt: Option<String>,
    tools_canonical: serde_json::Value,
) -> FrozenRequestPrefix {
    FrozenRequestPrefix {
        schema_version: 1,
        created_at: chrono::Utc::now().to_rfc3339(),
        system_prompt: normalize_system_prompt(system_prompt),
        tools_canonical: Some(tools_canonical),
    }
}

pub(crate) fn new_legacy_incomplete_frozen_request_prefix(
    system_prompt: Option<String>,
) -> FrozenRequestPrefix {
    FrozenRequestPrefix {
        schema_version: 1,
        created_at: chrono::Utc::now().to_rfc3339(),
        system_prompt: normalize_system_prompt(system_prompt),
        tools_canonical: None,
    }
}

#[cfg(test)]
async fn persist_frozen_prefix(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
    frozen_request_prefix: FrozenRequestPrefix,
) -> Result<(), String> {
    validate_trajectory_id(chat_id).map_err(|e| e.message)?;
    let Some(file_path) = find_trajectory_or_buddy_path(gcx.clone(), chat_id).await else {
        return Ok(());
    };

    let content = tokio::fs::read_to_string(&file_path)
        .await
        .map_err(|e| format!("Failed to read trajectory: {}", e))?;
    let json_result = tokio::task::spawn_blocking(move || {
        let mut trajectory = serde_json::from_str::<serde_json::Value>(&content)
            .map_err(|e| format!("Failed to parse trajectory: {}", e))?;
        if trajectory
            .get("frozen_request_prefix")
            .is_some_and(|value| !value.is_null())
        {
            return Ok(None);
        }
        trajectory["frozen_request_prefix"] =
            serde_json::to_value(frozen_request_prefix).map_err(|e| e.to_string())?;
        trajectory["updated_at"] = serde_json::Value::String(chrono::Utc::now().to_rfc3339());
        serde_json::to_string_pretty(&trajectory)
            .map(Some)
            .map_err(|e| format!("Failed to serialize trajectory: {}", e))
    })
    .await
    .map_err(|e| format!("Trajectory prefix persistence task failed: {}", e))??;
    let Some(json) = json_result else {
        return Ok(());
    };

    let tmp_path = unique_trajectory_tmp_path(&file_path);
    atomic_write_json_with_tmp_path(
        &file_path,
        &tmp_path,
        Ok(json),
        Some("Failed to write trajectory"),
        Some(TrajectorySourceIdentity::Normal),
    )
    .await
}

pub fn ensure_frozen_prefix(
    session: &mut ChatSession,
    system_prompt: Option<String>,
    tools_canonical: Option<serde_json::Value>,
) -> Option<FrozenRequestPrefix> {
    let system_prompt = normalize_system_prompt(system_prompt);
    if system_prompt.is_none() && tools_canonical.is_none() {
        return None;
    }

    match session.thread.frozen_request_prefix.as_mut() {
        Some(prefix) => {
            if frozen_prefix_is_complete(prefix) {
                return None;
            }

            let mut changed = false;
            if !frozen_prefix_has_system_prompt(prefix) {
                if let Some(system_prompt) = system_prompt {
                    prefix.system_prompt = Some(system_prompt);
                    changed = true;
                }
            }
            if !frozen_prefix_has_tools(prefix) {
                if let Some(tools_canonical) = tools_canonical {
                    prefix.tools_canonical = Some(tools_canonical);
                    changed = true;
                }
            }
            if changed {
                let prefix = prefix.clone();
                session.increment_version();
                Some(prefix)
            } else {
                None
            }
        }
        None => {
            let prefix = match tools_canonical {
                Some(tools_canonical) => new_frozen_request_prefix(system_prompt, tools_canonical),
                None => new_legacy_incomplete_frozen_request_prefix(system_prompt),
            };
            session.thread.frozen_request_prefix = Some(prefix.clone());
            session.increment_version();
            Some(prefix)
        }
    }
}

fn parse_stored_messages(value: serde_json::Value) -> Result<Vec<ChatMessage>, serde_json::Error> {
    // Reconstruction payloads deliberately remain opaque extras for archive browsing.
    serde_json::from_value(value)
}

fn fix_tool_call_indexes(messages: &mut [ChatMessage]) {
    for msg in messages.iter_mut() {
        if let Some(ref mut tool_calls) = msg.tool_calls {
            for (i, tc) in tool_calls.iter_mut().enumerate() {
                if tc.index.is_none() {
                    tc.index = Some(i);
                }
            }
        }
    }
}

fn extend_unique_paths(
    target: &mut Vec<PathBuf>,
    seen: &mut std::collections::HashSet<PathBuf>,
    paths: Vec<PathBuf>,
) {
    for path in paths {
        if seen.insert(path.clone()) {
            target.push(path);
        }
    }
}

async fn indexed_candidate_paths_in_dirs(
    gcx: &Arc<GlobalContext>,
    dirs: &[PathBuf],
    chat_id: &str,
    caller: trajectory_index::TrajectoryIndexListingCaller,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let app = AppState::from_gcx(gcx.clone()).await;
    for dir in dirs {
        let entries = match trajectory_index::list_trajectory_entries_with_rollout_shared_for(
            &app.chat.trajectory_index_coordinator,
            dir,
            None,
            caller,
        )
        .await
        {
            Ok(entries) => entries,
            Err(e) => {
                warn!("Failed to read trajectory index {:?}: {}", dir, e);
                continue;
            }
        };
        for entry in entries.iter() {
            if entry.id == chat_id
                && trajectory_index::trajectory_index_entry_is_fresh(dir, entry).await
            {
                paths.push(trajectory_index::trajectory_file_path_for_entry(dir, entry));
            }
        }
    }
    paths
}

// O(1) in the number of trajectory directories: the coordinator resolves the chat id through its
// cross-directory id map instead of listing every directory. A miss returns nothing and the caller
// falls back to the full scan below, so this can only ever save work. Hits arrive in
// coordinator-load order rather than `list_trajectory_dirs` order, so an id duplicated across
// directories can resolve to a different (still valid) file than the full scan would pick first.
async fn indexed_candidate_paths_by_id(
    gcx: &Arc<GlobalContext>,
    chat_id: &str,
    caller: trajectory_index::TrajectoryIndexListingCaller,
) -> Vec<PathBuf> {
    let app = AppState::from_gcx(gcx.clone()).await;
    let mut paths = Vec::new();
    for (dir, entry) in app
        .chat
        .trajectory_index_coordinator
        .find_entries_by_id(chat_id, caller)
        .await
    {
        if trajectory_index::trajectory_index_entry_is_fresh(&dir, &entry).await {
            paths.push(trajectory_index::trajectory_file_path_for_entry(
                &dir, &entry,
            ));
        }
    }
    paths
}

async fn unsynced_trajectory_dirs(gcx: &Arc<GlobalContext>, dirs: &[PathBuf]) -> Vec<PathBuf> {
    let app = AppState::from_gcx(gcx.clone()).await;
    app.chat
        .trajectory_index_coordinator
        .unsynced_directories(dirs)
}

async fn direct_normal_trajectory_candidate_paths(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let normal_dirs = get_all_trajectories_dirs(gcx.clone()).await;
    for dir in &normal_dirs {
        if let Some(path) = safe_trajectory_file_in_dir(dir, chat_id).await {
            if seen.insert(path.clone()) {
                paths.push(path);
            }
        }
        let own_folder = dir.join(chat_id);
        if is_real_dir(&own_folder).await {
            if let Some(path) = safe_trajectory_file_in_dir(&own_folder, chat_id).await {
                if seen.insert(path.clone()) {
                    paths.push(path);
                }
            }
        }
    }
    paths
}

async fn normal_trajectory_candidate_paths(gcx: Arc<GlobalContext>, chat_id: &str) -> Vec<PathBuf> {
    let mut paths = direct_normal_trajectory_candidate_paths(gcx.clone(), chat_id).await;
    let mut seen = paths.iter().cloned().collect();
    let indexed = indexed_candidate_paths_in_dirs(
        &gcx,
        &get_all_trajectories_dirs(gcx.clone()).await,
        chat_id,
        trajectory_index::TrajectoryIndexListingCaller::ChatPathLookup,
    )
    .await;
    extend_unique_paths(&mut paths, &mut seen, indexed);
    paths
}

async fn trajectory_candidate_paths(gcx: Arc<GlobalContext>, chat_id: &str) -> Vec<PathBuf> {
    let mut candidates = normal_trajectory_candidate_paths(gcx.clone(), chat_id).await;
    candidates.extend(task_trajectory_candidate_paths(gcx, chat_id).await);
    candidates
}

async fn nested_chat_folder_candidate_paths(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
) -> Vec<PathBuf> {
    existing_trajectory_paths_in_dirs(list_chat_folder_dirs(&gcx).await, chat_id).await
}

struct ValidTrajectoryCandidate {
    path: PathBuf,
    content: String,
    json: serde_json::Value,
}

async fn read_valid_trajectory_candidate(
    path: PathBuf,
    chat_id: &str,
) -> Option<ValidTrajectoryCandidate> {
    let content = match fs::read_to_string(&path).await {
        Ok(content) => content,
        Err(e) => {
            warn!(
                "Skipping trajectory candidate {} for chat {}: failed to read: {}",
                path.display(),
                chat_id,
                e
            );
            return None;
        }
    };
    let parsed = tokio::task::spawn_blocking(move || {
        serde_json::from_str(&content).map(|json| (content, json))
    })
    .await;
    let (content, json): (String, serde_json::Value) = match parsed {
        Ok(Ok(parsed)) => parsed,
        Ok(Err(e)) => {
            warn!(
                "Skipping trajectory candidate {} for chat {}: failed to parse: {}",
                path.display(),
                chat_id,
                e
            );
            return None;
        }
        Err(e) => {
            warn!(
                "Skipping trajectory candidate {} for chat {}: parse task failed: {}",
                path.display(),
                chat_id,
                e
            );
            return None;
        }
    };
    if !trajectory_candidate_has_minimum_schema(&json, chat_id, &path) {
        return None;
    }
    Some(ValidTrajectoryCandidate {
        path,
        content,
        json,
    })
}

async fn first_valid_trajectory_candidate(
    paths: Vec<PathBuf>,
    chat_id: &str,
) -> Option<ValidTrajectoryCandidate> {
    for path in paths {
        if let Some(candidate) = read_valid_trajectory_candidate(path, chat_id).await {
            return Some(candidate);
        }
    }
    None
}

async fn find_normal_trajectory_file(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
) -> Option<ValidTrajectoryCandidate> {
    validate_trajectory_id(chat_id).ok()?;
    let task_roots = get_all_task_roots(gcx.clone()).await;
    for path in normal_trajectory_candidate_paths(gcx, chat_id).await {
        let Some(candidate) = read_valid_trajectory_candidate(path, chat_id).await else {
            continue;
        };
        let Ok(source) = TrajectorySourceIdentity::from_json(&candidate.json) else {
            continue;
        };
        let source = effective_trajectory_source_for_path(source, &candidate.path, &task_roots);
        if trajectory_source_matches_hint(&source, &TrajectorySourceIdentity::Normal) {
            return Some(candidate);
        }
    }
    None
}

async fn first_normal_trajectory_candidate(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
) -> Option<ValidTrajectoryCandidate> {
    let direct = direct_normal_trajectory_candidate_paths(gcx.clone(), chat_id).await;
    let seen: std::collections::HashSet<_> = direct.iter().cloned().collect();
    if let Some(found) = first_valid_trajectory_candidate(direct, chat_id).await {
        return Some(found);
    }
    let indexed = indexed_candidate_paths_in_dirs(
        &gcx,
        &get_all_trajectories_dirs(gcx.clone()).await,
        chat_id,
        trajectory_index::TrajectoryIndexListingCaller::ChatPathLookup,
    )
    .await;
    first_valid_trajectory_candidate(
        indexed
            .into_iter()
            .filter(|path| !seen.contains(path))
            .collect(),
        chat_id,
    )
    .await
}

async fn find_trajectory_file(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
) -> Option<ValidTrajectoryCandidate> {
    validate_trajectory_id(chat_id).ok()?;
    let normal = direct_normal_trajectory_candidate_paths(gcx.clone(), chat_id).await;
    let mut seen: std::collections::HashSet<PathBuf> = normal.iter().cloned().collect();
    if let Some(found) = first_valid_trajectory_candidate(normal, chat_id).await {
        return Some(found);
    }
    let mut indexed_normal = Vec::new();
    extend_unique_paths(
        &mut indexed_normal,
        &mut seen,
        indexed_candidate_paths_in_dirs(
            &gcx,
            &get_all_trajectories_dirs(gcx.clone()).await,
            chat_id,
            trajectory_index::TrajectoryIndexListingCaller::ChatPathLookup,
        )
        .await,
    );
    if let Some(found) = first_valid_trajectory_candidate(indexed_normal, chat_id).await {
        return Some(found);
    }
    let mut direct = Vec::new();
    extend_unique_paths(
        &mut direct,
        &mut seen,
        indexed_candidate_paths_by_id(
            &gcx,
            chat_id,
            trajectory_index::TrajectoryIndexListingCaller::ChatPathLookup,
        )
        .await,
    );
    if let Some(found) = first_valid_trajectory_candidate(direct, chat_id).await {
        return Some(found);
    }
    let mut candidates = Vec::new();
    let task_dirs = list_task_trajectory_dirs(&gcx).await;
    let task_scoped = existing_trajectory_paths_in_dirs(task_dirs.clone(), chat_id).await;
    let mut all_dirs = get_all_trajectories_dirs(gcx.clone()).await;
    all_dirs.extend(task_dirs.into_iter().filter(|dir| {
        task_scoped.iter().any(|path| {
            path.parent() == Some(dir.as_path())
                || path.parent().and_then(Path::parent) == Some(dir.as_path())
        })
    }));
    let unsynced = unsynced_trajectory_dirs(&gcx, &all_dirs).await;
    let indexed = indexed_candidate_paths_in_dirs(
        &gcx,
        &unsynced,
        chat_id,
        trajectory_index::TrajectoryIndexListingCaller::ChatPathLookup,
    )
    .await;
    extend_unique_paths(&mut candidates, &mut seen, indexed);
    extend_unique_paths(&mut candidates, &mut seen, task_scoped);
    if let Some(found) = first_valid_trajectory_candidate(candidates, chat_id).await {
        return Some(found);
    }
    let nested = nested_chat_folder_candidate_paths(gcx, chat_id).await;
    first_valid_trajectory_candidate(nested, chat_id).await
}

async fn find_normal_trajectory_path(gcx: Arc<GlobalContext>, chat_id: &str) -> Option<PathBuf> {
    find_normal_trajectory_file(gcx, chat_id)
        .await
        .map(|candidate| candidate.path)
}

pub async fn find_trajectory_path(gcx: Arc<GlobalContext>, chat_id: &str) -> Option<PathBuf> {
    find_trajectory_file(gcx, chat_id)
        .await
        .map(|candidate| candidate.path)
}

pub(crate) async fn find_trajectory_parent_id(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
) -> Option<Option<String>> {
    let candidate = find_trajectory_file(gcx, chat_id).await?;
    let trajectory = candidate.json.as_object()?;
    Some(
        trajectory
            .get("parent_id")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
    )
}

pub async fn find_trajectory_or_buddy_path(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
) -> Option<PathBuf> {
    find_trajectory_or_buddy_file(gcx, chat_id)
        .await
        .map(|candidate| candidate.path)
}
async fn find_validated_trajectory_path_in_dirs(
    chat_id: &str,
    dirs: Vec<PathBuf>,
) -> Option<PathBuf> {
    for dir in dirs {
        let Some(path) = safe_trajectory_file_in_dir(&dir, chat_id).await else {
            continue;
        };
        if read_valid_trajectory_candidate(path.clone(), chat_id)
            .await
            .is_some()
        {
            return Some(path);
        }
    }
    None
}

async fn find_validated_trajectory_path_for_source(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
    source: &TrajectorySourceIdentity,
) -> Option<PathBuf> {
    match source {
        TrajectorySourceIdentity::Normal => find_normal_trajectory_path(gcx, chat_id).await,
        TrajectorySourceIdentity::Buddy => {
            let dirs = get_buddy_conversations_dir(gcx)
                .await
                .ok()
                .into_iter()
                .collect();
            find_validated_trajectory_path_in_dirs(chat_id, dirs).await
        }
        TrajectorySourceIdentity::Task {
            ..
        } => {
            let dirs = list_task_trajectory_dirs(&gcx).await;
            find_validated_trajectory_path_in_dirs(chat_id, dirs).await
        }
    }
}

pub async fn find_trajectory_path_for_active_chat(
    gcx: Arc<GlobalContext>,
    sessions: &super::SessionsMap,
    chat_id: &str,
) -> Option<PathBuf> {
    validate_trajectory_id(chat_id).ok()?;
    let source = {
        let session_arc = sessions.read().await.get(chat_id).cloned();
        match session_arc {
            Some(arc) => {
                let session = arc.lock().await;
                TrajectorySourceIdentity::from_session(&session)
            }
            None => {
                return find_trajectory_or_buddy_path(gcx, chat_id).await;
            }
        }
    };
    find_validated_trajectory_path_for_source(gcx, chat_id, &source).await
}

async fn find_trajectory_or_buddy_file(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
) -> Option<ValidTrajectoryCandidate> {
    validate_trajectory_id(chat_id).ok()?;
    if let Some(found) = first_normal_trajectory_candidate(gcx.clone(), chat_id).await {
        return Some(found);
    }
    let mut candidates = task_trajectory_candidate_paths(gcx.clone(), chat_id).await;
    if let Ok(buddy_dir) = get_buddy_conversations_dir(gcx.clone()).await {
        if let Some(buddy_path) = safe_trajectory_file_in_dir(&buddy_dir, chat_id).await {
            candidates.push(buddy_path);
        }
    }
    if let Some(found) = first_valid_trajectory_candidate(candidates, chat_id).await {
        return Some(found);
    }
    let nested = nested_chat_folder_candidate_paths(gcx, chat_id).await;
    first_valid_trajectory_candidate(nested, chat_id).await
}

async fn ensure_existing_trajectory_file_matches(path: &Path, chat_id: &str) -> Result<(), String> {
    match fs::symlink_metadata(path).await {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(format!(
                    "Existing trajectory file is not a real file for {}",
                    path.display()
                ));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("Failed to check trajectory existence: {}", e)),
    }
    let content = fs::read_to_string(path)
        .await
        .map_err(|e| format!("Failed to read existing trajectory: {}", e))?;
    let json: serde_json::Value = tokio::task::spawn_blocking(move || {
        serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse existing trajectory: {}", e))
    })
    .await
    .map_err(|e| format!("Existing trajectory parse task failed: {}", e))??;
    if trajectory_root_id_matches(&json, chat_id, path) {
        Ok(())
    } else {
        Err(trajectory_id_mismatch_error(path))
    }
}

async fn read_existing_trajectory_object(
    path: &Path,
    chat_id: &str,
) -> Result<Option<serde_json::Map<String, serde_json::Value>>, String> {
    #[cfg(test)]
    if let Some(reads) = test_trajectory_metadata_read_counters()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&trajectory_metadata_cache_key(path))
        .and_then(Weak::upgrade)
    {
        reads.fetch_add(1, Ordering::Relaxed);
    }
    #[cfg(test)]
    record_trajectory_metadata_disk_read(chat_id);
    match fs::symlink_metadata(path).await {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(format!(
                    "Existing trajectory file is not a real file for {}",
                    path.display()
                ));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Failed to check trajectory existence: {}", e)),
    }
    let content = fs::read_to_string(path)
        .await
        .map_err(|e| format!("Failed to read existing trajectory: {}", e))?;
    let json: serde_json::Value = tokio::task::spawn_blocking(move || {
        serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse existing trajectory: {}", e))
    })
    .await
    .map_err(|e| format!("Existing trajectory parse task failed: {}", e))??;
    if !trajectory_root_id_matches(&json, chat_id, path) {
        return Err(trajectory_id_mismatch_error(path));
    }
    json.as_object()
        .cloned()
        .ok_or_else(|| {
            format!(
                "Existing trajectory JSON root must be an object for {}",
                path.display()
            )
        })
        .map(Some)
}

/// Per-thread trajectory metadata read counter. `cargo test` gives every test
/// its own thread and a `#[tokio::test]` runtime is driven on that thread, so a
/// test that asserts on read counts observes only the reads it caused instead of
/// a process-wide total that unrelated concurrent tests inflate.
/// Per-path metadata disk-read counters, used only by tests.
///
/// This intentionally is *not* a single process-wide counter. The read it counts happens inside
/// the detached trajectory writer, i.e. on a `tokio::spawn`ed task on some other worker thread,
/// so a thread-local/task-local scope (the `TrajectoryIndexListingCounterScope::local()` pattern
/// in `trajectory_index.rs`) would never observe it. Keying by trajectory path lets a test count
/// exactly the file it cares about and ignore reads of any other trajectory.
#[cfg(test)]
fn test_trajectory_metadata_read_counters(
) -> &'static StdMutex<std::collections::HashMap<PathBuf, Weak<AtomicUsize>>> {
    static COUNTERS: OnceLock<StdMutex<std::collections::HashMap<PathBuf, Weak<AtomicUsize>>>> =
        OnceLock::new();
    COUNTERS.get_or_init(|| StdMutex::new(std::collections::HashMap::new()))
}

#[cfg(test)]
fn count_test_trajectory_metadata_reads(path: &Path) -> Arc<AtomicUsize> {
    // Count the real read path, including detached writers, but not unrelated tests
    // that can make progress while this save awaits archive mutation coordination.
    let reads = Arc::new(AtomicUsize::new(0));
    let mut counters = test_trajectory_metadata_read_counters()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    counters.retain(|_, counter| counter.strong_count() > 0);
    counters.insert(trajectory_metadata_cache_key(path), Arc::downgrade(&reads));
    reads
}

/// Per-chat metadata disk-read counters, used only by tests.
///
/// Same detached-writer reasoning as the per-path counters above, but keyed by `chat_id`: every
/// test already uses a unique chat id, so concurrently running tests can no longer inflate one
/// another's deltas even when they never learn the on-disk path.
#[cfg(test)]
fn trajectory_metadata_disk_reads_by_chat(
) -> &'static StdMutex<std::collections::HashMap<String, usize>> {
    static READS: OnceLock<StdMutex<std::collections::HashMap<String, usize>>> = OnceLock::new();
    READS.get_or_init(|| StdMutex::new(std::collections::HashMap::new()))
}

#[cfg(test)]
fn record_trajectory_metadata_disk_read(chat_id: &str) {
    *trajectory_metadata_disk_reads_by_chat()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(chat_id.to_string())
        .or_insert(0) += 1;
}

#[cfg(test)]
fn trajectory_metadata_disk_reads_for_chat(chat_id: &str) -> usize {
    trajectory_metadata_disk_reads_by_chat()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(chat_id)
        .copied()
        .unwrap_or(0)
}

/// Per-chat physical full-file rewrite counters, used only by tests.
///
/// Keyed by `chat_id` for the same reason as the metadata read counters above: the write happens
/// on a detached writer task, so a task-local scope would never observe it.
#[cfg(test)]
fn trajectory_physical_writes_by_chat(
) -> &'static StdMutex<std::collections::HashMap<String, usize>> {
    static WRITES: OnceLock<StdMutex<std::collections::HashMap<String, usize>>> = OnceLock::new();
    WRITES.get_or_init(|| StdMutex::new(std::collections::HashMap::new()))
}

#[cfg(test)]
fn record_trajectory_physical_write(chat_id: &str) {
    *trajectory_physical_writes_by_chat()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(chat_id.to_string())
        .or_insert(0) += 1;
}

#[cfg(test)]
fn trajectory_physical_writes_for_chat(chat_id: &str) -> usize {
    trajectory_physical_writes_by_chat()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(chat_id)
        .copied()
        .unwrap_or(0)
}

#[cfg(test)]
fn reset_test_trajectory_watcher_task() {
    let mut guard = TRAJECTORY_WATCHER_TASK
        .get_or_init(|| StdMutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(handle) = guard.take() {
        handle.abort();
    }
}

fn is_known_trajectory_top_level_key(key: &str) -> bool {
    matches!(
        key,
        "id" | "title"
            | "model"
            | "mode"
            | "tool_use"
            | "messages"
            | "created_at"
            | "updated_at"
            | "boost_reasoning"
            | "checkpoints_enabled"
            | "context_tokens_cap"
            | "auto_compression_cap"
            | "auto_compression_cap_pending"
            | "include_project_info"
            | "isTitleGenerated"
            | "auto_approve_editing_tools"
            | "auto_approve_dangerous_commands"
            | "autonomous_no_confirm"
            | "reasoning_effort"
            | "thinking_budget"
            | "temperature"
            | "frequency_penalty"
            | "max_tokens"
            | "previous_response_id"
            | "parallel_tool_calls"
            | "active_skill"
            | "auto_enrichment_enabled"
            | "buddy_meta"
            | "auto_compact_enabled"
            | "frozen_request_prefix"
            | "claude_code_identity"
            | "reactive_compact_attempts"
            | "wake_up_at"
            | "waiting_for_card_ids"
            | "pending_questions"
            | "native_decision_receipts"
            | "pending_browser_decision"
            | "pending_deliveries"
            | "dropped_deliveries"
            | "worktree"
            | "parent_id"
            | "link_type"
            | "root_chat_id"
            | "task_meta"
            | "goal"
            | "goal_ledger"
            | "goal_verification_blocked_until_ms"
            | "browser_meta"
            | "app_workspace"
    )
}

fn preserve_existing_trajectory_metadata(
    trajectory: &mut serde_json::Value,
    existing: Option<serde_json::Map<String, serde_json::Value>>,
) {
    let Some(existing) = existing else {
        return;
    };
    let Some(trajectory_object) = trajectory.as_object_mut() else {
        return;
    };
    for (key, value) in existing {
        let preserve_browser_meta = key == "browser_meta" && !trajectory_object.contains_key(&key);
        let preserve_unknown =
            !is_known_trajectory_top_level_key(&key) && !trajectory_object.contains_key(&key);
        if preserve_browser_meta || preserve_unknown {
            trajectory_object.insert(key, value);
        }
    }
}

pub(crate) fn transitioned_task_meta(
    source: Option<crate::chat::types::TaskMeta>,
    target_mode: &str,
) -> Option<crate::chat::types::TaskMeta> {
    source.map(|mut meta| {
        if meta.role == "planner" && target_mode != "task_planner" {
            meta.role = "linked".to_string();
        }
        meta
    })
}

fn validate_task_trajectory_role(role: &str) -> Result<(), String> {
    match role {
        "planner" | "linked" | "agents" | "subchats" => Ok(()),
        _ => Err(format!("Invalid task trajectory role: {role}")),
    }
}

fn validate_task_agent_id(agent_id: Option<&str>) -> Result<(), String> {
    if let Some(agent_id) = agent_id {
        validate_trajectory_id(agent_id).map_err(|e| e.message)?;
    }
    Ok(())
}

async fn safe_task_dir(gcx: Arc<GlobalContext>, task_id: &str) -> Result<PathBuf, String> {
    crate::tasks::storage::validate_task_id(task_id)?;
    if crate::tasks::storage::bound_task_active() {
        let (task_dir, _guard) = crate::tasks::storage::locked_task_dir(gcx, task_id).await?;
        if !is_real_dir(&task_dir).await {
            return Err("Accepted task directory is not a real directory".into());
        }
        return Ok(task_dir);
    }
    for tasks_dir in get_all_task_roots(gcx).await {
        let candidate = tasks_dir.join(task_id);
        if is_real_dir(&candidate).await {
            return Ok(candidate);
        }
    }
    Err(format!("Task not found: {task_id}"))
}

async fn safe_task_trajectory_dir(
    gcx: Arc<GlobalContext>,
    task_meta: &super::types::TaskMeta,
) -> Result<PathBuf, String> {
    validate_task_trajectory_role(&task_meta.role)?;
    validate_task_agent_id(task_meta.agent_id.as_deref())?;
    let task_dir = safe_task_dir(gcx, &task_meta.task_id).await?;
    let traj_dir = crate::tasks::storage::get_task_trajectory_dir(
        &task_dir,
        &task_meta.role,
        task_meta.agent_id.as_deref(),
    );
    ensure_real_dir_tree(&traj_dir).await?;
    Ok(traj_dir)
}

async fn safe_new_task_trajectory_file(
    gcx: Arc<GlobalContext>,
    task_meta: &super::types::TaskMeta,
    chat_id: &str,
) -> Result<PathBuf, String> {
    let traj_dir = safe_task_trajectory_dir(gcx, task_meta).await?;
    safe_new_trajectory_file_in_dir(&traj_dir, chat_id).await
}

async fn safe_new_buddy_trajectory_file(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
) -> Result<PathBuf, String> {
    let buddy_dir = get_or_create_buddy_conversations_dir(gcx).await?;
    safe_new_trajectory_file_in_dir(&buddy_dir, chat_id).await
}

async fn ensure_legacy_flat_slot_is_safe(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
) -> Result<(), String> {
    let trajectories_dir = get_trajectories_dir(gcx).await?;
    let legacy_path = safe_new_trajectory_file_in_dir(&trajectories_dir, chat_id).await?;
    if is_real_file(&legacy_path).await {
        read_existing_trajectory_object(&legacy_path, chat_id).await?;
    }
    Ok(())
}

async fn safe_new_nested_trajectory_file(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
    root_chat_id: Option<&str>,
) -> Result<PathBuf, String> {
    validate_trajectory_id(chat_id).map_err(|e| e.message)?;
    ensure_legacy_flat_slot_is_safe(gcx.clone(), chat_id).await?;
    let folder = chat_folder_name(chat_id, root_chat_id);
    let base = get_trajectories_dir(gcx).await?;
    let dir = base.join(folder);
    ensure_real_dir_tree(&dir).await?;
    safe_new_trajectory_file_in_dir(&dir, chat_id).await
}

async fn trajectory_source_path_is_allowed(gcx: Arc<GlobalContext>, path: &Path) -> bool {
    if !is_real_file(path).await {
        return false;
    }

    for root in get_all_trajectories_dirs(gcx.clone()).await {
        if canonical_child_path_under_root(&root, path).await.is_ok() {
            return true;
        }
    }
    for root in list_task_trajectory_dirs(&gcx).await {
        if canonical_child_path_under_root(&root, path).await.is_ok() {
            return true;
        }
    }
    if let Ok(root) = get_buddy_conversations_dir(gcx).await {
        if is_real_dir(&root).await && canonical_child_path_under_root(&root, path).await.is_ok() {
            return true;
        }
    }
    false
}

async fn resolve_trajectory_data_save_path(
    gcx: Arc<GlobalContext>,
    id: &str,
    data: &TrajectoryData,
) -> Result<PathBuf, String> {
    validate_trajectory_id(id).map_err(|e| e.message)?;
    if data.id != id {
        return Err("ID mismatch".to_string());
    }

    let task_meta_value = data.extra.get("task_meta").filter(|value| !value.is_null());
    let buddy_meta_present = data
        .extra
        .get("buddy_meta")
        .is_some_and(|value| !value.is_null());

    if task_meta_value.is_some() && buddy_meta_present {
        return Err("Trajectory cannot contain both task_meta and buddy_meta".to_string());
    }

    if let Some(value) = task_meta_value {
        let task_meta = serde_json::from_value::<super::types::TaskMeta>(value.clone())
            .map_err(|e| format!("Invalid task_meta: {}", e))?;
        return safe_new_task_trajectory_file(gcx.clone(), &task_meta, id).await;
    }

    if buddy_meta_present {
        return safe_new_buddy_trajectory_file(gcx.clone(), id).await;
    }

    if let Some(candidate) = find_normal_trajectory_file(gcx.clone(), id).await {
        return Ok(candidate.path);
    }
    let root_chat_id = data
        .extra
        .get("root_chat_id")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    safe_new_nested_trajectory_file(gcx.clone(), id, root_chat_id.as_deref()).await
}

async fn title_generation_backing_file_matches(
    file_path: &Path,
    chat_id: &str,
    source: &TrajectorySourceIdentity,
) -> bool {
    let content = match fs::read_to_string(file_path).await {
        Ok(content) => content,
        Err(e) => {
            warn!(
                "Skipping title update for {}: failed to read backing trajectory: {}",
                file_path.display(),
                e
            );
            return false;
        }
    };
    let parsed = tokio::task::spawn_blocking(move || serde_json::from_str(&content)).await;
    let json: serde_json::Value = match parsed {
        Ok(Ok(json)) => json,
        Ok(Err(e)) => {
            warn!(
                "Skipping title update for {}: failed to parse backing trajectory: {}",
                file_path.display(),
                e
            );
            return false;
        }
        Err(e) => {
            warn!(
                "Skipping title update for {}: parse task failed: {}",
                file_path.display(),
                e
            );
            return false;
        }
    };
    if !trajectory_root_id_matches(&json, chat_id, file_path) {
        return false;
    }
    let actual_source = match TrajectorySourceIdentity::from_json(&json) {
        Ok(source) => source,
        Err(e) => {
            warn!(
                "Skipping title update for {}: backing trajectory source is invalid: {}",
                file_path.display(),
                e
            );
            return false;
        }
    };
    if &actual_source != source {
        warn!(
            "Skipping title update for {}: backing trajectory source mismatch, expected {:?}, found {:?}",
            file_path.display(),
            source,
            actual_source
        );
        return false;
    }
    true
}

fn repaired_created_at(raw_created_at: Option<&str>, transition_identity_repaired: bool) -> String {
    if transition_identity_repaired {
        if let Some(created_at) = raw_created_at {
            if parsed_rfc3339_utc(created_at).is_some() {
                return created_at.to_string();
            }
        }
        return chrono::Utc::now().to_rfc3339();
    }

    raw_created_at
        .map(ToString::to_string)
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339())
}

fn parse_worktree_meta(value: &serde_json::Value) -> Option<WorktreeMeta> {
    if value.is_null() {
        return None;
    }
    serde_json::from_value(value.clone()).ok()
}

fn trajectory_worktree_from_extra(
    extra: &serde_json::Map<String, serde_json::Value>,
) -> Option<WorktreeMeta> {
    extra.get("worktree").and_then(parse_worktree_meta)
}

fn sanitize_worktree_extra(
    extra: &mut serde_json::Map<String, serde_json::Value>,
) -> Option<WorktreeMeta> {
    let Some(value) = extra.get("worktree") else {
        return None;
    };
    if value.is_null() {
        extra.remove("worktree");
        return None;
    }
    match serde_json::from_value::<WorktreeMeta>(value.clone()) {
        Ok(worktree) => Some(worktree),
        Err(e) => {
            warn!("Ignoring invalid trajectory worktree metadata: {}", e);
            extra.remove("worktree");
            None
        }
    }
}

async fn worktree_service_from_gcx(
    app: AppState,
    requested_source_root: Option<&Path>,
) -> Result<WorktreeService, String> {
    let cache_dir = app.paths.cache_dir.clone();
    let project_dirs = get_project_dirs(app.gcx.clone()).await;
    if project_dirs.is_empty() {
        return Err("No project root available".to_string());
    }
    let source_root = match requested_source_root {
        Some(requested) => {
            let requested = tokio::fs::canonicalize(requested).await.map_err(|e| {
                format!(
                    "Failed to resolve worktree source root '{}': {}",
                    requested.display(),
                    e
                )
            })?;
            let requested = dunce::simplified(&requested).to_path_buf();
            let mut matches = false;
            for dir in &project_dirs {
                if tokio::fs::canonicalize(dir)
                    .await
                    .map(|canonical| dunce::simplified(&canonical).to_path_buf() == requested)
                    .unwrap_or(false)
                {
                    matches = true;
                    break;
                }
            }
            if !matches {
                return Err("Worktree source root is not a current workspace directory".to_string());
            }
            requested
        }
        None => project_dirs[0].clone(),
    };
    WorktreeService::new_async(cache_dir, source_root).await
}

async fn validate_loaded_worktree_strict(
    app: AppState,
    chat_id: &str,
    worktree: WorktreeMeta,
) -> Option<WorktreeMeta> {
    let service =
        match worktree_service_from_gcx(app.clone(), Some(&worktree.source_workspace_root)).await {
            Ok(service) => service,
            Err(e) => {
                warn!(
                    "Ignoring trajectory worktree metadata for chat {}: {}",
                    chat_id, e
                );
                return None;
            }
        };
    match service.validate_worktree_meta_strict(&worktree).await {
        Ok(validated) => Some(validated),
        Err(e) => {
            debug!(
                "Ignoring untrusted trajectory worktree metadata for chat {}: {}",
                chat_id, e
            );
            None
        }
    }
}

async fn validate_loaded_legacy_task_agent_worktree(
    app: AppState,
    chat_id: &str,
    worktree: WorktreeMeta,
) -> Option<WorktreeMeta> {
    let service =
        match worktree_service_from_gcx(app.clone(), Some(&worktree.source_workspace_root)).await {
            Ok(service) => service,
            Err(e) => {
                warn!(
                    "Ignoring legacy task-agent worktree metadata for chat {}: {}",
                    chat_id, e
                );
                return None;
            }
        };
    match service
        .validate_legacy_task_agent_worktree_meta(&worktree)
        .await
    {
        Ok(validated) => Some(validated),
        Err(e) => {
            warn!(
                "Ignoring untrusted legacy task-agent worktree metadata for chat {}: {}",
                chat_id, e
            );
            None
        }
    }
}

async fn synthesize_legacy_task_agent_worktree(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
    task_meta: Option<&super::types::TaskMeta>,
) -> Option<WorktreeMeta> {
    let task_meta = task_meta?;
    if task_meta.role != "agents" {
        return None;
    }

    let source_workspace_root = get_project_dirs(gcx.clone()).await.into_iter().next();
    let task_record = crate::tasks::storage::load_task_meta(gcx.clone(), &task_meta.task_id)
        .await
        .ok();
    let board = crate::tasks::storage::load_board(gcx, &task_meta.task_id)
        .await
        .ok()?;

    let card = if let Some(card_id) = task_meta.card_id.as_deref() {
        board.cards.iter().find(|card| card.id == card_id)?
    } else {
        board
            .cards
            .iter()
            .find(|card| card.agent_chat_id.as_deref() == Some(chat_id))?
    };
    if card.agent_chat_id.as_deref() != Some(chat_id) {
        return None;
    }
    if let Some(agent_id) = task_meta.agent_id.as_deref() {
        if card.assignee.as_deref() != Some(agent_id) {
            return None;
        }
    }

    let root = PathBuf::from(card.agent_worktree.as_ref()?);
    let source_workspace_root = source_workspace_root.unwrap_or_else(|| root.clone());
    let id = card
        .agent_worktree_name
        .clone()
        .or_else(|| {
            root.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.to_string())
        })
        .unwrap_or_else(|| format!("{}-{}", task_meta.task_id, card.id));

    Some(WorktreeMeta {
        id,
        kind: "task_agent".to_string(),
        root,
        source_workspace_root: source_workspace_root.clone(),
        repo_root: source_workspace_root,
        branch: card.agent_branch.clone(),
        base_branch: if card.base_branch.is_some() || card.base_commit.is_some() {
            card.base_branch.clone()
        } else {
            task_record
                .as_ref()
                .and_then(|meta| meta.base_branch.clone())
        },
        base_commit: if card.base_branch.is_some() || card.base_commit.is_some() {
            card.base_commit.clone()
        } else {
            task_record
                .as_ref()
                .and_then(|meta| meta.base_commit.clone())
        },
        task_id: Some(task_meta.task_id.clone()),
        card_id: Some(card.id.clone()),
        agent_id: task_meta.agent_id.clone().or_else(|| card.assignee.clone()),
        enforce: true,
    })
}

fn trajectory_root_id_matches(t: &serde_json::Value, chat_id: &str, path: &Path) -> bool {
    match t.get("id").and_then(|value| value.as_str()) {
        Some(id) if id == chat_id => true,
        Some(id) => {
            warn!(
                "Rejecting trajectory {}: JSON id mismatch, expected {}, found {}",
                path.display(),
                chat_id,
                id
            );
            false
        }
        None => {
            warn!(
                "Rejecting trajectory {}: missing or non-string JSON id for requested chat {}",
                path.display(),
                chat_id
            );
            false
        }
    }
}

fn trajectory_candidate_has_minimum_schema(
    t: &serde_json::Value,
    chat_id: &str,
    path: &Path,
) -> bool {
    let Some(root) = t.as_object() else {
        warn!(
            "Rejecting trajectory {}: JSON root must be an object for requested chat {}",
            path.display(),
            chat_id
        );
        return false;
    };
    if !trajectory_root_id_matches(t, chat_id, path) {
        return false;
    }
    if root
        .get("messages")
        .is_some_and(|messages| messages.is_array())
    {
        return true;
    }
    warn!(
        "Rejecting trajectory {}: missing or non-array JSON messages for requested chat {}",
        path.display(),
        chat_id
    );
    false
}

pub(crate) fn trajectory_path_stem_matches_id(path: &Path, id: &str) -> bool {
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return false;
    };
    if stem == id {
        return true;
    }
    warn!(
        "Ignoring trajectory {} in list: filename id mismatch, expected {}, found {}",
        path.display(),
        stem,
        id
    );
    false
}

pub(crate) async fn normalize_legacy_planner_copy(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
    mode: &str,
    link_type: Option<&str>,
    task_meta: &mut Option<TaskMeta>,
) {
    let Some(meta) = task_meta.as_mut().filter(|meta| meta.role == "planner") else {
        return;
    };
    if !matches!(link_type, Some("mode_transition" | "handoff"))
        || mode.trim().is_empty()
        || crate::yaml_configs::customization_registry::map_legacy_mode_to_id(mode)
            == "task_planner"
    {
        return;
    }
    let Ok(board) = crate::tasks::storage::load_board_unlocked(gcx, &meta.task_id).await else {
        return;
    };
    if board
        .owner_chat_id
        .as_deref()
        .is_some_and(|owner| !owner.is_empty() && owner != chat_id)
        && !board
            .reviewer_chat_ids
            .iter()
            .any(|reviewer| reviewer == chat_id)
    {
        meta.role = "linked".into();
    }
}

async fn load_trajectory_candidate(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
    candidate: ValidTrajectoryCandidate,
) -> Option<LoadedTrajectory> {
    load_trajectory_candidate_with_delivery_view(gcx, chat_id, candidate, false).await
}

async fn load_trajectory_candidate_with_delivery_view(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
    candidate: ValidTrajectoryCandidate,
    durable_deliveries: bool,
) -> Option<LoadedTrajectory> {
    let app = AppState::from_gcx(gcx.clone()).await;
    let traj_path = candidate.path;
    let mut t = candidate.json;

    let messages_value = t
        .as_object_mut()
        .and_then(|object| object.remove("messages"))
        .unwrap_or_default();
    let persisted_goal_value = t.as_object_mut().and_then(|object| object.remove("goal"));
    let goal_ledger_value = t
        .as_object_mut()
        .and_then(|object| object.remove("goal_ledger"));
    let pending_deliveries = t
        .as_object_mut()
        .and_then(|object| object.remove("pending_deliveries"))
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let dropped_deliveries: Vec<refact_core::chat_types::DeliveryStatusRecord> = t
        .as_object_mut()
        .and_then(|object| object.remove("dropped_deliveries"))
        .and_then(|value| {
            serde_json::from_value::<Vec<refact_core::chat_types::DeliveryStatusRecord>>(value).ok()
        })
        .unwrap_or_default();
    let (messages, goal_ledger, goal): (
        Vec<ChatMessage>,
        Vec<GoalLedgerEntry>,
        Option<GoalSnapshot>,
    ) = tokio::task::spawn_blocking(move || {
        // Nested reconstruction payloads stay opaque in `extra` during load.
        // Invalid boundaries remain browsable and are rejected only by the send
        // projection. Never replace a failed transcript parse with empty history.
        let mut messages: Vec<ChatMessage> = match parse_stored_messages(messages_value) {
            Ok(messages) => messages,
            Err(error) => {
                tracing::warn!(%error, "Invalid stored trajectory messages; refusing empty-history fallback");
                return None;
            }
        };
        fix_tool_call_indexes(&mut messages);

        for (message_index, msg) in messages.iter_mut().enumerate() {
            if msg.message_id.is_empty() {
                let role = msg.role.clone();
                let content = msg.content.content_text_only();
                let source = msg
                    .extra
                    .get("event")
                    .and_then(|event| event.get("source"))
                    .and_then(|source| source.as_str())
                    .unwrap_or_default();
                msg.message_id = format!(
                    "legacy:{}:{:x}",
                    role,
                    md5::compute(
                        format!("{message_index}\n{role}\n{source}\n{content}").as_bytes()
                    )
                );
            }

            if let Some(tool_calls) = &msg.tool_calls {
                let filtered: Vec<_> = tool_calls
                    .iter()
                    .filter(|tc| !tc.function.name.is_empty())
                    .cloned()
                    .collect();

                if filtered.len() != tool_calls.len() {
                    tracing::warn!(
                        "Filtered out {} tool call(s) with empty names from message {}",
                        tool_calls.len() - filtered.len(),
                        msg.message_id
                    );
                }

                msg.tool_calls = if filtered.is_empty() {
                    None
                } else {
                    Some(filtered)
                };
            }
        }
        let persisted_goal: Option<GoalSnapshot> = persisted_goal_value
            .and_then(|value| serde_json::from_value(value).ok())
            .map(clamp_goal_snapshot_for_load);
        let goal_ledger: Vec<GoalLedgerEntry> = goal_ledger_value
            .and_then(|value| value.as_array().cloned())
            .map(|entries| {
                entries
                    .into_iter()
                    .filter_map(|entry| match serde_json::from_value(entry) {
                        Ok(parsed) => Some(parsed),
                        Err(error) => {
                            tracing::warn!("skipping unrecognized goal_ledger entry: {error}");
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let ledger_prior = refact_chat_api::reduce_goal_ledger(&goal_ledger).map(|state| {
            let mut seed = persisted_goal.clone().unwrap_or_else(|| GoalSnapshot {
                version: state.version,
                budget: state.budget.clone(),
                ..Default::default()
            });
            state.apply_to_snapshot(&mut seed);
            seed
        });
        let goal_prior = persisted_goal.or(ledger_prior);
        let goal = super::session::goal_snapshot_from_messages(&messages, goal_prior.as_ref())
            .or(goal_prior)
            .map(clamp_goal_snapshot_for_load);
        Some((messages, goal_ledger, goal))
    })
    .await
    .map_err(|e| {
        warn!(
            "Failed to deserialize trajectory messages for chat {}: {}",
            chat_id, e
        );
    })
    .ok()??;

    let mut task_meta: Option<super::types::TaskMeta> = t
        .get("task_meta")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    normalize_legacy_planner_copy(
        gcx.clone(),
        chat_id,
        t.get("mode")
            .and_then(|value| value.as_str())
            .unwrap_or_default(),
        t.get("link_type").and_then(|value| value.as_str()),
        &mut task_meta,
    )
    .await;

    let worktree = if let Some(candidate) = t.get("worktree").and_then(parse_worktree_meta) {
        validate_loaded_worktree_strict(app.clone(), chat_id, candidate).await
    } else if let Some(candidate) =
        synthesize_legacy_task_agent_worktree(gcx.clone(), chat_id, task_meta.as_ref()).await
    {
        validate_loaded_legacy_task_agent_worktree(app.clone(), chat_id, candidate).await
    } else {
        None
    };

    let wake_up_at = t
        .get("wake_up_at")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    let waiting_for_card_ids: Vec<String> = t
        .get("waiting_for_card_ids")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let goal_verification_blocked_until_ms = t
        .get("goal_verification_blocked_until_ms")
        .and_then(|v| v.as_u64());
    let native_decision_receipts = t
        .get("native_decision_receipts")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let pending_questions = t
        .get("pending_questions")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let pending_browser_decision = t
        .get("pending_browser_decision")
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    let parent_id = t
        .get("parent_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let link_type = t
        .get("link_type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let root_chat_id = t
        .get("root_chat_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let previous_response_id = t
        .get("previous_response_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let frozen_request_prefix_value = t.get("frozen_request_prefix").filter(|v| !v.is_null());
    let loaded_frozen_request_prefix =
        frozen_request_prefix_value.and_then(|v| serde_json::from_value(v.clone()).ok());
    let claude_code_identity_value = t.get("claude_code_identity").filter(|v| !v.is_null());
    let provider_state_present =
        previous_response_id.is_some() || claude_code_identity_value.is_some();
    let (frozen_request_prefix, transition_prefix_repaired) = repair_transition_frozen_prefix(
        link_type.as_deref(),
        &messages,
        loaded_frozen_request_prefix,
        raw_frozen_prefix_has_tools(frozen_request_prefix_value),
        t.get("created_at").and_then(|v| v.as_str()),
        provider_state_present,
    );
    let claude_code_identity = if transition_prefix_repaired {
        None
    } else {
        claude_code_identity_value.and_then(|v| serde_json::from_value(v.clone()).ok())
    };
    let previous_response_id = if transition_prefix_repaired {
        None
    } else {
        previous_response_id
    };

    let thread = ThreadParams {
        id: chat_id.to_string(),
        title: t
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("New Chat")
            .to_string(),
        model: t
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        mode: crate::yaml_configs::customization_registry::map_legacy_mode_to_id(
            t.get("mode").and_then(|v| v.as_str()).unwrap_or("agent"),
        )
        .to_string(),
        tool_use: t
            .get("tool_use")
            .and_then(|v| v.as_str())
            .unwrap_or("agent")
            .to_string(),
        boost_reasoning: t.get("boost_reasoning").and_then(|v| v.as_bool()),
        context_tokens_cap: t
            .get("context_tokens_cap")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize),
        auto_compression_cap_pending: t
            .get("auto_compression_cap_pending")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        recovery_policy: t
            .get("recovery_policy")
            .and_then(|v| serde_json::from_value(v.clone()).ok()),
        auto_compression_cap: t
            .get("auto_compression_cap")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize),
        include_project_info: t
            .get("include_project_info")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        checkpoints_enabled: t
            .get("checkpoints_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        is_title_generated: t
            .get("isTitleGenerated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        auto_approve_editing_tools: t
            .get("auto_approve_editing_tools")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        auto_approve_dangerous_commands: t
            .get("auto_approve_dangerous_commands")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        autonomous_no_confirm: t
            .get("autonomous_no_confirm")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        task_meta,
        worktree,
        parent_id,
        link_type,
        root_chat_id,
        reasoning_effort: t
            .get("reasoning_effort")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        thinking_budget: t
            .get("thinking_budget")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize),
        temperature: t
            .get("temperature")
            .and_then(|v| v.as_f64())
            .map(|n| n as f32),
        frequency_penalty: t
            .get("frequency_penalty")
            .and_then(|v| v.as_f64())
            .map(|n| n as f32),
        max_tokens: t
            .get("max_tokens")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize),
        parallel_tool_calls: t.get("parallel_tool_calls").and_then(|v| v.as_bool()),

        previous_response_id,

        browser_meta: t
            .get("browser_meta")
            .and_then(|v| serde_json::from_value(v.clone()).ok()),

        active_skill: t
            .get("active_skill")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),

        auto_enrichment_enabled: t.get("auto_enrichment_enabled").and_then(|v| v.as_bool()),

        buddy_meta: t
            .get("buddy_meta")
            .and_then(|v| serde_json::from_value(v.clone()).ok()),

        auto_compact_enabled: t.get("auto_compact_enabled").and_then(|v| v.as_bool()),
        frozen_request_prefix,
        claude_code_identity,
        app_workspace: super::app_workspace::stored_app_workspace_from_value(
            t.get("app_workspace"),
        ),
        // Runtime-only: saves strip this key and loads ignore injected values
        // (pinned by `reactive_compact_attempts_is_runtime_only_not_persisted`),
        // so a stale in-flight retry counter can never survive a reload.
        reactive_compact_attempts: None,
    };

    let auto_approve_editing_tools_present = t
        .get("auto_approve_editing_tools")
        .and_then(|v| v.as_bool())
        .is_some();
    let auto_approve_dangerous_commands_present = t
        .get("auto_approve_dangerous_commands")
        .and_then(|v| v.as_bool())
        .is_some();

    let created_at = repaired_created_at(
        t.get("created_at").and_then(|v| v.as_str()),
        transition_prefix_repaired,
    );

    let updated_at = t
        .get("updated_at")
        .and_then(|v| v.as_str())
        .unwrap_or(&created_at)
        .to_string();

    let pending_deliveries =
        if let Some(agent_id) = app.agents.find_agent_id_by_child_chat_id(chat_id).await {
            if !durable_deliveries && app.agents.has_runtime(&agent_id).await {
                Vec::new()
            } else {
                pending_deliveries
            }
        } else {
            pending_deliveries
        };

    let mut recovery: Option<refact_core::recovery::RecoveryRecord> = t
        .get("recovery")
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    if let Some(record) = &mut recovery {
        record.interrupt_on_reload();
        record.latest_error.raw_error =
            super::diagnostics::safe_provider_error_diagnostic(&record.latest_error.raw_error);
        for attempt in &mut record.attempts {
            if let Some(error) = &mut attempt.error {
                error.raw_error =
                    super::diagnostics::safe_provider_error_diagnostic(&error.raw_error);
            }
        }
    }
    Some(LoadedTrajectory {
        recovery,
        pending_deliveries,
        dropped_deliveries,
        goal_ledger,
        goal_verification_blocked_until_ms,
        pending_questions,
        native_decision_receipts,
        pending_browser_decision,
        source_path: traj_path,
        messages,
        thread,
        goal,
        created_at,
        updated_at,
        wake_up_at,
        waiting_for_card_ids,
        auto_approve_editing_tools_present,
        auto_approve_dangerous_commands_present,
        transition_identity_repaired: transition_prefix_repaired,
    })
}

pub async fn load_trajectory_for_chat(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
) -> Option<LoadedTrajectory> {
    let candidate = find_trajectory_or_buddy_file(gcx.clone(), chat_id).await?;
    load_trajectory_candidate(gcx, chat_id, candidate).await
}

pub(crate) async fn load_trajectory_for_recovery(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
) -> Option<LoadedTrajectory> {
    let candidate = find_trajectory_or_buddy_file(gcx.clone(), chat_id).await?;
    load_trajectory_candidate_with_delivery_view(gcx, chat_id, candidate, true).await
}

pub(crate) async fn rebind_persisted_task_agent(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    chat_id: &str,
) -> Result<bool, String> {
    let path = find_trajectory_path(gcx.clone(), chat_id)
        .await
        .ok_or("Agent trajectory unavailable")?;
    let (_, guard) = crate::tasks::storage::locked_task_dir(gcx.clone(), task_id).await?;
    let board = crate::tasks::storage::load_board_locked(gcx.clone(), task_id, &guard).await?;
    let Some(owner) = board.owner_chat_id.as_deref() else {
        return Ok(false);
    };
    super::trajectory_archive::with_trajectory_mutation_lock(async {
        if gcx.chat_sessions.read().await.contains_key(chat_id) {
            return Ok(false);
        }
        crate::tasks::storage::require_regular_task_file(&path).await?;
        let body = fs::read_to_string(&path)
            .await
            .map_err(|error| error.to_string())?;
        let mut value: serde_json::Value =
            serde_json::from_str(&body).map_err(|error| error.to_string())?;
        let meta = value.get("task_meta").ok_or("Agent task binding missing")?;
        let bound = meta
            .get("planner_chat_id")
            .and_then(|value| value.as_str())
            .ok_or("Agent planner missing")?
            .to_string();
        let card_id = meta
            .get("card_id")
            .and_then(|value| value.as_str())
            .ok_or("Agent card missing")?;
        if value["id"] != chat_id
            || meta["task_id"] != task_id
            || meta["role"] != "agents"
            || bound == owner
            || !board.planner_binding_is_current(&bound)
        {
            return Ok(false);
        }
        let Some(card) = board
            .get_card(card_id)
            .filter(|card| card.column == "doing")
        else {
            return Ok(false);
        };
        if card.agent_chat_id.as_deref() != Some(chat_id)
            && !card
                .team_members
                .iter()
                .any(|member| member.agent_chat_id.as_deref() == Some(chat_id))
            && !card.ab_variants.as_ref().is_some_and(|variants| {
                [&variants.a, &variants.b]
                    .iter()
                    .any(|variant| variant.chat_id == chat_id && variant.finish.is_none())
            })
        {
            return Ok(false);
        }
        value["task_meta"]["planner_chat_id"] = json!(owner);
        value["parent_id"] = json!(owner);
        value["root_chat_id"] = json!(owner);
        let body = serde_json::to_vec_pretty(&value).map_err(|error| error.to_string())?;
        crate::file_activity::write_temp_then_rename(&path, &body, None).await?;
        crate::file_activity::sync_dir(path.parent().ok_or("Agent trajectory parent missing")?)
            .await?;
        gcx.agents.rebind_parent(chat_id, &bound, owner).await?;
        Ok(true)
    })
    .await
}

pub fn validate_trajectory_label(label: &str) -> Result<String, String> {
    let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
    if label.is_empty() {
        return Err("argument `label` must be non-empty".to_string());
    }
    if label.chars().count() > TRAJECTORY_LABEL_MAX_CHARS {
        return Err(format!(
            "argument `label` must be at most {TRAJECTORY_LABEL_MAX_CHARS} characters"
        ));
    }
    Ok(label)
}

pub async fn set_trajectory_label(app: AppState, chat_id: &str, label: &str) -> Result<(), String> {
    validate_trajectory_id(chat_id).map_err(|error| error.message)?;
    let label = validate_trajectory_label(label)?;
    let session_arc = app
        .chat
        .sessions
        .read()
        .await
        .get(chat_id)
        .cloned()
        .ok_or_else(|| format!("Live trajectory '{chat_id}' not found"))?;
    {
        let mut session = session_arc.lock().await;
        session.set_title_from_trajectory_label(label);
    }
    try_save_trajectory_with_intent(app, session_arc, TrajectoryCommitIntent::Required)
        .await
        .map(|_| ())
}

pub async fn load_generic_trajectory_for_chat(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
) -> Option<LoadedTrajectory> {
    let candidate = find_trajectory_file(gcx.clone(), chat_id).await?;
    load_trajectory_candidate(gcx, chat_id, candidate).await
}

fn task_meta_is_planner_for_task(task_meta: Option<&TaskMeta>, task_id: &str) -> bool {
    task_meta.is_some_and(|meta| meta.role == "planner" && meta.task_id == task_id)
}

pub(crate) async fn chat_id_is_planner_for_task(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
    task_id: &str,
) -> bool {
    let session_arc = {
        let sessions = gcx.chat_sessions.read().await;
        sessions.get(chat_id).cloned()
    };
    if let Some(session_arc) = session_arc {
        let (mut task_meta, mode, link_type) = {
            let session = session_arc.lock().await;
            (
                session.thread.task_meta.clone(),
                session.thread.mode.clone(),
                session.thread.link_type.clone(),
            )
        };
        normalize_legacy_planner_copy(
            gcx.clone(),
            chat_id,
            &mode,
            link_type.as_deref(),
            &mut task_meta,
        )
        .await;
        return task_meta_is_planner_for_task(task_meta.as_ref(), task_id);
    }

    load_trajectory_for_chat(gcx, chat_id)
        .await
        .is_some_and(|loaded| {
            task_meta_is_planner_for_task(loaded.thread.task_meta.as_ref(), task_id)
        })
}

pub(crate) async fn verified_planner_linked_root_chat_id(
    gcx: Arc<GlobalContext>,
    current_chat_id: &str,
    current_root_chat_id: Option<&str>,
    task_meta: &TaskMeta,
) -> Option<String> {
    if task_meta.role == "planner" {
        return None;
    }
    let root_chat_id =
        current_root_chat_id.filter(|id| !id.is_empty() && *id != current_chat_id)?;
    if task_meta.planner_chat_id.as_deref() == Some(root_chat_id) {
        return Some(root_chat_id.to_string());
    }
    if chat_id_is_planner_for_task(gcx, root_chat_id, &task_meta.task_id).await {
        return Some(root_chat_id.to_string());
    }
    None
}

pub(crate) async fn resolve_task_planner_controller_chat_id(
    gcx: Arc<GlobalContext>,
    current_chat_id: &str,
    current_root_chat_id: Option<&str>,
    task_meta: Option<&TaskMeta>,
) -> String {
    let Some(task_meta) = task_meta else {
        return current_chat_id.to_string();
    };
    if let Some(root_chat_id) =
        verified_planner_linked_root_chat_id(gcx, current_chat_id, current_root_chat_id, task_meta)
            .await
    {
        return root_chat_id;
    }
    if task_meta.role == "planner" {
        return task_meta
            .planner_chat_id
            .clone()
            .filter(|id| {
                !id.is_empty()
                    && current_root_chat_id.filter(|root_id| *root_id != current_chat_id)
                        != Some(id.as_str())
            })
            .unwrap_or_else(|| current_chat_id.to_string());
    }
    task_meta
        .planner_chat_id
        .clone()
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| current_chat_id.to_string())
}

fn trajectory_source_matches_hint(
    actual: &TrajectorySourceIdentity,
    hint: &TrajectorySourceIdentity,
) -> bool {
    match (actual, hint) {
        (
            TrajectorySourceIdentity::Task {
                task_id,
                role,
                agent_id,
                card_id,
                planner_chat_id,
            },
            TrajectorySourceIdentity::Task {
                task_id: hint_task_id,
                role: hint_role,
                agent_id: hint_agent_id,
                card_id: hint_card_id,
                planner_chat_id: hint_planner_chat_id,
            },
        ) => {
            task_id == hint_task_id
                && role == hint_role
                && hint_agent_id
                    .as_ref()
                    .is_none_or(|hint_agent_id| Some(hint_agent_id) == agent_id.as_ref())
                && hint_card_id
                    .as_ref()
                    .is_none_or(|hint_card_id| Some(hint_card_id) == card_id.as_ref())
                && hint_planner_chat_id
                    .as_ref()
                    .is_none_or(|hint_planner_chat_id| {
                        Some(hint_planner_chat_id) == planner_chat_id.as_ref()
                    })
        }
        (left, right) => left == right,
    }
}

fn effective_trajectory_source_for_path(
    source: TrajectorySourceIdentity,
    path: &Path,
    task_roots: &[PathBuf],
) -> TrajectorySourceIdentity {
    let path_source = trajectory_source_identity_from_path(path, task_roots);
    if matches!(&source, TrajectorySourceIdentity::Normal)
        && !matches!(&path_source, TrajectorySourceIdentity::Normal)
    {
        path_source
    } else {
        source
    }
}

async fn load_generic_trajectory_for_chat_matching_source(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
    source: &TrajectorySourceIdentity,
) -> Option<LoadedTrajectory> {
    validate_trajectory_id(chat_id).ok()?;
    let task_roots = get_all_task_roots(gcx.clone()).await;
    let paths = trajectory_candidate_paths(gcx.clone(), chat_id).await;
    for path in paths {
        let Some(candidate) = read_valid_trajectory_candidate(path, chat_id).await else {
            continue;
        };
        let candidate_source = match TrajectorySourceIdentity::from_json(&candidate.json) {
            Ok(candidate_source) => {
                effective_trajectory_source_for_path(candidate_source, &candidate.path, &task_roots)
            }
            Err(e) => {
                warn!(
                    "Skipping trajectory candidate {} for chat {}: invalid source: {}",
                    candidate.path.display(),
                    chat_id,
                    e
                );
                continue;
            }
        };
        if !trajectory_source_matches_hint(&candidate_source, source) {
            continue;
        }
        return load_trajectory_candidate(gcx, chat_id, candidate).await;
    }
    None
}

pub async fn save_initial_task_chat_trajectory(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    chat_id: &str,
    mode: &str,
) -> Result<(), String> {
    let greeting = "## 🎯 Task Planner

I'm your **Task Planner**. I handle the complete task lifecycle - from investigation to execution.

**Planning Phase:**
- Analyze the codebase using search and exploration tools
- Create task cards with clear acceptance criteria
- Set priorities and dependencies between cards

**Execution Phase:**
- Spawn agents to work on ready cards (each in isolated git worktree)
- Monitor agent progress and receive completion notifications
- Merge successful work back to main branch
- Handle failures and coordinate retries

**How to use me:**
1. Describe what you want to accomplish
2. I'll investigate and create a structured plan (task cards)
3. When ready, I'll spawn agents to implement each card
4. I'll notify you as work completes and handle merging
5. We iterate until the task is done

**Ready when you are!** Tell me what you'd like to build or fix.";

    let greeting_msg = ChatMessage {
        message_id: Uuid::new_v4().to_string(),
        role: "assistant".to_string(),
        content: ChatContent::SimpleText(greeting.to_string()),
        finish_reason: Some("stop".to_string()),
        ..Default::default()
    };

    let task_meta = super::types::TaskMeta {
        task_id: task_id.to_string(),
        role: "planner".to_string(),
        agent_id: None,
        card_id: None,
        planner_chat_id: Some(chat_id.to_string()),
    };

    let snapshot = TrajectorySnapshot {
        recovery: None,
        recovery_policy: None,
        goal: None,
        goal_ledger: Vec::new(),
        pending_deliveries: Vec::new(),
        dropped_deliveries: Vec::new(),
        goal_verification_blocked_until_ms: None,
        pending_browser_decision: None,
        chat_id: chat_id.to_string(),
        title: String::new(),
        model: String::new(),
        mode: mode.to_string(),
        tool_use: "agent".to_string(),
        messages: if mode == "task_planner" {
            vec![greeting_msg]
        } else {
            Vec::new()
        },
        created_at: chrono::Utc::now().to_rfc3339(),
        boost_reasoning: false,
        checkpoints_enabled: true,
        context_tokens_cap: None,
        auto_compression_cap: None,
        auto_compression_cap_pending: true,
        include_project_info: true,
        is_title_generated: false,
        auto_approve_editing_tools: true,
        auto_approve_dangerous_commands: true,
        autonomous_no_confirm: false,
        auto_enrichment_enabled: Some(false),
        task_meta: Some(task_meta),
        worktree: None,
        version: 1,
        parent_id: None,
        link_type: None,
        root_chat_id: None,
        reasoning_effort: None,
        thinking_budget: None,
        temperature: None,
        frequency_penalty: None,
        max_tokens: None,
        parallel_tool_calls: None,
        previous_response_id: None,
        active_skill: None,
        buddy_meta: None,
        auto_compact_enabled: None,
        frozen_request_prefix: None,
        claude_code_identity: None,
        reactive_compact_attempts: None,
        wake_up_at: None,
        waiting_for_card_ids: Vec::new(),
        pending_questions: Vec::new(),
        native_decision_receipts: Vec::new(),
        app_workspace: None,
    };

    save_trajectory_snapshot(gcx, snapshot).await
}

pub async fn save_trajectory_as(
    gcx: Arc<GlobalContext>,
    thread: &ThreadParams,
    messages: &[ChatMessage],
) {
    save_trajectory_as_with_intent(gcx, thread, messages, TrajectoryCommitIntent::Required).await;
}

pub async fn save_trajectory_as_with_intent(
    gcx: Arc<GlobalContext>,
    thread: &ThreadParams,
    messages: &[ChatMessage],
    intent: TrajectoryCommitIntent,
) {
    if let Err(e) = save_trajectory_as_with_intent_checked(gcx, thread, messages, intent).await {
        warn!("Failed to save trajectory: {}", e);
    }
}

pub async fn save_trajectory_as_with_intent_checked(
    gcx: Arc<GlobalContext>,
    thread: &ThreadParams,
    messages: &[ChatMessage],
    intent: TrajectoryCommitIntent,
) -> Result<(), String> {
    if !intent.persists() {
        return Ok(());
    }
    let existing_session = gcx.chat_sessions.read().await.get(&thread.id).cloned();
    let (pending_questions, pending_browser_decision, native_decision_receipts) =
        if let Some(session) = existing_session {
            let session = session.lock().await;
            let browser = session.pending_browser_message.as_ref().map(|pending| {
                super::browser_decisions::snapshot_pending_browser_decision(pending, None)
            });
            (
                session.pending_questions.clone(),
                browser,
                session.native_decision_receipts.clone(),
            )
        } else {
            load_trajectory_for_chat(gcx.clone(), &thread.id)
                .await
                .map(|trajectory| {
                    (
                        trajectory.pending_questions,
                        trajectory.pending_browser_decision,
                        trajectory.native_decision_receipts,
                    )
                })
                .unwrap_or_default()
        };
    let snapshot = TrajectorySnapshot {
        recovery: None,
        recovery_policy: None,
        goal: None,
        goal_ledger: Vec::new(),
        pending_deliveries: Vec::new(),
        dropped_deliveries: Vec::new(),
        goal_verification_blocked_until_ms: None,
        pending_browser_decision,
        chat_id: thread.id.clone(),
        title: thread.title.clone(),
        model: thread.model.clone(),
        mode: thread.mode.clone(),
        tool_use: thread.tool_use.clone(),
        messages: messages.to_vec(),
        created_at: chrono::Utc::now().to_rfc3339(),
        boost_reasoning: thread.boost_reasoning.unwrap_or(false),
        checkpoints_enabled: thread.checkpoints_enabled,
        context_tokens_cap: thread.context_tokens_cap,

        auto_compression_cap: thread.auto_compression_cap,
        auto_compression_cap_pending: thread.auto_compression_cap_pending,
        include_project_info: thread.include_project_info,
        is_title_generated: thread.is_title_generated,
        auto_approve_editing_tools: thread.auto_approve_editing_tools,
        auto_approve_dangerous_commands: thread.auto_approve_dangerous_commands,
        autonomous_no_confirm: thread.autonomous_no_confirm,
        version: 1,
        task_meta: thread.task_meta.clone(),
        worktree: thread.worktree.clone(),
        parent_id: thread.parent_id.clone(),
        link_type: thread.link_type.clone(),
        root_chat_id: thread.root_chat_id.clone(),
        reasoning_effort: thread.reasoning_effort.clone(),
        thinking_budget: thread.thinking_budget,
        temperature: thread.temperature,
        frequency_penalty: thread.frequency_penalty,
        max_tokens: thread.max_tokens,
        parallel_tool_calls: thread.parallel_tool_calls,
        previous_response_id: thread.previous_response_id.clone(),
        active_skill: thread.active_skill.clone(),
        auto_enrichment_enabled: thread.auto_enrichment_enabled,
        buddy_meta: thread.buddy_meta.clone(),
        auto_compact_enabled: thread.auto_compact_enabled,
        frozen_request_prefix: thread.frozen_request_prefix.clone(),
        claude_code_identity: thread.claude_code_identity.clone(),
        reactive_compact_attempts: thread.reactive_compact_attempts,
        wake_up_at: None,
        waiting_for_card_ids: Vec::new(),
        pending_questions,
        native_decision_receipts,
        app_workspace: thread.app_workspace.clone(),
    };
    persist_trajectory_snapshot_with_intent(gcx, snapshot, intent).await
}

pub async fn save_trajectory_snapshot(
    gcx: Arc<GlobalContext>,
    snapshot: TrajectorySnapshot,
) -> Result<(), String> {
    let message_count = snapshot.messages.len() as u64;
    let trajectory_version = snapshot.version;
    let span = perf_diagnostics::span(
        PerfComponent::TrajectoryCommit,
        Some(&snapshot.chat_id),
        None,
    );
    let result =
        persist_trajectory_snapshot_with_intent(gcx, snapshot, TrajectoryCommitIntent::Required)
            .await;
    span.finish(
        if result.is_ok() {
            PerfOutcome::Success
        } else {
            PerfOutcome::Failure
        },
        None,
        Some(message_count),
        Some(trajectory_version),
        None,
    );
    result
}

struct DetachedTrajectoryWriterState {
    pending: Option<(u64, TrajectorySnapshot)>,
    coalesce_deadline: Option<Instant>,
    requested_version: u64,
    committed_version: u64,
    in_flight: bool,
    error: Option<String>,
    error_version: u64,
    notify: Arc<Notify>,
    wake: Arc<Notify>,
    worker: Option<tokio::task::JoinHandle<()>>,
    write_cache: Arc<AMutex<DetachedTrajectoryWriteCache>>,
}

#[derive(Default)]
struct DetachedTrajectoryWriteCache {
    backing_path: Option<PathBuf>,
    existing_metadata: Option<serde_json::Map<String, serde_json::Value>>,
    source: Option<TrajectorySourceIdentity>,
    backing_stamp: Option<TrajectoryFileStamp>,
}

#[derive(Clone, PartialEq, Eq)]
struct TrajectoryFileStamp {
    len: u64,
    modified_unix_ms: i64,
}

#[derive(Clone)]
struct TrajectoryMetadataCacheEntry {
    chat_id: String,
    metadata: serde_json::Map<String, serde_json::Value>,
    stamp: TrajectoryFileStamp,
    last_access: u64,
}

const MAX_CACHED_TRAJECTORY_METADATA: usize = 256;

static TRAJECTORY_METADATA_CACHE_ACCESS_COUNTER: OnceLock<std::sync::atomic::AtomicU64> =
    OnceLock::new();

fn next_trajectory_metadata_cache_access_stamp() -> u64 {
    TRAJECTORY_METADATA_CACHE_ACCESS_COUNTER
        .get_or_init(|| std::sync::atomic::AtomicU64::new(1))
        .fetch_add(1, Ordering::Relaxed)
}

fn trajectory_metadata_cache(
) -> &'static StdMutex<std::collections::HashMap<PathBuf, TrajectoryMetadataCacheEntry>> {
    static CACHE: OnceLock<
        StdMutex<std::collections::HashMap<PathBuf, TrajectoryMetadataCacheEntry>>,
    > = OnceLock::new();
    CACHE.get_or_init(|| StdMutex::new(std::collections::HashMap::new()))
}

fn trajectory_metadata_cache_key(path: &Path) -> PathBuf {
    crate::files_correction::canonicalize_normalized_path(path.to_path_buf())
}

fn preservable_trajectory_metadata(
    source: &serde_json::Map<String, serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    source
        .iter()
        .filter(|(key, _)| {
            // `created_at` is a known key but must survive re-saves, so it is cached too.
            matches!(
                key.as_str(),
                "browser_meta" | "created_at" | "app_workspace"
            ) || !is_known_trajectory_top_level_key(key)
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn store_trajectory_metadata_cache_entry(path: &Path, entry: TrajectoryMetadataCacheEntry) {
    let path = trajectory_metadata_cache_key(path);
    let mut cache = trajectory_metadata_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !cache.contains_key(&path) {
        while cache.len() >= MAX_CACHED_TRAJECTORY_METADATA {
            let Some(victim) = cache
                .iter()
                .min_by_key(|(_, entry)| entry.last_access)
                .map(|(path, _)| path.clone())
            else {
                break;
            };
            cache.remove(&victim);
        }
    }
    let mut entry = entry;
    entry.last_access = next_trajectory_metadata_cache_access_stamp();
    cache.insert(path, entry);
}

async fn trajectory_file_stamp(path: &Path) -> Option<TrajectoryFileStamp> {
    let metadata = fs::symlink_metadata(path).await.ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return None;
    }
    Some(TrajectoryFileStamp {
        len: metadata.len(),
        modified_unix_ms: metadata
            .modified()
            .ok()?
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_millis()
            .try_into()
            .ok()?,
    })
}

async fn trajectory_stamp_matches(path: &Path, expected: &TrajectoryFileStamp) -> bool {
    trajectory_file_stamp(path).await.as_ref() == Some(expected)
}

async fn cached_existing_trajectory_object(
    path: &Path,
    chat_id: &str,
) -> Result<Option<serde_json::Map<String, serde_json::Value>>, String> {
    let cache_key = trajectory_metadata_cache_key(path);
    let stamp = trajectory_file_stamp(path).await;
    if let Some(stamp) = stamp.as_ref() {
        let cached = {
            let mut cache = trajectory_metadata_cache()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(entry) = cache.get_mut(&cache_key) {
                entry.last_access = next_trajectory_metadata_cache_access_stamp();
                (entry.chat_id == chat_id && &entry.stamp == stamp).then(|| entry.metadata.clone())
            } else {
                None
            }
        };
        if cached.is_some() {
            return Ok(cached);
        }
    }

    let metadata = read_existing_trajectory_object(path, chat_id).await?;
    match (metadata.as_ref(), stamp) {
        (Some(metadata), Some(stamp)) => {
            store_trajectory_metadata_cache_entry(
                path,
                TrajectoryMetadataCacheEntry {
                    chat_id: chat_id.to_string(),
                    metadata: preservable_trajectory_metadata(metadata),
                    stamp,
                    last_access: 0,
                },
            );
        }
        _ => {
            trajectory_metadata_cache()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&cache_key);
        }
    }
    Ok(metadata)
}

async fn cache_written_trajectory_metadata(
    path: &Path,
    chat_id: &str,
    trajectory: &serde_json::Value,
) -> Option<TrajectoryFileStamp> {
    let metadata = preservable_trajectory_metadata(trajectory.as_object()?);
    let stamp = trajectory_file_stamp(path).await?;
    store_trajectory_metadata_cache_entry(
        path,
        TrajectoryMetadataCacheEntry {
            chat_id: chat_id.to_string(),
            metadata,
            stamp: stamp.clone(),
            last_access: 0,
        },
    );
    Some(stamp)
}

impl Default for DetachedTrajectoryWriterState {
    fn default() -> Self {
        Self {
            pending: None,
            coalesce_deadline: None,
            requested_version: 0,
            committed_version: 0,
            in_flight: false,
            error: None,
            error_version: 0,
            notify: Arc::new(Notify::new()),
            wake: Arc::new(Notify::new()),
            worker: None,
            write_cache: Arc::new(AMutex::new(Default::default())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DetachedTrajectoryWriterKey {
    workspace_folders: Vec<PathBuf>,
    source: TrajectorySourceIdentity,
    chat_id: String,
}

type DetachedTrajectoryWriterRegistry = std::collections::HashMap<
    DetachedTrajectoryWriterKey,
    Weak<AMutex<DetachedTrajectoryWriterState>>,
>;

fn detached_trajectory_writers() -> &'static StdMutex<DetachedTrajectoryWriterRegistry> {
    static WRITERS: OnceLock<StdMutex<DetachedTrajectoryWriterRegistry>> = OnceLock::new();
    WRITERS.get_or_init(|| StdMutex::new(std::collections::HashMap::new()))
}

fn detached_trajectory_writer_source(
    task_meta: Option<&TaskMeta>,
    buddy_meta_present: bool,
) -> TrajectorySourceIdentity {
    if let Some(task_meta) = task_meta {
        TrajectorySourceIdentity::from_task_meta(task_meta)
    } else if buddy_meta_present {
        TrajectorySourceIdentity::Buddy
    } else {
        TrajectorySourceIdentity::Normal
    }
}

fn detached_trajectory_writer_key_for(
    gcx: &Arc<GlobalContext>,
    chat_id: &str,
    source: TrajectorySourceIdentity,
) -> DetachedTrajectoryWriterKey {
    let workspace_folders = gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    DetachedTrajectoryWriterKey {
        workspace_folders,
        source,
        chat_id: chat_id.to_string(),
    }
}

fn detached_trajectory_writer_key(
    gcx: &Arc<GlobalContext>,
    snapshot: &TrajectorySnapshot,
) -> DetachedTrajectoryWriterKey {
    detached_trajectory_writer_key_for(
        gcx,
        &snapshot.chat_id,
        detached_trajectory_writer_source(
            snapshot.task_meta.as_ref(),
            snapshot.buddy_meta.is_some(),
        ),
    )
}

fn detached_trajectory_writer(
    gcx: &Arc<GlobalContext>,
    snapshot: &TrajectorySnapshot,
) -> Arc<AMutex<DetachedTrajectoryWriterState>> {
    let mut writers = detached_trajectory_writers()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    writers.retain(|_, writer| writer.strong_count() > 0);
    let key = detached_trajectory_writer_key(gcx, snapshot);
    if let Some(writer) = writers.get(&key).and_then(Weak::upgrade) {
        return writer;
    }
    let writer = Arc::new(AMutex::new(Default::default()));
    writers.insert(key, Arc::downgrade(&writer));
    writer
}

fn prune_detached_trajectory_writers() {
    detached_trajectory_writers()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .retain(|_, writer| writer.strong_count() > 0);
}

async fn flush_detached_trajectory_writers() -> Result<(), Vec<String>> {
    let errors = {
        let writers = {
            let mut writers = detached_trajectory_writers()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            writers.retain(|_, writer| writer.strong_count() > 0);
            writers
                .values()
                .filter_map(Weak::upgrade)
                .collect::<Vec<_>>()
        };
        futures::future::join_all(writers.into_iter().map(|writer| async move {
            let target_version = release_detached_trajectory_coalescing(&writer).await;
            if target_version == 0 {
                Ok(())
            } else {
                wait_for_detached_trajectory_commit(writer, target_version).await
            }
        }))
        .await
        .into_iter()
        .filter_map(Result::err)
        .collect::<Vec<_>>()
    };
    prune_detached_trajectory_writers();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Drops the coalescing window of a writer and wakes it, returning its latest requested version.
async fn release_detached_trajectory_coalescing(
    writer: &Arc<AMutex<DetachedTrajectoryWriterState>>,
) -> u64 {
    let (requested_version, wake) = {
        let mut state = writer.lock().await;
        state.coalesce_deadline = None;
        (state.requested_version, state.wake.clone())
    };
    wake.notify_one();
    requested_version
}

/// Forces any write still sitting in the coalescing window for one trajectory out to disk.
/// Used by durability-critical session saves, which must not return while bytes are still pending.
async fn flush_detached_trajectory_writer_for(
    gcx: &Arc<GlobalContext>,
    chat_id: &str,
    source: TrajectorySourceIdentity,
) -> Result<(), String> {
    let key = detached_trajectory_writer_key_for(gcx, chat_id, source);
    let writer = {
        let mut writers = detached_trajectory_writers()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        writers.retain(|_, writer| writer.strong_count() > 0);
        writers.get(&key).and_then(Weak::upgrade)
    };
    let Some(writer) = writer else {
        return Ok(());
    };
    let target_version = release_detached_trajectory_coalescing(&writer).await;
    if target_version == 0 {
        return Ok(());
    }
    let result = wait_for_detached_trajectory_commit(writer, target_version).await;
    prune_detached_trajectory_writers();
    result
}

async fn flush_detached_trajectory_writer_for_session(
    gcx: &Arc<GlobalContext>,
    session_arc: &Arc<AMutex<ChatSession>>,
) -> Result<(), String> {
    let (chat_id, source) = {
        let session = session_arc.lock().await;
        (
            session.chat_id.clone(),
            detached_trajectory_writer_source(
                session.thread.task_meta.as_ref(),
                session.thread.buddy_meta.is_some(),
            ),
        )
    };
    flush_detached_trajectory_writer_for(gcx, &chat_id, source).await
}

pub(crate) async fn persist_trajectory_snapshot_with_intent(
    gcx: Arc<GlobalContext>,
    snapshot: TrajectorySnapshot,
    intent: TrajectoryCommitIntent,
) -> Result<(), String> {
    let await_commit = intent.requires_durability();
    persist_trajectory_snapshot_awaiting(gcx, snapshot, intent, await_commit).await
}

/// Schedules a save and waits for it to reach disk even when the intent only asks for a
/// checkpoint. The session writer needs this: it owns `trajectory_committed_version` and
/// `trajectory_save_error`, so it may only mark a version committed once the bytes landed.
/// Coalescing still applies — the caller simply waits out the window instead of skipping it,
/// which is what lets a burst of save requests collapse into the single queued snapshot.
async fn persist_trajectory_snapshot_awaiting_commit(
    gcx: Arc<GlobalContext>,
    snapshot: TrajectorySnapshot,
    intent: TrajectoryCommitIntent,
) -> Result<(), String> {
    persist_trajectory_snapshot_awaiting(gcx, snapshot, intent, true).await
}

async fn persist_trajectory_snapshot_awaiting(
    gcx: Arc<GlobalContext>,
    snapshot: TrajectorySnapshot,
    intent: TrajectoryCommitIntent,
    await_commit: bool,
) -> Result<(), String> {
    if crate::tasks::storage::bound_task_matches(
        snapshot
            .task_meta
            .as_ref()
            .map(|meta| meta.task_id.as_str()),
    )? {
        return save_trajectory_snapshot_inner(gcx, snapshot, None).await;
    }
    if trajectory_writer_rollout_enabled() {
        schedule_detached_trajectory_writer(gcx, snapshot, intent, await_commit).await
    } else {
        save_trajectory_snapshot_inner(gcx, snapshot, None).await
    }
}

async fn schedule_detached_trajectory_writer(
    gcx: Arc<GlobalContext>,
    snapshot: TrajectorySnapshot,
    intent: TrajectoryCommitIntent,
    await_commit: bool,
) -> Result<(), String> {
    let writer = detached_trajectory_writer(&gcx, &snapshot);
    let coalesce = !intent.requires_durability() && !gcx.shutdown_flag.load(Ordering::Relaxed);
    let (target_version, wake) = {
        let mut state = writer.lock().await;
        state.requested_version = state.requested_version.saturating_add(1);
        let target_version = state.requested_version;
        state.pending = Some((target_version, snapshot));
        state.coalesce_deadline =
            coalesce.then(|| Instant::now() + TRAJECTORY_CHECKPOINT_COALESCE_WINDOW);
        // Both worker exits clear `in_flight` while still holding this lock, so `in_flight`
        // without a live handle means the task was aborted or panicked. Respawning is what keeps
        // `in_flight` from wedging at true and stalling every later write for this trajectory.
        let worker_alive = state
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished());
        if !state.in_flight || !worker_alive {
            state.in_flight = true;
            let worker_writer = writer.clone();
            let worker_gcx = gcx.clone();
            state.worker = Some(tokio::spawn(async move {
                run_detached_trajectory_writer(worker_gcx, worker_writer).await;
            }));
        }
        (target_version, state.wake.clone())
    };
    wake.notify_one();
    if await_commit {
        let result = wait_for_detached_trajectory_commit(writer.clone(), target_version).await;
        prune_detached_trajectory_writers();
        result?;
    }
    Ok(())
}

async fn run_detached_trajectory_writer(
    gcx: Arc<GlobalContext>,
    writer: Arc<AMutex<DetachedTrajectoryWriterState>>,
) {
    loop {
        let (version, snapshot, write_cache) = loop {
            let (deadline, wake) = {
                let mut state = writer.lock().await;
                if state.pending.is_none() {
                    state.in_flight = false;
                    state.notify.notify_waiters();
                    return;
                }
                let deadline = state
                    .coalesce_deadline
                    .filter(|deadline| *deadline > Instant::now())
                    .filter(|_| !gcx.shutdown_flag.load(Ordering::Relaxed));
                match deadline {
                    Some(deadline) => (deadline, state.wake.clone()),
                    None => {
                        state.coalesce_deadline = None;
                        let (version, snapshot) = state
                            .pending
                            .take()
                            .expect("pending write was just checked");
                        break (version, snapshot, state.write_cache.clone());
                    }
                }
            };
            let _ =
                tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), wake.notified())
                    .await;
        };
        #[cfg(test)]
        let hook_chat_id = snapshot.chat_id.clone();
        let result = save_trajectory_snapshot_inner(gcx.clone(), snapshot, Some(write_cache)).await;
        #[cfg(test)]
        if result.is_err() {
            wait_for_test_detached_trajectory_writer_failure(&hook_chat_id).await;
        }
        let mut state = writer.lock().await;
        match result {
            Ok(()) => {
                state.committed_version = state.committed_version.max(version);
                state.error = None;
                state.error_version = 0;
                state.notify.notify_waiters();
            }
            Err(error) => {
                warn!(
                    "Detached trajectory writer failed at version {}: {}",
                    version, error
                );
                state.error = Some(error);
                state.error_version = version;
                state.notify.notify_waiters();
                if state
                    .pending
                    .as_ref()
                    .is_some_and(|(pending_version, _)| *pending_version > version)
                {
                    drop(state);
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    continue;
                }
                state.in_flight = false;
                return;
            }
        }
    }
}

async fn wait_for_detached_trajectory_commit(
    writer: Arc<AMutex<DetachedTrajectoryWriterState>>,
    target_version: u64,
) -> Result<(), String> {
    let mut last_progress = None;
    let mut deadline = Instant::now() + TRAJECTORY_COMMIT_STALL_TIMEOUT;
    loop {
        let notified = {
            let state = writer.lock().await;
            if state.committed_version >= target_version {
                return Ok(());
            }
            if state.error_version >= target_version {
                let error = state.error.clone().unwrap_or_else(|| {
                    format!(
                        "Detached trajectory writer failed at version {}",
                        state.error_version
                    )
                });
                return Err(error);
            }
            if !state.in_flight && state.pending.is_none() {
                return Err(state.error.clone().unwrap_or_else(|| {
                    format!(
                        "Detached trajectory writer stopped before committing version {} \
                         (committed {})",
                        target_version, state.committed_version
                    )
                }));
            }
            let progress = (
                state.committed_version,
                state.error_version,
                state.requested_version,
            );
            if last_progress != Some(progress) {
                last_progress = Some(progress);
                deadline = Instant::now() + TRAJECTORY_COMMIT_STALL_TIMEOUT;
            }
            let mut notified = Box::pin(state.notify.clone().notified_owned());
            let already_notified = notified.as_mut().enable();
            drop(state);
            if already_notified {
                continue;
            }
            notified
        };
        if tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), notified)
            .await
            .is_err()
        {
            let state = writer.lock().await;
            return Err(format!(
                "Detached trajectory writer made no progress for {} ms while committing version {} \
                 (committed {}, last error {:?})",
                TRAJECTORY_COMMIT_STALL_TIMEOUT.as_millis(),
                target_version,
                state.committed_version,
                state.error
            ));
        }
    }
}

#[cfg(test)]
static TEST_MESSAGE_SERIALIZATION_FAILURE_ID: std::sync::OnceLock<
    std::sync::Mutex<Option<String>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
fn test_message_serialization_should_fail(message: &ChatMessage) -> bool {
    TEST_MESSAGE_SERIALIZATION_FAILURE_ID
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .ok()
        .and_then(|message_id| message_id.clone())
        .as_deref()
        == Some(message.message_id.as_str())
}

#[cfg(test)]
fn set_test_message_serialization_failure(message_id: Option<String>) {
    *TEST_MESSAGE_SERIALIZATION_FAILURE_ID
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test serialization failure lock poisoned") = message_id;
}

#[cfg(test)]
static TEST_DETACHED_TRAJECTORY_WRITER_FAILURE_HOOK: std::sync::OnceLock<
    std::sync::Mutex<Option<(String, Arc<Notify>, Arc<Notify>)>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
fn set_test_detached_trajectory_writer_failure_hook(
    hook: Option<(String, Arc<Notify>, Arc<Notify>)>,
) {
    *TEST_DETACHED_TRAJECTORY_WRITER_FAILURE_HOOK
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("detached trajectory writer failure hook lock poisoned") = hook;
}

#[cfg(test)]
async fn wait_for_test_detached_trajectory_writer_failure(chat_id: &str) {
    let hook = TEST_DETACHED_TRAJECTORY_WRITER_FAILURE_HOOK
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("detached trajectory writer failure hook lock poisoned")
        .clone();
    if let Some((hook_chat_id, failed, release)) = hook {
        if hook_chat_id != chat_id {
            return;
        }
        failed.notify_one();
        release.notified().await;
    }
}

fn trajectory_value_from_snapshot(
    snapshot: &TrajectorySnapshot,
    messages_json: Vec<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let mut trajectory = json!({
        "id": snapshot.chat_id,
        "title": snapshot.title,
        "model": snapshot.model,
        "mode": snapshot.mode,
        "tool_use": snapshot.tool_use,
        "messages": messages_json,
        "created_at": snapshot.created_at,
        "boost_reasoning": snapshot.boost_reasoning,
        "checkpoints_enabled": snapshot.checkpoints_enabled,
        "context_tokens_cap": snapshot.context_tokens_cap,
        "auto_compression_cap": snapshot.auto_compression_cap,
        "auto_compression_cap_pending": snapshot.auto_compression_cap_pending,
        "include_project_info": snapshot.include_project_info,
        "isTitleGenerated": snapshot.is_title_generated,
        "auto_approve_editing_tools": snapshot.auto_approve_editing_tools,
        "auto_approve_dangerous_commands": snapshot.auto_approve_dangerous_commands,
        "autonomous_no_confirm": snapshot.autonomous_no_confirm,
    });

    if let Some(ref effort) = snapshot.reasoning_effort {
        trajectory["reasoning_effort"] = serde_json::Value::String(effort.clone());
    }
    if let Some(budget) = snapshot.thinking_budget {
        trajectory["thinking_budget"] = json!(budget);
    }
    if let Some(temp) = snapshot.temperature {
        trajectory["temperature"] = json!(temp);
    }
    if let Some(freq) = snapshot.frequency_penalty {
        trajectory["frequency_penalty"] = json!(freq);
    }
    if let Some(max_t) = snapshot.max_tokens {
        trajectory["max_tokens"] = json!(max_t);
    }
    if let Some(ref prev) = snapshot.previous_response_id {
        trajectory["previous_response_id"] = serde_json::Value::String(prev.clone());
    }
    if let Some(parallel) = snapshot.parallel_tool_calls {
        trajectory["parallel_tool_calls"] = json!(parallel);
    }
    if let Some(ref skill) = snapshot.active_skill {
        trajectory["active_skill"] = serde_json::Value::String(skill.clone());
    }
    if let Some(auto_enrich) = snapshot.auto_enrichment_enabled {
        trajectory["auto_enrichment_enabled"] = json!(auto_enrich);
    }
    if let Some(ref buddy_meta) = snapshot.buddy_meta {
        trajectory["buddy_meta"] = serde_json::to_value(buddy_meta).unwrap_or_default();
    }
    if let Some(auto_compact) = snapshot.auto_compact_enabled {
        trajectory["auto_compact_enabled"] = json!(auto_compact);
    }
    if let Some(ref frozen_request_prefix) = snapshot.frozen_request_prefix {
        trajectory["frozen_request_prefix"] =
            serde_json::to_value(frozen_request_prefix).unwrap_or_default();
    }
    if let Some(ref claude_code_identity) = snapshot.claude_code_identity {
        trajectory["claude_code_identity"] =
            serde_json::to_value(claude_code_identity).unwrap_or_default();
    }
    if let Some(ref recovery) = snapshot.recovery {
        trajectory["recovery"] = json!(recovery);
    }
    if let Some(ref recovery_policy) = snapshot.recovery_policy {
        trajectory["recovery_policy"] = json!(recovery_policy);
    }
    if let Some(wake_up_at) = snapshot.wake_up_at {
        trajectory["wake_up_at"] = json!(wake_up_at);
    }
    if !snapshot.waiting_for_card_ids.is_empty() {
        trajectory["waiting_for_card_ids"] = json!(snapshot.waiting_for_card_ids);
    }
    if !snapshot.native_decision_receipts.is_empty() {
        trajectory["native_decision_receipts"] = json!(snapshot.native_decision_receipts);
    }
    if !snapshot.pending_questions.is_empty() {
        trajectory["pending_questions"] = serde_json::to_value(&snapshot.pending_questions)
            .map_err(|error| format!("failed to serialize pending questions: {error}"))?;
    }
    if let Some(ref pending_browser_decision) = snapshot.pending_browser_decision {
        trajectory["pending_browser_decision"] = serde_json::to_value(pending_browser_decision)
            .map_err(|error| format!("failed to serialize pending browser decision: {error}"))?;
    }
    if !snapshot.pending_deliveries.is_empty() {
        trajectory["pending_deliveries"] = json!(snapshot.pending_deliveries);
    }
    if !snapshot.dropped_deliveries.is_empty() {
        trajectory["dropped_deliveries"] = json!(snapshot.dropped_deliveries);
    }
    if let Some(ref goal) = snapshot.goal {
        trajectory["goal"] = serde_json::to_value(goal).unwrap_or_default();
    }
    if !snapshot.goal_ledger.is_empty() {
        trajectory["goal_ledger"] = serde_json::to_value(&snapshot.goal_ledger).unwrap_or_default();
    }
    if let Some(blocked_until_ms) = snapshot.goal_verification_blocked_until_ms {
        trajectory["goal_verification_blocked_until_ms"] = json!(blocked_until_ms);
    }
    if let Some(ref worktree) = snapshot.worktree {
        trajectory["worktree"] = serde_json::to_value(worktree).unwrap_or_default();
    }
    if let Some(ref app_workspace) = snapshot.app_workspace {
        trajectory["app_workspace"] = serde_json::to_value(app_workspace)
            .map_err(|error| format!("failed to serialize app workspace: {error}"))?;
    }

    if let Some(ref parent_id) = snapshot.parent_id {
        trajectory["parent_id"] = serde_json::Value::String(parent_id.clone());
    }
    if let Some(ref link_type) = snapshot.link_type {
        trajectory["link_type"] = serde_json::Value::String(link_type.clone());
    }

    let effective_root = snapshot
        .root_chat_id
        .clone()
        .unwrap_or_else(|| snapshot.chat_id.clone());
    trajectory["root_chat_id"] = serde_json::Value::String(effective_root);

    if let Some(ref task_meta) = snapshot.task_meta {
        trajectory["task_meta"] = serde_json::to_value(task_meta).unwrap_or_default();
    }

    Ok(trajectory)
}

async fn serialize_trajectory_snapshot_messages(
    messages: Vec<ChatMessage>,
    retain_background_messages: bool,
    collect_metrics: bool,
    chat_id: &str,
    message_count: usize,
    version: u64,
) -> Result<
    (
        Vec<serde_json::Value>,
        Option<Vec<serde_json::Value>>,
        Option<TrajectoryMetrics>,
        perf_diagnostics::PerfSpan,
    ),
    String,
> {
    let mut serialize_span =
        perf_diagnostics::span(PerfComponent::TrajectorySerialize, Some(chat_id), None);
    let message_serialization = tokio::task::spawn_blocking(move || {
        let messages_json: Result<Vec<serde_json::Value>, String> = messages
            .iter()
            .enumerate()
            .map(|(index, message)| {
                #[cfg(test)]
                if test_message_serialization_should_fail(message) {
                    return Err(format!(
                        "Failed to serialize trajectory message at index {index} (id {}, role {}): test failure",
                        message.message_id, message.role
                    ));
                }
                serde_json::to_value(message).map_err(|error| {
                    format!(
                        "Failed to serialize trajectory message at index {index} (id {}, role {}): {error}",
                        message.message_id, message.role
                    )
                })
            })
            .collect();
        messages_json.map(|messages_json| (messages, messages_json))
    })
    .await;
    let (messages, messages_json) = match message_serialization {
        Ok(Ok(result)) => {
            serialize_span.pause();
            result
        }
        Ok(Err(error)) => {
            serialize_span.finish(
                PerfOutcome::Failure,
                None,
                Some(message_count as u64),
                Some(version),
                None,
            );
            return Err(error);
        }
        Err(error) => {
            serialize_span.finish(
                PerfOutcome::Failure,
                None,
                Some(message_count as u64),
                Some(version),
                None,
            );
            return Err(format!(
                "Trajectory message serialization task failed: {}",
                error
            ));
        }
    };
    let metric_scan_span = perf_diagnostics::is_enabled()
        .then(|| perf_diagnostics::span(PerfComponent::TrajectoryMetricScan, Some(chat_id), None));
    let metric_scan = tokio::task::spawn_blocking(move || {
        let background_messages_json = retain_background_messages.then(|| messages_json.clone());
        let metrics = collect_metrics.then(|| scan_trajectory_metrics(&messages));
        (messages_json, background_messages_json, metrics)
    })
    .await;
    if let Some(span) = metric_scan_span {
        span.finish(
            if metric_scan.is_ok() {
                PerfOutcome::Success
            } else {
                PerfOutcome::Failure
            },
            None,
            Some(message_count as u64),
            Some(version),
            None,
        );
    }
    let (messages_json, background_messages_json, metrics) = match metric_scan {
        Ok(result) => result,
        Err(error) => {
            serialize_span.finish(
                PerfOutcome::Failure,
                None,
                Some(message_count as u64),
                Some(version),
                None,
            );
            return Err(format!("Trajectory metric scan task failed: {}", error));
        }
    };
    Ok((
        messages_json,
        background_messages_json,
        metrics,
        serialize_span,
    ))
}

async fn persist_trajectory_snapshot_file(
    gcx: Arc<GlobalContext>,
    snapshot: &TrajectorySnapshot,
    write_cache: Option<&Arc<AMutex<DetachedTrajectoryWriteCache>>>,
    existing_no_meta_path: Option<PathBuf>,
    source: TrajectorySourceIdentity,
    mut trajectory: serde_json::Value,
    message_count: usize,
    mut serialize_span: perf_diagnostics::PerfSpan,
) -> Result<(PathBuf, String, serde_json::Value, [u8; 32]), String> {
    let bound_task = crate::tasks::storage::bound_task_matches(
        snapshot
            .task_meta
            .as_ref()
            .map(|meta| meta.task_id.as_str()),
    )?;
    let cached_entry = if bound_task {
        None
    } else if let Some(write_cache) = write_cache {
        let write_cache = write_cache.lock().await;
        if write_cache.source.as_ref() == Some(&source) {
            match (
                write_cache.backing_path.clone(),
                write_cache.existing_metadata.clone(),
                write_cache.backing_stamp.clone(),
            ) {
                (Some(path), Some(metadata), Some(stamp)) => Some((path, metadata, stamp)),
                _ => None,
            }
        } else {
            None
        }
    } else {
        None
    };
    let cached_path_and_metadata = match cached_entry {
        Some((path, metadata, stamp))
            if metadata.get("id").and_then(|value| value.as_str())
                == Some(snapshot.chat_id.as_str())
                && trajectory_stamp_matches(&path, &stamp).await =>
        {
            Some((path, metadata))
        }
        _ => None,
    };
    let file_path_result = if let Some((path, _)) = cached_path_and_metadata.as_ref() {
        Ok(path.clone())
    } else if let Some(ref task_meta) = snapshot.task_meta {
        safe_new_task_trajectory_file(gcx.clone(), task_meta, &snapshot.chat_id).await
    } else if snapshot.buddy_meta.is_some() {
        safe_new_buddy_trajectory_file(gcx.clone(), &snapshot.chat_id).await
    } else if let Some(path) = existing_no_meta_path {
        Ok(path)
    } else {
        safe_new_nested_trajectory_file(
            gcx.clone(),
            &snapshot.chat_id,
            snapshot.root_chat_id.as_deref(),
        )
        .await
    };
    let file_path = match file_path_result {
        Ok(path) => path,
        Err(error) => {
            serialize_span.finish(
                PerfOutcome::Failure,
                None,
                Some(message_count as u64),
                Some(snapshot.version),
                None,
            );
            return Err(error);
        }
    };
    let existing_trajectory = match cached_path_and_metadata.map(|(_, metadata)| metadata) {
        Some(existing_trajectory) => Some(existing_trajectory),
        None => match cached_existing_trajectory_object(&file_path, &snapshot.chat_id).await {
            Ok(existing_trajectory) => existing_trajectory,
            Err(error) => {
                serialize_span.finish(
                    PerfOutcome::Failure,
                    None,
                    Some(message_count as u64),
                    Some(snapshot.version),
                    None,
                );
                return Err(error);
            }
        },
    };

    let updated_at = chrono::Utc::now().to_rfc3339();
    trajectory["updated_at"] = serde_json::Value::String(updated_at.clone());
    if let Some(existing_created_at) = existing_trajectory
        .as_ref()
        .and_then(|existing| existing.get("created_at"))
        .and_then(|created_at| created_at.as_str())
        .filter(|created_at| parsed_rfc3339_utc(created_at).is_some())
    {
        trajectory["created_at"] = serde_json::Value::String(existing_created_at.to_string());
    }
    super::app_workspace::keep_newest_app_workspace_in_trajectory(
        &mut trajectory,
        existing_trajectory
            .as_ref()
            .and_then(|existing| existing.get(super::app_workspace::APP_WORKSPACE_KEY)),
    );
    preserve_existing_trajectory_metadata(&mut trajectory, existing_trajectory);

    let tmp_path = unique_trajectory_tmp_path(&file_path);
    serialize_span.resume();
    let serialize_tmp_path = tmp_path.clone();
    let serialized = tokio::task::spawn_blocking(move || {
        let write_result = write_pretty_json_to_tmp_file(&serialize_tmp_path, &trajectory);
        (trajectory, write_result)
    })
    .await;
    let (trajectory, write_result) = match serialized {
        Ok((trajectory, write_result)) => {
            serialize_span.finish(
                if write_result.is_ok() {
                    PerfOutcome::Success
                } else {
                    PerfOutcome::Failure
                },
                write_result.as_ref().ok().map(|write| write.byte_len),
                Some(message_count as u64),
                Some(snapshot.version),
                None,
            );
            (trajectory, write_result)
        }
        Err(error) => {
            serialize_span.finish(
                PerfOutcome::Failure,
                None,
                Some(message_count as u64),
                Some(snapshot.version),
                None,
            );
            let _ = fs::remove_file(&tmp_path).await;
            return Err(format!("Trajectory serialization task failed: {}", error));
        }
    };
    let atomic_write_span = perf_diagnostics::span(
        PerfComponent::TrajectoryAtomicWrite,
        Some(&snapshot.chat_id),
        Some(&file_path),
    );
    let source_sha256 = write_result
        .as_ref()
        .map_err(Clone::clone)?
        .source_sha256
        .clone();
    let atomic_write_result = if bound_task {
        let task_meta = snapshot
            .task_meta
            .as_ref()
            .ok_or("Accepted task snapshot missing identity")?;
        let (_, guard) =
            crate::tasks::storage::locked_task_dir(gcx.clone(), &task_meta.task_id).await?;
        super::trajectory_archive::with_trajectory_mutation_lock(async {
            crate::tasks::storage::validate_task_trajectory_publication(
                gcx.clone(),
                &guard,
                task_meta,
                &snapshot.chat_id,
                &file_path,
            )
            .await?;
            atomic_finish_trajectory_write(
                &file_path,
                &tmp_path,
                write_result,
                Some(source.clone()),
            )
            .await
        })
        .await
    } else {
        atomic_finish_trajectory_write(&file_path, &tmp_path, write_result, Some(source.clone()))
            .await
    };
    atomic_write_span.finish(
        if atomic_write_result.is_ok() {
            PerfOutcome::Success
        } else {
            PerfOutcome::Failure
        },
        None,
        Some(message_count as u64),
        Some(snapshot.version),
        None,
    );
    atomic_write_result?;
    #[cfg(test)]
    record_trajectory_physical_write(&snapshot.chat_id);

    let backing_stamp =
        cache_written_trajectory_metadata(&file_path, &snapshot.chat_id, &trajectory).await;
    if let Some(write_cache) = write_cache {
        let metadata = trajectory.as_object().cloned();
        let mut write_cache = write_cache.lock().await;
        if let (Some(metadata), Some(stamp)) = (metadata, backing_stamp) {
            write_cache.backing_path = Some(file_path.clone());
            write_cache.existing_metadata = Some(metadata);
            write_cache.source = Some(source);
            write_cache.backing_stamp = Some(stamp);
        } else {
            *write_cache = DetachedTrajectoryWriteCache::default();
        }
    }

    info!(
        "Saved trajectory for chat {} ({} messages) to {:?}",
        snapshot.chat_id, message_count, file_path
    );
    Ok((file_path, updated_at, trajectory, source_sha256))
}

async fn publish_trajectory_snapshot_updates(
    gcx: Arc<GlobalContext>,
    app: &AppState,
    snapshot: &TrajectorySnapshot,
    source: &TrajectorySourceIdentity,
    file_path: &Path,
    trajectory: serde_json::Value,
    source_sha256: [u8; 32],
    message_count: usize,
    metrics: Option<TrajectoryMetrics>,
    background_messages_json: Option<Vec<serde_json::Value>>,
    updated_at: String,
    should_generate_title: bool,
    allow_background_generation: bool,
) -> Result<(), String> {
    if !is_internal_trace_link_type(snapshot.link_type.as_deref()) {
        if let Some(dir) = index_dir_for_trajectory_file(gcx.clone(), file_path).await {
            let source_hint = if let Some(task_meta) = snapshot.task_meta.as_ref() {
                Some(TrajectorySourceIdentity::from_task_meta(task_meta))
            } else if snapshot.buddy_meta.is_some() {
                Some(TrajectorySourceIdentity::Buddy)
            } else {
                Some(TrajectorySourceIdentity::Normal)
            };
            trajectory_index::upsert_trajectory_index_entry_from_committed_value(
                &app.chat.trajectory_index_coordinator,
                &dir,
                file_path,
                trajectory,
                source_hint,
                Some(source_sha256),
            )
            .await?;
        }
    } else if let Some(parent) = file_path.parent() {
        match prune_internal_traces_throttled(parent).await {
            Ok(0) => {}
            Ok(removed) => info!("pruned {removed} old internal traces from {:?}", parent),
            Err(error) => warn!("internal trace pruning failed: {error}"),
        }
    }

    if snapshot.buddy_meta.is_some() {
        prune_buddy_conversations_throttled(gcx.clone()).await;
    }

    if should_vectorize_trajectory(source, snapshot.link_type.as_deref()) {
        enqueue_trajectory_vectorization(gcx.clone(), file_path.to_path_buf()).await;
    }

    if snapshot.task_meta.is_none() && snapshot.buddy_meta.is_none() {
        let effective_root = snapshot
            .root_chat_id
            .clone()
            .unwrap_or_else(|| snapshot.chat_id.clone());
        let sessions = app.chat.sessions.clone();
        let source = TrajectorySourceIdentity::Normal;
        let live_session = sessions.read().await.get(&snapshot.chat_id).cloned();
        let runtime = live_session.as_ref().and_then(|session| {
            let session = session.try_lock().ok()?;
            Some(if source.matches_session(&session) {
                (
                    session.runtime.state.to_string(),
                    session.runtime.error.clone(),
                    session.thread.worktree.clone(),
                )
            } else {
                (SessionState::Idle.to_string(), None, None)
            })
        });
        let (session_state, session_error, session_worktree) = match runtime {
            Some((state, error, worktree)) => (Some(state), error, worktree),
            None if live_session.is_none() => (Some(SessionState::Idle.to_string()), None, None),
            None => (None, None, None),
        };
        let metrics = metrics.unwrap_or_default();
        let (total_lines_added, total_lines_removed) = metrics.line_changes;
        let (tasks_total, tasks_done, tasks_failed) = metrics.task_progress;
        let token_totals = metrics.token_totals;
        let tx = &app.chat.trajectory_events_tx;
        {
            let event = TrajectoryEvent {
                event_type: "updated".to_string(),
                id: snapshot.chat_id.clone(),
                updated_at: Some(updated_at),
                title: Some(trajectory_meta_title(&snapshot.title)),
                is_title_generated: Some(snapshot.is_title_generated),
                session_state,
                error: session_error,
                message_count: Some(message_count),
                parent_id: snapshot.parent_id.clone(),
                link_type: snapshot.link_type.clone(),
                root_chat_id: Some(effective_root),
                task_id: None,
                task_role: None,
                agent_id: None,
                card_id: None,
                model: Some(snapshot.model.clone()),
                mode: Some(snapshot.mode.clone()),
                worktree: snapshot.worktree.clone().or(session_worktree),
                total_lines_added: Some(total_lines_added),
                total_lines_removed: Some(total_lines_removed),
                tasks_total: Some(tasks_total),
                tasks_done: Some(tasks_done),
                tasks_failed: Some(tasks_failed),
                total_prompt_tokens: Some(token_totals.prompt_tokens),
                total_completion_tokens: Some(token_totals.completion_tokens),
                total_tokens: Some(token_totals.total_tokens),
                total_cache_read_tokens: Some(token_totals.cache_read_tokens),
                total_cache_creation_tokens: Some(token_totals.cache_creation_tokens),
                total_cost_usd: token_totals.cost_usd,
            };
            let _ = tx.send(event);
        }

        if should_generate_title {
            let _ = spawn_title_generation_task(
                gcx.clone(),
                snapshot.chat_id.clone(),
                background_messages_json.unwrap_or_default(),
                file_path.to_path_buf(),
                TrajectorySourceIdentity::Normal,
            );
        }
    } else if snapshot.task_meta.is_none() && snapshot.buddy_meta.is_some() {
        if should_generate_title {
            let _ = spawn_title_generation_task(
                gcx.clone(),
                snapshot.chat_id.clone(),
                background_messages_json.unwrap_or_default(),
                file_path.to_path_buf(),
                TrajectorySourceIdentity::Buddy,
            );
        }
    } else if let Some(ref task_meta) = snapshot.task_meta {
        let mut background_messages_json = background_messages_json.unwrap_or_default();

        if should_generate_title {
            let _ = spawn_title_generation_task(
                gcx.clone(),
                snapshot.chat_id.clone(),
                background_messages_json.clone(),
                file_path.to_path_buf(),
                TrajectorySourceIdentity::from_task_meta(task_meta),
            );
        }

        if allow_background_generation && task_meta.role == "planner" {
            let user_message_count = count_user_messages(&background_messages_json);
            if user_message_count >= 1 {
                spawn_task_name_generation_task(
                    gcx,
                    task_meta.task_id.clone(),
                    std::mem::take(&mut background_messages_json),
                );
            }
        }
    }

    Ok(())
}

async fn save_trajectory_snapshot_inner(
    gcx: Arc<GlobalContext>,
    snapshot: TrajectorySnapshot,
    write_cache: Option<Arc<AMutex<DetachedTrajectoryWriteCache>>>,
) -> Result<(), String> {
    save_trajectory_snapshot_inner_options(gcx, snapshot, write_cache, true).await
}

pub(crate) async fn save_tool_refresh_snapshot(
    gcx: Arc<GlobalContext>,
    snapshot: TrajectorySnapshot,
) -> Result<(), String> {
    save_trajectory_snapshot_inner_options(gcx, snapshot, None, false).await
}

async fn save_trajectory_snapshot_inner_options(
    gcx: Arc<GlobalContext>,
    mut snapshot: TrajectorySnapshot,
    write_cache: Option<Arc<AMutex<DetachedTrajectoryWriteCache>>>,
    allow_background_generation: bool,
) -> Result<(), String> {
    validate_trajectory_id(&snapshot.chat_id).map_err(|e| e.message)?;
    let chat_id = snapshot.chat_id.clone();
    let app = AppState::from_gcx(gcx.clone()).await;
    let existing_no_meta_path = if snapshot.task_meta.is_none() && snapshot.buddy_meta.is_none() {
        find_normal_trajectory_path(gcx.clone(), &snapshot.chat_id).await
    } else {
        None
    };
    if snapshot.messages.is_empty()
        && snapshot.pending_deliveries.is_empty()
        && snapshot.task_meta.is_none()
        && snapshot.buddy_meta.is_none()
        && snapshot.frozen_request_prefix.is_none()
        && snapshot.app_workspace.is_none()
        && snapshot.goal.is_none()
        && snapshot.auto_compression_cap.is_none()
        && !snapshot.auto_compression_cap_pending
        && snapshot.title.is_empty()
        && existing_no_meta_path.is_none()
    {
        return Ok(());
    }

    let message_count = snapshot.messages.len();
    let collect_metrics = snapshot.task_meta.is_none() && snapshot.buddy_meta.is_none();
    let should_generate_title = allow_background_generation
        && is_placeholder_title(&snapshot.title)
        && !snapshot.is_title_generated
        && message_count > 0;
    let should_generate_task_name = snapshot
        .task_meta
        .as_ref()
        .is_some_and(|task_meta| task_meta.role == "planner");
    let retain_background_messages = should_generate_title || should_generate_task_name;
    let messages = std::mem::take(&mut snapshot.messages);
    let (messages_json, background_messages_json, metrics, serialize_span) =
        serialize_trajectory_snapshot_messages(
            messages,
            retain_background_messages,
            collect_metrics,
            &chat_id,
            message_count,
            snapshot.version,
        )
        .await?;
    let trajectory = trajectory_value_from_snapshot(&snapshot, messages_json)?;
    let source = if let Some(task_meta) = snapshot.task_meta.as_ref() {
        TrajectorySourceIdentity::from_task_meta(task_meta)
    } else if snapshot.buddy_meta.is_some() {
        TrajectorySourceIdentity::Buddy
    } else {
        TrajectorySourceIdentity::Normal
    };
    let (file_path, updated_at, trajectory, source_sha256) = persist_trajectory_snapshot_file(
        gcx.clone(),
        &snapshot,
        write_cache.as_ref(),
        existing_no_meta_path,
        source.clone(),
        trajectory,
        message_count,
        serialize_span,
    )
    .await?;
    publish_trajectory_snapshot_updates(
        gcx,
        &app,
        &snapshot,
        &source,
        &file_path,
        trajectory,
        source_sha256,
        message_count,
        metrics,
        background_messages_json,
        updated_at,
        should_generate_title,
        allow_background_generation,
    )
    .await
}

pub async fn try_save_trajectory(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
) -> Result<bool, String> {
    try_save_trajectory_with_intent(app, session_arc, TrajectoryCommitIntent::Required).await
}

pub async fn try_save_trajectory_with_intent(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    intent: TrajectoryCommitIntent,
) -> Result<bool, String> {
    if !intent.persists() {
        return Ok(true);
    }
    let planner = {
        let session = session_arc.lock().await;
        session
            .thread
            .task_meta
            .as_ref()
            .filter(|m| m.role == "planner")
            .map(|m| (m.task_id.clone(), session.chat_id.clone()))
    };
    let owns_task = match planner {
        Some((task_id, chat_id)) => crate::tasks::storage::load_board(app.gcx.clone(), &task_id)
            .await?
            .require_owner(&chat_id)
            .is_ok(),
        None => false,
    };
    if owns_task {
        crate::tasks::goal::commit_session_task_goal(app.gcx.clone(), &session_arc).await?;
    }
    let task_id = session_arc
        .lock()
        .await
        .thread
        .task_meta
        .as_ref()
        .map(|meta| meta.task_id.clone());
    if crate::tasks::storage::bound_task_matches(task_id.as_deref())?
        || !trajectory_writer_rollout_enabled()
    {
        return try_save_trajectory_legacy_with_intent(app, session_arc, intent).await;
    }
    let dirty = session_arc.lock().await.trajectory_dirty;
    if !dirty {
        // A clean session can still have a coalesced checkpoint sitting in the detached writer,
        // so a durability-critical save has to drain it instead of returning straight away.
        if intent.requires_durability() {
            flush_detached_trajectory_writer_for_session(&app.gcx, &session_arc).await?;
        }
        return Ok(true);
    }
    let target_version =
        schedule_trajectory_writer(app.clone(), session_arc.clone(), intent).await?;
    if intent.requires_durability() {
        wait_for_trajectory_commit(session_arc.clone(), target_version).await?;
        flush_detached_trajectory_writer_for_session(&app.gcx, &session_arc).await?;
    }
    let session = session_arc.lock().await;
    Ok(!session.trajectory_dirty)
}

async fn try_save_trajectory_legacy_with_intent(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    intent: TrajectoryCommitIntent,
) -> Result<bool, String> {
    let snapshot = {
        let session = session_arc.lock().await;
        if !session.trajectory_dirty {
            return Ok(true);
        }
        trajectory_snapshot_from_session(&session)
    };
    commit_trajectory_snapshot_for_session(app.gcx, session_arc.clone(), snapshot, intent).await?;
    Ok(!session_arc.lock().await.trajectory_dirty)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionTrajectoryCommitOutcome {
    Committed,
    SkippedStale,
    SkippedEphemeral,
}

async fn commit_trajectory_snapshot_for_session(
    gcx: Arc<GlobalContext>,
    session_arc: Arc<AMutex<ChatSession>>,
    snapshot: TrajectorySnapshot,
    intent: TrajectoryCommitIntent,
) -> Result<SessionTrajectoryCommitOutcome, String> {
    if !intent.persists() {
        return Ok(SessionTrajectoryCommitOutcome::SkippedEphemeral);
    }
    let saved_version = snapshot.version;
    let chat_id = snapshot.chat_id.clone();
    let message_count = snapshot.messages.len() as u64;
    let span = perf_diagnostics::span(PerfComponent::TrajectoryCommit, Some(&chat_id), None);
    let save_mutex = {
        let session = session_arc.lock().await;
        session.trajectory_save_mutex.clone()
    };
    let save_mutex_wait_span = perf_diagnostics::is_enabled().then(|| {
        perf_diagnostics::span(PerfComponent::TrajectorySaveMutexWait, Some(&chat_id), None)
    });
    let _save_guard = save_mutex.lock().await;
    if let Some(span) = save_mutex_wait_span {
        span.finish(
            PerfOutcome::Success,
            None,
            Some(message_count),
            Some(saved_version),
            None,
        );
    }
    {
        let session = session_arc.lock().await;
        if !session.trajectory_commit_can_write(saved_version) {
            return Ok(SessionTrajectoryCommitOutcome::SkippedStale);
        }
    }

    let result = persist_trajectory_snapshot_awaiting_commit(gcx, snapshot, intent).await;
    span.finish(
        if result.is_ok() {
            PerfOutcome::Success
        } else {
            PerfOutcome::Failure
        },
        None,
        Some(message_count),
        Some(saved_version),
        None,
    );
    result.map_err(|error| format!("Failed to save trajectory for {}: {}", chat_id, error))?;

    let mut session = session_arc.lock().await;
    let committed = session.complete_trajectory_commit(saved_version);
    session.trajectory_commit_notify.notify_waiters();
    if committed {
        Ok(SessionTrajectoryCommitOutcome::Committed)
    } else {
        Ok(SessionTrajectoryCommitOutcome::SkippedStale)
    }
}

async fn schedule_trajectory_writer(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    intent: TrajectoryCommitIntent,
) -> Result<u64, String> {
    if !trajectory_writer_rollout_enabled() {
        try_save_trajectory_legacy_with_intent(app, session_arc.clone(), intent).await?;
        return Ok(session_arc.lock().await.trajectory_committed_version);
    }
    let should_spawn = {
        let mut session = session_arc.lock().await;
        let target_version = session.trajectory_version;
        if !session.trajectory_dirty {
            return Ok(target_version);
        }
        if session.trajectory_save_in_flight {
            session.trajectory_save_queued = true;
            return Ok(target_version);
        }
        session.trajectory_save_in_flight = true;
        session.trajectory_save_queued = false;
        session.trajectory_save_error = None;
        target_version
    };
    let gcx = app.gcx.clone();
    let writer_session_arc = session_arc.clone();
    tokio::spawn(async move {
        loop {
            let snapshot = {
                let session = writer_session_arc.lock().await;
                if !session.trajectory_dirty {
                    None
                } else {
                    Some(trajectory_snapshot_from_session(&session))
                }
            };
            let Some(snapshot) = snapshot else {
                let mut session = writer_session_arc.lock().await;
                session.trajectory_save_in_flight = false;
                session.trajectory_save_queued = false;
                session.trajectory_commit_notify.notify_waiters();
                return;
            };

            let result = commit_trajectory_snapshot_for_session(
                gcx.clone(),
                writer_session_arc.clone(),
                snapshot,
                intent,
            )
            .await;
            let mut session = writer_session_arc.lock().await;
            match result {
                Ok(_) => {
                    session.trajectory_save_error = None;
                }
                Err(error) => {
                    warn!("{}", error);
                    session.trajectory_dirty = true;
                    session.trajectory_save_in_flight = false;
                    session.trajectory_save_queued = false;
                    session.trajectory_save_error = Some(error);
                    session.trajectory_commit_notify.notify_waiters();
                    return;
                }
            }
            if session.trajectory_dirty || session.trajectory_save_queued {
                session.trajectory_save_queued = false;
                continue;
            }
            session.trajectory_save_in_flight = false;
            session.trajectory_commit_notify.notify_waiters();
            return;
        }
    });
    if intent.requires_durability() {
        wait_for_trajectory_commit(session_arc, should_spawn).await?;
    }
    Ok(should_spawn)
}

async fn wait_for_trajectory_commit(
    session_arc: Arc<AMutex<ChatSession>>,
    target_version: u64,
) -> Result<(), String> {
    let mut last_progress = None;
    let mut deadline = Instant::now() + TRAJECTORY_COMMIT_STALL_TIMEOUT;
    loop {
        let notified = {
            let session = session_arc.lock().await;
            if session.trajectory_committed_version >= target_version {
                return Ok(());
            }
            if let Some(error) = session.trajectory_save_error.clone() {
                return Err(error);
            }
            let progress = (
                session.trajectory_committed_version,
                session.trajectory_version,
            );
            if last_progress != Some(progress) {
                last_progress = Some(progress);
                deadline = Instant::now() + TRAJECTORY_COMMIT_STALL_TIMEOUT;
            }
            let mut notified = Box::pin(session.trajectory_commit_notify.clone().notified_owned());
            let already_notified = notified.as_mut().enable();
            drop(session);
            if already_notified {
                continue;
            }
            notified
        };
        if tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), notified)
            .await
            .is_err()
        {
            let session = session_arc.lock().await;
            return Err(format!(
                "Trajectory writer made no progress for {} ms while committing version {} \
                 (committed {})",
                TRAJECTORY_COMMIT_STALL_TIMEOUT.as_millis(),
                target_version,
                session.trajectory_committed_version
            ));
        }
    }
}

pub async fn flush_trajectory_for_session(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
) -> Result<(), String> {
    try_save_trajectory_with_intent(app, session_arc, TrajectoryCommitIntent::Required)
        .await
        .map(|_| ())
}

pub async fn flush_all_trajectories(app: AppState) -> Result<(), Vec<String>> {
    flush_all_trajectories_with_timeout(app, TRAJECTORY_FLUSH_TIMEOUT).await
}

/// Flushes every session this app owns and then joins the detached writers,
/// completing when each writer acknowledges its commit. Unbounded on purpose:
/// the wall-clock budget belongs to the shutdown caller, not to the flush.
///
/// The detached-writer registry is process-global rather than app-scoped, so
/// this join also waits for writers owned by any other `AppState` alive in the
/// process. That is exact in production, where there is one app; in a test
/// binary it makes one app's flush wait on another's.
async fn flush_all_trajectories_errors(app: AppState) -> Vec<String> {
    let sessions: Vec<Arc<AMutex<ChatSession>>> = {
        let sessions = app.chat.sessions.read().await;
        sessions.values().cloned().collect()
    };
    let results = futures::future::join_all(
        sessions
            .into_iter()
            .map(|session_arc| flush_trajectory_for_session(app.clone(), session_arc)),
    )
    .await;
    let mut errors: Vec<String> = results.into_iter().filter_map(Result::err).collect();
    if let Err(mut detached_errors) = flush_detached_trajectory_writers().await {
        errors.append(&mut detached_errors);
    }
    errors
}

async fn flush_all_trajectories_with_timeout(
    app: AppState,
    timeout: Duration,
) -> Result<(), Vec<String>> {
    let flushes = flush_all_trajectories_errors(app);
    let errors = match tokio::time::timeout(timeout, flushes).await {
        Ok(errors) => errors,
        Err(_) => vec![format!(
            "trajectory flush timed out after {} ms",
            timeout.as_millis()
        )],
    };
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn maybe_save_trajectory_background(app: AppState, session_arc: Arc<AMutex<ChatSession>>) {
    maybe_save_trajectory_background_with_intent(
        app,
        session_arc,
        TrajectoryCommitIntent::Checkpoint,
    );
}

pub fn maybe_save_trajectory_background_with_intent(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    intent: TrajectoryCommitIntent,
) {
    if app.runtime.shutdown_flag.load(Ordering::Relaxed) || !intent.persists() {
        return;
    }
    tokio::spawn(async move {
        if let Err(error) = schedule_trajectory_writer(app, session_arc, intent).await {
            warn!("{}", error);
        }
    });
}

pub async fn maybe_save_trajectory(app: AppState, session_arc: Arc<AMutex<ChatSession>>) {
    maybe_save_trajectory_with_intent(app, session_arc, TrajectoryCommitIntent::Required).await;
}

pub async fn maybe_save_trajectory_with_intent(
    app: AppState,
    session_arc: Arc<AMutex<ChatSession>>,
    intent: TrajectoryCommitIntent,
) {
    if intent == TrajectoryCommitIntent::Checkpoint {
        maybe_save_trajectory_background_with_intent(app, session_arc, intent);
        return;
    }
    if let Err(e) = try_save_trajectory_with_intent(app, session_arc.clone(), intent).await {
        warn!("{}", e);
    }
}

#[cfg(test)]
fn trajectory_commit_contract() -> [(&'static str, TrajectoryCommitIntent); 7] {
    [
        (
            "explicit acknowledged mutations",
            TrajectoryCommitIntent::Required,
        ),
        (
            "branch handoff mode transition",
            TrajectoryCommitIntent::Required,
        ),
        ("final failed subchat", TrajectoryCommitIntent::Required),
        ("close", TrajectoryCommitIntent::Required),
        ("graceful shutdown", TrajectoryCommitIntent::Required),
        (
            "assistant tool subchat progress",
            TrajectoryCommitIntent::Checkpoint,
        ),
        ("stream delta", TrajectoryCommitIntent::Ephemeral),
    ]
}

#[cfg(test)]
pub(crate) fn trajectory_commit_contract_for_test(boundary: &str) -> TrajectoryCommitIntent {
    trajectory_commit_contract()
        .into_iter()
        .find_map(|(name, intent)| (name == boundary).then_some(intent))
        .expect("known trajectory commit boundary")
}

pub(crate) async fn persist_loaded_trajectory_repair_raw(
    gcx: Arc<GlobalContext>,
    repair: &TrajectoryRepairPatch,
) -> Result<String, String> {
    let chat_id = &repair.chat_id;
    validate_trajectory_id(chat_id).map_err(|e| e.message)?;
    let file_path = &repair.source_path;
    if !trajectory_source_path_is_allowed(gcx.clone(), file_path).await {
        return Err(format!(
            "Trajectory source path is not in an approved trajectory root: {}",
            file_path.display()
        ));
    }
    let content = tokio::fs::read_to_string(&file_path)
        .await
        .map_err(|e| format!("Failed to read trajectory: {}", e))?;
    let mut trajectory = serde_json::from_str::<serde_json::Value>(&content)
        .map_err(|e| format!("Failed to parse trajectory: {}", e))?;
    let trajectory_object = trajectory
        .as_object_mut()
        .ok_or_else(|| "Trajectory JSON root must be an object".to_string())?;
    let current_chat_id = trajectory_object.get("id").and_then(|value| value.as_str());
    if current_chat_id != Some(chat_id.as_str()) {
        return Err(format!(
            "Trajectory source id mismatch for repair: expected {}, found {}",
            chat_id,
            current_chat_id.unwrap_or("<missing>")
        ));
    }

    trajectory_object.remove("previous_response_id");
    trajectory_object.remove("claude_code_identity");
    match &repair.frozen_request_prefix {
        Some(frozen_request_prefix) => {
            trajectory_object.insert(
                "frozen_request_prefix".to_string(),
                serde_json::to_value(frozen_request_prefix).map_err(|e| e.to_string())?,
            );
        }
        None => {
            trajectory_object.remove("frozen_request_prefix");
        }
    }
    trajectory_object.insert(
        "created_at".to_string(),
        serde_json::Value::String(repair.created_at.clone()),
    );
    trajectory_object.insert(
        "auto_approve_editing_tools".to_string(),
        json!(repair.auto_approve_editing_tools),
    );
    trajectory_object.insert(
        "auto_approve_dangerous_commands".to_string(),
        json!(repair.auto_approve_dangerous_commands),
    );
    let updated_at = chrono::Utc::now().to_rfc3339();
    trajectory_object.insert(
        "updated_at".to_string(),
        serde_json::Value::String(updated_at.clone()),
    );

    let tmp_path = unique_trajectory_tmp_path(file_path);
    let (trajectory, json_result) = tokio::task::spawn_blocking(move || {
        let json_result = serde_json::to_string_pretty(&trajectory)
            .map_err(|e| format!("Failed to serialize trajectory: {}", e));
        (trajectory, json_result)
    })
    .await
    .map_err(|e| format!("Trajectory repair serialization task failed: {}", e))?;
    atomic_write_json_with_tmp_path(
        file_path,
        &tmp_path,
        json_result,
        Some("Failed to write trajectory"),
        Some(trajectory_index::source_from_hint_or_value(
            &trajectory,
            None,
        )),
    )
    .await?;
    if let Some(dir) = index_dir_for_trajectory_file(gcx.clone(), file_path).await {
        let source_hint = Some(trajectory_index::source_from_hint_or_value(
            &trajectory,
            None,
        ));
        trajectory_index::upsert_trajectory_index_entry_from_owned_value_with_rollout(
            &AppState::from_gcx(gcx)
                .await
                .chat
                .trajectory_index_coordinator,
            &dir,
            file_path,
            trajectory,
            source_hint,
        )
        .await?;
    }
    Ok(updated_at)
}

async fn persist_loaded_trajectory_repair(gcx: Arc<GlobalContext>, mut loaded: LoadedTrajectory) {
    let chat_id = loaded.thread.id.clone();
    apply_mode_defaults_to_thread(
        gcx.clone(),
        &mut loaded.thread,
        loaded.auto_approve_editing_tools_present,
        loaded.auto_approve_dangerous_commands_present,
    )
    .await;
    if let Err(e) = persist_loaded_trajectory_repair_raw(gcx.clone(), &loaded.repair_patch()).await
    {
        warn!(
            "Failed to persist repaired trajectory for {}: {}",
            chat_id, e
        );
    }
}

fn loaded_trajectory_source(
    loaded: &LoadedTrajectory,
    task_roots: &[PathBuf],
) -> TrajectorySourceIdentity {
    let loaded_source = TrajectorySourceIdentity::from_session_parts(&loaded.thread);
    let path_source = trajectory_source_identity_from_path(&loaded.source_path, task_roots);
    if matches!(&loaded_source, TrajectorySourceIdentity::Normal)
        && !matches!(&path_source, TrajectorySourceIdentity::Normal)
    {
        path_source
    } else {
        loaded_source
    }
}

fn external_delete_matches_session(
    deleted_source: Option<&TrajectorySourceIdentity>,
    session: &ChatSession,
) -> bool {
    if is_active_buddy_session(session) {
        return false;
    }
    match deleted_source {
        Some(source) => source.matches_session_for_delete(session),
        None => TrajectorySourceIdentity::Normal.matches_session(session),
    }
}

fn can_apply_external_reload(session: &ChatSession) -> bool {
    session.runtime.state == SessionState::Idle
        && session.app_workspace_mutation_lock.try_lock().is_ok()
        && !session.trajectory_dirty
        && session.pending_deliveries.is_empty()
        && session.runner_pending_deliveries.is_empty()
        && !session.runtime.paused
        && session.runtime.pause_reasons.is_empty()
        && session.confirmation_paused_at.is_none()
        && !session.planner_pause_pending_tools
        && !session.has_pending_tool_result_window()
        && !session.planner_paused()
        && session.command_queue.is_empty()
        && !session.goal.as_ref().is_some_and(|goal| {
            matches!(
                goal.status,
                GoalStatus::Paused
                    | GoalStatus::Stopped
                    | GoalStatus::BudgetExhausted
                    | GoalStatus::NoProgress
            )
        })
}

async fn external_reload_transition(
    session_arc: &Arc<AMutex<ChatSession>>,
    expected_pending: Option<&ExternalReloadPending>,
    pending: Option<ExternalReloadPending>,
) -> Option<(
    super::worktree_lifecycle::AttachmentTransitionGuard,
    ThreadParams,
    u64,
)> {
    let transition = super::worktree_lifecycle::try_transition_lock(session_arc);
    let mut session = session_arc.lock().await;
    if expected_pending
        .is_some_and(|expected| session.external_reload_pending.as_ref() != Some(expected))
    {
        return None;
    }
    match transition {
        Ok(transition) => Some((
            transition,
            session.thread.clone(),
            session.trajectory_version,
        )),
        Err(error) => {
            if let Some(pending) =
                pending.filter(|pending| pending_reload_source_matches_session(pending, &session))
            {
                session.external_reload_pending = Some(pending);
            }
            tracing::debug!(chat_id = %session.chat_id, %error, "External reload deferred");
            None
        }
    }
}

fn external_reload_preserves_attachment(
    gcx: &GlobalContext,
    session_arc: &Arc<AMutex<ChatSession>>,
    session: &ChatSession,
    thread: &ThreadParams,
) -> bool {
    let established = !session.messages.is_empty()
        || session.thread.worktree.is_some()
        || session.thread.task_meta.is_some()
        || session.thread.buddy_meta.is_some()
        || session.thread.parent_id.is_some()
        || session.thread.root_chat_id.is_some();
    let current = gcx.chat_sessions.try_read().is_ok_and(|sessions| {
        sessions
            .get(&session.chat_id)
            .is_some_and(|current| Arc::ptr_eq(current, session_arc))
    });
    if !current
        || session.closed
        || thread.id != session.thread.id
        || (established
            && (session.thread.worktree != thread.worktree
                || session.thread.task_meta != thread.task_meta
                || session.thread.root_chat_id != thread.root_chat_id
                || session.thread.parent_id != thread.parent_id
                || session.thread.link_type != thread.link_type))
    {
        warn!(chat_id = %session.chat_id, "External reload cannot change an established checkout or ownership; retain WIP and use authorized attachment or lifecycle recovery");
        return false;
    }
    true
}

fn pending_reload_source_matches_session(
    pending: &ExternalReloadPending,
    session: &ChatSession,
) -> bool {
    match pending {
        ExternalReloadPending::Update {
            source,
        }
        | ExternalReloadPending::Delete {
            source,
        } => source.matches_session_for_delete(session),
    }
}

fn is_active_buddy_session(session: &ChatSession) -> bool {
    session.thread.buddy_meta.is_some()
}

fn apply_external_delete_to_session(
    _transition: &super::worktree_lifecycle::AttachmentTransitionGuard,
    session: &mut ChatSession,
    chat_id: &str,
) {
    session.release_turn_only_state();
    session.messages.clear();
    session.thread = ThreadParams {
        id: chat_id.to_string(),
        ..Default::default()
    };
    session.created_at = chrono::Utc::now().to_rfc3339();
    session.wake_up_at = None;
    session.waiting_for_card_ids.clear();
    session.set_goal_ledger(Vec::new());
    session.goal_verification_blocked_until_ms = None;
    session.pending_questions.clear();
    session.pending_browser_message = None;
    session.set_goal_projection(None);
    session.reset_compaction_runtime_state();
    session.external_reload_pending = None;
    let snapshot = session.snapshot();
    session.emit(snapshot);
}

fn apply_loaded_external_update_to_session(
    _transition: &super::worktree_lifecycle::AttachmentTransitionGuard,
    session: &mut ChatSession,
    loaded: LoadedTrajectory,
    transition_identity_repaired: bool,
) -> Option<u64> {
    session.release_turn_only_state();
    session.messages = loaded.messages;
    session.restore_pending_deliveries(loaded.pending_deliveries);
    session.merge_dropped_deliveries(loaded.dropped_deliveries);
    session.runtime.recovery = loaded.recovery;
    session.recovery_budget = None;
    session.thread = loaded.thread;
    session.reset_compaction_runtime_state();
    session.set_goal_ledger(loaded.goal_ledger);
    session.goal_verification_blocked_until_ms = loaded.goal_verification_blocked_until_ms;
    session.pending_questions = loaded.pending_questions;
    session.native_decision_receipts =
        crate::buddy::decision_receipts::restored(loaded.native_decision_receipts);
    session.pending_browser_message = None;
    if let Some(pending) = loaded.pending_browser_decision {
        super::browser_decisions::expire_restored_browser_decision(session, pending);
    }
    session.set_goal_projection(loaded.goal);
    session.created_at = loaded.created_at;
    session.wake_up_at = loaded.wake_up_at;
    session.waiting_for_card_ids = loaded.waiting_for_card_ids;
    session.external_reload_pending = None;
    if transition_identity_repaired {
        session.increment_version();
    }
    let snapshot = session.snapshot();
    session.emit(snapshot);
    transition_identity_repaired.then_some(session.trajectory_version)
}
fn session_is_passive_child_view(session: &ChatSession) -> bool {
    session.thread.buddy_meta.is_some()
        || (session.thread.parent_id.is_some()
            && session.thread.link_type.as_deref().map_or(true, |lt| {
                !matches!(lt, "handoff" | "mode_transition" | "branch")
            }))
}

fn session_is_stale_refresh_candidate(
    session: &ChatSession,
    allow_replacing_messages: bool,
) -> bool {
    if session.closed {
        return false;
    }
    if !can_apply_external_reload(session) {
        return false;
    }
    if !session.command_queue.is_empty() || session.draft_message.is_some() {
        return false;
    }
    if session
        .queue_processor_running
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        return false;
    }
    if !allow_replacing_messages
        && !session_is_passive_child_view(session)
        && !session.messages.is_empty()
    {
        return false;
    }
    true
}

pub async fn refresh_session_from_trajectory_if_stale(
    app: AppState,
    chat_id: &str,
    allow_replacing_messages: bool,
) -> bool {
    let session_arc = {
        let sessions_read = app.chat.sessions.read().await;
        sessions_read.get(chat_id).cloned()
    };
    let Some(session_arc) = session_arc else {
        return false;
    };
    let pending_source = {
        let session = session_arc.lock().await;
        TrajectorySourceIdentity::from_session(&session)
    };
    let Some((transition, before, version)) = external_reload_transition(
        &session_arc,
        None,
        Some(ExternalReloadPending::update(pending_source)),
    )
    .await
    else {
        return false;
    };
    {
        let session = session_arc.lock().await;
        if !session_is_stale_refresh_candidate(&session, allow_replacing_messages) {
            return false;
        }
    }
    let Some(mut loaded) = load_trajectory_for_chat(app.gcx.clone(), chat_id).await else {
        return false;
    };
    if loaded.messages.is_empty() {
        return false;
    }
    apply_mode_defaults_to_thread(
        app.gcx.clone(),
        &mut loaded.thread,
        loaded.auto_approve_editing_tools_present,
        loaded.auto_approve_dangerous_commands_present,
    )
    .await;
    let transition_identity_repaired = loaded.transition_identity_repaired;
    let mut session = session_arc.lock().await;
    if !session_is_stale_refresh_candidate(&session, allow_replacing_messages) {
        return false;
    }
    if super::worktree_lifecycle::check_unchanged_attachment(&session, &before).is_err()
        || session.trajectory_version != version
        || !external_reload_preserves_attachment(&app.gcx, &session_arc, &session, &loaded.thread)
    {
        session.external_reload_pending = Some(ExternalReloadPending::update(
            TrajectorySourceIdentity::from_session(&session),
        ));
        return false;
    }
    let session_has_identity = !session.messages.is_empty()
        || session.thread.task_meta.is_some()
        || session.thread.buddy_meta.is_some();
    if session_has_identity {
        let loaded_source = TrajectorySourceIdentity::from_session_parts(&loaded.thread);
        if !loaded_source.matches_session(&session) {
            return false;
        }
    }
    info!(
        "Refreshing stale session {} from trajectory ({} messages)",
        chat_id,
        loaded.messages.len()
    );
    apply_loaded_external_update_to_session(
        &transition,
        &mut session,
        loaded,
        transition_identity_repaired,
    );
    session.task_goal_messages = None;
    drop(session);
    if let Err(error) =
        crate::tasks::goal::hydrate_session_task_goal(app.gcx.clone(), &session_arc).await
    {
        tracing::warn!(%error, "Canonical task goal refresh unavailable");
        return false;
    }
    true
}

async fn apply_loaded_external_update_with_repair(
    gcx: Arc<GlobalContext>,
    session_arc: Arc<AMutex<ChatSession>>,
    chat_id: &str,
    mut loaded: LoadedTrajectory,
    expected_pending: Option<ExternalReloadPending>,
    pending_if_not_reloadable: Option<ExternalReloadPending>,
    log_message: &str,
) -> bool {
    let loaded_source = TrajectorySourceIdentity::from_session_parts(&loaded.thread);
    let Some((transition, before, version)) = external_reload_transition(
        &session_arc,
        expected_pending.as_ref(),
        pending_if_not_reloadable
            .clone()
            .or_else(|| expected_pending.clone()),
    )
    .await
    else {
        return false;
    };
    let transition_identity_repaired = loaded.transition_identity_repaired;
    apply_mode_defaults_to_thread(
        gcx.clone(),
        &mut loaded.thread,
        loaded.auto_approve_editing_tools_present,
        loaded.auto_approve_dangerous_commands_present,
    )
    .await;
    let repair_patch = loaded.repair_patch();
    let mut skipped_active_buddy = false;
    let mut skipped_source_mismatch = false;
    let repaired_version = {
        let mut session = session_arc.lock().await;
        if let Some(expected) = expected_pending {
            if session.external_reload_pending != Some(expected) {
                return false;
            }
        }
        if !loaded_source.matches_session(&session) {
            skipped_source_mismatch = true;
            None
        } else if is_active_buddy_session(&session) {
            skipped_active_buddy = true;
            None
        } else if !can_apply_external_reload(&session)
            || super::worktree_lifecycle::check_unchanged_attachment(&session, &before).is_err()
            || session.trajectory_version != version
            || !external_reload_preserves_attachment(&gcx, &session_arc, &session, &loaded.thread)
        {
            if let Some(pending) =
                pending_if_not_reloadable.or_else(|| session.external_reload_pending.clone())
            {
                session.external_reload_pending = Some(pending);
            }
            return false;
        } else {
            info!("{}", log_message);
            apply_loaded_external_update_to_session(
                &transition,
                &mut session,
                loaded,
                transition_identity_repaired,
            )
        }
    };
    if skipped_active_buddy || skipped_source_mismatch {
        if transition_identity_repaired {
            if let Err(e) = persist_loaded_trajectory_repair_raw(gcx.clone(), &repair_patch).await {
                warn!(
                    "Failed to persist repaired trajectory for {}: {}",
                    chat_id, e
                );
            }
        }
        return false;
    }
    if let Some(repaired_version) = repaired_version {
        if let Err(e) = persist_loaded_trajectory_repair_raw(gcx.clone(), &repair_patch).await {
            warn!(
                "Failed to persist repaired trajectory for {}: {}",
                chat_id, e
            );
        } else {
            let mut session = session_arc.lock().await;
            session.complete_trajectory_commit(repaired_version);
        }
    }
    true
}

enum ExternalDeleteRevalidationOutcome {
    Updated {
        loaded: LoadedTrajectory,
        applied_to_session: bool,
    },
    Deleted {
        applied_to_session: bool,
    },
    NoopStalePending,
    Deferred,
}

async fn apply_loaded_external_delete_with_revalidation(
    gcx: Arc<GlobalContext>,
    session_arc: Arc<AMutex<ChatSession>>,
    chat_id: &str,
    expected_pending: Option<ExternalReloadPending>,
    deleted_source: Option<TrajectorySourceIdentity>,
    mut loaded: LoadedTrajectory,
    transition: &super::worktree_lifecycle::AttachmentTransitionGuard,
    before: &ThreadParams,
    version: u64,
) -> ExternalDeleteRevalidationOutcome {
    let task_roots = get_all_task_roots(gcx.clone()).await;
    let loaded_source = loaded_trajectory_source(&loaded, &task_roots);
    let transition_identity_repaired = loaded.transition_identity_repaired;
    apply_mode_defaults_to_thread(
        gcx.clone(),
        &mut loaded.thread,
        loaded.auto_approve_editing_tools_present,
        loaded.auto_approve_dangerous_commands_present,
    )
    .await;
    let repair_patch = loaded.repair_patch();
    let outcome_loaded = loaded.clone();
    let same_source_loaded = match deleted_source.as_ref() {
        Some(source) if loaded_source != *source => {
            let mut loaded =
                load_generic_trajectory_for_chat_matching_source(gcx.clone(), chat_id, source)
                    .await;
            if let Some(loaded) = loaded.as_mut() {
                apply_mode_defaults_to_thread(
                    gcx.clone(),
                    &mut loaded.thread,
                    loaded.auto_approve_editing_tools_present,
                    loaded.auto_approve_dangerous_commands_present,
                )
                .await;
            }
            loaded
        }
        _ => None,
    };
    let same_source_repair_patch = same_source_loaded
        .as_ref()
        .map(LoadedTrajectory::repair_patch);
    let same_source_transition_identity_repaired = same_source_loaded
        .as_ref()
        .is_some_and(|loaded| loaded.transition_identity_repaired);
    let (repaired_version, applied_to_session) = {
        let mut session = session_arc.lock().await;
        if let Some(expected) = expected_pending {
            if session.external_reload_pending != Some(expected) {
                return ExternalDeleteRevalidationOutcome::NoopStalePending;
            }
        }
        let deleted_matches_session =
            external_delete_matches_session(deleted_source.as_ref(), &session);
        let fallback_matches_session = loaded_source.matches_session(&session);
        if !(deleted_matches_session || fallback_matches_session) {
            drop(session);
            if transition_identity_repaired {
                if let Err(e) =
                    persist_loaded_trajectory_repair_raw(gcx.clone(), &repair_patch).await
                {
                    warn!(
                        "Failed to persist repaired trajectory for {}: {}",
                        chat_id, e
                    );
                }
            }
            return ExternalDeleteRevalidationOutcome::Updated {
                loaded: outcome_loaded,
                applied_to_session: false,
            };
        }
        let replacement = if deleted_matches_session && !fallback_matches_session {
            same_source_loaded
                .as_ref()
                .map(|loaded| loaded.thread.clone())
        } else {
            Some(loaded.thread.clone())
        }
        .unwrap_or_else(|| ThreadParams {
            id: chat_id.into(),
            ..Default::default()
        });
        if super::worktree_lifecycle::check_unchanged_attachment(&session, before).is_err()
            || session.trajectory_version != version
            || !external_reload_preserves_attachment(&gcx, &session_arc, &session, &replacement)
        {
            session.external_reload_pending = Some(ExternalReloadPending::delete(
                TrajectorySourceIdentity::from_session(&session),
            ));
            return ExternalDeleteRevalidationOutcome::Updated {
                loaded: outcome_loaded,
                applied_to_session: false,
            };
        }
        if deleted_matches_session && !fallback_matches_session {
            if let Some(same_source_loaded) = same_source_loaded {
                if !can_apply_external_reload(&session) {
                    let pending_source = TrajectorySourceIdentity::from_session(&session);
                    session.external_reload_pending =
                        Some(ExternalReloadPending::delete(pending_source));
                    return ExternalDeleteRevalidationOutcome::Updated {
                        loaded: outcome_loaded,
                        applied_to_session: false,
                    };
                }
                info!(
                    "Reloading same-source trajectory for {} after delete revalidation",
                    chat_id
                );
                let repaired_version = apply_loaded_external_update_to_session(
                    transition,
                    &mut session,
                    same_source_loaded,
                    same_source_transition_identity_repaired,
                );
                (repaired_version, true)
            } else {
                if !can_apply_external_reload(&session) {
                    let pending_source = TrajectorySourceIdentity::from_session(&session);
                    session.external_reload_pending =
                        Some(ExternalReloadPending::delete(pending_source));
                    return ExternalDeleteRevalidationOutcome::Updated {
                        loaded: outcome_loaded,
                        applied_to_session: false,
                    };
                }
                info!(
                    "Clearing trajectory for {} after same-source delete with fallback mismatch",
                    chat_id
                );
                apply_external_delete_to_session(transition, &mut session, chat_id);
                (None, true)
            }
        } else if !can_apply_external_reload(&session) {
            let pending_source = TrajectorySourceIdentity::from_session(&session);
            session.external_reload_pending = Some(ExternalReloadPending::delete(pending_source));
            return ExternalDeleteRevalidationOutcome::Updated {
                loaded: outcome_loaded,
                applied_to_session: false,
            };
        } else {
            info!(
                "Reloading trajectory for {} after delete revalidation",
                chat_id
            );
            let repaired_version = apply_loaded_external_update_to_session(
                transition,
                &mut session,
                loaded,
                transition_identity_repaired,
            );
            (repaired_version, true)
        }
    };
    if let Some(repaired_version) = repaired_version {
        let applied_repair_patch = same_source_repair_patch.as_ref().unwrap_or(&repair_patch);
        if let Err(e) =
            persist_loaded_trajectory_repair_raw(gcx.clone(), applied_repair_patch).await
        {
            warn!(
                "Failed to persist repaired trajectory for {}: {}",
                chat_id, e
            );
        } else {
            let mut session = session_arc.lock().await;
            session.complete_trajectory_commit(repaired_version);
        }
    }
    if same_source_transition_identity_repaired {
        if let Some(repair_patch) = same_source_repair_patch.as_ref() {
            if let Err(e) = persist_loaded_trajectory_repair_raw(gcx.clone(), repair_patch).await {
                warn!(
                    "Failed to persist repaired trajectory for {}: {}",
                    chat_id, e
                );
            }
        }
    }
    if transition_identity_repaired {
        if let Err(e) = persist_loaded_trajectory_repair_raw(gcx.clone(), &repair_patch).await {
            warn!(
                "Failed to persist repaired trajectory for {}: {}",
                chat_id, e
            );
        }
    }
    return ExternalDeleteRevalidationOutcome::Updated {
        loaded: outcome_loaded,
        applied_to_session,
    };
}

async fn apply_external_delete_with_revalidation(
    gcx: Arc<GlobalContext>,
    session_arc: Arc<AMutex<ChatSession>>,
    chat_id: &str,
    expected_pending: Option<ExternalReloadPending>,
    deleted_source: Option<TrajectorySourceIdentity>,
) -> ExternalDeleteRevalidationOutcome {
    if let Some(expected) = expected_pending.as_ref() {
        if session_arc.lock().await.external_reload_pending.as_ref() != Some(expected) {
            return ExternalDeleteRevalidationOutcome::NoopStalePending;
        }
    }
    let pending_source = deleted_source
        .clone()
        .unwrap_or(TrajectorySourceIdentity::Normal);
    let Some((transition, before, version)) = external_reload_transition(
        &session_arc,
        expected_pending.as_ref(),
        Some(ExternalReloadPending::delete(pending_source)),
    )
    .await
    else {
        return ExternalDeleteRevalidationOutcome::Deferred;
    };
    let loaded = load_generic_trajectory_for_chat(gcx.clone(), chat_id).await;
    if let Some(loaded) = loaded {
        return apply_loaded_external_delete_with_revalidation(
            gcx,
            session_arc,
            chat_id,
            expected_pending,
            deleted_source,
            loaded,
            &transition,
            &before,
            version,
        )
        .await;
    }

    let mut session = session_arc.lock().await;
    if let Some(expected) = expected_pending {
        if session.external_reload_pending != Some(expected) {
            return ExternalDeleteRevalidationOutcome::NoopStalePending;
        }
    }
    if !external_delete_matches_session(deleted_source.as_ref(), &session) {
        return ExternalDeleteRevalidationOutcome::Deleted {
            applied_to_session: false,
        };
    }
    if !can_apply_external_reload(&session)
        || super::worktree_lifecycle::check_unchanged_attachment(&session, &before).is_err()
        || session.trajectory_version != version
        || !external_reload_preserves_attachment(
            &gcx,
            &session_arc,
            &session,
            &ThreadParams {
                id: chat_id.into(),
                ..Default::default()
            },
        )
    {
        let pending_source = TrajectorySourceIdentity::from_session(&session);
        session.external_reload_pending = Some(ExternalReloadPending::delete(pending_source));
        return ExternalDeleteRevalidationOutcome::Deleted {
            applied_to_session: false,
        };
    }
    info!("Trajectory file removed externally for {}", chat_id);
    apply_external_delete_to_session(&transition, &mut session, chat_id);
    ExternalDeleteRevalidationOutcome::Deleted {
        applied_to_session: true,
    }
}

pub async fn check_external_reload_pending(
    gcx: Arc<GlobalContext>,
    session_arc: Arc<AMutex<ChatSession>>,
) {
    let (chat_id, pending) = {
        let mut session = session_arc.lock().await;
        if is_active_buddy_session(&session) {
            return;
        }
        let pending = if can_apply_external_reload(&session) {
            session.external_reload_pending.clone()
        } else {
            None
        };
        if let Some(pending) = pending.as_ref() {
            if !pending_reload_source_matches_session(pending, &session) {
                warn!(
                    "Clearing pending external reload for {} because pending source no longer matches active session",
                    session.chat_id
                );
                session.external_reload_pending = None;
                return;
            }
        }
        (session.chat_id.clone(), pending)
    };
    match pending {
        Some(ExternalReloadPending::Delete {
            source,
        }) => {
            let expected_pending = ExternalReloadPending::delete(source.clone());
            apply_external_delete_with_revalidation(
                gcx.clone(),
                session_arc.clone(),
                &chat_id,
                Some(expected_pending),
                Some(source),
            )
            .await;
        }
        Some(ExternalReloadPending::Update {
            source,
        }) => {
            let expected_pending = ExternalReloadPending::update(source.clone());
            if let Some(loaded) =
                load_generic_trajectory_for_chat_matching_source(gcx.clone(), &chat_id, &source)
                    .await
            {
                apply_loaded_external_update_with_repair(
                    gcx.clone(),
                    session_arc.clone(),
                    &chat_id,
                    loaded,
                    Some(expected_pending),
                    None,
                    &format!("Applying pending external reload for {}", chat_id),
                )
                .await;
            } else {
                let mut session = session_arc.lock().await;
                if can_apply_external_reload(&session)
                    && session.external_reload_pending == Some(expected_pending)
                {
                    warn!(
                        "Clearing pending external reload for {} because matching trajectory source is missing",
                        chat_id
                    );
                    session.external_reload_pending = None;
                }
            }
        }
        None => {}
    }
}

async fn refresh_trajectory_index_entry_for_path(
    gcx: Arc<GlobalContext>,
    path: &Path,
    source_hint: Option<TrajectorySourceIdentity>,
) -> Result<(), String> {
    let app = AppState::from_gcx(gcx.clone()).await;
    let dir = index_dir_for_trajectory_file(gcx, path)
        .await
        .ok_or_else(|| format!("Trajectory path has no parent: {}", path.display()))?;
    let dir = dir.as_path();
    let content = fs::read_to_string(path)
        .await
        .map_err(|e| format!("Failed to read trajectory for index refresh: {}", e))?;
    let source_hash = trajectory_index::source_sha256(content.as_bytes());
    let parse_path = path.to_path_buf();
    let value = tokio::task::spawn_blocking(move || {
        serde_json::from_str::<serde_json::Value>(&content).map_err(|e| {
            format!(
                "Failed to parse trajectory '{}' for index refresh: {}",
                parse_path.display(),
                e
            )
        })
    })
    .await
    .map_err(|e| format!("Trajectory index refresh parse task failed: {}", e))??;
    trajectory_index::upsert_trajectory_index_entry_from_committed_value(
        &app.chat.trajectory_index_coordinator,
        dir,
        path,
        value,
        source_hint,
        Some(source_hash),
    )
    .await
}

/// Drops index entries for one chat id whose backing file no longer matches. The work is scoped
/// to that id, so it costs one id-map lookup plus a listing of each directory that actually
/// indexes it — normally zero or one.
///
/// It used to resolve the chat first and sweep every trajectory directory when that missed, which
/// is the common case here: this runs precisely when a file just disappeared. A directory the id
/// map does not mention cannot be holding an entry the coordinator would return, and listing it
/// would reconcile the entry away regardless, so sweeping bought nothing per watcher event.
async fn remove_stale_trajectory_index_entries(gcx: Arc<GlobalContext>, chat_id: &str) {
    let app = AppState::from_gcx(gcx.clone()).await;
    let dirs = app
        .chat
        .trajectory_index_coordinator
        .entry_dirs_for_id(chat_id);
    for dir in dirs {
        let entries = match trajectory_index::list_trajectory_entries_with_rollout_shared_for(
            &app.chat.trajectory_index_coordinator,
            &dir,
            None,
            trajectory_index::TrajectoryIndexListingCaller::StaleEntryCleanup,
        )
        .await
        {
            Ok(entries) => entries,
            Err(e) => {
                warn!("Failed to read trajectory index {:?}: {}", dir, e);
                continue;
            }
        };
        let mut should_remove = false;
        for entry in entries.iter() {
            if entry.id == chat_id
                && !trajectory_index::trajectory_index_entry_is_fresh(&dir, entry).await
            {
                should_remove = true;
                break;
            }
        }
        if should_remove {
            if let Err(e) = trajectory_index::remove_trajectory_index_entry_with_rollout(
                &app.chat.trajectory_index_coordinator,
                &dir,
                chat_id,
            )
            .await
            {
                warn!(
                    "Failed to remove stale trajectory {} from index {:?}: {}",
                    chat_id, dir, e
                );
            }
        }
    }
}

#[cfg(test)]
async fn process_trajectory_change(gcx: Arc<GlobalContext>, chat_id: &str, is_remove: bool) {
    process_trajectory_change_for_source(gcx, chat_id, is_remove, None).await;
}

async fn process_trajectory_change_for_source(
    gcx: Arc<GlobalContext>,
    chat_id: &str,
    is_remove: bool,
    changed_source: Option<TrajectorySourceIdentity>,
) {
    let app = AppState::from_gcx(gcx.clone()).await;
    let sessions = app.chat.sessions.clone();

    if is_remove {
        remove_stale_trajectory_index_entries(gcx.clone(), chat_id).await;
        let session_arc = {
            let sessions_read = sessions.read().await;
            sessions_read.get(chat_id).cloned()
        };
        let outcome = if let Some(session_arc) = session_arc {
            apply_external_delete_with_revalidation(
                gcx.clone(),
                session_arc,
                chat_id,
                None,
                changed_source.clone(),
            )
            .await
        } else if let Some(loaded) = load_generic_trajectory_for_chat(gcx.clone(), chat_id).await {
            if loaded.transition_identity_repaired {
                persist_loaded_trajectory_repair(gcx.clone(), loaded.clone()).await;
            }
            ExternalDeleteRevalidationOutcome::Updated {
                loaded,
                applied_to_session: false,
            }
        } else {
            ExternalDeleteRevalidationOutcome::Deleted {
                applied_to_session: false,
            }
        };
        let event = match outcome {
            ExternalDeleteRevalidationOutcome::Updated {
                loaded,
                applied_to_session,
            } => {
                debug!(
                    "External delete revalidation for {} updated list; applied_to_session={}",
                    chat_id, applied_to_session
                );
                let task_roots = get_all_task_roots(gcx.clone()).await;
                let meta = loaded_trajectory_to_meta(&loaded, &task_roots);
                let (session_state, session_error, _) =
                    get_session_runtime_for_trajectory_source(&sessions, chat_id, &meta.source)
                        .await;
                let is_title_generated = loaded.thread.is_title_generated;
                updated_trajectory_event_from_meta(
                    meta,
                    Some(is_title_generated),
                    session_state,
                    session_error,
                )
            }
            ExternalDeleteRevalidationOutcome::Deleted {
                applied_to_session,
            } => {
                debug!(
                    "External delete revalidation for {} deleted list item; applied_to_session={}",
                    chat_id, applied_to_session
                );
                deleted_trajectory_event(chat_id.to_string())
            }
            ExternalDeleteRevalidationOutcome::NoopStalePending
            | ExternalDeleteRevalidationOutcome::Deferred => return,
        };
        let _ = app.chat.trajectory_events_tx.send(event);
        return;
    }

    let mut loaded = match changed_source.as_ref() {
        Some(source) => {
            load_generic_trajectory_for_chat_matching_source(gcx.clone(), chat_id, source).await
        }
        None => load_generic_trajectory_for_chat(gcx.clone(), chat_id).await,
    };

    if let Some(loaded_for_index) = loaded.as_ref() {
        let source_hint = changed_source.clone().or_else(|| {
            Some(TrajectorySourceIdentity::from_session_parts(
                &loaded_for_index.thread,
            ))
        });
        if let Err(e) = refresh_trajectory_index_entry_for_path(
            gcx.clone(),
            &loaded_for_index.source_path,
            source_hint,
        )
        .await
        {
            warn!("Failed to refresh trajectory index for {}: {}", chat_id, e);
        }
    } else if stale_entry_cleanup_allowed(chat_id) {
        remove_stale_trajectory_index_entries(gcx.clone(), chat_id).await;
    }

    let session_arc = {
        let sessions_read = sessions.read().await;
        sessions_read.get(chat_id).cloned()
    };

    if let Some(session_arc) = session_arc {
        if let Some(t) = loaded.as_ref() {
            let task_roots = get_all_task_roots(gcx.clone()).await;
            let meta = loaded_trajectory_to_meta(t, &task_roots);
            let (session_state, session_error, _) =
                get_session_runtime_for_trajectory_source(&sessions, chat_id, &meta.source).await;
            let is_title_generated = t.thread.is_title_generated;
            let event = updated_trajectory_event_from_meta(
                meta,
                Some(is_title_generated),
                session_state,
                session_error,
            );
            let _ = app.chat.trajectory_events_tx.send(event);
        }

        let source_collision = {
            let session = session_arc.lock().await;
            loaded.as_ref().is_some_and(|loaded| {
                !TrajectorySourceIdentity::from_session_parts(&loaded.thread)
                    .matches_session(&session)
            })
        };
        if source_collision {
            if let Some(loaded) = loaded.take() {
                if loaded.transition_identity_repaired {
                    persist_loaded_trajectory_repair(gcx.clone(), loaded).await;
                }
            }
            return;
        }

        if let Some(loaded) = loaded.take() {
            let pending = ExternalReloadPending::update(
                TrajectorySourceIdentity::from_session_parts(&loaded.thread),
            );
            apply_loaded_external_update_with_repair(
                gcx.clone(),
                session_arc.clone(),
                chat_id,
                loaded,
                None,
                Some(pending),
                &format!("Reloading trajectory for {} from external change", chat_id),
            )
            .await;
        } else {
            let mut session = session_arc.lock().await;
            let pending_source = changed_source
                .clone()
                .unwrap_or_else(|| TrajectorySourceIdentity::Normal);
            if !pending_source.matches_session(&session) {
                return;
            }
            if !can_apply_external_reload(&session) {
                session.external_reload_pending =
                    Some(ExternalReloadPending::update(pending_source));
            }
        }
        return;
    }

    let Some(loaded) = loaded.take() else {
        return;
    };

    let task_roots = get_all_task_roots(gcx.clone()).await;
    let meta = loaded_trajectory_to_meta(&loaded, &task_roots);
    let (session_state, session_error, _) =
        get_session_runtime_for_trajectory_source(&sessions, chat_id, &meta.source).await;
    let is_title_generated = loaded.thread.is_title_generated;
    let event = updated_trajectory_event_from_meta(
        meta,
        Some(is_title_generated),
        session_state,
        session_error,
    );
    let _ = app.chat.trajectory_events_tx.send(event);

    if loaded.transition_identity_repaired {
        persist_loaded_trajectory_repair(gcx, loaded).await;
    }
}

fn task_trajectory_context_from_path(
    path: &Path,
    task_roots: &[PathBuf],
) -> Option<(String, String, Option<String>)> {
    let path = crate::files_correction::canonicalize_normalized_path(path.to_path_buf());
    for root in task_roots {
        let root = crate::files_correction::canonicalize_normalized_path(root.clone());
        if !is_real_dir_sync(&root) {
            continue;
        }
        let Ok(relative) = path.strip_prefix(&root) else {
            continue;
        };
        let parts: Vec<String> = relative
            .components()
            .filter_map(|component| component.as_os_str().to_str().map(|s| s.to_string()))
            .collect();
        if parts.len() < 4 || parts.get(1).map(|s| s.as_str()) != Some("trajectories") {
            continue;
        }
        let role = parts[2].as_str();
        if role != "planner" && role != "linked" && role != "agents" && role != "subchats" {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let agent_id = if role == "agents" && parts.len() >= 5 {
            Some(parts[3].clone())
        } else {
            None
        };
        return Some((parts[0].clone(), role.to_string(), agent_id));
    }
    None
}

fn is_under_task_root(path: &Path, task_roots: &[PathBuf]) -> bool {
    let path = crate::files_correction::canonicalize_normalized_path(path.to_path_buf());
    task_roots.iter().any(|root| {
        let root = crate::files_correction::canonicalize_normalized_path(root.clone());
        is_real_dir_sync(&root) && path.starts_with(&root)
    })
}

fn should_dispatch_trajectory_path(path: &Path, task_roots: &[PathBuf]) -> bool {
    if path.extension().and_then(|e| e.to_str()) != Some("json") {
        return false;
    }
    if path.file_name().is_some_and(|name| name == "index.json") {
        return false;
    }
    if !is_under_task_root(path, task_roots) {
        return true;
    }
    task_trajectory_context_from_path(path, task_roots).is_some()
}

fn is_trajectory_temporary_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.rsplit_once(".tmp."))
        .is_some_and(|(prefix, random)| prefix.ends_with(".json") && !random.is_empty())
}

fn trajectory_source_identity_from_path(
    path: &Path,
    task_roots: &[PathBuf],
) -> TrajectorySourceIdentity {
    task_trajectory_context_from_path(path, task_roots)
        .map(|(task_id, role, agent_id)| {
            TrajectorySourceIdentity::task(task_id, role, agent_id, None, None)
        })
        .unwrap_or(TrajectorySourceIdentity::Normal)
}

async fn collect_task_trajectory_sources_under_path(
    path: &Path,
    task_roots: &[PathBuf],
) -> Vec<(String, TrajectorySourceIdentity, PathBuf)> {
    if !is_under_task_root(path, task_roots) {
        return Vec::new();
    }
    let mut sources = Vec::new();
    let mut pending = vec![path.to_path_buf()];
    while let Some(path) = pending.pop() {
        if should_dispatch_trajectory_path(&path, task_roots) {
            if let Some(chat_id) = path.file_stem().and_then(|s| s.to_str()) {
                sources.push((
                    chat_id.to_string(),
                    trajectory_source_identity_from_path(&path, task_roots),
                    path,
                ));
            }
            continue;
        }

        if !is_real_dir(&path).await {
            continue;
        }

        let mut entries = match fs::read_dir(&path).await {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            pending.push(entry.path());
        }
    }
    sources
}

async fn collect_trajectory_sources_under_path(
    path: &Path,
    task_roots: &[PathBuf],
) -> Vec<(String, TrajectorySourceIdentity, PathBuf)> {
    if is_under_task_root(path, task_roots) {
        return collect_task_trajectory_sources_under_path(path, task_roots).await;
    }

    let mut sources = Vec::new();
    let mut pending = vec![path.to_path_buf()];
    while let Some(path) = pending.pop() {
        if should_dispatch_trajectory_path(&path, task_roots) {
            if let Some(chat_id) = path.file_stem().and_then(|stem| stem.to_str()) {
                sources.push((chat_id.to_string(), TrajectorySourceIdentity::Normal, path));
            }
            continue;
        }
        if !is_real_dir(&path).await {
            continue;
        }
        let mut entries = match fs::read_dir(&path).await {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            pending.push(entry.path());
        }
    }
    sources
}

fn schedule_trajectory_watcher_overflow_recovery(
    pending_scans: &mut std::collections::HashMap<PathBuf, Instant>,
    dropped_events: &AtomicUsize,
    watched_roots: &[PathBuf],
) {
    let dropped = dropped_events.swap(0, Ordering::Relaxed);
    if dropped == 0 {
        return;
    }
    warn!(
        "Trajectory watcher dropped {} events; scheduling a recovery scan",
        dropped
    );
    let now = Instant::now();
    for root in watched_roots {
        pending_scans.insert(root.clone(), now);
    }
}

type TrajectoryPendingKey = (String, TrajectorySourceIdentity);

struct TrajectoryPendingChange {
    last_event_at: Instant,
    is_remove: bool,
    path: PathBuf,
}

type TrajectoryPendingMap =
    std::collections::HashMap<TrajectoryPendingKey, TrajectoryPendingChange>;

fn insert_pending_trajectory_change(
    pending: &mut TrajectoryPendingMap,
    chat_id: String,
    is_remove: bool,
    source: TrajectorySourceIdentity,
    path: PathBuf,
    now: Instant,
) {
    pending.insert(
        (chat_id, source),
        TrajectoryPendingChange {
            last_event_at: now,
            is_remove,
            path,
        },
    );
}

enum TrajectoryWatcherMessage {
    Trajectory {
        chat_id: String,
        is_remove: bool,
        source: TrajectorySourceIdentity,
        path: PathBuf,
    },
    ScanPath(PathBuf),
}

fn trajectory_watcher_message_for_path(
    path: PathBuf,
    is_remove: bool,
    task_roots: &[PathBuf],
) -> Option<TrajectoryWatcherMessage> {
    if is_trajectory_temporary_file(&path) {
        return None;
    }
    if should_dispatch_trajectory_path(&path, task_roots) {
        let chat_id = path.file_stem()?.to_str()?.to_string();
        return Some(TrajectoryWatcherMessage::Trajectory {
            chat_id,
            is_remove,
            source: trajectory_source_identity_from_path(&path, task_roots),
            path,
        });
    }
    (!is_remove && is_under_task_root(&path, task_roots))
        .then_some(TrajectoryWatcherMessage::ScanPath(path))
}

async fn process_trajectory_watcher_change(
    gcx: Arc<GlobalContext>,
    chat_id: String,
    is_remove: bool,
    source: TrajectorySourceIdentity,
    path: PathBuf,
) {
    if trajectory_watcher_self_write_rollout_enabled()
        && (consume_trajectory_self_write(&path, &source).await
            || (!matches!(source, TrajectorySourceIdentity::Normal)
                && consume_trajectory_self_write(&path, &TrajectorySourceIdentity::Normal).await))
    {
        return;
    }
    process_trajectory_change_for_source(gcx, &chat_id, is_remove, Some(source)).await;
}

static TRAJECTORY_WATCHER_TASK: OnceLock<StdMutex<Option<tokio::task::JoinHandle<()>>>> =
    OnceLock::new();

pub fn start_trajectory_watcher(gcx: Arc<GlobalContext>) {
    let watcher_task = TRAJECTORY_WATCHER_TASK.get_or_init(|| StdMutex::new(None));
    {
        let guard = watcher_task
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.as_ref().is_some_and(|handle| !handle.is_finished()) {
            return;
        }
    }
    let handle = spawn_trajectory_watcher(
        gcx,
        None,
        #[cfg(test)]
        None,
    );
    let mut guard = watcher_task
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = Some(handle);
}

#[cfg(test)]
struct TrajectoryWatcherRegistrationGate {
    entered: tokio::sync::oneshot::Sender<()>,
    release: tokio::sync::oneshot::Receiver<()>,
}

fn spawn_trajectory_watcher(
    gcx: Arc<GlobalContext>,
    mut ready: Option<tokio::sync::oneshot::Sender<Result<Vec<PathBuf>, String>>>,
    #[cfg(test)] registration_gate: Option<TrajectoryWatcherRegistrationGate>,
) -> tokio::task::JoinHandle<()> {
    let gcx_weak = Arc::downgrade(&gcx);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<TrajectoryWatcherMessage>(2048);
    let dropped_events = Arc::new(AtomicUsize::new(0));

    tokio::spawn(async move {
        let trajectories_dirs = get_all_trajectories_dirs_from_weak(&gcx_weak).await;
        let task_roots = get_all_task_roots_from_weak(&gcx_weak).await;
        if trajectories_dirs.is_empty() && task_roots.is_empty() {
            warn!("No trajectories directories found, trajectory watcher not started");
            if let Some(ready) = ready.take() {
                let _ = ready.send(Err("No trajectories directories found".to_string()));
            }
            return;
        }

        for dir in &trajectories_dirs {
            if !is_real_dir(dir).await {
                warn!("Skipping non-real trajectories dir {:?} for watcher", dir);
            }
        }
        for dir in &task_roots {
            if !is_real_dir(dir).await {
                warn!("Skipping non-real tasks dir {:?} for watcher", dir);
            }
        }
        let watched_roots = futures::future::join_all(
            trajectories_dirs
                .iter()
                .chain(task_roots.iter())
                .map(fs::canonicalize),
        )
        .await
        .into_iter()
        .filter_map(Result::ok)
        .map(|root| dunce::simplified(&root).to_path_buf())
        .collect::<Vec<_>>();
        let recovery_roots = watched_roots.clone();
        let _self_write_cleanup = TrajectorySelfWriteScope {
            watched_roots,
        };

        let tx_clone = tx.clone();
        let dropped_events_for_callback = dropped_events.clone();
        let task_roots_for_callback = task_roots.clone();
        let event_callback = move |res: Result<notify::Event, notify::Error>| {
            if let Ok(event) = res {
                let dominated = matches!(
                    event.kind,
                    notify::EventKind::Create(_)
                        | notify::EventKind::Modify(_)
                        | notify::EventKind::Remove(_)
                );
                if !dominated {
                    return;
                }
                let is_remove = matches!(event.kind, notify::EventKind::Remove(_));
                for path in event.paths {
                    if let Some(message) = trajectory_watcher_message_for_path(
                        path,
                        is_remove,
                        &task_roots_for_callback,
                    ) {
                        if let Err(tokio::sync::mpsc::error::TrySendError::Full(_)) =
                            tx_clone.try_send(message)
                        {
                            let dropped =
                                dropped_events_for_callback.fetch_add(1, Ordering::Relaxed) + 1;
                            if dropped % 100 == 0 {
                                warn!("Trajectory watcher dropped {} events because the channel is full", dropped);
                            }
                        }
                    }
                }
            }
        };

        // Buddy conversations deliberately stay outside this watcher because active Buddy sessions
        // do not apply external trajectory reloads.
        let watcher = match RecommendedWatcher::new(event_callback, Config::default()) {
            Ok(w) => w,
            Err(e) => {
                warn!("Failed to create trajectory watcher: {}", e);
                if let Some(ready) = ready.take() {
                    let _ = ready.send(Err(format!("Failed to create trajectory watcher: {e}")));
                }
                return;
            }
        };

        let _watcher = Arc::new(std::sync::Mutex::new(watcher));
        #[cfg(test)]
        if let Some(gate) = registration_gate {
            let _ = gate.entered.send(());
            let _ = gate.release.await;
        }
        let mut registered_roots = Vec::new();
        let mut registration_errors = Vec::new();
        {
            let mut w = _watcher.lock().unwrap();
            for dir in &trajectories_dirs {
                if let Err(e) = w.watch(dir, RecursiveMode::Recursive) {
                    warn!("Failed to watch trajectories dir {:?}: {}", dir, e);
                    registration_errors
                        .push(format!("Failed to watch trajectories dir {dir:?}: {e}"));
                } else {
                    registered_roots.push(dir.clone());
                }
            }
            for dir in &task_roots {
                if let Err(e) = w.watch(dir, RecursiveMode::Recursive) {
                    warn!("Failed to watch tasks dir {:?}: {}", dir, e);
                    registration_errors.push(format!("Failed to watch tasks dir {dir:?}: {e}"));
                } else {
                    registered_roots.push(dir.clone());
                }
            }
        }
        if let Some(ready) = ready.take() {
            let result = if registration_errors.is_empty() {
                Ok(registered_roots)
            } else {
                Err(registration_errors.join("; "))
            };
            let _ = ready.send(result);
        }
        info!(
            "Trajectory watcher started for {} trajectory directories and {} task roots",
            trajectories_dirs.len(),
            task_roots.len()
        );

        let mut pending = TrajectoryPendingMap::new();
        let mut pending_scans: std::collections::HashMap<PathBuf, Instant> =
            std::collections::HashMap::new();
        let debounce = timeouts().watcher_debounce;

        loop {
            let timeout = if pending.is_empty() && pending_scans.is_empty() {
                timeouts().watcher_idle
            } else {
                timeouts().watcher_poll
            };

            tokio::select! {
                msg = rx.recv() => {
                    match msg {
                        Some(TrajectoryWatcherMessage::Trajectory {
                            chat_id,
                            is_remove,
                            source,
                            path,
                        }) => {
                            insert_pending_trajectory_change(
                                &mut pending,
                                chat_id,
                                is_remove,
                                source,
                                path,
                                Instant::now(),
                            );
                        }
                        Some(TrajectoryWatcherMessage::ScanPath(path)) => {
                            pending_scans.insert(path, Instant::now());
                        }
                        None => break,
                    }
                }
                _ = tokio::time::sleep(timeout) => {
                    if gcx_weak.upgrade().is_none() {
                        break;
                    }
                }
            }

            schedule_trajectory_watcher_overflow_recovery(
                &mut pending_scans,
                &dropped_events,
                &recovery_roots,
            );

            let now = Instant::now();
            let ready_scans: Vec<PathBuf> = pending_scans
                .iter()
                .filter(|(_, t)| now.duration_since(**t) >= debounce)
                .map(|(path, _)| path.clone())
                .collect();

            for path in ready_scans {
                pending_scans.remove(&path);
                for (chat_id, source, trajectory_path) in
                    collect_trajectory_sources_under_path(&path, &task_roots).await
                {
                    insert_pending_trajectory_change(
                        &mut pending,
                        chat_id,
                        false,
                        source,
                        trajectory_path,
                        Instant::now(),
                    );
                }
            }

            let now = Instant::now();
            let ready: Vec<_> = pending
                .iter()
                .filter(|(_, change)| now.duration_since(change.last_event_at) >= debounce)
                .map(|(key, change)| (key.clone(), change.is_remove, change.path.clone()))
                .collect();

            for ((chat_id, source), is_remove, path) in ready {
                pending.remove(&(chat_id.clone(), source.clone()));
                if let Some(gcx) = gcx_weak.upgrade() {
                    process_trajectory_watcher_change(gcx, chat_id, is_remove, source, path).await;
                }
            }
        }
    })
}

pub fn validate_trajectory_id(id: &str) -> Result<(), ScratchError> {
    if id.is_empty() || id.len() > 128 {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Invalid trajectory id".to_string(),
        ));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Invalid trajectory id".to_string(),
        ));
    }
    Ok(())
}

async fn atomic_write_json(
    path: &PathBuf,
    data: &impl Serialize,
    source: TrajectorySourceIdentity,
) -> Result<(), String> {
    let tmp_path = unique_trajectory_tmp_path(path);
    let json_result = serde_json::to_string(data).map_err(|e| e.to_string());
    atomic_write_json_with_tmp_path(path, &tmp_path, json_result, None, Some(source)).await
}

async fn write_title_if_unchanged(
    path: &PathBuf,
    original: &[u8],
    data: &TrajectoryData,
    source: TrajectorySourceIdentity,
) -> Result<bool, String> {
    super::trajectory_archive::with_trajectory_mutation_lock(async {
        let current = fs::read(path).await.map_err(|e| e.to_string())?;
        if current != original {
            return Ok(false);
        }
        atomic_write_json(path, data, source).await?;
        Ok(true)
    })
    .await
}

fn is_placeholder_title(title: &str) -> bool {
    let normalized = title.trim().to_lowercase();
    normalized.is_empty() || normalized == "new chat" || normalized == "untitled"
}

fn is_placeholder_task_name(name: &str) -> bool {
    let normalized = name.trim().to_lowercase();
    normalized.is_empty() || normalized == "new task" || normalized == "untitled"
}

fn count_user_messages(messages: &[serde_json::Value]) -> usize {
    messages
        .iter()
        .filter(|msg| {
            msg.get("role")
                .and_then(|r| r.as_str())
                .map(|r| r == "user")
                .unwrap_or(false)
        })
        .count()
}

fn json_message_is_ui_only(msg: &serde_json::Value) -> bool {
    msg.get("_ui_only").and_then(|v| v.as_bool()) == Some(true)
        || msg
            .get("extra")
            .and_then(|extra| extra.get("_ui_only"))
            .and_then(|v| v.as_bool())
            == Some(true)
}

fn extract_first_user_message(messages: &[serde_json::Value]) -> Option<String> {
    for msg in messages {
        if json_message_is_ui_only(msg) {
            continue;
        }
        if msg.get("role").and_then(|r| r.as_str()) != Some("user") {
            continue;
        }
        if let Some(content) = msg
            .get("content")
            .and_then(extract_text_with_image_placeholders_from_json)
        {
            let trimmed = content.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.chars().take(200).collect());
            }
        }
    }
    None
}

pub fn extract_text_with_image_placeholders_from_json(
    content_value: &serde_json::Value,
) -> Option<String> {
    if let Some(content) = content_value.as_str() {
        return Some(content.to_string());
    }
    if let Some(content_arr) = content_value.as_array() {
        let parts: Vec<String> = content_arr
            .iter()
            .filter_map(|item| {
                if item.get("type").and_then(|t| t.as_str()) == Some("image_url") {
                    return Some("[image]".to_string());
                }
                if let Some(m_type) = item.get("m_type").and_then(|t| t.as_str()) {
                    if m_type.starts_with("image/") {
                        return Some("[image]".to_string());
                    }
                }
                item.get("text")
                    .and_then(|t| t.as_str())
                    .or_else(|| item.get("m_content").and_then(|t| t.as_str()))
                    .map(|s| s.to_string())
            })
            .collect();
        if !parts.is_empty() {
            return Some(parts.join("\n\n"));
        }
    }
    None
}

fn build_title_generation_context(messages: &[serde_json::Value]) -> String {
    let mut context = String::new();
    let max_messages = 6;
    let max_chars_per_message = 500;
    let mut included_count = 0;

    for msg in messages.iter() {
        if included_count >= max_messages {
            break;
        }
        let role = msg
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("unknown");
        if json_message_is_ui_only(msg) {
            continue;
        }
        if role == "error" {
            continue;
        }
        if role == refact_chat_history::trajectory_ops::COMPRESSION_REPORT_ROLE
            || matches!(
                role,
                "system" | "tool" | "context_file" | "cd_instruction" | "plan" | "goal" | "event"
            )
        {
            continue;
        }
        let content_text = match msg
            .get("content")
            .and_then(extract_text_with_image_placeholders_from_json)
        {
            Some(text) => text,
            None => continue,
        };
        let truncated: String = content_text.chars().take(max_chars_per_message).collect();
        if !truncated.trim().is_empty() {
            context.push_str(&format!("{}: {}\n\n", role, truncated));
            included_count += 1;
        }
    }
    context
}

fn truncate_text_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    if max_chars <= 3 {
        return text.chars().take(max_chars).collect();
    }
    text.chars().take(max_chars - 3).collect::<String>() + "..."
}

fn clean_generated_title(raw_title: &str) -> String {
    let cleaned = raw_title
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_matches('`')
        .trim_matches('*')
        .replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    truncate_text_chars(&cleaned, 60)
}

pub(crate) fn trajectory_meta_title(title: &str) -> String {
    let cleaned = title.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_text_chars(&cleaned, TRAJECTORY_META_TITLE_MAX_CHARS)
}

pub(crate) fn task_context_from_task_meta(
    task_meta: Option<&super::types::TaskMeta>,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    task_meta
        .map(|meta| {
            (
                Some(meta.task_id.clone()),
                Some(meta.role.clone()),
                meta.agent_id.clone(),
                meta.card_id.clone(),
            )
        })
        .unwrap_or((None, None, None, None))
}

async fn generate_title_llm(
    gcx: Arc<GlobalContext>,
    messages: &[serde_json::Value],
    parent_chat_id: Option<&str>,
) -> Option<String> {
    let context = build_title_generation_context(messages);
    if context.trim().is_empty() {
        return None;
    }

    let subagent_config =
        match get_subagent_config(gcx.clone(), TITLE_GENERATION_SUBAGENT_ID, None).await {
            Some(config) => config,
            None => {
                warn!(
                    "subagent config '{}' not found",
                    TITLE_GENERATION_SUBAGENT_ID
                );
                return None;
            }
        };

    let title_prompt = match subagent_config.messages.user_template.as_ref() {
        Some(prompt) => prompt,
        None => {
            warn!(
                "messages.user_template not defined for subagent '{}'",
                TITLE_GENERATION_SUBAGENT_ID
            );
            return None;
        }
    };

    let prompt = format!("Chat conversation:\n{}\n\n{}", context, title_prompt);
    let chat_messages = vec![ChatMessage::new("user".to_string(), prompt)];

    match run_subchat_once(
        gcx,
        TITLE_GENERATION_SUBAGENT_ID,
        chat_messages,
        crate::subchat::TraceParent::from_parts(parent_chat_id, None),
    )
    .await
    {
        Ok(result) => {
            if let Some(last_msg) = result.messages.last() {
                let raw_title = last_msg.content.content_text_only();
                let cleaned = clean_generated_title(&raw_title);
                if !cleaned.is_empty() && cleaned.to_lowercase() != "new chat" {
                    info!("Generated title: {}", cleaned);
                    return Some(cleaned);
                }
            }
            None
        }
        Err(e) => {
            warn!("Title generation failed: {}", e);
            None
        }
    }
}

fn spawn_title_generation_task(
    gcx: Arc<GlobalContext>,
    id: String,
    messages: Vec<serde_json::Value>,
    file_path: PathBuf,
    source: TrajectorySourceIdentity,
) {
    tokio::spawn(run_title_generation_task(
        gcx, id, messages, file_path, source,
    ));
}

async fn run_title_generation_task(
    gcx: Arc<GlobalContext>,
    id: String,
    messages: Vec<serde_json::Value>,
    file_path: PathBuf,
    source: TrajectorySourceIdentity,
) {
    let app = AppState::from_gcx(gcx.clone()).await;
    let generated_title = match tokio::time::timeout(
        TITLE_GENERATION_LLM_TIMEOUT,
        generate_title_llm(gcx.clone(), &messages, Some(id.as_str())),
    )
    .await
    {
        Ok(title) => title,
        Err(_) => {
            warn!("Title generation timed out for {}", id);
            None
        }
    };
    let title = match generated_title {
        Some(t) => t,
        None => match extract_first_user_message(&messages) {
            Some(first_msg) => {
                let truncated: String = first_msg.chars().take(60).collect();
                if truncated.len() < first_msg.len() {
                    format!("{}...", truncated.trim_end())
                } else {
                    truncated
                }
            }
            None => return,
        },
    };
    let sessions = app.chat.sessions.clone();
    if !title_generation_backing_file_matches(&file_path, &id, &source).await {
        return;
    }
    let maybe_session_arc = {
        let sessions_read = sessions.read().await;
        sessions_read.get(&id).cloned()
    };
    if let Some(session_arc) = maybe_session_arc {
        let mut session = session_arc.lock().await;
        if TrajectorySourceIdentity::from_session(&session) != source {
            drop(session);
        } else {
            if session.thread.is_title_generated {
                info!("Title already generated for {}, skipping", id);
                return;
            }
            let suppressed_trajectory_events_tx =
                if matches!(source, TrajectorySourceIdentity::Buddy) {
                    session.trajectory_events_tx.take()
                } else {
                    None
                };
            session.set_title(title.clone(), true);
            if let Some(tx) = suppressed_trajectory_events_tx {
                session.trajectory_events_tx = Some(tx);
            }
            drop(session);
            maybe_save_trajectory_with_intent(
                app.clone(),
                session_arc,
                TrajectoryCommitIntent::Checkpoint,
            )
            .await;
            info!("Updated session {} with generated title: {}", id, title);
            return;
        }
    }
    let content = match fs::read_to_string(&file_path).await {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to read trajectory for title update: {}", e);
            return;
        }
    };
    let mut data: TrajectoryData = match serde_json::from_str(&content) {
        Ok(d) => d,
        Err(e) => {
            warn!("Failed to parse trajectory for title update: {}", e);
            return;
        }
    };
    if data.id != id {
        warn!(
            "Skipping title update for {}: JSON id mismatch, found {}",
            file_path.display(),
            data.id
        );
        return;
    }
    let actual_source = match TrajectorySourceIdentity::from_extra(&data.extra) {
        Ok(actual_source) => actual_source,
        Err(e) => {
            warn!(
                "Skipping title update for {}: trajectory source is invalid before write: {}",
                file_path.display(),
                e
            );
            return;
        }
    };
    if actual_source != source {
        warn!(
            "Skipping title update for {}: trajectory source changed before write",
            file_path.display()
        );
        return;
    }
    let already_generated = data
        .extra
        .get("isTitleGenerated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if already_generated {
        info!("Title already generated for {}, skipping", id);
        return;
    }
    let updated_at = chrono::Utc::now().to_rfc3339();
    data.title = title.clone();
    data.updated_at = updated_at.clone();
    data.extra
        .insert("isTitleGenerated".to_string(), serde_json::json!(true));
    match write_title_if_unchanged(&file_path, content.as_bytes(), &data, source.clone()).await {
        Ok(true) => {}
        Ok(false) => return,
        Err(e) => {
            warn!("Failed to write trajectory with generated title: {}", e);
            return;
        }
    }
    if let Some(dir) = index_dir_for_trajectory_file(gcx.clone(), &file_path).await {
        let value = match serde_json::to_value(&data) {
            Ok(value) => value,
            Err(e) => {
                warn!(
                    "Failed to serialize trajectory title update for index: {}",
                    e
                );
                return;
            }
        };
        if let Err(e) =
            trajectory_index::upsert_trajectory_index_entry_from_owned_value_with_rollout(
                &app.chat.trajectory_index_coordinator,
                &dir,
                &file_path,
                value,
                Some(source.clone()),
            )
            .await
        {
            warn!(
                "Failed to update trajectory index for generated title: {}",
                e
            );
        }
    }
    info!("Updated trajectory {} with generated title: {}", id, title);
    if !source.emits_generic_event() {
        return;
    }
    let content = match fs::read_to_string(&file_path).await {
        Ok(content) => content,
        Err(e) => {
            warn!("Failed to read updated trajectory for title SSE: {}", e);
            return;
        }
    };
    let updated_data: TrajectoryData = match serde_json::from_str(&content) {
        Ok(data) => data,
        Err(e) => {
            warn!("Failed to parse updated trajectory for title SSE: {}", e);
            return;
        }
    };
    let task_roots = get_all_task_roots(gcx.clone()).await;
    let mut meta = trajectory_data_to_meta_validated(app.clone(), &updated_data).await;
    apply_task_trajectory_context(&file_path, &task_roots, &mut meta);
    if meta.root_chat_id.is_none() {
        meta.root_chat_id = Some(id.clone());
    }
    let (session_state, session_error, session_worktree) =
        get_session_runtime_for_trajectory_source(&sessions, &id, &meta.source).await;
    let event = updated_trajectory_event_from_meta_with_worktree(
        meta,
        Some(true),
        session_state,
        session_error,
        session_worktree,
    );
    let tx = &app.chat.trajectory_events_tx;
    {
        let _ = tx.send(event);
    }
}

fn spawn_task_name_generation_task(
    gcx: Arc<GlobalContext>,
    task_id: String,
    messages: Vec<serde_json::Value>,
) {
    tokio::spawn(async move {
        let task_meta = match crate::tasks::storage::load_task_meta(gcx.clone(), &task_id).await {
            Ok(meta) => meta,
            Err(e) => {
                warn!("Failed to load task meta for name generation: {}", e);
                return;
            }
        };

        if task_meta.is_name_generated {
            return;
        }

        if !is_placeholder_task_name(&task_meta.name) {
            return;
        }

        let generated_name = generate_title_llm(gcx.clone(), &messages, None).await;
        let name = match generated_name {
            Some(n) => n,
            None => match extract_first_user_message(&messages) {
                Some(first_msg) => {
                    let truncated: String = first_msg.chars().take(60).collect();
                    if truncated.len() < first_msg.len() {
                        format!("{}...", truncated.trim_end())
                    } else {
                        truncated
                    }
                }
                None => return,
            },
        };

        match crate::tasks::storage::update_task_name(gcx.clone(), &task_id, &name).await {
            Ok(_) => {
                info!("Updated task {} with generated name: {}", task_id, name);
            }
            Err(e) => {
                warn!("Failed to update task name: {}", e);
            }
        }
    });
}

pub(crate) fn calculate_line_changes_from_messages(messages: &[serde_json::Value]) -> (i64, i64) {
    let mut total_added: i64 = 0;
    let mut total_removed: i64 = 0;

    for msg in messages {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role != "diff" {
            continue;
        }

        let content = match msg.get("content") {
            Some(serde_json::Value::String(s)) => s.as_str(),
            _ => continue,
        };

        if let Ok(chunks) = serde_json::from_str::<Vec<serde_json::Value>>(content) {
            for chunk in chunks {
                if let Some(lines_add) = chunk.get("lines_add").and_then(|v| v.as_str()) {
                    if !lines_add.is_empty() {
                        total_added += lines_add.lines().count() as i64;
                    }
                }
                if let Some(lines_remove) = chunk.get("lines_remove").and_then(|v| v.as_str()) {
                    if !lines_remove.is_empty() {
                        total_removed += lines_remove.lines().count() as i64;
                    }
                }
            }
        }
    }

    (total_added, total_removed)
}

pub(crate) fn calculate_task_progress_from_messages(
    messages: &[serde_json::Value],
) -> (i32, i32, i32) {
    // Build a set of successful tool call IDs (tool messages without tool_failed=true)
    let mut successful_tool_ids: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for msg in messages {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role != "tool" {
            continue;
        }
        let tool_failed = msg
            .get("tool_failed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !tool_failed {
            if let Some(tool_call_id) = msg.get("tool_call_id").and_then(|v| v.as_str()) {
                successful_tool_ids.insert(tool_call_id.to_string());
            }
        }
    }

    // Find the last successful tasks_set tool call (iterate in reverse)
    for msg in messages.iter().rev() {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role != "assistant" {
            continue;
        }

        let tool_calls = match msg.get("tool_calls").and_then(|v| v.as_array()) {
            Some(tc) => tc,
            None => continue,
        };

        // Iterate tool_calls in reverse to find the last tasks_set
        for tc in tool_calls.iter().rev() {
            let function = match tc.get("function") {
                Some(f) => f,
                None => continue,
            };

            let name = function.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if name != "tasks_set" {
                continue;
            }

            let tc_id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("");
            if tc_id.is_empty() || !successful_tool_ids.contains(tc_id) {
                continue;
            }

            // Parse the arguments
            let args_str = function
                .get("arguments")
                .and_then(|a| a.as_str())
                .unwrap_or("");
            if let Ok(args) = serde_json::from_str::<serde_json::Value>(args_str) {
                if let Some(tasks) = args.get("tasks").and_then(|t| t.as_array()) {
                    let mut total = 0i32;
                    let mut done = 0i32;
                    let mut failed = 0i32;

                    for task in tasks {
                        total += 1;
                        let status = task.get("status").and_then(|s| s.as_str()).unwrap_or("");
                        match status.to_lowercase().as_str() {
                            "completed" | "done" | "complete" => done += 1,
                            "failed" | "error" => failed += 1,
                            _ => {}
                        }
                    }

                    return (total, done, failed);
                }
            }
        }
    }

    (0, 0, 0)
}

#[derive(Default)]
pub(crate) struct TokenTotals {
    pub(crate) prompt_tokens: u64,
    pub(crate) completion_tokens: u64,
    pub(crate) total_tokens: u64,
    pub(crate) cache_read_tokens: u64,
    pub(crate) cache_creation_tokens: u64,
    pub(crate) cost_usd: Option<f64>,
}

pub(crate) fn calculate_token_totals_from_messages(messages: &[serde_json::Value]) -> TokenTotals {
    let mut prompt_tokens: u64 = 0;
    let mut completion_tokens: u64 = 0;
    let mut total_tokens: u64 = 0;
    let mut cache_read_tokens: u64 = 0;
    let mut cache_creation_tokens: u64 = 0;
    let mut cost_usd: Option<f64> = None;

    for msg in messages {
        let usage = match msg.get("usage") {
            Some(u) if !u.is_null() => u,
            _ => continue,
        };
        if let Some(v) = usage.get("prompt_tokens").and_then(|v| v.as_u64()) {
            prompt_tokens += v;
        }
        if let Some(v) = usage.get("completion_tokens").and_then(|v| v.as_u64()) {
            completion_tokens += v;
        }
        if let Some(v) = usage.get("total_tokens").and_then(|v| v.as_u64()) {
            total_tokens += v;
        }
        for key in &["cache_read_input_tokens", "cache_read_tokens"] {
            if let Some(v) = usage.get(key).and_then(|v| v.as_u64()) {
                cache_read_tokens += v;
                break;
            }
        }
        for key in &["cache_creation_input_tokens", "cache_creation_tokens"] {
            if let Some(v) = usage.get(key).and_then(|v| v.as_u64()) {
                cache_creation_tokens += v;
                break;
            }
        }
        if let Some(total) = usage
            .get("metering_usd")
            .and_then(|m| m.get("total_usd"))
            .and_then(|v| v.as_f64())
        {
            *cost_usd.get_or_insert(0.0) += total;
        }
    }

    TokenTotals {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cache_read_tokens,
        cache_creation_tokens,
        cost_usd,
    }
}

#[derive(Default)]
pub(crate) struct TrajectoryMetrics {
    pub(crate) line_changes: (i64, i64),
    pub(crate) task_progress: (i32, i32, i32),
    pub(crate) token_totals: TokenTotals,
}

fn task_progress_from_tasks_set_arguments(arguments: &str) -> Option<(i32, i32, i32)> {
    let args = serde_json::from_str::<serde_json::Value>(arguments).ok()?;
    let tasks = args.get("tasks")?.as_array()?;
    let mut total = 0i32;
    let mut done = 0i32;
    let mut failed = 0i32;
    for task in tasks {
        total += 1;
        match task
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_lowercase()
            .as_str()
        {
            "completed" | "done" | "complete" => done += 1,
            "failed" | "error" => failed += 1,
            _ => {}
        }
    }
    Some((total, done, failed))
}

fn accumulate_line_changes(content: &str, total_added: &mut i64, total_removed: &mut i64) {
    let Ok(chunks) = serde_json::from_str::<Vec<serde_json::Value>>(content) else {
        return;
    };
    for chunk in chunks {
        if let Some(lines_add) = chunk.get("lines_add").and_then(|v| v.as_str()) {
            if !lines_add.is_empty() {
                *total_added += lines_add.lines().count() as i64;
            }
        }
        if let Some(lines_remove) = chunk.get("lines_remove").and_then(|v| v.as_str()) {
            if !lines_remove.is_empty() {
                *total_removed += lines_remove.lines().count() as i64;
            }
        }
    }
}

/// Every per-save trajectory metric in one forward pass. Task progress keeps its "last successful
/// `tasks_set` carrying a task list" semantics: candidates are collected in message order and
/// resolved in reverse, the exact order the previous reverse scan visited them in.
fn scan_trajectory_metrics<'a, I>(messages: I) -> TrajectoryMetrics
where
    I: IntoIterator<Item = &'a ChatMessage>,
{
    let mut total_added: i64 = 0;
    let mut total_removed: i64 = 0;
    let mut token_totals = TokenTotals::default();
    let mut successful_tool_ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut tasks_set_calls: Vec<(&str, &str)> = Vec::new();

    for msg in messages {
        if let Some(usage) = msg.usage.as_ref() {
            token_totals.prompt_tokens += usage.prompt_tokens as u64;
            token_totals.completion_tokens += usage.completion_tokens as u64;
            token_totals.total_tokens += usage.total_tokens as u64;
            if let Some(cache_read_tokens) = usage.cache_read_tokens {
                token_totals.cache_read_tokens += cache_read_tokens as u64;
            }
            if let Some(cache_creation_tokens) = usage.cache_creation_tokens {
                token_totals.cache_creation_tokens += cache_creation_tokens as u64;
            }
            if let Some(metering) = usage.metering_usd.as_ref() {
                *token_totals.cost_usd.get_or_insert(0.0) += metering.total_usd;
            }
        }

        match msg.role.as_str() {
            "diff" => {
                if let ChatContent::SimpleText(content) = &msg.content {
                    accumulate_line_changes(content, &mut total_added, &mut total_removed);
                }
            }
            "tool" => {
                if !msg.tool_failed.unwrap_or(false) && !msg.tool_call_id.is_empty() {
                    successful_tool_ids.insert(msg.tool_call_id.as_str());
                }
            }
            "assistant" => {
                for tool_call in msg.tool_calls.iter().flatten() {
                    if tool_call.function.name == "tasks_set" && !tool_call.id.is_empty() {
                        tasks_set_calls
                            .push((tool_call.id.as_str(), tool_call.function.arguments.as_str()));
                    }
                }
            }
            _ => {}
        }
    }

    let task_progress = tasks_set_calls
        .iter()
        .rev()
        .filter(|(id, _)| successful_tool_ids.contains(id))
        .find_map(|(_, arguments)| task_progress_from_tasks_set_arguments(arguments))
        .unwrap_or_default();

    TrajectoryMetrics {
        line_changes: (total_added, total_removed),
        task_progress,
        token_totals,
    }
}

fn trajectory_data_to_meta(data: &TrajectoryData) -> TrajectoryMeta {
    let task_meta_json = data.extra.get("task_meta");
    let task_id = task_meta_json
        .and_then(|v| v.get("task_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let task_role = task_meta_json
        .and_then(|v| v.get("role"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let agent_id = task_meta_json
        .and_then(|v| v.get("agent_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let card_id = task_meta_json
        .and_then(|v| v.get("card_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let parent_id = data
        .extra
        .get("parent_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let link_type = data
        .extra
        .get("link_type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let root_chat_id = data
        .extra
        .get("root_chat_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let worktree = None;

    let (total_lines_added, total_lines_removed) =
        calculate_line_changes_from_messages(&data.messages);
    let (tasks_total, tasks_done, tasks_failed) =
        calculate_task_progress_from_messages(&data.messages);
    let token_totals = calculate_token_totals_from_messages(&data.messages);

    TrajectoryMeta {
        id: data.id.clone(),
        title: trajectory_meta_title(&data.title),
        created_at: data.created_at.clone(),
        updated_at: data.updated_at.clone(),
        model: data.model.clone(),
        mode: data.mode.clone(),
        message_count: data.messages.len(),
        parent_id,
        link_type,
        task_id,
        task_role,
        agent_id,
        card_id,
        session_state: None,
        root_chat_id,
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
        source: TrajectorySourceIdentity::from_extra(&data.extra).unwrap_or_default(),
    }
}

#[derive(Debug, Deserialize)]
pub struct TrajectoriesListQuery {
    pub limit: Option<usize>,
    pub cursor: Option<String>,
    pub displayable_only: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
pub struct TrajectoriesAllQuery {
    #[serde(default)]
    pub displayable_only: bool,
}

#[derive(Debug, Serialize)]
pub struct PaginatedTrajectories {
    pub items: Vec<TrajectoryMeta>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
    pub total_count: usize,
}

fn encode_cursor(updated_at: &str, id: &str) -> String {
    use base64::Engine;
    let cursor_data = format!("{}|{}", updated_at, id);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(cursor_data.as_bytes())
}

fn decode_cursor(cursor: &str) -> Option<(String, String)> {
    use base64::Engine;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cursor)
        .ok()?;
    let cursor_str = String::from_utf8(decoded).ok()?;
    let parts: Vec<&str> = cursor_str.splitn(2, '|').collect();
    if parts.len() == 2 {
        Some((parts[0].to_string(), parts[1].to_string()))
    } else {
        None
    }
}

async fn trajectory_data_to_meta_validated(app: AppState, data: &TrajectoryData) -> TrajectoryMeta {
    let mut meta = trajectory_data_to_meta(data);
    if let Some(worktree) = trajectory_worktree_from_extra(&data.extra) {
        meta.worktree = validate_loaded_worktree_strict(app, &data.id, worktree).await;
    }
    meta
}

fn loaded_trajectory_to_meta(loaded: &LoadedTrajectory, task_roots: &[PathBuf]) -> TrajectoryMeta {
    let effective_root = loaded
        .thread
        .root_chat_id
        .clone()
        .unwrap_or_else(|| loaded.thread.id.clone());
    let source = loaded_trajectory_source(loaded, task_roots);
    let (task_id, task_role, agent_id, card_id) =
        task_context_from_task_meta(loaded.thread.task_meta.as_ref());
    let metrics = scan_trajectory_metrics(&loaded.messages);
    let (total_lines_added, total_lines_removed) = metrics.line_changes;
    let (tasks_total, tasks_done, tasks_failed) = metrics.task_progress;
    let token_totals = metrics.token_totals;

    let mut meta = TrajectoryMeta {
        id: loaded.thread.id.clone(),
        title: trajectory_meta_title(&loaded.thread.title),
        created_at: loaded.created_at.clone(),
        updated_at: loaded.updated_at.clone(),
        model: loaded.thread.model.clone(),
        mode: loaded.thread.mode.clone(),
        message_count: loaded.messages.len(),
        parent_id: loaded.thread.parent_id.clone(),
        link_type: loaded.thread.link_type.clone(),
        task_id,
        task_role,
        agent_id,
        card_id,
        session_state: None,
        root_chat_id: Some(effective_root),
        worktree: loaded.thread.worktree.clone(),
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
        source,
    };
    apply_task_trajectory_context(&loaded.source_path, task_roots, &mut meta);
    meta
}

fn deleted_trajectory_event(id: String) -> TrajectoryEvent {
    TrajectoryEvent {
        event_type: "deleted".to_string(),
        id,
        updated_at: None,
        title: None,
        is_title_generated: None,
        session_state: None,
        error: None,
        message_count: None,
        parent_id: None,
        link_type: None,
        root_chat_id: None,
        task_id: None,
        task_role: None,
        agent_id: None,
        card_id: None,
        model: None,
        mode: None,
        worktree: None,
        total_lines_added: None,
        total_lines_removed: None,
        tasks_total: None,
        tasks_done: None,
        tasks_failed: None,
        total_prompt_tokens: None,
        total_completion_tokens: None,
        total_tokens: None,
        total_cache_read_tokens: None,
        total_cache_creation_tokens: None,
        total_cost_usd: None,
    }
}

fn updated_trajectory_event_from_meta(
    meta: TrajectoryMeta,
    is_title_generated: Option<bool>,
    session_state: String,
    session_error: Option<String>,
) -> TrajectoryEvent {
    updated_trajectory_event_from_meta_with_worktree(
        meta,
        is_title_generated,
        session_state,
        session_error,
        None,
    )
}

fn updated_trajectory_event_from_meta_with_worktree(
    meta: TrajectoryMeta,
    is_title_generated: Option<bool>,
    session_state: String,
    session_error: Option<String>,
    session_worktree: Option<WorktreeMeta>,
) -> TrajectoryEvent {
    TrajectoryEvent {
        event_type: "updated".to_string(),
        id: meta.id,
        updated_at: Some(meta.updated_at),
        title: Some(meta.title),
        is_title_generated,
        session_state: Some(session_state),
        error: session_error,
        message_count: Some(meta.message_count),
        parent_id: meta.parent_id,
        link_type: meta.link_type,
        root_chat_id: meta.root_chat_id,
        task_id: meta.task_id,
        task_role: meta.task_role,
        agent_id: meta.agent_id,
        card_id: meta.card_id,
        model: Some(meta.model),
        mode: Some(meta.mode),
        worktree: meta.worktree.or(session_worktree),
        total_lines_added: Some(meta.total_lines_added),
        total_lines_removed: Some(meta.total_lines_removed),
        tasks_total: Some(meta.tasks_total),
        tasks_done: Some(meta.tasks_done),
        tasks_failed: Some(meta.tasks_failed),
        total_prompt_tokens: Some(meta.total_prompt_tokens),
        total_completion_tokens: Some(meta.total_completion_tokens),
        total_tokens: Some(meta.total_tokens),
        total_cache_read_tokens: Some(meta.total_cache_read_tokens),
        total_cache_creation_tokens: Some(meta.total_cache_creation_tokens),
        total_cost_usd: meta.total_cost_usd,
    }
}

fn apply_task_trajectory_context(path: &Path, task_roots: &[PathBuf], meta: &mut TrajectoryMeta) {
    if let Some((task_id, role, agent_id)) = task_trajectory_context_from_path(path, task_roots) {
        meta.source = effective_trajectory_source_for_path(meta.source.clone(), path, task_roots);
        if meta.task_id.is_none() {
            meta.task_id = Some(task_id);
        }
        if meta.task_role.is_none() {
            meta.task_role = Some(role);
        }
        if meta.agent_id.is_none() {
            meta.agent_id = agent_id;
        }
    }
}

fn trajectory_list_candidate_matches_hydrated_data(
    candidate: &TrajectoryListCandidate,
    data: &TrajectoryData,
) -> bool {
    if data.id == candidate.id && trajectory_path_stem_matches_id(&candidate.path, &data.id) {
        return true;
    }
    warn!(
        "Ignoring trajectory {} during hydration: expected id {}, found {}",
        candidate.path.display(),
        candidate.id,
        data.id
    );
    false
}

async fn hydrate_trajectory_list_candidate(
    app: AppState,
    candidate: &TrajectoryListCandidate,
    task_roots: &[PathBuf],
) -> Option<TrajectoryMeta> {
    if let (Some(indexed_len), Some(indexed_modified_ms)) = (
        candidate.indexed_file_len,
        candidate.indexed_file_modified_unix_ms,
    ) {
        let fresh = fs::symlink_metadata(&candidate.path)
            .await
            .ok()
            .and_then(|metadata| {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return None;
                }
                let modified = metadata.modified().ok()?;
                let ms = modified.duration_since(UNIX_EPOCH).ok()?.as_millis();
                let modified_ms = i64::try_from(ms).ok()?;
                Some(metadata.len() == indexed_len && modified_ms == indexed_modified_ms)
            })
            .unwrap_or(false);
        if !fresh {
            return None;
        }
    }
    if let Some(mut meta) = candidate.indexed_meta.clone() {
        if let Some(worktree) = meta.worktree.clone() {
            meta.worktree = validate_loaded_worktree_strict(app.clone(), &meta.id, worktree).await;
        }
        apply_task_trajectory_context(&candidate.path, task_roots, &mut meta);
        return Some(meta);
    }
    let content = fs::read_to_string(&candidate.path).await.ok()?;
    let data = serde_json::from_str::<TrajectoryData>(&content).ok()?;
    if !trajectory_list_candidate_matches_hydrated_data(candidate, &data) {
        return None;
    }
    let mut meta = trajectory_data_to_meta_validated(app, &data).await;
    apply_task_trajectory_context(&candidate.path, task_roots, &mut meta);
    Some(meta)
}

async fn hydrate_trajectory_list_page(
    app: AppState,
    candidates: Vec<TrajectoryListCandidate>,
    limit: usize,
    task_roots: &[PathBuf],
) -> (Vec<TrajectoryMeta>, bool) {
    let mut items = Vec::with_capacity(limit);
    let mut has_more = false;
    for candidate in candidates {
        let Some(meta) =
            hydrate_trajectory_list_candidate(app.clone(), &candidate, task_roots).await
        else {
            continue;
        };
        if items.len() == limit {
            has_more = true;
            break;
        }
        items.push(meta);
    }
    (items, has_more)
}

// Directories the coordinator already indexed and that hold no renderable chat are skipped
// without a listing call, so a displayable-only refresh stops sweeping every task trajectory
// directory. An unknown directory is never skipped, so the first refresh still sees everything.
async fn list_trajectory_entries_for_listing(
    app: &AppState,
    dir: &Path,
    displayable_only: bool,
    caller: trajectory_index::TrajectoryIndexListingCaller,
) -> Result<Option<Arc<Vec<trajectory_index::TrajectoryIndexEntry>>>, String> {
    let coordinator = &app.chat.trajectory_index_coordinator;
    if displayable_only && coordinator.directory_has_displayable_entries(dir) == Some(false) {
        return Ok(None);
    }
    if !is_real_dir(dir).await {
        return Ok(None);
    }
    trajectory_index::list_trajectory_entries_with_rollout_shared_for(
        coordinator,
        dir,
        None,
        caller,
    )
    .await
    .map(Some)
}

async fn collect_trajectory_list_candidates(
    gcx: &Arc<GlobalContext>,
    cursor_filter: Option<&(String, String)>,
    displayable_only: bool,
    caller: trajectory_index::TrajectoryIndexListingCaller,
) -> Vec<TrajectoryListCandidate> {
    let mut candidates = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();
    let app = AppState::from_gcx(gcx.clone()).await;

    for trajectories_dir in list_trajectory_dirs(gcx).await {
        let entries = match list_trajectory_entries_for_listing(
            &app,
            &trajectories_dir,
            displayable_only,
            caller,
        )
        .await
        {
            Ok(Some(entries)) => entries,
            Ok(None) => continue,
            Err(e) => {
                warn!(
                    "Failed to list trajectory index {:?}: {}",
                    trajectories_dir, e
                );
                continue;
            }
        };
        for entry in entries.iter() {
            if matches!(entry.source, trajectory_index::TrajectoryIndexSource::Buddy) {
                continue;
            }
            if displayable_only && !entry.displayable_chat {
                continue;
            }
            if !seen_ids.insert(entry.id.clone()) {
                continue;
            }
            if let Some((cursor_updated_at, cursor_id)) = cursor_filter {
                if !cursor_precedes_item(
                    (entry.updated_at.as_str(), entry.id.as_str()),
                    (cursor_updated_at.as_str(), cursor_id.as_str()),
                ) {
                    continue;
                }
            }
            candidates.push(trajectory_index::list_candidate_from_entry(
                &trajectories_dir,
                entry,
            ));
        }
    }

    candidates
}

async fn is_real_dir(path: &Path) -> bool {
    matches!(fs::symlink_metadata(path).await, Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink())
}

fn is_real_dir_sync(path: &Path) -> bool {
    matches!(std::fs::symlink_metadata(path), Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink())
}

async fn is_real_file(path: &Path) -> bool {
    matches!(fs::symlink_metadata(path).await, Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink())
}

#[cfg(target_os = "macos")]
async fn allowed_real_dir_symlink_target(path: &Path) -> Option<PathBuf> {
    let expected_target = match path.to_str()? {
        "/tmp" => Path::new("/private/tmp"),
        "/var" => Path::new("/private/var"),
        _ => return None,
    };
    let resolved = fs::canonicalize(path).await.ok()?;
    (resolved == expected_target).then_some(resolved)
}

#[cfg(not(target_os = "macos"))]
async fn allowed_real_dir_symlink_target(_path: &Path) -> Option<PathBuf> {
    None
}

async fn ensure_real_dir_tree(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_)
            | Component::RootDir
            | Component::CurDir
            | Component::Normal(_) => {
                current.push(component.as_os_str());
            }
            Component::ParentDir => {
                return Err(format!(
                    "Refusing to create directory with parent component: {}",
                    path.display()
                ));
            }
        }
        if current.as_os_str().is_empty() {
            continue;
        }
        match fs::symlink_metadata(&current).await {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    if let Some(resolved) = allowed_real_dir_symlink_target(&current).await {
                        current = resolved;
                        continue;
                    }
                    return Err(format!(
                        "Refusing to use non-real directory component: {}",
                        current.display()
                    ));
                }
                if !metadata.is_dir() {
                    return Err(format!(
                        "Refusing to use non-real directory component: {}",
                        current.display()
                    ));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match fs::create_dir(&current).await {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => {
                        return Err(format!(
                            "Failed to create directory {}: {}",
                            current.display(),
                            e
                        ))
                    }
                }
                let metadata = fs::symlink_metadata(&current).await.map_err(|e| {
                    format!(
                        "Failed to inspect created directory {}: {}",
                        current.display(),
                        e
                    )
                })?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(format!(
                        "Created directory is not a real directory: {}",
                        current.display()
                    ));
                }
            }
            Err(e) => {
                return Err(format!(
                    "Failed to inspect directory component {}: {}",
                    current.display(),
                    e
                ));
            }
        }
    }
    if is_real_dir(path).await {
        Ok(())
    } else {
        Err(format!(
            "Directory is not a real directory: {}",
            path.display()
        ))
    }
}

async fn canonical_child_path_under_root(root: &Path, child: &Path) -> Result<PathBuf, String> {
    let root_canonical = fs::canonicalize(root)
        .await
        .map_err(|e| format!("Failed to resolve root {}: {}", root.display(), e))?;
    if !is_real_dir(root).await {
        return Err(format!("Root is not a real directory: {}", root.display()));
    }

    if is_real_file(child).await {
        let child_canonical = fs::canonicalize(child)
            .await
            .map_err(|e| format!("Failed to resolve child {}: {}", child.display(), e))?;
        if child_canonical.starts_with(&root_canonical) {
            return Ok(child.to_path_buf());
        }
        return Err(format!(
            "Child {} is outside root {}",
            child.display(),
            root.display()
        ));
    }

    if let Ok(metadata) = fs::symlink_metadata(child).await {
        if metadata.file_type().is_symlink() {
            return Err(format!("Refusing symlink child file: {}", child.display()));
        }
        return Err(format!("Child is not a real file: {}", child.display()));
    }

    let parent = child
        .parent()
        .ok_or_else(|| format!("Child has no parent: {}", child.display()))?;
    let parent_canonical = fs::canonicalize(parent)
        .await
        .map_err(|e| format!("Failed to resolve child parent {}: {}", parent.display(), e))?;
    if parent_canonical.starts_with(&root_canonical) {
        Ok(child.to_path_buf())
    } else {
        Err(format!(
            "Child parent {} is outside root {}",
            parent.display(),
            root.display()
        ))
    }
}

async fn safe_trajectory_file_in_dir(dir: &Path, chat_id: &str) -> Option<PathBuf> {
    validate_trajectory_id(chat_id).ok()?;
    ensure_real_dir_tree(dir).await.ok()?;
    let child = dir.join(format!("{}.json", chat_id));
    if !is_real_file(&child).await {
        return None;
    }
    canonical_child_path_under_root(dir, &child).await.ok()
}

async fn safe_new_trajectory_file_in_dir(dir: &Path, chat_id: &str) -> Result<PathBuf, String> {
    validate_trajectory_id(chat_id).map_err(|e| e.message)?;
    ensure_real_dir_tree(dir).await?;
    canonical_child_path_under_root(dir, &dir.join(format!("{}.json", chat_id))).await
}

fn cursor_precedes_item(item: (&str, &str), cursor: (&str, &str)) -> bool {
    item < cursor
}

pub async fn list_trajectories_page(
    app: AppState,
    limit: usize,
    cursor: Option<String>,
    displayable_only: bool,
) -> Result<PaginatedTrajectories, String> {
    list_trajectories_page_for(
        app,
        limit,
        cursor,
        displayable_only,
        trajectory_index::TrajectoryIndexListingCaller::ChatApi,
    )
    .await
}

pub async fn list_trajectories_page_for(
    app: AppState,
    limit: usize,
    cursor: Option<String>,
    displayable_only: bool,
    caller: trajectory_index::TrajectoryIndexListingCaller,
) -> Result<PaginatedTrajectories, String> {
    let gcx = app.gcx.clone();
    let limit = limit.clamp(1, 200);
    let cursor_filter = match cursor.as_deref() {
        Some(cursor) => {
            Some(decode_cursor(cursor).ok_or_else(|| "Invalid cursor format".to_string())?)
        }
        None => None,
    };

    let task_roots = get_all_task_roots(gcx.clone()).await;
    let mut candidates =
        collect_trajectory_list_candidates(&gcx, cursor_filter.as_ref(), displayable_only, caller)
            .await;
    let total_count = if cursor_filter.is_none() {
        candidates.len()
    } else {
        collect_trajectory_list_candidates(&gcx, None, displayable_only, caller)
            .await
            .len()
    };
    candidates.sort_by(|a, b| match b.updated_at.cmp(&a.updated_at) {
        std::cmp::Ordering::Equal => b.id.cmp(&a.id),
        other => other,
    });

    let (mut items, has_more) =
        hydrate_trajectory_list_page(app.clone(), candidates, limit, &task_roots).await;
    enrich_with_session_state(app, &mut items).await;

    let next_cursor = if has_more {
        items
            .last()
            .map(|last| encode_cursor(&last.updated_at, &last.id))
    } else {
        None
    };

    Ok(PaginatedTrajectories {
        items,
        next_cursor,
        has_more,
        total_count,
    })
}

pub async fn handle_v1_trajectories_list(
    State(app): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<TrajectoriesListQuery>,
) -> Result<Response<Body>, ScratchError> {
    let response = list_trajectories_page(
        app,
        params.limit.unwrap_or(50),
        params.cursor,
        params.displayable_only.unwrap_or(false),
    )
    .await
    .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, e))?;

    let json = serde_json::to_string(&response).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Serialization error: {}", e),
        )
    })?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(json))
        .unwrap())
}

pub async fn list_all_trajectories_meta(
    app: AppState,
    displayable_only: bool,
) -> Result<Vec<TrajectoryMeta>, String> {
    let gcx = app.gcx.clone();
    let mut result: Vec<TrajectoryMeta> = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();
    let task_roots = get_all_task_roots(gcx.clone()).await;

    for trajectories_dir in list_trajectory_dirs(&gcx).await {
        let entries = match list_trajectory_entries_for_listing(
            &app,
            &trajectories_dir,
            displayable_only,
            trajectory_index::TrajectoryIndexListingCaller::AllTrajectoriesMetadata,
        )
        .await
        {
            Ok(Some(entries)) => entries,
            Ok(None) => continue,
            Err(e) => {
                warn!(
                    "Failed to list trajectory index {:?}: {}",
                    trajectories_dir, e
                );
                continue;
            }
        };
        for entry in entries.iter() {
            if matches!(entry.source, trajectory_index::TrajectoryIndexSource::Buddy) {
                continue;
            }
            if displayable_only && !entry.displayable_chat {
                continue;
            }
            if seen_ids.insert(entry.id.clone()) {
                let mut meta = trajectory_index::meta_from_entry(&trajectories_dir, entry);
                if let Some(worktree) = meta.worktree.clone() {
                    meta.worktree =
                        validate_loaded_worktree_strict(app.clone(), &meta.id, worktree).await;
                }
                apply_task_trajectory_context(
                    &trajectory_index::trajectory_file_path_for_entry(&trajectories_dir, entry),
                    &task_roots,
                    &mut meta,
                );
                result.push(meta);
            }
        }
    }

    enrich_with_session_state(app, &mut result).await;
    result.sort_by(|a, b| match b.updated_at.cmp(&a.updated_at) {
        std::cmp::Ordering::Equal => b.id.cmp(&a.id),
        other => other,
    });

    Ok(result)
}

pub async fn handle_v1_trajectories_all(
    State(app): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<TrajectoriesAllQuery>,
) -> Result<Response<Body>, ScratchError> {
    let result = list_all_trajectories_meta(app, params.displayable_only)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&result).unwrap()))
        .unwrap())
}

async fn enrich_with_session_state(app: AppState, trajectories: &mut Vec<TrajectoryMeta>) {
    let session_arcs: Vec<(usize, Arc<AMutex<ChatSession>>)> = {
        let sessions = app.chat.sessions.read().await;
        trajectories
            .iter()
            .enumerate()
            .filter_map(|(idx, traj)| sessions.get(&traj.id).map(|arc| (idx, arc.clone())))
            .collect()
    };

    for (idx, session_arc) in session_arcs {
        let session = session_arc.lock().await;
        if !trajectories[idx].source.matches_session(&session) {
            continue;
        }
        trajectories[idx].session_state = Some(session.runtime.state.to_string());
        if trajectories[idx].worktree.is_none() {
            trajectories[idx].worktree = session.thread.worktree.clone();
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct TrajectoryGetQuery {
    #[serde(default)]
    pub metadata_only: bool,
}

fn trajectory_navigation_metadata(
    json: &serde_json::Value,
) -> Result<serde_json::Value, ScratchError> {
    let object = json.as_object().ok_or_else(|| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Trajectory is not a JSON object".to_string(),
        )
    })?;
    let mut out = serde_json::Map::new();
    for key in [
        "id",
        "title",
        "model",
        "mode",
        "parent_id",
        "link_type",
        "root_chat_id",
    ] {
        if let Some(value) = object.get(key).and_then(|value| value.as_str()) {
            out.insert(key.to_string(), json!(value));
        }
    }
    if let Some(value) = object.get("task_meta").filter(|value| !value.is_null()) {
        let task_meta = serde_json::from_value::<TaskMeta>(value.clone()).map_err(|_| {
            ScratchError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                "Trajectory task_meta is malformed".to_string(),
            )
        })?;
        out.insert("task_meta".to_string(), json!(task_meta));
    }
    Ok(serde_json::Value::Object(out))
}

pub async fn handle_v1_trajectories_get(
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
    axum::extract::Query(params): axum::extract::Query<TrajectoryGetQuery>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    validate_trajectory_id(&id)?;
    let candidate = find_trajectory_or_buddy_file(gcx, &id)
        .await
        .ok_or_else(|| {
            ScratchError::new(StatusCode::NOT_FOUND, "Trajectory not found".to_string())
        })?;
    let content = if params.metadata_only {
        trajectory_navigation_metadata(&candidate.json)?.to_string()
    } else {
        candidate.content
    };
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(content))
        .unwrap())
}
pub async fn handle_v1_trajectory_path(
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    let sessions = app.chat.sessions.clone();
    validate_trajectory_id(&id)?;
    let path = find_trajectory_path_for_active_chat(gcx, &sessions, &id)
        .await
        .ok_or_else(|| {
            ScratchError::new(StatusCode::NOT_FOUND, "Trajectory not found".to_string())
        })?;
    let body = serde_json::json!({ "path": path.to_string_lossy() });
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap())
}

pub async fn handle_v1_trajectories_save(
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    validate_trajectory_id(&id)?;
    let data: TrajectoryData = serde_json::from_slice(&body_bytes)
        .map_err(|e| ScratchError::new(StatusCode::BAD_REQUEST, format!("Invalid JSON: {}", e)))?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut data = data;
    data.updated_at = now.clone();
    if data.created_at.is_empty() {
        data.created_at = now.clone();
    }
    if data.id != id {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "ID mismatch".to_string(),
        ));
    }
    let emits_generic_event = data
        .extra
        .get("buddy_meta")
        .map_or(true, |value| value.is_null());
    let file_path = resolve_trajectory_data_save_path(gcx.clone(), &id, &data)
        .await
        .map_err(|e| {
            let status = if is_trajectory_id_mismatch_error(&e) {
                StatusCode::CONFLICT
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            ScratchError::new(status, e)
        })?;
    ensure_existing_trajectory_file_matches(&file_path, &id)
        .await
        .map_err(|e| ScratchError::new(StatusCode::CONFLICT, e))?;
    let is_new = !is_real_file(&file_path).await;
    let is_title_generated = data
        .extra
        .get("isTitleGenerated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let should_generate_title =
        is_placeholder_title(&data.title) && !is_title_generated && !data.messages.is_empty();
    let worktree = if let Some(candidate) = sanitize_worktree_extra(&mut data.extra) {
        match validate_loaded_worktree_strict(app.clone(), &id, candidate).await {
            Some(validated) => {
                data.extra.insert(
                    "worktree".to_string(),
                    serde_json::to_value(&validated).unwrap_or_default(),
                );
                Some(validated)
            }
            None => {
                data.extra.remove("worktree");
                None
            }
        }
    } else {
        None
    };
    atomic_write_json(
        &file_path,
        &data,
        TrajectorySourceIdentity::from_extra(&data.extra).map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Invalid trajectory source for watcher suppression: {}", e),
            )
        })?,
    )
    .await
    .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if !is_internal_trace_link_type(data.extra.get("link_type").and_then(|value| value.as_str())) {
        let source_hint = Some(
            TrajectorySourceIdentity::from_extra(&data.extra).map_err(|e| {
                ScratchError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Invalid trajectory source for index update: {}", e),
                )
            })?,
        );
        if let Some(dir) = index_dir_for_trajectory_file(gcx.clone(), &file_path).await {
            trajectory_index::upsert_trajectory_index_entry_from_owned_value_with_rollout(
                &app.chat.trajectory_index_coordinator,
                &dir,
                &file_path,
                serde_json::to_value(&data).map_err(|e| {
                    ScratchError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to serialize trajectory for index update: {}", e),
                    )
                })?,
                source_hint,
            )
            .await
            .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        }
    }
    let parent_id = data
        .extra
        .get("parent_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let link_type = data
        .extra
        .get("link_type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let effective_root = data
        .extra
        .get("root_chat_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| id.clone());
    let sessions = app.chat.sessions.clone();
    let source = TrajectorySourceIdentity::from_extra(&data.extra).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Invalid trajectory source for session enrichment: {}", e),
        )
    })?;
    let (session_state, session_error, session_worktree) =
        get_session_runtime_for_trajectory_source(&sessions, &id, &source).await;
    let (total_lines_added, total_lines_removed) =
        calculate_line_changes_from_messages(&data.messages);
    let (tasks_total, tasks_done, tasks_failed) =
        calculate_task_progress_from_messages(&data.messages);
    let token_totals = calculate_token_totals_from_messages(&data.messages);
    let (task_id, task_role, agent_id, card_id) = data
        .extra
        .get("task_meta")
        .and_then(|value| serde_json::from_value::<super::types::TaskMeta>(value.clone()).ok())
        .map(|meta| task_context_from_task_meta(Some(&meta)))
        .unwrap_or((None, None, None, None));
    if emits_generic_event {
        let event = TrajectoryEvent {
            event_type: if is_new {
                "created".to_string()
            } else {
                "updated".to_string()
            },
            id: id.clone(),
            updated_at: Some(data.updated_at.clone()),
            title: Some(trajectory_meta_title(&data.title)),
            is_title_generated: Some(is_title_generated),
            session_state: Some(session_state),
            error: session_error,
            message_count: Some(data.messages.len()),
            parent_id,
            link_type,
            root_chat_id: Some(effective_root),
            task_id,
            task_role,
            agent_id,
            card_id,
            model: Some(data.model.clone()),
            mode: Some(data.mode.clone()),
            worktree: worktree.or(session_worktree),
            total_lines_added: Some(total_lines_added),
            total_lines_removed: Some(total_lines_removed),
            tasks_total: Some(tasks_total),
            tasks_done: Some(tasks_done),
            tasks_failed: Some(tasks_failed),
            total_prompt_tokens: Some(token_totals.prompt_tokens),
            total_completion_tokens: Some(token_totals.completion_tokens),
            total_tokens: Some(token_totals.total_tokens),
            total_cache_read_tokens: Some(token_totals.cache_read_tokens),
            total_cache_creation_tokens: Some(token_totals.cache_creation_tokens),
            total_cost_usd: token_totals.cost_usd,
        };
        let _ = app.chat.trajectory_events_tx.send(event);
    }
    if should_generate_title {
        let source = TrajectorySourceIdentity::from_extra(&data.extra).map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Invalid trajectory source for title generation: {}", e),
            )
        })?;
        spawn_title_generation_task(
            gcx.clone(),
            id.clone(),
            data.messages.clone(),
            file_path.clone(),
            source,
        );
    }
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"status":"ok"}"#))
        .unwrap())
}

pub async fn handle_v1_trajectories_delete(
    State(app): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Response<Body>, ScratchError> {
    let gcx = app.gcx.clone();
    validate_trajectory_id(&id)?;
    let candidate = find_trajectory_file(gcx.clone(), &id)
        .await
        .ok_or_else(|| {
            ScratchError::new(StatusCode::NOT_FOUND, "Trajectory not found".to_string())
        })?;
    let file_path = candidate.path;
    fs::remove_file(&file_path)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if let Some(dir) = index_dir_for_trajectory_file(gcx.clone(), &file_path).await {
        if let Err(e) = trajectory_index::remove_trajectory_index_entry_with_rollout(
            &app.chat.trajectory_index_coordinator,
            &dir,
            &id,
        )
        .await
        {
            warn!(
                "Failed to remove trajectory {} from index {:?}: {}",
                id, dir, e
            );
        }
    }

    let sessions = app.chat.sessions.clone();
    let fallback = match find_trajectory_file(gcx.clone(), &id).await {
        Some(candidate) => load_trajectory_candidate(gcx.clone(), &id, candidate).await,
        None => None,
    };
    let event = if let Some(mut fallback) = fallback {
        if fallback.transition_identity_repaired {
            apply_mode_defaults_to_thread(
                gcx.clone(),
                &mut fallback.thread,
                fallback.auto_approve_editing_tools_present,
                fallback.auto_approve_dangerous_commands_present,
            )
            .await;
            match persist_loaded_trajectory_repair_raw(gcx.clone(), &fallback.repair_patch()).await
            {
                Ok(updated_at) => fallback.updated_at = updated_at,
                Err(e) => warn!(
                    "Failed to persist repaired trajectory for {}: {}",
                    fallback.thread.id, e
                ),
            }
        }
        let task_roots = get_all_task_roots(gcx).await;
        let meta = loaded_trajectory_to_meta(&fallback, &task_roots);
        let (session_state, session_error, session_worktree) =
            get_session_runtime_for_trajectory_source(&sessions, &id, &meta.source).await;
        updated_trajectory_event_from_meta_with_worktree(
            meta,
            Some(fallback.thread.is_title_generated),
            session_state,
            session_error,
            session_worktree,
        )
    } else {
        deleted_trajectory_event(id.clone())
    };
    let _ = app.chat.trajectory_events_tx.send(event);
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"status":"ok"}"#))
        .unwrap())
}

pub async fn handle_v1_trajectories_subscribe(
    State(app): State<AppState>,
) -> Result<Response<Body>, ScratchError> {
    let rx = app.chat.trajectory_events_tx.subscribe();
    let stream = async_stream::stream! {
        let mut rx = rx;
        loop {
            match rx.recv().await {
                Ok(event) => {
                    match serde_json::to_string(&event) {
                        Ok(json) => yield Ok::<_, std::convert::Infallible>(format!("data: {}\n\n", json)),
                        Err(e) => {
                            tracing::error!("Failed to serialize trajectory SSE event: {}", e);
                            break;
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(Body::wrap_stream(stream))
        .unwrap())
}

#[cfg(test)]
mod tests {
    #[test]
    fn malformed_nested_report_remains_browsable_without_emptying_archive() {
        let raw = serde_json::json!([
            {"role":"user", "content":"ARCHIVED_LOAD_SENTINEL"},
            {"role":"compression_report", "content":"invalid boundary", "compression_report":{
                "kind":"reconstructed_history", "schema_version":1,
                "payload":{"messages":"not an array"}
            }}
        ]);
        let messages = super::parse_stored_messages(raw).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[0].content.content_text_only(),
            "ARCHIVED_LOAD_SENTINEL"
        );
        assert!(refact_core::active_context::active_context(&messages).is_err());
        assert!(super::parse_stored_messages(serde_json::json!([{"role":42}])).is_err());
    }

    use super::*;
    use crate::chat::diagnostics::{is_ui_only_message, make_ui_only_error_message};
    use crate::chat::perf_diagnostics::{self, MemoryPerfSink, PerfClock, PerfRecorder};
    use crate::chat::types::{
        ActiveCommandContext, ChatEvent, CompressionPhase, CompressionReason, EventEnvelope,
    };
    use crate::vecdb::vdb_structs::{EmbeddingModelConfig, SearchResult, VecDbStatus, VecdbRecord};
    use async_trait::async_trait;
    use refact_chat_api::{
        BuddyThreadMeta, ClaudeCodeIdentity, FrozenRequestPrefix, GoalBudget, GoalProgress,
        GoalStatus,
    };
    use refact_core::vecdb_types::VecdbSearch;
    use serial_test::serial;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;

    struct BlockingRecordingVecdb {
        enqueued: Arc<StdMutex<Vec<String>>>,
        enqueue_started: Arc<Notify>,
        release_enqueue: Arc<Notify>,
    }

    struct PanickingVecdb {
        enqueue_started: Arc<Notify>,
    }

    #[async_trait]
    impl VecdbSearch for BlockingRecordingVecdb {
        async fn vecdb_search(
            &self,
            query: String,
            _top_n: usize,
            _filter_mb: Option<String>,
        ) -> Result<SearchResult, String> {
            Ok(SearchResult {
                query_text: query,
                results: vec![],
            })
        }

        async fn get_status(&self) -> Result<VecDbStatus, String> {
            Ok(VecDbStatus {
                files_unprocessed: 0,
                files_total: 0,
                requests_made_since_start: 0,
                vectors_made_since_start: 0,
                db_size: 0,
                db_cache_size: 0,
                state: "done".to_string(),
                queue_additions: false,
                vecdb_max_files_hit: false,
                vecdb_errors: Default::default(),
                cache_writes_failed: 0,
                vectors_missing_from_cache: 0,
            })
        }

        async fn remove_file(&self, _file_path: &PathBuf) -> Result<(), String> {
            Ok(())
        }

        async fn vectorizer_enqueue_files(
            &self,
            documents: &[String],
            _process_immediately: bool,
            _roots: refact_core::memory_plane::MemoryPlaneRoots,
        ) {
            self.enqueued.lock().unwrap().extend(documents.to_vec());
            self.enqueue_started.notify_one();
            self.release_enqueue.notified().await;
        }

        fn current_constants(&self) -> (EmbeddingModelConfig, usize) {
            (
                EmbeddingModelConfig {
                    model_id: String::new(),
                    endpoint: String::new(),
                    endpoint_style: String::new(),
                    embedding_endpoint_style: String::new(),
                    api_key: String::new(),
                    model_name: String::new(),
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
            Ok(vec![])
        }

        async fn vecdb_search_with_embedding(
            &self,
            _embedding: &Vec<f32>,
            _top_n: usize,
            _filter_mb: Option<String>,
        ) -> Result<Vec<VecdbRecord>, String> {
            Ok(vec![])
        }
    }

    #[async_trait]
    impl VecdbSearch for PanickingVecdb {
        async fn vecdb_search(
            &self,
            query: String,
            _top_n: usize,
            _filter_mb: Option<String>,
        ) -> Result<SearchResult, String> {
            Ok(SearchResult {
                query_text: query,
                results: vec![],
            })
        }

        async fn get_status(&self) -> Result<VecDbStatus, String> {
            Ok(VecDbStatus {
                files_unprocessed: 0,
                files_total: 0,
                requests_made_since_start: 0,
                vectors_made_since_start: 0,
                db_size: 0,
                db_cache_size: 0,
                state: "done".to_string(),
                queue_additions: false,
                vecdb_max_files_hit: false,
                vecdb_errors: Default::default(),
                cache_writes_failed: 0,
                vectors_missing_from_cache: 0,
            })
        }

        async fn remove_file(&self, _file_path: &PathBuf) -> Result<(), String> {
            Ok(())
        }

        async fn vectorizer_enqueue_files(
            &self,
            _documents: &[String],
            _process_immediately: bool,
            _roots: refact_core::memory_plane::MemoryPlaneRoots,
        ) {
            self.enqueue_started.notify_one();
            panic!("test VecDB enqueue failure");
        }

        fn current_constants(&self) -> (EmbeddingModelConfig, usize) {
            (
                EmbeddingModelConfig {
                    model_id: String::new(),
                    endpoint: String::new(),
                    endpoint_style: String::new(),
                    embedding_endpoint_style: String::new(),
                    api_key: String::new(),
                    model_name: String::new(),
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
            Ok(vec![])
        }

        async fn vecdb_search_with_embedding(
            &self,
            _embedding: &Vec<f32>,
            _top_n: usize,
            _filter_mb: Option<String>,
        ) -> Result<Vec<VecdbRecord>, String> {
            Ok(vec![])
        }
    }

    fn normalized_test_path(path: &Path) -> PathBuf {
        crate::files_correction::canonicalize_normalized_path(path.to_path_buf())
    }

    struct TestPerfClock {
        now: AtomicU64,
    }

    impl TestPerfClock {
        fn new() -> Self {
            Self {
                now: AtomicU64::new(0),
            }
        }
    }

    impl PerfClock for TestPerfClock {
        fn now_us(&self) -> u64 {
            self.now.fetch_add(1, Ordering::SeqCst)
        }
    }

    fn install_perf_recorder() -> (
        perf_diagnostics::TestRecorderGuard,
        Arc<MemoryPerfSink>,
        Arc<PerfRecorder>,
    ) {
        let sink = Arc::new(MemoryPerfSink::new());
        let recorder = Arc::new(PerfRecorder::with_salt(
            Arc::new(TestPerfClock::new()),
            sink.clone(),
            [5; 32],
        ));
        (
            perf_diagnostics::install_test_recorder(recorder.clone()),
            sink,
            recorder,
        )
    }

    fn serial_test_guard() -> std::sync::MutexGuard<'static, ()> {
        perf_diagnostics::PERF_RECORDER_TEST_LOCK.lock().unwrap()
    }

    #[test]
    fn trajectory_commit_stale_completion_cannot_clear_newer_dirty_state() {
        let mut session = ChatSession::new("trajectory-commit-stale".to_string());

        session.increment_version();
        assert!(session.complete_trajectory_commit(1));
        assert_eq!(session.trajectory_committed_version, 1);
        assert!(!session.trajectory_dirty);

        session.increment_version();
        assert!(session.complete_trajectory_commit(2));
        assert_eq!(session.trajectory_committed_version, 2);
        assert!(!session.trajectory_dirty);

        session.increment_version();
        assert!(!session.complete_trajectory_commit(1));
        assert_eq!(session.trajectory_committed_version, 2);
        assert!(session.trajectory_dirty);
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn trajectory_commit_rejects_a_stale_snapshot_before_file_write() {
        let _lock = serial_test_guard();
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(workspace.path()).await;
        let chat_id = "trajectory-commit-file-barrier";
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));

        let stale = {
            let mut session = session_arc.lock().await;
            session.add_message(ChatMessage::new("user".to_string(), "older".to_string()));
            trajectory_snapshot_from_session(&session)
        };
        {
            let mut session = session_arc.lock().await;
            session.add_message(ChatMessage::new("user".to_string(), "newer".to_string()));
        }
        let current = {
            let session = session_arc.lock().await;
            trajectory_snapshot_from_session(&session)
        };

        assert_eq!(
            commit_trajectory_snapshot_for_session(
                gcx.clone(),
                session_arc.clone(),
                current,
                TrajectoryCommitIntent::Required,
            )
            .await
            .unwrap(),
            SessionTrajectoryCommitOutcome::Committed
        );
        assert_eq!(
            commit_trajectory_snapshot_for_session(
                gcx.clone(),
                session_arc.clone(),
                stale,
                TrajectoryCommitIntent::Required,
            )
            .await
            .unwrap(),
            SessionTrajectoryCommitOutcome::SkippedStale
        );

        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        let content = loaded
            .messages
            .iter()
            .map(|message| message.content.content_text_only())
            .collect::<Vec<_>>();
        assert_eq!(content, vec!["older", "newer"]);
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn trajectory_writer_disabled_checkpoint_saves_synchronously() {
        let _lock = serial_test_guard();
        let _writer = TrajectoryWriterRolloutGuard::set(false);
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(workspace.path()).await;
        let chat_id = "checkpoint-writer-legacy";
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));

        session_arc
            .lock()
            .await
            .add_message(ChatMessage::new("user".to_string(), "first".to_string()));
        try_save_trajectory_with_intent(
            app,
            session_arc.clone(),
            TrajectoryCommitIntent::Checkpoint,
        )
        .await
        .unwrap();

        let session = session_arc.lock().await;
        assert!(!session.trajectory_dirty);
        assert!(!session.trajectory_save_in_flight);
        assert_eq!(
            session.trajectory_committed_version,
            session.trajectory_version
        );
        drop(session);
        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        assert_eq!(loaded.messages.len(), 1);
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn trajectory_writer_background_required_intent_waits_for_its_commit() {
        let _lock = serial_test_guard();
        let _writer = TrajectoryWriterRolloutGuard::set(true);
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(workspace.path()).await;
        let chat_id = "background-required-writer";
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        session_arc
            .lock()
            .await
            .add_message(ChatMessage::new("user".to_string(), "required".to_string()));

        maybe_save_trajectory_background_with_intent(
            app,
            session_arc.clone(),
            TrajectoryCommitIntent::Required,
        );
        tokio::time::timeout(
            Duration::from_secs(5),
            wait_for_trajectory_commit(session_arc.clone(), 1),
        )
        .await
        .expect("background required save should commit")
        .unwrap();

        let session = session_arc.lock().await;
        assert!(!session.trajectory_dirty);
        assert_eq!(
            session.trajectory_committed_version,
            session.trajectory_version
        );
        drop(session);
        assert!(load_trajectory_for_chat(gcx, chat_id).await.is_some());
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn trajectory_writer_paths_preserve_stale_required_and_shutdown_commits() {
        let _lock = serial_test_guard();
        for enabled in [false, true] {
            let _writer = TrajectoryWriterRolloutGuard::set(enabled);
            let workspace = tempfile::tempdir().unwrap();
            let (gcx, app) = make_app_with_workspace(workspace.path()).await;
            let chat_id = format!("writer-path-{enabled}");
            let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.clone())));

            let stale = {
                let mut session = session_arc.lock().await;
                session.set_title("writer paths".to_string(), true);
                session.add_message(ChatMessage::new("user".to_string(), "older".to_string()));
                trajectory_snapshot_from_session(&session)
            };
            let current = {
                let mut session = session_arc.lock().await;
                session.add_message(ChatMessage::new("user".to_string(), "newer".to_string()));
                trajectory_snapshot_from_session(&session)
            };
            assert_eq!(
                commit_trajectory_snapshot_for_session(
                    gcx.clone(),
                    session_arc.clone(),
                    current,
                    TrajectoryCommitIntent::Required,
                )
                .await
                .unwrap(),
                SessionTrajectoryCommitOutcome::Committed
            );
            assert_eq!(
                commit_trajectory_snapshot_for_session(
                    gcx.clone(),
                    session_arc.clone(),
                    stale,
                    TrajectoryCommitIntent::Required,
                )
                .await
                .unwrap(),
                SessionTrajectoryCommitOutcome::SkippedStale
            );

            let shutdown_version = {
                let mut session = session_arc.lock().await;
                session.add_message(ChatMessage::new("user".to_string(), "shutdown".to_string()));
                session.trajectory_version
            };
            app.chat
                .sessions
                .write()
                .await
                .insert(chat_id.clone(), session_arc.clone());
            // A flush is done when the writer acknowledges the commit, not when
            // a stopwatch says so: assert the acknowledgement.
            assert_eq!(
                flush_all_trajectories_errors(app).await,
                Vec::<String>::new()
            );
            let session = session_arc.lock().await;
            assert!(!session.trajectory_dirty);
            assert_eq!(session.trajectory_committed_version, shutdown_version);
            drop(session);

            let loaded = load_trajectory_for_chat(gcx, &chat_id).await.unwrap();
            assert_eq!(
                loaded
                    .messages
                    .iter()
                    .map(|message| message.content.content_text_only())
                    .collect::<Vec<_>>(),
                vec!["older", "newer", "shutdown"]
            );
            assert!(
                loaded.thread.is_title_generated,
                "this fixture must not leave a background title-generation save racing the flush"
            );
        }
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn required_writer_waits_for_a_checkpoint_and_commits_latest_version() {
        let _lock = serial_test_guard();
        let _writer = TrajectoryWriterRolloutGuard::set(true);
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(workspace.path()).await;
        let chat_id = "required-after-checkpoint";
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));

        {
            let mut session = session_arc.lock().await;
            session.add_message(ChatMessage::new(
                "user".to_string(),
                "checkpoint".to_string(),
            ));
        }
        maybe_save_trajectory_background_with_intent(
            app.clone(),
            session_arc.clone(),
            TrajectoryCommitIntent::Checkpoint,
        );
        {
            let mut session = session_arc.lock().await;
            session.add_message(ChatMessage::new("user".to_string(), "required".to_string()));
        }

        try_save_trajectory_with_intent(app, session_arc.clone(), TrajectoryCommitIntent::Required)
            .await
            .unwrap();
        let session = session_arc.lock().await;
        assert!(!session.trajectory_dirty);
        assert_eq!(
            session.trajectory_committed_version,
            session.trajectory_version
        );
        drop(session);
        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        assert_eq!(loaded.messages.len(), 2);
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn different_session_writers_commit_independently() {
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(workspace.path()).await;
        let left = Arc::new(AMutex::new(ChatSession::new("writer-left".to_string())));
        let right = Arc::new(AMutex::new(ChatSession::new("writer-right".to_string())));
        left.lock()
            .await
            .add_message(ChatMessage::new("user".to_string(), "left".to_string()));
        right
            .lock()
            .await
            .add_message(ChatMessage::new("user".to_string(), "right".to_string()));

        let (left_result, right_result) = tokio::join!(
            flush_trajectory_for_session(app.clone(), left),
            flush_trajectory_for_session(app, right),
        );
        left_result.unwrap();
        right_result.unwrap();
        assert_eq!(
            load_trajectory_for_chat(gcx.clone(), "writer-left")
                .await
                .unwrap()
                .messages[0]
                .content
                .content_text_only(),
            "left"
        );
        assert_eq!(
            load_trajectory_for_chat(gcx, "writer-right")
                .await
                .unwrap()
                .messages[0]
                .content
                .content_text_only(),
            "right"
        );
    }

    #[tokio::test]
    async fn trajectory_commit_finishes_before_blocked_vecdb_enqueue() {
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(workspace.path()).await;
        let enqueued = Arc::new(StdMutex::new(Vec::new()));
        let enqueue_started = Arc::new(Notify::new());
        let release_enqueue = Arc::new(Notify::new());
        *gcx.vec_db.lock().await = Some(Arc::new(BlockingRecordingVecdb {
            enqueued: enqueued.clone(),
            enqueue_started: enqueue_started.clone(),
            release_enqueue: release_enqueue.clone(),
        }));

        let mut save = tokio::spawn(save_trajectory_snapshot(
            gcx.clone(),
            test_snapshot(
                "commit-before-enqueue",
                "Committed",
                vec![ChatMessage::new("user".to_string(), "durable".to_string())],
            ),
        ));

        tokio::time::timeout(Duration::from_secs(60), enqueue_started.notified())
            .await
            .unwrap();
        assert!(load_trajectory_for_chat(gcx, "commit-before-enqueue")
            .await
            .is_some());
        tokio::time::timeout(Duration::from_millis(100), &mut save)
            .await
            .expect("required trajectory commit must not wait for VecDB enqueue")
            .unwrap()
            .unwrap();
        release_enqueue.notify_one();
    }

    #[tokio::test]
    async fn buddy_trajectory_save_is_not_enqueued_for_vectorization() {
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(workspace.path()).await;
        let enqueued = Arc::new(StdMutex::new(Vec::new()));
        let enqueue_started = Arc::new(Notify::new());
        let release_enqueue = Arc::new(Notify::new());
        *gcx.vec_db.lock().await = Some(Arc::new(BlockingRecordingVecdb {
            enqueued: enqueued.clone(),
            enqueue_started: enqueue_started.clone(),
            release_enqueue,
        }));
        let mut snapshot = test_snapshot(
            "buddy-not-vectorized",
            "Buddy",
            vec![ChatMessage::new("user".to_string(), "hello".to_string())],
        );
        snapshot.buddy_meta = Some(buddy_thread_meta());

        save_trajectory_snapshot(gcx, snapshot).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), enqueue_started.notified())
                .await
                .is_err()
        );

        assert!(enqueued.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn internal_trace_save_is_not_enqueued_for_vectorization() {
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(workspace.path()).await;
        let enqueued = Arc::new(StdMutex::new(Vec::new()));
        let enqueue_started = Arc::new(Notify::new());
        let release_enqueue = Arc::new(Notify::new());
        *gcx.vec_db.lock().await = Some(Arc::new(BlockingRecordingVecdb {
            enqueued: enqueued.clone(),
            enqueue_started: enqueue_started.clone(),
            release_enqueue,
        }));
        let mut snapshot = test_snapshot(
            "internal-trace-not-vectorized",
            "Trace",
            vec![ChatMessage::new("user".to_string(), "hello".to_string())],
        );
        snapshot.link_type = Some(internal_trace_link_type("title_generation"));

        save_trajectory_snapshot(gcx, snapshot).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), enqueue_started.notified())
                .await
                .is_err()
        );

        assert!(enqueued.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn vecdb_enqueue_failure_does_not_fail_trajectory_save() {
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(workspace.path()).await;
        let enqueue_started = Arc::new(Notify::new());
        *gcx.vec_db.lock().await = Some(Arc::new(PanickingVecdb {
            enqueue_started: enqueue_started.clone(),
        }));

        save_trajectory_snapshot(
            gcx.clone(),
            test_snapshot(
                "enqueue-failure",
                "Committed",
                vec![ChatMessage::new("user".to_string(), "durable".to_string())],
            ),
        )
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(60), enqueue_started.notified())
            .await
            .expect("best-effort VecDB enqueue should have run");
        assert!(load_trajectory_for_chat(gcx, "enqueue-failure")
            .await
            .is_some());
    }

    #[tokio::test]
    async fn detached_trajectory_writer_keys_preserve_workspace_boundaries() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let snapshot = test_snapshot("writer-key", "Writer key", Vec::new());
        {
            let mut folders = gcx.documents_state.workspace_folders.lock().unwrap();
            *folders = vec![PathBuf::from("alpha|beta"), PathBuf::from("gamma")];
        }
        let first = detached_trajectory_writer_key(&gcx, &snapshot);
        {
            let mut folders = gcx.documents_state.workspace_folders.lock().unwrap();
            *folders = vec![PathBuf::from("alpha"), PathBuf::from("beta|gamma")];
        }
        let second = detached_trajectory_writer_key(&gcx, &snapshot);
        assert_ne!(first, second);
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn trajectory_writer_failed_checkpoint_retries_before_a_required_commit() {
        let _lock = serial_test_guard();
        let _writer = TrajectoryWriterRolloutGuard::set(true);
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(workspace.path()).await;
        let chat_id = "checkpoint-retry-required";
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        let failing_message = ChatMessage {
            message_id: "checkpoint-failure".to_string(),
            role: "user".to_string(),
            content: ChatContent::SimpleText("first".to_string()),
            ..Default::default()
        };
        {
            let mut session = session_arc.lock().await;
            session.add_message(failing_message.clone());
        }
        set_test_message_serialization_failure(Some(failing_message.message_id));
        try_save_trajectory_with_intent(
            app.clone(),
            session_arc.clone(),
            TrajectoryCommitIntent::Checkpoint,
        )
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                if session_arc.lock().await.trajectory_save_error.is_some() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("checkpoint failure should reach the session");
        set_test_message_serialization_failure(None);

        {
            let mut session = session_arc.lock().await;
            session.add_message(ChatMessage::new("user".to_string(), "required".to_string()));
        }
        try_save_trajectory_with_intent(app, session_arc.clone(), TrajectoryCommitIntent::Required)
            .await
            .unwrap();

        let session = session_arc.lock().await;
        assert!(!session.trajectory_dirty);
        assert_eq!(
            session.trajectory_committed_version,
            session.trajectory_version
        );
        drop(session);
        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        assert_eq!(loaded.messages.len(), 2);
    }

    #[tokio::test]
    async fn trajectory_writer_cache_preserves_external_metadata() {
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(workspace.path()).await;
        let chat_id = "detached-writer-cache";
        let first = test_snapshot(
            chat_id,
            "First",
            vec![ChatMessage::new("user".to_string(), "first".to_string())],
        );
        let write_cache = Arc::new(AMutex::new(DetachedTrajectoryWriteCache::default()));
        save_trajectory_snapshot_inner(gcx.clone(), first, Some(write_cache.clone()))
            .await
            .unwrap();
        let path = workspace
            .path()
            .join(".refact")
            .join("trajectories")
            .join(chat_id)
            .join(format!("{chat_id}.json"));
        let mut external: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        external["external_metadata"] = json!({"preserved": true});
        tokio::fs::write(&path, serde_json::to_string_pretty(&external).unwrap())
            .await
            .unwrap();

        let second = test_snapshot(
            chat_id,
            "Second",
            vec![ChatMessage::new("user".to_string(), "second".to_string())],
        );
        save_trajectory_snapshot_inner(gcx, second, Some(write_cache))
            .await
            .unwrap();

        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(saved["external_metadata"], json!({"preserved": true}));
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn repeated_save_reuses_pretty_round_trip_metadata_without_a_second_disk_read() {
        let _lock = serial_test_guard();
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(workspace.path()).await;
        let chat_id = "metadata-cache-round-trip";
        let path = workspace
            .path()
            .join(".refact")
            .join("trajectories")
            .join(chat_id)
            .join(format!("{chat_id}.json"));
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "id": chat_id,
                "title": "Before",
                "messages": [],
                "browser_meta": {
                    "browser_runtime_id": "browser-round-trip",
                    "tab_urls": ["https://example.com/round-trip"]
                },
                "custom_future_field": {"nested": {"value": 61}},
                "custom_scalar": "preserve-me"
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let metadata_reads = count_test_trajectory_metadata_reads(&path);
        let reads_before = metadata_reads.load(Ordering::Relaxed);
        let chat_reads_before = trajectory_metadata_disk_reads_for_chat(chat_id);
        for title in ["First", "Second"] {
            save_trajectory_snapshot(
                gcx.clone(),
                test_snapshot(
                    chat_id,
                    title,
                    vec![ChatMessage::new("user".to_string(), title.to_string())],
                ),
            )
            .await
            .unwrap();
        }
        let reads_after = metadata_reads.load(Ordering::Relaxed);
        assert_eq!(
            reads_after - reads_before,
            1,
            "the second save must reuse the cached metadata instead of re-reading from disk"
        );
        let chat_reads_after = trajectory_metadata_disk_reads_for_chat(chat_id);
        assert_eq!(
            chat_reads_after - chat_reads_before,
            1,
            "the second save must reuse the cached metadata instead of re-reading from disk"
        );

        let text = tokio::fs::read_to_string(&path).await.unwrap();
        let saved: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(text, serde_json::to_string_pretty(&saved).unwrap());
        assert!(text.contains("\n  \"browser_meta\": {"));
        assert_eq!(saved["title"], "Second");
        assert_eq!(
            saved["browser_meta"]["browser_runtime_id"],
            "browser-round-trip"
        );
        assert_eq!(saved["custom_future_field"]["nested"]["value"], 61);
        assert_eq!(saved["custom_scalar"], "preserve-me");

        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.messages[0].content.content_text_only(), "Second");
    }

    #[tokio::test]
    async fn trajectory_writer_cache_rejects_replaced_trajectory_for_another_chat() {
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(workspace.path()).await;
        let chat_id = "detached-writer-cache-replaced";
        let write_cache = Arc::new(AMutex::new(DetachedTrajectoryWriteCache::default()));
        save_trajectory_snapshot_inner(
            gcx.clone(),
            test_snapshot(
                chat_id,
                "First",
                vec![ChatMessage::new("user".to_string(), "first".to_string())],
            ),
            Some(write_cache.clone()),
        )
        .await
        .unwrap();
        let path = workspace
            .path()
            .join(".refact")
            .join("trajectories")
            .join(chat_id)
            .join(format!("{chat_id}.json"));
        let foreign = json!({
            "id": "other-chat",
            "title": "Foreign",
            "messages": [],
        });
        tokio::fs::write(&path, serde_json::to_string_pretty(&foreign).unwrap())
            .await
            .unwrap();

        let error = save_trajectory_snapshot_inner(
            gcx,
            test_snapshot(
                chat_id,
                "Second",
                vec![ChatMessage::new("user".to_string(), "second".to_string())],
            ),
            Some(write_cache),
        )
        .await
        .unwrap_err();
        assert!(is_trajectory_id_mismatch_error(&error));
        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(saved["id"], "other-chat");
    }

    fn coalesce_test_snapshot(chat_id: &str, index: usize) -> TrajectorySnapshot {
        test_snapshot(
            chat_id,
            &format!("Title {index}"),
            vec![ChatMessage::new(
                "user".to_string(),
                format!("message {index}"),
            )],
        )
    }

    #[test]
    fn snapshot_payload_serialization_matches_expected_json() {
        let mut snapshot = test_snapshot(
            "snapshot-payload",
            "Payload",
            vec![ChatMessage::new("user".to_string(), "hello".to_string())],
        );
        snapshot.reasoning_effort = Some("high".to_string());
        snapshot.thinking_budget = Some(2048);
        snapshot.temperature = Some(0.25);
        snapshot.frequency_penalty = Some(0.5);
        snapshot.max_tokens = Some(4096);
        snapshot.previous_response_id = Some("response-1".to_string());
        snapshot.parallel_tool_calls = Some(true);
        snapshot.active_skill = Some("skill-1".to_string());
        snapshot.auto_enrichment_enabled = Some(false);
        snapshot.auto_compact_enabled = Some(true);
        snapshot.parent_id = Some("parent-1".to_string());
        snapshot.link_type = Some("branch".to_string());
        snapshot.root_chat_id = Some("root-1".to_string());

        let actual = trajectory_value_from_snapshot(
            &snapshot,
            snapshot
                .messages
                .iter()
                .map(serde_json::to_value)
                .collect::<Result<_, _>>()
                .unwrap(),
        )
        .unwrap();
        let expected = json!({
            "id": "snapshot-payload",
            "title": "Payload",
            "model": "model",
            "mode": "agent",
            "tool_use": "agent",
            "messages": [serde_json::to_value(&snapshot.messages[0]).unwrap()],
            "created_at": "2024-01-01T00:00:00Z",
            "boost_reasoning": false,
            "checkpoints_enabled": true,
            "context_tokens_cap": null,
            "auto_compression_cap": null,
            "auto_compression_cap_pending": false,
            "include_project_info": true,
            "isTitleGenerated": true,
            "auto_approve_editing_tools": false,
            "auto_approve_dangerous_commands": false,
            "autonomous_no_confirm": false,
            "reasoning_effort": "high",
            "thinking_budget": 2048,
            "temperature": 0.25,
            "frequency_penalty": 0.5,
            "max_tokens": 4096,
            "previous_response_id": "response-1",
            "parallel_tool_calls": true,
            "active_skill": "skill-1",
            "auto_enrichment_enabled": false,
            "auto_compact_enabled": true,
            "parent_id": "parent-1",
            "link_type": "branch",
            "root_chat_id": "root-1"
        });

        assert_eq!(
            serde_json::to_string(&actual).unwrap(),
            serde_json::to_string(&expected).unwrap()
        );
    }

    #[test]
    fn pretty_output_bytes_are_unchanged() {
        let fixture = json!({
            "id": "pretty-bytes-fixture",
            "title": "Ünïcøde ✓ \"quoted\" \\ backslash\n\ttabbed",
            "messages": [
                {
                    "role": "user",
                    "content": "hello",
                    "nested": {"deep": [1, 2.5, true, null, -0.125]}
                },
                {"role": "assistant", "content": [], "empty_object": {}}
            ],
            "counts": {"a": 0, "b": -17, "c": 1234567890},
            "control_chars": "\u{1}\u{7f}"
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pretty-bytes-fixture.json.tmp");

        let write = write_pretty_json_to_tmp_file(&path, &fixture).unwrap();
        let written = std::fs::read(&path).unwrap();
        let expected = serde_json::to_string_pretty(&fixture).unwrap();

        assert_eq!(written, expected.as_bytes());
        assert_eq!(write.byte_len, expected.len() as u64);
        assert_eq!(write.content_hash, md5::compute(expected.as_bytes()).0);
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn checkpoint_writes_coalesce_within_window() {
        let _lock = serial_test_guard();
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(workspace.path()).await;
        let chat_id = "detached-writer-coalesce-window";
        let writer = Arc::new(AMutex::new(DetachedTrajectoryWriterState {
            pending: Some((1, coalesce_test_snapshot(chat_id, 0))),
            coalesce_deadline: Some(Instant::now() + TRAJECTORY_CHECKPOINT_COALESCE_WINDOW),
            requested_version: 1,
            in_flight: true,
            ..Default::default()
        }));
        let worker = tokio::spawn(run_detached_trajectory_writer(gcx.clone(), writer.clone()));

        for index in 1..5usize {
            let wake = {
                let mut state = writer.lock().await;
                state.requested_version = index as u64 + 1;
                state.pending = Some((index as u64 + 1, coalesce_test_snapshot(chat_id, index)));
                state.coalesce_deadline =
                    Some(Instant::now() + TRAJECTORY_CHECKPOINT_COALESCE_WINDOW);
                state.wake.clone()
            };
            wake.notify_one();
        }
        assert_eq!(
            trajectory_physical_writes_for_chat(chat_id),
            0,
            "checkpoints must not reach disk while the coalescing window is open"
        );

        let target_version = release_detached_trajectory_coalescing(&writer).await;
        wait_for_detached_trajectory_commit(writer.clone(), target_version)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), worker)
            .await
            .expect("writer should stop once the pending slot is drained")
            .unwrap();

        assert_eq!(
            trajectory_physical_writes_for_chat(chat_id),
            1,
            "a burst of checkpoints must collapse into a single physical write"
        );
        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        assert_eq!(loaded.thread.title, "Title 4");
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.messages[0].content.content_text_only(), "message 4");
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn durable_intent_bypasses_coalescing() {
        let _lock = serial_test_guard();
        let _rollout = TrajectoryWriterRolloutGuard::set(true);
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(workspace.path()).await;
        let chat_id = "detached-writer-durable-bypass";
        let writer = detached_trajectory_writer(&gcx, &coalesce_test_snapshot(chat_id, 0));

        persist_trajectory_snapshot_with_intent(
            gcx.clone(),
            coalesce_test_snapshot(chat_id, 0),
            TrajectoryCommitIntent::Checkpoint,
        )
        .await
        .unwrap();
        assert!(
            writer.lock().await.coalesce_deadline.is_some(),
            "a checkpoint intent must arm the coalescing window"
        );

        persist_trajectory_snapshot_with_intent(
            gcx.clone(),
            coalesce_test_snapshot(chat_id, 1),
            TrajectoryCommitIntent::Required,
        )
        .await
        .unwrap();

        {
            let state = writer.lock().await;
            assert!(state.coalesce_deadline.is_none());
            assert!(state.pending.is_none());
            assert_eq!(state.committed_version, 2);
        }
        assert_eq!(trajectory_physical_writes_for_chat(chat_id), 1);
        let path = workspace
            .path()
            .join(".refact")
            .join("trajectories")
            .join(chat_id)
            .join(format!("{chat_id}.json"));
        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        assert_eq!(saved["title"], "Title 1");
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn aborted_writer_does_not_wedge_pending_coalesced_write() {
        let _lock = serial_test_guard();
        let _rollout = TrajectoryWriterRolloutGuard::set(true);
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(workspace.path()).await;
        let chat_id = "detached-writer-worker-abort";
        let writer = detached_trajectory_writer(&gcx, &coalesce_test_snapshot(chat_id, 0));

        persist_trajectory_snapshot_with_intent(
            gcx.clone(),
            coalesce_test_snapshot(chat_id, 0),
            TrajectoryCommitIntent::Checkpoint,
        )
        .await
        .unwrap();
        assert!(
            writer.lock().await.coalesce_deadline.is_some(),
            "a checkpoint intent must arm the coalescing window"
        );
        assert_eq!(trajectory_physical_writes_for_chat(chat_id), 0);

        let worker = writer
            .lock()
            .await
            .worker
            .take()
            .expect("checkpoint must have spawned a detached writer");
        worker.abort();
        let _ = worker.await;
        assert!(
            writer.lock().await.in_flight,
            "aborting the worker leaves in_flight set, which is exactly the wedge under test"
        );

        persist_trajectory_snapshot_with_intent(
            gcx.clone(),
            coalesce_test_snapshot(chat_id, 1),
            TrajectoryCommitIntent::Required,
        )
        .await
        .unwrap();

        {
            let state = writer.lock().await;
            assert!(state.pending.is_none());
            assert!(
                !state.in_flight,
                "in_flight must not stay stuck after the replacement worker drains the queue"
            );
            assert_eq!(state.committed_version, 2);
        }
        assert_eq!(
            trajectory_physical_writes_for_chat(chat_id),
            1,
            "the pending snapshot must land exactly once after the worker was aborted"
        );
        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        assert_eq!(loaded.thread.title, "Title 1");
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.messages[0].content.content_text_only(), "message 1");
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn pending_coalesced_write_is_flushed_on_shutdown() {
        let _lock = serial_test_guard();
        let _rollout = TrajectoryWriterRolloutGuard::set(true);
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(workspace.path()).await;
        let chat_id = "detached-writer-shutdown-flush";
        let writer = detached_trajectory_writer(&gcx, &coalesce_test_snapshot(chat_id, 0));

        for index in 0..3usize {
            persist_trajectory_snapshot_with_intent(
                gcx.clone(),
                coalesce_test_snapshot(chat_id, index),
                TrajectoryCommitIntent::Checkpoint,
            )
            .await
            .unwrap();
        }
        assert_eq!(trajectory_physical_writes_for_chat(chat_id), 0);

        gcx.shutdown_flag.store(true, Ordering::Relaxed);
        if let Err(errors) = flush_detached_trajectory_writers().await {
            assert!(
                !errors.iter().any(|error| error.contains(chat_id)),
                "{errors:?}"
            );
        }
        gcx.shutdown_flag.store(false, Ordering::Relaxed);

        assert_eq!(trajectory_physical_writes_for_chat(chat_id), 1);
        assert!(writer.lock().await.pending.is_none());
        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        assert_eq!(loaded.thread.title, "Title 2");
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.messages[0].content.content_text_only(), "message 2");
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn trajectory_writer_failure_is_stored_and_reported_to_waiters() {
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(workspace.path()).await;
        let mut snapshot = test_snapshot(
            "detached-writer-failure",
            "Failure",
            vec![ChatMessage::new("user".to_string(), "failure".to_string())],
        );
        snapshot.messages[0].message_id = "detached-writer-failure-message".to_string();
        let writer = Arc::new(AMutex::new(DetachedTrajectoryWriterState {
            pending: Some((1, snapshot.clone())),
            requested_version: 1,
            in_flight: true,
            ..Default::default()
        }));
        set_test_message_serialization_failure(Some(snapshot.messages[0].message_id.clone()));
        run_detached_trajectory_writer(gcx, writer.clone()).await;
        set_test_message_serialization_failure(None);

        let error = wait_for_detached_trajectory_commit(writer.clone(), 1)
            .await
            .unwrap_err();
        assert!(error.contains("test failure"));
        assert_eq!(writer.lock().await.error.as_deref(), Some(error.as_str()));
    }

    #[serial(trajectory_perf)]
    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn detached_trajectory_writer_drains_newer_snapshot_after_a_failed_write() {
        let _lock = serial_test_guard();
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(workspace.path()).await;
        let chat_id = "detached-writer-drains-newer";
        let mut failed = test_snapshot(
            chat_id,
            "Failed",
            vec![ChatMessage::new("user".to_string(), "failed".to_string())],
        );
        failed.messages[0].message_id = "detached-writer-drains-newer-failed".to_string();
        let newer = test_snapshot(
            chat_id,
            "Newer",
            vec![ChatMessage::new("user".to_string(), "newer".to_string())],
        );
        let writer = Arc::new(AMutex::new(DetachedTrajectoryWriterState {
            pending: Some((1, failed.clone())),
            requested_version: 1,
            in_flight: true,
            ..Default::default()
        }));
        let failure_seen = Arc::new(Notify::new());
        let release_failure = Arc::new(Notify::new());
        set_test_detached_trajectory_writer_failure_hook(Some((
            chat_id.to_string(),
            failure_seen.clone(),
            release_failure.clone(),
        )));
        set_test_message_serialization_failure(Some(failed.messages[0].message_id.clone()));
        let failure_seen_wait = failure_seen.notified();
        let task = tokio::spawn(run_detached_trajectory_writer(gcx.clone(), writer.clone()));

        tokio::time::timeout(Duration::from_secs(5), failure_seen_wait)
            .await
            .expect("failed write should pause before recording its outcome");
        set_test_message_serialization_failure(None);
        {
            let mut state = writer.lock().await;
            state.requested_version = 2;
            state.pending = Some((2, newer));
        }
        release_failure.notify_one();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("writer should drain the newer snapshot")
            .unwrap();
        set_test_detached_trajectory_writer_failure_hook(None);

        let state = writer.lock().await;
        assert_eq!(state.committed_version, 2);
        assert_eq!(state.error, None);
        assert!(!state.in_flight);
        drop(state);
        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        assert_eq!(loaded.messages[0].content.content_text_only(), "newer");
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn trajectory_writer_registry_cleans_completed_and_source_isolated_entries() {
        let _lock = serial_test_guard();
        let _writer = TrajectoryWriterRolloutGuard::set(true);
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(workspace.path()).await;
        let chat_id = "detached-writer-source-isolation";
        let normal = test_snapshot(
            chat_id,
            "Normal",
            vec![ChatMessage::new("user".to_string(), "normal".to_string())],
        );
        let mut buddy = test_snapshot(
            chat_id,
            "Buddy",
            vec![ChatMessage::new("user".to_string(), "buddy".to_string())],
        );
        buddy.mode = "buddy".to_string();
        buddy.buddy_meta = Some(buddy_thread_meta());
        let normal_key = detached_trajectory_writer_key(&gcx, &normal);
        let buddy_key = detached_trajectory_writer_key(&gcx, &buddy);

        let (normal_result, buddy_result) = tokio::join!(
            save_trajectory_snapshot(gcx.clone(), normal),
            save_trajectory_snapshot(gcx.clone(), buddy),
        );
        normal_result.unwrap();
        buddy_result.unwrap();
        assert_ne!(normal_key, buddy_key);
        prune_detached_trajectory_writers();
        let writers = detached_trajectory_writers()
            .lock()
            .expect("detached trajectory writer registry poisoned");
        assert!(!writers.contains_key(&normal_key));
        assert!(!writers.contains_key(&buddy_key));
        drop(writers);

        let normal = load_trajectory_for_chat(gcx.clone(), chat_id)
            .await
            .unwrap();
        assert_eq!(normal.thread.title, "Normal");
        let buddy_path = workspace
            .path()
            .join(".refact")
            .join("buddy")
            .join("chats")
            .join("conversations")
            .join(format!("{chat_id}.json"));
        let buddy: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(buddy_path).await.unwrap()).unwrap();
        assert_eq!(buddy["title"], "Buddy");
    }

    #[tokio::test]
    async fn trajectory_writer_shutdown_flush_is_bounded_and_reports_blocked_session() {
        let workspace = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(workspace.path()).await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new("flush-timeout".to_string())));
        session_arc
            .lock()
            .await
            .add_message(ChatMessage::new("user".to_string(), "pending".to_string()));
        app.chat
            .sessions
            .write()
            .await
            .insert("flush-timeout".to_string(), session_arc.clone());

        let guard = session_arc.lock().await;
        let errors = flush_all_trajectories_with_timeout(app, Duration::from_millis(10))
            .await
            .unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("timed out"));
        drop(guard);
    }

    fn assert_same_path(left: &Path, right: &Path) {
        assert_eq!(normalized_test_path(left), normalized_test_path(right));
    }

    fn assert_same_optional_path(left: Option<PathBuf>, right: &Path) {
        assert_same_path(&left.expect("path should exist"), right);
    }

    fn assert_same_path_str(left: &str, right: &Path) {
        assert_same_path(Path::new(left), right);
    }

    async fn write_conversation_file(dir: &Path, id: &str, age_secs: u64) -> PathBuf {
        let path = dir.join(format!("{id}.json"));
        tokio::fs::write(
            &path,
            serde_json::json!({
                "id": id,
                "chat_id": id,
                "title": id,
                "created_at": "2026-01-01T00:00:00Z",
                "messages": [{"role": "user", "content": "hi"}]
            })
            .to_string(),
        )
        .await
        .unwrap();
        let modified = SystemTime::now() - std::time::Duration::from_secs(age_secs);
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_modified(modified).unwrap();
        path
    }

    #[tokio::test]
    async fn prune_buddy_conversations_keeps_newest_and_updates_index() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("conversations");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let old_a = write_conversation_file(&dir, "old-a", 4 * 86_400).await;
        let old_b = write_conversation_file(&dir, "old-b", 3 * 86_400).await;
        let new_a = write_conversation_file(&dir, "new-a", 2000).await;
        let new_b = write_conversation_file(&dir, "new-b", 1000).await;
        let index_path = dir.join("index.json");
        let index_entries = ["old-a", "old-b", "new-a", "new-b"]
            .iter()
            .map(|id| {
                serde_json::json!({
                    "id": id,
                    "file_name": format!("{id}.json"),
                    "created_at": "2026-01-01T00:00:00Z",
                    "updated_at": "2026-01-01T00:00:00Z",
                    "title": id,
                    "model": "test-model",
                    "mode": "AGENT",
                    "message_count": 1
                })
            })
            .collect::<Vec<_>>();
        tokio::fs::write(
            &index_path,
            serde_json::json!({
                "schema_version": 1,
                "updated_at": "2026-01-01T00:00:00Z",
                "entries": index_entries
            })
            .to_string(),
        )
        .await
        .unwrap();

        let removed = prune_buddy_conversations_in_dir(&dir, 2).await.unwrap();

        assert_eq!(removed, 2);
        assert!(!old_a.exists());
        assert!(!old_b.exists());
        assert!(new_a.exists());
        assert!(new_b.exists());
        assert!(index_path.exists());
        let index = trajectory_index::read_trajectory_index(&dir)
            .await
            .unwrap()
            .unwrap();
        let ids: Vec<&str> = index
            .entries
            .iter()
            .map(|entry| entry.id.as_str())
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"new-a"));
        assert!(ids.contains(&"new-b"));
    }

    #[tokio::test]
    async fn prune_buddy_conversations_spares_recently_modified_overflow() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("conversations");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let newest = write_conversation_file(&dir, "newest", 100).await;
        let recent_overflow = write_conversation_file(&dir, "recent-overflow", 2 * 3600).await;
        let old_overflow = write_conversation_file(&dir, "old-overflow", 3 * 86_400).await;

        let removed = prune_buddy_conversations_in_dir(&dir, 1).await.unwrap();

        assert_eq!(removed, 1);
        assert!(newest.exists());
        assert!(recent_overflow.exists());
        assert!(!old_overflow.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn prune_buddy_conversations_skips_symlinked_entries() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let dir = temp.path().join("conversations");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let real = write_conversation_file(&dir, "real", 3 * 86_400).await;
        let outside_file = write_conversation_file(outside.path(), "outside", 3 * 86_400).await;
        std::os::unix::fs::symlink(&outside_file, dir.join("linked.json")).unwrap();

        let removed = prune_buddy_conversations_in_dir(&dir, 0).await.unwrap();

        assert_eq!(removed, 1);
        assert!(!real.exists());
        assert!(outside_file.exists());
        assert!(dir.join("linked.json").exists());
    }

    fn run_git(cwd: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_repo(root: &Path) {
        run_git(root, &["init"]);
        run_git(root, &["checkout", "-b", "main"]);
        run_git(root, &["config", "core.autocrlf", "false"]);
        run_git(root, &["config", "user.email", "test@example.com"]);
        run_git(root, &["config", "user.name", "Test User"]);
        std::fs::write(root.join("file.txt"), "hello\n").unwrap();
        run_git(root, &["add", "."]);
        run_git(root, &["commit", "-m", "initial"]);
    }

    async fn make_app_with_workspace(root: &Path) -> (Arc<GlobalContext>, AppState) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        *app.workspace
            .documents_state
            .workspace_folders
            .lock()
            .unwrap() = vec![normalized_test_path(root)];
        (gcx, app)
    }

    fn sample_trajectory(id: &str, title: &str, updated_at: &str) -> serde_json::Value {
        json!({
            "id": id,
            "title": title,
            "model": "model",
            "mode": "agent",
            "tool_use": "agent",
            "messages": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": updated_at,
            "include_project_info": true,
            "checkpoints_enabled": true
        })
    }

    async fn write_trajectory_file(path: &Path, id: &str, title: &str, updated_at: &str) {
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            path,
            serde_json::to_string(&sample_trajectory(id, title, updated_at)).unwrap(),
        )
        .await
        .unwrap();
    }

    async fn write_trajectory_file_with_user_message(
        path: &Path,
        id: &str,
        title: &str,
        message: &str,
    ) {
        let mut trajectory = sample_trajectory(id, title, "2024-01-01T00:00:01Z");
        trajectory["messages"] = json!([{ "role": "user", "content": message }]);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(path, serde_json::to_string(&trajectory).unwrap())
            .await
            .unwrap();
    }

    async fn write_trajectory_file_with_metadata(
        path: &Path,
        id: &str,
        title: &str,
        updated_at: &str,
        message: &str,
    ) {
        let mut trajectory = sample_trajectory(id, title, updated_at);
        trajectory["model"] = json!("fallback-model");
        trajectory["mode"] = json!("task_planner");
        trajectory["isTitleGenerated"] = json!(true);
        trajectory["parent_id"] = json!("parent-fallback");
        trajectory["link_type"] = json!("handoff");
        trajectory["root_chat_id"] = json!("root-fallback");
        trajectory["messages"] = json!([{ "role": "user", "content": message }]);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(path, serde_json::to_string(&trajectory).unwrap())
            .await
            .unwrap();
    }

    async fn write_schema_incomplete_trajectory_file(path: &Path, id: &str) {
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(path, serde_json::to_string(&json!({"id": id})).unwrap())
            .await
            .unwrap();
    }

    async fn write_buddy_conversation_file(path: &Path, id: &str, title: &str) {
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            path,
            serde_json::to_string(&json!({
                "id": id,
                "chat_id": id,
                "title": title,
                "model": "model",
                "mode": "buddy",
                "tool_use": "agent",
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": false,
                "messages": [{"role":"user","content":"hello buddy"}],
                "buddy_meta": {"is_buddy_chat": true, "buddy_chat_kind": "investigation"}
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    }

    fn task_meta(
        task_id: &str,
        role: &str,
        agent_id: Option<&str>,
        card_id: Option<&str>,
        planner_chat_id: Option<&str>,
    ) -> crate::chat::types::TaskMeta {
        crate::chat::types::TaskMeta {
            task_id: task_id.to_string(),
            role: role.to_string(),
            agent_id: agent_id.map(ToString::to_string),
            card_id: card_id.map(ToString::to_string),
            planner_chat_id: planner_chat_id.map(ToString::to_string),
        }
    }

    async fn write_task_trajectory_file_with_user_message(
        path: &Path,
        id: &str,
        title: &str,
        message: &str,
        task_meta: &crate::chat::types::TaskMeta,
    ) {
        let mut trajectory = sample_trajectory(id, title, "2024-01-01T00:00:01Z");
        trajectory["messages"] = json!([{ "role": "user", "content": message }]);
        trajectory["task_meta"] = serde_json::to_value(task_meta).unwrap();
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(path, serde_json::to_string(&trajectory).unwrap())
            .await
            .unwrap();
    }

    fn buddy_thread_meta() -> BuddyThreadMeta {
        BuddyThreadMeta {
            is_buddy_chat: true,
            buddy_chat_kind: "investigation".to_string(),
            workflow_id: None,
            source_chat_id: None,
            source_project: None,
            source_created_at: None,
        }
    }

    struct TestTrajectoryWatcher(Option<tokio::task::JoinHandle<()>>);

    impl TestTrajectoryWatcher {
        async fn stop(mut self) {
            let handle = self.0.take().unwrap();
            handle.abort();
            assert!(handle.await.unwrap_err().is_cancelled());
        }
    }

    impl Drop for TestTrajectoryWatcher {
        fn drop(&mut self) {
            if let Some(handle) = &self.0 {
                handle.abort();
            }
        }
    }

    async fn wait_for_watcher_start(
        ready: tokio::sync::oneshot::Receiver<Result<Vec<PathBuf>, String>>,
        expected_task_root: &Path,
    ) {
        let registered_roots = tokio::time::timeout(Duration::from_secs(60), ready)
            .await
            .expect("trajectory watcher registration timed out")
            .expect("trajectory watcher exited before registration")
            .expect("trajectory watcher root registration failed");
        let expected_task_root = normalized_test_path(expected_task_root);
        assert!(
            registered_roots
                .iter()
                .any(|root| normalized_test_path(root) == expected_task_root),
            "trajectory watcher did not register expected tasks root {expected_task_root:?}: {registered_roots:?}"
        );
    }

    fn nested_trajectory_path(
        workspace: &std::path::Path,
        folder: &str,
        chat_id: &str,
    ) -> std::path::PathBuf {
        workspace
            .join(".refact")
            .join("trajectories")
            .join(folder)
            .join(format!("{chat_id}.json"))
    }

    #[tokio::test]
    async fn new_root_chat_saves_into_its_own_folder() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "layout-root-chat";

        save_trajectory_snapshot(
            gcx.clone(),
            test_snapshot(
                chat_id,
                "Root",
                vec![ChatMessage::new("user".to_string(), "hi".to_string())],
            ),
        )
        .await
        .unwrap();

        let nested = nested_trajectory_path(dir.path(), chat_id, chat_id);
        assert!(tokio::fs::try_exists(&nested).await.unwrap());
        let flat = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        assert!(!tokio::fs::try_exists(&flat).await.unwrap());
        assert!(load_trajectory_for_chat(gcx, chat_id).await.is_some());
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn performance_diagnostics_cover_snapshot_save_boundaries_without_raw_id_or_path() {
        let _lock = serial_test_guard();
        let (_guard, sink, recorder) = install_perf_recorder();
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "performance-save-chat";
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                "diagnostic content remains private".to_string(),
            ));
            session.increment_version();
        }

        let snapshot = {
            let session = session_arc.lock().await;
            trajectory_snapshot_from_session(&session)
        };
        save_trajectory_snapshot(gcx, snapshot).await.unwrap();
        drop(app);

        let events = sink.events();
        let components: std::collections::HashSet<_> =
            events.iter().map(|event| event.component).collect();
        assert!(components.contains("trajectory.snapshot"));
        assert!(components.contains("trajectory.serialize"));
        assert!(components.contains("trajectory.atomic_write"));
        assert!(components.contains("trajectory.commit"));
        assert_eq!(
            events
                .iter()
                .filter(|event| event.component == "trajectory.serialize")
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .find(|event| event.component == "trajectory.serialize")
                .expect("serialize metric")
                .elapsed_us,
            2
        );
        assert!(events.iter().all(|event| event.outcome == "success"));
        assert!(events.iter().all(|event| {
            !format!("{event:?}").contains(chat_id)
                && !format!("{event:?}").contains(dir.path().to_str().unwrap())
        }));
        let expected_chat_hash = recorder.hash_identity_for_test(chat_id);
        assert!(events
            .iter()
            .any(|event| { event.chat_id_hash.as_deref() == Some(expected_chat_hash.as_str()) }));
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn trajectory_metadata_path_failure_records_one_serialize_failure() {
        let _lock = serial_test_guard();
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "serialize-path-failure";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(&path, "different-id", "Existing", "2024-01-01T00:00:00Z").await;
        let before = tokio::fs::read_to_string(&path).await.unwrap();
        let (_guard, sink, _) = install_perf_recorder();

        let result = save_trajectory_snapshot(
            gcx,
            test_snapshot(
                chat_id,
                "New",
                vec![ChatMessage::new("user".to_string(), "hi".to_string())],
            ),
        )
        .await;

        assert!(result.is_err());
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), before);
        let serialize_events: Vec<_> = sink
            .events()
            .into_iter()
            .filter(|event| event.component == "trajectory.serialize")
            .collect();
        assert_eq!(serialize_events.len(), 1);
        assert_eq!(serialize_events[0].outcome, "failure");
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn trajectory_message_serialization_failure_preserves_existing_file() {
        let _lock = serial_test_guard();
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "serialize-message-failure";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(&path, chat_id, "Durable", "2024-01-01T00:00:00Z").await;
        let before = tokio::fs::read_to_string(&path).await.unwrap();
        let (_guard, sink, _) = install_perf_recorder();
        let message = ChatMessage {
            message_id: "unserializable-message".to_string(),
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("private message content".to_string()),
            ..Default::default()
        };
        set_test_message_serialization_failure(Some(message.message_id.clone()));

        let error = save_trajectory_snapshot(gcx, test_snapshot(chat_id, "New", vec![message]))
            .await
            .unwrap_err();
        set_test_message_serialization_failure(None);

        assert!(error.contains("index 0"));
        assert!(error.contains("unserializable-message"));
        assert!(error.contains("assistant"));
        assert!(!error.contains("private message content"));
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), before);
        let serialize_events: Vec<_> = sink
            .events()
            .into_iter()
            .filter(|event| event.component == "trajectory.serialize")
            .collect();
        assert_eq!(serialize_events.len(), 1);
        assert_eq!(serialize_events[0].outcome, "failure");
    }

    #[tokio::test]
    async fn trajectory_commit_round_trips_every_message() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "serialize-all-messages";
        let messages = vec![
            ChatMessage {
                message_id: "message-user".to_string(),
                role: "user".to_string(),
                content: ChatContent::SimpleText("question".to_string()),
                ..Default::default()
            },
            ChatMessage {
                message_id: "message-assistant".to_string(),
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("answer".to_string()),
                reasoning_content: Some("reasoning".to_string()),
                tool_calls: Some(vec![crate::call_validation::ChatToolCall {
                    id: "call-1".to_string(),
                    index: Some(0),
                    function: crate::call_validation::ChatToolFunction {
                        name: "sleep".to_string(),
                        arguments: "{}".to_string(),
                    },
                    tool_type: "function".to_string(),
                    extra_content: None,
                    started_at_ms: Some(1_700_000_000_000),
                    completed_at_ms: Some(1_700_000_008_000),
                }]),
                ..Default::default()
            },
            ChatMessage {
                message_id: "message-tool".to_string(),
                role: "tool".to_string(),
                content: ChatContent::SimpleText("result".to_string()),
                tool_call_id: "call-1".to_string(),
                ..Default::default()
            },
        ];
        let expected = serde_json::to_value(&messages).unwrap();

        save_trajectory_snapshot(gcx.clone(), test_snapshot(chat_id, "All", messages))
            .await
            .unwrap();

        let path = nested_trajectory_path(dir.path(), chat_id, chat_id);
        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(saved["messages"], expected);
        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        let tool_call = loaded.messages[1]
            .tool_calls
            .as_ref()
            .unwrap()
            .first()
            .unwrap();
        assert_eq!(tool_call.started_at_ms, Some(1_700_000_000_000));
        assert_eq!(tool_call.completed_at_ms, Some(1_700_000_008_000));
    }

    #[tokio::test]
    async fn subchat_saves_into_root_chat_folder() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let root_id = "layout-parent-chat";
        let child_id = "subchat-layout-child";

        let mut snapshot = test_snapshot(
            child_id,
            "Child",
            vec![ChatMessage::new("user".to_string(), "hi".to_string())],
        );
        snapshot.root_chat_id = Some(root_id.to_string());
        snapshot.parent_id = Some(root_id.to_string());
        snapshot.link_type = Some("subagent".to_string());
        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();

        let nested = nested_trajectory_path(dir.path(), root_id, child_id);
        assert!(
            tokio::fs::try_exists(&nested).await.unwrap(),
            "child must live in the root chat folder"
        );
        assert!(load_trajectory_for_chat(gcx, child_id).await.is_some());
    }

    #[tokio::test]
    async fn resaving_a_trajectory_keeps_the_original_created_at() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "created-at-stable-on-resave";

        let mut first = test_snapshot(
            chat_id,
            "First",
            vec![ChatMessage::new("user".to_string(), "hi".to_string())],
        );
        first.created_at = "2024-01-01T00:00:00Z".to_string();
        save_trajectory_snapshot(gcx.clone(), first).await.unwrap();
        let path = nested_trajectory_path(dir.path(), chat_id, chat_id);
        let first_raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        let first_created_at = first_raw["created_at"].as_str().unwrap().to_string();
        let first_updated_at = first_raw["updated_at"].as_str().unwrap().to_string();

        let mut second = test_snapshot(
            chat_id,
            "Second",
            vec![
                ChatMessage::new("user".to_string(), "hi".to_string()),
                ChatMessage::new("assistant".to_string(), "there".to_string()),
            ],
        );
        second.created_at = "2025-06-01T00:00:00Z".to_string();
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        save_trajectory_snapshot(gcx, second).await.unwrap();
        let second_raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();

        assert_eq!(first_created_at, "2024-01-01T00:00:00Z");
        assert_eq!(second_raw["created_at"].as_str().unwrap(), first_created_at);
        assert_ne!(second_raw["updated_at"].as_str().unwrap(), first_updated_at);
    }

    #[tokio::test]
    async fn resaving_child_without_root_id_updates_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let root_id = "layout-resave-root";
        let child_id = "subchat-layout-resave-child";

        let mut first = test_snapshot(
            child_id,
            "Child",
            vec![ChatMessage::new("user".to_string(), "hi".to_string())],
        );
        first.root_chat_id = Some(root_id.to_string());
        first.parent_id = Some(root_id.to_string());
        first.link_type = Some("subagent".to_string());
        save_trajectory_snapshot(gcx.clone(), first).await.unwrap();

        let mut second = test_snapshot(
            child_id,
            "Child",
            vec![
                ChatMessage::new("user".to_string(), "hi".to_string()),
                ChatMessage::new("assistant".to_string(), "there".to_string()),
            ],
        );
        second.root_chat_id = None;
        second.parent_id = None;
        save_trajectory_snapshot(gcx.clone(), second).await.unwrap();

        let nested = nested_trajectory_path(dir.path(), root_id, child_id);
        let duplicate = nested_trajectory_path(dir.path(), child_id, child_id);
        assert!(tokio::fs::try_exists(&nested).await.unwrap());
        assert!(
            !tokio::fs::try_exists(&duplicate).await.unwrap(),
            "a re-save without root_chat_id must update in place, not fork a duplicate file"
        );

        let loaded = load_trajectory_for_chat(gcx, child_id)
            .await
            .expect("child must still resolve");
        assert_eq!(loaded.messages.len(), 2);
        assert_same_path(&loaded.source_path, &nested);
    }

    #[tokio::test]
    async fn legacy_flat_trajectory_keeps_being_written_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "layout-legacy-chat";
        let flat = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(&flat, chat_id, "Legacy", "2024-01-01T00:00:00Z").await;

        save_trajectory_snapshot(
            gcx.clone(),
            test_snapshot(
                chat_id,
                "Legacy Updated",
                vec![ChatMessage::new("user".to_string(), "hi".to_string())],
            ),
        )
        .await
        .unwrap();

        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&flat).await.unwrap()).unwrap();
        assert_eq!(saved["title"], "Legacy Updated");
        let nested = nested_trajectory_path(dir.path(), chat_id, chat_id);
        assert!(
            !tokio::fs::try_exists(&nested).await.unwrap(),
            "legacy chats must not be duplicated into a folder"
        );
    }

    #[tokio::test]
    async fn nested_trajectory_is_indexed_and_resolvable_through_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "layout-indexed-chat";

        save_trajectory_snapshot(
            gcx.clone(),
            test_snapshot(
                chat_id,
                "Indexed",
                vec![ChatMessage::new("user".to_string(), "hi".to_string())],
            ),
        )
        .await
        .unwrap();

        AppState::from_gcx(gcx.clone())
            .await
            .chat
            .trajectory_index_coordinator
            .flush_all()
            .await
            .unwrap();

        let root = dir.path().join(".refact").join("trajectories");
        assert!(
            !tokio::fs::try_exists(root.join(chat_id).join("index.json"))
                .await
                .unwrap(),
            "index.json must not be created inside a chat folder"
        );

        let index = trajectory_index::read_trajectory_index(&root)
            .await
            .unwrap()
            .expect("root index should exist");
        let entry = index
            .entries
            .iter()
            .find(|entry| entry.id == chat_id)
            .expect("nested chat must be indexed");
        assert_eq!(entry.file_name, format!("{chat_id}/{chat_id}.json"));
        assert!(
            trajectory_index::trajectory_index_entry_is_fresh(&root, entry).await,
            "a nested index entry must not be treated as stale"
        );
        assert_eq!(
            trajectory_index::trajectory_file_path_for_entry(&root, entry),
            nested_trajectory_path(dir.path(), chat_id, chat_id)
        );
    }

    #[tokio::test]
    async fn id_mismatched_legacy_flat_slot_refuses_the_save() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "layout-squatted-chat";

        let root = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&root).await.unwrap();
        tokio::fs::write(
            root.join(format!("{chat_id}.json")),
            serde_json::to_string(&serde_json::json!({
                "id": "some-other-chat",
                "title": "Squatter",
                "created_at": "2026-01-01T00:00:00Z",
                "messages": [],
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let result = save_trajectory_snapshot(
            gcx.clone(),
            test_snapshot(
                chat_id,
                "New",
                vec![ChatMessage::new("user".to_string(), "hi".to_string())],
            ),
        )
        .await;

        assert!(
            result.is_err(),
            "an id-mismatched legacy flat slot must refuse the save, not write elsewhere"
        );
        assert!(
            !tokio::fs::try_exists(nested_trajectory_path(dir.path(), chat_id, chat_id))
                .await
                .unwrap(),
            "refusal must not leave a nested copy behind"
        );
    }

    #[tokio::test]
    async fn internal_traces_are_not_written_into_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let root_id = "layout-trace-owner";
        let trace_id = "subchat-layout-trace";

        save_trajectory_snapshot(
            gcx.clone(),
            test_snapshot(
                root_id,
                "Owner",
                vec![ChatMessage::new("user".to_string(), "hi".to_string())],
            ),
        )
        .await
        .unwrap();

        let mut trace = test_snapshot(
            trace_id,
            "Trace",
            vec![ChatMessage::new("user".to_string(), "hi".to_string())],
        );
        trace.root_chat_id = Some(root_id.to_string());
        trace.link_type = Some(internal_trace_link_type("title_generation"));
        save_trajectory_snapshot(gcx.clone(), trace).await.unwrap();

        AppState::from_gcx(gcx.clone())
            .await
            .chat
            .trajectory_index_coordinator
            .flush_all()
            .await
            .unwrap();

        assert!(
            tokio::fs::try_exists(nested_trajectory_path(dir.path(), root_id, trace_id))
                .await
                .unwrap(),
            "the trace must still be written into the owning conversation folder"
        );

        let root = dir.path().join(".refact").join("trajectories");
        let index = trajectory_index::read_trajectory_index(&root)
            .await
            .unwrap()
            .expect("root index should exist");
        assert!(
            index.entries.iter().any(|entry| entry.id == root_id),
            "the owning chat stays indexed"
        );
        assert!(
            !index.entries.iter().any(|entry| entry.id == trace_id),
            "internal traces must not grow the index"
        );
    }

    fn write_temp_json(dir: &tempfile::TempDir, name: &str, contents: &str) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn top_level_link_type_classification_preserves_semantics() {
        let temp = tempfile::tempdir().unwrap();

        // Happy path: a top-level internal link_type is classified as an internal trace.
        assert!(parsed_top_level_link_type_is_internal(write_temp_json(
            &temp,
            "internal.json",
            r#"{"id":"a","link_type":"internal:title_generation","messages":[]}"#,
        )));

        // A link_type that is not the internal one.
        assert!(!parsed_top_level_link_type_is_internal(write_temp_json(
            &temp,
            "subagent.json",
            r#"{"id":"a","link_type":"subagent","messages":[]}"#,
        )));

        // No link_type field at all.
        assert!(!parsed_top_level_link_type_is_internal(write_temp_json(
            &temp,
            "missing.json",
            r#"{"id":"a","messages":[]}"#,
        )));

        // A null link_type is treated as absent, not internal.
        assert!(!parsed_top_level_link_type_is_internal(write_temp_json(
            &temp,
            "null.json",
            r#"{"id":"a","link_type":null}"#,
        )));

        // Not valid JSON.
        assert!(!parsed_top_level_link_type_is_internal(write_temp_json(
            &temp,
            "corrupt.json",
            "{not json at all",
        )));

        // A truncated document whose link_type *is* internal must still be rejected, so that a
        // torn write is never mistaken for a complete internal trace and pruned.
        assert!(!parsed_top_level_link_type_is_internal(write_temp_json(
            &temp,
            "truncated.json",
            r#"{"link_type":"internal:broken""#,
        )));

        // Valid JSON whose top level is not a map.
        assert!(!parsed_top_level_link_type_is_internal(write_temp_json(
            &temp,
            "array.json",
            r#"["link_type","internal:title_generation"]"#,
        )));
        assert!(!parsed_top_level_link_type_is_internal(write_temp_json(
            &temp,
            "scalar.json",
            r#""internal:title_generation""#,
        )));

        // A nested link_type must not be read as a top-level one.
        assert!(!parsed_top_level_link_type_is_internal(write_temp_json(
            &temp,
            "nested.json",
            r#"{"messages":[{"role":"user","content":"\"link_type\": \"internal:spoof\""}]}"#,
        )));

        // A missing file, and a directory, are both "not an internal trace".
        assert!(!parsed_top_level_link_type_is_internal(
            temp.path().join("does-not-exist.json")
        ));
        assert!(!parsed_top_level_link_type_is_internal(
            temp.path().to_path_buf()
        ));
    }

    #[tokio::test]
    async fn internal_trace_pruning_keeps_newest_and_preserves_displayable_files() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let displayable = dir.join("displayable.json");
        tokio::fs::write(
            &displayable,
            serde_json::json!({"id":"displayable","link_type":"subagent"}).to_string(),
        )
        .await
        .unwrap();
        for index in 0..4 {
            let path = dir.join(format!("internal-{index}.json"));
            let payload = if index == 0 {
                format!(
                    "{{\"id\":\"internal-0\",\"messages\":[{{\"role\":\"user\",\"content\":\"{}\"}}],\"link_type\":\"internal:title_generation\"}}",
                    "x".repeat((64 * 1024) + 1024)
                )
            } else {
                serde_json::json!({
                    "id": format!("internal-{index}"),
                    "messages": [{"role":"user","content":"trace"}],
                    "link_type": "internal:title_generation"
                })
                .to_string()
            };
            tokio::fs::write(&path, payload).await.unwrap();
            let modified = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(index + 1);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(modified)
                .unwrap();
        }

        let removed = prune_internal_traces_in_folder(dir, 2).await.unwrap();

        assert_eq!(removed, 2);
        assert!(displayable.exists());
        assert!(!dir.join("internal-0.json").exists());
        assert!(!dir.join("internal-1.json").exists());
        assert!(dir.join("internal-2.json").exists());
        assert!(dir.join("internal-3.json").exists());
    }

    #[tokio::test]
    async fn internal_trace_pruning_ignores_message_content_and_invalid_json() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let user_trajectory = dir.join("user.json");
        let corrupt = dir.join("corrupt.json");
        let truncated = dir.join("truncated.json");
        let old_trace = dir.join("old-trace.json");
        let new_trace = dir.join("new-trace.json");

        tokio::fs::write(
            &user_trajectory,
            serde_json::json!({
                "id": "user",
                "messages": [{"role": "user", "content": "\"link_type\": \"internal:spoof\""}]
            })
            .to_string(),
        )
        .await
        .unwrap();
        tokio::fs::write(&corrupt, "{\"link_type\":\"internal:broken\"")
            .await
            .unwrap();
        tokio::fs::write(&truncated, "{\"messages\":[")
            .await
            .unwrap();
        for (path, id, modified_secs) in
            [(&old_trace, "old-trace", 1), (&new_trace, "new-trace", 2)]
        {
            tokio::fs::write(
                path,
                serde_json::json!({
                    "id": id,
                    "link_type": "internal:title_generation",
                    "messages": []
                })
                .to_string(),
            )
            .await
            .unwrap();
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(modified_secs))
                .unwrap();
        }

        let removed = prune_internal_traces_in_folder(dir, 1).await.unwrap();

        assert_eq!(removed, 1);
        assert!(user_trajectory.exists());
        assert!(corrupt.exists());
        assert!(truncated.exists());
        assert!(!old_trace.exists());
        assert!(new_trace.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn internal_trace_pruning_skips_symlinked_entries() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let outside_trace = outside.path().join("outside.json");
        let real_trace = dir.join("real.json");
        tokio::fs::write(
            &outside_trace,
            serde_json::json!({"id": "outside", "link_type": "internal:test"}).to_string(),
        )
        .await
        .unwrap();
        tokio::fs::write(
            &real_trace,
            serde_json::json!({"id": "real", "link_type": "internal:test"}).to_string(),
        )
        .await
        .unwrap();
        std::os::unix::fs::symlink(&outside_trace, dir.join("linked.json")).unwrap();

        let removed = prune_internal_traces_in_folder(dir, 0).await.unwrap();

        assert_eq!(removed, 1);
        assert!(!real_trace.exists());
        assert!(outside_trace.exists());
        assert!(dir.join("linked.json").exists());
    }

    fn assert_prune_throttle_is_bounded_and_isolates_directories() {
        let mut throttle = PruneThrottle::default();
        let left = PathBuf::from("/tmp/left");
        let right = PathBuf::from("/tmp/right");

        assert!(throttle.claim(&left, 1, 3600));
        assert!(throttle.claim(&right, 1, 3600));
        assert!(!throttle.claim(&left, 1, 3600));

        for index in 0..PRUNE_THROTTLE_MAX_ENTRIES {
            assert!(throttle.claim(&PathBuf::from(format!("/tmp/prune-{index}")), 1, 3600));
        }

        assert!(throttle.entries.len() <= PRUNE_THROTTLE_MAX_ENTRIES);
        assert!(throttle.claim(&right, 3601, 3600));
    }

    #[test]
    fn index_entry_file_names_accept_one_folder_level_only() {
        assert!(trajectory_index::index_entry_file_name_is_valid("a.json"));
        assert!(trajectory_index::index_entry_file_name_is_valid(
            "root/a.json"
        ));
        assert!(!trajectory_index::index_entry_file_name_is_valid(
            "a/b/c.json"
        ));
        assert!(!trajectory_index::index_entry_file_name_is_valid(
            "../a.json"
        ));
        assert!(!trajectory_index::index_entry_file_name_is_valid(
            "root/index.json"
        ));
        assert!(!trajectory_index::index_entry_file_name_is_valid("a.txt"));
        assert!(!trajectory_index::index_entry_file_name_is_valid(
            "C:/evil.json"
        ));
        assert!(!trajectory_index::index_entry_file_name_is_valid(
            "/etc/evil.json"
        ));
    }

    #[test]
    fn internal_trace_events_are_not_displayable_chats() {
        let event_with_link_type = |link_type: Option<String>| -> TrajectoryEvent {
            let mut value = serde_json::json!({
                "type": "trajectory_updated",
                "id": "trace-chat",
            });
            if let Some(link_type) = link_type {
                value["link_type"] = serde_json::Value::String(link_type);
            }
            serde_json::from_value(value).unwrap()
        };

        assert!(!trajectory_event_is_displayable_chat(
            &event_with_link_type(Some(internal_trace_link_type("title_generation")))
        ));
        assert!(trajectory_event_is_displayable_chat(&event_with_link_type(
            None
        )));
    }

    #[tokio::test]
    async fn task_agent_index_stays_in_its_own_agent_folder() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let agent_dir = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-index-scope")
            .join("trajectories")
            .join("agents")
            .join("agent-7");
        tokio::fs::create_dir_all(&agent_dir).await.unwrap();

        let resolved = index_dir_for_trajectory_file(gcx, &agent_dir.join("agent-chat.json"))
            .await
            .expect("index dir must resolve");

        assert_eq!(
            resolved, agent_dir,
            "a task agent index must not be relocated to the shared agents/ folder"
        );
    }

    #[tokio::test]
    async fn remove_stale_entries_targets_only_the_owning_index_directory() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;

        let root_dir = dir.path().join(".refact").join("trajectories");
        let task_dir = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-1")
            .join("trajectories");
        tokio::fs::create_dir_all(&root_dir).await.unwrap();
        tokio::fs::create_dir_all(&task_dir).await.unwrap();

        let stale_id = "scoped-stale-entry-chat";
        let stale_path = root_dir.join(format!("{stale_id}.json"));
        let stale_value = serde_json::json!({
            "id": stale_id,
            "title": "Stale",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:01Z",
            "model": "test-model",
            "mode": "agent",
            "tool_use": "agent",
            "messages": []
        });
        tokio::fs::write(&stale_path, stale_value.to_string())
            .await
            .unwrap();
        trajectory_index::upsert_trajectory_index_entry_from_owned_value_with_rollout(
            &app.chat.trajectory_index_coordinator,
            &root_dir,
            &stale_path,
            stale_value,
            None,
        )
        .await
        .unwrap();
        tokio::fs::remove_file(&stale_path).await.unwrap();

        let fresh_id = "other-dir-chat";
        let fresh_path = task_dir.join(format!("{fresh_id}.json"));
        let fresh_value = serde_json::json!({
            "id": fresh_id,
            "title": "Fresh",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:01Z",
            "model": "test-model",
            "mode": "agent",
            "tool_use": "agent",
            "messages": []
        });
        tokio::fs::write(&fresh_path, fresh_value.to_string())
            .await
            .unwrap();
        trajectory_index::upsert_trajectory_index_entry_from_owned_value_with_rollout(
            &app.chat.trajectory_index_coordinator,
            &task_dir,
            &fresh_path,
            fresh_value,
            None,
        )
        .await
        .unwrap();

        remove_stale_trajectory_index_entries(gcx.clone(), stale_id).await;

        let root_entries = trajectory_index::list_trajectory_entries_with_rollout_for(
            &app.chat.trajectory_index_coordinator,
            &root_dir,
            None,
            trajectory_index::TrajectoryIndexListingCaller::StaleEntryCleanup,
        )
        .await
        .unwrap();
        let task_entries = trajectory_index::list_trajectory_entries_with_rollout_for(
            &app.chat.trajectory_index_coordinator,
            &task_dir,
            None,
            trajectory_index::TrajectoryIndexListingCaller::StaleEntryCleanup,
        )
        .await
        .unwrap();

        assert!(root_entries.iter().all(|entry| entry.id != stale_id));
        assert_eq!(task_entries.len(), 1);
        assert_eq!(task_entries[0].id, fresh_id);
    }

    fn listing_calls_for(
        app: &AppState,
        caller: trajectory_index::TrajectoryIndexListingCaller,
    ) -> u64 {
        app.chat
            .trajectory_index_coordinator
            .listing_counters_for_test()
            .calls_by_caller
            .iter()
            .find(|(name, _)| *name == caller.as_str())
            .map(|(_, calls)| *calls)
            .unwrap_or(0)
    }

    async fn seed_agent_task_dirs(workspace: &Path, count: usize) -> Vec<PathBuf> {
        let mut dirs = Vec::with_capacity(count);
        for number in 0..count {
            let dir = workspace
                .join(".refact")
                .join("tasks")
                .join(format!("task-{number}"))
                .join("trajectories")
                .join("agents");
            let id = format!("agent-chat-{number}");
            write_trajectory_file(
                &dir.join(format!("{id}.json")),
                &id,
                "Agent",
                "2024-01-01T00:00:01Z",
            )
            .await;
            dirs.push(dir);
        }
        dirs
    }

    async fn sync_every_trajectory_dir(gcx: &Arc<GlobalContext>, app: &AppState) -> Vec<PathBuf> {
        let dirs = list_trajectory_dirs(gcx).await;
        for dir in &dirs {
            trajectory_index::list_trajectory_entries_with_rollout_shared_for(
                &app.chat.trajectory_index_coordinator,
                dir,
                None,
                trajectory_index::TrajectoryIndexListingCaller::Sidebar,
            )
            .await
            .unwrap();
        }
        assert!(app
            .chat
            .trajectory_index_coordinator
            .unsynced_directories(&dirs)
            .is_empty());
        dirs
    }

    #[tokio::test]
    async fn direct_chat_path_lookup_does_not_list_or_rebuild_index() {
        for own_folder in [false, true] {
            let workspace = tempfile::tempdir().unwrap();
            let (gcx, app) = make_app_with_workspace(workspace.path()).await;
            let chat_id = "direct-with-broken-index";
            let root = workspace.path().join(".refact/trajectories");
            let expected = if own_folder {
                root.join(chat_id).join(format!("{chat_id}.json"))
            } else {
                root.join(format!("{chat_id}.json"))
            };
            write_trajectory_file(&expected, chat_id, "Direct", "2024-01-01T00:00:01Z").await;
            let index_path = root.join(trajectory_index::TRAJECTORY_INDEX_FILE);
            let broken_index = "{broken index";
            tokio::fs::write(&index_path, broken_index).await.unwrap();
            let before = listing_calls_for(
                &app,
                trajectory_index::TrajectoryIndexListingCaller::ChatPathLookup,
            );

            let found = find_trajectory_file(gcx.clone(), chat_id).await.unwrap();

            assert_same_path(&found.path, &expected);
            assert_eq!(found.json["title"], "Direct");
            assert_eq!(
                listing_calls_for(
                    &app,
                    trajectory_index::TrajectoryIndexListingCaller::ChatPathLookup,
                ),
                before
            );
            assert_eq!(
                tokio::fs::read_to_string(&index_path).await.unwrap(),
                broken_index
            );
            let normal_dirs = get_all_trajectories_dirs(gcx).await;
            assert_eq!(
                app.chat
                    .trajectory_index_coordinator
                    .unsynced_directories(&normal_dirs),
                normal_dirs
            );
        }
    }

    #[tokio::test]
    async fn chat_path_lookup_preserves_flat_own_folder_and_indexed_priority() {
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(workspace.path()).await;
        let chat_id = "direct-priority";
        let root = workspace.path().join(".refact/trajectories");
        let flat = root.join(format!("{chat_id}.json"));
        let own = root.join(chat_id).join(format!("{chat_id}.json"));
        let indexed = root.join("other-root").join(format!("{chat_id}.json"));
        let task = workspace
            .path()
            .join(".refact/tasks/priority-task/trajectories/agents")
            .join(format!("{chat_id}.json"));
        for path in [&flat, &own, &indexed, &task] {
            write_trajectory_file(path, chat_id, "Valid", "2024-01-01T00:00:01Z").await;
        }

        assert_same_optional_path(find_trajectory_path(gcx.clone(), chat_id).await, &flat);
        tokio::fs::write(&flat, "{corrupt").await.unwrap();
        assert_same_optional_path(find_trajectory_path(gcx.clone(), chat_id).await, &own);
        tokio::fs::write(&own, "{corrupt").await.unwrap();
        assert_same_optional_path(find_trajectory_path(gcx.clone(), chat_id).await, &indexed);
        tokio::fs::write(&indexed, "{corrupt").await.unwrap();
        assert_same_optional_path(find_trajectory_path(gcx, chat_id).await, &task);
    }

    #[tokio::test]
    async fn chat_path_lookup_still_finds_chat_in_unsynced_dir() {
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(workspace.path()).await;
        write_trajectory_file(
            &workspace
                .path()
                .join(".refact")
                .join("trajectories")
                .join("root-chat.json"),
            "root-chat",
            "Root",
            "2024-01-01T00:00:01Z",
        )
        .await;
        let chat_id = "unsynced-agent-chat";
        let task_dir = workspace
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-1")
            .join("trajectories")
            .join("agents");
        let expected = task_dir.join(format!("{chat_id}.json"));
        write_trajectory_file(&expected, chat_id, "Agent", "2024-01-01T00:00:01Z").await;

        let dirs = list_trajectory_dirs(&gcx).await;
        assert!(!app
            .chat
            .trajectory_index_coordinator
            .unsynced_directories(&dirs)
            .is_empty());
        let normal_dirs = get_all_trajectories_dirs(gcx.clone()).await.len() as u64;
        let before = listing_calls_for(
            &app,
            trajectory_index::TrajectoryIndexListingCaller::ChatPathLookup,
        );

        let found = find_trajectory_path(gcx.clone(), chat_id)
            .await
            .expect("an unsynced directory must still be searched");
        let swept = listing_calls_for(
            &app,
            trajectory_index::TrajectoryIndexListingCaller::ChatPathLookup,
        ) - before;

        assert_eq!(
            tokio::fs::canonicalize(&found).await.unwrap(),
            tokio::fs::canonicalize(&expected).await.unwrap()
        );
        assert!(
            swept > normal_dirs,
            "the directory the map cannot answer for must still be listed"
        );
    }

    #[tokio::test]
    async fn missing_chat_lookup_does_not_reconcile_unrelated_task_indexes() {
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(workspace.path()).await;
        let dirs = seed_agent_task_dirs(workspace.path(), 1).await;
        assert_eq!(dirs.len(), 1);
        for dir in &dirs {
            assert!(dir.is_dir());
            assert!(!dir.join("index.json").exists());
        }
        assert!(find_trajectory_path(gcx, "missing-agent-chat")
            .await
            .is_none());
        for dir in dirs {
            assert!(
                !dir.join("index.json").exists(),
                "a missing chat lookup must not rebuild an unrelated task index"
            );
        }
    }

    #[cfg(feature = "perf-tests")]
    #[tokio::test]
    async fn perf_trajectory_lookup_many_unrelated_dirs() {
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(workspace.path()).await;
        let dirs = seed_agent_task_dirs(workspace.path(), 2000).await;
        let chat_id = "perf-normal-chat";
        let path = workspace
            .path()
            .join(".refact/trajectories")
            .join(chat_id)
            .join(format!("{chat_id}.json"));
        write_trajectory_file(&path, chat_id, "Normal", "2024-01-01T00:00:01Z").await;
        for number in 0..1000 {
            std::fs::create_dir_all(
                workspace
                    .path()
                    .join(".refact/trajectories")
                    .join(format!("unrelated-{number}")),
            )
            .unwrap();
        }
        let started = Instant::now();
        assert_eq!(
            index_dir_for_trajectory_file(gcx.clone(), &path).await,
            path.parent().and_then(Path::parent).map(Path::to_path_buf)
        );
        eprintln!(
            "trajectory_perf index_root_us={}",
            started.elapsed().as_micros()
        );
        let started = Instant::now();
        assert!(find_trajectory_or_buddy_file(gcx.clone(), chat_id)
            .await
            .is_some());
        eprintln!(
            "trajectory_perf restore_hit_us={}",
            started.elapsed().as_micros()
        );
        let started = Instant::now();
        assert!(
            find_trajectory_or_buddy_file(gcx.clone(), "perf-missing-chat")
                .await
                .is_none()
        );
        eprintln!(
            "trajectory_perf restore_miss_us={}",
            started.elapsed().as_micros()
        );
        let started = Instant::now();
        assert!(find_trajectory_file(gcx, "perf-missing-chat")
            .await
            .is_none());
        eprintln!(
            "trajectory_perf path_miss_us={} task_dirs={}",
            started.elapsed().as_micros(),
            dirs.len()
        );
    }

    // Clears the process-wide metadata cache, so it must not run beside the tests that read it.

    #[test]
    fn internal_traces_are_not_displayable_chats() {
        let internal: TrajectoryListData = serde_json::from_value(serde_json::json!({
            "id": "internal-1",
            "updated_at": "2024-01-01T00:00:00Z",
            "mode": "agent",
            "link_type": "internal:title_generation"
        }))
        .unwrap();
        assert!(!trajectory_list_data_is_displayable_chat(&internal));

        let plain: TrajectoryListData = serde_json::from_value(serde_json::json!({
            "id": "plain-1",
            "updated_at": "2024-01-01T00:00:00Z",
            "mode": "agent"
        }))
        .unwrap();
        assert!(trajectory_list_data_is_displayable_chat(&plain));
    }

    fn test_snapshot(chat_id: &str, title: &str, messages: Vec<ChatMessage>) -> TrajectorySnapshot {
        TrajectorySnapshot {
            recovery: None,
            recovery_policy: None,
            pending_questions: Vec::new(),
            native_decision_receipts: Vec::new(),
            goal: None,
            goal_ledger: Vec::new(),
            pending_deliveries: Vec::new(),
            dropped_deliveries: Vec::new(),
            goal_verification_blocked_until_ms: None,
            pending_browser_decision: None,
            chat_id: chat_id.to_string(),
            title: title.to_string(),
            model: "model".to_string(),
            mode: "agent".to_string(),
            tool_use: "agent".to_string(),
            messages,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            boost_reasoning: false,
            checkpoints_enabled: true,
            context_tokens_cap: None,
            auto_compression_cap: None,
            auto_compression_cap_pending: false,
            include_project_info: true,
            is_title_generated: true,
            auto_approve_editing_tools: false,
            auto_approve_dangerous_commands: false,
            autonomous_no_confirm: false,
            version: 1,
            task_meta: None,
            worktree: None,
            parent_id: None,
            link_type: None,
            root_chat_id: None,
            reasoning_effort: None,
            thinking_budget: None,
            temperature: None,
            frequency_penalty: None,
            max_tokens: None,
            parallel_tool_calls: None,
            previous_response_id: None,
            active_skill: None,
            auto_enrichment_enabled: None,
            buddy_meta: None,
            auto_compact_enabled: None,
            frozen_request_prefix: None,
            claude_code_identity: None,
            reactive_compact_attempts: None,
            wake_up_at: None,
            waiting_for_card_ids: Vec::new(),
            app_workspace: None,
        }
    }

    async fn wait_for_trajectory_event(
        rx: &mut broadcast::Receiver<TrajectoryEvent>,
        id: &str,
    ) -> TrajectoryEvent {
        // Generous timeout: file-watcher notify events can be delayed under
        // heavy parallel test load (notify backend + tokio scheduler contention),
        // especially on macOS CI where FSEvents delivery can lag under cargo test load.
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            loop {
                match rx.recv().await {
                    Ok(event) if event.id == id => return event,
                    Ok(_) => continue,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(err) => panic!("trajectory event channel closed: {}", err),
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for trajectory event {id}"))
    }

    async fn drain_trajectory_events(rx: &mut broadcast::Receiver<TrajectoryEvent>) {
        loop {
            match tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await {
                Ok(Ok(_)) | Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => break,
            }
        }
    }

    async fn assert_no_trajectory_event_for(
        rx: &mut broadcast::Receiver<TrajectoryEvent>,
        duration: std::time::Duration,
    ) {
        match tokio::time::timeout(duration, rx.recv()).await {
            Err(_) | Ok(Err(broadcast::error::RecvError::Closed)) => {}
            Ok(Ok(event)) => panic!("unexpected trajectory event: {:?}", event),
            Ok(Err(broadcast::error::RecvError::Lagged(skipped))) => {
                panic!("unexpected trajectory event lag, skipped {skipped}")
            }
        }
    }

    async fn assert_no_chat_event_for(
        rx: &mut broadcast::Receiver<Arc<String>>,
        duration: std::time::Duration,
    ) {
        match tokio::time::timeout(duration, rx.recv()).await {
            Err(_) | Ok(Err(broadcast::error::RecvError::Closed)) => {}
            Ok(Ok(event)) => panic!("unexpected chat event: {event}"),
            Ok(Err(broadcast::error::RecvError::Lagged(skipped))) => {
                panic!("unexpected chat event lag, skipped {skipped}")
            }
        }
    }

    async fn wait_for_file_title(path: &Path, expected_title: &str) {
        let mut last_title = None;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Ok(content) = tokio::fs::read_to_string(path).await {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                        last_title = json
                            .get("title")
                            .and_then(|value| value.as_str())
                            .map(ToString::to_string);
                        let generated = json
                            .get("isTitleGenerated")
                            .and_then(|value| value.as_bool())
                            .unwrap_or(false);
                        if last_title.as_deref() == Some(expected_title) && generated {
                            return;
                        }
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "timed out waiting for title {expected_title:?}, last title was {:?}",
                last_title
            )
        });
    }

    async fn wait_for_session_title(session_arc: &Arc<AMutex<ChatSession>>, expected_title: &str) {
        let mut last_title = None;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                {
                    let session = session_arc.lock().await;
                    last_title = Some(session.thread.title.clone());
                    if session.thread.title == expected_title && session.thread.is_title_generated {
                        return;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "timed out waiting for session title {expected_title:?}, last title was {:?}",
                last_title
            )
        });
    }

    fn trajectory_worktree_sample() -> WorktreeMeta {
        trajectory_worktree_sample_with_id("wt-1")
    }

    fn trajectory_worktree_sample_with_id(id: &str) -> WorktreeMeta {
        WorktreeMeta {
            id: id.to_string(),
            kind: "task_agent".to_string(),
            root: std::path::PathBuf::from("/tmp/refact-wt"),
            source_workspace_root: std::path::PathBuf::from("/tmp/refact-src"),
            repo_root: std::path::PathBuf::from("/tmp/refact-src"),
            branch: Some("refact/task/card".to_string()),
            base_branch: Some("main".to_string()),
            base_commit: Some("abc123".to_string()),
            task_id: Some("task-1".to_string()),
            card_id: Some("card-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            enforce: true,
        }
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn watcher_picks_up_external_edit_to_task_planner_trajectory() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();

        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let watcher = TestTrajectoryWatcher(Some(spawn_trajectory_watcher(
            gcx.clone(),
            Some(ready_tx),
            None,
        )));
        wait_for_watcher_start(ready_rx, &dir.path().join(".refact").join("tasks")).await;
        drain_trajectory_events(&mut rx).await;

        let chat_id = "planner-watch-chat";
        let path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-watch")
            .join("trajectories")
            .join("planner")
            .join(format!("{}.json", chat_id));
        write_trajectory_file(&path, chat_id, "Planner Watch", "2024-01-01T00:00:01Z").await;
        if cfg!(target_os = "macos") {
            process_trajectory_change_for_source(
                gcx.clone(),
                chat_id,
                false,
                Some(trajectory_source_identity_from_path(
                    &path,
                    &get_all_task_roots(gcx.clone()).await,
                )),
            )
            .await;
        }

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "updated");
        assert_eq!(event.title.as_deref(), Some("Planner Watch"));
        watcher.stop().await;
    }

    #[test]
    fn trajectory_dispatch_ignores_generated_index_files() {
        let dir = tempfile::tempdir().unwrap();
        let task_root = dir.path().join(".refact").join("tasks");
        std::fs::create_dir_all(&task_root).unwrap();
        let task_index = task_root
            .join("task-index")
            .join("trajectories")
            .join("agents")
            .join("agent-1")
            .join("index.json");
        let task_chat = task_index.with_file_name("chat-1.json");
        let normal_index = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join("index.json");

        assert!(!should_dispatch_trajectory_path(
            &task_index,
            &[task_root.clone()]
        ));
        assert!(!should_dispatch_trajectory_path(
            &normal_index,
            &[task_root.clone()]
        ));
        assert!(should_dispatch_trajectory_path(&task_chat, &[task_root]));
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn watcher_picks_up_new_task_dir_created_after_startup() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();

        let (ready_tx, mut ready_rx) = tokio::sync::oneshot::channel();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let watcher = TestTrajectoryWatcher(Some(spawn_trajectory_watcher(
            gcx.clone(),
            Some(ready_tx),
            Some(TrajectoryWatcherRegistrationGate {
                entered: entered_tx,
                release: release_rx,
            }),
        )));
        tokio::time::timeout(Duration::from_secs(60), entered_rx)
            .await
            .expect("trajectory watcher did not enter registration gate")
            .expect("trajectory watcher exited before registration gate");
        assert!(
            futures::poll!(&mut ready_rx).is_pending(),
            "registration readiness must remain pending until watch registration completes"
        );
        release_tx.send(()).unwrap();
        let task_root = dir.path().join(".refact").join("tasks");
        wait_for_watcher_start(ready_rx, &task_root).await;
        assert!(!task_root.join("task-created-later").exists());
        drain_trajectory_events(&mut rx).await;

        let chat_id = "new-task-agent-chat";
        let path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-created-later")
            .join("trajectories")
            .join("agents")
            .join("agent-1")
            .join(format!("{}.json", chat_id));
        write_trajectory_file(&path, chat_id, "New Task Agent", "2024-01-01T00:00:02Z").await;
        if cfg!(target_os = "macos") {
            process_trajectory_change_for_source(
                gcx.clone(),
                chat_id,
                false,
                Some(trajectory_source_identity_from_path(
                    &path,
                    &get_all_task_roots(gcx.clone()).await,
                )),
            )
            .await;
        }

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "updated");
        assert_eq!(event.title.as_deref(), Some("New Task Agent"));
        watcher.stop().await;
    }

    #[tokio::test]
    async fn paginated_list_includes_task_planner_and_agent_trajectories() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let root = dir.path().join(".refact");

        write_trajectory_file(
            &root.join("trajectories").join("project-chat.json"),
            "project-chat",
            "Project Chat",
            "2024-01-01T00:00:03Z",
        )
        .await;
        write_trajectory_file(
            &root
                .join("tasks")
                .join("task-list")
                .join("trajectories")
                .join("planner")
                .join("planner-chat.json"),
            "planner-chat",
            "Planner Chat",
            "2024-01-01T00:00:02Z",
        )
        .await;
        write_trajectory_file(
            &root
                .join("tasks")
                .join("task-list")
                .join("trajectories")
                .join("agents")
                .join("agent-1")
                .join("agent-chat.json"),
            "agent-chat",
            "Agent Chat",
            "2024-01-01T00:00:01Z",
        )
        .await;

        let response = handle_v1_trajectories_list(
            State(app),
            axum::extract::Query(TrajectoriesListQuery {
                limit: Some(10),
                cursor: None,
                displayable_only: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let items = payload["items"].as_array().unwrap();
        let ids: std::collections::HashSet<_> = items
            .iter()
            .filter_map(|item| item["id"].as_str())
            .collect();

        assert_eq!(payload["total_count"].as_u64(), Some(3));
        assert!(ids.contains("project-chat"));
        assert!(ids.contains("planner-chat"));
        assert!(ids.contains("agent-chat"));

        let planner = items
            .iter()
            .find(|item| item["id"].as_str() == Some("planner-chat"))
            .unwrap();
        assert_eq!(planner["task_id"].as_str(), Some("task-list"));
        assert_eq!(planner["task_role"].as_str(), Some("planner"));

        let agent = items
            .iter()
            .find(|item| item["id"].as_str() == Some("agent-chat"))
            .unwrap();
        assert_eq!(agent["task_id"].as_str(), Some("task-list"));
        assert_eq!(agent["task_role"].as_str(), Some("agents"));
        assert_eq!(agent["agent_id"].as_str(), Some("agent-1"));
    }

    #[tokio::test]
    async fn paginated_list_displayable_only_filters_task_and_subagent_trajectories() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let root = dir.path().join(".refact");

        write_trajectory_file(
            &root.join("trajectories").join("project-chat.json"),
            "project-chat",
            "Project Chat",
            "2024-01-01T00:00:05Z",
        )
        .await;

        let subagent_path = root.join("trajectories").join("subagent-chat.json");
        let mut subagent =
            sample_trajectory("subagent-chat", "Subagent Chat", "2024-01-01T00:00:04Z");
        subagent["parent_id"] = json!("project-chat");
        subagent["link_type"] = json!("subagent");
        tokio::fs::create_dir_all(subagent_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&subagent_path, serde_json::to_string(&subagent).unwrap())
            .await
            .unwrap();

        let planner_path = root
            .join("tasks")
            .join("task-list")
            .join("trajectories")
            .join("planner")
            .join("planner-chat.json");
        let mut planner =
            sample_trajectory("planner-chat", "Planner Chat", "2024-01-01T00:00:035Z");
        planner["mode"] = json!("task_planner");
        planner["task_meta"] = json!({ "task_id": "task-list", "role": "planner" });
        tokio::fs::create_dir_all(planner_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&planner_path, serde_json::to_string(&planner).unwrap())
            .await
            .unwrap();

        let review_path = root
            .join("tasks")
            .join("task-list")
            .join("trajectories")
            .join("planner")
            .join("review-chat.json");
        let mut review = sample_trajectory("review-chat", "Review Chat", "2024-01-01T00:00:045Z");
        review["mode"] = json!("review");
        review["parent_id"] = json!("planner-chat");
        review["link_type"] = json!("mode_transition");
        review["task_meta"] = json!({ "task_id": "task-list", "role": "planner" });
        tokio::fs::write(&review_path, serde_json::to_string(&review).unwrap())
            .await
            .unwrap();

        let task_path = root
            .join("trajectories")
            .join("legacy-task-agent-chat.json");
        let mut legacy_task = sample_trajectory(
            "legacy-task-agent-chat",
            "Legacy Task Agent Chat",
            "2024-01-01T00:00:02Z",
        );
        legacy_task["mode"] = json!("task_agent");
        tokio::fs::write(&task_path, serde_json::to_string(&legacy_task).unwrap())
            .await
            .unwrap();

        let buddy_path = root.join("trajectories").join("buddy-chat.json");
        let mut buddy = sample_trajectory("buddy-chat", "Buddy Chat", "2024-01-01T00:00:015Z");
        buddy["buddy_meta"] = json!({ "is_buddy_chat": true });
        tokio::fs::write(&buddy_path, serde_json::to_string(&buddy).unwrap())
            .await
            .unwrap();

        let missing_link_path = root.join("trajectories").join("missing-link-child.json");
        let mut missing_link_child = sample_trajectory(
            "missing-link-child",
            "Missing Link Child",
            "2024-01-01T00:00:01Z",
        );
        missing_link_child["parent_id"] = json!("project-chat");
        tokio::fs::write(
            &missing_link_path,
            serde_json::to_string(&missing_link_child).unwrap(),
        )
        .await
        .unwrap();

        let task_dir = root.join("tasks").join("task-list");
        let task_record = serde_json::from_value(json!({
            "id":"task-list", "name":"legacy planner copies", "status":"active",
            "created_at":"2024-01-01T00:00:00Z", "updated_at":"2024-01-01T00:00:00Z"
        }))
        .unwrap();
        crate::tasks::storage::save_task_meta(gcx.clone(), "task-list", &task_record)
            .await
            .unwrap();
        let board =
            serde_json::from_value(json!({"owner_chat_id":"planner-chat", "cards":[]})).unwrap();
        crate::tasks::storage::save_board(gcx.clone(), "task-list", &board)
            .await
            .unwrap();
        review["mode"] = json!("solo");
        tokio::fs::write(&review_path, serde_json::to_string(&review).unwrap())
            .await
            .unwrap();
        let linked = load_trajectory_for_chat(gcx.clone(), "review-chat")
            .await
            .unwrap();
        assert_eq!(linked.thread.task_meta.as_ref().unwrap().role, "linked");
        assert!(!chat_id_is_planner_for_task(gcx.clone(), "review-chat", "task-list").await);
        let mut reviewer_board = board.clone();
        reviewer_board.reviewer_chat_ids.push("review-chat".into());
        crate::tasks::storage::save_board(gcx.clone(), "task-list", &reviewer_board)
            .await
            .unwrap();
        let reviewer = load_trajectory_for_chat(gcx.clone(), "review-chat")
            .await
            .unwrap();
        assert_eq!(reviewer.thread.task_meta.as_ref().unwrap().role, "planner");
        crate::tasks::storage::save_board(gcx.clone(), "task-list", &board)
            .await
            .unwrap();
        review["link_type"] = json!("handoff");
        tokio::fs::write(&review_path, serde_json::to_string(&review).unwrap())
            .await
            .unwrap();
        let handoff = load_trajectory_for_chat(gcx.clone(), "review-chat")
            .await
            .unwrap();
        assert_eq!(handoff.thread.task_meta.as_ref().unwrap().role, "linked");
        crate::tasks::storage::save_board(gcx.clone(), "task-list", &reviewer_board)
            .await
            .unwrap();
        let handoff_reviewer = load_trajectory_for_chat(gcx.clone(), "review-chat")
            .await
            .unwrap();
        assert_eq!(
            handoff_reviewer.thread.task_meta.as_ref().unwrap().role,
            "planner"
        );
        crate::tasks::storage::save_board(gcx.clone(), "task-list", &board)
            .await
            .unwrap();
        review.as_object_mut().unwrap().remove("link_type");
        tokio::fs::write(&review_path, serde_json::to_string(&review).unwrap())
            .await
            .unwrap();
        let custom = load_trajectory_for_chat(gcx.clone(), "review-chat")
            .await
            .unwrap();
        assert_eq!(custom.thread.task_meta.as_ref().unwrap().role, "planner");
        review["link_type"] = json!("mode_transition");
        tokio::fs::write(&review_path, serde_json::to_string(&review).unwrap())
            .await
            .unwrap();
        let owner = load_trajectory_for_chat(gcx.clone(), "planner-chat")
            .await
            .unwrap();
        assert_eq!(owner.thread.task_meta.as_ref().unwrap().role, "planner");
        planner["mode"] = json!("solo");
        tokio::fs::write(&planner_path, serde_json::to_string(&planner).unwrap())
            .await
            .unwrap();
        let solo_owner = load_trajectory_for_chat(gcx.clone(), "planner-chat")
            .await
            .unwrap();
        assert_eq!(
            solo_owner.thread.task_meta.as_ref().unwrap().role,
            "planner"
        );
        assert!(chat_id_is_planner_for_task(gcx.clone(), "planner-chat", "task-list").await);
        planner["mode"] = json!("task_planner");
        tokio::fs::write(&planner_path, serde_json::to_string(&planner).unwrap())
            .await
            .unwrap();
        let linked_path = task_dir
            .join("trajectories")
            .join("linked")
            .join("review-chat.json");
        tokio::fs::create_dir_all(linked_path.parent().unwrap())
            .await
            .unwrap();
        let mut linked_copy = review.clone();
        linked_copy["task_meta"]["role"] = json!("linked");
        tokio::fs::write(&linked_path, serde_json::to_string(&linked_copy).unwrap())
            .await
            .unwrap();
        let all = list_trajectories_page(app.clone(), 20, None, false)
            .await
            .unwrap();
        assert_eq!(
            all.items
                .iter()
                .filter(|item| item.id == "review-chat")
                .count(),
            1
        );
        tokio::fs::write(task_dir.join("board.yaml"), "not: [valid")
            .await
            .unwrap();
        tokio::fs::remove_file(&linked_path).await.unwrap();
        let unreadable = load_trajectory_for_chat(gcx.clone(), "review-chat")
            .await
            .unwrap();
        assert_eq!(
            unreadable.thread.task_meta.as_ref().unwrap().role,
            "planner"
        );
        let page = list_trajectories_page(app, 10, None, true).await.unwrap();
        let ids: std::collections::HashSet<_> =
            page.items.iter().map(|item| item.id.as_str()).collect();

        assert_eq!(
            ids,
            std::collections::HashSet::from(["project-chat", "review-chat"])
        );
        assert_eq!(page.total_count, 2);
        assert!(!page.has_more);
    }

    #[test]
    fn displayable_chat_predicates_follow_mode_not_task_scope() {
        let review = TrajectoryListData {
            id: "review-chat".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            mode: Some("review".to_string()),
            extra: serde_json::from_value(json!({
                "parent_id": "planner-chat",
                "link_type": "mode_transition",
                "task_meta": { "task_id": "task-list", "role": "planner" }
            }))
            .unwrap(),
        };
        assert!(trajectory_list_data_is_displayable_chat(&review));

        let planner = TrajectoryListData {
            id: "planner-chat".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            mode: Some("task_planner".to_string()),
            extra: serde_json::from_value(json!({
                "task_meta": { "task_id": "task-list", "role": "planner" }
            }))
            .unwrap(),
        };
        assert!(!trajectory_list_data_is_displayable_chat(&planner));

        let buddy = TrajectoryListData {
            id: "buddy-chat".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            mode: Some("review".to_string()),
            extra: serde_json::from_value(json!({ "buddy_meta": { "is_buddy_chat": true } }))
                .unwrap(),
        };
        assert!(!trajectory_list_data_is_displayable_chat(&buddy));

        let subagent = TrajectoryListData {
            id: "subagent-chat".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            mode: Some("agent".to_string()),
            extra: serde_json::from_value(json!({
                "parent_id": "project-chat",
                "link_type": "subagent"
            }))
            .unwrap(),
        };
        assert!(!trajectory_list_data_is_displayable_chat(&subagent));

        let review_event: TrajectoryEvent = serde_json::from_value(json!({
            "type": "updated",
            "id": "review-chat",
            "mode": "review",
            "task_id": "task-list",
            "parent_id": "planner-chat",
            "link_type": "mode_transition"
        }))
        .unwrap();
        assert!(trajectory_event_is_displayable_chat(&review_event));

        let planner_event: TrajectoryEvent = serde_json::from_value(json!({
            "type": "updated",
            "id": "planner-chat",
            "mode": "task_planner",
            "task_id": "task-list"
        }))
        .unwrap();
        assert!(!trajectory_event_is_displayable_chat(&planner_event));
    }

    #[tokio::test]
    async fn paginated_list_backfills_when_hydration_skips_changed_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let root = dir.path().join(".refact").join("trajectories");
        write_trajectory_file(
            &root.join("top-chat.json"),
            "top-chat",
            "Top Chat",
            "2024-01-01T00:00:03Z",
        )
        .await;
        write_trajectory_file(
            &root.join("skip-chat.json"),
            "skip-chat",
            "Skip Chat",
            "2024-01-01T00:00:02Z",
        )
        .await;
        write_trajectory_file(
            &root.join("backfill-chat.json"),
            "backfill-chat",
            "Backfill Chat",
            "2024-01-01T00:00:01Z",
        )
        .await;
        tokio::fs::write(
            root.join("skip-chat.json"),
            serde_json::to_string(&sample_trajectory(
                "other-chat",
                "Changed",
                "2024-01-01T00:00:02Z",
            ))
            .unwrap(),
        )
        .await
        .unwrap();

        let candidates = vec![
            TrajectoryListCandidate {
                id: "top-chat".to_string(),
                updated_at: "2024-01-01T00:00:03Z".to_string(),
                path: root.join("top-chat.json"),
                indexed_meta: None,
                indexed_file_len: None,
                indexed_file_modified_unix_ms: None,
            },
            TrajectoryListCandidate {
                id: "skip-chat".to_string(),
                updated_at: "2024-01-01T00:00:02Z".to_string(),
                path: root.join("skip-chat.json"),
                indexed_meta: None,
                indexed_file_len: None,
                indexed_file_modified_unix_ms: None,
            },
            TrajectoryListCandidate {
                id: "backfill-chat".to_string(),
                updated_at: "2024-01-01T00:00:01Z".to_string(),
                path: root.join("backfill-chat.json"),
                indexed_meta: None,
                indexed_file_len: None,
                indexed_file_modified_unix_ms: None,
            },
        ];
        let task_roots = Vec::new();

        let (items, has_more) = hydrate_trajectory_list_page(app, candidates, 2, &task_roots).await;
        let ids: Vec<_> = items.iter().map(|item| item.id.as_str()).collect();

        assert_eq!(ids, vec!["top-chat", "backfill-chat"]);
        assert!(!has_more);
    }

    #[tokio::test]
    async fn trajectory_path_helpers_and_save_reject_malformed_chat_id() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        tokio::fs::create_dir_all(dir.path().join(".refact").join("trajectories"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(
            dir.path()
                .join(".refact")
                .join("buddy")
                .join("chats")
                .join("conversations"),
        )
        .await
        .unwrap();
        tokio::fs::write(dir.path().join(".refact").join("bad.json"), "{}")
            .await
            .unwrap();
        tokio::fs::write(
            dir.path()
                .join(".refact")
                .join("buddy")
                .join("chats")
                .join("bad.json"),
            "{}",
        )
        .await
        .unwrap();

        assert!(find_trajectory_path(gcx.clone(), "../bad").await.is_none());
        assert!(find_trajectory_or_buddy_path(gcx.clone(), "../bad")
            .await
            .is_none());

        let err = save_trajectory_snapshot(
            gcx,
            test_snapshot(
                "../save-bad",
                "Bad",
                vec![ChatMessage::new("user".to_string(), "hello".to_string())],
            ),
        )
        .await
        .unwrap_err();
        assert_eq!(err, "Invalid trajectory id");
        assert!(
            !tokio::fs::try_exists(dir.path().join(".refact").join("save-bad.json"))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn trajectory_get_metadata_only_projects_whitelist_and_keeps_raw_default() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "get-metadata-only";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let mut trajectory = sample_trajectory(chat_id, "Linked Child", "2024-01-01T00:00:01Z");
        trajectory["mode"] = json!("agent");
        trajectory["parent_id"] = json!("parent-chat");
        trajectory["link_type"] = json!("subagent");
        trajectory["root_chat_id"] = json!("root-chat");
        trajectory["task_meta"] = json!({
            "task_id": "task-1",
            "role": "agents",
            "card_id": "T-1",
            "secret_extra": "drop-me",
        });
        trajectory["messages"] = json!([{ "role": "user", "content": "private history" }]);
        trajectory["goal"] = json!({ "content": "private goal" });
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        let raw = serde_json::to_string_pretty(&trajectory).unwrap();
        tokio::fs::write(&path, &raw).await.unwrap();
        let get = |metadata_only: bool| {
            handle_v1_trajectories_get(
                State(app.clone()),
                AxumPath(chat_id.to_string()),
                axum::extract::Query(TrajectoryGetQuery {
                    metadata_only,
                }),
            )
        };

        let full = hyper::body::to_bytes(get(false).await.unwrap().into_body())
            .await
            .unwrap();
        assert_eq!(full.as_ref(), raw.as_bytes());
        let metadata = hyper::body::to_bytes(get(true).await.unwrap().into_body())
            .await
            .unwrap();
        let metadata: serde_json::Value = serde_json::from_slice(&metadata).unwrap();
        assert_eq!(
            metadata,
            json!({
                "id": chat_id,
                "title": "Linked Child",
                "model": "model",
                "mode": "agent",
                "parent_id": "parent-chat",
                "link_type": "subagent",
                "root_chat_id": "root-chat",
                "task_meta": { "task_id": "task-1", "role": "agents", "card_id": "T-1" },
            })
        );

        trajectory["task_meta"] = json!({ "role": "agents" });
        tokio::fs::write(&path, serde_json::to_string(&trajectory).unwrap())
            .await
            .unwrap();
        let err = get(true).await.unwrap_err();
        assert_eq!(err.status_code, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(get(false).await.is_ok());
    }

    #[tokio::test]
    async fn generic_trajectory_delete_does_not_delete_buddy_conversation_file() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "buddy-delete-isolated";
        let buddy_path = dir
            .path()
            .join(".refact")
            .join("buddy")
            .join("chats")
            .join("conversations")
            .join(format!("{}.json", chat_id));
        write_buddy_conversation_file(&buddy_path, chat_id, "Keep Buddy").await;

        let err = handle_v1_trajectories_delete(State(app), AxumPath(chat_id.to_string()))
            .await
            .unwrap_err();

        assert_eq!(err.status_code, StatusCode::NOT_FOUND);
        assert!(tokio::fs::try_exists(&buddy_path).await.unwrap());
    }

    #[tokio::test]
    async fn generic_trajectory_delete_does_not_emit_buddy_fallback_update() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "buddy-delete-no-fallback-update";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let buddy_path = dir
            .path()
            .join(".refact")
            .join("buddy")
            .join("chats")
            .join("conversations")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(
            &normal_path,
            chat_id,
            "Delete Normal",
            "2024-01-01T00:00:01Z",
        )
        .await;
        write_buddy_conversation_file(&buddy_path, chat_id, "Keep Buddy").await;

        handle_v1_trajectories_delete(State(app), AxumPath(chat_id.to_string()))
            .await
            .unwrap();

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "deleted");
        assert_eq!(event.title, None);
        assert!(!tokio::fs::try_exists(&normal_path).await.unwrap());
        assert!(tokio::fs::try_exists(&buddy_path).await.unwrap());
    }

    #[tokio::test]
    async fn explicit_buddy_inclusive_lookup_loads_buddy_conversation_file() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "buddy-read-inclusive";
        let buddy_path = dir
            .path()
            .join(".refact")
            .join("buddy")
            .join("chats")
            .join("conversations")
            .join(format!("{}.json", chat_id));
        write_buddy_conversation_file(&buddy_path, chat_id, "Readable Buddy").await;

        assert!(find_trajectory_path(gcx.clone(), chat_id).await.is_none());
        assert_same_optional_path(
            find_trajectory_or_buddy_path(gcx.clone(), chat_id).await,
            &buddy_path,
        );
        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();

        assert_eq!(loaded.thread.title, "Readable Buddy");
        assert_eq!(
            loaded.thread.buddy_meta.unwrap().buddy_chat_kind,
            "investigation"
        );
        assert_eq!(loaded.messages.len(), 1);
    }

    #[tokio::test]
    async fn empty_chat_workspace_only_snapshot_is_durable() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let binding_id = "6f1c1d2e-0000-4000-8000-0000000000b1";
        let populated = refact_chat_api::StoredAppWorkspace::from_value(json!({
            "schema_version": 1,
            "revision": 1,
            "bindings": [{
                "binding_id": binding_id,
                "app_id": "6f1c1d2e-0000-4000-8000-000000000001",
                "params": {"query": "workspace-only"},
                "state": {"page": 2}
            }],
            "layout": {"kind": "leaf", "tabs": [
                {"kind": "chat", "ref": "self"},
                {"kind": "app", "binding_id": binding_id}
            ], "active": 1}
        }));
        populated.supported().unwrap().validate().unwrap();
        let mut cleared = refact_chat_api::AppWorkspace::empty();
        cleared.revision = 2;
        for (chat_id, workspace) in [
            ("empty-no-workspace", None),
            ("empty-with-workspace", Some(populated)),
            (
                "empty-cleared-workspace",
                Some(refact_chat_api::StoredAppWorkspace::Supported(cleared)),
            ),
            (
                "empty-unsupported-workspace",
                Some(refact_chat_api::StoredAppWorkspace::from_value(
                    json!({"schema_version": 99, "revision": 3, "future_layout": ["opaque"]}),
                )),
            ),
        ] {
            let mut snapshot = test_snapshot(chat_id, "", Vec::new());
            snapshot.app_workspace = workspace.clone();
            save_trajectory_snapshot(gcx.clone(), snapshot)
                .await
                .unwrap();
            let path = dir
                .path()
                .join(".refact/trajectories")
                .join(chat_id)
                .join(format!("{chat_id}.json"));
            let Some(workspace) = workspace else {
                assert!(!path.exists(), "a fresh default chat must still be skipped");
                continue;
            };
            assert!(
                path.is_file(),
                "workspace-only snapshot must reach disk: {chat_id}"
            );
            let persisted: serde_json::Value =
                serde_json::from_slice(&tokio::fs::read(&path).await.unwrap()).unwrap();
            assert_eq!(
                persisted["app_workspace"],
                serde_json::to_value(&workspace).unwrap()
            );
            assert_eq!(persisted["messages"], json!([]));
            let loaded = load_trajectory_for_chat(gcx.clone(), chat_id)
                .await
                .unwrap();
            assert_eq!(loaded.thread.app_workspace, Some(workspace));
            assert!(loaded.messages.is_empty());
        }
    }

    #[tokio::test]
    async fn pending_new_chat_cap_survives_empty_first_save_and_model_late_load() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "pending-new-chat-cap";
        let mut snapshot = test_snapshot(chat_id, "", Vec::new());
        snapshot.auto_compression_cap_pending = true;
        snapshot.context_tokens_cap = Some(100_000);
        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();
        let mut loaded = load_trajectory_for_chat(gcx.clone(), chat_id)
            .await
            .unwrap();
        assert!(loaded.messages.is_empty());
        assert!(loaded.thread.auto_compression_cap_pending);
        assert!(!loaded.thread.resolve_pending_compression_cap(None));
        assert_eq!(loaded.thread.auto_compression_cap, None);
        assert!(loaded.thread.resolve_pending_compression_cap(Some(80_000)));
        assert_eq!(loaded.thread.auto_compression_cap, Some(72_000));
        save_trajectory_as(gcx.clone(), &loaded.thread, &loaded.messages).await;
        let restored = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        assert_eq!(restored.thread.auto_compression_cap, Some(72_000));
        assert!(!restored.thread.auto_compression_cap_pending);
    }

    #[serial(trajectory_perf)]
    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn save_trajectory_as_is_durable_with_the_detached_writer() {
        let _lock = serial_test_guard();
        let _writer = TrajectoryWriterRolloutGuard::set(true);
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let thread = ThreadParams {
            id: "save-trajectory-as-durable".to_string(),
            title: "Durable direct save".to_string(),
            ..Default::default()
        };

        save_trajectory_as(
            gcx.clone(),
            &thread,
            &[ChatMessage::new(
                "user".to_string(),
                "persisted".to_string(),
            )],
        )
        .await;

        let loaded = load_trajectory_for_chat(gcx, &thread.id).await;
        assert!(loaded.is_some());
        assert_eq!(
            loaded.unwrap().messages[0].content.content_text_only(),
            "persisted"
        );
    }

    #[tokio::test]
    async fn empty_chat_can_clear_previously_saved_auto_compression_cap() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "empty-chat-clear-auto-compression-cap";
        let mut thread = ThreadParams {
            id: chat_id.to_string(),
            title: "Empty capped chat".to_string(),
            auto_compression_cap: Some(64_000),
            ..Default::default()
        };

        save_trajectory_as(gcx.clone(), &thread, &[]).await;
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                if load_trajectory_for_chat(gcx.clone(), chat_id)
                    .await
                    .is_some()
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("checkpoint should persist the empty capped chat");
        assert_eq!(
            load_trajectory_for_chat(gcx.clone(), chat_id)
                .await
                .unwrap()
                .thread
                .auto_compression_cap,
            Some(64_000)
        );

        thread.auto_compression_cap = None;
        save_trajectory_as(gcx.clone(), &thread, &[]).await;
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                if load_trajectory_for_chat(gcx.clone(), chat_id)
                    .await
                    .is_some_and(|loaded| loaded.thread.auto_compression_cap.is_none())
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("checkpoint should clear the persisted cap");
        assert_eq!(
            load_trajectory_for_chat(gcx, chat_id)
                .await
                .unwrap()
                .thread
                .auto_compression_cap,
            None
        );
    }

    #[tokio::test]
    async fn generic_loader_ignores_buddy_only_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "buddy-generic-loader-isolated";
        let buddy_path = dir
            .path()
            .join(".refact")
            .join("buddy")
            .join("chats")
            .join("conversations")
            .join(format!("{chat_id}.json"));
        write_buddy_conversation_file(&buddy_path, chat_id, "Only Buddy").await;

        assert!(load_generic_trajectory_for_chat(gcx.clone(), chat_id)
            .await
            .is_none());
        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();

        assert_eq!(loaded.thread.title, "Only Buddy");
        assert!(loaded.thread.buddy_meta.is_some());
    }

    #[tokio::test]
    async fn generic_watcher_remove_with_buddy_fallback_emits_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "buddy-watcher-remove-fallback";
        let buddy_path = dir
            .path()
            .join(".refact")
            .join("buddy")
            .join("chats")
            .join("conversations")
            .join(format!("{chat_id}.json"));
        write_buddy_conversation_file(&buddy_path, chat_id, "Fallback Buddy").await;

        process_trajectory_change(gcx, chat_id, true).await;

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "deleted");
        assert_eq!(event.title, None);
        assert_eq!(event.mode, None);
        assert!(tokio::fs::try_exists(&buddy_path).await.unwrap());
    }

    #[tokio::test]
    async fn generic_watcher_remove_with_loadable_fallback_emits_updated() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "watcher-remove-generic-fallback-updated";
        let fallback_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-watch-fallback")
            .join("trajectories")
            .join("planner")
            .join(format!("{chat_id}.json"));
        write_trajectory_file_with_metadata(
            &fallback_path,
            chat_id,
            "Watcher Fallback",
            "2024-01-01T00:00:01Z",
            "fallback after remove",
        )
        .await;

        process_trajectory_change(gcx, chat_id, true).await;

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "updated");
        assert_eq!(event.title.as_deref(), Some("Watcher Fallback"));
        assert_eq!(event.task_id.as_deref(), Some("task-watch-fallback"));
        assert_eq!(event.task_role.as_deref(), Some("planner"));
        assert!(tokio::fs::try_exists(&fallback_path).await.unwrap());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_normal_trajectory_root_is_ignored_for_load() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "symlink-load-ignored";
        let refact_dir = dir.path().join(".refact");
        tokio::fs::create_dir_all(&refact_dir).await.unwrap();
        std::os::unix::fs::symlink(outside.path(), refact_dir.join("trajectories")).unwrap();
        let outside_path = outside.path().join(format!("{chat_id}.json"));
        write_trajectory_file(
            &outside_path,
            chat_id,
            "Outside Symlink Load",
            "2024-01-01T00:00:00Z",
        )
        .await;

        assert!(find_trajectory_path(gcx.clone(), chat_id).await.is_none());
        assert!(load_trajectory_for_chat(gcx, chat_id).await.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_normal_trajectory_root_is_not_used_for_save() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "symlink-save-blocked";
        let refact_dir = dir.path().join(".refact");
        tokio::fs::create_dir_all(&refact_dir).await.unwrap();
        std::os::unix::fs::symlink(outside.path(), refact_dir.join("trajectories")).unwrap();
        let outside_path = outside.path().join(format!("{chat_id}.json"));
        write_trajectory_file(
            &outside_path,
            chat_id,
            "Keep Outside Save",
            "2024-01-01T00:00:00Z",
        )
        .await;
        let before = tokio::fs::read_to_string(&outside_path).await.unwrap();

        let err = save_trajectory_snapshot(
            gcx,
            test_snapshot(
                chat_id,
                "Should Not Escape",
                vec![ChatMessage::new("user".to_string(), "hello".to_string())],
            ),
        )
        .await
        .unwrap_err();

        assert!(err.contains("Refusing to use non-real directory component"));
        assert_eq!(
            tokio::fs::read_to_string(outside_path).await.unwrap(),
            before
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_normal_trajectory_root_is_not_used_for_delete() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "symlink-delete-blocked";
        let refact_dir = dir.path().join(".refact");
        tokio::fs::create_dir_all(&refact_dir).await.unwrap();
        std::os::unix::fs::symlink(outside.path(), refact_dir.join("trajectories")).unwrap();
        let outside_path = outside.path().join(format!("{chat_id}.json"));
        write_trajectory_file(
            &outside_path,
            chat_id,
            "Keep Outside Delete",
            "2024-01-01T00:00:00Z",
        )
        .await;

        let err = handle_v1_trajectories_delete(State(app), AxumPath(chat_id.to_string()))
            .await
            .unwrap_err();

        assert_eq!(err.status_code, StatusCode::NOT_FOUND);
        assert!(tokio::fs::try_exists(outside_path).await.unwrap());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_normal_trajectory_file_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "symlink-file-blocked";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();
        let outside_path = outside.path().join(format!("{chat_id}.json"));
        write_trajectory_file(
            &outside_path,
            chat_id,
            "Keep Outside File",
            "2024-01-01T00:00:00Z",
        )
        .await;
        let before = tokio::fs::read_to_string(&outside_path).await.unwrap();
        std::os::unix::fs::symlink(
            &outside_path,
            trajectories_dir.join(format!("{chat_id}.json")),
        )
        .unwrap();

        assert!(find_trajectory_path(gcx.clone(), chat_id).await.is_none());
        let delete_err = handle_v1_trajectories_delete(State(app), AxumPath(chat_id.to_string()))
            .await
            .unwrap_err();
        assert_eq!(delete_err.status_code, StatusCode::NOT_FOUND);

        let save_err = save_trajectory_snapshot(
            gcx,
            test_snapshot(
                chat_id,
                "Should Not Follow File",
                vec![ChatMessage::new("user".to_string(), "hello".to_string())],
            ),
        )
        .await
        .unwrap_err();

        assert!(save_err.contains("Refusing symlink child file"));
        assert_eq!(
            tokio::fs::read_to_string(outside_path).await.unwrap(),
            before
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn macos_system_temp_symlink_roots_are_allowed_for_save() {
        let dir = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "macos-var-save-ok";

        save_trajectory_snapshot(
            gcx,
            test_snapshot(
                chat_id,
                "macOS /var Save",
                vec![ChatMessage::new("user".to_string(), "hello".to_string())],
            ),
        )
        .await
        .unwrap();

        assert!(tokio::fs::try_exists(
            dir.path()
                .join(".refact")
                .join("trajectories")
                .join(format!("{chat_id}.json"))
        )
        .await
        .unwrap());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_task_trajectory_root_is_ignored_for_load_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "symlink-task-root-blocked";
        let tasks_dir = dir.path().join(".refact").join("tasks");
        tokio::fs::create_dir_all(tasks_dir.parent().unwrap())
            .await
            .unwrap();
        std::os::unix::fs::symlink(outside.path(), &tasks_dir).unwrap();
        let outside_path = outside
            .path()
            .join("task-symlink-root")
            .join("trajectories")
            .join("planner")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(
            &outside_path,
            chat_id,
            "Outside Task Root",
            "2024-01-01T00:00:00Z",
        )
        .await;

        assert!(find_trajectory_path(gcx.clone(), chat_id).await.is_none());
        assert!(load_trajectory_for_chat(gcx, chat_id).await.is_none());
        let err = handle_v1_trajectories_delete(State(app), AxumPath(chat_id.to_string()))
            .await
            .unwrap_err();
        assert_eq!(err.status_code, StatusCode::NOT_FOUND);
        assert!(tokio::fs::try_exists(outside_path).await.unwrap());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_task_trajectory_container_does_not_escape_discovery() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(workspace.path()).await;
        let task_dir = workspace.path().join(".refact/tasks/container-task");
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        std::os::unix::fs::symlink(outside.path(), task_dir.join("trajectories")).unwrap();
        let chat_id = "symlink-container-chat";
        let outside_path = outside
            .path()
            .join("agents")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(
            &outside_path,
            chat_id,
            "Outside Container",
            "2024-01-01T00:00:00Z",
        )
        .await;
        let valid_dirs = seed_agent_task_dirs(workspace.path(), 1).await;
        let discovered = list_task_trajectory_dirs(&gcx).await;
        assert!(discovered.contains(&valid_dirs[0]));
        assert!(!discovered
            .iter()
            .any(|path| path.starts_with(task_dir.join("trajectories"))));
        assert!(
            !is_task_trajectory_index_dir(
                &task_dir.join("trajectories/agents"),
                &get_all_task_roots(gcx.clone()).await,
            )
            .await
        );
        assert!(find_trajectory_path(gcx.clone(), chat_id).await.is_none());
        assert!(load_trajectory_for_chat(gcx, chat_id).await.is_none());
        let error = handle_v1_trajectories_delete(State(app), AxumPath(chat_id.into()))
            .await
            .unwrap_err();
        assert_eq!(error.status_code, StatusCode::NOT_FOUND);
        assert!(tokio::fs::try_exists(outside_path).await.unwrap());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_task_trajectory_agent_dir_is_ignored_for_load_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "symlink-task-agent-blocked";
        let agents_dir = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-symlink-agent")
            .join("trajectories")
            .join("agents");
        tokio::fs::create_dir_all(&agents_dir).await.unwrap();
        std::os::unix::fs::symlink(outside.path(), agents_dir.join("agent-1")).unwrap();
        let outside_path = outside.path().join(format!("{chat_id}.json"));
        write_trajectory_file(
            &outside_path,
            chat_id,
            "Outside Task Agent",
            "2024-01-01T00:00:00Z",
        )
        .await;

        assert!(find_trajectory_path(gcx.clone(), chat_id).await.is_none());
        assert!(load_trajectory_for_chat(gcx, chat_id).await.is_none());
        let err = handle_v1_trajectories_delete(State(app), AxumPath(chat_id.to_string()))
            .await
            .unwrap_err();
        assert_eq!(err.status_code, StatusCode::NOT_FOUND);
        assert!(tokio::fs::try_exists(outside_path).await.unwrap());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_buddy_conversations_dir_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "symlink-buddy-blocked";
        let buddy_parent = dir.path().join(".refact").join("buddy").join("chats");
        tokio::fs::create_dir_all(&buddy_parent).await.unwrap();
        std::os::unix::fs::symlink(outside.path(), buddy_parent.join("conversations")).unwrap();
        let outside_path = outside.path().join(format!("{chat_id}.json"));
        write_buddy_conversation_file(&outside_path, chat_id, "Outside Buddy").await;

        assert!(find_trajectory_or_buddy_path(gcx.clone(), chat_id)
            .await
            .is_none());
        assert!(load_trajectory_for_chat(gcx, chat_id).await.is_none());
    }

    #[tokio::test]
    async fn trajectory_id_mismatch_normal_candidate_does_not_shadow_task_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "safe-task-shadow";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-shadow")
            .join("trajectories")
            .join("planner")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(
            &normal_path,
            "other-chat",
            "Mismatched Normal",
            "2024-01-01T00:00:00Z",
        )
        .await;
        write_trajectory_file(&task_path, chat_id, "Valid Task", "2024-01-01T00:00:01Z").await;

        let candidate = find_trajectory_file(gcx.clone(), chat_id).await.unwrap();
        assert_same_path(&candidate.path, &task_path);
        assert_eq!(candidate.json["id"], chat_id);
        assert_same_optional_path(find_trajectory_path(gcx.clone(), chat_id).await, &task_path);
        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        assert_eq!(loaded.thread.title, "Valid Task");
    }

    #[tokio::test]
    async fn trajectory_id_mismatch_delete_skips_invalid_higher_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "delete-shadow";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-delete-shadow")
            .join("trajectories")
            .join("planner")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(
            &normal_path,
            "other-chat",
            "Keep Mismatched",
            "2024-01-01T00:00:00Z",
        )
        .await;
        write_trajectory_file(&task_path, chat_id, "Delete Valid", "2024-01-01T00:00:01Z").await;

        handle_v1_trajectories_delete(State(app), AxumPath(chat_id.to_string()))
            .await
            .unwrap();

        assert!(tokio::fs::try_exists(&normal_path).await.unwrap());
        assert!(!tokio::fs::try_exists(&task_path).await.unwrap());
    }

    #[tokio::test]
    async fn generic_list_does_not_use_active_buddy_state_error_or_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "list-active-buddy-clean";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(
            &normal_path,
            chat_id,
            "Generic List Item",
            "2024-01-01T00:00:01Z",
        )
        .await;
        let listed_without_session = list_all_trajectories_meta(app.clone(), false)
            .await
            .unwrap();
        let item_without_session = listed_without_session
            .iter()
            .find(|item| item.id == chat_id)
            .cloned()
            .unwrap();
        assert_eq!(item_without_session.session_state, None);
        assert!(item_without_session.worktree.is_none());

        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.mode = "buddy".to_string();
            session.thread.buddy_meta = Some(buddy_thread_meta());
            session.thread.worktree = Some(trajectory_worktree_sample_with_id("buddy-wt"));
            session.runtime.state = SessionState::Generating;
            session.runtime.error = Some("buddy list error".to_string());
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc);

        let listed = list_all_trajectories_meta(app, false).await.unwrap();
        let item = listed
            .iter()
            .find(|item| item.id == chat_id)
            .cloned()
            .unwrap();

        assert_eq!(item.session_state, None);
        assert!(item.worktree.is_none());
        assert_eq!(
            serde_json::to_value(&item).unwrap(),
            serde_json::to_value(&item_without_session).unwrap()
        );
    }

    #[tokio::test]
    async fn generic_list_keeps_normal_active_session_state_and_worktree_enrichment() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "list-normal-active-enriched";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(
            &normal_path,
            chat_id,
            "Normal List Item",
            "2024-01-01T00:00:01Z",
        )
        .await;
        let worktree = trajectory_worktree_sample_with_id("normal-wt");
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.worktree = Some(worktree.clone());
            session.runtime.state = SessionState::Generating;
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc);

        let listed = list_all_trajectories_meta(app, false).await.unwrap();
        let item = listed.iter().find(|item| item.id == chat_id).unwrap();

        assert_eq!(item.session_state.as_deref(), Some("generating"));
        assert_eq!(
            item.worktree.as_ref().map(|meta| meta.id.as_str()),
            Some("normal-wt")
        );
    }

    #[tokio::test]
    async fn http_delete_higher_priority_trajectory_emits_updated_from_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "http-delete-fallback-updated";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-fallback")
            .join("trajectories")
            .join("planner")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(
            &normal_path,
            chat_id,
            "Deleted Higher Priority",
            "2024-01-01T00:00:02Z",
        )
        .await;
        write_trajectory_file_with_metadata(
            &task_path,
            chat_id,
            "Fallback Lower Priority",
            "2024-01-01T00:00:01Z",
            "fallback message",
        )
        .await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.title = "Active Normal Collision".to_string();
            session.runtime.state = SessionState::Generating;
            session.runtime.error = Some("busy fallback state".to_string());
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc);

        handle_v1_trajectories_delete(State(app), AxumPath(chat_id.to_string()))
            .await
            .unwrap();

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "updated");
        assert_eq!(event.updated_at.as_deref(), Some("2024-01-01T00:00:01Z"));
        assert_eq!(event.title.as_deref(), Some("Fallback Lower Priority"));
        assert_eq!(event.is_title_generated, Some(true));
        assert_eq!(event.message_count, Some(1));
        assert_eq!(event.parent_id.as_deref(), Some("parent-fallback"));
        assert_eq!(event.link_type.as_deref(), Some("handoff"));
        assert_eq!(event.root_chat_id.as_deref(), Some("root-fallback"));
        assert_eq!(event.task_id.as_deref(), Some("task-fallback"));
        assert_eq!(event.task_role.as_deref(), Some("planner"));
        assert_eq!(event.card_id, None);
        assert_eq!(event.model.as_deref(), Some("fallback-model"));
        assert_eq!(event.mode.as_deref(), Some("task_planner"));
        assert_eq!(event.session_state.as_deref(), Some("idle"));
        assert_eq!(event.error, None);
        assert_ne!(event.title.as_deref(), Some("Active Normal Collision"));
        assert!(!tokio::fs::try_exists(&normal_path).await.unwrap());
        assert!(tokio::fs::try_exists(&task_path).await.unwrap());
    }

    #[tokio::test]
    async fn http_delete_higher_priority_trajectory_repairs_fallback_before_updated_event() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "http-delete-repaired-fallback-updated-at";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-repaired-fallback")
            .join("trajectories")
            .join("planner")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(
            &normal_path,
            chat_id,
            "Deleted Higher Priority Repair",
            "2024-01-01T00:00:02Z",
        )
        .await;
        tokio::fs::create_dir_all(task_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &task_path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Repaired Lower Priority Fallback",
                "model": "fallback-model",
                "mode": "task_planner",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "mode_transition",
                "previous_response_id": "resp_stale",
                "messages": [
                    {"role":"system","content":"target fallback system"},
                    {"role":"user","content":"fallback message"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2026-05-29T00:00:00Z",
                    "system_prompt": "stale source system",
                    "tools_canonical": [{"type":"function","function":{"name":"source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "created_at": "not-a-date",
                "updated_at": "2024-01-01T00:00:01Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "custom_future_field": {"keep": true, "nested": {"value": 73}}
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        handle_v1_trajectories_delete(State(app), AxumPath(chat_id.to_string()))
            .await
            .unwrap();

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "updated");
        assert_eq!(
            event.title.as_deref(),
            Some("Repaired Lower Priority Fallback")
        );
        assert_eq!(event.task_id.as_deref(), Some("task-repaired-fallback"));
        assert_eq!(event.task_role.as_deref(), Some("planner"));
        assert_ne!(event.updated_at.as_deref(), Some("2024-01-01T00:00:01Z"));
        assert!(!tokio::fs::try_exists(&normal_path).await.unwrap());
        assert!(tokio::fs::try_exists(&task_path).await.unwrap());

        let repaired: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&task_path).await.unwrap()).unwrap();
        assert_eq!(event.updated_at.as_deref(), repaired["updated_at"].as_str());
        assert!(parsed_rfc3339_utc(repaired["created_at"].as_str().unwrap()).is_some());
        assert_eq!(
            repaired["frozen_request_prefix"]["system_prompt"],
            "target fallback system"
        );
        assert!(repaired["frozen_request_prefix"]["tools_canonical"].is_null());
        assert!(repaired.get("previous_response_id").is_none());
        assert!(repaired.get("claude_code_identity").is_none());
        assert_eq!(repaired["custom_future_field"]["nested"]["value"], 73);
    }

    #[tokio::test]
    async fn http_delete_normal_with_task_fallback_does_not_reload_task_into_normal_session() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "http-delete-normal-task-fallback-no-normal-mutation";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let task_meta = task_meta(
            "task-normal-delete-fallback",
            "agents",
            Some("agent-normal-delete-fallback"),
            Some("card-normal-delete-fallback"),
            None,
        );
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-normal-delete-fallback")
            .join("trajectories")
            .join("agents")
            .join("agent-normal-delete-fallback")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(
            &normal_path,
            chat_id,
            "Deleted Normal",
            "2024-01-01T00:00:02Z",
        )
        .await;
        write_task_trajectory_file_with_user_message(
            &task_path,
            chat_id,
            "Task Fallback After Normal Delete",
            "task fallback should stay out of normal session",
            &task_meta,
        )
        .await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        let mut chat_rx = {
            let mut session = session_arc.lock().await;
            session.thread.title = "Active Normal Before Delete".to_string();
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                "keep active normal after normal delete".to_string(),
            ));
            session.runtime.state = SessionState::Idle;
            session.trajectory_dirty = false;
            session.subscribe()
        };
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        handle_v1_trajectories_delete(State(app), AxumPath(chat_id.to_string()))
            .await
            .unwrap();

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "updated");
        assert_eq!(
            event.title.as_deref(),
            Some("Task Fallback After Normal Delete")
        );
        assert_eq!(
            event.task_id.as_deref(),
            Some("task-normal-delete-fallback")
        );
        assert_eq!(event.task_role.as_deref(), Some("agents"));
        assert_eq!(
            event.agent_id.as_deref(),
            Some("agent-normal-delete-fallback")
        );
        assert_eq!(event.session_state.as_deref(), Some("idle"));
        assert!(!tokio::fs::try_exists(&normal_path).await.unwrap());
        assert!(tokio::fs::try_exists(&task_path).await.unwrap());

        let session = session_arc.lock().await;
        assert_eq!(session.external_reload_pending, None);
        assert_eq!(session.thread.title, "Active Normal Before Delete");
        assert!(session.thread.task_meta.is_none());
        assert_eq!(session.messages.len(), 1);
        assert_eq!(
            session.messages[0].content.content_text_only(),
            "keep active normal after normal delete"
        );
        drop(session);
        assert_no_chat_event_for(&mut chat_rx, std::time::Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn http_delete_only_remaining_trajectory_emits_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "http-delete-only-deleted";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(
            &normal_path,
            chat_id,
            "Only Remaining",
            "2024-01-01T00:00:01Z",
        )
        .await;

        handle_v1_trajectories_delete(State(app), AxumPath(chat_id.to_string()))
            .await
            .unwrap();

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "deleted");
        assert_eq!(event.updated_at, None);
        assert_eq!(event.title, None);
        assert_eq!(event.session_state, None);
        assert!(!tokio::fs::try_exists(&normal_path).await.unwrap());
    }

    #[tokio::test]
    async fn ordinary_save_preserves_existing_browser_meta_and_unknown_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "metadata-preserve-browser-custom";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Existing Metadata",
                "model": "model",
                "mode": "agent",
                "tool_use": "agent",
                "messages": [{"role":"user","content":"old"}],
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "browser_meta": {
                    "browser_runtime_id": "browser-preserve",
                    "tab_urls": ["https://example.com/preserve"],
                    "active_tab_id": "tab-preserve",
                    "attach_screenshot_on_send": true
                },
                "custom_future_field": {
                    "keep": true,
                    "nested": {"value": 57}
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        save_trajectory_snapshot(
            gcx,
            test_snapshot(
                chat_id,
                "Updated Metadata",
                vec![ChatMessage::new("user".to_string(), "new".to_string())],
            ),
        )
        .await
        .unwrap();

        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(saved["title"], "Updated Metadata");
        assert_eq!(
            saved["browser_meta"]["browser_runtime_id"],
            "browser-preserve"
        );
        assert_eq!(saved["custom_future_field"]["nested"]["value"], 57);
    }

    #[tokio::test]
    async fn ordinary_save_removes_stale_provider_identity_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "metadata-clear-stale-provider";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Provider Existing",
                "model": "model",
                "mode": "agent",
                "tool_use": "agent",
                "messages": [{"role":"user","content":"old"}],
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "previous_response_id": "resp_stale",
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2026-05-29T00:00:00Z",
                    "system_prompt": "stale system",
                    "tools_canonical": [{"type":"function"}]
                },
                "claude_code_identity": {
                    "device_id": "device-stale",
                    "session_id": "session-stale"
                },
                "custom_future_field": {"keep": true}
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        save_trajectory_snapshot(
            gcx,
            test_snapshot(
                chat_id,
                "Provider Cleared",
                vec![ChatMessage::new("user".to_string(), "new".to_string())],
            ),
        )
        .await
        .unwrap();

        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(saved["title"], "Provider Cleared");
        assert!(saved.get("previous_response_id").is_none());
        assert!(saved.get("frozen_request_prefix").is_none());
        assert!(saved.get("claude_code_identity").is_none());
        assert_eq!(saved["custom_future_field"]["keep"], true);
    }

    #[tokio::test]
    async fn trajectory_id_mismatch_http_save_does_not_overwrite_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "http-save-mismatch-guard";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(
            &path,
            "other-chat",
            "Keep Mismatched",
            "2024-01-01T00:00:00Z",
        )
        .await;
        let before = tokio::fs::read_to_string(&path).await.unwrap();
        let payload = sample_trajectory(chat_id, "Should Not Save", "2024-01-01T00:00:01Z");

        let err = handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap_err();

        assert_eq!(err.status_code, StatusCode::CONFLICT);
        assert_eq!(tokio::fs::read_to_string(path).await.unwrap(), before);
    }

    #[tokio::test]
    async fn http_buddy_save_does_not_emit_generic_trajectory_sse() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "http-buddy-save-no-generic-sse";
        let mut payload = sample_trajectory(chat_id, "Buddy Saved", "2024-01-01T00:00:01Z");
        payload["mode"] = json!("buddy");
        payload["buddy_meta"] = json!({
            "is_buddy_chat": true,
            "buddy_chat_kind": "investigation",
            "workflow_id": null
        });

        handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap();

        assert_no_trajectory_event_for(&mut rx, std::time::Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn trajectory_save_namespace_generic_save_does_not_overwrite_existing_buddy_file() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "namespace-generic-buddy-collision";
        let buddy_path = dir
            .path()
            .join(".refact")
            .join("buddy")
            .join("chats")
            .join("conversations")
            .join(format!("{chat_id}.json"));
        write_buddy_conversation_file(&buddy_path, chat_id, "Keep Buddy").await;
        let before = tokio::fs::read_to_string(&buddy_path).await.unwrap();
        let mut payload = sample_trajectory(chat_id, "Generic Saved", "2024-01-01T00:00:01Z");
        payload["messages"] = json!([{ "role": "user", "content": "generic" }]);

        handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap();

        assert_eq!(
            tokio::fs::read_to_string(&buddy_path).await.unwrap(),
            before
        );
        let normal_path = nested_trajectory_path(dir.path(), chat_id, chat_id);
        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&normal_path).await.unwrap()).unwrap();
        assert_eq!(saved["title"], "Generic Saved");
        assert!(saved.get("buddy_meta").is_none());
    }

    #[tokio::test]
    async fn trajectory_save_namespace_generic_save_does_not_overwrite_existing_task_file() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "namespace-generic-task-collision";
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-generic-collision")
            .join("trajectories")
            .join("agents")
            .join("agent-1")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(&task_path, chat_id, "Keep Task", "2024-01-01T00:00:00Z").await;
        let before = tokio::fs::read_to_string(&task_path).await.unwrap();
        let payload = sample_trajectory(chat_id, "Generic Saved", "2024-01-01T00:00:01Z");

        handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap();

        assert_eq!(tokio::fs::read_to_string(&task_path).await.unwrap(), before);
        let normal_path = nested_trajectory_path(dir.path(), chat_id, chat_id);
        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&normal_path).await.unwrap()).unwrap();
        assert_eq!(saved["title"], "Generic Saved");
        assert!(saved.get("task_meta").is_none());
    }

    #[tokio::test]
    async fn trajectory_save_namespace_task_save_does_not_overwrite_existing_normal_file() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "namespace-task-normal-collision";
        let task_id = "task-normal-collision";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(&normal_path, chat_id, "Keep Normal", "2024-01-01T00:00:00Z").await;
        let before = tokio::fs::read_to_string(&normal_path).await.unwrap();
        let task_dir = dir.path().join(".refact").join("tasks").join(task_id);
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        let mut payload = sample_trajectory(chat_id, "Task Saved", "2024-01-01T00:00:01Z");
        payload["task_meta"] = json!({
            "task_id": task_id,
            "role": "agents",
            "agent_id": "agent-1",
            "card_id": "card-1"
        });

        handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap();

        assert_eq!(
            tokio::fs::read_to_string(&normal_path).await.unwrap(),
            before
        );
        let task_path = task_dir
            .join("trajectories")
            .join("agents")
            .join("agent-1")
            .join(format!("{chat_id}.json"));
        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&task_path).await.unwrap()).unwrap();
        assert_eq!(saved["title"], "Task Saved");
        assert_eq!(saved["task_meta"]["task_id"], task_id);
    }

    #[tokio::test]
    async fn trajectory_save_namespace_buddy_save_does_not_overwrite_existing_normal_or_task_file()
    {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "namespace-buddy-normal-task-collision";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-buddy-collision")
            .join("trajectories")
            .join("planner")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(&normal_path, chat_id, "Keep Normal", "2024-01-01T00:00:00Z").await;
        write_trajectory_file(&task_path, chat_id, "Keep Task", "2024-01-01T00:00:00Z").await;
        let normal_before = tokio::fs::read_to_string(&normal_path).await.unwrap();
        let task_before = tokio::fs::read_to_string(&task_path).await.unwrap();
        let mut payload = sample_trajectory(chat_id, "Buddy Saved", "2024-01-01T00:00:01Z");
        payload["mode"] = json!("buddy");
        payload["buddy_meta"] = json!({
            "is_buddy_chat": true,
            "buddy_chat_kind": "investigation",
            "workflow_id": null
        });

        handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap();

        assert_eq!(
            tokio::fs::read_to_string(&normal_path).await.unwrap(),
            normal_before
        );
        assert_eq!(
            tokio::fs::read_to_string(&task_path).await.unwrap(),
            task_before
        );
        let buddy_path = dir
            .path()
            .join(".refact")
            .join("buddy")
            .join("chats")
            .join("conversations")
            .join(format!("{chat_id}.json"));
        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&buddy_path).await.unwrap()).unwrap();
        assert_eq!(saved["title"], "Buddy Saved");
        assert_eq!(saved["buddy_meta"]["is_buddy_chat"], true);
    }

    #[tokio::test]
    async fn trajectory_save_namespace_task_and_buddy_meta_rejects_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "namespace-task-buddy-meta-conflict";
        let task_id = "task-meta-conflict";
        tokio::fs::create_dir_all(dir.path().join(".refact").join("tasks").join(task_id))
            .await
            .unwrap();
        let mut payload = sample_trajectory(chat_id, "Meta Conflict", "2024-01-01T00:00:01Z");
        payload["task_meta"] = json!({
            "task_id": task_id,
            "role": "planner",
            "planner_chat_id": chat_id
        });
        payload["buddy_meta"] = json!({
            "is_buddy_chat": true,
            "buddy_chat_kind": "investigation",
            "workflow_id": null
        });

        let err = handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap_err();

        assert_eq!(err.status_code, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(err.message.contains("both task_meta and buddy_meta"));
        assert!(!tokio::fs::try_exists(
            dir.path()
                .join(".refact")
                .join("trajectories")
                .join(format!("{chat_id}.json"))
        )
        .await
        .unwrap());
        assert!(!tokio::fs::try_exists(
            dir.path()
                .join(".refact")
                .join("buddy")
                .join("chats")
                .join("conversations")
                .join(format!("{chat_id}.json"))
        )
        .await
        .unwrap());
        assert!(!tokio::fs::try_exists(
            dir.path()
                .join(".refact")
                .join("tasks")
                .join(task_id)
                .join("trajectories")
                .join("planner")
                .join(format!("{chat_id}.json"))
        )
        .await
        .unwrap());
    }

    #[tokio::test]
    async fn trajectory_save_namespace_malformed_non_null_task_meta_rejects_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "namespace-malformed-task-meta";
        let mut payload = sample_trajectory(chat_id, "Malformed Task", "2024-01-01T00:00:01Z");
        payload["task_meta"] = json!({"task_id": "task-malformed"});

        let err = handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap_err();

        assert_eq!(err.status_code, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(err.message.contains("Invalid task_meta"));
        assert!(!tokio::fs::try_exists(
            dir.path()
                .join(".refact")
                .join("trajectories")
                .join(format!("{chat_id}.json"))
        )
        .await
        .unwrap());
        assert!(!tokio::fs::try_exists(
            dir.path()
                .join(".refact")
                .join("tasks")
                .join("task-malformed")
        )
        .await
        .unwrap());
    }

    #[tokio::test]
    async fn trajectory_id_mismatch_title_generation_does_not_update_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "title-mismatch-guard";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        let path = trajectories_dir.join(format!("{chat_id}.json"));
        write_trajectory_file(&path, "other-chat", "New Chat", "2024-01-01T00:00:00Z").await;
        let before = tokio::fs::read_to_string(&path).await.unwrap();

        spawn_title_generation_task(
            gcx,
            chat_id.to_string(),
            vec![json!({"role":"user","content":"Generate guarded title"})],
            path.clone(),
            TrajectorySourceIdentity::Normal,
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        assert_eq!(tokio::fs::read_to_string(path).await.unwrap(), before);
    }

    #[tokio::test]
    async fn title_generation_mismatched_backing_file_does_not_update_active_session() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "title-active-mismatch-guard";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        let path = trajectories_dir.join(format!("{chat_id}.json"));
        write_trajectory_file(&path, "other-chat", "New Chat", "2024-01-01T00:00:00Z").await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.title = "New Chat".to_string();
            session.thread.is_title_generated = false;
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        spawn_title_generation_task(
            gcx,
            chat_id.to_string(),
            vec![json!({"role":"user","content":"Generate guarded active title"})],
            path.clone(),
            TrajectorySourceIdentity::Normal,
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let session = session_arc.lock().await;
        assert_eq!(session.thread.title, "New Chat");
        assert!(!session.thread.is_title_generated);
    }

    #[tokio::test]
    async fn generic_title_generation_with_active_buddy_updates_file_and_sse_only() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "title-generic-active-buddy";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        let path = trajectories_dir.join(format!("{chat_id}.json"));
        let expected_title = "Generic collision request";
        write_trajectory_file_with_user_message(&path, chat_id, "New Chat", expected_title).await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.title = "Buddy Session Title".to_string();
            session.thread.mode = "buddy".to_string();
            session.thread.buddy_meta = Some(buddy_thread_meta());
            session.thread.is_title_generated = false;
            session.trajectory_events_tx = Some(app.chat.trajectory_events_tx.clone());
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        spawn_title_generation_task(
            gcx,
            chat_id.to_string(),
            vec![json!({"role":"user","content":expected_title})],
            path.clone(),
            TrajectorySourceIdentity::Normal,
        );

        wait_for_file_title(&path, expected_title).await;
        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "updated");
        assert_eq!(event.title.as_deref(), Some(expected_title));
        assert_eq!(event.is_title_generated, Some(true));
        assert_eq!(event.session_state.as_deref(), Some("idle"));
        let session = session_arc.lock().await;
        assert_eq!(session.thread.title, "Buddy Session Title");
        assert!(!session.thread.is_title_generated);
        drop(session);

        let original = tokio::fs::read(&path).await.unwrap();
        let mut stale: TrajectoryData = serde_json::from_slice(&original).unwrap();
        stale.title = "Stale generated title".to_string();
        stale
            .extra
            .insert("isTitleGenerated".to_string(), json!(true));
        let mut replacement: serde_json::Value = serde_json::from_slice(&original).unwrap();
        replacement["created_at"] = json!("replacement-identity");
        replacement["messages"] = json!([]);
        let replacement = serde_json::to_vec(&replacement).unwrap();
        let tmp = unique_trajectory_tmp_path(&path);
        tokio::fs::write(&tmp, &replacement).await.unwrap();
        atomic_write_file(&tmp, &path).await.unwrap();
        let written =
            write_title_if_unchanged(&path, &original, &stale, TrajectorySourceIdentity::Normal)
                .await
                .unwrap();
        assert_eq!(tokio::fs::read(&path).await.unwrap(), replacement);
        assert!(!written);
    }

    #[tokio::test]
    async fn buddy_title_generation_with_active_generic_updates_file_without_generic_sse() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "title-buddy-active-generic";
        let buddy_dir = dir
            .path()
            .join(".refact")
            .join("buddy")
            .join("chats")
            .join("conversations");
        let path = buddy_dir.join(format!("{chat_id}.json"));
        let expected_title = "Buddy collision request";
        write_buddy_conversation_file(&path, chat_id, "New Chat").await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.title = "Generic Session Title".to_string();
            session.thread.is_title_generated = false;
            session.trajectory_events_tx = Some(app.chat.trajectory_events_tx.clone());
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        spawn_title_generation_task(
            gcx,
            chat_id.to_string(),
            vec![json!({"role":"user","content":expected_title})],
            path.clone(),
            TrajectorySourceIdentity::Buddy,
        );

        wait_for_file_title(&path, expected_title).await;
        let session = session_arc.lock().await;
        assert_eq!(session.thread.title, "Generic Session Title");
        assert!(!session.thread.is_title_generated);
        drop(session);
        assert_no_trajectory_event_for(&mut rx, std::time::Duration::from_millis(200)).await;
    }

    #[tokio::test]
    async fn task_title_generation_with_active_normal_updates_task_file_only() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "title-task-active-normal";
        let task_meta = task_meta(
            "task-title-normal",
            "agents",
            Some("agent-1"),
            Some("card-1"),
            None,
        );
        let path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-title-normal")
            .join("trajectories")
            .join("agents")
            .join("agent-1")
            .join(format!("{chat_id}.json"));
        let expected_title = "Task collision request";
        write_task_trajectory_file_with_user_message(
            &path,
            chat_id,
            "New Chat",
            expected_title,
            &task_meta,
        )
        .await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.title = "Normal Session Title".to_string();
            session.thread.is_title_generated = false;
            session.trajectory_events_tx = Some(app.chat.trajectory_events_tx.clone());
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        spawn_title_generation_task(
            gcx,
            chat_id.to_string(),
            vec![json!({"role":"user","content":expected_title})],
            path.clone(),
            TrajectorySourceIdentity::from_task_meta(&task_meta),
        );

        wait_for_file_title(&path, expected_title).await;
        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "updated");
        assert_eq!(event.title.as_deref(), Some(expected_title));
        assert_eq!(event.task_id.as_deref(), Some("task-title-normal"));
        assert_eq!(event.task_role.as_deref(), Some("agents"));
        assert_eq!(event.agent_id.as_deref(), Some("agent-1"));
        assert_eq!(event.card_id.as_deref(), Some("card-1"));
        let session = session_arc.lock().await;
        assert_eq!(session.thread.title, "Normal Session Title");
        assert!(!session.thread.is_title_generated);
    }

    #[tokio::test]
    async fn generic_title_generation_with_active_generic_updates_session() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "title-generic-active-generic";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        let path = trajectories_dir.join(format!("{chat_id}.json"));
        let expected_title = "Generic active request";
        write_trajectory_file_with_user_message(&path, chat_id, "New Chat", expected_title).await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.title = "New Chat".to_string();
            session.thread.is_title_generated = false;
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                expected_title.to_string(),
            ));
            session.trajectory_events_tx = Some(app.chat.trajectory_events_tx.clone());
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        spawn_title_generation_task(
            gcx,
            chat_id.to_string(),
            vec![json!({"role":"user","content":expected_title})],
            path.clone(),
            TrajectorySourceIdentity::Normal,
        );

        wait_for_session_title(&session_arc, expected_title).await;
        wait_for_file_title(&path, expected_title).await;
        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.title.as_deref(), Some(expected_title));
    }

    #[tokio::test]
    async fn active_buddy_title_generation_uses_set_title_side_effects_without_generic_sse() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut trajectory_rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "title-buddy-set-title-event";
        let buddy_dir = dir
            .path()
            .join(".refact")
            .join("buddy")
            .join("chats")
            .join("conversations");
        let path = buddy_dir.join(format!("{chat_id}.json"));
        let expected_title = "Buddy event request";
        write_buddy_conversation_file(&path, chat_id, "New Chat").await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.title = "New Chat".to_string();
            session.thread.mode = "buddy".to_string();
            session.thread.buddy_meta = Some(buddy_thread_meta());
            session.thread.is_title_generated = false;
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                expected_title.to_string(),
            ));
            session.trajectory_events_tx = Some(app.chat.trajectory_events_tx.clone());
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        spawn_title_generation_task(
            gcx,
            chat_id.to_string(),
            vec![json!({"role":"user","content":expected_title})],
            path.clone(),
            TrajectorySourceIdentity::Buddy,
        );

        wait_for_session_title(&session_arc, expected_title).await;
        {
            let session = session_arc.lock().await;
            assert!(session.trajectory_version > 0);
        }
        assert_no_trajectory_event_for(&mut trajectory_rx, std::time::Duration::from_millis(200))
            .await;
    }

    #[tokio::test]
    async fn title_generation_backing_file_matcher_rejects_source_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let chat_id = "title-source-match";
        let generic_path = dir.path().join("generic.json");
        let buddy_path = dir.path().join("buddy.json");
        let task_path = dir.path().join("task.json");
        let invalid_path = dir.path().join("invalid.json");
        let task_meta = task_meta(
            "task-title-source",
            "agents",
            Some("agent-title-source"),
            Some("card-title-source"),
            None,
        );
        let task_source = TrajectorySourceIdentity::from_task_meta(&task_meta);
        write_trajectory_file(
            &generic_path,
            chat_id,
            "Generic Title",
            "2024-01-01T00:00:00Z",
        )
        .await;
        write_buddy_conversation_file(&buddy_path, chat_id, "Buddy Title").await;
        write_task_trajectory_file_with_user_message(
            &task_path,
            chat_id,
            "Task Title",
            "task title source",
            &task_meta,
        )
        .await;
        let mut invalid = sample_trajectory(chat_id, "Invalid", "2024-01-01T00:00:00Z");
        invalid["task_meta"] = serde_json::to_value(&task_meta).unwrap();
        invalid["buddy_meta"] = json!({"is_buddy_chat": true, "buddy_chat_kind": "investigation"});
        tokio::fs::write(&invalid_path, serde_json::to_string(&invalid).unwrap())
            .await
            .unwrap();

        assert!(
            title_generation_backing_file_matches(
                &generic_path,
                chat_id,
                &TrajectorySourceIdentity::Normal,
            )
            .await
        );
        assert!(
            title_generation_backing_file_matches(
                &buddy_path,
                chat_id,
                &TrajectorySourceIdentity::Buddy,
            )
            .await
        );
        assert!(title_generation_backing_file_matches(&task_path, chat_id, &task_source).await);
        assert!(
            !title_generation_backing_file_matches(
                &generic_path,
                chat_id,
                &TrajectorySourceIdentity::Buddy,
            )
            .await
        );
        assert!(
            !title_generation_backing_file_matches(
                &buddy_path,
                chat_id,
                &TrajectorySourceIdentity::Normal,
            )
            .await
        );
        assert!(
            !title_generation_backing_file_matches(
                &task_path,
                chat_id,
                &TrajectorySourceIdentity::Normal,
            )
            .await
        );
        assert!(!title_generation_backing_file_matches(&invalid_path, chat_id, &task_source).await);
    }

    #[tokio::test]
    async fn pristine_session_is_born_clean_and_heals_from_late_trajectory() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "subchat-late-trajectory";

        let session_arc = crate::chat::get_or_create_session_with_trajectory(
            app.clone(),
            &app.chat.sessions,
            chat_id,
        )
        .await
        .unwrap();
        let mut chat_rx = {
            let session = session_arc.lock().await;
            assert!(session.messages.is_empty());
            assert!(!session.trajectory_dirty);
            session.subscribe()
        };

        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file_with_user_message(&path, chat_id, "Child Chat", "child work").await;

        let reopened = crate::chat::get_or_create_session_with_trajectory(
            app.clone(),
            &app.chat.sessions,
            chat_id,
        )
        .await
        .unwrap();
        assert!(Arc::ptr_eq(&session_arc, &reopened));
        {
            let session = reopened.lock().await;
            assert_eq!(session.messages.len(), 1);
            assert_eq!(session.thread.title, "Child Chat");
        }

        let json = chat_rx.recv().await.unwrap();
        let envelope: EventEnvelope = serde_json::from_str(&json).unwrap();
        match envelope.event {
            ChatEvent::Snapshot {
                messages, ..
            } => {
                assert_eq!(messages.len(), 1);
            }
            other => panic!("expected Snapshot, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pristine_session_save_does_not_overwrite_late_trajectory() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "subchat-cleanup-no-clobber";

        let session_arc = crate::chat::get_or_create_session_with_trajectory(
            app.clone(),
            &app.chat.sessions,
            chat_id,
        )
        .await
        .unwrap();

        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file_with_user_message(&path, chat_id, "Child Chat", "child work").await;

        let saved = try_save_trajectory(app.clone(), session_arc.clone())
            .await
            .unwrap();
        assert!(saved);

        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        assert_eq!(raw["messages"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn stale_refresh_skips_busy_dirty_or_drafting_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "subchat-refresh-guards";

        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file_with_user_message(&path, chat_id, "Child Chat", "child work").await;

        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        {
            let mut session = session_arc.lock().await;
            session.trajectory_dirty = true;
        }
        assert!(!refresh_session_from_trajectory_if_stale(app.clone(), chat_id, false).await);

        {
            let mut session = session_arc.lock().await;
            session.trajectory_dirty = false;
            session.runtime.state = SessionState::Generating;
        }
        assert!(!refresh_session_from_trajectory_if_stale(app.clone(), chat_id, false).await);

        {
            let mut session = session_arc.lock().await;
            session.runtime.state = SessionState::Idle;
            session.draft_message = Some(ChatMessage::new("user".to_string(), "draft".to_string()));
        }
        assert!(!refresh_session_from_trajectory_if_stale(app.clone(), chat_id, false).await);

        {
            let mut session = session_arc.lock().await;
            session.draft_message = None;
        }
        assert!(refresh_session_from_trajectory_if_stale(app.clone(), chat_id, false).await);
        let session = session_arc.lock().await;
        assert_eq!(session.messages.len(), 1);
    }

    #[tokio::test]
    async fn reopening_child_chat_replaces_stale_seed_messages() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "subchat-reopen-refresh";

        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let mut trajectory = sample_trajectory(chat_id, "Child Chat", "2024-01-01T00:00:01Z");
        trajectory["parent_id"] = json!("parent-chat");
        trajectory["link_type"] = json!("subagent");
        trajectory["messages"] = json!([{ "role": "user", "content": "seed prompt" }]);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, serde_json::to_string(&trajectory).unwrap())
            .await
            .unwrap();

        let session_arc = crate::chat::get_or_create_session_with_trajectory(
            app.clone(),
            &app.chat.sessions,
            chat_id,
        )
        .await
        .unwrap();
        {
            let session = session_arc.lock().await;
            assert_eq!(session.messages.len(), 1);
            assert_eq!(session.thread.parent_id.as_deref(), Some("parent-chat"));
        }

        trajectory["messages"] = json!([
            { "role": "user", "content": "seed prompt" },
            { "role": "assistant", "content": "final answer" }
        ]);
        tokio::fs::write(&path, serde_json::to_string(&trajectory).unwrap())
            .await
            .unwrap();

        let reopened = crate::chat::get_or_create_session_with_trajectory(
            app.clone(),
            &app.chat.sessions,
            chat_id,
        )
        .await
        .unwrap();
        assert!(Arc::ptr_eq(&session_arc, &reopened));
        let session = reopened.lock().await;
        assert_eq!(session.messages.len(), 2);
    }

    #[tokio::test]
    async fn trajectory_id_missing_is_not_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join("missing-id.json");
        let mut payload = sample_trajectory("missing-id", "Missing Id", "2024-01-01T00:00:00Z");
        payload.as_object_mut().unwrap().remove("id");
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, serde_json::to_string(&payload).unwrap())
            .await
            .unwrap();

        assert!(load_trajectory_for_chat(gcx, "missing-id").await.is_none());
    }

    #[test]
    fn test_validate_trajectory_id_rejects_forward_slash() {
        assert!(validate_trajectory_id("a/b").is_err());
        assert!(validate_trajectory_id("/absolute").is_err());
    }

    #[test]
    fn test_validate_trajectory_id_rejects_backslash() {
        assert!(validate_trajectory_id("a\\b").is_err());
        assert!(validate_trajectory_id("\\windows\\path").is_err());
    }

    #[test]
    fn test_validate_trajectory_id_rejects_null_byte() {
        assert!(validate_trajectory_id("test\0id").is_err());
    }

    #[test]
    fn test_validate_trajectory_id_accepts_valid() {
        assert!(validate_trajectory_id("abc-123").is_ok());
        assert!(validate_trajectory_id("chat_456").is_ok());
        assert!(validate_trajectory_id("550e8400-e29b-41d4-a716-446655440000").is_ok());
        assert!(validate_trajectory_id("planner-task-1").is_ok());
        assert!(validate_trajectory_id("A1b2C3").is_ok());
    }

    #[test]
    fn test_validate_trajectory_id_rejects_empty() {
        assert!(validate_trajectory_id("").is_err());
    }

    #[test]
    fn test_validate_trajectory_id_rejects_too_long() {
        let long_id = "a".repeat(129);
        assert!(validate_trajectory_id(&long_id).is_err());
        let max_id = "a".repeat(128);
        assert!(validate_trajectory_id(&max_id).is_ok());
    }

    #[test]
    fn test_validate_trajectory_id_rejects_invalid_chars() {
        assert!(validate_trajectory_id("has space").is_err());
        assert!(validate_trajectory_id("has.dot").is_err());
        assert!(validate_trajectory_id("has@symbol").is_err());
        assert!(validate_trajectory_id("has#hash").is_err());
    }

    #[test]
    fn test_trajectory_meta_uses_bounded_title() {
        let data = TrajectoryData {
            id: "big-title-chat".to_string(),
            title: "B".repeat(1024 * 1024),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            model: "model".to_string(),
            mode: "agent".to_string(),
            tool_use: "agent".to_string(),
            messages: Vec::new(),
            extra: serde_json::Map::new(),
        };
        let meta = trajectory_data_to_meta(&data);
        assert_eq!(meta.title.chars().count(), TRAJECTORY_META_TITLE_MAX_CHARS);
    }

    #[test]
    fn test_cursor_precedes_item_matches_descending_order() {
        assert!(cursor_precedes_item(
            ("2024-01-01T00:00:00Z", "a"),
            ("2024-01-02T00:00:00Z", "b"),
        ));
        assert!(cursor_precedes_item(
            ("2024-01-01T00:00:00Z", "a"),
            ("2024-01-01T00:00:00Z", "b"),
        ));
        assert!(!cursor_precedes_item(
            ("2024-01-03T00:00:00Z", "a"),
            ("2024-01-02T00:00:00Z", "b"),
        ));
    }

    #[test]
    fn build_title_generation_context_skips_hidden_plan_and_event_roles() {
        let messages = vec![
            json!({"role": "user", "content": "Visible user request"}),
            json!({
                "role": "plan",
                "content": "Hidden base plan must not title the chat",
                "extra": {"plan": {"mode": "agent", "version": 1}}
            }),
            json!({
                "role": "event",
                "content": "Hidden plan delta must not title the chat",
                "extra": {
                    "event": {
                        "subkind": "plan_delta",
                        "source": "tool.update_plan",
                        "payload": {"seq": 1}
                    }
                }
            }),
            json!({"role": "assistant", "content": "Visible assistant response"}),
        ];

        let context = build_title_generation_context(&messages);

        assert!(context.contains("Visible user request"));
        assert!(context.contains("Visible assistant response"));
        assert!(!context.contains("Hidden base plan"));
        assert!(!context.contains("Hidden plan delta"));
    }

    #[test]
    fn build_title_generation_context_skips_ui_only_reactive_compaction_report() {
        let messages = vec![
            json!({
                "role": "summarization",
                "content": "Legacy diagnostic report",
                "summarization_tier": "legacy_reactive",
                "_ui_only": true
            }),
            json!({"role": "user", "content": "Fix sanitizers"}),
        ];
        let context = build_title_generation_context(&messages);
        assert!(context.contains("Fix sanitizers"));
        assert!(!context.contains("Legacy diagnostic report"));
    }

    #[test]
    fn build_title_generation_context_skips_extra_ui_only_reactive_report() {
        let messages = vec![
            json!({
                "role": "summarization",
                "content": "Legacy diagnostic report",
                "summarization_tier": "legacy_reactive",
                "extra": {"_ui_only": true}
            }),
            json!({"role": "user", "content": "Fix sanitizers"}),
        ];
        let context = build_title_generation_context(&messages);
        assert!(context.contains("Fix sanitizers"));
        assert!(!context.contains("Legacy diagnostic report"));
    }

    #[test]
    fn test_fix_tool_call_indexes_sets_missing() {
        use crate::call_validation::{ChatToolCall, ChatToolFunction};
        let mut messages = vec![ChatMessage {
            role: "assistant".to_string(),
            tool_calls: Some(vec![
                ChatToolCall {
                    id: "call_1".to_string(),
                    index: None,
                    function: ChatToolFunction {
                        name: "test".to_string(),
                        arguments: "{}".to_string(),
                    },
                    tool_type: "function".to_string(),
                    extra_content: None,
                    started_at_ms: None,
                    completed_at_ms: None,
                },
                ChatToolCall {
                    id: "call_2".to_string(),
                    index: None,
                    function: ChatToolFunction {
                        name: "test2".to_string(),
                        arguments: "{}".to_string(),
                    },
                    tool_type: "function".to_string(),
                    extra_content: None,
                    started_at_ms: None,
                    completed_at_ms: None,
                },
            ]),
            ..Default::default()
        }];
        fix_tool_call_indexes(&mut messages);
        let tool_calls = messages[0].tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls[0].index, Some(0));
        assert_eq!(tool_calls[1].index, Some(1));
    }

    #[test]
    fn test_fix_tool_call_indexes_preserves_existing() {
        use crate::call_validation::{ChatToolCall, ChatToolFunction};
        let mut messages = vec![ChatMessage {
            role: "assistant".to_string(),
            tool_calls: Some(vec![ChatToolCall {
                id: "call_1".to_string(),
                index: Some(5),
                function: ChatToolFunction {
                    name: "test".to_string(),
                    arguments: "{}".to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
                started_at_ms: None,
                completed_at_ms: None,
            }]),
            ..Default::default()
        }];
        fix_tool_call_indexes(&mut messages);
        let tool_calls = messages[0].tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls[0].index, Some(5));
    }

    fn metric_scan_tool_call(
        id: &str,
        name: &str,
        arguments: serde_json::Value,
    ) -> crate::call_validation::ChatToolCall {
        crate::call_validation::ChatToolCall {
            id: id.to_string(),
            index: None,
            function: crate::call_validation::ChatToolFunction {
                arguments: arguments.to_string(),
                name: name.to_string(),
            },
            tool_type: "function".to_string(),
            extra_content: None,
            started_at_ms: None,
            completed_at_ms: None,
        }
    }

    fn metric_scan_usage(
        prompt: usize,
        completion: usize,
        total: usize,
        cache_read: Option<usize>,
        cache_creation: Option<usize>,
        total_usd: Option<f64>,
    ) -> crate::call_validation::ChatUsage {
        crate::call_validation::ChatUsage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: total,
            cache_read_tokens: cache_read,
            cache_creation_tokens: cache_creation,
            metering_usd: total_usd.map(|total_usd| crate::call_validation::MeteringUsd {
                prompt_usd: 0.0,
                generated_usd: 0.0,
                cache_read_usd: None,
                cache_creation_usd: None,
                total_usd,
            }),
        }
    }

    fn metric_scan_fixture_messages() -> Vec<ChatMessage> {
        vec![
            ChatMessage {
                role: "user".to_string(),
                content: ChatContent::Multimodal(vec![
                    crate::call_validation::MultimodalElement {
                        m_type: "text".to_string(),
                        m_content: "please refactor".to_string(),
                    },
                    crate::call_validation::MultimodalElement {
                        m_type: "image/png".to_string(),
                        m_content: "base64".to_string(),
                    },
                ]),
                ..Default::default()
            },
            ChatMessage {
                role: "assistant".to_string(),
                thinking_blocks: Some(vec![json!({"type": "thinking", "thinking": "plan it"})]),
                tool_calls: Some(vec![
                    metric_scan_tool_call(
                        "call-a",
                        "tasks_set",
                        json!({"tasks": [
                            {"status": "completed"},
                            {"status": "FAILED"},
                            {"status": "in_progress"},
                        ]}),
                    ),
                    metric_scan_tool_call("call-r", "cat", json!({"paths": "a.rs"})),
                ]),
                usage: Some(metric_scan_usage(
                    100,
                    50,
                    150,
                    Some(10),
                    Some(5),
                    Some(0.03),
                )),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                tool_call_id: "call-a".to_string(),
                usage: Some(metric_scan_usage(1, 2, 3, None, None, None)),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                tool_call_id: "call-r".to_string(),
                tool_failed: Some(true),
                ..Default::default()
            },
            ChatMessage {
                role: "diff".to_string(),
                content: ChatContent::SimpleText(
                    json!([
                        {"file_name": "a.rs", "lines_add": "a\nb\nc", "lines_remove": "y\nz"},
                        {"file_name": "b.rs", "lines_add": "x\n", "lines_remove": ""},
                    ])
                    .to_string(),
                ),
                ..Default::default()
            },
            ChatMessage {
                role: "assistant".to_string(),
                tool_calls: Some(vec![metric_scan_tool_call(
                    "call-b",
                    "tasks_set",
                    json!({"tasks": [{"status": "done"}, {"status": "done"}]}),
                )]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                tool_call_id: "call-b".to_string(),
                tool_failed: Some(true),
                ..Default::default()
            },
            ChatMessage {
                role: "assistant".to_string(),
                tool_calls: Some(vec![metric_scan_tool_call(
                    "call-c",
                    "tasks_set",
                    json!({"not_tasks": []}),
                )]),
                ..Default::default()
            },
            ChatMessage {
                role: "tool".to_string(),
                tool_call_id: "call-c".to_string(),
                ..Default::default()
            },
            ChatMessage {
                role: "diff".to_string(),
                content: ChatContent::SimpleText("not json".to_string()),
                ..Default::default()
            },
            ChatMessage {
                role: "assistant".to_string(),
                content: ChatContent::SimpleText("done".to_string()),
                usage: Some(metric_scan_usage(200, 100, 300, None, None, None)),
                ..Default::default()
            },
        ]
    }

    #[test]
    fn metric_scan_results_identical_to_previous_implementation() {
        let messages = metric_scan_fixture_messages();
        let metrics = scan_trajectory_metrics(&messages);
        assert_eq!(metrics.line_changes, (4, 2));
        assert_eq!(metrics.task_progress, (3, 1, 1));
        assert_eq!(metrics.token_totals.prompt_tokens, 301);
        assert_eq!(metrics.token_totals.completion_tokens, 152);
        assert_eq!(metrics.token_totals.total_tokens, 453);
        assert_eq!(metrics.token_totals.cache_read_tokens, 10);
        assert_eq!(metrics.token_totals.cache_creation_tokens, 5);
        assert_eq!(metrics.token_totals.cost_usd, Some(0.03));
    }

    #[test]
    fn metric_scan_correct_after_history_truncation() {
        let messages = metric_scan_fixture_messages();
        for (tail, expected_lines, expected_progress) in [
            (2usize, (0i64, 0i64), (0i32, 0i32, 0i32)),
            (4, (0, 0), (3, 1, 1)),
            (5, (4, 2), (3, 1, 1)),
            (8, (4, 2), (3, 1, 1)),
        ] {
            let truncated = messages[..tail].to_vec();
            let metrics = scan_trajectory_metrics(&truncated);
            assert_eq!(metrics.line_changes, expected_lines, "truncated to {tail}");
            assert_eq!(
                metrics.task_progress, expected_progress,
                "truncated to {tail}"
            );
        }

        let rewritten = messages[5..].to_vec();
        let metrics = scan_trajectory_metrics(&rewritten);
        assert_eq!(metrics.line_changes, (0, 0));
        assert_eq!(metrics.task_progress, (0, 0, 0));
        assert_eq!(metrics.token_totals.prompt_tokens, 200);
        assert_eq!(metrics.token_totals.completion_tokens, 100);
        assert_eq!(metrics.token_totals.total_tokens, 300);
        assert_eq!(metrics.token_totals.cache_read_tokens, 0);
        assert_eq!(metrics.token_totals.cache_creation_tokens, 0);
        assert_eq!(metrics.token_totals.cost_usd, None);
    }

    fn mark_active_compression(session: &mut ChatSession, phase: CompressionPhase) {
        session.is_compressing = true;
        session.runtime.is_compressing = true;
        session.compression_phase = Some(phase);
        session.runtime.compression_phase = Some(phase);
        session.compression_reason = Some(CompressionReason::PressureLow);
        session.runtime.compression_reason = Some(CompressionReason::PressureLow);
        session.compression_attempt_generation = 9;
        session.active_compression_attempt = Some(9);
    }

    async fn assert_external_remove_clears_active_compression(phase: CompressionPhase) {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = format!("external-remove-{:?}", phase).to_lowercase();
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.clone())));
        let mut chat_rx = {
            let mut session = session_arc.lock().await;
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                "before delete".to_string(),
            ));
            mark_active_compression(&mut session, phase);
            session.trajectory_dirty = false;
            session.subscribe()
        };
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.clone(), session_arc.clone());

        process_trajectory_change(gcx, &chat_id, true).await;

        let session = session_arc.lock().await;
        assert!(session.messages.is_empty());
        assert!(!session.is_compressing);
        assert!(!session.runtime.is_compressing);
        assert_eq!(session.compression_phase, None);
        assert_eq!(session.runtime.compression_phase, None);
        assert_eq!(session.compression_reason, None);
        assert_eq!(session.runtime.compression_reason, None);
        assert_eq!(session.active_compression_attempt, None);
        drop(session);

        let json = chat_rx.recv().await.unwrap();
        let envelope: EventEnvelope = serde_json::from_str(&json).unwrap();
        match envelope.event {
            ChatEvent::Snapshot {
                runtime,
                messages,
                ..
            } => {
                assert!(messages.is_empty());
                assert!(!runtime.is_compressing);
                assert_eq!(runtime.compression_phase, None);
                assert_eq!(runtime.compression_reason, None);
            }
            other => panic!("expected Snapshot, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn external_remove_clears_checking_compression_state_and_snapshot() {
        assert_external_remove_clears_active_compression(CompressionPhase::Checking).await;
    }

    #[tokio::test]
    async fn immediate_external_delete_with_loadable_trajectory_reloads_instead_of_clearing() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "immediate-delete-loadable-reloads";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file_with_user_message(
            &path,
            chat_id,
            "Reloaded After Stale Delete",
            "recreated message",
        )
        .await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        let mut chat_rx = {
            let mut session = session_arc.lock().await;
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                "old in-memory message".to_string(),
            ));
            session.thread.title = "Old In Memory".to_string();
            session.external_reload_pending = Some(ExternalReloadPending::delete(
                TrajectorySourceIdentity::Normal,
            ));
            session.runtime.state = SessionState::Idle;
            session.trajectory_dirty = false;
            session.subscribe()
        };
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        let before = serde_json::to_value(&session_arc.lock().await.thread).unwrap();
        assert_eq!(before["id"], chat_id);
        let bytes = std::fs::read(&path).unwrap();
        let transition =
            super::super::worktree_lifecycle::try_transition_lock(&session_arc).unwrap();
        process_trajectory_change(gcx.clone(), chat_id, true).await;
        assert_eq!(
            serde_json::to_value(&session_arc.lock().await.thread).unwrap(),
            before
        );
        assert_eq!(
            session_arc.lock().await.messages[0]
                .content
                .content_text_only(),
            "old in-memory message"
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(
            session_arc.lock().await.external_reload_pending,
            Some(ExternalReloadPending::delete(
                TrajectorySourceIdentity::Normal
            ))
        );
        assert!(chat_rx.try_recv().is_err());
        drop(transition);
        process_trajectory_change(gcx.clone(), chat_id, true).await;

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "updated");
        assert_eq!(event.title.as_deref(), Some("Reloaded After Stale Delete"));

        let session = session_arc.lock().await;
        assert_eq!(session.external_reload_pending, None);
        assert_eq!(session.thread.title, "Reloaded After Stale Delete");
        assert_eq!(session.messages.len(), 1);
        assert_eq!(
            session.messages[0].content.content_text_only(),
            "recreated message"
        );
        drop(session);

        let json = chat_rx.recv().await.unwrap();
        let envelope: EventEnvelope = serde_json::from_str(&json).unwrap();
        match envelope.event {
            ChatEvent::Snapshot {
                thread,
                messages,
                ..
            } => {
                assert_eq!(thread.title, "Reloaded After Stale Delete");
                assert_eq!(messages.len(), 1);
            }
            other => panic!("expected Snapshot, got {other:?}"),
        }

        let mut foreign = load_trajectory_for_chat(gcx.clone(), chat_id)
            .await
            .unwrap();
        foreign.thread.worktree = Some(trajectory_worktree_sample_with_id("foreign-checkout"));
        let before = serde_json::to_value(&session_arc.lock().await.thread).unwrap();
        assert!(!foreign.thread.worktree.as_ref().unwrap().id.is_empty());
        assert!(
            !apply_loaded_external_update_with_repair(
                gcx.clone(),
                session_arc.clone(),
                chat_id,
                foreign,
                None,
                Some(ExternalReloadPending::update(
                    TrajectorySourceIdentity::Normal
                )),
                "foreign reload",
            )
            .await
        );
        assert_eq!(
            serde_json::to_value(&session_arc.lock().await.thread).unwrap(),
            before
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let mut current = load_trajectory_for_chat(gcx.clone(), chat_id)
            .await
            .unwrap();
        current.thread.title = "Current attachment reload".into();
        let transition =
            super::super::worktree_lifecycle::try_transition_lock(&session_arc).unwrap();
        assert!(
            !apply_loaded_external_update_with_repair(
                gcx.clone(),
                session_arc.clone(),
                chat_id,
                current,
                None,
                Some(ExternalReloadPending::update(
                    TrajectorySourceIdentity::Normal
                )),
                "current blocked reload",
            )
            .await
        );
        assert_eq!(
            serde_json::to_value(&session_arc.lock().await.thread).unwrap(),
            before
        );
        drop(transition);
        check_external_reload_pending(gcx, session_arc.clone()).await;
        assert_eq!(session_arc.lock().await.external_reload_pending, None);
    }

    #[tokio::test]
    async fn stale_pending_delete_helper_does_not_overwrite_newer_update_pending() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "stale-delete-keeps-update-pending";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file_with_user_message(
            &path,
            chat_id,
            "Newer Update",
            "newer update message",
        )
        .await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.external_reload_pending = Some(ExternalReloadPending::update(
                TrajectorySourceIdentity::Normal,
            ));
            session.runtime.state = SessionState::Generating;
            session.trajectory_dirty = false;
        }

        let outcome = apply_external_delete_with_revalidation(
            gcx,
            session_arc.clone(),
            chat_id,
            Some(ExternalReloadPending::delete(
                TrajectorySourceIdentity::Normal,
            )),
            None,
        )
        .await;

        let session = session_arc.lock().await;
        assert!(matches!(
            outcome,
            ExternalDeleteRevalidationOutcome::NoopStalePending
        ));
        assert_eq!(
            session.external_reload_pending,
            Some(ExternalReloadPending::update(
                TrajectorySourceIdentity::Normal
            ))
        );
    }

    #[tokio::test]
    async fn pending_external_delete_applies_when_session_becomes_reloadable() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "pending-external-delete";
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        let mut chat_rx = {
            let mut session = session_arc.lock().await;
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                "before deferred delete".to_string(),
            ));
            session.thread.title = "Delete Pending".to_string();
            session.thread.reactive_compact_attempts = Some(2);
            session.created_at = "2024-01-01T00:00:00Z".to_string();
            session.wake_up_at = Some(chrono::Utc::now() + chrono::Duration::minutes(10));
            session.waiting_for_card_ids = vec!["card-stale".to_string()];
            session.runtime.state = SessionState::Generating;
            session.trajectory_dirty = true;
            mark_active_compression(&mut session, CompressionPhase::Running);
            session.subscribe()
        };
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        process_trajectory_change(gcx.clone(), chat_id, true).await;
        {
            let session = session_arc.lock().await;
            assert_eq!(
                session.external_reload_pending,
                Some(ExternalReloadPending::delete(
                    TrajectorySourceIdentity::Normal
                ))
            );
            assert_eq!(session.messages.len(), 1);
            assert!(session.active_compression_attempt.is_some());
        }

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "deleted");

        {
            let mut session = session_arc.lock().await;
            session.runtime.state = SessionState::Idle;
            session.trajectory_dirty = false;
        }
        check_external_reload_pending(gcx, session_arc.clone()).await;

        let session = session_arc.lock().await;
        assert_eq!(session.external_reload_pending, None);
        assert!(session.messages.is_empty());
        assert_eq!(session.thread.id, chat_id);
        assert_eq!(session.thread.title, ThreadParams::default().title);
        assert_eq!(session.thread.reactive_compact_attempts, None);
        assert_ne!(session.created_at, "2024-01-01T00:00:00Z");
        assert!(session.wake_up_at.is_none());
        assert!(session.waiting_for_card_ids.is_empty());
        assert!(!session.is_compressing);
        assert!(!session.runtime.is_compressing);
        assert_eq!(session.compression_phase, None);
        assert_eq!(session.runtime.compression_phase, None);
        assert_eq!(session.compression_reason, None);
        assert_eq!(session.runtime.compression_reason, None);
        assert_eq!(session.active_compression_attempt, None);
        drop(session);

        let json = chat_rx.recv().await.unwrap();
        let envelope: EventEnvelope = serde_json::from_str(&json).unwrap();
        match envelope.event {
            ChatEvent::Snapshot {
                thread,
                runtime,
                messages,
                ..
            } => {
                assert_eq!(thread.id, chat_id);
                assert_eq!(thread.reactive_compact_attempts, None);
                assert!(messages.is_empty());
                assert!(!runtime.is_compressing);
                assert_eq!(runtime.compression_phase, None);
            }
            other => panic!("expected Snapshot, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pending_external_delete_revalidates_recreated_trajectory_before_clearing() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "pending-delete-recreated-reloads";
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        let mut chat_rx = {
            let mut session = session_arc.lock().await;
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                "before stale delete".to_string(),
            ));
            session.thread.title = "Busy Before Delete".to_string();
            session.runtime.state = SessionState::Generating;
            session.trajectory_dirty = true;
            session.subscribe()
        };
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        process_trajectory_change(gcx.clone(), chat_id, true).await;
        {
            let session = session_arc.lock().await;
            assert_eq!(
                session.external_reload_pending,
                Some(ExternalReloadPending::delete(
                    TrajectorySourceIdentity::Normal
                ))
            );
            assert_eq!(session.thread.title, "Busy Before Delete");
        }

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "deleted");

        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file_with_user_message(
            &path,
            chat_id,
            "Recreated Before Apply",
            "recreated before pending delete",
        )
        .await;
        {
            let mut session = session_arc.lock().await;
            session.runtime.state = SessionState::Idle;
            session.trajectory_dirty = false;
        }
        check_external_reload_pending(gcx, session_arc.clone()).await;

        let session = session_arc.lock().await;
        assert_eq!(session.external_reload_pending, None);
        assert_eq!(session.thread.title, "Recreated Before Apply");
        assert_eq!(session.messages.len(), 1);
        assert_eq!(
            session.messages[0].content.content_text_only(),
            "recreated before pending delete"
        );
        drop(session);

        let json = chat_rx.recv().await.unwrap();
        let envelope: EventEnvelope = serde_json::from_str(&json).unwrap();
        match envelope.event {
            ChatEvent::Snapshot {
                thread,
                messages,
                ..
            } => {
                assert_eq!(thread.title, "Recreated Before Apply");
                assert_eq!(messages.len(), 1);
            }
            other => panic!("expected Snapshot, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn generic_watcher_remove_does_not_clear_active_buddy_session() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "active-buddy-remove-guard";
        let buddy_path = dir
            .path()
            .join(".refact")
            .join("buddy")
            .join("chats")
            .join("conversations")
            .join(format!("{chat_id}.json"));
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(
            &normal_path,
            chat_id,
            "Removed Normal",
            "2024-01-01T00:00:01Z",
        )
        .await;
        write_buddy_conversation_file(&buddy_path, chat_id, "Active Buddy File").await;
        tokio::fs::remove_file(&normal_path).await.unwrap();
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        let mut chat_rx = {
            let mut session = session_arc.lock().await;
            session.thread.title = "Active Buddy Session".to_string();
            session.thread.mode = "buddy".to_string();
            session.thread.buddy_meta = Some(buddy_thread_meta());
            session.external_reload_pending = Some(ExternalReloadPending::update(
                TrajectorySourceIdentity::Buddy,
            ));
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                "keep buddy message".to_string(),
            ));
            session.runtime.state = SessionState::Idle;
            session.trajectory_dirty = false;
            session.subscribe()
        };
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        process_trajectory_change(gcx.clone(), chat_id, true).await;
        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "deleted");
        assert_eq!(event.title, None);
        assert!(tokio::fs::try_exists(&buddy_path).await.unwrap());
        assert!(!tokio::fs::try_exists(&normal_path).await.unwrap());

        let session = session_arc.lock().await;
        assert_eq!(
            session.external_reload_pending,
            Some(ExternalReloadPending::update(
                TrajectorySourceIdentity::Buddy
            ))
        );
        assert_eq!(session.thread.title, "Active Buddy Session");
        assert!(session.thread.buddy_meta.is_some());
        assert_eq!(session.messages.len(), 1);
        assert_eq!(
            session.messages[0].content.content_text_only(),
            "keep buddy message"
        );
        drop(session);

        assert_no_chat_event_for(&mut chat_rx, std::time::Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn generic_watcher_remove_with_active_buddy_persists_repaired_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "active-buddy-remove-repaired-fallback";
        let fallback_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-active-buddy-repair-fallback")
            .join("trajectories")
            .join("planner")
            .join(format!("{chat_id}.json"));
        tokio::fs::create_dir_all(fallback_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &fallback_path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Active Buddy Repaired Fallback",
                "model": "model",
                "mode": "task_planner",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "mode_transition",
                "previous_response_id": "resp_source",
                "messages": [
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"fallback survives active buddy repair"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2026-05-29T00:00:00Z",
                    "system_prompt": "source system",
                    "tools_canonical": [{"type":"function","function":{"name":"source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:01Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "custom_future_field": {"keep": true}
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        let mut chat_rx = {
            let mut session = session_arc.lock().await;
            session.thread.title = "Active Buddy Session".to_string();
            session.thread.mode = "buddy".to_string();
            session.thread.buddy_meta = Some(buddy_thread_meta());
            session.external_reload_pending = Some(ExternalReloadPending::delete(
                TrajectorySourceIdentity::Buddy,
            ));
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                "keep active buddy during repaired fallback".to_string(),
            ));
            session.runtime.state = SessionState::Generating;
            session.trajectory_dirty = false;
            session.subscribe()
        };
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        process_trajectory_change(gcx.clone(), chat_id, true).await;

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "updated");
        assert_eq!(
            event.title.as_deref(),
            Some("Active Buddy Repaired Fallback")
        );
        assert_eq!(
            event.task_id.as_deref(),
            Some("task-active-buddy-repair-fallback")
        );
        assert_eq!(event.session_state.as_deref(), Some("idle"));

        let session = session_arc.lock().await;
        assert_eq!(
            session.external_reload_pending,
            Some(ExternalReloadPending::delete(
                TrajectorySourceIdentity::Buddy
            ))
        );
        assert_eq!(session.thread.title, "Active Buddy Session");
        assert_eq!(session.thread.mode, "buddy");
        assert!(session.thread.buddy_meta.is_some());
        assert_eq!(session.messages.len(), 1);
        assert_eq!(
            session.messages[0].content.content_text_only(),
            "keep active buddy during repaired fallback"
        );
        drop(session);

        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&fallback_path).await.unwrap())
                .unwrap();
        assert_eq!(
            raw["frozen_request_prefix"]["system_prompt"],
            "target task planner system"
        );
        assert!(raw["frozen_request_prefix"]["tools_canonical"].is_null());
        assert!(raw.get("claude_code_identity").is_none());
        assert!(raw.get("previous_response_id").is_none());
        assert_eq!(raw["custom_future_field"]["keep"], true);
        assert_no_chat_event_for(&mut chat_rx, std::time::Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn generic_watcher_update_with_active_buddy_persists_repaired_generic() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "active-buddy-update-repaired-generic";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Repaired Generic Update",
                "model": "model",
                "mode": "task_planner",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "mode_transition",
                "previous_response_id": "resp_source",
                "messages": [
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"generic update active buddy repair"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2026-05-29T00:00:00Z",
                    "system_prompt": "source system",
                    "tools_canonical": [{"type":"function","function":{"name":"source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:01Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "custom_future_field": {"keep": true}
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        let mut chat_rx = {
            let mut session = session_arc.lock().await;
            session.thread.title = "Active Buddy Session".to_string();
            session.thread.mode = "buddy".to_string();
            session.thread.buddy_meta = Some(buddy_thread_meta());
            session.external_reload_pending = Some(ExternalReloadPending::update(
                TrajectorySourceIdentity::Buddy,
            ));
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                "keep active buddy during repaired update".to_string(),
            ));
            session.runtime.state = SessionState::Generating;
            session.trajectory_dirty = true;
            session.subscribe()
        };
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        process_trajectory_change(gcx.clone(), chat_id, false).await;
        check_external_reload_pending(gcx.clone(), session_arc.clone()).await;

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "updated");
        assert_eq!(event.title.as_deref(), Some("Repaired Generic Update"));
        assert_eq!(event.session_state.as_deref(), Some("idle"));

        let session = session_arc.lock().await;
        assert_eq!(
            session.external_reload_pending,
            Some(ExternalReloadPending::update(
                TrajectorySourceIdentity::Buddy
            ))
        );
        assert_eq!(session.thread.title, "Active Buddy Session");
        assert_eq!(session.thread.mode, "buddy");
        assert!(session.thread.buddy_meta.is_some());
        assert_eq!(session.messages.len(), 1);
        assert_eq!(
            session.messages[0].content.content_text_only(),
            "keep active buddy during repaired update"
        );
        drop(session);

        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        assert_eq!(
            raw["frozen_request_prefix"]["system_prompt"],
            "target task planner system"
        );
        assert!(raw["frozen_request_prefix"]["tools_canonical"].is_null());
        assert!(raw.get("claude_code_identity").is_none());
        assert!(raw.get("previous_response_id").is_none());
        assert_eq!(raw["custom_future_field"]["keep"], true);
        assert_no_chat_event_for(&mut chat_rx, std::time::Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn normal_save_under_session_lock_does_not_reenter_or_leak_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let id = "locked-snapshot-publication";
        let session = Arc::new(AMutex::new(ChatSession::new(id.into())));
        app.chat
            .sessions
            .write()
            .await
            .insert(id.into(), session.clone());
        let mut locked = session.lock().await;
        locked.runtime.state = SessionState::Generating;
        locked.runtime.error = Some("not snapshot runtime".into());
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            save_trajectory_snapshot(
                gcx,
                test_snapshot(
                    id,
                    "Snapshot",
                    vec![ChatMessage::new("user".into(), "keep".into())],
                ),
            ),
        )
        .await
        .expect("saving must not reacquire the held session lock")
        .unwrap();
        let event = wait_for_trajectory_event(&mut rx, id).await;
        assert_eq!(event.session_state, None);
        assert_eq!(event.error, None);
        drop(locked);
    }

    #[tokio::test]
    async fn normal_save_sse_keeps_matching_active_normal_state_error_and_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "normal-save-active-normal-enriched";
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.worktree = Some(trajectory_worktree_sample_with_id(
                "normal-save-enriched-wt",
            ));
            session.runtime.state = SessionState::Generating;
            session.runtime.error = Some("normal save enriched error".to_string());
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc);

        save_trajectory_snapshot(
            gcx,
            test_snapshot(
                chat_id,
                "Normal Save Enriched",
                vec![ChatMessage::new(
                    "user".to_string(),
                    "normal save".to_string(),
                )],
            ),
        )
        .await
        .unwrap();

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "updated");
        assert_eq!(event.session_state.as_deref(), Some("generating"));
        assert_eq!(event.error.as_deref(), Some("normal save enriched error"));
        assert_eq!(
            event.worktree.as_ref().map(|worktree| worktree.id.as_str()),
            Some("normal-save-enriched-wt")
        );
    }

    #[tokio::test]
    async fn task_http_save_sse_keeps_matching_active_task_state_error_and_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "task-http-save-active-task-enriched";
        let task_id = "task-http-save-active-task";
        tokio::fs::create_dir_all(dir.path().join(".refact").join("tasks").join(task_id))
            .await
            .unwrap();
        let task_meta = task_meta(
            task_id,
            "agents",
            Some("agent-http-save-enriched"),
            Some("card-http-save-enriched"),
            None,
        );
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.task_meta = Some(task_meta);
            session.thread.worktree =
                Some(trajectory_worktree_sample_with_id("task-save-enriched-wt"));
            session.runtime.state = SessionState::Generating;
            session.runtime.error = Some("task save enriched error".to_string());
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc);
        let mut payload =
            sample_trajectory(chat_id, "Task Http Save Enriched", "2024-01-01T00:00:01Z");
        payload["task_meta"] = json!({
            "task_id": task_id,
            "role": "agents",
            "agent_id": "agent-http-save-enriched",
            "card_id": "card-http-save-enriched"
        });

        handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap();

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "created");
        assert_eq!(event.task_id.as_deref(), Some(task_id));
        assert_eq!(event.session_state.as_deref(), Some("generating"));
        assert_eq!(event.error.as_deref(), Some("task save enriched error"));
        assert_eq!(
            event.worktree.as_ref().map(|worktree| worktree.id.as_str()),
            Some("task-save-enriched-wt")
        );
    }

    #[tokio::test]
    async fn task_path_hint_update_with_normal_same_id_loads_task_without_session_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "task-path-hint-update-normal-collision";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file_with_user_message(
            &normal_path,
            chat_id,
            "Normal Same Id",
            "normal must not be loaded",
        )
        .await;
        let task_meta = task_meta(
            "task-path-hint-update",
            "agents",
            Some("agent-path-hint"),
            Some("card-path-hint"),
            None,
        );
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-path-hint-update")
            .join("trajectories")
            .join("agents")
            .join("agent-path-hint")
            .join(format!("{chat_id}.json"));
        write_task_trajectory_file_with_user_message(
            &task_path,
            chat_id,
            "Task Path Hint Updated",
            "task path hint replacement",
            &task_meta,
        )
        .await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        let mut chat_rx = {
            let mut session = session_arc.lock().await;
            session.thread.title = "Active Normal Path Hint".to_string();
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                "keep active normal path hint".to_string(),
            ));
            session.runtime.state = SessionState::Generating;
            session.trajectory_dirty = true;
            session.subscribe()
        };
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        process_trajectory_change_for_source(
            gcx.clone(),
            chat_id,
            false,
            Some(trajectory_source_identity_from_path(
                &task_path,
                &get_all_task_roots(gcx).await,
            )),
        )
        .await;

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "updated");
        assert_eq!(event.title.as_deref(), Some("Task Path Hint Updated"));
        assert_eq!(event.task_id.as_deref(), Some("task-path-hint-update"));
        assert_eq!(event.task_role.as_deref(), Some("agents"));
        assert_eq!(event.agent_id.as_deref(), Some("agent-path-hint"));
        assert_eq!(event.card_id.as_deref(), Some("card-path-hint"));
        assert_eq!(event.session_state.as_deref(), Some("idle"));

        let session = session_arc.lock().await;
        assert_eq!(session.external_reload_pending, None);
        assert_eq!(session.thread.title, "Active Normal Path Hint");
        assert!(session.thread.task_meta.is_none());
        assert_eq!(session.messages.len(), 1);
        assert_eq!(
            session.messages[0].content.content_text_only(),
            "keep active normal path hint"
        );
        drop(session);
        assert_no_chat_event_for(&mut chat_rx, std::time::Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn planner_chat_id_verified_root_requires_planner_for_same_task() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _app) = make_app_with_workspace(dir.path()).await;
        let child_meta = task_meta(
            "task-root-check",
            "subchats",
            None,
            None,
            Some("stale-planner"),
        );
        let planner_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-root-check")
            .join("trajectories")
            .join("planner")
            .join("planner-root-check.json");
        write_task_trajectory_file_with_user_message(
            &planner_path,
            "planner-root-check",
            "Planner Root Check",
            "planner root",
            &task_meta(
                "task-root-check",
                "planner",
                None,
                None,
                Some("planner-root-check"),
            ),
        )
        .await;

        assert_eq!(
            verified_planner_linked_root_chat_id(
                gcx.clone(),
                "child-root-check",
                Some("normal-root"),
                &child_meta,
            )
            .await,
            None
        );
        assert_eq!(
            verified_planner_linked_root_chat_id(
                gcx,
                "child-root-check",
                Some("planner-root-check"),
                &child_meta,
            )
            .await
            .as_deref(),
            Some("planner-root-check")
        );
    }

    #[test]
    fn watcher_overflow_schedules_recovery_scans_and_resets_drop_counter() {
        let normal_root = PathBuf::from("normal-trajectories");
        let task_root = PathBuf::from("task-trajectories");
        let dropped_events = AtomicUsize::new(3);
        let mut pending_scans = std::collections::HashMap::new();

        schedule_trajectory_watcher_overflow_recovery(
            &mut pending_scans,
            &dropped_events,
            &[normal_root.clone(), task_root.clone()],
        );

        assert_eq!(dropped_events.load(Ordering::Relaxed), 0);
        assert_eq!(pending_scans.len(), 2);
        assert!(pending_scans.contains_key(&normal_root));
        assert!(pending_scans.contains_key(&task_root));
    }

    #[test]
    fn watcher_pending_debounce_keys_by_chat_and_source() {
        let chat_id = "debounce-source-collision".to_string();
        let task_source = TrajectorySourceIdentity::task(
            "task-debounce".to_string(),
            "agents".to_string(),
            Some("agent-debounce".to_string()),
            None,
            None,
        );
        let mut pending = TrajectoryPendingMap::new();
        let now = Instant::now();

        insert_pending_trajectory_change(
            &mut pending,
            chat_id.clone(),
            false,
            TrajectorySourceIdentity::Normal,
            PathBuf::from("normal.json"),
            now,
        );
        insert_pending_trajectory_change(
            &mut pending,
            chat_id.clone(),
            true,
            task_source.clone(),
            PathBuf::from("task.json"),
            now,
        );

        assert_eq!(pending.len(), 2);
        let normal = pending
            .get(&(chat_id.clone(), TrajectorySourceIdentity::Normal))
            .unwrap();
        assert_eq!(normal.last_event_at, now);
        assert!(!normal.is_remove);
        assert_eq!(normal.path, PathBuf::from("normal.json"));
        let task = pending.get(&(chat_id, task_source)).unwrap();
        assert_eq!(task.last_event_at, now);
        assert!(task.is_remove);
        assert_eq!(task.path, PathBuf::from("task.json"));
    }

    #[tokio::test]
    async fn pending_task_update_applies_task_despite_normal_priority() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "pending-task-update-normal-priority";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file_with_user_message(
            &normal_path,
            chat_id,
            "Normal Higher Priority",
            "normal must not apply",
        )
        .await;
        let task_meta = task_meta(
            "task-pending-update-apply",
            "agents",
            Some("agent-pending-apply"),
            Some("card-pending-apply"),
            None,
        );
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-pending-update-apply")
            .join("trajectories")
            .join("agents")
            .join("agent-pending-apply")
            .join(format!("{chat_id}.json"));
        write_task_trajectory_file_with_user_message(
            &task_path,
            chat_id,
            "Task Pending Applied",
            "task pending applied",
            &task_meta,
        )
        .await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.task_meta = Some(task_meta.clone());
            session.thread.title = "Busy Task Before Pending Apply".to_string();
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                "old busy task".to_string(),
            ));
            session.external_reload_pending = Some(ExternalReloadPending::update(
                TrajectorySourceIdentity::from_task_meta(&task_meta),
            ));
            session.runtime.state = SessionState::Idle;
            session.trajectory_dirty = false;
        }

        AppState::from_gcx(gcx.clone())
            .await
            .chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());
        check_external_reload_pending(gcx, session_arc.clone()).await;

        let session = session_arc.lock().await;
        assert_eq!(session.external_reload_pending, None);
        assert_eq!(session.thread.title, "Task Pending Applied");
        assert_eq!(session.thread.task_meta, Some(task_meta));
        assert_eq!(session.messages.len(), 1);
        assert_eq!(
            session.messages[0].content.content_text_only(),
            "task pending applied"
        );
    }

    #[tokio::test]
    async fn pending_task_update_missing_source_clears_while_normal_same_id_exists() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "pending-task-update-missing-normal-exists";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file_with_user_message(
            &normal_path,
            chat_id,
            "Normal Same Id Exists",
            "normal must not replace missing task",
        )
        .await;
        let task_meta = task_meta(
            "task-missing-pending-update",
            "agents",
            Some("agent-missing-pending"),
            Some("card-missing-pending"),
            None,
        );
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.task_meta = Some(task_meta.clone());
            session.thread.title = "Active Task Missing Pending Source".to_string();
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                "keep active task when matching source missing".to_string(),
            ));
            session.external_reload_pending = Some(ExternalReloadPending::update(
                TrajectorySourceIdentity::from_task_meta(&task_meta),
            ));
            session.runtime.state = SessionState::Idle;
            session.trajectory_dirty = false;
        }

        check_external_reload_pending(gcx, session_arc.clone()).await;

        let session = session_arc.lock().await;
        assert_eq!(session.external_reload_pending, None);
        assert_eq!(session.thread.title, "Active Task Missing Pending Source");
        assert_eq!(session.thread.task_meta, Some(task_meta));
        assert_eq!(session.messages.len(), 1);
        assert_eq!(
            session.messages[0].content.content_text_only(),
            "keep active task when matching source missing"
        );
    }

    #[tokio::test]
    async fn pending_update_source_mismatch_clears_without_loading_other_source() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "pending-update-source-mismatch-clears";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file_with_user_message(
            &normal_path,
            chat_id,
            "Mismatched Normal Pending",
            "normal mismatch must not apply",
        )
        .await;
        let task_meta = task_meta(
            "task-pending-mismatch",
            "agents",
            Some("agent-pending-mismatch"),
            Some("card-pending-mismatch"),
            None,
        );
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.task_meta = Some(task_meta.clone());
            session.thread.title = "Active Task Mismatched Pending".to_string();
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                "keep task on mismatched pending".to_string(),
            ));
            session.external_reload_pending = Some(ExternalReloadPending::update(
                TrajectorySourceIdentity::Normal,
            ));
            session.runtime.state = SessionState::Idle;
            session.trajectory_dirty = false;
        }

        check_external_reload_pending(gcx, session_arc.clone()).await;

        let session = session_arc.lock().await;
        assert_eq!(session.external_reload_pending, None);
        assert_eq!(session.thread.title, "Active Task Mismatched Pending");
        assert_eq!(session.thread.task_meta, Some(task_meta));
        assert_eq!(session.messages.len(), 1);
        assert_eq!(
            session.messages[0].content.content_text_only(),
            "keep task on mismatched pending"
        );
    }

    #[tokio::test]
    async fn pending_delete_source_mismatch_clears_without_loading_other_source() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "pending-delete-source-mismatch-clears";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file_with_user_message(
            &normal_path,
            chat_id,
            "Normal Same Id For Delete Mismatch",
            "normal delete mismatch must not apply",
        )
        .await;
        let task_meta = task_meta(
            "task-delete-mismatch",
            "agents",
            Some("agent-delete-mismatch"),
            Some("card-delete-mismatch"),
            None,
        );
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.title = "Active Normal Delete Mismatch".to_string();
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                "keep normal on mismatched delete".to_string(),
            ));
            session.external_reload_pending = Some(ExternalReloadPending::delete(
                TrajectorySourceIdentity::from_task_meta(&task_meta),
            ));
            session.runtime.state = SessionState::Idle;
            session.trajectory_dirty = false;
        }

        check_external_reload_pending(gcx, session_arc.clone()).await;

        let session = session_arc.lock().await;
        assert_eq!(session.external_reload_pending, None);
        assert_eq!(session.thread.title, "Active Normal Delete Mismatch");
        assert!(session.thread.task_meta.is_none());
        assert_eq!(session.messages.len(), 1);
        assert_eq!(
            session.messages[0].content.content_text_only(),
            "keep normal on mismatched delete"
        );
    }

    #[tokio::test]
    async fn task_delete_with_active_normal_same_id_does_not_clear_session() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "task-delete-active-normal-collision";
        let task_meta = task_meta(
            "task-delete-normal-collision",
            "agents",
            Some("agent-3"),
            Some("card-3"),
            None,
        );
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        let mut chat_rx = {
            let mut session = session_arc.lock().await;
            session.thread.title = "Active Normal Delete".to_string();
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                "keep normal during task delete".to_string(),
            ));
            session.runtime.state = SessionState::Idle;
            session.trajectory_dirty = false;
            session.subscribe()
        };
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        process_trajectory_change_for_source(
            gcx,
            chat_id,
            true,
            Some(TrajectorySourceIdentity::from_task_meta(&task_meta)),
        )
        .await;

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "deleted");
        let session = session_arc.lock().await;
        assert_eq!(session.external_reload_pending, None);
        assert_eq!(session.thread.title, "Active Normal Delete");
        assert_eq!(session.messages.len(), 1);
        assert_eq!(
            session.messages[0].content.content_text_only(),
            "keep normal during task delete"
        );
        drop(session);
        assert_no_chat_event_for(&mut chat_rx, std::time::Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn task_delete_with_normal_fallback_emits_updated_without_loading_normal_into_task() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "task-delete-normal-fallback-no-task-mutation";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file_with_user_message(
            &normal_path,
            chat_id,
            "Normal Fallback After Task Delete",
            "normal fallback should stay out of task session",
        )
        .await;
        let task_meta = task_meta(
            "task-delete-normal-fallback",
            "agents",
            Some("agent-delete-normal-fallback"),
            Some("card-delete-normal-fallback"),
            None,
        );
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        let mut chat_rx = {
            let mut session = session_arc.lock().await;
            session.thread.title = "Active Task Before Delete".to_string();
            session.thread.task_meta = Some(task_meta.clone());
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                "keep active task after task delete".to_string(),
            ));
            session.runtime.state = SessionState::Idle;
            session.trajectory_dirty = false;
            session.subscribe()
        };
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        process_trajectory_change_for_source(
            gcx,
            chat_id,
            true,
            Some(TrajectorySourceIdentity::from_task_meta(&task_meta)),
        )
        .await;

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "updated");
        assert_eq!(
            event.title.as_deref(),
            Some("Normal Fallback After Task Delete")
        );
        assert_eq!(event.task_id, None);
        assert_eq!(event.task_role, None);
        assert_eq!(event.session_state.as_deref(), Some("idle"));
        assert!(tokio::fs::try_exists(&normal_path).await.unwrap());

        let session = session_arc.lock().await;
        assert_eq!(
            session.external_reload_pending,
            Some(ExternalReloadPending::delete(
                TrajectorySourceIdentity::from_task_meta(&task_meta)
            ))
        );
        assert_eq!(session.messages.len(), 1);
        assert_eq!(
            session.messages[0].content.content_text_only(),
            "keep active task after task delete"
        );
        assert_eq!(session.thread.id, chat_id);
        assert_eq!(session.thread.task_meta, Some(task_meta.clone()));
        assert_eq!(session.thread.title, "Active Task Before Delete");
        drop(session);

        assert!(chat_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn busy_active_task_pending_delete_resolves_without_loading_normal_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "busy-task-delete-normal-fallback-pending";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file_with_user_message(
            &normal_path,
            chat_id,
            "Normal Fallback While Task Busy",
            "normal fallback while task busy",
        )
        .await;
        let task_meta = task_meta(
            "task-busy-delete-normal-fallback",
            "agents",
            Some("agent-busy-delete-normal-fallback"),
            Some("card-busy-delete-normal-fallback"),
            None,
        );
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        let mut chat_rx = {
            let mut session = session_arc.lock().await;
            session.thread.title = "Busy Task Before Delete".to_string();
            session.thread.task_meta = Some(task_meta.clone());
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                "keep busy task after delete".to_string(),
            ));
            session.runtime.state = SessionState::Generating;
            session.trajectory_dirty = true;
            session.subscribe()
        };
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        process_trajectory_change_for_source(
            gcx.clone(),
            chat_id,
            true,
            Some(TrajectorySourceIdentity::from_task_meta(&task_meta)),
        )
        .await;

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "updated");
        assert_eq!(
            event.title.as_deref(),
            Some("Normal Fallback While Task Busy")
        );
        {
            let session = session_arc.lock().await;
            assert_eq!(
                session.external_reload_pending,
                Some(ExternalReloadPending::delete(
                    TrajectorySourceIdentity::from_task_meta(&task_meta)
                ))
            );
            assert_eq!(session.thread.title, "Busy Task Before Delete");
            assert_eq!(session.thread.task_meta, Some(task_meta.clone()));
            assert_eq!(session.messages.len(), 1);
        }

        {
            let mut session = session_arc.lock().await;
            session.runtime.state = SessionState::Idle;
            session.trajectory_dirty = false;
        }
        check_external_reload_pending(gcx, session_arc.clone()).await;

        let session = session_arc.lock().await;
        assert_eq!(
            session.external_reload_pending,
            Some(ExternalReloadPending::delete(
                TrajectorySourceIdentity::from_task_meta(&task_meta)
            ))
        );
        assert_eq!(session.messages.len(), 1);
        assert_eq!(
            session.messages[0].content.content_text_only(),
            "keep busy task after delete"
        );
        assert_eq!(session.thread.id, chat_id);
        assert_eq!(session.thread.task_meta, Some(task_meta.clone()));
        assert_eq!(session.thread.title, "Busy Task Before Delete");
        drop(session);

        assert!(chat_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn normal_delete_with_active_task_same_id_does_not_clear_session() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "normal-delete-active-task-collision";
        let active_task_meta = task_meta(
            "task-active-normal-delete-collision",
            "agents",
            Some("agent-4"),
            Some("card-4"),
            None,
        );
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        let mut chat_rx = {
            let mut session = session_arc.lock().await;
            session.thread.title = "Active Task Delete".to_string();
            session.thread.task_meta = Some(active_task_meta.clone());
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                "keep task during normal delete".to_string(),
            ));
            session.runtime.state = SessionState::Idle;
            session.trajectory_dirty = false;
            session.subscribe()
        };
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        process_trajectory_change_for_source(
            gcx,
            chat_id,
            true,
            Some(TrajectorySourceIdentity::Normal),
        )
        .await;

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "deleted");
        let session = session_arc.lock().await;
        assert_eq!(session.external_reload_pending, None);
        assert_eq!(session.thread.title, "Active Task Delete");
        assert_eq!(session.thread.task_meta, Some(active_task_meta));
        assert_eq!(session.messages.len(), 1);
        assert_eq!(
            session.messages[0].content.content_text_only(),
            "keep task during normal delete"
        );
        drop(session);
        assert_no_chat_event_for(&mut chat_rx, std::time::Duration::from_millis(100)).await;
    }

    #[tokio::test]
    async fn same_task_identity_update_and_delete_still_mutate_or_pend_active_task() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "same-task-update-delete";
        let task_meta = task_meta(
            "task-same-source",
            "agents",
            Some("agent-5"),
            Some("card-5"),
            None,
        );
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-same-source")
            .join("trajectories")
            .join("agents")
            .join("agent-5")
            .join(format!("{chat_id}.json"));
        write_task_trajectory_file_with_user_message(
            &task_path,
            chat_id,
            "Same Task Updated",
            "same task replacement",
            &task_meta,
        )
        .await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.task_meta = Some(task_meta.clone());
            session.thread.title = "Old Same Task".to_string();
            session.runtime.state = SessionState::Idle;
            session.trajectory_dirty = false;
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        process_trajectory_change_for_source(
            gcx.clone(),
            chat_id,
            false,
            Some(TrajectorySourceIdentity::from_task_meta(&task_meta)),
        )
        .await;
        {
            let session = session_arc.lock().await;
            assert_eq!(session.external_reload_pending, None);
            assert_eq!(session.thread.title, "Same Task Updated");
            assert_eq!(session.thread.task_meta, Some(task_meta.clone()));
            assert_eq!(
                session.messages[0].content.content_text_only(),
                "same task replacement"
            );
        }
        tokio::fs::remove_file(&task_path).await.unwrap();
        {
            let mut session = session_arc.lock().await;
            session.runtime.state = SessionState::Generating;
            session.trajectory_dirty = true;
        }
        process_trajectory_change_for_source(
            gcx,
            chat_id,
            true,
            Some(TrajectorySourceIdentity::from_task_meta(&task_meta)),
        )
        .await;
        let session = session_arc.lock().await;
        assert_eq!(
            session.external_reload_pending,
            Some(ExternalReloadPending::delete(
                TrajectorySourceIdentity::from_task_meta(&task_meta)
            ))
        );
        assert_eq!(session.thread.title, "Same Task Updated");
    }

    #[tokio::test]
    async fn same_normal_identity_update_and_delete_still_mutate_or_pend_active_normal() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "same-normal-update-delete";
        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file_with_user_message(
            &normal_path,
            chat_id,
            "Same Normal Updated",
            "same normal replacement",
        )
        .await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.title = "Old Same Normal".to_string();
            session.runtime.state = SessionState::Idle;
            session.trajectory_dirty = false;
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        process_trajectory_change_for_source(
            gcx.clone(),
            chat_id,
            false,
            Some(TrajectorySourceIdentity::Normal),
        )
        .await;
        {
            let session = session_arc.lock().await;
            assert_eq!(session.external_reload_pending, None);
            assert_eq!(session.thread.title, "Same Normal Updated");
            assert!(session.thread.task_meta.is_none());
            assert_eq!(
                session.messages[0].content.content_text_only(),
                "same normal replacement"
            );
        }
        tokio::fs::remove_file(&normal_path).await.unwrap();
        {
            let mut session = session_arc.lock().await;
            session.runtime.state = SessionState::Generating;
            session.trajectory_dirty = true;
        }
        process_trajectory_change_for_source(
            gcx,
            chat_id,
            true,
            Some(TrajectorySourceIdentity::Normal),
        )
        .await;
        let session = session_arc.lock().await;
        assert_eq!(
            session.external_reload_pending,
            Some(ExternalReloadPending::delete(
                TrajectorySourceIdentity::Normal
            ))
        );
        assert_eq!(session.thread.title, "Same Normal Updated");
    }

    #[tokio::test]
    async fn generic_watcher_update_valid_file_emits_metadata_rich_updated() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let mut rx = app.chat.trajectory_events_tx.subscribe();
        let chat_id = "valid-update-rich-event";
        let path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-update-rich")
            .join("trajectories")
            .join("planner")
            .join(format!("{chat_id}.json"));
        write_trajectory_file_with_metadata(
            &path,
            chat_id,
            "Watcher Update Rich",
            "2024-01-01T00:00:01Z",
            "valid update message",
        )
        .await;

        process_trajectory_change(gcx, chat_id, false).await;

        let event = wait_for_trajectory_event(&mut rx, chat_id).await;
        assert_eq!(event.event_type, "updated");
        assert_eq!(event.updated_at.as_deref(), Some("2024-01-01T00:00:01Z"));
        assert_eq!(event.title.as_deref(), Some("Watcher Update Rich"));
        assert_eq!(event.is_title_generated, Some(true));
        assert_eq!(event.message_count, Some(1));
        assert_eq!(event.parent_id.as_deref(), Some("parent-fallback"));
        assert_eq!(event.link_type.as_deref(), Some("handoff"));
        assert_eq!(event.root_chat_id.as_deref(), Some("root-fallback"));
        assert_eq!(event.task_id.as_deref(), Some("task-update-rich"));
        assert_eq!(event.task_role.as_deref(), Some("planner"));
        assert_eq!(event.model.as_deref(), Some("fallback-model"));
        assert_eq!(event.mode.as_deref(), Some("task_planner"));
        assert_eq!(event.session_state.as_deref(), Some("idle"));
    }

    #[tokio::test]
    async fn external_delete_then_save_does_not_persist_stale_delete_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "external-delete-save-fresh-metadata";
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        let stale_created_at = "2024-01-01T00:00:00Z";
        {
            let mut session = session_arc.lock().await;
            session.messages.push(ChatMessage::new(
                "user".to_string(),
                "before external delete".to_string(),
            ));
            session.created_at = stale_created_at.to_string();
            session.wake_up_at = Some(chrono::Utc::now() + chrono::Duration::minutes(10));
            session.waiting_for_card_ids = vec!["card-stale".to_string()];
            session.thread.reactive_compact_attempts = Some(2);
            session.runtime.state = SessionState::Idle;
            session.trajectory_dirty = false;
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        process_trajectory_change(gcx.clone(), chat_id, true).await;
        {
            let mut session = session_arc.lock().await;
            session.add_message(ChatMessage::new(
                "user".to_string(),
                "after external delete".to_string(),
            ));
        }
        try_save_trajectory(app, session_arc.clone()).await.unwrap();

        let path = nested_trajectory_path(dir.path(), chat_id, chat_id);
        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();

        assert_ne!(saved["created_at"].as_str(), Some(stale_created_at));
        assert!(saved.get("wake_up_at").is_none());
        assert!(saved.get("waiting_for_card_ids").is_none());
        assert!(saved.get("reactive_compact_attempts").is_none());
        assert_eq!(saved["messages"].as_array().unwrap().len(), 1);
        assert_eq!(saved["messages"][0]["content"], "after external delete");
    }

    #[test]
    fn test_trajectory_snapshot_from_session_captures_fields() {
        use std::collections::VecDeque;
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;
        use tokio::sync::{broadcast, Mutex as AMutex, Notify};

        let (tx, _rx) = broadcast::channel(16);
        let session = ChatSession {
            execution_abort: Arc::new(AtomicBool::new(false)),
            review_readiness_pending: None,
            review_readiness_resume_nonce: None,
            verifier_readiness_pending: None,
            verifier_readiness_resume_nonce: None,
            delivery_save_id: uuid::Uuid::new_v4(),
            runner_lease: None,
            runner_message_digest: None,
            native_commands_in_flight: Default::default(),
            recovery_budget: None,
            recovery_notify: Arc::new(tokio::sync::Notify::new()),
            recovery_policy_source: refact_core::recovery::RecoveryPolicySource::Global,
            task_goal_messages: None,
            task_goal_partial_write: None,
            planner_pause: None,
            planner_pause_revision: 0,
            planner_pause_pending_tools: false,
            board_restored_pending_tools: false,
            pending_questions: Vec::new(),
            native_decision_receipts: Vec::new(),
            chat_id: "test-123".to_string(),
            work_provenance: Default::default(),
            interruptible_waits: 0,
            wait_delivery_boundary: false,
            wait_tool_window_active: false,
            wait_interrupt_flag: Arc::new(AtomicBool::new(false)),
            derived_privacy_zones: crate::privacy::records::new_derived_privacy_zones(),
            thread: ThreadParams {
                recovery_policy: None,
                id: "test-123".to_string(),
                title: "Test Thread".to_string(),
                model: "gpt-4".to_string(),
                mode: "AGENT".to_string(),
                tool_use: "agent".to_string(),
                boost_reasoning: Some(true),
                reasoning_effort: None,
                thinking_budget: None,
                temperature: None,
                frequency_penalty: None,
                max_tokens: None,
                parallel_tool_calls: None,
                context_tokens_cap: Some(8000),
                auto_compression_cap: Some(7000),
                auto_compression_cap_pending: false,
                include_project_info: false,
                checkpoints_enabled: true,
                is_title_generated: true,
                auto_approve_editing_tools: false,
                auto_approve_dangerous_commands: false,
                autonomous_no_confirm: false,
                task_meta: None,
                worktree: None,
                parent_id: Some("parent-chat-id".to_string()),
                link_type: Some("subagent".to_string()),
                root_chat_id: Some("root-chat-id".to_string()),
                previous_response_id: None,
                browser_meta: None,
                active_skill: None,
                auto_enrichment_enabled: None,
                buddy_meta: None,
                auto_compact_enabled: None,
                frozen_request_prefix: None,
                claude_code_identity: None,
                reactive_compact_attempts: None,
                app_workspace: None,
            },
            messages: vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
            runtime: super::super::types::RuntimeState::default(),
            goal: None,
            goal_active: false,
            goal_status: None,
            goal_turns_used: 0,
            goal_tokens_used: 0,
            goal_no_progress_turns: 0,
            is_compressing: false,
            compression_phase: None,
            compression_reason: None,
            compression_attempt_generation: 0,
            active_compression_attempt: None,
            compression_attempt_started_at_ms: None,
            compression_abort_flag: None,
            draft_message: None,
            draft_usage: None,
            command_queue: VecDeque::new(),
            queued_user_merge_targets: Default::default(),
            queued_user_restart_target: None,
            pending_deliveries: VecDeque::new(),
            runner_pending_deliveries: Vec::new(),
            delivered_delivery_ids: Default::default(),
            dropped_deliveries: VecDeque::new(),
            turn_depth: 0,
            delivery_wake_sources: Default::default(),
            event_seq: 0,
            event_tx: tx,
            recent_request_ids: VecDeque::new(),
            recent_request_ids_set: std::collections::HashSet::new(),
            abort_flag: Arc::new(AtomicBool::new(false)),
            abort_notify: Arc::new(Notify::new()),
            user_interrupt_flag: Arc::new(AtomicBool::new(false)),
            queue_processor_running: Arc::new(AtomicBool::new(false)),
            queue_notify: Arc::new(Notify::new()),
            queue_processor_counters: Arc::new(
                crate::chat::types::QueueProcessorCounters::default(),
            ),
            last_activity: Instant::now(),
            work_project_root: None,
            work_capsule_state: Default::default(),
            work_catalog_publisher: Default::default(),
            work_catalog_signature: Default::default(),
            last_stream_delta_at: None,
            live_diagnostics: Default::default(),
            command_enqueued_at: std::collections::HashMap::new(),
            stream_started_at: None,
            confirmation_paused_at: None,
            approval_wrapper_calls: Default::default(),
            automation_approval_receipts: Default::default(),
            app_workspace_mutation_lock: Default::default(),
            last_tool_started_at: None,
            last_tool_progress_at: None,
            trajectory_dirty: false,
            trajectory_version: 5,
            trajectory_committed_version: 0,
            trajectory_save_in_flight: false,
            trajectory_save_queued: false,
            trajectory_save_mutex: Arc::new(AMutex::new(())),
            trajectory_commit_notify: Arc::new(Notify::new()),
            trajectory_save_error: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            closed: false,
            closed_flag: Arc::new(AtomicBool::new(false)),
            external_reload_pending: None,
            last_prompt_messages: Vec::new(),
            tool_exposure_has_parent: Arc::new(tokio::sync::OnceCell::new()),
            tool_catalog: None,
            turn_tool_pool: None,
            pending_context_rebuild: None,
            pending_mode_handoff: None,
            last_rebuild_attempt_version: None,
            pending_max_new_tokens_boost: None,
            cache_guard_snapshot: None,
            cache_guard_request_generation: 0,
            cache_guard_snapshot_generation: 0,
            cache_guard_reset_generation: 0,
            cache_guard_force_next: false,
            provider_usage_stale: false,
            task_agent_error: None,
            trajectory_events_tx: None,
            pending_browser_message: None,
            post_tool_side_effects: VecDeque::new(),
            active_command: ActiveCommandContext::default(),
            skills_available_count: 0,
            skills_included: Vec::new(),
            post_turn_task_handles: Vec::new(),
            openai_codex_websocket: Default::default(),
            suppress_auto_enrichment_for_next_turn: false,
            enrichment_identities: std::collections::HashSet::new(),
            wake_up_at: None,
            waiting_for_card_ids: Vec::new(),
            background_agents: std::collections::HashMap::new(),
            goal_stopped_by_abort: false,
            goal_ledger: Vec::new(),
            sub_goal_graph_cache: Default::default(),
            goal_turn_evidence: false,
            goal_verification_generation: 0,
            goal_verification_blocked_until_ms: None,
        };

        let snapshot = trajectory_snapshot_from_session(&session);
        assert_eq!(snapshot.chat_id, "test-123");
        assert_eq!(snapshot.title, "Test Thread");
        assert_eq!(snapshot.model, "gpt-4");
        assert_eq!(snapshot.mode, "AGENT");
        assert!(snapshot.boost_reasoning);
        assert_eq!(snapshot.context_tokens_cap, Some(8000));
        assert_eq!(snapshot.auto_compression_cap, Some(7000));
        assert!(!snapshot.include_project_info);
        assert!(snapshot.is_title_generated);
        assert_eq!(snapshot.version, 5);
        assert_eq!(snapshot.messages.len(), 1);
    }

    #[test]
    fn test_trajectory_roundtrip_active_skill() {
        use super::super::types::ActiveCommandContext;
        use super::super::types::*;
        use std::collections::VecDeque;
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;
        use std::time::Instant;
        use tokio::sync::{broadcast, Mutex as AMutex, Notify};

        let (tx, _rx) = broadcast::channel(16);
        let mut session = ChatSession {
            execution_abort: Arc::new(AtomicBool::new(false)),
            review_readiness_pending: None,
            review_readiness_resume_nonce: None,
            verifier_readiness_pending: None,
            verifier_readiness_resume_nonce: None,
            delivery_save_id: uuid::Uuid::new_v4(),
            runner_lease: None,
            runner_message_digest: None,
            native_commands_in_flight: Default::default(),
            recovery_budget: None,
            recovery_notify: Arc::new(tokio::sync::Notify::new()),
            recovery_policy_source: refact_core::recovery::RecoveryPolicySource::Global,
            task_goal_messages: None,
            task_goal_partial_write: None,
            planner_pause: None,
            planner_pause_revision: 0,
            planner_pause_pending_tools: false,
            board_restored_pending_tools: false,
            pending_questions: Vec::new(),
            native_decision_receipts: Vec::new(),
            chat_id: "skill-test".to_string(),
            work_provenance: Default::default(),
            interruptible_waits: 0,
            wait_delivery_boundary: false,
            wait_tool_window_active: false,
            wait_interrupt_flag: Arc::new(AtomicBool::new(false)),
            derived_privacy_zones: crate::privacy::records::new_derived_privacy_zones(),
            thread: ThreadParams {
                recovery_policy: None,
                id: "skill-test".to_string(),
                active_skill: Some("my-skill".to_string()),
                ..Default::default()
            },
            messages: vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
            runtime: RuntimeState::default(),
            goal: None,
            goal_active: false,
            goal_status: None,
            goal_turns_used: 0,
            goal_tokens_used: 0,
            goal_no_progress_turns: 0,
            is_compressing: false,
            compression_phase: None,
            compression_reason: None,
            compression_attempt_generation: 0,
            active_compression_attempt: None,
            compression_attempt_started_at_ms: None,
            compression_abort_flag: None,
            draft_message: None,
            draft_usage: None,
            command_queue: VecDeque::new(),
            queued_user_merge_targets: Default::default(),
            queued_user_restart_target: None,
            pending_deliveries: VecDeque::new(),
            runner_pending_deliveries: Vec::new(),
            delivered_delivery_ids: Default::default(),
            dropped_deliveries: VecDeque::new(),
            turn_depth: 0,
            delivery_wake_sources: Default::default(),
            event_seq: 0,
            event_tx: tx,
            recent_request_ids: VecDeque::new(),
            recent_request_ids_set: std::collections::HashSet::new(),
            abort_flag: Arc::new(AtomicBool::new(false)),
            abort_notify: Arc::new(Notify::new()),
            user_interrupt_flag: Arc::new(AtomicBool::new(false)),
            queue_processor_running: Arc::new(AtomicBool::new(false)),
            queue_notify: Arc::new(Notify::new()),
            queue_processor_counters: Arc::new(
                crate::chat::types::QueueProcessorCounters::default(),
            ),
            last_activity: Instant::now(),
            work_project_root: None,
            work_capsule_state: Default::default(),
            work_catalog_publisher: Default::default(),
            work_catalog_signature: Default::default(),
            last_stream_delta_at: None,
            live_diagnostics: Default::default(),
            command_enqueued_at: std::collections::HashMap::new(),
            stream_started_at: None,
            confirmation_paused_at: None,
            approval_wrapper_calls: Default::default(),
            automation_approval_receipts: Default::default(),
            app_workspace_mutation_lock: Default::default(),
            last_tool_started_at: None,
            last_tool_progress_at: None,
            trajectory_dirty: false,
            trajectory_version: 1,
            trajectory_committed_version: 0,
            trajectory_save_in_flight: false,
            trajectory_save_queued: false,
            trajectory_save_mutex: Arc::new(AMutex::new(())),
            trajectory_commit_notify: Arc::new(Notify::new()),
            trajectory_save_error: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            closed: false,
            closed_flag: Arc::new(AtomicBool::new(false)),
            external_reload_pending: None,
            last_prompt_messages: Vec::new(),
            tool_exposure_has_parent: Arc::new(tokio::sync::OnceCell::new()),
            tool_catalog: None,
            turn_tool_pool: None,
            pending_context_rebuild: None,
            pending_mode_handoff: None,
            last_rebuild_attempt_version: None,
            pending_max_new_tokens_boost: None,
            cache_guard_snapshot: None,
            cache_guard_request_generation: 0,
            cache_guard_snapshot_generation: 0,
            cache_guard_reset_generation: 0,
            cache_guard_force_next: false,
            provider_usage_stale: false,
            task_agent_error: None,
            trajectory_events_tx: None,
            pending_browser_message: None,
            post_tool_side_effects: VecDeque::new(),
            active_command: ActiveCommandContext::default(),
            skills_available_count: 0,
            skills_included: Vec::new(),
            post_turn_task_handles: Vec::new(),
            openai_codex_websocket: Default::default(),
            suppress_auto_enrichment_for_next_turn: false,
            enrichment_identities: std::collections::HashSet::new(),
            wake_up_at: None,
            waiting_for_card_ids: Vec::new(),
            background_agents: std::collections::HashMap::new(),
            goal_stopped_by_abort: false,
            goal_ledger: Vec::new(),
            sub_goal_graph_cache: Default::default(),
            goal_turn_evidence: false,
            goal_verification_generation: 0,
            goal_verification_blocked_until_ms: None,
        };

        let snapshot = trajectory_snapshot_from_session(&session);
        assert_eq!(snapshot.active_skill, Some("my-skill".to_string()));

        session.thread.active_skill = None;
        let snapshot_none = trajectory_snapshot_from_session(&session);
        assert!(snapshot_none.active_skill.is_none());
    }

    #[tokio::test]
    async fn reopen_preserves_pinned_plan() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let mut plan_extra = serde_json::Map::new();
        plan_extra.insert(
            "plan".to_string(),
            json!({
                "mode": "agent",
                "version": 1,
                "created_at_ms": 123,
                "supersedes": null,
            }),
        );
        let plan = ChatMessage {
            role: "plan".to_string(),
            content: ChatContent::SimpleText("base plan".to_string()),
            preserve: Some(true),
            extra: plan_extra,
            ..Default::default()
        };
        let delta = crate::chat::internal_roles::plan_delta(
            "tool.set_plan",
            json!({"seq": 1}),
            "append update",
        );
        let mut session = ChatSession::new("plan-reopen".to_string());
        session.created_at = "2024-01-01T00:00:00Z".to_string();
        session.add_message(plan);
        session.add_message(delta);

        let snapshot = trajectory_snapshot_from_session(&session);
        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "plan-reopen").await.unwrap();
        let plans: Vec<_> = loaded
            .messages
            .iter()
            .filter(|message| message.role == "plan")
            .collect();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].content.content_text_only(), "base plan");
        assert_eq!(plans[0].preserve, Some(true));
        assert_eq!(plans[0].extra["plan"]["version"], json!(1));
        let deltas: Vec<_> = loaded
            .messages
            .iter()
            .filter(|message| {
                message.role == "event" && message.extra["event"]["subkind"] == json!("plan_delta")
            })
            .collect();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].content.content_text_only(), "append update");
    }

    #[tokio::test]
    async fn legacy_fallback_ids_are_distinct_for_repeated_hidden_events() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();
        tokio::fs::write(
            trajectories_dir.join("legacy-hidden-events.json"),
            serde_json::to_string(&json!({
                "id": "legacy-hidden-events",
                "title": "Legacy Hidden Events",
                "model": "model",
                "mode": "agent",
                "tool_use": "agent",
                "messages": [
                    {
                        "role": "event",
                        "content": "append update",
                        "extra": {
                            "event": {
                                "subkind": "plan_delta",
                                "source": "tool.update_plan",
                                "payload": {"seq": 1}
                            }
                        }
                    },
                    {
                        "role": "event",
                        "content": "append update",
                        "extra": {
                            "event": {
                                "subkind": "plan_delta",
                                "source": "tool.update_plan",
                                "payload": {"seq": 2}
                            }
                        }
                    },
                    {
                        "message_id": "kept-existing-id",
                        "role": "user",
                        "content": "visible"
                    }
                ],
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "legacy-hidden-events")
            .await
            .unwrap();

        assert_eq!(loaded.messages.len(), 3);
        assert!(loaded.messages[0].message_id.starts_with("legacy:event:"));
        assert!(loaded.messages[1].message_id.starts_with("legacy:event:"));
        assert_ne!(loaded.messages[0].message_id, loaded.messages[1].message_id);
        assert_eq!(loaded.messages[2].message_id, "kept-existing-id");
    }

    #[tokio::test]
    async fn wake_up_at_round_trips_through_trajectory_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        {
            *app.workspace
                .documents_state
                .workspace_folders
                .lock()
                .unwrap() = vec![dir.path().to_path_buf()];
        }

        let wake_up_at = chrono::Utc::now() + chrono::Duration::minutes(5);
        let mut session = ChatSession::new("wake-roundtrip".to_string());
        session.thread.title = "Wake Roundtrip".to_string();
        session.created_at = "2024-01-01T00:00:00Z".to_string();
        session.wake_up_at = Some(wake_up_at);
        session.add_message(ChatMessage::new("user".to_string(), "wait".to_string()));

        let snapshot = trajectory_snapshot_from_session(&session);
        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();

        drop(session);

        let loaded = load_trajectory_for_chat(gcx, "wake-roundtrip")
            .await
            .unwrap();
        assert_eq!(loaded.wake_up_at, Some(wake_up_at));
    }

    #[test]
    fn clamp_goal_snapshot_for_load_migrates_implicit_legacy_budget() {
        let goal = GoalSnapshot {
            owner: None,
            revision: 0,
            plan: None,
            content: "ship legacy".to_string(),
            version: 1,
            active: true,
            status: GoalStatus::BudgetExhausted,
            budget: GoalBudget::legacy_default_hard_limits(),
            progress: GoalProgress {
                turns_used: 10,
                ..Default::default()
            },
            attempts: Vec::new(),
            events: Vec::new(),
            criteria: Vec::new(),
            snoozed_until_ms: None,
            wait: None,
            stop_reason: None,
            transferred_from: None,
            transferred_to: None,
            sub_goals: Default::default(),
        };

        let loaded = clamp_goal_snapshot_for_load(goal);

        assert_eq!(loaded.budget, GoalBudget::default());
        assert_eq!(loaded.status, GoalStatus::Active);
    }

    #[test]
    fn clamp_goal_snapshot_for_load_heals_stale_verifying() {
        let goal = GoalSnapshot {
            owner: None,
            revision: 0,
            plan: None,
            content: "ship it".to_string(),
            version: 1,
            active: true,
            status: GoalStatus::Verifying,
            budget: GoalBudget::default(),
            progress: GoalProgress::default(),
            attempts: Vec::new(),
            events: Vec::new(),
            criteria: Vec::new(),
            snoozed_until_ms: None,
            wait: None,
            stop_reason: None,
            transferred_from: None,
            transferred_to: None,
            sub_goals: Default::default(),
        };
        assert_eq!(
            clamp_goal_snapshot_for_load(goal).status,
            GoalStatus::Active
        );

        let inactive = GoalSnapshot {
            owner: None,
            revision: 0,
            plan: None,
            content: "ship it".to_string(),
            version: 1,
            active: false,
            status: GoalStatus::Verifying,
            budget: GoalBudget::default(),
            progress: GoalProgress::default(),
            attempts: Vec::new(),
            events: Vec::new(),
            criteria: Vec::new(),
            snoozed_until_ms: None,
            wait: None,
            stop_reason: None,
            transferred_from: None,
            transferred_to: None,
            sub_goals: Default::default(),
        };
        assert_eq!(
            clamp_goal_snapshot_for_load(inactive).status,
            GoalStatus::Paused
        );
    }

    #[test]
    fn clamp_goal_snapshot_for_load_keeps_stopped_goals_stopped() {
        let goal = GoalSnapshot {
            owner: None,
            revision: 0,
            plan: None,
            content: "ship it".to_string(),
            version: 1,
            active: true,
            status: GoalStatus::Stopped,
            budget: GoalBudget::default(),
            progress: GoalProgress::default(),
            attempts: Vec::new(),
            events: Vec::new(),
            criteria: Vec::new(),
            snoozed_until_ms: None,
            wait: None,
            stop_reason: None,
            transferred_from: None,
            transferred_to: None,
            sub_goals: Default::default(),
        };
        assert_eq!(
            clamp_goal_snapshot_for_load(goal).status,
            GoalStatus::Stopped
        );
    }

    #[test]
    fn clamp_goal_snapshot_for_load_preserves_explicit_legacy_budget() {
        let budget = GoalBudget {
            explicit: true,
            ..GoalBudget::legacy_default_hard_limits()
        };
        let goal = GoalSnapshot {
            owner: None,
            revision: 0,
            plan: None,
            content: "ship explicit".to_string(),
            version: 1,
            active: true,
            status: GoalStatus::BudgetExhausted,
            budget,
            progress: GoalProgress {
                turns_used: 12,
                ..Default::default()
            },
            attempts: Vec::new(),
            events: Vec::new(),
            criteria: Vec::new(),
            snoozed_until_ms: None,
            wait: None,
            stop_reason: None,
            transferred_from: None,
            transferred_to: None,
            sub_goals: Default::default(),
        };

        let loaded = clamp_goal_snapshot_for_load(goal);

        assert_eq!(loaded.budget.max_turns, Some(10));
        assert_eq!(loaded.budget.max_minutes, Some(15));
        assert_eq!(loaded.budget.max_tokens, Some(200_000));
        assert_eq!(loaded.budget.no_progress_turns, Some(2));
        assert!(loaded.budget.explicit);
        assert_eq!(loaded.progress.turns_used, 10);
        assert_eq!(loaded.status, GoalStatus::BudgetExhausted);
    }

    #[tokio::test]
    async fn goal_round_trips_through_trajectory_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        {
            *app.workspace
                .documents_state
                .workspace_folders
                .lock()
                .unwrap() = vec![dir.path().to_path_buf()];
        }

        let mut session = ChatSession::new("goal-roundtrip".to_string());
        session.thread.title = "Goal Roundtrip".to_string();
        session.created_at = "2024-01-01T00:00:00Z".to_string();
        session.install_goal("agent", "finish the card", true, GoalBudget::default());
        session.add_message(crate::chat::internal_roles::goal_delta(
            "tool.update_goal",
            json!({"seq": 1, "at_ms": 999}),
            "add tests",
        ));
        session.goal.as_mut().unwrap().progress = GoalProgress {
            turns_used: 3,
            tokens_used: 456,
            started_at_ms: 111,
            no_progress_turns: 1,
            last_nudge_at_ms: 222,
            cost_used_cents: 0,
        };
        session.refresh_goal_runtime_mirror();

        let started_at_ms = session.goal.as_ref().unwrap().progress.started_at_ms;
        let snapshot = trajectory_snapshot_from_session(&session);
        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "goal-roundtrip")
            .await
            .unwrap();
        let loaded_goal = loaded.goal.clone().expect("loaded goal");

        assert_eq!(
            loaded_goal.content,
            "finish the card\n\n---\n\n## Goal updates\n\nadd tests"
        );
        assert_eq!(loaded_goal.status, GoalStatus::Active);
        assert_eq!(loaded_goal.progress.turns_used, 3);
        assert_eq!(loaded_goal.progress.tokens_used, 456);
        assert_eq!(loaded_goal.progress.no_progress_turns, 1);
        assert_eq!(loaded_goal.progress.started_at_ms, started_at_ms);
        assert_eq!(loaded_goal.transferred_from, None);
        assert_eq!(loaded_goal.transferred_to, None);
        assert!(loaded.messages.iter().any(|message| message.role == "goal"));
        assert!(loaded.messages.iter().any(|message| {
            message.role == "event" && message.extra["event"]["subkind"] == json!("goal_delta")
        }));

        let rehydrated = ChatSession::new_with_trajectory(
            "goal-roundtrip".to_string(),
            loaded.messages,
            loaded.thread,
            loaded.created_at,
            loaded.wake_up_at,
            loaded.waiting_for_card_ids,
            loaded.goal,
        );
        assert_eq!(rehydrated.goal_turns_used, 3);
        assert_eq!(rehydrated.goal_tokens_used, 456);
        assert_eq!(rehydrated.goal_no_progress_turns, 1);
        assert_eq!(rehydrated.event_seq, 0);
    }

    /// Persisted goal status and authorization-to-autocontinue are separate:
    /// the base goal message still says `active: true`, but a paused goal must
    /// come back paused and must not re-arm pursuit on load.
    #[tokio::test]
    async fn loading_a_paused_goal_does_not_rearm_autonomous_pursuit() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        {
            *app.workspace
                .documents_state
                .workspace_folders
                .lock()
                .unwrap() = vec![dir.path().to_path_buf()];
        }

        let mut session = ChatSession::new("goal-paused-restore".to_string());
        session.thread.title = "Paused goal".to_string();
        session.created_at = "2024-01-01T00:00:00Z".to_string();
        session.install_goal("agent", "ship the card", true, GoalBudget::default());
        session.goal_set_status_reason(GoalStatus::Paused, "goal_control");
        assert!(!session.goal_can_pursue());

        save_trajectory_snapshot(gcx.clone(), trajectory_snapshot_from_session(&session))
            .await
            .unwrap();
        let loaded = load_trajectory_for_chat(gcx, "goal-paused-restore")
            .await
            .unwrap();

        assert_eq!(loaded.goal.as_ref().unwrap().status, GoalStatus::Paused);
        assert!(loaded.messages.iter().any(|message| {
            message.role == "goal" && message.extra["goal"]["active"] == json!(true)
        }));

        let rehydrated = ChatSession::new_with_trajectory(
            "goal-paused-restore".to_string(),
            loaded.messages,
            loaded.thread,
            loaded.created_at,
            loaded.wake_up_at,
            loaded.waiting_for_card_ids,
            loaded.goal,
        );

        assert_eq!(rehydrated.goal_status, Some(GoalStatus::Paused));
        assert_eq!(rehydrated.runtime.goal_status, Some(GoalStatus::Paused));
        assert!(!rehydrated.goal_can_pursue());
        assert!(rehydrated.command_queue.is_empty());
    }

    #[tokio::test]
    async fn goal_ledger_round_trips_and_replay_wins_over_missing_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        {
            *app.workspace
                .documents_state
                .workspace_folders
                .lock()
                .unwrap() = vec![dir.path().to_path_buf()];
        }

        let mut session = ChatSession::new("goal-ledger-roundtrip".to_string());
        session.thread.title = "Ledger Roundtrip".to_string();
        session.created_at = "2024-01-01T00:00:00Z".to_string();
        session.install_goal("agent", "ship the card", true, GoalBudget::default());
        session.goal_record_progress_with_cost(50, false, 2);
        session.goal_set_status_reason(GoalStatus::Stopped, "goal_control");
        session.goal_verification_blocked_until_ms = Some(4_242);

        let mut snapshot = trajectory_snapshot_from_session(&session);
        assert!(!snapshot.goal_ledger.is_empty());
        snapshot.goal = None;
        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "goal-ledger-roundtrip")
            .await
            .unwrap();
        assert!(!loaded.goal_ledger.is_empty());
        let loaded_goal = loaded.goal.expect("goal restored from ledger replay");
        assert_eq!(loaded_goal.status, GoalStatus::Stopped);
        assert_eq!(loaded_goal.stop_reason.as_deref(), Some("goal_control"));
        assert_eq!(loaded_goal.progress.turns_used, 1);
        assert_eq!(loaded_goal.progress.tokens_used, 50);
        assert_eq!(loaded_goal.progress.cost_used_cents, 2);
        assert_eq!(loaded_goal.progress.no_progress_turns, 1);
        assert_eq!(loaded.goal_verification_blocked_until_ms, Some(4_242));
    }

    #[tokio::test]
    async fn trajectory_snapshot_roundtrip_preserves_waiting_for_card_ids() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        {
            *app.workspace
                .documents_state
                .workspace_folders
                .lock()
                .unwrap() = vec![dir.path().to_path_buf()];
        }

        let card_ids = vec!["T-1".to_string(), "T-10".to_string(), "T-2".to_string()];
        let mut session = ChatSession::new("wait-card-roundtrip".to_string());
        session.thread.title = "Wait Card Roundtrip".to_string();
        session.created_at = "2024-01-01T00:00:00Z".to_string();
        session.waiting_for_card_ids = card_ids.clone();
        session.add_message(ChatMessage::new("user".to_string(), "waiting".to_string()));

        let snapshot = trajectory_snapshot_from_session(&session);
        assert_eq!(snapshot.waiting_for_card_ids, card_ids);
        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "wait-card-roundtrip")
            .await
            .unwrap();
        assert_eq!(loaded.waiting_for_card_ids, card_ids);
    }

    #[tokio::test]
    async fn pending_questions_roundtrip_through_actual_trajectory_writer() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join("config"),
        )
        .await;
        let app = AppState::from_gcx(gcx.clone()).await;
        *app.workspace
            .documents_state
            .workspace_folders
            .lock()
            .unwrap() = vec![dir.path().to_path_buf()];
        let mut session = ChatSession::new("pending-questions-roundtrip".into());
        session.thread.title = "Pending questions".into();
        session
            .register_pending_questions(refact_chat_api::PendingQuestionSet {
                tool_call_id: "ask-native".into(),
                questions: vec![refact_chat_api::PendingQuestion {
                    id: "choice".into(),
                    question_type: "yes_no".into(),
                    text: "Continue?".into(),
                    options: None,
                }],
                asked_at_ms: 123,
                status: refact_chat_api::PendingQuestionStatus::Pending,
                answers: Vec::new(),
                resolution_id: None,
                response_protocol_version: refact_chat_api::QUESTION_RESPONSE_PROTOCOL_VERSION,
                response: None,
            })
            .unwrap();
        save_trajectory_snapshot(gcx.clone(), trajectory_snapshot_from_session(&session))
            .await
            .unwrap();
        let loaded = load_trajectory_for_chat(gcx.clone(), &session.chat_id)
            .await
            .unwrap();
        assert_eq!(loaded.pending_questions, session.pending_questions);
        session
            .answer_pending_questions(
                "ask-native",
                vec![refact_chat_api::QuestionAnswer {
                    id: "choice".into(),
                    answer: json!(true),
                }],
                "resolution-1".into(),
            )
            .unwrap();
        save_trajectory_snapshot(gcx.clone(), trajectory_snapshot_from_session(&session))
            .await
            .unwrap();
        let answered = load_trajectory_for_chat(gcx, &session.chat_id)
            .await
            .unwrap();
        assert_eq!(answered.pending_questions, session.pending_questions);
        assert_eq!(
            answered.pending_questions[0].resolution_id.as_deref(),
            Some("resolution-1")
        );
    }

    #[tokio::test]
    async fn browser_decision_descriptor_roundtrips_to_disk_and_expires_on_restore() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join("config"),
        )
        .await;
        let app = AppState::from_gcx(gcx.clone()).await;
        *app.workspace
            .documents_state
            .workspace_folders
            .lock()
            .unwrap() = vec![dir.path().to_path_buf()];
        let mut session = ChatSession::new("browser-decision-roundtrip".into());
        session.pending_browser_message = Some(crate::chat::types::PendingBrowserMessage {
            pending_message_id: "pending-browser".into(),
            content: json!({"secret": "must-not-persist"}),
            attachments: vec![],
            client_message_id: Some("optimistic-browser".into()),
            checkpoints: vec![],
            context_files: vec![],
            suppress_auto_enrichment: false,
            skill_activation_name: None,
            skill_context_msg: None,
        });

        save_trajectory_snapshot(gcx.clone(), trajectory_snapshot_from_session(&session))
            .await
            .unwrap();
        let loaded = load_trajectory_for_chat(gcx, &session.chat_id)
            .await
            .unwrap();
        let descriptor = loaded
            .pending_browser_decision
            .clone()
            .expect("descriptor saved");
        assert_eq!(descriptor.pending_message_id, "pending-browser");
        assert!(descriptor.requires_revalidation);
        assert_eq!(
            crate::chat::browser_decisions::restore_browser_decision(descriptor),
            crate::chat::browser_decisions::RestoredBrowserDecision::Expired {
                pending_message_id: "pending-browser".into(),
                client_message_id: Some("optimistic-browser".into()),
            }
        );
        let content = tokio::fs::read_to_string(loaded.source_path).await.unwrap();
        assert!(!content.contains("must-not-persist"));
    }

    #[tokio::test]
    async fn trajectory_save_removes_malformed_worktree_extra_from_json() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;
        {
            *app.workspace
                .documents_state
                .workspace_folders
                .lock()
                .unwrap() = vec![dir.path().to_path_buf()];
        }
        let chat_id = "malformed-worktree-save";
        let payload = json!({
            "id": chat_id,
            "title": "Malformed Worktree",
            "model": "m",
            "mode": "agent",
            "tool_use": "agent",
            "messages": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "include_project_info": true,
            "checkpoints_enabled": true,
            "worktree": {"root":"/tmp/untrusted"}
        });

        handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap();

        let path = nested_trajectory_path(dir.path(), chat_id, chat_id);
        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert!(saved.get("worktree").is_none());
    }

    #[tokio::test]
    async fn trajectory_save_preserves_valid_worktree_extra_in_json() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("repo");
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            cache.clone(),
            crate::test_tmp::scratch_dir("cfg"),
        )
        .await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        }
        let app = AppState::from_gcx(gcx.clone()).await;
        let service = WorktreeService::new(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                branch: Some("refact/chat/save-preserve".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let worktree = created.worktree.meta.clone();
        let chat_id = "valid-worktree-save";
        let payload = json!({
            "id": chat_id,
            "title": "Valid Worktree",
            "model": "m",
            "mode": "agent",
            "tool_use": "agent",
            "messages": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "include_project_info": true,
            "checkpoints_enabled": true,
            "worktree": serde_json::to_value(&worktree).unwrap()
        });

        handle_v1_trajectories_save(
            State(app),
            AxumPath(chat_id.to_string()),
            hyper::body::Bytes::from(serde_json::to_vec(&payload).unwrap()),
        )
        .await
        .unwrap();

        let path = nested_trajectory_path(&source, chat_id, chat_id);
        let saved: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(saved["worktree"]["id"], worktree.id);
        assert_eq!(saved["worktree"]["branch"], "refact/chat/save-preserve");
    }

    #[tokio::test]
    async fn trajectory_worktree_save_load_roundtrips_top_level_field() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("repo");
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            cache.clone(),
            crate::test_tmp::scratch_dir("cfg"),
        )
        .await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        }
        let app = AppState::from_gcx(gcx.clone()).await;
        let service = WorktreeService::new(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                branch: Some("refact/chat/roundtrip".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();

        let worktree = created.worktree.meta.clone();
        let chat_id = "wt-roundtrip".to_string();
        let snapshot = TrajectorySnapshot {
            recovery: None,
            recovery_policy: None,
            pending_questions: Vec::new(),
            native_decision_receipts: Vec::new(),
            goal: None,
            goal_ledger: Vec::new(),
            pending_deliveries: Vec::new(),
            dropped_deliveries: Vec::new(),
            goal_verification_blocked_until_ms: None,
            pending_browser_decision: None,
            chat_id: chat_id.clone(),
            title: "Worktree Chat".to_string(),
            model: "model".to_string(),
            mode: "agent".to_string(),
            tool_use: "agent".to_string(),
            messages: vec![ChatMessage::new("user".to_string(), "Hello".to_string())],
            created_at: "2024-01-01T00:00:00Z".to_string(),
            boost_reasoning: false,
            checkpoints_enabled: true,
            context_tokens_cap: None,
            auto_compression_cap: None,
            auto_compression_cap_pending: true,
            include_project_info: true,
            is_title_generated: true,
            auto_approve_editing_tools: false,
            auto_approve_dangerous_commands: false,
            autonomous_no_confirm: false,
            version: 1,
            task_meta: None,
            worktree: Some(worktree.clone()),
            parent_id: None,
            link_type: None,
            root_chat_id: None,
            reasoning_effort: None,
            thinking_budget: None,
            temperature: None,
            frequency_penalty: None,
            max_tokens: None,
            parallel_tool_calls: None,
            previous_response_id: None,
            active_skill: None,
            auto_enrichment_enabled: None,
            buddy_meta: None,
            auto_compact_enabled: None,
            frozen_request_prefix: None,
            claude_code_identity: None,
            reactive_compact_attempts: None,
            wake_up_at: None,
            waiting_for_card_ids: Vec::new(),
            app_workspace: None,
        };

        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();
        let path = nested_trajectory_path(&source, &chat_id, &chat_id);
        let raw = tokio::fs::read_to_string(path).await.unwrap();
        let raw_json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(raw_json["worktree"]["id"], worktree.id);
        assert_eq!(raw_json["worktree"]["branch"], "refact/chat/roundtrip");

        let loaded = load_trajectory_for_chat(gcx.clone(), &chat_id)
            .await
            .unwrap();
        assert_eq!(loaded.thread.worktree, Some(worktree.clone()));
        let listed = list_all_trajectories_meta(app, false).await.unwrap();
        let listed_worktree = listed
            .iter()
            .find(|item| item.id == chat_id)
            .and_then(|item| item.worktree.clone())
            .unwrap();
        assert_eq!(listed_worktree.id, worktree.id);
    }

    #[tokio::test]
    async fn frozen_request_prefix_and_claude_code_identity_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let tools_canonical = json!([
            {
                "type": "function",
                "function": {
                    "name": "cat",
                    "description": "Read a file",
                    "parameters": {"type": "object"}
                }
            }
        ]);
        let frozen_request_prefix = FrozenRequestPrefix {
            schema_version: 1,
            created_at: "2026-05-29T00:00:00Z".to_string(),
            system_prompt: Some("verbatim system".to_string()),
            tools_canonical: Some(tools_canonical.clone()),
        };
        let claude_code_identity = ClaudeCodeIdentity {
            device_id: "device-123".to_string(),
            session_id: "session-456".to_string(),
        };
        let mut snapshot = test_snapshot(
            "frozen-prefix-roundtrip",
            "Frozen Prefix",
            vec![ChatMessage::new("user".to_string(), "hello".to_string())],
        );
        snapshot.frozen_request_prefix = Some(frozen_request_prefix.clone());
        snapshot.claude_code_identity = Some(claude_code_identity.clone());

        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();

        let path = nested_trajectory_path(
            dir.path(),
            "frozen-prefix-roundtrip",
            "frozen-prefix-roundtrip",
        );
        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(
            raw["frozen_request_prefix"]["tools_canonical"],
            tools_canonical
        );
        assert_eq!(raw["claude_code_identity"]["device_id"], "device-123");

        let loaded = load_trajectory_for_chat(gcx, "frozen-prefix-roundtrip")
            .await
            .unwrap();
        assert_eq!(
            loaded.thread.frozen_request_prefix,
            Some(frozen_request_prefix)
        );
        assert_eq!(
            loaded.thread.claude_code_identity,
            Some(claude_code_identity)
        );
    }

    #[tokio::test]
    async fn mode_transition_load_discards_stale_copied_frozen_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("transition-stale-frozen.json"),
            serde_json::to_string(&json!({
                "id":"transition-stale-frozen",
                "title":"Transition",
                "created_at":"2024-01-01T00:00:00Z",
                "updated_at":"2024-01-01T00:00:00Z",
                "model":"model",
                "mode":"task_planner",
                "tool_use":"agent",
                "parent_id":"source-chat",
                "link_type":"mode_transition",
                "previous_response_id":"resp_source",
                "messages":[
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2026-05-29T00:00:00Z",
                    "system_prompt": "old planning system",
                    "tools_canonical": [{"type":"function","function":{"name":"old_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "include_project_info":true,
                "checkpoints_enabled":true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "transition-stale-frozen")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(
            prefix.system_prompt.as_deref(),
            Some("target task planner system")
        );
        assert!(prefix.tools_canonical.is_none());
        assert!(loaded.thread.claude_code_identity.is_none());
        assert!(loaded.thread.previous_response_id.is_none());
    }

    #[tokio::test]
    async fn mode_transition_no_frozen_prefix_with_copied_provider_state_is_repaired() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("transition-no-prefix-provider.json"),
            serde_json::to_string(&json!({
                "id":"transition-no-prefix-provider",
                "title":"Transition",
                "created_at":"2024-01-02T00:00:00Z",
                "updated_at":"2024-01-02T00:00:00Z",
                "model":"model",
                "mode":"task_planner",
                "tool_use":"agent",
                "parent_id":"source-chat",
                "link_type":"mode_transition",
                "previous_response_id":"resp_source",
                "messages":[
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "include_project_info":true,
                "checkpoints_enabled":true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "transition-no-prefix-provider")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(
            prefix.system_prompt.as_deref(),
            Some("target task planner system")
        );
        assert!(prefix.tools_canonical.is_none());
        assert!(loaded.thread.claude_code_identity.is_none());
        assert!(loaded.thread.previous_response_id.is_none());
    }

    #[tokio::test]
    async fn mode_transition_matching_system_with_stale_tools_is_repaired_by_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("transition-stale-tools-timestamp.json"),
            serde_json::to_string(&json!({
                "id":"transition-stale-tools-timestamp",
                "title":"Transition",
                "created_at":"2024-01-02T00:00:00Z",
                "updated_at":"2024-01-02T00:00:00Z",
                "model":"model",
                "mode":"task_planner",
                "tool_use":"agent",
                "parent_id":"source-chat",
                "link_type":"mode_transition",
                "messages":[
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2024-01-01T00:00:00Z",
                    "system_prompt": "target task planner system",
                    "tools_canonical": [{"type":"function","function":{"name":"old_source_tool"}}]
                },
                "include_project_info":true,
                "checkpoints_enabled":true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "transition-stale-tools-timestamp")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(
            prefix.system_prompt.as_deref(),
            Some("target task planner system")
        );
        assert!(prefix.tools_canonical.is_none());
        assert!(loaded.thread.claude_code_identity.is_none());
        assert!(loaded.thread.previous_response_id.is_none());
    }

    #[tokio::test]
    async fn mode_transition_invalid_prefix_timestamp_with_provider_state_is_repaired() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("transition-invalid-prefix-time-provider.json"),
            serde_json::to_string(&json!({
                "id":"transition-invalid-prefix-time-provider",
                "title":"Transition",
                "created_at":"2024-01-02T00:00:00Z",
                "updated_at":"2024-01-02T00:00:00Z",
                "model":"model",
                "mode":"task_planner",
                "tool_use":"agent",
                "parent_id":"source-chat",
                "link_type":"mode_transition",
                "previous_response_id":"resp_source",
                "messages":[
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "not-a-date",
                    "system_prompt": "target task planner system",
                    "tools_canonical": [{"type":"function","function":{"name":"old_source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "include_project_info":true,
                "checkpoints_enabled":true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "transition-invalid-prefix-time-provider")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(
            prefix.system_prompt.as_deref(),
            Some("target task planner system")
        );
        assert!(prefix.tools_canonical.is_none());
        assert!(loaded.thread.claude_code_identity.is_none());
        assert!(loaded.thread.previous_response_id.is_none());
        assert!(loaded.transition_identity_repaired);
    }

    #[tokio::test]
    async fn mode_transition_invalid_trajectory_created_at_with_tools_and_provider_state_is_repaired(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("transition-invalid-trajectory-time-provider.json"),
            serde_json::to_string(&json!({
                "id":"transition-invalid-trajectory-time-provider",
                "title":"Transition",
                "created_at":"not-a-date",
                "updated_at":"2024-01-02T00:00:00Z",
                "model":"model",
                "mode":"task_planner",
                "tool_use":"agent",
                "parent_id":"source-chat",
                "link_type":"mode_transition",
                "previous_response_id":"resp_source",
                "messages":[
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2024-01-03T00:00:00Z",
                    "system_prompt": "target task planner system",
                    "tools_canonical": [{"type":"function","function":{"name":"old_source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "include_project_info":true,
                "checkpoints_enabled":true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "transition-invalid-trajectory-time-provider")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(
            prefix.system_prompt.as_deref(),
            Some("target task planner system")
        );
        assert!(prefix.tools_canonical.is_none());
        assert!(loaded.thread.claude_code_identity.is_none());
        assert!(loaded.thread.previous_response_id.is_none());
        assert!(loaded.transition_identity_repaired);
    }

    #[tokio::test]
    async fn mode_transition_missing_trajectory_created_at_with_tools_and_provider_state_is_repaired(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("transition-missing-trajectory-time-provider.json"),
            serde_json::to_string(&json!({
                "id":"transition-missing-trajectory-time-provider",
                "title":"Transition",
                "updated_at":"2024-01-02T00:00:00Z",
                "model":"model",
                "mode":"task_planner",
                "tool_use":"agent",
                "parent_id":"source-chat",
                "link_type":"mode_transition",
                "previous_response_id":"resp_source",
                "messages":[
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2024-01-03T00:00:00Z",
                    "system_prompt": "target task planner system",
                    "tools_canonical": [{"type":"function","function":{"name":"old_source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "include_project_info":true,
                "checkpoints_enabled":true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "transition-missing-trajectory-time-provider")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(
            prefix.system_prompt.as_deref(),
            Some("target task planner system")
        );
        assert!(prefix.tools_canonical.is_none());
        assert!(loaded.thread.claude_code_identity.is_none());
        assert!(loaded.thread.previous_response_id.is_none());
        assert!(loaded.transition_identity_repaired);
    }

    #[tokio::test]
    async fn mode_transition_missing_frozen_system_with_tools_is_repaired() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("transition-missing-system-tools.json"),
            serde_json::to_string(&json!({
                "id":"transition-missing-system-tools",
                "title":"Transition",
                "created_at":"2024-01-01T00:00:00Z",
                "updated_at":"2024-01-01T00:00:00Z",
                "model":"model",
                "mode":"task_planner",
                "tool_use":"agent",
                "parent_id":"source-chat",
                "link_type":"mode_transition",
                "messages":[
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2024-01-02T00:00:00Z",
                    "system_prompt": null,
                    "tools_canonical": [{"type":"function","function":{"name":"old_source_tool"}}]
                },
                "include_project_info":true,
                "checkpoints_enabled":true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "transition-missing-system-tools")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(
            prefix.system_prompt.as_deref(),
            Some("target task planner system")
        );
        assert!(prefix.tools_canonical.is_none());
        assert!(loaded.transition_identity_repaired);
    }

    #[tokio::test]
    async fn handoff_load_discards_stale_copied_frozen_prefix_and_provider_state() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("handoff-stale-frozen-provider.json"),
            serde_json::to_string(&json!({
                "id":"handoff-stale-frozen-provider",
                "title":"Handoff",
                "created_at":"2024-01-02T00:00:00Z",
                "updated_at":"2024-01-02T00:00:00Z",
                "model":"model",
                "mode":"agent",
                "tool_use":"agent",
                "parent_id":"source-chat",
                "link_type":"handoff",
                "previous_response_id":"resp_source",
                "messages":[
                    {"role":"system","content":"target handoff system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2024-01-01T00:00:00Z",
                    "system_prompt": "source system",
                    "tools_canonical": [{"type":"function","function":{"name":"old_source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "include_project_info":true,
                "checkpoints_enabled":true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "handoff-stale-frozen-provider")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(
            prefix.system_prompt.as_deref(),
            Some("target handoff system")
        );
        assert!(prefix.tools_canonical.is_none());
        assert!(loaded.thread.claude_code_identity.is_none());
        assert!(loaded.thread.previous_response_id.is_none());
        assert!(loaded.transition_identity_repaired);
    }

    #[tokio::test]
    async fn mode_transition_preserves_legitimate_prefix_created_after_trajectory() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("transition-legitimate-prefix.json"),
            serde_json::to_string(&json!({
                "id":"transition-legitimate-prefix",
                "title":"Transition",
                "created_at":"2024-01-01T00:00:00Z",
                "updated_at":"2024-01-01T00:00:00Z",
                "model":"model",
                "mode":"task_planner",
                "tool_use":"agent",
                "parent_id":"source-chat",
                "link_type":"mode_transition",
                "messages":[
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2024-01-02T00:00:00Z",
                    "system_prompt": "target task planner system",
                    "tools_canonical": [{"type":"function","function":{"name":"target_tool"}}]
                },
                "include_project_info":true,
                "checkpoints_enabled":true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "transition-legitimate-prefix")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(
            prefix.tools_canonical,
            Some(json!([{"type":"function","function":{"name":"target_tool"}}]))
        );
        assert!(!loaded.transition_identity_repaired);
    }

    #[tokio::test]
    async fn ordinary_branch_load_preserves_copied_frozen_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let traj_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        tokio::fs::write(
            traj_dir.join("branch-preserve-frozen.json"),
            serde_json::to_string(&json!({
                "id":"branch-preserve-frozen",
                "title":"Branch",
                "created_at":"2024-01-01T00:00:00Z",
                "updated_at":"2024-01-01T00:00:00Z",
                "model":"model",
                "mode":"agent",
                "tool_use":"agent",
                "parent_id":"source-chat",
                "link_type":"branch",
                "previous_response_id":"resp_source",
                "messages":[
                    {"role":"system","content":"branch system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2026-05-29T00:00:00Z",
                    "system_prompt": "source system",
                    "tools_canonical": [{"type":"function","function":{"name":"source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "include_project_info":true,
                "checkpoints_enabled":true
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "branch-preserve-frozen")
            .await
            .unwrap();
        let prefix = loaded.thread.frozen_request_prefix.unwrap();
        assert_eq!(prefix.system_prompt.as_deref(), Some("source system"));
        assert_eq!(
            prefix.tools_canonical,
            Some(json!([{"type":"function","function":{"name":"source_tool"}}]))
        );
        assert!(loaded.thread.claude_code_identity.is_some());
        assert_eq!(
            loaded.thread.previous_response_id.as_deref(),
            Some("resp_source")
        );
    }

    #[tokio::test]
    async fn mode_transition_open_persists_repaired_frozen_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "transition-open-repair";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();
        let path = trajectories_dir.join(format!("{chat_id}.json"));
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Transition Open",
                "model": "model",
                "mode": "task_planner",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "mode_transition",
                "previous_response_id": "resp_source",
                "messages": [
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2026-05-29T00:00:00Z",
                    "system_prompt": "old planning system",
                    "tools_canonical": [{"type":"function","function":{"name":"old_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "browser_meta": {
                    "browser_runtime_id": "browser-open",
                    "tab_urls": ["https://example.com/open"],
                    "active_tab_id": "tab-open",
                    "attach_screenshot_on_send": true
                },
                "custom_future_field": {
                    "keep": true,
                    "nested": {"value": 43}
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let session_arc = crate::chat::get_or_create_session_with_trajectory(
            app.clone(),
            &app.chat.sessions,
            chat_id,
        )
        .await
        .unwrap();
        {
            let session = session_arc.lock().await;
            let prefix = session.thread.frozen_request_prefix.as_ref().unwrap();
            assert_eq!(
                prefix.system_prompt.as_deref(),
                Some("target task planner system")
            );
            assert!(prefix.tools_canonical.is_none());
            assert!(session.thread.claude_code_identity.is_none());
        }

        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(
            raw["frozen_request_prefix"]["system_prompt"],
            "target task planner system"
        );
        assert!(raw["frozen_request_prefix"]["tools_canonical"].is_null());
        assert!(raw.get("claude_code_identity").is_none());
        assert!(raw.get("previous_response_id").is_none());
        assert_eq!(raw["browser_meta"]["browser_runtime_id"], "browser-open");
        assert_eq!(raw["custom_future_field"]["nested"]["value"], 43);
    }

    #[tokio::test]
    async fn mode_transition_no_system_provider_cleanup_persists_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "transition-no-system-provider-open";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();
        let path = trajectories_dir.join(format!("{chat_id}.json"));
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Transition Open",
                "model": "model",
                "mode": "task_planner",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "mode_transition",
                "previous_response_id": "resp_source",
                "messages": [
                    {"role":"user","content":"hello without system"}
                ],
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "browser_meta": {
                    "browser_runtime_id": "browser-immediate",
                    "tab_urls": ["https://example.com/immediate"],
                    "active_tab_id": "tab-immediate",
                    "attach_screenshot_on_send": false
                },
                "custom_future_field": {
                    "keep": true,
                    "nested": {"value": 44}
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let session_arc = crate::chat::get_or_create_session_with_trajectory(
            app.clone(),
            &app.chat.sessions,
            chat_id,
        )
        .await
        .unwrap();
        {
            let session = session_arc.lock().await;
            assert!(session.thread.frozen_request_prefix.is_none());
            assert!(session.thread.claude_code_identity.is_none());
            assert!(session.thread.previous_response_id.is_none());
        }

        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert!(raw.get("frozen_request_prefix").is_none());
        assert!(raw.get("claude_code_identity").is_none());
        assert!(raw.get("previous_response_id").is_none());
        assert_eq!(
            raw["browser_meta"]["browser_runtime_id"],
            "browser-immediate"
        );
        assert_eq!(raw["custom_future_field"]["nested"]["value"], 44);
    }

    #[tokio::test]
    async fn mode_transition_empty_provider_cleanup_persists_to_original_global_path_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "transition-empty-provider-global-open";
        let global_trajectories_dir = get_global_trajectories_dir(gcx.clone()).await;
        tokio::fs::create_dir_all(&global_trajectories_dir)
            .await
            .unwrap();
        let path = global_trajectories_dir.join(format!("{chat_id}.json"));
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Transition Global Open",
                "model": "model",
                "mode": "task_planner",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "mode_transition",
                "previous_response_id": "resp_source",
                "messages": [],
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "browser_meta": {
                    "browser_runtime_id": "browser-global",
                    "tab_urls": ["https://example.com/global"],
                    "active_tab_id": "tab-global",
                    "attach_screenshot_on_send": true
                },
                "custom_future_field": {
                    "keep": true,
                    "nested": {"value": 46}
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let project_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));

        let session_arc = crate::chat::get_or_create_session_with_trajectory(
            app.clone(),
            &app.chat.sessions,
            chat_id,
        )
        .await
        .unwrap();
        {
            let session = session_arc.lock().await;
            assert!(session.messages.is_empty());
            assert!(session.thread.frozen_request_prefix.is_none());
            assert!(session.thread.claude_code_identity.is_none());
            assert!(session.thread.previous_response_id.is_none());
        }

        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        assert_eq!(raw["messages"].as_array().unwrap().len(), 0);
        assert!(raw.get("frozen_request_prefix").is_none());
        assert!(raw.get("claude_code_identity").is_none());
        assert!(raw.get("previous_response_id").is_none());
        assert_eq!(raw["browser_meta"]["browser_runtime_id"], "browser-global");
        assert_eq!(raw["custom_future_field"]["nested"]["value"], 46);
        assert!(!tokio::fs::try_exists(project_path).await.unwrap());
    }

    #[tokio::test]
    async fn no_meta_snapshot_does_not_overwrite_buddy_conversation_collision() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "buddy-normal-collision";
        let buddy_path = dir
            .path()
            .join(".refact")
            .join("buddy")
            .join("chats")
            .join("conversations")
            .join(format!("{chat_id}.json"));
        write_buddy_conversation_file(&buddy_path, chat_id, "Keep Buddy Collision").await;

        save_trajectory_snapshot(
            gcx.clone(),
            test_snapshot(
                chat_id,
                "Normal Chat",
                vec![ChatMessage::new(
                    "user".to_string(),
                    "hello normal".to_string(),
                )],
            ),
        )
        .await
        .unwrap();

        let normal_path = nested_trajectory_path(dir.path(), chat_id, chat_id);
        assert!(tokio::fs::try_exists(&normal_path).await.unwrap());
        let buddy_raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&buddy_path).await.unwrap()).unwrap();
        assert_eq!(buddy_raw["title"], "Keep Buddy Collision");
        assert!(buddy_raw.get("buddy_meta").is_some());
        assert_same_optional_path(
            find_trajectory_path(gcx.clone(), chat_id).await,
            &normal_path,
        );
        assert_ne!(
            find_trajectory_or_buddy_path(gcx, chat_id)
                .await
                .map(|path| normalized_test_path(&path)),
            Some(normalized_test_path(&buddy_path))
        );
    }

    #[tokio::test]
    async fn bound_task_snapshot_persists_to_original_project_after_workspace_reorder() {
        let (gcx, _temp, source) = crate::tools::tool_spawn_ab::tests::setup_repo_task().await;
        let (foreign_gcx, _foreign_temp, foreign_source) =
            crate::tools::tool_spawn_ab::tests::setup_repo_task().await;
        let chat_id = format!("bound-queued-trajectory-{}", Uuid::new_v4());
        let relative = Path::new(".refact/tasks/task-1/trajectories/agents/accepted-agent")
            .join(format!("{chat_id}.json"));
        let foreign_path = foreign_source.join(&relative);
        let meta = task_meta(
            "task-1",
            "agents",
            Some("accepted-agent"),
            Some("T-1"),
            Some("planner"),
        );
        write_task_trajectory_file_with_user_message(
            &foreign_path,
            &chat_id,
            "Foreign sentinel",
            "foreign message remains",
            &meta,
        )
        .await;
        let foreign_before = tokio::fs::read(&foreign_path).await.unwrap();
        assert!(!foreign_before.is_empty());
        drop(foreign_gcx);
        let binding =
            crate::tasks::storage::TaskDirectoryBinding::capture(gcx.clone(), &source, "task-1")
                .await
                .unwrap();
        let app = AppState::from_gcx(gcx.clone()).await;
        crate::tasks::admission::reserve(&app, &chat_id, "original", "planner", 0)
            .await
            .unwrap();
        let attempt = crate::tasks::admission::current_attempt(&chat_id).unwrap();
        binding
            .scope(crate::tasks::storage::update_board_atomic(
                gcx.clone(),
                "task-1",
                |board| {
                    let card = board.get_card_mut("T-1").unwrap();
                    card.column = "doing".into();
                    card.assignee = Some("accepted-agent".into());
                    card.agent_chat_id = Some(chat_id.clone());
                    Ok(())
                },
            ))
            .await
            .unwrap();
        let agent_binding = binding.with_attempt(&attempt, "T-1", "accepted-agent", None);
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![foreign_source, source.clone()];
        let mut snapshot = test_snapshot(
            &chat_id,
            "Accepted queued agent",
            vec![ChatMessage::new(
                "user".into(),
                "accepted original message".into(),
            )],
        );
        snapshot.task_meta = Some(meta);
        agent_binding
            .scope(save_trajectory_snapshot(gcx.clone(), snapshot.clone()))
            .await
            .unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&tokio::fs::read(source.join(&relative)).await.unwrap())
                .unwrap();
        assert_eq!(saved["title"], "Accepted queued agent");
        assert_eq!(saved["task_meta"]["agent_id"], "accepted-agent");
        assert_eq!(saved["messages"][0]["content"], "accepted original message");
        assert_eq!(
            tokio::fs::read(&foreign_path).await.unwrap(),
            foreign_before
        );
        let accepted_before = tokio::fs::read(source.join(&relative)).await.unwrap();
        binding
            .scope(crate::tasks::storage::update_board_atomic(
                gcx.clone(),
                "task-1",
                |board| {
                    let card = board.get_card_mut("T-1").unwrap();
                    card.assignee = Some("replacement-agent".into());
                    card.agent_chat_id = Some("replacement-chat".into());
                    Ok(())
                },
            ))
            .await
            .unwrap();
        snapshot.title = "Late replaced agent must not publish".into();
        let failure = agent_binding
            .scope(save_trajectory_snapshot(gcx.clone(), snapshot))
            .await
            .unwrap_err();
        assert_eq!(failure, "Accepted task publication claim was replaced");
        assert_eq!(
            tokio::fs::read(source.join(&relative)).await.unwrap(),
            accepted_before
        );
        assert_eq!(
            tokio::fs::read(&foreign_path).await.unwrap(),
            foreign_before
        );
        let mut session = ChatSession::new(chat_id.clone());
        session.thread.task_meta = Some(task_meta(
            "task-1",
            "agents",
            Some("accepted-agent"),
            Some("T-1"),
            Some("planner"),
        ));
        session.add_message(ChatMessage::new(
            "user".into(),
            "must not reach foreign project".into(),
        ));
        gcx.chat_sessions
            .write()
            .await
            .insert(chat_id.clone(), Arc::new(AMutex::new(session)));
        let original_task = source.join(".refact/tasks/task-1");
        tokio::fs::rename(&original_task, source.join("retained-task"))
            .await
            .unwrap();
        let failure = agent_binding
            .scope(app.chat.facade.maybe_save_session(&chat_id))
            .await
            .unwrap_err();
        assert!(
            failure.contains("Accepted") || failure.contains("accepted"),
            "{failure}"
        );
        assert_eq!(
            tokio::fs::read(&foreign_path).await.unwrap(),
            foreign_before
        );
        attempt.end();
        gcx.shutdown_flag.store(true, Ordering::Relaxed);
    }

    #[tokio::test]
    async fn no_meta_snapshot_does_not_overwrite_task_trajectory_collision() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "task-normal-collision";
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-collision")
            .join("trajectories")
            .join("agents")
            .join("agent-1")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(
            &task_path,
            chat_id,
            "Keep Task Collision",
            "2024-01-01T00:00:00Z",
        )
        .await;
        let task_before = tokio::fs::read_to_string(&task_path).await.unwrap();

        save_trajectory_snapshot(
            gcx.clone(),
            test_snapshot(
                chat_id,
                "Normal Chat",
                vec![ChatMessage::new(
                    "user".to_string(),
                    "hello normal".to_string(),
                )],
            ),
        )
        .await
        .unwrap();

        let normal_path = nested_trajectory_path(dir.path(), chat_id, chat_id);
        assert!(tokio::fs::try_exists(&normal_path).await.unwrap());
        assert_eq!(
            tokio::fs::read_to_string(&task_path).await.unwrap(),
            task_before
        );
        let normal_raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&normal_path).await.unwrap()).unwrap();
        assert_eq!(normal_raw["title"], "Normal Chat");
        let task_raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&task_path).await.unwrap()).unwrap();
        assert_eq!(task_raw["title"], "Keep Task Collision");
        assert_same_optional_path(
            find_trajectory_path(gcx.clone(), chat_id).await,
            &normal_path,
        );
        assert_ne!(
            find_trajectory_path(gcx, chat_id)
                .await
                .map(|path| normalized_test_path(&path)),
            Some(normalized_test_path(&task_path))
        );
    }

    #[tokio::test]
    async fn mode_transition_external_reload_persists_provider_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "transition-external-reload-provider";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();
        let path = trajectories_dir.join(format!("{chat_id}.json"));
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Transition External Reload",
                "model": "model",
                "mode": "task_planner",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "mode_transition",
                "previous_response_id": "resp_source",
                "messages": [],
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "browser_meta": {
                    "browser_runtime_id": "browser-immediate",
                    "tab_urls": ["https://example.com/immediate"],
                    "active_tab_id": "tab-immediate",
                    "attach_screenshot_on_send": false
                },
                "custom_future_field": {
                    "keep": true,
                    "nested": {"value": 44}
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        process_trajectory_change(gcx, chat_id, false).await;

        {
            let session = session_arc.lock().await;
            assert!(session.messages.is_empty());
            assert!(session.thread.frozen_request_prefix.is_none());
            assert!(session.thread.claude_code_identity.is_none());
            assert!(session.thread.previous_response_id.is_none());
            assert!(!session.trajectory_dirty);
        }
        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(raw["messages"].as_array().unwrap().len(), 0);
        assert!(raw.get("frozen_request_prefix").is_none());
        assert!(raw.get("claude_code_identity").is_none());
        assert!(raw.get("previous_response_id").is_none());
        assert_eq!(
            raw["browser_meta"]["browser_runtime_id"],
            "browser-immediate"
        );
        assert_eq!(raw["custom_future_field"]["nested"]["value"], 44);
    }

    #[tokio::test]
    async fn mode_transition_pending_external_reload_persists_provider_cleanup_when_idle() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "transition-pending-external-reload-provider";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();
        let path = trajectories_dir.join(format!("{chat_id}.json"));
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Pending Transition External Reload",
                "model": "model",
                "mode": "task_planner",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "mode_transition",
                "previous_response_id": "resp_source",
                "messages": [
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2026-05-29T00:00:00Z",
                    "system_prompt": "source system",
                    "tools_canonical": [{"type":"function","function":{"name":"source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "browser_meta": {
                    "browser_runtime_id": "browser-pending",
                    "tab_urls": ["https://example.com/pending"],
                    "active_tab_id": "tab-pending",
                    "attach_screenshot_on_send": true
                },
                "custom_future_field": {
                    "keep": true,
                    "nested": {"value": 45}
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.runtime.state = SessionState::Generating;
            session.trajectory_dirty = true;
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        process_trajectory_change(gcx.clone(), chat_id, false).await;
        {
            let session = session_arc.lock().await;
            assert_eq!(
                session.external_reload_pending,
                Some(ExternalReloadPending::update(
                    TrajectorySourceIdentity::Normal
                ))
            );
            assert!(session.thread.claude_code_identity.is_none());
            assert!(session.thread.frozen_request_prefix.is_none());
        }

        {
            let mut session = session_arc.lock().await;
            session.runtime.state = SessionState::Idle;
            session.trajectory_dirty = false;
        }
        check_external_reload_pending(gcx, session_arc.clone()).await;

        {
            let session = session_arc.lock().await;
            assert_eq!(session.external_reload_pending, None);
            let prefix = session.thread.frozen_request_prefix.as_ref().unwrap();
            assert_eq!(
                prefix.system_prompt.as_deref(),
                Some("target task planner system")
            );
            assert!(prefix.tools_canonical.is_none());
            assert!(session.thread.claude_code_identity.is_none());
            assert!(session.thread.previous_response_id.is_none());
            assert!(!session.trajectory_dirty);
        }
        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(
            raw["frozen_request_prefix"]["system_prompt"],
            "target task planner system"
        );
        assert!(raw["frozen_request_prefix"]["tools_canonical"].is_null());
        assert!(raw.get("claude_code_identity").is_none());
        assert!(raw.get("previous_response_id").is_none());
        assert_eq!(raw["browser_meta"]["browser_runtime_id"], "browser-pending");
        assert_eq!(raw["custom_future_field"]["nested"]["value"], 45);
    }

    #[tokio::test]
    async fn mode_transition_no_active_session_external_change_persists_provider_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "transition-no-active-session-repair";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();
        let path = trajectories_dir.join(format!("{chat_id}.json"));
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "No Active Session Repair",
                "model": "model",
                "mode": "task_planner",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "mode_transition",
                "previous_response_id": "resp_source",
                "messages": [
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2026-05-29T00:00:00Z",
                    "system_prompt": "source system",
                    "tools_canonical": [{"type":"function","function":{"name":"source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "browser_meta": {
                    "browser_runtime_id": "browser-1",
                    "tab_urls": ["https://example.com"],
                    "active_tab_id": "tab-1",
                    "attach_screenshot_on_send": true
                },
                "custom_future_field": {
                    "keep": true,
                    "nested": {"value": 42}
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        process_trajectory_change(gcx, chat_id, false).await;

        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(path).await.unwrap()).unwrap();
        assert_eq!(
            raw["frozen_request_prefix"]["system_prompt"],
            "target task planner system"
        );
        assert!(raw["frozen_request_prefix"]["tools_canonical"].is_null());
        assert!(raw.get("claude_code_identity").is_none());
        assert!(raw.get("previous_response_id").is_none());
        assert_eq!(raw["browser_meta"]["browser_runtime_id"], "browser-1");
        assert_eq!(
            raw["browser_meta"]["tab_urls"],
            json!(["https://example.com"])
        );
        assert_eq!(raw["custom_future_field"]["nested"]["value"], 42);
    }

    #[tokio::test]
    async fn repair_source_id_mismatch_errors_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "repair-source-original";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Repair Source Original",
                "model": "model",
                "mode": "task_planner",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "mode_transition",
                "previous_response_id": "resp_source",
                "messages": [
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2026-05-29T00:00:00Z",
                    "system_prompt": "source system",
                    "tools_canonical": [{"type":"function","function":{"name":"source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "custom_future_field": {"keep": true}
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let mut loaded = load_trajectory_for_chat(gcx.clone(), chat_id)
            .await
            .unwrap();
        assert!(loaded.transition_identity_repaired);
        apply_mode_defaults_to_thread(
            gcx.clone(),
            &mut loaded.thread,
            loaded.auto_approve_editing_tools_present,
            loaded.auto_approve_dangerous_commands_present,
        )
        .await;
        let repair_patch = loaded.repair_patch();
        let replacement = serde_json::to_string(&json!({
            "id": "repair-source-replacement",
            "title": "Replacement Must Stay",
            "model": "model",
            "mode": "agent",
            "tool_use": "agent",
            "messages": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "include_project_info": true,
            "checkpoints_enabled": true,
            "previous_response_id": "replacement-response"
        }))
        .unwrap();
        tokio::fs::write(&path, &replacement).await.unwrap();

        let err = persist_loaded_trajectory_repair_raw(gcx.clone(), &repair_patch)
            .await
            .unwrap_err();

        assert!(err.contains("Trajectory source id mismatch for repair"));
        assert_eq!(tokio::fs::read_to_string(path).await.unwrap(), replacement);
    }

    #[tokio::test]
    async fn repair_source_outside_approved_roots_errors_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "repair-source-outside-root";
        let outside_path = outside.path().join(format!("{chat_id}.json"));
        write_trajectory_file(
            &outside_path,
            chat_id,
            "Outside Repair Source",
            "2024-01-01T00:00:00Z",
        )
        .await;
        let before = tokio::fs::read_to_string(&outside_path).await.unwrap();
        let repair_patch = TrajectoryRepairPatch {
            chat_id: chat_id.to_string(),
            source_path: outside_path.clone(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            frozen_request_prefix: None,
            auto_approve_editing_tools: true,
            auto_approve_dangerous_commands: true,
        };

        let err = persist_loaded_trajectory_repair_raw(gcx, &repair_patch)
            .await
            .unwrap_err();

        assert!(err.contains("not in an approved trajectory root"));
        assert_eq!(
            tokio::fs::read_to_string(outside_path).await.unwrap(),
            before
        );
    }

    #[tokio::test]
    async fn mode_transition_repair_persists_to_loaded_task_path_with_normal_collision() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "transition-task-normal-collision-repair";
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join("task-loaded-repair")
            .join("trajectories")
            .join("agents")
            .join("agent-1")
            .join(format!("{chat_id}.json"));
        tokio::fs::create_dir_all(task_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &task_path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Task Collision Repair",
                "model": "model",
                "mode": "task_planner",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "mode_transition",
                "previous_response_id": "task-response",
                "messages": [
                    {"role":"system","content":"target task planner system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2026-05-29T00:00:00Z",
                    "system_prompt": "source system",
                    "tools_canonical": [{"type":"function","function":{"name":"source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "task_meta": {
                    "task_id": "task-loaded-repair",
                    "role": "agents",
                    "agent_id": "agent-1",
                    "card_id": "T-1"
                },
                "custom_future_field": {"task": true}
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let mut loaded = load_trajectory_for_chat(gcx.clone(), chat_id)
            .await
            .unwrap();
        assert_same_path(&loaded.source_path, &task_path);
        assert!(loaded.transition_identity_repaired);
        apply_mode_defaults_to_thread(
            gcx.clone(),
            &mut loaded.thread,
            loaded.auto_approve_editing_tools_present,
            loaded.auto_approve_dangerous_commands_present,
        )
        .await;
        let repair_patch = loaded.repair_patch();

        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        tokio::fs::create_dir_all(normal_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &normal_path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Normal Collision Must Stay",
                "model": "model",
                "mode": "agent",
                "tool_use": "agent",
                "messages": [],
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "previous_response_id": "normal-response",
                "custom_future_field": {"normal": true}
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let normal_before = tokio::fs::read_to_string(&normal_path).await.unwrap();

        persist_loaded_trajectory_repair_raw(gcx.clone(), &repair_patch)
            .await
            .unwrap();

        assert_eq!(
            tokio::fs::read_to_string(&normal_path).await.unwrap(),
            normal_before
        );
        let task_raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&task_path).await.unwrap()).unwrap();
        assert_eq!(
            task_raw["frozen_request_prefix"]["system_prompt"],
            "target task planner system"
        );
        assert!(task_raw["frozen_request_prefix"]["tools_canonical"].is_null());
        assert!(task_raw.get("claude_code_identity").is_none());
        assert!(task_raw.get("previous_response_id").is_none());
        assert_eq!(task_raw["custom_future_field"]["task"], true);
    }

    #[tokio::test]
    async fn handoff_invalid_or_missing_created_at_repairs_and_normalizes_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();

        for (chat_id, created_at) in [
            ("handoff-invalid-created-at-open", Some(json!("not-a-date"))),
            ("handoff-missing-created-at-open", None),
        ] {
            let path = trajectories_dir.join(format!("{chat_id}.json"));
            let mut payload = json!({
                "id": chat_id,
                "title": "Handoff Open",
                "model": "model",
                "mode": "agent",
                "tool_use": "agent",
                "parent_id": "source-chat",
                "link_type": "handoff",
                "previous_response_id": "resp_source",
                "messages": [
                    {"role":"system","content":"target handoff system"},
                    {"role":"user","content":"hello"}
                ],
                "frozen_request_prefix": {
                    "schema_version": 1,
                    "created_at": "2024-01-03T00:00:00Z",
                    "system_prompt": "target handoff system",
                    "tools_canonical": [{"type":"function","function":{"name":"source_tool"}}]
                },
                "claude_code_identity": {
                    "device_id":"source-device",
                    "session_id":"source-session"
                },
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true
            });
            if let Some(created_at) = created_at {
                payload["created_at"] = created_at;
            }
            tokio::fs::write(&path, serde_json::to_string(&payload).unwrap())
                .await
                .unwrap();

            let session_arc = crate::chat::get_or_create_session_with_trajectory(
                app.clone(),
                &app.chat.sessions,
                chat_id,
            )
            .await
            .unwrap();
            {
                let session = session_arc.lock().await;
                let prefix = session.thread.frozen_request_prefix.as_ref().unwrap();
                assert_eq!(
                    prefix.system_prompt.as_deref(),
                    Some("target handoff system")
                );
                assert!(prefix.tools_canonical.is_none());
                assert!(session.thread.claude_code_identity.is_none());
                assert!(session.thread.previous_response_id.is_none());
                assert!(parsed_rfc3339_utc(&session.created_at).is_some());
            }

            let raw: serde_json::Value =
                serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
            let saved_created_at = raw["created_at"].as_str().unwrap();
            assert!(parsed_rfc3339_utc(saved_created_at).is_some());
            assert_ne!(saved_created_at, "not-a-date");
            assert_eq!(
                raw["frozen_request_prefix"]["system_prompt"],
                "target handoff system"
            );
            assert!(raw["frozen_request_prefix"]["tools_canonical"].is_null());
            assert!(raw.get("claude_code_identity").is_none());
            assert!(raw.get("previous_response_id").is_none());
        }
    }

    #[test]
    fn frozen_lazy_ensure_noops_on_empty_inputs_and_preserves_existing_prefix() {
        let mut session = ChatSession::new("lazy-frozen".to_string());
        assert!(ensure_frozen_prefix(&mut session, None, None).is_none());
        assert!(session.thread.frozen_request_prefix.is_none());
        assert_eq!(session.trajectory_version, 0);

        let no_tools = ensure_frozen_prefix(
            &mut session,
            Some("system no tools".to_string()),
            Some(json!([])),
        )
        .expect("known no-tools prefix should be installed");
        assert_eq!(no_tools.tools_canonical, Some(json!([])));
        assert!(frozen_prefix_is_complete(&no_tools));
        let version_after_no_tools = session.trajectory_version;

        let with_tools = ensure_frozen_prefix(
            &mut session,
            Some("system with tools".to_string()),
            Some(json!([{"type":"function","function":{"name":"cat"}}])),
        );
        assert!(with_tools.is_none());
        assert_eq!(session.trajectory_version, version_after_no_tools);
        assert_eq!(
            session
                .thread
                .frozen_request_prefix
                .as_ref()
                .and_then(|prefix| prefix.tools_canonical.clone()),
            Some(json!([]))
        );

        let mut session = ChatSession::new("lazy-frozen".to_string());

        let first = ensure_frozen_prefix(
            &mut session,
            Some("system one".to_string()),
            Some(json!([{"type":"function","function":{"name":"cat"}}])),
        )
        .expect("missing prefix should be installed");

        assert_eq!(first.schema_version, 1);
        assert_eq!(first.system_prompt.as_deref(), Some("system one"));
        assert!(session.thread.frozen_request_prefix.is_some());
        let version_after_first = session.trajectory_version;

        let second = ensure_frozen_prefix(
            &mut session,
            Some("system two".to_string()),
            Some(json!([{"type":"function","function":{"name":"shell"}}])),
        );
        assert!(second.is_none());
        assert_eq!(session.trajectory_version, version_after_first);
        let prefix = session.thread.frozen_request_prefix.as_ref().unwrap();
        assert_eq!(prefix.system_prompt.as_deref(), Some("system one"));
        assert_eq!(
            prefix.tools_canonical,
            Some(json!([{"type":"function","function":{"name":"cat"}}]))
        );
    }

    #[test]
    fn frozen_lazy_ensure_fills_partial_prefix_once() {
        let mut session = ChatSession::new("partial-frozen".to_string());
        let partial = ensure_frozen_prefix(&mut session, Some("system only".to_string()), None)
            .expect("partial prefix should be installed");
        assert_eq!(partial.system_prompt.as_deref(), Some("system only"));
        assert!(partial.tools_canonical.is_none());
        let version_after_partial = session.trajectory_version;

        let filled = ensure_frozen_prefix(
            &mut session,
            Some("replacement system".to_string()),
            Some(json!([{"type":"function","function":{"name":"cat"}}])),
        )
        .expect("partial prefix should be filled");
        assert_eq!(filled.system_prompt.as_deref(), Some("system only"));
        assert_eq!(
            filled.tools_canonical,
            Some(json!([{"type":"function","function":{"name":"cat"}}]))
        );
        assert_eq!(session.trajectory_version, version_after_partial + 1);

        let version_after_fill = session.trajectory_version;
        assert!(ensure_frozen_prefix(
            &mut session,
            Some("new system".to_string()),
            Some(json!([{"type":"function","function":{"name":"shell"}}])),
        )
        .is_none());
        assert_eq!(session.trajectory_version, version_after_fill);
        let prefix = session.thread.frozen_request_prefix.as_ref().unwrap();
        assert_eq!(prefix.system_prompt.as_deref(), Some("system only"));
        assert_eq!(
            prefix.tools_canonical,
            Some(json!([{"type":"function","function":{"name":"cat"}}]))
        );
    }

    #[tokio::test]
    async fn frozen_legacy_normal_session_migrates_through_session_save() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "legacy-freeze-normal";
        let trajectories_dir = dir.path().join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&trajectories_dir).await.unwrap();
        tokio::fs::write(
            trajectories_dir.join(format!("{chat_id}.json")),
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Legacy",
                "model": "test/model",
                "mode": "agent",
                "tool_use": "agent",
                "messages": [
                    {"role":"system","content":"legacy system"},
                    {"role":"user","content":"hello"}
                ],
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        let session_arc = crate::chat::get_or_create_session_with_trajectory(
            app.clone(),
            &app.chat.sessions,
            chat_id,
        )
        .await
        .unwrap();
        {
            let session = session_arc.lock().await;
            let prefix = session.thread.frozen_request_prefix.as_ref().unwrap();
            assert_eq!(prefix.system_prompt.as_deref(), Some("legacy system"));
            assert!(prefix.tools_canonical.is_none());
        }

        let raw: serde_json::Value = serde_json::from_str(
            &tokio::fs::read_to_string(trajectories_dir.join(format!("{chat_id}.json")))
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            raw["frozen_request_prefix"]["system_prompt"],
            "legacy system"
        );
        assert!(raw["frozen_request_prefix"]["tools_canonical"].is_null());
        assert_eq!(raw["messages"][0]["content"], "legacy system");
    }

    #[tokio::test]
    async fn frozen_legacy_task_session_migration_does_not_create_generic_copy() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let task_id = "task-legacy-freeze";
        let agent_id = "agent-1";
        let chat_id = "legacy-task-freeze";
        let planner_id = "legacy-freeze-planner";
        let task_path = dir
            .path()
            .join(".refact")
            .join("tasks")
            .join(task_id)
            .join("trajectories")
            .join("agents")
            .join(agent_id)
            .join(format!("{chat_id}.json"));
        tokio::fs::create_dir_all(task_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &task_path,
            serde_json::to_string(&json!({
                "id": chat_id,
                "title": "Legacy Task",
                "model": "model",
                "mode": "task_agent",
                "tool_use": "agent",
                "messages": [
                    {"role":"system","content":"task system"},
                    {"role":"user","content":"hello task"}
                ],
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "include_project_info": true,
                "checkpoints_enabled": true,
                "task_meta": {
                    "task_id": task_id,
                    "role": "agents",
                    "agent_id": agent_id,
                    "card_id": "T-1",
                    "planner_chat_id": planner_id
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::create_dir_all(dir.path().join(".refact").join("tasks").join(task_id))
            .await
            .unwrap();
        let meta = serde_json::from_value(json!({
            "id": task_id, "name": "Legacy freeze", "status": "active",
            "created_at": "2024-01-01T00:00:00Z", "updated_at": "2024-01-01T00:00:00Z"
        }))
        .unwrap();
        crate::tasks::storage::save_task_meta(gcx.clone(), task_id, &meta)
            .await
            .unwrap();
        let board = serde_json::from_value(json!({
            "owner_chat_id": planner_id,
            "cards": [{"id": "T-1", "title": "Legacy freeze", "column": "doing",
                "assignee": agent_id, "agent_chat_id": chat_id,
                "created_at": "2024-01-01T00:00:00Z"}]
        }))
        .unwrap();
        crate::tasks::storage::save_board(gcx, task_id, &board)
            .await
            .unwrap();
        crate::tasks::admission::reserve(
            &app,
            chat_id,
            &crate::coordination::roots::project_identity(dir.path()),
            planner_id,
            0,
        )
        .await
        .unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            crate::tasks::admission::wait_for_permit(chat_id),
        )
        .await
        .unwrap()
        .unwrap();
        let session_arc = crate::chat::get_or_create_session_with_trajectory(
            app.clone(),
            &app.chat.sessions,
            chat_id,
        )
        .await
        .unwrap();
        assert!(
            crate::chat::session_runtime::board_execution(&app, &session_arc)
                .await
                .unwrap()
                .is_some()
        );
        {
            let session = session_arc.lock().await;
            let prefix = session.thread.frozen_request_prefix.as_ref().unwrap();
            assert_eq!(prefix.system_prompt.as_deref(), Some("task system"));
            assert!(prefix.tools_canonical.is_none());
        }

        let generic_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        let raw: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&task_path).await.unwrap()).unwrap();
        assert!(!tokio::fs::try_exists(generic_path).await.unwrap());
        assert_eq!(raw["frozen_request_prefix"]["system_prompt"], "task system");
        assert!(raw["frozen_request_prefix"]["tools_canonical"].is_null());
        crate::tasks::admission::end(chat_id).await;
    }

    #[tokio::test]
    async fn trajectory_persistence_preserves_ui_only_messages() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let mut session = ChatSession::new("ui-only-roundtrip".to_string());
        session.created_at = "2024-01-01T00:00:00Z".to_string();
        session.add_message(ChatMessage::new("user".to_string(), "hello".to_string()));
        session.add_message(make_ui_only_error_message("context_length_exceeded"));

        let snapshot = trajectory_snapshot_from_session(&session);
        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "ui-only-roundtrip")
            .await
            .unwrap();
        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[0].role, "user");
        assert_eq!(loaded.messages[1].role, "error");
        assert!(is_ui_only_message(&loaded.messages[1]));
    }

    #[tokio::test]
    async fn cache_guard_snapshot_is_runtime_only_and_not_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let raw_request_marker = "raw-cache-guard-request-body-marker";
        let mut session = ChatSession::new("cache-guard-runtime-only".to_string());
        session.created_at = "2024-01-01T00:00:00Z".to_string();
        session.cache_guard_snapshot = Some(json!({
            "messages": [{"role": "user", "content": raw_request_marker}],
            "provider_specific_fields": {"semantic": "value"}
        }));
        session.add_message(ChatMessage::new("user".to_string(), "visible".to_string()));

        let snapshot = trajectory_snapshot_from_session(&session);
        let snapshot_messages_json = serde_json::to_string(&snapshot.messages).unwrap();
        assert!(!snapshot_messages_json.contains(raw_request_marker));

        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();

        let path = nested_trajectory_path(
            dir.path(),
            "cache-guard-runtime-only",
            "cache-guard-runtime-only",
        );
        let saved_json = tokio::fs::read_to_string(path).await.unwrap();
        assert!(!saved_json.contains("cache_guard_snapshot"));
        assert!(!saved_json.contains(raw_request_marker));
    }

    #[tokio::test]
    async fn reactive_compact_attempts_is_runtime_only_not_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let mut snapshot = test_snapshot(
            "reactive-attempts-roundtrip",
            "Reactive Attempts",
            vec![ChatMessage::new("user".to_string(), "visible".to_string())],
        );
        snapshot.reactive_compact_attempts = Some(2);
        save_trajectory_snapshot(gcx.clone(), snapshot)
            .await
            .unwrap();

        let traj_path = nested_trajectory_path(
            dir.path(),
            "reactive-attempts-roundtrip",
            "reactive-attempts-roundtrip",
        );
        let saved = tokio::fs::read_to_string(&traj_path).await.unwrap();
        assert!(!saved.contains("reactive_compact_attempts"));

        let mut raw: serde_json::Value = serde_json::from_str(&saved).unwrap();
        raw["reactive_compact_attempts"] = json!(99);
        tokio::fs::write(&traj_path, serde_json::to_string(&raw).unwrap())
            .await
            .unwrap();

        let loaded = load_trajectory_for_chat(gcx, "reactive-attempts-roundtrip")
            .await
            .unwrap();
        assert_eq!(loaded.thread.reactive_compact_attempts, None);
    }

    #[tokio::test]
    async fn pending_deliveries_save_load_preserves_identity_and_clears_delivered() {
        use refact_core::chat_types::{PendingDelivery, PushMode};
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let mut session = ChatSession::new("pending-roundtrip".into());
        session.created_at = "2024-01-01T00:00:00Z".into();
        let mut delivery = PendingDelivery::with_id(
            "stable-id",
            vec![ChatMessage::new("user".into(), "queued".into())],
            PushMode::WhenIdle,
            "test",
            true,
        );
        delivery.enqueued_at_ms = 42;
        session.pending_deliveries.push_back(delivery.clone());
        assert!(!session_is_stale_refresh_candidate(&session, true));
        save_trajectory_snapshot(gcx.clone(), trajectory_snapshot_from_session(&session))
            .await
            .unwrap();
        let loaded = load_trajectory_for_chat(gcx.clone(), "pending-roundtrip")
            .await
            .unwrap();
        assert_eq!(loaded.pending_deliveries, vec![delivery.clone()]);
        let mut restored = ChatSession::new("pending-roundtrip".into());
        restored.restore_pending_deliveries(loaded.pending_deliveries);
        let restored_delivery = restored.pending_deliveries_for_snapshot();
        assert_eq!(restored_delivery.len(), 1);
        assert_eq!(restored_delivery[0].id, delivery.id);
        assert_eq!(restored_delivery[0].enqueued_at_ms, delivery.enqueued_at_ms);
        assert!(restored_delivery[0].expires_at_ms.is_some());
        restored.pending_deliveries.clear();
        restored.messages = delivery.stamp_messages();
        restored.restore_pending_deliveries(vec![delivery]);
        assert!(restored.pending_deliveries.is_empty());
        save_trajectory_snapshot(gcx.clone(), trajectory_snapshot_from_session(&restored))
            .await
            .unwrap();
        assert!(load_trajectory_for_chat(gcx, "pending-roundtrip")
            .await
            .unwrap()
            .pending_deliveries
            .is_empty());
    }

    #[tokio::test]
    async fn legacy_pending_delivery_without_expiry_survives_restore_unexpired() {
        use refact_core::chat_types::{DeliveryStatus, PushMode};
        let dir = tempfile::tempdir().unwrap();
        let (gcx, _) = make_app_with_workspace(dir.path()).await;
        let chat_id = "legacy-pending-no-expiry";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(chat_id)
            .join(format!("{chat_id}.json"));
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &path,
            serde_json::to_string_pretty(&json!({
                "id": chat_id,
                "title": "Legacy",
                "messages": [],
                "pending_deliveries": [{
                    "id": "legacy-pending",
                    "messages": [
                        serde_json::to_value(ChatMessage::new(
                            "user".to_string(),
                            "queued".to_string(),
                        ))
                        .unwrap()
                    ],
                    "push": "when_idle",
                    "source": "planner",
                    "wake": true,
                    "supersede_key": "agent_steer:T-1"
                }]
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        assert_eq!(loaded.pending_deliveries.len(), 1);
        assert_eq!(loaded.pending_deliveries[0].enqueued_at_ms, 0);
        assert!(loaded.pending_deliveries[0].expires_at_ms.is_none());

        let mut restored = ChatSession::new(chat_id.to_string());
        restored.restore_pending_deliveries(loaded.pending_deliveries);

        assert_eq!(
            restored.delivery_status("legacy-pending"),
            DeliveryStatus::Pending,
            "a record written before ttl existed must not be read as expired at epoch zero"
        );
        let pending = restored.pending_deliveries_for_snapshot();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].push, PushMode::WhenIdle);
        assert_eq!(pending[0].supersede_key.as_deref(), Some("agent_steer:T-1"));
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        assert!(pending[0].expires_at_ms.is_some());
        assert!(!pending[0].is_expired_at(now));
    }

    #[test]
    fn trajectory_save_uses_unique_tmp() {
        let file_path = PathBuf::from("chat.json");
        let first = unique_trajectory_tmp_path(&file_path);
        let second = unique_trajectory_tmp_path(&file_path);

        assert_ne!(first, second);
        assert_eq!(
            first
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap()
                .len(),
            22
        );
        assert!(first
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap()
            .starts_with("chat.json.tmp."));
        assert!(second
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap()
            .starts_with("chat.json.tmp."));
    }

    #[tokio::test]
    async fn trajectory_save_cleans_up_on_error() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("chat.json");
        let tmp_path = unique_trajectory_tmp_path(&file_path);
        tokio::fs::write(&tmp_path, "stale").await.unwrap();

        let err = atomic_write_json_with_tmp_path(
            &file_path,
            &tmp_path,
            Err("Failed to serialize trajectory: injected".to_string()),
            Some("Failed to write trajectory"),
            None,
        )
        .await
        .unwrap_err();

        assert_eq!(err, "Failed to serialize trajectory: injected");
        assert!(!tmp_path.exists());
        let leftovers = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .map(|name| name.contains(".tmp"))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(leftovers, 0);
    }

    async fn engine_write_trajectory_json(path: &Path, value: &serde_json::Value) {
        let tmp_path = unique_trajectory_tmp_path(path);
        let write_result = write_pretty_json_to_tmp_file(&tmp_path, value);
        atomic_finish_trajectory_write(
            path,
            &tmp_path,
            write_result,
            Some(TrajectorySourceIdentity::Normal),
        )
        .await
        .unwrap();
    }

    async fn insert_idle_session_for_trajectory(
        app: &AppState,
        chat_id: &str,
        title: &str,
    ) -> Arc<AMutex<ChatSession>> {
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.title = title.to_string();
            session
                .messages
                .push(ChatMessage::new("user".to_string(), "before".to_string()));
            session.trajectory_dirty = false;
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());
        session_arc
    }

    #[serial(trajectory_perf, runtime_settings)]
    #[tokio::test]
    async fn self_write_fingerprint_computed_without_reading_file_back() {
        crate::runtime_settings::reset_for_test();
        clear_all_trajectory_self_writes();
        let dir = tempfile::tempdir().unwrap();
        let path = canonical_trajectory_write_path(&dir.path().join("no-read-back.json"))
            .await
            .unwrap();
        let on_disk = "{\"id\":\"on-disk-bytes\"}";
        let in_memory = "{\"id\":\"in-memory-buf\"}";
        assert_eq!(on_disk.len(), in_memory.len());
        let reads = count_test_trajectory_self_write_reads(&path);

        let handle = register_trajectory_self_write(
            &path,
            TrajectorySourceIdentity::Normal,
            in_memory.len() as u64,
            md5::compute(in_memory).0,
        )
        .await
        .unwrap();
        fs::write(&path, on_disk).await.unwrap();
        complete_trajectory_self_write(Some(handle)).await;

        assert_eq!(reads.load(Ordering::Relaxed), 0);
        let fingerprint = {
            let registry = trajectory_self_writes()
                .lock()
                .expect("trajectory self-write registry poisoned");
            registry
                .get(&(path.clone(), TrajectorySourceIdentity::Normal))
                .and_then(|records| records.front())
                .and_then(|record| record.fingerprint.clone())
                .unwrap()
        };
        assert_eq!(fingerprint.content_hash, md5::compute(in_memory).0);
        assert_ne!(fingerprint.content_hash, md5::compute(on_disk).0);

        assert!(!consume_trajectory_self_write(&path, &TrajectorySourceIdentity::Normal).await);
        assert_eq!(reads.load(Ordering::Relaxed), 1);
        clear_all_trajectory_self_writes();
    }

    #[serial(trajectory_perf, runtime_settings)]
    #[tokio::test]
    async fn self_write_is_suppressed() {
        crate::runtime_settings::reset_for_test();
        clear_all_trajectory_self_writes();
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "self-write-suppressed";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file_with_user_message(&path, chat_id, "Before Self Write", "before")
            .await;
        let session_arc =
            insert_idle_session_for_trajectory(&app, chat_id, "Before Self Write").await;
        let mut trajectory_events = app.chat.trajectory_events_tx.subscribe();

        let mut value: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        value["title"] = json!("Written By The Engine");
        engine_write_trajectory_json(&path, &value).await;

        process_trajectory_watcher_change(
            gcx,
            chat_id.to_string(),
            false,
            TrajectorySourceIdentity::Normal,
            path,
        )
        .await;

        let session = session_arc.lock().await;
        assert_eq!(session.thread.title, "Before Self Write");
        assert_eq!(session.external_reload_pending, None);
        drop(session);
        assert!(trajectory_events.try_recv().is_err());
        clear_all_trajectory_self_writes();
    }

    #[serial(trajectory_perf, runtime_settings)]
    #[tokio::test]
    async fn external_edit_is_not_suppressed() {
        crate::runtime_settings::reset_for_test();
        clear_all_trajectory_self_writes();
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "external-edit-detected";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file_with_user_message(&path, chat_id, "Before Self Write", "before")
            .await;
        let session_arc =
            insert_idle_session_for_trajectory(&app, chat_id, "Before Self Write").await;
        let mut trajectory_events = app.chat.trajectory_events_tx.subscribe();

        let mut value: serde_json::Value =
            serde_json::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        value["title"] = json!("Written By The Engine");
        engine_write_trajectory_json(&path, &value).await;
        value["title"] = json!("Edited Outside The Engine");
        tokio::fs::write(&path, serde_json::to_string(&value).unwrap())
            .await
            .unwrap();

        process_trajectory_watcher_change(
            gcx,
            chat_id.to_string(),
            false,
            TrajectorySourceIdentity::Normal,
            path,
        )
        .await;

        let session = session_arc.lock().await;
        assert_eq!(session.thread.title, "Edited Outside The Engine");
        drop(session);
        assert_eq!(trajectory_events.try_recv().unwrap().id, chat_id);
        clear_all_trajectory_self_writes();
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn trajectory_watcher_self_write_consumes_exact_fingerprint_once() {
        clear_all_trajectory_self_writes();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("self-write.json");
        let content = "{\"id\":\"self-write\"}";
        let handle = register_trajectory_self_write(
            &path,
            TrajectorySourceIdentity::Normal,
            content.len() as u64,
            md5::compute(content).0,
        )
        .await
        .unwrap();
        fs::write(&path, content).await.unwrap();
        complete_trajectory_self_write(Some(handle)).await;

        assert!(consume_trajectory_self_write(&path, &TrajectorySourceIdentity::Normal).await);
        assert!(!consume_trajectory_self_write(&path, &TrajectorySourceIdentity::Normal).await);
        clear_all_trajectory_self_writes();
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn trajectory_watcher_self_write_respects_source_and_expires_records() {
        clear_all_trajectory_self_writes();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("same-id.json");
        let normal = TrajectorySourceIdentity::Normal;
        let task = TrajectorySourceIdentity::task(
            "task-source".to_string(),
            "agents".to_string(),
            Some("agent-source".to_string()),
            None,
            None,
        );
        let content = "{\"id\":\"same-id\"}";
        let handle = register_trajectory_self_write(
            &path,
            normal.clone(),
            content.len() as u64,
            md5::compute(content).0,
        )
        .await
        .unwrap();
        fs::write(&path, content).await.unwrap();
        complete_trajectory_self_write(Some(handle)).await;

        assert!(!consume_trajectory_self_write(&path, &task).await);
        {
            let mut registry = trajectory_self_writes()
                .lock()
                .expect("trajectory self-write registry poisoned");
            for records in registry.values_mut() {
                for record in records {
                    record.created_at = Instant::now() - TRAJECTORY_SELF_WRITE_TTL;
                }
            }
        }
        assert!(!consume_trajectory_self_write(&path, &normal).await);
        clear_all_trajectory_self_writes();
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn trajectory_watcher_self_write_consumes_remove_after_atomic_replacement() {
        clear_all_trajectory_self_writes();
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "self-write-remove";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file_with_user_message(&path, chat_id, "Before Self Write", "before")
            .await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.title = "Before Self Write".to_string();
            session
                .messages
                .push(ChatMessage::new("user".to_string(), "before".to_string()));
            session.trajectory_dirty = false;
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let tmp_path = unique_trajectory_tmp_path(&path);
        atomic_write_json_with_tmp_path(
            &path,
            &tmp_path,
            Ok(content),
            Some("Failed to write trajectory"),
            Some(TrajectorySourceIdentity::Normal),
        )
        .await
        .unwrap();
        process_trajectory_watcher_change(
            gcx,
            chat_id.to_string(),
            true,
            TrajectorySourceIdentity::Normal,
            path,
        )
        .await;

        let session = session_arc.lock().await;
        assert_eq!(session.thread.title, "Before Self Write");
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.external_reload_pending, None);
        drop(session);
        clear_all_trajectory_self_writes();
    }

    #[serial(trajectory_perf, runtime_settings)]
    #[tokio::test]
    async fn trajectory_watcher_self_write_disable_restores_external_reload_path() {
        crate::runtime_settings::reset_for_test();
        clear_all_trajectory_self_writes();
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "self-write-disabled";
        let path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file_with_user_message(&path, chat_id, "Before Self Write", "before")
            .await;
        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.title = "Before Self Write".to_string();
            session
                .messages
                .push(ChatMessage::new("user".to_string(), "before".to_string()));
            session.trajectory_dirty = false;
        }
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc.clone());
        let mut trajectory_events = app.chat.trajectory_events_tx.subscribe();

        let mut disabled = crate::runtime_settings::TrajectoryRuntimeSettings::default();
        disabled.trajectory_watcher_self_write_enabled = false;
        crate::runtime_settings::install_live(&disabled);
        assert!(!trajectory_watcher_self_write_rollout_enabled());

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let tmp_path = unique_trajectory_tmp_path(&path);
        atomic_write_json_with_tmp_path(
            &path,
            &tmp_path,
            Ok(content),
            Some("Failed to write trajectory"),
            Some(TrajectorySourceIdentity::Normal),
        )
        .await
        .unwrap();
        process_trajectory_watcher_change(
            gcx,
            chat_id.to_string(),
            true,
            TrajectorySourceIdentity::Normal,
            path,
        )
        .await;

        assert_eq!(trajectory_events.try_recv().unwrap().id, chat_id);
        crate::runtime_settings::reset_for_test();
        clear_all_trajectory_self_writes();
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn trajectory_watcher_self_write_cleanup_keeps_other_watcher_scope() {
        clear_all_trajectory_self_writes();
        let dir = tempfile::tempdir().unwrap();
        let first_root = dir.path().join("first");
        let second_root = dir.path().join("second");
        tokio::fs::create_dir_all(&first_root).await.unwrap();
        tokio::fs::create_dir_all(&second_root).await.unwrap();
        let first_path = first_root.join("first.json");
        let second_path = second_root.join("second.json");
        let first_handle = register_trajectory_self_write(
            &first_path,
            TrajectorySourceIdentity::Normal,
            "{\"id\":\"first\"}".len() as u64,
            md5::compute("{\"id\":\"first\"}").0,
        )
        .await
        .unwrap();
        let second_handle = register_trajectory_self_write(
            &second_path,
            TrajectorySourceIdentity::Normal,
            "{\"id\":\"second\"}".len() as u64,
            md5::compute("{\"id\":\"second\"}").0,
        )
        .await
        .unwrap();
        tokio::fs::write(&first_path, "{\"id\":\"first\"}")
            .await
            .unwrap();
        tokio::fs::write(&second_path, "{\"id\":\"second\"}")
            .await
            .unwrap();
        complete_trajectory_self_write(Some(first_handle)).await;
        complete_trajectory_self_write(Some(second_handle)).await;

        clear_trajectory_self_writes(&[dunce::simplified(
            &tokio::fs::canonicalize(&first_root).await.unwrap(),
        )
        .to_path_buf()]);

        assert!(
            !consume_trajectory_self_write(&first_path, &TrajectorySourceIdentity::Normal).await
        );
        assert!(
            consume_trajectory_self_write(&second_path, &TrajectorySourceIdentity::Normal).await
        );
        clear_all_trajectory_self_writes();
    }

    #[test]
    fn trajectory_watcher_ignores_atomic_temporary_files() {
        let dir = tempfile::tempdir().unwrap();
        let task_root = dir.path().join(".refact").join("tasks");
        std::fs::create_dir_all(&task_root).unwrap();
        let temporary = task_root
            .join("task-1")
            .join("trajectories")
            .join("agents")
            .join("agent-1")
            .join("chat.json.tmp.abc123");

        assert!(is_trajectory_temporary_file(&temporary));
        assert!(trajectory_watcher_message_for_path(temporary, false, &[task_root]).is_none());
    }

    #[tokio::test]
    async fn trajectory_worktree_unregistered_top_level_metadata_is_stripped() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("repo");
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            cache.clone(),
            crate::test_tmp::scratch_dir("cfg"),
        )
        .await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        }
        let app = AppState::from_gcx(gcx.clone()).await;
        let traj_dir = source.join(".refact").join("trajectories");
        tokio::fs::create_dir_all(&traj_dir).await.unwrap();
        let untrusted = json!({
            "id": "untrusted-wt-chat",
            "title": "Untrusted",
            "model": "m",
            "mode": "agent",
            "tool_use": "agent",
            "messages": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "include_project_info": true,
            "checkpoints_enabled": true,
            "worktree": {
                "id": "wt-evil",
                "kind": "chat",
                "root": dir.path().join("evil").to_string_lossy().to_string(),
                "source_workspace_root": source.to_string_lossy().to_string(),
                "repo_root": source.to_string_lossy().to_string(),
                "enforce": true
            }
        });
        tokio::fs::write(
            traj_dir.join("untrusted-wt-chat.json"),
            serde_json::to_string(&untrusted).unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx.clone(), "untrusted-wt-chat")
            .await
            .unwrap();
        assert!(loaded.thread.worktree.is_none());
        let listed = list_all_trajectories_meta(app, false).await.unwrap();
        let listed_worktree = listed
            .iter()
            .find(|item| item.id == "untrusted-wt-chat")
            .and_then(|item| item.worktree.clone());
        assert!(listed_worktree.is_none());
    }

    #[tokio::test]
    async fn trajectory_worktree_legacy_task_agent_hydrates_from_board_mirror() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("repo");
        let cache = dir.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            cache.clone(),
            crate::test_tmp::scratch_dir("cfg"),
        )
        .await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
        }
        let _app = AppState::from_gcx(gcx.clone()).await;

        let task_id = "task-legacy";
        let agent_id = "agent-1";
        let card_id = "card-1";
        let chat_id = "legacy-agent-chat";
        let task_dir = source.join(".refact").join("tasks").join(task_id);
        tokio::fs::create_dir_all(task_dir.join("trajectories").join("agents").join(agent_id))
            .await
            .unwrap();
        let meta = crate::tasks::types::TaskMeta {
            schema_version: 1,
            id: task_id.to_string(),
            name: "Task".to_string(),
            status: crate::tasks::types::TaskStatus::Active,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            cards_total: 1,
            cards_done: 0,
            cards_failed: 0,
            agents_active: 1,
            base_branch: Some("main".to_string()),
            base_commit: Some("base123".to_string()),
            default_agent_model: None,
            is_name_generated: true,
            last_agents_summary_at: None,
            planner_session_state: None,
        };
        tokio::fs::write(
            task_dir.join("meta.yaml"),
            serde_yaml::to_string(&meta).unwrap(),
        )
        .await
        .unwrap();
        let project_hash = crate::worktrees::service::project_hash_for_path(
            &dunce::simplified(&source.canonicalize().unwrap()).to_path_buf(),
        );
        let worktree_cache_dir = cache.join("worktrees").join(&project_hash);
        std::fs::create_dir_all(&worktree_cache_dir).unwrap();
        let agent_worktree = worktree_cache_dir.join("agent-worktree");
        let agent_worktree_arg = agent_worktree.to_string_lossy().to_string();
        run_git(
            &source,
            &[
                "worktree",
                "add",
                "-b",
                "refact/task/card",
                &agent_worktree_arg,
                "main",
            ],
        );
        let board = crate::tasks::types::TaskBoard {
            schema_version: 1,
            rev: 1,
            columns: Vec::new(),
            cards: vec![crate::tasks::types::BoardCard {
                id: card_id.to_string(),
                title: "Card".to_string(),
                column: "doing".to_string(),
                priority: "P1".to_string(),
                depends_on: Vec::new(),
                instructions: String::new(),
                assignee: Some(agent_id.to_string()),
                agent_chat_id: Some(chat_id.to_string()),
                status_updates: Vec::new(),
                comments: vec![],
                final_report: None,
                final_report_structured: None,
                verifier_report: None,
                created_at: "2024-01-01T00:00:00Z".to_string(),
                started_at: None,
                last_heartbeat_at: None,
                paused_at: None,
                paused_by: None,
                completed_at: None,
                agent_branch: Some("refact/task/card".to_string()),
                agent_worktree: Some(agent_worktree.to_string_lossy().to_string()),
                agent_worktree_name: Some("wt-legacy".to_string()),
                base_branch: None,
                base_commit: None,
                ab_variants: None,
                team_members: vec![],
                target_files: Vec::new(),
                scope_guard_mode: Default::default(),
                attention: None,
                deliverable: None,
                verification: Vec::new(),
            }],
            ..Default::default()
        };
        tokio::fs::write(
            task_dir.join("board.yaml"),
            serde_yaml::to_string(&board).unwrap(),
        )
        .await
        .unwrap();
        let trajectory = json!({
            "id": chat_id,
            "title": "Legacy Agent",
            "model": "m",
            "mode": "task_agent",
            "tool_use": "agent",
            "messages": [],
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "include_project_info": true,
            "checkpoints_enabled": true,
            "task_meta": {
                "task_id": task_id,
                "role": "agents",
                "agent_id": agent_id,
                "card_id": card_id
            }
        });
        tokio::fs::write(
            task_dir
                .join("trajectories")
                .join("agents")
                .join(agent_id)
                .join(format!("{}.json", chat_id)),
            serde_json::to_string(&trajectory).unwrap(),
        )
        .await
        .unwrap();

        let loaded = load_trajectory_for_chat(gcx, chat_id).await.unwrap();
        let worktree = loaded.thread.worktree.unwrap();
        assert_eq!(worktree.id, "wt-legacy");
        assert_eq!(worktree.kind, "task_agent");
        assert_eq!(worktree.root, agent_worktree);
        assert_eq!(worktree.branch.as_deref(), Some("refact/task/card"));
        assert_eq!(worktree.base_branch.as_deref(), Some("main"));
        assert_eq!(worktree.base_commit.as_deref(), Some("base123"));
        assert!(worktree.enforce);
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn trajectory_path_handler_returns_400_for_invalid_id() {
        let dir = tempfile::tempdir().unwrap();
        let (_gcx, app) = make_app_with_workspace(dir.path()).await;
        let err = handle_v1_trajectory_path(State(app), AxumPath("../bad".to_string()))
            .await
            .unwrap_err();
        assert_eq!(err.status_code, StatusCode::BAD_REQUEST);
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn trajectory_path_handler_returns_nested_normal_trajectory_path_for_active_session() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let root_chat_id = "path-normal-nested-root";
        let chat_id = "path-normal-nested-active";
        let path = nested_trajectory_path(dir.path(), root_chat_id, chat_id);
        write_trajectory_file(&path, chat_id, "Normal", "2024-01-01T00:00:00Z").await;

        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        session_arc.lock().await.thread.root_chat_id = Some(root_chat_id.to_string());
        gcx.chat_sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc);

        let response = handle_v1_trajectory_path(State(app), AxumPath(chat_id.to_string()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_same_path_str(payload["path"].as_str().unwrap(), &path);
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn trajectory_path_handler_does_not_leak_other_source_for_active_buddy_session() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "path-collision-buddy";

        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(&normal_path, chat_id, "Normal", "2024-01-01T00:00:00Z").await;

        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.mode = "buddy".to_string();
            session.thread.buddy_meta = Some(BuddyThreadMeta {
                is_buddy_chat: true,
                buddy_chat_kind: "investigation".to_string(),
                workflow_id: None,
                source_chat_id: None,
                source_project: None,
                source_created_at: None,
            });
        }
        gcx.chat_sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc);

        let err = handle_v1_trajectory_path(State(app), AxumPath(chat_id.to_string()))
            .await
            .unwrap_err();
        assert_eq!(err.status_code, StatusCode::NOT_FOUND);
        assert!(tokio::fs::try_exists(&normal_path).await.unwrap());
    }

    #[serial(trajectory_perf)]
    #[tokio::test]
    async fn trajectory_path_handler_returns_buddy_path_when_active_buddy_session_has_buddy_file() {
        let dir = tempfile::tempdir().unwrap();
        let (gcx, app) = make_app_with_workspace(dir.path()).await;
        let chat_id = "path-buddy-resolved";

        let normal_path = dir
            .path()
            .join(".refact")
            .join("trajectories")
            .join(format!("{chat_id}.json"));
        write_trajectory_file(&normal_path, chat_id, "Normal", "2024-01-01T00:00:00Z").await;

        let buddy_path = dir
            .path()
            .join(".refact")
            .join("buddy")
            .join("chats")
            .join("conversations")
            .join(format!("{chat_id}.json"));
        write_buddy_conversation_file(&buddy_path, chat_id, "Buddy").await;

        let session_arc = Arc::new(AMutex::new(ChatSession::new(chat_id.to_string())));
        {
            let mut session = session_arc.lock().await;
            session.thread.mode = "buddy".to_string();
            session.thread.buddy_meta = Some(BuddyThreadMeta {
                is_buddy_chat: true,
                buddy_chat_kind: "investigation".to_string(),
                workflow_id: None,
                source_chat_id: None,
                source_project: None,
                source_created_at: None,
            });
        }
        gcx.chat_sessions
            .write()
            .await
            .insert(chat_id.to_string(), session_arc);

        let response = handle_v1_trajectory_path(State(app), AxumPath(chat_id.to_string()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let returned = payload["path"].as_str().unwrap();
        assert_same_path_str(returned, &buddy_path);
        assert_ne!(
            normalized_test_path(Path::new(returned)),
            normalized_test_path(&normal_path)
        );
    }
}
