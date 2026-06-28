pub mod app;
pub mod approvals;
pub mod ask_questions;
pub mod client;
pub mod clipboard;
pub mod color;
pub mod commands;
pub mod composer;
pub mod diff_model;
pub mod events_pane;
pub mod history;
pub mod key_hint;
pub mod keymap;
pub mod notifications;
pub mod overlay;
pub mod pickers;
pub mod protocol;
pub mod read_only_views;
pub mod render;
pub mod sessions;
pub mod streaming;
pub mod style;
pub mod table_detect;
pub mod terminal;
pub mod terminal_palette;
mod terminal_probe;
pub mod text_formatting;
pub mod text_safety;
pub mod theme;
pub mod tools;
pub mod ui;
pub mod ui_consts;
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
