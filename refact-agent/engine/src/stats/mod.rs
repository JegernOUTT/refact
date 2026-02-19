pub mod event;
pub mod reader;
pub mod writer;

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;
use crate::global_context::GlobalContext;

pub async fn get_stats_dir(gcx: Arc<ARwLock<GlobalContext>>) -> PathBuf {
    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    if let Some(first) = project_dirs.first() {
        first.join(".refact").join("stats")
    } else {
        gcx.read().await.config_dir.join("stats")
    }
}
