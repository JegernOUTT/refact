pub mod app;
pub mod approvals;
pub mod client;
pub mod events_pane;
pub mod pickers;
pub mod render;
pub mod terminal;
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
