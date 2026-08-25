pub mod misc;
pub mod session;
pub mod workflow;

use std::sync::OnceLock;

use crate::pickers::PickerItem;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandAvailability {
    Always,
    IdleOnly,
    ActiveTurnOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandPicker {
    FileMention,
    Theme,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalToggle {
    ClearTranscript,
    Events,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoTopic {
    Help,
    Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandAction {
    BackendCommand { command: &'static str },
    OpenPicker { picker: CommandPicker },
    LocalToggle { toggle: LocalToggle },
    ShowInfo { topic: InfoTopic },
    Session { command: session::SessionCommand },
    Workflow { command: workflow::WorkflowCommand },
    Misc { command: misc::MiscCommand },
    Unavailable { reason: &'static str },
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

const MENTION_COMMAND: CommandDef = CommandDef {
    name: "mention",
    aliases: &["file", "files"],
    description: "picker reuse: insert a file mention",
    args_hint: "[path]",
    availability: CommandAvailability::Always,
    action: CommandAction::OpenPicker {
        picker: CommandPicker::FileMention,
    },
};

const STOP_COMMAND: CommandDef = CommandDef {
    name: "stop",
    aliases: &["cancel", "clean"],
    description: "backend command: stop the active generation",
    args_hint: "",
    availability: CommandAvailability::ActiveTurnOnly,
    action: CommandAction::BackendCommand { command: "stop" },
};

const COMMAND_GROUPS: &[&[CommandDef]] = &[
    &session::SESSION_COMMANDS,
    misc::MISC_COMMANDS,
    &[MENTION_COMMAND, STOP_COMMAND],
    workflow::WORKFLOW_COMMANDS,
    misc::UNAVAILABLE_COMMANDS,
];

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
    static COMMANDS: OnceLock<Vec<CommandDef>> = OnceLock::new();
    COMMANDS.get_or_init(|| {
        COMMAND_GROUPS
            .iter()
            .flat_map(|group| group.iter())
            .copied()
            .collect()
    })
}

pub fn command_by_name(name: &str) -> Option<&'static CommandDef> {
    command_registry()
        .iter()
        .find(|command| command.matches_name(name))
}

pub fn command_picker_items(context: CommandContext) -> Vec<PickerItem> {
    command_registry()
        .iter()
        .filter(|command| command.available(context))
        .map(|command| command.picker_item())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_registry_resolves_aliases() {
        let command = command_by_name("/exit").unwrap();
        assert_eq!(command.name, "quit");
    }

    #[test]
    fn board_command_is_available_with_tasks_alias() {
        let board = command_by_name("board").expect("board command");
        let tasks = command_by_name("tasks").expect("tasks alias");
        assert_eq!(board.name, "board");
        assert_eq!(tasks.name, "board");
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
        assert!(items.iter().any(|item| item.title == "/resume"));
        assert!(items
            .iter()
            .any(|item| item.description.contains("aliases exit")));
    }

    #[test]
    fn session_command_group_is_visible_in_popup() {
        let items = command_picker_items(CommandContext { active_turn: false });
        for title in [
            "/new",
            "/resume",
            "/fork",
            "/rename",
            "/archive",
            "/model",
            "/mode",
            "/reasoning",
            "/permissions",
            "/status",
            "/init",
        ] {
            assert!(
                items.iter().any(|item| item.title == title),
                "missing {title}"
            );
        }
    }

    #[test]
    fn goal_command_wires_to_show_goal() {
        let command = command_by_name("goal").unwrap();

        assert_eq!(
            command.action,
            CommandAction::Workflow {
                command: workflow::WorkflowCommand::ShowGoal,
            }
        );
    }

    #[test]
    fn misc_command_group_is_visible_in_popup() {
        let items = command_picker_items(CommandContext { active_turn: false });
        for title in [
            "/theme",
            "/help",
            "/events",
            "/quit",
            "/keymap",
            "/vim",
            "/copy",
            "/raw",
            "/subagents",
            "/mcp",
            "/skills",
            "/memories",
            "/hooks",
            "/logout",
            "/import",
            "/browser",
            "/worktrees",
        ] {
            assert!(
                items.iter().any(|item| item.title == title),
                "missing {title}"
            );
        }
    }

    #[test]
    fn registry_has_no_duplicate_names_or_aliases() {
        let commands = command_registry();
        for (index, command) in commands.iter().enumerate() {
            assert!(!command.name.is_empty());
            for other in commands.iter().skip(index + 1) {
                assert_ne!(command.name, other.name, "duplicate command name");
                assert!(
                    !command.aliases.iter().any(|alias| *alias == other.name),
                    "alias /{} conflicts with command /{}",
                    command.name,
                    other.name
                );
                assert!(
                    !other.aliases.iter().any(|alias| *alias == command.name),
                    "alias /{} conflicts with command /{}",
                    other.name,
                    command.name
                );
                for alias in command.aliases {
                    assert!(
                        !other.aliases.iter().any(|other_alias| alias == other_alias),
                        "duplicate alias /{alias}"
                    );
                }
            }
        }
    }

    #[test]
    fn registry_has_handler_or_explicit_unavailable_reason() {
        for command in command_registry() {
            match command.action {
                CommandAction::BackendCommand { command: "stop" } => {}
                CommandAction::BackendCommand { command: other } => {
                    panic!(
                        "/{} maps to unexpected backend command /{other}",
                        command.name
                    );
                }
                CommandAction::Unavailable { reason } => assert!(!reason.trim().is_empty()),
                CommandAction::OpenPicker { .. }
                | CommandAction::LocalToggle { .. }
                | CommandAction::ShowInfo { .. }
                | CommandAction::Session { .. }
                | CommandAction::Workflow { .. }
                | CommandAction::Misc { .. } => {}
            }
        }
    }

    #[test]
    fn workflow_commands_document_mechanism() {
        for name in [
            "plan", "goal", "agent", "diff", "review", "mention", "compact", "init",
        ] {
            let command = command_by_name(name).unwrap();
            assert!(
                command.description.contains(':') || command.description.contains("Insert"),
                "{name} must document its mechanism"
            );
        }
    }
}
