use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use git2::Repository;
use tokio::fs;
use tracing::warn;
use uuid::Uuid;
use chrono::Utc;

use refact_buddy_core::user_action::UserAction;
use crate::app_state::AppState;
use crate::global_context::GlobalContext;
use crate::files_correction::get_project_dirs;
use super::board_lock::{self, BoardLockGuard, BOARD_LOCK_FILE};
use super::types::{
    BoardCard, BoardReviewAttestation, BoardReviewPolicy, BoardRuleViolation, CardClaim,
    ClaimOutcome, DependencyOverride, TaskBoard, TaskMeta, TaskStatus, TrajectoryInfo,
};
use crate::coordination::error::WorkError;
use super::events::{emit_task_event, emit_task_updated, TaskEvent};
use crate::coordination::board_rows::{self, BoardProvenance};

const TASKS_DIR: &str = "tasks";
const PLANNER_ID_COUNTER_FILE: &str = "planner_next_id";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoardConflict {
    pub expected: u64,
    pub actual: u64,
}

impl BoardConflict {
    pub fn message(&self) -> String {
        format!(
            "Board rev mismatch: expected {}, actual {}",
            self.expected, self.actual
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardSaveError {
    Conflict(BoardConflict),
    Failed(String),
}

impl BoardSaveError {
    pub fn message(&self) -> String {
        match self {
            BoardSaveError::Conflict(conflict) => conflict.message(),
            BoardSaveError::Failed(error) => error.clone(),
        }
    }

    pub fn conflict(&self) -> Option<BoardConflict> {
        match self {
            BoardSaveError::Conflict(conflict) => Some(*conflict),
            BoardSaveError::Failed(_) => None,
        }
    }
}

impl From<String> for BoardSaveError {
    fn from(value: String) -> Self {
        BoardSaveError::Failed(value)
    }
}

impl std::fmt::Display for BoardSaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

pub async fn get_tasks_dir(gcx: Arc<GlobalContext>) -> Result<PathBuf, String> {
    let project_dirs = get_project_dirs(gcx).await;
    let workspace_root = project_dirs.first().ok_or("No workspace folder found")?;
    Ok(workspace_root.join(".refact").join(TASKS_DIR))
}

pub async fn get_global_tasks_dir(gcx: Arc<GlobalContext>) -> PathBuf {
    let config_dir = gcx.config_dir.clone();
    config_dir.join(TASKS_DIR)
}

pub async fn get_all_tasks_dirs(gcx: Arc<GlobalContext>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = get_project_dirs(gcx.clone())
        .await
        .into_iter()
        .map(|p| p.join(".refact").join(TASKS_DIR))
        .filter(|p| p.exists())
        .collect();

    let global_dir = get_global_tasks_dir(gcx).await;
    if global_dir.exists() {
        dirs.push(global_dir);
    }

    dirs
}

pub async fn find_task_dir(gcx: Arc<GlobalContext>, task_id: &str) -> Result<PathBuf, String> {
    validate_task_id(task_id)?;
    for tasks_dir in get_all_tasks_dirs(gcx).await {
        let candidate = tasks_dir.join(task_id);
        if candidate.is_dir() {
            return Ok(candidate);
        }
    }
    Err(format!("Task not found: {}", task_id))
}

pub async fn ensure_tasks_dir(gcx: Arc<GlobalContext>) -> Result<PathBuf, String> {
    let dir = get_tasks_dir(gcx).await?;
    if !dir.exists() {
        fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
    }
    Ok(dir)
}

pub fn validate_task_id(task_id: &str) -> Result<(), String> {
    if task_id.is_empty() {
        return Err("Task ID cannot be empty".into());
    }
    if task_id.len() > 128 {
        return Err("Task ID too long".into());
    }
    if !task_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(
            "Task ID must contain only alphanumeric characters, hyphens, or underscores".into(),
        );
    }
    Ok(())
}

pub async fn list_tasks(gcx: Arc<GlobalContext>) -> Result<Vec<TaskMeta>, String> {
    let tasks_dirs = get_all_tasks_dirs(gcx.clone()).await;

    let mut tasks = vec![];
    let mut seen_ids = HashSet::new();

    for tasks_dir in tasks_dirs {
        if !tasks_dir.exists() {
            continue;
        }

        match super::index::list_tasks_from_index_or_rebuild(&tasks_dir).await {
            Ok(metas) => {
                for meta in metas {
                    if seen_ids.insert(meta.id.clone()) {
                        tasks.push(meta);
                    }
                }
            }
            Err(e) => warn!("Failed to list tasks from {:?}: {}", tasks_dir, e),
        }
    }

    tasks.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(tasks)
}

async fn load_task_meta_from_path(path: &PathBuf) -> Result<TaskMeta, String> {
    let content = fs::read_to_string(path).await.map_err(|e| e.to_string())?;
    serde_yaml::from_str(&content).map_err(|e| e.to_string())
}

pub async fn load_task_meta(gcx: Arc<GlobalContext>, task_id: &str) -> Result<TaskMeta, String> {
    let task_dir = find_task_dir(gcx, task_id).await?;
    let meta_path = task_dir.join("meta.yaml");
    load_task_meta_from_path(&meta_path).await
}

pub async fn save_task_meta(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    meta: &TaskMeta,
) -> Result<(), String> {
    let task_dir = find_task_dir(gcx, task_id).await?;
    let meta_path = task_dir.join("meta.yaml");
    let content = serde_yaml::to_string(meta).map_err(|e| e.to_string())?;
    fs::write(&meta_path, content)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(tasks_dir) = task_dir.parent() {
        super::index::upsert_task_index_entry(tasks_dir, meta).await?;
    }
    Ok(())
}

pub async fn load_board(gcx: Arc<GlobalContext>, task_id: &str) -> Result<TaskBoard, String> {
    let task_dir = find_task_dir(gcx, task_id).await?;
    read_board_from_dir(&task_dir).await
}

async fn read_board_from_dir(task_dir: &Path) -> Result<TaskBoard, String> {
    read_board_with_provenance(task_dir)
        .await
        .map(|(board, _)| board)
}

async fn read_board_with_provenance(
    task_dir: &Path,
) -> Result<(TaskBoard, BoardProvenance), String> {
    let board_path = task_dir.join("board.yaml");
    if !board_path.exists() {
        return Ok((TaskBoard::default(), BoardProvenance::default()));
    }
    let content = fs::read_to_string(&board_path)
        .await
        .map_err(|e| e.to_string())?;
    let value: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| e.to_string())?;
    let board = serde_yaml::from_value(value.clone()).map_err(|e| e.to_string())?;
    let provenance = value
        .get("work_provenance")
        .and_then(|value| serde_yaml::from_value(value.clone()).ok())
        .unwrap_or_default();
    Ok((board, provenance))
}

async fn write_board_locked(
    gcx: &GlobalContext,
    guard: &BoardLockGuard,
    task_dir: &Path,
    board: &TaskBoard,
    provenance: &BoardProvenance,
) -> Result<(), String> {
    if !guard.guards(BOARD_LOCK_FILE) {
        return Err("Board write attempted without the board lock".to_string());
    }
    if guard.dir() != task_dir {
        return Err(format!(
            "Board write attempted under a lock for a different task dir: {} vs {}",
            guard.dir().display(),
            task_dir.display()
        ));
    }
    let board_path = task_dir.join("board.yaml");
    let mut value = serde_yaml::to_value(board).map_err(|e| e.to_string())?;
    value
        .as_mapping_mut()
        .ok_or("board must be a mapping")?
        .insert(
            serde_yaml::Value::String("work_provenance".into()),
            serde_yaml::to_value(provenance).map_err(|e| e.to_string())?,
        );
    let content = serde_yaml::to_string(&value).map_err(|e| e.to_string())?;
    crate::file_activity::write_file_durably(
        &board_path,
        content.as_bytes(),
        crate::file_activity::ReplaceMode::Replace,
    )
    .await?;
    gcx.work_boards.publish(task_dir, Some((board, provenance)));
    Ok(())
}

async fn locked_task_dir(
    gcx: Arc<GlobalContext>,
    task_id: &str,
) -> Result<(PathBuf, BoardLockGuard), String> {
    let task_dir = find_task_dir(gcx, task_id).await?;
    let guard = board_lock::acquire_board(&task_dir).await?;
    let canonical = guard.dir().to_path_buf();
    Ok((canonical, guard))
}

async fn board_coordination(
    gcx: Arc<GlobalContext>,
    task_id: &str,
) -> Result<refact_work_core::CoordinationSettings, String> {
    let task_dir = find_task_dir(gcx, task_id).await?;
    let project_root = task_dir
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or_else(|| format!("Task directory {} has no project root", task_dir.display()))?;
    crate::runtime_settings::coordination_for_project(project_root).await
}

fn validate_board(
    board: &TaskBoard,
    settings: &refact_work_core::CoordinationSettings,
) -> Result<(), String> {
    board.validate_for_write(
        settings.board_max_reviewers,
        settings.board_claim_key_max_len,
    )
}

fn rule_error(violation: BoardRuleViolation) -> String {
    WorkError::new(violation.code(), violation.message(), violation.retryable()).to_string()
}

/// Create `board.yaml` for a task that must not have one yet.
///
/// This is the only writer allowed to run without a revision check, and it
/// still refuses to clobber an existing board.
pub async fn create_board(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    board: &TaskBoard,
) -> Result<(), BoardSaveError> {
    let settings = board_coordination(gcx.clone(), task_id)
        .await
        .map_err(BoardSaveError::from)?;
    validate_board(board, &settings).map_err(BoardSaveError::from)?;
    let source = board_rows::actor_provenance(&gcx).await?;
    let (task_dir, guard) = locked_task_dir(gcx.clone(), task_id).await?;
    let existing = read_board_from_dir(&task_dir).await?;
    if task_dir.join("board.yaml").exists() {
        return Err(BoardSaveError::Conflict(BoardConflict {
            expected: 0,
            actual: existing.rev,
        }));
    }
    let provenance = board_rows::write_provenance(board, None, source);
    write_board_locked(&gcx, &guard, &task_dir, board, &provenance).await?;
    Ok(())
}

#[cfg(test)]
pub async fn save_board(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    board: &TaskBoard,
) -> Result<(), String> {
    validate_board(board, &board_coordination(gcx.clone(), task_id).await?)?;
    let (task_dir, guard) = locked_task_dir(gcx.clone(), task_id)
        .await
        .map_err(|e| e.to_string())?;
    write_board_locked(&gcx, &guard, &task_dir, board, &BoardProvenance::default()).await
}

/// Replace `board.yaml` only when the on-disk revision still matches
/// `expected_rev`, under the cross-process board lock.
pub async fn save_board_checked(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    expected_rev: u64,
    board: &TaskBoard,
) -> Result<(), BoardSaveError> {
    let settings = board_coordination(gcx.clone(), task_id)
        .await
        .map_err(BoardSaveError::from)?;
    validate_board(board, &settings).map_err(BoardSaveError::Failed)?;
    let source = board_rows::actor_provenance(&gcx).await?;
    let (task_dir, guard) = locked_task_dir(gcx.clone(), task_id).await?;
    let (existing, previous) = read_board_with_provenance(&task_dir).await?;
    if existing.rev != expected_rev {
        return Err(BoardSaveError::Conflict(BoardConflict {
            expected: expected_rev,
            actual: existing.rev,
        }));
    }
    let provenance = board_rows::write_provenance(board, Some((&existing, &previous)), source);
    write_board_locked(&gcx, &guard, &task_dir, board, &provenance).await?;
    Ok(())
}

pub async fn update_board_atomic<F, T>(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    updater: F,
) -> Result<(TaskBoard, T), String>
where
    F: FnOnce(&mut TaskBoard) -> Result<T, String>,
{
    let planner_actor = match crate::file_activity::current_actor() {
        Some(actor)
            if crate::chat::trajectories::chat_id_is_planner_for_task(
                gcx.clone(),
                &actor,
                task_id,
            )
            .await =>
        {
            Some(actor)
        }
        _ => None,
    };
    update_board_atomic_with_reviewer(gcx, task_id, planner_actor, None, updater).await
}

pub async fn update_board_atomic_as_reviewer<F, T>(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    reviewer_chat_id: &str,
    updater: F,
) -> Result<(TaskBoard, T), String>
where
    F: FnOnce(&mut TaskBoard) -> Result<T, String>,
{
    if !crate::chat::trajectories::chat_id_is_planner_for_task(
        gcx.clone(),
        reviewer_chat_id,
        task_id,
    )
    .await
    {
        return Err("task review requires an authenticated planner chat".to_string());
    }
    update_board_atomic_with_reviewer(
        gcx,
        task_id,
        None,
        Some(reviewer_chat_id.to_string()),
        updater,
    )
    .await
}

pub async fn attest_card_review(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
    reviewer_chat_id: &str,
) -> Result<(TaskBoard, BoardReviewAttestation), String> {
    let allow_self_review = board_allows_self_review(gcx.clone(), task_id).await?;
    let card_id = card_id.to_string();
    let reviewer_chat_id = reviewer_chat_id.to_string();
    let reviewer_for_auth = reviewer_chat_id.clone();
    let reviewer_for_update = reviewer_chat_id.clone();
    update_board_atomic_as_reviewer(gcx, task_id, &reviewer_for_auth, move |board| {
        if board.owner_chat_id.as_deref() == Some(reviewer_for_update.as_str()) {
            return Err("task board owner cannot attest a review".to_string());
        }
        let card = board
            .get_card(&card_id)
            .ok_or_else(|| format!("Card {} not found", card_id))?;
        if card.column != "done" {
            return Err("only a done card can be attested".to_string());
        }
        let report = card
            .verifier_report
            .clone()
            .ok_or_else(|| "no verifier report to attest; wait for the verifier".to_string())?;
        if !report.passed {
            return Err("cannot attest a failing verifier report".to_string());
        }
        if !allow_self_review
            && board
                .card_claims
                .get(&card_id)
                .is_some_and(|claim| claim.claimant_chat_id == reviewer_for_update)
        {
            return Err("task review policy refuses self-review by the card claimant".to_string());
        }
        let reviewed_at = Utc::now().to_rfc3339();
        board.attest_review(
            &card_id,
            &reviewer_for_update,
            &report,
            reviewed_at,
            allow_self_review,
        )?;
        board
            .review_attestations
            .get(&card_id)
            .cloned()
            .ok_or_else(|| "review attestation was not stored".to_string())
    })
    .await
}

async fn update_board_atomic_with_reviewer<F, T>(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    planner_actor: Option<String>,
    reviewer_actor: Option<String>,
    updater: F,
) -> Result<(TaskBoard, T), String>
where
    F: FnOnce(&mut TaskBoard) -> Result<T, String>,
{
    let settings = board_coordination(gcx.clone(), task_id).await?;
    let source = board_rows::actor_provenance(&gcx).await?;
    let (task_dir, guard) = locked_task_dir(gcx.clone(), task_id).await?;

    let (existing, previous) = read_board_with_provenance(&task_dir).await?;
    let mut board = existing.clone();
    let failed_before: HashSet<String> = board
        .cards
        .iter()
        .filter(|card| card.column == "failed" || card.column == "regressed")
        .map(|card| card.id.clone())
        .collect();
    if let Some(actor) = planner_actor.as_deref() {
        board.authorize_owner(actor)?;
    }
    if let Some(reviewer) = reviewer_actor.as_deref() {
        board.authorize_reviewer(reviewer, settings.board_allow_self_review)?;
    }
    let result = updater(&mut board)?;
    for card in &mut board.cards {
        if card.column != "doing" {
            card.attention = None;
        }
    }
    validate_board(&board, &settings)?;
    let new_failed = board
        .cards
        .iter()
        .filter(|card| {
            (card.column == "failed" || card.column == "regressed")
                && !failed_before.contains(&card.id)
        })
        .map(|card| {
            let reason_short = card
                .final_report
                .as_deref()
                .filter(|report| !report.trim().is_empty())
                .or_else(|| {
                    card.status_updates
                        .last()
                        .map(|update| update.message.as_str())
                })
                .unwrap_or(&card.title)
                .chars()
                .take(80)
                .collect::<String>();
            UserAction::TaskFailed {
                task_id: task_id.to_string(),
                reason_short,
                ts: Utc::now(),
            }
        })
        .collect::<Vec<_>>();
    board.rev += 1;
    let provenance = board_rows::write_provenance(&board, Some((&existing, &previous)), source);
    write_board_locked(&gcx, &guard, &task_dir, &board, &provenance).await?;
    drop(guard);
    if !new_failed.is_empty() {
        let user_activity = gcx.user_activity.clone();
        if let Ok(mut ring) = user_activity.try_lock() {
            for action in new_failed {
                ring.push(action);
            }
        };
    }
    emit_task_event(
        gcx,
        TaskEvent::BoardChanged {
            task_id: task_id.to_string(),
            rev: board.rev,
            board: board.clone(),
        },
    )
    .await;
    Ok((board, result))
}

/// Compare-and-set claim inside the guarded board transaction: the card must be
/// unclaimed, or already held by the same requester under the same idempotency
/// key. Every other requester loses with a structured `already_claimed` error
/// and the board is not rewritten.
pub async fn claim_card(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
    claimant_chat_id: &str,
    idempotency_key: &str,
) -> Result<(TaskBoard, ClaimOutcome), String> {
    let card_id = card_id.to_string();
    let claimant_chat_id = claimant_chat_id.to_string();
    let idempotency_key = idempotency_key.to_string();
    let claimed_at = Utc::now().to_rfc3339();
    update_board_atomic(gcx, task_id, move |board| {
        board
            .claim_card(&card_id, &claimant_chat_id, &idempotency_key, claimed_at)
            .map_err(rule_error)
    })
    .await
}

pub async fn release_card_claim(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
    claimant_chat_id: &str,
) -> Result<TaskBoard, String> {
    let card_id = card_id.to_string();
    let claimant_chat_id = claimant_chat_id.to_string();
    let (board, ()) = update_board_atomic(gcx, task_id, move |board| {
        board
            .release_claim(&card_id, &claimant_chat_id)
            .map_err(rule_error)
    })
    .await?;
    Ok(board)
}

/// Explicit, separately named dependency override. It is recorded on the board
/// with the acting principal and reason; there is no silent bypass.
pub async fn override_card_dependencies(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
    overridden_by: &str,
    reason: &str,
) -> Result<(TaskBoard, Vec<String>), String> {
    if reason.trim().is_empty() {
        return Err(WorkError::invalid("dependency override requires a reason").to_string());
    }
    let card_id = card_id.to_string();
    let overridden_by = overridden_by.to_string();
    let reason = reason.to_string();
    let overridden_at = Utc::now().to_rfc3339();
    update_board_atomic(gcx, task_id, move |board| {
        board.require_owner(&overridden_by)?;
        board
            .record_dependency_override(&card_id, &overridden_by, &reason, overridden_at)
            .map_err(rule_error)
    })
    .await
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredCard {
    pub task_id: String,
    pub card_id: String,
    pub title: Option<String>,
    pub column: Option<String>,
    pub agent_chat_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiscoveredBoardAuthority {
    pub task_id: String,
    pub rev: u64,
    pub owner_chat_id: Option<String>,
    pub reviewer_chat_ids: Vec<String>,
    pub review_policy: BoardReviewPolicy,
    pub claims: std::collections::BTreeMap<String, CardClaim>,
    pub dependency_overrides: std::collections::BTreeMap<String, DependencyOverride>,
}

/// Read-only project-wide discovery served from the published board projection,
/// so it never reopens every `board.yaml`. Metadata only: the ids it returns
/// confer no authority over the cards they name.
pub async fn discover_project_cards(
    gcx: Arc<GlobalContext>,
) -> Result<(Vec<DiscoveredCard>, bool), String> {
    let project_dirs = get_project_dirs(gcx.clone()).await;
    let root = project_dirs.first().ok_or("No workspace folder found")?;
    let (rows, unavailable) = gcx.work_boards.rows(root).await;
    let mut cards = rows
        .into_iter()
        .filter_map(|row| {
            Some(DiscoveredCard {
                task_id: row.metadata.task_id?,
                card_id: row.metadata.card_id?,
                title: row.title,
                column: row.metadata.runtime_status,
                agent_chat_id: row.chat_id,
            })
        })
        .collect::<Vec<_>>();
    cards.sort_by(|a, b| {
        a.task_id
            .cmp(&b.task_id)
            .then_with(|| a.card_id.cmp(&b.card_id))
    });
    cards.dedup_by(|a, b| a.task_id == b.task_id && a.card_id == b.card_id);
    Ok((cards, unavailable))
}

/// Claim and role state for a single board. Metadata only; it grants nothing.
pub async fn discover_board_authority(
    gcx: Arc<GlobalContext>,
    task_id: &str,
) -> Result<DiscoveredBoardAuthority, String> {
    let board = load_board(gcx, task_id).await?;
    Ok(DiscoveredBoardAuthority {
        task_id: task_id.to_string(),
        rev: board.rev,
        owner_chat_id: board.owner_chat_id,
        reviewer_chat_ids: board.reviewer_chat_ids,
        review_policy: board.review_policy,
        claims: board.card_claims,
        dependency_overrides: board.dependency_overrides,
    })
}

pub async fn board_allows_self_review(
    gcx: Arc<GlobalContext>,
    task_id: &str,
) -> Result<bool, String> {
    Ok(board_coordination(gcx, task_id)
        .await?
        .board_allow_self_review)
}

pub fn violation_error(violation: BoardRuleViolation) -> String {
    rule_error(violation)
}

pub async fn load_planner_instructions(
    gcx: Arc<GlobalContext>,
    task_id: &str,
) -> Result<String, String> {
    let task_dir = find_task_dir(gcx, task_id).await?;
    let path = task_dir.join("planner_instructions.md");
    if !path.exists() {
        return Ok(String::new());
    }
    fs::read_to_string(&path).await.map_err(|e| e.to_string())
}

pub async fn save_planner_instructions(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    content: &str,
) -> Result<(), String> {
    let task_dir = find_task_dir(gcx, task_id).await?;
    let path = task_dir.join("planner_instructions.md");
    fs::write(&path, content).await.map_err(|e| e.to_string())
}

fn detect_git_branch_and_commit(workspace_root: &Path) -> (Option<String>, Option<String>) {
    let repo = match Repository::open(workspace_root) {
        Ok(r) => r,
        Err(_) => return (None, None),
    };

    let branch = match repo.head() {
        Ok(head) => head.shorthand().map(|s| s.to_string()),
        Err(_) => None,
    };

    let commit = match repo.head() {
        Ok(head) => match head.peel_to_commit() {
            Ok(c) => Some(c.id().to_string()),
            Err(_) => None,
        },
        Err(_) => None,
    };

    (branch, commit)
}

pub async fn create_task(gcx: Arc<GlobalContext>, name: &str) -> Result<TaskMeta, String> {
    let tasks_dir = ensure_tasks_dir(gcx.clone()).await?;
    let task_id = Uuid::new_v4().to_string();
    let task_dir = tasks_dir.join(&task_id);

    fs::create_dir_all(&task_dir)
        .await
        .map_err(|e| e.to_string())?;
    fs::create_dir_all(task_dir.join("trajectories").join("planner"))
        .await
        .map_err(|e| e.to_string())?;
    fs::create_dir_all(task_dir.join("trajectories").join("agents"))
        .await
        .map_err(|e| e.to_string())?;

    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    let (base_branch, base_commit) = project_dirs
        .first()
        .map(|p| detect_git_branch_and_commit(p))
        .unwrap_or((None, None));

    let now = Utc::now().to_rfc3339();
    let has_user_provided_name = !name.trim().is_empty() && name.to_lowercase() != "new task";
    let meta = TaskMeta {
        schema_version: 1,
        id: task_id.clone(),
        name: if has_user_provided_name {
            name.to_string()
        } else {
            "New Task".to_string()
        },
        status: TaskStatus::Planning,
        created_at: now.clone(),
        updated_at: now,
        cards_total: 0,
        cards_done: 0,
        cards_failed: 0,
        agents_active: 0,
        base_branch,
        base_commit,
        default_agent_model: None,
        is_name_generated: has_user_provided_name,
        last_agents_summary_at: None,
        planner_session_state: None,
    };

    save_task_meta(gcx.clone(), &task_id, &meta).await?;
    create_board(gcx.clone(), &task_id, &TaskBoard::default())
        .await
        .map_err(|e| e.message())?;
    save_planner_instructions(gcx.clone(), &task_id, "").await?;

    emit_task_event(
        gcx.clone(),
        TaskEvent::TaskCreated {
            task_id: task_id.clone(),
            meta: meta.clone(),
        },
    )
    .await;
    {
        let ev = crate::buddy::actor::make_runtime_event(
            "task_created",
            &format!("Task created: {}", meta.name),
            "task",
            &format!("task_{}", task_id),
            "completed",
            None,
        );
        crate::buddy::actor::buddy_enqueue_event(
            crate::app_state::AppState::from_gcx(gcx).await,
            ev,
        )
        .await;
    }

    Ok(meta)
}

pub async fn delete_task(gcx: Arc<GlobalContext>, task_id: &str) -> Result<(), String> {
    let task_dir = find_task_dir(gcx.clone(), task_id).await?;
    let guard = board_lock::acquire_board(&task_dir).await?;
    let tasks_dir = task_dir.parent().map(Path::to_path_buf);
    fs::remove_dir_all(&task_dir)
        .await
        .map_err(|e| e.to_string())?;
    gcx.work_boards.publish(guard.dir(), None);
    drop(guard);
    if let Some(tasks_dir) = tasks_dir {
        if let Err(e) = super::index::remove_task_index_entry(&tasks_dir, task_id).await {
            warn!(
                "Failed to remove task {} from index {:?}: {}",
                task_id, tasks_dir, e
            );
        }
    }
    emit_task_event(
        gcx,
        TaskEvent::TaskDeleted {
            task_id: task_id.to_string(),
        },
    )
    .await;
    Ok(())
}

pub async fn update_task_name(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    name: &str,
) -> Result<TaskMeta, String> {
    let mut meta = load_task_meta(gcx.clone(), task_id).await?;
    meta.name = name.to_string();
    meta.is_name_generated = true;
    meta.updated_at = Utc::now().to_rfc3339();
    save_task_meta(gcx.clone(), task_id, &meta).await?;
    emit_task_updated(gcx, task_id.to_string(), meta.clone()).await;
    Ok(meta)
}

pub async fn update_task_stats(gcx: Arc<GlobalContext>, task_id: &str) -> Result<TaskMeta, String> {
    let mut meta = load_task_meta(gcx.clone(), task_id).await?;
    let board = load_board(gcx.clone(), task_id).await?;

    meta.cards_total = board.cards.len();
    meta.cards_done = board.cards.iter().filter(|c| c.column == "done").count();
    meta.cards_failed = board
        .cards
        .iter()
        .filter(|c| c.column == "failed" || c.column == "regressed")
        .count();
    meta.agents_active = active_agent_count(&board.cards);
    meta.updated_at = Utc::now().to_rfc3339();

    save_task_meta(gcx.clone(), task_id, &meta).await?;
    emit_task_updated(gcx, task_id.to_string(), meta.clone()).await;
    Ok(meta)
}

pub(crate) fn active_agent_count(cards: &[BoardCard]) -> usize {
    cards
        .iter()
        .filter(|card| card.column == "doing")
        .map(|card| {
            if let Some(variants) = card.ab_variants.as_ref() {
                if variants.winner.is_none() {
                    return [variants.a.finish.as_ref(), variants.b.finish.as_ref()]
                        .into_iter()
                        .filter(|finish| finish.is_none())
                        .count();
                }
            }
            usize::from(card.agent_chat_id.is_some())
        })
        .sum()
}

pub fn get_task_trajectory_dir(task_dir: &PathBuf, role: &str, agent_id: Option<&str>) -> PathBuf {
    let base = task_dir.join("trajectories").join(role);
    match agent_id {
        Some(id) => base.join(id),
        None => base,
    }
}

fn should_list_task_trajectory(role: &str, data: &serde_json::Value) -> bool {
    if role != "planner" {
        return true;
    }
    match data.get("link_type").and_then(|v| v.as_str()) {
        Some("handoff" | "mode_transition" | "branch") | None => true,
        Some(_) => false,
    }
}

async fn read_planner_id_counter(task_dir: &Path) -> u32 {
    let path = task_dir.join(PLANNER_ID_COUNTER_FILE);
    match fs::read_to_string(&path).await {
        Ok(content) => content.trim().parse::<u32>().unwrap_or(0),
        Err(_) => 0,
    }
}

async fn write_planner_id_counter(task_dir: &Path, value: u32) -> Result<(), String> {
    let path = task_dir.join(PLANNER_ID_COUNTER_FILE);
    crate::file_activity::write_file_durably(
        &path,
        value.to_string().as_bytes(),
        crate::file_activity::ReplaceMode::Replace,
    )
    .await
    .map(|_| ())
}

/// Reserve the next planner chat id in the standard `planner-{task_id}-{n}` format.
///
/// The returned number is durably reserved before the lock is released, so two
/// callers never receive the same id even when neither has created its chat yet.
pub async fn next_planner_chat_id(
    gcx: Arc<GlobalContext>,
    task_id: &str,
) -> Result<String, String> {
    let task_dir = find_task_dir(gcx.clone(), task_id).await?;
    let guard = board_lock::acquire_planner_ids(&task_dir).await?;
    let task_dir = guard.dir().to_path_buf();

    let existing = list_task_trajectories_for(
        gcx,
        task_id,
        "planner",
        None,
        crate::chat::trajectory_index::TrajectoryIndexListingCaller::TaskPlannerIdAllocation,
    )
    .await?;
    let prefix = format!("planner-{}-", task_id);
    let max_listed = existing
        .iter()
        .filter_map(|t| {
            t.id.strip_prefix(&prefix)
                .and_then(|s| s.parse::<u32>().ok())
        })
        .max()
        .unwrap_or(0);
    let reserved = max_listed.max(read_planner_id_counter(&task_dir).await) + 1;
    write_planner_id_counter(&task_dir, reserved).await?;
    drop(guard);

    Ok(format!("planner-{}-{}", task_id, reserved))
}

pub async fn list_task_trajectories(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    role: &str,
    agent_id: Option<&str>,
) -> Result<Vec<TrajectoryInfo>, String> {
    list_task_trajectories_for(
        gcx,
        task_id,
        role,
        agent_id,
        crate::chat::trajectory_index::TrajectoryIndexListingCaller::TaskTrajectoryApi,
    )
    .await
}

pub async fn list_task_trajectories_for(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    role: &str,
    agent_id: Option<&str>,
    caller: crate::chat::trajectory_index::TrajectoryIndexListingCaller,
) -> Result<Vec<TrajectoryInfo>, String> {
    let task_dir = find_task_dir(gcx.clone(), task_id).await?;
    let traj_dir = get_task_trajectory_dir(&task_dir, role, agent_id);

    if !traj_dir.exists() {
        return Ok(vec![]);
    }

    let coordinator = AppState::from_gcx(gcx.clone())
        .await
        .chat
        .trajectory_index_coordinator;
    let entries = crate::chat::trajectory_index::list_trajectory_entries_with_rollout_shared_for(
        &coordinator,
        &traj_dir,
        None,
        caller,
    )
    .await?;
    let mut trajectories: Vec<TrajectoryInfo> = entries
        .iter()
        .filter(|entry| {
            should_list_task_trajectory(
                role,
                &serde_json::json!({ "link_type": entry.link_type.as_deref() }),
            )
        })
        .map(|entry| TrajectoryInfo {
            id: entry.id.clone(),
            title: entry.title.clone(),
            created_at: entry.created_at.clone(),
            updated_at: entry.updated_at.clone(),
            session_state: None,
            mode: Some(entry.mode.clone()),
            parent_id: entry.parent_id.clone(),
            waiting_for_card_ids: entry.waiting_for_card_ids.clone(),
        })
        .collect();

    trajectories.sort_by(|a, b| a.created_at.cmp(&b.created_at));

    let sessions = gcx.chat_sessions.read().await;
    for traj in &mut trajectories {
        if let Some(session_arc) = sessions.get(&traj.id) {
            let session = session_arc.lock().await;
            traj.session_state = Some(session.runtime.state.to_string());
            if !session.waiting_for_card_ids.is_empty() {
                traj.waiting_for_card_ids = session.waiting_for_card_ids.clone();
            }
        }
    }

    Ok(trajectories)
}

pub fn infer_task_id_from_chat_id(chat_id: &str) -> Option<String> {
    if let Some(id) = chat_id.strip_prefix("orch-") {
        return Some(id.to_string());
    }
    if let Some(id) = chat_id.strip_prefix("plan-") {
        return Some(id.to_string());
    }
    if let Some(rest) = chat_id.strip_prefix("planner-") {
        if let Some((task_id, suffix)) = rest.rsplit_once('-') {
            if !task_id.is_empty()
                && !suffix.is_empty()
                && suffix.chars().all(|c| c.is_ascii_digit())
            {
                return Some(task_id.to_string());
            }
        }
        return Some(rest.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn subprocess_board_claim() {
        let Ok(root) = std::env::var("REFACT_TEST_BOARD_RACE_ROOT") else {
            return;
        };
        let id = std::env::var("REFACT_TEST_BOARD_RACE_ID").unwrap();
        let root = PathBuf::from(root);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let gcx = crate::global_context::tests::make_test_gcx().await;
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
            std::fs::write(root.join(format!("ready-{id}")), "ready").unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !root.join("go").exists() {
                assert!(std::time::Instant::now() < deadline);
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            let result = update_board_atomic(gcx, "task-race", |board| {
                let card = board.cards.first_mut().ok_or("card missing")?;
                if card.agent_chat_id.is_some() {
                    return Err("already claimed".into());
                }
                card.agent_chat_id = Some(id.clone());
                card.column = "doing".into();
                Ok(())
            })
            .await;
            std::fs::write(
                root.join(format!("result-{id}")),
                if result.is_ok() { "won" } else { "claimed" },
            )
            .unwrap();
        });
    }

    fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                rust_sources(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    #[test]
    fn board_persistence_has_no_writer_outside_the_guarded_transaction() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_sources(&src, &mut files);
        files.sort();
        let this_file = src.join("tasks").join("storage.rs");
        let bootstrap = src.join("coordination").join("board_bootstrap.rs");
        let mut violations = Vec::new();
        let mut guarded_callers = std::collections::BTreeSet::new();
        for file in &files {
            let text = std::fs::read_to_string(file).unwrap();
            let relative = file.strip_prefix(&src).unwrap().display().to_string();
            let test_only_module = file
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("tests.rs"));
            let first_test_marker = if test_only_module {
                0
            } else if *file == this_file {
                text.find("\nmod tests {").unwrap_or(text.len())
            } else {
                text.find("#[cfg(test)]").unwrap_or(text.len())
            };
            let mut offset = 0usize;
            let mut current_fn = String::new();
            for line in text.split_inclusive('\n') {
                let start = offset;
                offset += line.len();
                let trimmed = line.trim_start();
                if let Some(rest) = trimmed
                    .strip_prefix("pub async fn ")
                    .or_else(|| trimmed.strip_prefix("pub(crate) async fn "))
                    .or_else(|| trimmed.strip_prefix("async fn "))
                    .or_else(|| trimmed.strip_prefix("pub fn "))
                    .or_else(|| trimmed.strip_prefix("pub(crate) fn "))
                    .or_else(|| trimmed.strip_prefix("fn "))
                {
                    current_fn = rest
                        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                        .next()
                        .unwrap_or_default()
                        .to_string();
                }
                if trimmed.starts_with("//") {
                    continue;
                }
                let mentions_board = line.contains("board.yaml");
                let calls_writer = line.contains("write_board_locked(");
                if !(mentions_board || calls_writer) {
                    continue;
                }
                if start >= first_test_marker {
                    continue;
                }
                if *file == this_file {
                    if calls_writer && !trimmed.starts_with("async fn write_board_locked(") {
                        guarded_callers.insert(current_fn.clone());
                    }
                    continue;
                }
                let read_only = *file == bootstrap
                    && !calls_writer
                    && !["write", "create", "rename", "remove", "open"]
                        .iter()
                        .any(|verb| line.contains(verb));
                if !read_only {
                    violations.push(format!("{relative}: {}", line.trim()));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "board writers outside storage.rs: {violations:#?}"
        );
        assert_eq!(
            guarded_callers,
            [
                "create_board",
                "save_board",
                "save_board_checked",
                "update_board_atomic_with_reviewer"
            ]
            .into_iter()
            .map(str::to_string)
            .collect()
        );
    }

    #[test]
    fn cooperating_processes_cannot_both_claim_one_board_card() {
        let root = tempfile::tempdir().unwrap();
        let task = root.path().join(".refact/tasks/task-race");
        std::fs::create_dir_all(&task).unwrap();
        let board: TaskBoard = serde_json::from_value(serde_json::json!({"cards":[{
            "id":"card","title":"Claim","column":"planned","created_at":"2026-01-01T00:00:00Z"
        }]}))
        .unwrap();
        std::fs::write(
            task.join("board.yaml"),
            serde_yaml::to_string(&board).unwrap(),
        )
        .unwrap();
        let spawn = |id: &str| {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tasks::storage::tests::subprocess_board_claim",
                    "--nocapture",
                ])
                .env("REFACT_TEST_BOARD_RACE_ROOT", root.path())
                .env("REFACT_TEST_BOARD_RACE_ID", id)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap()
        };
        let mut a = spawn("a");
        let mut b = spawn("b");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !root.path().join("ready-a").exists() || !root.path().join("ready-b").exists() {
            if std::time::Instant::now() >= deadline {
                let _ = a.kill();
                let _ = b.kill();
                let _ = a.wait();
                let _ = b.wait();
                panic!("board subprocesses did not reach barrier");
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        std::fs::write(root.path().join("go"), "go").unwrap();
        assert!(a.wait().unwrap().success());
        assert!(b.wait().unwrap().success());
        let a = std::fs::read_to_string(root.path().join("result-a")).unwrap();
        let b = std::fs::read_to_string(root.path().join("result-b")).unwrap();
        assert_ne!(a, b);
        let board: TaskBoard =
            serde_yaml::from_str(&std::fs::read_to_string(task.join("board.yaml")).unwrap())
                .unwrap();
        assert_eq!(board.rev, 1);
        assert!(board.cards[0].agent_chat_id.is_some());
    }
    use super::*;
    use refact_buddy_core::user_action::UserAction;
    use crate::tasks::types::{
        AbVariantFinish, AbVariantInfo, AbVariants, BoardCard, StatusUpdate, TaskBoard, TaskMeta,
        TaskStatus,
    };
    use chrono::Utc;
    use serde_json::json;

    #[tokio::test]
    async fn task_failed_pushed_on_status_transition() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() =
                vec![temp.path().to_path_buf()];
        }
        let task_id = "task-failed-ring";
        let task_dir = temp.path().join(".refact/tasks").join(task_id);
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        let now = Utc::now().to_rfc3339();
        let meta = TaskMeta {
            schema_version: 1,
            id: task_id.to_string(),
            name: "Task".to_string(),
            status: TaskStatus::Active,
            created_at: now.clone(),
            updated_at: now.clone(),
            cards_total: 1,
            cards_done: 0,
            cards_failed: 0,
            agents_active: 0,
            base_branch: None,
            base_commit: None,
            default_agent_model: None,
            is_name_generated: false,
            last_agents_summary_at: None,
            planner_session_state: None,
        };
        save_task_meta(gcx.clone(), task_id, &meta).await.unwrap();
        create_board(
            gcx.clone(),
            task_id,
            &TaskBoard {
                cards: vec![BoardCard {
                    id: "card-1".to_string(),
                    title: "Card".to_string(),
                    column: "doing".to_string(),
                    priority: "P1".to_string(),
                    depends_on: Vec::new(),
                    instructions: String::new(),
                    assignee: None,
                    agent_chat_id: None,
                    status_updates: vec![StatusUpdate {
                        timestamp: now.clone(),
                        message: "failure reason with details".to_string(),
                    }],
                    comments: vec![],
                    final_report: None,
                    final_report_structured: None,
                    verifier_report: None,
                    created_at: now.clone(),
                    started_at: None,
                    last_heartbeat_at: None,
                    completed_at: None,
                    agent_branch: None,
                    agent_worktree: None,
                    agent_worktree_name: None,
                    base_branch: None,
                    base_commit: None,
                    ab_variants: None,
                    team_members: vec![],
                    target_files: Vec::new(),
                    scope_guard_mode: Default::default(),
                    attention: None,
                }],
                ..Default::default()
            },
        )
        .await
        .unwrap();

        update_board_atomic(gcx.clone(), task_id, |board| {
            board.cards[0].column = "failed".to_string();
            Ok(())
        })
        .await
        .unwrap();

        let user_activity = gcx.user_activity.clone();
        let ring = user_activity.lock().await;
        assert!(ring.snapshot().iter().any(|action| matches!(
            action,
            UserAction::TaskFailed { task_id, reason_short, .. }
                if task_id == "task-failed-ring" && reason_short == "failure reason with details"
        )));
    }

    #[test]
    fn planner_listing_hides_subagentic_child_links() {
        assert!(!should_list_task_trajectory(
            "planner",
            &json!({"link_type": "gather_files"})
        ));
        assert!(!should_list_task_trajectory(
            "planner",
            &json!({"link_type": "subagent"})
        ));
    }

    #[test]
    fn planner_listing_keeps_real_planner_chats() {
        assert!(should_list_task_trajectory("planner", &json!({})));
        assert!(should_list_task_trajectory(
            "planner",
            &json!({"link_type": "handoff"})
        ));
        assert!(should_list_task_trajectory(
            "planner",
            &json!({"link_type": "mode_transition"})
        ));
    }

    #[test]
    fn non_planner_listing_keeps_child_links() {
        assert!(should_list_task_trajectory(
            "subchats",
            &json!({"link_type": "gather_files"})
        ));
    }

    fn test_meta(task_id: &str, name: &str, updated_at: &str) -> TaskMeta {
        TaskMeta {
            schema_version: 1,
            id: task_id.to_string(),
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

    async fn setup_workspace(root: &std::path::Path) -> Arc<GlobalContext> {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
        gcx
    }

    #[tokio::test]
    async fn list_tasks_rebuilds_missing_index_and_uses_it_afterward() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = setup_workspace(temp.path()).await;
        let tasks_dir = temp.path().join(".refact/tasks");
        let task_dir = tasks_dir.join("task-1");
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        let meta = test_meta("task-1", "Task", "2026-01-01T00:00:00Z");
        tokio::fs::write(
            task_dir.join("meta.yaml"),
            serde_yaml::to_string(&meta).unwrap(),
        )
        .await
        .unwrap();

        let listed = list_tasks(gcx).await.unwrap();

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "task-1");
        assert!(crate::tasks::index::task_index_path(&tasks_dir).exists());
    }

    #[tokio::test]
    async fn create_update_delete_task_maintains_index() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = setup_workspace(temp.path()).await;
        let meta = create_task(gcx.clone(), "Indexed Task").await.unwrap();
        let tasks_dir = temp.path().join(".refact/tasks");
        let index = crate::tasks::index::read_task_index(&tasks_dir)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(index.entries.len(), 1);
        assert_eq!(index.entries[0].id, meta.id);

        let updated = update_task_name(gcx.clone(), &meta.id, "Renamed")
            .await
            .unwrap();
        let index = crate::tasks::index::read_task_index(&tasks_dir)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(index.entries.len(), 1);
        assert_eq!(index.entries[0].meta.name, updated.name);

        delete_task(gcx.clone(), &meta.id).await.unwrap();
        let index = crate::tasks::index::read_task_index(&tasks_dir)
            .await
            .unwrap()
            .unwrap();
        assert!(index.entries.is_empty());
    }

    #[test]
    fn active_agent_count_counts_unfinished_ab_variants() {
        fn card(id: &str) -> BoardCard {
            BoardCard {
                id: id.to_string(),
                title: id.to_string(),
                column: "doing".to_string(),
                priority: "P1".to_string(),
                depends_on: vec![],
                instructions: String::new(),
                assignee: Some("ab".to_string()),
                agent_chat_id: None,
                status_updates: vec![],
                comments: vec![],
                final_report: None,
                final_report_structured: None,
                verifier_report: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                started_at: None,
                last_heartbeat_at: None,
                completed_at: None,
                agent_branch: None,
                agent_worktree: None,
                agent_worktree_name: None,
                base_branch: None,
                base_commit: None,
                ab_variants: None,
                team_members: vec![],
                target_files: vec![],
                scope_guard_mode: Default::default(),
                attention: None,
            }
        }
        fn variant(key: &str, finished: bool) -> AbVariantInfo {
            AbVariantInfo {
                agent_id: format!("agent-{key}"),
                chat_id: format!("chat-{key}"),
                worktree: format!("/tmp/{key}"),
                worktree_name: None,
                branch: None,
                model: None,
                finish: finished.then(|| AbVariantFinish {
                    success: true,
                    final_report: "done".to_string(),
                    final_report_structured: None,
                    completed_at: "2026-01-01T00:00:01Z".to_string(),
                    commit_hash: None,
                }),
            }
        }

        let mut ab = card("T-1");
        ab.ab_variants = Some(AbVariants {
            a: variant("a", true),
            b: variant("b", false),
            winner: None,
        });
        let mut singleton = card("T-2");
        singleton.assignee = Some("agent".to_string());
        singleton.agent_chat_id = Some("chat-single".to_string());

        assert_eq!(active_agent_count(&[ab, singleton]), 2);
    }

    async fn task_with_board(root: &std::path::Path, task_id: &str) -> Arc<GlobalContext> {
        let gcx = setup_workspace(root).await;
        let task_dir = root.join(".refact/tasks").join(task_id);
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        save_task_meta(
            gcx.clone(),
            task_id,
            &test_meta(task_id, "Task", "2026-01-01T00:00:00Z"),
        )
        .await
        .unwrap();
        create_board(gcx.clone(), task_id, &TaskBoard::default())
            .await
            .unwrap();
        gcx
    }

    struct BoardLockSettingsGuard(crate::runtime_settings::TrajectoryRuntimeSettings);

    impl BoardLockSettingsGuard {
        fn generous_timeout() -> Self {
            let previous = crate::runtime_settings::current();
            let mut settings = previous.clone();
            settings.coordination.lock_timeout_secs = 120;
            crate::runtime_settings::install_startup(settings);
            Self(previous)
        }
    }

    impl Drop for BoardLockSettingsGuard {
        fn drop(&mut self) {
            crate::runtime_settings::install_startup(self.0.clone());
        }
    }

    #[tokio::test]
    async fn create_board_refuses_to_clobber_existing_board() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = task_with_board(temp.path(), "task-create-once").await;

        let again = create_board(gcx.clone(), "task-create-once", &TaskBoard::default()).await;

        assert!(matches!(again, Err(BoardSaveError::Conflict(_))));
    }

    #[tokio::test]
    async fn save_board_checked_rejects_stale_revision() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = task_with_board(temp.path(), "task-checked").await;

        let (board, _) = update_board_atomic(gcx.clone(), "task-checked", |board| {
            board.cards.clear();
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(board.rev, 1);

        let stale = save_board_checked(gcx.clone(), "task-checked", 0, &board).await;
        assert_eq!(
            stale.unwrap_err().conflict(),
            Some(BoardConflict {
                expected: 0,
                actual: 1
            })
        );

        let mut fresh = board.clone();
        fresh.rev = 2;
        save_board_checked(gcx.clone(), "task-checked", 1, &fresh)
            .await
            .unwrap();
        assert_eq!(load_board(gcx, "task-checked").await.unwrap().rev, 2);
    }

    #[tokio::test]
    #[serial_test::serial(runtime_settings)]
    async fn concurrent_updates_do_not_lose_cards() {
        let _lock_settings = BoardLockSettingsGuard::generous_timeout();
        let temp = tempfile::tempdir().unwrap();
        let gcx = task_with_board(temp.path(), "task-concurrent-cards").await;

        let mut handles = vec![];
        for i in 0..8 {
            let gcx = gcx.clone();
            handles.push(tokio::spawn(async move {
                update_board_atomic(gcx, "task-concurrent-cards", move |board| {
                    board.cards.push(BoardCard {
                        id: format!("card-{i}"),
                        title: format!("card-{i}"),
                        column: "todo".to_string(),
                        priority: "P1".to_string(),
                        depends_on: vec![],
                        instructions: String::new(),
                        assignee: None,
                        agent_chat_id: None,
                        status_updates: vec![],
                        comments: vec![],
                        final_report: None,
                        final_report_structured: None,
                        verifier_report: None,
                        created_at: "2026-01-01T00:00:00Z".to_string(),
                        started_at: None,
                        last_heartbeat_at: None,
                        completed_at: None,
                        agent_branch: None,
                        agent_worktree: None,
                        agent_worktree_name: None,
                        base_branch: None,
                        base_commit: None,
                        ab_variants: None,
                        team_members: vec![],
                        target_files: vec![],
                        scope_guard_mode: Default::default(),
                        attention: None,
                    });
                    Ok(())
                })
                .await
            }));
        }
        for handle in handles {
            handle.await.unwrap().unwrap();
        }

        let board = load_board(gcx, "task-concurrent-cards").await.unwrap();
        assert_eq!(board.cards.len(), 8);
        assert_eq!(board.rev, 8);
    }

    #[tokio::test]
    async fn board_write_rejects_lock_for_another_task_dir() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = task_with_board(temp.path(), "task-guard-a").await;
        let other_dir = temp.path().join(".refact/tasks").join("task-guard-b");
        tokio::fs::create_dir_all(&other_dir).await.unwrap();

        let task_dir = find_task_dir(gcx.clone(), "task-guard-a").await.unwrap();
        let foreign_guard = board_lock::acquire_board(&other_dir).await.unwrap();

        let error = write_board_locked(
            &gcx,
            &foreign_guard,
            &task_dir,
            &TaskBoard::default(),
            &BoardProvenance::default(),
        )
        .await
        .unwrap_err();

        assert!(error.contains("different task dir"), "{error}");
    }

    #[tokio::test]
    async fn board_write_rejects_planner_id_lock() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = task_with_board(temp.path(), "task-guard-kind").await;
        let task_dir = find_task_dir(gcx.clone(), "task-guard-kind").await.unwrap();
        let planner_guard = board_lock::acquire_planner_ids(&task_dir).await.unwrap();

        let error = write_board_locked(
            &gcx,
            &planner_guard,
            planner_guard.dir(),
            &TaskBoard::default(),
            &BoardProvenance::default(),
        )
        .await
        .unwrap_err();

        assert!(error.contains("without the board lock"), "{error}");
    }

    #[tokio::test]
    async fn planner_chat_ids_are_reserved_durably_without_created_chats() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = task_with_board(temp.path(), "task-planner-ids").await;

        let first = next_planner_chat_id(gcx.clone(), "task-planner-ids")
            .await
            .unwrap();
        let second = next_planner_chat_id(gcx.clone(), "task-planner-ids")
            .await
            .unwrap();

        assert_eq!(first, "planner-task-planner-ids-1");
        assert_eq!(second, "planner-task-planner-ids-2");
    }

    #[tokio::test]
    #[serial_test::serial(runtime_settings)]
    async fn concurrent_planner_chat_id_reservations_are_unique() {
        let _lock_settings = BoardLockSettingsGuard::generous_timeout();
        let temp = tempfile::tempdir().unwrap();
        let gcx = task_with_board(temp.path(), "task-planner-race").await;

        let mut handles = vec![];
        for _ in 0..8 {
            let gcx = gcx.clone();
            handles.push(tokio::spawn(async move {
                next_planner_chat_id(gcx, "task-planner-race").await
            }));
        }
        let mut ids = vec![];
        for handle in handles {
            ids.push(handle.await.unwrap().unwrap());
        }

        let unique: HashSet<String> = ids.iter().cloned().collect();
        assert_eq!(unique.len(), 8, "duplicate planner ids: {ids:?}");
        assert!(ids
            .iter()
            .all(|id| id.starts_with("planner-task-planner-race-")));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn planner_ids_are_unique_across_a_canonical_and_an_aliased_task_dir() {
        let temp = tempfile::tempdir().unwrap();
        let canonical_root = temp.path().join("canonical");
        let alias_root = temp.path().join("alias");
        tokio::fs::create_dir_all(alias_root.join(".refact"))
            .await
            .unwrap();
        let task_id = "task-alias-ids";
        let canonical_gcx = task_with_board(&canonical_root, task_id).await;
        std::os::unix::fs::symlink(
            canonical_root.join(".refact").join("tasks"),
            alias_root.join(".refact").join("tasks"),
        )
        .unwrap();
        let alias_gcx = setup_workspace(&alias_root).await;

        let canonical_dir = find_task_dir(canonical_gcx.clone(), task_id).await.unwrap();
        let aliased_dir = find_task_dir(alias_gcx.clone(), task_id).await.unwrap();
        assert_ne!(canonical_dir, aliased_dir);
        assert_eq!(
            board_lock::canonical_dir(&aliased_dir).await.unwrap(),
            canonical_dir
        );

        let mut handles = vec![];
        for index in 0..8 {
            let gcx = match index % 2 {
                0 => canonical_gcx.clone(),
                _ => alias_gcx.clone(),
            };
            handles.push(tokio::spawn(async move {
                next_planner_chat_id(gcx, task_id).await
            }));
        }
        let mut ids = vec![];
        for handle in handles {
            ids.push(handle.await.unwrap().unwrap());
        }

        let unique: HashSet<String> = ids.iter().cloned().collect();
        assert_eq!(unique.len(), 8, "duplicate planner ids: {ids:?}");
        let prefix = format!("planner-{task_id}-");
        let mut reserved = ids
            .iter()
            .map(|id| id.strip_prefix(&prefix).unwrap().parse::<u32>().unwrap())
            .collect::<Vec<_>>();
        reserved.sort();
        assert_eq!(reserved, (1..=8).collect::<Vec<u32>>(), "{ids:?}");
    }

    async fn task_with_cards(
        root: &std::path::Path,
        task_id: &str,
        cards: Vec<BoardCard>,
    ) -> Arc<GlobalContext> {
        let gcx = task_with_board(root, task_id).await;
        update_board_atomic(gcx.clone(), task_id, move |board| {
            board.cards = cards;
            Ok(())
        })
        .await
        .unwrap();
        gcx
    }

    fn claimable_card(id: &str, depends_on: Vec<&str>, column: &str) -> BoardCard {
        BoardCard {
            id: id.to_string(),
            title: id.to_string(),
            column: column.to_string(),
            priority: "P1".to_string(),
            depends_on: depends_on.into_iter().map(String::from).collect(),
            instructions: String::new(),
            assignee: None,
            agent_chat_id: None,
            status_updates: vec![],
            comments: vec![],
            final_report: None,
            final_report_structured: None,
            verifier_report: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            started_at: None,
            last_heartbeat_at: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
            base_branch: None,
            base_commit: None,
            ab_variants: None,
            team_members: vec![],
            target_files: vec![],
            scope_guard_mode: Default::default(),
            attention: None,
        }
    }

    #[tokio::test]
    async fn concurrent_claims_leave_exactly_one_winner_and_one_board_revision() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = task_with_cards(
            temp.path(),
            "task-claim-race",
            vec![claimable_card("T-1", vec![], "planned")],
        )
        .await;
        let base_rev = load_board(gcx.clone(), "task-claim-race")
            .await
            .unwrap()
            .rev;

        let mut handles = vec![];
        for index in 0..8 {
            let gcx = gcx.clone();
            handles.push(tokio::spawn(async move {
                claim_card(
                    gcx,
                    "task-claim-race",
                    "T-1",
                    &format!("agent-{index}"),
                    &format!("key-{index}"),
                )
                .await
            }));
        }
        let mut winners = vec![];
        let mut losers = vec![];
        for handle in handles {
            match handle.await.unwrap() {
                Ok((_, outcome)) => winners.push(outcome),
                Err(error) => losers.push(error),
            }
        }

        assert_eq!(winners, vec![ClaimOutcome::Claimed]);
        assert_eq!(losers.len(), 7);
        for error in &losers {
            let decoded: serde_json::Value = serde_json::from_str(error).unwrap();
            assert_eq!(decoded["code"], "already_claimed");
            assert_eq!(decoded["retryable"], false);
        }
        let board = load_board(gcx, "task-claim-race").await.unwrap();
        assert_eq!(board.rev, base_rev + 1);
        assert_eq!(board.card_claims.len(), 1);
    }

    #[tokio::test]
    async fn repeating_a_claim_with_the_same_key_keeps_the_original_holder_stamp() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = task_with_cards(
            temp.path(),
            "task-claim-retry",
            vec![claimable_card("T-1", vec![], "planned")],
        )
        .await;

        let (first, outcome) = claim_card(gcx.clone(), "task-claim-retry", "T-1", "agent-a", "key")
            .await
            .unwrap();
        assert_eq!(outcome, ClaimOutcome::Claimed);
        let (second, outcome) =
            claim_card(gcx.clone(), "task-claim-retry", "T-1", "agent-a", "key")
                .await
                .unwrap();

        assert_eq!(outcome, ClaimOutcome::AlreadyHeld);
        assert_eq!(second.rev, first.rev + 1);
        assert_eq!(
            second.card_claims["T-1"].claimed_at,
            first.card_claims["T-1"].claimed_at
        );
    }

    #[tokio::test]
    async fn claiming_a_blocked_card_reports_its_unsatisfied_dependencies() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = task_with_cards(
            temp.path(),
            "task-claim-blocked",
            vec![
                claimable_card("T-1", vec![], "planned"),
                claimable_card("T-2", vec!["T-1"], "planned"),
            ],
        )
        .await;

        let error = claim_card(gcx.clone(), "task-claim-blocked", "T-2", "agent-a", "key")
            .await
            .unwrap_err();

        let decoded: serde_json::Value = serde_json::from_str(&error).unwrap();
        assert_eq!(decoded["code"], "dependency_unsatisfied");
        assert_eq!(decoded["retryable"], true);
        assert!(decoded["message"].as_str().unwrap().contains("T-1"));
        assert!(load_board(gcx, "task-claim-blocked")
            .await
            .unwrap()
            .card_claims
            .is_empty());
    }

    #[tokio::test]
    async fn dependency_override_is_owner_gated_and_recorded_on_the_board() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = task_with_cards(
            temp.path(),
            "task-override",
            vec![
                claimable_card("T-1", vec![], "planned"),
                claimable_card("T-2", vec!["T-1"], "planned"),
            ],
        )
        .await;
        update_board_atomic(gcx.clone(), "task-override", |board| {
            board.owner_chat_id = Some("planner-chat".to_string());
            Ok(())
        })
        .await
        .unwrap();

        assert!(override_card_dependencies(
            gcx.clone(),
            "task-override",
            "T-2",
            "other-chat",
            "why"
        )
        .await
        .is_err());
        assert!(override_card_dependencies(
            gcx.clone(),
            "task-override",
            "T-2",
            "planner-chat",
            "   "
        )
        .await
        .is_err());

        let (board, unsatisfied) = override_card_dependencies(
            gcx.clone(),
            "task-override",
            "T-2",
            "planner-chat",
            "dependency landed out of band",
        )
        .await
        .unwrap();

        assert_eq!(unsatisfied, vec!["T-1".to_string()]);
        let recorded = &board.dependency_overrides["T-2"];
        assert_eq!(recorded.overridden_by, "planner-chat");
        assert_eq!(recorded.reason, "dependency landed out of band");
        claim_card(gcx, "task-override", "T-2", "agent-a", "key")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn discovery_reports_claim_and_role_state_without_granting_control() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = task_with_cards(
            temp.path(),
            "task-discovery",
            vec![claimable_card("T-1", vec![], "planned")],
        )
        .await;
        update_board_atomic(gcx.clone(), "task-discovery", |board| {
            board.owner_chat_id = Some("planner-chat".to_string());
            board.reviewer_chat_ids = vec!["reviewer-chat".to_string()];
            Ok(())
        })
        .await
        .unwrap();
        claim_card(gcx.clone(), "task-discovery", "T-1", "agent-a", "key")
            .await
            .unwrap();

        let (cards, _) = discover_project_cards(gcx.clone()).await.unwrap();
        let authority = discover_board_authority(gcx.clone(), "task-discovery")
            .await
            .unwrap();

        assert!(cards
            .iter()
            .any(|card| card.task_id == "task-discovery" && card.card_id == "T-1"));
        assert_eq!(authority.owner_chat_id.as_deref(), Some("planner-chat"));
        assert_eq!(
            authority.reviewer_chat_ids,
            vec!["reviewer-chat".to_string()]
        );
        assert_eq!(authority.claims["T-1"].claimant_chat_id, "agent-a");

        let discovered = cards
            .iter()
            .find(|card| card.card_id == "T-1")
            .unwrap()
            .clone();
        let denied = claim_card(
            gcx.clone(),
            &discovered.task_id,
            &discovered.card_id,
            "curious-chat",
            "key",
        )
        .await
        .unwrap_err();
        assert!(denied.contains("already_claimed"));
        assert_eq!(
            load_board(gcx, "task-discovery").await.unwrap().card_claims["T-1"].claimant_chat_id,
            "agent-a"
        );
    }

    #[tokio::test]
    async fn list_tasks_repairs_corrupt_index() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = setup_workspace(temp.path()).await;
        let tasks_dir = temp.path().join(".refact/tasks");
        let task_dir = tasks_dir.join("task-1");
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        let meta = test_meta("task-1", "Task", "2026-01-01T00:00:00Z");
        tokio::fs::write(
            task_dir.join("meta.yaml"),
            serde_yaml::to_string(&meta).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::write(crate::tasks::index::task_index_path(&tasks_dir), "not-json")
            .await
            .unwrap();

        let listed = list_tasks(gcx).await.unwrap();

        assert_eq!(listed.len(), 1);
        assert!(crate::tasks::index::read_task_index(&tasks_dir)
            .await
            .unwrap()
            .is_some());
    }
}
