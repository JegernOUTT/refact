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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{
        TaskBoardCard, TaskBoardReadyCards, TaskBoardResponse, TaskBoardTask, TaskBoardViewData,
    };

    fn board_data() -> TaskBoardViewData {
        TaskBoardViewData {
            task: TaskBoardTask {
                id: "task-1".to_string(),
                name: "Board".to_string(),
                status: "active".to_string(),
            },
            board: TaskBoardResponse {
                cards: vec![
                    TaskBoardCard {
                        id: "T-1".to_string(),
                        title: "No chat".to_string(),
                        column: "planned".to_string(),
                        ..TaskBoardCard::default()
                    },
                    TaskBoardCard {
                        id: "T-2".to_string(),
                        title: "Linked chat".to_string(),
                        column: "doing".to_string(),
                        agent_chat_id: Some("agent-chat-2".to_string()),
                        ..TaskBoardCard::default()
                    },
                ],
                ..TaskBoardResponse::default()
            },
            ready: TaskBoardReadyCards {
                ready: vec!["T-1".to_string()],
                ..TaskBoardReadyCards::default()
            },
        }
    }

    #[test]
    fn accepting_a_linked_card_opens_its_agent_chat() {
        let mut app = App::notice_only("test");
        app.show_task_board(board_data());

        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::empty())),
            AppAction::None
        );
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())),
            AppAction::SubscribeCurrent
        );
        assert_eq!(app.chat_id(), "agent-chat-2");
    }
}
