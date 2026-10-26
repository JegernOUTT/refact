//! Local work discovery (M1-03).
//!
//! Builds one catalog of the work happening in a project from in-memory live
//! state only: the chat session map and the background-agent registry, plus
//! indexed worktree-registry metadata. Trajectory bodies are never scanned and
//! nothing here reads message content.
//!
//! Session locks are tried, never awaited: a session busy inside a turn
//! contributes a `stale` row rather than blocking the catalog.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use refact_work_core::identity::WorkKey;

use crate::agents::types::{BackgroundAgent, BgAgentStatus};
use crate::app_state::AppState;
use crate::chat::types::ChatSession;

use super::roots::{self, ProjectRoots};
use super::types::{Disclosure, WorkKindTag, WorkRow, WorkStateTag, WorkMetadata, Freshness};

/// Everything discovered for one project, before filtering/paging/privacy.
pub struct WorkCatalog {
    pub project: String,
    pub roots: ProjectRoots,
    /// Deduped rows keyed by [`WorkKey`].
    rows: BTreeMap<String, WorkRow>,
    pub degraded: Vec<String>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Coarse state for a live chat session.
///
/// Only lifecycle matters here. Compression phases, streaming deltas and tool
/// progress deliberately do not change the result.
fn session_state(session: &ChatSession) -> WorkStateTag {
    use crate::chat::types::SessionState;
    if session.closed {
        if session.thread.link_type.as_deref() == Some("review_stage")
            && session.runtime.state == SessionState::Error
        {
            return WorkStateTag::Cancelled;
        }
        return WorkStateTag::Completed;
    }
    match session.runtime.state {
        SessionState::Generating | SessionState::ExecutingTools => WorkStateTag::Running,
        SessionState::Paused if !session.runtime.pause_reasons.is_empty() => {
            WorkStateTag::WaitingApproval
        }
        SessionState::Paused => WorkStateTag::Parked,
        SessionState::WaitingIde | SessionState::WaitingUserInput => WorkStateTag::WaitingApproval,
        SessionState::Completed => WorkStateTag::Completed,
        SessionState::Error => WorkStateTag::Failed,
        SessionState::Idle => {
            if session.runtime.queue_size > 0 || !session.command_queue.is_empty() {
                WorkStateTag::Queued
            } else {
                WorkStateTag::Idle
            }
        }
    }
}

fn agent_state(agent: &BackgroundAgent) -> WorkStateTag {
    match agent.status {
        BgAgentStatus::Queued => WorkStateTag::Queued,
        BgAgentStatus::Running => WorkStateTag::Running,
        BgAgentStatus::WaitingForApproval => WorkStateTag::WaitingApproval,
        BgAgentStatus::Completed => WorkStateTag::Completed,
        BgAgentStatus::Failed => WorkStateTag::Failed,
        BgAgentStatus::Cancelled => WorkStateTag::Cancelled,
        BgAgentStatus::Interrupted => WorkStateTag::Interrupted,
        BgAgentStatus::Closed => WorkStateTag::Closed,
    }
}

/// Classify a chat by its task metadata: planner role vs board executor vs
/// a plain top-level chat.
fn session_kind(session: &ChatSession) -> WorkKindTag {
    match session.thread.task_meta.as_ref() {
        Some(meta) if meta.role.eq_ignore_ascii_case("planner") => WorkKindTag::TaskPlanner,
        Some(_) => WorkKindTag::BoardAgent,
        None => WorkKindTag::Chat,
    }
}

/// Build the row for one live chat session.
fn row_from_session(session: &ChatSession, roots: &ProjectRoots, stale: bool) -> WorkRow {
    let root = match session.thread.worktree.as_ref() {
        Some(meta) => roots
            .get(&meta.id)
            .cloned()
            .unwrap_or_else(|| roots::worktree_root_ref(meta)),
        None => {
            let fresh = roots::project_root_ref(
                session
                    .work_project_root
                    .as_deref()
                    .expect("scoped before projection"),
            );
            roots.get(&fresh.id).cloned().unwrap_or(fresh)
        }
    };
    let state = session_state(session);
    let attention = matches!(state, WorkStateTag::WaitingApproval | WorkStateTag::Failed)
        || !session.waiting_for_card_ids.is_empty();
    let title = if session.thread.title.trim().is_empty() {
        None
    } else {
        Some(session.thread.title.clone())
    };
    let (source_records, provenance_complete) = session
        .work_provenance
        .snapshot(session.messages.len(), &crate::file_activity::settings());
    WorkRow {
        key: WorkKey::trajectory(session.chat_id.clone()),
        metadata: WorkMetadata {
            task_id: session.thread.task_meta.as_ref().map(|m| m.task_id.clone()),
            card_id: session
                .thread
                .task_meta
                .as_ref()
                .and_then(|m| m.card_id.clone()),
            runtime_status: Some(format!("{:?}", session.runtime.state)),
            freshness: Freshness::Live,
            source_revision: session.event_seq,
            last_activity_at: Some(
                now_ms().saturating_sub(session.last_activity.elapsed().as_millis() as u64),
            ),
            ..Default::default()
        },
        kind: session_kind(session),
        state,
        attention,
        title,
        summary: session
            .goal
            .as_ref()
            .map(|goal| format!("goal {:?}", goal.status)),
        root,
        parent_chat_id: session.thread.parent_id.clone(),
        root_chat_id: session.thread.root_chat_id.clone(),
        chat_id: Some(session.chat_id.clone()),
        model: Some(session.thread.model.clone()).filter(|m| !m.is_empty()),
        current_tool: None,
        declared_files: Vec::new(),
        observed_files: Vec::new(),
        queue_size: session.runtime.queue_size.max(session.command_queue.len()),
        step_count: 0,
        open_questions: session.waiting_for_card_ids.len(),
        updated_at_ms: now_ms().saturating_sub(session.last_activity.elapsed().as_millis() as u64),
        stale,
        disclosure: Disclosure::Full,
        source_records,
        provenance_complete,
    }
}

/// Build the row for one background agent.
fn row_from_agent(agent: &BackgroundAgent, roots: &ProjectRoots) -> WorkRow {
    let root = agent
        .worktree_id
        .as_ref()
        .and_then(|id| roots.get(id).cloned())
        .unwrap_or_else(|| roots.project_root());
    let state = agent_state(agent);
    let open_questions = agent
        .questions
        .iter()
        .filter(|question| question.answer.is_none())
        .count();
    let attention =
        matches!(state, WorkStateTag::WaitingApproval | WorkStateTag::Failed) || open_questions > 0;
    WorkRow {
        key: WorkKey::background_agent(agent.agent_id.clone()),
        metadata: WorkMetadata {
            run: Some(WorkKey::background_agent(agent.agent_id.clone()).run(agent.run_generation)),
            runtime_status: Some(agent.status.as_str().into()),
            last_activity_at: Some(agent.last_update_at.timestamp_millis().max(0) as u64),
            ..Default::default()
        },
        kind: WorkKindTag::Subagent,
        state,
        attention,
        title: Some(agent.title.clone()).filter(|title| !title.is_empty()),
        summary: agent
            .progress
            .clone()
            .or_else(|| agent.result_summary.clone())
            .or_else(|| agent.goal_summary.clone()),
        root,
        parent_chat_id: Some(agent.parent_chat_id.clone()),
        root_chat_id: agent.parent_root_chat_id.clone(),
        chat_id: agent.child_chat_id.clone(),
        model: Some(agent.model.clone()).filter(|model| !model.is_empty()),
        current_tool: agent.current_tool.clone(),
        // Declared targets and observed edits stay separate: `target_files` is
        // what the agent said it would touch, `edited_files` is what the
        // registry actually recorded. Coverage of observed edits is partial
        // until the file-activity source is wired in.
        declared_files: agent.target_files.clone(),
        observed_files: agent.edited_files.clone(),
        queue_size: agent.pending_deliveries.len(),
        step_count: agent.step_count,
        open_questions,
        updated_at_ms: agent.last_update_at.timestamp_millis().max(0) as u64,
        stale: false,
        disclosure: Disclosure::Full,
        source_records: Vec::new(),
        provenance_complete: false,
    }
}

/// Collect live chat rows without ever awaiting a session lock.
///
/// The outer sessions map is read under its async lock (cheap, never held
/// across work). Each per-session mutex is only `try_lock`ed; a contended
/// session yields a minimal `stale` row so a busy chat cannot stall discovery.
async fn collect_session_rows(
    app: &AppState,
    roots: &ProjectRoots,
    degraded: &mut Vec<String>,
) -> Vec<WorkRow> {
    let sessions: Vec<(String, Arc<tokio::sync::Mutex<ChatSession>>)> = {
        let guard = app.chat.sessions.read().await;
        guard
            .iter()
            .map(|(id, session)| (id.clone(), session.clone()))
            .collect()
    };

    let mut rows = Vec::new();
    let mut contended = 0usize;
    for (_chat_id, session_arc) in sessions {
        match session_arc.try_lock() {
            Ok(session) => {
                let Some(source) = super::scope::session_source_root(&session) else {
                    continue;
                };
                if roots::project_identity(&source) != roots.project {
                    continue;
                }
                let mut row = row_from_session(&session, roots, false);
                row.updated_at_ms = app.work_views.observation_time(session.last_activity);
                row.metadata.last_activity_at = Some(row.updated_at_ms);
                rows.push(row);
            }
            Err(_) => contended += 1,
        }
    }
    if contended > 0 {
        degraded.push("live source temporarily unavailable".into());
    }
    rows
}

/// Discover all work for the project owning `source_workspace_root`.
///
/// `source_workspace_root: None` resolves the active project (the source
/// checkout behind the active worktree when the engine runs inside one).
pub async fn build_catalog(
    app: &AppState,
    source_workspace_root: Option<&str>,
) -> Result<WorkCatalog, String> {
    let source_root =
        roots::resolve_source_workspace_root(app.gcx.clone(), source_workspace_root).await?;
    let roots = roots::load_project_roots(app.gcx.clone(), source_root).await;
    let mut degraded = roots.degraded.clone();

    let mut rows: BTreeMap<String, WorkRow> = BTreeMap::new();

    for row in super::history::indexed_rows(app, &roots, &mut degraded).await {
        rows.insert(row.key.to_string(), row);
    }
    let (board_rows, boards_unavailable) = app.work_boards.rows(&roots.source_workspace_root).await;
    if boards_unavailable {
        degraded.push("board index unavailable".into());
    }
    for mut row in board_rows {
        if let Some(location) = roots.get(&row.root.id) {
            row.root = location.clone();
        } else {
            row.root.path = None;
        }
        rows.insert(row.key.to_string(), row);
    }
    for row in collect_session_rows(app, &roots, &mut degraded).await {
        rows.insert(row.key.to_string(), row);
    }

    // Background agents come second so they win the dedup for their own key
    // space; a subagent's child chat is folded into the agent row below.
    let agents = app.agents.list_all().await;
    let tickets = app.work_scheduler.snapshot().tickets;
    let mut child_chat_ids: HashSet<String> = HashSet::new();
    for agent in &agents {
        let child_row = agent
            .child_chat_id
            .as_ref()
            .and_then(|id| rows.get(&WorkKey::trajectory(id).to_string()));
        let owned_scope = app.agent_executions.saved_scope(&agent.agent_id);
        let in_scope = owned_scope
            .as_ref()
            .is_some_and(|source| roots::project_identity(source) == roots.project)
            || child_row.is_some()
            || agent
                .worktree_id
                .as_ref()
                .is_some_and(|id| roots.get(id).is_some());
        if !in_scope {
            continue;
        }
        let mut row = row_from_agent(agent, &roots);
        if row.state == WorkStateTag::Queued {
            if let Some((records, complete)) =
                app.agent_executions.launch_provenance(&agent.agent_id)
            {
                row.source_records = records;
                row.provenance_complete = complete;
            }
        }
        if let Some(child) = child_row {
            row.root = child.root.clone();
        }
        let live = app.agents.has_runtime(&agent.agent_id).await;
        if !live && !row.state.is_terminal() {
            row.state = WorkStateTag::Interrupted;
            row.stale = true;
        }
        row.metadata.freshness = if live {
            Freshness::Live
        } else if now_ms().saturating_sub(row.updated_at_ms)
            <= crate::file_activity::settings()
                .recent_window_secs
                .saturating_mul(1000)
        {
            Freshness::Recent
        } else {
            Freshness::Stale
        };
        if let Some(child) = &agent.child_chat_id {
            child_chat_ids.insert(child.clone());
        }
        rows.insert(row.key.to_string(), row);
    }

    // Dedup: a background agent's child chat is the same unit of work as the
    // agent record, so drop the duplicate trajectory row and keep the agent's
    // richer view. Carry over anything only the live session knew.
    let mut folded: Vec<(String, WorkRow)> = Vec::new();
    for (key, row) in rows.iter() {
        if let Some(chat_id) = &row.chat_id {
            if row.key.runtime == refact_work_core::identity::RuntimeKind::Trajectory
                && child_chat_ids.contains(chat_id)
            {
                folded.push((key.clone(), row.clone()));
            }
        }
    }
    let by_child: HashMap<String, String> = agents
        .iter()
        .filter_map(|agent| {
            agent.child_chat_id.clone().map(|child| {
                (
                    child,
                    WorkKey::background_agent(agent.agent_id.clone()).to_string(),
                )
            })
        })
        .collect();
    for (key, session_row) in folded {
        rows.remove(&key);
        let Some(chat_id) = session_row.chat_id.as_ref() else {
            continue;
        };
        let Some(agent_key) = by_child.get(chat_id) else {
            continue;
        };
        if let Some(agent_row) = rows.get_mut(agent_key) {
            if agent_row.queue_size == 0 {
                agent_row.queue_size = session_row.queue_size;
            }
            if agent_row.title.is_none() {
                agent_row.title = session_row.title.clone();
            }
            agent_row.stale |= session_row.stale;
            agent_row.source_records = session_row.source_records;
            agent_row.provenance_complete = session_row.provenance_complete;
        }
    }

    // Keep only work belonging to this project. Rows whose root resolved to a
    // foreign project (e.g. a chat scoped to another checkout) are excluded.
    let project = roots.project.clone();
    rows.retain(|_, row| row.root.project == project);

    for ticket in tickets
        .iter()
        .filter(|ticket| ticket.scope.project.as_str() == project)
    {
        rows.entry(ticket.run.work.to_string())
            .or_insert_with(|| WorkRow {
                key: ticket.run.work.clone(),
                metadata: WorkMetadata {
                    run: Some(ticket.run.clone()),
                    freshness: Freshness::Live,
                    ..Default::default()
                },
                kind: if ticket.run.work.runtime == refact_work_core::RuntimeKind::Trajectory {
                    WorkKindTag::BoardAgent
                } else {
                    WorkKindTag::Subagent
                },
                state: WorkStateTag::Queued,
                attention: false,
                title: None,
                summary: None,
                root: roots.project_root(),
                parent_chat_id: None,
                root_chat_id: Some(ticket.scope.root.to_string()),
                chat_id: (ticket.run.work.runtime == refact_work_core::RuntimeKind::Trajectory)
                    .then(|| ticket.run.work.id.clone()),
                model: None,
                current_tool: None,
                declared_files: Vec::new(),
                observed_files: Vec::new(),
                queue_size: 0,
                step_count: 0,
                open_questions: 0,
                updated_at_ms: 0,
                stale: false,
                disclosure: Disclosure::Full,
                source_records: Vec::new(),
                provenance_complete: false,
            });
    }
    let observations = crate::file_activity::observations(None);
    for row in rows.values_mut() {
        let actor = row.chat_id.as_deref().unwrap_or(&row.key.id);
        row.metadata.observed_reads = observations
            .reads
            .iter()
            .filter(|read| read.actor == actor)
            .map(|read| read.path.clone())
            .collect();
        row.observed_files.extend(
            observations
                .writes
                .iter()
                .filter(|write| write.actor.as_deref() == Some(actor))
                .map(|write| write.path.clone()),
        );
        row.observed_files.sort();
        row.observed_files.dedup();
        row.metadata.observed_reads.sort();
        row.metadata.observed_reads.dedup();
    }

    // Both adapters use the same admission source. A parked runner can still
    // have ExecutingTools as its raw session state; never call that running.
    for row in rows.values_mut() {
        if let Some(ticket) = tickets.iter().find(|t| t.run.work == row.key) {
            row.metadata.run = Some(ticket.run.clone());
            row.metadata.priority = Some(ticket.priority);
            row.metadata.freshness = Freshness::Live;
            row.stale = false;
            row.root_chat_id = Some(ticket.scope.root.to_string());
            if row.state != WorkStateTag::WaitingApproval {
                row.state = match ticket.state {
                    refact_work_core::TicketState::Reserved
                    | refact_work_core::TicketState::Pending => WorkStateTag::Queued,
                    refact_work_core::TicketState::Parked
                    | refact_work_core::TicketState::Evicted => WorkStateTag::Parked,
                    refact_work_core::TicketState::Active => WorkStateTag::Running,
                };
            }
        }
    }

    // Work this engine does not own is only as live as the engine that could be
    // running it. Without presence data nothing is claimed either way.
    if let Some(liveness) = super::presence::project_liveness(&project) {
        for row in rows.values_mut() {
            if row.metadata.freshness != Freshness::Live {
                row.metadata.freshness =
                    super::presence::apply_to_freshness(row.metadata.freshness.clone(), liveness);
            }
        }
    }

    Ok(WorkCatalog {
        project,
        roots,
        rows,
        degraded,
    })
}

impl WorkCatalog {
    pub fn retain_visible(&mut self, rows: Vec<WorkRow>) {
        self.rows = rows
            .into_iter()
            .map(|row| (row.key.to_string(), row))
            .collect();
    }
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&WorkRow> {
        self.rows.get(key)
    }

    /// Look a row up by a caller-supplied id, accepting either a fully
    /// qualified `runtime:id` key or a bare chat/agent id.
    pub fn find(&self, id: &str) -> Option<&WorkRow> {
        if let Some(row) = self.rows.get(id) {
            return Some(row);
        }
        self.rows
            .values()
            .find(|row| row.key.id == id || row.chat_id.as_deref() == Some(id))
    }

    pub fn all_rows(&self) -> Vec<WorkRow> {
        self.rows.values().cloned().collect()
    }

    /// Rows sorted into presentation order.
    pub fn sorted_rows(&self) -> Vec<WorkRow> {
        let mut rows = self.all_rows();
        rows.sort_by_key(|row| row.sort_key());
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::types::ChatSession;

    #[tokio::test]
    async fn unbound_and_foreign_sessions_are_not_adopted_by_selected_project() {
        let project = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![project.path().into()];
        let app = AppState::from_gcx(gcx).await;
        for (id, source) in [
            ("local", Some(project.path().to_path_buf())),
            ("foreign", Some(foreign.path().to_path_buf())),
            ("unknown", None),
        ] {
            let mut session = ChatSession::new(id.into());
            session.work_project_root = source;
            app.chat
                .sessions
                .write()
                .await
                .insert(id.into(), Arc::new(tokio::sync::Mutex::new(session)));
        }
        let catalog = build_catalog(&app, None).await.unwrap();
        assert_eq!(catalog.len(), 1);
        assert!(catalog.find("local").is_some());
        assert!(catalog.find("foreign").is_none());
        assert!(catalog.find("unknown").is_none());
    }

    #[tokio::test]
    async fn message_events_maintain_provenance_without_a_subscriber_or_catalog_body_scan() {
        let project = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![project.path().into()];
        let app = AppState::from_gcx(gcx).await;
        let mut session = ChatSession::new("indexed".into());
        session.work_project_root = Some(project.path().into());
        assert_eq!(session.event_tx.receiver_count(), 0);
        let mut source = crate::call_validation::ChatMessage::new("tool".into(), "source".into());
        source.message_id = "source".into();
        source.extra.insert("privacy".into(), serde_json::json!({"files":[{
            "path": project.path().join("guarded.rs"), "zone":"guarded", "attribution":"declared"
        }]}));
        session.add_message(source.clone());
        let session = Arc::new(tokio::sync::Mutex::new(session));
        app.chat
            .sessions
            .write()
            .await
            .insert("indexed".into(), session.clone());
        for _ in 0..3 {
            let catalog = build_catalog(&app, None).await.unwrap();
            let row = catalog.find("indexed").unwrap();
            assert!(row.provenance_complete);
            assert_eq!(row.source_records.len(), 1);
        }
        {
            let mut session = session.lock().await;
            let mut malformed = source.clone();
            malformed
                .extra
                .insert("privacy".into(), serde_json::json!({"files":false}));
            session.update_message("source", malformed);
        }
        assert!(
            !build_catalog(&app, None)
                .await
                .unwrap()
                .find("indexed")
                .unwrap()
                .provenance_complete
        );
        session.lock().await.replace_messages(vec![source]);
        assert!(
            build_catalog(&app, None)
                .await
                .unwrap()
                .find("indexed")
                .unwrap()
                .provenance_complete
        );
        session.lock().await.remove_message("source");
        let catalog = build_catalog(&app, None).await.unwrap();
        let row = catalog.find("indexed").unwrap();
        assert!(row.provenance_complete);
        assert!(row.source_records.is_empty());
    }

    #[test]
    fn runner_mirrors_and_restores_refresh_provenance() {
        let mut source = crate::call_validation::ChatMessage::new("tool".into(), "source".into());
        source.extra.insert(
            "privacy".into(),
            serde_json::json!({"files":[{
                "path":"/p/guarded.rs", "zone":"guarded", "attribution":"declared"
            }]}),
        );
        let mut session = ChatSession::new_with_trajectory(
            "restored".into(),
            vec![source.clone()],
            Default::default(),
            "2026-01-01T00:00:00Z".into(),
            None,
            vec![],
            None,
        );
        let settings = crate::file_activity::settings();
        assert_eq!(session.work_provenance.snapshot(1, &settings).0.len(), 1);
        session.mirror_runner_messages(vec![]);
        assert_eq!(
            session.work_provenance.snapshot(0, &settings),
            (vec![], true)
        );
        session.mirror_runner_messages(vec![source]);
        assert_eq!(session.work_provenance.snapshot(1, &settings).0.len(), 1);
    }
}
