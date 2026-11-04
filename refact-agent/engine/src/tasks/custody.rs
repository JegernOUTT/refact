use std::sync::Arc;

use crate::app_state::AppState;
use crate::global_context::GlobalContext;
use super::types::TaskBoard;

#[cfg(test)]
tokio::task_local! {
    pub(crate) static REBIND_FAILURE: bool;
}

pub(crate) fn current_planner_chat_id(board: &TaskBoard) -> Option<String> {
    board
        .owner_chat_id
        .clone()
        .filter(|owner| !owner.trim().is_empty())
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
