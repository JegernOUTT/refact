use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use tokio::fs;
use tracing::warn;

pub use refact_buddy_core::storage::*;

use super::memory_lifecycle::{
    apply_memory_lifecycle_op_status, memory_op_duplicate_should_replace, MemoryLifecycleOp,
    MemoryOpStatus, MemoryOpsRecord, MemoryOpsState, MemorySource,
};
use super::settings::AutonomyLevel;
use crate::app_state::AppState;

const MEMORY_OPS_COMPACT_KEEP_DAYS: i64 = 7;
const MEMORY_OPS_PENDING_TTL_DAYS: i64 = 30;
const MEMORY_OPS_AUTOMATED_FINAL_MAX: usize = 10_000;

static MEMORY_OPS_IO_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const MEMORY_OPS_COMPACT_TRIGGER_BYTES: u64 = 8 * 1024 * 1024;

static MEMORY_OPS_COMPACTED_LEN: std::sync::Mutex<Option<HashMap<PathBuf, u64>>> =
    std::sync::Mutex::new(None);

fn last_compacted_len(project_root: &Path) -> Option<u64> {
    MEMORY_OPS_COMPACTED_LEN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .and_then(|lens| lens.get(project_root).copied())
}

fn remember_compacted_len(project_root: &Path, len: u64) {
    MEMORY_OPS_COMPACTED_LEN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(project_root.to_path_buf(), len);
}

/// Proof that the caller holds the cross-process memory ops file lock.
pub(crate) struct MemoryOpsLock(());

fn memory_ops_lock_path_for(queue_path: &Path) -> PathBuf {
    let mut raw = queue_path.as_os_str().to_owned();
    raw.push(".lock");
    PathBuf::from(raw)
}

fn memory_ops_lock_path(project_root: &Path) -> PathBuf {
    memory_ops_lock_path_for(&memory_ops_path(project_root))
}

fn with_memory_ops_lock<T>(
    lock_path: &Path,
    body: impl FnOnce(&MemoryOpsLock) -> Result<T, String>,
) -> Result<T, String> {
    let mut lock = crate::daemon::lock::open_lock(lock_path)
        .map_err(|err| format!("Failed to open memory ops lock {:?}: {}", lock_path, err))?;
    let guard = crate::daemon::lock::lock_blocking(&mut lock)
        .map_err(|err| format!("Failed to lock memory ops queue {:?}: {}", lock_path, err))?;
    let result = body(&MemoryOpsLock(()));
    drop(guard);
    result
}

async fn memory_ops_transaction<T, F>(project_root: &Path, body: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&Path, &MemoryOpsLock) -> Result<T, String> + Send + 'static,
{
    let root = project_root.to_path_buf();
    let _order = MEMORY_OPS_IO_LOCK.lock().await;
    tokio::task::spawn_blocking(move || {
        let lock_path = memory_ops_lock_path(&root);
        with_memory_ops_lock(&lock_path, |lock| body(&root, lock))
    })
    .await
    .map_err(|err| format!("memory ops io task failed: {}", err))?
}

#[derive(Clone, PartialEq, Eq)]
struct MemoryOpsFileStamp {
    len: u64,
    modified: Option<std::time::SystemTime>,
    identity: Option<(u64, u64)>,
}

fn stamp_from_metadata(metadata: &std::fs::Metadata) -> MemoryOpsFileStamp {
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        Some((metadata.dev(), metadata.ino()))
    };
    #[cfg(not(unix))]
    let identity = None;
    MemoryOpsFileStamp {
        len: metadata.len(),
        modified: metadata.modified().ok(),
        identity,
    }
}

struct MemoryOpsCacheEntry {
    root: PathBuf,
    stamp: MemoryOpsFileStamp,
    state: MemoryOpsState,
}

/// How many project roots keep a parsed `MemoryOpsState` in memory at once.
///
/// One process serves many project roots (the daemon and its workers routinely
/// hold more than one), so a single global slot thrashed to a ~0% hit rate as
/// soon as two roots were active. Each entry holds a whole `MemoryOpsState`,
/// which is far heavier than the `MAX_CACHED_TRAJECTORY_DIRECTORIES = 4096`
/// path entries in `chat::trajectory_index`, so the bound here is small: 32
/// comfortably covers the hot set of roots a single process touches while
/// capping worst-case retention at 32 parsed queues.
const MEMORY_OPS_CACHE_CAPACITY: usize = 32;

/// Least-recently-used first, most-recently-used last. Capped at
/// `MEMORY_OPS_CACHE_CAPACITY`, so the linear scans below are bounded and
/// cheaper than a hash lookup plus a separate recency structure.
static MEMORY_OPS_CACHE: std::sync::Mutex<Vec<MemoryOpsCacheEntry>> =
    std::sync::Mutex::new(Vec::new());

async fn memory_ops_file_stamp(project_root: &Path) -> Option<MemoryOpsFileStamp> {
    let metadata = fs::metadata(memory_ops_path(project_root)).await.ok()?;
    Some(stamp_from_metadata(&metadata))
}

fn memory_ops_file_stamp_sync(project_root: &Path) -> Option<MemoryOpsFileStamp> {
    let metadata = std::fs::metadata(memory_ops_path(project_root)).ok()?;
    Some(stamp_from_metadata(&metadata))
}

fn cached_memory_ops_state(
    cache: &std::sync::Mutex<Vec<MemoryOpsCacheEntry>>,
    project_root: &Path,
    stamp: &MemoryOpsFileStamp,
) -> Option<MemoryOpsState> {
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    memory_ops_cache_lookup(&mut cache, project_root, stamp)
}

fn store_memory_ops_state(
    cache: &std::sync::Mutex<Vec<MemoryOpsCacheEntry>>,
    project_root: &Path,
    stamp: MemoryOpsFileStamp,
    state: &MemoryOpsState,
) {
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    memory_ops_cache_store(&mut cache, project_root, stamp, state);
}

/// Drop whatever is cached for `project_root`, whatever its stamp. A rewrite
/// replaces the queue file wholesale, so the parsed state we are holding is
/// superseded the moment the rename lands; dropping it here keeps that
/// guarantee explicit instead of relying on the next lookup noticing a stamp
/// difference (which a same-length rewrite on a filesystem without inode
/// identity would not necessarily show).
fn invalidate_memory_ops_state(
    cache: &std::sync::Mutex<Vec<MemoryOpsCacheEntry>>,
    project_root: &Path,
) {
    cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .retain(|entry| entry.root != project_root);
}

/// Lookup half of the cache, split out from the global so it can be tested on a
/// private `Vec` without racing the other tests that share `MEMORY_OPS_CACHE`.
fn memory_ops_cache_lookup(
    cache: &mut Vec<MemoryOpsCacheEntry>,
    project_root: &Path,
    stamp: &MemoryOpsFileStamp,
) -> Option<MemoryOpsState> {
    let index = cache.iter().position(|entry| entry.root == project_root)?;
    // A stamp mismatch means the file moved on: never serve it, and drop the
    // entry so a superseded state cannot occupy the bound.
    if cache[index].stamp != *stamp {
        cache.remove(index);
        return None;
    }
    let entry = cache.remove(index);
    let state = entry.state.clone();
    cache.push(entry);
    Some(state)
}

/// Store half of the cache. See [`memory_ops_cache_lookup`].
fn memory_ops_cache_store(
    cache: &mut Vec<MemoryOpsCacheEntry>,
    project_root: &Path,
    stamp: MemoryOpsFileStamp,
    state: &MemoryOpsState,
) {
    if let Some(index) = cache.iter().position(|entry| entry.root == project_root) {
        cache.remove(index);
    }
    cache.push(MemoryOpsCacheEntry {
        root: project_root.to_path_buf(),
        stamp,
        state: state.clone(),
    });
    while cache.len() > MEMORY_OPS_CACHE_CAPACITY {
        cache.remove(0);
    }
}

fn memory_ops_path(project_root: &Path) -> PathBuf {
    project_root.join(".refact/buddy/memory_ops.jsonl")
}

fn memory_ops_backup_path(project_root: &Path) -> PathBuf {
    project_root.join(".refact/buddy/memory_ops.jsonl.bak")
}

fn memory_ops_bad_path(project_root: &Path) -> PathBuf {
    project_root.join(".refact/buddy/memory_ops.jsonl.bad")
}

fn create_parent_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create dir {:?}: {}", parent, e))?;
    }
    Ok(())
}

fn fsync_parent_dir(path: &Path) {
    if let Some(parent) = path.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
}

fn serialize_memory_ops_records(records: &[MemoryOpsRecord]) -> Result<String, String> {
    let mut buf = String::new();
    for record in records {
        let line = serde_json::to_string(record)
            .map_err(|e| format!("Failed to serialize memory op record: {}", e))?;
        buf.push_str(&line);
        buf.push('\n');
    }
    Ok(buf)
}

fn memory_ops_tmp_path(path: &Path) -> PathBuf {
    path.with_extension(format!("jsonl.{}.tmp", uuid::Uuid::new_v4()))
}

fn write_tmp_file(tmp: &Path, buf: &str) -> Result<(), String> {
    let mut file =
        std::fs::File::create(tmp).map_err(|e| format!("Failed to create {:?}: {}", tmp, e))?;
    file.write_all(buf.as_bytes())
        .map_err(|e| format!("Failed to write {:?}: {}", tmp, e))?;
    file.sync_all()
        .map_err(|e| format!("Failed to fsync {:?}: {}", tmp, e))
}

#[cfg(test)]
pub(crate) async fn rewrite_memory_ops_records(
    project_root: &Path,
    records: Vec<MemoryOpsRecord>,
) -> Result<(), String> {
    memory_ops_transaction(project_root, move |root, lock| {
        rewrite_memory_ops_records_locked(lock, &memory_ops_path(root), records)
    })
    .await
}

fn stage_memory_ops_tmp(path: &Path, records: &[MemoryOpsRecord]) -> Result<PathBuf, String> {
    create_parent_dir(path)?;
    let buf = serialize_memory_ops_records(records)?;
    let tmp = memory_ops_tmp_path(path);
    match write_tmp_file(&tmp, &buf) {
        Ok(()) => Ok(tmp),
        Err(err) => {
            let _ = std::fs::remove_file(&tmp);
            Err(err)
        }
    }
}

fn commit_memory_ops_tmp(tmp: &Path, path: &Path) -> Result<(), String> {
    match std::fs::rename(tmp, path) {
        Ok(()) => {
            fsync_parent_dir(path);
            Ok(())
        }
        Err(err) => {
            let _ = std::fs::remove_file(tmp);
            Err(format!("Failed to rename {:?} to {:?}: {}", tmp, path, err))
        }
    }
}

fn rewrite_memory_ops_records_locked(
    _lock: &MemoryOpsLock,
    path: &Path,
    records: Vec<MemoryOpsRecord>,
) -> Result<(), String> {
    let tmp = stage_memory_ops_tmp(path, &records)?;
    commit_memory_ops_tmp(&tmp, path)
}

const MEMORY_OPS_TMP_GC_MAX_AGE_SECS: u64 = 3600;

pub async fn gc_stale_memory_ops_tmp_files(project_root: &Path) -> usize {
    gc_stale_memory_ops_tmp_files_with_max_age(project_root, MEMORY_OPS_TMP_GC_MAX_AGE_SECS).await
}

async fn gc_stale_memory_ops_tmp_files_with_max_age(
    project_root: &Path,
    max_age_secs: u64,
) -> usize {
    let removed = memory_ops_transaction(project_root, move |root, lock| {
        Ok(gc_stale_memory_ops_tmp_files_locked(
            lock,
            root,
            max_age_secs,
        ))
    })
    .await;
    match removed {
        Ok(removed) => removed,
        Err(err) => {
            warn!(
                "buddy: failed to collect stale memory ops tmp files: {}",
                err
            );
            0
        }
    }
}

fn gc_stale_memory_ops_tmp_files_locked(
    _lock: &MemoryOpsLock,
    project_root: &Path,
    max_age_secs: u64,
) -> usize {
    let dir = project_root.join(".refact/buddy");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let now = std::time::SystemTime::now();
    let mut removed = 0usize;
    for entry in rd.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !(name.starts_with("memory_ops.jsonl.") && name.ends_with(".tmp")) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let old_enough = metadata
            .modified()
            .ok()
            .and_then(|mtime| now.duration_since(mtime).ok())
            .map(|age| age.as_secs() >= max_age_secs)
            .unwrap_or(false);
        if !old_enough {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(err) => warn!(
                "buddy: failed to remove stale memory ops tmp file {:?}: {}",
                path, err
            ),
        }
    }
    if removed > 0 {
        warn!(
            "buddy: removed {} stale memory ops tmp file(s) from {:?}",
            removed, dir
        );
    }
    removed
}

fn memory_op_timestamp(op: &MemoryLifecycleOp) -> DateTime<Utc> {
    op.applied_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .or_else(|| DateTime::parse_from_rfc3339(&op.created_at).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|| DateTime::<Utc>::from(std::time::UNIX_EPOCH))
}

fn memory_op_is_final_status(status: MemoryOpStatus) -> bool {
    matches!(
        status,
        MemoryOpStatus::Applied
            | MemoryOpStatus::Rejected
            | MemoryOpStatus::Failed
            | MemoryOpStatus::Skipped
    )
}

fn memory_source_is_automated(source: MemorySource) -> bool {
    matches!(
        source,
        MemorySource::Trajectory
            | MemorySource::MemoryGarden
            | MemorySource::BehaviorLearner
            | MemorySource::KnowledgeConflictResolver
    )
}

fn memory_op_survives_compaction(op: &MemoryLifecycleOp, now: DateTime<Utc>) -> bool {
    if !memory_op_is_final_status(op.status) {
        let cutoff = now - chrono::Duration::days(MEMORY_OPS_PENDING_TTL_DAYS);
        return memory_op_timestamp(op) >= cutoff;
    }
    if !memory_source_is_automated(op.source) {
        return true;
    }
    let cutoff = now - chrono::Duration::days(MEMORY_OPS_COMPACT_KEEP_DAYS);
    memory_op_timestamp(op) >= cutoff
}

fn cap_automated_final_records(records: Vec<MemoryOpsRecord>, max: usize) -> Vec<MemoryOpsRecord> {
    let mut automated: Vec<(DateTime<Utc>, usize)> = records
        .iter()
        .enumerate()
        .filter_map(|(index, record)| {
            let MemoryOpsRecord::Op {
                op,
            } = record;
            (memory_op_is_final_status(op.status) && memory_source_is_automated(op.source))
                .then(|| (memory_op_timestamp(op), index))
        })
        .collect();
    if automated.len() <= max {
        return records;
    }
    automated.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let dropped: HashSet<usize> = automated[max..].iter().map(|(_, index)| *index).collect();
    records
        .into_iter()
        .enumerate()
        .filter(|(index, _)| !dropped.contains(index))
        .map(|(_, record)| record)
        .collect()
}

fn compact_memory_ops_records(
    records: impl IntoIterator<Item = MemoryOpsRecord>,
    now: DateTime<Utc>,
) -> MemoryOpsState {
    let mut by_op_id: BTreeMap<String, MemoryLifecycleOp> = BTreeMap::new();
    let mut without_op_id = Vec::new();
    for record in records {
        let op = record.into_op();
        let op = op.normalized();
        if !memory_op_survives_compaction(&op, now) {
            continue;
        }
        if op.op_id.trim().is_empty() {
            without_op_id.push(MemoryOpsRecord::Op {
                op,
            });
            continue;
        }
        by_op_id
            .entry(op.op_id.clone())
            .and_modify(|existing| {
                if memory_op_timestamp(&op) >= memory_op_timestamp(existing) {
                    *existing = op.clone();
                }
            })
            .or_insert(op);
    }
    let mut records = without_op_id;
    records.extend(by_op_id.into_values().map(|op| MemoryOpsRecord::Op {
        op,
    }));
    let records = cap_automated_final_records(records, MEMORY_OPS_AUTOMATED_FINAL_MAX);
    MemoryOpsState::from_records(records)
}

struct MemoryOpsFileRead {
    records: Vec<MemoryOpsRecord>,
    malformed: Vec<(usize, String, String)>,
}

fn parse_memory_ops_content(content: &str) -> MemoryOpsFileRead {
    let mut records = Vec::new();
    let mut malformed = Vec::<(usize, String, String)>::new();
    for (idx, raw) in content.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<MemoryOpsRecord>(line) {
            Ok(record) => records.push(record),
            Err(err) => malformed.push((idx + 1, raw.to_string(), err.to_string())),
        }
    }
    MemoryOpsFileRead {
        records,
        malformed,
    }
}

fn read_memory_ops_file_sync(project_root: &Path) -> Result<MemoryOpsFileRead, String> {
    let path = memory_ops_path(project_root);
    match std::fs::read_to_string(&path) {
        Ok(content) => Ok(parse_memory_ops_content(&content)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(MemoryOpsFileRead {
            records: Vec::new(),
            malformed: Vec::new(),
        }),
        Err(err) => Err(format!(
            "Failed to read memory ops queue {:?}: {}",
            path, err
        )),
    }
}

async fn read_memory_ops_file(project_root: &Path) -> Option<MemoryOpsFileRead> {
    let path = memory_ops_path(project_root);
    let content = match fs::read_to_string(&path).await {
        Ok(content) => content,
        Err(err) if err.kind() == ErrorKind::NotFound => String::new(),
        Err(err) => {
            warn!(
                "buddy: failed to read memory ops queue at {:?}: {}",
                path, err
            );
            return None;
        }
    };
    Some(parse_memory_ops_content(&content))
}

fn read_memory_ops_records_locked(
    lock: &MemoryOpsLock,
    project_root: &Path,
) -> Result<(Vec<MemoryOpsRecord>, u32), String> {
    let path = memory_ops_path(project_root);
    let read = read_memory_ops_file_sync(project_root)?;
    let malformed_lines = read.malformed.len().min(u32::MAX as usize) as u32;
    if malformed_lines > 0 {
        warn!(
            "buddy: quarantining {} malformed memory ops queue line(s) from {:?}",
            malformed_lines, path
        );
        match quarantine_memory_ops_bad_lines(project_root, &path, &read.malformed) {
            Ok(()) => {
                if let Err(err) =
                    rewrite_memory_ops_records_locked(lock, &path, read.records.clone())
                {
                    warn!(
                        "buddy: failed to repair memory ops queue after quarantine: {}",
                        err
                    );
                }
            }
            Err(err) => {
                warn!(
                    "buddy: failed to quarantine malformed memory ops lines: {}",
                    err
                );
            }
        }
    }
    Ok((read.records, malformed_lines))
}

fn memory_bad_line_hash(raw: &str, err: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    hasher.update(b"\0");
    hasher.update(err.as_bytes());
    hex::encode(hasher.finalize())
}

fn existing_quarantine_hashes(path: &Path) -> HashSet<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return HashSet::new();
    };
    content
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line.trim()).ok())
        .filter_map(|value| {
            value
                .get("raw_hash")
                .and_then(|hash| hash.as_str())
                .map(|hash| hash.to_string())
        })
        .collect()
}

fn quarantine_memory_ops_bad_lines(
    project_root: &Path,
    path: &Path,
    malformed: &[(usize, String, String)],
) -> Result<(), String> {
    if malformed.is_empty() {
        return Ok(());
    }
    let bad_path = memory_ops_bad_path(project_root);
    let existing_hashes = existing_quarantine_hashes(&bad_path);
    create_parent_dir(&bad_path)?;
    let mut buf = String::new();
    for (line_number, raw, err) in malformed {
        let raw = crate::llm::safe_truncate(raw.trim(), 2000).to_string();
        let line_hash = memory_bad_line_hash(&raw, err);
        if existing_hashes.contains(&line_hash) {
            continue;
        }
        let record = serde_json::json!({
            "quarantined_at": Utc::now().to_rfc3339(),
            "source_path": path.to_string_lossy(),
            "line": line_number,
            "error": err,
            "raw_hash": line_hash,
            "raw": raw,
        });
        buf.push_str(&record.to_string());
        buf.push('\n');
    }
    if buf.is_empty() {
        return Ok(());
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&bad_path)
        .map_err(|e| format!("Failed to open memory ops quarantine {:?}: {}", bad_path, e))?;
    file.write_all(buf.as_bytes()).map_err(|e| {
        format!(
            "Failed to append memory ops quarantine {:?}: {}",
            bad_path, e
        )
    })?;
    file.flush().map_err(|e| {
        format!(
            "Failed to flush memory ops quarantine {:?}: {}",
            bad_path, e
        )
    })
}

pub async fn enqueue_memory_op(
    project_root: &Path,
    op: MemoryLifecycleOp,
) -> Result<MemoryOpsState, String> {
    enqueue_memory_op_with_compact_threshold(project_root, op, MEMORY_OPS_COMPACT_TRIGGER_BYTES)
        .await
}

async fn enqueue_memory_op_with_compact_threshold(
    project_root: &Path,
    op: MemoryLifecycleOp,
    compact_threshold_bytes: u64,
) -> Result<MemoryOpsState, String> {
    memory_ops_transaction(project_root, move |root, lock| {
        enqueue_memory_op_locked(lock, root, op, compact_threshold_bytes)
    })
    .await
}

fn enqueue_memory_op_locked(
    lock: &MemoryOpsLock,
    project_root: &Path,
    op: MemoryLifecycleOp,
    compact_threshold_bytes: u64,
) -> Result<MemoryOpsState, String> {
    let incoming_has_key = !op.idempotency_key.trim().is_empty();
    let stamp = memory_ops_file_stamp_sync(project_root);
    let current = match stamp
        .as_ref()
        .and_then(|stamp| cached_memory_ops_state(&MEMORY_OPS_CACHE, project_root, stamp))
    {
        Some(state) => state,
        None => {
            let (records, malformed_lines) = read_memory_ops_records_locked(lock, project_root)?;
            let state = MemoryOpsState::from_records_with_malformed(records, malformed_lines);
            if let Some(stamp) = memory_ops_file_stamp_sync(project_root) {
                store_memory_ops_state(&MEMORY_OPS_CACHE, project_root, stamp, &state);
            }
            state
        }
    };
    if let Some(existing) = current.matching_op(&op) {
        if !memory_op_duplicate_should_replace(existing.status, op.status) {
            return Ok(current);
        }
    }
    let mut op = op.normalized();
    if !incoming_has_key {
        op.idempotency_key.clear();
    }

    let path = memory_ops_path(project_root);
    create_parent_dir(&path)?;
    let record = MemoryOpsRecord::Op {
        op,
    };
    let serialized = serde_json::to_string(&record)
        .map_err(|e| format!("Failed to serialize memory op record: {}", e))?;
    let replayed = serde_json::from_str::<MemoryOpsRecord>(&serialized)
        .map_err(|e| format!("Serialized memory op record did not round-trip: {}", e))?;
    let line = format!("{}\n", serialized);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("Failed to open memory ops queue {:?}: {}", path, e))?;
    file.write_all(line.as_bytes())
        .map_err(|e| format!("Failed to append memory ops queue {:?}: {}", path, e))?;
    file.flush()
        .map_err(|e| format!("Failed to flush memory ops queue {:?}: {}", path, e))?;
    drop(file);
    let state = match archive_memory_ops_if_oversized_locked(
        lock,
        project_root,
        compact_threshold_bytes,
    )? {
        Some(compacted) => compacted,
        None => current.with_appended_record(replayed),
    };
    if let Some(stamp) = memory_ops_file_stamp_sync(project_root) {
        store_memory_ops_state(&MEMORY_OPS_CACHE, project_root, stamp, &state);
    }
    Ok(state)
}

fn drafts_path(project_root: &Path) -> PathBuf {
    project_root.join(".refact/buddy/drafts.json")
}

pub async fn load_drafts(project_root: &Path) -> Vec<refact_buddy_core::types::BuddyDraft> {
    let path = drafts_path(project_root);
    let content = match fs::read_to_string(&path).await {
        Ok(content) => content,
        Err(err) if err.kind() == ErrorKind::NotFound => return Vec::new(),
        Err(err) => {
            warn!("buddy: failed to read drafts at {:?}: {}", path, err);
            return Vec::new();
        }
    };
    match serde_json::from_str::<Vec<refact_buddy_core::types::BuddyDraft>>(&content) {
        Ok(drafts) => {
            let now = DateTime::<Utc>::from(std::time::SystemTime::now());
            drafts
                .into_iter()
                .filter(|draft| draft.expires_at > now)
                .collect()
        }
        Err(err) => {
            warn!("buddy: failed to parse drafts at {:?}: {}", path, err);
            Vec::new()
        }
    }
}

pub async fn save_drafts(
    project_root: &Path,
    drafts: &[refact_buddy_core::types::BuddyDraft],
) -> Result<(), String> {
    atomic_write_json(&drafts_path(project_root), &drafts.to_vec()).await
}

pub async fn load_memory_ops(project_root: &Path) -> MemoryOpsState {
    load_memory_ops_with_cache(project_root, &MEMORY_OPS_CACHE).await
}

async fn load_memory_ops_with_cache(
    project_root: &Path,
    cache: &std::sync::Mutex<Vec<MemoryOpsCacheEntry>>,
) -> MemoryOpsState {
    let stamp = memory_ops_file_stamp(project_root).await;
    if let Some(state) = stamp
        .as_ref()
        .and_then(|stamp| cached_memory_ops_state(cache, project_root, stamp))
    {
        return state;
    }
    let Some(read) = read_memory_ops_file(project_root).await else {
        return MemoryOpsState::default();
    };
    let malformed_lines = read.malformed.len().min(u32::MAX as usize) as u32;
    let state = MemoryOpsState::from_records_with_malformed(read.records, malformed_lines);
    if let Some(stamp) = stamp {
        if memory_ops_file_stamp(project_root).await.as_ref() == Some(&stamp) {
            store_memory_ops_state(cache, project_root, stamp, &state);
        }
    }
    state
}

pub async fn load_memory_ops_repairing(project_root: &Path) -> MemoryOpsState {
    let loaded = memory_ops_transaction(project_root, |root, lock| {
        let (records, malformed_lines) = read_memory_ops_records_locked(lock, root)?;
        Ok(MemoryOpsState::from_records_with_malformed(
            records,
            malformed_lines,
        ))
    })
    .await;
    match loaded {
        Ok(state) => state,
        Err(err) => {
            warn!("buddy: failed to load memory ops queue: {}", err);
            MemoryOpsState::default()
        }
    }
}

async fn snapshot_memory_ops(project_root: &Path) -> MemoryOpsState {
    load_memory_ops_repairing(project_root).await
}

static MEMORY_OPS_APPLY_CLAIMS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

struct MemoryOpClaimGuard {
    ids: Vec<String>,
}

impl Drop for MemoryOpClaimGuard {
    fn drop(&mut self) {
        if self.ids.is_empty() {
            return;
        }
        let mut claims = MEMORY_OPS_APPLY_CLAIMS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        claims.retain(|id| !self.ids.contains(id));
    }
}

fn memory_op_claim_key(project_root: &Path, op_id: &str) -> String {
    format!("{}\u{1f}{}", project_root.to_string_lossy(), op_id)
}

fn claim_memory_op_ids(
    project_root: &Path,
    candidate_ids: &[String],
) -> (HashSet<String>, MemoryOpClaimGuard) {
    let mut claimed = HashSet::new();
    let mut ids = Vec::new();
    {
        let mut claims = MEMORY_OPS_APPLY_CLAIMS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for op_id in candidate_ids {
            if op_id.trim().is_empty() {
                continue;
            }
            let key = memory_op_claim_key(project_root, op_id);
            if claims.iter().any(|id| id == &key) {
                continue;
            }
            claims.push(key.clone());
            ids.push(key);
            claimed.insert(op_id.clone());
        }
    }
    (
        claimed,
        MemoryOpClaimGuard {
            ids,
        },
    )
}

async fn revalidation_snapshot(
    project_root: &Path,
    claimed: &HashSet<String>,
) -> std::collections::HashMap<String, MemoryLifecycleOp> {
    if claimed.is_empty() {
        return std::collections::HashMap::new();
    }
    let fresh = snapshot_memory_ops(project_root).await;
    fresh
        .ops
        .into_iter()
        .filter(|op| claimed.contains(&op.op_id))
        .map(|op| {
            let normalized = op.normalized();
            (normalized.op_id.clone(), normalized)
        })
        .collect()
}

async fn merge_processed_ops_and_rewrite(
    project_root: &Path,
    processed: Vec<(MemoryLifecycleOp, MemoryLifecycleOp)>,
) -> Result<MemoryOpsState, String> {
    memory_ops_transaction(project_root, move |root, lock| {
        merge_processed_ops_and_rewrite_locked(lock, root, processed)
    })
    .await
}

fn merge_processed_ops_and_rewrite_locked(
    lock: &MemoryOpsLock,
    project_root: &Path,
    processed: Vec<(MemoryLifecycleOp, MemoryLifecycleOp)>,
) -> Result<MemoryOpsState, String> {
    let (records, malformed_lines) = read_memory_ops_records_locked(lock, project_root)?;
    let mut state = MemoryOpsState::from_records_with_malformed(records, malformed_lines);
    for (expected, updated) in processed {
        match state.ops.iter_mut().find(|op| op.op_id == expected.op_id) {
            Some(current) if current.clone().normalized() == expected => {
                *current = updated.normalized();
            }
            Some(current) => {
                warn!(
                    "buddy: memory op {} changed concurrently ({} vs expected {}), keeping newer state",
                    expected.op_id,
                    current.status.as_str(),
                    expected.status.as_str()
                );
            }
            None => {
                warn!(
                    "buddy: memory op {} vanished during apply, not re-appending",
                    expected.op_id
                );
            }
        }
    }
    let state = MemoryOpsState::from_records_with_malformed(
        state.ops.into_iter().map(|op| MemoryOpsRecord::Op {
            op,
        }),
        malformed_lines,
    );
    rewrite_memory_ops_records_locked(
        lock,
        &memory_ops_path(project_root),
        state.canonical_records(),
    )?;
    invalidate_memory_ops_state(&MEMORY_OPS_CACHE, project_root);
    Ok(state)
}

pub async fn compact_memory_ops(project_root: &Path) -> Result<MemoryOpsState, String> {
    memory_ops_transaction(project_root, |root, lock| {
        compact_memory_ops_locked(lock, root)
    })
    .await
}

fn compact_memory_ops_locked(
    lock: &MemoryOpsLock,
    project_root: &Path,
) -> Result<MemoryOpsState, String> {
    let (records, _) = read_memory_ops_records_locked(lock, project_root)?;
    let state = compact_memory_ops_records(records, Utc::now());
    let path = memory_ops_path(project_root);
    rewrite_memory_ops_records_locked(lock, &path, state.canonical_records())?;
    remember_compacted_file(project_root, &state);
    Ok(state)
}

fn remember_compacted_file(project_root: &Path, state: &MemoryOpsState) {
    if let Some(stamp) = memory_ops_file_stamp_sync(project_root) {
        remember_compacted_len(project_root, stamp.len);
        store_memory_ops_state(&MEMORY_OPS_CACHE, project_root, stamp, state);
    }
}

pub async fn archive_memory_ops_if_oversized(
    project_root: &Path,
    threshold_bytes: u64,
) -> Result<bool, String> {
    memory_ops_transaction(project_root, move |root, lock| {
        Ok(archive_memory_ops_if_oversized_locked(lock, root, threshold_bytes)?.is_some())
    })
    .await
}

fn link_live_queue_to_backup(path: &Path, backup: &Path) -> Result<(), String> {
    match std::fs::remove_file(backup) {
        Ok(()) => {}
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => {
            return Err(format!(
                "Failed to remove existing backup {:?}: {}",
                backup, err
            ))
        }
    }
    match std::fs::hard_link(path, backup) {
        Ok(()) => Ok(()),
        Err(_) => std::fs::copy(path, backup)
            .map(|_| ())
            .map_err(|err| format!("Failed to back up {:?} to {:?}: {}", path, backup, err)),
    }
}

fn archive_memory_ops_if_oversized_locked(
    lock: &MemoryOpsLock,
    project_root: &Path,
    threshold_bytes: u64,
) -> Result<Option<MemoryOpsState>, String> {
    let path = memory_ops_path(project_root);
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(format!(
                "Failed to stat memory ops queue {:?}: {}",
                path, err
            ))
        }
    };
    if metadata.len() <= threshold_bytes {
        return Ok(None);
    }
    if let Some(floor) = last_compacted_len(project_root) {
        if metadata.len() < floor.saturating_mul(2) {
            return Ok(None);
        }
    }
    let (records, _) = read_memory_ops_records_locked(lock, project_root)?;
    let compacted = compact_memory_ops_records(records, Utc::now());
    let tmp = stage_memory_ops_tmp(&path, &compacted.canonical_records())?;
    if let Err(err) = link_live_queue_to_backup(&path, &memory_ops_backup_path(project_root)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    commit_memory_ops_tmp(&tmp, &path)?;
    remember_compacted_file(project_root, &compacted);
    Ok(Some(compacted))
}

pub async fn drain_memory_ops(
    project_root: &Path,
    gcx: AppState,
    autonomy: AutonomyLevel,
    max_applies: usize,
) -> Result<(MemoryOpsState, usize), String> {
    let state = snapshot_memory_ops(project_root).await;
    if autonomy == AutonomyLevel::ReadOnly {
        return Ok((state, 0));
    }
    let mut candidates: Vec<(MemoryLifecycleOp, bool)> = Vec::new();
    for op in state.ops.iter() {
        if candidates.len() >= max_applies {
            break;
        }
        let auto_apply = op.status == MemoryOpStatus::Pending;
        let eligible = op.status == MemoryOpStatus::Approved || auto_apply;
        if !eligible {
            continue;
        }
        candidates.push((op.clone().normalized(), auto_apply));
    }
    let candidate_ids: Vec<String> = candidates.iter().map(|(op, _)| op.op_id.clone()).collect();
    let (claimed, _claim_guard) = claim_memory_op_ids(project_root, &candidate_ids);
    let fresh_by_id = revalidation_snapshot(project_root, &claimed).await;
    let mut changed = 0usize;
    let mut lost_race = false;
    let mut processed: Vec<(MemoryLifecycleOp, MemoryLifecycleOp)> = Vec::new();
    for (original, auto_apply) in candidates {
        if !claimed.contains(&original.op_id) {
            lost_race = true;
            continue;
        }
        if fresh_by_id.get(&original.op_id) != Some(&original) {
            lost_race = true;
            continue;
        }
        let mut working = original.clone();
        if auto_apply {
            working.status = MemoryOpStatus::Approved;
            working.error = None;
        }
        let updated = apply_memory_lifecycle_op_status(gcx.clone(), &working)
            .await
            .normalized();
        if updated != original {
            changed += 1;
            processed.push((original, updated));
        }
    }
    if changed > 0 {
        let state = merge_processed_ops_and_rewrite(project_root, processed).await?;
        return Ok((state, changed));
    }
    if lost_race {
        return Ok((snapshot_memory_ops(project_root).await, changed));
    }
    Ok((state, changed))
}

pub async fn apply_memory_batch(
    project_root: &Path,
    gcx: AppState,
    batch_key: &str,
    max_applies: usize,
) -> Result<(MemoryOpsState, usize, usize, usize), String> {
    let state = snapshot_memory_ops(project_root).await;
    let candidates: Vec<MemoryLifecycleOp> = state
        .ops
        .iter()
        .filter(|op| {
            refact_buddy_core::memory_lifecycle_model::memory_op_awaits_approval(op)
                && refact_buddy_core::memory_lifecycle_model::memory_op_batch_key(op) == batch_key
        })
        .take(max_applies)
        .map(|op| op.clone().normalized())
        .collect();
    let candidate_ids: Vec<String> = candidates.iter().map(|op| op.op_id.clone()).collect();
    let (claimed, _claim_guard) = claim_memory_op_ids(project_root, &candidate_ids);
    let fresh_by_id = revalidation_snapshot(project_root, &claimed).await;
    let mut applied = 0usize;
    let mut failed = 0usize;
    let mut lost_race = false;
    let mut processed: Vec<(MemoryLifecycleOp, MemoryLifecycleOp)> = Vec::new();
    for original in candidates {
        if !claimed.contains(&original.op_id) {
            lost_race = true;
            continue;
        }
        if fresh_by_id.get(&original.op_id) != Some(&original) {
            lost_race = true;
            continue;
        }
        let mut working = original.clone();
        working.status = MemoryOpStatus::Approved;
        working.error = None;
        let updated = apply_memory_lifecycle_op_status(gcx.clone(), &working)
            .await
            .normalized();
        if updated.status == MemoryOpStatus::Failed {
            failed += 1;
        } else {
            applied += 1;
        }
        processed.push((original, updated));
    }
    let state = if !processed.is_empty() {
        merge_processed_ops_and_rewrite(project_root, processed).await?
    } else if lost_race {
        snapshot_memory_ops(project_root).await
    } else {
        state
    };
    let remaining = state
        .ops
        .iter()
        .filter(|op| {
            refact_buddy_core::memory_lifecycle_model::memory_op_awaits_approval(op)
                && refact_buddy_core::memory_lifecycle_model::memory_op_batch_key(op) == batch_key
        })
        .count();
    Ok((state, applied, failed, remaining))
}

#[cfg_attr(not(test), allow(dead_code))]
pub async fn apply_artifact_decisions(
    project_root: &Path,
    gcx: AppState,
    decisions: &[(String, bool)],
) -> Result<(MemoryOpsState, usize, usize), String> {
    let state = snapshot_memory_ops(project_root).await;
    let mut failed = 0usize;
    let mut seen: HashSet<String> = HashSet::new();
    let mut candidates: Vec<(MemoryLifecycleOp, bool)> = Vec::new();
    for (op_id, accept) in decisions {
        if !seen.insert(op_id.clone()) {
            continue;
        }
        let Some(op) = state.ops.iter().find(|op| op.op_id == *op_id) else {
            failed += 1;
            continue;
        };
        if memory_op_is_final_status(op.status) {
            continue;
        }
        candidates.push((op.clone().normalized(), *accept));
    }
    let candidate_ids: Vec<String> = candidates.iter().map(|(op, _)| op.op_id.clone()).collect();
    let (claimed, _claim_guard) = claim_memory_op_ids(project_root, &candidate_ids);
    let fresh_by_id = revalidation_snapshot(project_root, &claimed).await;
    let mut decided = 0usize;
    let mut lost_race = false;
    let mut processed: Vec<(MemoryLifecycleOp, MemoryLifecycleOp)> = Vec::new();
    for (original, accept) in candidates {
        if !claimed.contains(&original.op_id) {
            failed += 1;
            lost_race = true;
            continue;
        }
        if fresh_by_id.get(&original.op_id) != Some(&original) {
            lost_race = true;
            continue;
        }
        let updated = if accept {
            let mut working = original.clone();
            working.status = MemoryOpStatus::Approved;
            working.error = None;
            let updated = apply_memory_lifecycle_op_status(gcx.clone(), &working)
                .await
                .normalized();
            if updated.status == MemoryOpStatus::Failed {
                failed += 1;
            }
            updated
        } else {
            let mut updated = original.clone();
            updated.status = MemoryOpStatus::Rejected;
            updated.error = None;
            updated
        };
        if updated != original {
            decided += 1;
            processed.push((original, updated));
        }
    }
    if decided > 0 {
        let state = merge_processed_ops_and_rewrite(project_root, processed).await?;
        return Ok((state, decided, failed));
    }
    if lost_race {
        return Ok((snapshot_memory_ops(project_root).await, decided, failed));
    }
    Ok((state, decided, failed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::memory_lifecycle::{
        MemoryLifecycleOp, MemoryOpStatus, MemoryOpType, MEMORY_OP_EVIDENCE_MAX_CHARS,
    };

    fn test_op(op_id: &str, evidence: &str, status: MemoryOpStatus) -> MemoryLifecycleOp {
        let mut op = MemoryLifecycleOp::pending(
            op_id,
            MemorySource::MemoryGarden,
            MemoryOpType::CreateMemory,
            vec![".refact/knowledge/item.md".to_string()],
            evidence,
            0.91,
            Utc::now().to_rfc3339(),
        );
        op.status = status;
        op
    }

    fn op_with_time(
        op_id: &str,
        source: MemorySource,
        status: MemoryOpStatus,
        created_at: DateTime<Utc>,
    ) -> MemoryLifecycleOp {
        let mut op = test_op(op_id, op_id, status);
        op.source = source;
        op.created_at = created_at.to_rfc3339();
        op.idempotency_key = format!("key-{op_id}");
        op
    }

    async fn write_memory_ops_records_for_test(root: &Path, ops: Vec<MemoryLifecycleOp>) {
        let records = ops
            .into_iter()
            .map(|op| MemoryOpsRecord::Op {
                op,
            })
            .collect::<Vec<_>>();
        rewrite_memory_ops_records(root, records).await.unwrap();
    }

    #[tokio::test]
    async fn gc_removes_only_matching_stale_tmp_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let buddy_dir = root.join(".refact/buddy");
        tokio::fs::create_dir_all(&buddy_dir).await.unwrap();
        let stale_tmp = buddy_dir.join("memory_ops.jsonl.aaaa-bbbb.tmp");
        let live_queue = buddy_dir.join("memory_ops.jsonl");
        let backup = buddy_dir.join("memory_ops.jsonl.bak");
        let unrelated_tmp = buddy_dir.join("other_file.tmp");
        for path in [&stale_tmp, &live_queue, &backup, &unrelated_tmp] {
            tokio::fs::write(path, "x").await.unwrap();
        }

        let removed_fresh = gc_stale_memory_ops_tmp_files(root).await;
        assert_eq!(removed_fresh, 0, "fresh tmp files must not be collected");
        assert!(stale_tmp.exists());

        let removed = gc_stale_memory_ops_tmp_files_with_max_age(root, 0).await;

        assert_eq!(removed, 1);
        assert!(!stale_tmp.exists());
        assert!(live_queue.exists());
        assert!(backup.exists());
        assert!(unrelated_tmp.exists());
    }

    #[tokio::test]
    async fn rewrite_failure_cleans_up_tmp_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let buddy_dir = root.join(".refact/buddy");
        tokio::fs::create_dir_all(&buddy_dir).await.unwrap();
        let path = buddy_dir.join("memory_ops.jsonl");
        tokio::fs::create_dir_all(path.join("occupied"))
            .await
            .unwrap();

        let result = rewrite_memory_ops_records(root, Vec::new()).await;

        assert!(result.is_err());
        let mut rd = tokio::fs::read_dir(&buddy_dir).await.unwrap();
        let mut leftover_tmp = Vec::new();
        while let Ok(Some(entry)) = rd.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("memory_ops.jsonl.") && name.ends_with(".tmp") {
                leftover_tmp.push(name);
            }
        }
        assert!(
            leftover_tmp.is_empty(),
            "failed rewrite must not leave tmp debris: {:?}",
            leftover_tmp
        );
    }

    #[tokio::test]
    async fn memory_ops_malformed_line_is_quarantined_and_removed_from_source() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = memory_ops_path(root);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        let valid = MemoryOpsRecord::Op {
            op: test_op("op-1", "first", MemoryOpStatus::Pending),
        };
        let content = format!(
            "not json\n{}\n{{\"kind\":\"op\",\"op\":\n",
            serde_json::to_string(&valid).unwrap()
        );
        tokio::fs::write(&path, content).await.unwrap();

        let state = load_memory_ops_repairing(root).await;

        assert_eq!(state.ops.len(), 1);
        assert_eq!(state.ops[0].op_id, "op-1");
        assert_eq!(state.malformed_lines, 2);
        let repaired = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(repaired.lines().count(), 1);
        assert!(!repaired.contains("not json"));
        assert!(repaired.contains("op-1"));
        let bad_content = tokio::fs::read_to_string(memory_ops_bad_path(root))
            .await
            .unwrap();
        assert!(bad_content.contains("not json"));
        let bad_line_count = bad_content.lines().count();

        let reloaded = load_memory_ops(root).await;
        assert_eq!(reloaded.ops.len(), 1);
        assert_eq!(reloaded.malformed_lines, 0);
        let bad_content_second = tokio::fs::read_to_string(memory_ops_bad_path(root))
            .await
            .unwrap();
        assert_eq!(bad_content_second.lines().count(), bad_line_count);

        let compacted = compact_memory_ops(root).await.unwrap();
        assert_eq!(compacted.ops.len(), 1);
    }

    #[tokio::test]
    async fn memory_ops_enqueue_pending_duplicate_does_not_reopen_finalized_or_approved() {
        let statuses = [
            MemoryOpStatus::Applied,
            MemoryOpStatus::Rejected,
            MemoryOpStatus::Approved,
        ];
        for status in statuses {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            let first = test_op(
                &format!("op-{}-first", status.as_str()),
                status.as_str(),
                status,
            );
            let mut pending = test_op(
                &format!("op-{}-pending", status.as_str()),
                "new pending",
                MemoryOpStatus::Pending,
            );
            pending.idempotency_key = first.idempotency_key.clone();

            enqueue_memory_op(root, first.clone()).await.unwrap();
            let state = enqueue_memory_op(root, pending).await.unwrap();
            let content = tokio::fs::read_to_string(memory_ops_path(root))
                .await
                .unwrap();

            assert_eq!(state.ops, vec![first.normalized()]);
            assert_eq!(state.pending_count, 0);
            assert_eq!(content.lines().count(), 1);
        }
    }

    #[tokio::test]
    async fn memory_ops_compaction_leaves_latest_status_per_key() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let first = test_op("op-1", "same", MemoryOpStatus::Pending);
        let mut second = test_op("op-2", "same", MemoryOpStatus::Failed);
        second.idempotency_key = first.idempotency_key.clone();
        second.error = Some("apply failed".to_string());
        let third = test_op("op-3", "other", MemoryOpStatus::Applied);

        enqueue_memory_op(root, first).await.unwrap();
        enqueue_memory_op(root, second.clone()).await.unwrap();
        enqueue_memory_op(root, third.clone()).await.unwrap();
        let compacted = compact_memory_ops(root).await.unwrap();
        let replayed = load_memory_ops(root).await;
        let content = tokio::fs::read_to_string(memory_ops_path(root))
            .await
            .unwrap();

        assert_eq!(compacted.ops, vec![second.normalized(), third.normalized()]);
        assert_eq!(replayed.ops, compacted.ops);
        assert_eq!(content.lines().count(), 2);
        assert_eq!(replayed.failed_count, 1);
        assert_eq!(replayed.applied_count, 1);
    }

    #[tokio::test]
    async fn compact_memory_ops_applies_pending_ttl_and_preserves_git_final() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let old_pending = op_with_time(
            "op-pending-old",
            MemorySource::MemoryGarden,
            MemoryOpStatus::Pending,
            Utc::now() - chrono::Duration::days(40),
        );
        let fresh_pending = op_with_time(
            "op-pending-fresh",
            MemorySource::MemoryGarden,
            MemoryOpStatus::Pending,
            Utc::now(),
        );
        let old_git_applied = op_with_time(
            "op-git-applied-old",
            MemorySource::Git,
            MemoryOpStatus::Applied,
            Utc::now() - chrono::Duration::days(40),
        );
        write_memory_ops_records_for_test(
            root,
            vec![old_pending, fresh_pending.clone(), old_git_applied.clone()],
        )
        .await;

        let state = compact_memory_ops(root).await.unwrap();

        let mut ops = state.ops.clone();
        ops.sort_by(|a, b| a.op_id.cmp(&b.op_id));
        let mut expected = vec![fresh_pending.normalized(), old_git_applied.normalized()];
        expected.sort_by(|a, b| a.op_id.cmp(&b.op_id));
        assert_eq!(ops, expected);
        assert_eq!(state.pending_count, 1);
        assert_eq!(state.applied_count, 1);
    }

    #[tokio::test]
    async fn archive_memory_ops_renames_to_bak_when_oversized() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let keep = op_with_time(
            "op-keep",
            MemorySource::MemoryGarden,
            MemoryOpStatus::Pending,
            Utc::now() - chrono::Duration::days(29),
        );
        let drop = op_with_time(
            "op-drop",
            MemorySource::MemoryGarden,
            MemoryOpStatus::Applied,
            Utc::now() - chrono::Duration::days(30),
        );
        write_memory_ops_records_for_test(root, vec![keep.clone(), drop]).await;
        let before = tokio::fs::read_to_string(memory_ops_path(root))
            .await
            .unwrap();

        let archived = archive_memory_ops_if_oversized(root, 1).await.unwrap();

        assert!(archived);
        assert_eq!(
            tokio::fs::read_to_string(memory_ops_backup_path(root))
                .await
                .unwrap(),
            before
        );
        let state = load_memory_ops(root).await;
        assert_eq!(state.ops, vec![keep.normalized()]);
    }

    #[test]
    fn cap_automated_final_records_keeps_newest_and_all_audit_records() {
        let now = Utc::now();
        let mut records = Vec::new();
        for age_days in [1, 5, 3, 4, 2] {
            records.push(MemoryOpsRecord::Op {
                op: op_with_time(
                    &format!("op-auto-{age_days}"),
                    MemorySource::Trajectory,
                    MemoryOpStatus::Skipped,
                    now - chrono::Duration::days(age_days),
                ),
            });
        }
        records.push(MemoryOpsRecord::Op {
            op: op_with_time(
                "op-git-old",
                MemorySource::Git,
                MemoryOpStatus::Applied,
                now - chrono::Duration::days(9),
            ),
        });
        records.push(MemoryOpsRecord::Op {
            op: op_with_time(
                "op-auto-pending-old",
                MemorySource::Trajectory,
                MemoryOpStatus::Pending,
                now - chrono::Duration::days(9),
            ),
        });

        let capped = cap_automated_final_records(records, 2);

        let mut ids = capped
            .into_iter()
            .map(|record| record.into_op().op_id)
            .collect::<Vec<_>>();
        ids.sort();
        assert_eq!(
            ids,
            vec![
                "op-auto-1",
                "op-auto-2",
                "op-auto-pending-old",
                "op-git-old"
            ]
        );
    }

    #[tokio::test]
    async fn memory_ops_enqueue_replay_and_compact_sanitize_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let raw = format!(
            "password=secret token ghp_AbCdEfGhIj1234567890 {}",
            "x".repeat(MEMORY_OP_EVIDENCE_MAX_CHARS * 2)
        );

        let mut op = test_op("op-secret", &raw, MemoryOpStatus::Pending);
        op.evidence = raw;
        enqueue_memory_op(root, op).await.unwrap();

        let content = tokio::fs::read_to_string(memory_ops_path(root))
            .await
            .unwrap();
        assert!(!content.contains("password=secret"));
        assert!(!content.contains("ghp_AbCdEfGhIj1234567890"));
        assert!(content.len() < MEMORY_OP_EVIDENCE_MAX_CHARS * 3);

        let replayed = load_memory_ops(root).await;
        assert_eq!(replayed.ops.len(), 1);
        assert!(!replayed.ops[0].evidence.contains("password=secret"));
        assert!(!replayed.ops[0]
            .evidence
            .contains("ghp_AbCdEfGhIj1234567890"));
        assert!(replayed.ops[0].evidence.len() <= MEMORY_OP_EVIDENCE_MAX_CHARS);

        compact_memory_ops(root).await.unwrap();
        let compacted = tokio::fs::read_to_string(memory_ops_path(root))
            .await
            .unwrap();
        assert!(!compacted.contains("password=secret"));
        assert!(!compacted.contains("ghp_AbCdEfGhIj1234567890"));
    }

    #[tokio::test]
    async fn merge_processed_preserves_concurrent_appends() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let processed_src = test_op("op-processed", "will be applied", MemoryOpStatus::Pending);
        enqueue_memory_op(root, processed_src.clone())
            .await
            .unwrap();
        let expected = processed_src.normalized();
        let mut updated = expected.clone();
        updated.status = MemoryOpStatus::Applied;
        let appended = test_op("op-appended", "arrived mid apply", MemoryOpStatus::Pending);
        enqueue_memory_op(root, appended).await.unwrap();

        let state = merge_processed_ops_and_rewrite(root, vec![(expected, updated)])
            .await
            .unwrap();

        assert_eq!(state.ops.len(), 2);
        assert_eq!(
            state
                .ops
                .iter()
                .find(|op| op.op_id == "op-processed")
                .unwrap()
                .status,
            MemoryOpStatus::Applied
        );
        assert!(state.ops.iter().any(|op| op.op_id == "op-appended"));
        let reloaded = load_memory_ops(root).await;
        assert_eq!(reloaded.ops.len(), 2);
    }

    #[tokio::test]
    async fn merge_does_not_regress_concurrently_changed_op() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let original = test_op("op-cas", "pending body", MemoryOpStatus::Pending);
        enqueue_memory_op(root, original.clone()).await.unwrap();
        let expected = original.normalized();

        let mut rejected = expected.clone();
        rejected.status = MemoryOpStatus::Rejected;
        write_memory_ops_records_for_test(root, vec![rejected.clone()]).await;

        let mut stale_applied = expected.clone();
        stale_applied.status = MemoryOpStatus::Applied;
        let state = merge_processed_ops_and_rewrite(root, vec![(expected, stale_applied)])
            .await
            .unwrap();

        assert_eq!(state.ops.len(), 1);
        assert_eq!(state.ops[0].status, MemoryOpStatus::Rejected);
        let reloaded = load_memory_ops(root).await;
        assert_eq!(reloaded.ops[0].status, MemoryOpStatus::Rejected);
    }

    #[tokio::test]
    async fn merge_does_not_resurrect_vanished_op() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let survivor = test_op("op-survivor", "stays", MemoryOpStatus::Pending);
        write_memory_ops_records_for_test(root, vec![survivor.clone()]).await;

        let vanished =
            test_op("op-vanished", "compacted away", MemoryOpStatus::Pending).normalized();
        let mut updated = vanished.clone();
        updated.status = MemoryOpStatus::Applied;
        let state = merge_processed_ops_and_rewrite(root, vec![(vanished, updated)])
            .await
            .unwrap();

        assert_eq!(state.ops.len(), 1);
        assert_eq!(state.ops[0].op_id, "op-survivor");
        let reloaded = load_memory_ops(root).await;
        assert_eq!(reloaded.ops.len(), 1);
        assert_eq!(reloaded.ops[0].op_id, "op-survivor");
    }

    #[test]
    fn claim_memory_op_ids_excludes_already_claimed() {
        let root = Path::new("/tmp/claims-test-root");
        let ids = vec!["claim-a".to_string(), "claim-b".to_string()];
        let (first, _first_guard) = claim_memory_op_ids(root, &ids);
        assert_eq!(first.len(), 2);

        let (second, _second_guard) = claim_memory_op_ids(root, &ids);
        assert!(second.is_empty());

        let other_root = Path::new("/tmp/claims-test-other-root");
        let (cross, _cross_guard) = claim_memory_op_ids(other_root, &ids);
        assert_eq!(
            cross.len(),
            2,
            "claims must be scoped per project root, not global by op_id"
        );

        drop(_first_guard);
        let (third, _third_guard) = claim_memory_op_ids(root, &ids);
        assert_eq!(third.len(), 2);
    }

    const CROSS_PROCESS_ROOT_ENV: &str = "REFACT_TEST_MEMORY_OPS_ROOT";
    const CROSS_PROCESS_COUNT_ENV: &str = "REFACT_TEST_MEMORY_OPS_COUNT";
    const CROSS_PROCESS_PREFIX_ENV: &str = "REFACT_TEST_MEMORY_OPS_PREFIX";

    fn pending_op(op_id: &str) -> MemoryLifecycleOp {
        op_with_time(
            op_id,
            MemorySource::Trajectory,
            MemoryOpStatus::Pending,
            Utc::now(),
        )
    }

    fn record_op_ids(root: &Path) -> Vec<String> {
        let content = std::fs::read_to_string(memory_ops_path(root)).unwrap_or_default();
        parse_memory_ops_content(&content)
            .records
            .into_iter()
            .map(|record| record.into_op().op_id)
            .collect()
    }

    #[tokio::test]
    async fn unreadable_queue_fails_closed_instead_of_persisting_empty_state() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let survivor = pending_op("op-survivor");
        write_memory_ops_records_for_test(root, vec![survivor.clone()]).await;
        let path = memory_ops_path(root);
        let saved = tokio::fs::read_to_string(&path).await.unwrap();

        tokio::fs::remove_file(&path).await.unwrap();
        tokio::fs::create_dir(&path).await.unwrap();

        let compact_err = compact_memory_ops(root).await.unwrap_err();
        assert!(
            compact_err.contains("Failed to read memory ops queue"),
            "unexpected compaction error: {compact_err}"
        );
        assert!(enqueue_memory_op(root, pending_op("op-new")).await.is_err());
        assert!(path.is_dir(), "failed read must not rewrite the queue");
        assert_eq!(load_memory_ops(root).await, MemoryOpsState::default());

        tokio::fs::remove_dir(&path).await.unwrap();
        tokio::fs::write(&path, saved).await.unwrap();
        assert_eq!(
            load_memory_ops_repairing(root).await.ops,
            vec![survivor.normalized()]
        );
    }

    fn cross_process_ready_marker(root: &Path) -> PathBuf {
        root.join("cross-process-parent-ready")
    }

    fn cross_process_done_marker(root: &Path) -> PathBuf {
        root.join("cross-process-parent-done")
    }

    #[tokio::test]
    async fn memory_ops_cross_process_appender_child() {
        let Ok(root) = std::env::var(CROSS_PROCESS_ROOT_ENV) else {
            return;
        };
        let cap: usize = std::env::var(CROSS_PROCESS_COUNT_ENV)
            .expect("child needs a record cap")
            .parse()
            .expect("record cap must be a number");
        let prefix = std::env::var(CROSS_PROCESS_PREFIX_ENV).expect("child needs an id prefix");
        let root = PathBuf::from(root);
        let ready = cross_process_ready_marker(&root);
        let done = cross_process_done_marker(&root);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);

        while !ready.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "parent never signalled that its compaction loop had started"
            );
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }

        let mut appended = 0usize;
        while appended < cap && !done.exists() {
            enqueue_memory_op(&root, pending_op(&format!("{prefix}-{appended}")))
                .await
                .expect("child append must succeed");
            appended += 1;
            assert!(
                std::time::Instant::now() < deadline,
                "parent never signalled that it had run enough compaction rounds"
            );
        }
        println!("APPENDED={appended}");
    }

    /// Regression guard for the cross-process fd lock around `memory_ops.jsonl`.
    ///
    /// A second process appends to the queue in a tight loop while this process
    /// runs the full read-modify-rewrite path (`enqueue` + `archive` + `compact`)
    /// in a tight loop of its own. Neither side sleeps, so the appender lands
    /// inside the rewriter's read/rename window constantly. Without the fd lock
    /// the rewriter renames a stale snapshot over the live queue and silently
    /// eats every record appended since it read, which shows up here as large
    /// contiguous runs of missing `child-*` ops.
    #[tokio::test]
    async fn cross_process_appends_survive_concurrent_compaction() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        write_memory_ops_records_for_test(&root, vec![pending_op("op-seed")]).await;

        // The child keeps appending until this process says stop, so overlap is
        // guaranteed for every round below instead of depending on how the two
        // processes happen to be scheduled. The cap only bounds the worst case.
        let child_cap = 8000usize;
        let min_parent_rounds = 25usize;
        // Safety valve: the queue is rewritten in full on every round, so cap the
        // rounds to keep the test's worst case bounded.
        let max_parent_rounds = 600usize;
        let suffix = module_path!()
            .split_once("::")
            .map(|(_, rest)| rest)
            .unwrap_or(module_path!());
        let child_test = format!("{suffix}::memory_ops_cross_process_appender_child");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &child_test,
                "--test-threads=1",
                "--nocapture",
                "--quiet",
            ])
            .env(CROSS_PROCESS_ROOT_ENV, &root)
            .env(CROSS_PROCESS_COUNT_ENV, child_cap.to_string())
            .env(CROSS_PROCESS_PREFIX_ENV, "child")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("failed to spawn cross-process appender");

        let ready_marker = cross_process_ready_marker(&root);
        let done_marker = cross_process_done_marker(&root);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        let mut parent_ids = Vec::new();
        let mut round = 0usize;
        std::fs::write(&ready_marker, b"go").unwrap();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                let _ = std::fs::write(&done_marker, b"stop");
                let _ = child.kill();
                let _ = child.wait();
                panic!("cross-process appender did not finish in 120s after {round} rounds");
            }
            if round >= max_parent_rounds {
                let _ = std::fs::write(&done_marker, b"stop");
                tokio::task::yield_now().await;
                continue;
            }
            let op_id = format!("parent-{round}");
            enqueue_memory_op(&root, pending_op(&op_id)).await.unwrap();
            parent_ids.push(op_id);
            // Defeat the "already compacted" growth heuristic so the archive path
            // (stage tmp -> back up -> rename) really runs on every single round.
            remember_compacted_len(&root, 0);
            archive_memory_ops_if_oversized(&root, 1).await.unwrap();
            compact_memory_ops(&root).await.unwrap();
            round += 1;
            // Release the child only once enough rounds have overlapped its
            // appends, so a loaded machine cannot quietly reduce this to a
            // handful of non-overlapping rounds.
            if round >= min_parent_rounds && !done_marker.exists() {
                std::fs::write(&done_marker, b"stop").unwrap();
            }
        };
        let output = child.wait_with_output().unwrap();
        assert!(
            status.success(),
            "child appender failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            round >= min_parent_rounds,
            "parent ran only {round} compaction rounds (needed {min_parent_rounds}); \
             the test would not reliably detect a missing lock"
        );
        let child_stdout = String::from_utf8_lossy(&output.stdout);
        let appended: usize = child_stdout
            .lines()
            .find_map(|line| line.trim().strip_prefix("APPENDED="))
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or_else(|| panic!("child did not report its append count: {child_stdout}"));
        assert!(
            appended > 0 && appended < child_cap,
            "child appended {appended} records (cap {child_cap}); it must append \
             continuously throughout the parent's rounds without hitting the cap"
        );

        let mut counts: HashMap<String, usize> = HashMap::new();
        for op_id in record_op_ids(&root) {
            *counts.entry(op_id).or_default() += 1;
        }
        let loaded: HashSet<String> = load_memory_ops_repairing(&root)
            .await
            .ops
            .into_iter()
            .map(|op| op.op_id)
            .collect();

        let mut expected: Vec<String> = vec!["op-seed".to_string()];
        expected.extend((0..appended).map(|index| format!("child-{index}")));
        expected.extend(parent_ids.iter().cloned());
        let lost: Vec<&String> = expected
            .iter()
            .filter(|op_id| {
                counts.get(*op_id).copied().unwrap_or(0) != 1 || !loaded.contains(*op_id)
            })
            .collect();
        let lost_children = lost
            .iter()
            .filter(|op_id| op_id.starts_with("child-"))
            .count();
        let lost_parents = lost
            .iter()
            .filter(|op_id| op_id.starts_with("parent-"))
            .count();
        assert!(
            lost.is_empty(),
            "{} of {} expected memory ops lost or duplicated across processes \
             ({lost_children} of {appended} child records, {lost_parents} of {} parent \
             records) after {round} parent compaction rounds; first missing: {:?}",
            lost.len(),
            expected.len(),
            parent_ids.len(),
            lost.iter().take(12).collect::<Vec<_>>()
        );

        let debris: Vec<String> = std::fs::read_dir(memory_ops_path(&root).parent().unwrap())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(
            debris.is_empty(),
            "staging files left behind after concurrent rewrites: {debris:?}"
        );
    }
}
