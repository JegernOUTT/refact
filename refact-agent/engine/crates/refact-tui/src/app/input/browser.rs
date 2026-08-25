use super::super::*;

impl App {
    pub(super) fn handle_browser_key(&mut self, key: KeyEvent) -> AppAction {
        let dispatch = self.keymap.dispatch(KeyContext::Browser, key);
        match dispatch.action {
            Some(KeyAction::Cancel) => {
                self.browser_surface = None;
                AppAction::None
            }
            Some(KeyAction::MoveUp) => {
                if let Some(surface) = self.browser_surface.as_mut() {
                    surface.move_context_selection(-1);
                }
                AppAction::None
            }
            Some(KeyAction::MoveDown) => {
                if let Some(surface) = self.browser_surface.as_mut() {
                    surface.move_context_selection(1);
                }
                AppAction::None
            }
            Some(KeyAction::ToggleSelectedTool) => {
                if self.browser_state.context_prompt.is_some() {
                    if let Some(surface) = self.browser_surface.as_mut() {
                        surface.toggle_context_selection();
                    }
                }
                AppAction::None
            }
            Some(KeyAction::Accept) if self.browser_state.context_prompt.is_some() => {
                let options = self
                    .browser_surface
                    .as_ref()
                    .map(|surface| surface.context_options());
                options
                    .map(|options| self.submit_browser_context_decision(options))
                    .unwrap_or(AppAction::None)
            }
            _ => AppAction::None,
        }
    }
}
