use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::chat::types::{ChatSession, TaskMeta};
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::{WorktreeMeta, WorktreeReference};

const REFUSED: &str = "Source synchronization authority is unavailable, unauthorized or changed; request worktree_sync Status from the owning chat";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SyncSelector {
    Own,
    Card(String, Option<String>),
    Child(String),
}

#[derive(Clone)]
pub(crate) struct SyncCaller {
    app: AppState,
    chat_id: String,
    root_chat_id: String,
    model: String,
    task_meta: Option<TaskMeta>,
    scope_worktree: Option<WorktreeMeta>,
    nested: bool,
    abort_flag: Arc<AtomicBool>,
}

pub(crate) struct SyncAuthority {
    pub(crate) worktree: WorktreeMeta,
    pub(crate) actor: String,
    pub(crate) provenance: Value,
    pub(crate) records: Vec<refact_privacy::FileRecord>,
    digest: String,
}

impl SyncCaller {
    pub(crate) async fn from_context(ccx: &Arc<Mutex<AtCommandsContext>>) -> Self {
        let c = ccx.lock().await;
        Self {
            app: c.app.clone(),
            chat_id: c.chat_id.clone(),
            root_chat_id: c.root_chat_id.clone(),
            model: c.current_model.clone(),
            task_meta: c.task_meta.clone(),
            scope_worktree: c.execution_scope_worktree(),
            nested: c.background_agent_id.is_some() || c.subchat_depth != 0,
            abort_flag: c.abort_flag.clone(),
        }
    }

    pub(crate) fn app(&self) -> &AppState {
        &self.app
    }
}

async fn session(app: &AppState, chat_id: &str) -> Option<Arc<Mutex<ChatSession>>> {
    app.chat.sessions.read().await.get(chat_id).cloned()
}

fn execution_live(app: &AppState, chat_id: &str) -> bool {
    app.agent_executions
        .get(chat_id)
        .is_some_and(|run| run.state() != crate::agents::execution::ExecutionState::Gone)
}

pub(crate) async fn ensure_source_quiescent(
    app: &AppState,
    meta: &WorktreeMeta,
    references: &[WorktreeReference],
    actor: Option<&str>,
) -> Result<(), String> {
    let sessions: Vec<_> = app.chat.sessions.read().await.values().cloned().collect();
    let mut chats: std::collections::HashSet<String> = references
        .iter()
        .filter_map(|r| r.chat_id.clone())
        .collect();
    for agent in app.agents.list_all().await {
        if agent.worktree_id.as_deref() == Some(meta.id.as_str()) {
            if agent
                .child_chat_id
                .as_ref()
                .is_some_and(|chat| actor == Some(format!("chat:{chat}").as_str()))
            {
                continue;
            }
            if !agent.status.is_terminal() || !agent.followups.is_empty() {
                return Err(
                    "Source synchronization refused: source agent writer is active or queued"
                        .into(),
                );
            }
            if let Some(chat) = agent.child_chat_id {
                chats.insert(chat);
            }
        }
    }
    for session in sessions {
        let session = session.lock().await;
        if session.thread.worktree.as_ref().is_some_and(|w| {
            w.id == meta.id && w.source_workspace_root == meta.source_workspace_root
        }) {
            chats.insert(session.chat_id.clone());
        }
    }
    for chat in chats {
        if actor == Some(format!("chat:{chat}").as_str()) {
            continue;
        }
        if execution_live(app, &chat)
            || app.agent_executions.task_active(&chat)
            || crate::tasks::admission::is_tracked(&chat).await
        {
            return Err(
                "Source synchronization refused: source writer execution is active or queued"
                    .into(),
            );
        }
        if let Some(agent) = app.agents.find_agent_id_by_child_chat_id(&chat).await {
            let record = app.agents.get_any(&agent).await?;
            if !record.status.is_terminal() || !record.followups.is_empty() {
                return Err(
                    "Source synchronization refused: source agent is active or queued".into(),
                );
            }
        }
        if let Some(session) = session(app, &chat).await {
            let session = session.lock().await;
            if !session.command_queue.is_empty()
                || matches!(
                    session.runtime.state,
                    refact_runtime_api::SessionState::Generating
                        | refact_runtime_api::SessionState::ExecutingTools
                        | refact_runtime_api::SessionState::WaitingIde
                        | refact_runtime_api::SessionState::Paused
                )
            {
                return Err(
                    "Source synchronization refused: source chat writer is active or queued".into(),
                );
            }
        }
    }
    Ok(())
}

async fn live_caller(caller: &SyncCaller) -> Result<Value, String> {
    if caller.abort_flag.load(Ordering::Relaxed)
        || caller.app.runtime.shutdown_flag.load(Ordering::Relaxed)
        || caller
            .app
            .agent_executions
            .get(&caller.chat_id)
            .is_some_and(|run| run.is_cancelled())
    {
        return Err(REFUSED.into());
    }
    if let Some(reason) = crate::agents::seal::seal_reason(&caller.app, &caller.chat_id).await {
        return Err(reason.message());
    }
    let session = session(&caller.app, &caller.chat_id).await.ok_or(REFUSED)?;
    let session = session.lock().await;
    if session.closed
        || session.chat_id != caller.chat_id
        || !Arc::ptr_eq(&session.abort_flag, &caller.abort_flag)
        || session.runtime.state != refact_runtime_api::SessionState::ExecutingTools
        || session.thread.task_meta != caller.task_meta
        || session.thread.worktree != caller.scope_worktree
    {
        return Err(REFUSED.into());
    }
    Ok(
        json!({"root":session.thread.root_chat_id,"parent":session.thread.parent_id,
        "link":session.thread.link_type,"source":session.work_project_root,
        "worktree":session.thread.worktree,"task":session.thread.task_meta}),
    )
}

struct Registered {
    meta: WorktreeMeta,
    references: Vec<WorktreeReference>,
}

async fn registered(
    app: &AppState,
    meta: &WorktreeMeta,
    recovery: bool,
) -> Result<Registered, String> {
    let service = WorktreeService::new_async(
        app.gcx.cache_dir.clone(),
        meta.source_workspace_root.clone(),
    )
    .await
    .map_err(|_| REFUSED)?;
    registered_id(&service, &meta.id, Some(meta), recovery).await
}

async fn registered_id(
    service: &WorktreeService,
    id: &str,
    expected: Option<&WorktreeMeta>,
    recovery: bool,
) -> Result<Registered, String> {
    if recovery {
        let registry = service.load_registry().await.map_err(|_| REFUSED)?;
        let record = registry
            .records
            .into_iter()
            .find(|r| r.meta.id == id)
            .ok_or(REFUSED)?;
        if expected.is_some_and(|meta| meta != &record.meta) {
            return Err(REFUSED.into());
        }
        return Ok(Registered {
            meta: record.meta,
            references: record.references,
        });
    }
    let view = service.get_worktree(id).await.map_err(|_| REFUSED)?;
    if expected.is_some_and(|meta| meta != &view.meta)
        || !view.status.path_exists
        || !view.status.is_git_worktree
        || view.status.error.is_some()
    {
        return Err(REFUSED.into());
    }
    Ok(Registered {
        meta: view.meta,
        references: view.references,
    })
}

fn has_reference(view: &Registered, kind: &str, chat: &str, task: Option<&TaskMeta>) -> bool {
    view.references.iter().any(|r| {
        r.kind == kind
            && r.chat_id.as_deref() == Some(chat)
            && r.task_id.as_deref() == task.map(|t| t.task_id.as_str())
            && r.card_id == task.and_then(|t| t.card_id.clone())
            && r.agent_id == task.and_then(|t| t.agent_id.clone())
    })
}

async fn resolve_own(
    caller: &SyncCaller,
    recovery: bool,
) -> Result<(WorktreeMeta, String, Value, Vec<PathBuf>), String> {
    let thread = live_caller(caller).await?;
    let meta = caller
        .scope_worktree
        .clone()
        .ok_or("No active source worktree; pass card_id or child_chat_id for a delegated source")?;
    let view = registered(&caller.app, &meta, recovery).await?;
    let mut extra = Vec::new();
    let card_identity = match caller.task_meta.as_ref() {
        None => {
            if caller.nested
                && caller
                    .app
                    .agents
                    .find_agent_id_by_child_chat_id(&caller.chat_id)
                    .await
                    .is_none()
            {
                return Err(REFUSED.into());
            }
            if !matches!(meta.kind.as_str(), "chat" | "subagent")
                || meta.task_id.is_some()
                || meta.card_id.is_some()
                || meta.agent_id.is_some()
                || !has_reference(&view, &meta.kind, &caller.chat_id, None)
            {
                return Err(REFUSED.into());
            }
            Value::Null
        }
        Some(task) => {
            let (card_id, agent_id, planner) =
                match (&task.card_id, &task.agent_id, &task.planner_chat_id) {
                    (Some(c), Some(a), Some(p)) if task.role == "agents" => (c, a, p),
                    _ => return Err(REFUSED.into()),
                };
            if meta.kind != "task_agent"
                || meta.task_id.as_deref() != Some(task.task_id.as_str())
                || meta.card_id.as_deref() != Some(card_id.as_str())
                || meta.agent_id.as_deref() != Some(agent_id.as_str())
                || !has_reference(&view, "task_agent", &caller.chat_id, Some(task))
                || !crate::tasks::admission::is_tracked(&caller.chat_id).await
            {
                return Err(REFUSED.into());
            }
            let (board, _) = crate::tools::command_checkout::read_board(
                &meta.source_workspace_root,
                &task.task_id,
            )
            .await?;
            board.require_owner(planner).map_err(|_| REFUSED)?;
            let card = board.get_card(card_id).ok_or(REFUSED)?;
            if card.assignee.as_deref() != Some(agent_id.as_str())
                || card.agent_chat_id.as_deref() != Some(caller.chat_id.as_str())
                || card.agent_worktree_name.as_deref() != Some(meta.id.as_str())
                || card.paused_at.is_some()
            {
                return Err(REFUSED.into());
            }
            extra.push(
                meta.source_workspace_root
                    .join(".refact/tasks")
                    .join(&task.task_id)
                    .join("board.yaml"),
            );
            card_digest(&board.owner_chat_id, card)
        }
    };
    let actor = format!("chat:{}", caller.chat_id);
    Ok((
        meta,
        actor,
        json!({"thread":thread,"references":view.references,"card":card_identity}),
        extra,
    ))
}

fn card_digest(owner: &Option<String>, card: &crate::tasks::types::BoardCard) -> Value {
    json!({"owner":owner,"column":card.column,"paused":card.paused_at,"assignee":card.assignee,
        "chat":card.agent_chat_id,"name":card.agent_worktree_name,"path":card.agent_worktree,
        "branch":card.agent_branch,"team":card.team_members.len(),
        "winner":card.ab_variants.as_ref().map(|v| v.winner.clone())})
}

async fn root_caller(caller: &SyncCaller) -> Result<Value, String> {
    if caller.nested
        || caller.root_chat_id != caller.chat_id
        || caller
            .app
            .agents
            .find_agent_id_by_child_chat_id(&caller.chat_id)
            .await
            .is_some()
    {
        return Err(REFUSED.into());
    }
    live_caller(caller).await
}

async fn resolve_card(
    caller: &SyncCaller,
    card_id: &str,
    expected_worktree: Option<&str>,
    recovery: bool,
) -> Result<(WorktreeMeta, String, Value, Vec<PathBuf>), String> {
    let thread = root_caller(caller).await?;
    let task = caller.task_meta.clone().ok_or(REFUSED)?;
    let planner =
        crate::tools::command_checkout::planner_identity(&caller.app, &caller.chat_id, &task)
            .await?;
    let work =
        crate::coordination::scope::model_caller(&caller.app, &caller.chat_id, &caller.model)
            .await
            .map_err(|_| REFUSED)?;
    let source = work.source_root.clone();
    let (board, _) = crate::tools::command_checkout::read_board(&source, &task.task_id).await?;
    board.require_owner(&caller.chat_id).map_err(|error| {
        format!("{error}; owning planner chat: {}. Open that planner chat to synchronize this card source", board.owner_chat_id.as_deref().unwrap_or("unclaimed"))
    })?;
    let card = board.get_card(card_id).ok_or(REFUSED)?;
    if !recovery
        && (card.column != "done"
            || card.paused_at.is_some()
            || !card.team_members.is_empty()
            || !crate::tools::command_checkout::selected_winner_matches(card))
    {
        return Err("Card source synchronization requires a completed, unpaused card with a selected A/B winner; use Status or Abort for recovery".into());
    }
    let service = WorktreeService::new_async(caller.app.gcx.cache_dir.clone(), source.clone())
        .await
        .map_err(|_| REFUSED)?;
    if recovery {
        let registry = service.load_registry().await.map_err(|_| REFUSED)?;
        let mut candidates = registry.records.into_iter().filter(|r| {
            r.meta.kind == "task_agent"
                && r.meta.enforce
                && r.meta.task_id.as_deref() == Some(task.task_id.as_str())
                && r.meta.card_id.as_deref() == Some(card_id)
                && expected_worktree.is_none_or(|id| r.meta.id == id)
        });
        let view = candidates.next().ok_or(REFUSED)?;
        if candidates.next().is_some() {
            return Err("Multiple registered card sources; narrow worktree_id for recovery".into());
        }
        let meta = view.meta;
        if meta.source_workspace_root != source
            || meta.agent_id.as_deref().is_none_or(str::is_empty)
            || !view.references.iter().any(|r| {
                r.kind == "task_agent"
                    && r.task_id == meta.task_id
                    && r.card_id == meta.card_id
                    && r.agent_id == meta.agent_id
                    && r.chat_id.is_some()
            })
        {
            return Err(REFUSED.into());
        }
        for reference in view.references.iter().filter(|r| r.kind == "task_agent") {
            let chat = reference.chat_id.as_deref().ok_or(REFUSED)?;
            let thread = match session(&caller.app, chat).await {
                Some(session) => session.lock().await.thread.clone(),
                None => {
                    crate::chat::trajectories::load_trajectory_for_chat(
                        caller.app.gcx.clone(),
                        chat,
                    )
                    .await
                    .ok_or(REFUSED)?
                    .thread
                }
            };
            let bound = thread.task_meta.as_ref().ok_or(REFUSED)?;
            if thread.worktree.as_ref() != Some(&meta)
                || bound.role != "agents"
                || Some(&bound.task_id) != meta.task_id.as_ref()
                || bound.card_id != meta.card_id
                || bound.agent_id != meta.agent_id
                || reference.task_id != meta.task_id
                || reference.card_id != meta.card_id
                || reference.agent_id != meta.agent_id
            {
                return Err(REFUSED.into());
            }
        }
        ensure_source_quiescent(&caller.app, &meta, &view.references, None).await?;
        let actor = format!("card:{}:{}:{}", task.task_id, card_id, caller.chat_id);
        let board_path = source
            .join(".refact/tasks")
            .join(&task.task_id)
            .join("board.yaml");
        return Ok((
            meta,
            actor,
            json!({"thread":thread,"planner":planner,"owner":board.owner_chat_id,
            "card_id":card_id,"references":view.references}),
            vec![board_path],
        ));
    }
    let id = card.agent_worktree_name.as_deref().ok_or(REFUSED)?;
    let agent_chat = card.agent_chat_id.as_deref().ok_or(REFUSED)?;
    let view = registered_id(&service, id, None, recovery).await?;
    let meta = view.meta.clone();
    if meta.kind != "task_agent"
        || !meta.enforce
        || meta.task_id.as_deref() != Some(task.task_id.as_str())
        || meta.card_id.as_deref() != Some(card.id.as_str())
        || meta.agent_id.as_deref().is_none_or(str::is_empty)
        || meta.agent_id != card.assignee
        || dunce::canonicalize(&meta.source_workspace_root)
            .ok()
            .as_ref()
            != Some(&source)
        || card.agent_worktree.as_deref().map(std::path::Path::new) != Some(meta.root.as_path())
        || meta.branch.is_none()
        || meta.branch != card.agent_branch
        || !view.references.iter().any(|r| {
            r.kind == "task_agent"
                && r.task_id == meta.task_id
                && r.card_id == meta.card_id
                && r.agent_id == meta.agent_id
                && r.chat_id.as_deref() == Some(agent_chat)
        })
    {
        return Err(REFUSED.into());
    }
    if !card.team_members.is_empty() {
        return Err(REFUSED.into());
    }
    crate::tools::command_checkout::ensure_card_agent_stopped(
        &caller.app,
        &task,
        &caller.chat_id,
        &source,
        card,
        &meta,
    )
    .await?;
    let actor = format!("card:{}:{}:{}", task.task_id, card.id, caller.chat_id);
    let board_path = source
        .join(".refact/tasks")
        .join(&task.task_id)
        .join("board.yaml");
    Ok((
        meta,
        actor,
        json!({"thread":thread,"planner":planner,"card":card_digest(&board.owner_chat_id, card),"references":view.references}),
        vec![board_path],
    ))
}

async fn resolve_child(
    caller: &SyncCaller,
    child: &str,
    recovery: bool,
) -> Result<(WorktreeMeta, String, Value, Vec<PathBuf>), String> {
    let thread = root_caller(caller).await?;
    if caller.task_meta.is_some() {
        return Err(REFUSED.into());
    }
    let agent_id = caller
        .app
        .agents
        .find_agent_id_by_child_chat_id(child)
        .await
        .ok_or(REFUSED)?;
    let record = caller
        .app
        .agents
        .get(&caller.chat_id, &agent_id)
        .await
        .map_err(|_| REFUSED)?;
    if record.child_chat_id.as_deref() != Some(child)
        || record.parent_chat_id != caller.chat_id
        || !record.status.is_terminal()
        || !record.followups.is_empty()
        || record.worktree_id.is_none()
        || execution_live(&caller.app, child)
        || crate::tasks::admission::is_tracked(child).await
    {
        return Err("Child source synchronization requires a direct child whose execution is stopped with no queued follow-ups".into());
    }
    let child_thread = match session(&caller.app, child).await {
        Some(live) => {
            let s = live.lock().await;
            if s.chat_id != child
                || !s.command_queue.is_empty()
                || matches!(
                    s.runtime.state,
                    refact_runtime_api::SessionState::Generating
                        | refact_runtime_api::SessionState::ExecutingTools
                        | refact_runtime_api::SessionState::WaitingIde
                        | refact_runtime_api::SessionState::Paused
                )
            {
                return Err(REFUSED.into());
            }
            s.thread.clone()
        }
        None => {
            crate::chat::trajectories::load_trajectory_for_chat(caller.app.gcx.clone(), child)
                .await
                .ok_or(REFUSED)?
                .thread
        }
    };
    let meta = child_thread.worktree.clone().ok_or(REFUSED)?;
    if child_thread.parent_id.as_deref() != Some(caller.chat_id.as_str())
        || child_thread.root_chat_id.as_deref() != Some(caller.chat_id.as_str())
        || child_thread.task_meta.is_some()
        || meta.kind != "subagent"
        || record.worktree_id.as_deref() != Some(meta.id.as_str())
        || meta.task_id.is_some()
        || meta.card_id.is_some()
        || meta.agent_id.is_some()
    {
        return Err(REFUSED.into());
    }
    let view = registered(&caller.app, &meta, recovery).await?;
    if !has_reference(&view, "subagent", child, None) {
        return Err(REFUSED.into());
    }
    let actor = format!("parent:{}:{}", caller.chat_id, child);
    Ok((
        meta,
        actor,
        json!({"thread":thread,"child":{"agent":agent_id,"generation":record.run_generation,
        "status":record.status.as_str(),"worktree":record.worktree_id,"parent":record.parent_chat_id},
        "references":view.references}),
        Vec::new(),
    ))
}

pub(crate) async fn resolve(
    caller: &SyncCaller,
    selector: &SyncSelector,
    recovery: bool,
) -> Result<SyncAuthority, String> {
    let (worktree, actor, identity, extra) = match selector {
        SyncSelector::Own => resolve_own(caller, recovery).await?,
        SyncSelector::Card(card, worktree) => {
            resolve_card(caller, card, worktree.as_deref(), recovery).await?
        }
        SyncSelector::Child(child) => resolve_child(caller, child, recovery).await?,
    };
    let mut paths = vec![
        worktree.root.clone(),
        worktree.source_workspace_root.clone(),
    ];
    paths.extend(extra);
    let (records, model) = clear_paths(caller, paths).await?;
    let policy = crate::coordination::privacy_guard::policy_key(&caller.app.gcx, &model)
        .map_err(|_| REFUSED)?;
    let digest_input =
        json!({"worktree":worktree,"actor":actor,"identity":identity,"policy":policy});
    let digest = crate::file_activity::digest_of(&digest_input.to_string());
    let provenance = json!({"selector":match selector {
            SyncSelector::Own => json!({"kind":"own"}),
            SyncSelector::Card(card, _) => json!({"kind":"card","card_id":card}),
            SyncSelector::Child(child) => json!({"kind":"child","child_chat_id":child}),
        },"worktree_id":worktree.id,"source_workspace_root":worktree.source_workspace_root,"actor":actor});
    Ok(SyncAuthority {
        worktree,
        actor,
        provenance,
        records,
        digest,
    })
}

async fn clear_paths(
    caller: &SyncCaller,
    paths: Vec<PathBuf>,
) -> Result<
    (
        Vec<refact_privacy::FileRecord>,
        crate::caps::BaseModelRecord,
    ),
    String,
> {
    let work =
        crate::coordination::scope::model_caller(&caller.app, &caller.chat_id, &caller.model)
            .await
            .map_err(|_| REFUSED)?;
    let model = work.model.ok_or("Model privacy policy unavailable")?;
    let records = crate::privacy::records::declared_file_records(&caller.app.gcx, paths)
        .map_err(|e| e.to_string())?;
    crate::privacy::destinations::clear_for_model(&caller.app.gcx, Audit(records.clone()), &model)
        .map_err(|e| e.to_string())?;
    Ok((records, model))
}

impl SyncAuthority {
    pub(crate) async fn clear_source_paths<'a>(
        &mut self,
        caller: &SyncCaller,
        relative: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), String> {
        let mut paths = Vec::new();
        for path in relative {
            let rel = std::path::Path::new(path);
            if rel.is_absolute()
                || rel
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return Err("Sync path must be a relative path inside the source worktree".into());
            }
            paths.push(self.worktree.root.join(rel));
        }
        if paths.is_empty() {
            return Ok(());
        }
        let (records, _) = clear_paths(caller, paths).await?;
        self.records.extend(records);
        Ok(())
    }

    pub(crate) fn phase_validator(
        &self,
        caller: SyncCaller,
        selector: SyncSelector,
        recovery: bool,
    ) -> refact_worktrees::locking::PhaseValidator {
        let runtime = tokio::runtime::Handle::current();
        let expected = self.digest.clone();
        Arc::new(move || {
            let current = runtime.block_on(resolve(&caller, &selector, recovery))?;
            if current.digest != expected {
                return Err(REFUSED.into());
            }
            Ok(())
        })
    }

    pub(crate) fn same(&self, other: &Self) -> bool {
        self.digest == other.digest
    }
}

struct Audit(Vec<refact_privacy::FileRecord>);

impl refact_privacy::PrivacyAudited for Audit {
    fn privacy_records(
        &self,
    ) -> Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError> {
        Ok(self.0.iter().cloned().enumerate().collect())
    }
}
