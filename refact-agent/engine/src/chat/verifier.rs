use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

use chrono::Utc;
use serde_json::{json, Value};

use crate::chat::evidence::{
    completion_decision, evaluate_gates, EvidenceContext, EvidenceDefaults, EvidenceRecord,
    EvidenceService, GateOutcome,
};
use crate::chat::evidence_revision::supersede_stale;
use crate::chat::internal_roles::{event, EventSubkind};
use crate::chat::verifier_diff::{git_changed_files_summary, resolve_verifier_diff_base};
use crate::chat::verify_cmd::VerifyCommandPolicy;
use crate::chat::verify_cmd::instruction_skip_notes;
use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::tools::command_checkout::{self, ValidatedCommandCheckout};
use crate::exec::command_policy::{build_exec_request, CommandKind, CommandPolicyInput, ExecSource};
use crate::exec::{ExecOutputStream, ExecStatus};
use crate::global_context::{try_load_caps_quickly_if_not_present, GlobalContext};
use crate::tasks::storage;
use crate::tasks::types::{
    BoardCard, StatusUpdate, VerificationOutcome, VerificationResult, VerifierReport,
    VerifierReportClassification,
};

pub(crate) const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_OUTPUT_TAIL_CHARS: usize = 4000;
const MAX_OUTPUT_CAPTURE_BYTES: usize = 512 * 1024;
const MAX_DIFF_LINES: usize = 200;
const VERIFIER_SOURCE: &str = "chat.verifier";
const STALE_VERIFIER_WRITE: &str = "stale verifier write";
const CONFIGURATION_RETRY: &str = "verification configuration snapshot advanced";
const CONFIGURATION_ATTEMPTS: usize = 3;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ExpectedCardState {
    pub board_rev: u64,
    card_fingerprint: Value,
    verification: Option<VerificationSnapshot>,
}

impl ExpectedCardState {
    pub fn from_card(board_rev: u64, card: &BoardCard) -> Self {
        Self {
            board_rev,
            card_fingerprint: verifier_card_fingerprint(card),
            verification: None,
        }
    }

    pub fn matches_card(&self, card: &BoardCard) -> bool {
        self.card_fingerprint == verifier_card_fingerprint(card)
    }

    pub fn matches_board_card(&self, board_rev: u64, card: &BoardCard) -> bool {
        board_rev >= self.board_rev && self.matches_card(card)
    }

    pub(crate) fn from_finished_card(
        gcx: &GlobalContext,
        board_rev: u64,
        card: &BoardCard,
        owner: Option<&str>,
        fence: Option<FinishVerificationFence>,
    ) -> Self {
        let mut expected = Self::from_card(board_rev, card);
        if let Some(fence) = fence {
            if fence.snapshot.owner.as_deref() == owner
                && finish_candidate_matches(card, &fence.snapshot)
                && policy_settings_fingerprint(gcx)
                    .is_ok_and(|current| current == fence.snapshot.configuration.policy_settings)
            {
                expected.verification = Some(fence.snapshot);
            }
        }
        expected
    }
}

pub(crate) struct FinishVerificationFence {
    snapshot: VerificationSnapshot,
    configuration: ConfigurationSnapshot,
    card: ExpectedCardState,
}

impl FinishVerificationFence {
    pub(crate) fn is_current(
        &self,
        gcx: &GlobalContext,
        board: &crate::tasks::types::TaskBoard,
    ) -> bool {
        board
            .get_card(
                self.snapshot
                    .worktree_meta
                    .card_id
                    .as_deref()
                    .unwrap_or_default(),
            )
            .is_some_and(|card| self.card.matches_board_card(board.rev, card))
            && self.configuration.ensure_current(gcx).is_ok()
            && self.snapshot.matches(gcx, board, &self.configuration)
    }
}

fn finish_candidate_matches(card: &BoardCard, snapshot: &VerificationSnapshot) -> bool {
    let meta = &snapshot.worktree_meta;
    card.column == "done"
        && card.assignee == meta.agent_id
        && card.agent_worktree_name.as_deref() == Some(meta.id.as_str())
        && card.agent_worktree.as_deref().map(Path::new) == Some(meta.root.as_path())
        && card.agent_branch == meta.branch
        && card.agent_chat_id.as_ref().is_some_and(|chat| {
            snapshot.references.iter().any(|reference| {
                reference.kind == "task_agent"
                    && reference.chat_id.as_ref() == Some(chat)
                    && reference.agent_id == meta.agent_id
                    && reference.card_id == meta.card_id
                    && reference.task_id == meta.task_id
            })
        })
        && command_checkout::selected_winner_matches(card)
        && card.team_members.is_empty()
}

pub(crate) async fn capture_finish_verification(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
    owner: &str,
    chat_id: &str,
    worktree: &crate::worktrees::types::WorktreeMeta,
) -> Result<FinishVerificationFence, String> {
    let source = dunce::canonicalize(&worktree.source_workspace_root)
        .map_err(|_| "verification source unavailable")?;
    let (board, _) = command_checkout::read_board(&source, task_id).await?;
    board.require_owner(owner)?;
    let card = board
        .get_card(card_id)
        .ok_or("verification card unavailable")?;
    let identity = if let Some(variants) = &card.ab_variants {
        variants
            .variant_key_for_chat_id(chat_id)
            .and_then(|key| variants.variant(key))
            .is_some_and(|variant| {
                worktree.agent_id.as_deref() == Some(variant.agent_id.as_str())
                    && variant.worktree_name.as_deref() == Some(worktree.id.as_str())
                    && Path::new(&variant.worktree) == worktree.root.as_path()
                    && variant.branch == worktree.branch
            })
    } else {
        card.agent_chat_id.as_deref() == Some(chat_id)
            && card.assignee == worktree.agent_id
            && card.agent_worktree_name.as_deref() == Some(worktree.id.as_str())
            && card.agent_worktree.as_deref().map(Path::new) == Some(worktree.root.as_path())
            && card.agent_branch == worktree.branch
    };
    if !identity
        || worktree.kind != "task_agent"
        || !worktree.enforce
        || worktree.task_id.as_deref() != Some(task_id)
        || worktree.card_id.as_deref() != Some(card_id)
        || worktree.source_workspace_root != source
        || !card.team_members.is_empty()
    {
        return Err(stale_verifier_error(task_id, card_id));
    }
    let service = crate::worktrees::service::WorktreeService::new_async(
        gcx.cache_dir.clone(),
        source.clone(),
    )
    .await?;
    let view = service.get_worktree(&worktree.id).await?;
    if view.meta != *worktree
        || view.status.conflicted
        || !view.status.path_exists
        || !view.status.is_git_worktree
        || view.status.error.is_some()
        || view.status.branch != worktree.branch
        || dunce::canonicalize(&worktree.root).ok().as_ref() != Some(&worktree.root)
        || !view.references.iter().any(|reference| {
            reference.kind == "task_agent"
                && reference.chat_id.as_deref() == Some(chat_id)
                && reference.agent_id == worktree.agent_id
                && reference.card_id == worktree.card_id
                && reference.task_id == worktree.task_id
        })
    {
        return Err(stale_verifier_error(task_id, card_id));
    }
    let head = view
        .status
        .head_commit
        .ok_or("verification candidate head unavailable")?;
    let snapshot = VerificationSnapshot::capture_registered(
        &gcx,
        source,
        worktree,
        &view.references,
        &head,
        task_id,
        card_id,
        &board,
    )
    .await?;
    let mut snapshot = snapshot;
    snapshot.runtime = Some((
        chat_id.into(),
        gcx.agent_executions
            .get(chat_id)
            .map(|run| run.admission_generation()),
    ));
    let configuration = ConfigurationSnapshot::capture(&gcx).await?;
    configuration.ensure_current(&gcx)?;
    if !snapshot.matches(&gcx, &board, &configuration) {
        return Err(stale_verifier_error(task_id, card_id));
    }
    Ok(FinishVerificationFence {
        snapshot,
        configuration,
        card: ExpectedCardState::from_card(board.rev, card),
    })
}

pub async fn load_expected_card_state(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
) -> Result<ExpectedCardState, String> {
    let board = storage::load_board(gcx, task_id).await?;
    let card = board
        .get_card(card_id)
        .ok_or_else(|| format!("Card {} not found", card_id))?;
    Ok(ExpectedCardState::from_card(board.rev, card))
}

fn verifier_card_fingerprint(card: &BoardCard) -> Value {
    json!({
        "id": &card.id,
        "title": &card.title,
        "column": &card.column,
        "priority": &card.priority,
        "depends_on": &card.depends_on,
        "instructions": &card.instructions,
        "assignee": &card.assignee,
        "agent_chat_id": &card.agent_chat_id,
        "final_report": &card.final_report,
        "final_report_structured": &card.final_report_structured,
        "verifier_report": &card.verifier_report,
        "created_at": &card.created_at,
        "started_at": &card.started_at,
        "completed_at": &card.completed_at,
        "agent_branch": &card.agent_branch,
        "agent_worktree": &card.agent_worktree,
        "agent_worktree_name": &card.agent_worktree_name,
        "base_branch": &card.base_branch,
        "base_commit": &card.base_commit,
        "ab_variants": &card.ab_variants,
        "team_members": &card.team_members,
        "target_files": &card.target_files,
        "scope_guard_mode": &card.scope_guard_mode,
        "paused_at": &card.paused_at,
        "paused_by": &card.paused_by,
        "deliverable": &card.deliverable,
        "verification": &card.verification,
    })
}

fn stale_verifier_error(task_id: &str, card_id: &str) -> String {
    format!(
        "{} for task {} card {}: current card state no longer matches the verifier request",
        STALE_VERIFIER_WRITE, task_id, card_id
    )
}

fn is_stale_verifier_error(error: &str) -> bool {
    error.starts_with(STALE_VERIFIER_WRITE)
}

#[derive(Clone, Debug)]
pub struct VerifyCardRequest {
    pub task_id: String,
    pub card_id: String,
    pub expected_state: Option<ExpectedCardState>,
}

#[derive(Clone)]
pub(crate) struct RuntimeVerifierActor {
    task_id: String,
    card_id: String,
    owner: String,
    source: PathBuf,
    expected: ExpectedCardState,
    chat_id: String,
    worktree: crate::worktrees::types::WorktreeMeta,
    session: Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
    abort: Arc<std::sync::atomic::AtomicBool>,
    admission_generation: Option<u64>,
    terminal_version: Option<u64>,
    scheduled_head: String,
}

impl RuntimeVerifierActor {
    pub(crate) fn task_id(&self) -> &str {
        &self.task_id
    }
    pub(crate) fn card_id(&self) -> &str {
        &self.card_id
    }
    pub(crate) fn owner(&self) -> &str {
        &self.owner
    }
    pub(crate) fn source(&self) -> &Path {
        &self.source
    }
    pub(crate) fn expected(&self) -> &ExpectedCardState {
        &self.expected
    }

    async fn after_finish(app: &AppState, request: &VerifyCardRequest) -> Result<Self, String> {
        let expected = request
            .expected_state
            .clone()
            .ok_or("runtime verifier requires finish snapshot")?;
        let board = storage::load_board(app.gcx.clone(), &request.task_id).await?;
        let card = board
            .get_card(&request.card_id)
            .ok_or("finished card unavailable")?;
        if !expected.matches_board_card(board.rev, card)
            || card.column != "done"
            || !command_checkout::selected_winner_matches(card)
            || !card.team_members.is_empty()
        {
            return Err(stale_verifier_error(&request.task_id, &request.card_id));
        }
        let chat_id = card
            .agent_chat_id
            .clone()
            .ok_or("finished agent unavailable")?;
        let session = app
            .chat
            .sessions
            .read()
            .await
            .get(&chat_id)
            .cloned()
            .ok_or("finished session unavailable")?;
        let (source, worktree, abort) = {
            let session = session.lock().await;
            (
                crate::coordination::scope::session_source_root(&session)
                    .ok_or("finished source unavailable")?,
                session
                    .thread
                    .worktree
                    .clone()
                    .ok_or("finished worktree unavailable")?,
                session.abort_flag.clone(),
            )
        };
        let source = dunce::canonicalize(source).map_err(|e| e.to_string())?;
        let service = crate::worktrees::service::WorktreeService::new_async(
            app.gcx.cache_dir.clone(),
            source.clone(),
        )
        .await?;
        let view = service.get_worktree(&worktree.id).await?;
        if view.meta != worktree {
            return Err(stale_verifier_error(&request.task_id, &request.card_id));
        }
        let scheduled_head = view
            .status
            .head_commit
            .ok_or("finished candidate head unavailable")?;
        let actor = Self {
            task_id: request.task_id.clone(),
            card_id: request.card_id.clone(),
            owner: board
                .owner_chat_id
                .clone()
                .ok_or("finished owner unavailable")?,
            source,
            expected,
            chat_id: chat_id.clone(),
            worktree,
            session,
            abort,
            admission_generation: app
                .agent_executions
                .get(&chat_id)
                .map(|run| run.admission_generation()),
            terminal_version: None,
            scheduled_head,
        };
        actor.ensure_current(app, true).await?;
        Ok(actor)
    }

    pub(crate) async fn ensure_current(
        &self,
        app: &AppState,
        finishing: bool,
    ) -> Result<(), String> {
        let stale = || stale_verifier_error(&self.task_id, &self.card_id);
        if app.runtime.shutdown_flag.load(Ordering::Relaxed) {
            return Err(stale());
        }
        let live = app
            .chat
            .sessions
            .read()
            .await
            .get(&self.chat_id)
            .cloned()
            .ok_or_else(stale)?;
        if !Arc::ptr_eq(&live, &self.session) {
            return Err(stale());
        }
        if let Some(run) = app.agent_executions.get(&self.chat_id) {
            if Some(run.admission_generation()) != self.admission_generation || run.is_cancelled() {
                return Err(stale());
            }
        }
        {
            let session = live.lock().await;
            let task = session.thread.task_meta.as_ref().ok_or_else(stale)?;
            let state_ok = session.runtime.state == refact_runtime_api::SessionState::Completed
                || (finishing
                    && session.runtime.state == refact_runtime_api::SessionState::ExecutingTools
                    && session
                        .messages
                        .iter()
                        .rev()
                        .find(|message| message.role == "assistant")
                        .and_then(|message| message.tool_calls.as_ref())
                        .is_some_and(|calls| {
                            calls.len() == 1 && calls[0].function.name == "agent_finish"
                        }));
            if !state_ok
                || session.closed
                || session.runtime.paused
                || !session.runtime.pause_reasons.is_empty()
                || session.user_interrupt_flag.load(Ordering::Relaxed)
                || !session.command_queue.is_empty()
                || !Arc::ptr_eq(&self.abort, &session.abort_flag)
                || self
                    .terminal_version
                    .is_some_and(|version| session.trajectory_version != version)
                || task.role != "agents"
                || task.task_id != self.task_id
                || task.card_id.as_deref() != Some(&self.card_id)
                || task.agent_id != self.worktree.agent_id
                || task.planner_chat_id.as_deref() != Some(&self.owner)
                || session.thread.parent_id.as_deref() != Some(&self.owner)
                || session.thread.root_chat_id.as_deref() != Some(&self.owner)
                || session.thread.link_type.as_deref() != Some("task_agent")
                || session.thread.worktree.as_ref() != Some(&self.worktree)
                || crate::coordination::scope::session_source_root(&session).as_deref()
                    != Some(&self.source)
            {
                return Err(stale());
            }
        }
        let (board, _) = command_checkout::read_board(&self.source, &self.task_id).await?;
        board.require_owner(&self.owner).map_err(|_| stale())?;
        let card = board.get_card(&self.card_id).ok_or_else(stale)?;
        if !self.expected.matches_board_card(board.rev, card)
            || card.column != "done"
            || card.agent_chat_id.as_deref() != Some(&self.chat_id)
            || card.assignee != self.worktree.agent_id
            || card.agent_worktree_name.as_deref() != Some(&self.worktree.id)
            || card.agent_worktree.as_deref().map(Path::new) != Some(self.worktree.root.as_path())
            || !command_checkout::selected_winner_matches(card)
            || !card.team_members.is_empty()
        {
            return Err(stale());
        }
        let current_head = crate::chat::verifier_diff::git_head_commit(&self.worktree.root).await;
        if current_head.as_deref() != Some(self.scheduled_head.as_str()) {
            return Err(format!(
                "{}; checkout {} at {} expected HEAD {}, actual {}",
                stale(),
                self.worktree.id,
                self.worktree.root.display(),
                self.scheduled_head,
                current_head.as_deref().unwrap_or("unavailable")
            ));
        }
        Ok(())
    }

    async fn wait_completed(&mut self, app: &AppState) -> Result<(), String> {
        let mut events = self.session.lock().await.event_tx.subscribe();
        let mut shutdown_check = tokio::time::interval(Duration::from_secs(1));
        let wait = async {
            loop {
                self.ensure_current(app, true).await?;
                {
                    let session = self.session.lock().await;
                    if session.runtime.state == refact_runtime_api::SessionState::Completed {
                        self.terminal_version = Some(session.trajectory_version);
                        break;
                    }
                }
                tokio::select! {
                    event = events.recv() => {
                        if matches!(event, Err(tokio::sync::broadcast::error::RecvError::Closed)) {
                            return Err("finished session events closed".to_string());
                        }
                    }
                    _ = shutdown_check.tick() => {
                        if app.runtime.shutdown_flag.load(Ordering::Relaxed) {
                            return Err("verifier shutdown".to_string());
                        }
                    }
                }
            }
            self.ensure_current(app, false).await
        };
        tokio::time::timeout(DRAIN_TIMEOUT, wait)
            .await
            .map_err(|_| "finish boundary unavailable before verifier deadline".to_string())?
    }
}

#[derive(Clone)]
enum VerifierActor {
    Planner(Arc<tokio::sync::Mutex<AtCommandsContext>>),
    Finished(RuntimeVerifierActor),
}

struct CardEvidenceRunner {
    gcx: Arc<GlobalContext>,
    actor: VerifierActor,
    request: VerifyCardRequest,
    candidate: ValidatedCommandCheckout,
    expected: ExpectedCardState,
    output_records: std::sync::Mutex<crate::coordination::provenance::WorkProvenance>,
}

impl CardEvidenceRunner {
    async fn model(&self) -> Result<String, String> {
        match &self.actor {
            VerifierActor::Planner(ccx) => Ok(ccx.lock().await.current_model.clone()),
            VerifierActor::Finished(_) => resolve_verifier_model(self.gcx.clone()).await,
        }
    }

    async fn abort_flag(&self) -> Arc<std::sync::atomic::AtomicBool> {
        match &self.actor {
            VerifierActor::Planner(ccx) => ccx.lock().await.abort_flag.clone(),
            VerifierActor::Finished(actor) => {
                actor.session.lock().await.user_interrupt_flag.clone()
            }
        }
    }

    async fn revalidate(&self) -> Result<(), String> {
        for _ in 0..CONFIGURATION_ATTEMPTS {
            let result = self.revalidate_once().await;
            if result
                .as_ref()
                .is_err_and(|error| error == CONFIGURATION_RETRY)
            {
                continue;
            }
            return result;
        }
        Err("verification configuration remained unavailable".into())
    }

    async fn revalidate_once(&self) -> Result<(), String> {
        let app = AppState::from_gcx(self.gcx.clone()).await;
        let current = match &self.actor {
            VerifierActor::Planner(ccx) => {
                command_checkout::resolve_planner_verifier(
                    ccx,
                    &self.request.task_id,
                    &self.request.card_id,
                )
                .await?
            }
            VerifierActor::Finished(actor) => {
                command_checkout::resolve_finished_verifier(&app, actor).await?
            }
        };
        let board = storage::load_board(self.gcx.clone(), &self.request.task_id).await?;
        let card = board
            .get_card(&self.request.card_id)
            .ok_or("verification card unavailable")?;
        let configuration = ConfigurationSnapshot::capture(&self.gcx).await?;
        configuration.ensure_current(&self.gcx)?;
        let matches = self.candidate.same_candidate(&current)
            && self.expected.matches_board_card(board.rev, card)
            && self
                .expected
                .verification
                .as_ref()
                .is_some_and(|snapshot| snapshot.matches(&self.gcx, &board, &configuration));
        configuration.ensure_current(&self.gcx)?;
        if !matches {
            return Err(stale_verifier_error(
                &self.request.task_id,
                &self.request.card_id,
            ));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl crate::chat::evidence::EvidenceRunner for CardEvidenceRunner {
    async fn run(
        &mut self,
        worktree: &Path,
        spec: &crate::chat::evidence::VerificationSpec,
    ) -> VerificationResult {
        run_verification_argv_scoped(
            self.gcx.clone(),
            worktree,
            &spec.command,
            spec.cwd.clone(),
            spec.env.clone(),
            spec.argv.clone(),
            Duration::from_secs(spec.timeout_secs),
            DRAIN_TIMEOUT,
            Some(self),
        )
        .await
    }
}

#[derive(Clone, Debug, PartialEq)]
struct VerificationSnapshot {
    owner: Option<String>,
    board_contract: Value,
    task_contract: Value,
    task_meta_path: PathBuf,
    source: PathBuf,
    root: PathBuf,
    head: String,
    files: Vec<(PathBuf, Option<String>)>,
    registry_path: PathBuf,
    worktree_meta: crate::worktrees::types::WorktreeMeta,
    references: Vec<crate::worktrees::types::WorktreeReference>,
    configuration: ConfigurationContent,
    runtime: Option<(String, Option<u64>)>,
}

#[derive(Clone, Debug, PartialEq)]
struct ConfigurationContent {
    caps: Value,
    models: String,
    policy_settings: Value,
}

struct ConfigurationSnapshot {
    content: ConfigurationContent,
    capabilities_generation: u64,
}

impl ConfigurationSnapshot {
    async fn capture(gcx: &GlobalContext) -> Result<Self, String> {
        let (caps, capabilities_generation) = {
            let state = gcx.caps_state.read().await;
            (
                state.caps.clone(),
                gcx.tool_catalog_generations
                    .capabilities
                    .load(Ordering::SeqCst),
            )
        };
        let models = caps
            .as_ref()
            .map(|caps| {
                let models = caps
                    .chat_models
                    .iter()
                    .map(|(id, model)| (format!("chat:{id}"), &model.base))
                    .chain(
                        caps.completion_models
                            .iter()
                            .map(|(id, model)| (format!("completion:{id}"), &model.base)),
                    )
                    .chain(std::iter::once((
                        "embedding".into(),
                        &caps.embedding_model.base,
                    )));
                let models = models.map(|(id, base)| (id, json!({
                "endpoint":base.endpoint, "style":base.endpoint_style,
                "wire":base.wire_format, "key":base.api_key, "credential":base.credential,
                "auth":base.auth_token, "tokenizer_key":base.tokenizer_api_key,
                "headers":base.extra_headers, "similar":base.similar_models,
            }))).collect::<std::collections::BTreeMap<_, _>>();
                serde_json::to_vec(&models).map(|bytes| crate::file_activity::digest_bytes(&bytes))
            })
            .transpose()
            .map_err(|_| "verification model configuration unavailable")?
            .unwrap_or_default();
        let caps = serde_json::to_value((
            &caps,
            caps.as_ref().map(|caps| &caps.model_caps),
            caps.as_ref().map(|caps| &caps.user_defaults),
            caps.as_ref().map(|caps| &caps.provider_base_names),
            caps.as_ref().map(|caps| {
                caps.chat_models
                    .iter()
                    .map(|(id, model)| {
                        (
                            id,
                            (
                                &model.scratchpad,
                                &model.scratchpad_patch,
                                &model.live_fields,
                            ),
                        )
                    })
                    .collect::<std::collections::BTreeMap<_, _>>()
            }),
        ))
        .map_err(|_| "verification model configuration unavailable")?;
        Ok(Self {
            content: ConfigurationContent {
                caps,
                models,
                policy_settings: policy_settings_fingerprint(gcx)?,
            },
            capabilities_generation,
        })
    }

    fn ensure_current(&self, gcx: &GlobalContext) -> Result<(), String> {
        if gcx
            .tool_catalog_generations
            .capabilities
            .load(Ordering::SeqCst)
            != self.capabilities_generation
        {
            return Err(CONFIGURATION_RETRY.into());
        }
        Ok(())
    }
}

fn policy_settings_fingerprint(gcx: &GlobalContext) -> Result<Value, String> {
    let privacy = gcx
        .privacy_policy_load
        .read()
        .map_err(|_| "verification privacy policy unavailable")?;
    serde_json::to_value((
        &privacy.policy,
        &privacy.error,
        crate::runtime_settings::current(),
    ))
    .map_err(|_| "verification configuration unavailable".into())
}

fn task_contract(path: &Path) -> Result<Value, String> {
    let bytes = bounded_file_bytes(path)?.ok_or("verification task metadata unavailable")?;
    let meta: crate::tasks::types::TaskMeta =
        serde_yaml::from_slice(&bytes).map_err(|_| "verification task metadata invalid")?;
    Ok(json!({"id":meta.id,"base_commit":meta.base_commit,"base_branch":meta.base_branch}))
}

fn board_contract(board: &crate::tasks::types::TaskBoard) -> Value {
    json!({"review_policy":board.review_policy})
}

impl VerificationSnapshot {
    async fn capture(
        gcx: &GlobalContext,
        source: PathBuf,
        candidate: &ValidatedCommandCheckout,
        task_id: &str,
        card_id: &str,
        board: &crate::tasks::types::TaskBoard,
    ) -> Result<Self, String> {
        Self::capture_registered(
            gcx,
            source,
            candidate.metadata(),
            candidate.references(),
            candidate.head(),
            task_id,
            card_id,
            board,
        )
        .await
    }

    async fn capture_registered(
        gcx: &GlobalContext,
        source: PathBuf,
        meta: &crate::worktrees::types::WorktreeMeta,
        references: &[crate::worktrees::types::WorktreeReference],
        head: &str,
        task_id: &str,
        card_id: &str,
        board: &crate::tasks::types::TaskBoard,
    ) -> Result<Self, String> {
        if head.len() != 40
            || !head
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err("verification candidate head unavailable".into());
        }
        let task_dir = source.join(".refact/tasks").join(task_id);
        let paths = [
            task_dir.join("assignments").join(format!("{card_id}.json")),
            crate::tools::shell_gate::policy_path(&source),
        ];
        let registry_path =
            crate::worktrees::service::WorktreeService::new(gcx.cache_dir.clone(), source.clone())?
                .registry_path();
        let files = paths
            .into_iter()
            .map(|path| {
                let digest = bounded_file_digest(&path)?;
                Ok((path, digest))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            owner: board.owner_chat_id.clone(),
            board_contract: board_contract(board),
            task_contract: task_contract(&task_dir.join("meta.yaml"))?,
            task_meta_path: task_dir.join("meta.yaml"),
            source,
            root: meta.root.clone(),
            head: head.into(),
            files,
            registry_path,
            worktree_meta: meta.clone(),
            references: references.to_vec(),
            configuration: ConfigurationSnapshot::capture(gcx).await?.content,
            runtime: None,
        })
    }

    fn source_is_current(&self, gcx: &GlobalContext) -> bool {
        let raw = gcx
            .documents_state
            .workspace_folders
            .lock()
            .unwrap()
            .clone();
        crate::files_correction::project_dirs_for_unscoped_paths(&gcx.cache_dir, &raw)
            .into_iter()
            .find_map(|root| {
                let source = dunce::canonicalize(root).ok()?;
                let task_id = self.worktree_meta.task_id.as_ref()?;
                source
                    .join(".refact/tasks")
                    .join(task_id)
                    .is_dir()
                    .then_some(source)
            })
            .is_some_and(|source| source == self.source)
    }

    fn matches(
        &self,
        gcx: &GlobalContext,
        board: &crate::tasks::types::TaskBoard,
        configuration: &ConfigurationSnapshot,
    ) -> bool {
        board.owner_chat_id == self.owner
            && self.runtime.as_ref().is_none_or(|(chat, generation)| {
                gcx.agent_executions.get(chat).is_none_or(|run| {
                    Some(run.admission_generation()) == *generation && !run.is_cancelled()
                })
            })
            && board_contract(board) == self.board_contract
            && task_contract(&self.task_meta_path)
                .is_ok_and(|current| current == self.task_contract)
            && dunce::canonicalize(&self.source).is_ok_and(|source| source == self.source)
            && dunce::canonicalize(&self.root).is_ok_and(|root| root == self.root)
            && self.source_is_current(gcx)
            && git2::Repository::open(&self.root).is_ok_and(|repo| {
                repo.head()
                    .is_ok_and(|head| head.shorthand() == self.worktree_meta.branch.as_deref())
            })
            && git2::Repository::open(&self.root)
                .ok()
                .and_then(|repo| repo.head().ok()?.target().map(|id| id.to_string()))
                .as_deref()
                == Some(self.head.as_str())
            && self.files.iter().all(|(path, digest)| {
                bounded_file_digest(path).is_ok_and(|current| current == *digest)
            })
            && selected_registry_matches(&self.registry_path, &self.worktree_meta, &self.references)
            && configuration.content == self.configuration
            && policy_settings_fingerprint(gcx)
                .is_ok_and(|current| current == self.configuration.policy_settings)
    }
}

fn bounded_file_bytes(path: &Path) -> Result<Option<Vec<u8>>, String> {
    use std::io::Read;
    if fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err("verification input alias refused".into());
    }
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("verification input unavailable".into()),
    };
    let cap = crate::file_activity::settings().max_work_history_bytes;
    let mut bytes = Vec::new();
    file.take(cap as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "verification input unreadable")?;
    if bytes.len() > cap {
        return Err("verification input exceeds bound".into());
    }
    Ok(Some(bytes))
}

fn bounded_file_digest(path: &Path) -> Result<Option<String>, String> {
    bounded_file_bytes(path)
        .map(|bytes| bytes.map(|bytes| crate::file_activity::digest_bytes(&bytes)))
}

fn selected_registry_matches(
    path: &Path,
    expected: &crate::worktrees::types::WorktreeMeta,
    references: &[crate::worktrees::types::WorktreeReference],
) -> bool {
    bounded_file_bytes(path)
        .ok()
        .flatten()
        .and_then(|bytes| {
            serde_json::from_slice::<crate::worktrees::types::WorktreeRegistry>(&bytes).ok()
        })
        .is_some_and(|registry| {
            registry.source_workspace_root == expected.source_workspace_root
                && registry
                    .records
                    .iter()
                    .find(|record| record.meta.id == expected.id)
                    .is_some_and(|record| {
                        record.meta == *expected && record.references == references
                    })
        })
}

async fn check_cwd_in_worktree(worktree: &Path, effective_cwd: &Path) -> Result<(), String> {
    let canonical_worktree = tokio::fs::canonicalize(worktree)
        .await
        .map_err(|e| format!("cannot access worktree '{}': {}", worktree.display(), e))?;
    if let Ok(canonical_cwd) = tokio::fs::canonicalize(effective_cwd).await {
        if !canonical_cwd.starts_with(&canonical_worktree) {
            return Err(format!(
                "cwd '{}' is outside the worktree",
                effective_cwd.display()
            ));
        }
    }
    Ok(())
}

fn append_verifier_status(card: &mut BoardCard, report: &VerifierReport) {
    let message = match report.classification {
        VerifierReportClassification::Unavailable => format!(
            "Verifier: UNAVAILABLE — {}",
            report
                .concerns
                .first()
                .map(String::as_str)
                .unwrap_or("verification infrastructure unavailable")
        ),
        _ if report.passed => "Verifier: PASS".to_string(),
        VerifierReportClassification::HumanReview => {
            "Verifier: NEEDS REVIEW — advisory concerns or missing evidence".to_string()
        }
        _ => {
            let first = report
                .concerns
                .first()
                .map(|s| s.as_str())
                .unwrap_or("verification failed");
            format!("Verifier: FAIL — {}", first)
        }
    };
    card.status_updates.push(StatusUpdate {
        timestamp: Utc::now().to_rfc3339(),
        message,
    });
}

pub async fn store_verifier_report(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
    expected_state: Option<&ExpectedCardState>,
    report: VerifierReport,
    evidence_spec_ids: Vec<String>,
) -> Result<bool, String> {
    for _ in 0..CONFIGURATION_ATTEMPTS {
        let configuration = ConfigurationSnapshot::capture(&gcx).await?;
        let result = store_verifier_report_with_configuration(
            gcx.clone(),
            task_id,
            card_id,
            expected_state,
            report.clone(),
            evidence_spec_ids.clone(),
            configuration,
        )
        .await;
        if result
            .as_ref()
            .is_err_and(|error| error == CONFIGURATION_RETRY)
        {
            continue;
        }
        return result;
    }
    Err("verification configuration remained unavailable".into())
}

async fn store_verifier_report_with_configuration(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
    expected_state: Option<&ExpectedCardState>,
    report: VerifierReport,
    evidence_spec_ids: Vec<String>,
    configuration: ConfigurationSnapshot,
) -> Result<bool, String> {
    let card_id = card_id.to_string();
    let task_id_owned = task_id.to_string();
    let expected_state = expected_state.cloned();
    let snapshot_gcx = gcx.clone();
    match storage::update_board_atomic(gcx, task_id, move |board| {
        configuration.ensure_current(&snapshot_gcx)?;
        let board_rev = board.rev;
        let Some(card) = board.get_card(&card_id) else {
            if expected_state.is_some() {
                return Err(stale_verifier_error(&task_id_owned, &card_id));
            }
            return Err(format!("Card {} not found", card_id));
        };
        if let Some(expected) = expected_state.as_ref() {
            let matches = expected.matches_board_card(board_rev, card)
                && expected
                    .verification
                    .as_ref()
                    .is_none_or(|snapshot| snapshot.matches(&snapshot_gcx, board, &configuration));
            configuration.ensure_current(&snapshot_gcx)?;
            if !matches {
                return Err(stale_verifier_error(&task_id_owned, &card_id));
            }
        }
        configuration.ensure_current(&snapshot_gcx)?;
        board.review_attestations.remove(&card_id);
        let card = board
            .get_card_mut(&card_id)
            .ok_or_else(|| format!("Card {} not found", card_id))?;
        card.verifier_report = Some(report.clone());
        append_verifier_status(card, &report);
        board.attach_card_evidence(
            &card_id,
            crate::tasks::types::OUTCOME_FRAGMENT_VERIFICATION,
            &evidence_spec_ids,
        );
        Ok(())
    })
    .await
    {
        Ok(_) => Ok(true),
        Err(error) if is_stale_verifier_error(&error) => {
            tracing::info!("{}", error);
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

async fn schedule_card_verifier(gcx: Arc<GlobalContext>, request: VerifyCardRequest) {
    if gcx.shutdown_flag.load(Ordering::SeqCst) {
        return;
    }
    let app = AppState::from_gcx(gcx.clone()).await;
    let task_gcx = gcx.clone();
    let mut tasks = gcx.card_verifier_tasks.lock().await;
    while let Some(result) = tasks.try_join_next() {
        if let Err(error) = result {
            tracing::warn!("card verifier task failed: {error}");
        }
    }
    if gcx.shutdown_flag.load(Ordering::SeqCst) {
        return;
    }
    tasks.spawn(async move {
        let result = match RuntimeVerifierActor::after_finish(&app, &request).await {
            Ok(mut actor) => match actor.wait_completed(&app).await {
                Ok(()) => Ok(actor),
                Err(error) => Err(actor
                    .ensure_current(&app, true)
                    .await
                    .err()
                    .unwrap_or(error)),
            },
            Err(error) => Err(error),
        };
        let result = match result {
            Ok(actor) => verify_card(task_gcx, request, VerifierActor::Finished(actor)).await,
            Err(error) => {
                settle_verifier_failure(
                    task_gcx,
                    &request,
                    request.expected_state.as_ref(),
                    Vec::new(),
                    error,
                )
                .await
            }
        };
        if let Err(error) = result {
            tracing::warn!("card verification not committed: {error}");
        }
    });
}

pub async fn shutdown_card_verifiers(gcx: Arc<GlobalContext>) {
    let mut tasks = gcx.card_verifier_tasks.lock().await;
    tasks.abort_all();
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result {
            if !error.is_cancelled() {
                tracing::warn!("card verifier task failed during shutdown: {error}");
            }
        }
    }
}

pub async fn schedule_card_verifier_after_finish(
    gcx: Arc<GlobalContext>,
    task_id: String,
    card_id: String,
    expected_state: ExpectedCardState,
) {
    schedule_card_verifier(
        gcx,
        VerifyCardRequest {
            task_id,
            card_id,
            expected_state: Some(expected_state),
        },
    )
    .await;
}

pub(crate) async fn verify_card_as_planner(
    ccx: Arc<tokio::sync::Mutex<AtCommandsContext>>,
    request: VerifyCardRequest,
) -> Result<VerifierReport, String> {
    let gcx = ccx.lock().await.app.gcx.clone();
    verify_card(gcx, request, VerifierActor::Planner(ccx)).await
}

async fn verify_card(
    gcx: Arc<GlobalContext>,
    request: VerifyCardRequest,
    actor: VerifierActor,
) -> Result<VerifierReport, String> {
    let mut expected = request.expected_state.clone();
    let mut attempted = Vec::new();
    let mut settled = None;
    match verify_card_inner(
        gcx.clone(),
        &request,
        actor.clone(),
        &mut expected,
        &mut attempted,
        &mut settled,
    )
    .await
    {
        Ok(report) => Ok(report),
        Err(error) => {
            if is_stale_verifier_error(&error) {
                return Err(error);
            }
            if let VerifierActor::Finished(actor) = &actor {
                actor
                    .ensure_current(&AppState::from_gcx(gcx.clone()).await, false)
                    .await?;
            }
            if let Some(report) = settled.filter(|report: &VerifierReport| {
                report.classification == VerifierReportClassification::VerificationFailed
            }) {
                if let Some(expected) = expected
                    .as_ref()
                    .filter(|state| state.verification.is_some())
                {
                    if store_verifier_report(
                        gcx.clone(),
                        &request.task_id,
                        &request.card_id,
                        Some(expected),
                        report.clone(),
                        Vec::new(),
                    )
                    .await?
                    {
                        return Ok(report);
                    }
                    return Err(stale_verifier_error(&request.task_id, &request.card_id));
                }
                return Err(error);
            }
            settle_verifier_failure(gcx, &request, expected.as_ref(), attempted, error).await
        }
    }
}

async fn settle_verifier_failure(
    gcx: Arc<GlobalContext>,
    request: &VerifyCardRequest,
    expected: Option<&ExpectedCardState>,
    command_results: Vec<VerificationResult>,
    error: String,
) -> Result<VerifierReport, String> {
    if is_stale_verifier_error(&error) {
        return Err(error);
    }
    let expected = expected
        .filter(|expected| expected.verification.is_some())
        .ok_or_else(|| {
            format!("verification unavailable without original finish fence: {error}")
        })?;
    let failed = command_results
        .iter()
        .any(|result| result.outcome == VerificationOutcome::CommandFailed);
    let report = VerifierReport {
        passed: false,
        notes: Vec::new(),
        command_results,
        concerns: vec![format!(
            "Verification infrastructure unavailable; human review required: {error}"
        )],
        recommendation: if failed { "fix-needed" } else { "human-review" }.into(),
        classification: if failed {
            VerifierReportClassification::VerificationFailed
        } else {
            VerifierReportClassification::Unavailable
        },
    };
    if !store_verifier_report(
        gcx,
        &request.task_id,
        &request.card_id,
        Some(expected),
        report.clone(),
        Vec::new(),
    )
    .await?
    {
        return Err(stale_verifier_error(&request.task_id, &request.card_id));
    }
    Ok(report)
}

async fn verify_card_inner(
    gcx: Arc<GlobalContext>,
    request: &VerifyCardRequest,
    actor: VerifierActor,
    attempt_expected: &mut Option<ExpectedCardState>,
    attempted: &mut Vec<VerificationResult>,
    settled: &mut Option<VerifierReport>,
) -> Result<VerifierReport, String> {
    let board = storage::load_board(gcx.clone(), &request.task_id).await?;
    let card = board
        .get_card(&request.card_id)
        .ok_or_else(|| format!("Card {} not found", request.card_id))?
        .clone();
    if let Some(expected) = request.expected_state.as_ref() {
        if !expected.matches_board_card(board.rev, &card) {
            return Err(stale_verifier_error(&request.task_id, &request.card_id));
        }
    }
    let app = AppState::from_gcx(gcx.clone()).await;
    let candidate = match &actor {
        VerifierActor::Planner(ccx) => {
            command_checkout::resolve_planner_verifier(ccx, &request.task_id, &request.card_id)
                .await?
        }
        VerifierActor::Finished(actor) => {
            command_checkout::resolve_finished_verifier(&app, actor).await?
        }
    };
    let worktree = candidate.root().to_path_buf();
    let source = candidate.metadata().source_workspace_root.clone();
    let expected_dir = source.join(".refact/tasks").join(&request.task_id);
    if dunce::canonicalize(storage::find_task_dir(gcx.clone(), &request.task_id).await?)
        .map_err(|e| e.to_string())?
        != expected_dir
    {
        return Err(stale_verifier_error(&request.task_id, &request.card_id));
    }
    let mut expected = request
        .expected_state
        .clone()
        .unwrap_or_else(|| ExpectedCardState::from_card(board.rev, &card));
    if expected.verification.is_none() {
        if matches!(&actor, VerifierActor::Finished(_)) {
            return Err("verification unavailable without original finish fence".into());
        }
        expected.verification = Some(
            VerificationSnapshot::capture(
                &gcx,
                source,
                &candidate,
                &request.task_id,
                &request.card_id,
                &board,
            )
            .await?,
        );
    }
    *attempt_expected = Some(expected.clone());

    let task_meta = storage::load_task_meta(gcx.clone(), &request.task_id).await?;
    let defaults = EvidenceDefaults::from_settings();
    let assignment =
        crate::tasks::assignment::load_assignment(gcx.clone(), &request.task_id, &card.id).await?;
    let specs =
        crate::tasks::assignment::specs_for_validation(assignment.as_ref(), &card, &defaults);
    let skipped_instruction_notes = instruction_skip_notes(&card.instructions);
    let commit = Some(candidate.head().to_string());
    let service = EvidenceService::new(
        VerifyCommandPolicy::load(gcx.clone()).await,
        EvidenceContext {
            worktree: worktree.clone(),
            commit: commit.clone(),
            scope: card.id.clone(),
        },
    );
    let mut runner = CardEvidenceRunner {
        gcx: gcx.clone(),
        actor,
        request: request.clone(),
        candidate,
        expected,
        output_records: std::sync::Mutex::new(Default::default()),
    };
    runner.revalidate().await?;
    let cache = crate::chat::evidence_cache::evidence_cache();
    {
        let mut guard = cache.lock().await;
        guard.set_limits(crate::chat::evidence_cache::EvidenceCacheLimits::from_settings());
        tracing::debug!(
            card_id = %card.id,
            entries = guard.len(),
            bytes = guard.bytes(),
            "evidence cache bounds applied before verification"
        );
    }
    let evidence = crate::chat::evidence_cache::execute_cached_all(
        &service,
        cache,
        specs.clone(),
        &mut runner,
        crate::chat::evidence_cache::CacheMode::Revalidate,
    )
    .await;
    *attempted = evidence
        .iter()
        .map(EvidenceRecord::to_verification_result)
        .collect();
    runner.revalidate().await?;
    let evidence = supersede_stale(&worktree, commit.as_deref(), &specs, evidence).await;
    let command_results = evidence
        .iter()
        .map(EvidenceRecord::to_verification_result)
        .collect::<Vec<_>>();
    let mut gates = evaluate_gates(&specs, &evidence);
    for note in skipped_instruction_notes
        .iter()
        .filter(|note| note.starts_with("Verify line not executable:"))
    {
        gates.blocks.push(crate::chat::evidence::GateBlock {
            spec_id: String::new(),
            command: String::new(),
            verdict: crate::chat::evidence::EvidenceVerdict::NotRun,
            exit_code: None,
            reason: format!("not run: {}", note),
            from_cache: false,
        });
    }
    let deliverable = match assignment.as_ref() {
        Some(assignment) => assignment.deliverable.clone(),
        None => crate::tasks::assignment::typed_parts(&card)?.0,
    };
    if let Some(contract) = deliverable {
        let status = contract.evaluate(card.final_report.as_deref().unwrap_or_default());
        if !status.met {
            gates.blocks.push(crate::chat::evidence::GateBlock {
                spec_id: format!("{}:deliverable", card.id),
                command: String::new(),
                verdict: crate::chat::evidence::EvidenceVerdict::Failed,
                exit_code: None,
                reason: format!(
                    "Structural deliverable check failed: {}",
                    serde_json::to_string(&status).unwrap_or_default()
                ),
                from_cache: false,
            });
        }
    }
    gates
        .blocks
        .extend(crate::coordination::disagreement::completion_gate_blocks(&app).await);
    let reported_success = card
        .final_report_structured
        .as_ref()
        .is_some_and(|report| report.success);
    let decision = completion_decision(&gates, reported_success);
    let mut concerns = decision.reasons;
    concerns.extend(skipped_instruction_notes);
    concerns.extend(specs.iter().map(|spec| {
        let source = if assignment.as_ref().is_some_and(|assignment| {
            assignment
                .verify
                .iter()
                .any(|frozen| frozen.spec_id == spec.spec_id)
        }) {
            "frozen assignment typed spec"
        } else {
            "live planner card Verify line"
        };
        format!("Command source: `{}` — {}", spec.command, source)
    }));
    if decision.reported_success && !decision.allowed {
        concerns.insert(
            0,
            "Agent reported success, but a required verification gate did not pass".to_string(),
        );
    }

    let mut notes = Vec::new();
    let (review_unavailable, review_has_concerns, review_error) = if gates.judge_allowed() {
        runner.revalidate().await?;
        let meta = runner.candidate.metadata();
        let worktree_base = Some((meta.base_commit.clone(), meta.base_branch.clone()));
        let card_pair_present = card.base_commit.is_some() || card.base_branch.is_some();
        let (base_commit, base_branch) = if card_pair_present {
            (card.base_commit.clone(), card.base_branch.clone())
        } else if let Some((commit, branch)) = worktree_base {
            if commit.is_some() || branch.is_some() {
                (commit, branch)
            } else {
                (task_meta.base_commit, task_meta.base_branch)
            }
        } else {
            (task_meta.base_commit, task_meta.base_branch)
        };
        let diff_base = resolve_verifier_diff_base(base_commit, base_branch)?;
        let diff = git_changed_files_summary(&worktree, &diff_base, MAX_DIFF_LINES)
            .await
            .unwrap_or_else(|error| format!("diff unavailable: {}", error));
        let prompt = verifier_prompt(&card, &command_results, &diff);
        let mut paths = vec![
            worktree.clone(),
            runner
                .candidate
                .metadata()
                .source_workspace_root
                .join(".refact/tasks")
                .join(&request.task_id)
                .join("board.yaml"),
        ];
        paths.extend(
            diff.lines()
                .skip(2)
                .filter(|line| !line.starts_with('('))
                .map(|line| worktree.join(line)),
        );
        let mut records = crate::privacy::records::declared_file_records(&gcx, paths)
            .map_err(|_| "verification review provenance unavailable")?;
        let (observed, complete) = runner
            .output_records
            .lock()
            .map_err(|_| "verification provenance unavailable")?
            .snapshot(0, &crate::file_activity::settings());
        if !complete {
            return Err("verification provenance bound exceeded".into());
        }
        records.extend(observed);
        let review =
            run_verifier_review(gcx.clone(), prompt, card.agent_chat_id.as_deref(), records).await;
        let review_unavailable = review.is_err();
        let review_error = review.as_ref().err().cloned();
        let findings = review.unwrap_or_else(|error| ReviewFindings {
            concerns: vec![if verifier_model_unavailable(Some(&error)) {
                format!("Verifier model unavailable; infrastructure unavailable: {error}")
            } else {
                format!("Verifier review subchat unavailable; human review recommended: {error}")
            }],
            informational: Vec::new(),
        });
        notes.extend(findings.informational);
        let model_concerns = findings.concerns;
        let review_has_concerns = !review_unavailable && !model_concerns.is_empty();
        concerns.extend(model_concerns.into_iter().map(|concern| {
            if review_unavailable {
                concern
            } else {
                format!("ADVISORY: {}", concern)
            }
        }));
        (review_unavailable, review_has_concerns, review_error)
    } else {
        (false, false, None)
    };

    let classification =
        if review_unavailable && verifier_model_unavailable(review_error.as_deref()) {
            VerifierReportClassification::Unavailable
        } else {
            classify_verifier_report(&gates, review_unavailable, review_has_concerns)
        };
    let passed = classification == VerifierReportClassification::Passed;
    let recommendation = match classification {
        VerifierReportClassification::Passed => "merge",
        VerifierReportClassification::HumanReview | VerifierReportClassification::Unavailable => {
            "human-review"
        }
        VerifierReportClassification::VerificationFailed
        | VerifierReportClassification::Unknown => "fix-needed",
    }
    .to_string();

    let report = VerifierReport {
        passed,
        command_results,
        concerns,
        notes,
        recommendation,
        classification,
    };
    *settled = Some(report.clone());
    runner.revalidate().await?;
    let stored = store_verifier_report(
        gcx,
        &request.task_id,
        &request.card_id,
        Some(&runner.expected),
        report.clone(),
        specs.iter().map(|spec| spec.spec_id.clone()).collect(),
    )
    .await?;
    if !stored {
        return Err(stale_verifier_error(&request.task_id, &request.card_id));
    }
    Ok(report)
}

pub(crate) fn outcome_is_infrastructure(outcome: &VerificationOutcome) -> bool {
    matches!(
        outcome,
        VerificationOutcome::InfrastructureFailed
            | VerificationOutcome::Rejected
            | VerificationOutcome::PolicyDenied
    )
}

/// Gates first, judge second. A blocked gate settles the classification on its
/// own; only once every required gate passed does the model's opinion matter.
pub(crate) fn classify_verifier_report(
    gates: &GateOutcome,
    review_unavailable: bool,
    review_has_concerns: bool,
) -> VerifierReportClassification {
    if !gates.completion_allowed() {
        if !gates.blocks.is_empty()
            && gates.blocks.iter().all(|block| {
                matches!(
                    block.verdict,
                    crate::chat::evidence::EvidenceVerdict::Unavailable
                        | crate::chat::evidence::EvidenceVerdict::NotRun
                        | crate::chat::evidence::EvidenceVerdict::Superseded
                )
            })
        {
            return VerifierReportClassification::Unavailable;
        }
        return if gates.has_contract_violation() {
            VerifierReportClassification::VerificationFailed
        } else {
            VerifierReportClassification::HumanReview
        };
    }
    if review_has_concerns {
        return VerifierReportClassification::HumanReview;
    }
    if review_unavailable {
        return VerifierReportClassification::HumanReview;
    }
    VerifierReportClassification::Passed
}

pub(crate) fn verification_failure_is_infrastructure(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    lower.contains("command timed out")
        || lower.contains("blocking waiting for file lock")
        || lower.contains("failed to spawn command")
        || lower.contains("could not find `cargo.toml`")
}

pub(crate) fn verification_outcome_for_exit(
    exit_code: Option<i32>,
    output: &str,
) -> VerificationOutcome {
    if verification_failure_is_infrastructure(output) {
        return VerificationOutcome::InfrastructureFailed;
    }
    match exit_code {
        Some(0) => VerificationOutcome::Passed,
        Some(_) => VerificationOutcome::CommandFailed,
        None => VerificationOutcome::InfrastructureFailed,
    }
}

pub(crate) fn verifier_command_env(
    command: &str,
    env: Vec<(String, String)>,
    skip_gui_build: Option<&str>,
) -> HashMap<String, String> {
    let mut env = env.into_iter().collect::<HashMap<_, _>>();
    if command == "cargo" {
        env.entry("CARGO_INCREMENTAL".to_string())
            .or_insert_with(|| "0".to_string());
        env.entry("RUST_MIN_STACK".to_string())
            .or_insert_with(|| "16777216".to_string());
        if let Some(skip_gui_build) = skip_gui_build {
            env.entry("REFACT_SKIP_GUI_BUILD".to_string())
                .or_insert_with(|| skip_gui_build.to_string());
        }
    }
    env
}

pub(crate) fn worktree_skip_gui_build(cwd: &Path, env: &HashMap<String, String>) -> Option<String> {
    let target_dir = env
        .get("CARGO_TARGET_DIR")
        .cloned()
        .or_else(|| std::env::var("CARGO_TARGET_DIR").ok())
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            }
        })
        .unwrap_or_else(|| cwd.join("target"));
    let build_dir = target_dir.join("debug").join("build");
    let entries = fs::read_dir(build_dir).ok()?;
    let mut newest: Option<(SystemTime, String)> = None;

    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("refact-lsp-")
        {
            continue;
        }
        let output_path = entry.path().join("output");
        let Ok(metadata) = fs::metadata(&output_path) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        let Ok(contents) = fs::read_to_string(output_path) else {
            continue;
        };
        if newest
            .as_ref()
            .map_or(true, |(newest_modified, _)| modified > *newest_modified)
        {
            newest = Some((modified, contents));
        }
    }

    let (_, output) = newest?;
    output.lines().find_map(|line| {
        line.strip_prefix("cargo:rustc-env=REFACT_SKIP_GUI_BUILD=")
            .map(str::to_string)
    })
}

fn verifier_model_unavailable(error: Option<&str>) -> bool {
    error.is_some_and(|error| {
        let lower = error.to_ascii_lowercase();
        lower.contains("model")
            && (lower.contains("not found")
                || lower.contains("not available")
                || lower.contains("no light/default model")
                || lower.contains("no verifier model")
                || lower.contains("no model configured"))
    })
}

pub(crate) fn unavailable_verifier_report(error: String) -> VerifierReport {
    let model_unavailable = verifier_model_unavailable(Some(&error));
    VerifierReport {
        passed: false,
        command_results: Vec::new(),
        notes: Vec::new(),
        concerns: vec![if model_unavailable {
            format!("Verifier model unavailable; infrastructure unavailable: {error}")
        } else {
            format!("Verifier failed to launch; human review recommended: {error}")
        }],
        recommendation: "human-review".to_string(),
        classification: if model_unavailable {
            VerifierReportClassification::Unavailable
        } else {
            VerifierReportClassification::HumanReview
        },
    }
}

/// The single confined exec path for verification commands. The evidence service
/// and the post-merge runner both reach execution through here.
pub(crate) async fn run_verification_argv_impl(
    gcx: Arc<GlobalContext>,
    worktree: &Path,
    command: &str,
    cwd: Option<PathBuf>,
    env: Vec<(String, String)>,
    argv: Vec<String>,
    timeout: Duration,
    drain_timeout: Duration,
) -> VerificationResult {
    run_verification_argv_scoped(
        gcx,
        worktree,
        command,
        cwd,
        env,
        argv,
        timeout,
        drain_timeout,
        None,
    )
    .await
}

struct VerifierOutputAudit(crate::call_validation::ChatMessage);

impl refact_privacy::PrivacyAudited for VerifierOutputAudit {
    fn privacy_records(
        &self,
    ) -> Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError> {
        refact_privacy::records_from_messages(std::slice::from_ref(&self.0))
    }
}

fn unrun_verification(
    command: &str,
    reason: String,
    outcome: VerificationOutcome,
) -> VerificationResult {
    VerificationResult {
        command: command.into(),
        exit_code: None,
        passed: false,
        output_tail: reason,
        outcome,
        from_cache: false,
        executed_at: None,
    }
}

async fn run_verification_argv_scoped(
    gcx: Arc<GlobalContext>,
    worktree: &Path,
    command: &str,
    cwd: Option<PathBuf>,
    env: Vec<(String, String)>,
    argv: Vec<String>,
    timeout: Duration,
    drain_timeout: Duration,
    runner: Option<&CardEvidenceRunner>,
) -> VerificationResult {
    if argv.is_empty() {
        return VerificationResult {
            command: command.to_string(),
            exit_code: None,
            passed: false,
            output_tail: "empty verification command".to_string(),
            outcome: VerificationOutcome::Rejected,
            from_cache: false,
            executed_at: None,
        };
    };
    let effective_cwd = cwd.map_or_else(|| worktree.to_path_buf(), |cwd| worktree.join(cwd));
    if let Err(reason) = check_cwd_in_worktree(worktree, &effective_cwd).await {
        return VerificationResult {
            command: command.to_string(),
            exit_code: None,
            passed: false,
            output_tail: reason,
            outcome: VerificationOutcome::InfrastructureFailed,
            from_cache: false,
            executed_at: None,
        };
    }
    let effective_cwd = if runner.is_some() {
        match tokio::fs::canonicalize(&effective_cwd).await {
            Ok(cwd) if cwd.starts_with(worktree) => dunce::simplified(&cwd).to_path_buf(),
            _ => {
                return unrun_verification(
                    command,
                    "candidate cwd is unavailable or escaped".into(),
                    VerificationOutcome::Rejected,
                )
            }
        }
    } else {
        effective_cwd
    };
    if let Some(runner) = runner {
        if let Err(error) = runner.revalidate().await {
            return unrun_verification(command, error, VerificationOutcome::Rejected);
        }
        if let Err(error) = runner.revalidate().await {
            return unrun_verification(command, error, VerificationOutcome::Rejected);
        }
        let policy = crate::tools::shell_gate::load_policy(gcx.clone()).await;
        let gate = crate::tools::shell_gate::evaluate(
            command,
            &policy,
            &crate::tools::shell_gate::GateContext {
                workspace_roots: vec![runner.candidate.root().to_path_buf()],
                needs_confirmation: false,
            },
        );
        if gate.decision.result != refact_tool_api::MatchConfirmDenyResult::PASS {
            return unrun_verification(
                command,
                format!("not run: {}", gate.reason),
                VerificationOutcome::PolicyDenied,
            );
        }
    }
    let verifier_env = env.into_iter().collect::<HashMap<_, _>>();
    let command_name = argv.first().map(String::as_str).unwrap_or_default();
    let skip_gui_build = if command_name == "cargo" {
        worktree_skip_gui_build(&effective_cwd, &verifier_env)
    } else {
        None
    };
    let input = CommandPolicyInput {
        source: ExecSource::Verifier,
        command: CommandKind::Argv(&argv),
        cwd: Some(effective_cwd.clone()),
        env: verifier_command_env(
            command_name,
            verifier_env.into_iter().collect(),
            skip_gui_build.as_deref(),
        ),
        chat_mode: None,
        escalation: None,
    };
    let policy = if let Some(runner) = runner {
        crate::exec::command_policy::build_verifier_exec_request(
            gcx.clone(),
            input,
            &runner.candidate,
        )
        .await
    } else {
        build_exec_request(gcx.clone(), input).await
    };
    let mut request = match policy {
        Ok(policy) => policy
            .request
            .with_timeout(timeout)
            .with_output_drain_timeout(drain_timeout)
            .with_transcript_limit(MAX_OUTPUT_CAPTURE_BYTES * 2),
        Err(error) => {
            return VerificationResult {
                command: command.to_string(),
                exit_code: None,
                passed: false,
                output_tail: format!("failed to spawn command: {}", error.message),
                outcome: VerificationOutcome::InfrastructureFailed,
                from_cache: false,
                executed_at: None,
            };
        }
    };
    if let Some(runner) = runner {
        request = request
            .with_abort_flag(runner.abort_flag().await)
            .with_observe(true)
            .with_owner(crate::exec::ExecOwnerMeta {
                workspace: Some(runner.candidate.root().to_path_buf()),
                ..Default::default()
            });
        if let Err(error) = runner.revalidate().await {
            return unrun_verification(command, error, VerificationOutcome::Rejected);
        }
        if check_cwd_in_worktree(worktree, &effective_cwd)
            .await
            .is_err()
        {
            return unrun_verification(
                command,
                "candidate cwd changed before spawn".into(),
                VerificationOutcome::Rejected,
            );
        }
    }
    let executed_at = Utc::now().to_rfc3339();
    let result = match gcx.exec_registry.spawn(request).await {
        Ok(result) => result,
        Err(error) => {
            return VerificationResult {
                command: command.to_string(),
                exit_code: None,
                passed: false,
                output_tail: format!("failed to spawn command: {}", error),
                outcome: VerificationOutcome::InfrastructureFailed,
                from_cache: false,
                executed_at: None,
            };
        }
    };
    let read = gcx
        .exec_registry
        .read(&result.snapshot.meta.process_id, 0, None)
        .await;
    let mut stdout = String::new();
    let mut stderr = String::new();
    for chunk in read.chunks {
        match chunk.stream {
            ExecOutputStream::Stdout | ExecOutputStream::Combined => stdout.push_str(&chunk.text),
            ExecOutputStream::Stderr => stderr.push_str(&chunk.text),
        }
    }
    let mut output = format!("{stdout}{stderr}");
    if let Some(runner) = runner {
        let clearance = async {
            let model = runner
                .model()
                .await
                .map_err(|_| "verification destination unavailable")?;
            let destination = crate::privacy::records::provider_destination(&model);
            let mut message = crate::call_validation::ChatMessage {
                role: "tool".into(),
                content: crate::call_validation::ChatContent::SimpleText(output.clone()),
                ..Default::default()
            };
            let decision = crate::privacy::records::apply_shell_observation(
                &gcx,
                command,
                &effective_cwd,
                &destination,
                result.observation.clone(),
                &crate::privacy::records::new_derived_privacy_zones(),
                &mut message,
            )
            .await
            .map_err(|_| "verification output privacy check unavailable")?;
            if !matches!(decision, crate::privacy::records::ShellReadDecision::Pass) {
                return Err(
                    "verification output requires privacy approval or is withheld".to_string(),
                );
            }
            let caps = try_load_caps_quickly_if_not_present(gcx.clone(), 0)
                .await
                .map_err(|_| "verification destination unavailable")?;
            let record = crate::caps::resolve_chat_model(caps, &model)
                .map_err(|_| "verification destination unavailable")?;
            crate::privacy::destinations::clear_for_model(
                &gcx,
                VerifierOutputAudit(message.clone()),
                &record.base,
            )
            .map_err(|_| "verification output is not cleared for destination")?;
            if let Ok(review_model) = resolve_verifier_model(gcx.clone()).await {
                let caps = try_load_caps_quickly_if_not_present(gcx.clone(), 0)
                    .await
                    .map_err(|_| "verification review destination unavailable")?;
                let review_record = crate::caps::resolve_chat_model(caps, &review_model)
                    .map_err(|_| "verification review destination unavailable")?;
                crate::privacy::destinations::clear_for_model(
                    &gcx,
                    VerifierOutputAudit(message.clone()),
                    &review_record.base,
                )
                .map_err(|_| "verification output is not cleared for review destination")?;
            }
            let settings = crate::file_activity::settings();
            let mut provenance = runner
                .output_records
                .lock()
                .map_err(|_| "verification provenance unavailable")?;
            provenance.observe(&message, 0, &settings);
            if !provenance.snapshot(0, &settings).1 {
                return Err("verification provenance bound exceeded".to_string());
            }
            Ok::<_, String>(message.content.content_text_only())
        }
        .await;
        match clearance {
            Ok(cleared) => output = cleared,
            Err(error) => {
                let exit_code = match &result.snapshot.status {
                    ExecStatus::Exited {
                        exit_code,
                    } => *exit_code,
                    _ => None,
                };
                return VerificationResult {
                    command: command.into(),
                    exit_code,
                    passed: false,
                    output_tail: format!("command attempted; output withheld: {error}"),
                    outcome: VerificationOutcome::PolicyDenied,
                    from_cache: false,
                    executed_at: Some(executed_at.clone()),
                };
            }
        }
    }
    let (exit_code, passed, outcome) = match &result.snapshot.status {
        ExecStatus::Exited {
            exit_code,
        } => (
            *exit_code,
            exit_code == &Some(0),
            verification_outcome_for_exit(*exit_code, &output),
        ),
        ExecStatus::SandboxLauncherFailed {
            exit_code,
        } => {
            output.push_str(&format!(
                "sandbox launcher failed before command execution with exit code {exit_code}"
            ));
            (
                Some(*exit_code),
                false,
                VerificationOutcome::InfrastructureFailed,
            )
        }
        ExecStatus::TimedOut => {
            output.push_str(&format!(
                "command timed out after {} seconds",
                timeout.as_secs()
            ));
            (None, false, VerificationOutcome::InfrastructureFailed)
        }
        ExecStatus::Failed {
            message,
        } => {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(message);
            (None, false, VerificationOutcome::InfrastructureFailed)
        }
        ExecStatus::Killed | ExecStatus::Starting | ExecStatus::Running => {
            (None, false, VerificationOutcome::InfrastructureFailed)
        }
    };
    VerificationResult {
        command: command.to_string(),
        exit_code,
        passed,
        output_tail: tail_chars(&output, MAX_OUTPUT_TAIL_CHARS),
        outcome,
        from_cache: false,
        executed_at: Some(executed_at),
    }
}

fn verifier_prompt(card: &BoardCard, commands: &[VerificationResult], diff: &str) -> String {
    format!(
        "Review this completed task card. Return concise ADVISORY concerns only. Deterministic gates already passed; your opinion cannot establish a verification failure. Only requirements written in the card apply; do not invent checks. For informational observations without verifier evidence, use a separate line prefixed 'not run: <reason>'. Negative-control steps are informational and are not executed by the verifier. If the changed files look safe and commands passed, answer exactly PASS.\n\nCard: {} - {}\n\nInstructions:\n{}\n\nFinal report:\n{}\n\nCommand results:\n{}\n\nChanged files:\n{}",
        card.id,
        card.title,
        card.instructions,
        card.final_report.as_deref().unwrap_or(""),
        serde_json::to_string_pretty(commands).unwrap_or_default(),
        diff
    )
}

async fn run_verifier_review(
    gcx: Arc<GlobalContext>,
    prompt: String,
    parent_chat_id: Option<&str>,
    records: Vec<refact_privacy::FileRecord>,
) -> Result<ReviewFindings, String> {
    let model = resolve_verifier_model(gcx.clone()).await?;
    let config = crate::subchat::SubchatConfig {
        runner_lease_token: None,
        native_job_fence: None,
        history_source_model: None,
        tool_name: "verifier".to_string(),
        stateful: false,
        autonomous_no_confirm: true,
        auto_approve_editing_tools: false,
        auto_approve_dangerous_commands: false,
        chat_id: None,
        title: None,
        parent_id: None,
        link_type: None,
        root_chat_id: None,
        tools: crate::subchat::ToolsPolicy::None,
        max_steps: 1,
        prepend_system_prompt: false,
        wrap_up: None,
        task_meta: None,
        worktree: None,
        model,
        mode: "agent".to_string(),
        n_ctx: 32_000,
        implicit_context_window: false,
        max_new_tokens: 1024,
        temperature: Some(0.0),
        reasoning_effort: None,
        cache_control: crate::llm::params::CacheControl::Ephemeral,
        parent_tool_call_id: None,
        parent_subchat_tx: None,
        abort_flag: None,
        soft_abort: false,
        activity_stamp: None,
        background_agent_id: None,
        subchat_depth: 1,
        final_step_force_answer: false,
        buddy_meta: None,
        step_progress: None,
        trace_parent: crate::subchat::TraceParent::from_parts(parent_chat_id, None),
    };
    let mut message = event(
        EventSubkind::VerifierReport,
        VERIFIER_SOURCE,
        json!({ "kind": "verifier_review_prompt" }),
        prompt,
    );
    crate::privacy::records::merge_records(&mut message, records);
    let result = crate::subchat::run_subchat(gcx, vec![message], config).await?;
    let answer = result
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant")
        .map(|message| message.content.content_text_only())
        .unwrap_or_default();
    Ok(parse_review_concerns(&answer))
}

async fn resolve_verifier_model(gcx: Arc<GlobalContext>) -> Result<String, String> {
    let caps = try_load_caps_quickly_if_not_present(gcx, 0)
        .await
        .map_err(|e| e.message.clone())?;
    for model in [
        caps.defaults.chat_light_model.as_str(),
        caps.defaults.chat_default_model.as_str(),
    ] {
        if model.is_empty() {
            continue;
        }
        if crate::caps::resolve_model(&caps.chat_models, model).is_ok() {
            return Ok(model.to_string());
        }
        return Err(format!(
            "verifier model '{model}' is not available in the model catalog"
        ));
    }
    Err("no light/default model configured for verifier".to_string())
}

#[derive(Default)]
struct ReviewFindings {
    concerns: Vec<String>,
    informational: Vec<String>,
}

fn parse_review_concerns(answer: &str) -> ReviewFindings {
    let mut findings = ReviewFindings::default();
    for line in answer
        .lines()
        .map(|line| line.trim().trim_start_matches(['-', '*', ' ']).trim())
        .filter(|line| !line.is_empty() && !line.eq_ignore_ascii_case("pass"))
    {
        if line
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("not run:"))
        {
            findings.informational.push(line.to_string());
        } else {
            findings.concerns.push(line.to_string());
        }
    }
    findings
}

fn tail_chars(text: &str, max_chars: usize) -> String {
    let len = text.chars().count();
    if len <= max_chars {
        return text.to_string();
    }
    text.chars().skip(len - max_chars).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::chat::evidence::{gate_outcome_for_verdicts, specs_from_card, EvidenceVerdict};
    use crate::tasks::types::{FinalReport, ScopeGuardMode, TaskBoard, TaskMeta, TaskStatus};

    fn run_in_isolated_verifier_process(test_name: &str) -> bool {
        const MARKER: &str = "REFACT_TEST_VERIFIER_REPORT_PROCESS";
        if std::env::var(MARKER).ok().as_deref() == Some(test_name) {
            return false;
        }
        let output = tempfile::NamedTempFile::new().expect("create isolated verifier output");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--nocapture", "--test-threads=1"])
            .env(MARKER, test_name)
            .stdout(output.as_file().try_clone().unwrap())
            .stderr(output.as_file().try_clone().unwrap())
            .spawn()
            .expect("spawn isolated verifier test");
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            let status = match child.try_wait() {
                Ok(status) => status,
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("poll isolated verifier test failed: {error}");
                }
            };
            if let Some(status) = status {
                let output =
                    fs::read_to_string(output.path()).expect("read isolated verifier test");
                assert!(
                    status.success(),
                    "isolated verifier test failed: {status}\n{output}"
                );
                assert!(
                    output.contains("running 1 test")
                        && output.contains("1 passed; 0 failed; 0 ignored;"),
                    "isolated verifier test must run exactly one non-ignored test: {output}"
                );
                return true;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("isolated verifier test exceeded 60 seconds: {test_name}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub(crate) async fn register_candidate(
        gcx: Arc<GlobalContext>,
        task_id: &str,
        card_id: &str,
        model: &str,
    ) -> Arc<tokio::sync::Mutex<AtCommandsContext>> {
        use crate::chat::types::{ChatSession, TaskMeta as ThreadTaskMeta};
        let source = crate::files_correction::get_project_dirs(gcx.clone())
            .await
            .remove(0);
        crate::privacy::save_privacy_policy(gcx.clone(), Default::default())
            .await
            .unwrap();
        let service =
            crate::worktrees::service::WorktreeService::new(gcx.cache_dir.clone(), source.clone())
                .unwrap();
        let view = service
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                kind: Some("task_agent".into()),
                task_id: Some(task_id.into()),
                card_id: Some(card_id.into()),
                agent_id: Some("worker".into()),
                chat_id: Some("worker-chat".into()),
                branch: Some(format!(
                    "refact/task/{task_id}/card/{card_id}/{}",
                    uuid::Uuid::new_v4()
                )),
                base_branch: Some("main".into()),
                ..Default::default()
            })
            .await
            .unwrap()
            .worktree;
        let mut board = storage::load_board(gcx.clone(), task_id).await.unwrap();
        board.owner_chat_id = Some("planner".into());
        let card = board.get_card_mut(card_id).unwrap();
        card.assignee = Some("worker".into());
        card.agent_chat_id = Some("worker-chat".into());
        card.agent_worktree = Some(view.meta.root.to_string_lossy().into_owned());
        card.agent_worktree_name = Some(view.meta.id.clone());
        card.agent_branch = view.meta.branch.clone();
        storage::save_board(gcx.clone(), task_id, &board)
            .await
            .unwrap();
        let app = AppState::from_gcx(gcx.clone()).await;
        let task = ThreadTaskMeta {
            task_id: task_id.into(),
            role: "planner".into(),
            ..Default::default()
        };
        let mut parent = ChatSession::new("planner".into());
        parent.thread.task_meta = Some(task.clone());
        parent.thread.mode = "task_planner".into();
        parent.thread.model = model.into();
        parent.work_project_root = Some(source);
        let mut child = ChatSession::new("worker-chat".into());
        child.thread = crate::tools::tool_task_spawn_agent::build_agent_thread_params(
            "worker-chat",
            "Verifier fixture",
            model,
            task_id,
            "worker",
            card_id,
            "planner",
            view.meta,
        );
        child.runtime.state = refact_runtime_api::SessionState::Completed;
        {
            let mut sessions = app.chat.sessions.write().await;
            sessions.insert("planner".into(), Arc::new(tokio::sync::Mutex::new(parent)));
            sessions.insert(
                "worker-chat".into(),
                Arc::new(tokio::sync::Mutex::new(child)),
            );
        }
        let (defaults_root, _) = crate::caps::project_model_defaults_state(gcx.clone()).await;
        {
            let mut state = gcx.caps_state.write().await;
            let mut caps = state.caps.as_deref().cloned().unwrap_or_default();
            caps.chat_models.entry(model.into()).or_insert_with(|| {
                Arc::new(crate::caps::ChatModelRecord {
                    base: crate::caps::BaseModelRecord {
                        id: model.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                })
            });
            state.caps = Some(Arc::new(caps));
            state.loaded_project_root = Some(defaults_root);
            state.last_attempted_ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
        }
        Arc::new(tokio::sync::Mutex::new(
            AtCommandsContext::new_from_app(
                app,
                4096,
                20,
                false,
                vec![],
                "planner".into(),
                None,
                model.into(),
                Some(task),
                None,
            )
            .await,
        ))
    }

    pub(crate) fn init_repo(root: &Path) {
        for args in [
            vec!["init", "-b", "main"],
            vec!["config", "user.name", "Test"],
            vec!["config", "user.email", "test@example.com"],
        ] {
            crate::worktrees::git::run_git(root, &args).unwrap();
        }
        std::fs::write(root.join("tracked"), "base").unwrap();
        crate::worktrees::git::run_git(root, &["add", "tracked"]).unwrap();
        crate::worktrees::git::run_git(root, &["commit", "-m", "base"]).unwrap();
    }

    fn card(instructions: &str) -> BoardCard {
        BoardCard {
            id: "T-verify".to_string(),
            title: "Verifier card".to_string(),
            column: "done".to_string(),
            priority: "P1".to_string(),
            depends_on: Vec::new(),
            instructions: instructions.to_string(),
            assignee: None,
            agent_chat_id: None,
            status_updates: Vec::new(),
            comments: vec![],
            final_report: Some("done".to_string()),
            final_report_structured: None,
            verifier_report: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: None,
            last_heartbeat_at: None,
            paused_at: None,
            paused_by: None,
            completed_at: Some(Utc::now().to_rfc3339()),
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
            base_branch: None,
            base_commit: None,
            ab_variants: None,
            team_members: vec![],
            target_files: Vec::new(),
            scope_guard_mode: ScopeGuardMode::Off,
            attention: None,
            deliverable: None,
            verification: Vec::new(),
        }
    }

    fn verifier_report(passed: bool) -> VerifierReport {
        VerifierReport {
            passed,
            notes: Vec::new(),
            command_results: vec![VerificationResult {
                command: "cargo test verifier".to_string(),
                exit_code: Some(if passed { 0 } else { 1 }),
                passed,
                output_tail: "ok".to_string(),
                outcome: VerificationOutcome::Unknown,
                from_cache: false,
                executed_at: None,
            }],
            concerns: if passed {
                Vec::new()
            } else {
                vec!["failed".to_string()]
            },
            recommendation: if passed { "merge" } else { "fix-needed" }.to_string(),
            classification: if passed {
                VerifierReportClassification::Passed
            } else {
                VerifierReportClassification::VerificationFailed
            },
        }
    }

    async fn write_task(root: &Path, gcx: Arc<GlobalContext>, card: BoardCard) {
        let task_id = "task-1";
        let task_dir = root.join(".refact").join("tasks").join(task_id);
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
        let now = Utc::now().to_rfc3339();
        let meta = TaskMeta {
            schema_version: 1,
            id: task_id.to_string(),
            name: "Task".to_string(),
            status: TaskStatus::Active,
            created_at: now.clone(),
            updated_at: now,
            cards_total: 1,
            cards_done: 1,
            cards_failed: 0,
            agents_active: 0,
            base_branch: Some("main".to_string()),
            base_commit: None,
            default_agent_model: None,
            is_name_generated: false,
            last_agents_summary_at: None,
            planner_session_state: None,
        };
        storage::save_task_meta(gcx.clone(), task_id, &meta)
            .await
            .unwrap();
        storage::save_board(
            gcx,
            task_id,
            &TaskBoard {
                cards: vec![card],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    #[test]
    fn verifier_commands_include_acceptance_verify_lines() {
        let card = card(
            "## Acceptance Criteria\n- verifier.rs created\n- Verify: `cargo test --lib -p refact-lsp -- verifier merge_agent`",
        );

        assert_eq!(
            crate::chat::evidence::specs_from_card(
                &card,
                &EvidenceDefaults {
                    timeout_secs: 60,
                    output_cap_bytes: 4_000,
                },
            )
            .into_iter()
            .map(|spec| spec.command)
            .collect::<Vec<_>>(),
            vec!["cargo test --lib -p refact-lsp -- verifier merge_agent".to_string()]
        );
    }

    #[test]
    fn verifier_commands_ignore_structured_final_report_commands() {
        let mut card = card("## Acceptance Criteria\n- [ ] done");
        card.final_report_structured = Some(FinalReport {
            verification: vec![VerificationResult {
                command: "cargo test --lib -p refact-lsp -- verifier".to_string(),
                exit_code: Some(0),
                passed: true,
                output_tail: "ok".to_string(),
                outcome: VerificationOutcome::Unknown,
                from_cache: false,
                executed_at: None,
            }],
            ..Default::default()
        });

        assert!(crate::chat::evidence::specs_from_card(
            &card,
            &EvidenceDefaults {
                timeout_secs: 60,
                output_cap_bytes: 4_000,
            },
        )
        .is_empty());
    }

    #[test]
    fn informational_review_preserves_notes_without_overriding_gates() {
        let findings = parse_review_concerns("- NoT RuN: negative control is informational");
        assert_eq!(
            findings.informational,
            vec!["NoT RuN: negative control is informational"]
        );
        assert!(findings.concerns.is_empty());
        let passed = gate_outcome_for_verdicts(&[(true, EvidenceVerdict::Passed)]);
        assert_eq!(
            classify_verifier_report(&passed, false, !findings.concerns.is_empty()),
            VerifierReportClassification::Passed
        );
        let mixed =
            parse_review_concerns("not run: optional check\ntest was not run, therefore insecure");
        assert_eq!(mixed.concerns, vec!["test was not run, therefore insecure"]);
        assert_eq!(
            classify_verifier_report(&passed, false, !mixed.concerns.is_empty()),
            VerifierReportClassification::HumanReview
        );
        let missing = gate_outcome_for_verdicts(&[(true, EvidenceVerdict::NotRun)]);
        assert_eq!(
            classify_verifier_report(&missing, false, !findings.concerns.is_empty()),
            VerifierReportClassification::Unavailable
        );
    }

    #[test]
    fn typed_classification_does_not_infer_state_from_concern_text() {
        let unavailable = gate_outcome_for_verdicts(&[(true, EvidenceVerdict::Unavailable)]);
        assert_eq!(
            classify_verifier_report(&unavailable, false, false),
            VerifierReportClassification::Unavailable
        );
        let passed = gate_outcome_for_verdicts(&[(true, EvidenceVerdict::Passed)]);
        assert_eq!(
            classify_verifier_report(&passed, true, false),
            VerifierReportClassification::HumanReview
        );
        assert_eq!(
            classify_verifier_report(&passed, false, true),
            VerifierReportClassification::HumanReview
        );
        assert_eq!(
            classify_verifier_report(&passed, false, false),
            VerifierReportClassification::Passed
        );
    }

    #[tokio::test]
    async fn store_verifier_report_lands_when_expected_card_state_matches() {
        if run_in_isolated_verifier_process(
            "chat::verifier::tests::store_verifier_report_lands_when_expected_card_state_matches",
        ) {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        init_repo(temp.path());
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut card = card("## Acceptance Criteria\n- Verify: `cargo test verifier`");
        card.id = "T-1".to_string();
        write_task(temp.path(), gcx.clone(), card).await;
        let ccx = register_candidate(gcx.clone(), "task-1", "T-1", "model").await;
        let candidate = command_checkout::resolve_planner_verifier(&ccx, "task-1", "T-1")
            .await
            .unwrap();
        let mut expected = load_expected_card_state(gcx.clone(), "task-1", "T-1")
            .await
            .unwrap();

        let board = storage::load_board(gcx.clone(), "task-1").await.unwrap();
        expected.verification = Some(
            VerificationSnapshot::capture(
                &gcx,
                temp.path().to_path_buf(),
                &candidate,
                "task-1",
                "T-1",
                &board,
            )
            .await
            .unwrap(),
        );
        let meta_before = fs::read(temp.path().join(".refact/tasks/task-1/meta.yaml")).unwrap();
        storage::update_board_atomic(gcx.clone(), "task-1", |board| {
            let mut other = board.get_card("T-1").unwrap().clone();
            other.id = "T-2".into();
            other.column = "planned".into();
            other.assignee = None;
            other.agent_chat_id = None;
            other.agent_worktree = None;
            other.agent_worktree_name = None;
            other.agent_branch = None;
            board.cards.push(other);
            Ok(())
        })
        .await
        .unwrap();
        let meta = storage::update_task_stats(gcx.clone(), "task-1")
            .await
            .unwrap();
        assert_eq!(meta.cards_total, 2);
        assert_ne!(
            fs::read(temp.path().join(".refact/tasks/task-1/meta.yaml")).unwrap(),
            meta_before
        );
        let mut caps = gcx.caps_state.write().await;
        caps.caps = Some(Arc::new(caps.caps.as_deref().unwrap().clone()));
        gcx.tool_catalog_generations.advance_capabilities();
        let pending = store_verifier_report(
            gcx.clone(),
            "task-1",
            "T-1",
            Some(&expected),
            verifier_report(true),
            Vec::new(),
        );
        tokio::pin!(pending);
        assert!(futures::poll!(&mut pending).is_pending());
        drop(caps);
        let stored = pending.await.unwrap();

        assert!(stored);
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert!(card.verifier_report.as_ref().unwrap().passed);
        assert!(card
            .status_updates
            .iter()
            .any(|update| update.message == "Verifier: PASS"));
    }

    #[tokio::test]
    async fn store_verifier_report_rejects_runtime_settings_change_only() {
        if run_in_isolated_verifier_process(
            "chat::verifier::tests::store_verifier_report_rejects_runtime_settings_change_only",
        ) {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        init_repo(temp.path());
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut card = card("## Acceptance Criteria\n- Verify: `cargo test verifier`");
        card.id = "T-1".into();
        write_task(temp.path(), gcx.clone(), card).await;
        let ccx = register_candidate(gcx.clone(), "task-1", "T-1", "model").await;
        let candidate = command_checkout::resolve_planner_verifier(&ccx, "task-1", "T-1")
            .await
            .unwrap();
        let mut expected = load_expected_card_state(gcx.clone(), "task-1", "T-1")
            .await
            .unwrap();
        let board = storage::load_board(gcx.clone(), "task-1").await.unwrap();
        let snapshot = VerificationSnapshot::capture(
            &gcx,
            temp.path().to_path_buf(),
            &candidate,
            "task-1",
            "T-1",
            &board,
        )
        .await
        .unwrap();
        let before = ConfigurationSnapshot::capture(&gcx).await.unwrap();
        assert!(snapshot.matches(&gcx, &board, &before));
        expected.verification = Some(snapshot.clone());
        let board_path = temp.path().join(".refact/tasks/task-1/board.yaml");
        let meta_path = temp.path().join(".refact/tasks/task-1/meta.yaml");
        let board_bytes = fs::read(&board_path).unwrap();
        let meta_bytes = fs::read(&meta_path).unwrap();

        {
            let _settings = crate::tools::settings_guard::SettingsGuard::install(|settings| {
                settings.max_queue_size += 1;
            });
            let changed = ConfigurationSnapshot::capture(&gcx).await.unwrap();
            assert_eq!(changed.content.caps, before.content.caps);
            assert_eq!(changed.content.models, before.content.models);
            assert_eq!(
                changed.capabilities_generation,
                before.capabilities_generation
            );
            assert_eq!(
                changed.content.policy_settings[0],
                before.content.policy_settings[0]
            );
            assert_eq!(
                changed.content.policy_settings[1],
                before.content.policy_settings[1]
            );
            assert_ne!(
                changed.content.policy_settings[2],
                before.content.policy_settings[2]
            );
            // Even the original configuration fails solely on the live settings
            // fingerprint: every other input to matches is unchanged.
            assert!(!snapshot.matches(&gcx, &board, &before));
            assert!(!store_verifier_report(
                gcx.clone(),
                "task-1",
                "T-1",
                Some(&expected),
                verifier_report(true),
                Vec::new(),
            )
            .await
            .unwrap());
            assert_eq!(fs::read(&board_path).unwrap(), board_bytes);
            assert_eq!(fs::read(&meta_path).unwrap(), meta_bytes);
        }

        let restored = ConfigurationSnapshot::capture(&gcx).await.unwrap();
        assert_eq!(restored.content, before.content);
        assert!(snapshot.matches(&gcx, &board, &restored));
        assert!(store_verifier_report(
            gcx.clone(),
            "task-1",
            "T-1",
            Some(&expected),
            verifier_report(true),
            Vec::new(),
        )
        .await
        .unwrap());
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        assert!(
            board
                .get_card("T-1")
                .unwrap()
                .verifier_report
                .as_ref()
                .unwrap()
                .passed
        );
    }

    #[tokio::test]
    async fn store_verifier_report_noops_when_card_restarted() {
        let temp = tempfile::tempdir().unwrap();
        init_repo(temp.path());
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut card = card("## Acceptance Criteria\n- Verify: `cargo test verifier`");
        card.id = "T-1".to_string();
        card.agent_chat_id = Some("agent-old".to_string());
        write_task(temp.path(), gcx.clone(), card).await;
        let ccx = register_candidate(gcx.clone(), "task-1", "T-1", "model").await;
        let candidate = command_checkout::resolve_planner_verifier(&ccx, "task-1", "T-1")
            .await
            .unwrap();
        let mut expected = load_expected_card_state(gcx.clone(), "task-1", "T-1")
            .await
            .unwrap();

        let board = storage::load_board(gcx.clone(), "task-1").await.unwrap();
        expected.verification = Some(
            VerificationSnapshot::capture(
                &gcx,
                temp.path().to_path_buf(),
                &candidate,
                "task-1",
                "T-1",
                &board,
            )
            .await
            .unwrap(),
        );
        storage::update_board_atomic(gcx.clone(), "task-1", |board| {
            let card = board.get_card_mut("T-1").unwrap();
            card.column = "doing".to_string();
            card.agent_chat_id = Some("agent-new".to_string());
            card.assignee = Some("agent-new".to_string());
            card.final_report = None;
            card.final_report_structured = None;
            Ok(())
        })
        .await
        .unwrap();

        let stored = store_verifier_report(
            gcx.clone(),
            "task-1",
            "T-1",
            Some(&expected),
            verifier_report(false),
            Vec::new(),
        )
        .await
        .unwrap();

        assert!(!stored);
        let request = VerifyCardRequest {
            task_id: "task-1".into(),
            card_id: "T-1".into(),
            expected_state: Some(expected.clone()),
        };
        assert!(is_stale_verifier_error(
            &settle_verifier_failure(
                gcx.clone(),
                &request,
                Some(&expected),
                Vec::new(),
                "finished session unavailable".into(),
            )
            .await
            .unwrap_err()
        ));
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert!(card.verifier_report.is_none());
        assert_eq!(card.column, "doing");
        assert_eq!(card.agent_chat_id.as_deref(), Some("agent-new"));
    }

    #[tokio::test]
    async fn store_verifier_report_noops_when_card_cleaned_up_after_merge() {
        let temp = tempfile::tempdir().unwrap();
        init_repo(temp.path());
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut card = card("## Acceptance Criteria\n- Verify: `cargo test verifier`");
        card.id = "T-1".to_string();
        card.agent_branch = Some("refact/task/task-1/card/T-1/agent".to_string());
        card.agent_worktree = Some(temp.path().join("agent").to_string_lossy().to_string());
        card.agent_worktree_name = Some("wt-1".to_string());
        write_task(temp.path(), gcx.clone(), card).await;
        let ccx = register_candidate(gcx.clone(), "task-1", "T-1", "model").await;
        let candidate = command_checkout::resolve_planner_verifier(&ccx, "task-1", "T-1")
            .await
            .unwrap();
        let mut expected = load_expected_card_state(gcx.clone(), "task-1", "T-1")
            .await
            .unwrap();

        let board = storage::load_board(gcx.clone(), "task-1").await.unwrap();
        expected.verification = Some(
            VerificationSnapshot::capture(
                &gcx,
                temp.path().to_path_buf(),
                &candidate,
                "task-1",
                "T-1",
                &board,
            )
            .await
            .unwrap(),
        );
        storage::update_board_atomic(gcx.clone(), "task-1", |board| {
            let card = board.get_card_mut("T-1").unwrap();
            card.agent_branch = None;
            card.agent_worktree = None;
            card.agent_worktree_name = None;
            Ok(())
        })
        .await
        .unwrap();

        let stored = store_verifier_report(
            gcx.clone(),
            "task-1",
            "T-1",
            Some(&expected),
            verifier_report(false),
            Vec::new(),
        )
        .await
        .unwrap();

        assert!(!stored);
        let request = VerifyCardRequest {
            task_id: "task-1".into(),
            card_id: "T-1".into(),
            expected_state: Some(expected.clone()),
        };
        assert!(is_stale_verifier_error(
            &settle_verifier_failure(
                gcx.clone(),
                &request,
                Some(&expected),
                Vec::new(),
                "finish boundary unavailable".into(),
            )
            .await
            .unwrap_err()
        ));
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert!(card.verifier_report.is_none());
        assert!(card.agent_worktree.is_none());
        assert!(card.agent_worktree_name.is_none());
    }

    #[tokio::test]
    async fn store_verifier_report_noops_when_report_already_changed() {
        let temp = tempfile::tempdir().unwrap();
        init_repo(temp.path());
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut card = card("## Acceptance Criteria\n- Verify: `cargo test verifier`");
        card.id = "T-1".to_string();
        write_task(temp.path(), gcx.clone(), card).await;
        let ccx = register_candidate(gcx.clone(), "task-1", "T-1", "model").await;
        let candidate = command_checkout::resolve_planner_verifier(&ccx, "task-1", "T-1")
            .await
            .unwrap();
        let mut expected = load_expected_card_state(gcx.clone(), "task-1", "T-1")
            .await
            .unwrap();

        let board = storage::load_board(gcx.clone(), "task-1").await.unwrap();
        expected.verification = Some(
            VerificationSnapshot::capture(
                &gcx,
                temp.path().to_path_buf(),
                &candidate,
                "task-1",
                "T-1",
                &board,
            )
            .await
            .unwrap(),
        );
        let (started, waiting) = tokio::sync::oneshot::channel();
        let (release, released) = tokio::sync::oneshot::channel();
        let pending = async {
            started.send(()).unwrap();
            released.await.unwrap();
            store_verifier_report(
                gcx.clone(),
                "task-1",
                "T-1",
                Some(&expected),
                verifier_report(false),
                Vec::new(),
            )
            .await
            .unwrap()
        };
        tokio::pin!(pending);
        assert!(futures::poll!(&mut pending).is_pending());
        waiting.await.unwrap();
        let before = candidate.head().to_string();
        crate::worktrees::git::run_git(
            candidate.root(),
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--allow-empty",
                "-m",
                "new candidate",
            ],
        )
        .unwrap();
        assert_ne!(
            crate::chat::verifier_diff::git_head_commit(candidate.root())
                .await
                .unwrap(),
            before
        );
        let board = storage::load_board(gcx.clone(), "task-1").await.unwrap();
        let configuration = ConfigurationSnapshot::capture(&gcx).await.unwrap();
        assert!(!expected
            .verification
            .as_ref()
            .unwrap()
            .matches(&gcx, &board, &configuration));
        storage::update_board_atomic(gcx.clone(), "task-1", |board| {
            let card = board.get_card_mut("T-1").unwrap();
            card.verification.push(json!({"changed":"spec"}));
            card.verifier_report = Some(verifier_report(true));
            Ok(())
        })
        .await
        .unwrap();
        release.send(()).unwrap();
        assert!(!pending.await);

        let stored = store_verifier_report(
            gcx.clone(),
            "task-1",
            "T-1",
            Some(&expected),
            verifier_report(false),
            Vec::new(),
        )
        .await
        .unwrap();

        assert!(!stored);
        let board = storage::load_board(gcx.clone(), "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert!(card.verifier_report.as_ref().unwrap().passed);
    }

    #[test]
    fn blocked_and_unavailable_gates_classify_as_human_review_not_verification_failure() {
        for (verdict, classification) in [
            (
                EvidenceVerdict::Blocked,
                VerifierReportClassification::HumanReview,
            ),
            (
                EvidenceVerdict::Unavailable,
                VerifierReportClassification::Unavailable,
            ),
            (
                EvidenceVerdict::NotRun,
                VerifierReportClassification::Unavailable,
            ),
            (
                EvidenceVerdict::Superseded,
                VerifierReportClassification::Unavailable,
            ),
        ] {
            let gates = gate_outcome_for_verdicts(&[(true, verdict)]);
            assert_eq!(
                classify_verifier_report(&gates, false, false),
                classification,
                "{verdict:?} means evidence was not obtained, not that the work failed"
            );
        }
    }

    #[test]
    fn a_genuine_gate_failure_still_classifies_as_verification_failure() {
        for verdict in [EvidenceVerdict::Failed] {
            let gates = gate_outcome_for_verdicts(&[(true, verdict)]);
            assert_eq!(
                classify_verifier_report(&gates, false, false),
                VerifierReportClassification::VerificationFailed,
                "{verdict:?} must fail verification"
            );
        }

        let mixed = gate_outcome_for_verdicts(&[
            (true, EvidenceVerdict::Passed),
            (true, EvidenceVerdict::Failed),
        ]);
        assert_eq!(
            classify_verifier_report(&mixed, false, false),
            VerifierReportClassification::VerificationFailed
        );
    }

    #[test]
    fn a_passed_and_blocked_mix_is_human_review() {
        let gates = gate_outcome_for_verdicts(&[
            (true, EvidenceVerdict::Passed),
            (true, EvidenceVerdict::Blocked),
        ]);

        assert_eq!(
            classify_verifier_report(&gates, false, false),
            VerifierReportClassification::HumanReview
        );
        assert_eq!(gates.required_passed, 1);
        assert_eq!(gates.required_total, 2);
    }

    #[test]
    fn a_failing_gate_outranks_a_model_that_reports_success() {
        let gates = gate_outcome_for_verdicts(&[(true, EvidenceVerdict::Failed)]);

        assert_eq!(
            classify_verifier_report(&gates, false, false),
            VerifierReportClassification::VerificationFailed
        );
        assert!(!gates.judge_allowed(), "the judge must not even be asked");
        assert!(!completion_decision(&gates, true).allowed);
    }

    #[tokio::test]
    #[serial_test::serial(runtime_settings)]
    async fn agent_finish_spawns_verifier_through_helper() {
        let temp = tempfile::tempdir().unwrap();
        init_repo(temp.path());
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut card = card("");
        card.id = "T-1".into();
        write_task(temp.path(), gcx.clone(), card).await;
        register_candidate(gcx.clone(), "task-1", "T-1", "model").await;
        let app = AppState::from_gcx(gcx.clone()).await;
        let child = app
            .chat
            .sessions
            .read()
            .await
            .get("worker-chat")
            .cloned()
            .unwrap();
        child.lock().await.abort_flag.store(true, Ordering::Relaxed);
        let worktree = child.lock().await.thread.worktree.clone().unwrap();
        let fence = capture_finish_verification(
            gcx.clone(),
            "task-1",
            "T-1",
            "planner",
            "worker-chat",
            &worktree,
        )
        .await
        .unwrap();
        let board = storage::load_board(gcx.clone(), "task-1").await.unwrap();
        assert!(fence.is_current(&gcx, &board));
        let expected_state = ExpectedCardState::from_finished_card(
            &gcx,
            board.rev,
            board.get_card("T-1").unwrap(),
            board.owner_chat_id.as_deref(),
            Some(fence),
        );
        assert!(expected_state.verification.is_some());
        let request = VerifyCardRequest {
            task_id: "task-1".into(),
            card_id: "T-1".into(),
            expected_state: Some(expected_state.clone()),
        };
        let mut actor = RuntimeVerifierActor::after_finish(&app, &request)
            .await
            .unwrap();
        actor.wait_completed(&app).await.unwrap();
        let candidate = command_checkout::resolve_finished_verifier(&app, &actor)
            .await
            .unwrap();
        assert_eq!(candidate.metadata(), &actor.worktree);
        let board_path = temp.path().join(".refact/tasks/task-1/board.yaml");
        let raced = command_checkout::BEFORE_BOARD_RECHECK
            .scope(
                std::cell::RefCell::new(Some(Box::new(move || {
                    let mut board: TaskBoard =
                        serde_yaml::from_slice(&fs::read(&board_path).unwrap()).unwrap();
                    board.rev += 1;
                    let mut other = board.cards[0].clone();
                    other.id = "unrelated".into();
                    board.cards.push(other);
                    fs::write(board_path, serde_yaml::to_string(&board).unwrap()).unwrap();
                }) as Box<dyn FnOnce() + Send>)),
                command_checkout::resolve_finished_verifier(&app, &actor),
            )
            .await
            .unwrap();
        assert!(candidate.same_candidate(&raced));
        schedule_card_verifier_after_finish(
            gcx.clone(),
            "task-1".into(),
            "T-1".into(),
            expected_state.clone(),
        )
        .await;
        let mut tasks = gcx.card_verifier_tasks.lock().await;
        assert_eq!(tasks.len(), 1);
        tokio::time::timeout(Duration::from_secs(10), tasks.join_next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        drop(tasks);
        assert!(gcx.card_verifier_tasks.lock().await.is_empty());
        let board = storage::load_board(gcx.clone(), "task-1").await.unwrap();
        let report = board
            .get_card("T-1")
            .unwrap()
            .verifier_report
            .as_ref()
            .unwrap();
        assert_eq!(
            report.classification,
            VerifierReportClassification::Unavailable
        );
        assert_eq!(
            report.concerns,
            vec!["not run: No verification commands found in planner instructions"]
        );
        assert!(report.command_results.is_empty());

        let mut expected = expected_state;
        expected.card_fingerprint = verifier_card_fingerprint(board.get_card("T-1").unwrap());
        let configuration = ConfigurationSnapshot::capture(&gcx).await.unwrap();
        let mut changed_caps = gcx.caps_state.write().await;
        let mut caps = changed_caps.caps.as_deref().unwrap().clone();
        caps.defaults.chat_light_model = "changed-model".into();
        changed_caps.caps = Some(Arc::new(caps));
        gcx.tool_catalog_generations.advance_capabilities();
        drop(changed_caps);
        assert_eq!(
            configuration.ensure_current(&gcx).unwrap_err(),
            CONFIGURATION_RETRY
        );
        let changed = ConfigurationSnapshot::capture(&gcx).await.unwrap();
        assert!(!expected
            .verification
            .as_ref()
            .unwrap()
            .matches(&gcx, &board, &changed));
        assert!(!store_verifier_report(
            gcx.clone(),
            "task-1",
            "T-1",
            Some(&expected),
            verifier_report(true),
            Vec::new(),
        )
        .await
        .unwrap());
        let mut state = gcx.caps_state.write().await;
        let mut caps = state.caps.as_deref().unwrap().clone();
        caps.defaults.chat_light_model.clear();
        state.caps = Some(Arc::new(caps));
        drop(state);
        gcx.tool_catalog_generations.advance_capabilities();
        app.chat
            .sessions
            .write()
            .await
            .remove("worker-chat")
            .unwrap();
        schedule_card_verifier_after_finish(gcx.clone(), "task-1".into(), "T-1".into(), expected)
            .await;
        let mut tasks = gcx.card_verifier_tasks.lock().await;
        tokio::time::timeout(Duration::from_secs(10), tasks.join_next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        drop(tasks);
        let board = storage::load_board(gcx.clone(), "task-1").await.unwrap();
        let report = board
            .get_card("T-1")
            .unwrap()
            .verifier_report
            .as_ref()
            .unwrap();
        assert_eq!(
            report.classification,
            VerifierReportClassification::Unavailable
        );
        assert_eq!(report.concerns, vec!["Verification infrastructure unavailable; human review required: finished session unavailable"]);
        assert!(report.command_results.is_empty());
        assert!(!report.passed);
    }

    #[tokio::test]
    async fn verifier_is_not_scheduled_after_shutdown_starts() {
        let mut card = card("");
        card.id = "T-stopped".to_string();
        let expected_state = ExpectedCardState::from_card(0, &card);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        gcx.shutdown_flag.store(true, Ordering::SeqCst);

        schedule_card_verifier_after_finish(
            gcx.clone(),
            "missing-task".to_string(),
            "T-stopped".to_string(),
            expected_state,
        )
        .await;

        assert!(gcx.card_verifier_tasks.lock().await.is_empty());
    }

    #[tokio::test]
    async fn cwd_outside_worktree_is_rejected() {
        let worktree = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;

        let result = run_verification_argv_impl(
            gcx,
            worktree.path(),
            "cargo check",
            Some(outside.path().to_path_buf()),
            Vec::new(),
            vec!["cargo".to_string(), "check".to_string()],
            Duration::from_secs(30),
            DRAIN_TIMEOUT,
        )
        .await;

        assert!(!result.passed);
        assert!(result.output_tail.contains("outside the worktree"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stdin_null_does_not_hang() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;

        let result = tokio::time::timeout(
            Duration::from_secs(5),
            run_verification_argv_impl(
                gcx,
                temp.path(),
                "cat",
                None,
                Vec::new(),
                vec!["cat".to_string()],
                Duration::from_secs(30),
                DRAIN_TIMEOUT,
            ),
        )
        .await
        .expect("should not hang with stdin=null");

        assert!(result.passed);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_kills_process_group() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;

        let result = run_verification_argv_impl(
            gcx,
            temp.path(),
            "sleep 30",
            None,
            Vec::new(),
            vec!["sleep".to_string(), "30".to_string()],
            Duration::from_millis(200),
            DRAIN_TIMEOUT,
        )
        .await;

        assert!(!result.passed);
        assert!(result.output_tail.contains("timed out"));
        assert!(result.exit_code.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn drain_times_out_when_descendant_holds_pipe() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;

        // bash spawns a background sleep that inherits the stdout/stderr pipes and then exits;
        // without the drain timeout the verifier hangs waiting for EOF from the background sleep
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            run_verification_argv_impl(
                gcx,
                temp.path(),
                "bash -c 'sleep 60 &'",
                None,
                Vec::new(),
                vec![
                    "bash".to_string(),
                    "-c".to_string(),
                    "sleep 60 &".to_string(),
                ],
                Duration::from_secs(30),
                Duration::from_millis(500),
            ),
        )
        .await
        .expect("verifier must not hang when descendant holds pipe open");

        assert!(result.output_tail.contains("drain timed out"));
        assert!(!result.passed);
    }
}
