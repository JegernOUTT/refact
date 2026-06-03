use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use tokio::fs;
use uuid::Uuid;

use crate::conductor::GoalLedger;

#[derive(Debug)]
pub enum ConductorStoreError {
    InvalidGoalId(String),
    Io {
        action: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
}

impl fmt::Display for ConductorStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidGoalId(goal_id) => write!(f, "invalid conductor goal id: {goal_id}"),
            Self::Io {
                action,
                path,
                source,
            } => write!(f, "failed to {action} {:?}: {source}", path),
            Self::Json { path, source } => {
                write!(f, "failed to parse conductor ledger {:?}: {source}", path)
            }
        }
    }
}

impl std::error::Error for ConductorStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json { source, .. } => Some(source),
            Self::InvalidGoalId(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGoalLedger {
    pub goal_id: String,
    pub ledger: GoalLedger,
}

pub async fn save_goal_ledger(
    project_root: &Path,
    goal_id: &str,
    ledger: &GoalLedger,
) -> Result<(), ConductorStoreError> {
    let path = goal_ledger_path(project_root, goal_id)?;
    let dir = path.parent().unwrap_or(project_root);
    fs::create_dir_all(dir)
        .await
        .map_err(|source| io_error("create conductor directory", dir, source))?;
    let content =
        serde_json::to_string_pretty(ledger).map_err(|source| ConductorStoreError::Json {
            path: path.clone(),
            source,
        })?;
    let tmp_path = path.with_extension(format!("json.{}.tmp", Uuid::new_v4()));
    fs::write(&tmp_path, content)
        .await
        .map_err(|source| io_error("write conductor ledger temp file", &tmp_path, source))?;
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(&path)
            .await
            .map_err(|source| io_error("remove existing conductor ledger", &path, source))?;
    }
    if let Err(source) = fs::rename(&tmp_path, &path).await {
        let _ = fs::remove_file(&tmp_path).await;
        return Err(io_error("replace conductor ledger", &path, source));
    }
    Ok(())
}

pub async fn load_goal_ledger(
    project_root: &Path,
    goal_id: &str,
) -> Result<Option<GoalLedger>, ConductorStoreError> {
    let path = goal_ledger_path(project_root, goal_id)?;
    match read_goal_ledger_file(&path).await {
        Ok(ledger) => Ok(Some(ledger)),
        Err(ConductorStoreError::Io { source, .. }) if source.kind() == ErrorKind::NotFound => {
            Ok(None)
        }
        Err(err) => Err(err),
    }
}

pub async fn list_goal_ledgers(
    project_root: &Path,
) -> Result<Vec<StoredGoalLedger>, ConductorStoreError> {
    let dir = conductor_store_dir(project_root);
    let mut rd = match fs::read_dir(&dir).await {
        Ok(rd) => rd,
        Err(source) if source.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(io_error("read conductor directory", &dir, source)),
    };
    let mut entries = Vec::new();
    while let Some(entry) = rd
        .next_entry()
        .await
        .map_err(|source| io_error("read conductor directory entry", &dir, source))?
    {
        let path = entry.path();
        if !path.extension().map(|ext| ext == "json").unwrap_or(false) {
            continue;
        }
        let Some(goal_id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if validate_goal_id(goal_id).is_err() || !is_regular_file(&path).await? {
            continue;
        }
        entries.push(StoredGoalLedger {
            goal_id: goal_id.to_string(),
            ledger: read_goal_ledger_file(&path).await?,
        });
    }
    entries.sort_by(|left, right| left.goal_id.cmp(&right.goal_id));
    Ok(entries)
}

pub async fn remove_goal_ledger(
    project_root: &Path,
    goal_id: &str,
) -> Result<bool, ConductorStoreError> {
    let path = goal_ledger_path(project_root, goal_id)?;
    match fs::remove_file(&path).await {
        Ok(()) => Ok(true),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(false),
        Err(source) => Err(io_error("remove conductor ledger", &path, source)),
    }
}

fn conductor_store_dir(project_root: &Path) -> PathBuf {
    project_root.join(".refact/buddy/conductor")
}

fn goal_ledger_path(project_root: &Path, goal_id: &str) -> Result<PathBuf, ConductorStoreError> {
    validate_goal_id(goal_id)?;
    Ok(conductor_store_dir(project_root).join(format!("{goal_id}.json")))
}

fn validate_goal_id(goal_id: &str) -> Result<(), ConductorStoreError> {
    let valid = !goal_id.is_empty()
        && goal_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    if valid {
        Ok(())
    } else {
        Err(ConductorStoreError::InvalidGoalId(goal_id.to_string()))
    }
}

async fn is_regular_file(path: &Path) -> Result<bool, ConductorStoreError> {
    let metadata = fs::symlink_metadata(path)
        .await
        .map_err(|source| io_error("stat conductor ledger", path, source))?;
    Ok(metadata.is_file() && !metadata.file_type().is_symlink())
}

async fn read_goal_ledger_file(path: &Path) -> Result<GoalLedger, ConductorStoreError> {
    if !is_regular_file(path).await? {
        return Err(io_error(
            "read conductor ledger",
            path,
            std::io::Error::new(
                ErrorKind::InvalidData,
                "conductor ledger is not a regular file",
            ),
        ));
    }
    let content = fs::read_to_string(path)
        .await
        .map_err(|source| io_error("read conductor ledger", path, source))?;
    serde_json::from_str(&content).map_err(|source| ConductorStoreError::Json {
        path: path.to_path_buf(),
        source,
    })
}

fn io_error(action: &'static str, path: &Path, source: std::io::Error) -> ConductorStoreError {
    ConductorStoreError::Io {
        action,
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conductor::{ConductorMemo, ConductorWakeReason, MemoKind, PendingQuestion};

    fn sample_ledger() -> GoalLedger {
        GoalLedger {
            status: Some(crate::conductor::GoalStatus::Running),
            autonomy: Some(crate::conductor::GoalAutonomy::FullAuto),
            planner_task_id: Some("planner-task".to_string()),
            task_ids: vec!["task-a".to_string(), "task-b".to_string()],
            chat_ids: vec!["chat-a".to_string(), "chat-b".to_string()],
            memos: vec![ConductorMemo {
                id: "memo-a".to_string(),
                kind: MemoKind::Decision,
                content: "Use conductor store".to_string(),
                created_at: "2026-06-03T00:00:00Z".to_string(),
                source_chat_id: Some("chat-a".to_string()),
                related_task_id: Some("task-a".to_string()),
            }],
            pending_questions: vec![PendingQuestion {
                id: "question-a".to_string(),
                question: "Proceed?".to_string(),
                asked_at: "2026-06-03T00:00:01Z".to_string(),
                source_chat_id: Some("chat-b".to_string()),
                blocking: true,
                answer: Some("Yes".to_string()),
                answered_at: Some("2026-06-03T00:00:02Z".to_string()),
            }],
            no_progress_wakes: 1,
            turn_failures: 0,
            last_wake_at: Some("2026-06-03T00:00:04Z".to_string()),
            last_progress_at: Some("2026-06-03T00:00:03Z".to_string()),
            last_wake_reason: Some(ConductorWakeReason::Heartbeat),
        }
    }

    #[tokio::test]
    async fn save_and_load_goal_ledger_roundtrip_preserves_fields() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = sample_ledger();

        save_goal_ledger(dir.path(), "goal-a", &ledger)
            .await
            .unwrap();
        let loaded = load_goal_ledger(dir.path(), "goal-a").await.unwrap();

        assert_eq!(loaded, Some(ledger));
    }

    #[tokio::test]
    async fn list_goal_ledgers_returns_sorted_results() {
        let dir = tempfile::tempdir().unwrap();
        save_goal_ledger(dir.path(), "goal-c", &sample_ledger())
            .await
            .unwrap();
        save_goal_ledger(dir.path(), "goal-a", &sample_ledger())
            .await
            .unwrap();
        save_goal_ledger(dir.path(), "goal-b", &sample_ledger())
            .await
            .unwrap();

        let entries = list_goal_ledgers(dir.path()).await.unwrap();
        let goal_ids = entries
            .into_iter()
            .map(|entry| entry.goal_id)
            .collect::<Vec<_>>();

        assert_eq!(goal_ids, vec!["goal-a", "goal-b", "goal-c"]);
    }

    #[tokio::test]
    async fn remove_goal_ledger_removes_only_target_goal() {
        let dir = tempfile::tempdir().unwrap();
        let first = sample_ledger();
        let mut second = sample_ledger();
        second.planner_task_id = Some("second-planner".to_string());
        save_goal_ledger(dir.path(), "goal-a", &first)
            .await
            .unwrap();
        save_goal_ledger(dir.path(), "goal-b", &second)
            .await
            .unwrap();

        assert!(remove_goal_ledger(dir.path(), "goal-a").await.unwrap());
        assert!(!remove_goal_ledger(dir.path(), "goal-a").await.unwrap());

        assert_eq!(load_goal_ledger(dir.path(), "goal-a").await.unwrap(), None);
        assert_eq!(
            load_goal_ledger(dir.path(), "goal-b").await.unwrap(),
            Some(second)
        );
    }

    #[tokio::test]
    async fn invalid_goal_ids_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        for goal_id in [
            "",
            ".",
            "..",
            "../goal",
            "goal/evil",
            "goal\\evil",
            "goal evil",
        ] {
            assert!(matches!(
                save_goal_ledger(dir.path(), goal_id, &sample_ledger()).await,
                Err(ConductorStoreError::InvalidGoalId(_))
            ));
            assert!(matches!(
                load_goal_ledger(dir.path(), goal_id).await,
                Err(ConductorStoreError::InvalidGoalId(_))
            ));
            assert!(matches!(
                remove_goal_ledger(dir.path(), goal_id).await,
                Err(ConductorStoreError::InvalidGoalId(_))
            ));
        }
    }

    #[tokio::test]
    async fn corrupt_json_returns_controlled_error() {
        let dir = tempfile::tempdir().unwrap();
        let store_dir = conductor_store_dir(dir.path());
        fs::create_dir_all(&store_dir).await.unwrap();
        fs::write(store_dir.join("goal-bad.json"), "{not-json")
            .await
            .unwrap();

        let load_error = load_goal_ledger(dir.path(), "goal-bad").await.unwrap_err();
        let list_error = list_goal_ledgers(dir.path()).await.unwrap_err();

        assert!(matches!(load_error, ConductorStoreError::Json { .. }));
        assert!(matches!(list_error, ConductorStoreError::Json { .. }));
    }
}
