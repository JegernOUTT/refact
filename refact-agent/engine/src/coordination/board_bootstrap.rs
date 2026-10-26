use std::path::{Path, PathBuf};
use tokio::io::AsyncReadExt;
use crate::tasks::{index::TaskIndex, types::TaskBoard};
use super::board_index::BoardIndexFailure;
use super::board_rows::{BoardProvenance, rows_from_board};
use super::types::WorkRow;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SourceStamp {
    file: Option<(std::time::SystemTime, u64, bool, bool)>,
    parent: Option<(std::time::SystemTime, u64, bool, bool)>,
}

pub(super) async fn source_stamp(path: &Path) -> SourceStamp {
    async fn stamp(path: &Path) -> Option<(std::time::SystemTime, u64, bool, bool)> {
        let metadata = tokio::fs::symlink_metadata(path).await.ok()?;
        Some((
            metadata.modified().ok()?,
            metadata.len(),
            metadata.is_dir(),
            metadata.file_type().is_symlink(),
        ))
    }
    SourceStamp {
        file: stamp(path).await,
        parent: match path.parent() {
            Some(parent) => stamp(parent).await,
            None => None,
        },
    }
}

#[derive(Debug)]
pub(super) struct BootstrapFailure {
    pub reason: BoardIndexFailure,
    pub source: PathBuf,
    pub stamp: SourceStamp,
}

#[cfg(test)]
tokio::task_local! {
    pub(super) static BEFORE_FAILURE_RETURN: Box<dyn Fn(&BootstrapFailure) + Send + Sync>;
}

struct StampedSource {
    path: PathBuf,
    stamp: SourceStamp,
}

impl StampedSource {
    async fn capture(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            stamp: source_stamp(path).await,
        }
    }

    fn failure(&self, reason: BoardIndexFailure) -> BootstrapFailure {
        BootstrapFailure {
            reason,
            source: self.path.clone(),
            stamp: self.stamp.clone(),
        }
    }
}

pub(crate) async fn read_bounded(
    path: &Path,
    root: &Path,
    cap: usize,
) -> Result<String, BoardIndexFailure> {
    let canonical = tokio::fs::canonicalize(path)
        .await
        .map_err(|_| BoardIndexFailure::Read)?;
    if !dunce::simplified(&canonical).starts_with(root) {
        return Err(BoardIndexFailure::UnsafePath);
    }
    if !tokio::fs::metadata(&canonical)
        .await
        .map_err(|_| BoardIndexFailure::Read)?
        .is_file()
    {
        return Err(BoardIndexFailure::UnsafePath);
    }
    let file = tokio::fs::File::open(canonical)
        .await
        .map_err(|_| BoardIndexFailure::Read)?;
    if !file
        .metadata()
        .await
        .map_err(|_| BoardIndexFailure::Read)?
        .is_file()
    {
        return Err(BoardIndexFailure::UnsafePath);
    }
    let mut bytes = Vec::new();
    file.take(cap.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| BoardIndexFailure::Read)?;
    if bytes.len() > cap {
        return Err(BoardIndexFailure::ByteLimit);
    }
    String::from_utf8(bytes).map_err(|_| BoardIndexFailure::Parse)
}

async fn task_ids(
    tasks: &Path,
    root: &Path,
    cap: usize,
    bytes: &mut usize,
) -> Result<Vec<String>, BootstrapFailure> {
    let index_path = tasks.join("index.json");
    let index_source = StampedSource::capture(&index_path).await;
    match tokio::fs::symlink_metadata(&index_path).await {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(index_source.failure(BoardIndexFailure::Read)),
        Ok(_) => {
            let body = read_bounded(&index_path, root, *bytes)
                .await
                .map_err(|reason| index_source.failure(reason))?;
            *bytes = bytes.saturating_sub(body.len());
            let index = serde_json::from_str::<TaskIndex>(&body)
                .map_err(|_| index_source.failure(BoardIndexFailure::Parse))?;
            if index.schema_version != crate::tasks::index::TASK_INDEX_SCHEMA_VERSION {
                return Err(index_source.failure(BoardIndexFailure::Parse));
            }
            if index.entries.len() > cap {
                return Err(index_source.failure(BoardIndexFailure::RowLimit));
            }
            return index
                .entries
                .into_iter()
                .map(|entry| {
                    if entry.id != entry.dir_name
                        || crate::tasks::storage::validate_task_id(&entry.id).is_err()
                    {
                        return Err(index_source.failure(BoardIndexFailure::UnsafePath));
                    }
                    Ok(entry.id)
                })
                .collect();
        }
    }
    let directory_source = StampedSource::capture(tasks).await;
    let failure = |reason| directory_source.failure(reason);
    let mut directory = tokio::fs::read_dir(tasks)
        .await
        .map_err(|_| failure(BoardIndexFailure::Read))?;
    let mut ids = Vec::new();
    let mut visited = 0usize;
    while let Some(entry) = directory
        .next_entry()
        .await
        .map_err(|_| failure(BoardIndexFailure::Read))?
    {
        visited = visited
            .checked_add(1)
            .filter(|count| *count <= cap)
            .ok_or_else(|| failure(BoardIndexFailure::RowLimit))?;
        let kind = entry
            .file_type()
            .await
            .map_err(|_| failure(BoardIndexFailure::Read))?;
        if kind.is_symlink() {
            return Err(failure(BoardIndexFailure::UnsafePath));
        }
        if !kind.is_dir() {
            continue;
        }
        let id = entry
            .file_name()
            .into_string()
            .map_err(|_| failure(BoardIndexFailure::UnsafePath))?;
        if id == ".locks" {
            continue;
        }
        crate::tasks::storage::validate_task_id(&id)
            .map_err(|_| failure(BoardIndexFailure::UnsafePath))?;
        ids.push(id);
    }
    ids.sort();
    Ok(ids)
}

pub(super) async fn load(
    root: &Path,
) -> (
    Vec<(PathBuf, Option<Vec<WorkRow>>)>,
    Option<BootstrapFailure>,
) {
    match load_inner(root).await {
        Ok(rows) => (rows, None),
        Err(failure) => {
            #[cfg(test)]
            let _ = BEFORE_FAILURE_RETURN.try_with(|hook| hook(&failure));
            (vec![], Some(failure))
        }
    }
}

async fn load_inner(root: &Path) -> Result<Vec<(PathBuf, Option<Vec<WorkRow>>)>, BootstrapFailure> {
    let settings = crate::file_activity::settings();
    let tasks = root.join(".refact/tasks");
    let tasks_source = StampedSource::capture(&tasks).await;
    match tokio::fs::symlink_metadata(&tasks).await {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
        Ok(_) => return Err(tasks_source.failure(BoardIndexFailure::UnsafePath)),
        Err(_) => return Err(tasks_source.failure(BoardIndexFailure::Read)),
    }
    match tokio::fs::canonicalize(&tasks).await {
        Ok(path) if dunce::simplified(&path).starts_with(root) => {}
        _ => return Err(tasks_source.failure(BoardIndexFailure::UnsafePath)),
    }
    let mut remaining_bytes = settings.max_work_history_bytes;
    let ids = task_ids(
        &tasks,
        root,
        settings.max_work_history_rows,
        &mut remaining_bytes,
    )
    .await?;
    let mut loaded = vec![];
    let mut row_count = 0usize;
    let mut projection_bytes = 0usize;
    for id in ids {
        let task_dir = tasks.join(&id);
        let task_source = StampedSource::capture(&task_dir).await;
        match tokio::fs::symlink_metadata(&task_dir).await {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => return Err(task_source.failure(BoardIndexFailure::UnsafePath)),
            Err(_) => return Err(task_source.failure(BoardIndexFailure::Read)),
        }
        let task_dir = match tokio::fs::canonicalize(&task_dir).await {
            Ok(path) if dunce::simplified(&path).starts_with(root) => {
                dunce::simplified(&path).to_path_buf()
            }
            _ => return Err(task_source.failure(BoardIndexFailure::UnsafePath)),
        };
        let board_path = task_dir.join("board.yaml");
        let board_source = StampedSource::capture(&board_path).await;
        match tokio::fs::symlink_metadata(&board_path).await {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(board_source.failure(BoardIndexFailure::Read)),
            Ok(_) => {}
        }
        let body = read_bounded(&board_path, root, remaining_bytes)
            .await
            .map_err(|reason| board_source.failure(reason))?;
        remaining_bytes = remaining_bytes.saturating_sub(body.len());
        let value = serde_yaml::from_str::<serde_yaml::Value>(&body)
            .map_err(|_| board_source.failure(BoardIndexFailure::Parse))?;
        let board = serde_yaml::from_value::<TaskBoard>(value.clone())
            .map_err(|_| board_source.failure(BoardIndexFailure::Parse))?;
        let provenance = value
            .get("work_provenance")
            .and_then(|value| serde_yaml::from_value::<BoardProvenance>(value.clone()).ok())
            .unwrap_or_default();
        if board.cards.len() > settings.max_work_history_rows {
            return Err(board_source.failure(BoardIndexFailure::RowLimit));
        }
        let rows = rows_from_board(root, &id, &board, &provenance, &settings)
            .ok_or_else(|| board_source.failure(BoardIndexFailure::ProjectionLimit))?;
        row_count = row_count.saturating_add(rows.len());
        let bytes = super::bounds::serialized_size(
            &(
                &task_dir,
                rows.iter()
                    .map(|row| (row, &row.source_records))
                    .collect::<Vec<_>>(),
            ),
            settings
                .max_work_history_bytes
                .saturating_sub(projection_bytes),
        )
        .ok_or_else(|| board_source.failure(BoardIndexFailure::ByteLimit))?;
        projection_bytes = projection_bytes.saturating_add(bytes);
        if row_count > settings.max_work_history_rows {
            return Err(board_source.failure(BoardIndexFailure::RowLimit));
        }
        loaded.push((task_dir, Some(rows)));
    }
    Ok(loaded)
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
        assert_eq!(
            read_bounded(&path, root.path(), 3).await.unwrap_err(),
            BoardIndexFailure::ByteLimit
        );
        assert_eq!(read_bounded(&path, root.path(), 4).await.unwrap(), "abcd");
        tokio::fs::write(tasks.join("index.json"), "not-json")
            .await
            .unwrap();
        assert_eq!(
            task_ids(&tasks, root.path(), 10, &mut bytes)
                .await
                .unwrap_err()
                .reason,
            BoardIndexFailure::Parse
        );
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
        assert_eq!(
            load(root.path()).await.1.unwrap().reason,
            BoardIndexFailure::UnsafePath
        );
        assert_eq!(
            read_bounded(&tasks.join("task-link"), root.path(), 1024)
                .await
                .unwrap_err(),
            BoardIndexFailure::UnsafePath
        );
        let index = super::super::board_index::WorkBoardIndex::default();
        assert_eq!(
            index.snapshot(root.path()).await.health.reason,
            Some(BoardIndexFailure::UnsafePath)
        );
        assert!(index.rows(root.path()).await.1);
    }
}
