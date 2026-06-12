pub mod workflow;

use crate::pickers::PickerItem;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandAvailability {
    Always,
    IdleOnly,
    ActiveTurnOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandPicker {
    Model,
    Mode,
    FileMention,
    Permissions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalToggle {
    NewChat,
    ClearTranscript,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoTopic {
    Help,
    Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandAction {
    SendPrompt { prompt: &'static str },
    BackendCommand { command: &'static str },
    OpenPicker { picker: CommandPicker },
    LocalToggle { toggle: LocalToggle },
    ShowInfo { topic: InfoTopic },
    Workflow { command: workflow::WorkflowCommand },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandDef {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub args_hint: &'static str,
    pub availability: CommandAvailability,
    pub action: CommandAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandContext {
    pub active_turn: bool,
}

impl CommandDef {
    pub fn available(self, context: CommandContext) -> bool {
        match self.availability {
            CommandAvailability::Always => true,
            CommandAvailability::IdleOnly => !context.active_turn,
            CommandAvailability::ActiveTurnOnly => context.active_turn,
        }
    }

    pub fn matches_name(self, name: &str) -> bool {
        let name = name.trim_start_matches('/');
        self.name == name || self.aliases.iter().any(|alias| *alias == name)
    }

    pub fn picker_item(self) -> PickerItem {
        let description = if self.aliases.is_empty() {
            self.description.to_string()
        } else {
            format!(
                "{} · aliases {}",
                self.description,
                self.aliases.join(", /")
            )
        };
        PickerItem {
            id: self.name.to_string(),
            title: format!("/{}", self.name),
            description,
        }
    }
}

pub fn command_registry() -> &'static [CommandDef] {
    &COMMANDS
}

pub fn command_by_name(name: &str) -> Option<&'static CommandDef> {
    COMMANDS.iter().find(|command| command.matches_name(name))
}

pub fn command_picker_items(context: CommandContext) -> Vec<PickerItem> {
    COMMANDS
        .iter()
        .filter(|command| command.available(context))
        .map(|command| command.picker_item())
        .collect()
}

const COMMANDS: [CommandDef; 18] = [
    CommandDef {
        name: "new",
        aliases: &[],
        description: "Start a new chat",
        args_hint: "",
        availability: CommandAvailability::Always,
        action: CommandAction::LocalToggle {
            toggle: LocalToggle::NewChat,
        },
    },
    CommandDef {
        name: "clear",
        aliases: &[],
        description: "Clear the local transcript view",
        args_hint: "",
        availability: CommandAvailability::Always,
        action: CommandAction::LocalToggle {
            toggle: LocalToggle::ClearTranscript,
        },
    },
    CommandDef {
        name: "quit",
        aliases: &["exit"],
        description: "Exit the TUI",
        args_hint: "",
        availability: CommandAvailability::Always,
        action: CommandAction::LocalToggle {
            toggle: LocalToggle::Quit,
        },
    },
    CommandDef {
        name: "model",
        aliases: &[],
        description: "Choose the model for the next message",
        args_hint: "",
        availability: CommandAvailability::Always,
        action: CommandAction::OpenPicker {
            picker: CommandPicker::Model,
        },
    },
    CommandDef {
        name: "mode",
        aliases: &["tool-use"],
        description: "Choose the chat mode for the next message",
        args_hint: "",
        availability: CommandAvailability::Always,
        action: CommandAction::OpenPicker {
            picker: CommandPicker::Mode,
        },
    },
    CommandDef {
        name: "mention",
        aliases: &["file", "files"],
        description: "picker reuse: insert a file mention",
        args_hint: "[path]",
        availability: CommandAvailability::Always,
        action: CommandAction::OpenPicker {
            picker: CommandPicker::FileMention,
        },
    },
    CommandDef {
        name: "help",
        aliases: &["?"],
        description: "Show TUI help",
        args_hint: "",
        availability: CommandAvailability::Always,
        action: CommandAction::ShowInfo {
            topic: InfoTopic::Help,
        },
    },
    CommandDef {
        name: "status",
        aliases: &[],
        description: "Show current session status",
        args_hint: "",
        availability: CommandAvailability::Always,
        action: CommandAction::ShowInfo {
            topic: InfoTopic::Status,
        },
    },
    CommandDef {
        name: "stop",
        aliases: &["cancel"],
        description: "backend command: stop the active generation",
        args_hint: "",
        availability: CommandAvailability::ActiveTurnOnly,
        action: CommandAction::BackendCommand { command: "stop" },
    },
    workflow::REVIEW_COMMAND,
    workflow::PLAN_COMMAND,
    workflow::GOAL_COMMAND,
    workflow::AGENT_COMMAND,
    workflow::DIFF_COMMAND,
    workflow::COMPACT_COMMAND,
    CommandDef {
        name: "copy",
        aliases: &[],
        description: "Copy the last response",
        args_hint: "",
        availability: CommandAvailability::Always,
        action: CommandAction::BackendCommand { command: "copy" },
    },
    CommandDef {
        name: "raw",
        aliases: &[],
        description: "Toggle raw transcript display",
        args_hint: "",
        availability: CommandAvailability::Always,
        action: CommandAction::BackendCommand { command: "raw" },
    },
    CommandDef {
        name: "permissions",
        aliases: &["approval"],
        description: "Open permissions controls",
        args_hint: "",
        availability: CommandAvailability::Always,
        action: CommandAction::OpenPicker {
            picker: CommandPicker::Permissions,
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_registry_resolves_aliases() {
        let command = command_by_name("/exit").unwrap();
        assert_eq!(command.name, "quit");
    }

    #[test]
    fn active_only_commands_hide_when_idle() {
        let idle = command_picker_items(CommandContext { active_turn: false });
        assert!(!idle.iter().any(|item| item.id == "stop"));
        let active = command_picker_items(CommandContext { active_turn: true });
        assert!(active.iter().any(|item| item.id == "stop"));
    }

    #[test]
    fn command_picker_items_use_registry_rows() {
        let items = command_picker_items(CommandContext { active_turn: false });
        assert!(items.iter().any(|item| item.title == "/mention"));
        assert!(items
            .iter()
            .any(|item| item.description.contains("aliases exit")));
    }

    #[test]
    fn workflow_commands_document_mechanism() {
        for name in [
            "plan", "goal", "agent", "diff", "review", "mention", "compact",
        ] {
            let command = command_by_name(name).unwrap();
            assert!(
                command.description.contains(':') || command.description.contains("Insert"),
                "{name} must document its mechanism"
            );
        }
    }
}
