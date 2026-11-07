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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CardSealState {
    pub column: String,
    pub agent_chat_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BoardStamp {
    modified: std::time::SystemTime,
    len: u64,
}

#[derive(Default)]
struct CachedCardStates {
    stamp: Option<BoardStamp>,
    cards: std::collections::HashMap<String, CardSealState>,
}

impl CachedCardStates {
    fn replace(&mut self, stamp: Option<BoardStamp>, board: &TaskBoard) {
        self.cards = board
            .cards
            .iter()
            .map(|card| {
                (
                    card.id.clone(),
                    CardSealState {
                        column: card.column.clone(),
                        agent_chat_id: card.agent_chat_id.clone(),
                    },
                )
            })
            .collect();
        self.stamp = stamp;
    }
}

#[derive(Default)]
pub(crate) struct CardStateCache {
    tasks: std::sync::Mutex<
        std::collections::HashMap<PathBuf, Arc<tokio::sync::Mutex<CachedCardStates>>>,
    >,
    parsed:
        std::sync::Mutex<std::collections::HashMap<PathBuf, (String, TaskBoard, BoardProvenance)>>,
    #[cfg(test)]
    parses: std::sync::atomic::AtomicUsize,
}

impl CardStateCache {
    fn task(&self, task_dir: &Path) -> Arc<tokio::sync::Mutex<CachedCardStates>> {
        self.tasks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(task_dir.to_path_buf())
            .or_default()
            .clone()
    }

    #[cfg(test)]
    pub(crate) fn parse_count(&self) -> usize {
        self.parses.load(std::sync::atomic::Ordering::Relaxed)
    }
}

async fn board_stamp(path: &Path) -> Result<Option<BoardStamp>, String> {
    match fs::metadata(path).await {
        Ok(meta) => Ok(Some(BoardStamp {
            modified: meta.modified().map_err(|error| error.to_string())?,
            len: meta.len(),
        })),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) async fn card_seal_state(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
) -> Result<Option<CardSealState>, String> {
    let task_dir = find_task_dir(gcx.clone(), task_id).await?;
    let task_dir = board_lock::canonical_dir(&task_dir).await?;
    let board_path = task_dir.join("board.yaml");
    let entry = gcx.task_card_states.task(&task_dir);
    {
        let cached = entry.lock().await;
        if let Some(stamp) = board_stamp(&board_path).await? {
            if cached.stamp.as_ref() == Some(&stamp) {
                return Ok(cached.cards.get(card_id).cloned());
            }
        }
    }
    let _guard = board_lock::acquire_board(&task_dir).await?;
    let mut cached = entry.lock().await;
    let stamp = board_stamp(&board_path).await?;
    if stamp.is_none() || cached.stamp != stamp {
        let board = read_board_from_dir(&gcx, &task_dir).await?;
        cached.replace(stamp, &board);
    }
    Ok(cached.cards.get(card_id).cloned())
}

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

#[derive(Clone)]
enum TaskPrincipal {
    Internal,
    OwnerPlanner(PlannerProof),
    BoundPlanner(PlannerProof),
}

#[derive(Clone)]
struct PlannerProof {
    principal: String,
    source_root: PathBuf,
    model: crate::caps::BaseModelRecord,
    origin: PlannerOrigin,
}

#[derive(Debug)]
enum PlannerProofError {
    Busy,
    Rejected(String),
}

impl From<String> for PlannerProofError {
    fn from(error: String) -> Self {
        Self::Rejected(error)
    }
}

impl From<&str> for PlannerProofError {
    fn from(error: &str) -> Self {
        Self::Rejected(error.to_string())
    }
}

impl From<PlannerProofError> for String {
    fn from(error: PlannerProofError) -> Self {
        match error {
            PlannerProofError::Busy => {
                static BUSY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
                BUSY.get_or_init(|| {
                    WorkError::new(
                        "planner_busy",
                        "Planner identity is busy; claim retained for retry",
                        true,
                    )
                    .to_string()
                })
                .clone()
            }
            PlannerProofError::Rejected(error) => error,
        }
    }
}

pub(crate) fn planner_proof_busy(error: &str) -> bool {
    error == String::from(PlannerProofError::Busy)
}

#[derive(Clone)]
enum PlannerOrigin {
    Live {
        session: Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
        worktree: Option<crate::worktrees::types::WorktreeMeta>,
    },
    Persisted {
        path: PathBuf,
        digest: String,
    },
}

async fn retry_planner_busy<T, F, Fut>(mut operation: F) -> Result<T, PlannerProofError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, PlannerProofError>>,
{
    let mut retry = 0;
    loop {
        match operation().await {
            Err(PlannerProofError::Busy) if retry < 4 => {
                tokio::time::sleep(std::time::Duration::from_millis(25 << retry)).await;
                retry += 1;
            }
            result => return result,
        }
    }
}

impl PlannerProof {
    async fn capture(
        gcx: Arc<GlobalContext>,
        task_id: &str,
        principal: &str,
    ) -> Result<Self, String> {
        retry_planner_busy(|| Self::capture_once(gcx.clone(), task_id, principal))
            .await
            .map_err(String::from)
    }

    async fn capture_once(
        gcx: Arc<GlobalContext>,
        task_id: &str,
        principal: &str,
    ) -> Result<Self, PlannerProofError> {
        if crate::file_activity::current_actor().is_some_and(|actor| actor != principal) {
            return Err("Task principal does not match the authenticated caller".into());
        }
        let live = gcx.chat_sessions.read().await.get(principal).cloned();
        if let Some(live) = live {
            let session = live.try_lock().map_err(|_| PlannerProofError::Busy)?;
            if session.chat_id != principal
                || !session
                    .thread
                    .task_meta
                    .as_ref()
                    .is_some_and(|meta| meta.role == "planner" && meta.task_id == task_id)
                || session.thread.model.trim().is_empty()
            {
                return Err("Task principal is not a bound planner with a model".into());
            }
            let source = crate::coordination::scope::session_source_root(&session)
                .ok_or("Task planner has no authenticated source project")?;
            let worktree = session.thread.worktree.clone();
            let model = crate::caps::BaseModelRecord {
                id: session.thread.model.clone(),
                ..Default::default()
            };
            let mut normalized_meta = session.thread.task_meta.clone();
            let mode = session.thread.mode.clone();
            let link_type = session.thread.link_type.clone();
            drop(session);
            crate::chat::trajectories::normalize_legacy_planner_copy(
                gcx.clone(),
                principal,
                &mode,
                link_type.as_deref(),
                &mut normalized_meta,
            )
            .await;
            if !normalized_meta
                .as_ref()
                .is_some_and(|meta| meta.role == "planner")
            {
                return Err("Task principal is a nonauthoritative legacy planner copy".into());
            }
            let source_root = crate::coordination::roots::resolve_source_workspace_root(
                gcx.clone(),
                source.to_str(),
            )
            .await?;
            if let Some(worktree) = &worktree {
                let service = crate::worktrees::service::WorktreeService::new_async(
                    gcx.cache_dir.clone(),
                    source_root.clone(),
                )
                .await?;
                if service.validate_worktree_meta_strict(worktree).await? != *worktree {
                    return Err("Task planner checkout identity changed".into());
                }
            }
            return Ok(Self {
                principal: principal.into(),
                source_root,
                model,
                origin: PlannerOrigin::Live {
                    session: live,
                    worktree,
                },
            });
        }
        if !crate::chat::trajectories::chat_id_is_planner_for_task(gcx.clone(), principal, task_id)
            .await
        {
            return Err("Task finalization requires an authenticated bound planner".into());
        }
        let path = crate::chat::trajectories::find_trajectory_path(gcx.clone(), principal)
            .await
            .ok_or("Task planner trajectory is unavailable")?;
        let app = AppState::from_gcx(gcx.clone()).await;
        let source_root = crate::coordination::scope::restored_source_root(&app, &path)
            .await
            .ok_or("Task planner trajectory has no authenticated source project")?;
        let path = dunce::canonicalize(path).map_err(|error| error.to_string())?;
        require_regular_task_file(&path).await?;
        let body = crate::coordination::board_bootstrap::read_bounded(
            &path,
            &source_root,
            crate::file_activity::settings().max_work_history_bytes,
        )
        .await
        .map_err(|_| "Task planner trajectory cannot be authenticated")?;
        let value: serde_json::Value =
            serde_json::from_str(&body).map_err(|error| error.to_string())?;
        if value.get("id").and_then(serde_json::Value::as_str) != Some(principal)
            || value
                .get("task_meta")
                .and_then(|value| value.get("task_id"))
                .and_then(serde_json::Value::as_str)
                != Some(task_id)
            || value
                .get("task_meta")
                .and_then(|value| value.get("role"))
                .and_then(serde_json::Value::as_str)
                != Some("planner")
            || value
                .get("model")
                .and_then(serde_json::Value::as_str)
                .is_none_or(|model| model.trim().is_empty())
        {
            return Err("Task planner saved binding changed".into());
        }
        Ok(Self {
            principal: principal.into(),
            source_root,
            model: crate::caps::BaseModelRecord {
                id: value
                    .get("model")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .into(),
                ..Default::default()
            },
            origin: PlannerOrigin::Persisted {
                path,
                digest: crate::file_activity::digest_of(&body),
            },
        })
    }

    async fn validate(
        &self,
        gcx: &Arc<GlobalContext>,
        task_id: &str,
        task_dir: &Path,
    ) -> Result<crate::caps::BaseModelRecord, PlannerProofError> {
        let live = gcx.chat_sessions.read().await.get(&self.principal).cloned();
        match (&self.origin, live) {
            (
                PlannerOrigin::Live {
                    session: accepted,
                    worktree,
                },
                Some(current),
            ) if Arc::ptr_eq(accepted, &current) => {
                let session = current.try_lock().map_err(|_| PlannerProofError::Busy)?;
                if session.chat_id != self.principal
                    || !session
                        .thread
                        .task_meta
                        .as_ref()
                        .is_some_and(|meta| meta.role == "planner" && meta.task_id == task_id)
                    || session.thread.model != self.model.id
                    || &session.thread.worktree != worktree
                    || crate::coordination::scope::session_source_root(&session)
                        .and_then(|path| dunce::canonicalize(path).ok())
                        .as_ref()
                        != Some(&self.source_root)
                {
                    return Err("Task planner source or task binding changed".into());
                }
                Ok(crate::caps::BaseModelRecord {
                    id: session.thread.model.clone(),
                    ..Default::default()
                })
            }
            (
                PlannerOrigin::Persisted {
                    path,
                    digest,
                },
                None,
            ) => {
                if !path.starts_with(task_dir) {
                    return Err(
                        "Saved planner does not belong to the accepted task directory".into(),
                    );
                }
                require_regular_task_file(path).await?;
                let body = crate::coordination::board_bootstrap::read_bounded(
                    path,
                    task_dir,
                    crate::file_activity::settings().max_work_history_bytes,
                )
                .await
                .map_err(|_| "Saved task planner identity unavailable")?;
                if crate::file_activity::digest_of(&body) != *digest {
                    return Err("Saved task planner revision changed; retry".into());
                }
                Ok(self.model.clone())
            }
            _ => Err("Task planner live identity changed; retry".into()),
        }
    }
}

#[derive(Clone)]
pub(crate) struct TaskDirectoryBinding {
    task_id: String,
    task_dir: PathBuf,
    source_root: PathBuf,
    source_identity: std::fs::Metadata,
    task_identity: std::fs::Metadata,
    owner: Option<String>,
    actor: Option<String>,
    principal: TaskPrincipal,
    attempt: Option<super::admission::Attempt>,
    claim: Option<AcceptedTaskClaim>,
}

#[derive(Clone)]
struct AcceptedTaskClaim {
    card_id: String,
    agent_id: String,
    variant: Option<&'static str>,
}

impl std::fmt::Debug for TaskDirectoryBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TaskDirectoryBinding")
            .field("task_id", &self.task_id)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for AcceptedTaskAttempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcceptedTaskAttempt")
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

pub(crate) fn bound_task_matches(task_id: Option<&str>) -> Result<bool, String> {
    match BOUND_TASK.try_with(|access| access.binding.task_id.clone()) {
        Ok(bound) => match task_id {
            Some(task) if bound == task => Ok(true),
            Some(_) => Err("Task transaction does not match the accepted binding".into()),
            None => Ok(false),
        },
        Err(_) => Ok(false),
    }
}

pub(crate) fn bound_task_active() -> bool {
    BOUND_TASK.try_with(|_| ()).is_ok()
}

#[derive(Clone)]
struct BoundTaskAccess {
    binding: TaskDirectoryBinding,
    reconciliation: bool,
}

tokio::task_local! {
    static BOUND_TASK: BoundTaskAccess;
}

#[cfg(test)]
tokio::task_local! {
    pub(super) static TASK_BOARD_LOCK_WAIT: Arc<tokio::sync::Notify>;
}

impl TaskDirectoryBinding {
    pub(crate) async fn acknowledge_custody_reply<F, T>(
        &self,
        gcx: Arc<GlobalContext>,
        planner: &str,
        updater: F,
    ) -> Result<T, String>
    where
        F: FnOnce(&mut TaskBoard) -> Result<T, String>,
    {
        self.validate_identity()?;
        let mut binding = self.clone();
        let board = read_board_from_dir(&gcx, &binding.task_dir).await?;
        if !board.planner_binding_is_current(planner) {
            return Err("Reply planner is no longer in board custody".into());
        }
        binding.owner = board.owner_chat_id;
        let access = BoundTaskAccess {
            binding,
            reconciliation: true,
        };
        BOUND_TASK
            .scope(
                access,
                update_board_atomic_with_reviewer(
                    gcx,
                    &self.task_id,
                    BoardWriter::BoundPlanner(
                        crate::file_activity::current_actor().unwrap_or_else(|| planner.into()),
                    ),
                    |board, _| {
                        if board.planner_binding_is_current(planner) {
                            Ok(())
                        } else {
                            Err("Reply planner custody changed".into())
                        }
                    },
                    |board, _| updater(board),
                ),
            )
            .await
            .map(|(_, value)| value)
    }

    pub(crate) async fn capture(
        gcx: Arc<GlobalContext>,
        source_root: &Path,
        task_id: &str,
    ) -> Result<Self, String> {
        let principal = match crate::file_activity::current_actor() {
            Some(actor) => TaskPrincipal::OwnerPlanner(
                PlannerProof::capture(gcx.clone(), task_id, &actor).await?,
            ),
            None => TaskPrincipal::Internal,
        };
        Self::capture_as(gcx, source_root, task_id, principal).await
    }

    async fn capture_as(
        gcx: Arc<GlobalContext>,
        source_root: &Path,
        task_id: &str,
        principal: TaskPrincipal,
    ) -> Result<Self, String> {
        validate_task_id(task_id)?;
        let source_root = crate::coordination::roots::resolve_source_workspace_root(
            gcx.clone(),
            Some(&source_root.to_string_lossy()),
        )
        .await?;
        let task_dir = find_task_dir(gcx.clone(), task_id).await?;
        let canonical = dunce::canonicalize(&task_dir).map_err(|e| e.to_string())?;
        let local = source_root.join(".refact/tasks").join(task_id);
        let global = gcx.config_dir.join("tasks").join(task_id);
        let accepted = [local, global]
            .into_iter()
            .find(|path| dunce::canonicalize(path).ok().as_ref() == Some(&canonical))
            .ok_or("Task directory does not belong to the accepted source project")?;
        for path in accepted.ancestors().take(3) {
            require_regular_directory(path).await?;
        }
        let actor = crate::file_activity::current_actor();
        if let TaskPrincipal::OwnerPlanner(proof) | TaskPrincipal::BoundPlanner(proof) = &principal
        {
            if proof.source_root != source_root {
                return Err("Task planner does not belong to the accepted source project".into());
            }
        }
        let identity = std::fs::metadata(&canonical).map_err(|e| e.to_string())?;
        let guard = board_lock::acquire_board(&canonical).await?;
        if guard.dir() != canonical || !same_directory_identity(&identity, &canonical)? {
            return Err("Accepted task directory changed before capture".into());
        }
        require_regular_task_file(&canonical.join("board.yaml")).await?;
        let board = read_board_from_dir(&gcx, &canonical).await?;
        if let TaskPrincipal::OwnerPlanner(proof) = &principal {
            if board.owner_chat_id.is_some() {
                board.require_owner(&proof.principal)?;
            }
        }
        if guard.dir() != canonical || !same_directory_identity(&identity, &canonical)? {
            return Err("Accepted task directory changed during capture".into());
        }
        let binding = Self {
            task_id: task_id.into(),
            task_identity: std::fs::metadata(&canonical).map_err(|e| e.to_string())?,
            source_identity: std::fs::metadata(&source_root).map_err(|e| e.to_string())?,
            task_dir: canonical,
            source_root,
            owner: board.owner_chat_id,
            actor,
            principal,
            attempt: None,
            claim: None,
        };
        drop(guard);
        binding.validate_identity()?;
        Ok(binding)
    }

    pub(crate) async fn scope<F: std::future::Future>(&self, future: F) -> F::Output {
        BOUND_TASK
            .scope(
                BoundTaskAccess {
                    binding: self.clone(),
                    reconciliation: false,
                },
                future,
            )
            .await
    }

    pub(crate) fn with_attempt(
        &self,
        attempt: &super::admission::Attempt,
        card_id: &str,
        agent_id: &str,
        variant: Option<&'static str>,
    ) -> Self {
        let mut binding = self.clone();
        binding.attempt = Some(attempt.clone());
        binding.claim = Some(AcceptedTaskClaim {
            card_id: card_id.into(),
            agent_id: agent_id.into(),
            variant,
        });
        binding
    }

    pub(crate) fn task_id(&self) -> &str {
        &self.task_id
    }

    pub(crate) fn validate_task(&self, task_id: &str) -> Result<(), String> {
        if self.task_id != task_id {
            return Err("Accepted task identity does not match the transaction".into());
        }
        self.validate_identity()
    }

    fn validate_identity(&self) -> Result<(), String> {
        if dunce::canonicalize(&self.task_dir).ok().as_ref() != Some(&self.task_dir)
            || !same_directory_identity(&self.task_identity, &self.task_dir)?
            || !same_directory_identity(&self.source_identity, &self.source_root)?
        {
            return Err(
                "Accepted task directory changed; claim retained for reconciliation".into(),
            );
        }
        Ok(())
    }

    fn require_owner(&self, board: &TaskBoard) -> Result<(), String> {
        if board.owner_chat_id != self.owner
            && !(self.owner.is_none() && self.actor.is_some() && board.owner_chat_id == self.actor)
        {
            return Err("Accepted task owner changed; claim retained for reconciliation".into());
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct AcceptedTaskAttempt {
    binding: TaskDirectoryBinding,
    attempt: super::admission::Attempt,
    original: BoardCard,
    agent_id: String,
    variant: Option<&'static str>,
}

impl AcceptedTaskAttempt {
    pub(crate) fn new(
        binding: &TaskDirectoryBinding,
        original: &BoardCard,
        agent_id: &str,
        attempt: super::admission::Attempt,
        variant: Option<&'static str>,
    ) -> Self {
        Self {
            binding: binding.clone(),
            attempt,
            original: original.clone(),
            agent_id: agent_id.into(),
            variant,
        }
    }

    pub(crate) async fn reconcile(
        &self,
        gcx: Arc<GlobalContext>,
        restore_original: bool,
        error: &str,
    ) -> Result<(), String> {
        let busy = planner_proof_busy(error);
        let access = BoundTaskAccess {
            binding: self.binding.clone(),
            reconciliation: true,
        };
        BOUND_TASK.scope(access, async {
            let reason: String = error.chars().take(300).collect();
            update_board_atomic_with_reviewer(
                gcx, &self.binding.task_id, BoardWriter::Internal, |_, _| Ok(()), |board, _| {
                    if !self.attempt.owns_setup() {
                        return Err("Accepted task attempt changed; reconciliation required".into());
                    }
                    let card = board.get_card_mut(&self.original.id)
                        .ok_or("Accepted task card was removed; reconciliation required")?;
                    if card.column != "doing" {
                        return Err("Accepted task claim is no longer current".into());
                    }
                    if busy {
                        let matches = if let Some(key) = self.variant {
                            card.ab_variants.as_ref().filter(|variants| variants.winner.is_none())
                                .and_then(|variants| variants.variant(key))
                                .is_some_and(|variant| variant.chat_id == self.attempt.chat_id()
                                    && variant.agent_id == self.agent_id && variant.finish.is_none())
                        } else {
                            card.agent_chat_id.as_deref() == Some(self.attempt.chat_id())
                                && card.assignee.as_deref() == Some(self.agent_id.as_str())
                        };
                        if !matches {
                            return Err("Accepted task claim was replaced".into());
                        }
                        card.attention = Some(super::types::CardAttention {
                            kind: "no_session".into(),
                            reason: "Planner busy during agent setup; inspect and restart from the owning planner".into(),
                            since: Utc::now().to_rfc3339(),
                            count: 1,
                        });
                        card.status_updates.push(super::types::StatusUpdate {
                            timestamp: Utc::now().to_rfc3339(),
                            message: "Planner busy during setup; claim and checkout retained. Inspect and restart_agent from the owning planner.".into(),
                        });
                        return Ok(());
                    }
                    if let Some(key) = self.variant {
                        let variant = card.ab_variants.as_mut()
                            .filter(|variants| variants.winner.is_none())
                            .and_then(|variants| variants.variant_mut(key))
                            .filter(|variant| variant.agent_id == self.agent_id
                                && variant.chat_id == self.attempt.chat_id() && variant.finish.is_none())
                            .ok_or("Accepted A/B claim was replaced")?;
                        variant.finish = Some(super::types::AbVariantFinish {
                            success: false, final_report: format!("A/B setup failed: {reason}"),
                            completed_at: Utc::now().to_rfc3339(), ..Default::default()
                        });
                    } else {
                        if card.assignee.as_deref() != Some(self.agent_id.as_str())
                            || card.agent_chat_id.as_deref() != Some(self.attempt.chat_id())
                        {
                            return Err("Accepted task claim was replaced".into());
                        }
                        if restore_original {
                            card.assignee = self.original.assignee.clone();
                            card.agent_chat_id = self.original.agent_chat_id.clone();
                            card.column = self.original.column.clone();
                            card.started_at = self.original.started_at.clone();
                            card.completed_at = self.original.completed_at.clone();
                            card.final_report = self.original.final_report.clone();
                            card.final_report_structured = self.original.final_report_structured.clone();
                            card.agent_worktree = self.original.agent_worktree.clone();
                            card.agent_worktree_name = self.original.agent_worktree_name.clone();
                            card.agent_branch = self.original.agent_branch.clone();
                            card.base_branch = self.original.base_branch.clone();
                            card.base_commit = self.original.base_commit.clone();
                        } else {
                            card.column = "failed".into();
                            card.completed_at = Some(Utc::now().to_rfc3339());
                            card.final_report = Some(format!("Agent setup failed: {reason}"));
                        }
                    }
                    card.status_updates.push(super::types::StatusUpdate {
                        timestamp: Utc::now().to_rfc3339(),
                        message: format!("Agent setup failed: {reason}. Previous checkout retained for inspection."),
                    });
                    Ok(())
                },
            ).await.map(|_| ())
        }).await
    }
}

pub(crate) fn same_directory_identity(
    before: &std::fs::Metadata,
    path: &Path,
) -> Result<bool, String> {
    let after = std::fs::symlink_metadata(path)
        .map_err(|e| format!("Accepted directory unavailable; reconciliation required: {e}"))?;
    if !after.is_dir()
        || after.file_type().is_symlink()
        || before.created().ok() != after.created().ok()
    {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != after.dev() || before.ino() != after.ino() {
            return Ok(false);
        }
    }
    #[cfg(not(unix))]
    if before.created().is_err() || after.created().is_err() {
        return Err("Accepted directory identity is unavailable".into());
    }
    Ok(true)
}

async fn require_regular_directory(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path)
        .await
        .map_err(|e| e.to_string())?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("Unsafe accepted task directory alias".into());
    }
    Ok(())
}

pub(crate) async fn require_regular_task_file(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path)
        .await
        .map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err("Accepted task file is not a regular non-symlink file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            return Err("Accepted task file has an unsafe hardlink alias".into());
        }
    }
    #[cfg(windows)]
    if task_meta_windows_link_count(path).await? != 1 {
        return Err("Accepted task file has an unsafe hardlink alias".into());
    }
    Ok(())
}

async fn resolve_bound_task_dir(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    access: &BoundTaskAccess,
) -> Result<PathBuf, String> {
    access.binding.validate_task(task_id)?;
    for path in access.binding.task_dir.ancestors().take(3) {
        require_regular_directory(path).await?;
    }
    if !access.reconciliation {
        crate::coordination::roots::resolve_source_workspace_root(
            gcx,
            Some(&access.binding.source_root.to_string_lossy()),
        )
        .await
        .map_err(|e| format!("Accepted task source is no longer authorized: {e}"))?;
        if crate::file_activity::current_actor() != access.binding.actor {
            return Err("Accepted task principal changed".into());
        }
    }
    Ok(access.binding.task_dir.clone())
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
    if let Ok(access) = BOUND_TASK.try_with(Clone::clone) {
        return resolve_bound_task_dir(gcx, task_id, &access).await;
    }
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
    if BOUND_TASK.try_with(|_| ()).is_ok() {
        let (task_dir, _guard) = locked_task_dir(gcx, task_id).await?;
        let meta_path = task_dir.join("meta.yaml");
        require_regular_task_file(&meta_path).await?;
        let meta = load_task_meta_from_path(&meta_path).await?;
        if meta.id != task_id {
            return Err("Accepted task metadata identity changed".into());
        }
        return Ok(meta);
    }
    let task_dir = find_task_dir(gcx, task_id).await?;
    load_task_meta_from_path(&task_dir.join("meta.yaml")).await
}

#[cfg(windows)]
async fn task_meta_windows_link_count(path: &Path) -> Result<u32, String> {
    use std::os::windows::io::AsRawHandle;

    #[repr(C)]
    #[derive(Default)]
    struct FileStandardInfo {
        allocation_size: i64,
        end_of_file: i64,
        number_of_links: u32,
        delete_pending: u8,
        directory: u8,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandleEx(
            file: std::os::windows::io::RawHandle,
            class: i32,
            info: *mut std::ffi::c_void,
            size: u32,
        ) -> i32;
    }

    const FILE_STANDARD_INFO_CLASS: i32 = 1;
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> std::io::Result<u32> {
        let file = std::fs::File::open(path)?;
        let mut info = FileStandardInfo::default();
        let result = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FILE_STANDARD_INFO_CLASS,
                std::ptr::from_mut(&mut info).cast(),
                std::mem::size_of::<FileStandardInfo>() as u32,
            )
        };
        if result == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(info.number_of_links)
    })
    .await
    .map_err(|error| format!("Failed to query task metadata links: {error}"))?
    .map_err(|error| format!("Failed to query task metadata links: {error}"))
}

pub async fn save_task_meta(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    meta: &TaskMeta,
) -> Result<(), String> {
    let (task_dir, _guard) = locked_task_dir(gcx, task_id)
        .await
        .map_err(|error| format!("Failed to lock task metadata {task_id}: {error}"))?;
    let meta_path = task_dir.join("meta.yaml");
    let permissions = match fs::symlink_metadata(&meta_path).await {
        Ok(metadata) => {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(format!(
                    "Task metadata is not a regular non-symlink file: {}",
                    meta_path.display()
                ));
            }
            #[cfg(unix)]
            let link_count = {
                use std::os::unix::fs::MetadataExt;
                metadata.nlink()
            };
            #[cfg(windows)]
            let link_count = u64::from(task_meta_windows_link_count(&meta_path).await.map_err(
                |error| {
                    format!(
                        "Failed to inspect task metadata {}: {error}",
                        meta_path.display()
                    )
                },
            )?);
            #[cfg(any(unix, windows))]
            if link_count != 1 {
                return Err(format!(
                    "Atomic publication of hardlinked task metadata is unsupported: {}",
                    meta_path.display()
                ));
            }
            #[cfg(not(any(unix, windows)))]
            return Err(format!(
                "Atomic publication of task metadata is unsupported on this platform: {}",
                meta_path.display()
            ));
            #[cfg(any(unix, windows))]
            {
                Some(metadata.permissions())
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "Failed to inspect task metadata {}: {error}",
                meta_path.display()
            ));
        }
    };
    let content = serde_yaml::to_string(meta).map_err(|error| {
        format!(
            "Failed to serialize task metadata {}: {error}",
            meta_path.display()
        )
    })?;
    crate::file_activity::write_temp_then_rename(&meta_path, content.as_bytes(), permissions)
        .await
        .map_err(|error| {
            format!(
                "Failed to publish task metadata {}: {error}",
                meta_path.display()
            )
        })?;
    crate::file_activity::sync_dir(&task_dir)
        .await
        .map_err(|error| {
            format!(
                "Task metadata {} was published but directory sync failed: {error}",
                meta_path.display()
            )
        })?;
    if let Some(tasks_dir) = task_dir.parent() {
        super::index::upsert_task_index_entry(tasks_dir, meta)
            .await
            .map_err(|error| {
                format!(
                    "Task metadata {} was published but index update failed: {error}",
                    meta_path.display()
                )
            })?;
    }
    Ok(())
}

pub async fn load_board(gcx: Arc<GlobalContext>, task_id: &str) -> Result<TaskBoard, String> {
    if BOUND_TASK.try_with(|_| ()).is_ok() {
        let (_, guard) = locked_task_dir(gcx.clone(), task_id).await?;
        return load_board_locked(gcx, task_id, &guard).await;
    }
    let task_dir = find_task_dir(gcx.clone(), task_id).await?;
    read_board_from_dir(&gcx, &task_dir).await
}

pub(crate) async fn load_board_unlocked(
    gcx: Arc<GlobalContext>,
    task_id: &str,
) -> Result<TaskBoard, String> {
    let task_dir = find_task_dir(gcx.clone(), task_id).await?;
    read_board_from_dir(&gcx, &task_dir).await
}

pub(crate) async fn load_board_locked(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    guard: &BoardLockGuard,
) -> Result<TaskBoard, String> {
    validate_task_id(task_id)?;
    validate_locked_task_dir(gcx.clone(), task_id, guard).await?;
    let task_dir = find_task_dir(gcx.clone(), task_id).await?;
    if dunce::canonicalize(task_dir).map_err(|error| error.to_string())? != guard.dir() {
        return Err("Board read attempted under a different task lock domain".into());
    }
    read_board_from_dir(&gcx, guard.dir()).await
}

async fn read_board_from_dir(gcx: &GlobalContext, task_dir: &Path) -> Result<TaskBoard, String> {
    read_board_with_provenance(gcx, task_dir)
        .await
        .map(|(board, _)| board)
}

async fn read_board_with_provenance(
    gcx: &GlobalContext,
    task_dir: &Path,
) -> Result<(TaskBoard, BoardProvenance), String> {
    let board_path = task_dir.join("board.yaml");
    if !board_path.exists() {
        return Ok((TaskBoard::default(), BoardProvenance::default()));
    }
    let content = fs::read_to_string(&board_path)
        .await
        .map_err(|e| e.to_string())?;
    let key = dunce::canonicalize(&board_path).map_err(|error| error.to_string())?;
    let digest = crate::file_activity::digest_of(&content);
    if let Some((_, board, provenance)) = gcx
        .task_card_states
        .parsed
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&key)
        .filter(|(cached, _, _)| cached == &digest)
    {
        return Ok((board.clone(), provenance.clone()));
    }
    #[cfg(test)]
    gcx.task_card_states
        .parses
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    #[cfg(not(test))]
    let _ = gcx;
    let value: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| e.to_string())?;
    let board: TaskBoard = serde_yaml::from_value(value.clone()).map_err(|e| e.to_string())?;
    let provenance: BoardProvenance = value
        .get("work_provenance")
        .and_then(|value| serde_yaml::from_value(value.clone()).ok())
        .unwrap_or_default();
    let mut parsed = gcx
        .task_card_states
        .parsed
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if parsed.len() >= 16 {
        parsed.clear();
    }
    parsed.insert(key, (digest, board.clone(), provenance.clone()));
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
    let entry = gcx.task_card_states.task(task_dir);
    let mut cached = entry.lock().await;
    crate::file_activity::write_file_durably(
        &board_path,
        content.as_bytes(),
        crate::file_activity::ReplaceMode::Replace,
    )
    .await?;
    cached.replace(board_stamp(&board_path).await.unwrap_or(None), board);
    gcx.work_boards.publish(task_dir, Some((board, provenance)));
    Ok(())
}

pub(crate) async fn locked_task_dir(
    gcx: Arc<GlobalContext>,
    task_id: &str,
) -> Result<(PathBuf, BoardLockGuard), String> {
    retry_planner_busy(|| locked_task_dir_once(gcx.clone(), task_id))
        .await
        .map_err(String::from)
}

async fn locked_task_dir_once(
    gcx: Arc<GlobalContext>,
    task_id: &str,
) -> Result<(PathBuf, BoardLockGuard), PlannerProofError> {
    let task_dir = find_task_dir(gcx.clone(), task_id).await?;
    let canonical_before = dunce::canonicalize(&task_dir).map_err(|e| e.to_string())?;
    let identity = std::fs::metadata(&canonical_before).map_err(|e| e.to_string())?;
    #[cfg(not(test))]
    let guard = board_lock::acquire_board(&canonical_before).await?;
    #[cfg(test)]
    let guard = {
        use std::future::Future;
        let mut acquire = std::pin::pin!(board_lock::acquire_board(&canonical_before));
        let mut notified = false;
        std::future::poll_fn(|cx| {
            let result = acquire.as_mut().poll(cx);
            if result.is_pending() && !notified {
                notified = true;
                let _ = TASK_BOARD_LOCK_WAIT.try_with(|waiting| waiting.notify_one());
            }
            result
        })
        .await?
    };
    let canonical = guard.dir().to_path_buf();
    if canonical != canonical_before || !same_directory_identity(&identity, &canonical)? {
        return Err(
            "Task directory changed while awaiting its lock; reconciliation required".into(),
        );
    }
    validate_locked_task_dir(gcx, task_id, &guard).await?;
    Ok((canonical, guard))
}

async fn validate_locked_task_dir(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    guard: &BoardLockGuard,
) -> Result<(), PlannerProofError> {
    if !guard.guards(BOARD_LOCK_FILE) {
        return Err("Task validation requires its board lock".into());
    }
    let canonical = guard.dir();
    if let Ok(access) = BOUND_TASK.try_with(Clone::clone) {
        let checked = resolve_bound_task_dir(gcx.clone(), task_id, &access).await?;
        if checked != canonical {
            return Err("Accepted task lock domain changed".into());
        }
        require_regular_task_file(&canonical.join("board.yaml")).await?;
        require_regular_task_file(&canonical.join("meta.yaml")).await?;
        if load_task_meta_from_path(&canonical.join("meta.yaml"))
            .await?
            .id
            != task_id
        {
            return Err("Accepted task metadata identity changed".into());
        }
        let board = read_board_from_dir(&gcx, canonical).await?;
        access.binding.require_owner(&board)?;
        if !access.reconciliation {
            if access
                .binding
                .attempt
                .as_ref()
                .is_some_and(|attempt| !attempt.owns_setup())
            {
                return Err("Accepted task admission attempt changed".into());
            }
            if let TaskPrincipal::OwnerPlanner(proof) | TaskPrincipal::BoundPlanner(proof) =
                &access.binding.principal
            {
                let policy = crate::coordination::privacy_guard::policy_key(&gcx, &proof.model)
                    .map_err(|error| error.to_string())?;
                let (_, provenance) = read_board_with_provenance(&gcx, canonical).await?;
                let mut records = crate::privacy::records::declared_file_records(
                    &gcx,
                    [canonical.join("board.yaml"), canonical.join("meta.yaml")],
                )?;
                records.extend(provenance.records);
                let model = proof.validate(&gcx, task_id, canonical).await?;
                crate::privacy::destinations::clear_for_model(
                    &gcx,
                    DiscoveryAudit(records),
                    &model,
                )
                .map_err(|error| error.to_string())?;
                if crate::coordination::privacy_guard::policy_key(&gcx, &model)
                    .map_err(|error| error.to_string())?
                    != policy
                {
                    return Err("Accepted task privacy policy changed".into());
                }
            }
        }
    }
    Ok(())
}

pub(crate) async fn validate_task_trajectory_publication(
    gcx: Arc<GlobalContext>,
    guard: &BoardLockGuard,
    task_meta: &crate::chat::types::TaskMeta,
    chat_id: &str,
    file_path: &Path,
) -> Result<(), String> {
    let access = BOUND_TASK
        .try_with(Clone::clone)
        .map_err(|_| "Accepted task publication binding is unavailable")?;
    let task_dir = guard.dir();
    access.binding.validate_task(&task_meta.task_id)?;
    if access.reconciliation
        || !guard.guards(BOARD_LOCK_FILE)
        || task_dir != access.binding.task_dir
        || !file_path.starts_with(task_dir)
    {
        return Err("Accepted task trajectory escaped its publication binding".into());
    }
    if let Some(attempt) = &access.binding.attempt {
        let claim = access
            .binding
            .claim
            .as_ref()
            .ok_or("Accepted task publication claim is unavailable")?;
        if !attempt.owns_setup()
            || attempt.chat_id() != chat_id
            || task_meta.role != "agents"
            || task_meta.card_id.as_deref() != Some(claim.card_id.as_str())
            || task_meta.agent_id.as_deref() != Some(claim.agent_id.as_str())
        {
            return Err("Accepted task snapshot does not match its admission claim".into());
        }
        let board = read_board_from_dir(&gcx, task_dir).await?;
        let card = board
            .get_card(&claim.card_id)
            .filter(|card| card.column == "doing")
            .ok_or("Accepted task publication claim is no longer current")?;
        if let Some(key) = claim.variant {
            card.ab_variants
                .as_ref()
                .filter(|variants| variants.winner.is_none())
                .and_then(|variants| variants.variant(key))
                .filter(|variant| {
                    variant.agent_id == claim.agent_id
                        && variant.chat_id == chat_id
                        && variant.finish.is_none()
                })
                .ok_or("Accepted A/B publication claim was replaced")?;
        } else if card.ab_variants.is_some()
            || card.assignee.as_deref() != Some(claim.agent_id.as_str())
            || card.agent_chat_id.as_deref() != Some(chat_id)
        {
            return Err("Accepted task publication claim was replaced".into());
        }
    } else if task_meta.role == "agents" {
        return Err("Accepted task publication admission attempt is unavailable".into());
    }
    validate_locked_task_dir(gcx, &task_meta.task_id, guard).await?;
    access.binding.validate_identity()?;
    if access
        .binding
        .attempt
        .as_ref()
        .is_some_and(|attempt| !attempt.owns_setup())
    {
        return Err("Accepted task admission attempt changed".into());
    }
    Ok(())
}

fn record_board_withdrawals(
    gcx: &GlobalContext,
    task_id: &str,
    existing: &TaskBoard,
    board: &mut TaskBoard,
) -> Vec<(BoardCard, Vec<String>)> {
    let mut withdrawals = Vec::new();
    for previous in &existing.cards {
        let ended = match board.get_card(&previous.id) {
            None => previous.column == "doing",
            Some(next) => {
                (previous.column == "doing" && next.column != "doing")
                    || (previous.column != "doing" && next.column == "doing")
                    || (previous.column == "doing"
                        && previous.agent_chat_id.as_ref().is_some_and(|old| {
                            next.agent_chat_id.as_ref().is_some_and(|new| old != new)
                        }))
            }
        };
        if ended {
            let replied =
                crate::tools::tool_agent_planner_qna::canonical_reply_ids(board, &previous.id);
            let mut removed = previous.clone();
            let card = board.get_card_mut(&previous.id).unwrap_or(&mut removed);
            let ids = crate::tools::tool_agent_planner_qna::record_withdrawn_questions(
                gcx, task_id, card, &replied,
            );
            withdrawals.push((card.clone(), ids));
        }
    }
    withdrawals
}

async fn commit_board(
    gcx: &Arc<GlobalContext>,
    guard: BoardLockGuard,
    task_dir: &Path,
    task_id: &str,
    existing: &TaskBoard,
    board: &TaskBoard,
    provenance: &BoardProvenance,
    withdrawals: Vec<(BoardCard, Vec<String>)>,
) -> Result<(), String> {
    write_board_locked(gcx, &guard, task_dir, board, provenance).await?;
    for (card, ids) in withdrawals {
        crate::tools::tool_agent_planner_qna::settle_withdrawn_questions(gcx, task_id, &card, &ids);
    }
    drop(guard);
    let changed = existing
        .cards
        .iter()
        .filter(|card| {
            card.column == "doing"
                && board.get_card(&card.id).is_none_or(|next| {
                    next.column != "doing" || next.agent_chat_id != card.agent_chat_id
                })
        })
        .map(|card| card.id.as_str())
        .collect::<HashSet<_>>();
    if changed.is_empty() {
        return Ok(());
    }
    let mut stopped = HashSet::new();
    for card in existing
        .cards
        .iter()
        .filter(|card| changed.contains(card.id.as_str()))
    {
        if let Some(chat_id) = card.agent_chat_id.as_deref() {
            if board.get_card(&card.id).is_none_or(|next| {
                next.column != "doing"
                    || next
                        .agent_chat_id
                        .as_deref()
                        .is_some_and(|id| id != chat_id)
            }) && stopped.insert(chat_id.to_string())
            {
                crate::chat::notifications::stop_task_chat_processes(
                    gcx, chat_id, task_id, &card.id,
                )
                .await;
            }
        }
    }
    for process in gcx
        .exec_registry
        .list(Default::default())
        .await
        .into_iter()
        .filter(|process| !process.status.is_terminal())
    {
        let Some(chat_id) = process
            .meta
            .owner
            .chat_id
            .filter(|id| !stopped.contains(id))
        else {
            continue;
        };
        let session = gcx.chat_sessions.read().await.get(&chat_id).cloned();
        let Some(session) = session else {
            continue;
        };
        let meta = match session.try_lock() {
            Ok(session) => session.thread.task_meta.clone(),
            Err(_) => continue,
        };
        let Some(meta) = meta.filter(|meta| meta.task_id == task_id && meta.role == "agents")
        else {
            continue;
        };
        let Some(card_id) = meta.card_id.filter(|id| changed.contains(id.as_str())) else {
            continue;
        };
        if board.get_card(&card_id).is_none_or(|card| {
            card.column != "doing"
                || card
                    .agent_chat_id
                    .as_deref()
                    .is_some_and(|id| id != chat_id)
        }) && stopped.insert(chat_id.clone())
        {
            crate::chat::notifications::stop_task_chat_processes(gcx, &chat_id, task_id, &card_id)
                .await;
        }
    }
    Ok(())
}

async fn board_coordination(
    gcx: Arc<GlobalContext>,
    task_id: &str,
) -> Result<refact_work_core::CoordinationSettings, String> {
    let task_dir = find_task_dir(gcx, task_id).await?;
    board_coordination_at(&task_dir).await
}

async fn board_coordination_at(
    task_dir: &Path,
) -> Result<refact_work_core::CoordinationSettings, String> {
    let project_root = task_dir
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or_else(|| format!("Task directory {} has no project root", task_dir.display()))?;
    crate::runtime_settings::coordination_for_project(project_root).await
}

fn require_reply_metadata_unchanged(existing: &TaskBoard, board: &TaskBoard) -> Result<(), String> {
    if existing.planner_replies != board.planner_replies
        || existing.card_attempt_generations != board.card_attempt_generations
    {
        return Err("Planner reply metadata is coordinator-owned".into());
    }
    Ok(())
}

fn demote_untrusted_reply_statuses(existing: &TaskBoard, board: &mut TaskBoard) {
    for card in &mut board.cards {
        for (index, update) in card.status_updates.iter_mut().enumerate() {
            let unchanged = existing
                .get_card(&card.id)
                .and_then(|card| card.status_updates.get(index))
                .is_some_and(|old| {
                    old.timestamp == update.timestamp && old.message == update.message
                });
            if !unchanged
                && (update.message.starts_with("[REPLY:")
                    || update.message.starts_with("[REPLY-DELIVERY:"))
            {
                update.message.insert_str(0, "[UNTRUSTED STATUS] ");
            }
        }
    }
}

fn advance_card_attempt_generations(
    existing: &TaskBoard,
    board: &mut TaskBoard,
) -> Result<(), String> {
    let next = existing
        .card_attempt_generations
        .values()
        .copied()
        .chain(
            existing
                .planner_replies
                .values()
                .flat_map(|rows| rows.values())
                .map(|reply| reply.attempt.generation),
        )
        .max()
        .unwrap_or(existing.rev)
        .max(existing.rev)
        .checked_add(1)
        .ok_or("Task board generation exhausted")?;
    board.card_attempt_generations = board
        .cards
        .iter()
        .map(|card| {
            let unchanged = existing.get_card(&card.id).is_some_and(|old| {
                old.assignee == card.assignee
                    && old.agent_chat_id == card.agent_chat_id
                    && old.started_at == card.started_at
                    && old.column == card.column
            });
            let generation = if unchanged {
                existing
                    .card_attempt_generations
                    .get(&card.id)
                    .copied()
                    .unwrap_or(0)
            } else {
                next
            };
            (card.id.clone(), generation)
        })
        .collect();
    Ok(())
}

pub(crate) async fn bound_planner_binding(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    principal: &str,
) -> Result<TaskDirectoryBinding, String> {
    let proof = PlannerProof::capture(gcx.clone(), task_id, principal).await?;
    TaskDirectoryBinding::capture_as(
        gcx,
        &proof.source_root.clone(),
        task_id,
        TaskPrincipal::BoundPlanner(proof),
    )
    .await
}

pub(crate) async fn accept_planner_reply<T>(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    accept: impl FnOnce(&TaskBoard) -> Result<T, String>,
) -> Result<T, String> {
    let (_, guard) = locked_task_dir(gcx.clone(), task_id).await?;
    let board = load_board_locked(gcx, task_id, &guard).await?;
    let result = accept(&board);
    drop(guard);
    result
}

fn validate_board(
    board: &TaskBoard,
    settings: &refact_work_core::CoordinationSettings,
) -> Result<(), String> {
    if crate::coordination::bounds::serialized_size(
        &(&board.planner_replies, &board.card_attempt_generations),
        settings.max_work_history_bytes,
    )
    .is_none()
        || board
            .planner_replies
            .values()
            .map(|rows| rows.len())
            .sum::<usize>()
            > settings.max_work_history_rows
    {
        return Err("Planner reply metadata exceeds the configured limit".into());
    }
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
    let source = board_rows::actor_provenance(&gcx).await?;
    let (task_dir, guard) = locked_task_dir(gcx.clone(), task_id).await?;
    let settings = board_coordination_at(&task_dir).await?;
    validate_board(board, &settings)?;
    let existing = read_board_from_dir(&gcx, &task_dir).await?;
    if task_dir.join("board.yaml").exists() {
        return Err(BoardSaveError::Conflict(BoardConflict {
            expected: 0,
            actual: existing.rev,
        }));
    }
    require_reply_metadata_unchanged(&existing, board)?;
    let mut board = board.clone();
    demote_untrusted_reply_statuses(&existing, &mut board);
    advance_card_attempt_generations(&existing, &mut board)?;
    validate_board(&board, &settings)?;
    let withdrawals = record_board_withdrawals(&gcx, task_id, &existing, &mut board);
    let provenance = board_rows::write_provenance(&board, None, source);
    commit_board(
        &gcx,
        guard,
        &task_dir,
        task_id,
        &existing,
        &board,
        &provenance,
        withdrawals,
    )
    .await?;
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
    let source = board_rows::actor_provenance(&gcx).await?;
    let (task_dir, guard) = locked_task_dir(gcx.clone(), task_id).await?;
    let settings = board_coordination_at(&task_dir).await?;
    validate_board(board, &settings)?;
    let (existing, previous) = read_board_with_provenance(&gcx, &task_dir).await?;
    if existing.rev != expected_rev {
        return Err(BoardSaveError::Conflict(BoardConflict {
            expected: expected_rev,
            actual: existing.rev,
        }));
    }
    require_reply_metadata_unchanged(&existing, board)?;
    let mut board = board.clone();
    demote_untrusted_reply_statuses(&existing, &mut board);
    advance_card_attempt_generations(&existing, &mut board)?;
    validate_board(&board, &settings)?;
    let withdrawals = record_board_withdrawals(&gcx, task_id, &existing, &mut board);
    let provenance = board_rows::write_provenance(&board, Some((&existing, &previous)), source);
    commit_board(
        &gcx,
        guard,
        &task_dir,
        task_id,
        &existing,
        &board,
        &provenance,
        withdrawals,
    )
    .await?;
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
    update_board_atomic_with_reviewer(
        gcx,
        task_id,
        planner_actor.map_or(BoardWriter::Internal, BoardWriter::Planner),
        |_, _| Ok(()),
        |board, _| updater(board),
    )
    .await
}

pub(crate) async fn update_board_atomic_with_owner_guard<F, G, T>(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    expected_owner: &str,
    owner_guard: G,
    updater: F,
) -> Result<(TaskBoard, T), String>
where
    F: FnOnce(&mut TaskBoard) -> Result<T, String>,
    G: FnOnce(&TaskBoard) -> Result<(), String>,
{
    update_board_atomic_as_bound_planner(gcx, task_id, expected_owner, |board, _| {
        if board.owner_chat_id.is_some() {
            board.require_owner(expected_owner)?;
        }
        owner_guard(board)?;
        updater(board)
    })
    .await
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
        BoardWriter::Reviewer(reviewer_chat_id.to_string()),
        |_, _| Ok(()),
        |board, _| updater(board),
    )
    .await
}

pub(crate) async fn update_board_atomic_as_bound_planner<F, T>(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    principal: &str,
    updater: F,
) -> Result<(TaskBoard, T), String>
where
    F: FnOnce(&mut TaskBoard, bool) -> Result<T, String>,
{
    let proof = PlannerProof::capture(gcx.clone(), task_id, principal).await?;
    let binding = TaskDirectoryBinding::capture_as(
        gcx.clone(),
        &proof.source_root.clone(),
        task_id,
        TaskPrincipal::BoundPlanner(proof),
    )
    .await?;
    binding
        .scope(update_board_atomic_with_reviewer(
            gcx,
            task_id,
            BoardWriter::BoundPlanner(principal.into()),
            |_, _| Ok(()),
            updater,
        ))
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

enum BoardWriter {
    Internal,
    Planner(String),
    Reviewer(String),
    BoundPlanner(String),
}

async fn update_board_atomic_with_reviewer<F, G, T>(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    writer: BoardWriter,
    pre_authorize: G,
    updater: F,
) -> Result<(TaskBoard, T), String>
where
    F: FnOnce(&mut TaskBoard, bool) -> Result<T, String>,
    G: FnOnce(&TaskBoard, &refact_work_core::CoordinationSettings) -> Result<(), String>,
{
    let reconciling = BOUND_TASK
        .try_with(|access| access.reconciliation)
        .unwrap_or(false);
    let source = if reconciling {
        None
    } else {
        board_rows::actor_provenance(&gcx).await?
    };
    let (task_dir, guard) = locked_task_dir(gcx.clone(), task_id).await?;
    let settings = board_coordination_at(&task_dir).await?;

    let (existing, previous) = read_board_with_provenance(&gcx, &task_dir).await?;
    pre_authorize(&existing, &settings)?;
    let mut board = existing.clone();
    let failed_before: HashSet<String> = board
        .cards
        .iter()
        .filter(|card| card.column == "failed" || card.column == "regressed")
        .map(|card| card.id.clone())
        .collect();
    match &writer {
        BoardWriter::Internal => {}
        BoardWriter::Planner(actor) => board.authorize_owner(actor)?,
        BoardWriter::Reviewer(actor) => {
            board.authorize_reviewer(actor, settings.board_allow_self_review)?
        }
        BoardWriter::BoundPlanner(principal) => {
            if crate::file_activity::current_actor().is_some_and(|actor| actor != *principal) {
                return Err("Task principal changed".into());
            }
        }
    }
    let result = updater(&mut board, settings.board_allow_self_review)?;
    let reopened = existing
        .cards
        .iter()
        .filter(|old| {
            old.column == "failed"
                && board
                    .get_card(&old.id)
                    .is_some_and(|new| new.column == "planned")
        })
        .map(|card| card.id.clone())
        .collect::<Vec<_>>();
    for card_id in reopened {
        board.card_claims.remove(&card_id);
        if let Some(card) = board.get_card_mut(&card_id) {
            card.assignee = None;
            card.agent_chat_id = None;
            card.started_at = None;
            card.completed_at = None;
            card.ab_variants = None;
            card.team_members.clear();
        }
    }
    if board.card_attempt_generations != existing.card_attempt_generations
        || (!matches!(&writer, BoardWriter::BoundPlanner(_))
            && board.planner_replies != existing.planner_replies)
    {
        return Err("Planner reply metadata is coordinator-owned".into());
    }
    if !matches!(&writer, BoardWriter::BoundPlanner(_)) {
        demote_untrusted_reply_statuses(&existing, &mut board);
    }
    advance_card_attempt_generations(&existing, &mut board)?;
    for card in &mut board.cards {
        if card.column != "doing"
            || existing.get_card(&card.id).is_some_and(|old| {
                card.agent_chat_id.is_some() && card.agent_chat_id != old.agent_chat_id
            })
        {
            card.paused_at = None;
            card.paused_by = None;
        }
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
    let withdrawals = record_board_withdrawals(&gcx, task_id, &existing, &mut board);
    let provenance = board_rows::write_provenance(&board, Some((&existing, &previous)), source);
    commit_board(
        &gcx,
        guard,
        &task_dir,
        task_id,
        &existing,
        &board,
        &provenance,
        withdrawals,
    )
    .await?;
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

#[derive(Debug, serde::Serialize)]
pub(crate) struct DiscoveredClaim {
    pub claimant_chat_id: String,
    pub claimed_at: String,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct DiscoveredOverride {
    pub overridden_by: String,
    pub overridden_at: String,
    pub unsatisfied: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct ModelBoardAuthority {
    pub task_id: String,
    pub rev: u64,
    pub owner_chat_id: Option<String>,
    pub reviewer_chat_ids: Vec<String>,
    pub review_policy: BoardReviewPolicy,
    pub claims: std::collections::BTreeMap<String, DiscoveredClaim>,
    pub dependency_overrides: std::collections::BTreeMap<String, DiscoveredOverride>,
}

struct DiscoveryAudit(Vec<refact_privacy::FileRecord>);

impl refact_privacy::PrivacyAudited for DiscoveryAudit {
    fn privacy_records(
        &self,
    ) -> Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError> {
        Ok(self.0.iter().cloned().enumerate().collect())
    }
}

pub(crate) async fn discover_model_cards(
    gcx: &Arc<GlobalContext>,
    caller: &crate::coordination::scope::WorkCaller,
) -> Result<(Vec<DiscoveredCard>, Vec<refact_privacy::FileRecord>, bool), WorkError> {
    let model = caller
        .model
        .as_ref()
        .ok_or_else(WorkError::policy_unavailable)?;
    let before = crate::coordination::privacy_guard::policy_key(gcx, model)?;
    let (mut rows, unavailable) = gcx.work_boards.rows(&caller.source_root).await;
    for row in &mut rows {
        let task_id = row
            .metadata
            .task_id
            .as_deref()
            .ok_or_else(WorkError::not_visible)?;
        validate_task_id(task_id).map_err(|_| WorkError::not_visible())?;
        row.declared_files.push(
            caller
                .source_root
                .join(".refact/tasks")
                .join(task_id)
                .join("board.yaml")
                .to_string_lossy()
                .into_owned(),
        );
        row.declared_files
            .extend(row.source_records.iter().map(|record| record.path.clone()));
    }
    let rows = crate::coordination::privacy_guard::clear_rows(gcx, rows, model)?;
    let mut records = Vec::new();
    let mut cards = Vec::new();
    for row in rows {
        records.extend(row.source_records);
        if let (Some(task_id), Some(card_id)) = (row.metadata.task_id, row.metadata.card_id) {
            cards.push(DiscoveredCard {
                task_id,
                card_id,
                title: row.title,
                column: row.metadata.runtime_status,
                agent_chat_id: row.chat_id,
            });
        }
    }
    cards.sort_by(|a, b| {
        a.task_id
            .cmp(&b.task_id)
            .then_with(|| a.card_id.cmp(&b.card_id))
    });
    cards.dedup_by(|a, b| a.task_id == b.task_id && a.card_id == b.card_id);
    if crate::coordination::privacy_guard::policy_key(gcx, model)? != before {
        return Err(WorkError::policy_unavailable());
    }
    Ok((cards, records, unavailable))
}

pub(crate) async fn discover_model_authority(
    gcx: &Arc<GlobalContext>,
    caller: &crate::coordination::scope::WorkCaller,
    task_id: &str,
) -> Result<(ModelBoardAuthority, Vec<refact_privacy::FileRecord>), WorkError> {
    validate_task_id(task_id).map_err(|_| WorkError::not_visible())?;
    let model = caller
        .model
        .as_ref()
        .ok_or_else(WorkError::policy_unavailable)?;
    let before = crate::coordination::privacy_guard::policy_key(gcx, model)?;
    let path = caller
        .source_root
        .join(".refact/tasks")
        .join(task_id)
        .join("board.yaml");
    for directory in [path.parent(), path.parent().and_then(Path::parent)]
        .into_iter()
        .flatten()
    {
        let metadata = fs::symlink_metadata(directory)
            .await
            .map_err(|_| WorkError::not_visible())?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(WorkError::not_visible());
        }
    }
    let body = crate::coordination::board_bootstrap::read_bounded(
        &path,
        &caller.source_root,
        crate::file_activity::settings().max_work_history_bytes,
    )
    .await
    .map_err(|_| WorkError::not_visible())?;
    let value: serde_yaml::Value =
        serde_yaml::from_str(&body).map_err(|_| WorkError::not_visible())?;
    let board: TaskBoard =
        serde_yaml::from_value(value.clone()).map_err(|_| WorkError::not_visible())?;
    let provenance: BoardProvenance = value
        .get("work_provenance")
        .and_then(|value| serde_yaml::from_value(value.clone()).ok())
        .ok_or_else(WorkError::not_visible)?;
    let settings = crate::file_activity::settings();
    if !provenance.authority_valid_for(&board, &settings)
        || board_rows::rows_from_board(&caller.source_root, task_id, &board, &provenance, &settings)
            .is_none()
    {
        return Err(WorkError::not_visible());
    }
    let mut records = crate::privacy::records::declared_file_records(
        gcx,
        std::iter::once(path).chain(
            provenance
                .records
                .iter()
                .map(|record| PathBuf::from(&record.path)),
        ),
    )
    .map_err(|_| WorkError::policy_unavailable())?;
    records.extend(provenance.records);
    crate::privacy::destinations::clear_for_model(gcx, DiscoveryAudit(records.clone()), model)
        .map_err(|_| WorkError::not_visible())?;
    if crate::coordination::privacy_guard::policy_key(gcx, model)? != before {
        return Err(WorkError::policy_unavailable());
    }
    Ok((
        ModelBoardAuthority {
            task_id: task_id.to_string(),
            rev: board.rev,
            owner_chat_id: board.owner_chat_id,
            reviewer_chat_ids: board.reviewer_chat_ids,
            review_policy: board.review_policy,
            claims: board
                .card_claims
                .into_iter()
                .map(|(id, claim)| {
                    (
                        id,
                        DiscoveredClaim {
                            claimant_chat_id: claim.claimant_chat_id,
                            claimed_at: claim.claimed_at,
                        },
                    )
                })
                .collect(),
            dependency_overrides: board
                .dependency_overrides
                .into_iter()
                .map(|(id, value)| {
                    (
                        id,
                        DiscoveredOverride {
                            overridden_by: value.overridden_by,
                            overridden_at: value.overridden_at,
                            unsatisfied: value.unsatisfied,
                        },
                    )
                })
                .collect(),
        },
        records,
    ))
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
    gcx.task_card_states
        .tasks
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(guard.dir());
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
    let mut trajectory_dirs = vec![get_task_trajectory_dir(&task_dir, role, agent_id)];
    if role == "planner"
        && agent_id.is_none()
        && caller == crate::chat::trajectory_index::TrajectoryIndexListingCaller::TaskTrajectoryApi
    {
        trajectory_dirs.push(get_task_trajectory_dir(&task_dir, "linked", None));
    }

    let coordinator = AppState::from_gcx(gcx.clone())
        .await
        .chat
        .trajectory_index_coordinator;
    let mut entries = Vec::new();
    for traj_dir in trajectory_dirs {
        if !traj_dir.exists() {
            continue;
        }
        entries.extend(
            crate::chat::trajectory_index::list_trajectory_entries_with_rollout_shared_for(
                &coordinator,
                &traj_dir,
                None,
                caller,
            )
            .await?
            .iter()
            .cloned(),
        );
    }
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
    #[tokio::test]
    async fn reply_ack_binding_follows_custody_without_granting_old_owner_writes() {
        let temp = tempfile::tempdir().unwrap();
        let task = "reply-ack-custody";
        let gcx = task_with_cards(temp.path(), task, vec![]).await;
        update_board_atomic(gcx.clone(), task, |board| {
            board.transfer_owner("old", 1);
            Ok(())
        })
        .await
        .unwrap();
        let binding = TaskDirectoryBinding::capture(gcx.clone(), temp.path(), task)
            .await
            .unwrap();
        update_board_atomic(gcx.clone(), task, |board| {
            board.transfer_owner("new", 2);
            Ok(())
        })
        .await
        .unwrap();
        binding
            .acknowledge_custody_reply(gcx.clone(), "old", |board| {
                board.planner_replies.entry("card".into()).or_default();
                Ok(())
            })
            .await
            .unwrap();
        let board = load_board(gcx.clone(), task).await.unwrap();
        assert!(board.planner_replies.contains_key("card"));
        assert!(board.require_owner("old").is_err());
        assert!(binding
            .acknowledge_custody_reply(gcx, "foreign", |_| Ok(()))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn busy_setup_reconciliation_retains_claim_with_durable_attention() {
        let temp = tempfile::tempdir().unwrap();
        let task = "busy-setup-attention";
        let gcx = task_with_cards(
            temp.path(),
            task,
            vec![claimable_card("T-1", vec![], "planned")],
        )
        .await;
        let app = AppState::from_gcx(gcx.clone()).await;
        let chat = format!("busy-{}", Uuid::new_v4());
        super::super::admission::reserve(&app, &chat, "project", "planner", 0)
            .await
            .unwrap();
        let binding = TaskDirectoryBinding::capture(gcx.clone(), temp.path(), task)
            .await
            .unwrap();
        let original = load_board(gcx.clone(), task).await.unwrap().cards[0].clone();
        let receipt = AcceptedTaskAttempt::new(
            &binding,
            &original,
            "agent",
            super::super::admission::current_attempt(&chat).unwrap(),
            None,
        );
        update_board_atomic(gcx.clone(), task, |board| {
            let card = &mut board.cards[0];
            card.column = "doing".into();
            card.assignee = Some("agent".into());
            card.agent_chat_id = Some(chat.clone());
            Ok(())
        })
        .await
        .unwrap();
        receipt
            .reconcile(gcx.clone(), false, &String::from(PlannerProofError::Busy))
            .await
            .unwrap();
        super::super::admission::end(&chat).await;
        let board = load_board(gcx, task).await.unwrap();
        assert_eq!(board.cards[0].column, "doing");
        assert_eq!(board.cards[0].agent_chat_id.as_deref(), Some(chat.as_str()));
        assert_eq!(
            board.cards[0].attention.as_ref().unwrap().kind,
            "no_session"
        );
        assert!(board.cards[0].completed_at.is_none());
    }

    #[tokio::test]
    async fn live_planner_lock_contention_is_typed_retryable() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let session = Arc::new(tokio::sync::Mutex::new(
            crate::chat::types::ChatSession::new("busy-planner".into()),
        ));
        gcx.chat_sessions
            .write()
            .await
            .insert("busy-planner".into(), session.clone());
        let _held = session.lock().await;
        let error = match PlannerProof::capture_once(gcx, "task", "busy-planner").await {
            Err(error) => error,
            Ok(_) => panic!("locked live identity must not fall back to persisted proof"),
        };
        assert!(matches!(&error, PlannerProofError::Busy));
        let encoded = String::from(error);
        assert!(planner_proof_busy(&encoded));
        let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(value["retryable"], true);
    }

    #[tokio::test]
    async fn reopening_failed_card_releases_spawn_identity() {
        let temp = tempfile::tempdir().unwrap();
        let task_id = "reopen-failed";
        let mut card = claimable_card("T-1", vec![], "failed");
        card.assignee = Some("old-agent".into());
        card.agent_chat_id = Some("old-chat".into());
        let gcx = task_with_cards(temp.path(), task_id, vec![card]).await;
        update_board_atomic(gcx.clone(), task_id, |board| {
            board.card_claims.insert(
                "T-1".into(),
                CardClaim {
                    claimant_chat_id: "old-chat".into(),
                    idempotency_key: "old-key".into(),
                    claimed_at: "old".into(),
                },
            );
            Ok(())
        })
        .await
        .unwrap();
        let (board, ()) = update_board_atomic(gcx.clone(), task_id, |board| {
            board.get_card_mut("T-1").unwrap().column = "planned".into();
            Ok(())
        })
        .await
        .unwrap();
        assert!(board.card_claim("T-1").is_none());
        assert_eq!(board.get_card("T-1").unwrap().agent_chat_id, None);
        claim_card(gcx, task_id, "T-1", "new-chat", "new-key")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn parsed_board_cache_rechecks_external_content() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = task_with_board(temp.path(), "cache-board").await;
        let dir = find_task_dir(gcx.clone(), "cache-board").await.unwrap();
        let original = load_board(gcx.clone(), "cache-board").await.unwrap();
        let before = gcx.task_card_states.parse_count();
        assert_eq!(
            load_board(gcx.clone(), "cache-board").await.unwrap().rev,
            original.rev
        );
        assert_eq!(gcx.task_card_states.parse_count(), before);
        let mut replacement = original;
        replacement.transfer_owner("external", 1);
        fs::write(
            dir.join("board.yaml"),
            serde_yaml::to_string(&replacement).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(
            load_board(gcx.clone(), "cache-board")
                .await
                .unwrap()
                .owner_chat_id
                .as_deref(),
            Some("external")
        );
        assert_eq!(gcx.task_card_states.parse_count(), before + 1);
    }

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
    use crate::tasks::types::{BoardCard, TaskBoard, TaskMeta, TaskStatus};
    use chrono::Utc;
    use serde_json::json;

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

    async fn save_meta_at_barrier(
        gcx: Arc<GlobalContext>,
        task_dir: &Path,
        meta: &TaskMeta,
        fail: bool,
        observe: impl std::future::Future<Output = ()>,
    ) -> Result<(), String> {
        let staged = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let barrier = crate::file_activity::DurableRenameBarrier {
            target: board_lock::canonical_dir(task_dir)
                .await
                .unwrap()
                .join("meta.yaml"),
            staged: staged.clone(),
            release: release.clone(),
            fail,
        };
        let save = crate::file_activity::BEFORE_DURABLE_RENAME
            .scope(barrier, save_task_meta(gcx, &meta.id, meta));
        let mut reader = Box::pin(async {
            staged.notified().await;
            let staged_files = std::fs::read_dir(task_dir)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "durable-tmp"))
                .collect::<Vec<_>>();
            assert_eq!(staged_files.len(), 1);
            assert_eq!(
                fs::read_to_string(&staged_files[0]).await.unwrap(),
                serde_yaml::to_string(meta).unwrap()
            );
            observe.await;
            release.notify_one();
        });
        assert!(futures::poll!(reader.as_mut()).is_pending());
        tokio::time::timeout(std::time::Duration::from_secs(30), async {
            tokio::join!(save, reader).0
        })
        .await
        .expect("metadata save and reader must reach the publication barrier")
    }

    #[tokio::test]
    async fn task_meta_publication_keeps_complete_documents_and_preserves_old_on_failure() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = setup_workspace(temp.path()).await;
        let task_id = "task-meta-publication";
        let task_dir = temp.path().join(".refact/tasks").join(task_id);
        fs::create_dir_all(&task_dir).await.unwrap();
        let meta_path = task_dir.join("meta.yaml");
        let index_path = task_dir.parent().unwrap().join("index.json");
        let mut old = test_meta(task_id, "Original", "2026-01-01T00:00:00Z");
        old.status = TaskStatus::Active;
        old.cards_total = 7;
        old.cards_done = 2;
        old.cards_failed = 1;
        old.agents_active = 3;
        old.base_branch = Some("main".into());
        old.base_commit = Some("base-commit".into());
        old.default_agent_model = Some("provider/model".into());
        old.is_name_generated = true;
        old.last_agents_summary_at = Some("2026-01-01T00:00:00Z".into());
        old.planner_session_state = Some("idle".into());
        let old_content = serde_yaml::to_string(&old).unwrap();

        save_meta_at_barrier(gcx.clone(), &task_dir, &old, false, async {
            assert_eq!(
                fs::read(&meta_path).await.unwrap_err().kind(),
                std::io::ErrorKind::NotFound
            );
            assert!(!index_path.exists());
        })
        .await
        .unwrap();
        assert_eq!(fs::read_to_string(&meta_path).await.unwrap(), old_content);
        let old_index = fs::read(&index_path).await.unwrap();
        let index = super::super::index::read_task_index(task_dir.parent().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(index.entries.len(), 1);
        assert_eq!(
            serde_yaml::to_string(&index.entries[0].meta).unwrap(),
            old_content
        );
        let mut new = old.clone();
        new.name = "Replacement".into();
        new.updated_at = "2026-01-02T00:00:00Z".into();
        let new_content = serde_yaml::to_string(&new).unwrap();
        assert_ne!(new_content, old_content);

        let error = save_meta_at_barrier(gcx.clone(), &task_dir, &new, true, async {
            let loaded = load_task_meta(gcx.clone(), task_id).await.unwrap();
            assert_eq!(serde_yaml::to_string(&loaded).unwrap(), old_content);
            assert_eq!(fs::read(&index_path).await.unwrap(), old_index);
        })
        .await
        .unwrap_err();
        assert!(
            error.contains("injected failure before durable rename"),
            "{error}"
        );
        assert!(error.contains("Failed to publish task metadata"), "{error}");
        assert_eq!(fs::read_to_string(&meta_path).await.unwrap(), old_content);
        assert_eq!(fs::read(&index_path).await.unwrap(), old_index);

        save_meta_at_barrier(gcx.clone(), &task_dir, &new, false, async {
            let loaded = load_task_meta(gcx.clone(), task_id).await.unwrap();
            assert_eq!(serde_yaml::to_string(&loaded).unwrap(), old_content);
            assert_eq!(fs::read(&index_path).await.unwrap(), old_index);
        })
        .await
        .unwrap();
        let loaded = load_task_meta(gcx, task_id).await.unwrap();
        assert_eq!(serde_yaml::to_string(&loaded).unwrap(), new_content);
        let index = super::super::index::read_task_index(task_dir.parent().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(index.entries.len(), 1);
        assert_eq!(
            serde_yaml::to_string(&index.entries[0].meta).unwrap(),
            new_content
        );
        assert!(std::fs::read_dir(&task_dir).unwrap().all(|entry| {
            entry
                .unwrap()
                .path()
                .extension()
                .is_none_or(|ext| ext != "durable-tmp")
        }));
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
                        paused_at: None,
                        paused_by: None,
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
                        deliverable: None,
                        verification: Vec::new(),
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
            paused_at: None,
            paused_by: None,
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
            deliverable: None,
            verification: Vec::new(),
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
    async fn bound_task_transaction_keeps_original_after_workspace_reorder_and_refuses_relocation()
    {
        let original_root = tempfile::tempdir().unwrap();
        let foreign_root = tempfile::tempdir().unwrap();
        let task_id = "duplicate-task";
        let gcx = task_with_cards(
            original_root.path(),
            task_id,
            vec![claimable_card("T-1", vec![], "planned")],
        )
        .await;
        let foreign_gcx = task_with_cards(
            foreign_root.path(),
            task_id,
            vec![claimable_card("T-1", vec![], "planned")],
        )
        .await;
        update_board_atomic(foreign_gcx, task_id, |board| {
            board.cards[0].title = "foreign sentinel, never mutate".into();
            board.cards[0].instructions = "nonempty foreign task".into();
            Ok(())
        })
        .await
        .unwrap();
        let original_dir = original_root.path().join(".refact/tasks").join(task_id);
        let foreign_dir = foreign_root.path().join(".refact/tasks").join(task_id);
        let sentinel = fs::read(foreign_dir.join("board.yaml")).await.unwrap();
        assert!(!sentinel.is_empty());
        let binding = TaskDirectoryBinding::capture(gcx.clone(), original_root.path(), task_id)
            .await
            .unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![foreign_root.path().into(), original_root.path().into()];
        binding
            .scope(async {
                assert_eq!(
                    load_board(gcx.clone(), task_id).await.unwrap().cards[0].title,
                    "T-1"
                );
                update_board_atomic(gcx.clone(), task_id, |board| {
                    board.cards[0].title = "accepted original mutation".into();
                    Ok(())
                })
                .await
                .unwrap();
            })
            .await;
        assert_eq!(
            fs::read(foreign_dir.join("board.yaml")).await.unwrap(),
            sentinel
        );
        let app = AppState::from_gcx(gcx.clone()).await;
        let chat_id = format!("binding-{}", Uuid::new_v4());
        super::super::admission::reserve(&app, &chat_id, "original", "planner", 0)
            .await
            .unwrap();
        let original_card = binding
            .scope(load_board(gcx.clone(), task_id))
            .await
            .unwrap()
            .cards[0]
            .clone();
        let receipt = AcceptedTaskAttempt::new(
            &binding,
            &original_card,
            "accepted-agent",
            super::super::admission::current_attempt(&chat_id).unwrap(),
            None,
        );
        binding
            .scope(update_board_atomic(gcx.clone(), task_id, |board| {
                let card = &mut board.cards[0];
                card.column = "doing".into();
                card.assignee = Some("accepted-agent".into());
                card.agent_chat_id = Some(chat_id.clone());
                Ok(())
            }))
            .await
            .unwrap();
        let original = fs::read(original_dir.join("board.yaml")).await.unwrap();
        assert!(String::from_utf8(original.clone())
            .unwrap()
            .contains("accepted original mutation"));
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![foreign_root.path().into()];
        let error = binding
            .scope(update_board_atomic(gcx.clone(), task_id, |_| Ok(())))
            .await
            .unwrap_err();
        assert!(error.contains("no longer authorized"), "{error}");
        let retained_dir = original_root.path().join("retained-task");
        fs::rename(&original_dir, &retained_dir).await.unwrap();
        fs::create_dir(&original_dir).await.unwrap();
        fs::copy(
            foreign_dir.join("board.yaml"),
            original_dir.join("board.yaml"),
        )
        .await
        .unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![original_root.path().into(), foreign_root.path().into()];
        assert!(binding
            .scope(load_board(gcx.clone(), task_id))
            .await
            .is_err());
        assert!(receipt
            .reconcile(gcx, true, "original relocated")
            .await
            .is_err());
        assert_eq!(
            fs::read(original_dir.join("board.yaml")).await.unwrap(),
            sentinel
        );
        super::super::admission::end(&chat_id).await;
        assert_eq!(
            fs::read(retained_dir.join("board.yaml")).await.unwrap(),
            original
        );
        assert_eq!(
            fs::read(foreign_dir.join("board.yaml")).await.unwrap(),
            sentinel
        );
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
}
