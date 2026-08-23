use super::super::*;

impl App {
    pub(super) fn handle_vim_key(&mut self, key: KeyEvent) -> Option<AppAction> {
        if !self.vim.enabled() {
            return None;
        }
        let dispatch = self.keymap.dispatch(self.vim.context(), key);
        let outcome = self.vim.handle_dispatch(dispatch);
        if !outcome.consumed {
            return None;
        }
        self.input_queue.clear_selection();
        match outcome.effect {
            VimEffect::None => {}
            VimEffect::MoveLeft => self.composer.move_left(false),
            VimEffect::MoveDown => self.composer.move_down_or_history(false),
            VimEffect::MoveUp => self.composer.move_up_or_history(false),
            VimEffect::MoveRight => self.composer.move_right(false),
            VimEffect::WordForward => self.composer.move_word_forward(false),
            VimEffect::WordBackward => self.composer.move_word_backward(false),
            VimEffect::LineStart => self.composer.move_home(false),
            VimEffect::LineEnd => self.composer.move_end(false),
            VimEffect::DeleteLine => self.composer.delete_current_line(),
            VimEffect::Append => self.composer.move_right(false),
            VimEffect::OpenBelow => self.composer.open_line_below(),
        }
        Some(AppAction::None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vim_modes_are_distinct() {
        assert_ne!(VimMode::Normal, VimMode::Insert);
    }
}
