use crate::ui::goal_dock::{self, GoalPresentation};

use super::*;

impl App {
    fn show_current_goal(&mut self) -> AppAction {
        self.composer.clear();
        if !self.goal_surfaces_enabled() {
            return match current_goal_cell_data(self.transcript_state.messages()) {
                Some(goal) => {
                    self.push_history_item(TranscriptItem::Goal(goal));
                    AppAction::None
                }
                None => {
                    self.add_notice("No current goal is installed for this chat");
                    AppAction::None
                }
            };
        }
        match self.goal_presentation() {
            Some(goal) => {
                self.goal_overlay_open = true;
                if let Some(cell) = current_goal_cell_data(self.transcript_state.messages()) {
                    self.push_history_item(TranscriptItem::Goal(goal.to_goal_cell_data(cell)));
                }
            }
            None => self.add_notice("No current goal is installed for this chat"),
        }
        AppAction::None
    }

    pub(crate) fn goal_surfaces_enabled(&self) -> bool {
        goal_dock::surfaces_enabled()
    }

    pub(crate) fn goal_presentation(&self) -> Option<GoalPresentation> {
        GoalPresentation::from_messages(
            self.transcript_state.messages(),
            self.runtime_snapshot.as_ref(),
        )
    }

    pub(crate) fn goal_overlay_open(&self) -> bool {
        self.goal_overlay_open && self.goal_surfaces_enabled() && self.goal_presentation().is_some()
    }

    fn goal_control_action(&mut self, action: GoalControlAction) -> AppAction {
        let Some(goal) = self.goal_presentation() else {
            self.add_notice("No current goal is installed for this chat");
            return AppAction::None;
        };
        if !goal.allows(action) {
            self.add_notice("That goal control is not available for the current goal status");
            return AppAction::None;
        }
        AppAction::GoalControl { action }
    }

    pub(super) fn execute_goal_command(&mut self, args: &str, allow_set: bool) -> AppAction {
        let args = args.trim();
        if args.is_empty() {
            return self.show_current_goal();
        }
        if !self.goal_surfaces_enabled() {
            self.add_notice("Goal controls require REFACT_TUI_SURFACES=1");
            return AppAction::None;
        }
        let (action, rest) = split_command_name_and_args(args);
        match action {
            "set" => {
                if !allow_set {
                    self.add_notice("A goal is already installed; use /goal update to evolve it");
                    return AppAction::None;
                }
                if rest.trim().is_empty() {
                    self.add_notice("Usage: /goal set <goal text>");
                    AppAction::None
                } else {
                    AppAction::GoalCommand {
                        kind: GoalCommandKind::Set,
                        content: Some(rest.trim().to_string()),
                        budget: None,
                    }
                }
            }
            "update" => {
                if rest.trim().is_empty() {
                    self.add_notice("Usage: /goal update <note>");
                    AppAction::None
                } else {
                    AppAction::GoalCommand {
                        kind: GoalCommandKind::Update,
                        content: Some(rest.trim().to_string()),
                        budget: None,
                    }
                }
            }
            "budget" => {
                let values = rest.split_whitespace().collect::<Vec<_>>();
                if values.len() > 5 {
                    self.add_notice(
                        "Usage: /goal budget [turns] [minutes] [tokens] [cost_cents] [no_progress]",
                    );
                    return AppAction::None;
                }
                let budget = goal_dock::budget_from_inputs(
                    values.first().copied().unwrap_or_default(),
                    values.get(1).copied().unwrap_or_default(),
                    values.get(2).copied().unwrap_or_default(),
                    values.get(3).copied().unwrap_or_default(),
                    values.get(4).copied().unwrap_or_default(),
                );
                match budget {
                    Ok(budget) => AppAction::GoalCommand {
                        kind: GoalCommandKind::SetBudget,
                        content: None,
                        budget: Some(budget),
                    },
                    Err(error) => {
                        self.add_notice(error);
                        AppAction::None
                    }
                }
            }
            "pause" => self.goal_control_action(GoalControlAction::Pause),
            "resume" => self.goal_control_action(GoalControlAction::Resume),
            "stop" => self.goal_control_action(GoalControlAction::Stop),
            _ => {
                self.add_notice(
                    "Usage: /goal [set <text>|update <note>|budget [turns minutes tokens cost no_progress]|pause|resume|stop]",
                );
                AppAction::None
            }
        }
    }

    pub(crate) fn handle_goal_overlay_key(&mut self, key: KeyEvent) -> AppAction {
        let dispatch = self.keymap.dispatch(KeyContext::Goal, key);
        if matches!(dispatch.action, Some(KeyAction::Cancel)) {
            self.goal_overlay_open = false;
            return AppAction::None;
        }
        let action = match dispatch.action {
            Some(KeyAction::GoalPause) => GoalControlAction::Pause,
            Some(KeyAction::GoalResume) => GoalControlAction::Resume,
            Some(KeyAction::GoalStop) => GoalControlAction::Stop,
            _ => return AppAction::None,
        };
        self.goal_control_action(action)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum GoalCommandKind {
    Set,
    SetBudget,
    Update,
}
