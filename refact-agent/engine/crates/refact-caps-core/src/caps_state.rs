use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex as AMutex;

use crate::code_assistant_caps::CodeAssistantCaps;

#[derive(Clone)]
pub struct CapsState {
    pub caps: Option<Arc<CodeAssistantCaps>>,
    pub reading_lock: Arc<AMutex<bool>>,
    pub last_error: String,
    pub last_attempted_ts: u64,
    pub models_dev_startup_refresh_attempted: bool,
    pub loaded_project_root: Option<Option<PathBuf>>,
}

impl CapsState {
    pub fn project_root_changed(&self, current: &Option<PathBuf>) -> bool {
        self.caps.is_some()
            && self
                .loaded_project_root
                .as_ref()
                .is_some_and(|loaded| loaded != current)
    }
}
