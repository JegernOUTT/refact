use std::path::{Path, PathBuf};
use tokio::io::AsyncReadExt;
use crate::tasks::{index::TaskIndex, types::TaskBoard};
use super::board_rows::{BoardProvenance, rows_from_board};
use super::types::WorkRow;

pub(crate) async fn read_bounded(path: &Path, root: &Path, cap: usize) -> std::io::Result<String> {
    let invalid = || std::io::Error::other("Work source unavailable or over limit");
    let canonical = tokio::fs::canonicalize(path).await?;
    if !dunce::simplified(&canonical).starts_with(root) {
        return Err(invalid());
    }
    let file = tokio::fs::File::open(canonical).await?;
    if !file.metadata().await?.is_file() {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    file.take(cap.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > cap {
        return Err(invalid());
    }
    String::from_utf8(bytes).map_err(|_| invalid())
}

async fn task_ids(
    tasks: &Path,
    root: &Path,
    cap: usize,
    bytes: &mut usize,
) -> Result<Vec<String>, ()> {
    if let Ok(body) = read_bounded(&tasks.join("index.json"), root, *bytes).await {
        *bytes = bytes.saturating_sub(body.len());
        if let Ok(index) = serde_json::from_str::<TaskIndex>(&body) {
            if index.schema_version == crate::tasks::index::TASK_INDEX_SCHEMA_VERSION {
                if index.entries.len() > cap {
                    return Err(());
                }
                return index
                    .entries
                    .into_iter()
                    .map(|entry| {
                        if entry.id != entry.dir_name
                            || crate::tasks::storage::validate_task_id(&entry.id).is_err()
                        {
                            return Err(());
                        }
                        Ok(entry.id)
                    })
                    .collect();
            }
        }
    }
    let mut directory = tokio::fs::read_dir(tasks).await.map_err(|_| ())?;
    let mut ids = Vec::new();
    let mut visited = 0usize;
    while let Some(entry) = directory.next_entry().await.map_err(|_| ())? {
        visited = visited
            .checked_add(1)
            .filter(|count| *count <= cap)
            .ok_or(())?;
        let kind = entry.file_type().await.map_err(|_| ())?;
        if kind.is_symlink() {
            return Err(());
        }
        if !kind.is_dir() {
            continue;
        }
        let id = entry.file_name().into_string().map_err(|_| ())?;
        if id == ".locks" {
            continue;
        }
        crate::tasks::storage::validate_task_id(&id).map_err(|_| ())?;
        ids.push(id);
    }
    ids.sort();
    Ok(ids)
}

pub(super) async fn load(root: &Path) -> (Vec<(PathBuf, Option<Vec<WorkRow>>)>, bool) {
    let settings = crate::file_activity::settings();
    let tasks = root.join(".refact/tasks");
    match tokio::fs::symlink_metadata(&tasks).await {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return (vec![], false),
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
        _ => return (vec![], true),
    }
    let mut remaining_bytes = settings.max_work_history_bytes;
    let Ok(ids) = task_ids(
        &tasks,
        root,
        settings.max_work_history_rows,
        &mut remaining_bytes,
    )
    .await
    else {
        return (vec![], true);
    };
    let mut loaded = vec![];
    let mut row_count = 0usize;
    let mut projection_bytes = 0usize;
    for id in ids {
        let task_dir = tasks.join(&id);
        let Ok(meta) = tokio::fs::symlink_metadata(&task_dir).await else {
            continue;
        };
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return (vec![], true);
        }
        let task_dir = match tokio::fs::canonicalize(task_dir).await {
            Ok(path) if dunce::simplified(&path).starts_with(root) => {
                dunce::simplified(&path).to_path_buf()
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            _ => return (vec![], true),
        };
        if !task_dir.join("board.yaml").exists() {
            continue;
        }
        let body = match read_bounded(&task_dir.join("board.yaml"), root, remaining_bytes).await {
            Ok(body) => body,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return (vec![], true),
        };
        remaining_bytes = remaining_bytes.saturating_sub(body.len());
        let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(&body) else {
            return (vec![], true);
        };
        let Ok(board) = serde_yaml::from_value::<TaskBoard>(value.clone()) else {
            return (vec![], true);
        };
        let provenance = value
            .get("work_provenance")
            .and_then(|value| serde_yaml::from_value::<BoardProvenance>(value.clone()).ok())
            .unwrap_or_default();
        let Some(rows) = rows_from_board(root, &id, &board, &provenance, &settings) else {
            return (vec![], true);
        };
        row_count = row_count.saturating_add(rows.len());
        let Some(bytes) = super::bounds::serialized_size(
            &rows
                .iter()
                .map(|row| (row, &row.source_records))
                .collect::<Vec<_>>(),
            settings
                .max_work_history_bytes
                .saturating_sub(projection_bytes),
        ) else {
            return (vec![], true);
        };
        projection_bytes = projection_bytes.saturating_add(bytes);
        if row_count > settings.max_work_history_rows {
            return (vec![], true);
        }
        loaded.push((task_dir, Some(rows)));
    }
    (loaded, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_index_falls_back_once_with_explicit_scan_and_byte_bounds() {
        let root = tempfile::tempdir().unwrap();
        let tasks = root.path().join(".refact/tasks");
        tokio::fs::create_dir_all(tasks.join("task-one"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(tasks.join("task-two"))
            .await
            .unwrap();
        let mut bytes = 1024;
        assert!(task_ids(&tasks, root.path(), 1, &mut bytes).await.is_err());
        assert_eq!(
            task_ids(&tasks, root.path(), 2, &mut bytes)
                .await
                .unwrap()
                .len(),
            2
        );
        let path = tasks.join("large");
        tokio::fs::write(&path, "abcd").await.unwrap();
        assert!(read_bounded(&path, root.path(), 3).await.is_err());
        assert_eq!(read_bounded(&path, root.path(), 4).await.unwrap(), "abcd");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bootstrap_rejects_foreign_symlinks_without_reading_them() {
        let root = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        let tasks = root.path().join(".refact/tasks");
        tokio::fs::create_dir_all(&tasks).await.unwrap();
        std::os::unix::fs::symlink(foreign.path(), tasks.join("task-link")).unwrap();
        let mut bytes = 1024;
        assert!(task_ids(&tasks, root.path(), 10, &mut bytes).await.is_err());
        assert!(load(root.path()).await.1);
    }
}
