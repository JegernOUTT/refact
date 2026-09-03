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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_state_can_be_constructed_empty() {
        let state = CapsState {
            caps: None,
            reading_lock: Arc::new(AMutex::new(false)),
            last_error: String::new(),
            last_attempted_ts: 0,
            models_dev_startup_refresh_attempted: false,
            loaded_project_root: None,
        };

        assert!(state.caps.is_none());
        assert!(state.last_error.is_empty());
        assert_eq!(state.last_attempted_ts, 0);
        assert!(!state.models_dev_startup_refresh_attempted);
        assert!(!state.project_root_changed(&Some(PathBuf::from("/project"))));
    }

    #[test]
    fn project_root_changed_only_for_tracked_loaded_caps() {
        let mut state = CapsState {
            caps: Some(Arc::new(CodeAssistantCaps::default())),
            reading_lock: Arc::new(AMutex::new(false)),
            last_error: String::new(),
            last_attempted_ts: 0,
            models_dev_startup_refresh_attempted: false,
            loaded_project_root: None,
        };
        let project = Some(PathBuf::from("/project"));

        assert!(!state.project_root_changed(&project));

        state.loaded_project_root = Some(None);
        assert!(state.project_root_changed(&project));

        state.loaded_project_root = Some(project.clone());
        assert!(!state.project_root_changed(&project));
        assert!(state.project_root_changed(&None));
    }
}
