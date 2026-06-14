use super::{CommandAction, CommandAvailability, CommandDef, InfoTopic, LocalToggle};
use crate::pickers::PickerItem;
use crate::theme::TuiTheme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MiscCommand {
    Theme,
    ToggleVim,
    DebugConfig,
    CopyLastAssistant,
    RawTranscript,
    Subagents,
}

pub const CLEAR_COMMAND: CommandDef = CommandDef {
    name: "clear",
    aliases: &[],
    description: "local toggle: clear the local transcript view",
    args_hint: "",
    availability: CommandAvailability::Always,
    action: CommandAction::LocalToggle {
        toggle: LocalToggle::ClearTranscript,
    },
};

pub const QUIT_COMMAND: CommandDef = CommandDef {
    name: "quit",
    aliases: &["exit"],
    description: "local toggle: exit the TUI",
    args_hint: "",
    availability: CommandAvailability::Always,
    action: CommandAction::LocalToggle {
        toggle: LocalToggle::Quit,
    },
};

pub const EVENTS_COMMAND: CommandDef = CommandDef {
    name: "events",
    aliases: &["ps"],
    description: "local toggle: show daemon events and workers pane",
    args_hint: "",
    availability: CommandAvailability::Always,
    action: CommandAction::LocalToggle {
        toggle: LocalToggle::Events,
    },
};

pub const HELP_COMMAND: CommandDef = CommandDef {
    name: "help",
    aliases: &["?"],
    description: "generated help: show active keymap bindings",
    args_hint: "",
    availability: CommandAvailability::Always,
    action: CommandAction::ShowInfo {
        topic: InfoTopic::Help,
    },
};

pub const KEYMAP_COMMAND: CommandDef = CommandDef {
    name: "keymap",
    aliases: &[],
    description: "generated help: show active keymap bindings",
    args_hint: "",
    availability: CommandAvailability::Always,
    action: CommandAction::ShowInfo {
        topic: InfoTopic::Help,
    },
};

pub const THEME_COMMAND: CommandDef = CommandDef {
    name: "theme",
    aliases: &[],
    description: "theme picker: apply a built-in TUI theme live",
    args_hint: "[dark|light|plain]",
    availability: CommandAvailability::Always,
    action: CommandAction::Misc {
        command: MiscCommand::Theme,
    },
};

pub const VIM_COMMAND: CommandDef = CommandDef {
    name: "vim",
    aliases: &[],
    description: "local toggle: enable or disable composer vim mode",
    args_hint: "",
    availability: CommandAvailability::Always,
    action: CommandAction::Misc {
        command: MiscCommand::ToggleVim,
    },
};

pub const DEBUG_CONFIG_COMMAND: CommandDef = CommandDef {
    name: "debug-config",
    aliases: &["debug"],
    description: "info: show TUI config path, theme, vim, and command registry",
    args_hint: "",
    availability: CommandAvailability::Always,
    action: CommandAction::Misc {
        command: MiscCommand::DebugConfig,
    },
};

pub const COPY_COMMAND: CommandDef = CommandDef {
    name: "copy",
    aliases: &[],
    description: "terminal clipboard: copy the last assistant message via OSC52",
    args_hint: "",
    availability: CommandAvailability::Always,
    action: CommandAction::Misc {
        command: MiscCommand::CopyLastAssistant,
    },
};

pub const RAW_COMMAND: CommandDef = CommandDef {
    name: "raw",
    aliases: &[],
    description: "local overlay: open transcript raw and copy view",
    args_hint: "",
    availability: CommandAvailability::Always,
    action: CommandAction::Misc {
        command: MiscCommand::RawTranscript,
    },
};

pub const SUBAGENTS_COMMAND: CommandDef = CommandDef {
    name: "subagents",
    aliases: &["multi-agents"],
    description: "local info: list active and recent subagent activity",
    args_hint: "",
    availability: CommandAvailability::Always,
    action: CommandAction::Misc {
        command: MiscCommand::Subagents,
    },
};

pub const UNAVAILABLE_COMMANDS: &[CommandDef] = &[
    unavailable(
        "side",
        &[],
        "side conversations have no Refact daemon chat command",
    ),
    unavailable(
        "btw",
        &[],
        "background side-note routing has no Refact daemon chat command",
    ),
    unavailable(
        "skills",
        &[],
        "skills marketplace and editor are GUI-only today",
    ),
    unavailable("hooks", &[], "hook editing is GUI-only today"),
    unavailable("memories", &[], "memory browsing is not exposed in the TUI"),
    unavailable(
        "mcp",
        &[],
        "MCP configuration and marketplace are not exposed in the TUI",
    ),
    unavailable("apps", &[], "Refact daemon has no apps command surface"),
    unavailable(
        "plugins",
        &[],
        "plugin marketplace management is not exposed in the TUI",
    ),
    unavailable("ide", &[], "IDE attach state is not exposed in the TUI"),
    unavailable(
        "statusline",
        &[],
        "terminal statusline preferences are not exposed in the TUI",
    ),
    unavailable("pets", &["pet"], "Buddy pets are GUI-only today"),
    unavailable(
        "personality",
        &[],
        "Buddy personality settings are GUI-only today",
    ),
    unavailable(
        "realtime",
        &[],
        "realtime voice controls are GUI-only today",
    ),
    unavailable(
        "settings",
        &[],
        "interactive settings are GUI-only; edit the TUI config file for keymap and theme",
    ),
    unavailable("feedback", &[], "feedback submission has no TUI endpoint"),
    unavailable(
        "logout",
        &[],
        "daemon auth/logout is not enabled for the TUI",
    ),
    unavailable(
        "rollout",
        &[],
        "rollout controls have no Refact daemon endpoint",
    ),
    unavailable(
        "approve",
        &[],
        "approval decisions happen in the approval modal or /permissions picker",
    ),
    unavailable(
        "test-approval",
        &[],
        "synthetic approval injection is not part of the release TUI",
    ),
    unavailable(
        "app",
        &[],
        "Refact daemon has no app chooser command surface",
    ),
    unavailable(
        "experimental",
        &[],
        "experimental Codex flags have no Refact TUI equivalent",
    ),
    unavailable(
        "setup-default-sandbox",
        &[],
        "Codex sandbox defaults do not apply to Refact daemon chats",
    ),
    unavailable(
        "sandbox-add-read-dir",
        &[],
        "Codex sandbox read directories do not apply to Refact daemon chats",
    ),
    unavailable(
        "debug-m-drop",
        &[],
        "Codex model-debug mutation has no Refact backend surface",
    ),
    unavailable(
        "debug-m-update",
        &[],
        "Codex model-debug mutation has no Refact backend surface",
    ),
];

const fn unavailable(
    name: &'static str,
    aliases: &'static [&'static str],
    reason: &'static str,
) -> CommandDef {
    CommandDef {
        name,
        aliases,
        description: reason,
        args_hint: "",
        availability: CommandAvailability::Always,
        action: CommandAction::Unavailable { reason },
    }
}

pub fn theme_picker_items() -> Vec<PickerItem> {
    TuiTheme::builtin_names()
        .iter()
        .map(|name| PickerItem {
            id: (*name).to_string(),
            title: format!("{} theme", name),
            description: format!("Apply the built-in {} theme live", name),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_picker_lists_every_builtin_theme() {
        let ids = theme_picker_items()
            .into_iter()
            .map(|item| item.id)
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            TuiTheme::builtin_names()
                .iter()
                .map(|name| (*name).to_string())
                .collect::<Vec<_>>()
        );
    }
}
