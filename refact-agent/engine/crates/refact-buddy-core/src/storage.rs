use std::collections::VecDeque;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use chrono::Utc;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tracing::warn;

use crate::diagnostics::DiagnosticContext;
use crate::runtime_queue::RuntimeQueue;
use crate::settings::BuddySettings;
use crate::state::default_buddy_state;
use crate::types::BuddyRuntimeEvent;

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
    serde_json::from_str::<BuddyRuntimeEvent>(line)
        .ok()
        .map(|event| RuntimeQueueRecord::Event { event })
}

pub(crate) const DEFAULT_MAIN_PROMPT: &str = "You are the user's named project companion inside Refact.\nYou help with code tasks, project setup, diagnostics, and keeping things running smoothly.\nYou are friendly, concise, and focused on being genuinely useful.\n";

pub(crate) fn diagnostics_history_path(project_root: &Path) -> PathBuf {
    project_root.join(".refact/buddy/diagnostics.jsonl")
}

pub(crate) fn runtime_queue_path(project_root: &Path) -> PathBuf {
    project_root.join(".refact/buddy/runtime_queue.jsonl")
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
            warn!(
                "buddy: failed to compact runtime queue after load pruning: {}",
                err
            );
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

pub async fn atomic_write_text(path: &Path, content: &str) -> Result<(), String> {
    let tmp_path = path.with_extension("write.tmp");
    fs::write(&tmp_path, content)
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
        let settings = BuddySettings::default();
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
