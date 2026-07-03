use std::collections::{BTreeMap, HashSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tracing::warn;

pub use refact_buddy_core::storage::*;

use super::memory_lifecycle::{
    apply_memory_lifecycle_op_status, memory_op_auto_apply_eligible,
    memory_op_duplicate_should_replace, MemoryLifecycleOp, MemoryOpStatus, MemoryOpsRecord,
    MemoryOpsState, MemorySource,
};
use super::settings::AutonomyLevel;
use crate::app_state::AppState;

const MEMORY_OPS_COMPACT_KEEP_DAYS: i64 = 7;
const MEMORY_OPS_PENDING_TTL_DAYS: i64 = 30;

static MEMORY_OPS_IO_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn memory_ops_path(project_root: &Path) -> PathBuf {
    project_root.join(".refact/buddy/memory_ops.jsonl")
}

fn memory_ops_backup_path(project_root: &Path) -> PathBuf {
    project_root.join(".refact/buddy/memory_ops.jsonl.bak")
}

fn memory_ops_bad_path(project_root: &Path) -> PathBuf {
    project_root.join(".refact/buddy/memory_ops.jsonl.bad")
}

pub(crate) async fn rewrite_memory_ops_records(
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
    let result = write_sync_rename(&tmp, path, &buf).await;
    if result.is_err() {
        let _ = fs::remove_file(&tmp).await;
    }
    result
}

async fn write_sync_rename(tmp: &Path, path: &Path, buf: &str) -> Result<(), String> {
    let mut file = fs::File::create(tmp)
        .await
        .map_err(|e| format!("Failed to create {:?}: {}", tmp, e))?;
    file.write_all(buf.as_bytes())
        .await
        .map_err(|e| format!("Failed to write {:?}: {}", tmp, e))?;
    file.sync_all()
        .await
        .map_err(|e| format!("Failed to fsync {:?}: {}", tmp, e))?;
    drop(file);
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)
            .await
            .map_err(|e| format!("Failed to remove existing file: {}", e))?;
    }
    fs::rename(tmp, path)
        .await
        .map_err(|e| format!("Failed to rename {:?} to {:?}: {}", tmp, path, e))
}

const MEMORY_OPS_TMP_GC_MAX_AGE_SECS: u64 = 3600;

pub async fn gc_stale_memory_ops_tmp_files(project_root: &Path) -> usize {
    gc_stale_memory_ops_tmp_files_with_max_age(project_root, MEMORY_OPS_TMP_GC_MAX_AGE_SECS).await
}

async fn gc_stale_memory_ops_tmp_files_with_max_age(
    project_root: &Path,
    max_age_secs: u64,
) -> usize {
    let dir = project_root.join(".refact/buddy");
    let Ok(mut rd) = fs::read_dir(&dir).await else {
        return 0;
    };
    let now = std::time::SystemTime::now();
    let mut removed = 0usize;
    while let Ok(Some(entry)) = rd.next_entry().await {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !(name.starts_with("memory_ops.jsonl.") && name.ends_with(".tmp")) {
            continue;
        }
        let Ok(metadata) = entry.metadata().await else {
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
        match fs::remove_file(&path).await {
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

fn memory_op_survives_compaction(op: &MemoryLifecycleOp, now: DateTime<Utc>) -> bool {
    if !memory_op_is_final_status(op.status) {
        let cutoff = now - chrono::Duration::days(MEMORY_OPS_PENDING_TTL_DAYS);
        return memory_op_timestamp(op) >= cutoff;
    }
    if op.source != MemorySource::MemoryGarden {
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

struct MemoryOpsFileRead {
    records: Vec<MemoryOpsRecord>,
    malformed: Vec<(usize, String, String)>,
}

async fn read_memory_ops_file(project_root: &Path) -> MemoryOpsFileRead {
    let path = memory_ops_path(project_root);
    let content = match fs::read_to_string(&path).await {
        Ok(content) => content,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            return MemoryOpsFileRead {
                records: Vec::new(),
                malformed: Vec::new(),
            }
        }
        Err(err) => {
            warn!(
                "buddy: failed to read memory ops queue at {:?}: {}, starting empty",
                path, err
            );
            return MemoryOpsFileRead {
                records: Vec::new(),
                malformed: Vec::new(),
            };
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
    MemoryOpsFileRead { records, malformed }
}

async fn read_memory_ops_records_locked(project_root: &Path) -> (Vec<MemoryOpsRecord>, u32) {
    let path = memory_ops_path(project_root);
    let read = read_memory_ops_file(project_root).await;
    let malformed_lines = read.malformed.len().min(u32::MAX as usize) as u32;
    if malformed_lines > 0 {
        warn!(
            "buddy: quarantining {} malformed memory ops queue line(s) from {:?}",
            malformed_lines, path
        );
        match quarantine_memory_ops_bad_lines(project_root, &path, &read.malformed).await {
            Ok(()) => {
                if let Err(err) = rewrite_memory_ops_records(&path, read.records.clone()).await {
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
    (read.records, malformed_lines)
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

pub async fn enqueue_memory_op(
    project_root: &Path,
    op: MemoryLifecycleOp,
) -> Result<MemoryOpsState, String> {
    let _guard = MEMORY_OPS_IO_LOCK.lock().await;
    let incoming_has_key = !op.idempotency_key.trim().is_empty();
    let (records, malformed_lines) = read_memory_ops_records_locked(project_root).await;
    let current = MemoryOpsState::from_records_with_malformed(records, malformed_lines);
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
    let read = read_memory_ops_file(project_root).await;
    let malformed_lines = read.malformed.len().min(u32::MAX as usize) as u32;
    MemoryOpsState::from_records_with_malformed(read.records, malformed_lines)
}

pub async fn load_memory_ops_repairing(project_root: &Path) -> MemoryOpsState {
    let _guard = MEMORY_OPS_IO_LOCK.lock().await;
    let (records, malformed_lines) = read_memory_ops_records_locked(project_root).await;
    MemoryOpsState::from_records_with_malformed(records, malformed_lines)
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
    (claimed, MemoryOpClaimGuard { ids })
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
    let _guard = MEMORY_OPS_IO_LOCK.lock().await;
    let (records, malformed_lines) = read_memory_ops_records_locked(project_root).await;
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
        state.ops.into_iter().map(|op| MemoryOpsRecord::Op { op }),
        malformed_lines,
    );
    rewrite_memory_ops_records(&memory_ops_path(project_root), state.canonical_records()).await?;
    Ok(state)
}

pub async fn compact_memory_ops(project_root: &Path) -> Result<MemoryOpsState, String> {
    let _guard = MEMORY_OPS_IO_LOCK.lock().await;
    let (records, _) = read_memory_ops_records_locked(project_root).await;
    let state = compact_memory_ops_records(records, Utc::now());
    let path = memory_ops_path(project_root);
    rewrite_memory_ops_records(&path, state.canonical_records()).await?;
    Ok(state)
}

pub async fn archive_memory_ops_if_oversized(
    project_root: &Path,
    threshold_bytes: u64,
) -> Result<bool, String> {
    let _guard = MEMORY_OPS_IO_LOCK.lock().await;
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
    let (records, _) = read_memory_ops_records_locked(project_root).await;
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

pub async fn drain_memory_ops(
    project_root: &Path,
    gcx: AppState,
    autonomy: AutonomyLevel,
    max_applies: usize,
) -> Result<(MemoryOpsState, usize), String> {
    let state = snapshot_memory_ops(project_root).await;
    let mut candidates: Vec<(MemoryLifecycleOp, bool)> = Vec::new();
    for op in state.ops.iter() {
        if candidates.len() >= max_applies {
            break;
        }
        let auto_apply = op.status == MemoryOpStatus::Pending
            && op.requires_approval
            && autonomy == AutonomyLevel::SafeAuto
            && memory_op_auto_apply_eligible(op);
        let eligible = op.status == MemoryOpStatus::Approved
            || (op.status == MemoryOpStatus::Pending && !op.requires_approval)
            || auto_apply;
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
        MemoryCreatePayload, MemoryLifecycleOp, MemoryLifecyclePayload, MemoryOpStatus,
        MemoryOpType, MEMORY_OP_EVIDENCE_MAX_CHARS, MEMORY_OP_EXACT_DUPLICATE_REASON,
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

    async fn test_gcx_with_workspace(dir: &Path) -> AppState {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            *AppState::from_gcx(gcx.clone())
                .await
                .workspace
                .documents_state
                .workspace_folders
                .lock()
                .unwrap() = vec![dir.to_path_buf()];
        }
        AppState::from_gcx(gcx).await
    }

    async fn write_test_memory(root: &Path, name: &str) -> String {
        let dir = root.join(crate::file_filter::KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join(name);
        tokio::fs::write(
            &path,
            "---\ntitle: Test\ntags:\n- memory\n---\n\nExisting body\n",
        )
        .await
        .unwrap();
        path.to_string_lossy().to_string()
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
    async fn gc_missing_buddy_dir_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(gc_stale_memory_ops_tmp_files(dir.path()).await, 0);
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

        let result = rewrite_memory_ops_records(&path, Vec::new()).await;

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
            now - chrono::Duration::days(29),
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
        let (state, changed) = drain_memory_ops(
            root,
            AppState::from_gcx(gcx).await,
            AutonomyLevel::Suggest,
            100,
        )
        .await
        .unwrap();

        assert_eq!(state.ops.len(), 1);
        assert_eq!(state.ops[0].status, MemoryOpStatus::Pending);
        assert_eq!(state.ops[0].error, None);
        assert_eq!(changed, 0);
        assert_eq!(state.pending_count, 1);
        assert_eq!(state.failed_count, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn drain_applies_approved_mark_review_needed_op() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let target = write_test_memory(root, "review.md").await;
        let gcx = test_gcx_with_workspace(root).await;
        let mut op = test_op("op-review", "review", MemoryOpStatus::Approved);
        op.op_type = MemoryOpType::MarkReviewNeeded;
        op.requires_approval = false;
        op.target_paths = vec![target];
        op.payload = MemoryLifecyclePayload {
            review_after: Some("2026-05-02".to_string()),
            ..Default::default()
        };

        enqueue_memory_op(root, op).await.unwrap();
        let (state, changed) = drain_memory_ops(root, gcx, AutonomyLevel::Suggest, 100)
            .await
            .unwrap();

        assert_eq!(changed, 1);
        assert_eq!(state.ops.len(), 1);
        assert!(matches!(
            state.ops[0].status,
            MemoryOpStatus::Applied | MemoryOpStatus::Skipped
        ));
        assert_eq!(state.pending_count, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn drain_safe_auto_picks_exact_duplicate_but_suggest_does_not() {
        let suggest_dir = tempfile::tempdir().unwrap();
        let suggest_root = suggest_dir.path();
        let suggest_gcx = test_gcx_with_workspace(suggest_root).await;
        let mut op = test_op(
            "op-auto-merge",
            &format!(
                "{}: canonical=a, superseded=b",
                MEMORY_OP_EXACT_DUPLICATE_REASON
            ),
            MemoryOpStatus::Pending,
        );
        op.op_type = MemoryOpType::MergeArchive;
        op.requires_approval = true;
        op.confidence = 0.94;
        op.payload = MemoryLifecyclePayload {
            canonical: Some(MemoryCreatePayload {
                content: "Canonical merged body with plenty of alphabetic text.".to_string(),
                ..Default::default()
            }),
            superseded_paths: vec!["knowledge/missing.md".to_string()],
            ..Default::default()
        };
        enqueue_memory_op(suggest_root, op.clone()).await.unwrap();

        let (suggest_state, suggest_changed) =
            drain_memory_ops(suggest_root, suggest_gcx, AutonomyLevel::Suggest, 100)
                .await
                .unwrap();

        assert_eq!(suggest_changed, 0);
        assert_eq!(suggest_state.ops[0].status, MemoryOpStatus::Pending);

        let safe_dir = tempfile::tempdir().unwrap();
        let safe_root = safe_dir.path();
        let safe_gcx = test_gcx_with_workspace(safe_root).await;
        enqueue_memory_op(safe_root, op).await.unwrap();

        let (safe_state, safe_changed) =
            drain_memory_ops(safe_root, safe_gcx, AutonomyLevel::SafeAuto, 100)
                .await
                .unwrap();

        assert_eq!(safe_changed, 1);
        assert_ne!(safe_state.ops[0].status, MemoryOpStatus::Pending);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn apply_memory_batch_applies_only_matching_class() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let target = write_test_memory(root, "batchdoc.md").await;
        let gcx = test_gcx_with_workspace(root).await;

        let mut review_op = test_op("op-batch-review", "needs review", MemoryOpStatus::Pending);
        review_op.op_type = MemoryOpType::MarkReviewNeeded;
        review_op.requires_approval = true;
        review_op.target_paths = vec![target.clone()];
        review_op.payload = MemoryLifecyclePayload {
            review_after: Some("2026-05-02".to_string()),
            ..Default::default()
        };
        let mut archive_op = test_op("op-batch-archive", "stale doc", MemoryOpStatus::Pending);
        archive_op.op_type = MemoryOpType::ArchiveCandidate;
        archive_op.requires_approval = true;
        archive_op.target_paths = vec![target];

        enqueue_memory_op(root, review_op).await.unwrap();
        enqueue_memory_op(root, archive_op).await.unwrap();

        let (state, applied, failed, remaining) =
            apply_memory_batch(root, gcx, "review", 100).await.unwrap();

        assert_eq!(applied, 1);
        assert_eq!(failed, 0);
        assert_eq!(remaining, 0);
        let review = state
            .ops
            .iter()
            .find(|op| op.op_id == "op-batch-review")
            .unwrap();
        assert!(matches!(
            review.status,
            MemoryOpStatus::Applied | MemoryOpStatus::Skipped
        ));
        let archive = state
            .ops
            .iter()
            .find(|op| op.op_id == "op-batch-archive")
            .unwrap();
        assert_eq!(archive.status, MemoryOpStatus::Pending);

        let reloaded = load_memory_ops(root).await;
        assert_eq!(reloaded.pending_count, 1);
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
    async fn load_memory_ops_is_read_only_even_with_malformed_lines() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = memory_ops_path(root);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        let valid = MemoryOpsRecord::Op {
            op: test_op("op-1", "first", MemoryOpStatus::Pending),
        };
        let content = format!("not json\n{}\n", serde_json::to_string(&valid).unwrap());
        tokio::fs::write(&path, &content).await.unwrap();

        let state = load_memory_ops(root).await;

        assert_eq!(state.ops.len(), 1);
        assert_eq!(state.malformed_lines, 1);
        let after = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(after, content);
        assert!(!memory_ops_bad_path(root).exists());
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

        let vanished = test_op("op-vanished", "compacted away", MemoryOpStatus::Pending).normalized();
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

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_batch_and_decisions_apply_op_only_once() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let target = write_test_memory(root, "race.md").await;
        let gcx = test_gcx_with_workspace(root).await;
        let mut op = test_op("op-race", "needs review", MemoryOpStatus::Pending);
        op.op_type = MemoryOpType::MarkReviewNeeded;
        op.requires_approval = true;
        op.target_paths = vec![target];
        op.payload = MemoryLifecyclePayload {
            review_after: Some("2026-05-02".to_string()),
            ..Default::default()
        };
        enqueue_memory_op(root, op).await.unwrap();

        let root_a = root.to_path_buf();
        let gcx_a = gcx.clone();
        let batch = tokio::spawn(async move {
            apply_memory_batch(&root_a, gcx_a, "review", 100).await.unwrap()
        });
        let root_b = root.to_path_buf();
        let gcx_b = gcx.clone();
        let decisions = tokio::spawn(async move {
            apply_artifact_decisions(&root_b, gcx_b, &[("op-race".to_string(), true)])
                .await
                .unwrap()
        });
        let (_, batch_applied, batch_failed, _) = batch.await.unwrap();
        let (_, decided, _) = decisions.await.unwrap();

        assert!(
            batch_applied + batch_failed + decided <= 1,
            "op must be applied by at most one processor: batch={}/{} decisions={}",
            batch_applied,
            batch_failed,
            decided
        );
        let state = load_memory_ops(root).await;
        assert_eq!(state.ops.len(), 1);
    }
}
