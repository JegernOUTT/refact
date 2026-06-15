use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;

use crate::chat::trajectories::{
    TrajectoryData, TrajectoryListCandidate, TrajectoryMeta, calculate_line_changes_from_messages,
    calculate_task_progress_from_messages, calculate_token_totals_from_messages,
    trajectory_list_data_is_displayable_chat, trajectory_meta_title,
};
use crate::chat::types::{TrajectorySourceIdentity, WorktreeMeta};

pub const TRAJECTORY_INDEX_SCHEMA_VERSION: u32 = 1;
pub const TRAJECTORY_INDEX_FILE: &str = "index.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TrajectoryIndex {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub entries: Vec<TrajectoryIndexEntry>,
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
    let key = trajectory_index_path(dir).to_string_lossy().to_string();
    let mut locks = get_trajectory_index_locks().lock().await;
    locks
        .entry(key)
        .or_insert_with(|| Arc::new(AMutex::new(())))
        .clone()
}

pub fn trajectory_index_path(dir: &Path) -> PathBuf {
    dir.join(TRAJECTORY_INDEX_FILE)
}

pub fn trajectory_file_path_for_entry(dir: &Path, entry: &TrajectoryIndexEntry) -> PathBuf {
    dir.join(&entry.file_name)
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
    _dir: &Path,
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
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("Trajectory path has no UTF-8 file name: {:?}", path))?
        .to_string();
    if file_name == TRAJECTORY_INDEX_FILE || !file_name.ends_with(".json") {
        return Err(format!("Not a trajectory JSON file: {:?}", path));
    }
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
    let path = trajectory_index_path(dir);
    let content = match fs::read_to_string(&path).await {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Failed to read trajectory index {:?}: {e}", path)),
    };
    let index = serde_json::from_str::<TrajectoryIndex>(&content)
        .map_err(|e| format!("Failed to parse trajectory index {:?}: {e}", path))?;
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
    fs::create_dir_all(dir)
        .await
        .map_err(|e| format!("Failed to create trajectory directory {:?}: {e}", dir))?;
    let path = trajectory_index_path(dir);
    let tmp_path = dir.join(format!(".{}.tmp-{}", TRAJECTORY_INDEX_FILE, Uuid::new_v4()));
    let content = serde_json::to_string_pretty(index)
        .map_err(|e| format!("Failed to serialize trajectory index {:?}: {e}", path))?;
    fs::write(&tmp_path, content).await.map_err(|e| {
        format!(
            "Failed to write temporary trajectory index {:?}: {e}",
            tmp_path
        )
    })?;
    crate::chat::trajectories::atomic_write_file(&tmp_path, &path).await
}

pub async fn trajectory_index_entry_is_fresh(dir: &Path, entry: &TrajectoryIndexEntry) -> bool {
    if entry.file_name == TRAJECTORY_INDEX_FILE
        || entry.file_name.contains('/')
        || entry.file_name.contains('\\')
        || !entry.file_name.ends_with(".json")
    {
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
    let lock = get_trajectory_index_lock(dir).await;
    let _guard = lock.lock().await;
    let mut index = match read_trajectory_index(dir).await {
        Ok(Some(index)) => index,
        Ok(None) | Err(_) => TrajectoryIndex {
            schema_version: TRAJECTORY_INDEX_SCHEMA_VERSION,
            updated_at: Utc::now().to_rfc3339(),
            entries: scan_trajectory_entries(dir, None).await?,
        },
    };
    index.entries.retain(|existing| existing.id != entry.id);
    index.entries.push(entry);
    index.updated_at = Utc::now().to_rfc3339();
    write_trajectory_index_atomic(dir, &index).await
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

pub async fn remove_trajectory_index_entry(dir: &Path, chat_id: &str) -> Result<(), String> {
    let lock = get_trajectory_index_lock(dir).await;
    let _guard = lock.lock().await;
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
    write_trajectory_index_atomic(dir, &index).await
}

async fn scan_trajectory_entries(
    dir: &Path,
    source_hint: Option<TrajectorySourceIdentity>,
) -> Result<Vec<TrajectoryIndexEntry>, String> {
    let mut indexed_entries = Vec::new();
    let mut entries = match fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(indexed_entries),
        Err(e) => {
            return Err(format!(
                "Failed to read trajectory directory {:?}: {e}",
                dir
            ))
        }
    };
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("Failed to iterate trajectory directory {:?}: {e}", dir))?
    {
        let path = entry.path();
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if file_name == TRAJECTORY_INDEX_FILE
            || file_name.starts_with('.')
            || path.extension().and_then(|e| e.to_str()) != Some("json")
        {
            continue;
        }
        let metadata = match entry.metadata().await {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            continue;
        }
        let content = match fs::read_to_string(&path).await {
            Ok(content) => content,
            Err(_) => continue,
        };
        let value = match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(value) => value,
            Err(e) => {
                tracing::warn!("Failed to parse trajectory {:?}: {}", path, e);
                continue;
            }
        };
        match entry_from_trajectory_value(dir, &path, &value, source_hint.clone()) {
            Ok(entry) => indexed_entries.push(entry),
            Err(e) => tracing::warn!("Failed to index trajectory {:?}: {}", path, e),
        }
    }
    Ok(indexed_entries)
}

struct DiskTrajectoryFile {
    file_name: String,
    file_len: u64,
    file_modified_unix_ms: i64,
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
        let mut files = Vec::new();
        for entry in read_dir {
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
            let metadata = match entry.metadata() {
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
            files.push(DiskTrajectoryFile {
                file_name,
                file_len: metadata.len(),
                file_modified_unix_ms,
            });
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
) -> Option<TrajectoryIndexEntry> {
    let content = fs::read_to_string(path).await.ok()?;
    let value = match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(value) => value,
        Err(e) => {
            tracing::warn!("Failed to parse trajectory {:?}: {}", path, e);
            return None;
        }
    };
    match entry_from_trajectory_value(dir, path, &value, source_hint) {
        Ok(entry) => Some(entry),
        Err(e) => {
            tracing::debug!("Skipping non-indexable trajectory {:?}: {}", path, e);
            None
        }
    }
}

async fn persist_reconciled_trajectory_index(
    dir: &Path,
    entries: &[TrajectoryIndexEntry],
) -> Result<(), String> {
    let lock = get_trajectory_index_lock(dir).await;
    let _guard = lock.lock().await;
    let index = TrajectoryIndex {
        schema_version: TRAJECTORY_INDEX_SCHEMA_VERSION,
        updated_at: Utc::now().to_rfc3339(),
        entries: entries.to_vec(),
    };
    write_trajectory_index_atomic(dir, &index).await
}

pub async fn rebuild_trajectory_index_from_disk(
    dir: &Path,
    source_hint: Option<TrajectorySourceIdentity>,
) -> Result<Vec<TrajectoryIndexEntry>, String> {
    let lock = get_trajectory_index_lock(dir).await;
    let _guard = lock.lock().await;
    let entries = scan_trajectory_entries(dir, source_hint).await?;
    let index = TrajectoryIndex {
        schema_version: TRAJECTORY_INDEX_SCHEMA_VERSION,
        updated_at: Utc::now().to_rfc3339(),
        entries: entries.clone(),
    };
    write_trajectory_index_atomic(dir, &index).await?;
    Ok(entries)
}

pub async fn list_trajectory_entries_from_index_or_rebuild(
    dir: &Path,
    source_hint: Option<TrajectorySourceIdentity>,
) -> Result<Vec<TrajectoryIndexEntry>, String> {
    let disk_files = scan_trajectory_dir_files(dir).await?;

    let (existing_entries, index_unreadable) = match read_trajectory_index(dir).await {
        Ok(Some(index)) => (index.entries, false),
        Ok(None) => (Vec::new(), false),
        Err(_) => (Vec::new(), true),
    };

    let by_file: HashMap<String, TrajectoryIndexEntry> = existing_entries
        .into_iter()
        .map(|entry| (entry.file_name.clone(), entry))
        .collect();

    let mut new_entries: Vec<TrajectoryIndexEntry> = Vec::with_capacity(disk_files.len());
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
        let was_indexed = by_file.contains_key(&disk.file_name);
        let path = dir.join(&disk.file_name);
        match read_and_index_single_trajectory(dir, &path, source_hint.clone()).await {
            Some(entry) => {
                new_entries.push(entry);
                content_changed = true;
            }
            None => {
                if was_indexed {
                    content_changed = true;
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
            .any(|file_name| !disk_names.contains(file_name.as_str()));
    }

    if content_changed {
        persist_reconciled_trajectory_index(dir, &new_entries).await?;
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
    use serde_json::json;

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
}
