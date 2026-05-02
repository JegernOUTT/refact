use std::collections::VecDeque;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tracing::warn;

use super::diagnostics::DiagnosticContext;
use super::memory_lifecycle::{MemoryLifecycleOp, MemoryOpsRecord, MemoryOpsState};
use super::runtime_queue::RuntimeQueue;
use super::state::default_buddy_state;
use super::types::BuddyRuntimeEvent;

/// Maximum number of items kept after replay+cap on load. Mirrors the in-memory cap.
const RUNTIME_QUEUE_MAX_ITEMS: usize = 100;

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

const DEFAULT_MAIN_PROMPT: &str = "You are Buddy, a persistent project companion inside Refact.\nYou help with code tasks, project setup, diagnostics, and keeping things running smoothly.\nYou are friendly, concise, and focused on being genuinely useful.\n";

fn diagnostics_history_path(project_root: &Path) -> PathBuf {
    project_root.join(".refact/buddy/diagnostics.jsonl")
}

fn runtime_queue_path(project_root: &Path) -> PathBuf {
    project_root.join(".refact/buddy/runtime_queue.jsonl")
}

fn memory_ops_path(project_root: &Path) -> PathBuf {
    project_root.join(".refact/buddy/memory_ops.jsonl")
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
    let path = memory_ops_path(project_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create dir {:?}: {}", parent, e))?;
    }
    let record = MemoryOpsRecord::Op {
        op: op.normalized(),
    };
    let line = format!(
        "{}\n",
        serde_json::to_string(&record)
            .map_err(|e| format!("Failed to serialize memory op record: {}", e))?
    );
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
    let path = memory_ops_path(project_root);
    let content = match fs::read_to_string(&path).await {
        Ok(content) => content,
        Err(err) if err.kind() == ErrorKind::NotFound => return MemoryOpsState::default(),
        Err(err) => {
            warn!(
                "buddy: failed to read memory ops queue at {:?}: {}, starting empty",
                path, err
            );
            return MemoryOpsState::default();
        }
    };

    let mut records = Vec::new();
    let mut malformed_lines = 0u32;
    for (idx, raw) in content.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<MemoryOpsRecord>(line) {
            Ok(record) => records.push(record),
            Err(err) => {
                malformed_lines = malformed_lines.saturating_add(1);
                warn!(
                    "buddy: failed to parse memory ops queue line {} in {:?}: {}",
                    idx + 1,
                    path,
                    err
                );
            }
        }
    }
    MemoryOpsState::from_records_with_malformed(records, malformed_lines)
}

#[allow(dead_code)]
pub async fn compact_memory_ops(project_root: &Path) -> Result<MemoryOpsState, String> {
    let state = load_memory_ops(project_root).await;
    let path = memory_ops_path(project_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create dir {:?}: {}", parent, e))?;
    }

    let mut buf = String::new();
    for record in state.canonical_records() {
        let line = serde_json::to_string(&record)
            .map_err(|e| format!("Failed to serialize memory op record: {}", e))?;
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
        .map_err(|e| format!("Failed to rename {:?} to {:?}: {}", tmp, path, e))?;
    Ok(state)
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
        MemoryLifecycleOp, MemoryOpStatus, MemoryOpType, MemorySource,
    };

    fn test_op(op_id: &str, evidence: &str, status: MemoryOpStatus) -> MemoryLifecycleOp {
        let mut op = MemoryLifecycleOp::pending(
            op_id,
            MemorySource::MemoryGarden,
            MemoryOpType::CreateMemory,
            vec![".refact/knowledge/item.md".to_string()],
            evidence,
            0.91,
            "2026-05-02T00:00:00Z",
        );
        op.status = status;
        op
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
    async fn memory_ops_malformed_line_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = memory_ops_path(root);
        tokio::fs::create_dir_all(path.parent().unwrap()).await.unwrap();
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
        let content = tokio::fs::read_to_string(memory_ops_path(root)).await.unwrap();

        assert_eq!(compacted.ops, vec![second.normalized(), third.normalized()]);
        assert_eq!(replayed.ops, compacted.ops);
        assert_eq!(content.lines().count(), 2);
        assert_eq!(replayed.failed_count, 1);
        assert_eq!(replayed.applied_count, 1);
    }

    #[tokio::test]
    async fn memory_ops_missing_queue_is_empty_and_compactable() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let loaded = load_memory_ops(root).await;
        let compacted = compact_memory_ops(root).await.unwrap();

        assert!(loaded.is_empty());
        assert!(compacted.is_empty());
        assert_eq!(tokio::fs::read_to_string(memory_ops_path(root)).await.unwrap(), "");
    }
}
