use std::sync::Arc;

use crate::app_state::AppState;
use crate::chat::types::{ChatSession, ThreadParams};
use crate::global_context::GlobalContext;
use super::types::TaskBoard;

#[cfg(test)]
tokio::task_local! {
    pub(crate) static REBIND_FAILURE: bool;
    pub(crate) static OWNER_REPAIR_SELECTED: (Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>);
}

pub(crate) fn current_planner_chat_id(board: &TaskBoard) -> Option<String> {
    board
        .owner_chat_id
        .clone()
        .filter(|owner| !owner.trim().is_empty())
}

fn is_standalone_planner(id: &str, meta: &crate::chat::types::TaskMeta) -> bool {
    meta.role == "planner"
        && meta.agent_id.is_none()
        && meta.card_id.is_none()
        && meta
            .planner_chat_id
            .as_deref()
            .is_none_or(|planner| planner == id)
}

fn is_legacy_planner_origin(mode: &str, link: Option<&str>, has_parent: bool) -> bool {
    let mode = crate::yaml_configs::customization_registry::map_legacy_mode_to_id(mode);
    !matches!(mode, "" | "task_agent")
        && match link {
            None => !has_parent,
            Some("handoff" | "mode_transition" | "branch") => mode == "task_planner",
            _ => false,
        }
}

pub(crate) fn is_legacy_owner_candidate(id: &str, thread: &ThreadParams) -> bool {
    thread
        .task_meta
        .as_ref()
        .is_some_and(|meta| is_standalone_planner(id, meta))
        && thread.buddy_meta.is_none()
        && is_legacy_planner_origin(
            &thread.mode,
            thread.link_type.as_deref(),
            thread.parent_id.is_some(),
        )
}

pub(crate) fn is_saved_legacy_owner_candidate(
    id: &str,
    saved: &crate::chat::trajectories::LoadedTrajectory,
) -> bool {
    !saved.initial_planner_creation && is_legacy_owner_candidate(id, &saved.thread)
}

pub(super) async fn has_saved_planner_history(
    planner_dir: &std::path::Path,
    task_id: &str,
) -> Result<bool, String> {
    use crate::chat::trajectories::{INITIAL_PLANNER_CREATION_KEY, PLANNER_BOOTSTRAP_CREATION};
    match tokio::fs::symlink_metadata(planner_dir).await {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Err("Unsafe task planner directory".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.to_string()),
    }
    let mut entries = tokio::fs::read_dir(planner_dir)
        .await
        .map_err(|error| error.to_string())?;
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        let path = entry.path();
        if entry.file_name() == "index.json"
            || path.extension().is_none_or(|ext| ext != "json")
            || !entry
                .file_type()
                .await
                .map_err(|e| e.to_string())?
                .is_file()
        {
            continue;
        }
        let body = tokio::fs::read(&path).await.map_err(|e| e.to_string())?;
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&body) else {
            return Ok(true);
        };
        let id = value["id"].as_str().unwrap_or_default();
        let mode = value["mode"].as_str().unwrap_or("agent");
        let link = value["link_type"].as_str();
        let creation = value[INITIAL_PLANNER_CREATION_KEY].as_str();
        if path.file_stem().and_then(|stem| stem.to_str()) == Some(id)
            && value["buddy_meta"].is_null()
            && serde_json::from_value::<crate::chat::types::TaskMeta>(value["task_meta"].clone())
                .is_ok_and(|meta| meta.task_id == task_id && is_standalone_planner(id, &meta))
            && creation != Some(PLANNER_BOOTSTRAP_CREATION)
            && is_legacy_planner_origin(mode, link, !value["parent_id"].is_null())
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) async fn repair_owner_on_planner_use(
    app: &AppState,
    session_arc: &Arc<tokio::sync::Mutex<ChatSession>>,
) -> Result<(), String> {
    use crate::chat::{trajectories, trajectory_index};
    let (path, chat_id, meta, source) = {
        let session = session_arc.lock().await;
        let Some(path) = &session.restored_planner_path else {
            return Ok(());
        };
        if session.closed || !is_legacy_owner_candidate(&session.chat_id, &session.thread) {
            return Ok(());
        }
        let Some(source) = crate::coordination::scope::session_source_root(&session) else {
            return Ok(());
        };
        (
            path.clone(),
            session.chat_id.clone(),
            session.thread.task_meta.clone().unwrap(),
            source,
        )
    };
    if super::storage::bound_task_active() || crate::file_activity::current_actor().is_some() {
        return Ok(());
    }
    let gcx = &app.gcx;
    let task = &meta.task_id;
    let binding = super::storage::TaskDirectoryBinding::capture(gcx.clone(), &source, task).await?;
    let board = binding
        .scope(super::storage::load_board(gcx.clone(), task))
        .await?;
    if board.owner_chat_id.is_none() && !board.reviewer_chat_ids.contains(&chat_id) {
        let dir = binding
            .scope(super::storage::find_task_dir(gcx.clone(), task))
            .await?
            .join("trajectories/planner");
        if dunce::canonicalize(&dir).ok().as_ref() != Some(&dir)
            || path != dir.join(format!("{chat_id}.json"))
        {
            return Ok(());
        }
        super::storage::require_regular_task_file(&path).await?;
        let entries = trajectory_index::list_trajectory_entries_with_rollout_shared_for(
            &app.chat.trajectory_index_coordinator,
            &dir,
            None,
            trajectory_index::TrajectoryIndexListingCaller::ChatPathLookup,
        )
        .await?;
        let mut candidates: Vec<_> = entries
            .iter()
            .filter_map(|entry| {
                if trajectories::validate_trajectory_id(&entry.id).is_err()
                    || entry.file_name != format!("{}.json", entry.id)
                    || board.reviewer_chat_ids.contains(&entry.id)
                {
                    return None;
                }
                Some((
                    chrono::DateTime::parse_from_rfc3339(&entry.created_at).ok()?,
                    &entry.id,
                    entry,
                ))
            })
            .collect();
        candidates.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(a.1)));
        for (created, id, entry) in candidates {
            let candidate_path = dir.join(&entry.file_name);
            if super::storage::require_regular_task_file(&candidate_path)
                .await
                .is_err()
            {
                continue;
            }
            let Some(saved) = trajectories::load_trajectory_for_chat(gcx.clone(), id).await else {
                continue;
            };
            if saved.source_path != candidate_path
                || !is_saved_legacy_owner_candidate(id, &saved)
                || !saved
                    .thread
                    .task_meta
                    .as_ref()
                    .is_some_and(|saved_meta| saved_meta.task_id == *task)
            {
                continue;
            }
            if chrono::DateTime::parse_from_rfc3339(&saved.created_at).ok() != Some(created) {
                return Err("Saved planner creation order changed; retry".into());
            }
            #[cfg(test)]
            if let Ok((selected, resume)) = OWNER_REPAIR_SELECTED.try_with(Clone::clone) {
                selected.notify_one();
                resume.notified().await;
            }
            binding
                .scope(super::storage::link_ownerless_planner(
                    gcx.clone(),
                    task,
                    &chat_id,
                    || {
                        let sessions = gcx
                            .chat_sessions
                            .try_read()
                            .map_err(|_| "Session registry is busy; retry")?;
                        if !sessions
                            .get(&chat_id)
                            .is_some_and(|current| Arc::ptr_eq(current, session_arc))
                        {
                            return Err("Restored planner session changed; retry".into());
                        }
                        let session = session_arc
                            .try_lock()
                            .map_err(|_| "Planner session is busy; retry")?;
                        if session.closed
                            || session
                                .closed_flag
                                .load(std::sync::atomic::Ordering::Acquire)
                            || session.restored_planner_path.as_ref() != Some(&path)
                            || session.thread.task_meta.as_ref() != Some(&meta)
                            || crate::coordination::scope::session_source_root(&session).as_ref()
                                != Some(&source)
                            || !is_legacy_owner_candidate(&chat_id, &session.thread)
                        {
                            return Err("Restored planner identity changed; retry".into());
                        }
                        Ok(())
                    },
                    &dir,
                    entry,
                ))
                .await?;
            break;
        }
    }
    session_arc.lock().await.restored_planner_path = None;
    Ok(())
}

pub(crate) async fn rebind_task_agents(
    gcx: Arc<GlobalContext>,
    task_id: &str,
) -> Result<usize, String> {
    #[cfg(test)]
    if REBIND_FAILURE.try_with(|fail| *fail).unwrap_or(false) {
        return Err("injected custody reconciliation failure".into());
    }
    let app = AppState::from_gcx(gcx.clone()).await;
    let board = super::storage::load_board(gcx.clone(), task_id).await?;
    let chats = board
        .cards
        .iter()
        .filter(|card| card.column == "doing")
        .flat_map(|card| {
            let mut chats = card.agent_chat_id.iter().cloned().collect::<Vec<_>>();
            chats.extend(
                card.team_members
                    .iter()
                    .filter_map(|member| member.agent_chat_id.clone()),
            );
            if let Some(variants) = &card.ab_variants {
                chats.extend(
                    [&variants.a, &variants.b]
                        .into_iter()
                        .filter(|variant| variant.finish.is_none())
                        .map(|variant| variant.chat_id.clone()),
                );
            }
            chats
        })
        .collect::<std::collections::HashSet<_>>();
    for chat in chats {
        if !gcx.chat_sessions.read().await.contains_key(&chat) {
            if let Err(error) =
                crate::chat::trajectories::rebind_persisted_task_agent(gcx.clone(), task_id, &chat)
                    .await
            {
                tracing::warn!(%error, "persisted custody reconciliation deferred");
            }
        }
    }
    let sessions = gcx
        .chat_sessions
        .read()
        .await
        .values()
        .cloned()
        .collect::<Vec<_>>();
    let mut matching = Vec::new();
    for session_arc in sessions {
        let matches = session_arc.try_lock().is_ok_and(|session| {
            session
                .thread
                .task_meta
                .as_ref()
                .is_some_and(|meta| meta.role == "agents" && meta.task_id == task_id)
        });
        if matches {
            matching.push(session_arc);
        }
    }
    if matching.is_empty() {
        return Ok(0);
    }
    let (_, guard) = match super::storage::locked_task_dir(gcx.clone(), task_id).await {
        Ok(locked) => locked,
        Err(error) => {
            tracing::warn!(%error, "live custody reconciliation deferred");
            return Ok(0);
        }
    };
    let board = match super::storage::load_board_locked(gcx.clone(), task_id, &guard).await {
        Ok(board) => board,
        Err(error) => {
            tracing::warn!(%error, "live custody reconciliation deferred");
            return Ok(0);
        }
    };
    let Some(owner) = current_planner_chat_id(&board) else {
        return Ok(0);
    };
    let mut changed_sessions = Vec::new();
    for session_arc in matching {
        let changed = {
            let Ok(mut session) = session_arc.try_lock() else {
                continue;
            };
            let Some(old) = session
                .thread
                .task_meta
                .clone()
                .filter(|meta| meta.role == "agents" && meta.task_id == task_id)
            else {
                continue;
            };
            let Some(bound) = old.planner_chat_id.as_deref() else {
                continue;
            };
            if session.closed || bound == owner || !board.planner_binding_is_current(bound) {
                continue;
            }
            let Some(card) = old
                .card_id
                .as_deref()
                .and_then(|id| board.get_card(id))
                .filter(|card| card.column == "doing")
            else {
                continue;
            };
            let chat = session.chat_id.as_str();
            let bound_card = card.agent_chat_id.as_deref() == Some(chat)
                || card
                    .team_members
                    .iter()
                    .any(|member| member.agent_chat_id.as_deref() == Some(chat))
                || card.ab_variants.as_ref().is_some_and(|variants| {
                    [&variants.a, &variants.b]
                        .iter()
                        .any(|variant| variant.chat_id == chat && variant.finish.is_none())
                });
            if !bound_card {
                continue;
            }
            let mut new = old.clone();
            new.planner_chat_id = Some(owner.clone());
            if let Some(agent_id) = old.agent_id.as_deref() {
                if !app.agent_executions.rebind_config(
                    agent_id,
                    &old,
                    new.clone(),
                    Some(owner.clone()),
                    Some(owner.clone()),
                ) {
                    continue;
                }
            }
            session.apply_custody_rebind(new, Some(owner.clone()), Some(owner.clone()));
            Some((session.chat_id.clone(), bound.to_string(), owner.clone()))
        };
        if let Some((chat, old, owner)) = changed {
            changed_sessions.push((session_arc, chat, old, owner));
        }
    }
    drop(guard);
    let rebound = changed_sessions.len();
    for (session_arc, chat, old, owner) in changed_sessions {
        if let Err(error) = gcx.agents.rebind_parent(&chat, &old, &owner).await {
            tracing::warn!(%error, "agent registry custody reconciliation deferred");
        }
        if crate::chat::session_runtime::board_execution(&app, &session_arc)
            .await
            .is_ok()
        {
            let mut session = session_arc.lock().await;
            if session.runtime.interruption.take().is_some() {
                if !session.runtime.paused
                    && session.runtime.pause_reasons.is_empty()
                    && session.runtime.state == crate::chat::types::SessionState::Interrupted
                {
                    session.set_runtime_state(crate::chat::types::SessionState::Idle, None);
                }
            }
        }
        if let Err(error) =
            crate::chat::trajectories::try_save_trajectory(app.clone(), session_arc).await
        {
            tracing::warn!(%error, "custody trajectory persistence deferred");
        }
    }
    Ok(rebound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::types::SessionState;

    #[tokio::test]
    async fn live_custody_rebind_converges_config_trajectory_and_registry() {
        let (_temp, app, task, chat, session, _) =
            crate::chat::task_agent_monitor::tests::setup_monitor_case(
                "doing",
                SessionState::Generating,
                std::time::Duration::ZERO,
                vec![],
            )
            .await;
        let (record, _, _) = app
            .agents
            .create(crate::agents::types::CreateAgentRequest {
                parent_chat_id: "planner-test".into(),
                parent_root_chat_id: Some("planner-test".into()),
                parent_tool_call_id: None,
                kind: crate::agents::types::BgAgentKind::Subagent,
                config_name: "subagent".into(),
                title: "custody".into(),
                prompt: "test".into(),
                target_files: vec![],
                model: "test".into(),
                model_type: None,
                goal_summary: None,
                plan_present: false,
                worktree_id: None,
                worktree_branch: None,
            })
            .await
            .unwrap();
        app.agents
            .mark_running(&record.agent_id, chat.clone())
            .await
            .unwrap();
        let mut config = crate::subchat::test_support::minimal_config(&chat, "planner-test");
        {
            let mut live = session.lock().await;
            live.work_project_root = Some(_temp.path().to_path_buf());
            let meta = live.thread.task_meta.as_mut().unwrap();
            meta.agent_id = Some(record.agent_id.clone());
            config.task_meta = Some(meta.clone());
            live.thread.parent_id = config.parent_id.clone();
            live.thread.root_chat_id = config.root_chat_id.clone();
        }
        app.agent_executions
            .remember_config(&record.agent_id, config);
        super::super::storage::update_board_atomic(app.gcx.clone(), &task, |board| {
            board.get_card_mut("T-1").unwrap().assignee = Some(record.agent_id.clone());
            board.transfer_owner("planner-test", 1);
            board.transfer_owner("successor", 2);
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(rebind_task_agents(app.gcx.clone(), &task).await.unwrap(), 1);
        let live = session.lock().await;
        assert_eq!(live.runtime.state, SessionState::Generating);
        assert!(live.runtime.interruption.is_none());
        assert_eq!(
            live.thread
                .task_meta
                .as_ref()
                .unwrap()
                .planner_chat_id
                .as_deref(),
            Some("successor")
        );
        assert_eq!(live.thread.parent_id.as_deref(), Some("successor"));
        drop(live);
        assert_eq!(
            app.agent_executions
                .saved_config(&record.agent_id)
                .unwrap()
                .task_meta
                .unwrap()
                .planner_chat_id
                .as_deref(),
            Some("successor")
        );
        let stored = crate::chat::trajectories::load_trajectory_for_chat(app.gcx.clone(), &chat)
            .await
            .unwrap();
        assert_eq!(
            stored.thread.task_meta.unwrap().planner_chat_id.as_deref(),
            Some("successor")
        );
        assert_eq!(
            app.agents
                .get_any(&record.agent_id)
                .await
                .unwrap()
                .parent_chat_id,
            "successor"
        );
        assert_eq!(rebind_task_agents(app.gcx.clone(), &task).await.unwrap(), 0);
    }
}
