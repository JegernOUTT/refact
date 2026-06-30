use serde_json::{json, Value};

use super::{CommandAction, CommandAvailability, CommandDef};
use crate::pickers::PickerItem;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionCommand {
    New,
    Resume,
    Fork,
    Rename,
    Archive,
    Model,
    Mode,
    Reasoning,
    Permissions,
    Status,
    Init,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningLevel {
    Off,
    On,
    Low,
    Medium,
    High,
}

impl ReasoningLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::On => "On",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }
}

pub const REASONING_LEVELS: [ReasoningLevel; 5] = [
    ReasoningLevel::Off,
    ReasoningLevel::On,
    ReasoningLevel::Low,
    ReasoningLevel::Medium,
    ReasoningLevel::High,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSessionCommand {
    pub command: SessionCommand,
    pub args: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct PermissionPolicy {
    pub auto_approve_editing_tools: bool,
    pub auto_approve_dangerous_commands: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StatusUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub context_window_tokens: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StatusSnapshot {
    pub daemon_online: bool,
    pub daemon_version: Option<String>,
    pub daemon_port: Option<u16>,
    pub daemon_base_url: Option<String>,
    pub worker: String,
    pub project: String,
    pub project_root: Option<String>,
    pub model: String,
    pub mode: String,
    pub reasoning: String,
    pub permission_policy: PermissionPolicy,
    pub session_id: String,
    pub usage: Option<StatusUsage>,
    pub retry_hint: Option<String>,
}

pub const NEW_COMMAND: CommandDef = CommandDef {
    name: "new",
    aliases: &[],
    description: "session: start a new chat",
    args_hint: "",
    availability: CommandAvailability::Always,
    action: CommandAction::Session {
        command: SessionCommand::New,
    },
};

pub const RESUME_COMMAND: CommandDef = CommandDef {
    name: "resume",
    aliases: &["sessions", "history"],
    description: "session picker: open recent project chats",
    args_hint: "",
    availability: CommandAvailability::Always,
    action: CommandAction::Session {
        command: SessionCommand::Resume,
    },
};

pub const FORK_COMMAND: CommandDef = CommandDef {
    name: "fork",
    aliases: &["branch"],
    description: "session: branch the current chat into a new chat",
    args_hint: "",
    availability: CommandAvailability::IdleOnly,
    action: CommandAction::Session {
        command: SessionCommand::Fork,
    },
};

pub const RENAME_COMMAND: CommandDef = CommandDef {
    name: "rename",
    aliases: &["title"],
    description: "session: rename the current chat",
    args_hint: "<title>",
    availability: CommandAvailability::Always,
    action: CommandAction::Session {
        command: SessionCommand::Rename,
    },
};

pub const ARCHIVE_COMMAND: CommandDef = CommandDef {
    name: "archive",
    aliases: &["remove"],
    description: "session: remove the current chat from recent sessions",
    args_hint: "",
    availability: CommandAvailability::IdleOnly,
    action: CommandAction::Session {
        command: SessionCommand::Archive,
    },
};

pub const MODEL_COMMAND: CommandDef = CommandDef {
    name: "model",
    aliases: &[],
    description: "model picker: choose the model for the next message",
    args_hint: "",
    availability: CommandAvailability::Always,
    action: CommandAction::Session {
        command: SessionCommand::Model,
    },
};

pub const MODE_COMMAND: CommandDef = CommandDef {
    name: "mode",
    aliases: &["tool-use"],
    description: "mode picker: choose the chat mode for the next message",
    args_hint: "",
    availability: CommandAvailability::Always,
    action: CommandAction::Session {
        command: SessionCommand::Mode,
    },
};

pub const REASONING_COMMAND: CommandDef = CommandDef {
    name: "reasoning",
    aliases: &[],
    description: "session params: set reasoning effort for subsequent turns",
    args_hint: "<off|on|low|medium|high>",
    availability: CommandAvailability::IdleOnly,
    action: CommandAction::Session {
        command: SessionCommand::Reasoning,
    },
};

pub const PERMISSIONS_COMMAND: CommandDef = CommandDef {
    name: "permissions",
    aliases: &["approval"],
    description: "permissions picker: set per-chat tool auto-approve policy",
    args_hint: "",
    availability: CommandAvailability::Always,
    action: CommandAction::Session {
        command: SessionCommand::Permissions,
    },
};

pub const STATUS_COMMAND: CommandDef = CommandDef {
    name: "status",
    aliases: &[],
    description: "info: show daemon, worker, session, and usage status",
    args_hint: "",
    availability: CommandAvailability::Always,
    action: CommandAction::Session {
        command: SessionCommand::Status,
    },
};

pub const INIT_COMMAND: CommandDef = CommandDef {
    name: "init",
    aliases: &[],
    description: "structured prompt: bootstrap project instructions",
    args_hint: "",
    availability: CommandAvailability::IdleOnly,
    action: CommandAction::Session {
        command: SessionCommand::Init,
    },
};

pub const SESSION_COMMANDS: [CommandDef; 11] = [
    NEW_COMMAND,
    RESUME_COMMAND,
    FORK_COMMAND,
    RENAME_COMMAND,
    ARCHIVE_COMMAND,
    MODEL_COMMAND,
    MODE_COMMAND,
    REASONING_COMMAND,
    PERMISSIONS_COMMAND,
    STATUS_COMMAND,
    INIT_COMMAND,
];

pub fn parse_session_command(input: &str) -> Option<ParsedSessionCommand> {
    let input = input.trim().trim_start_matches('/').trim_start();
    let (name, args) = match input.find(char::is_whitespace) {
        Some(index) => {
            let (name, args) = input.split_at(index);
            (name, args.trim())
        }
        None => (input, ""),
    };
    SESSION_COMMANDS
        .iter()
        .find(|command| command.matches_name(name))
        .and_then(|command| match command.action {
            CommandAction::Session { command } => Some(ParsedSessionCommand {
                command,
                args: args.to_string(),
            }),
            _ => None,
        })
}

pub fn init_prompt() -> &'static str {
    "Please bootstrap this project for future coding sessions. Inspect the repository structure, identify the main build/test commands, summarize the coding conventions, and create or update an AGENTS.md file with concise project-specific instructions if one is missing or stale. Keep changes minimal and explain what you changed."
}

pub fn parse_reasoning_level(args: &str) -> Result<Option<ReasoningLevel>, String> {
    let arg = args.trim().to_ascii_lowercase();
    if arg.is_empty() {
        return Ok(None);
    }
    match arg.as_str() {
        "off" | "none" => Ok(Some(ReasoningLevel::Off)),
        "on" | "boost" | "thinking" => Ok(Some(ReasoningLevel::On)),
        "low" => Ok(Some(ReasoningLevel::Low)),
        "medium" => Ok(Some(ReasoningLevel::Medium)),
        "high" => Ok(Some(ReasoningLevel::High)),
        _ => Err(format!(
            "expected one of: off, on, low, medium, high; got {args}"
        )),
    }
}

pub fn reasoning_picker_items(levels: &[ReasoningLevel]) -> Vec<PickerItem> {
    levels
        .iter()
        .copied()
        .map(|level| PickerItem {
            id: level.as_str().to_string(),
            title: level.title().to_string(),
            description: reasoning_level_description(level).to_string(),
        })
        .collect()
}

pub fn reasoning_patch(level: ReasoningLevel) -> Value {
    match level {
        ReasoningLevel::Off => json!({
            "boost_reasoning": false,
            "reasoning_effort": null,
            "thinking_budget": null,
        }),
        ReasoningLevel::On => json!({
            "boost_reasoning": true,
            "reasoning_effort": null,
            "thinking_budget": null,
        }),
        _ => json!({
            "boost_reasoning": true,
            "reasoning_effort": level.as_str(),
            "thinking_budget": null,
        }),
    }
}

fn reasoning_level_description(level: ReasoningLevel) -> &'static str {
    match level {
        ReasoningLevel::Off => "Disable reasoning for subsequent turns",
        ReasoningLevel::On => "Enable model-native reasoning boost",
        ReasoningLevel::Low => "Use low reasoning effort",
        ReasoningLevel::Medium => "Use medium reasoning effort",
        ReasoningLevel::High => "Use high reasoning effort",
    }
}

pub fn permission_picker_items() -> Vec<PickerItem> {
    vec![
        PickerItem {
            id: "editing_tools".to_string(),
            title: "Allow editing tools for this chat".to_string(),
            description: "Server flag auto_approve_editing_tools; equivalent to Allow Chat for file edits".to_string(),
        },
        PickerItem {
            id: "dangerous_commands".to_string(),
            title: "Allow dangerous commands for this chat".to_string(),
            description: "Server flag auto_approve_dangerous_commands; shell/destructive tools still require explicit policy".to_string(),
        },
    ]
}

pub fn selected_permission_ids(policy: PermissionPolicy) -> Vec<String> {
    let mut selected = Vec::new();
    if policy.auto_approve_editing_tools {
        selected.push("editing_tools".to_string());
    }
    if policy.auto_approve_dangerous_commands {
        selected.push("dangerous_commands".to_string());
    }
    selected
}

pub fn permission_policy_from_items(items: &[PickerItem]) -> PermissionPolicy {
    PermissionPolicy {
        auto_approve_editing_tools: items.iter().any(|item| item.id == "editing_tools"),
        auto_approve_dangerous_commands: items.iter().any(|item| item.id == "dangerous_commands"),
    }
}

pub fn permission_policy_patch(policy: PermissionPolicy) -> Value {
    json!({
        "auto_approve_editing_tools": policy.auto_approve_editing_tools,
        "auto_approve_dangerous_commands": policy.auto_approve_dangerous_commands,
    })
}

pub fn permission_policy_notice(policy: PermissionPolicy) -> String {
    let mut allowed = Vec::new();
    if policy.auto_approve_editing_tools {
        allowed.push("editing tools");
    }
    if policy.auto_approve_dangerous_commands {
        allowed.push("dangerous commands");
    }
    let policy = if allowed.is_empty() {
        "ask before every tool class".to_string()
    } else {
        format!("allow {} for this chat", allowed.join(" and "))
    };
    format!(
        "Permissions updated: {policy}. TUI sends Allow Once decisions only for the current pause; the server enforces these per-chat Allow Chat flags."
    )
}

pub fn status_card_text(snapshot: &StatusSnapshot) -> String {
    format!(
        "Status\nDaemon: {}\nWorker: {}\nProject: {}\nModel: {} · mode {} · reason:{}\nSession: {}\nUsage: {}",
        daemon_line(snapshot),
        snapshot.worker,
        project_line(snapshot),
        snapshot.model,
        snapshot.mode,
        snapshot.reasoning,
        short_session_id(&snapshot.session_id),
        usage_line(snapshot.usage.as_ref())
    )
}

fn daemon_line(snapshot: &StatusSnapshot) -> String {
    if !snapshot.daemon_online {
        return "offline".to_string();
    }
    match (&snapshot.daemon_version, snapshot.daemon_port) {
        (Some(version), Some(port)) => format!("v{version} on port {port}"),
        (Some(version), None) => format!("v{version}"),
        (None, Some(port)) => format!("online on port {port}"),
        (None, None) => snapshot
            .daemon_base_url
            .as_ref()
            .map(|url| format!("online at {url}"))
            .unwrap_or_else(|| "online, details loading".to_string()),
    }
}

fn project_line(snapshot: &StatusSnapshot) -> String {
    match &snapshot.project_root {
        Some(root) if !root.is_empty() => format!("{} ({root})", snapshot.project),
        _ => snapshot.project.clone(),
    }
}

fn usage_line(usage: Option<&StatusUsage>) -> String {
    let Some(usage) = usage else {
        return "not reported".to_string();
    };
    let base = format!(
        "{} prompt + {} completion = {} total tokens",
        usage.prompt_tokens, usage.completion_tokens, usage.total_tokens
    );
    match usage.context_window_tokens.filter(|tokens| *tokens > 0) {
        Some(window) => format!(
            "{base}; {}% context left",
            context_left_percent(usage.total_tokens, window)
        ),
        None => base,
    }
}

fn context_left_percent(used: u64, window: u64) -> u64 {
    let remaining = window.saturating_sub(used);
    (((remaining as u128 * 100) + (window as u128 / 2)) / window as u128) as u64
}

fn short_session_id(chat_id: &str) -> String {
    chat_id.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_session_command_and_alias() {
        let cases = [
            ("/new", SessionCommand::New, ""),
            ("/sessions", SessionCommand::Resume, ""),
            ("/branch", SessionCommand::Fork, ""),
            (
                "/rename Better title",
                SessionCommand::Rename,
                "Better title",
            ),
            ("/remove", SessionCommand::Archive, ""),
            ("/model", SessionCommand::Model, ""),
            ("/tool-use", SessionCommand::Mode, ""),
            ("/reasoning high", SessionCommand::Reasoning, "high"),
            ("/approval", SessionCommand::Permissions, ""),
            ("/status", SessionCommand::Status, ""),
            ("/init", SessionCommand::Init, ""),
        ];
        for (input, command, args) in cases {
            let parsed = parse_session_command(input).unwrap();
            assert_eq!(parsed.command, command);
            assert_eq!(parsed.args, args);
        }
    }

    #[test]
    fn permission_policy_patch_maps_supported_server_flags() {
        let policy = permission_policy_from_items(&[PickerItem {
            id: "editing_tools".to_string(),
            title: String::new(),
            description: String::new(),
        }]);
        assert_eq!(
            permission_policy_patch(policy),
            json!({"auto_approve_editing_tools": true, "auto_approve_dangerous_commands": false})
        );
        assert_eq!(selected_permission_ids(policy), vec!["editing_tools"]);
    }

    #[test]
    fn reasoning_patch_maps_effort_and_off_to_setparams() {
        assert_eq!(
            parse_reasoning_level("high"),
            Ok(Some(ReasoningLevel::High))
        );
        assert_eq!(parse_reasoning_level("none"), Ok(Some(ReasoningLevel::Off)));
        assert_eq!(parse_reasoning_level("boost"), Ok(Some(ReasoningLevel::On)));
        assert!(parse_reasoning_level("turbo").is_err());
        assert_eq!(
            reasoning_patch(ReasoningLevel::High),
            json!({"boost_reasoning": true, "reasoning_effort": "high", "thinking_budget": null})
        );
        assert_eq!(
            reasoning_patch(ReasoningLevel::On),
            json!({"boost_reasoning": true, "reasoning_effort": null, "thinking_budget": null})
        );
        assert_eq!(
            reasoning_patch(ReasoningLevel::Off),
            json!({"boost_reasoning": false, "reasoning_effort": null, "thinking_budget": null})
        );
    }

    #[test]
    fn status_snapshot_formats_daemon_worker_session_and_usage() {
        let text = status_card_text(&StatusSnapshot {
            daemon_online: true,
            daemon_version: Some("1.2.3".to_string()),
            daemon_port: Some(8488),
            daemon_base_url: Some("http://127.0.0.1:8488".to_string()),
            worker: "ready pid 42 http 9000 lsp 9001".to_string(),
            project: "demo".to_string(),
            project_root: Some("/tmp/demo".to_string()),
            model: "gpt-demo".to_string(),
            mode: "agent".to_string(),
            reasoning: "high".to_string(),
            permission_policy: PermissionPolicy {
                auto_approve_editing_tools: true,
                auto_approve_dangerous_commands: false,
            },
            session_id: "abcdef123456".to_string(),
            usage: Some(StatusUsage {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
                context_window_tokens: Some(1000),
            }),
            retry_hint: None,
        });
        assert_eq!(
            text,
            "Status\nDaemon: v1.2.3 on port 8488\nWorker: ready pid 42 http 9000 lsp 9001\nProject: demo (/tmp/demo)\nModel: gpt-demo · mode agent · reason:high\nSession: abcdef12\nUsage: 100 prompt + 50 completion = 150 total tokens; 85% context left"
        );
    }
}
