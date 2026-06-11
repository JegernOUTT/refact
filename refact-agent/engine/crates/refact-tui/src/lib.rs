pub mod app;
pub mod client;
pub mod render;
pub mod terminal;
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
