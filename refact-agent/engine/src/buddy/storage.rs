use std::collections::{BTreeMap, HashSet, VecDeque};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tracing::warn;

use super::diagnostics::DiagnosticContext;
use super::memory_lifecycle::{
    apply_memory_lifecycle_op_status, memory_op_duplicate_should_replace, MemoryLifecycleOp,
    MemoryOpStatus, MemoryOpsRecord, MemoryOpsState, MemorySource,
};
use super::runtime_queue::RuntimeQueue;
use super::state::default_buddy_state;
use super::types::BuddyRuntimeEvent;
use crate::app_state::AppState;

/// Maximum number of items kept after replay+cap on load. Mirrors the in-memory cap.
const RUNTIME_QUEUE_MAX_ITEMS: usize = 100;
const MEMORY_OPS_COMPACT_KEEP_DAYS: i64 = 7;

/// One line of the runtime queue JSONL log. The tagged enum lets the writer
/// record more than just events: removals (eviction or coalescing wipeout) and
/// the current `now_playing` slot. Old log files that contain bare
/// `BuddyRuntimeEvent` JSON objects are still understood — see [`parse_record`].
///
/// Note: every variant uses **struct** form because `#[serde(tag = "kind")]`
/// does not support newtype variants that hold an `Option` (the `now_playing`
/// case), so for consistency all three are struct variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeQueueRecord {
    /// Insert/update an event by id.
    Event { event: BuddyRuntimeEvent },
    /// Tombstone — remove an event by id.
    Removed { id: String },
    /// Replace the `now_playing` slot. `event = None` clears it.
    NowPlaying { event: Option<BuddyRuntimeEvent> },
}

fn parse_record(line: &str) -> Option<RuntimeQueueRecord> {
    if let Ok(rec) = serde_json::from_str::<RuntimeQueueRecord>(line) {
        return Some(rec);
    }
    // Backward-compat: legacy lines were just a serialized BuddyRuntimeEvent.
    serde_json::from_str::<BuddyRuntimeEvent>(line)
        .ok()
        .map(|event| RuntimeQueueRecord::Event { event })
}

const DEFAULT_MAIN_PROMPT: &str = "You are the user's named project companion inside Refact.\nYou help with code tasks, project setup, diagnostics, and keeping things running smoothly.\nYou are friendly, concise, and focused on being genuinely useful.\n";

fn diagnostics_history_path(project_root: &Path) -> PathBuf {
    project_root.join(".refact/buddy/diagnostics.jsonl")
}

fn runtime_queue_path(project_root: &Path) -> PathBuf {
    project_root.join(".refact/buddy/runtime_queue.jsonl")
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

async fn rewrite_memory_ops_records(
    path: &Path,
    records: Vec<MemoryOpsRecord>,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create dir {:?}: {}", parent, e))?;
    }
    let mut buf = String::new();
    for record in records {
        let line = serde_json::to_string(&record)
            .map_err(|e| format!("Failed to serialize memory op record: {}", e))?;
        buf.push_str(&line);
        buf.push('\n');
    }
    let tmp = path.with_extension(format!("jsonl.{}.tmp", uuid::Uuid::new_v4()));
    fs::write(&tmp, &buf)
        .await
        .map_err(|e| format!("Failed to write {:?}: {}", tmp, e))?;
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)
            .await
            .map_err(|e| format!("Failed to remove existing file: {}", e))?;
    }
    fs::rename(&tmp, path)
        .await
        .map_err(|e| format!("Failed to rename {:?} to {:?}: {}", tmp, path, e))
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

fn memory_op_survives_compaction(op: &MemoryLifecycleOp, now: DateTime<Utc>) -> bool {
    if op.source != MemorySource::MemoryGarden {
        return true;
    }
    if !memory_op_is_final_status(op.status) {
        return true;
    }
    let cutoff = now - chrono::Duration::days(MEMORY_OPS_COMPACT_KEEP_DAYS);
    memory_op_timestamp(op) >= cutoff
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
            without_op_id.push(MemoryOpsRecord::Op { op });
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
    records.extend(by_op_id.into_values().map(|op| MemoryOpsRecord::Op { op }));
    MemoryOpsState::from_records(records)
}

async fn read_memory_ops_records(project_root: &Path) -> (Vec<MemoryOpsRecord>, u32) {
    let path = memory_ops_path(project_root);
    let content = match fs::read_to_string(&path).await {
        Ok(content) => content,
        Err(err) if err.kind() == ErrorKind::NotFound => return (Vec::new(), 0),
        Err(err) => {
            warn!(
                "buddy: failed to read memory ops queue at {:?}: {}, starting empty",
                path, err
            );
            return (Vec::new(), 0);
        }
    };

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
    let malformed_lines = malformed.len().min(u32::MAX as usize) as u32;
    if malformed_lines > 0 {
        warn!(
            "buddy: quarantining {} malformed memory ops queue line(s) from {:?}",
            malformed_lines, path
        );
        match quarantine_memory_ops_bad_lines(project_root, &path, &malformed).await {
            Ok(()) => {
                if let Err(err) = rewrite_memory_ops_records(&path, records.clone()).await {
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
    (records, malformed_lines)
}

fn memory_bad_line_hash(raw: &str, err: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    hasher.update(b"\0");
    hasher.update(err.as_bytes());
    hex::encode(hasher.finalize())
}

async fn existing_quarantine_hashes(path: &Path) -> HashSet<String> {
    let Ok(content) = fs::read_to_string(path).await else {
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

async fn quarantine_memory_ops_bad_lines(
    project_root: &Path,
    path: &Path,
    malformed: &[(usize, String, String)],
) -> Result<(), String> {
    if malformed.is_empty() {
        return Ok(());
    }
    let bad_path = memory_ops_bad_path(project_root);
    let existing_hashes = existing_quarantine_hashes(&bad_path).await;
    if let Some(parent) = bad_path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create dir {:?}: {}", parent, e))?;
    }
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
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&bad_path)
        .await
        .map_err(|e| format!("Failed to open memory ops quarantine {:?}: {}", bad_path, e))?;
    file.write_all(buf.as_bytes()).await.map_err(|e| {
        format!(
            "Failed to append memory ops quarantine {:?}: {}",
            bad_path, e
        )
    })?;
    file.flush().await.map_err(|e| {
        format!(
            "Failed to flush memory ops quarantine {:?}: {}",
            bad_path, e
        )
    })
}

/// Append-only persistence: every mutation writes one JSON record. The caller
/// must serialize concurrent calls — see the writer task in `actor.rs`. The
/// writer task is the single producer for this file in production code, so the
/// on-disk order matches the in-memory mutation order.
pub async fn append_runtime_record(
    project_root: &Path,
    record: &RuntimeQueueRecord,
) -> Result<(), String> {
    let path = runtime_queue_path(project_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create dir {:?}: {}", parent, e))?;
    }
    let line = format!(
        "{}\n",
        serde_json::to_string(record)
            .map_err(|e| format!("Failed to serialize runtime queue record: {}", e))?
    );
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
        .map_err(|e| format!("Failed to open runtime queue {:?}: {}", path, e))?;
    file.write_all(line.as_bytes())
        .await
        .map_err(|e| format!("Failed to append runtime queue {:?}: {}", path, e))?;
    file.flush()
        .await
        .map_err(|e| format!("Failed to flush runtime queue {:?}: {}", path, e))
}

/// Replay the JSONL log into a `RuntimeQueue`.
///
/// Records are applied in file order:
///   * `Event` inserts or updates by id, preserving first-seen position.
///   * `Removed { id }` removes that id from the queue (tombstone).
///   * `NowPlaying(opt)` overwrites `queue.now_playing`.
///
/// After replay, the queue is capped to `RUNTIME_QUEUE_MAX_ITEMS` from the
/// front (oldest first) as a defensive safety net for corrupted/imported logs;
/// in normal operation the writer emits explicit removals so this cap is a
/// no-op.
pub async fn load_runtime_queue(project_root: &Path) -> RuntimeQueue {
    let path = runtime_queue_path(project_root);
    let content = match fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(err) if err.kind() == ErrorKind::NotFound => return RuntimeQueue::new(),
        Err(err) => {
            warn!(
                "buddy: failed to read runtime queue at {:?}: {}, starting empty",
                path, err
            );
            return RuntimeQueue::new();
        }
    };

    // IndexMap preserves first-insertion order, which is the queue's logical order.
    let mut events: IndexMap<String, BuddyRuntimeEvent> = IndexMap::new();
    let mut now_playing: Option<BuddyRuntimeEvent> = None;

    for (idx, raw) in content.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        match parse_record(line) {
            Some(RuntimeQueueRecord::Event { event }) => {
                let id = event.id.clone();
                events.insert(id, event);
            }
            Some(RuntimeQueueRecord::Removed { id }) => {
                events.shift_remove(&id);
                if let Some(ref np) = now_playing {
                    if np.id == id {
                        now_playing = None;
                    }
                }
            }
            Some(RuntimeQueueRecord::NowPlaying { event }) => {
                now_playing = event;
            }
            None => {
                warn!(
                    "buddy: failed to parse runtime queue line {} in {:?}",
                    idx + 1,
                    path
                );
            }
        }
    }

    let mut queue = RuntimeQueue::new();
    let total = events.len();
    let skip = total.saturating_sub(RUNTIME_QUEUE_MAX_ITEMS);
    for (i, (_, ev)) in events.into_iter().enumerate() {
        if i < skip {
            continue;
        }
        queue.items.push_back(ev);
    }
    queue.now_playing = now_playing;
    let removed = queue.prune_expired_at(Utc::now());
    if skip > 0 || !removed.is_empty() {
        if let Err(err) = compact_runtime_queue(project_root, &queue).await {
            warn!("buddy: failed to compact runtime queue after load pruning: {}", err);
        }
    }
    queue
}

/// Rewrite the JSONL file from a canonical in-memory queue, dropping all
/// tombstones and stale Event records. Called periodically from the writer
/// task to keep the log bounded.
pub async fn compact_runtime_queue(
    project_root: &Path,
    queue: &RuntimeQueue,
) -> Result<(), String> {
    let path = runtime_queue_path(project_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create dir {:?}: {}", parent, e))?;
    }
    let mut buf = String::new();
    for ev in &queue.items {
        let rec = RuntimeQueueRecord::Event { event: ev.clone() };
        let line = serde_json::to_string(&rec)
            .map_err(|e| format!("Failed to serialize runtime queue record: {}", e))?;
        buf.push_str(&line);
        buf.push('\n');
    }
    if queue.now_playing.is_some() {
        let rec = RuntimeQueueRecord::NowPlaying {
            event: queue.now_playing.clone(),
        };
        let line = serde_json::to_string(&rec)
            .map_err(|e| format!("Failed to serialize runtime queue record: {}", e))?;
        buf.push_str(&line);
        buf.push('\n');
    }
    let tmp = path.with_extension("jsonl.tmp");
    fs::write(&tmp, &buf)
        .await
        .map_err(|e| format!("Failed to write {:?}: {}", tmp, e))?;
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(&path)
            .await
            .map_err(|e| format!("Failed to remove existing file: {}", e))?;
    }
    fs::rename(&tmp, &path)
        .await
        .map_err(|e| format!("Failed to rename {:?} to {:?}: {}", tmp, path, e))
}

#[allow(dead_code)]
pub async fn enqueue_memory_op(
    project_root: &Path,
    op: MemoryLifecycleOp,
) -> Result<MemoryOpsState, String> {
    let incoming_has_key = !op.idempotency_key.trim().is_empty();
    let current = load_memory_ops(project_root).await;
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
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create dir {:?}: {}", parent, e))?;
    }
    let record = MemoryOpsRecord::Op { op };
    let serialized = serde_json::to_string(&record)
        .map_err(|e| format!("Failed to serialize memory op record: {}", e))?;
    serde_json::from_str::<MemoryOpsRecord>(&serialized)
        .map_err(|e| format!("Serialized memory op record did not round-trip: {}", e))?;
    let line = format!("{}\n", serialized);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
        .map_err(|e| format!("Failed to open memory ops queue {:?}: {}", path, e))?;
    file.write_all(line.as_bytes())
        .await
        .map_err(|e| format!("Failed to append memory ops queue {:?}: {}", path, e))?;
    file.flush()
        .await
        .map_err(|e| format!("Failed to flush memory ops queue {:?}: {}", path, e))?;
    Ok(load_memory_ops(project_root).await)
}

pub async fn load_memory_ops(project_root: &Path) -> MemoryOpsState {
    let (records, malformed_lines) = read_memory_ops_records(project_root).await;
    MemoryOpsState::from_records_with_malformed(records, malformed_lines)
}

#[allow(dead_code)]
pub async fn compact_memory_ops(project_root: &Path) -> Result<MemoryOpsState, String> {
    let (records, _) = read_memory_ops_records(project_root).await;
    let state = compact_memory_ops_records(records, Utc::now());
    let path = memory_ops_path(project_root);
    rewrite_memory_ops_records(&path, state.canonical_records()).await?;
    Ok(state)
}

pub async fn archive_memory_ops_if_oversized(
    project_root: &Path,
    threshold_bytes: u64,
) -> Result<bool, String> {
    let path = memory_ops_path(project_root);
    let metadata = match fs::metadata(&path).await {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => {
            return Err(format!(
                "Failed to stat memory ops queue {:?}: {}",
                path, err
            ))
        }
    };
    if metadata.len() <= threshold_bytes {
        return Ok(false);
    }
    let (records, _) = read_memory_ops_records(project_root).await;
    let compacted = compact_memory_ops_records(records, Utc::now());
    let backup = memory_ops_backup_path(project_root);
    if fs::try_exists(&backup)
        .await
        .map_err(|e| format!("Failed to check backup {:?}: {}", backup, e))?
    {
        fs::remove_file(&backup)
            .await
            .map_err(|e| format!("Failed to remove existing backup {:?}: {}", backup, e))?;
    }
    fs::rename(&path, &backup)
        .await
        .map_err(|e| format!("Failed to rename {:?} to {:?}: {}", path, backup, e))?;
    rewrite_memory_ops_records(&path, compacted.canonical_records()).await?;
    Ok(true)
}

#[allow(dead_code)]
pub async fn apply_queued_memory_ops(
    project_root: &Path,
    gcx: AppState,
) -> Result<MemoryOpsState, String> {
    let state = load_memory_ops(project_root).await;
    for op in state.ops {
        if !matches!(
            op.status,
            MemoryOpStatus::Pending | MemoryOpStatus::Approved
        ) {
            continue;
        }
        let updated = apply_memory_lifecycle_op_status(gcx.clone(), &op).await;
        enqueue_memory_op(project_root, updated).await?;
    }
    Ok(load_memory_ops(project_root).await)
}

pub async fn atomic_write_json<T: Serialize>(path: &Path, data: &T) -> Result<(), String> {
    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_string(data).map_err(|e| e.to_string())?;
    fs::write(&tmp_path, &json)
        .await
        .map_err(|e| e.to_string())?;
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)
            .await
            .map_err(|e| format!("Failed to remove existing file: {}", e))?;
    }
    fs::rename(&tmp_path, path)
        .await
        .map_err(|e| format!("Failed to rename: {}", e))
}

pub async fn append_diagnostic(project_root: &Path, ctx: &DiagnosticContext) -> Result<(), String> {
    let path = diagnostics_history_path(project_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create dir {:?}: {}", parent, e))?;
    }

    let line = format!(
        "{}\n",
        serde_json::to_string(ctx).map_err(|e| format!("Failed to serialize diagnostic: {}", e))?
    );

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
        .map_err(|e| format!("Failed to open diagnostics history {:?}: {}", path, e))?;
    file.write_all(line.as_bytes())
        .await
        .map_err(|e| format!("Failed to append diagnostics history {:?}: {}", path, e))?;
    file.flush()
        .await
        .map_err(|e| format!("Failed to flush diagnostics history {:?}: {}", path, e))?;
    Ok(())
}

pub async fn load_diagnostics(project_root: &Path) -> Result<Vec<DiagnosticContext>, String> {
    Ok(load_diagnostics_inner(project_root, None)
        .await?
        .into_iter()
        .collect())
}

pub async fn load_recent_diagnostics(
    project_root: &Path,
    limit: usize,
) -> Result<Vec<DiagnosticContext>, String> {
    Ok(load_diagnostics_inner(project_root, Some(limit))
        .await?
        .into_iter()
        .collect())
}

async fn load_diagnostics_inner(
    project_root: &Path,
    limit: Option<usize>,
) -> Result<VecDeque<DiagnosticContext>, String> {
    let path = diagnostics_history_path(project_root);
    let content = match fs::read_to_string(&path).await {
        Ok(content) => content,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(VecDeque::new()),
        Err(err) => {
            return Err(format!(
                "Failed to read diagnostics history {:?}: {}",
                path, err
            ));
        }
    };

    let mut out = VecDeque::new();
    for (index, raw_line) in content.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        match serde_json::from_str::<DiagnosticContext>(line) {
            Ok(ctx) => {
                out.push_back(ctx);
                if let Some(limit) = limit {
                    while out.len() > limit {
                        out.pop_front();
                    }
                }
            }
            Err(err) => {
                warn!(
                    "buddy: failed to parse diagnostic history line {} in {:?}: {}",
                    index + 1,
                    path,
                    err
                );
            }
        }
    }
    Ok(out)
}

pub async fn bootstrap_buddy_storage(project_root: &Path) -> Result<(), String> {
    let buddy_dir = project_root.join(".refact/buddy");
    let dirs = [
        buddy_dir.clone(),
        buddy_dir.join("skills"),
        buddy_dir.join("chats/conversations"),
        buddy_dir.join("chats/workflows"),
    ];
    for dir in &dirs {
        fs::create_dir_all(dir)
            .await
            .map_err(|e| format!("Failed to create dir {:?}: {}", dir, e))?;
    }
    let state_path = buddy_dir.join("state.json");
    if !state_path.exists() {
        let state = default_buddy_state();
        atomic_write_json(&state_path, &state).await?;
    }
    let settings_path = buddy_dir.join("settings.json");
    if !settings_path.exists() {
        let settings = super::settings::BuddySettings::default();
        atomic_write_json(&settings_path, &settings).await?;
    }
    let prompt_path = buddy_dir.join("main_prompt.md");
    if !prompt_path.exists() {
        fs::write(&prompt_path, DEFAULT_MAIN_PROMPT)
            .await
            .map_err(|e| format!("Failed to write main_prompt.md: {}", e))?;
    }
    Ok(())
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

    fn legacy_test_op(op_id: &str, evidence: &str, status: MemoryOpStatus) -> MemoryLifecycleOp {
        let mut op = MemoryLifecycleOp::default();
        op.op_id = op_id.to_string();
        op.source = MemorySource::MemoryGarden;
        op.op_type = MemoryOpType::CreateMemory;
        op.target_paths = vec![".refact/knowledge/item.md".to_string()];
        op.evidence = evidence.to_string();
        op.confidence = 0.91;
        op.requires_approval = false;
        op.status = status;
        op.created_at = Utc::now().to_rfc3339();
        op
    }

    fn explicit_key_test_op(op_id: &str, key: &str, status: MemoryOpStatus) -> MemoryLifecycleOp {
        let mut op = test_op(op_id, key, status);
        op.idempotency_key = key.to_string();
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
            .map(|op| MemoryOpsRecord::Op { op })
            .collect::<Vec<_>>();
        rewrite_memory_ops_records(&memory_ops_path(root), records)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn memory_ops_enqueue_then_replay_preserves_order() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let first = test_op("op-1", "first", MemoryOpStatus::Pending);
        let second = test_op("op-2", "second", MemoryOpStatus::Approved);

        enqueue_memory_op(root, first.clone()).await.unwrap();
        enqueue_memory_op(root, second.clone()).await.unwrap();
        let state = load_memory_ops(root).await;

        assert_eq!(state.ops, vec![first.normalized(), second.normalized()]);
        assert_eq!(state.pending_count, 1);
        assert_eq!(state.approved_count, 1);
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

        let state = load_memory_ops(root).await;

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
    async fn memory_ops_duplicate_idempotency_key_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let first = test_op("op-1", "same", MemoryOpStatus::Pending);
        let mut second = test_op("op-2", "same", MemoryOpStatus::Applied);
        second.idempotency_key = first.idempotency_key.clone();
        second.applied_at = Some("2026-05-02T00:01:00Z".to_string());

        enqueue_memory_op(root, first).await.unwrap();
        enqueue_memory_op(root, second.clone()).await.unwrap();
        let state = load_memory_ops(root).await;

        assert_eq!(state.ops.len(), 1);
        assert_eq!(state.ops[0], second.normalized());
        assert_eq!(state.applied_count, 1);
    }

    #[tokio::test]
    async fn memory_ops_enqueue_same_idempotency_key_with_different_op_id_is_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let first = explicit_key_test_op("op-1", "semantic-key", MemoryOpStatus::Pending);
        let second = explicit_key_test_op("op-2", "semantic-key", MemoryOpStatus::Applied);

        enqueue_memory_op(root, first).await.unwrap();
        let state = enqueue_memory_op(root, second.clone()).await.unwrap();

        assert_eq!(state.ops, vec![second.normalized()]);
        assert_eq!(state.applied_count, 1);
    }

    #[tokio::test]
    async fn memory_ops_enqueue_missing_key_uses_legacy_op_id_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let first = legacy_test_op("op-legacy", "first", MemoryOpStatus::Pending);
        let second = legacy_test_op("op-legacy", "second", MemoryOpStatus::Applied);
        let expected = second.clone().normalized();

        enqueue_memory_op(root, first).await.unwrap();
        let state = enqueue_memory_op(root, second).await.unwrap();

        assert_eq!(state.ops, vec![expected]);
        assert_eq!(state.applied_count, 1);
    }

    #[tokio::test]
    async fn memory_ops_enqueue_different_keys_with_same_op_id_are_not_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let first = explicit_key_test_op("op-collide", "old-key", MemoryOpStatus::Applied);
        let second = explicit_key_test_op("op-collide", "new-key", MemoryOpStatus::Pending);

        enqueue_memory_op(root, first.clone()).await.unwrap();
        let state = enqueue_memory_op(root, second.clone()).await.unwrap();

        assert_eq!(state.ops, vec![first.normalized(), second.normalized()]);
        assert_eq!(state.applied_count, 1);
        assert_eq!(state.pending_count, 1);
    }

    #[tokio::test]
    async fn memory_ops_enqueue_existing_rejected_old_key_does_not_suppress_new_key() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let first = explicit_key_test_op("op-collide", "old-key", MemoryOpStatus::Rejected);
        let second = explicit_key_test_op("op-collide", "new-key", MemoryOpStatus::Pending);

        enqueue_memory_op(root, first.clone()).await.unwrap();
        let state = enqueue_memory_op(root, second.clone()).await.unwrap();

        assert_eq!(state.ops, vec![first.normalized(), second.normalized()]);
        assert_eq!(state.rejected_count, 1);
        assert_eq!(state.pending_count, 1);
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
    async fn memory_ops_enqueue_pending_duplicate_still_coalesces() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let first = test_op("op-pending-first", "same", MemoryOpStatus::Pending);
        let mut second = test_op("op-pending-second", "new pending", MemoryOpStatus::Pending);
        second.idempotency_key = first.idempotency_key.clone();

        enqueue_memory_op(root, first).await.unwrap();
        let state = enqueue_memory_op(root, second.clone()).await.unwrap();
        let content = tokio::fs::read_to_string(memory_ops_path(root))
            .await
            .unwrap();

        assert_eq!(state.ops, vec![second.normalized()]);
        assert_eq!(state.pending_count, 1);
        assert_eq!(content.lines().count(), 2);
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
    async fn compact_memory_ops_dedups_by_op_id_keeping_latest() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let now = Utc::now();
        let mut latest = op_with_time(
            "op-same",
            MemorySource::MemoryGarden,
            MemoryOpStatus::Pending,
            now,
        );
        latest.evidence = "latest".to_string();
        latest.idempotency_key = "latest-key".to_string();
        let mut older = op_with_time(
            "op-same",
            MemorySource::MemoryGarden,
            MemoryOpStatus::Pending,
            now - chrono::Duration::hours(1),
        );
        older.evidence = "older".to_string();
        older.idempotency_key = "older-key".to_string();
        write_memory_ops_records_for_test(root, vec![latest.clone(), older]).await;

        let state = compact_memory_ops(root).await.unwrap();

        assert_eq!(state.ops, vec![latest.normalized()]);
        let content = tokio::fs::read_to_string(memory_ops_path(root))
            .await
            .unwrap();
        assert_eq!(content.lines().count(), 1);
        assert!(content.contains("latest"));
    }

    #[tokio::test]
    async fn compact_memory_ops_preserves_pending_regardless_of_age() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let pending = op_with_time(
            "op-pending-old",
            MemorySource::MemoryGarden,
            MemoryOpStatus::Pending,
            Utc::now() - chrono::Duration::days(30),
        );
        write_memory_ops_records_for_test(root, vec![pending.clone()]).await;

        let state = compact_memory_ops(root).await.unwrap();

        assert_eq!(state.ops, vec![pending.normalized()]);
        assert_eq!(state.pending_count, 1);
    }

    #[tokio::test]
    async fn compact_memory_ops_drops_old_applied_garden_records() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let old = op_with_time(
            "op-old-applied",
            MemorySource::MemoryGarden,
            MemoryOpStatus::Applied,
            Utc::now() - chrono::Duration::days(30),
        );
        write_memory_ops_records_for_test(root, vec![old]).await;

        let state = compact_memory_ops(root).await.unwrap();

        assert!(state.is_empty());
        let content = tokio::fs::read_to_string(memory_ops_path(root))
            .await
            .unwrap();
        assert_eq!(content, "");
    }

    #[tokio::test]
    async fn compact_memory_ops_keeps_non_garden_op_types() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let op = op_with_time(
            "op-manual-old",
            MemorySource::Manual,
            MemoryOpStatus::Applied,
            Utc::now() - chrono::Duration::days(30),
        );
        write_memory_ops_records_for_test(root, vec![op.clone()]).await;

        let state = compact_memory_ops(root).await.unwrap();

        assert_eq!(state.ops, vec![op.normalized()]);
        assert_eq!(state.applied_count, 1);
    }

    #[tokio::test]
    async fn compact_memory_ops_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let now = Utc::now();
        let keep = op_with_time(
            "op-keep",
            MemorySource::MemoryGarden,
            MemoryOpStatus::Pending,
            now - chrono::Duration::days(30),
        );
        let drop = op_with_time(
            "op-drop",
            MemorySource::MemoryGarden,
            MemoryOpStatus::Skipped,
            now - chrono::Duration::days(30),
        );
        write_memory_ops_records_for_test(root, vec![keep.clone(), drop]).await;

        let first = compact_memory_ops(root).await.unwrap();
        let first_content = tokio::fs::read_to_string(memory_ops_path(root))
            .await
            .unwrap();
        let second = compact_memory_ops(root).await.unwrap();
        let second_content = tokio::fs::read_to_string(memory_ops_path(root))
            .await
            .unwrap();

        assert_eq!(first, second);
        assert_eq!(first_content, second_content);
        assert_eq!(second.ops, vec![keep.normalized()]);
    }

    #[tokio::test]
    async fn archive_memory_ops_renames_to_bak_when_oversized() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let keep = op_with_time(
            "op-keep",
            MemorySource::MemoryGarden,
            MemoryOpStatus::Pending,
            Utc::now() - chrono::Duration::days(30),
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

    #[tokio::test]
    async fn archive_memory_ops_no_op_when_under_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let op = op_with_time(
            "op-small",
            MemorySource::MemoryGarden,
            MemoryOpStatus::Pending,
            Utc::now(),
        );
        write_memory_ops_records_for_test(root, vec![op.clone()]).await;
        let size = tokio::fs::metadata(memory_ops_path(root))
            .await
            .unwrap()
            .len();

        let archived = archive_memory_ops_if_oversized(root, size + 1)
            .await
            .unwrap();

        assert!(!archived);
        assert!(!memory_ops_backup_path(root).exists());
        assert_eq!(load_memory_ops(root).await.ops, vec![op.normalized()]);
    }

    #[tokio::test]
    async fn memory_ops_missing_queue_is_empty_and_compactable() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let loaded = load_memory_ops(root).await;
        let compacted = compact_memory_ops(root).await.unwrap();

        assert!(loaded.is_empty());
        assert!(compacted.is_empty());
        assert_eq!(
            tokio::fs::read_to_string(memory_ops_path(root))
                .await
                .unwrap(),
            ""
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

    #[tokio::test(flavor = "multi_thread")]
    async fn pending_approval_required_queue_apply_keeps_op_pending() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let op = test_op("op-archive", "archive", MemoryOpStatus::Pending);
        let mut op = MemoryLifecycleOp {
            op_type: MemoryOpType::Archive,
            requires_approval: true,
            ..op
        };
        op.status = MemoryOpStatus::Pending;

        enqueue_memory_op(root, op.clone()).await.unwrap();
        let state = apply_queued_memory_ops(root, AppState::from_gcx(gcx).await)
            .await
            .unwrap();

        assert_eq!(state.ops.len(), 1);
        assert_eq!(state.ops[0].status, MemoryOpStatus::Pending);
        assert_eq!(state.ops[0].error, None);
        assert_eq!(state.pending_count, 1);
        assert_eq!(state.failed_count, 0);
    }
}
