use serde_json::Value;

use super::{CommandAction, CommandAvailability, CommandDef};
use crate::protocol::{TranscriptMessage, TranscriptRole};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowCommand {
    ShowPlan,
    ShowGoal,
    AgentMode,
    GitDiff,
    ReviewPrompt,
    CompactPrompt,
}

pub const PLAN_COMMAND: CommandDef = CommandDef {
    name: "plan",
    aliases: &[],
    description: "local plan cell: show the current hidden plan",
    args_hint: "",
    availability: CommandAvailability::Always,
    action: CommandAction::Workflow {
        command: WorkflowCommand::ShowPlan,
    },
};

pub const GOAL_COMMAND: CommandDef = CommandDef {
    name: "goal",
    aliases: &[],
    description: "local goal cell: show the current hidden goal",
    args_hint: "",
    availability: CommandAvailability::Always,
    action: CommandAction::Workflow {
        command: WorkflowCommand::ShowGoal,
    },
};

pub const AGENT_COMMAND: CommandDef = CommandDef {
    name: "agent",
    aliases: &[],
    description: "backend set_params: switch to Agent mode",
    args_hint: "",
    availability: CommandAvailability::IdleOnly,
    action: CommandAction::Workflow {
        command: WorkflowCommand::AgentMode,
    },
};

pub const DIFF_COMMAND: CommandDef = CommandDef {
    name: "diff",
    aliases: &[],
    description: "local git: render the project git diff",
    args_hint: "",
    availability: CommandAvailability::Always,
    action: CommandAction::Workflow {
        command: WorkflowCommand::GitDiff,
    },
};

pub const REVIEW_COMMAND: CommandDef = CommandDef {
    name: "review",
    aliases: &[],
    description: "structured prompt: request a code review",
    args_hint: "",
    availability: CommandAvailability::IdleOnly,
    action: CommandAction::Workflow {
        command: WorkflowCommand::ReviewPrompt,
    },
};

pub const COMPACT_COMMAND: CommandDef = CommandDef {
    name: "compact",
    aliases: &[],
    description: "structured prompt: trigger chat compaction fallback",
    args_hint: "",
    availability: CommandAvailability::IdleOnly,
    action: CommandAction::Workflow {
        command: WorkflowCommand::CompactPrompt,
    },
};

pub fn review_prompt() -> &'static str {
    "Review the current project changes. Inspect the git diff and relevant files, then report correctness, regression, safety, and test coverage findings by severity. Do not edit files unless I explicitly ask."
}

pub fn compact_prompt() -> &'static str {
    "Please compact this chat if needed. First use ctx_probe() or compress_chat_probe(); if compression is useful, apply the available ctx_apply()/compress_chat_apply() path while preserving the current goal, files, tests, decisions, and open blockers."
}

pub fn agent_mode_patch() -> Value {
    serde_json::json!({"mode": "agent", "tool_use": "agent"})
}

pub fn synthesize_current_plan(messages: &[TranscriptMessage]) -> Option<String> {
    let base = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| is_role(message, "plan"))
        .max_by_key(|(index, message)| (plan_version(message), *index))?
        .1
        .content
        .trim()
        .to_string();
    if base.is_empty() {
        return None;
    }
    let notes = messages
        .iter()
        .filter(|message| is_plan_delta(message))
        .filter_map(|message| {
            let note = message.content.trim();
            (!note.is_empty()).then(|| note.to_string())
        })
        .collect::<Vec<_>>();
    if notes.is_empty() {
        Some(base)
    } else {
        Some(format!(
            "{base}\n\n---\n\n## Plan updates\n\n{}",
            notes.join("\n\n")
        ))
    }
}

pub fn synthesize_current_goal(messages: &[TranscriptMessage]) -> Option<String> {
    let base = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| is_role(message, "goal"))
        .max_by_key(|(index, message)| (goal_version(message), *index))?
        .1
        .content
        .trim()
        .to_string();
    if base.is_empty() {
        return None;
    }
    let notes = messages
        .iter()
        .filter(|message| is_goal_delta(message))
        .filter_map(|message| {
            let note = message.content.trim();
            (!note.is_empty()).then(|| note.to_string())
        })
        .collect::<Vec<_>>();
    if notes.is_empty() {
        Some(base)
    } else {
        Some(format!(
            "{base}\n\n---\n\n## Goal updates\n\n{}",
            notes.join("\n\n")
        ))
    }
}

fn plan_version(message: &TranscriptMessage) -> u64 {
    metadata_field(message, "plan")
        .and_then(|plan| plan.get("version"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn goal_version(message: &TranscriptMessage) -> u64 {
    metadata_field(message, "goal")
        .and_then(|goal| goal.get("version"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn is_role(message: &TranscriptMessage, role: &str) -> bool {
    match (&message.role, role) {
        (TranscriptRole::User, "user")
        | (TranscriptRole::Assistant, "assistant")
        | (TranscriptRole::Tool, "tool")
        | (TranscriptRole::Notice, "notice")
        | (TranscriptRole::Plan, "plan")
        | (TranscriptRole::Goal, "goal")
        | (TranscriptRole::Event, "event") => true,
        (TranscriptRole::Other(value), _) => value == role,
        _ => false,
    }
}

fn is_plan_delta(message: &TranscriptMessage) -> bool {
    if !is_role(message, "event") {
        return false;
    }
    metadata_field(message, "event")
        .and_then(|event| event.get("subkind"))
        .and_then(Value::as_str)
        == Some("plan_delta")
}

fn is_goal_delta(message: &TranscriptMessage) -> bool {
    if !is_role(message, "event") {
        return false;
    }
    metadata_field(message, "event")
        .and_then(|event| event.get("subkind"))
        .and_then(Value::as_str)
        == Some("goal_delta")
}

fn metadata_field<'a>(message: &'a TranscriptMessage, key: &str) -> Option<&'a Value> {
    message
        .extra
        .get(key)
        .or_else(|| message.extra.get("extra").and_then(|extra| extra.get(key)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::TranscriptMessage;
    use serde_json::json;

    #[test]
    fn synthesizes_base_plan_and_deltas() {
        let messages = vec![
            TranscriptMessage::from_wire(&json!({
                "role": "plan",
                "content": "base plan",
                "extra": {"plan": {"mode": "agent", "version": 1}}
            })),
            TranscriptMessage::from_wire(&json!({
                "role": "event",
                "content": "delta one",
                "extra": {"event": {"subkind": "plan_delta", "payload": {"seq": 1}}}
            })),
            TranscriptMessage::from_wire(&json!({
                "role": "event",
                "content": "ignored",
                "extra": {"event": {"subkind": "system_notice"}}
            })),
        ];

        assert_eq!(
            synthesize_current_plan(&messages).unwrap(),
            "base plan\n\n---\n\n## Plan updates\n\ndelta one"
        );
    }

    #[test]
    fn current_plan_uses_highest_version_then_latest_index() {
        let messages = vec![
            TranscriptMessage::from_wire(&json!({
                "role": "plan",
                "content": "latest index",
                "extra": {"plan": {"version": 1}}
            })),
            TranscriptMessage::from_wire(&json!({
                "role": "plan",
                "content": "highest version",
                "extra": {"plan": {"version": 2}}
            })),
        ];

        assert_eq!(
            synthesize_current_plan(&messages).unwrap(),
            "highest version"
        );
    }

    #[test]
    fn synthesizes_base_goal_and_deltas() {
        let messages = vec![
            TranscriptMessage::from_wire(&json!({
                "role": "goal",
                "content": "base goal",
                "extra": {"goal": {"version": 1}}
            })),
            TranscriptMessage::from_wire(&json!({
                "role": "event",
                "content": "goal delta one",
                "extra": {"event": {"subkind": "goal_delta", "payload": {"seq": 1}}}
            })),
            TranscriptMessage::from_wire(&json!({
                "role": "event",
                "content": "ignored",
                "extra": {"event": {"subkind": "plan_delta"}}
            })),
        ];

        assert_eq!(
            synthesize_current_goal(&messages).unwrap(),
            "base goal\n\n---\n\n## Goal updates\n\ngoal delta one"
        );
    }
}
