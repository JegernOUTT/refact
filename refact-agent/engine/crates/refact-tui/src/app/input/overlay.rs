use super::super::*;

impl App {
    pub(super) fn handle_transcript_overlay_key(&mut self, key: KeyEvent) -> AppAction {
        let Some(overlay) = self.transcript_overlay.as_mut() else {
            return AppAction::None;
        };
        let context = if overlay.search_input().is_some() {
            KeyContext::OverlaySearch
        } else {
            KeyContext::Overlay
        };
        let dispatch = self.keymap.dispatch(context, key);
        match overlay.handle_dispatch(dispatch) {
            PagerAction::None => AppAction::None,
            PagerAction::Close => {
                self.transcript_overlay = None;
                AppAction::None
            }
            PagerAction::Yank => {
                let height = self.transcript_overlay_visible_height.unwrap_or(100);
                self.copy_visible_overlay_text(height)
            }
        }
    }

    pub(super) fn handle_transcript_overlay_paste(&mut self, text: &str) {
        if let Some(overlay) = self.transcript_overlay.as_mut() {
            overlay.insert_search_text(text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_contexts_are_distinct() {
        assert_ne!(KeyContext::Overlay, KeyContext::OverlaySearch);
    }
}
