use super::super::*;

impl App {
    pub(super) fn handle_task_board_key(&mut self, key: KeyEvent) -> AppAction {
        let dispatch = self.keymap.dispatch(KeyContext::Board, key);
        match dispatch.action {
            Some(KeyAction::Cancel) => {
                self.board_surface = None;
                AppAction::None
            }
            Some(KeyAction::MoveDown) => {
                if let Some(board) = self.board_surface.as_mut() {
                    board.select_next();
                }
                AppAction::None
            }
            Some(KeyAction::MoveUp) => {
                if let Some(board) = self.board_surface.as_mut() {
                    board.select_previous();
                }
                AppAction::None
            }
            Some(KeyAction::ToggleSelectedTool) => {
                if let Some(board) = self.board_surface.as_mut() {
                    board.toggle_detail();
                }
                AppAction::None
            }
            Some(KeyAction::Accept) => {
                let target = self
                    .board_surface
                    .as_ref()
                    .and_then(|board| board.selected_agent_chat());
                match target {
                    Some((chat_id, title)) => self.resume_chat(chat_id, title, None),
                    None => {
                        self.add_notice("The selected card has no agent chat to open");
                        AppAction::None
                    }
                }
            }
            _ => AppAction::None,
        }
    }
}
