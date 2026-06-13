use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;

use super::types::TaskMeta;

pub const TASK_INDEX_SCHEMA_VERSION: u32 = 1;
pub const TASK_INDEX_FILE: &str = "index.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TaskIndex {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub entries: Vec<TaskIndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskIndexEntry {
    pub id: String,
    pub dir_name: String,
    pub meta: TaskMeta,
    #[serde(default)]
    pub meta_len: u64,
    #[serde(default)]
    pub meta_modified_unix_ms: i64,
}

fn default_schema_version() -> u32 {
    TASK_INDEX_SCHEMA_VERSION
}

static TASK_INDEX_LOCKS: std::sync::OnceLock<AMutex<HashMap<String, Arc<AMutex<()>>>>> =
    std::sync::OnceLock::new();

fn get_task_index_locks() -> &'static AMutex<HashMap<String, Arc<AMutex<()>>>> {
    TASK_INDEX_LOCKS.get_or_init(|| AMutex::new(HashMap::new()))
}

async fn get_task_index_lock(tasks_dir: &Path) -> Arc<AMutex<()>> {
    let key = task_index_path(tasks_dir).to_string_lossy().to_string();
    let mut locks = get_task_index_locks().lock().await;
    locks
        .entry(key)
        .or_insert_with(|| Arc::new(AMutex::new(())))
        .clone()
}

pub fn task_index_path(tasks_dir: &Path) -> PathBuf {
    tasks_dir.join(TASK_INDEX_FILE)
}

fn task_meta_path_for_entry(tasks_dir: &Path, entry: &TaskIndexEntry) -> PathBuf {
    tasks_dir.join(&entry.dir_name).join("meta.yaml")
}

fn task_meta_path_for_id(tasks_dir: &Path, task_id: &str) -> PathBuf {
    tasks_dir.join(task_id).join("meta.yaml")
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

async fn atomic_rename(tmp_path: &Path, dest_path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    if dest_path.exists() {
        fs::remove_file(dest_path)
            .await
            .map_err(|e| format!("Failed to remove existing file: {e}"))?;
    }
    fs::rename(tmp_path, dest_path)
        .await
        .map_err(|e| format!("Failed to rename task index: {e}"))
}

pub async fn read_task_index(tasks_dir: &Path) -> Result<Option<TaskIndex>, String> {
    let path = task_index_path(tasks_dir);
    let content = match fs::read_to_string(&path).await {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Failed to read task index {:?}: {e}", path)),
    };
    let index = serde_json::from_str::<TaskIndex>(&content)
        .map_err(|e| format!("Failed to parse task index {:?}: {e}", path))?;
    if index.schema_version != TASK_INDEX_SCHEMA_VERSION {
        return Err(format!(
            "Unsupported task index schema version {} in {:?}",
            index.schema_version, path
        ));
    }
    Ok(Some(index))
}

pub async fn write_task_index_atomic(tasks_dir: &Path, index: &TaskIndex) -> Result<(), String> {
    fs::create_dir_all(tasks_dir)
        .await
        .map_err(|e| format!("Failed to create tasks directory {:?}: {e}", tasks_dir))?;
    let path = task_index_path(tasks_dir);
    let tmp_path = tasks_dir.join(format!(".{}.tmp-{}", TASK_INDEX_FILE, Uuid::new_v4()));
    let content = serde_json::to_string_pretty(index)
        .map_err(|e| format!("Failed to serialize task index {:?}: {e}", path))?;
    fs::write(&tmp_path, content)
        .await
        .map_err(|e| format!("Failed to write temporary task index {:?}: {e}", tmp_path))?;
    atomic_rename(&tmp_path, &path).await
}

async fn task_index_entry_is_fresh(tasks_dir: &Path, entry: &TaskIndexEntry) -> bool {
    let meta_path = task_meta_path_for_entry(tasks_dir, entry);
    if entry.dir_name != entry.id || entry.dir_name.contains('/') || entry.dir_name.contains('\\') {
        return false;
    }
    let metadata = match fs::symlink_metadata(&meta_path).await {
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
    metadata.len() == entry.meta_len && modified_ms == entry.meta_modified_unix_ms
}

fn task_index_entry_from_meta(tasks_dir: &Path, meta: &TaskMeta) -> Result<TaskIndexEntry, String> {
    if meta.id.is_empty() || meta.id.contains('/') || meta.id.contains('\\') {
        return Err(format!("Invalid task id for index: {}", meta.id));
    }
    let meta_path = task_meta_path_for_id(tasks_dir, &meta.id);
    let metadata = std::fs::symlink_metadata(&meta_path)
        .map_err(|e| format!("Failed to read task meta metadata {:?}: {e}", meta_path))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "Task meta path is not a regular file: {:?}",
            meta_path
        ));
    }
    Ok(TaskIndexEntry {
        id: meta.id.clone(),
        dir_name: meta.id.clone(),
        meta: meta.clone(),
        meta_len: metadata.len(),
        meta_modified_unix_ms: unix_modified_ms(&metadata)?,
    })
}

fn meta_from_entry(entry: &TaskIndexEntry) -> TaskMeta {
    entry.meta.clone()
}

pub async fn upsert_task_index_entry(tasks_dir: &Path, meta: &TaskMeta) -> Result<(), String> {
    let lock = get_task_index_lock(tasks_dir).await;
    let _guard = lock.lock().await;
    let entry = task_index_entry_from_meta(tasks_dir, meta)?;
    let mut index = match read_task_index(tasks_dir).await {
        Ok(Some(index)) => index,
        Ok(None) | Err(_) => build_task_index(tasks_dir, &scan_task_metas(tasks_dir).await?),
    };
    index.entries.retain(|existing| existing.id != entry.id);
    index.entries.push(entry);
    index.updated_at = Utc::now().to_rfc3339();
    write_task_index_atomic(tasks_dir, &index).await
}

pub async fn remove_task_index_entry(tasks_dir: &Path, task_id: &str) -> Result<(), String> {
    let lock = get_task_index_lock(tasks_dir).await;
    let _guard = lock.lock().await;
    let mut index = match read_task_index(tasks_dir).await? {
        Some(index) => index,
        None => return Ok(()),
    };
    let before = index.entries.len();
    index.entries.retain(|entry| entry.id != task_id);
    if index.entries.len() == before {
        return Ok(());
    }
    index.updated_at = Utc::now().to_rfc3339();
    write_task_index_atomic(tasks_dir, &index).await
}

async fn scan_task_metas(tasks_dir: &Path) -> Result<Vec<TaskMeta>, String> {
    let mut metas = Vec::new();
    let mut entries = match fs::read_dir(tasks_dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(metas),
        Err(e) => {
            return Err(format!(
                "Failed to read tasks directory {:?}: {e}",
                tasks_dir
            ))
        }
    };
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("Failed to iterate tasks directory {:?}: {e}", tasks_dir))?
    {
        let path = entry.path();
        let metadata = match entry.metadata().await {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if !metadata.is_dir() {
            continue;
        }
        let meta_path = path.join("meta.yaml");
        let content = match fs::read_to_string(&meta_path).await {
            Ok(content) => content,
            Err(_) => continue,
        };
        match serde_yaml::from_str::<TaskMeta>(&content) {
            Ok(meta) => metas.push(meta),
            Err(e) => tracing::warn!("Failed to parse task meta {:?}: {}", meta_path, e),
        }
    }
    Ok(metas)
}

async fn task_ids_on_disk(tasks_dir: &Path) -> Result<HashSet<String>, String> {
    let mut ids = HashSet::new();
    let mut entries = match fs::read_dir(tasks_dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ids),
        Err(e) => {
            return Err(format!(
                "Failed to read tasks directory {:?}: {e}",
                tasks_dir
            ));
        }
    };
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("Failed to iterate tasks directory {:?}: {e}", tasks_dir))?
    {
        let metadata = match entry.metadata().await {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if !metadata.is_dir() {
            continue;
        }
        let Some(dir_name) = entry.file_name().to_str().map(ToString::to_string) else {
            continue;
        };
        if fs::symlink_metadata(tasks_dir.join(&dir_name).join("meta.yaml"))
            .await
            .ok()
            .is_some_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        {
            ids.insert(dir_name);
        }
    }
    Ok(ids)
}

fn build_task_index(tasks_dir: &Path, metas: &[TaskMeta]) -> TaskIndex {
    let mut entries = Vec::new();
    for meta in metas {
        match task_index_entry_from_meta(tasks_dir, meta) {
            Ok(entry) => entries.push(entry),
            Err(e) => tracing::warn!("Failed to index task {}: {}", meta.id, e),
        }
    }
    TaskIndex {
        schema_version: TASK_INDEX_SCHEMA_VERSION,
        updated_at: Utc::now().to_rfc3339(),
        entries,
    }
}

pub async fn rebuild_task_index_from_disk(tasks_dir: &Path) -> Result<Vec<TaskMeta>, String> {
    let lock = get_task_index_lock(tasks_dir).await;
    let _guard = lock.lock().await;
    let metas = scan_task_metas(tasks_dir).await?;
    let index = build_task_index(tasks_dir, &metas);
    write_task_index_atomic(tasks_dir, &index).await?;
    Ok(metas)
}

pub async fn refresh_task_index_from_metas(
    tasks_dir: &Path,
    metas: &[TaskMeta],
) -> Result<(), String> {
    let lock = get_task_index_lock(tasks_dir).await;
    let _guard = lock.lock().await;
    let index = build_task_index(tasks_dir, metas);
    write_task_index_atomic(tasks_dir, &index).await
}

pub async fn list_tasks_from_index_or_rebuild(tasks_dir: &Path) -> Result<Vec<TaskMeta>, String> {
    let index = match read_task_index(tasks_dir).await {
        Ok(Some(index)) => index,
        Ok(None) | Err(_) => return rebuild_task_index_from_disk(tasks_dir).await,
    };
    let disk_ids = task_ids_on_disk(tasks_dir).await?;
    let index_ids: HashSet<String> = index.entries.iter().map(|entry| entry.id.clone()).collect();
    if disk_ids != index_ids {
        return rebuild_task_index_from_disk(tasks_dir).await;
    }
    let mut metas = Vec::with_capacity(index.entries.len());
    for entry in &index.entries {
        if !task_index_entry_is_fresh(tasks_dir, entry).await {
            return rebuild_task_index_from_disk(tasks_dir).await;
        }
        metas.push(meta_from_entry(entry));
    }
    Ok(metas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::types::TaskStatus;

    fn task_meta(id: &str, name: &str, updated_at: &str) -> TaskMeta {
        TaskMeta {
            schema_version: 1,
            id: id.to_string(),
            name: name.to_string(),
            status: TaskStatus::Planning,
            created_at: updated_at.to_string(),
            updated_at: updated_at.to_string(),
            cards_total: 0,
            cards_done: 0,
            cards_failed: 0,
            agents_active: 0,
            base_branch: None,
            base_commit: None,
            default_agent_model: None,
            is_name_generated: false,
            last_agents_summary_at: None,
            planner_session_state: None,
        }
    }

    async fn write_meta(tasks_dir: &Path, meta: &TaskMeta) {
        let task_dir = tasks_dir.join(&meta.id);
        fs::create_dir_all(&task_dir).await.unwrap();
        let content = serde_yaml::to_string(meta).unwrap();
        fs::write(task_dir.join("meta.yaml"), content)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn missing_index_rebuilds_from_meta_files_and_creates_index() {
        let temp = tempfile::tempdir().unwrap();
        let tasks_dir = temp.path().join(".refact/tasks");
        write_meta(
            &tasks_dir,
            &task_meta("task-1", "One", "2026-01-01T00:00:00Z"),
        )
        .await;
        write_meta(
            &tasks_dir,
            &task_meta("task-2", "Two", "2026-01-02T00:00:00Z"),
        )
        .await;

        let metas = list_tasks_from_index_or_rebuild(&tasks_dir).await.unwrap();

        assert_eq!(metas.len(), 2);
        assert!(task_index_path(&tasks_dir).exists());
        let index = read_task_index(&tasks_dir).await.unwrap().unwrap();
        assert_eq!(index.entries.len(), 2);
    }

    #[tokio::test]
    async fn corrupt_index_rebuilds_from_disk() {
        let temp = tempfile::tempdir().unwrap();
        let tasks_dir = temp.path().join(".refact/tasks");
        fs::create_dir_all(&tasks_dir).await.unwrap();
        write_meta(
            &tasks_dir,
            &task_meta("task-1", "One", "2026-01-01T00:00:00Z"),
        )
        .await;
        fs::write(task_index_path(&tasks_dir), "not json")
            .await
            .unwrap();

        let metas = list_tasks_from_index_or_rebuild(&tasks_dir).await.unwrap();

        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].id, "task-1");
        let index = read_task_index(&tasks_dir).await.unwrap().unwrap();
        assert_eq!(index.entries.len(), 1);
    }

    #[tokio::test]
    async fn valid_index_rebuilds_when_new_task_appears_on_disk() {
        let temp = tempfile::tempdir().unwrap();
        let tasks_dir = temp.path().join(".refact/tasks");
        write_meta(
            &tasks_dir,
            &task_meta("task-1", "One", "2026-01-01T00:00:00Z"),
        )
        .await;
        assert_eq!(
            list_tasks_from_index_or_rebuild(&tasks_dir)
                .await
                .unwrap()
                .len(),
            1
        );

        write_meta(
            &tasks_dir,
            &task_meta("task-2", "Two", "2026-01-02T00:00:00Z"),
        )
        .await;
        let metas = list_tasks_from_index_or_rebuild(&tasks_dir).await.unwrap();

        assert_eq!(metas.len(), 2);
        let index = read_task_index(&tasks_dir).await.unwrap().unwrap();
        assert_eq!(index.entries.len(), 2);
    }

    #[tokio::test]
    async fn upsert_after_missing_index_preserves_existing_siblings() {
        let temp = tempfile::tempdir().unwrap();
        let tasks_dir = temp.path().join(".refact/tasks");
        let mut task_1 = task_meta("task-1", "One", "2026-01-01T00:00:00Z");
        let task_2 = task_meta("task-2", "Two", "2026-01-02T00:00:00Z");
        write_meta(&tasks_dir, &task_1).await;
        write_meta(&tasks_dir, &task_2).await;

        task_1.name = "Renamed".to_string();
        write_meta(&tasks_dir, &task_1).await;
        upsert_task_index_entry(&tasks_dir, &task_1).await.unwrap();
        let metas = list_tasks_from_index_or_rebuild(&tasks_dir).await.unwrap();

        assert_eq!(metas.len(), 2);
        assert!(metas.iter().any(|meta| meta.id == "task-2"));
    }

    #[tokio::test]
    async fn stale_entry_is_removed_when_task_directory_is_deleted() {
        let temp = tempfile::tempdir().unwrap();
        let tasks_dir = temp.path().join(".refact/tasks");
        let meta = task_meta("task-1", "One", "2026-01-01T00:00:00Z");
        write_meta(&tasks_dir, &meta).await;
        upsert_task_index_entry(&tasks_dir, &meta).await.unwrap();
        fs::remove_dir_all(tasks_dir.join("task-1")).await.unwrap();

        let metas = list_tasks_from_index_or_rebuild(&tasks_dir).await.unwrap();

        assert!(metas.is_empty());
        let index = read_task_index(&tasks_dir).await.unwrap().unwrap();
        assert!(index.entries.is_empty());
    }

    #[tokio::test]
    async fn upsert_updates_existing_task_without_duplicate_entries() {
        let temp = tempfile::tempdir().unwrap();
        let tasks_dir = temp.path().join(".refact/tasks");
        let mut meta = task_meta("task-1", "One", "2026-01-01T00:00:00Z");
        write_meta(&tasks_dir, &meta).await;
        upsert_task_index_entry(&tasks_dir, &meta).await.unwrap();

        meta.name = "Renamed".to_string();
        meta.status = TaskStatus::Active;
        meta.updated_at = "2026-01-02T00:00:00Z".to_string();
        write_meta(&tasks_dir, &meta).await;
        upsert_task_index_entry(&tasks_dir, &meta).await.unwrap();

        let index = read_task_index(&tasks_dir).await.unwrap().unwrap();
        assert_eq!(index.entries.len(), 1);
        assert_eq!(index.entries[0].meta.name, "Renamed");
        assert_eq!(index.entries[0].meta.status, TaskStatus::Active);
    }

    #[tokio::test]
    async fn index_json_is_ignored_during_rebuild() {
        let temp = tempfile::tempdir().unwrap();
        let tasks_dir = temp.path().join(".refact/tasks");
        fs::create_dir_all(&tasks_dir).await.unwrap();
        fs::write(
            task_index_path(&tasks_dir),
            serde_json::json!({"id":"not-a-task"}).to_string(),
        )
        .await
        .unwrap();

        let metas = rebuild_task_index_from_disk(&tasks_dir).await.unwrap();

        assert!(metas.is_empty());
        let index = read_task_index(&tasks_dir).await.unwrap().unwrap();
        assert!(index.entries.is_empty());
    }
}
