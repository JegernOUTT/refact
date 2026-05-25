use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::tools::task_tool_helpers::require_bound_planner_task;
use crate::tools::tool_task_check_agents::{
    agent_status_input_schema, format_agent_statuses, get_agent_statuses,
    has_active_agent_statuses, parse_agent_status_query,
};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const WAKE_UP_AFTER_SECS_MIN: u64 = 30;
const WAKE_UP_AFTER_SECS_MAX: u64 = 1800;

fn wait_for_agents_input_schema() -> Value {
    let mut schema = agent_status_input_schema();
    if let Some(props) = schema
        .get_mut("properties")
        .and_then(|p| p.as_object_mut())
    {
        props.insert(
            "wake_up_after_secs".to_string(),
            json!({
                "type": "integer",
                "minimum": 30,
                "maximum": 1800,
                "description": "If set, the backend will wake the planner after this many seconds (30-1800) if no agent reports back. Recommended for long-running cards."
            }),
        );
    }
    schema
}

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

pub struct ToolTaskWaitForAgents;

impl ToolTaskWaitForAgents {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskWaitForAgents {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "task_wait_for_agents".to_string(),
            display_name: "Task Wait For Agents".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Check spawned task agents using the same compact status view as task_check_agents, then stop the planner turn so it waits for agent completion messages. Pass wake_up_after_secs to set an auto-wake timer (30-1800 s).".to_string(),
            input_schema: wait_for_agents_input_schema(),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
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
                "task_wait_for_agents can only be called by the task planner. \
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

        if statuses.is_empty() {
            result.push_str("\nNo agents are currently running.\n");
        } else if has_active_agent_statuses(&statuses) {
            result.push_str("\n⏳ **Agents are still working.** Do not check again, wait for the completion message to arrive.\n");
        } else {
            result.push_str("\nNo agents are currently running.\n");
        }

        if let Some(secs) = wake_up_secs {
            let wake_at = Utc::now() + chrono::Duration::seconds(secs as i64);
            let sessions = app.chat.sessions.read().await;
            if let Some(session_arc) = sessions.get(&chat_id) {
                let mut session = session_arc.lock().await;
                session.wake_up_at = Some(wake_at);
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
                content: ChatContent::SimpleText(result),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;
    use std::sync::Arc;

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

    #[tokio::test]
    async fn wait_agents_records_wake_up_at_on_session_when_argument_present() {
        use crate::app_state::AppState;
        use crate::chat::types::ChatSession;

        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx.clone()).await;

        let chat_id = "planner-wake-test".to_string();
        let session = ChatSession::new(chat_id.clone());
        let session_arc = Arc::new(tokio::sync::Mutex::new(session));
        app.chat
            .sessions
            .write()
            .await
            .insert(chat_id.clone(), session_arc.clone());

        let secs = 120u64;
        let before = Utc::now();
        let args: HashMap<String, Value> =
            [("wake_up_after_secs".to_string(), json!(secs))]
                .into_iter()
                .collect();

        let wake_up_secs = parse_wake_up_after_secs(&args).unwrap();
        assert_eq!(wake_up_secs, Some(secs));

        let wake_at = Utc::now() + chrono::Duration::seconds(secs as i64);
        {
            let sessions = app.chat.sessions.read().await;
            if let Some(sa) = sessions.get(&chat_id) {
                let mut s = sa.lock().await;
                s.wake_up_at = Some(wake_at);
                s.mark_persisted_runtime_changed();
            }
        }

        let session = session_arc.lock().await;
        let stored = session.wake_up_at.unwrap();
        let expected = before + chrono::Duration::seconds(secs as i64);
        let diff = (stored - expected).num_seconds().abs();
        assert!(diff <= 2, "wake_up_at set within ±2s, diff={}", diff);
        assert!(session.trajectory_dirty);
    }

    #[test]
    fn wait_agents_does_not_set_wake_up_at_when_argument_absent() {
        let args: HashMap<String, Value> = HashMap::new();
        let result = parse_wake_up_after_secs(&args).unwrap();
        assert_eq!(result, None);
    }
}
