use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::chat::types::TaskMeta;
use crate::tasks::types::{BoardCard, TaskBoard};
use crate::worktrees::scope::ExecutionScope;
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::{WorktreeMeta, WorktreeReference};

const REFUSED: &str =
    "command checkout is unavailable, unauthorized or changed; request a fresh checkout target";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskCardCheckout {
    kind: CheckoutKind,
    pub(crate) card_id: String,
    pub(crate) expected_board_rev: u64,
    pub(crate) expected_agent_chat_id: String,
    pub(crate) expected_worktree_id: String,
    pub(crate) expected_head: String,
    pub(crate) expected_cwd: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CheckoutKind {
    TaskCard,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ValidatedCommandCheckout {
    task_id: String,
    owner_chat_id: String,
    selector: TaskCardCheckout,
    meta: WorktreeMeta,
    references: Vec<WorktreeReference>,
    root: PathBuf,
    cwd: PathBuf,
    board_fingerprint: String,
    planner_identity: Value,
    #[serde(skip)]
    records: Vec<refact_privacy::FileRecord>,
}

impl ValidatedCommandCheckout {
    fn binding(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|_| REFUSED.into())
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub(crate) fn bind_approval(
        &self,
        approval: &mut refact_work_core::ExecutionApproval,
        request: &crate::exec::ExecSpawnRequest,
    ) -> Result<(), String> {
        if request.cwd.as_deref() != Some(self.cwd()) {
            return Err(REFUSED.into());
        }
        let binding = serde_json::to_string(&json!({
            "execution": approval.fingerprint,
            "checkout": self,
            "mode": request.mode.to_string(),
            "tty": request.tty,
            "rows": request.rows,
            "cols": request.cols,
            "timeout": request.timeout,
            "startup_wait": request.startup_wait,
            "readiness": request.readiness,
            "output_limits": request.output_limits,
        }))
        .map_err(|_| REFUSED)?;
        approval.fingerprint = crate::file_activity::digest_of(&binding);
        Ok(())
    }

    pub(crate) async fn revalidate(
        &self,
        ccx: &Arc<Mutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
        request: &crate::exec::ExecSpawnRequest,
    ) -> Result<(), String> {
        let current = resolve(ccx, args).await?.ok_or(REFUSED)?;
        if current.binding()? != self.binding()? || request.cwd.as_deref() != Some(self.cwd()) {
            return Err(REFUSED.into());
        }
        ensure_not_cancelled(ccx).await
    }
}

pub(crate) fn input_schema() -> Value {
    json!({
        "type":"object", "additionalProperties":false,
        "description":"Explicit current planner-owned completed-card checkout. Obtain the root target receipt from work_inspect selector=worktree; stale receipts require a fresh request. Registration or a known path alone grants no execution authority. Workdir defaults to this checkout and must remain inside it; normal command consent, privacy and sandbox rules still apply.",
        "required":["kind","card_id","expected_board_rev","expected_agent_chat_id","expected_worktree_id","expected_head","expected_cwd"],
        "properties":{
            "kind":{"type":"string","enum":["task_card"]},
            "card_id":{"type":"string","minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9_-]+$"},
            "expected_board_rev":{"type":"integer","minimum":0},
            "expected_agent_chat_id":{"type":"string","minLength":1,"maxLength":128,"pattern":"^[A-Za-z0-9_-]+$"},
            "expected_worktree_id":{"type":"string","minLength":1,"maxLength":128,"pattern":"^[A-Za-z0-9_-]+$"},
            "expected_head":{"type":"string","minLength":40,"maxLength":40,"pattern":"^[0-9a-f]{40}$"},
            "expected_cwd":{"type":"string","minLength":1,"maxLength":4096,"description":"Expected canonical absolute cwd, an immutable narrowing expectation, never a path grant. Must equal the server-resolved workdir."}
        }
    })
}

fn parse(args: &HashMap<String, Value>) -> Result<Option<TaskCardCheckout>, String> {
    let Some(value) = args.get("checkout") else {
        return Ok(None);
    };
    let selector: TaskCardCheckout = serde_json::from_value(value.clone())
        .map_err(|_| "invalid bounded command checkout selector")?;
    selector.validate()?;
    Ok(Some(selector))
}

impl TaskCardCheckout {
    fn validate(&self) -> Result<(), String> {
        let id = |value: &str, cap| {
            !value.is_empty()
                && value.len() <= cap
                && value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        };
        if !id(&self.card_id, 64)
            || !id(&self.expected_agent_chat_id, 128)
            || !id(&self.expected_worktree_id, 128)
            || self.expected_cwd.len() > 4096
            || !Path::new(&self.expected_cwd).is_absolute()
            || self.expected_cwd.chars().any(char::is_control)
            || self.expected_head.len() != 40
            || !self
                .expected_head
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err("invalid bounded command checkout selector".into());
        }
        Ok(())
    }
}

async fn ensure_not_cancelled(ccx: &Arc<Mutex<AtCommandsContext>>) -> Result<(), String> {
    let context = ccx.lock().await;
    if context.abort_flag.load(Ordering::Relaxed)
        || context.app.runtime.shutdown_flag.load(Ordering::Relaxed)
        || context
            .app
            .agent_executions
            .get(&context.chat_id)
            .is_some_and(|run| run.is_cancelled())
    {
        return Err(REFUSED.into());
    }
    Ok(())
}

pub(crate) async fn resolve(
    ccx: &Arc<Mutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
) -> Result<Option<ValidatedCommandCheckout>, String> {
    let Some(selector) = parse(args)? else {
        return Ok(None);
    };
    let raw_cwd = match args.get("workdir") {
        Some(Value::String(value)) => Some(value.as_str()),
        None => None,
        _ => return Err(REFUSED.into()),
    };
    ensure_not_cancelled(ccx).await?;
    let (app, chat_id, model, meta, scope) = {
        let context = ccx.lock().await;
        if context.background_agent_id.is_some() || context.subchat_depth != 0 {
            return Err(REFUSED.into());
        }
        (
            context.app.clone(),
            context.chat_id.clone(),
            context.current_model.clone(),
            context.task_meta.clone().ok_or(REFUSED)?,
            context.execution_scope.clone(),
        )
    };
    if meta.role != "planner" || meta.agent_id.is_some() || meta.card_id.is_some() {
        return Err(REFUSED.into());
    }
    let candidate = resolve_task_card(
        &app,
        &chat_id,
        &model,
        &meta,
        scope.as_ref(),
        &selector,
        raw_cwd,
    )
    .await?;
    ensure_not_cancelled(ccx).await?;
    Ok(Some(candidate))
}

pub(crate) async fn resolve_task_card(
    app: &AppState,
    chat_id: &str,
    model: &str,
    expected_meta: &TaskMeta,
    scope: Option<&ExecutionScope>,
    selector: &TaskCardCheckout,
    raw_cwd: Option<&str>,
) -> Result<ValidatedCommandCheckout, String> {
    selector.validate()?;
    let planner = planner_identity(app, chat_id, expected_meta).await?;
    let caller = crate::coordination::scope::model_caller(app, chat_id, model)
        .await
        .map_err(|_| REFUSED)?;
    let model_record = caller.model.as_ref().ok_or(REFUSED)?;
    let policy_key = crate::coordination::privacy_guard::policy_key(&app.gcx, model_record)
        .map_err(|_| REFUSED)?;
    let authority = TaskCardAuthority {
        task: expected_meta.clone(),
        owner: chat_id.to_string(),
        source: caller.source_root.clone(),
        verifier: None,
    };
    let mut candidate = resolve_registered_task_card(app, &authority, selector, raw_cwd).await?;
    let live_worktree: Option<WorktreeMeta> =
        serde_json::from_value(planner["worktree"].clone()).map_err(|_| REFUSED)?;
    let live_scope = live_worktree.as_ref().map(ExecutionScope::from_worktree);
    for scope in [scope, live_scope.as_ref()].into_iter().flatten() {
        scope.ensure_active_root().map_err(|_| REFUSED)?;
        if scope.is_enforced()
            && (scope.worktree() != &candidate.meta
                || canonical_dir(scope.effective_root()).await? != candidate.root)
        {
            return Err(REFUSED.into());
        }
    }
    let privacy = crate::privacy::load_privacy_if_needed(app.gcx.clone()).await;
    crate::privacy::check_file_privacy(
        privacy,
        &candidate.cwd,
        &crate::privacy::FilePrivacyLevel::AllowToSendAnywhere,
    )
    .map_err(|_| REFUSED)?;
    candidate.records = crate::privacy::records::declared_file_records(
        &app.gcx,
        [
            candidate.root.clone(),
            candidate.cwd.clone(),
            raw_cwd
                .map(PathBuf::from)
                .map(|path| {
                    if path.is_absolute() {
                        path
                    } else {
                        candidate.root.join(path)
                    }
                })
                .unwrap_or_else(|| candidate.root.clone()),
            authority
                .source
                .join(".refact/tasks")
                .join(&expected_meta.task_id)
                .join("board.yaml"),
        ],
    )
    .map_err(|_| REFUSED)?;
    crate::privacy::destinations::clear_for_model(
        &app.gcx,
        CheckoutAudit(candidate.records.clone()),
        model_record,
    )
    .map_err(|_| REFUSED)?;
    if planner_identity(app, chat_id, expected_meta).await? != planner
        || crate::coordination::privacy_guard::policy_key(&app.gcx, model_record)
            .map_err(|_| REFUSED)?
            != policy_key
    {
        return Err(REFUSED.into());
    }
    let current_caller = crate::coordination::scope::model_caller(app, chat_id, model)
        .await
        .map_err(|_| REFUSED)?;
    if current_caller.source_root != authority.source
        || current_caller.root_chat_id != caller.root_chat_id
        || canonical_dir(&current_caller.checkout_root).await?
            != canonical_dir(&caller.checkout_root).await?
    {
        return Err(REFUSED.into());
    }
    if crate::coordination::privacy_guard::policy_key(&app.gcx, model_record)
        .map_err(|_| REFUSED)?
        != policy_key
    {
        return Err(REFUSED.into());
    }
    let current = resolve_registered_task_card(app, &authority, selector, raw_cwd).await?;
    if current.binding()? != candidate.binding()?
        || planner_identity(app, chat_id, expected_meta).await? != planner
        || crate::coordination::privacy_guard::policy_key(&app.gcx, model_record)
            .map_err(|_| REFUSED)?
            != policy_key
    {
        return Err(REFUSED.into());
    }
    candidate.planner_identity = planner;
    Ok(candidate)
}

pub(crate) struct TaskCardAuthority {
    task: TaskMeta,
    owner: String,
    source: PathBuf,
    verifier: Option<crate::chat::verifier::ExpectedCardState>,
}

async fn card_selector(
    app: &AppState,
    source: &Path,
    task_id: &str,
    card_id: &str,
) -> Result<TaskCardCheckout, String> {
    let (board, _) = read_board(source, task_id).await?;
    let card = board.get_card(card_id).ok_or(REFUSED)?;
    let id = card.agent_worktree_name.as_deref().ok_or(REFUSED)?;
    let service = WorktreeService::new_async(app.gcx.cache_dir.clone(), source.to_path_buf())
        .await
        .map_err(|_| REFUSED)?;
    let view = service.get_worktree(id).await.map_err(|_| REFUSED)?;
    Ok(TaskCardCheckout {
        kind: CheckoutKind::TaskCard,
        card_id: card_id.to_string(),
        expected_board_rev: board.rev,
        expected_agent_chat_id: card.agent_chat_id.clone().ok_or(REFUSED)?,
        expected_worktree_id: id.to_string(),
        expected_head: view.status.head_commit.ok_or(REFUSED)?,
        expected_cwd: canonical_dir(&view.meta.root)
            .await?
            .to_string_lossy()
            .into_owned(),
    })
}

pub(crate) async fn resolve_planner_verifier(
    ccx: &Arc<Mutex<AtCommandsContext>>,
    task_id: &str,
    card_id: &str,
) -> Result<ValidatedCommandCheckout, String> {
    let (app, chat, model, task) = {
        let context = ccx.lock().await;
        (
            context.app.clone(),
            context.chat_id.clone(),
            context.current_model.clone(),
            context.task_meta.clone().ok_or(REFUSED)?,
        )
    };
    if task.task_id != task_id {
        return Err(REFUSED.into());
    }
    planner_identity(&app, &chat, &task).await?;
    let caller = crate::coordination::scope::model_caller(&app, &chat, &model)
        .await
        .map_err(|_| REFUSED)?;
    let selector = card_selector(&app, &caller.source_root, task_id, card_id).await?;
    resolve(ccx, &HashMap::from([("checkout".into(), json!(selector))]))
        .await?
        .ok_or_else(|| REFUSED.into())
}

pub(crate) async fn resolve_finished_verifier(
    app: &AppState,
    actor: &crate::chat::verifier::RuntimeVerifierActor,
) -> Result<ValidatedCommandCheckout, String> {
    actor.ensure_current(app, false).await?;
    let authority = TaskCardAuthority {
        task: TaskMeta {
            task_id: actor.task_id().into(),
            role: "planner".into(),
            ..Default::default()
        },
        owner: actor.owner().into(),
        source: actor.source().to_path_buf(),
        verifier: Some(actor.expected().clone()),
    };
    let selector = card_selector(app, &authority.source, actor.task_id(), actor.card_id()).await?;
    let candidate = resolve_registered_task_card(app, &authority, &selector, None).await?;
    actor.ensure_current(app, false).await?;
    Ok(candidate)
}

impl ValidatedCommandCheckout {
    pub(crate) fn references(&self) -> &[WorktreeReference] {
        &self.references
    }

    pub(crate) fn head(&self) -> &str {
        &self.selector.expected_head
    }

    pub(crate) fn metadata(&self) -> &WorktreeMeta {
        &self.meta
    }

    pub(crate) fn same_candidate(&self, other: &Self) -> bool {
        self.task_id == other.task_id
            && self.owner_chat_id == other.owner_chat_id
            && self.selector.expected_head == other.selector.expected_head
            && self.selector.expected_agent_chat_id == other.selector.expected_agent_chat_id
            && self.meta == other.meta
            && self.references == other.references
            && self.root == other.root
            && self.cwd == other.cwd
            && self.planner_identity == other.planner_identity
    }
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static BEFORE_BOARD_RECHECK: std::cell::RefCell<Option<Box<dyn FnOnce() + Send>>>;
}

pub(crate) async fn resolve_registered_task_card(
    app: &AppState,
    authority: &TaskCardAuthority,
    selector: &TaskCardCheckout,
    raw_cwd: Option<&str>,
) -> Result<ValidatedCommandCheckout, String> {
    selector.validate()?;
    let expected_meta = &authority.task;
    let chat_id = authority.owner.as_str();
    let source = authority.source.clone();
    let (board, board_fingerprint) = read_board(&source, &expected_meta.task_id).await?;
    board.require_owner(chat_id).map_err(|error| {
        format!(
            "{error}; owning planner chat: {}. Open that planner chat to operate on this checkout",
            board.owner_chat_id.as_deref().unwrap_or("unclaimed")
        )
    })?;
    let card = board.get_card(&selector.card_id).ok_or(REFUSED)?;
    if (authority.verifier.is_none() && board.rev != selector.expected_board_rev)
        || card.column != "done"
        || card.paused_at.is_some()
        || (card.ab_variants.is_some()
            && (authority.verifier.is_none() || !selected_winner_matches(card)))
        || !card.team_members.is_empty()
        || card.agent_chat_id.as_deref() != Some(selector.expected_agent_chat_id.as_str())
        || card.agent_worktree_name.as_deref() != Some(selector.expected_worktree_id.as_str())
    {
        return Err(REFUSED.into());
    }
    if authority
        .verifier
        .as_ref()
        .is_some_and(|expected| !expected.matches_board_card(board.rev, card))
    {
        return Err(REFUSED.into());
    }
    let service = WorktreeService::new_async(app.gcx.cache_dir.clone(), source.clone())
        .await
        .map_err(|_| REFUSED)?;
    let view = service
        .get_worktree(&selector.expected_worktree_id)
        .await
        .map_err(|_| REFUSED)?;
    if view.status.head_commit.as_deref() != Some(selector.expected_head.as_str()) {
        return Err(format!("Checkout {} at {} changed: expected HEAD {}, actual {}; request a fresh checkout target", selector.expected_worktree_id, view.meta.root.display(), selector.expected_head, view.status.head_commit.as_deref().unwrap_or("unavailable")));
    }
    let meta = &view.meta;
    let root = canonical_dir(&meta.root).await?;
    if root != meta.root {
        return Err(REFUSED.into());
    }
    if meta.kind != "task_agent"
        || !meta.enforce
        || meta.task_id.as_deref() != Some(expected_meta.task_id.as_str())
        || meta.card_id.as_deref() != Some(card.id.as_str())
        || meta.agent_id.as_deref().is_none_or(str::is_empty)
        || meta.agent_id != card.assignee
        || canonical_dir(&meta.source_workspace_root).await? != source
        || card.agent_worktree.as_deref().map(Path::new) != Some(meta.root.as_path())
        || meta.branch.is_none()
        || view.status.conflicted
        || meta.branch != card.agent_branch
        || !view.status.path_exists
        || !view.status.is_git_worktree
        || view.status.error.is_some()
        || view.status.branch != meta.branch
        || view.status.head_commit.as_deref() != Some(selector.expected_head.as_str())
        || !view.references.iter().any(|reference| {
            reference.kind == "task_agent"
                && reference.task_id == meta.task_id
                && reference.card_id == meta.card_id
                && reference.agent_id == meta.agent_id
                && reference.chat_id.as_deref() == Some(selector.expected_agent_chat_id.as_str())
        })
    {
        return Err(REFUSED.into());
    }
    let cwd = contained_cwd(&root, raw_cwd).await?;
    if cwd.to_str() != Some(selector.expected_cwd.as_str()) {
        return Err(REFUSED.into());
    }
    ensure_agent_stopped(app, authority, card, meta).await?;
    let current = service
        .get_worktree(&selector.expected_worktree_id)
        .await
        .map_err(|_| REFUSED)?;
    if current.meta != view.meta
        || current.references != view.references
        || current.status.head_commit != view.status.head_commit
        || current.status.branch != view.status.branch
        || contained_cwd(&root, raw_cwd).await? != cwd
    {
        return Err(REFUSED.into());
    }
    #[cfg(test)]
    let _ = BEFORE_BOARD_RECHECK.try_with(|hook| {
        if let Some(change) = hook.borrow_mut().take() {
            change();
        }
    });
    let (current_board, current_fingerprint) = read_board(&source, &expected_meta.task_id).await?;
    current_board.require_owner(chat_id).map_err(|error| {
        format!(
            "{error}; owning planner chat: {}. Open that planner chat to operate on this checkout",
            current_board
                .owner_chat_id
                .as_deref()
                .unwrap_or("unclaimed")
        )
    })?;
    let unchanged = if let Some(expected) = authority.verifier.as_ref() {
        current_board
            .get_card(&selector.card_id)
            .is_some_and(|current| {
                expected.matches_board_card(current_board.rev, current)
                    && board.review_policy == current_board.review_policy
            })
    } else {
        current_fingerprint == board_fingerprint
    };
    if !unchanged {
        return Err(format!(
            "{REFUSED}; card {} checkout {} expected head {} and cwd {}",
            selector.card_id,
            selector.expected_worktree_id,
            selector.expected_head,
            selector.expected_cwd
        ));
    }
    Ok(ValidatedCommandCheckout {
        task_id: expected_meta.task_id.clone(),
        owner_chat_id: chat_id.to_string(),
        selector: selector.clone(),
        meta: view.meta,
        references: view.references,
        root,
        cwd,
        board_fingerprint,
        planner_identity: Value::Null,
        records: Vec::new(),
    })
}

pub(crate) async fn planner_identity(
    app: &AppState,
    chat_id: &str,
    expected: &TaskMeta,
) -> Result<Value, String> {
    if expected.role != "planner"
        || expected.agent_id.is_some()
        || expected.card_id.is_some()
        || app
            .agents
            .find_agent_id_by_child_chat_id(chat_id)
            .await
            .is_some()
    {
        return Err(REFUSED.into());
    }
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(chat_id)
        .cloned()
        .ok_or(REFUSED)?;
    let session = session.lock().await;
    if session.thread.task_meta.as_ref() != Some(expected)
        || session.thread.mode != "task_planner"
        || session.runtime.state == refact_runtime_api::SessionState::Completed
        || session.runtime.state == refact_runtime_api::SessionState::Error
    {
        return Err(REFUSED.into());
    }
    Ok(
        json!({"meta":session.thread.task_meta,"worktree":session.thread.worktree,
        "source":session.work_project_root,"root":session.thread.root_chat_id,
        "parent":session.thread.parent_id,"model":session.thread.model}),
    )
}

pub(crate) async fn ensure_card_agent_stopped(
    app: &AppState,
    task: &TaskMeta,
    owner: &str,
    source: &Path,
    card: &BoardCard,
    worktree: &WorktreeMeta,
) -> Result<(), String> {
    let authority = TaskCardAuthority {
        task: task.clone(),
        owner: owner.to_string(),
        source: source.to_path_buf(),
        verifier: None,
    };
    ensure_agent_stopped(app, &authority, card, worktree).await
}

async fn ensure_agent_stopped(
    app: &AppState,
    authority: &TaskCardAuthority,
    card: &BoardCard,
    worktree: &WorktreeMeta,
) -> Result<(), String> {
    let task = &authority.task;
    let owner = authority.owner.as_str();
    let chat_id = card.agent_chat_id.as_deref().ok_or(REFUSED)?;
    if authority.verifier.is_none()
        && (crate::tasks::admission::is_tracked(chat_id).await
            || app
                .agent_executions
                .get(chat_id)
                .is_some_and(|run| run.state() != crate::agents::execution::ExecutionState::Gone))
    {
        return Err(REFUSED.into());
    }
    let live = app.chat.sessions.read().await.get(chat_id).cloned();
    let thread = if let Some(session) = live {
        let session = session.lock().await;
        if session.runtime.state != refact_runtime_api::SessionState::Completed
            || !session.command_queue.is_empty()
        {
            return Err(REFUSED.into());
        }
        session.thread.clone()
    } else {
        crate::chat::trajectories::load_trajectory_for_chat(app.gcx.clone(), chat_id)
            .await
            .ok_or(REFUSED)?
            .thread
    };
    let agent = thread.task_meta.as_ref().ok_or(REFUSED)?;
    if agent.role != "agents"
        || agent.task_id != task.task_id
        || agent.card_id.as_deref() != Some(card.id.as_str())
        || agent.agent_id != card.assignee
        || agent.planner_chat_id.as_deref() != Some(owner)
        || thread.parent_id.as_deref() != Some(owner)
        || thread.root_chat_id.as_deref() != Some(owner)
        || thread.link_type.as_deref() != Some("task_agent")
        || thread.worktree.as_ref() != Some(worktree)
    {
        return Err(REFUSED.into());
    }
    Ok(())
}

pub(crate) fn selected_winner_matches(card: &BoardCard) -> bool {
    let Some(variants) = &card.ab_variants else {
        return true;
    };
    let Some(winner) = variants
        .winner
        .as_deref()
        .and_then(|key| variants.variant(key))
    else {
        return false;
    };
    winner.finish.as_ref().is_some_and(|finish| finish.success)
        && card.assignee.as_deref() == Some(winner.agent_id.as_str())
        && card.agent_chat_id.as_deref() == Some(winner.chat_id.as_str())
        && card.agent_worktree.as_deref() == Some(winner.worktree.as_str())
        && card.agent_worktree_name == winner.worktree_name
        && card.agent_branch == winner.branch
}

pub(crate) async fn read_board(source: &Path, task: &str) -> Result<(TaskBoard, String), String> {
    crate::tasks::storage::validate_task_id(task).map_err(|_| REFUSED)?;
    let dir = source.join(".refact/tasks").join(task);
    for path in [
        source.join(".refact"),
        source.join(".refact/tasks"),
        dir.clone(),
        dir.join("board.yaml"),
    ] {
        if tokio::fs::symlink_metadata(path)
            .await
            .map_err(|_| REFUSED)?
            .file_type()
            .is_symlink()
        {
            return Err(REFUSED.into());
        }
    }
    let body = crate::coordination::board_bootstrap::read_bounded(
        &dir.join("board.yaml"),
        source,
        crate::file_activity::settings().max_work_history_bytes,
    )
    .await
    .map_err(|_| REFUSED)?;
    let board = serde_yaml::from_str(&body).map_err(|_| REFUSED)?;
    Ok((board, crate::file_activity::digest_of(&body)))
}

async fn canonical_dir(path: &Path) -> Result<PathBuf, String> {
    let path = tokio::fs::canonicalize(path).await.map_err(|_| REFUSED)?;
    if !tokio::fs::metadata(&path)
        .await
        .map_err(|_| REFUSED)?
        .is_dir()
    {
        return Err(REFUSED.into());
    }
    Ok(dunce::simplified(&path).to_path_buf())
}

async fn contained_cwd(root: &Path, raw: Option<&str>) -> Result<PathBuf, String> {
    let raw = raw.filter(|raw| !raw.is_empty()).unwrap_or(".");
    if raw.len() > 4096 || raw.chars().any(char::is_control) {
        return Err(REFUSED.into());
    }
    let path = Path::new(raw);
    if path
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(REFUSED.into());
    }
    let requested = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    if !requested.starts_with(root) {
        return Err(REFUSED.into());
    }
    let cwd = canonical_dir(&requested).await?;
    if !cwd.starts_with(root) {
        return Err(REFUSED.into());
    }
    Ok(cwd)
}

struct CheckoutAudit(Vec<refact_privacy::FileRecord>);

impl refact_privacy::PrivacyAudited for CheckoutAudit {
    fn privacy_records(
        &self,
    ) -> Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError> {
        Ok(self.0.iter().cloned().enumerate().collect())
    }
}

pub(crate) async fn inspection_receipt(
    ccx: &Arc<Mutex<AtCommandsContext>>,
    worktree_id: &str,
) -> Option<(TaskCardCheckout, Vec<refact_privacy::FileRecord>)> {
    let result = async {
        let (app, chat_id, model, task) = {
            let context = ccx.lock().await;
            if context.background_agent_id.is_some() || context.subchat_depth != 0 {
                return Err(REFUSED);
            }
            (
                context.app.clone(),
                context.chat_id.clone(),
                context.current_model.clone(),
                context.task_meta.clone().ok_or(REFUSED)?,
            )
        };
        planner_identity(&app, &chat_id, &task)
            .await
            .map_err(|_| REFUSED)?;
        let caller = crate::coordination::scope::model_caller(&app, &chat_id, &model)
            .await
            .map_err(|_| REFUSED)?;
        let (board, _) = read_board(&caller.source_root, &task.task_id)
            .await
            .map_err(|_| REFUSED)?;
        board.require_owner(&chat_id).map_err(|_| REFUSED)?;
        let card = board
            .cards
            .iter()
            .find(|card| {
                card.column == "done" && card.agent_worktree_name.as_deref() == Some(worktree_id)
            })
            .ok_or(REFUSED)?;
        let service = WorktreeService::new_async(app.gcx.cache_dir.clone(), caller.source_root)
            .await
            .map_err(|_| REFUSED)?;
        let view = service
            .get_worktree(worktree_id)
            .await
            .map_err(|_| REFUSED)?;
        let selector = TaskCardCheckout {
            kind: CheckoutKind::TaskCard,
            card_id: card.id.clone(),
            expected_board_rev: board.rev,
            expected_agent_chat_id: card.agent_chat_id.clone().ok_or(REFUSED)?,
            expected_worktree_id: worktree_id.to_string(),
            expected_head: view.status.head_commit.ok_or(REFUSED)?,
            expected_cwd: canonical_dir(&view.meta.root)
                .await
                .map_err(|_| REFUSED)?
                .to_string_lossy()
                .into_owned(),
        };
        let args = HashMap::from([("checkout".into(), json!(selector))]);
        let candidate = resolve(ccx, &args)
            .await
            .map_err(|_| REFUSED)?
            .ok_or(REFUSED)?;
        Ok::<_, &str>((selector, candidate.records))
    };
    result.await.ok()
}
