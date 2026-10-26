use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::types::ChatSession;
use crate::tools::task_tool_helpers::require_bound_planner_task;
use crate::tools::tool_task_check_agents::{
    AgentStatus, format_agent_statuses, get_agent_statuses, has_active_agent_statuses,
    parse_agent_status_query, waitable_agent_statuses,
};

const WAKE_UP_AFTER_SECS_MIN: u64 = 30;
const WAKE_UP_AFTER_SECS_MAX: u64 = 1800;
const MAX_WAITING_CARD_IDS: usize = 50;

pub(crate) fn parse_wake_up_after_secs(
    args: &HashMap<String, Value>,
) -> Result<Option<u64>, String> {
    let Some(value) = args.get("wake_up_after_secs") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let secs = value
        .as_u64()
        .ok_or_else(|| "wake_up_after_secs must be a non-negative integer".to_string())?;
    if secs < WAKE_UP_AFTER_SECS_MIN {
        return Err(format!(
            "wake_up_after_secs must be at least {} seconds",
            WAKE_UP_AFTER_SECS_MIN
        ));
    }
    if secs > WAKE_UP_AFTER_SECS_MAX {
        return Err(format!(
            "wake_up_after_secs must be at most {} seconds",
            WAKE_UP_AFTER_SECS_MAX
        ));
    }
    Ok(Some(secs))
}

fn natural_card_id_key(id: &str) -> (String, u64) {
    match id.rfind('-') {
        Some(pos) => {
            let prefix = &id[..pos];
            let suffix = &id[pos + 1..];
            match suffix.parse::<u64>() {
                Ok(n) => (prefix.to_string(), n),
                Err(_) => (id.to_string(), 0),
            }
        }
        None => (id.to_string(), 0),
    }
}

pub(crate) fn resolve_waiting_card_ids(statuses: &[&AgentStatus]) -> Vec<String> {
    let mut ids: Vec<String> = statuses.iter().map(|s| s.status_id()).collect();
    ids.sort_by(|a, b| natural_card_id_key(a).cmp(&natural_card_id_key(b)));
    ids.truncate(MAX_WAITING_CARD_IDS);
    ids
}

fn status_filter_was_explicit(args: &HashMap<String, Value>) -> bool {
    args.get("status_filter")
        .is_some_and(|value| !value.is_null())
}

fn replace_session_wake_up_at(session: &mut ChatSession, wake_up_after_secs: Option<u64>) {
    session.wake_up_at =
        wake_up_after_secs.map(|secs| Utc::now() + chrono::Duration::seconds(secs as i64));
}

pub(crate) async fn execute_task_agents_wait(
    ccx: Arc<AMutex<AtCommandsContext>>,
    tool_call_id: &String,
    args: &HashMap<String, Value>,
) -> Result<(bool, Vec<ContextEnum>), String> {
    let ccx_lock = ccx.lock().await;

    let is_planner = ccx_lock
        .task_meta
        .as_ref()
        .map(|m| m.role == "planner")
        .unwrap_or(false);

    if !is_planner {
        return Err(
            "wait(target=task_agents) can only be called by the task planner. \
                 Switch to the planner chat to check agent status."
                .to_string(),
        );
    }

    drop(ccx_lock);

    let wake_up_secs = parse_wake_up_after_secs(args)?;

    let query = parse_agent_status_query(args)?;
    let task_id = require_bound_planner_task(&ccx, args).await?;
    let (gcx, chat_facade, chat_id, app) = {
        let ccx_lock = ccx.lock().await;
        (
            ccx_lock.app.gcx.clone(),
            ccx_lock.app.chat.facade.clone(),
            ccx_lock.chat_id.clone(),
            ccx_lock.app.clone(),
        )
    };

    let statuses = get_agent_statuses(gcx, chat_facade, &task_id).await?;
    let mut result = format_agent_statuses(&statuses, &query)?;
    let waitable = waitable_agent_statuses(&statuses, &query, status_filter_was_explicit(args));
    let resolved_card_ids = resolve_waiting_card_ids(&waitable);

    if statuses.is_empty() {
        result.push_str("\nNo agents are currently running.\n");
    } else if !resolved_card_ids.is_empty() {
        result.push_str("\n⏳ **Agents are still working.** Do not check again, wait for the completion message to arrive.\n");
    } else if has_active_agent_statuses(&statuses) {
        result.push_str("\nNo matching live agents are running or awaiting approval.\n");
    } else {
        result.push_str("\nNo agents are currently running.\n");
    }

    if resolved_card_ids.is_empty() {
        result.push_str("\nNo matching live agents are running or awaiting approval; planner wait mode was not entered.\n");
        return Ok((
                false,
                vec![ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(json!({"outcome": if resolved_card_ids.is_empty() {"no_matching_agents"} else {"parked"}, "target":"task_agents", "evidence":result}).to_string()),
                    tool_calls: None,
                    tool_call_id: tool_call_id.clone(),
                    tool_failed: Some(true),
                    ..Default::default()
                })],
            ));
    }

    {
        let sessions = app.chat.sessions.read().await;
        if let Some(session_arc) = sessions.get(&chat_id) {
            let mut session = session_arc.lock().await;
            replace_session_wake_up_at(&mut session, wake_up_secs);
            session.waiting_for_card_ids = resolved_card_ids.clone();
            session.mark_persisted_runtime_changed();
        }
    }

    {
        let ccx_lock = ccx.lock().await;
        ccx_lock
            .abort_flag
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(json!({"outcome":"parked","target":"task_agents","card_ids":resolved_card_ids,"evidence":result}).to_string()),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;
    use std::sync::Arc;

    fn make_status(card_id: &str, column: &str) -> AgentStatus {
        AgentStatus {
            card_id: card_id.to_string(),
            variant_key: None,
            card_title: format!("{} title", card_id),
            agent_chat_id: format!("agent-{}", card_id),
            column: column.to_string(),
            priority: "P1".to_string(),
            session_state: Some(refact_runtime_api::SessionState::Generating),
            last_status_update: None,
            last_activity_at: None,
            started_at: None,
            paused_at: None,
            paused_by: None,
            final_report: (column == "done").then(|| "done".to_string()),
            last_tool_name: None,
            attention: None,
            change_seq: 0,
        }
    }

    fn offline_status(card_id: &str) -> AgentStatus {
        let mut status = make_status(card_id, "doing");
        status.session_state = None;
        status
    }

    #[test]
    fn wait_agents_rejects_wake_up_secs_below_30() {
        let mut args = HashMap::new();
        args.insert("wake_up_after_secs".to_string(), json!(29));
        let err = parse_wake_up_after_secs(&args).unwrap_err();
        assert!(err.contains("at least 30"), "got: {}", err);
    }

    #[test]
    fn wait_agents_rejects_wake_up_secs_above_1800() {
        let mut args = HashMap::new();
        args.insert("wake_up_after_secs".to_string(), json!(1801));
        let err = parse_wake_up_after_secs(&args).unwrap_err();
        assert!(err.contains("at most 1800"), "got: {}", err);
    }

    #[test]
    fn wait_agents_replaces_timed_wait_with_indefinite_wait() {
        let mut session = ChatSession::new("planner-timed-to-indefinite".to_string());
        replace_session_wake_up_at(&mut session, Some(120));
        assert!(session.wake_up_at.is_some());

        replace_session_wake_up_at(&mut session, None);

        assert_eq!(session.wake_up_at, None);
    }

    #[test]
    fn wait_agents_does_not_wait_for_offline_doing_cards() {
        let statuses = vec![offline_status("T-1")];
        let query = parse_agent_status_query(&HashMap::new()).unwrap();
        let waitable = waitable_agent_statuses(&statuses, &query, false);
        let ids = resolve_waiting_card_ids(&waitable);
        assert!(ids.is_empty());
    }

    #[test]
    fn wait_agents_does_not_wait_when_card_filter_matches_no_live_agent() {
        let statuses = vec![make_status("T-1", "doing")];
        let args = HashMap::from([("card_ids".to_string(), json!(["T-404"]))]);
        let query = parse_agent_status_query(&args).unwrap();
        let waitable = waitable_agent_statuses(&statuses, &query, false);
        let ids = resolve_waiting_card_ids(&waitable);
        assert!(ids.is_empty());
    }

    #[test]
    fn wait_agents_default_query_can_wait_for_paused_live_agent() {
        let mut status = make_status("T-1", "doing");
        status.session_state = Some(refact_runtime_api::SessionState::Paused);
        let statuses = vec![status];
        let query = parse_agent_status_query(&HashMap::new()).unwrap();
        let waitable = waitable_agent_statuses(&statuses, &query, false);
        let ids = resolve_waiting_card_ids(&waitable);
        assert_eq!(ids, vec!["T-1"]);
    }
}
