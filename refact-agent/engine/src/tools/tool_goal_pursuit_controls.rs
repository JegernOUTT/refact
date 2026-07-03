use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::internal_roles::{self, EventSubkind};
use crate::chat::types::{ChatSession, GoalStatus};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const GOAL_CONTROL_SOURCE: &str = "tool.goal_pursuit";
const MIN_SNOOZE_MINUTES: u64 = 1;
const MAX_SNOOZE_MINUTES: u64 = 24 * 60;

fn epoch_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn tool_message(tool_call_id: &str, content: String) -> Result<(bool, Vec<ContextEnum>), String> {
    Ok((
        false,
        vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(content),
            tool_call_id: tool_call_id.to_string(),
            ..Default::default()
        })],
    ))
}

fn purge_goal_generated_regenerates(session: &mut ChatSession) {
    session.command_queue.retain(|request| {
        !(matches!(
            request.command,
            crate::chat::types::ChatCommand::Regenerate {}
        ) && (request.client_request_id.starts_with("goal-nudge-")
            || request
                .client_request_id
                .starts_with("goal-verifier-regenerate-")))
    });
}

fn require_pursuable_goal(session: &ChatSession) -> Result<(), String> {
    let Some(goal) = session.goal.as_ref() else {
        return Err("no goal installed; call set_goal first".to_string());
    };
    if !goal.active {
        return Err("this chat does not own the goal".to_string());
    }
    if goal.status != GoalStatus::Active {
        return Err(format!(
            "goal is not actively pursued (status: {})",
            serde_json::to_value(goal.status)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default()
        ));
    }
    Ok(())
}

async fn session_for_chat(
    ccx: Arc<AMutex<AtCommandsContext>>,
) -> Result<Arc<AMutex<ChatSession>>, String> {
    let (gcx, chat_id) = {
        let ccx = ccx.lock().await;
        (ccx.app.gcx.clone(), ccx.chat_id.clone())
    };
    let sessions = gcx.chat_sessions.read().await;
    sessions
        .get(&chat_id)
        .cloned()
        .ok_or_else(|| format!("chat session `{chat_id}` not found"))
}

pub struct ToolPauseGoal {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolPauseGoal {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "pause_goal".to_string(),
            display_name: "Pause Goal".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Pause autonomous pursuit of the active goal when you are blocked or waiting on the user. Pursuit stays paused until the user resumes it (goal_control) — use snooze_goal instead for a timed wait.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "reason": {
                        "type": "string",
                        "description": "Short reason why pursuit is paused."
                    }
                },
                "required": [],
            }),
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
        let reason = args
            .get("reason")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let session_arc = session_for_chat(ccx).await?;
        let mut session = session_arc.lock().await;
        require_pursuable_goal(&session)?;
        purge_goal_generated_regenerates(&mut session);
        let ledger_reason = if reason.is_empty() {
            "pause_goal".to_string()
        } else {
            format!("pause_goal: {reason}")
        };
        session.goal_set_status_reason(GoalStatus::Paused, &ledger_reason);
        session.add_message(internal_roles::event(
            EventSubkind::GoalPursuit,
            GOAL_CONTROL_SOURCE,
            json!({"kind": "paused", "trigger": "pause_goal", "at_ms": epoch_ms_now()}),
            if reason.is_empty() {
                "Goal pursuit paused by the agent.".to_string()
            } else {
                format!("Goal pursuit paused by the agent: {reason}")
            },
        ));
        tool_message(
            tool_call_id,
            "Goal pursuit paused. The user can resume it with goal_control(resume).".to_string(),
        )
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

pub struct ToolSnoozeGoal {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolSnoozeGoal {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "snooze_goal".to_string(),
            display_name: "Snooze Goal".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Suspend goal-pursuit nudges for a bounded time (e.g. while waiting for CI or a background process). The goal stays active; pursuit resumes automatically after the snooze or on the next user message.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "minutes": {
                        "type": "integer",
                        "description": "How long to snooze pursuit, in minutes (1-1440)."
                    },
                    "reason": {
                        "type": "string",
                        "description": "Short reason for the snooze."
                    }
                },
                "required": ["minutes"],
            }),
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
        let minutes = args
            .get("minutes")
            .and_then(|value| value.as_u64())
            .ok_or("argument `minutes` must be a positive integer")?
            .clamp(MIN_SNOOZE_MINUTES, MAX_SNOOZE_MINUTES);
        let reason = args
            .get("reason")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let session_arc = session_for_chat(ccx).await?;
        let mut session = session_arc.lock().await;
        require_pursuable_goal(&session)?;
        let until_ms = epoch_ms_now().saturating_add(minutes.saturating_mul(60_000));
        purge_goal_generated_regenerates(&mut session);
        session.goal_set_snooze(Some(until_ms));
        session.add_message(internal_roles::event(
            EventSubkind::GoalPursuit,
            GOAL_CONTROL_SOURCE,
            json!({
                "kind": "snoozed",
                "trigger": "snooze_goal",
                "at_ms": epoch_ms_now(),
                "until_ms": until_ms,
            }),
            if reason.is_empty() {
                format!("Goal pursuit snoozed for {minutes} minutes.")
            } else {
                format!("Goal pursuit snoozed for {minutes} minutes: {reason}")
            },
        ));
        tool_message(
            tool_call_id,
            format!(
                "Goal pursuit snoozed for {minutes} minutes; it resumes automatically afterwards or on the next user message."
            ),
        )
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_chat_api::GoalBudget;

    fn session_with_goal() -> ChatSession {
        let mut session = ChatSession::new("goal-controls-test".to_string());
        session.install_goal("agent", "ship it", true, GoalBudget::default());
        session
    }

    #[test]
    fn require_pursuable_goal_rejects_missing_or_held_goals() {
        let session = ChatSession::new("no-goal".to_string());
        assert!(require_pursuable_goal(&session).is_err());

        let mut paused = session_with_goal();
        paused.goal_set_status(GoalStatus::Paused);
        assert!(require_pursuable_goal(&paused).is_err());

        let active = session_with_goal();
        assert!(require_pursuable_goal(&active).is_ok());
    }

    #[test]
    fn snooze_helper_sets_projection_and_survives_rebuild() {
        let mut session = session_with_goal();
        assert!(session.goal_set_snooze(Some(99_000)));
        assert_eq!(
            session.goal.as_ref().unwrap().snoozed_until_ms,
            Some(99_000)
        );

        session.rebuild_goal_projection_from_messages();
        assert_eq!(
            session.goal.as_ref().unwrap().snoozed_until_ms,
            Some(99_000)
        );

        assert!(session.goal_reset_no_progress() || session.goal.is_some());
        assert_eq!(session.goal.as_ref().unwrap().snoozed_until_ms, None);
    }

    #[test]
    fn purge_removes_goal_generated_regenerates_only() {
        let mut session = session_with_goal();
        session
            .command_queue
            .push_back(crate::chat::types::CommandRequest {
                client_request_id: "goal-nudge-x".to_string(),
                priority: true,
                command: crate::chat::types::ChatCommand::Regenerate {},
            });
        session
            .command_queue
            .push_back(crate::chat::types::CommandRequest {
                client_request_id: "user-x".to_string(),
                priority: false,
                command: crate::chat::types::ChatCommand::Regenerate {},
            });

        purge_goal_generated_regenerates(&mut session);

        assert_eq!(session.command_queue.len(), 1);
        assert_eq!(
            session.command_queue.front().unwrap().client_request_id,
            "user-x"
        );
    }
}
