use super::super::*;

impl App {
    pub(super) fn handle_settings_key(&mut self, key: KeyEvent) -> AppAction {
        let dispatch = self.keymap.dispatch(KeyContext::Settings, key);
        match dispatch.action {
            Some(KeyAction::Cancel) => {
                self.settings_surface = None;
                AppAction::None
            }
            Some(KeyAction::Accept) => self
                .settings_surface
                .as_mut()
                .and_then(|settings| settings.activate())
                .map(|patch| AppAction::SetParams { patch })
                .unwrap_or(AppAction::None),
            Some(KeyAction::MoveUp) => {
                if let Some(settings) = self.settings_surface.as_mut() {
                    settings.select_previous();
                }
                AppAction::None
            }
            Some(KeyAction::MoveDown) => {
                if let Some(settings) = self.settings_surface.as_mut() {
                    settings.select_next();
                }
                AppAction::None
            }
            Some(KeyAction::MoveHome) => {
                if let Some(settings) = self.settings_surface.as_mut() {
                    settings.select_first();
                }
                AppAction::None
            }
            Some(KeyAction::MoveEnd) => {
                if let Some(settings) = self.settings_surface.as_mut() {
                    settings.select_last();
                }
                AppAction::None
            }
            Some(KeyAction::Backspace) => {
                if let Some(settings) = self.settings_surface.as_mut() {
                    settings.pop_text();
                }
                AppAction::None
            }
            None => {
                if let Some(text) = dispatch.text {
                    if let Some(settings) = self.settings_surface.as_mut() {
                        settings.push_text(&text.to_string());
                    }
                }
                AppAction::None
            }
            Some(_) => AppAction::None,
        }
    }

    pub(super) fn handle_settings_paste(&mut self, text: &str) {
        if let Some(settings) = self.settings_surface.as_mut() {
            settings.push_text(text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::OpenProjectResponse;
    use serde_json::json;
    use std::path::PathBuf;

    fn app_with_settings() -> App {
        let mut app = App::new(OpenProjectResponse {
            project_id: "p".to_string(),
            slug: "demo".to_string(),
            root: PathBuf::from("/tmp/demo"),
            pinned: None,
            worker: None,
            cron_pending: None,
        });
        app.test_set_thread_params(json!({"include_project_info": true}));
        app.test_open_settings_surface();
        app
    }

    #[test]
    fn settings_toggles_dispatch_single_set_params_patch() {
        let mut app = app_with_settings();
        for _ in 0..10 {
            app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::empty()));
        }

        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())),
            AppAction::SetParams {
                patch: json!({"include_project_info": false}),
            }
        );
    }
}
