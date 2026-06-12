pub mod app;
pub mod approvals;
pub mod client;
pub mod commands;
pub mod composer;
pub mod events_pane;
pub mod history;
pub mod keymap;
pub mod overlay;
pub mod pickers;
pub mod protocol;
pub mod render;
pub mod sessions;
pub mod streaming;
pub mod terminal;
pub mod theme;
pub mod tools;
pub mod ui;
pub mod vendored;

use std::path::PathBuf;

pub async fn run_tui(
    daemon_url: Option<String>,
    project_hint: Option<PathBuf>,
) -> Result<(), app::TuiError> {
    let options = app::TuiOptions {
        daemon_url,
        project_hint,
    };
    app::run(options).await
}
