use super::super::*;

impl App {
    pub(super) fn handle_worktree_key(&mut self, key: KeyEvent) -> AppAction {
        let dispatch = self.keymap.dispatch(KeyContext::Worktree, key);
        match dispatch.action {
            Some(KeyAction::Accept) => self
                .pending_worktree_merge
                .take()
                .map(|confirmation| AppAction::Worktree {
                    action: surfaces::WorktreeAction::Merge { confirmation },
                })
                .unwrap_or(AppAction::None),
            Some(KeyAction::Cancel) => {
                self.pending_worktree_merge = None;
                self.add_notice("Worktree merge canceled; no local merge or push was performed");
                AppAction::None
            }
            _ => AppAction::None,
        }
    }
}
