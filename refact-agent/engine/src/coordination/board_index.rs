use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tokio::sync::Mutex as AsyncMutex;
use crate::tasks::types::TaskBoard;
use super::board_rows::{BoardProvenance, rows_from_board};
use super::types::WorkRow;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BoardIndexFailure {
    Read,
    Parse,
    UnsafePath,
    RowLimit,
    ByteLimit,
    ProjectionLimit,
    Capacity,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct BoardIndexHealth {
    pub coverage: &'static str,
    pub unavailable: bool,
    pub reason: Option<BoardIndexFailure>,
    pub retry: &'static str,
    pub retry_after_ms: Option<u64>,
}

pub(crate) struct BoardIndexSnapshot {
    pub rows: Vec<WorkRow>,
    pub health: BoardIndexHealth,
}

#[derive(Default)]
pub struct WorkBoardIndex {
    projects: Mutex<BTreeMap<PathBuf, Arc<Project>>>,
}

#[derive(Default)]
struct Project {
    bootstrap_gate: AsyncMutex<()>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    entries: BTreeMap<PathBuf, Entry>,
    epoch: u64,
    unavailable: bool,
    initialized: bool,
    bootstrapping: bool,
    attempted_limits: (usize, usize),
    failure: Option<super::board_bootstrap::BootstrapFailure>,
    retry_attempt: u32,
    retry_at: Option<std::time::Instant>,
    check_at: Option<std::time::Instant>,
}

struct Entry {
    epoch: u64,
    rows: Vec<WorkRow>,
    bytes: usize,
    failed: bool,
}

struct BootstrapGuard<'a> {
    project: &'a Project,
    committed: bool,
}

impl Drop for BootstrapGuard<'_> {
    fn drop(&mut self) {
        if !self.committed {
            let mut state = self.project.state.lock().unwrap_or_else(|e| e.into_inner());
            state.unavailable = true;
            state.initialized = false;
            state.bootstrapping = false;
        }
    }
}

impl WorkBoardIndex {
    fn project(&self, root: &Path) -> Option<Arc<Project>> {
        let mut projects = self.projects.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(project) = projects.get(root) {
            return Some(project.clone());
        }
        if projects.len() >= crate::file_activity::settings().max_work_views {
            return None;
        }
        let project = Arc::new(Project::default());
        projects.insert(root.into(), project.clone());
        Some(project)
    }

    pub(crate) fn publish(&self, task_dir: &Path, board: Option<(&TaskBoard, &BoardProvenance)>) {
        let Some(root) = project_root(task_dir) else {
            return;
        };
        let Some(project) = self.project(root) else {
            return;
        };
        let settings = crate::file_activity::settings();
        let rows = board
            .map(|(board, provenance)| {
                rows_from_board(
                    root,
                    task_dir.file_name()?.to_str()?,
                    board,
                    provenance,
                    &settings,
                )
            })
            .unwrap_or_else(|| Some(vec![]));
        let mut state = project.state.lock().unwrap_or_else(|e| e.into_inner());
        state.epoch = state.epoch.saturating_add(1);
        let epoch = state.epoch;
        if board.is_none() && state.initialized && !state.bootstrapping {
            state.entries.remove(task_dir);
            return;
        }
        state.insert(task_dir.to_path_buf(), rows, epoch, &settings);
    }

    pub async fn rows(&self, root: &Path) -> (Vec<WorkRow>, bool) {
        let snapshot = self.snapshot(root).await;
        (snapshot.rows, snapshot.health.unavailable)
    }

    pub(crate) async fn snapshot(&self, root: &Path) -> BoardIndexSnapshot {
        let Some(project) = self.project(root) else {
            return BoardIndexSnapshot {
                rows: vec![],
                health: BoardIndexHealth {
                    coverage: "partial",
                    unavailable: true,
                    reason: Some(BoardIndexFailure::Capacity),
                    retry: "capacity_change",
                    retry_after_ms: None,
                },
            };
        };
        let gate = project.bootstrap_gate.lock().await;
        let settings = crate::file_activity::settings();
        let limits = (
            settings.max_work_history_rows,
            settings.max_work_history_bytes,
        );
        let now = std::time::Instant::now();
        let source = {
            let state = project.state.lock().unwrap_or_else(|e| e.into_inner());
            state
                .failure
                .as_ref()
                .filter(|_| state.check_at.is_none_or(|at| now >= at))
                .map(|failure| (failure.source.clone(), failure.stamp.clone()))
        };
        let changed = match source {
            Some((path, stamp)) => {
                let changed = super::board_bootstrap::source_stamp(&path).await != stamp;
                project
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .check_at = Some(now + std::time::Duration::from_secs(1));
                changed
            }
            None => false,
        };
        let bootstrap = {
            let mut state = project.state.lock().unwrap_or_else(|e| e.into_inner());
            let retry = state.failure.as_ref().is_some_and(|failure| {
                changed
                    || (matches!(
                        failure.reason,
                        BoardIndexFailure::Read | BoardIndexFailure::Parse
                    ) && state.retry_at.is_some_and(|at| now >= at))
            });
            let needed = !state.initialized
                || (state.is_unavailable() && state.attempted_limits != limits)
                || retry;
            state.bootstrapping = needed;
            if needed {
                state
                    .entries
                    .retain(|_, entry| entry.epoch > 0 && !entry.failed);
                state.unavailable = false;
            }
            needed
        };
        if bootstrap {
            let mut guard = BootstrapGuard {
                project: &project,
                committed: false,
            };
            let (loaded, failure) = super::board_bootstrap::load(root).await;
            let mut state = project.state.lock().unwrap_or_else(|e| e.into_inner());
            state.bootstrap(loaded, failure.is_some(), &settings);
            state.record_failure(failure, std::time::Instant::now());
            state.attempted_limits = limits;
            state.initialized = true;
            state.bootstrapping = false;
            state
                .entries
                .retain(|_, entry| entry.failed || !entry.rows.is_empty());
            guard.committed = true;
        }
        drop(gate);
        let state = project.state.lock().unwrap_or_else(|e| e.into_inner());
        let settings = crate::file_activity::settings();
        let health = state.health(&settings, std::time::Instant::now());
        if state.is_unavailable() || !state.within_limits(&settings) {
            return BoardIndexSnapshot {
                rows: vec![],
                health,
            };
        }
        BoardIndexSnapshot {
            rows: state
                .entries
                .values()
                .flat_map(|entry| entry.rows.clone())
                .map(|mut row| {
                    if row.source_records.len() > settings.max_work_provenance_records
                        || row.source_records.iter().fold(0usize, |bytes, record| {
                            bytes
                                .saturating_add(record.path.len())
                                .saturating_add(record.zone.len())
                        }) > settings.max_work_provenance_bytes
                    {
                        row.source_records.clear();
                        row.provenance_complete = false;
                    }
                    row
                })
                .collect(),
            health,
        }
    }
}

impl State {
    fn record_failure(
        &mut self,
        failure: Option<super::board_bootstrap::BootstrapFailure>,
        now: std::time::Instant,
    ) {
        if failure.is_some() {
            self.retry_attempt = self.retry_attempt.saturating_add(1).min(7);
            let seconds = (1u64 << self.retry_attempt.saturating_sub(1)).min(60);
            self.retry_at = Some(now + std::time::Duration::from_secs(seconds));
            self.check_at = Some(now + std::time::Duration::from_secs(1));
        } else {
            self.retry_attempt = 0;
            self.retry_at = None;
            self.check_at = None;
        }
        self.failure = failure;
    }

    fn health(
        &self,
        settings: &refact_work_core::CoordinationSettings,
        now: std::time::Instant,
    ) -> BoardIndexHealth {
        let unavailable = self.is_unavailable() || !self.within_limits(settings);
        let reason = self
            .failure
            .as_ref()
            .map(|failure| failure.reason)
            .or_else(|| unavailable.then_some(BoardIndexFailure::ProjectionLimit));
        let timed = matches!(
            reason,
            Some(BoardIndexFailure::Read | BoardIndexFailure::Parse)
        );
        BoardIndexHealth {
            coverage: "partial",
            unavailable,
            reason,
            retry: if timed {
                "backoff_or_source_change"
            } else if unavailable {
                "source_or_limits_change"
            } else {
                "none"
            },
            retry_after_ms: timed
                .then(|| {
                    self.retry_at.map(|at| {
                        at.saturating_duration_since(now)
                            .as_millis()
                            .min(u64::MAX as u128) as u64
                    })
                })
                .flatten(),
        }
    }

    fn is_unavailable(&self) -> bool {
        self.unavailable || self.entries.values().any(|entry| entry.failed)
    }

    fn bootstrap(
        &mut self,
        loaded: Vec<(PathBuf, Option<Vec<WorkRow>>)>,
        unavailable: bool,
        settings: &refact_work_core::CoordinationSettings,
    ) {
        self.unavailable |= unavailable;
        for (path, rows) in loaded {
            if !self.entries.contains_key(&path) {
                self.insert(path, rows, 0, settings);
            }
        }
    }

    fn within_limits(&self, settings: &refact_work_core::CoordinationSettings) -> bool {
        self.entries.len() <= settings.max_work_history_rows
            && self.entries.values().fold(0usize, |count, entry| {
                count.saturating_add(entry.rows.len())
            }) <= settings.max_work_history_rows
            && self
                .entries
                .values()
                .fold(0usize, |bytes, entry| bytes.saturating_add(entry.bytes))
                <= settings.max_work_history_bytes
    }

    fn insert(
        &mut self,
        path: PathBuf,
        rows: Option<Vec<WorkRow>>,
        epoch: u64,
        settings: &refact_work_core::CoordinationSettings,
    ) {
        if self
            .entries
            .get(&path)
            .is_some_and(|entry| entry.epoch > epoch)
        {
            return;
        }
        let Some(rows) = rows else {
            if self.entries.contains_key(&path)
                || self.entries.len() < settings.max_work_history_rows
            {
                self.entries.insert(
                    path,
                    Entry {
                        epoch,
                        rows: vec![],
                        bytes: 0,
                        failed: true,
                    },
                );
            } else {
                self.unavailable = true;
            }
            return;
        };
        if !self.entries.contains_key(&path) && self.entries.len() >= settings.max_work_history_rows
        {
            self.unavailable = true;
            return;
        }
        let bytes = super::bounds::serialized_size(
            &(
                &path,
                rows.iter()
                    .map(|row| (row, &row.source_records))
                    .collect::<Vec<_>>(),
            ),
            settings.max_work_history_bytes,
        )
        .unwrap_or(usize::MAX);
        self.entries.insert(
            path.clone(),
            Entry {
                epoch,
                rows,
                bytes,
                failed: false,
            },
        );
        if !self.within_limits(settings) {
            self.entries.insert(
                path,
                Entry {
                    epoch,
                    rows: vec![],
                    bytes: 0,
                    failed: true,
                },
            );
        }
    }
}

pub(crate) fn project_root(task_dir: &Path) -> Option<&Path> {
    let tasks = task_dir.parent()?;
    let config = tasks.parent()?;
    (tasks.file_name()? == "tasks" && config.file_name()? == ".refact")
        .then(|| config.parent())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_work_core::CoordinationSettings;
    use crate::coordination::board_rows::tests::board;

    #[tokio::test]
    async fn cancelled_retry_bootstrap_stays_unavailable_and_retries_on_next_read() {
        let root = tempfile::tempdir().unwrap();
        let index = WorkBoardIndex::default();
        let project = index.project(root.path()).unwrap();
        {
            let mut state = project.state.lock().unwrap();
            state.initialized = true;
            state.unavailable = false;
            state.bootstrapping = true;
        }
        let mut suspended = Box::pin(async {
            let _guard = BootstrapGuard {
                project: &project,
                committed: false,
            };
            std::future::pending::<()>().await;
        });
        assert!(matches!(
            futures::poll!(suspended.as_mut()),
            std::task::Poll::Pending
        ));
        drop(suspended);
        {
            let state = project.state.lock().unwrap();
            assert!(state.unavailable);
            assert!(!state.initialized);
            assert!(!state.bootstrapping);
        }
        assert!(!index.rows(root.path()).await.1);
    }

    #[test]
    fn bootstrap_cannot_overwrite_publications_or_resurrect_deleted_tasks() {
        let settings = CoordinationSettings::default();
        let root = Path::new("/project");
        let path = root.join(".refact/tasks/task-a");
        let rows = rows_from_board(
            root,
            "task-a",
            &board(),
            &BoardProvenance::default(),
            &settings,
        )
        .unwrap();
        let mut state = State::default();
        state.insert(path.clone(), Some(rows.clone()), 1, &settings);
        state.bootstrap(vec![(path.clone(), Some(vec![]))], false, &settings);
        assert_eq!(state.entries[&path].rows.len(), 1);
        state.insert(path.clone(), Some(vec![]), 2, &settings);
        state.bootstrap(vec![(path.clone(), Some(rows.clone()))], false, &settings);
        state.insert(path.clone(), Some(rows), 1, &settings);
        assert!(state.entries[&path].rows.is_empty());
        assert_eq!(state.entries[&path].epoch, 2);
    }

    #[test]
    fn concurrent_over_limit_publication_cannot_be_replaced_with_old_bootstrap_rows() {
        let settings = CoordinationSettings::default();
        let path = PathBuf::from("/project/.refact/tasks/task-a");
        let rows = rows_from_board(
            Path::new("/project"),
            "task-a",
            &board(),
            &BoardProvenance::default(),
            &settings,
        )
        .unwrap();
        let mut state = State::default();
        state.insert(path.clone(), None, 1, &settings);
        state.bootstrap(vec![(path.clone(), Some(rows))], false, &settings);
        assert!(state.is_unavailable());
        assert!(state.entries[&path].failed);
        assert!(state.entries[&path].rows.is_empty());
    }

    #[test]
    fn aggregate_limits_fail_closed_and_live_reduction_never_discards_accepted_rows() {
        let settings = CoordinationSettings {
            max_work_history_rows: 1,
            ..Default::default()
        };
        let mut state = State::default();
        state.insert("/a".into(), Some(vec![]), 1, &settings);
        state.insert("/b".into(), Some(vec![]), 2, &settings);
        assert!(state.unavailable);
        assert_eq!(state.entries.len(), 1);
        let narrow = CoordinationSettings {
            max_work_history_bytes: 1,
            ..settings
        };
        assert!(!state.within_limits(&narrow));
        assert_eq!(state.entries.len(), 1);
    }

    #[tokio::test]
    async fn raising_limits_rebuilds_failed_bootstrap_and_completed_deletions_do_not_accumulate() {
        let root = tempfile::tempdir().unwrap();
        let index = WorkBoardIndex::default();
        let project = index.project(root.path()).unwrap();
        {
            let mut state = project.state.lock().unwrap();
            state.initialized = true;
            state.unavailable = true;
            state.attempted_limits = (0, 0);
        }
        assert_eq!(index.rows(root.path()).await.0.len(), 0);
        assert!(!index.rows(root.path()).await.1);
        for id in 0..100 {
            index.publish(&root.path().join(format!(".refact/tasks/task-{id}")), None);
        }
        assert!(project.state.lock().unwrap().entries.is_empty());
    }

    #[tokio::test]
    async fn deleting_or_repairing_failed_publication_restores_other_boards_without_restart() {
        let root = tempfile::tempdir().unwrap();
        let index = WorkBoardIndex::default();
        assert!(!index.rows(root.path()).await.1);
        let path = root.path().join(".refact/tasks/task-a");
        let project = index.project(root.path()).unwrap();
        let settings = crate::file_activity::settings();
        project
            .state
            .lock()
            .unwrap()
            .insert(path.clone(), None, 1, &settings);
        assert!(index.rows(root.path()).await.1);
        index.publish(&path, None);
        assert!(!index.rows(root.path()).await.1);
        project
            .state
            .lock()
            .unwrap()
            .insert(path.clone(), None, 2, &settings);
        index.publish(&path, Some((&board(), &BoardProvenance::default())));
        assert!(!index.rows(root.path()).await.1);
        assert_eq!(index.rows(root.path()).await.0.len(), 1);
    }

    #[tokio::test]
    async fn repaired_bootstrap_retries_with_backoff_or_source_change_and_stops_scanning_when_healthy(
    ) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(".refact/tasks/task-a");
        tokio::fs::create_dir_all(&path).await.unwrap();
        tokio::fs::write(path.join("board.yaml"), "cards: [")
            .await
            .unwrap();
        let index = WorkBoardIndex::default();
        let first = index.snapshot(root.path()).await;
        assert!(first.rows.is_empty());
        assert_eq!(first.health.reason, Some(BoardIndexFailure::Parse));
        assert_eq!(first.health.retry, "backoff_or_source_change");
        assert!(first.health.retry_after_ms.unwrap() <= 1000);
        let project = index.project(root.path()).unwrap();
        assert!(index.rows(root.path()).await.1);
        assert_eq!(project.state.lock().unwrap().retry_attempt, 1);
        for _ in 0..10 {
            project.state.lock().unwrap().retry_at = Some(std::time::Instant::now());
            assert!(index.rows(root.path()).await.1);
        }
        {
            let state = project.state.lock().unwrap();
            assert_eq!(state.retry_attempt, 7);
            assert!(
                state
                    .health(&crate::file_activity::settings(), std::time::Instant::now())
                    .retry_after_ms
                    .unwrap()
                    <= 60_000
            );
        }
        let board_path = path.join("board.yaml");
        tokio::fs::remove_file(&board_path).await.unwrap();
        tokio::fs::create_dir(&board_path).await.unwrap();
        let before = super::super::board_bootstrap::source_stamp(&board_path).await;
        let repaired_body = serde_yaml::to_string(&board()).unwrap();
        let repair_path = board_path.clone();
        project.state.lock().unwrap().retry_at = Some(std::time::Instant::now());
        let failed_during_repair = super::super::board_bootstrap::BEFORE_FAILURE_RETURN
            .scope(
                Box::new(move |failure| {
                    assert_eq!(failure.reason, BoardIndexFailure::UnsafePath);
                    assert_eq!(failure.source, repair_path);
                    std::fs::remove_dir(&repair_path).unwrap();
                    std::fs::write(&repair_path, &repaired_body).unwrap();
                }),
                index.snapshot(root.path()),
            )
            .await;
        assert_eq!(
            failed_during_repair.health.reason,
            Some(BoardIndexFailure::UnsafePath)
        );
        assert_eq!(failed_during_repair.health.retry, "source_or_limits_change");
        assert_eq!(failed_during_repair.health.retry_after_ms, None);
        assert_eq!(
            project
                .state
                .lock()
                .unwrap()
                .failure
                .as_ref()
                .unwrap()
                .stamp,
            before
        );
        assert_ne!(
            super::super::board_bootstrap::source_stamp(&board_path).await,
            before
        );
        project.state.lock().unwrap().retry_at = None;
        project.state.lock().unwrap().check_at = Some(std::time::Instant::now());
        let repaired = index.snapshot(root.path()).await;
        assert!(!repaired.health.unavailable);
        assert_eq!(repaired.rows.len(), 1);
        assert_eq!(repaired.health.reason, None);
        assert_eq!(repaired.health.retry, "none");
        tokio::fs::write(path.join("board.yaml"), "cards: [")
            .await
            .unwrap();
        let cached = index.snapshot(root.path()).await;
        assert!(!cached.health.unavailable);
        assert_eq!(cached.rows.len(), 1);
        assert_eq!(project.state.lock().unwrap().retry_attempt, 0);
        let health = serde_json::to_string(&first.health).unwrap();
        assert!(!health.contains("task-a"));
        assert!(!health.contains(&root.path().to_string_lossy().to_string()));
    }
}
